# Rust-first native system contracts

This page records the **rust-first** replacement of the Scala-legacy genesis bootstrap: the
registry, Proof-of-Stake state, and REV vault are no longer rholang contracts (`Registry.rho`,
`Pos.rhox`, `RevVault.rho`, …) that re-implement a `TreeHashMap` trie *in interpreted rholang*.
They are now **native Rust state + system processes**. Scala serves as a *checklist* of required
behavior only (per the prime directive in [`AGENTS.md`](../AGENTS.md)); the fragile trie and the
registry-bootstrap echo are gone.

## Why

The blessed `Registry.rho` built a depth-4 keccak-256 nybble trie with 16-bit bitmasks, using peek
receives, persistent sends, list-channels (`@[node, *storeToken]`) and tuple-channels
(`@(map, "depth")`). Any interpreter bug made the registry **silently empty** — genesis "succeeded"
while `compute_bonds` returned zero. Native state makes that failure mode unrepresentable: the
bonds/registry/vault live in typed Rust maps, folded into the same content-addressed radix trie as
the tuple space.

## State model

The block state hash *is* the radix-trie root of the tuple space; there is no second state
component. Native state therefore enters the same trie, under dedicated prefixes:

| Prefix | Byte | Content | Leaf key |
|---|---|---|---|
| `PREFIX_REGISTRY` | `0x03` | registry `uri → Par` | `blake2b256(uri)` |
| `PREFIX_POS` | `0x04` | PoS state (see below) | `blake2b256(b"pos:…")` |
| `PREFIX_VAULT` | `0x05` | vault `address → NonNegI64` | `blake2b256(address)` |
| `PREFIX_TXN` | `0x06` | cross-shard 2PC records (`txn-id → TxnRecord`) | `blake2b256(txn-id)` |
| `PREFIX_HTTP` | `0x07` | HTTP-result oracle (`url → (value, block)`) | `blake2b256(b"http:records")` |

The PoS leaves under `PREFIX_POS`:

| Leaf | Content |
|---|---|
| `pos:bonds` | the full bond **pool** (`Validator → NonNegI64`) |
| `pos:active` | the **active** consensus validator set (top-N of the pool) |
| `pos:trusted` | the trusted validator-**stakeholder** set (admission gate) |
| `pos:withdrawers` | pending withdrawers (`Validator → quarantine deadline`) |
| `pos:params` | immutable PoS parameters (min/max bond, epoch, quarantine, active cap) |
| `pos:coop` | the Coop slashing-vault balance (confiscated stake) |
| `pos:vault` | the staking-vault balance — escrowed bonds + the phlo that funds the rewards |

Leaves are the new `PersistedData::NativeLeaf(Vec<u8>)` (the previously-free 2-bit tag `3`). The
trie prefix disambiguates registry vs PoS vs vault, so a single leaf kind suffices.

`PREFIX_HTTP` holds the deterministic HTTP-result oracle (RCHIP #54) in a single leaf,
`http:records`, mapping `url → (value, captured block)`. `record` is **first writer wins**: the value
is asserted inside a signed deploy, so replay/validation never performs a network fetch and later
deploys compare against the record rather than re-fetching.

The typed layer is `rholang/src/native_state.rs` (`NativeSystemState`), wrapping the byte-oriented
`rspace/src/native_store.rs` (`InMemNativeStore`):

- `InMemNativeStore` is a write-through overlay on a `NativeHistoryReader`: reads fall through to the
  persisted trie, writes buffer in an overlay + tombstone set, and `drain_changes` emits the
  `NativeStoreAction::{Put,Delete}`s folded into the next checkpoint via
  `HistoryRepository::checkpoint_with_native`.
- `NativeSystemState` exposes typed accessors — `bonds()`/`set_bonds()` (the pool),
  `active()`/`set_active()` (the consensus set), `trusted()`/`set_trusted()`,
  `withdrawers()`/`set_withdrawers()`, `params()`/`set_params()`, `coop_balance()`/`set_coop_balance()`,
  `pos_vault_balance()`/`set_pos_vault_balance()`, `vault_balance()`/`set_vault_balance()`, and
  `registry_lookup()`/`registry_insert()`, with canonical byte encodings (sorted `BTreeMap`,
  fixed-width `Validator` + little-endian stake).

## The staking vault (`pos:vault`)

Every REV movement in the mechanism is a transfer between a user's vault, the **staking vault** (the
contract's `posVault`), and the Coop multisig vault — so nothing is minted and nothing is burned:

| Step | Effect | Scala |
|---|---|---|
| bond | validator vault → staking vault | `Pos.rhox:397-404` (`deposit!(deployerId, amount, posVaultAddr)`) |
| pre-charge | deployer vault → staking vault | same (`chargeDeploy`) |
| refund | staking vault → deployer vault | `Pos.rhox:417-454` (`refundDeploy`) |
| slash | staking vault → Coop vault | `Pos.rhox:470-482` |
| withdrawal | staking vault → validator vault | `Pos.rhox:556-567` (`payWithdrawer`) |

The genesis install funds the vault with **exactly** the initial bond sum (`Pos.rhox:167-174`), so at
genesis the distributable *pot* (`vault − bonded − withdrawers − committed rewards`) is zero. The pot
is the phlo the deploys since the last epoch boundary actually burned: the charge goes in and the
unused surplus comes straight back out. A debit the vault cannot cover is a **platform error** (the
deploy fails) rather than a partial payment — the Scala's `payWithdrawer` ignores its failed transfer
and drops the withdrawer anyway (`// FIXME fix transfer in failure case`), which loses the bond.
`install_genesis` also returns `Result` now: a bond sum that does not fit an `i64` is a genesis that
cannot be installed, not one to clamp.

## Dynamic validators

The validator lifecycle is native and on-chain (`rholang/src/native_state.rs`):

1. **observer** — any key that is not active/bonded. Any node may run as an observer (no validator
   key, or a key that is not in the active set); it syncs and serves but does not propose.
2. **trusted** — admission into the validator *stakeholder group* (`pos:trusted`). Only a trusted key
   may bond. A genesis validator is trusted by construction; a trusted stakeholder admits a new key
   via `rho:rchain:pos!("trust", *deployerId, targetPubKey, *ret)`, and revokes via `"untrust"`.
3. **bonded → active** — `"bond"` checks trust, `[minimum_bond, maximum_bond]`, and the deployer's
   REV vault, moves the stake into the staking vault, inserts it into the pool, and recomputes
   `pos:active` (top `number_of_active_validators` by descending stake, deterministic tie-break). A
   bonded validator is immediately eligible to propose; the block's bond cache is the active set at
   the block post-state.
4. **withdrawing** — `"withdraw"` deactivates the validator immediately and escrows the stake until
   the quarantine deadline; `close_block` refunds it.
5. **removed** — `slash` (consensus, for bonded offenders) and `untrust` (governance) remove the
   validator and confiscate the stake to the Coop vault (`pos:coop`).

`compute_bonds` (used by finality, the supermajority check, and `Validate::bonds_cache`) reads
`pos:active`; `getBonds` returns the pool and `getActiveValidators` the active set.

## Replay determinism

Native mutations are folded into the radix root, so replay reproduces them **only** if they are pure
functions of `(deploy, random_state)` — never wall-clock time or OS entropy. The system-deploy
operations (`pre_charge`/`refund`/`close_block`/`slash`) and the genesis install
(`compute_genesis(…, pos_genesis)`) obey this; `replay_compute_state` re-installs the genesis PoS
state (pool, trusted set, params, derived active set, the Coop vault and the staking vault) and the
genesis vaults on the genesis replay
(`with_cost_accounting == false`) so the replayed root matches the play root. This is
asserted by `casper/tests/consensus.rs::empty_state_hash_fixed_matches_runtime` and
`genesis_deploy_replay_recomputes_state`.

## System processes

The native `rho:*` protocol is installed as ordinary system-process `Definition`s:

- `rho:registry:lookup` / `insertArbitrary` / `insertSigned:secp256k1` — backed by the native
  registry map.
- `rho:rchain:pos` — `getBonds` (pool) → `RhoMap`, `getActiveValidators` (active set) → `RhoSet`,
  `bond` / `withdraw` (validator lifecycle), and `trust` / `untrust` (stakeholder admission) via a
  `remainder` install pattern.
- `rho:rchain:revVault` / `multiSigRevVault` — `getBalance` / `deposit` / `transfer` / `findOrCreate`
  over the vault balance map.
- `rho:block:data` — the current block's number, sender and informational timestamp.
- `rho:io:http` — the deterministic HTTP-result oracle (RCHIP #54): `record` (first writer wins),
  `get`, `check`, `height`, over the `http:records` leaf.

The `bond` (trust + min/max + vault-funds + `(validator, stake)` into the pool and the staking vault,
recomputing the active set), `withdraw` (immediate deactivation, quarantined refund), `trust`/`untrust` (stakeholder
admission/revocation), `slash` (confiscation to the Coop vault) and vault `findOrCreate` methods are
implemented natively, returning the `(Bool, Either)` result the PoS/vault contracts expect. **Still
deferred:** reward computation/distribution and the vault **unforgeable-name capability** (the
simplified model keys vaults by REV address, so `findOrCreate` returns the address rather than a fresh
unforgeable).

**Also deferred, and previously unregistered:** the **`revvaultexport`** offline tooling
(`legacy/node/src/main/scala/coop/rchain/node/revvaultexport/`, seven files — the rho-trie traverser, the
balance getter, and the mainnet1 balance/reporting mains). No Rust module and no CLI subcommand exist for
it; `docs/src/contributor/architecture.md` listed it as a done feature until that claim was checked and
corrected. A deferral registered here is a decision; an unregistered one was an omission.

`default_blessed_terms` installs only the interpreted contracts a consumer actually reaches through
`rho:registry:lookup` — `ListOps`, `NonNegativeNumber` and `MakeMint` — plus the registry aliases that
make those lookups (and the native `rho:rchain:revVault` / `rho:rchain:pos` channels) resolve. The
vault and PoS **system** contracts are not installed as sources: their state is native
(`install_genesis`), `compute_bonds`/`get_active_validators` read the `pos:active` leaf directly (no
rholang exploratory deploy), and the pre-charge/refund/close-block/slash system deploys carry a
`NativeSystemDeployOp` rather than routing through `rho:registry:lookup` + `Pos.rhox`. Installing the
interpreted equivalents would shadow consensus-critical logic with slower, less auditable copies.
`spec/GENESIS.md` is the manifest (what is installed, why, and what is deliberately excluded).

## Cross-links

- [`spec/AUDIT.md`](AUDIT.md) §9 — the rust-first fragility audit that motivated this.
- [`spec/RHO-CALCULUS.md`](RHO-CALCULUS.md) — the ρ-calculus core this realizes.
- [`spec/TYPE-SYSTEM.md`](TYPE-SYSTEM.md) — the no-silent-partiality discipline.

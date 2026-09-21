# Genesis content — the manifest

Genesis content is **consensus identity**: every node of a network must agree on it, and changing it
later is a genesis change, not a deploy. So this list is deliberately small, and every entry names the
consumer that justifies it. `casper/src/genesis/standard_deploys.rs::GENESIS_ALIASES` is the
machine-readable form of the alias table below; `spec/TEST-COVERAGE.md` pins it with tests.

Before this landed, `default_blessed_terms` returned `Vec::new()`: a fresh chain's native registry was
empty, so `lookup!(\`rho:rchain:revVault\`, *ch)` answered `Nil`. A `Nil` reply is **silent** — an
unmatched `for` is not an error, it just never fires — which is why the failure presented to
consumers as their own bug (and why the rgov family returned `[]` with no diagnostic).

## What genesis installs

| # | Contract | Installed | Shorthand seeded | `rho:id` (hardcodable) | Consumer it unblocks |
|---|---|---|---|---|---|
| 1 | `ListOps.rho` | blessed deploy (`LIST_OPS_PK`) | `rho:lang:listOps` | `rho:id:6fzorimqngeedepkrizgiqms6zjt76zjeciktt1eifequy4osz3o` | rgov `rholang/core/CrowdFund.rho:8` — `lookup!` then `@(_, *ListOps)` then `ListOps!("fold", …)` |
| 2 | `NonNegativeNumber.rho` | blessed deploy (`NON_NEGATIVE_NUMBER_PK`) | `rho:lang:nonNegativeNumber` | `rho:id:hxyadh1ffypra47ry9mk6b8r1i33ar1w9wjsez4khfe9huzrfcyo` | `MakeMint.rho:27` looks this up *during its own deploy*, so it must be seeded for makeMint to install at all |
| 3 | `MakeMint.rho` (adapted, below) | blessed deploy (`MAKE_MINT_PK`) | `rho:rchain:makeMint` | `rho:id:asysrwfgzf8bf7sxkiowp4b3tcsy4f8ombi3w96ysox4u3qdmn1o` | rgov `src/actions/makeMint.rho:13` and wallet `snippets.ts:751` — `lookup!` then `@(nonce, *MakeMint)` then `MakeMint!(*ch)` |

Note on the URIs: they are this port's own zbase32 encoding of `blake2b256(deployer public key)`
(`rholang/src/registry.rs:43`), which deliberately does not reproduce the Scala's CRC14+ZBase32 bit
order (`registry.rs:3-6`). They are **stable across chains of this port** because each blessed deploy's
key is a fixed constant — that is what makes them hardcodable — but they are **not** the 54-char
mainnet values carried in the `.rho` header comments.

## Native system channels, aliased

The PoS and vault channels are native (`rholang/src/system_processes.rs::definitions`), so only the
*registry alias* was missing. Each resolves to `(9223372036854775807, bundle+{channel})` — the
`(nonce, value)` shape `rho:registry:insertSigned:secp256k1` stores, which is what consumers
destructure.

| Shorthand | Channel | Consumer it unblocks |
|---|---|---|
| `rho:rchain:revVault` | native, arity-1 + remainder | wallet `src/utils/rho.ts:7,18`; rgov `src/actions/transfer.rho:4`, `checkBalance.rho:10` — `lookup!` then `@(_, RevVault)` then the vault methods |
| `rho:rchain:pos` | native, arity-1 + remainder | wallet bonding `src/utils/rho.ts:29` — `lookup!` then `@(_, PoS)` then `PoS!("bond", …)` |

## Excluded, with reasons

Nothing here is excluded for being hard; each is excluded because no consumer reaches it, or because
its source contradicts the port's native design.

| Source | Why not |
|---|---|
| `Registry.rho` | Its bootstrap handshake sends **one** item on `rho:registry:lookup` (`Registry.rho:393`), but this port's native definition is **arity 2** (`system_processes.rs:463-469`), so the handshake cannot match and the registry would stay silently empty. It would also install the interpreted `TreeHashMap` trie that the port replaced with native state (`spec/RUST-FIRST.md`). The aliases are seeded natively instead. |
| `AuthKey.rho` | Registered *by* `Registry.rho`; needed only by the interpreted vault sources. **No consumer in either checkout reads `rho:rchain:authKey`** — the wallet and rgov reach auth keys through `rho:rchain:revVault` `deployerAuthKey`/`unforgeableAuthKey` method calls. |
| `Either.rho` | Zero demand: no occurrence of `rho:lang:either`, `Left`/`Right` in either consumer. Its only in-repo consumer would be `RevVault.rho`, which is not installed (native). |
| `RevVault.rho`, `MultiSigRevVault.rho`, `Pos.rhox` | The vault and PoS **system** contracts are native here (`rholang/src/native_state.rs` + `system_deploy::NativeSystemDeployOp`), and installing the interpreted equivalents would shadow consensus-critical logic. Their sources additionally read `rho:lang:treeHashMap` and `rho:registry:systemContractManager`/`rho:rchain:configPublicKeyCheck`, none of which exist in this port. |
| `rho:lang:treeHashMap`, `rho:rchain:configPublicKeyCheck`, `rho:registry:systemContractManager` | Provided only by the interpreted `Registry.rho` (see above); no consumer uses them. |
| `rho:rchain:multiSigRevVault` | The native channel works by direct binding and via `lookup!` once aliased, but **no consumer in either checkout looks it up**, so it is not seeded (its API deviation is a separate, recorded issue in `spec/API-SCHEMA.md`). |

### Test-only and example sources (explicitly not candidates)

`legacy/rholang/examples/**` (76 files — the `tut-*` teaching programs, `old/**`, `linking/**`,
`vault_demo/**`), `legacy/rholang/src/main/k/**` (28 K-framework test inputs),
`legacy/casper/src/test/resources/**` (23), `legacy/rholang/src/test/resources/**` (12),
`legacy/integration-tests/resources/**` (14), `legacy/rspace-bench/...` (3),
`legacy/casper/src/main/resources/**` (10 — the superseded Scala genesis sources),
`examples/**` (2), `qucalc/rholang/**` + `qucalc/examples/**` (12), `rspace-bench/benches/resources/**`
(3). None is embedded by production Rust; each is consumed only by a test, a benchmark, or the legacy
Scala tree. The only production `include_str!` of rholang is the nine files in
`casper/src/genesis/resources/`.

## The one adapted source

`MakeMint.rho` cannot install verbatim: its epilogue asks `rho:registry:systemContractManager` for a
write-only dispatcher and defines a `securityCheck` arm calling `rho:rchain:configPublicKeyCheck`.
Neither channel exists in this port, so the `for` waiting on them never fires, nothing registers, and
`lookup!(\`rho:rchain:makeMint\`)` would answer `Nil` forever. `standard_deploys.rs::make_mint_source`
registers the contract's own bundle instead and drops the unused arm; both markers are asserted, so a
drift in the vendored source fails the build rather than shipping an unadapted epilogue. No consumer
calls `securityCheck`, and the consumer path — `lookup!` → `(nonce, bundle)` → call it — is unchanged.

## What this changes for a consumer

- **The shorthands resolve.** `rho:rchain:revVault`, `rho:rchain:pos`, `rho:rchain:makeMint`,
  `rho:lang:listOps` (and `rho:lang:nonNegativeNumber`) answer their lookup with
  `(9223372036854775807, bundle+{dispatcher})`, on a fresh chain, with no bootstrap deploy.
- **The `rho:id`s above are constant** for every chain of this port: hardcoding them is safe (and
  means a `down && up` no longer invalidates them for this set). Anything registered *by a deploy*
  still shifts per chain — `rho:registry:insertArbitrary` derives its URI from a random seed
  (`system_processes.rs:1284`), which is exactly why the rgov contracts need `insertSigned` to become
  genesis content (see below).
- **`down && up` still regenerates the genesis address**, so a *recorded* master URI from a deployed
  bootstrap goes stale. With the blessed set installed, this no longer applies to the node's own
  contracts.

## Not in scope here

The rgov **governance** contracts (`Kudos`, `Inbox`, `Directory`, `memberIdGovRev`, `Issue`, and the
master directory) live in `rchain-community/rgov`, not in this repository, and are not vendored by
this change. Making them genesis content is a separate sourcing decision with its own requirements —
fixed keys/timestamps, `insertSigned` instead of `insertArbitrary` so their URIs stop shifting per
chain, and the dependency markers substituted with those constants.

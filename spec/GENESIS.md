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

## The rgov governance class contracts (vendored)

Installed from `casper/src/genesis/resources/rgov/` — vendored from `rchain-community/rgov` (commit,
licence position and adaptations: `resources/rgov/NOTICE`). They are here because a governance client
otherwise has to deploy the set at runtime with `scripts/bootstrap-rgov.ts` and *record* the URIs the
deployment happens to produce: the master directory's member list is a product of the chain, so it
shifts on every fresh chain and goes stale silently.

| # | Contract (`rgov`) | `rho:id` (hardcodable) | Consumer it unblocks |
|---|---|---|---|
| 1 | `rholang/core/Kudos.rho` | `rho:id:c35xabt84irokn3kp7qh9gs31f58rzje8ntifucmg19s1q6gek8y` | the master directory's `kudos` member; `Kudos!("peek"/"award", …)` |
| 2 | `rholang/core/Inbox.rho` | `rho:id:8qbr8guigfush1n8y64ubkjnakwrh9m3suty68pkoebgbjwuiuio` | the member-directory handshake and the wallet's `newInbox`/`sendMail` snippets |
| 3 | `rholang/core/Directory.rho` | `rho:id:xp7ih4n3kghz89kkc3smgighxs1ou8wx6hrt54z9smr55tfd1i3o` | every per-deployer dictionary the member directory creates |
| 4 | `rholang/core/memberIdGovRev.rho` (the "roll") | `rho:id:j9wmz843xzfporr7qnghzsj46doxctpxg4msw9tjtrqex1smngjy` | `MemberDirectory!("make"/"makeFromURI", …)` — the master-directory path |
| 5 | `rholang/core/Issue.rho` | `rho:id:jne1e6mptyjp96zrak8reb4s8hmw3fonrkkkaxbiok18xxs1oj1o` | the ballot/issue snippets (`newIssue`, `castVote`, `tallyVotes`) |

What was adapted (each asserted at build time, so a vendored-file change fails the build):

- **Self-registration → `rho:registry:insertSigned:secp256k1`.** Upstream registers with
  `insertArbitrary`, whose URI is `build_uri(blake2b256(random seed))` — a *different* URI on every
  chain. The signed form derives it from the deployer key, and these deploys' keys are fixed, so the
  URIs above are constants. Only the **class** registration is converted; per-deployer registrations
  (a member directory's own write capability, an inbox instance's send capability) stay
  `insertArbitrary`, because they are runtime state by nature.
- **Fixed keys, derived not pasted**: `contract_key(name) = blake2b256("rnode/genesis/rgov/<name>")`,
  timestamp `1700000000000`. A pasted hex constant is unauditable; a named hash can be recomputed by
  anyone reading the source, and the URIs above are exactly `build_uri(blake2b256(pubkey))` of it.
- **Dependency markers substituted**: `memberIdGovRev.rho`'s
  `match ("import", "./directory.rho", \`rho:id:...\`)` / `("./inbox.rho", …)` markers become the
  installed URIs above — the same string substitution `bootstrap-rgov.ts` performs at runtime.
- **Deploy-time self-test traffic removed** (`Inbox.rho`'s trailing test program, `Directory.rho`'s
  post-registration exercise, and the demo prints around them): a chain's genesis must not send test
  messages or print demo output. The class definitions are untouched.
- The master-directory template
  (`resources/rgov/create-master-contract-directory-testnet.rho`) is vendored **verbatim** and
  rendered by `rgov::master_directory_template()`, which substitutes the seven recorded member URIs
  with the installed constants (`directory`, `echo`→`directory`, `inbox`, `issue`, `kudos`, `roll`,
  `log`→`directory` — `Echo.rho`/`mq.rho` never register, so their slots alias to `Directory`,
  exactly as upstream's bootstrap does). Creating a master directory stays a **runtime** deploy: it
  is keyed by the deployer's `deployerId`, so each client makes their own — that URI is inherently
  per-deployer, and the thing this change removes is having to *discover* the member URIs first.

## Install order

Genesis installs the set **in one order, and only one of the constraints is sharp**:

| Constraint | Why | If violated | Caught by |
|---|---|---|---|
| `non_negative_number` → `make_mint` | `MakeMint.rho:27` looks the counter up (`lookup!(\`rho:lang:nonNegativeNumber\`, …)`) **during its own deploy**, and waits on a reply pattern a `Nil` reply cannot match | the deploy still *succeeds*, `MakeMint` never registers, and `lookup!(\`rho:rchain:makeMint\`)` answers `Nil` forever — silently | the genesis ceremony's completeness check (`missing_genesis_aliases`), pinned by `installing_make_mint_before_its_dependency_is_caught_by_the_genesis_check` |
| `directory`, `inbox` → `roll` | **not a genesis constraint** — `memberIdGovRev` resolves those imports per *call*, not at deploy time, so its position in the list is free (all three are genesis content, so a caller always finds them) | nothing at genesis; a client's `"makeFromURI"` would need them installed, which they are by the time anyone can call | — (a negative test for it is what established this; see `BLESSED_DEPENDENCIES`) |
| `kudos`, `issue` vs anything | independent: each self-registers and reads nothing at deploy time | — | — |

The order itself is pinned twice: `genesis::tests::blessed_terms_are_ordered_by_dependency` asserts
the dependency table against the returned list *and* the exact sequence (a change there is a genesis
change), and every entry's *usability* is asserted by the call probes on a fresh chain.

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

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

## The rgov governance set — the **testnet** setup

Installed from `casper/src/genesis/resources/rgov/` (vendored from `rchain-community/rgov`; commit,
licence position and every adaptation: `resources/rgov/NOTICE`), so that a governance client on a
fresh chain needs **no bootstrap step and no recorded URI**: it hardcodes the constants below.

Two registrations live in these files and only one keeps upstream's shape:

- a **class** registers with `insertArbitrary!(bundle+{*X}, …)` — the stored value is the bare class,
  which is what every rgov consumer destructures (`for (Dir <- lookCh)`, `for (@(_, *X) <- ch)` for the
  node's own signed contracts). **Do not convert this to `insertSigned`**: its value is a
  `(nonce, value)` tuple, and the master-directory template then stalls *silently* — a deploy that
  reports `processedWithSuccess` and produces nothing. That was tried, and it is why the class keys
  below are chosen keys with a copy behind them rather than derived ones;
- each class therefore **publishes** the URI it was given (`["<name>", <uri>]` on the fixed channel
  `rnode:genesis:rgov-uri`), and genesis copies the entry onto a key *we* choose, which is what makes
  the key independent of the install order (the registered URI is `blake2b256` of the deploy's RNG
  state — deterministic on genesis, but it moves if any deploy is inserted before it).

### The constants a client hardcodes

| Contract (`rgov`) | `rho:id` | Consumer it unblocks |
|---|---|---|
| `Kudos.rho` | `rho:id:6hstcrmii97pxfnnwturmhmhfbtomdnh6q6fc6nrnsgfbuyjogyy` | the directory's `Kudos` slot; `Kudos!("peek"/"award", …)` |
| `Inbox.rho` | `rho:id:cbqr7s4o9yb6trpcitj7ne3qdyci8ph7u8yn8qp1hs1woe59iweo` | the `newInbox`/`sendMail`/`sendChat` snippets |
| `Directory.rho` | `rho:id:atsx1axaqqyjq841y8em3wgtrf8fe9iwm5ykwjyk3j66fxyskt7y` | every per-deployer dictionary |
| `memberIdGovRev.rho` (the "roll") | `rho:id:sff83mg96h5rt3emdncpnkrwgfobh9uuyhgw3tuctkeq17fnykqy` | `MemberDirectory!("make"/"makeFromURI", …)` |
| `Issue.rho` | `rho:id:q5eo8oexygm1yu3ha7g4t55gnau3onyex39198hm4d69n7ifhsto` | `newIssue`, `castVote`, `tallyVotes`, delegations |
| `Ballot.rho` | `rho:id:hpg1dns31bbdwb4yf9u6teabt7doutus6xfnu6uij6rweoszc8qy` | the ballot snippets |
| `Chat.rho` | `rho:id:yaer85qmkisrnr3h7yir389u687jhrzs4p1h67jtqasp4j5fw8sy` | `newChat`, `sendChat`, `readChat` |
| `Group.rho` | `rho:id:4ms51n1oramet9iu94df4483xp88jogfsfcnnsmen6xpraz7gs9o` | `newGroup`, `joinGroup`, `addMember` |
| **the master directory's read cap** | `rho:id:wxc4mwdh7otq4fd6iuxt84inepssyz5tugojf7ao68dkh4ebbncy` | the `MasterURI` every governance snippet takes — the wallet's `master-uri.ts` |

`ballot`, `chat` and `group` are **not** in upstream's deployment order: the master directory has
slots for them here because the wallet's editor asks the directory for those class *names*, and a slot
that was never filled answers `Nil` — which a client cannot tell from "broken".

### The key: the genesis ceremony's own

`masterDirectory`, `extraSlots` and `memberDirectory` are signed by **the key that creates the genesis
block** — `create_genesis_block`'s `ValidatorIdentity`. That is the standard genesis-ceremony
arrangement, and it is what the master directory's admin capability requires:

- **They must be one key.** The template publishes its
  `@[*deployerId, "MasterContractAdmin"]` capability for *its own* deployer, and the `GetMe` feature's
  registration is gated on reading that capability back. Signed by different keys the gate never
  opens, the feature registers nothing, the directory answers `Nil` for `GetMe`, and a client calling
  it gets silence — found on a node: the handshake reached "directory answered GetMe" and never
  entered `getMe`.
- **It must be a key whose private half is not public.** An earlier revision signed them with a key
  derived from a string literal in `rgov.rs`; anyone reading the source could compute it and exercise
  the capability on any network that installed it. The ceremony identity is threaded in for that
  reason, and `spec/TEST-COVERAGE.md` records the change.
- **Nothing a client hardcodes moves because of it.** The eight class keys are the classes' own fixed
  keys and the read cap is derived from the deploy *order* (the deploy's RNG state), not the signer;
  `the_published_keys_are_constants` asserts exactly that, and it is why the constants above are the
  same under either key.

`extraSlots` is our own term, not upstream's: rather than rewrite upstream's seven-slot template body,
it takes the write capability the template published and writes `Chat`, `Ballot` and `Group` in.

## Ceiling of this arrangement, and what a public network needs

### Open item (this is where the handshake currently stops)

On a fresh chain, with the constants above: the read cap resolves, the directory answers `GetMe`, and
`getMe` **runs** — it derives the deployer's REV address and logs four lines — then it enters the
feature's own `createMe` (inbox/dictionary creation) and stops before answering, so a client still
sees nothing at its reply channel. That is upstream `MemberDirectory.rho` flow logic rather than the
genesis installation, and the node's own contract logs are where it is legible
(`getMe!(*deployerId, *ret, *log)` with `log` wired to a *drain*, not to `rho:io:stdout` — the
feature logs multi-element lines and stdout takes one datum, so pointing the log at stdout makes the
contract error mid-flow). Diagnosing it is a separate piece of work; it is recorded here so nobody
mistakes the current state for "the family works".

## Testnet vs mainnet

Genesis installing steps 2–4 above is a **testnet** convenience with a real cost, and a public network
must not do it:

- **The ceremony key holds `@[*deployerId, "MasterContractAdmin"]`** and the chain's only `GetMe`
  feature: the capability belongs to whoever ran genesis. That is a real, secret key and an
  identifiable operator — but it is still *one* key over every client's first governance call. A
  network that would rather each client run its own directory must install none of steps 2–4; that is
  a genesis flag to land, not something this arrangement can express. What makes the shared model
  tolerable is verifiability: the class URIs are chain constants, so a client can check what the
  directory hands it against the table above instead of trusting the operator.
- **A directory slot that was never filled answers `Nil`**, and a consumer cannot distinguish that
  from "broken" — so on mainnet a client must handle an absent class explicitly rather than wait.
- **Class URIs derived from the deploy RNG move when the blessed order changes** (`BLESSED_DEPENDENCIES`
  and the pinned sequence in `casper/src/genesis/mod.rs` are the guard). The *chosen* keys above do
  not move, which is why a client hardcodes them and not the registered URIs.
- **The blessed deploys are free** (`phlo_price 0`, `phlo_limit MAX`) and unbounded in reduce steps
  except by the genesis path's own limits; a production genesis should charge or bound them.
- **Per-deployer state stays runtime**: an inbox, a dictionary, a master directory for a *new* key —
  none of that is genesis content, and the wallet's `newInbox` creates it for its own key.

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

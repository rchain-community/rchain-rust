# The node's API response schema (the standard)

**Why this file exists.** There was no published statement of *what a client receives*. Every client
guessed, and each guess was only falsified when someone ran it — which is how the node came to wrap
`rho:registry:lookup` replies in `(uri, value)` (C18) while every oracle-era client consumed the
value alone, and how the whole rgov governance contract family came to return `[]` with no
diagnostic. A shape is a published contract: changing one is a breaking change, so every row here
cites the oracle it follows and the test that enforces it. The consumer side
(`r-wallet/scripts/test-output-json.ts`) asserts the same shapes, so a regression from either
direction is caught.

**Scope.** What rholang code and HTTP clients receive. The HTTP DTOs live in the OpenAPI artifact
(`node/src/web/http.rs`, served at `/api/v1/openapi.json`); this file covers what it cannot: the
rholang-typed values, and the reply shapes of the system processes.

## Normative rules

1. **`RhoExpr` is externally tagged and carries the reference's `data` wrapper**:
   `{"ExprInt":{"data":42}}`, `{"ExprString":{"data":"…"}}`, `{"ExprBytes":{"data":"<hex>"}}`,
   `{"ExprUnforg":{"data":{"UnforgDeploy":{"data":"<hex>"}}}}`. Collections:
   `ExprList`/`ExprTuple`/`ExprSet`/`ExprPar` are arrays under `data`; `ExprMap` is a JSON **object**
   under `data`; an unforgeable nests one level deeper.

   **This rule was wrong until AUDIT C38, and the way it was wrong is worth keeping.** It said the
   wire had no `data` envelope, and that the envelope was "the Scala/OpenAPI *schema* artifact's
   spelling, not the wire's". The reference's own types say otherwise: every arm is a case class whose
   single field is named `data` — `final case class ExprInt(data: Long)`,
   `ExprMap(data: Map[String, RhoExpr])`, `UnforgPrivate(data: String)`
   (`legacy/node/src/main/scala/coop/rchain/node/api/WebApi.scala:131-151`) — and the node's JSON codec
   is *derived* from those case classes, so the field name is the serialization. The document a client
   generates from (`legacy/docs/rnode-api/rnode-openapi.json`, and the `rnode-openapi-schema.ts` beside
   it) is derived from the same case classes, which is why the two agree and there is no "schema
   spelling" to discount. The port emitted the unwrapped form, this file blessed it, and law 42 was
   then written from the port — so code and law agreed with each other and neither with a client. The
   corpus could not catch it: it compares the node to the model.

   Note the lossiness that follows: a set is indistinguishable from a list on the wire.
2. **Terminal results are a list.** A deploy or explore result is always wrapped one level
   (`[42]`, `[]` when the term sent nothing), because the result channel is a `Par`.
3. **Where a reply is read from, in order — and the response says which.** A deploy's `expr` is read
   from these channels, first non-empty wins, and the response's `replySource` names the one that
   answered (`"firstPrivateName"`, `"out"`, or `"none"`):
   - **`POST /api/v1/explore-deploy`** (and `-by-block-hash`):
     1. the term's **first `new`-bound name** — the reference node's own convention, stated in the
        Scala's `BlockApiImpl` as "be sure the first new should be `return`", and the only channel the
        port used to read;
     2. **`@"out"`** — the channel the corpus, the examples and the system-process conformance tests
        use, added by AUDIT C38 because reading only the first meant a term written this way returned
        `{"expr": []}`, indistinguishable from a term that produced nothing.
   - **`GET /api/v1/deploy-status/{sig}`**: the deploy's own `rho:rchain:deployId` channel
     (`rho:id:<sig>`), which is how the reference node reports a deploy's result.
4. **`[]` means the term sent nothing to the channels in rule 3** — not "failed" and not
   "unsupported". On the explore path `replySource: "none"` says it explicitly; a term that must be
   seen sends to one of the channels in rule 3. A reduce error is a 400 with the error text, and a
   deploy that fails is `processedWithError`.
4. **`POST /api/deploy` and `POST /api/propose` return a JSON-encoded string**
   (`"Success!\nDeployId is: <hex>"`), not a JSON object. `deploy-status` returns the
   `DeployExecStatus` enum instead: `{processedWithSuccess:{deployResult,block}}`,
   `{processedWithError:{deployError,block}}`, `{notProcessed:{status}}` — **camelCase on both the
   variant and its fields** (C16), and `notProcessed.status` is one of `"Pooled"`,
   `"Block not yet available"`, `"Unknown"`.
5. **`POST /api/explore-deploy` takes a raw JSON string body** (the term), not
   `ExploreDeployRequest`.
6. **`rho:rchain:deployId` / `deployerId` are bound by the normalizer env on the deploy path only.**
   They are absent under an exploratory deploy, so a term that reads them fails there with
   `No value set for \`rho:rchain:deployId\`` — a property of explore, not of the contract.

## System-process reply shapes

Oracle order of preference: the legacy Scala source, the genesis `.rho` sources it deploys, the
recorded expected outputs of the Scala-era examples/corpus. "Rust-first" means the process has no
Scala oracle (a documented extension) and this file is its definition.

| urn | arity | reply | oracle | status |
|---|---|---|---|---|
| `rho:registry:lookup` | 2 | **the stored value alone**; `Nil` when unknown | genesis `Registry.rho:397-401` + `legacy/rholang/examples/tut-registry.rho:8,42-47` | **fixed (C18)** — was `(uri, value)` |
| `rho:registry:insertArbitrary` | 2 | a `rho:id:` uri | `Registry.rho:409-426` | ✅ |
| `rho:registry:insertSigned:secp256k1` | 3 | a `rho:id:` uri, or `Nil` | `Registry.rho:433-469` | ✅ (stores `(nonce, data)` as the value) |
| `rho:registry:ops` | 3 | a uri | `SystemProcesses.scala:308-320` | ⚠️ shape ✅, URI encoding differs (z-base-32 vs CRC14) — deliberate, see AUDIT §337-343 |
| `rho:io:stdout` / `stderr` | 1 | none (prints) | `SystemProcesses.scala:206-222` | ✅ |
| `rho:io:stdoutAck` / `stderrAck` | 2 | `Nil` | `SystemProcesses.scala:211-230` | ✅ |
| `rho:crypto:{sha256,keccak256,blake2b256}Hash` | 2 | `ByteArray` | `SystemProcesses.scala:182-192` | ✅ |
| `rho:crypto:{secp256k1,ed25519}Verify` | 4 | `Bool` | `SystemProcesses.scala:159-180` | ✅ |
| `rho:rev:address` | 3 | `String`, else `Nil` | `SystemProcesses.scala:232-295` | ✅ |
| `rho:rchain:deployerId:ops` | 3 | `ByteArray` | `SystemProcesses.scala:297-306` | ✅ |
| `sys:authToken:ops` | 3 | `Bool` | `SystemProcesses.scala:322-332` | ✅ |
| `rho:block:data` | 1 | **`(blockNumber, sender, timestamp)`** — three, deliberately (pinned as a law-39 catalog row) | oracle sends **two**: `SystemProcesses.scala:355-361` produces `(blockNumber, sender)` from a `(blockNumber, sender, seqNum)` record | **documented extension**, not a defect: `docs/src/rholang/reference.md:93` specifies "number, sender and informational timestamp", and the node's own genesis vault consumes all three (`casper/src/genesis/resources/RevVault.rho:207-209` binds `@blockNumber, @sender, @timestamp`). The port substituted `timestamp` for the oracle's unexposed `seqNum`. **Consequence to know:** a *legacy* two-name consumer (`legacy/casper/src/test/resources/BlockDataContractTest.rho:15-16`) cannot match this and will stall — it must be amended, or the reply versioned, before that corpus is relied on |
| `rho:rchain:revVault` | 1 | `Int`, `Nil`, `(true, addr_string)` | `legacy/casper/src/main/resources/RevVault.rho:103-121,196-204` | ❌ **open** — the oracle's `findOrCreate` returns a *vault capability*, and `balance`/`transfer` are methods of that vault taking an `authKey` |
| `rho:rchain:multiSigRevVault` | — | shares the single-sig handler | `MultiSigRevVault.rho` is a different contract | ❌ **open** — the urn should not be advertised until it has its own contract |
| `rho:rchain:{revVault,pos,makeMint}`, `rho:lang:{listOps,nonNegativeNumber}` | — | `(9223372036854775807, bundle+{dispatcher})` — the signed-registration shape consumers destructure as `@(_, X)` | genesis content + aliases; `Registry.rho:371-379` is the oracle's shorthand aliasing | ✅ **resolved** — genesis now installs the interpreted library contracts and seeds the shorthand aliases natively (`spec/GENESIS.md`), so `lookup!(\`rho:rchain:revVault\`, *ch)` resolves *and* the value answers a call. Native channels keep working by direct binding too |
| `rho:rchain:authKey`, `rho:lang:either`, `rho:rchain:systemContractManager`, `rho:rchain:configPublicKeyCheck`, `rho:lang:treeHashMap` | — | **nothing — not seeded** | provided only by the interpreted `Registry.rho`, which this port does not install (its bootstrap handshake cannot match the native arity-2 `rho:registry:lookup`) | **deliberate, no consumer**: no occurrence in either the wallet or rgov consumer trees. Recorded with the reasoning in `spec/GENESIS.md` |
| `rho:rchain:pos` (native) | 1 | `Map`, `Set`, `(Bool, Nil\|String)` | `Pos.rhox:271-362` | ⚠️ shape ✅; `trust`/`untrust` have no oracle (extensions) |
| `rho:txn`, `rho:gov:*`, `rho:qucalc:*`, `rho:io:http` | — | as implemented | — | **Rust-first** — no Scala oracle. `docs/src/qucalc/architecture.md:100-107` already tabulates the qucalc/gov rows |

## Consumer-side consequences (what this standard bought us)

- A client consumes a lookup as `lookup!(uri, *ch) | for (X <- ch) { X!(…) }` — no unwrapping.
  Consumers of *system* contracts destructure `@(_, X)` because those are registered with a
  `(nonce, data)` **value**, which is a property of the stored value, not of the lookup.
- A client that receives `[]` has learned that the term produced nothing; it must not read that as
  success. `r-wallet/scripts/test-output-json.ts` treats `[]` as a failure unless the contract is
  explicitly allow-listed with a reason.
- Result rendering lossiness is a wire property (sets→arrays, unforgeables→bare hex), so a client
  should not round-trip its own output back as input without knowing that.

## Enforcement

- **Catalog (law 39)**: the rows a call can be *spelled* for are data in `spec/Rchain/Protocol.lean`
  (`replyCatalog`), emitted to `spec/conformance/protocol.tsv` and checked against a running node by
  `rholang/tests/lean_protocol_corpus.rs` — each row calls its urn with the arguments it spells and
  classifies every slot of the reply, arity included. This file is the tie on the other side:
  `tools/check-lean-conformance.sh` fails if a catalog urn has no row here. Read a row here as a
  *claim*, and that corpus as the machine-checked form of it; `spec/INVENTORY.md` row 39 names the
  urns the catalog cannot reach yet (the `ByteArray`-argument ones).
- **Node**: `rholang/tests/system_process_conformance.rs` asserts reply *shapes* — and, since C18,
  also **reachability** (`a_looked_up_contract_can_be_called_through_its_lookup_reply`). A shape
  assertion alone can pass while every real client fails, so reachability is part of the standard.
- **Consumer**: `r-wallet/scripts/test-output-json.ts` asserts the shapes it depends on from the
  other side (`check_process_api_schema`), naming the register entry when one drifts.
- **Register**: every change to this file that alters a row is a breaking change to a published
  contract and gets an entry in `spec/AUDIT.md`.

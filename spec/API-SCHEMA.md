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

1. **`RhoExpr` is externally tagged and unwrapped**: `{"ExprInt":42}`, `{"ExprString":"…"}`,
   `{"ExprBytes":"<hex>"}`, `{"ExprUnforg":{"UnforgDeploy":"<hex>"}}`. There is **no** `{data: …}`
   envelope — that is the Scala/OpenAPI *schema* artifact's spelling, not the wire's.
   Collections: `ExprList`/`ExprTuple`/`ExprSet`/`ExprPar` are all JSON arrays; `ExprMap` is a JSON
   object. Note the lossiness that follows: a set is indistinguishable from a list on the wire.
2. **Terminal results are a list.** A deploy or explore result is always wrapped one level
   (`[42]`, `[]` when the term sent nothing), because the result channel is a `Par`.
3. **`[]` means the term sent nothing to its result channel** — not "failed" and not
   "unsupported". Anything that must be seen has to be sent to `rho:rchain:deployId`.
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
| `rho:block:data` | 1 | **`(blockNumber, sender, timestamp)`** | `SystemProcesses.scala:355-361` sends `(blockNumber, sender)` | ❌ **open** — a 3-element send cannot match the oracle's two-name consumer pattern (`legacy/casper/src/test/resources/BlockDataContractTest.rho:15-16`); `seqNum` is not exposed |
| `rho:rchain:revVault` | 1 | `Int`, `Nil`, `(true, addr_string)` | `legacy/casper/src/main/resources/RevVault.rho:103-121,196-204` | ❌ **open** — the oracle's `findOrCreate` returns a *vault capability*, and `balance`/`transfer` are methods of that vault taking an `authKey` |
| `rho:rchain:multiSigRevVault` | — | shares the single-sig handler | `MultiSigRevVault.rho` is a different contract | ❌ **open** — the urn should not be advertised until it has its own contract |
| `rho:rchain:{pos,makeMint,authKey}`, `rho:lang:{listOps,either,nonNegativeNumber}`, `rho:rchain:systemContractManager`, `rho:rchain:configPublicKeyCheck` | — | **nothing — lookup answers `Nil`** | genesis contracts; `Registry.rho:371-379` is the shorthand aliasing | ❌ **open** — `casper/src/genesis/mod.rs:151` (`default_blessed_terms`) installs no rholang contracts, so the shorthands are unseeded. Direct binding (`new revVault(\`rho:rchain:revVault\`)`) works; the lookup path does not |
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

- **Node**: `rholang/tests/system_process_conformance.rs` asserts reply *shapes* — and, since C18,
  also **reachability** (`a_looked_up_contract_can_be_called_through_its_lookup_reply`). A shape
  assertion alone can pass while every real client fails, so reachability is part of the standard.
- **Consumer**: `r-wallet/scripts/test-output-json.ts` asserts the shapes it depends on from the
  other side (`check_process_api_schema`), naming the register entry when one drifts.
- **Register**: every change to this file that alters a row is a breaking change to a published
  contract and gets an entry in `spec/AUDIT.md`.

import Rchain.Par

/-!
# Law 43 — the envelope schema is the envelope, and it is checked against both parties

A response's *envelope* is its top-level shape: which keys a client finds, and — for a tagged union —
which tags. AUDIT C16 is what happens when that shape is pinned by nothing: `DeployExecStatus`'s fields
were snake_case in a camelCase response, so a client reading `deployResult` **got nothing and nothing
errored**.

The shape has two parties in the tree, and they can disagree silently:

- the DTOs (`node/src/api/dto.rs`, `models/src/casper/protocol/deploy_service.rs`), whose `serde`
  attributes *are* the serialization;
- the served OpenAPI document (`node/src/web/http.rs`'s `OPENAPI_JSON`), a hand-written string that
  `GET /api/v1/openapi` hands to clients — nothing checked it against the DTOs, and it was **stale**
  when this law was written (AUDIT C29: `ApiStatus` declared 8 properties where the endpoint returns
  13; `LightBlockInfo` 15 where the type has 16).

This module is the catalog both parties are checked against, and `Corpus.envelopeCases` +
`node/tests/lean_envelope_corpus.rs` are the checks: each row's keys are compared to the DTO's
serialization *and* to the served schema, so a shape that drifts on either side fails a test that names
the key. The `decide`d checks on the table itself are the C16 rule — **no key contains an underscore**,
no key repeats within a row, every tag is capitalized (a client switches on the tag), and the two row
shapes (a struct's keys, a union's tags) are exclusive.

**Boundary, stated:** the law pins the envelope's *keys and tags*, not the *types* of their values
(`errored` being a boolean, `cost` an integer). A schema whose types drift is still unchecked, and
`spec/API-SCHEMA.md` remains the register for the reply *values* (law 39's catalog covers those).
-/

namespace Rchain

/-- A response envelope: the type's name (as the DTO and the served schema name it), the endpoint the
reader should think of, and either a struct's keys or a tagged union's `tag → keys`. -/
structure EnvelopeRow where
  name : String
  endpoint : String
  keys : List String
  variants : List (String × List String) := []

/-- Does any key contain an underscore? C16's defect in one predicate: a camelCase envelope with a
snake_case key renders the field a client asked for as absent. -/
def hasUnderscore (k : String) : Bool := k.toList.any (fun c => c == '_')

/-- A tag is capitalized (a client switches on `ProcessedWithSuccess`, not `processedWithSuccess`). -/
def tagCapitalized (t : String) : Bool :=
  match t.toList with
  | [] => false
  | c :: _ => c.isUpper

/-- The row's keys are non-empty, snake-free, and distinct. -/
def keysWellFormed (ks : List String) : Bool :=
  !ks.isEmpty && ks.all (fun k => !hasUnderscore k && !k.isEmpty)
    && ks.eraseDups.length == ks.length

/-- The row's shape: a struct's keys, or a tagged union's variants — not both, not neither. -/
def shapeWellFormed (r : EnvelopeRow) : Bool :=
  if r.variants.isEmpty then keysWellFormed r.keys
  else r.keys.isEmpty && r.variants.all (fun v => tagCapitalized v.1 && keysWellFormed v.2)

/-- The catalog. Each row is checked twice by the corpus — against the DTO's own serialization and
against the served OpenAPI schema — so a row is not an opinion about a shape but the shape both parties
are held to. `endpoint` is for the reader; the check is on the type, which both parties name. -/
def envelopeCatalog : List EnvelopeRow :=
  [ { name := "ApiStatus", endpoint := "GET /api/v1/status",
      keys := ["version", "address", "networkId", "shardId", "peers", "nodes", "minPhloPrice",
        "latestBlockNumber", "autopropose", "proposeOnDeploy", "manualPropose", "adminHttp", "devMode"] }
  , { name := "NodeCapabilities", endpoint := "GET /api/v1/capabilities",
      keys := ["autopropose", "proposeOnDeploy", "manualPropose", "adminHttp", "devMode", "faucet"] }
  , { name := "LightBlockInfo", endpoint := "the `block` field of most responses",
      keys := ["version", "shardId", "blockHash", "blockNumber", "sender", "seqNum", "preStateHash",
        "postStateHash", "justifications", "bonds", "sigAlgorithm", "sig", "blockSize", "deployCount",
        "rejectedDeploys", "timestamp"] }
  , { name := "DeployInfo", endpoint := "the `deploys` entries of a block response",
      keys := ["deployer", "term", "timestamp", "sig", "sigAlgorithm", "phloPrice", "phloLimit",
        "validAfterBlockNumber", "cost", "errored", "systemDeployError"] }
  , { name := "RhoDataResponse", endpoint := "POST /api/v1/data-at-name",
      keys := ["expr", "block"] }
  , { name := "DeployExecStatus", endpoint := "the `deployResult` body of a deploy response",
      keys := [],
      variants :=
        [ ("ProcessedWithSuccess", ["deployResult", "block"])
        , ("ProcessedWithError", ["deployError", "block"])
        , ("NotProcessed", ["status"]) ] }
  ]

/-- The catalog is well-formed, `decide`d: names unique, endpoints named, and every row's keys or
tags conforming (non-empty, no underscore — C16's rule — distinct, and tags capitalized). -/
theorem envelopeCatalog_decide :
    (envelopeCatalog.all (fun r =>
        !r.name.isEmpty && !r.endpoint.isEmpty && shapeWellFormed r)
      && (envelopeCatalog.map EnvelopeRow.name).eraseDups.length
        == envelopeCatalog.length) = true := by
  decide

/-- The count the Rust consumer asserts it read. -/
def envelopeCaseCount : Nat := 6

/-- The catalog carries exactly `envelopeCaseCount` rows. -/
theorem envelopeCatalog_length : envelopeCatalog.length = envelopeCaseCount := by decide

end Rchain

# Navigation for AI agents

This page is a goal-indexed map of the documentation and the machine-checked specification. Read it
first, then jump to the single page that answers your question. It mirrors the documentation map in
[`AGENTS.md`](../../AGENTS.md) but is organized by *goal* rather than by artifact.

The **authoritative formal specification** is the [`spec/`](../../spec/) tree — the law catalog
([`spec/INVENTORY.md`](../../spec/INVENTORY.md), **49 rows**: the 29 calculus laws, 14 covering the
surface a client writes, 4 for the Proof-of-Stake epoch, one for the fee consequence of a denied
deploy, and one for what a matched deploy is charged), the ρ-calculus core
([`spec/RHO-CALCULUS.md`](../../spec/RHO-CALCULUS.md)), and the ρ→CoC type discipline
([`spec/TYPE-SYSTEM.md`](../../spec/TYPE-SYSTEM.md)). This book explains those; it never duplicates them.

## By goal

| I want to… | Read |
|---|---|
| Understand *why* rholang exists and what makes it powerful | [Why rholang](rholang/why-rholang.md) |
| Learn the language from scratch | [Processes and names](rholang/processes-names.md) → [Sends and receives](rholang/sends-receives.md) |
| Understand `@` and `*` (quote/eval) | [Names are quoted processes](rholang/names-are-processes.md) |
| Understand what an **unforgeable name** is and why `new` matters | [Unforgeable names](rholang/unforgeable-names.md) |
| Understand pattern matching / spatial matching | [Patterns and matching](rholang/patterns-matching.md) |
| Build a secure contract (facets, revocation, sealer/unsealer, multisig) | [Object capabilities](rholang/object-capabilities.md), [Smart contracts](rholang/smart-contracts.md) |
| **Port an existing app** (or debug a parser that worked elsewhere) | [Porting an app from rnode](developer/porting-a-client.md) — reply shapes, refused terms, the traps |
| See the exact grammar and sorts | [Grammar and sorts](formal/grammar-sorts.md) |
| Map a language feature to its **law** and its proof | [The 29 laws](formal/the-29-laws.md) |
| Map a **syntax, matching, reply-shape or JSON** feature to its law | [Laws 30–43: the surface](formal/laws-30-43.md) |
| Understand how concurrent effects are linearized (claim queues, the DFS gate, the relaxed mode) | [The channel scheduler](formal/channel-scheduler.md) |
| Understand how effect concurrency becomes sound on-chain (validated speculation, the Law 24 certificate) | [On-chain scheduling: validated speculation](formal/onchain-scheduling.md) |
| Understand `≡` and `⟶` precisely | [Structural congruence and reduction](formal/congruence-reduction.md) |
| Understand the "no silent partiality" / totality guarantee | [Closedness and the Calculus of Constructions](formal/closedness-coc.md) |
| Understand consensus / finality | [Consensus (Casper)](node/consensus.md) |
| Understand the tuple space / storage | [The tuple space (RSpace)](node/rspace.md), [Storage](node/storage.md) |
| Run a validator (hardware requirements / sizing) | [Running a validator: hardware requirements](node/validator-requirements.md) |
| Build an app against a running node (deploy rholang, read responses) | [Building applications on the local devnet](developer/building-apps.md) |
| Understand the port (why Rust, module status) | [Part V](contributor/why-rust.md) |
| Find the machine-checked proofs | [`spec/Rchain/`](../../spec/Rchain/) (Lean), [`spec/coq/`](../../spec/coq/) (Coq) |

## The invariant catalog, in one screen

RChain's behavior is pinned by **49 laws** ([`spec/INVENTORY.md`](../../spec/INVENTORY.md)): the 29
below, about the calculus, rows 30–43 about the surface a client writes and a matcher reads
([Laws 30–43](formal/laws-30-43.md)), rows 44–47 about the native Proof-of-Stake epoch
([Laws 44–47](formal/laws-44-47.md)), row 48 about the fee consequence of a denied deploy (a rule
neither tree implements, recorded as an open design question), and row 49 about what a matched deploy
is charged — the first law about gas rather than state. The first 29 group as:

- **Rholang (Laws 1–6)** — canonicalization, α-equivalence, substitution, reduction, spatial matching,
  closedness.
- **RSpace (Laws 7–11)** — join commutativity, deterministic COMM, merge monoid, Merkle determinism,
  replay determinism.
- **Rosette (Laws 12–13)** — actor atomicity, reflection (orphaned; the VM is out of scope).
- **Casper (Laws 14–17)** — >2/3 finality, fringe/seen-set monotonicity, block validation, merge
  determinism.
- **Storage (Law 18)** — contiguous height map, order-independent fringe identity.
- **Crypto (Law 19)** — Blake2b256, splittable `Blake2b512Random`, signatures, Curve25519 (axiomatized).
- **Scheduler (Laws 20–22)** — per-channel claim queues in DFS path order, the gate scheduler, the
  dispatch-time next-step closure (see [The channel scheduler](formal/channel-scheduler.md)).
- **Scheduler, on-chain (Laws 23–25)** — read-determinism, DFS-order serializability, validated
  speculation (see [On-chain scheduling](formal/onchain-scheduling.md)).

And the 14 surface rows, which exist because every defect that started the formalisation programme
lived there and **nothing errored**:

- **Grammar and lexing (30–33)** — every term the parser accepts is in the BNFC grammar, every grammar
  term is accepted modulo a data list of deviations, each spelling lexes one way, `parse (print p) ≡ p`.
- **Normalization (34, 36)** — a value position is normalized against an *empty* par; the normalizer's
  output is well-scoped and closed.
- **Matching (35, 37, 38)** — concreteness is sound (connective, free var, wildcard **or remainder**);
  the matcher is sound and complete; silence is specified (no step, no error).
- **Replies and protocols (39–41)** — reply shapes, call arity, channel balance.
- **JSON (42, 43)** — the rho-value round-trip and the envelope rule; each endpoint's shape equals the
  schema's.

## The formalization, in one screen

| Artifact | What it proves/states | Build |
|---|---|---|
| `spec/Rchain/*.lean` (Lean 4) | Law 1, `≡`/`⟶` core, `Closed`, totality fundamentals **proven**; law 5's matcher is **defined** with `spatialMatch_implies_linear` proven and its two soundness/saturation axioms owed; Laws 3, 4, 7–18 **stated**; Laws 20–22 **proven** in `Scheduler.lean` (path order, the bakery core, and the await chain; the *global* liveness statement needs a finiteness hypothesis and is not a theorem as the old axiom stated it); Laws 23–25 in `SchedulerOnchain.lean` — Law 23 proven, Law 24 fully proven (`pinned_run_publication`, with no certificate hypothesis), Law 25 **proven** (`published_state_is_the_oracles`); crypto **axiomatized**. The register is `spec/laws.tsv` / `spec/LAWS.md`, emitted from `Rchain/Laws.lean` | `cd spec && lake build` |
| `spec/coq/*.v` (Coq) | Laws 2–6 (substitution / α-equivalence metatheory) **stated** | `make -C spec/coq` |
| `spec/conformance/*.tsv` | the conformance corpora: *emitted* from the Lean definitions, committed, and read by a Rust consumer that runs the same cases through the node | `tools/emit-lean-corpus.sh` |
| `spec/INVENTORY.md` | the 49-row law catalog with source-of-truth + status | — |
| `spec/TYPE-SYSTEM.md` | the ρ→CoC type discipline (totality, refinements) | — |

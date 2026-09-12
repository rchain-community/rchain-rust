# Navigation for AI agents

This page is a goal-indexed map of the documentation and the machine-checked specification. Read it
first, then jump to the single page that answers your question. It mirrors the documentation map in
[`AGENTS.md`](../../AGENTS.md) but is organized by *goal* rather than by artifact.

The **authoritative formal specification** is the [`spec/`](../../spec/) tree — the 25-law catalog
([`spec/INVENTORY.md`](../../spec/INVENTORY.md)), the ρ-calculus core
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
| See the exact grammar and sorts | [Grammar and sorts](formal/grammar-sorts.md) |
| Map a language feature to its **law** and its proof | [The 25 laws](formal/the-25-laws.md) |
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

RChain's behavior is pinned by **25 laws** (see [The 25 laws](formal/the-25-laws.md) and
[`spec/INVENTORY.md`](../../spec/INVENTORY.md)). They group as:

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

## The formalization, in one screen

| Artifact | What it proves/states | Build |
|---|---|---|
| `spec/Rchain/*.lean` (Lean 4) | Law 1, `≡`/`⟶` core, `Closed`, totality fundamentals **proven**; Laws 3–5, 7–18 **stated**; Laws 20–22 **proven** in `Scheduler.lean` (except `law20_deadlock_freedom`, **stated**); Laws 23–25 in `SchedulerOnchain.lean` — Law 23 proven, Law 24 witnesses + supporting lemmas proven (publication **stated**), Law 25 **stated** + `gate_replay_terminates` proven; crypto **axiomatized** | `cd spec && lake build` |
| `spec/coq/*.v` (Coq) | Laws 2–6 (substitution / α-equivalence metatheory) **stated** | `make -C spec/coq` |
| `spec/INVENTORY.md` | the 25-law catalog with source-of-truth + status | — |
| `spec/TYPE-SYSTEM.md` | the ρ→CoC type discipline (totality, refinements) | — |

# Security audit (September 2026)

This chapter is a point-in-time report. It records what an adversarial review of the node found at
commit `67dd6fb7b`, what it could not settle, and how the node's safety properties compare with three
other production chains read from their own source.

It is written to be useful when it is unflattering. The node's one confirmed high-severity weakness is
in this chapter, together with the structural properties that make whole classes of defect
unrepresentable. Both halves matter: a review that lists only residual defects misrepresents a system
whose thesis is carrying invariants in the semantics rather than in review.

**Scope, and how to read it.** The review itself was report-only: nothing was changed *by the pass*,
and no finding was fixed while it ran. What you are reading is therefore a photograph of `67dd6fb7b`:
the findings, their probes and the numbers those probes produced — the amplification table in §3, the
probe results in §7 — are true of that commit and not of whatever `dev` says today, and they are left
as measured rather than edited. **Where a *status* has moved since, it is stated as a status and given
its current figure** (the register's totals, the witness count, which questions have closed), because a
reader checking this chapter against today's tree should not find it wrong — only dated.

**Every confirmed finding was closed out — the P1s and P2s the same day, and the lower-severity tail
over the following days.** The fixes are in their own commits rather than edited into this text, so
that the photograph stays a photograph; what follows is the outcome, not a rewrite, and where a status
has moved since, it is stated as a status rather than edited into the measurement.

The three P1s were one root cause — the gas model did not bound work — and were closed by a per-block
phlo cap, by making the five operations that charged less than their work charge for it, and by
charging the parse. The P2s closed by a DAG write order that now writes data before the pointer it is
found by, by saturating arithmetic in the `qucalc` governance folds, and by a block `version`
predicate that had existed since the port with **no caller**.

**Signing and an SBOM on the release path closed as a decision, and the other item did not close
because it cannot.** The release path was resolved on 2026-09-28: a `sha256sum` is published beside the
binary and nothing further, because a signature needs a trust root and a key policy that a code change
should not choose silently — the decision and its reasoning are recorded on the step in
`.github/workflows/build-rnode.yml`, which is where a reader should look for it. And *cancellation
inside a running builtin* is **not achievable on this design**: a builtin runs its CPU synchronously inside
`Box::pin(async { … })` with no `await` before it, so a deadline can bound how long the *caller* waits
and never the work itself. That finding is what determined the shape of the whole remediation — bound
the work, because interrupting it is not available — and it is worth stating here because a reader who
expects a cancellation token will go looking for one that was deliberately not written.

**Two corrections belong in this chapter rather than in a commit.** The remediation advice in §3 named
a `HashSet` where `Par` derives no `Hash`, so it did not compile; it is marked there. And one of the
five planned charge fixes was **reverted because implementing it showed the finding was wrong**: the
crypto builtins' inputs have to pass through the storage path, which already charges proportionally,
so their flat charge bought no unbounded work. A finding that does not survive being implemented is a
finding that was wrong, and it is recorded as one rather than quietly dropped.

The places where the close-out *went beyond* the reference implementation are registered in
[`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) §3 — the per-block cost bounds, the DAG write
order, the block `version` check, the governance bound, the ordered dedup and the three charges that
followed, and (item 12, the largest of them) the **randomised active-validator draw**, which replaces a
selection rule the reference's own source marks as a placeholder.

## 1. Method

The approach is the one this project's own audit history found to work, with its refutation stage
rebuilt.

**Three stages.** Independent lenses read the tree adversarially, each required to produce either a
reproduction or nothing. Every candidate finding was then handed to a *second* agent whose instruction
was to refute it. Survivors were graded and given a falsifier.

**Why the refutation stage was rebuilt.** The earlier rule was "default to REFUTED whenever the
mechanism could not be re-established from source". That rule cannot distinguish *the mechanism is
absent* from *the mechanism is real but not re-establishable by reading* — and the second category is
exactly the high-severity dynamic class (races, crafted-depth exhaustion, load-driven denial of
service, crash consistency). A default-REFUTED filter therefore suppresses the best material.

This pass used three verdicts instead — **`CONFIRMED` / `NOT-RE-ESTABLISHED` / `REFUTED`** — where only
`REFUTED` removes a finding, and a `REFUTED` verdict must carry a positive artifact: a named line, a
test, or a measurement showing the mechanism absent. An empty-handed refutation is a self-reported
clean, and this project's audit record already holds that "an independent read is evidence and a
self-reported clean is not".

**The discipline earned its keep.** Ten candidate findings were refuted, including the one the pass
opened expecting to lead with:

| Candidate | Verdict | The artifact that killed it |
|---|---|---|
| Deeply-nested protobuf reaches an unbounded recursion in `wire.rs` and exhausts the stack from the unauthenticated deploy port | REFUTED | `prost`'s `RECURSION_LIMIT = 100` refuses the message at decode, before any handler runs; the server stack is 32 MiB (`node/src/main.rs`) |
| The `qucalc` `i64` overflow panics the interpreter and is a remote denial of service | REFUTED | Continuation dispatch is spawned and `join_spawned` maps the `JoinError`; the deploy fails, the node survives (measured) |
| The Docker image builds an unpinned dependency set with no CI signal | REFUTED | `ci.yml` runs `cargo clippy --locked --workspace --all-targets --all-features` on every push |
| The T1 coverage headline is contradicted by the coverage register | REFUTED | The cited rows are a date-stamped batch snapshot the same document supersedes |
| Vendored genesis `.rho` integrity is checked only at audit time | half REFUTED | The check runs on every push, and an edited contract changes the genesis post-state hash |

Two of the pass's own severity assessments were also corrected by measurement rather than argument: the
`qucalc` `trust_levels` function is **linear**, not quadratic (its relaxation is monotone over levels in
`{0,1,2,3,4,5}`, so it converges in at most six iterations whatever the input size), and the LMDB
crash window is bounded to two fsyncs and can self-heal on a multi-validator network.

**Limits, stated plainly.** The pass read code and ran targeted tests. It did not run a live multi-node
attack, did not re-measure coverage, and did not re-read all 89 T1 modules line by line. Findings below
marked *measured* carry a command and a number; those marked *read* are code-path arguments.

## 2. Results

| | | Closed out |
|---|---|---|
| Confirmed P0 (chain split / fund loss / RCE) | **none** | — |
| Confirmed P1 (remote unauthenticated denial of service) | 3, sharing one root cause | **all three, same day** |
| Confirmed P2 | 4 | **all four, same day** |
| Confirmed P3 | 14 | **all fourteen** — the signing/SBOM one as a decision (a checksum, nothing further), the rest fixed |
| Not re-established (refutation incomplete) | 1 | **closed since** — the cost asymmetry was measured at 14.2 s for a 4097-member fold and bounded to 512 (§3) |
| Refuted during the pass | 10 | — |

The absence of a P0 is the headline result, and it is a result about the project's prior work: on a tree
where earlier passes had confirmed remote denial-of-service defects, this pass could not find a chain
split, a fund loss, or remote code execution.

**The closure column is the part a later reader needs most**, and both of its exceptions have since
been settled. The *not-re-established* row was the `qucalc::gov::censure` cost asymmetry: the pass's
refutation never completed, so what an attacker actually pays at the ingress was unanswered, and the
close-out recorded that rather than guessing. **It was measured afterwards** — 14.2 seconds for a
4097-member fold, i.e. the charge was not merely asymmetric but absent — and the bound that replaced it
is 512, set from that measurement. The register now reads **204 of 204 rows `done`**, with no
not-re-established row left. The P3 row's exception was signing and an SBOM, a decision rather than
work, and the decision has been taken: an `sha256sum` beside the binary and nothing further, recorded
on the release step with the reason (a signature needs a trust root that a code change should not pick
on the project's behalf).

## 3. The gas model does not bound work

This is the pass's principal finding and the only one with a high severity.

**The cost model charges a flat rate for work that is superlinear in attacker input.** Deducing a
`Set` is implemented with a linear scan in `models/src/sorter.rs` (`par_set`), so `Set.union` costs
Θ(N²) — measured with an empirical exponent of **2.43**, i.e. super-quadratic, because the accumulating
result vector is itself rescanned. The cost table charges a **flat 13 phlo** for the operation
regardless of N.

Measured in release, on a term parsed from source:

| N | source | phlo charged | union | total |
|---|---|---|---|---|
| 5 000 | 23.9 KB | 13 | 0.42 s | 0.45 s |
| 10 000 | 48.9 KB | 13 | 1.65 s | 1.7 s |
| 20 000 | 108.9 KB | 13 | 7.97 s | 8.0 s |
| 40 000 | 228.9 KB | 13 | 64.1 s | **98.0 s** |

That is roughly 7.5×10⁹ nanoseconds of node CPU per phlo, against a cost table whose own fair rate is
about one phlo per byte of real work.

**This is a divergence from the Scala reference, not inherited behaviour.** The Scala deduplicates
through a `HashSet` (`SortedParHashSet.apply`), expected O(N). The port replaced a hash set with a
linear scan. `par_map` has the same defect.

Three aggravating factors:

1. **Execution-time parsing is uncharged.** `Costs::parsing_cost` is defined and called nowhere in the
   tree, and `phlo_limit = 0` is accepted at ingress, so a deploy can force the full parse and
   normalise before the first charge lands. The Scala charges `parsingCost` before `sourceToADT`.
2. **Nothing bounds a block.** The validator-side check list has no total-phlo cap and no deploy-count
   cap. `MAX_BLOCK_DEPLOYS` is used only by the local proposer; nothing on the receiving side enforces
   it, and there is no bonded-sender check on the block path. The only transmission bound is the 256 MiB
   streamed block size.
3. **A running call cannot be interrupted.** The reduce-step budget and the cancellation flag are
   checked at continuation boundaries only, so a single builtin call of arbitrarily long duration runs
   to completion. There is no wall-clock timeout on the block execution path.

**Why no gate caught this.** None of the project's gates is about cost. The law register covers semantics —
canonicalisation, substitution, COMM, matching, merge, finality arithmetic. A cost regression is not a
panic, not an `unsafe`, not a type escape, not a coverage drop, and not a law violation, so it is
invisible to every check the project runs. That is a gap in the shape of the register, not in the
strictness of any particular gate.

**Prioritised remediation.** Add a per-block phlo cap and a validator-side deploy-count cap; make the
set/map dedup ordered; wire `parsing_cost` at execution time before the cost is sampled; and check the
cancellation flag inside long builtins. The `par_set` change is small and removes the amplification's
root.

*Corrected during the close-out, 2026-09-28.* This paragraph first said "replace the linear scan with a
`HashSet`", and **that does not compile**: `Par` derives no `Hash` anywhere in the AST and no manual
impl exists, so `HashSet<Par>` is a type error. The fix that landed is **sort-then-dedup** — `par_set`
sorts by `Par`'s derived total order, which puts equal elements adjacent, then `Vec::dedup`s them, and
because the sort is stable it keeps the first of each run, which is the element the scan kept;
`par_map` sorts by key and makes one keep-last pass, reproducing last-write-wins. Byte-for-byte the
same output at Θ(N log N). The advice was wrong in the ordinary way — a remediation sentence is a claim
like any other, and this one was never compiled before it was published.

*And the remediation has since been carried out* (2026-09-28), in the order given. The ordered dedup,
the two validator-side caps and the execution-time parse charge landed first — and then the per-block
cap turned out to be **inoperative on its own**, because a cost model bounds work only when a phlo buys
bounded work. A second pass made the five operations whose charge did not track their work charge for
it. The worst was `rho::gov:censure`, cubic in its arguments and charged **nothing at all**; the cost of
that being free was measured at **14.2 seconds** for a 4097-member fold, and the bound that replaced it
is 512 — set from that measurement, not chosen.

**What that sweep is worth, stated rather than implied.** The five were found by *reading the cost
table for charges that do not track their work* — an inspection, not a proof that no sixth exists. The
claim is "these five", not "the cost model is now sound", and a sixth charge of the same shape would
need the same reading to find. The per-block cap bounds the total a block can spend; it does not, on
its own, make any individual charge proportional, which is why both halves were needed and why the cap
alone was found to be inoperative.

**The cancellation item is not "left" — it is not achievable on this design**, which the close-out
established by reading rather than assumed. A builtin runs its CPU synchronously inside
`Box::pin(async { … })` with no `await` before it, so it blocks a worker thread to completion; `tokio`'s
spawn handle *detaches* rather than aborts when dropped, and the code's own comment says so; and the
cancellation flag is read only at dispatch boundaries. A deadline can therefore bound how long the
*caller* waits and never the work itself — which is why this remediation took the shape it did. Bound
the work, because interrupting it is not available, and a reader who goes looking for the cancellation
token will not find one.

Details and the divergences they carry: `spec/RUST-VS-SCALA.md` §3.

## 4. Other confirmed findings

**The DAG can wedge silently after a crash (P2).** `casper/src/dag.rs::insert` writes the block
metadata — which marks the block known, so a re-offer is a no-op — *before* the fringe record. A crash
between the two leaves a store where the block is present and its fringe record is not;
`get_pre_state_for_parents` then refuses every block for which that one is the max-fringe parent, and
the missing record is skipped **silently** on restore, while the three sibling arms all fail closed.
The record is keyed by the fringe rather than the block, so on a multi-validator network a later block
declaring the same fringe repairs it. **On a single-validator or standalone node it does not: the node
is wedged permanently and the recovery is to wipe the data directory and resync.** The write order is a
faithful port of the Scala; the fix — write the fringe record before the metadata — is the same
data-then-pointer order the RSpace history layer already follows, and it turns the window into a
harmless orphan record. **That half has landed** (`casper/src/dag.rs`, AUDIT F-5). Making the silent
restore arm loud is a complementary one-line change and **is still owed** — the fold that skips a
missing entry in silence is unchanged.

**Governance arithmetic is unchecked in release (P2) — closed.** `qucalc`'s `rho:gov:*` handlers
performed plain `i64` arithmetic on values taken straight from deploy arguments with no range check. In
debug builds this panicked and the deploy failed; in release it wrapped, and the wrapped results were
silently wrong — `resolveWeights` returned a clamped zero where a maximum was asked for, a delegation
could produce a *negative* weight, `censure` could promote a voucher from `i64::MIN` to `i64::MAX`, and a
ranked tally could elect the landslide loser or report no winner for a unanimous vote. These are pure
functions and every node wrapped identically, so it was a correctness defect for anything reading the
governance channels as an oracle rather than a consensus split — which is why it was fixed rather than
registered: the folds now **saturate** (`0c1ee2f2c`), and the `censure` fold's charge, which was absent
altogether, was added with a bound measured at 14.2 s (§3).

**Block `timestamp` is hashed and consumed but never validated (P2 for any contract that reads it).**
The acceptance predicates check every other field that the content hash covers, but not `timestamp`.
`timestamp` is not merely informational: it is exposed to contracts on `rho:block:data`, so a bonded
proposer can choose a value that every honest validator replays, and a contract that reads it changes
its output and therefore the post-state hash. **The `version` half of this finding is closed** — its
predicate had existed since the port with no caller, and it is now one of `block_summary`'s pure checks,
answering `InvalidVersion`. **The `timestamp` half is the port's own and remains open**: no predicate
was written for it, and the question of what a validator could check (a bound? a window? nothing, since
the proposer's clock is unverifiable) has no obvious answer. The generalisable question — *which fields
are in the hash but absent from the acceptance predicate?* — now has a three-item answer:
`timestamp`, `rejected_blocks`, `rejected_senders`.

**DAG memory grows quadratically in block count (P2, registered — and accepted).** Each message retains
its whole ancestry, so total residency is N(N+1)/2 where N is every block ever accepted, and the
message map is never pruned. This is a known, measured residual — the node's own metric reads
17 319 555 entries at 5 885 blocks, exactly N(N+1)/2. What this pass adds is the reachability analysis:
the input rate is attacker-controllable, and nothing a bonded validator must respect bounds it, which
puts 16 GB of resident memory roughly twelve hours away at a two-second block interval.

**The decision taken on it is to accept and record the rate, not to bound it** (2026-09-28). Bounding
it means designing pruning of finalized ancestry into the DAG and its liveness rules — a design change,
not a patch, and one that touches the rules the finalizer depends on. The rate above is the measurement
that makes accepting it a decision rather than an omission: the growth is a known function of a known
input, so an operator can size for it and a later pass can revisit with a number rather than a hunch.

**Fourteen lower-severity items** span: two production `await-holding-lock` sites that contradict the
audit record's claim that only test modules remain; a published API-schema rule that states the wrong
tag casing for the deploy-status envelope (the wire emits capitalized tags, and the machine-checked
envelope law agrees with the wire); a served `openapi.json` that declares the wrong request body for
`/explore-deploy`; a `cargo-deny` invocation that runs only the advisory check, leaving the authored
licence allow-list as dead letter; no checksum, signature, SBOM or reproducible-build step in any
workflow; a `next-audit-number` helper that does not see an allocated number and would re-issue it;
counted-but-unenforced classes in the type-system gate; a coverage-ledger check that no workflow
invokes; and hand-repeated counts that have drifted (the README's line count and the two documents'
disagreement over how many crates the workspace has).

**All but one of those have since closed**, re-checked against the tree rather than remembered:
`await-holding-lock` is now reported **only inside `#[cfg(test)]` modules** (the gate still allow-lists
the lint, so this is a property of the sites, not of the lint); the deploy-status casing is stated
correctly in `operating.md` and `building-apps.md` and held by law 43's envelope corpus; the served
`openapi.json` declares a string body for `/api/explore-deploy` and says so in `spec/API-SCHEMA.md`;
`cargo-deny` runs `check` rather than `check advisories`, so the licence allow-list is live; the release
workflow publishes a checksum; `next-audit-number` reads the emitted register's own rendering, which is
the shape it had missed; the README's lines and the crate count agree with the workspace (13);
`coverage.yml` exists and refuses a stale commit. The one that remains is the type-system gate's
counted classes — still reported, still not enforced — and it is no longer a *finding*: the baseline's
own header now says they are a dated measurement rather than a ratchet, so the gate does not claim to
enforce what it does not.

## 5. What a green register does not cover

The project's check-off is genuinely green — **204 of 204 findings closed**, all 89 T1 rows with a
verdict — and, unusually, the law register does not overclaim: all **88** registered Rust witnesses
resolve to real functions, there are **zero `#[ignore]`d tests** in the tree, and the Lean gate refuses
`sorry`, `admit` and `opaque`. That was checked adversarially rather than assumed.

What the register does not cover is specific:

- **The assurance floor moved down and nothing measures the new one.** The close-out deleted the
  instrument, mutation and register-join harnesses on the grounds that their failures were always "a
  document disagrees with the tree". That is a defensible trade, but no mutation tool remains, so the
  real strength of the 87% line-coverage floor is now unmeasured. In the sampled module the coverage is
  assertion-dense (five mutation-style defects, five caught); the aggregate is *execution* coverage and
  is satisfiable by tests that assert only well-formedness. **This pass could not put a single number on
  the workspace's mutation score, and the apparatus that would produce one was deleted.**
- **The computed gates are weaker than they read.** The type-system gate's counted classes — `cast`,
  `lax`, `get`, `index`, `div`, `overflow` — are reported and **not enforced**; the ratchet that once
  failed the build is no longer wired.
- **The push path is much thinner than the nightly.** The formal gate, the coverage floor and the devnet
  fuzz are nightly or manual only, so a spec-side change that breaks a conformance corpus can land on a
  push and stay green for a day.
- **Nothing in the register asks what an attacker pays.** Cost is not a row type the audit can express.

## 6. Inherent and extrinsic safety

Bug-hunting audits the *extrinsic* half of a security posture — what is currently broken, which bounds
fail, which checks are missing. That half must be redone forever, because a check can be bypassed,
removed or forgotten. The other half, in Mark Miller's distinction, is **inherent safety**: a property
that follows from the semantics themselves, holding without trusting the implementer and without a
runtime check continuously enforcing it.

**Two classes of privilege-escalation defect are unrepresentable in rholang, and one is proved.**

*Authority cannot be forged.* There is no grammar production that writes a private name, and no rholang
operation destructures one. A name is allocated from a splittable hash-derived RNG and exists only to be
received: "invoke on a channel you were not given" has no term in the language.

*Authority cannot be captured across a call boundary.* A COMM step transfers exactly the evaluated
datum into the tuple space and substitutes it into the *receiver's* body in the receiver's own
environment; the sender's environment is never consulted. Reaching into a caller's variables or
continuation is not guarded — it is unsayable. Closedness is a **theorem**, not a convention: a closed
program cannot *grow* a free variable by reduction. Confused-deputy, in the sense of a deputy acting
with borrowed authority, has no expression.

Also proved: substitution preserves sort and closedness, sort is functional and decidable, and
canonicalisation is idempotent and commutative. Determinism here is itself a safety property — merge is
a commutative monoid and canonicalisation is idempotent, so execution *order* cannot change the state,
which removes a class of divergence rather than testing for it.

**Honest caveats.** Three, and they matter:

1. **Unforgeability is axiomatised, not proved.** The claim that a name's bytes cannot be guessed rests
   on the crypto axioms (law 19), not on a theorem. A break in the hash would falsify an axiom, not
   contradict a proof. Say "axiomatised at the crypto boundary" — never "proved unforgeable".
2. **The `rho:*` namespace is ambient authority.** Twenty-seven system urns are resolved
   unconditionally for every deploy, so any contract that can spell the public string can reach the
   channel — including `rho:gov:*`, whose folds were the pass's worst cost finding and are now charged
   and bounded (§3, §4). Mutation methods on some channels are additionally gated by the caller's own
   deployer identity, which is genuine capability discipline, but the channel and everything read-only
   are ambient.
3. **The blessed genesis keys are published constants in the source.** The capability they confer is
   therefore forgeable, and what actually prevents their use is a negative allow-list that rejects
   deploys signed with them. That is the cleanest seam in the codebase: a forgeable capability restored
   by an access-control list.

Genuine capability distribution does exist — a deploy's own identity is bound at normalise time from its
verified signature, so a deploy receives exactly its own identity and never anyone else's.

**The honest summary:** the language and calculus are safe-by-structure and partly proved; the chain
layer's authority distribution is access control by another name; and the cryptographic substrate is
assumed rather than established. That is where assurance ends.

## 7. Comparison with other chains

**Context first, because it is the most relevant fact in any comparison.** Solana, Sui and Bitcoin SV
are live mainnet networks and have been for years; as of September 2026 their market capitalisations
are on the order of $70bn, $5bn and $0.4bn respectively. This node is **pre-testnet**. It has had none
of the adversarial exposure, external audit budget or years of production hardening those networks
have absorbed. A comparison that omits that asymmetry is not neutral — it flatters the incumbents by
grading a codebase that has never faced an attacker as though it had.

With that stated, the comparison is worth making, and it is not made by reading declared limits. Four
nodes were probed with the *same* question, because reading constants and calling them bounds is how a
comparison goes wrong. The question was: **is there an operation reachable by an unauthenticated
remote attacker whose cost to the node is superlinear in attacker input, while the charge to the
attacker is flat or sublinear?**

Every one of the four has such an operation. This is a defect class in the design of VM cost models
generally, not a distinguishing weakness of any one chain.

**Two rows below have moved since the probe, and the table is left as measured rather than edited.**
This node's amplification instance — the `Set` dedup's Θ(N²·⁴³) for a flat 13 phlo — is now
sort-then-dedup, i.e. Θ(N log N), and the zero-phlo governance folds are charged and bounded (§3, §4),
so "worst instance found" describes the tree the probe ran against. The containment cell said "no
per-block cap"; a per-block phlo cap and a validator-side deploy-count cap have since landed, which is
the row the README's own table marks as the one this node did not lead and fixed within the day. What
remains of that cell is the third item: there is still **no timeout on the block execution path**, and
that is not repairable by bounding work, because a builtin cannot be interrupted (§3).

| | Worst instance found | What the attacker pays | Blast-radius containment |
|---|---|---|---|
| **This node** | `Set` dedup Θ(N^2.43) for a flat 13 phlo; 98 CPU-seconds at N = 40 000. Plus a zero-phlo path | 13 phlo | **no per-block cap**; no deploy-count cap on validators; no block-path timeout |
| **Solana** | `sol_big_mod_exp` charges 8 042 compute units for ≥10 ms of bignum work (≥40×). Every transaction copies up to 64 MiB of account data with **zero units charged** | flat 5 000 lamports, *identical to a transaction that loads nothing* | good: 48 M block compute-unit budget, 64 MiB loaded-data cap |
| **Sui** | A zkLogin native charges a flat 200 units for hashing up to 256 KiB passed by reference and reusable across calls (~82× against its own keccak price) | gas; transactions bounded to 128 KiB | best of the four: instruction tiers rising to 1000×, 128 KiB transaction bound, per-package verification meter |
| **Bitcoin SV** | `OP_CAT` doubles the top stack element with its size guard inside a pre-Genesis branch: **~40 bytes of script → roughly a terabyte of work**. The legacy sighash is Θ(N²) and reachable via an attacker-chosen flag bit, computed *before* verification | byte-shaped fees on a *failing* transaction collect nothing | **none in consensus**: opcode limit `UINT32_MAX`, stack cap `INT64_MAX`, and the cancellation token is skipped on the block path |

**Where this node sits:** mid-pack on amplification — better than Bitcoin SV, comparable to Solana,
behind Sui — and second-worst on containment, because Sui and Solana both bound the blast radius and
this node bounds a block hardly at all. Its instance was also the narrowest of the four to repair — a
regression against its own Scala oracle, which deduplicates through a hash set — and it is the one that
has since been repaired (§3).

On the inherent-safety axis (§6) the ordering is different, and more favourable:

| | Unforgeable authority | No ambient authority | Encapsulation | Reentrancy / confused deputy | Machine-checked |
|---|---|---|---|---|---|
| **This node** | **unrepresentable** (soundness axiomatised) | none in the language; **27 ambient urns** in the chain layer | n/a — capabilities are copyable references by design | **unrepresentable** | **Lean 4 + Coq; closedness proved** |
| **Sui** | **unrepresentable** — linear `UID` freshly minted from the transaction digest | banned at the bytecode level | **unrepresentable** — linear resources | **unrepresentable** — no dynamic dispatch | static verifier yes; **Move Prover absent** |
| **Solana** | checked — a program-derived address has no private key, but granting the privilege is a runtime check | present — any program may invoke any executable program | absent — accounts are flat cloneable buffers | checked at runtime | ABI digests only, which are checksums |
| **Bitcoin SV** | absent as a concept | fewest surfaces, by having almost no semantics | linear by consensus validation | moot — no calls exist | **nothing** |

Bitcoin SV deserves the specific note that it *negates* the model deliberately: an operator RPC injects
a consensus blacklist, and spending a blacklisted output causes blocks on the active chain to be
disconnected. That is safety by legal process rather than by semantic property — the opposite of what an
object-capability design is for. Sui is the closest analogue to this node and makes a broader set of
properties unrepresentable, but enforces them with a static verifier at publish time rather than in the
semantics, and its prover does not ship.

**Caveats that limit this table.** Solana's VM and bignum library are unvendored registry pins, and
Sui's crypto and P2P layers are git dependencies, so in both cases the component the worst native calls
into — and, for Sui, the entire network ingress path — could not be read. That is recorded as "could not
trace", not as a clean bill of health. The Solana tree compared is the archived `solana-labs/solana`
monorepo, whose last commit predates this audit; the live lineage is Anza's Agave fork.

## 8. Open questions

**These are decisions, not defects, and the close-out left them open on purpose.** Nothing here is
something the review found broken and nobody got to; each is a choice the project has to make
deliberately, and the close-out recorded them as an agenda rather than resolving them on the way past.
A reader looking for what is *unfinished* should read this section; a reader looking for what was
*found* should read §3 and §4. **One item has left this list since it was written** — the `legacy/`
tree, which was not decided but resolved — and it is kept below as a closed entry rather than deleted,
so a later reader does not raise it again.

**Active-validator-set selection has no oracle — and the rule has now been changed, which is why this
item was acted on rather than left listed.** The law register pins the epoch-boundary *timing*, not the
membership: if the rule is wrong, the epoch-boundary validator set differs from any other
implementation — a chain split — with nothing in either oracle able to arbitrate. That is still true of
the rule that replaced the old one. What is *no longer* true is the specific defect the review found
while reading it: the old seed, `hash(shard_id, block_number, sender, pre_state_hash)`, was computed at
the moment of use, so the proposer of the drawing block could reroll the set freely by proposing a
different block — a free, unbounded reroll by the one party that also chose the sample frame. As of
2026-09-28 `select_active` draws uniformly without replacement from the pool, seeded by a
`pos:epoch_seed` leaf written one boundary ahead from the **last finalised fringe's state hash** — the
>2/3-agreed frontier, which no single proposer moves — so the block that draws is not the block that
chose the entropy, and the entropy is not one party's to choose. **The residuals are named in [`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md)
§3 item 12** — the seed-setter's influence through its justification set (reduced, not closed: a
proposer may still present a stale fringe, so the steering space is the distinct fringes its own
candidates induce rather than one per justification subset — and the pre-state anchor that would
have restored the larger space is deliberately *not* kept beside the fringe), capital pre-positioning
before a public seed, the sybil exposure uniform sampling carries where stake-weighting does not, and a
security budget that now fluctuates epoch to epoch — and the uniform draw is the one decision there
worth revisiting. **And the draw carries a liveness consequence that the deterministic rule masked**,
measured on devnets with a cap below the validator count: an epoch's proposal duty can land on a
validator whose own view of the active set is stale, while the node that could propose is drawn out —
so the chain waits. With no cap every bonded validator is always in the set, so there is always a
proposer; that is why this arrives with the draw rather than before it. The gate that produces it is
*self-imposed* — a drawn-out validator's block would still be accepted by its peers, since no
receiving-side rule tests the sender against the active set — so reading the **pool** rather than the
drawn set in `check_active_validator` would remove the hazard, at the cost of letting non-active
validators propose. That is a change to proposal behaviour and is left as a decision. Measurements and
the control are in [`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) §3 item 12.
The membership predicate remains unverified by either oracle; it is now *different* and *documented*,
which is not the same as *checked*.

**Two permissive defaults define what the contract treats as an arithmetic fault.** An epoch length of
zero and a minimum bond of zero make every block an epoch boundary and zero the reward, where the
contract divides by both and faults — a node running these mints and pays on a schedule the contract
cannot express. **Where they are reachable is narrower than it first reads, and worth stating**: a
*configured* node never sees them (`epoch-length` is 10000 and `bond-minimum` 1 in
`node/src/configuration/`), and `PosParams::default()` is reached only when no genesis PoS state has
been installed at all (`rholang/src/native_state.rs`, the `None` arm of the params read). So this is a
decision about ad-hoc runtimes and tests, not about a devnet or a testnet.

**Whether the margin on term depth is worth stating.** The parser and storage depth guards have
measured values but their agreement with the depth measure is an acknowledged proof debt. They are also
a hard fork: a term an older node accepts, a newer one refuses.

**Resolved: the `legacy/` tree was archived out of the working tree** (2026-09-28). It was built and
scanned by no CI job, and it carried its own 2020–21 dependency manifest whose advisories `cargo-deny`
cannot see because it walks only the Rust lockfile — so it was never a runtime surface, but
re-enabling its build would have resolved to known-vulnerable versions with nothing to say so. It now
lives at the commit that froze it (`1b7583649`), and every `legacy/…` citation in this repository — in
code comments, in `spec/`, and in this book — resolves there, which is why the revision is recorded
rather than the tree merely deleted. See the README's *Where the Scala went*.

**Whether the fringe's liveness predicate should be re-decided.** It compares cardinalities — how many
messages, not which senders — so two messages from one sender plus one from a second satisfy a
three-validator bond map. This is a faithful port of an upstream gap and is already registered as an
upstream design property; it is listed here only because a liveness rule that counts rather than names
is the kind of thing worth re-deciding deliberately rather than inheriting.

# OCapN + chain bridge — HAZOP and root cause analysis

**Reviewed revision.** `dev` at `918ce16fe490674449f62b3c89bbc8dd171e34e3` — the merge of PR #251
(`ocapn/stage6`), which closed #250. Everything below is about **that revision**; fixes proposed here
land on a later branch, and where a fix has landed the row says so.

**Why this study exists.** The revision is green: 24/24 against the OCapN project's conformance suite
at `31f0b80` (`ocapn-conformance/run-7.txt`) and an `@endo/ocapn` transcript against a real node
(`endo-spike/run-2.txt`). A conformance suite answers *is the wire right*. It does not answer *what
does the wire let a stranger do* — allocate, wait on, spend, or crash. Six defects were found by the
two oracles during the revision's own verification and fixed before merge, which is the measure of how
much the oracles were worth and also of how much they left unexamined. This is the pass they did not
run.

**Method.** The worksheet format is this repository's own (`docs/src/spec/testnet-acceptance.md` §1):
per unit, `Table A — deviations`, `Table B — barriers`, `Provenance`. The synthesis format is
`spec/audit/evidence/n117-audit.md`. Guide words, applied to every node: **No/None, More, Less
(including part of), As well as, Reverse, Other than, Early, Late**.

**Units (nodes), with their design intent.**

| node | what it is | intent |
|---|---|---|
| N1 | inbound handshake and session acceptance (`owner::accept_and_book`, `SessionRegistry::admit`) | a completed handshake is booked, then answered; a crossing kills exactly one leg |
| N2 | the session loop and its tables (`owner::SessionLoop`, `conn::handle_message`) | one task owns the socket and the tables; a peer's message is acted on or refused, never silently ignored |
| N3 | dial-out and the enlivener (`enliven.rs`) | a sturdyref names a peer we dial, fetch from, and hand back as a forward |
| N4 | the bootstrap and the gift store (`bootstrap.rs`, `handoff.rs`) | `fetch` resolves a swiss number; a gift is deposited once, withdrawn once per handoff count |
| N5 | the cross-session proxy (`proxy.rs`) | a delivery on one session becomes a delivery on another, and its answer comes back |
| N6 | bridge ingest (`ChainCapability::deliver`) | a delivery becomes a signed deploy; the method rides the message |
| N7 | bridge registration (`invoke_member_term`, `insertArbitrary`, the pattern builder) | a returned capability becomes addressable chain state without leaving the node as a URI |
| N8 | the wire (`syrup.rs`, `netstring.rs`, `tcp_testing_only.rs`) | a bounded, streaming parse of an adversarial byte stream |
| N9 | the node's listener lifecycle (`serve_ocapn`, config gating, per-connection tasks) | the listener serves until stopped, and is off unless configured |
| N10 | capability addressing (`target_uri` + `pattern` → `lookup!` → call) | a peer's descriptor reaches the chain object it names, or breaks with a reason |

## §1 The confirmed hazard inventory (this study's input)

Facts, cited to `file:line`. "Measured" means a run or a test produced it; "read" means the code says
so and no run was made — the distinction is carried into the worksheet.

| id | hazard | the sharpest fact | how known |
|---|---|---|---|
| **H1** | no timeouts on any socket, and connections are uncapped | `tcp_testing_only.rs:77` (connect), `:120` (read), `:140` (write) have no timeout; grep confirms no `set_read_timeout`/`set_keepalive` anywhere in the crate. `node/src/api/ocapn.rs:126-137` accepts in an unconditional loop and spawns a task per connection with no cap. **A peer that connects and never sends `op:start-session` pins a task for ever** (`conn.rs:846`). | read |
| **H2** | six unbounded, peer-growable, never-evicted tables | `Session.exports` (`conn.rs:231`; `op:gc-exports` ignored at `:471`); `Session.answers`, keyed by the **peer's** `answer_pos` (`conn.rs:233`; `op:gc-answers` ignored at `:471`); `Handoffs.gifts`, keyed `(gift_id, session)` from the **peer's** bytes (`handoff.rs:250-272`); `Gift.withdrawn`, keyed by an **attacker-chosen `u64`** (`handoff.rs:230,294`); `PromiseCell.listeners` (`fixtures.rs:312,342`); `SessionRegistry.peers`, keyed by the **peer's own designator/transport strings** (`owner.rs:234,249`) — and `forget` clears a slot but **never removes the entry** (`owner.rs:350-363`). | read |
| **H3** | unbounded consensus growth and node spend | every delivery to the ERTP capability runs `register!(reply, *uriOut)` **unconditionally** — including a reply that is pure data (`casper/src/shard_invoke.rs:127-131`) — storing the whole reply value, capabilities included, as a permanent registry entry (`rholang/src/system_processes.rs:1639-1641` → `native_state.rs:2356-2362`; **no delete exists anywhere**). Each call is also one signed deploy paid from the node's own REV (`CHAIN_PHLO_LIMIT = 1_000_000`, `CHAIN_PHLO_PRICE = 1`, `node/src/api/ocapn.rs:80-81`). No rate limit. | read (the cost was *measured* during the revision: `preCharge: insufficient funds (0 < 1000000)`, `endo-spike/run-2.txt:24`) |
| **H4** | the design page's invariants are false | `docs/src/node/ocapn.md` invariant 1 — "the only chain-visible object is the caller-signed deploy" — is contradicted by H3; invariant 2 claims the node's OCapN designator is its secp256k1 deployer key and that a signed session-key↔deployer-key binding is carried in the session. The code's designator is the literal `"rnode"` (`node/src/api/ocapn.rs:112-119`), the session key is Ed25519, and **no such binding exists**: every bridged deploy is signed by the node's own key, so the chain sees the node as the caller — which `ChainCapability::key`'s own doc comment admits (`:198-201`). | read |
| **H5** | no operator visibility | `node/src/api/ocapn.rs` (478 lines) contains **no logging at all**: nothing says the listener is serving, and a refused delivery or a 30 s timeout leaves no trace. The node's other listeners use `rchain_shared::log`. | read |
| **H6** | the greeter's unbounded dial, inside another session's loop | `fixtures.rs:173` — `Session::dial(...)` with no timeout, awaited **inside the serving session's loop** (`conn.rs:591` → `target.deliver`), so a give naming a silent exporter blocks that loop indefinitely. The enlivener carries exactly this bound for exactly this reason (`enliven.rs:38`); the greeter was missed. | read |

**Two defects in this revision's own evidence artifact**, found while assembling this study and
verified against the files:

| id | defect | the verification |
|---|---|---|
| **H7** | `ocapn-conformance/README.md`'s per-module table carries a **run-1 column filled with run-4's numbers** | `run-1.txt` contains `FAIL: test_op_listen_*` ×3 (`:152,171,190`) and `ERROR: test_gc_*` ×4 (`:54,73,82,91`), so run-1 was `op_listen` 0/3 and `op_gc` 0/4 — **9/24**, not the 16/24 the table shows under run-1's name. |
| **H8** | the same page says "**Six** whole-suite runs are kept" while `run-1 … run-7` — seven — are beside it. | `ls spec/audit/evidence/ocapn-conformance/` |

H7 and H8 are the same class as the false clause-4 claim corrected in this revision's own issues: a
claim about an artifact, written from memory, that nothing re-derives from the artifact. They are
worked examples in the RCA below, not incidental.

## §2 The RCA: hypotheses to be tested, not asserted

| # | systemic cause | the evidence that would confirm it |
|---|---|---|
| **R1** | **self-interoperation hides protocol defects** — both ends are ours, so a symmetric misreading passes our own tests | `Session::dial` never wrote its own start-session and our own round-trip test passed (`ocapn/tests/session_round_trip.rs`) |
| **R2** | **a round-trip test validates the assumption, not the requirement** | `par_value`'s test asserted *our* `ETuple ↔ Record` choice as though it were a property; the choice was only spellable for a tuple with a symbol head |
| **R3** | **fixtures are permissive** — they never exercise the mapping's own rules | no fixture ever sent a non-symbol-headed tuple, so the label rule was unreachable |
| **R4** | **prose claims are not tied to artifacts** | the clause-4 claim; H7 and H8 above |
| **R5** | **the local gate list ≠ the CI gate list** | five of the eight `ci.yml` gate steps were run locally; the type-system gate caught the `expect` in CI (`4b6010223` names it) |
| **R6** | **"green at a revision" read as "conformant"** | C216/C217 — the draft and the two reference implementations disagree with each other |

## §3 The proposed remedies (to be judged, not assumed)

Tiered by the design pass; tiers 1–2 are unambiguous and land independently of this study's verdict.

**Tier 1.** H4 doc corrections **with an observable** (rule 1: a doc edit is paperwork) — a test
asserting the listener's designator is the literal and derives nothing from the deploy key. H5 logging
in the house idiom (`LogSource`, never `info` per connection). H1a a handshake bound in
`conn.rs::read_start_session` (30 s, the crate's own constant). H6 the greeter's dial bound, reusing
it.

**Tier 2.** H1b a connect bound in the netlayer. H1d a session `Semaphore` in `serve_ocapn` and the
fixture peer. H3b a shared `RateLimiter` (`shared/src/rate_limiter.rs`, already bounding the faucet
routes) on `ChainCapability::deliver`.

**Tier 3.** H2 — a `ocapn/src/capacity.rs` `Bounded<K,V>` with **no unbounded constructor**, wired to
the six sites, each cap ≥ 20× the suite's observed usage, each refusal an honest `break`/`abort` naming
the bound; plus `SessionRegistry::forget` removing an emptied key, which is a bug on its own.

**Not proposed, deliberately.** A steady-state socket read timeout: it would break three suite tests
(including one that legitimately withdraws before depositing) and makes "an idle session"
unrepresentable. Tier 4: H3a is the one hazard judged **not fixable cheaply** — the node cannot know
whether a reply holds a capability before registering it, because the value does not survive the
evaluation — so it gets a corrected invariant, a doc comment, and a `todo` register row naming the
two-deploy design that would close it.

## §4 The worksheet

Nine agents: six lenses (protocol, concurrency, resources, authority, operations, verification), a red
team that **ran** its attacks against the fixture peer, and a steelman, then adjudication. Every row
carries: node · guide word · deviation · **how it is known** (measured / reasoned) · severity ·
likelihood · the steelman's outcome · disposition.

Severity: **S1** chain-visible or node-down · **S2** node-level resource or authority, recoverable ·
**S3** one session or task, recoverable · **S4** diagnostic. Likelihood: **L-A** any peer that reaches
the port · **L-B** a peer deliberately speaking the protocol · **L-C** operator misconfiguration ·
**L-D** needs a race or a specific fixture shape.

### Table A — deviations

| # | node | word | deviation | known | S | L | steelman | disposition |
|---|---|---|---|---|---|---|---|---|
| **A1** | N6/N7 | Other than | **A value delivered by a peer became Rholang *code* in a deploy the node signs with its own key.** `PrettyPrinter` writes a string as `"…"` with no escape (`pretty_printer.rs:481`) and Rholang's lexer reads to the next `"` (`parser.rs:121-137`), so an argument containing a quote ends its literal and the rest is parsed as a program. Reachable by any peer that completes the handshake, through either published capability — and through the admin txn route (`txn_coordinator.rs`'s `render_arg`), where the caller supplies the destination. | **measured** (probe: the payload rendered as a second send and `source_to_adt` parsed it) | **S1** | L-A | **fails** — no safeguard existed on either path | **C220, fixed here**: `check_renderable` + `Result` on all three builders + three regression tests |
| **A2** | N6 | More | **The node is the caller for every peer.** One key signs all bridged deploys, so the chain sees the node as the counterparty; the session key is ephemeral Ed25519 and **no session↔deployer binding exists**. | read | S2 | L-A | **fails unless documented** — legitimate *while* the surface is unpublished, and A1 made it a full break | docs corrected; the binding is **declined with Law 63a** (AUDIT C221) — it changes what the node *knows*, not who pays, so the relay is the closure |
| **A3** | N7 | More | **Every ERTP delivery mints a permanent, undeletable registry entry** — including replies that are pure data — and spends the node's own REV (`CHAIN_PHLO_LIMIT` 1 000 000), with no rate limit. | measured cost (`preCharge: insufficient funds`), read for the growth | S2 | L-A | **fails unless documented** — `insertArbitrary` is *meant* to mint; doing it unboundedly on a peer's behalf is the defect | **spend half fixed here** (one node-wide limiter, from the chain's cadence rather than a round number); the growth half is **closed by decision (C221, 2026-10-06)** — the relay closes it, and the relay is a cross-implementation change, so the residual (permanent growth, rate-bounded only) is stated in the row rather than left open |
| **B1** | N1/N9 | More | **No socket has a timeout and connections are uncapped.** A peer that connects and sends nothing pins a task for ever; 3 000 idle connections cost +41 MB and the RSS is **never returned**. | **measured** (`fds=3007`, `13.9KB/conn`, `rss 55704KB` retained) | S2 | L-A | **saves** only under "an operator binds to a trusted interface" — which the page concedes | handshake bound + connect bound + session cap land here (**C222**) |
| **B2** | N2/N4 | More | **Six tables grow without bound and are never evicted**, three of them keyed by peer-chosen bytes: `answers` (the peer's `answer_pos`), `gifts` (its gift id), `withdrawn` (an arbitrary `u64`), plus `exports`, `PromiseCell.listeners` and `SessionRegistry.peers`. **Measured: 3 independent grow-forever vectors, no cap, no refusal** (`gifts` at 191 B/msg, ~7 MB/s/session). `op:gc-exports`/`op:gc-answers` are accepted and **ignored** — 50 000 answers collected nothing. | **measured** | S2 | L-A | **saves** for five of six as per-session state on a dev surface; **fails** on the sixth (below) | **fixed here (C223)**: all six bounded, and the *keys* bounded separately — the resource lens's correction, since a count cap of 1024 over a peer-chosen 4 MiB gift id is a 4 GiB table. **Both of the residues named here are closed too**: an inbound `op:gc-exports`/`op:gc-answers` now removes positions from the export and answer tables rather than being a no-op, and the 10 s deposit wait below moved off the serving loop (Law 61) |
| **B3** | N2 | More | `SessionRegistry.peers` is keyed by the peer's own designator/transport and **`forget` never removes the key** (`owner.rs:350-363`) — ordinary operation accumulates one entry per distinct peer for the process's life. **And a second defect underneath it, found while writing this row's test: `forget` compared the caller's identifier against the *stored* one, and the two slots store different identifiers** (the dialed slot is keyed by ours, the accepted slot by the peer's) — so an **accepted** session, which is every peer that connects to us, was never cleared at all. A test that asserted `live() == None` could not see either: the stale handle answers `Closed` and looks like a session that ended normally. | read (measured, confounded by allocator retention); the second defect **measured** by `a_peer_whose_sessions_have_ended_is_forgotten` | S2 | L-A | **fails** — `forget`'s contract is to stop comparing against a dead session, and it is the *key* that leaks | fixed here (both halves) |
| **B4** | N3/N4 | Other than | **The node dials any host:port a peer names** — a sturdyref's peer locator and a give's `exporter-location` are peer-controlled — and writes its fresh `op:start-session` there, so it is a blind port-probe into the node's network position. | **measured** (the attacker's server received our 306-byte start-session) | S2 | L-A | **fails** — dialing is what OCapN *is*, but with no allow-list and no connect timeout, "dial" and "scan the LAN" are one operation | **registered** (C222); connect bound lands here |
| **B5** | N4 | Late | `DEPOSIT_WAIT` polls for 10 s **inside the serving session loop**, so any peer stalls its own session 10 s per delivery, repeatably, using only its own key and id. | **measured** (`fetch right after a withdraw for an undeposited gift: 10.02s`) | S3 | L-A | **fails** — the constant's own comment says a claim "has to fail rather than hold a session's loop open", and 10 s of polling is exactly that | registered (C223) |
| **C1** | N3 | More | **The greeter's dial is unbounded and runs inside the serving loop**: one give naming a silent exporter blocks that session **for ever** — measured still blocked at 35 s, past the enlivener's own 30 s bound. | **measured** | S2 | L-B | **fails** — the codebase's own prior: the enlivener carries this bound for this reason; the greeter was missed | fixed in C222 |
| **C2** | N2/N5 | Reverse | `SessionHandle::deliver`'s mpsc **send** has no timeout (depth 64). Every *receive* is 30 s-bounded; the send is not, so a saturated channel wedges a session permanently rather than for 30 s. | reasoned | S2 | L-D | **fails** (the asymmetry is not defensible once named) | fixed in C222 |
| **C3** | N3 | More | The enlivener bounds its **fetch** but not its **dial** (`new_outgoing_connection`, no timeout). | reasoned | S2 | L-B | fails | fixed in C222 (connect bound) |
| **C4** | N9 | Early | The proposed session cap is **defeated by idle or stuck peers** holding permits — and idle is legal by design. The cap converts an unbounded DoS into a bounded one, not into no DoS. | reasoned | S2 | L-B | **fails unless** the cap is paired with an idle/per-peer bound | C222 lands the cap *with* C1 and B1 so no permit is permanently holdable, and records the residue |
| **D1** | N1 | Other than | A **spec-permitted** locator (`hints` as Syrup `f`, which `Locators.md` allows) is decoded but then refused, because the signature is verified over a **re-encoding** rather than the received bytes. | read | S3 | L-B | **saves** — the suite always sends `{}` and never verifies our signature | registered (C224) |
| **D2** | N1/N3 | More | **Every RChain node's designator is the constant `"rnode"`**, and peer identity *is* `(designator, transport)` — so two nodes are the same peer: a sturdyref to one resolves at the other, and the registry conflates their sessions. | read | S2 | L-A | **accepted** for a single-node dev surface; a real interop defect the moment there are two | registered (C224) |
| **D3** | N3/N6 | No | Every **outbound** `fetch` sends the swiss number as `Bytes`; C217's tolerance is inbound-only, so this port cannot enliven a sturdyref at a peer that accepts only a String. | reasoned | S3 | L-B | **saves** by absence of evidence (the one foreign peer we dialled was Endo, and it dialled *us*) | registered (C224) |
| **D4** | N6 | Other than | A capability **as an argument** is refused: `desc:import-*` maps to a tuple whose label is a Symbol, and `par_value` refuses Symbols inbound — so an ERTP `deposit`/`transfer` taking a purse is unreachable over the bridge. | read | S3 | L-B | **saves** — the refusal is a break with a reason, never a silent copy of authority | registered (C224) |
| **D5** | N2 | Late | Replaying an `answer_pos` is **silently accepted and re-points** that slot at the newer export — a hole in the "refusals, never silent" discipline. | **measured** (2nd delivery on the same position fulfilled with a different object) | S4 | L-B | **fails** as a discipline breach; not cross-peer exploitable | registered (C223) |
| **E1** | — | Reverse | **The design page's invariants assert the opposite of the code** in two of three: "the only chain-visible object is the deploy" (A3 contradicts it) and "the designator is the secp256k1 deployer key, and a signed session↔deployer binding is carried in the session" (the designator is `"rnode"`; no binding exists). | read | S2 | L-C | **fails** — the criterion is *unmarked*, and the code's own comment already admits the truth | fixed here |
| **E2** | — | Other than | The conformance README's **run-1 column carries run-4's numbers**: run-1 was `op_listen` 0/3 and `op_gc` 0/4 — **9/24**, not 16/24. | measured (against `run-1.txt`) | S3 | L-C | **fails** — the page's purpose is that a measurement can be compared later | fixed here |
| **E3** | — | Less | The same page says "**Six** whole-suite runs are kept" beside seven `run-*.txt`. | measured | S4 | L-C | fails (trivial) | fixed here |
| **E4** | — | Other than | The same page names branch **`ocapn/ertp-interop`** as what was measured, while runs 5–7 each say `ocapn/stage6`. | measured | S3 | L-C | fails — it is the field a reader uses to know what was measured | fixed here |
| **E5** | — | No | The Endo README quotes a failure line (`fetch expects a byte-array swiss number`) that appears in **no** kept artifact. | measured | S4 | L-C | fails | fixed here |
| **E6** | N9/N6 | No | **No operator visibility**: nothing says the listener is serving; a refused delivery, a cap refusal or a 30 s timeout leaves no trace; no counter for the one action that costs money. `docs/src/node/operating.md` never mentions `ocapn-listen`. | read | S2 | L-A | **fails unless documented** — the peer-facing breaks do not rescue the operator | **fixed here**: an `info` on bind naming the *resolved* address, a `warn` at the session ceiling, a `debug` per unserved session (not `warn` — a crossed hello is legitimate and a peer-driven stream of warnings would make them meaningless); `operating.md` now names the listener and what binding it publishes. A spend **counter** stays owed — the metrics surface has no OCapN slot, and adding one is its own unit |

### Table B — barriers (attacks that **failed**, each measured by the red team)

| attack | defence |
|---|---|
| netstring: non-digit prefix, `u64` overflow, declared length past the cap | `netstring.rs:38-44`, `tcp_testing_only.rs:28,113-118` |
| Syrup nested past 256 (pre- and post-handshake); trailing bytes after a value | `syrup.rs:35,333-336,131-140` |
| deliver to an unallocated export or answer; negative/non-integer position | `conn.rs:583-588,806-826`, `captp.rs:265-270` |
| a second `op:start-session`; an unknown operation; `op:listen` on a non-promise | `conn.rs:460-469,472-477,667-671` |
| piping onto an answer whose delivery broke | `conn.rs:575-583,596-598` |
| a handoff receive forged with the wrong key; one naming another session | `bootstrap.rs:155-159,161-179` |
| a replayed handoff count | `handoff.rs:294-298` |
| a full message is acted on before the next is parsed | `conn.rs:434-445` |

**The wire is the part that held.** Nine parse-shaped attacks, every one a named refusal, no panic,
no partial result — the crate's own claim, independently confirmed.

## §5 The merged fault tree

```
                    a stranger reaches api-server.ocapn-listen
                                   │
        ┌──────────────────────────┼───────────────────────────────┐
        │                          │                               │
   A1 injection              B1..B5 resource                 A2/A3 authority
   (S1, FIXED)               (S2, registered)               (S2, documented)
        │                          │                               │
  the printer is not        no socket timeout,             one key signs for all
  a faithful serializer     no connect timeout,            → the chain sees the node
        │                   no session cap,                       │
  two paths *parsed*        six unbounded tables           A3 every delivery writes
  what it printed           (three keyed by peer bytes)   permanent consensus state
        │                          │                     and spends the node's REV
  audit C220                C1/C2/C3 a blocked loop       (no rate limit)
                            is a pinned task
                                   │
                            B4 the same dial-out has
                            no allow-list → SSRF
```

## §6 The RCA, with falsifiers

| # | hypothesis | verdict | what would have caught it earlier, cheaply |
|---|---|---|---|
| **R1** | self-interoperation hides protocol defects | **partly** — real, but the offered instance was wrong: `session_round_trip`'s hand-driven client would have *hung* on the defect it supposedly hid. The genuine instance is R2. | one foreign peer, which is what found it |
| **R2** | a round-trip test validates the assumption, not the requirement | **confirmed** — `par_value`'s test asserted *our* `ETuple ↔ Record` choice; a round trip is satisfied by any symmetric misreading. | assert the **wire form** for a value, not the round trip |
| **R3** | fixtures are permissive | **confirmed by absence** — no fixture ever sent a boolean-headed tuple, so the label rule was unreachable | one fixture whose reply is the `(ok, value)` shape the codebase itself uses |
| **R4** | prose claims are not tied to artifacts | **confirmed, four times in one revision**: the clause-4 claim, H7's column, H8's count, E4's branch — and a fifth in the Endo page (E5). | emit the table from the transcripts with a `--check` — the register's own pattern |
| **R5** | the local gate list ≠ the CI gate list | **partly, and in the wrong slot** — the statement is true (5 of 8 locally), but explains none of the six defects; the one gate that caught something (type-system) *is* in the local list | run the whole list; `make check-register` is one command |
| **R6** | "green at a revision" read as "conformant" | **confirmed as a finding, refuted as a cause** — none of the six was a spec-vs-reference disagreement | name each oracle's *input domain* |

**The mechanism behind the six, in one sentence.** Coverage follows the **input domain**, not the
protocol name: the Python suite produces fixtures, roles, refusals and a dial-back; Endo produces a
real application, a chain, and the value shapes that go with it. They are disjoint, and every input
neither can emit was untested — so "green" meant "no oracle's inputs failed", which is exactly what it
should be read as, and was not.

**And the deeper one, for A1.** The printer's unfaithfulness was **documented in the tree**: a test named
`the_documented_warts_print_what_the_grammar_cannot_read_back` says in bold that "a printed term must
not be pasted into the REPL as source", and `spec/audit/passes.md` §16 records it. The knowledge
existed and the two call sites crossed the boundary anyway, because **nothing distinguished the
display use of the printer from the serialize use**. The fix is a checked serializer
(`check_renderable`) and a doc section saying which is which — a distinction, not a new rule.

## §7 The verdict

**The conformance claim stands; the operability claim does not.** 24/24 at `31f0b80` remains true and
was independently re-verified — the wire is right, and the barrier table above is the red team's
measured confirmation of nine attacks that all failed. What the suite could not see is everything that
is not a conformance failure, and that is where this study found its hazards.

**One was critical and is fixed.** A1 (C220): a peer's value became code in a deploy signed with the
node's own key. Reachable by any peer that completes an unauthenticated handshake, on a node that has
`ocapn-listen` set and a deployer key. Default configuration is not exposed. Fixed at the root — the
printer is no longer trusted as a serializer — with three tests, and the same class closed at the
second door (`txn_coordinator`).

**Nothing else is critical, and most of it is cheaper than it looks.** One exploit-worth of code
(tiers 1–2: three bounds and a rate limit, each with a named observable) plus paperwork (four artifact
claims, one design page, one operator sentence). The steelman saved one row (H1 under a loopback bind
is defensible as *documented*, not as *undefended*) and lost eight.

**What the review recommends against.** Faking a bound on A3's consensus growth: the node cannot know
whether a reply holds a capability before registering it, so the honest state is a corrected invariant
and a `todo` row naming the two-deploy design. A bound that does not bound would be worse than the row.

**The most transferable output** is the coverage map in §6: **write down each oracle's input domain,
and treat every input no oracle can emit as untested.** The next protocol-shaped unit in this repo
should carry that list before it carries a green suite.


## §6 The RCA, with falsifiers

*(adjudicated)*

## §7 The verdict

*(adjudicated)*

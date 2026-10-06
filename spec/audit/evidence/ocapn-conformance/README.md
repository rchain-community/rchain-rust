# OCapN conformance: the first run

**What this is.** The result of running `ocapn/ocapn-test-suite` — the OCapN project's own
conformance suite, the thing every implementation is checked against — against this repo's
`ocapn-tcp-testing` peer. It is the first time this port has spoken CapTP to a foreign
implementation rather than to itself.

## The run

| | |
|---|---|
| Date | 2026-10-05 |
| Implementation under test | `rchain-ocapn` — **`ocapn/stage6` for runs 5–7**, `ocapn/ertp-interop` for runs 1–4 — `cargo build -p rchain-ocapn --bin ocapn-tcp-testing` |
| Suite | `github.com/ocapn/ocapn-test-suite` at `31f0b80` |
| Netlayer | `tcp-testing-only` (the suite's own; no encryption — it is not a deployment transport) |

**Eleven** whole-suite runs are kept, and every one from `run-7.txt` on is **24/24**. The later runs
are the ones a reader needs to date a change: `run-8.txt` is the session-layer residue work (issue
#254), `run-9.txt` wave 2 (issue #255), `run-10.txt` the bridge's outbound tuple encoding changing from
a Syrup **list** to OCapN's **tagged** value (`<desc:tagged 'rho:tuple' [fields…]>`, AUDIT C226), and
`run-11.txt` the deferred-answer work that moved the deposit wait off the serving loop (Law 61, AUDIT
C223). `run-10.txt` is the one that matters for the wire shape: the suite is unchanged at **24/24**, so
the shape the law forced is one a foreign implementation already speaks. (The suite never sends a tuple
of its own, so that run shows the change is *compatible*; it is not the run that exercises it.
`node/tests/lean_syrup_corpus.rs` and `node/tests/ocapn_listener.rs` are the parties that do.)

**Six** whole-suite runs precede them. `run-1.txt` (`failures=9, errors=8`), `run-2.txt` after
`op:listen` (`failures=6, errors=8`), `run-3.txt` after `op:gc-exports` (`failures=6, errors=5`), and
`run-4.txt` after `op:gc-answers` (`failures=6, errors=4`) are the stage-0–3 pass; `run-5.txt` is the
driver's baseline; `run-6.txt` is stage 6's first half (the owned dialed session and the sturdyref
enlivener: `op_start_session` 3/5 → **5/5**); and **`run-7.txt` is the finished suite, 24/24**. **Read
the per-module numbers, not a summary line.** The runner reports a `setUp` error against the test it
aborted as well as the error itself, so its tallies exceed the test count; running each module on its
own gives the numbers below.

## Per module

**Read each column against its own run file.** Earlier versions of this table carried a "run-1" column
holding run-4's numbers, which is worse than no table: `run-1.txt` is `op_listen` **0/3** and `op_gc`
**0/4** (three `FAIL:` and four `ERROR:` lines, `:152,171,190` and `:54,73,82,91`), i.e. **9/24**, and
the 16/24 in that column is `run-4.txt`'s state. Found by the HAZOP
(`spec/audit/evidence/ocapn-hazop.md`, rows E2–E4):

| Module | run-1 | run-4 | run-6 (stage 6, half) | **run-7 (finished)** | Note |
|---|---|---|---|---|---|
| `op_abort` | 1 / 1 | 1 / 1 | 1 / 1 | **1 / 1** | ✅ |
| `op_deliver` | 4 / 4 | 4 / 4 | 4 / 4 | **4 / 4** | ✅ including both promise-pipelining tests and the break-propagation test |
| `op_start_session` | 3 / 5 | 3 / 5 | **5 / 5** | **5 / 5** | ✅ the crossed-hellos tests need the sturdyref enlivener, which run-6 has |
| `op_listen` | **0 / 3** | 3 / 3 | 3 / 3 | **3 / 3** | ✅ the promise-resolver fixture, heard both before and after the settlement |
| `op_gc` | **0 / 4** | 4 / 4 | 4 / 4 | **4 / 4** | ✅ `op:gc-exports` with its wire-delta accounting, and `op:gc-answers` |
| `third_party_handoffs` | 1 / 7 | 1 / 7 | 1 / 7 | **7 / 7** | ✅ all three roles — Receiver, Exporter and Gifter |
| **Total** | **9 / 24** | **16 / 24** | **18 / 24** | **24 / 24** | |

So **every stage of the implementation guide now passes**: the handshake (with the two refusals it
must make and the crossed-hello rule, asserted from both sides), `op:deliver`, the export table,
promise pipelining through the answer table, `fulfill`/`break` through `resolve-me-desc`, `op:listen`
with its promise/resolver pair, the GC accounting in both directions, the *sturdyref enlivener*, and
third-party handoffs with their signature and replay checks.

**What run-6 → run-7 took**, because it is the part a later reader will otherwise re-derive:
`Session::dial` never wrote its own start-session (it read the peer's and stopped), a session was
booked in the registry *after* it answered the peer (so a delivery on the first round trip could
reach an object that could not find its own session), and the gift store was keyed by gift id alone
(two independent handoffs in one process sharing `b"my-gift"` shared a replay guard). The three are
written up at the head of `run-7.txt`, and the last one is why `handoff::Handoffs` is keyed by
`(gift id, the gifter's session)`.

## How to reproduce

```sh
cargo build -p rchain-ocapn --bin ocapn-tcp-testing
./target/debug/ocapn-tcp-testing 127.0.0.1:22045 &   # a fresh session key per session
python3 spec/audit/evidence/ocapn-conformance/run-suite.py /path/to/ocapn-test-suite \
    'ocapn://rnode-ocapn.tcp-testing-only?host=127.0.0.1&port=22045' --all
```

**The driver is in this directory (`run-suite.py`), and that is deliberate**: the suite's own
`test_runner.py` imports the Tor netlayer at load, so a `tcp-testing-only` run needs one that does
not, and a gate nobody can re-run is not a gate. It runs **one module per invocation** — a module
that fails in `setUp` would otherwise make every later module's numbers a statement about the first —
and reports counts from the result object rather than the printed tally, which double-counts a
`setUp` error against the aborted test. `--module tests.op_deliver` runs a single module; `-v` keeps
the per-test lines for the record. `run-5.txt` is the driver's baseline run. The suite needs Python
3.10+ and `cryptography`.

## Why this file exists

The numbers here are a *measurement*, and a measurement whose configuration no longer exists is
worth nothing (the lesson of AUDIT C215). The suite revision, the port's branch, and the per-module
counts are recorded so a later run can be compared against them rather than remembered.

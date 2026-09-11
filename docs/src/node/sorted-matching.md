# Sorted (content-addressed) matching

> **Status:** implemented. RSpace now selects the sorted-first candidate by content hash (see
> [Determinism](rspace.md#determinism)). This changes post-state hashes only for multi-candidate deploys,
> so it remains a consensus-affecting change if the node is ever on a live chain.

## 1. Problem

When a produce or consume has more than one matching candidate, RSpace must pick one. Today that choice
is made by **insertion order** — the list is scanned front-to-back and the first match wins, and the hot
store prepends new entries, so the *newest* candidate wins:

- `rspace/src/space_matcher.rs` — `find_matching_data_candidate` walks `data` from the head and returns
  the first datum whose pattern matches; `extract_first_match` walks `match_candidates` front-to-back and
  returns the first continuation that fully matches.
- `rspace/src/hot_store.rs` — `put_datum` / `put_continuation` do `insert(0, …)`, so lists are
  newest-first (`HotStoreState` holds per-channel `Vec<Datum<A>>` / `Vec<WaitingContinuation<P, K>>`).

This is faithful to the Scala **replay** path (`ReplayRSpace.scala` scans in natural order, filtered by
the recorded COMM). But the **live** Scala oracle is *not* deterministic here: it shuffles the candidate
lists before matching —

- `legacy/rspace/.../RSpaceOps.scala:50-53` — `shuffleWithIndex = Random.shuffle(…)` (unseeded global RNG);
- `legacy/rspace/.../RSpace.scala:82-87,115-144` — live `produce`/`consume` call `shuffleWithIndex` on the
  data and continuation lists.

The Rust port deliberately replaced that shuffle with deterministic first-match — a *deterministic
improvement*, but one whose determinism is **order-sensitive**.

The order sensitivity has two independent consequences:

1. **Matching.** *Which* datum/continuation is consumed depends on the order produces/consumes are applied
   to the channel — this is the order-sensitivity that the change removes.
2. **State hash.** The trie leaf is already canonical (the serializers sort — `encode_datums` /
   `encode_continuations` in `scodec_serialize.rs`), so the state hash is a pure function of the *set*
   that survives. The order-sensitivity reaches the state hash only *through selection* — a different
   order consumes a different datum, leaving a different set.

Together these force same-channel effects to be applied in a single fixed order (the reducer's DFS
order). That, plus the **continuation-footprint** problem — a produce/consume's continuation channel
footprint is only discovered *after* its own effect runs — is what rules out the channel-sharded effect
scheduler (see [Effect scheduling](../formal/effect-scheduling.md) S.3/S.4): a sibling effect cannot be
safely reordered or run concurrently.

The current docs already gesture at the intended behavior but state it inaccurately:
[`rspace.md`](rspace.md) says "the space selects by a **sorted** ordering of the candidates" — no
implementation (live Scala, Scala replay, or Rust) does this today.

## 2. Change — content-addressed selection

Make the *choice* content-addressed, so the outcome of a comm does not depend on the order effects arrive.

- **Selection.** Sort the candidate data by `Datum.source.hash` and the candidate continuations by
  `WaitingContinuation.source.hash` before first-match; select the **sorted-first** match. Multi-channel
  joins keep the existing sorted-order channel iteration (Law 7).
- **Storage is already canonical.** The leaf serializers sort before hashing
  (`encode_datums`/`encode_continuations`/`encode_joins` in `scodec_serialize.rs`), so the trie leaf is
  already a function of the *set*, not the order. No storage change is needed.

The invariant becomes: **the datum/continuation a comm consumes is a pure function of the channel's
content**, not of the order the produces/consumes arrived.

## 3. Determinism and the concurrency it unlocks

Content-addressed selection removes the order-sensitivity of *which stored candidate* a comm consumes
(the *selection* sensitivity). It does **not** remove the *arrival-order* sensitivity (which produce
matches a waiting continuation first), nor the deeper **continuation-footprint** problem: a continuation's
effects are discovered only after its trigger runs, so an effect's *closure* can reach a
"disjoint-looking" sibling's channel. The per-channel atomicity is already present (`TwoStepLock` /
`MultiLock` in `rspace/src/concurrent/`), but it serializes conflicts — it does not make reordering sound.

Consequently the channel-sharded effect scheduler (run disjoint-channel effects concurrently) is
**unsound**, not merely "blocked" — see [Effect scheduling](../formal/effect-scheduling.md) S.3/S.4 and
the proved counterexample `Rchain.Effect.effect_reorder_diverges`.

> This document establishes only the content-addressed matching/storage precondition. It does **not**
> enable effect-level concurrency, and does not claim to.

## 4. Consensus impact

Any deploy with a multi-candidate comm (a channel with more than one matching datum or continuation at
commit time) will produce a **different post-state hash** under sorted selection than under
first-match-wins:

- the golden vectors in `rholang/testdata/differential/*.tsv` must be regenerated;
- replay is unaffected structurally (`ReplayRSpace` is trace-driven and follows the recorded COMM), but
  the recorded traces themselves change because play selects different candidates;
- if the node is already on a live chain, this is a **consensus-breaking protocol change** and must be
  activated behind a version boundary, not shipped as a transparent refactor.

## 5. Faithfulness to the Scala oracle

This is a further deviation from the Scala oracle, but a principled one:

- live Scala is **random** (`Random.shuffle`) — non-deterministic across runs;
- the Rust port is currently **deterministic first-match** (newest-first) — a deviation already;
- sorted selection is a **third deterministic rule**, consistent with the content-addressing theme of
  Laws 8 and 10, and with the "produce refs sorted" language already in Law 8.

The deviation should be recorded in `spec/AUDIT.md` §6 (Scala-deviation register).

## 6. Law mapping

- **Law 8** — "produce refs sorted; content-addressed events" is extended from the *event's* produce list
  to the *selection* itself: the candidate data/continuation lists are sorted-first by content hash.
- **Law 10** — unchanged: the radix trie's value list was already canonical (sorted by the serializers).
- **Law 4** — "first-match-wins" is unchanged in spirit; for RSpace comms the "first" is now the
  sorted-first content-addressed candidate (Law 8), not the newest insertion.

`spec/INVENTORY.md` wording for Law 8 was updated to match.

## 7. Implementation (done)

| Area | File | Change |
|---|---|---|
| Selection (data) | `rspace/src/rspace.rs` `fetch_channel_to_index_data` / `extract_produce_candidate` | sort candidate data by `datum.source` before matching (the in-flight datum stays first on its channel) |
| Selection (continuations) | `rspace/src/space_matcher.rs` `extract_first_match` | sort waiting continuations by `wc.source` before first-match |
| Replay | `rspace/src/replay_rspace.rs` | no change (trace-driven); mirrors the produce-side prepend |
| Test | `rholang/tests/execution.rs` | same-channel race case added to `concurrent_and_sequential_state_hashes_match` |

Golden vectors in `rholang/testdata/differential/execution.tsv` were unchanged (the existing deploy
vectors have no multi-candidate comms).

## 8. Rollout status

1. ~~Review + approve~~ done.
2. ~~Implement selection~~ done (storage was already canonical).
3. ~~Add the same-channel-race differential test~~ done; golden vectors unchanged.
4. ~~Update `spec/INVENTORY.md` (Law 8) and `spec/AUDIT.md` §6~~ done.
5. Activation: dev-only for now — the change alters post-state hashes for multi-candidate deploys and
   must be versioned if the node is ever on a live chain.

> **Formal.** See [The 22 laws](../formal/the-22-laws.md) (Laws 4, 7, 8, 10) and the [RSpace
> overview](rspace.md).

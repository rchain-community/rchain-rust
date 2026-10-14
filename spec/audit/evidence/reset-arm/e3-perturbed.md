# E3 — the perturbed build: the detector fires, and the anchor refuses (2026-10-13)

**What this is.** The last route for F-U5-01's arm, and the one §109/#345 recorded as the survivor: a build
whose merge differs, so one node diverges *intrinsically* — the property §110 showed no external edit could
aim. This run measures where that route actually reaches.

**The build.** A one-line, **never-committed** perturbation at `casper/src/multi_parent_casper.rs:468`:
every non-genesis block's pre-state is computed with one byte flipped, so a clean node's block fails *this*
node's `InvalidPreStateHash` — a `FailureCause::Divergence`, the one class the restore budget spends.
Genesis does not take that path (its pre-state comes from config), so the perturbed node still joins and
proves the perturbation is surgical. The tree was reverted after the build; the image is `rnode:drill`, the
other three nodes `rnode:local`.

```
RNODE_IMAGE_devnet_validator_1=rnode:drill \
DEVNET_EXTRA_FLAGS_devnet_validator_{1,2,3}="--self-reset" \
  tools/devnet.sh up --validators 4 --fresh
```

**What happened — the perturbation works, the detector fires, and the act is refused.**

| node | height | `unrestorableRecords` | `resetMarker` |
|---|---|---|---|
| bootstrap | 99 | 0 | — |
| **v1 (perturbed)** | **1** | **78** | **—** |
| v2 | 99 | 0 | — |
| v3 | 99 | 0 | — |

v1 sat at height 1 while the clean nodes ran to 99: it refused **every** clean block, which is the
divergence the perturbation was for. And the trigger's two halves split exactly:

```
96 ×  cannot restore block 0f534081… (seq 2): its revalidation budget is spent after 3 attempt(s) and it
      still fails against this node's DAG, so this node refuses it and its descendants for the process
      lifetime unless a reset follows (AUDIT C249; #320)
```

— the **detector fired 96 times**: a genuine, repeated `Divergence`, three revalidations spent, the request
raised. And then:

```
could not name a safe state: the 0 block(s) this node holds above genesis would have to be replayed from it
```

— the **`RefuseNoAnchor` arm**. v1 diverged *from height 1*, so it never finalised anything, so
`rewind_plan` named no anchor and `note_unrestorable_if_exhausted` withheld the reset — **correctly**: a node
with nothing finalised above genesis has nowhere to rewind *to*, and the code's own doc says the request is
raised "only where the rewind has somewhere to go".

**The finding, and it completes §110 rather than repeating it.** The trigger needs **three** things
together: a genuine repeated `Divergence`, a spent budget, **and a finalised anchor with a height**. The
external edits (§110) could not hold the second; a *whole-build* perturbation cannot produce the third —
the sharper the perturbation, the less the node finalises, and a perturbation sharp enough to be reliably
refused is one that refuses from the earliest block. So the act is reachable only by a node that diverged
**after finalising something** — a *partial* view divergence — which is the same target §110 showed no
instrument can aim, reached from the other side.

**What this closes.** The perturbed build **does not** reach the drive loop's third exit either — §109's
recorded expectation ("exercises the re-entry without satisfying the falsifier") was **too generous**: it
exercises the *detector* and stops at the anchor. F-U5-01's arm therefore has **no route at all** on this
tree, external or built, and the reason is structural rather than a gap in the tooling: the trigger's three
preconditions cannot be met simultaneously by anything that is not a genuine partial divergence.

**Where the run is kept.** The perturbed build is not in the tree (the line was reverted); what is
committed is this reading, and the `reset-drill` instrument beside it for the tamper route it serves.

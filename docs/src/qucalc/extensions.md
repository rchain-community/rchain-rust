# Using the QuCalc extensions

QuCalc adds eight system processes to the node. They are called exactly like any
other rholo system process — you `new` the URN into scope, send it your
arguments and a return channel, and receive the answer on that channel.

```rholang
new zfa(`rho:qucalc:zfa`), ret in {
  zfa!([0, 1], *ret) |
  for (@result <- ret) { Nil }     //  result == (true, -1)
}
```

Nothing here needs a wallet, an oracle, or an off-chain service. The processes
are gas-metered and replay-deterministic like the rest of the node, so a tally
or a ZFA verdict is reproduced identically by every validator.

- [`rho:qucalc:*` — proofs and capabilities](#rhoqucalc--proofs-and-capabilities)
- [`rho:gov:*` — group decisions](#rhogov--group-decisions)
- [Worked example: a proof that outlives its deploy](#worked-example-a-proof-that-outlives-its-deploy)
- [Worked example: a delegated, trust-weighted vote](#worked-example-a-delegated-trust-weighted-vote)
- [The rholang library layer](#the-rholang-library-layer)
- [Reaching the extensions from a browser](#reaching-the-extensions-from-a-browser)
- [Running the examples](#running-the-examples)

## Conventions

**Twists.** A *twist history* is a sequence of values `0..7` — the eight-symbol
alphabet `^ v > < / \ + -`. Every process that takes a history takes a **list of
numbers** (`[0, 1]`), one value per twist. The symbol names are notation for reading
a history, not a syntax the system processes accept: the node parses a `RhoList` of
`RhoNumber` and nothing else, so `"^v"` is a deploy error rather than a history. (The
`qucalc` library carries `from_symbols`/`to_symbols` for converting between the two
spellings; the node does not call them.)

| value | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| symbol | `^` | `v` | `>` | `<` | `/` | `\` | `+` | `-` |
| parity | pos | neg | pos | neg | pos | neg | pos | neg |

**Member ids.** Every `rho:gov:*` argument that names a member accepts either a
plain string or a `deployerId` unforgeable, which is canonicalized to the base16
encoding of its public key. Using `*deployerId` is what binds an action to its
signer: a member can set their own delegate, rating, censure or ballot, and
cannot forge another's.

**Return values.** Each process takes a return channel as its last argument and
produces exactly one value on it. A process that cannot honour a **well-formed**
request produces `Nil` rather than failing the deploy — not-ZFA, no winner, an
unknown URI — so check for it.

**Malformed arguments are a different outcome, and the difference matters.** An
argument of the wrong shape fails the deploy with an error; it does not return
`Nil`. A caller that treats every `Nil` as a refusal will still have to handle a
deploy that failed outright.

## `rho:qucalc:*` — proofs and capabilities

### `zfa(history, ret)`

Verify a twist history is a half-spin ZFA closure. Returns the tuple
`(isZfa, phase)`, where `phase` is the scalar the history folds to, encoded as
an integer. Arithmetic is exact over `{−1, 0, 1}` — never floating point — which
is what makes the predicate safe to replay.

```rholang
new zfa(`rho:qucalc:zfa`), ret in {
  zfa!([0, 1], *ret) |                      // "^v"
  for (@(ok, phase) <- ret) {
    // ok == true, phase == -1  (σ_y·−σ_y = −I, the scalar −I)
    Nil
  }
}
```

The phase codes are:

| scalar | `+I` | `−I` | `+iI` | `−iI` |
|---|---|---|---|---|
| code | `1` | `-1` | `2` | `-2` |

A history that is not closed returns `false`, and its phase is meaningless.

### `grant(history, ret)`

Mint a ZFA-balanced history as a **capability**: a content-addressed registry
URI whose stored value is the history. Returns the URI, or `Nil` if the history
is not ZFA-closed — `grant` refuses to mint a proof of something unproven.

```rholang
new grant(`rho:qucalc:grant`), ret in {
  grant!([0, 1], *ret) |
  for (@uri <- ret) {
    // uri : the minted capability, e.g. `rho:id:…`
    Nil
  }
}
```

Because the URI is derived from the hash of the history, the same history always
mints the same capability. The value persists in the registry, so it outlives
the deploy that created it.

### `verify(uri, ret)`

Re-check a previously granted capability, in a *later* deploy. Returns `true`
only if the URI resolves in the registry and its stored history is still
ZFA-closed; `false` for an unknown URI.

```rholang
new verify(`rho:qucalc:verify`), ret in {
  verify!(`rho:id:…`, *ret) |
  for (@ok <- ret) { Nil }
}
```

This pair is the point of the extension: `grant` in one deploy, `verify` in
another, with no shared state but the registry.

### `fuse(subject, predicate, ret)`

Dialectical synthesis. Two histories are fused through their shared middle term;
if the result is a stable closure, it is minted like `grant` and returned as
`(geometry, uri)` — the synthesized history and its capability. If the fusion
does not close, returns `Nil`.

```rholang
new fuse(`rho:qucalc:fuse`), ret in {
  fuse!([0, 1], [2, 3], *ret) |          // "^v" and "><"
  for (@out <- ret) {
    match out {
      Nil            => { /* thesis and antithesis did not synthesize */ }
      (geometry, uri) => { /* the fluxoid, and its capability */ }
    }
  }
}
```

This is the primitive behind [`syllogism.rho`](../../../qucalc/examples/syllogism.rho):
two peers each name a premise, fuse them through the shared middle term, and the
conclusion seals as a capability neither could have forged alone.

## `rho:gov:*` — group decisions

These four are **pure functions**. They read no state and write none: you hand
them the group's signed facts and they fold them deterministically. Persisting
those facts is the caller's job, which is why they compose with any membership
model.

### `resolveWeights(directVoters, delegations, trust, ret)`

Liquid democracy. Returns `Map<directVoter, weight>`.

| argument | shape |
|---|---|
| `directVoters` | list of member ids — those who voted themselves |
| `delegations` | map of member id → member id |
| `trust` | map of member id → integer base weight |

A member who votes counts for themselves; a member who does not has their weight
flow along the delegation edge to whoever ultimately did vote. Cycles and
dead ends abstain rather than dividing by zero.

```rholang
new resolve(`rho:gov:resolveWeights`), ret in {
  resolve!(["alice"], {"bob": "alice", "carol": "bob"}, {}, *ret) |
  for (@weights <- ret) {
    // {"alice": 3} — bob delegated to alice, carol through bob
    Nil
  }
}
```

Voting *is* the per-issue override: no separate revocation step is needed.

### `trustLevels(ratings, admins, ret)`

The admin-rooted web of trust, as a least fixed point. Returns
`Map<member, level>`.

| argument | shape |
|---|---|
| `ratings` | map of rater → (map of ratee → level) |
| `admins` | list of member ids, seeded at the maximum level |

A rating confers *strictly below* the rater's own level, so two unvouched
members cannot bootstrap each other, and a forged high rating is capped rather
than honoured.

### `censure(censures, levels, vouchers, ret)`

Accountability. Returns `(discredited, newLevels)`.

| argument | shape |
|---|---|
| `censures` | map of censurer → (map of target → 1) |
| `levels` | map of member → level, as from `trustLevels` |
| `vouchers` | map of member → list of members who vouched for them |

A target is discredited when a **⅔ quorum of eligible censurers** (floored at 2)
names them, at which point every voucher is slashed by the level they staked. No
single member — an admin included — can discredit alone, and no single admin can
block a quorum.

### `tally(ballots, weights, mode, ret)`

Weighted tally. `mode` is `"ranked"` (instant-runoff) or `"approval"`. Returns
the winning option string, or `Nil` when there are no ballots.

```rholang
new tally(`rho:gov:tally`), ret in {
  tally!(
    {"alice": ["pizza", "tacos"], "bob": ["tacos"]},
    {"alice": 3, "bob": 1},
    "ranked",
    *ret
  ) |
  for (@winner <- ret) { Nil }     // "pizza" — 3 of 4 continuing, an outright majority
}
```

Feed it the map from `resolveWeights` and the tally is delegation-weighted; feed
it weights from `trustLevels` and it is trust-weighted; feed it `{}` and every
ballot counts once.

## Worked example: a proof that outlives its deploy

The `grant` → `verify` pair across two separate deploys.

**Deploy 1** — mint:

```rholang
new grant(`rho:qucalc:grant`), stdout(`rho:io:stdout`), ret in {
  grant!("^v><", *ret) |
  for (@uri <- ret) {
    match uri {
      Nil => stdout!("not ZFA-closed — nothing minted")
      _   => stdout!(["minted", uri])
    }
  }
}
```

**Deploy 2** — anyone, later, with only the URI:

```rholang
new verify(`rho:qucalc:verify`), stdout(`rho:io:stdout`), ret in {
  verify!(`rho:id:…`, *ret) |          // the uri printed above
  for (@ok <- ret) { stdout!(["verified", ok]) }
}
```

The second deploy shares no channel with the first. What carries between them is
the capability, and possessing it is the whole of the authorization.

## Worked example: a delegated, trust-weighted vote

Composing three of the governance processes in one deploy: trust decides the
base weights, delegation moves them, the tally reads the result.

```rholang
new trustLevels(`rho:gov:trustLevels`),
    resolve(`rho:gov:resolveWeights`),
    tally(`rho:gov:tally`),
    levelsCh, weightsCh, winnerCh,
    stdout(`rho:io:stdout`) in {

  // 1. Admin-rooted trust: alice is an admin and vouches for bob.
  trustLevels!({"alice": {"bob": 3}}, ["alice"], *levelsCh) |

  for (@levels <- levelsCh) {
    // 2. Carol did not vote; her weight flows to bob.
    resolve!(["alice", "bob"], {"carol": "bob"}, levels, *weightsCh) |

    for (@weights <- weightsCh) {
      // 3. Tally the ballots under those weights.
      tally!(
        {"alice": ["ship-auth", "pay-debt"], "bob": ["pay-debt"]},
        weights,
        "ranked",
        *winnerCh
      ) |
      for (@winner <- winnerCh) { stdout!(["decision", winner]) }
    }
  }
}
```

Each step is a pure fold, so every validator recomputes the same decision from
the same signed facts. [`liquid_democracy.rho`](../../../qucalc/examples/liquid_democracy.rho)
is the fuller version of this, with the envelope layer that binds each fact to
its signer.

## The rholang library layer

Calling the system processes directly is fine, but
[`qucalc.rho`](../../../qucalc/rholang/qucalc.rho) and
[`gov.rho`](../../../qucalc/rholang/gov.rho) wrap them behind a contract so a
deploy reads as one verb per line:

```rholang
contract QuCalc(@"zfa",    @twists, ret)                    = { … }
contract QuCalc(@"grant",  @twists, ret)                    = { … }
contract QuCalc(@"verify", @cap, ret)                       = { … }
contract QuCalc(@"fuse",   @subject, @predicate, ret)       = { … }
contract QuCalc(@"ratify", @proposal, @twists, @nonce, ret) = { … }
```

`ratify` has no system-process equivalent — it is composed: grant the history,
then bind the resulting capability to a proposal under a nonce. That is the
pattern to copy when adding a verb of your own; prefer composing in rholang over
adding a system process, since a system process must be installed by the node.

The capability-facet stores — [`Directory.rho`](../../../qucalc/rholang/Directory.rho),
[`Inbox.rho`](../../../qucalc/rholang/Inbox.rho),
[`Chat.rho`](../../../qucalc/rholang/Chat.rho) — are ports of the rgov contracts
and compose with the above without knowing anything about ZFA.

## Reaching the extensions from a browser

[quantum-os](https://github.com/rhovision-org/quantum-os) drives these
extensions from a browser peer with its `/global` command, under a zero-trust
split: a headless room agent expands a macro into rholang and posts it to the
room, and the browser lints, signs and deploys it. **The signing key never
reaches the agent** — a compromised agent can post misleading text into a chat
room and nothing more.

```
/global
new ret in {
  %ballot("Q4 budget", ["ship auth", "pay down debt"]) |
  %directory("Q4 notes")
}
```

Macro call sites are written `%name(…)` and expand in place inside an ordinary
rholang program; everything else is passed through untouched. The macro registry
lives in
[`packages/browser/src/global-macros.js`](https://github.com/rhovision-org/quantum-os/blob/main/packages/browser/src/global-macros.js),
shared by both halves so the rholang a user reviews in chat is the rholang their
browser signs.

Two things that surface are worth stating plainly, because they are easy to
assume otherwise:

- The macro expander does **not** restrict which rholang you may write, and the
  linter checks only that the expansion is well-formed. What a deploy can reach
  is decided by the unforgeable names it holds — capability security, not a
  denylist of identifiers.
- The browser currently signs with **ECDSA P-256** (Web Crypto), where RChain
  deploys require secp256k1. It is a working placeholder for the pipeline shape;
  swap it before deploying anything to a real network.

## Running the examples

The rholang examples run end-to-end through the real `RhoRuntime` — parse,
normalize, reduce:

```bash
cargo test -p rholang --test rho_examples
```

The Rust coprocessor demo:

```bash
cargo run -p qucalc --example ai_coprocessor
```

A local devnet, to deploy against a running node:

```bash
tools/devnet.sh
```

See [`examples.md`](examples.md) for what each example demonstrates and which
quantum-os demo it mirrors.

# The public testnet — `testnet.rhobot.net`

A small public RChain testnet running this codebase, used by the Rholang playground and the quantum-os
room agents. Two hosts — a genesis master that holds the bond, and a joining node that can be bonded into
the pool — funded dev wallets, and an **idle** chain that produces a block only when a deploy arrives.

The **generalised** procedure — standing up a testnet of your own, from the stake split to a rebuild —
is [Running a public testnet](running-a-public-testnet.md). This page is the concrete instance: the live
hosts, the genesis, the wallets, and the incident record.

> **Status: reads, writes and the single-validator shape all work — re-verified on the current chain
> on 2026-09-29** (evidence in [Status](#status-of-the-verified-path); the chain was rebuilt that day and
> on 2026-09-27 before it, so the older transcripts there are labelled as such). A brand-new key can be funded by transfer, deploy,
> be trusted, and bond into the validator pool — **but do not add one to this net yet**: see
> [Do not onboard a validator yet](#do-not-onboard-a-validator-yet). `/api/status`, `/api/explore-deploy`, `getBonds`,
> `getActiveValidators`, `/health`, and `rnode deploy` with no extra flags all work — note that a CLI
> deploy does need a **funded** key, or it is accepted and mined and then reports `processedWithError`
> for phlo. The chain is
> deliberately **idle** (no `--autopropose`): blocks appear when a deploy arrives. A
> continuously-producing chain cannot be restarted on a 1 GB host — that is what broke the previous
> chain, and it is [K7](#known-issues), the most important operational constraint here. Not
> production, holds no value, and its chain can be reset at any time.

---

# Part 1 — For users

## What it is

| | |
|---|---|
| Chain | `testnet` network id, shard `/root`, genesis `9f09e7a0…81dc` |
| Validators | genesis signed for **one** — A (`0410b8c5…`, stake **1000**). B (`04675f16…`) runs as a plain **observer**: it carried a 100 bond on the 2026-09-27 chain and withdrew from it, and the 2026-09-29 rebuild gave it no bond at all, so the pool and the active set are A alone. **No validator is to be added — see below.** |
| Hosts | A `164.90.140.144` (private `10.108.0.3`), B `104.131.176.164` (private `10.108.0.4`) |
| Cost | 2 × DigitalOcean `s-1vcpu-1gb`, **$12/mo** |
| Binary | rchain-rust `dev` @ `94ea0a1d2`, static musl, `sha256:d37c4068dd62…`, on both hosts |
| Endpoint | **https://testnet.rhobot.net** (nginx → node A's HTTP API) |

Short hashes in this document are the first twelve hex characters of the value they name, and each is
labelled with what it is a digest *of*: `sha256:` is `sha256sum` over the `rnode` artifact as shipped,
so it can be recomputed from the release; the bare ones are block hashes and addresses as the node
prints them. (Before the September 2026 audit the binary's digest was written unlabelled, which made it
indistinguishable from a commit and unverifiable either way.)

A's stake is 1000 against B's 100 on purpose: with no `--autopropose`, A is the only proposer, so A
must hold **more than ⅔ of the whole bond pool** (including any stake sitting in withdrawal
quarantine) or no block is ever finalised. That is what keeps A able to finalise after observers bond.

## Connect

| | |
|---|---|
| HTTP API | **https://testnet.rhobot.net** (nginx → node A's `40403`) |
| Health snapshot | `https://testnet.rhobot.net/health` — JSON, refreshed by each node every 60 s |
| Direct, if you prefer | `GET http://164.90.140.144:40403/api/status`, `POST http://164.90.140.144:40403/api/explore-deploy` (eval a read-only term) |
| Admin | `POST http://164.90.140.144:40405/api/v1/propose` — force a block (keep this one private in general) |

Ports `40400` (protocol) and `40404` (discovery) are open on both nodes; `40401`, `40403` and `40405`
are open on both for clients — what each one is for is in
[running a public testnet § Ports](running-a-public-testnet.md#ports).

## Use it from r-wallet

[r-wallet](https://rhowallet.org) can select this net directly: in the wallet's network list, choose
**RChain testnet**. Balances, transfers and deploys then go to `https://testnet.rhobot.net`, which is
node A's HTTP API.

Two entries in that list are easy to confuse, and they are different chains:

| entry in r-wallet | what it actually is |
|---|---|
| **RChain testnet** | this net — `testnet.rhobot.net` |
| **Rholang playground** | the playground node at `playground.rhobot.net` (formerly `rnodeapi.rhobot.net`) — a dev-mode chain that is *not* this one |

Until 2026-09-22 the wallet's only "testnet" entry pointed at the playground, so picking it put you on
the wrong chain, with a dead faucet link; that is fixed in
[rchain-wallet#12](https://github.com/rchain-community/rchain-wallet/pull/12).

Two practical notes:

- the wallet needs a key holding REV to deploy or transfer — fund that key's address from the **faucet**
  ([Getting REV](#getting-rev--the-faucet) below), rather than importing a funded key that belongs to
  someone else;
- it does not need a node's admin API: both testnet nodes run `--propose-on-deploy`, so the node that
  receives a deploy proposes it.

## Getting REV — the faucet

Test REV comes from a **faucet**, not from an account someone hands you. There are two doors to it, and
both sign the same `revVault` transfer.

**From r-wallet, or any client: the node's own faucet.**

```
POST https://<node>/api/faucet   {"address": "<your REV address>"}
→ {"deployId":"3045…","amount":30000000,"to":"1111…"}        # 30,000,000 drops = 0.3 REV
```

That is the endpoint r-wallet calls against whichever node it is pointed at, and it is all the wallet
needs to fund a fresh address. It is a **dev-mode** endpoint: the node must have been started with
`--dev-mode --deployer-private-key`, and it signs the transfer from that deployer's wallet. Without the
key a node answers `400 "faucet requires --dev-mode --deployer-private-key"`, and reports `faucet: false`
in its capability list.

| node | faucet |
|---|---|
| `playground.rhobot.net` (and `rnodeapi.rhobot.net`) | ✅ **works** — dev-mode plus a deployer key |
| `testnet.rhobot.net` | ❌ **by design** — that key is also the dummy-deploy injector, and this net's idle chain is load-bearing for its sizing ([K7](#known-issues)). See the room faucet below |

**In a room: `/facil faucet`.**

A quantum-os facilitator started with `--key <funded deploy key>` answers

```
/facil faucet <your REV address>     # or just /facil faucet, once it has remembered your address
```

and signs a fixed **10 REV** transfer to it. It remembers the address per peer, has no rate limit — a
faucet on a test system is meant to be asked repeatedly — and refuses to move anything if it was started
without a key. Plain English works too: `/facil ask give me some test rev` routes to the same function,
never to an LLM decision to move funds.

To put that faucet on **this** net, give a facilitator a funded testnet key and point it here; it will
also answer over HTTP, in the same wire shape as the node's own endpoint:

```sh
node scripts/qos-cli/agent.mjs --room <room> --role facilitator \
  --rnode https://testnet.rhobot.net --key <a funded key on this net> --faucet-http 8080
```

**The one thing the faucet needs is your REV address**: `/rholang key show` prints it in the playground,
and r-wallet shows the address of the key you hold. Test REV holds no value — never send a faucet an
address whose key you care about.

## What works today

| | |
|---|---|
| `eval` / `explore-deploy` | ✅ works — the full Rholang/QLF macro surface |
| Reads of chain state (`getBonds`, `getActiveValidators`) | ✅ works |
| `GET /api/status` | ✅ works |
| `/health` monitoring snapshot | ✅ works |
| **Deploys — browser, room agents, `rnode deploy` CLI** | ✅ works (the CLI needed `--valid-after-block-number` before the 2026-09-21 binary — K1) |
| Transfers, including funding a brand-new key | ✅ verified: a fresh key's balance went `0` → `100000000000`, and it could then deploy |
| **Becoming a validator** | ✅ verified end to end: `trust` → `(true)`, `bond` → `(true)`, bond pool 2 → 3, active set 3 |

## Monitoring

`https://testnet.rhobot.net/health` is refreshed every 60 seconds by each node and answers with
that node's snapshot (node A's, since nginx fronts A):

```json
{ "host": "testnet-a", "ok": true, "rnode_unit": "active", "api_reachable": true,
  "blocks": 6, "blocks_since_last_tick": 0, "peers": 1, "nodes": 1,
  "finalized_fringe": false, "autopropose": false,
  "mem_available_mb": 627, "disk_free_mb": 22665 }
```

`ok: false` means the unit is down, the API is unreachable, or the height is zero. **`finalized_fringe:
false` is expected here and is not a failure** — this chain is idle, and nothing is finalised while no
blocks are produced. Joining nodes still sync: they restore from the *approved genesis* fringe, which
was verified twice with node B. DigitalOcean's dashboard also graphs CPU/RAM/disk for both hosts
(`do-agent`).

## Limits to expect

- **Idle chain, by design.** No `--autopropose` and no injected dummy deploy: a block is produced
  when a deploy arrives (`--propose-on-deploy`). Heights stay put while nobody is doing anything.
- **A rebuild resets everything.** Changing `bonds.txt`/`wallets.txt` means a new genesis and a new
  chain.
- Disk now grows with real usage only (~6.6 KB per block) instead of ~140 MB/day.
- **Start-up is the expensive part.** Replay costs about **0.25 MB of RAM and ~0.2 s per existing
  block** before the API opens at all (a 1142-block chain: ~285 MB, ~3.5 minutes), while *producing*
  blocks is nearly free. Budget ≥2 GB for ~1k blocks, ≥4 GB to be comfortable — see
  [K7](#known-issues) and [rchain-rust#60](https://github.com/rchain-community/rchain-rust/issues/60).
- No SLA, no backups of chain state beyond the genesis files.

---

# Part 2 — For maintainers

## Related pages

Where this page and the generalised ones overlap, prefer those: this one is the record for **this** net —
our hosts, keys, genesis, health checks and incident log (K1–K7).

| page | what lives there |
|---|---|
| [Running a public testnet](running-a-public-testnet.md) | the generalised procedure: stake split, genesis ceremony, joining, admitting a validator to a running chain, monitoring, sizing, rebuilds, and the port map |
| [Operating the node](operating.md#deploying-and-block-production) | `--shard-id`, the deploy-anchor rule, reading a term's return value, the block production modes, finality and withdraw behaviour |
| [Running a validator: hardware requirements](validator-requirements.md) | host sizing, including the start-up replay floor (K7) |
| [#60](https://github.com/rchain-community/rchain-rust/issues/60) / [#68](https://github.com/rchain-community/rchain-rust/issues/68) | the start-up replay: measurements, reproductions, and what is still unattributed |
| [#39](https://github.com/rchain-community/rchain-rust/issues/39) | the validator lifecycle; this net's verified transcript is posted there |

The deploy-anchor fix (K1) reached `dev` as
[#58](https://github.com/rchain-community/rchain-rust/pull/58), and the node docs above came in as
[#61](https://github.com/rchain-community/rchain-rust/pull/61).

## Topology

```
testnet.rhobot.net ──► node A 164.90.140.144 (10.108.0.3)   genesis master, stake 1000
                        └─ nginx + Let's Encrypt (cert to 2026-12-20), /health from a timer
                        └─ rnode: -s --dev-mode --propose-on-deploy --attest-on-new-blocks
                       node B 104.131.176.164 (10.108.0.4)   joining node; bonded at genesis, withdrawn
                        └─ rnode: --dev-mode --propose-on-deploy --attest-on-new-blocks
                           --bootstrap A  (no -s)             at the first boundary (2026-09-27)
```

Both live in the `default-nyc3` VPC, the same one as rhobot-2, so they can also talk over
private addresses (`10.108.0.0/20`).

**Why 1000/100, and why no `--autopropose`.** Both were learned by breaking it:

1. **Finality needs >⅔ of the whole pool, and A is the only proposer.** At 300/100 the chain
   finalised fine — right up until an observer bonded 100, which took A to 60% and stopped finality
   dead. `withdraw` is not an immediate escape either: the stake is escrowed until the quarantine
   deadline, so it goes on diluting the pool. 1000 tolerates about four joiners at stake 100.
2. **The previous chain outgrew the host.** It ran `--autopropose` plus an injected dummy deploy,
   about one block every 2.5 s. Start-up replay costs roughly **0.25 MB and ~0.2 s per existing block**
   before the API opens at all, so by ~1140 blocks every restart needed ~285 MB plus minutes of silence,
   on a 957 MB host that was also running nginx and do-agent: the kernel OOM-killed rnode, the next start
   replayed the same DAG and died again, and the API never came up (K7 — the "unresponsive API" symptom,
   which is *not* the injector). Omitting `--autopropose` and `--deployer-private-key` makes blocks arrive
   only when deploys do, keeping restart cost proportional to real usage. Generalised sizing guidance is in
   [Running a validator: hardware requirements](validator-requirements.md).

## Genesis

Built once with `scripts/localnet/keys.mjs`; the exact files are on each node:

```
/var/lib/rnode/genesis/bonds.txt    2 lines: <65-byte pubkey> <stake>  (A 1000, B 100)
/var/lib/rnode/genesis/wallets.txt  4 funded REV addresses (the dev keys)
/etc/rnode/validator.key            that node's validator key (0600 rnode:rnode)
/etc/rnode/deployer.env             DEPLOYER_PRIVATE_KEY=… (kept on disk, now unused: no injector)
```

Genesis hash `9f09e7a02d17ef41e40dd3b170dc67c73b9e61b493883bf8b86caa356b3981dc`; A's node id
`875f523b412b4a2a40e2f47bb550aa24ddb1a822`, B's `2129c7448c1d67da2f8d85f0d88c276ea711bdce`.

The genesis hash depends only on the genesis *inputs* — bonds, wallets, parameters **and the genesis
content itself** — not on either node's identity, so it is stable across rebuilds but changes when any of
those change: the 2026-09-26 rebuilds that signed for one validator all produced `e525129d…`, adding B's
bond moved it to `6a6db0db…`, and the 2026-09-29 rebuild — one bond *and* the content change below —
produced `9f09e7a0…`. **The old single-bond hash is not reachable again**: #71's fix moved the content, so
the same bond set no longer gives the same block.

**The content half of that list was missing until 2026-09-28, and it is the half that bites.** The
blessed contract set and the governance deploys are genesis *state*, so a change to any of them moves the
hash exactly as a bond change does — and unlike a bond change, nothing about it is obvious to a node
operator. #71's fix is the worked example: publishing the master directory's grant capability (see
[`spec/GENESIS.md`](https://github.com/rchain-community/rchain-rust/blob/dev/spec/GENESIS.md)) adds a
registered capability and a native registry entry, so **every chain built from that commit onward has a
different genesis hash, and the `e525129d…`/`6a6db0db…` hashes above name chains built before it.** An existing net is
untouched — its genesis is already committed history — but a rebuilt data directory is a *different*
chain, and a node pointed at the old bootstrap will not join it. Any change under
`casper/src/genesis/` belongs on the hard-fork tracker before it lands for this reason.

A node id is **not** derived from the validator key — a rebuilt data directory gets a fresh node
identity, so any `--bootstrap` URI pointing at the master has to be updated after a rebuild. The
2026-09-26 rebuild with the dependency-bump binary is the most recent instance: A's id changed, so B's
`--bootstrap` was retargeted, and genesis artefacts are produced once, at genesis.

Both nodes start with `--pos-multi-sig-public-keys <dave's pubkey> --pos-multi-sig-quorum 1`, which
puts **dave** — a `wallets.txt`-funded key that can actually pay phlo — into the trusted set at
genesis. That is what makes live admission possible (K6): dave is the key that can `trust` others.
Give the same list to every node, or a joiner's own view of the genesis PoS spec will not match the
chain it is joining.

Bond parameters come from defaults: `--bond-minimum 1`, `--bond-maximum 100`,
`number_of_active_validators 10`. **10 is larger than the bond pool**, so every properly bonded
validator is active — no top-N truncation to reason about.

`--validator-private-key-path` (a file, not a flag value) works because the fix merged
2026-09-21; on older binaries it is silently ignored and the key must be passed inline.

### Genesis wallets

`wallets.txt` funds the standard dev keys from `scripts/localnet/pk.txt` with 1,000,000,000,000 each, so
tooling already wired to them works unchanged, and so a facilitator can be handed a deploy key that has
REV to give away:

| key | REV address |
|---|---|
| `deployer` (`3554e876…`) | the facilitator faucet's key |
| `dave` (`7707a3e0…`) | `1111pJu4TJaJDNJDTinnftr2fcHvMfnDeTRXRzwgPfwuKmGMa5juj` |
| `alice`, `bob`, `carol` | see `wallet.txt` |

Throwaway development keys, published on purpose. Never use them for anything real. Users are not sent
here — they get REV from the faucet; this table is the answer to "which address funds them".

## Operating the nodes

```bash
systemctl {status,restart,log} rnode            # the node
systemctl list-timers rnode-health.timer        # monitoring
journalctl -t rnode-health -n 20                # health warnings only
```

Rebuild / re-key (the whole network):

```bash
# on the master
rnode --profile docker run -s --dev-mode --propose-on-deploy --no-upnp --host <ip> \
  --data-dir /var/lib/rnode \
  --bonds-file /var/lib/rnode/genesis/bonds.txt \
  --wallets-file /var/lib/rnode/genesis/wallets.txt \
  --pos-multi-sig-public-keys <dave pubkey> --pos-multi-sig-quorum 1 \
  --validator-private-key-path /etc/rnode/validator.key
# a joining validator: same flags minus -s, plus --bootstrap, and its own validator key
```

There is **no `--no-autopropose` flag** — you omit `--autopropose`. (`tools/devnet.sh` accepts
`--no-autopropose` because that is *its* CLI; it only omits the node flag.) Passing it makes the
node exit 1 in a restart loop. The production-mode matrix and the finality consequences of an idle chain
are in [Operating the node](operating.md#block-production-modes).

## Adding a node (observer)

An observer is any node without a bonded key. It replicates the chain and can be started
anywhere:

```bash
rnode --profile docker run --host <its-ip> --data-dir /var/lib/rnode \
  --pos-multi-sig-public-keys <dave pubkey> --pos-multi-sig-quorum 1 \
  --bootstrap rnode://875f523b412b4a2a40e2f47bb550aa24ddb1a822@164.90.140.144?protocol=40400&discovery=40404
```

(The multi-sig flags must match the genesis master's, or the joiner's own view of the genesis PoS
spec will not match — see [Genesis](#genesis).)

Success looks like this in the log — note the LFS step, which restores from the **approved genesis**
fringe. That is why joining works even though this idle chain has no *last-finalised* fringe:

```
INFO [casper.engine.NodeSyncing] Blocks for approved state added to DAG.
INFO [casper.engine.NodeSyncing] LFS state is successfully restored.
INFO [casper.engine.NodeLaunch] Making a transition to Running state.
```

A join takes about 15 seconds and ~19 MB, measured.

## Do not onboard a validator yet

**This net takes one validator on purpose, and adding a second is unsafe today.** Three blockers, all
measured:

1. ~~**Three validators panic at the first epoch boundary.**~~ **Fixed, 2026-09-28.** With bonds
   A 100 / B 100 / C 50 and `--epoch-length 10`, all three nodes used to die in the same second at
   block 10 with `Cannot process duplicate actions on one key`
   (`rspace/src/history/instances/radix_history.rs:69`) and the chain froze. The duplicate was **two
   native actions on one key, across two accepted blocks**: at a boundary every proposer runs
   `close_block` and writes the same `PREFIX_POS` leaves, and the merge concatenated both blocks'
   actions into one batch. The merge now keeps one action per slot, last accepted host in ascending
   order — a `BTreeSet<Blake2b256Hash>` iteration, so every node picks the same winner
   ([#83](https://github.com/rchain-community/rchain-rust/issues/83), commit `b5e024d0c`). **Verified
   on the configuration that killed every node**: a fresh three-validator devnet at `--epoch-length 10`
   crossed heights 10, 20, 30 and 40 with all three containers healthy and no panic.
2. **A silent validator caps finality, whatever the survivors hold.** Two constraints, and the second
   is the binding one. The quorum itself is a *strict* supermajority — `sdk/src/consensus.rs:15`,
   `stake * 3 > total * 2`, with a test named `two_thirds_is_not_supermajority` — taken over the
   **active set** (`compute_bonds` reads `pos:active`, `casper/src/runtime_manager.rs:1370-1381`;
   `defaults.conf` caps that set at 100, so on a small net it equals the whole pool), and there is no
   inactivity leak, no decay and no eviction, so an absent validator's stake counts forever. The
   binding constraint was stricter still, and until 2026-09-29 it was not just arithmetic: the fringe's
   **full-partition filter** counts a candidate message only when **every validator that has seen it has
   itself seen a message from every validator of the partition** (`all_bonded`,
   `block-storage/src/dag/finalizer.rs`), and the partition was the **whole bonded set** — so one bonded
   validator that produced nothing stopped the fringe advancing regardless of the survivors' stake share.
   Measured 2026-09-29 with `--stakes 100,100,50`: the survivors at **80 %** did not resume finality. The
   partition is now the **live weight set** (the bonded validators whose latest message is within
   `LIVENESS_WINDOW` heights of the tip, `block-storage/src/dag/liveness.rs`) while the quorum stays the
   whole bonded set, so a stopped validator stops blocking the partition and a minority still cannot
   finalise alone. Attesting also *is*
   proposing: the `--attest-on-new-blocks` tap enqueues into the proposer's queue, so a validator with
   no node contributes nothing while still being counted
   ([#70](https://github.com/rchain-community/rchain-rust/issues/70)). **Confirmed from the other side
   on the live two-host net the same day:** with B stopped and A holding **91 %** of the pool, finality
   did not resume either — so the survivors' share is not the variable, the partition filter is.
3. ~~**A fresh multi-validator network never forms at all.**~~ **Fixed, 2026-09-29.** It was two
   faults, both on the same path. First, a node recorded a peer only if its *reply* to that peer's
   handshake succeeded — and a joining node dials before its own server binds, so the reply was
   refused and the peer was lost permanently (`peers: 0` against the joiner's `peers: 1`). Second,
   once the peer was registered, the joiner latched on the **genesis master's *announcement*** — an
   empty `FinalizedFringe { hashes: [] }` broadcast as genesis is created — and discarded the answer
   to its own request in silence, then "restored" nothing and ran on an empty DAG. See
   [#100](https://github.com/rchain-community/rchain-rust/issues/100); `tools/devnet.sh` also now
   waits for the bootstrap to have **committed genesis** before starting any node, which is what let
   a joiner receive that announcement in the first place. **Verified:** a fresh two-validator devnet
   syncs 27 history / 198 data items, the joiner tracks the bootstrap's height, and both finalise in
   lockstep — 264/257, 286/278, 317/309 as the chain grew.

The split is 1000 against 100 because **every bonded validator must have its message seen** for the
fringe to advance, and B is the only other participant: A's share of the active set is what decides
whether A alone can carry a quorum *when B is running*, and the 2026-09-22 incident is consistent with
the partition reading rather than the arithmetic one — A held 1000 of a 1200 active set, **above** the
threshold, and finality still stopped while the other two produced nothing. Anyone bonding on top takes
A's share down, and the recovery needs the absent validator to speak again, not merely a larger share.

**Before a validator is added: #70's recovery case has been measured, and what blocked it is now
fixed or in review.** The 2026-09-29 three-validator run (`--stakes 100,100,50 --epoch-length 10
--no-autopropose --propose-on-deploy`, recorded on
[#70](https://github.com/rchain-community/rchain-rust/issues/70)) answered all three questions. The
survivors at 80 % did **not** resume finality, for the partition reason above — since fixed: the
partition is the live weight set, and the quorum the whole bonded map. The run's own blocker was a
**joiner**: the 50-stake validator stalled at its first epoch boundary with `missing justification` and
never recovered, because a block could be in the DAG index before the store that index is built from
held it, and nothing re-queued it
([#103](https://github.com/rchain-community/rchain-rust/issues/103), fixed in #106). And #105's
live-testnet run found a third: a node that attributes one failure to a **bonded** validator's block is
estranged from its chain permanently — the height maximum skips failed justifications and
`neglected_invalid_block` refuses any block justifying a failed bonded sender
([#105](https://github.com/rchain-community/rchain-rust/issues/105), AUDIT C173, open). So the order is
C173's decision, then this measurement again, and until then add nothing to a live net: use it as a
single-proposer chain and read or deploy against A.

**A second, independent way for a live net to lose a validator:**
[#105](https://github.com/rchain-community/rchain-rust/issues/105). On 2026-09-29 the two-bond shape was
rebuilt on this net itself (genesis A 1000 / B 100, both attesting) and **node B failed five blocks with
`InvalidStateHash` and four with `InvalidBlockNumber` while A failed none**; finality froze at block 8 —
the last block B proposed — while the height reached 25, and stopping B did not restore it. The wedge
there is the height rule (`casper/src/validate.rs:221-241`), which *skips failed justifications* when it
computes the expected height, so once a node has failed one block no higher-numbered block is ever valid
to it. That is #103's theme — one unprocessable block is permanent — reached by a different mechanism,
and it is why this chain is **single-bond** rather than two-bond: the two-bond rebuild could not
finalise at all on the current binary.

## Onboarding an observer into the validator pool

The generalised procedure — the admission routes, the funding prerequisites and a verified transcript — is
in [Running a public testnet of your own](running-a-public-testnet.md).
What follows is this net's version, with the keys and addresses actually in play here.

The implementation models the full lifecycle natively (`rholang/src/native_state.rs`):

> **observer** — any key that is not bonded · **trusted** — admission into the validator
> stakeholder group; only a trusted key may bond · **bonded / pool** — a bond within
> `[minimum, maximum]`, deducted from the validator's REV vault · **active** — the consensus set:
> the top `number_of_active_validators` of the pool by stake, recomputed on every membership
> change · **withdrawing** — deactivation, stake escrowed until the quarantine deadline ·
> **removed** — `slash`/`untrust`, stake confiscated.

Bonding is done through the `rho:rchain:pos` **system process** (native methods `bond`,
`withdraw`, `trust`, `untrust`, `getBonds`, `getActiveValidators`). There is no CLI or HTTP
endpoint for it: it is a deploy. `bond` takes the *caller's own* `rho:rchain:deployerId` as an
unforgeable capability, so **a key can only bond itself** — nobody can bond it on its behalf.

`native_state.rs::bond` enforces, in order:

| check | failure string |
|---|---|
| not already in pool/active | `Public key is already bonded.` |
| **is trusted** | `Validator is not trusted: observer admission is required before bonding.` |
| `minimum ≤ stake ≤ maximum` | `Bond is less than minimum (…)` / `greater than maximum (…)` |
| `vault_balance ≥ stake` | `insufficient funds to bond … (have …)` |

### Two admission routes

**(a) At genesis** — `bonds.txt`, plus `--pos-multi-sig-public-keys` to pre-trust keys that are
not themselves bonded. `trusted` is seeded as *genesis bond keys ∪ that list*. Changing either
means a **new genesis and a new chain**.

**(b) Live, on a running chain** — a trusted key confers trust, then the newcomer bonds:

```
1. a trusted key deploys        pos!("trust", [*deployerId, "<newcomer 65-byte pubkey>".hexToBytes(), *ret])
2. the newcomer deploys         pos!("bond",  [*deployerId, <stake>, *ret])      # 1..100 here
```

Two funding prerequisites, both easy to miss and both **verified working here** (the general form, with
the reasoning, is upstream):

- the **trusting key must hold REV**, because it pays for the `trust` deploy's phlo from its own vault. A
  genesis-trusted key that is not in `wallets.txt` cannot deploy at all — which is why **dave** is the
  trusted key on this net (see K6);
- the **newcomer must hold REV ≥ stake**, because the bond is deducted from its vault.

Funding either one is an ordinary transfer. There is **no `pos` method to read a vault balance**: the
native dispatcher implements `getBonds`, `getActiveValidators`, `getTrusted`, `bond`, `withdraw`,
`trust` and `untrust` and nothing else, so `pos!("getBalance", …)` fails with
`pos: unknown method getBalance` (`rholang/src/system_processes.rs`). A balance read has to go through
the REV vault contract, or a client macro that wraps it — not `pos`. The terms this net uses are below.

### The exact terms

Every term must bind the names it uses — a raw deploy/eval does **not** get `return` for free
(the browser and macro path adds it via `wrapProgram`, which merges
`new return, stdout(\`rho:io:stdout\`), …zfa/grant/verify/fuse… in { … }`). Omitting it fails
with `Top level free variables are not allowed`.

```
// read the pool (works today)
new return, pos(`rho:rchain:pos`), ret in {
  pos!("getBonds", [*ret]) | for (@b <- ret) { return!(b) }
}
// → {"expr":[{"ExprMap":[["0410b8c5…0c3c73",{"ExprInt":1000}],["04675f16…514404",{"ExprInt":100}]]}]}

// read the consensus set (works today)
new return, pos(`rho:rchain:pos`), ret in {
  pos!("getActiveValidators", [*ret]) | for (@v <- ret) { return!(v) }
}
// → {"expr":[{"ExprSet":[{"ExprBytes":"0410b8c5…"},{"ExprBytes":"04675f16…"}]}]}

// confer trust on a newcomer (deploy signed by a trusted, funded key)
new return, pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("trust", [*deployerId, "<65-byte hex pubkey>".hexToBytes(), *ret]) |
  for (@r <- ret) { return!(r) }
}

// bond yourself (deploy signed by the newcomer; stake 1..100)
new return, pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("bond", *deployerId, 100, *ret) | for (@r <- ret) { return!(r) }
}

// withdraw (a request: the bond leaves the pool at the next epoch boundary, then is escrowed)
new return, pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("withdraw", *deployerId, *ret) | for (@r <- ret) { return!(r) }
}
```

Deploy them with:

```bash
rnode --profile docker deploy --phlo-limit 90000 --phlo-price 1 --shard-id /root \
  --private-key <hex> term.rho
rnode --profile docker deploy-status --deploy-signature <deployId>
```

Two easy-to-miss details:

- `--shard-id /root`, or the node answers
  `Deploy shardId '' is not a member of this node's shards: [/root]`;
- `--valid-after-block-number <current height>` **only on a binary built before 2026-09-21** — the
  current testnet binary is anchored at the node's height automatically (K1). If `deploy-status`
  answers `notProcessed / Unknown`, the deploy was swept from the pool as expired.

### Status of the verified path

#### Current chain — rebuilt 2026-09-29, single-bond

A `875f523b…`, B `2129c744…`, genesis `9f09e7a0…`, binary `sha256:d37c4068dd62…` (`dev` @ `94ea0a1d2`).
Genesis signed for **one** validator (A, stake 1000) and B runs as a plain observer with no bond. Height 8,
finalised block 4, `GET /api/status` → `peers: 1`, `/health` → `ok: true`, `api_reachable: true`,
`finalized_fringe: true`, and neither node has failed a block.

This is the third shape of the day, and the reason is worth keeping: the 2026-09-27 chain had two bonds
with B withdrawn; the 2026-09-29 rebuild went back to two bonds to make the #70 measurement on this net,
and it **could not finalise with two bonded validators at all** —
[#105](https://github.com/rchain-community/rchain-rust/issues/105) — so the bond was dropped and
the chain rebuilt single-bond, which finalises normally. The 2026-09-26 transcript below is kept as
history.

#### Re-verified on the current chain — 2026-09-26, genesis `6a6db0db…`, height 0 → 23

The chain was rebuilt on 2026-09-26 with the dependency-bump binary (`2f9cee7d3c92…`), this time with
**two bonds at genesis** (A 1000, B 100) and `--attest-on-new-blocks` on **both** nodes, and the whole
validator cycle was run against it. Six deploys produced blocks 1–23.

| step | result |
|---|---|
| `getBonds` / `getActiveValidators` after the rebuild | ✅ A 1000 + B 100, both active; `getTrusted` = {A, B, dave} |
| a deploy submitted to A | ✅ block proposed by A (`0410b8c5…`) |
| a deploy submitted to B | ✅ block proposed by B (`04675f16…`) — **both validators propose**, alternating |
| finality | ✅ `GET /api/last-finalized-block` returns a block both nodes agree on (8, 12, 16 during the run) and `/health` reports `finalized_fringe: true` |
| `pos!("withdraw", …)` signed by B's validator key | ✅ `(true)` |
| `getActiveValidators` / `getBonds` immediately after the withdraw | **unchanged** — the withdrawal is a request, not a deactivation |
| the same, after the epoch boundary at block 20 | ✅ pool and active set are A alone; B's 100 sits in `withdrawers` until its deadline |
| `GET /api/status` on both, after | ✅ same height, peers 1; `/health` `ok: true` |

Two things this pinned down, both of which earlier revisions of this page had wrong:

- **`withdraw` does not deactivate immediately.** It records a *pending* withdrawal
  (`pos:pending_withdrawers`), and `close_block` moves the bond out of the pool only at an epoch
  boundary (`block % epoch_length == 0`, here every 10 blocks). On an **idle** chain that means a
  withdrawal does not take effect until something else produces a block across a boundary — a withdraw
  on a quiet net looks ignored for as long as the net stays quiet.
- **A bond-set change is safe once the fringe is non-empty.** [#73](https://github.com/rchain-community/rchain-rust/issues/73)
  records that a bond-set change while the *finalized fringe is empty* permanently wedges the chain.
  With two validators attesting, the fringe advanced and the withdrawal at block 20 was absorbed with no
  proposal failure and no bond-map disagreement — the hazard is the unfinalised case, not bond changes
  as such.

**A caution that still stands:** with `--attest-on-new-blocks` on both validators, six deploys produced
23 blocks in about two minutes. Each attestation is itself a remote block for the other node, so the
chain runs a storm until the deploys are finalised and `suppress_attestation` stops it. That is the
unbounded-attestation problem of
[#70](https://github.com/rchain-community/rchain-rust/issues/70) observed again; it is why this net is
otherwise kept to a single proposer, and why a deploy to an unbonded node is worse than useless — it is
accepted into that node's pool, and since deploys are not gossiped, nothing ever proposes it.

#### Re-verified on the current chain — 2026-09-22, genesis `aab081c7…`, height 3 → 7

The chain was rebuilt on 2026-09-22 from a fresh genesis, so the transcript further down describes the
chain *before* that rebuild. These claims were therefore re-run against the current one, through
`https://testnet.rhobot.net`:

| step | result |
|---|---|
| `GET /api/status` | ✅ node `cf360190…`, peers 1, nodes 2 |
| `getBonds` / `getActiveValidators` | ✅ `{A: 1000, B: 100, alice: 100}`, active set 3 — the key admitted below is still in the pool |
| fund a brand-new key: `getBalance` → transfer 1e11 → `getBalance` | ✅ `0` → `100000000000` (fresh key `1111PXDQTD…`) |
| the new key deploys `return!(1)` | ✅ accepted, `Success!` |
| `rnode deploy` with no `--valid-after-block-number`, funded key | ✅ `processedWithSuccess` in block 6 |
| the same with an *unfunded* key | ✅ accepted and mined in block 5, then `processedWithError` for phlo — included, not dropped |
| height during the run | ✅ 3 → 7 |

Everything in that table was run against the current chain. The transcript below was run against the
previous one and is kept because the code paths are the same — but it is **not** evidence about the
current chain, and the current chain is young (single-digit height), so treat the two separately.

#### Original transcript — previous chain, 2026-09-21

Everything below was run against **that** chain, with the deploys signed by the reference client, which
wraps each term in a result slot so its *return value* is readable (see
[Using it from quantum-os](#using-it-from-quantum-os)), and with dev's registry-lookup fix in the
running binary:

| step | result |
|---|---|
| `getBonds` / `getActiveValidators` reads | ✅ |
| fund a brand-new key: `getBalance` → transfer 1e11 → `getBalance` | ✅ `0` → `100000000000` |
| the new key deploys `return!(1)`, no flags | ✅ value `"1"` |
| **a trusted key deploys `pos!("trust", …)` for it** | ✅ value **`(true)`** |
| **the new key deploys `pos!("bond", …, 100)`** | ✅ value **`(true)`** |
| `getBonds` afterwards | ✅ grew 2 → 3 (A 1000, newcomer 100, B 100) |
| `getActiveValidators` afterwards | ✅ 3 validators |
| `pos!("withdraw", …)` | ✅ value `(true)` — a *request*; the bond leaves the pool at the next epoch boundary, then is escrowed (see the 2026-09-26 note below) |
| `rnode deploy` with no `--valid-after-block-number` | ✅ `processedWithSuccess` |

**A caution learned the hard way.** Bonding a key that has no running node still counts against
finality: when the newcomer bonded 100, A's share fell from 75% to 60% of the pool, and with no
`--autopropose` A is the only proposer, so blocks kept arriving but nothing was finalised any more
(`Finalized fringe is not available`). `withdraw` is not an instant escape either — the stake stays in
the pool until the quarantine deadline. Hence A's 1000.

## Known issues

**K1 — `rnode deploy` used to drop deploys in silence. ✅ FIXED 2026-09-21 and deployed to both nodes.**

Without the flag the CLI sent `valid_after_block_number = -1`
(`node/src/runtime/node_main.rs`: `valid_after_block_number.unwrap_or(-1)`), and a deploy is expired
once `height - valid_after_block_number > DEPLOY_LIFESPAN` (50) — enforced when the pool is swept
(`casper/src/dag.rs::expire_deploys`) and again by the proposer
(`casper/src/blocks/proposer/proposer.rs`). On any chain taller than ~49 blocks every unflagged CLI
deploy was therefore *deleted from the pool*: the node answered `Response: Success!` with a DeployId
and the deploy was then silently never proposed, which is why the proposer logged
`No pooled deploys; injecting dummy deploy for block #NNN` indefinitely. The node's own faucet
documents the rule (`node/src/api/faucet.rs`): *"must be the current chain height (not `-1`)"*.

Measured on the testnet, same term and key:

| deploy | status |
|---|---|
| no flag, before the fix (height 904) | `notProcessed / Unknown` |
| `--valid-after-block-number 904`, before the fix | `processedWithSuccess` |
| **no flag, after the fix** (height ~1060) | **`processedWithSuccess`** |

rhobot hid it: at height 8, `-1 < 8 - 50` is false, so deploys there always passed. The dummy-deploy
injector was never involved — it only fires when the pool really is empty, and it was enabled
throughout.

The fix (`fix/deploy-expiry-negative`, commit `756f1727d`) makes a negative anchor mean "not
specified" and resolves it from the node's own status, which already carries `latest_block_number` —
the same thing the faucet, the browser client, `gateway::current_height` and
`txn_coordinator::run_phase_at` do. The fix is merged to `dev` as
[#58](https://github.com/rchain-community/rchain-rust/pull/58), and both nodes run a binary that
includes it (`d37c4068dd62…`, which also carries the upstream registry-lookup fix), so `rnode deploy` works
with no extra flags. Rollbacks are kept in place as `/usr/local/bin/rnode.old-<sha>`.
**A binary built before that commit still needs `--valid-after-block-number <height>`.**

**K2 — an earlier diagnosis in this document was wrong; corrected.** It blamed the dummy-deploy
injector and claimed that removing it left A's HTTP API unresponsive. K1 shows the injector has
nothing to do with deploy inclusion, and the unresponsive-API observation is better explained by
start-up latency: a healthy restart took ~55s before `/api/status` answered, and the check that
appeared to hang was made ~25s in. The injector is now **off** — this net runs `--propose-on-deploy` with no `--autopropose`, so a block
appears when a deploy arrives and an idle chain stays idle. **Do not treat the injector as a suspect for deploy problems.**

**K3 — transfers credit a spendable vault; an earlier claim here was wrong and is retracted.**
Phlo is charged against the deployer's vault (`native_state.rs::pre_charge`, which derives the address
as `RevAddress::from_public_key(deployer)` and returns `preCharge: insufficient funds (… < …)` as a
*value* — which is exactly what `processedWithError` on a trivial deploy looks like). An earlier
revision of this document concluded from such errors that only `wallets.txt`-funded keys could ever
spend. That was an artifact of a chain that was already OOM-thrashing (K7); on a healthy chain the
same experiment gives the opposite answer:

| step | measured |
|---|---|
| a brand-new key's balance | `0` |
| transfer 1e11 to it | balance `100000000000` |
| it deploys `return!(1)` | **value `"1"`** — a transfer-funded key deploys fine |

So funding a new key works. The requirement that does bite is that the **trusted** key must be funded,
because it pays for its own `trust` deploy — and the genesis bond keys are not in `wallets.txt`. Hence
`--pos-multi-sig-public-keys <dave>` (K6).

**K4 — read deploy output from the term, not from `deploy-status`.** `stdout!(…)` from a deploy does
not reach the journal, and `deploy-status` for a failed deploy answers
`"deploy error message not available in cache or deploy executed on another node"`. The way in is the
registry result-slot pattern the browser client uses: the reference client's `deployTerm` wraps the term
so that its return value is stored and handed back. That is how the
[verified path](#status-of-the-verified-path) was measured.

An earlier revision warned that this read-back "lags by about one deploy". That was wrong: the lag was
dev's registry-lookup divergence (C18 — the native handler wrapped its reply in `(uri, value)` while
the genesis `Registry.rho` forwards it unwrapped, so a client's `for (X <- ch) { X!(…) }` silently did
nothing, with no error and no result). It is fixed in the binary these nodes run (`d37c4068dd62…`), and
with it deploy result values are readable — which is what unblocked the whole diagnosis.

**K5 — disk and memory growth.** Disk now grows only with real usage (~6.6 KB/block) since the injector
is gone. Memory is the constraint that matters, and it grows with the length of the chain rather than
with activity — see K7.

**K6 — live validator admission works. Two earlier claims here were wrong; both are retracted.**

The first said the genesis-seeded trusted set reads as empty at runtime; the second said no key on
this chain could be both trusted and able to deploy. Both came from experiments run on a chain that
was already OOM-thrashing (K7), where every deploy failed on phlo for reasons unrelated to trust.

On the rebuilt chain the whole path is verified — see
[Status of the verified path](#status-of-the-verified-path):

- a trusted key's `pos!("trust", …)` returns **`(true)`**, so `trusted` *is* seeded and readable at
  deploy time; it contains dave, admitted by `--pos-multi-sig-public-keys`;
- the newly trusted key's `pos!("bond", …, 100)` returns **`(true)`**, and both the bond pool and the
  active validator set grow;
- an untrusted key's `trust` still answers
  `(false, "Only a trusted stakeholder can admit validators.")` — correct.

The one real requirement: **the trusted key must be able to pay phlo**, so it has to be funded. The
genesis bond keys are not in `wallets.txt`, so the trusted set is seeded with dave instead, via
`--pos-multi-sig-public-keys <hex> --pos-multi-sig-quorum 1` — on every node, so a joiner's own view
of the genesis PoS spec matches the chain it is joining.

Re-verified on the 2026-09-22 rebuilt chain (`aab081c7…`): dave's `trust` returns `(true)`, alice's
`bond 100` returns `(true)`, the bond pool goes 2 → 3 and the active set 2 → 3 — and A still holds
83 % of the pool, above the ⅔ threshold, so admitting a validator does not stall finality.

**K7 — start-up replay is the expensive part of a node's life, and it is invisible while it runs.
This is the most important operational constraint here. Now tracked upstream as
[rchain-rust#60](https://github.com/rchain-community/rchain-rust/issues/60).**

Measured on the same 1142-block state, on a 4 GB host so the replay could actually finish:

| configuration | start-up RSS | API reachable after |
|---|---|---|
| fresh chain (≤6 blocks) | 18–20 MB | **15 s** |
| replay, read-only | 57 → 284 MB (oscillating) | **225 s** |
| replay, `-s --dev-mode --propose-on-deploy` | 52 → 285 MB (oscillating) | **210 s** |
| an isolated chain *producing* blocks (`--autopropose` + injector) | 20 → 23 MB while going 10 → 141 blocks | — |

Confirmed again in production on 2026-09-22: redeploying the rhobot **playground** node to pick up C21
cost **~11 minutes** before its API answered, RSS peaking above 1.4 GB on a 3.9 GB box — on a chain
that is only **16 MB** on disk. The restart is the cost, not the data, which is why a rebuild that
starts from an empty data directory (as this testnet does) comes up in seconds while a restart of the
same node does not.

So the cost is all in start-up — **roughly 0.25 MB of RSS and ~0.2 s per existing block** — while
producing blocks is nearly free (~0.02 MB/block). An earlier revision of this section said "about 1 MB
per block"; that was arithmetic from the OOM below, and the measurement does not support it.

What actually killed the previous chain: on a 957 MB host that was also running nginx, do-agent and
certbot, rnode was OOM-killed while replaying, at **738 MB `anon-rss`** — higher than the ~285 MB the
same state needs in isolation, and the extra several hundred MB is *not yet explained* (issue #60 lists
the candidates: a bonded validator identity, the running injector, a peer that is itself replaying,
concurrent LFS state transfer). After the kill, every restart replayed the same DAG and died again, in a
loop: the API never answered (`api-unreachable no-blocks`) while `systemctl is-active` still reported
`active`. That, not the injector, is the real explanation of the "removing the injector left the API
unresponsive" observation in K2.

What was done about it here, and what it means for operators:

- the rebuilt chain runs **without `--autopropose` and without `--deployer-private-key`**, so blocks
  arrive only when deploys do and restart cost tracks real usage instead of wall-clock time;
- a fresh chain starts in **15 s at 18–20 MB**, where the 1142-block chain could not restart usefully;
- the health check no longer fails on a missing finalised fringe — an idle chain has none, and joiners
  restore from the approved genesis fringe instead;
- **host sizing:** 1 GB is fine for a few hundred blocks, but budget ≥2 GB for ~1k blocks and ≥4 GB to
  be comfortable — and expect ~30 s per 150 blocks of unavailability after every restart, with no
  readiness signal until the API appears;
- if a long chain does fail to come up, check whether the PID's RSS is *growing* before assuming a hang:
  replay is silent, and restarting faster does not help.

## Using it from quantum-os

The room agents and the playground both take a node URL, so pointing them at this net is one flag:

```sh
# the qos-cli agents (facilitator, observer) talk to whichever node the room is on
node agent.mjs --rnode https://testnet.rhobot.net …

# the playground's console takes the same thing interactively
/rholang rnode https://testnet.rhobot.net
```

That is the whole integration. The agents sign with a local key and read each term's return value out of
the registry result slot — the same path the verified transcripts on this page were produced with.

## Housekeeping

- Snapshots/rollback: unit files are backed up in place (`rnode.service.bak-<epoch>`), node binaries as
  `/usr/local/bin/rnode.old-<sha>`, and each rebuild left the previous data dir as
  `/var/lib/rnode.bak-<timestamp>`. The genesis files are the source of truth and are tiny; those old
  data dirs (4.5 MB each) can be deleted once the new chain is confirmed.
- Firewall: `ufw` allows `22`, `40400`, `40401`, `40403`, `40404`, `40405`, plus `80`/`443` on A.
  Nothing else — the old rhobot box's 36-rule ruleset was pruned to what actually has listeners.
- Certificates renew via `certbot.timer` on A (nginx authenticator), first expiry 2026-12-20.
- To move the testnet to another host: copy `bonds.txt`, `wallets.txt`, the validator key and the
  static musl `rnode` binary. The binary is self-contained (no Docker, no runtime deps).

# Running a public testnet of your own

A **persistent, multi-host** network — as opposed to the local Docker [devnet](devnet.md) or the bare
topology harness (`tools/devnet.sh up --nodes N`, see [Operating the node](operating.md)). This page is
the operational walkthrough: planning the stake split, the genesis ceremony, joining, admitting a
validator to a running chain, monitoring, and the sizing trap that will bite a small host.

It is written from a live two-host testnet (two 1 GB droplets: one genesis master, one joining validator,
plus observers); that net's own page — its hosts, keys, genesis and incident log — is
[The public testnet](testnet.md), and the measurements quoted here come from it.

> **Security.** A testnet holds no value. Use throwaway keys, publish them deliberately, and expect to
> rebuild the chain at will — a rebuild resets everything.

## 1. Decide the stake split before anything else

Finality needs **more than 2/3 of the whole bond pool** to attest. On a chain that is not producing blocks
continuously, the only proposer is the validator that has something to propose — so the founding
validator's share is a correctness parameter, not a cosmetic one.

- Give the founding validator **more than 2/3 of the pool at genesis** (e.g. `1000` against `100`), so that
  observers bonding `100` each do not push it under the threshold. At `300`/`100`, a single observer
  bonding `100` took the founder to 60% and finality stopped dead — blocks kept extending the DAG and
  nothing was finalised.
- `withdraw` is not an immediate escape: it only stages the request, the validator keeps validating until
  the next epoch boundary, and the stake stays in the pool until then and **escrowed until the quarantine
  deadline** after that — so it keeps counting against 2/3 throughout (`rholang/src/native_state.rs`,
  `close_block`).
- If the ratio cannot be guaranteed (or the net must keep finalising while idle), run a second validator
  with `--autopropose`, and remember the [block production modes](operating.md#block-production-modes).

## 2. Genesis ceremony (the master)

`rnode run -s` makes a node the **genesis master**: with an empty DAG it creates the genesis block and is
bonded as its first validator. It needs a validator key and a wallets file
([standalone genesis prerequisites](operating.md#standalone-genesis-prerequisites)).

```sh
# data dir holds the two genesis files; keep them, they are the source of truth
install -d -m 755 /var/lib/rnode/genesis
echo "<65-byte uncompressed pubkey> 1000" > /var/lib/rnode/genesis/bonds.txt   # the founder, majority stake
echo "<observer pubkey> 100"              >> /var/lib/rnode/genesis/bonds.txt   # optional second founder
: > /var/lib/rnode/genesis/wallets.txt    # parsed strictly: must exist, may be empty

rnode --profile docker run -s --dev-mode --propose-on-deploy --no-upnp --host <public-ip> \
  --data-dir /var/lib/rnode \
  --bonds-file /var/lib/rnode/genesis/bonds.txt \
  --wallets-file /var/lib/rnode/genesis/wallets.txt \
  --pos-multi-sig-public-keys <funded key pubkey> --pos-multi-sig-quorum 1 \
  --validator-private-key-path /etc/rnode/validator.key
```

Two things about that command decide whether live admission will work later:

- **`--pos-multi-sig-public-keys` seeds the trusted stakeholder set.** `trusted` is the genesis bond keys
  ∪ this list (`casper/src/genesis/mod.rs`). Seeding it with a key that is also **funded** by `wallets.txt`
  is what makes live admission possible, because the trusting key pays phlo for its own `trust` deploy —
  and the genesis *bond* keys are not funded unless you fund them. `--pos-multi-sig-quorum` must be ≤ the
  number of keys (`node/src/configuration/configuration.rs`).
- **Every node must be given the same list.** A joiner reconstructs its own view of the genesis PoS spec
  from its config; if the trusted set or the bond parameters differ, its view will not match the chain it
  is joining.

Record two identifiers, because both change on a rebuild: the **genesis hash** and the **node id**
(`/api/status` → `address`; the same value is the TLS certificate's CommonName).

## 3. Joining

```sh
rnode --profile docker run --dev-mode --propose-on-deploy --no-upnp --host <its-public-ip> \
  --data-dir /var/lib/rnode \
  --bonds-file /var/lib/rnode/genesis/bonds.txt \
  --wallets-file /var/lib/rnode/genesis/wallets.txt \
  --pos-multi-sig-public-keys <same list as the master> --pos-multi-sig-quorum 1 \
  --bootstrap rnode://<master-node-id>@<master-host>?protocol=40400&discovery=40404 \
  --validator-private-key-path /etc/rnode/validator.key
```

Success looks like this, and a fresh chain does it in about **15 s and ~19 MB**:

```
INFO [casper.engine.NodeSyncing] Blocks for approved state added to DAG.
INFO [casper.engine.NodeSyncing] LFS state is successfully restored.
INFO [casper.engine.NodeLaunch] Making a transition to Running state.
```

The restore uses the **approved** fringe written by the genesis ceremony (`put_approved_block`), so joining
works even on a chain that has produced no new blocks since genesis. Note that such an idle chain has no
*last-finalised* fringe — see [Finality](operating.md#finality-and-why-an-idle-chain-can-report-no-fringe)
— so a joiner can always restore the genesis state, but has no recent fringe to sync *to*.

## 4. Admitting a validator to a running chain

**The node joins and syncs first - step 3, not this one.** A bond is only a change to the chain's state: the
deploy needs the newcomer's *public* key and nothing else, so a key whose node is not running can be trusted
and bonded successfully. But the validator set is consensus weight from the moment it lands, and two things
then work against you:

* **The node decides whether it may propose from the newest block in its *own* DAG.** One that has not
  restored the chain has a newest block that predates its bond, and is told `Proposal failed: ReadOnlyMode`
  (`ProposeStatus::NotBonded`) however bonded the chain says it is. Sync, then the bond is in its view.
* **A bonded validator that is not speaking is silent stake.** It counts in the >2/3 denominator finality
  needs, and the finalised fringe cannot advance without its latest message. Bonding a node that is down or
  still syncing therefore dilutes the quorum and can stall finality for everyone - which is what happened on
  this testnet: the two newcomers activated at a block boundary while their nodes were stopped, and the
  fringe stopped until they were up and had produced a block.

An admission tool that applies this order - preflight the newcomer's node, trust, bond, wait for the boundary,
then require the newcomer's node to propose - is
[`scripts/qos-cli/admit.mjs`](https://github.com/rchain-community/quantum-os/blob/main/scripts/qos-cli/admit.mjs).

So: start it, wait for its `LFS state is successfully restored`, confirm it is following the chain, and only
then `trust` and `bond` - and let it stay up, because an active validator that is not proposing is the case
the network cannot make progress without.

The lifecycle is native (`rholang/src/native_state.rs`): **observer** (any unbonded key) → **trusted**
(admitted to the validator stakeholder group) → **bonded** (a stake within `[minimum, maximum]`, deducted
from the caller's own vault) → **active** (the consensus set) → **withdrawing** → **removed**.

Everything is a deploy against the `rho:rchain:pos` system process — there is no CLI or HTTP endpoint for
it. `bond` takes the caller's *own* `rho:rchain:deployerId` as an unforgeable capability, so **a key can
only bond itself**; nobody can bond on someone else's behalf.

`bond` enforces, in order:

| check | failure string |
|---|---|
| not already in pool/active | `Public key is already bonded.` |
| **is trusted** | `Validator is not trusted: observer admission is required before bonding.` |
| `minimum ≤ stake ≤ maximum` | `Bond is less than minimum (…)` / `greater than maximum (…)` |
| `vault_balance ≥ stake` | `insufficient funds to bond … (have …)` |

Two routes in:

- **At genesis** — a `bonds.txt` line, and/or `--pos-multi-sig-public-keys` for keys that should be trusted
  without being bonded. Changing either means a new genesis and a new chain.
- **Live** — a trusted key confers trust, then the newcomer bonds itself.

Both need **funding**, and this is where testnets usually stall:

- the **trusting key must hold REV**, because it pays phlo for the `trust` deploy from its own vault. This
  is why the trusted set is seeded at genesis with a *funded* key rather than with the bonded validator
  keys;
- the **newcomer must hold REV ≥ stake**, because the bond is deducted from its vault.

A transfer funds either one: `revVault!("transfer", *deployerId, "<rev address>", <amount>, *ret)` — the
reply is `Nil` on success and an error string on failure. Read a balance with
`revVault!("getBalance", "<rev address>", *ret)` (the method is `getBalance`, **not** `balance`); derive a
key's address with `scripts/localnet/keys.mjs`:

```sh
node -e "import('./keys.mjs').then(m=>console.log(m.revAddressOf('<private key hex>')))"
```

Note that `rho:rchain:revVault` is a **native system process in this port**, not the genesis `RevVault.rho`
contract; its effective API is the native dispatch in `rholang/src/system_processes.rs`
(`getBalance`, `transfer`, `findOrCreate`), and `revVault` must be bound directly
(`new revVault(\`rho:rchain:revVault\`)`) — the registry shorthand table is not seeded, so
`lookup!(\`rho:rchain:revVault\`, *ch)` answers `Nil`. `spec/API-SCHEMA.md` tracks these divergences.

### The terms

Every term must bind the names it uses. A raw deploy or `eval` does **not** get `return` for free — the
browser/macro client adds it via `wrapProgram` — so a term written for the browser fails as
`Top level free variables are not allowed` when deployed directly.

```rholang
// read the pool
new return, pos(`rho:rchain:pos`), ret in {
  pos!("getBonds", [*ret]) | for (@b <- ret) { return!(b) }
}
// → {"expr":[{"ExprMap":{"data":{"0410b8c5…":{"ExprInt":{"data":1000}},"04675f16…":{"ExprInt":{"data":100}}}}}]}

// read the consensus set
new return, pos(`rho:rchain:pos`), ret in {
  pos!("getActiveValidators", [*ret]) | for (@v <- ret) { return!(v) }
}

// confer trust on a newcomer (deploy signed by a trusted, funded key)
new return, pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("trust", [*deployerId, "<65-byte hex pubkey>".hexToBytes(), *ret]) |
  for (@r <- ret) { return!(r) }
}

// bond yourself (deploy signed by the newcomer; stake within [minimum, maximum])
new return, pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("bond", [*deployerId, 100, *ret]) | for (@r <- ret) { return!(r) }
}

// withdraw (deactivates immediately; stake escrowed until the quarantine deadline)
new return, pos(`rho:rchain:pos`), deployerId(`rho:rchain:deployerId`), ret in {
  pos!("withdraw", [*deployerId, *ret]) | for (@r <- ret) { return!(r) }
}
```

Deploy them with the shard named, and read the result out of the term rather than from `deploy-status`
(both covered in [Operating the node](operating.md#rnode-deploy-and-deploy-status)):

```sh
rnode --profile docker deploy --phlo-limit 90000 --phlo-price 1 --shard-id /root \
  --private-key <hex> term.rho
rnode --profile docker deploy-status --deploy-signature <deployId>
```

### The whole path, verified

One full pass on a live two-validator net (`trusted` seeded at genesis with a funded key, as in §2):

| step | result |
|---|---|
| a new key's balance (`getBalance`) | `0` |
| transfer 1e11 to it | balance `100000000000` |
| it deploys `return!(1)`, no flags | value `"1"` |
| **a trusted key deploys `pos!("trust", …)`** | value **`(true)`** |
| **the new key deploys `pos!("bond", …, 100)`** | value **`(true)`** |
| `getBonds` / `getActiveValidators` | grew 2 → 3 / 3 validators |
| an untrusted key's `trust` | `(false, "Only a trusted stakeholder can admit validators.")` |

Read those values through the result-slot pattern (as the browser client and
`scripts/qos-cli/rholang-client.mjs` do) — `deploy-status` cannot tell you *why* a deploy failed, only
that it did.

## 5. Monitoring

Run a timer that curls each node's HTTP API and writes a snapshot — `unit`, `api_reachable`, `blocks`,
`blocks_since_last_tick`, `peers`, `finalized_fringe`, `mem_available_mb`, `disk_free_mb`. Two notes from
running this in anger:

- **A missing finalised fringe is not (necessarily) a failure.** An idle chain has none, and failing the
  check on it makes a healthy net permanently red. Log it as a note and keep it in the snapshot.
- **`systemctl is-active` is not health.** During the start-up replay described in §6 the unit reports
  `active` while the API never answers and the process burns CPU — the only signal is RSS growth on the
  PID.

## 6. Sizing: the restart trap

A node's RAM need is **not** its steady-state RSS. Restarting a chain costs roughly **0.25 MB and ~0.2 s
per existing block** in start-up replay, before the HTTP API answers at all — a ~1140-block chain needs
~285 MB and takes ~3.5 minutes, while *producing* blocks costs almost nothing. On a small host the replay
is OOM-killed, and because every restart replays the same DAG the node loops instead of recovering.

Give the net headroom for the restart, not just for the run — `≥ 2 GB` for ~1k blocks, `≥ 4 GB` to be
comfortable — and see [Running a validator: hardware requirements](validator-requirements.md) and
[#60](https://github.com/rchain-community/rchain-rust/issues/60). A chain that produces blocks
continuously (`--autopropose` plus the dev-mode injector) reaches that height in an afternoon, so it hits
the limit quickly; one that grows only with real traffic stays safe far longer.

## 7. Rebuilding, and changing the parameters

A rebuild is how you change the stake split, the trusted set, or the funded wallets — and it resets the
chain completely:

1. stop the nodes; keep `bonds.txt`, `wallets.txt` and every validator key;
2. edit the genesis files, wipe the data dirs (or move them aside as `<data-dir>.bak-<timestamp>` if you
   want a rollback), and start the master again with `-s`;
3. **re-read the genesis hash and node id** — both change — and update any `--bootstrap` URI or bookmark
   that pinned the old ones;
4. let the joiners wipe and re-bootstrap (they will restore from the new approved fringe).

Old data dirs are small (a few MB) but not load-bearing: the genesis files and the validator keys are the
source of truth.

## Ports

| Service | Default port | Reachable |
|---|---|---|
| protocol (TLS peer transport) | 40400 | public between nodes |
| gRPC API — Deploy | 40401 | public for clients |
| gRPC API — Propose / Repl | 40402 | **loopback only** |
| HTTP API | 40403 | public for clients |
| discovery (Kademlia) | 40404 | public between nodes |
| admin HTTP (`POST /api/v1/propose`) | 40405 | keep it private |

`--use-random-ports` (or explicit `--protocol-port`/`--discovery-port`/`--api-port-*`) lets a second node
run beside an existing one on the same host, which is useful for testing.

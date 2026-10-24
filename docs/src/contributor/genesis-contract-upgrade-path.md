# Upgrading a blessed contract without a new genesis (RCHIP #43)

**Status: functional alternative. No consensus change, no schedule row, no new system process, and no
decisions deferred to you.** The mechanism is already installed by the genesis; what was missing was
the procedure and the genesis-time check that keeps it usable.

## The finding

`casper/src/genesis/resources/rgov/MasterDictionary.rho` is installed at genesis and already has
everything an upgrade needs:

| property | in the dictionary |
|---|---|
| names are **rooted** in the owner's identity (`<revAddr>/<path>`), derived from the deployer id | `pathOf`, root authority |
| **versions are append-only** per path — `resolveAt(p, 3)` answers the same thing for ever | `appendVersion`, `resolveAt` |
| a **governed alias tier** holds the short names, and the genesis **already aliases the blessed contracts** to their paths | `setAlias`, `seed_rgov_class_aliases` |
| `resolveName` returns the newest version of the aliased path | `resolveName` |
| a path can be closed to further versions | `setSealed` |
| the root is rotatable **only by its holder** | `setRootAuthority` |

An upgrade is therefore **an ordinary deploy**: append a version to the aliased path. `resolveName`
returns the newest; a client that pinned `resolveAt(p, v1)` keeps v1 for ever. Nothing in consensus
changes — no header field, no new system process, no `SCHEDULE` row, and the genesis does not move.

## The procedure

1. **Append.** The root holder signs a deploy that calls `appendVersion(<root>/rgov/<name>, <value>,
   ret)`. `<value>` is the implementation's resolve URI — the code itself is a `.rho` published at a
   path, so there is no artifact store and no attachment to invent.

2. **No alias change.** The alias points at the **path**, and `resolveName` returns the newest version
   of that path, so appending *is* the cut-over. Re-point with `setAlias` only when moving the alias to
   a *different* path — which is the operation that is deliberate and visible, and that never rewrites
   what an earlier client already resolved.

3. **Optional precondition, in the same deploy.** Because a deploy is atomic, a stale upgrade can
   refuse itself: read `resolveName` (or `resolveAt`) first and abort unless it is the version the
   deployer expects. This is a deploy-time guard, not a consensus rule, and it is the cheapest thing
   that prevents a racing governance action from applying to a code base nobody reviewed.

   ```rholang
   // Sketch, not compiled here. <md> is the dictionary facet the genesis published; the root holder
   // signs this deploy, so the rooted-path check inside the contract is the authorization.
   new md, current, check, done in {
     md!("resolveName", ["rgov.rev"], *current) |
     for (@v <- current) {
       if (v == 3) {          // the version this upgrade was reviewed against
         md!("appendVersion", [<(root)/rgov/rev>, <newImpl>, *done])
       } else {
         Nil                  // a stale upgrade: do nothing, visibly
       }
     }
   }
   ```

4. **State migration is the deploy's job.** The dictionary moves *pointers*, not data. If a fix needs a
   tuple rewritten, the same deploy reads it through the old implementation and writes it through the
   new one — and it is as safe as any other deploy, because it is one.

## What this cannot do

- **A client that captured the implementation channel is not upgraded.** It holds a name, not a
  lookup. The fix is in the client (resolve per use), not in the chain — and it is the one real
  constraint on this approach, so it belongs in the client guidance rather than in a design document.
- **A sealed path cannot gain a version.** `setSealed` is permanent, so a blessed contract must not be
  sealed while it might still need a fix.
- **A contract installed outside the dictionary** — a bare name captured at genesis with no alias —
  has nothing to move. That is a genesis-output defect, not a missing mechanism.
- **It does not cover a change in the contract's *interface*.** A client calling a method that no
  longer exists fails; that is a versioned-API problem, not a registry one.

## The genesis-time check — this is the actual "last genesis" requirement

Everything above works only if the genesis leaves the blessed contracts *reachable through the alias
tier*. So before cutting the genesis, verify, per blessed contract:

1. it is reachable by a short name through the alias tier;
2. the alias points at a **path**, not at a constant or a captured channel;
3. the path is **not sealed**;
4. the root holder is an identity you can still sign with — or has been rotated to the governance
   multi-sig with `setRootAuthority`, which is itself a deploy.

Those four are checks on the genesis output. They are what turns "a fix later" into a deploy instead of
a chain replacement, and they are cheaper than any mechanism.

## Decisions deferred, with the default taken

| decision | default taken now | why it can wait |
|---|---|---|
| **authorization** | the root holder — the same key the ceremony signs with — delegable later with `setRootAuthority` | rotation is a deploy, so naming governance as the long-term holder is not a prerequisite |
| **per-contract vs batched** | both: several `appendVersion` calls in one deploy are atomic | it changes nothing about consensus or the wire |
| **where the new source lives** | the path itself: `appendVersion` stores the resolve URI, the code is published at a path | there is no artifact store to design, and no size limit to negotiate |
| **Rholang vs fixed migration** | the deploy | the dictionary imposes no migration shape, so there is nothing to choose until there is a migration to write |

## Appendix — the consensus-gated design, which is not needed

If a contract ever has to be replaced *without* clients resolving — a blessed name captured at genesis
with no alias — then the upgrade becomes a consensus change: a `SCHEDULE` row and a system deploy
carrying `expected_version` and `expected_source_hash`, applied against the block's pre-state. That is
a real mechanism (roughly what an earlier draft of this page specified), but it is the fallback, not the
first step, and it does not have to be decided before the genesis.

## What this page claims

That a blessed contract can be fixed **after** the genesis, on the running chain, as an ordinary
deploy, with no fork — and that the requirement on the genesis is therefore a **check** that each
blessed contract is aliased, unsealed and reachable, rather than a new mechanism.

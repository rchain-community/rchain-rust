import Rchain.Par
import Rchain.Match
import Rchain.Silence

/-!
# Law 41 — channel balance: a replicable reader restores what it consumes

A *store* is a channel a contract creates once and every later operation reads or writes. A linear
receive (`<-`) consumes the datum it takes, so a reader that does not put one back leaves the channel
empty: every later read then waits on a channel that will never speak again, and **nothing errors**.
That is AUDIT C22 item 1, and the concrete instance is `Inbox.rho`'s zero-argument `read` — it consumed
its own store, so after one read the inbox answered nothing, for the life of the chain, silently.

This module states the law so that "the store was consumed" is a disagreement with something written
down rather than a paragraph of post-mortem:

- `replicatedRead` — the reader as a `contract` installs it: a **replicated** receive (`Receive`'s
  persistent flag). Replication is what makes a second read possible at all, and it is what "replicable
  reader" means: the same reader is called again, so the store must be able to answer it again.
- `readStore` — one read, computed: find the datum (a send on the channel whose value matches the bind's
  pattern) and the reader; the residue is the reader's continuation, with the replicated reader left
  behind and the datum it supplied removed. This is the model's `Par` being flat: a redex is a pair of
  entries, not a subtree, so a read is a search over `sends`/`receives`.
- `storeSurvives` — the law, computed: after that read, is a datum back on the channel *and* does the
  result form a contract step (law 38's `takesStep`)? A reader that restores answers again; one that
  consumes does not, and its silence is the law's violation.

The corpus (`Rchain/Corpus.lean`'s `storeCases`, `spec/conformance/store.tsv`) carries the surface
rholang beside the model's view, and `rholang/tests/lean_store_corpus.rs` runs each term through the
node: the node must answer exactly the reads the model says the store survives.
-/

namespace Rchain

/-- A `Par` with only sends. -/
def sendsOnly (ss : List Send) : Par := Par.mk ss [] [] [] [] [] [] []

/-- A `Par` with only receives. -/
def receivesOnly (rs : List Receive) : Par := Par.mk [] rs [] [] [] [] [] []

/-- The reader a `contract` installs: a **replicated** receive on `chan` (the persistent flag), whose
body is `body` and whose bind pattern is `pattern`. Replication is the specification's word for "this
reader can be called again" — which is the whole question law 41 asks of the store it reads. -/
def replicatedRead (chan pattern body : Par) : Par :=
  receivesOnly [Receive.mk [ReceiveBind.mk [pattern] chan 1] body true 1]

/-- A bind that accepts this send: the same channel, one pattern against one datum, and the pattern
matching. Anything else fails closed (the boundary note in `Rchain/Match.lean`). -/
def bindTakes (chan : Par) (s : Send) (b : ReceiveBind) : Bool :=
  match b.patterns, s.data, stringChan s.chan, stringChan b.source, stringChan chan with
  | [pat], [d], some a, some c, some w => a == w && c == w && spatialMatch d pat
  | _, _, _, _, _ => false

/-- Find a receive with a bind that takes this send. A **replicated** receive stays behind — it is a
contract, and this read is one call of it; a linear one is consumed by the read. -/
def readStoreRecvs (chan : Par) (s : Send) : List Receive → Option Par
  | [] => none
  | r :: rs =>
    if r.binds.any (bindTakes chan s) then
      some (parMerge (if r.persistent then receivesOnly [r] else receivesOnly []) r.body)
    else readStoreRecvs chan s rs

/-- Try each send as the datum the reader takes. -/
def readStoreSends (chan : Par) : List Send → List Receive → Option Par
  | [], _ => none
  | s :: ss, rs =>
    match readStoreRecvs chan s rs with
    | some residue => some (parMerge (sendsOnly ss) residue)
    | none => readStoreSends chan ss rs

/-- One read of the store on `chan`, if the store answers: the residue of the step — the reader's
continuation, the replicated reader left behind, and the datum it supplied removed. -/
def readStore (chan : Par) (p : Par) : Option Par := readStoreSends chan p.sends p.receives

/-- Is a datum on the channel `chan`? A store answers when there is one for its reader. -/
def hasSendOn (chan : Par) (p : Par) : Bool :=
  p.sends.any (fun s =>
    match stringChan chan, stringChan s.chan with
    | some w, some c => w == c
    | _, _ => false)

/-- **Law 41.** Does the store on `chan` answer a second read after one read has happened? The datum
must be back *and* the pair must form a step (law 38's `takesStep`): a reader that restores answers
again, a reader that consumes has consumed the store for good, and "for good" is the part that used to
be unstated. -/
def storeSurvives (chan : Par) (p : Par) : Bool :=
  match readStore chan p with
  | none => false
  | some residue => hasSendOn chan residue && takesStep residue

end Rchain

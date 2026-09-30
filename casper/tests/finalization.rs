//! Finalization test (CBC-Casper, Law 14).
//!
//! The `Finalizer` only advances the fringe on a *fork* structure: a justification must reference
//! messages *beyond* the candidate next layer (`calculate_next_fringe_support_map` reads
//! `parents ∖ next_layer`), so a lockstep DAG — which the full block pipeline's `latest_msgs`
//! proposer always produces, and which the Scala `MultiParentCasperFinalizationSpec` round-robin
//! scenario built — never finalizes. That Scala spec is itself `ignore`d ("TODO consider adjusting
//! or removing when new finalizer test is implemented"). This test drives the fork structure
//! through `DagMessageState` directly and asserts the fringe advances past genesis.

use std::collections::BTreeSet;

use rchain_models::block_hash::BlockHash;
use rchain_shared::refined::NonNegI64;

#[test]
fn fork_structure_advances_fringe() {
    use rchain_block_storage::dag::message_state::DagMessageState;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, SeqNum};

    fn validator(byte: u8) -> Validator {
        Validator::new([byte; 65])
    }
    fn id(sender: u8, height: i64) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[0] = sender;
        bytes[1] = (height & 0xff) as u8;
        bytes[2] = ((height >> 8) & 0xff) as u8;
        BlockHash::new(bytes)
    }
    fn h(n: i64) -> BlockHeight {
        BlockHeight::try_from(n).expect("height")
    }
    fn s(n: i64) -> SeqNum {
        SeqNum::try_from(n).expect("seq num")
    }

    let v0 = validator(0);
    let v1 = validator(1);
    let v2 = validator(2);
    // The genesis sender is not a bonded validator, so a bonded validator's self-parent chain never
    // reaches genesis (mirrors the Scala genesis being signed by a distinct "genesis validator").
    let g = validator(255);
    let bonds: std::collections::BTreeMap<Validator, NonNegI64> = [
        (v0.clone(), NonNegI64::try_from(10).unwrap()),
        (v1.clone(), NonNegI64::try_from(10).unwrap()),
        (v2.clone(), NonNegI64::try_from(10).unwrap()),
    ]
    .into_iter()
    .collect();

    let st: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
    let genesis = st.create_message(id(255, 0), h(0), g, s(0), bonds.clone(), &BTreeSet::new());
    let st = st.insert_msg(&genesis);

    // Layer 1: a three-way fork — each validator sees only genesis.
    let a1 = st.create_message(
        id(0, 1),
        h(1),
        v0.clone(),
        s(1),
        bonds.clone(),
        &[genesis.clone()].into_iter().collect(),
    );
    let st = st.insert_msg(&a1);
    let b1 = st.create_message(
        id(1, 1),
        h(1),
        v1.clone(),
        s(1),
        bonds.clone(),
        &[genesis.clone()].into_iter().collect(),
    );
    let st = st.insert_msg(&b1);
    let c1 = st.create_message(
        id(2, 1),
        h(1),
        v2.clone(),
        s(1),
        bonds.clone(),
        &[genesis.clone()].into_iter().collect(),
    );
    let st = st.insert_msg(&c1);

    // Layer 2: convergence — each sees all of layer 1.
    let l1: BTreeSet<_> = [a1.clone(), b1.clone(), c1.clone()].into_iter().collect();
    let a2 = st.create_message(id(0, 2), h(2), v0.clone(), s(2), bonds.clone(), &l1);
    let st = st.insert_msg(&a2);
    let b2 = st.create_message(id(1, 2), h(2), v1.clone(), s(2), bonds.clone(), &l1);
    let st = st.insert_msg(&b2);
    let c2 = st.create_message(id(2, 2), h(2), v2.clone(), s(2), bonds.clone(), &l1);
    let st = st.insert_msg(&c2);

    // Layer 3: convergence — each sees all of layer 2.
    let l2: BTreeSet<_> = [a2.clone(), b2.clone(), c2.clone()].into_iter().collect();
    let a3 = st.create_message(id(0, 3), h(3), v0.clone(), s(3), bonds.clone(), &l2);
    let st = st.insert_msg(&a3);
    let b3 = st.create_message(id(1, 3), h(3), v1.clone(), s(3), bonds.clone(), &l2);
    let st = st.insert_msg(&b3);
    let c3 = st.create_message(id(2, 3), h(3), v2.clone(), s(3), bonds.clone(), &l2);
    let st = st.insert_msg(&c3);

    // Layer 4: the finalizing message sees all of layer 3 → the layer-1 fork is finalized.
    let l3: BTreeSet<_> = [a3.clone(), b3.clone(), c3.clone()].into_iter().collect();
    let a4 = st.create_message(id(0, 4), h(4), v0.clone(), s(4), bonds.clone(), &l3);

    assert!(
        !a4.fringe.is_empty(),
        "fringe should advance past the empty genesis fringe"
    );
    assert_eq!(
        a4.fringe,
        [id(0, 1), id(1, 1), id(2, 1)].into_iter().collect(),
        "fringe should be the layer-1 fork"
    );
}

/// **A bonded validator that has stopped producing messages must not cap the fringe (#70).**
///
/// The fringe gate's partition ranged over the *whole* bonded map, so a validator that produced no
/// message could never be "seen by every seer": the partition was unsatisfiable, and finality stopped
/// whatever share of the stake the survivors held. Measured on a three-validator devnet at
/// `100/100/50` on 2026-09-29 — with the 50-stake validator stopped, the two survivors at 80 % of the
/// pool did not resume finality.
///
/// The falsifier is two-way, because either half alone is satisfiable by the wrong rule:
///
/// - through `liveness::calculate_finalization` — the partition ranges over the *live* weight set, so
///   four bonded validators of which three are speaking still finalise the three-way fork;
/// - through the raw gate with the same map twice — the pre-fix shape — the silent fourth member
///   leaves the partition unsatisfiable and the fringe does **not** advance. Delete the liveness rule
///   from the first call and this is the behaviour that comes back.
///
/// The quorum stays the whole bonded map in both, which is the other half of the design: 300 of 400
/// is a supermajority, and a minority still could not finalise on its own.
#[test]
fn a_silent_bonded_validator_does_not_cap_the_fringe() {
    use rchain_block_storage::dag::finalizer::Finalizer;
    use rchain_block_storage::dag::liveness;
    use rchain_block_storage::dag::message_state::DagMessageState;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, SeqNum};

    fn validator(byte: u8) -> Validator {
        Validator::new([byte; 65])
    }
    fn id(sender: u8, height: i64) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[0] = sender;
        bytes[1] = (height & 0xff) as u8;
        BlockHash::new(bytes)
    }
    fn h(n: i64) -> BlockHeight {
        BlockHeight::try_from(n).expect("height")
    }
    fn s(n: i64) -> SeqNum {
        SeqNum::try_from(n).expect("seq num")
    }

    let v0 = validator(0);
    let v1 = validator(1);
    let v2 = validator(2);
    // Bonded, and never speaks: the validator the rule exists for.
    let silent = validator(3);
    let g = validator(255);
    let bonds: std::collections::BTreeMap<Validator, NonNegI64> = [
        (v0.clone(), NonNegI64::try_from(100).unwrap()),
        (v1.clone(), NonNegI64::try_from(100).unwrap()),
        (v2.clone(), NonNegI64::try_from(100).unwrap()),
        (silent.clone(), NonNegI64::try_from(100).unwrap()),
    ]
    .into_iter()
    .collect();

    let st: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
    let genesis = st.create_message(id(255, 0), h(0), g, s(0), bonds.clone(), &BTreeSet::new());
    let st = st.insert_msg(&genesis);

    // Layers 1–3: a three-way fork, then convergence, exactly the shape the finalizer needs (a
    // justification must reference messages *beyond* the candidate next layer).
    let mut state = st;
    let mut previous: Vec<_> = Vec::new();
    let mut layer1: std::collections::BTreeSet<_> = std::collections::BTreeSet::new();
    for (i, v) in [v0.clone(), v1.clone(), v2.clone()].into_iter().enumerate() {
        let parents: std::collections::BTreeSet<_> = if previous.is_empty() {
            [genesis.clone()].into_iter().collect()
        } else {
            previous.iter().cloned().collect()
        };
        let m = state.create_message(id(i as u8, 1), h(1), v, s(1), bonds.clone(), &parents);
        state = state.insert_msg(&m);
        layer1.insert(m);
    }
    previous = layer1.iter().cloned().collect();
    for height in 2..=3 {
        let mut layer = std::collections::BTreeSet::new();
        for (i, v) in [v0.clone(), v1.clone(), v2.clone()].into_iter().enumerate() {
            let m = state.create_message(
                id(i as u8, height),
                h(height),
                v,
                s(height),
                bonds.clone(),
                &previous.iter().cloned().collect(),
            );
            state = state.insert_msg(&m);
            layer.insert(m);
        }
        previous = layer.iter().cloned().collect();
    }

    // The finalizing message's justifications: all of layer 3.
    let justifications: std::collections::BTreeSet<_> = previous.iter().cloned().collect();
    let finalizer = Finalizer::new(&state.msg_map);

    let (_, live) = liveness::calculate_finalization(&finalizer, &justifications, &bonds);
    assert_eq!(
        live.map(|f| f
            .iter()
            .map(|m| m.id)
            .collect::<std::collections::BTreeSet<_>>()),
        Some([id(0, 1), id(1, 1), id(2, 1)].into_iter().collect()),
        "the three speaking validators finalise the layer-1 fork without the silent fourth"
    );

    // The control: the same map as both the partition and the quorum — the shape before the liveness
    // rule — where the silent validator the partition requires can never have seen anything.
    let (_, without) = finalizer.calculate_finalization(&justifications, &bonds, &bonds);
    assert!(
        without.is_none(),
        "with the silent validator in the partition, no candidate can be a full partition and \
         nothing advances — this is the behaviour the rule replaces"
    );
}

/// **The case a real net produces, and the fixture above cannot see.**
///
/// `a_silent_bonded_validator_does_not_cap_the_fringe` builds a validator that *never speaks*, so it is
/// absent from `latest_heights` and the window drops it precisely because nothing ever saw a message from
/// it. What a net actually does is different: a validator produces blocks and then stops, so its **last
/// message stays in `latest_msgs`** (`block-storage/src/dag/message_state.rs:90-102` — one entry per
/// sender, no bond check, no eviction) and every later candidate still justifies it. The window then drops
/// it — `heights_behind(tip, stale) > LIVENESS_WINDOW` — while the justification set still carries it, so
/// the coverage gate is asked to compare a message set against a *different* set's size:
///
/// ```text
/// min_msgs  = [v0@8, v1@8, v2@8, stopped@1]   -> 4 entries, 4 senders
/// partitions = {v0, v1, v2}                   -> 3 bonded senders, the live set
/// check_min_messages: 4 == 3?  no  ->  NoAdvance::Coverage, forever
/// ```
///
/// That is the second stop of #70, and it is why finality advances a few heights after a validator is
/// killed and then stops: the stale message survives the window that was meant to retire it.
#[test]
fn a_validator_that_spoke_and_then_stopped_does_not_cap_the_fringe() {
    use rchain_block_storage::dag::finalizer::Finalizer;
    use rchain_block_storage::dag::liveness;
    use rchain_block_storage::dag::message_state::DagMessageState;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, SeqNum};

    fn validator(byte: u8) -> Validator {
        Validator::new([byte; 65])
    }
    fn id(sender: u8, height: i64) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[0] = sender;
        bytes[1] = (height & 0xff) as u8;
        BlockHash::new(bytes)
    }
    fn h(n: i64) -> BlockHeight {
        BlockHeight::try_from(n).expect("height")
    }
    fn s(n: i64) -> SeqNum {
        SeqNum::try_from(n).expect("seq num")
    }

    let v0 = validator(0);
    let v1 = validator(1);
    let v2 = validator(2);
    // Bonded, speaks at height 1, then stops. Its message never leaves `latest_msgs`.
    let stopped = validator(3);
    let g = validator(255);
    let bonds: std::collections::BTreeMap<Validator, NonNegI64> = [
        (v0.clone(), NonNegI64::try_from(100).unwrap()),
        (v1.clone(), NonNegI64::try_from(100).unwrap()),
        (v2.clone(), NonNegI64::try_from(100).unwrap()),
        (stopped.clone(), NonNegI64::try_from(100).unwrap()),
    ]
    .into_iter()
    .collect();

    let st: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
    let genesis = st.create_message(id(255, 0), h(0), g, s(0), bonds.clone(), &BTreeSet::new());
    let mut state = st.insert_msg(&genesis);

    // Height 1: all four speak, so the stopped validator's last message is at the tip's floor.
    let mut layer: std::collections::BTreeSet<_> = std::collections::BTreeSet::new();
    for (i, v) in [v0.clone(), v1.clone(), v2.clone(), stopped.clone()]
        .into_iter()
        .enumerate()
    {
        let m = state.create_message(
            id(i as u8, 1),
            h(1),
            v,
            s(1),
            bonds.clone(),
            &[genesis.clone()].into_iter().collect(),
        );
        state = state.insert_msg(&m);
        layer.insert(m);
    }
    let stale = layer
        .iter()
        .find(|m| m.sender == stopped)
        .expect("the stopped validator's last message")
        .clone();

    // Heights 2..=8: the three survivors only, each justifying the previous layer **plus the stale
    // message** — which is what `latest_msgs` hands them, one entry per sender.
    let mut previous: Vec<_> = layer
        .iter()
        .filter(|m| m.sender != stopped)
        .cloned()
        .collect();
    for height in 2..=8 {
        let mut next = std::collections::BTreeSet::new();
        let mut parents: std::collections::BTreeSet<_> = previous.iter().cloned().collect();
        parents.insert(stale.clone());
        for (i, v) in [v0.clone(), v1.clone(), v2.clone()].into_iter().enumerate() {
            let m = state.create_message(
                id(i as u8, height),
                h(height),
                v,
                s(height),
                bonds.clone(),
                &parents,
            );
            state = state.insert_msg(&m);
            next.insert(m);
        }
        previous = next.iter().cloned().collect();
    }

    // The candidate's parents: the latest message per sender, which still includes the stale one.
    let mut justifications: std::collections::BTreeSet<_> = previous.iter().cloned().collect();
    justifications.insert(stale.clone());

    let finalizer = Finalizer::new(&state.msg_map);
    let (_, live) = liveness::calculate_finalization(&finalizer, &justifications, &bonds);

    assert!(
        live.is_some(),
        "three of four bonded validators are live and hold 75 % of the stake; a message from a \
         validator that stopped seven heights ago must not hold the fringe at the height it stopped at. \
         Today the coverage gate compares the four-message justification set against the three-sender \
         live partition and refuses — which is the second stop of #70."
    );
}

/// **The devnet's own stake split, with one validator stopped — and it advances.**
///
/// This is the separator, and it is what the next fix is aimed by. The 2026-09-30 devnet run pinned
/// finality in 3 of 3 attempts *after* the derivation was fixed (`80782e184`), and the node's own stall
/// line says the remaining pin is **`Support`** — `150 of 250`, i.e. 60 %, below the 2/3 threshold — not
/// `Coverage`. **This fixture proves the derivation is not that stop**, so the fix belongs in the support
/// arithmetic rather than in `inPartition`.
#[test]
fn the_devnet_stake_split_finalises_with_one_validator_stopped() {
    use rchain_block_storage::dag::finalizer::Finalizer;
    use rchain_block_storage::dag::liveness;
    use rchain_block_storage::dag::message_state::DagMessageState;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, SeqNum};

    fn validator(byte: u8) -> Validator {
        Validator::new([byte; 65])
    }
    fn id(sender: u8, height: i64) -> BlockHash {
        let mut bytes = [0u8; 32];
        bytes[0] = sender;
        bytes[1] = (height & 0xff) as u8;
        BlockHash::new(bytes)
    }
    fn h(n: i64) -> BlockHeight {
        BlockHeight::try_from(n).expect("height")
    }
    fn s(n: i64) -> SeqNum {
        SeqNum::try_from(n).expect("seq num")
    }

    let v0 = validator(0);
    let v1 = validator(1);
    let stopped = validator(2);
    let g = validator(255);
    // The devnet's genesis stakes: 100 / 100 / 50, validator 2 the 50 that gets killed.
    let bonds: std::collections::BTreeMap<Validator, NonNegI64> = [
        (v0.clone(), NonNegI64::try_from(100).unwrap()),
        (v1.clone(), NonNegI64::try_from(100).unwrap()),
        (stopped.clone(), NonNegI64::try_from(50).unwrap()),
    ]
    .into_iter()
    .collect();

    let st: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
    let genesis = st.create_message(id(255, 0), h(0), g, s(0), bonds.clone(), &BTreeSet::new());
    let mut state = st.insert_msg(&genesis);

    let mut layer: std::collections::BTreeSet<_> = std::collections::BTreeSet::new();
    for (i, v) in [v0.clone(), v1.clone(), stopped.clone()]
        .into_iter()
        .enumerate()
    {
        let m = state.create_message(
            id(i as u8, 1),
            h(1),
            v,
            s(1),
            bonds.clone(),
            &[genesis.clone()].into_iter().collect(),
        );
        state = state.insert_msg(&m);
        layer.insert(m);
    }
    let stale = layer
        .iter()
        .find(|m| m.sender == stopped)
        .expect("the stopped validator's last message")
        .clone();

    let mut previous: Vec<_> = layer
        .iter()
        .filter(|m| m.sender != stopped)
        .cloned()
        .collect();
    for height in 2..=8 {
        let mut parents: std::collections::BTreeSet<_> = previous.iter().cloned().collect();
        parents.insert(stale.clone());
        let mut next = std::collections::BTreeSet::new();
        for (i, v) in [v0.clone(), v1.clone()].into_iter().enumerate() {
            let m = state.create_message(
                id(i as u8, height),
                h(height),
                v,
                s(height),
                bonds.clone(),
                &parents,
            );
            state = state.insert_msg(&m);
            next.insert(m);
        }
        previous = next.iter().cloned().collect();
    }

    let mut justifications: std::collections::BTreeSet<_> = previous.iter().cloned().collect();
    justifications.insert(stale.clone());

    let finalizer = Finalizer::new(&state.msg_map);
    let (_, live) = liveness::calculate_finalization(&finalizer, &justifications, &bonds);
    let (_, raw) = finalizer.calculate_finalization(&justifications, &bonds, &bonds);

    assert!(
        live.is_some(),
        "with 200 of 250 stake live (80 %), the derivation must publish a fringe — the pin measured on \
         the devnet is Support, not Coverage, so it is not this function"
    );
    assert!(
        raw.is_none(),
        "the control: without the liveness rule the partition is the whole bonded map and nothing advances"
    );
    assert!(
        liveness::calculate_finalization_detailed(&finalizer, &justifications, &bonds)
            .2
            .is_none(),
        "and it reports no stall reason at all"
    );
}
/// **The chain the node's own block creator builds now reaches a fringe — and it did not.**
///
/// Driven through the production entry point, `DagMessageState::create_msg_and_update_sender`, with three
/// validators at the devnet's own `100/100/50`, all live, no kill and no stopped validator. Before the
/// round snapshot this refused and never advanced:
///
/// ```text
/// Support { supporting: 0, total: 250, full_partitions: 0, candidates: 2 }
/// ```
///
/// **`supporting: 0` was not a stake shortfall.** `calculate_fringe` sums the stake of the candidates whose
/// `seen_by` values all equal the live partition, and no candidate qualified, so the numerator was zero
/// before the quorum was ever consulted. `calculate_next_fringe_support_map` builds each candidate's
/// `seen_by` from `mv.parents ∖ next_layer` — the justifications *beyond* the candidate next layer — so a
/// block whose justifications **are** every sender's newest message credits nobody with having seen
/// anything: the head of each round has an empty remainder and the later movers have seen a prefix.
///
/// **This file's own header has said so since it was written**: "a lockstep DAG — which the full block
/// pipeline's `latest_msgs` proposer always produces, and which the Scala `MultiParentCasperFinalizationSpec`
/// round-robin scenario built — never finalizes. That Scala spec is itself `ignore`d." What had never been
/// done is connect that sentence to a devnet's stalled finality. Three devnet runs and a fixed derivation
/// went into chasing a gate whose input the node never produced.
///
/// The fix is the parent set and nothing else: `create_msg_and_update_sender` now justifies
/// `DagMessageState::round_parents` — the `latest_msgs` as of the last round boundary — while the height
/// rule (`max + 1`), the validity rules and the gate all stay as they were. **The control is inline**: the
/// same chain built with `latest_msgs` as parents still does not advance, so this test is red if the round
/// snapshot is dropped.
#[test]
fn the_chain_the_nodes_own_proposer_builds_reaches_a_fringe() {
    use rchain_block_storage::dag::finalizer::{Finalizer, NoAdvance};
    use rchain_block_storage::dag::liveness;
    use rchain_block_storage::dag::message_state::DagMessageState;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, NonNegI64 as Stake, SeqNum};

    fn validator(b: u8) -> Validator {
        Validator::new([b; 65])
    }
    fn id(sender: u8, height: i64) -> BlockHash {
        let mut b = [0u8; 32];
        b[0] = sender;
        b[1] = (height & 0xff) as u8;
        b[2] = ((height >> 8) & 0xff) as u8;
        BlockHash::new(b)
    }
    fn h(n: i64) -> BlockHeight {
        BlockHeight::try_from(n).expect("height")
    }
    fn s(n: i64) -> SeqNum {
        SeqNum::try_from(n).expect("seq num")
    }
    fn byte_of(v: &Validator) -> u8 {
        (0u8..=255)
            .find(|b| Validator::new([*b; 65]) == *v)
            .expect("a validator this fixture built")
    }

    let vs = [validator(0), validator(1), validator(2)];
    let g = validator(255);
    // The devnet's own genesis split, and no validator is ever stopped.
    let bonds: std::collections::BTreeMap<Validator, Stake> = [
        (vs[0].clone(), Stake::try_from(100).unwrap()),
        (vs[1].clone(), Stake::try_from(100).unwrap()),
        (vs[2].clone(), Stake::try_from(50).unwrap()),
    ]
    .into_iter()
    .collect();

    let st: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
    let genesis = st.create_message(id(255, 0), h(0), g, s(0), bonds.clone(), &BTreeSet::new());
    let mut state = st.insert_msg(&genesis);

    for height in 1..=60_i64 {
        let v = vs[(height as usize - 1) % 3].clone();
        let (next, _m) = state
            .create_msg_and_update_sender(&v, |snd, ht| id(byte_of(snd), i64::from(ht)))
            .expect("a message");
        state = next;
    }

    let justifications: std::collections::BTreeSet<_> =
        state.latest_msgs.values().cloned().collect();
    let tip = justifications
        .iter()
        .map(|m| m.height)
        .max()
        .expect("a tip");
    let live = liveness::live_weight_set(
        &bonds,
        &liveness::latest_heights(justifications.iter().map(|m| (m.sender.clone(), m.height))),
        tip,
        liveness::LIVENESS_WINDOW,
    );
    assert_eq!(
        live.len(),
        3,
        "all three validators are live at the tip, so nothing here is a retired validator or a rate"
    );

    let finalizer = Finalizer::new(&state.msg_map);
    let (_parent, fringe, why) =
        liveness::calculate_finalization_detailed(&finalizer, &justifications, &bonds);
    let fringe = fringe.unwrap_or_else(|| {
        panic!(
            "the chain the block creator builds must publish a fringe now that it justifies the round \
             snapshot instead of `latest_msgs` — got {why:?}"
        )
    });
    let finalized = fringe
        .iter()
        .map(|m| m.height)
        .max()
        .expect("a published fringe is non-empty");
    assert!(
        finalized < tip,
        "and the fringe trails the tip rather than claiming it: {finalized:?} of {tip:?}"
    );

    // **The control, and it is what makes this test a falsifier.** The same three validators on the same
    // chain, with `latest_msgs` as the parent set — the shape before the round snapshot — do not advance.
    let st2: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
    let genesis2 = st2.create_message(id(255, 0), h(0), g, s(0), bonds.clone(), &BTreeSet::new());
    let mut state2 = st2.insert_msg(&genesis2);
    for height in 1..=60_i64 {
        let v = vs[(height as usize - 1) % 3].clone();
        let max = state2
            .latest_msgs
            .values()
            .map(|m| m.height)
            .max()
            .expect("a tip");
        let parents: std::collections::BTreeSet<_> = state2.latest_msgs.values().cloned().collect();
        let m = state2.create_message(
            id((height - 1) as u8 % 3, height),
            max + rchain_shared::refined::NonNegI64::one(),
            v,
            s(height),
            bonds.clone(),
            &parents,
        );
        state2 = state2.insert_msg(&m);
    }
    let justifications2: std::collections::BTreeSet<_> =
        state2.latest_msgs.values().cloned().collect();
    let finalizer2 = Finalizer::new(&state2.msg_map);
    let (_p2, fringe2, why2) =
        liveness::calculate_finalization_detailed(&finalizer2, &justifications2, &bonds);
    assert!(
        fringe2.is_none(),
        "`latest_msgs` as the parent set still refuses — this is the shape the round snapshot replaced, \
         and if it starts advancing the snapshot is not what fixed this (got {why2:?})"
    );
    assert!(
        matches!(
            why2,
            Some(NoAdvance::Support {
                supporting: 0,
                full_partitions: 0,
                ..
            })
        ),
        "and it refuses for the recorded reason: no candidate is a full partition, so the numerator is \
         zero before any quorum is consulted — got {why2:?}"
    );
}

/// **What the gate needs from the proposer, measured rather than reasoned: the parent set, not the
/// heights and not the rule.**
///
/// The fixture above shows the chain the node builds (`latest_msgs`) is refused. This one shows what
/// changes it, by holding everything else fixed — the same three validators, the same stakes, the same
/// height rule (`max + 1`, one message per height), the same round-robin order — and varying **only which
/// messages a block justifies**:
///
/// | parent set | result |
/// |---|---|
/// | the current `latest_msgs` | refuses, `Support { supporting: 0, … }`, and never advances |
/// | the `latest_msgs` **as of the start of the round** | publishes a fringe, and keeps up |
///
/// The second row is the finding. At 20, 60 and 120 rounds the fringe sits exactly **12 heights** behind
/// the tip — 48/60, 168/180, 348/360 — so it advances with the chain rather than falling further behind.
/// The lag is the structure the gate asks for (a candidate needs, for every live sender, a parent that is
/// not that sender's *oldest* unfinalized message, which needs at least two unfinalized layers), not a
/// rate.
///
/// So the correction lives in the proposer's parent set. Heights are untouched, no validity rule is
/// touched, and an unpatched node accepts a patched node's blocks — the blocks are ordinary blocks that
/// justify a snapshot instead of the newest message of every sender.
#[test]
fn a_round_snapshot_of_the_latest_messages_is_what_the_gate_needs() {
    use rchain_block_storage::dag::finalizer::Finalizer;
    use rchain_block_storage::dag::liveness;
    use rchain_block_storage::dag::message_state::DagMessageState;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, NonNegI64 as Stake, SeqNum};

    fn validator(b: u8) -> Validator {
        Validator::new([b; 65])
    }
    fn id(sender: u8, height: i64) -> BlockHash {
        let mut b = [0u8; 32];
        b[0] = sender;
        b[1] = (height & 0xff) as u8;
        b[2] = ((height >> 8) & 0xff) as u8;
        BlockHash::new(b)
    }
    fn h(n: i64) -> BlockHeight {
        BlockHeight::try_from(n).expect("height")
    }
    fn s(n: i64) -> SeqNum {
        SeqNum::try_from(n).expect("seq num")
    }

    /// -> (tip height, fringe height if any, the refusal if any). `snapshot` is the parent policy:
    /// `false` justifies the current `latest_msgs`, `true` the set that was latest when the round began.
    fn drive(snapshot: bool, rounds: i64) -> (i64, Option<i64>, Option<String>) {
        let vs = [validator(0), validator(1), validator(2)];
        let g = validator(255);
        let bonds: std::collections::BTreeMap<Validator, Stake> = [
            (vs[0].clone(), Stake::try_from(100).unwrap()),
            (vs[1].clone(), Stake::try_from(100).unwrap()),
            (vs[2].clone(), Stake::try_from(50).unwrap()),
        ]
        .into_iter()
        .collect();
        let st: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
        let genesis = st.create_message(id(255, 0), h(0), g, s(0), bonds.clone(), &BTreeSet::new());
        let mut state = st.insert_msg(&genesis);
        let mut round_parents: std::collections::BTreeSet<_> =
            [genesis.clone()].into_iter().collect();

        for round in 1..=rounds {
            let at_round_start: std::collections::BTreeSet<_> =
                state.latest_msgs.values().cloned().collect();
            for (i, v) in vs.iter().enumerate() {
                let max = state
                    .latest_msgs
                    .values()
                    .map(|m| m.height)
                    .max()
                    .expect("a tip");
                let parents: std::collections::BTreeSet<_> = if snapshot {
                    round_parents.clone()
                } else {
                    state.latest_msgs.values().cloned().collect()
                };
                let m = state.create_message(
                    id(i as u8, round * 3 + i as i64),
                    max + rchain_shared::refined::NonNegI64::one(),
                    v.clone(),
                    s(round * 3 + i as i64),
                    bonds.clone(),
                    &parents,
                );
                state = state.insert_msg(&m);
            }
            round_parents = at_round_start;
        }

        let justifications: std::collections::BTreeSet<_> =
            state.latest_msgs.values().cloned().collect();
        let tip = justifications
            .iter()
            .map(|m| m.height)
            .max()
            .expect("a tip");
        let finalizer = Finalizer::new(&state.msg_map);
        let (_p, fringe, why) =
            liveness::calculate_finalization_detailed(&finalizer, &justifications, &bonds);
        (
            i64::from(tip),
            fringe
                .as_ref()
                .and_then(|f| f.iter().map(|m| m.height).max())
                .map(i64::from),
            why.map(|w| format!("{w:?}")),
        )
    }

    // 1. The node's parent set: nothing is ever published.
    let (tip, fringe, why) = drive(false, 20);
    assert_eq!(
        (tip, fringe),
        (60, None),
        "the current `latest_msgs` parent set never advances — got {fringe:?}, {why:?}"
    );

    // 2. A round snapshot: it publishes, and the lag does not grow with the chain.
    let mut lags = Vec::new();
    for rounds in [20, 60, 120] {
        let (tip, fringe, why) = drive(true, rounds);
        let fringe = fringe.unwrap_or_else(|| panic!("no fringe at {rounds} rounds: {why:?}"));
        lags.push(tip - fringe);
    }
    assert_eq!(
        lags,
        vec![12, 12, 12],
        "the fringe tracks the tip at a constant distance — a lag that grew with the chain would be a \
         rate problem, and a lag that is constant is the unfinalized region the gate's rule needs"
    );
}

/// **The half of the rule the fixture above could not reach: a validator proposes *once* per round.**
///
/// `block_creator.rs:58-75` derives both the new block's `block_num` and its `seq_num` **from the
/// justification set** — `max(justifications.block_num) + 1` and
/// `justifications.find(sender).seq_num + 1`. The parent set is a cross-sender snapshot, so a second
/// proposal inside one round justifies the *same* set and reuses the proposer's own `(sender, seq_num)`.
/// The DAG refuses that, correctly:
///
/// ```text
/// ERROR Self-created block #93 (seq 92) failed validation with internal error: failed to insert block
///       into DAG: equivocation detected: sender produced two blocks with the same sequence number
/// ```
///
/// — and after three consecutive refusals `consecutive_failures` halts the autopropose timer
/// (`node_runtime.rs:1504`), so the chain produces nothing while deploys sit in the pool. That is a devnet
/// finding, not a fixture one: every test in this file supplied its own `sender_seq`, and
/// `create_msg_and_update_sender` takes its seq from `latest_msgs` rather than from the parent set, so all
/// of them agreed the chain was linked while the node disagreed.
///
/// **The rule is the guard, not a correction to the snapshot.** `has_advanced_past_the_round` refuses a
/// second proposal from a validator that has already spoken, and the parent set stays a pure snapshot —
/// adding the proposer's own newest message back was tried and measured, and the gate then refuses again
/// (`0 of 250`, a devnet back to height 20 with finality at 3), because that message is exactly the parent
/// that is *not* beyond the next layer.
///
/// This test drives the proposer's rule and the block creator's derivation together, and asserts both
/// halves: the guard declines a second attempt, and no two blocks ever share a `(sender, seq_num)`.
#[test]
fn a_validator_does_not_propose_twice_in_one_round() {
    use rchain_block_storage::dag::message_state::DagMessageState;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, NonNegI64 as Stake, SeqNum};

    fn validator(b: u8) -> Validator {
        Validator::new([b; 65])
    }
    fn id(sender: u8, height: i64) -> BlockHash {
        let mut b = [0u8; 32];
        b[0] = sender;
        b[1] = (height & 0xff) as u8;
        b[2] = ((height >> 8) & 0xff) as u8;
        BlockHash::new(b)
    }
    fn h(n: i64) -> BlockHeight {
        BlockHeight::try_from(n).expect("height")
    }
    fn s(n: i64) -> SeqNum {
        SeqNum::try_from(n).expect("seq num")
    }
    fn byte_of(v: &Validator) -> u8 {
        (0u8..=255)
            .find(|b| Validator::new([*b; 65]) == *v)
            .expect("a validator this fixture built")
    }

    let vs = [validator(0), validator(1), validator(2)];
    let bonds: std::collections::BTreeMap<Validator, Stake> = [
        (vs[0].clone(), Stake::try_from(100).unwrap()),
        (vs[1].clone(), Stake::try_from(100).unwrap()),
        (vs[2].clone(), Stake::try_from(50).unwrap()),
    ]
    .into_iter()
    .collect();

    /// Drive `order` as proposal *attempts*, applying the guard when `guarded`. Returns the number of
    /// attempts the guard declined and every `(sender, seq)` produced.
    fn drive(
        guarded: bool,
        order: &[usize],
        vs: &[Validator; 3],
        bonds: &std::collections::BTreeMap<Validator, Stake>,
        genesis_id: u8,
    ) -> (usize, Vec<(Validator, SeqNum)>) {
        let g = validator(255);
        let st: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
        let genesis = st.create_message(
            id(genesis_id, 0),
            h(0),
            g,
            s(0),
            bonds.clone(),
            &BTreeSet::new(),
        );
        let mut state = st.insert_msg(&genesis);
        let mut declined = 0;
        let mut produced = Vec::new();

        for (step, who) in order.iter().enumerate() {
            let v = vs[*who].clone();
            if guarded && state.has_advanced_past_the_round(&v) {
                declined += 1;
                continue;
            }
            let parents = state.parents_for_new_block();
            // The block creator's own derivation, verbatim in shape: both numbers come off the parent set.
            let block_num = parents
                .iter()
                .map(|m| m.height)
                .max()
                .map(|m| m + rchain_shared::refined::NonNegI64::one())
                .unwrap_or_else(BlockHeight::zero);
            let seq_num = parents
                .iter()
                .find(|m| m.sender == v)
                .map(|m| m.sender_seq + rchain_shared::refined::NonNegI64::one())
                .unwrap_or_else(SeqNum::zero);
            produced.push((v.clone(), seq_num));
            let m = state.create_message(
                id(*who as u8, i64::from(block_num) * 16 + step as i64),
                block_num,
                v.clone(),
                seq_num,
                bonds.clone(),
                &parents,
            );
            state = state.insert_msg(&m);
        }
        (declined, produced)
    }

    // `v0` tries twice in a row at the start of nearly every round — what a two-validator tail of a chain
    // does, and what the autopropose tap produces when it fires on each validated block.
    let order = [0usize, 0, 1, 0, 0, 2, 0, 0, 1, 1, 2, 0, 0, 1];

    let (declined, produced) = drive(true, &order, &vs, &bonds, 255);
    assert!(
        declined > 0,
        "the guard must actually fire on this order — a test that never declines the second attempt is not \
         exercising the rule"
    );
    let mut seen: std::collections::BTreeMap<Validator, SeqNum> = std::collections::BTreeMap::new();
    for (v, seq) in &produced {
        assert!(
            seen.get(v).is_none_or(|prev| *prev < *seq),
            "{} produced two blocks with sequence {seq:?} — the DAG refuses a repeated (sender, seq_num) as \
             an equivocation, and it is right to",
            byte_of(v)
        );
        seen.insert(v.clone(), *seq);
    }

    // **The control.** The same attempts with the guard ignored, which is what the node did before it had
    // one: the second attempt reuses the sequence, and this is the assertion that would have caught it.
    let (_declined, unguarded) = drive(false, &order, &vs, &bonds, 255);
    let mut seen: std::collections::BTreeMap<Validator, SeqNum> = std::collections::BTreeMap::new();
    let repeated = unguarded.iter().any(|(v, seq)| {
        let repeat = seen.get(v).is_some_and(|prev| prev >= seq);
        seen.insert(v.clone(), *seq);
        repeat
    });
    assert!(
        repeated,
        "without the guard a second proposal in a round must reuse the sequence — if it does not, this \
         order does not reach the case and the test above proves nothing"
    );
}

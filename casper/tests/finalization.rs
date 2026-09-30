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

/// **The chain the node's own block creator builds cannot be finalised — in-process, with every validator
/// live, and no devnet.**
///
/// `DagMessageState::create_msg_and_update_sender` is the production call: a block's height is
/// `max(latest_msgs) + 1` and its justifications are **every** `latest_msgs` entry, one per sender. So the
/// chain it builds is *totally connected* — every block sees every validator's most recent block — and the
/// fringe gate refuses it:
///
/// ```text
/// Support { supporting: 0, total: 250, full_partitions: 0, candidates: 2 }
/// ```
///
/// **`supporting: 0` is not a stake shortfall.** `calculate_fringe` sums the stake of the candidates whose
/// `seen_by` values all equal the live partition, and here *no* candidate does, so the numerator is zero
/// before the quorum is ever consulted. The live set is all three validators (asserted below), no validator
/// is silent, and the stake split is the devnet's own.
///
/// **What the gate wants and this chain does not have.** `calculate_next_fringe_support_map` computes each
/// candidate's `seen_by` from `mv.parents ∖ next_layer` — the justify-cations *beyond* the candidate next
/// layer — so a block whose justifications **are** the next layer credits nobody with having seen it. In a
/// totally connected chain that is every block: the mover at the head of a round has an empty remainder,
/// and the later movers see only a prefix of the layer. So the gate's demand ("every live seer has seen
/// every next-layer message, through messages past that layer") is satisfiable only by the *fork* shape the
/// tests above construct — which is the opposite of what a proposer justifying `latest_msgs` produces.
///
/// **This file's own header has said so since it was written**: "a lockstep DAG — which the full block
/// pipeline's `latest_msgs` proposer always produces, and which the Scala `MultiParentCasperFinalizationSpec`
/// round-robin scenario built — never finalizes. That Scala spec is itself `ignore`d." What was never done
/// is connect that sentence to a devnet's stalled finality. That is what this fixture is: the campaign's
/// `0 of 250 (0 full partition(s) among N candidate(s))` lines, produced on demand, with no kill and no
/// stopped validator.
///
/// The third block is the separator, and it is what makes this a finding rather than an assertion: the
/// **same** validators at the **same** stakes on a fork-shaped DAG publish a fringe. The variable is the
/// parent set, not the stake split and not a departed validator.
///
/// **This test pins the defect, so it is the falsifier's premise and it will fail when the gate is
/// corrected.** That failure is the signal, and the fix inverts it: the assertions become "the derivation
/// publishes a fringe on the chain the proposer builds", with the fork control unchanged as the shape the
/// rule already accepts.
#[test]
fn the_dag_the_nodes_own_proposer_builds_cannot_advance_the_fringe() {
    use rchain_block_storage::dag::finalizer::{Finalizer, NoAdvance};
    use rchain_block_storage::dag::liveness;
    use rchain_block_storage::dag::message_state::DagMessageState;
    use rchain_models::validator::Validator;
    use rchain_shared::refined::{BlockHeight, NonNegI64 as Stake, SeqNum};

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

    // Round-robin over three live validators, 60 blocks, each through the production entry point.
    for height in 1..=60_i64 {
        let v = vs[(height as usize - 1) % 3].clone();
        let (next, _m) = state
            .create_msg_and_update_sender(&v, |snd, ht| id(byte_of(snd), i64::from(ht)))
            .expect("a message");
        state = next;
    }

    let justifications: std::collections::BTreeSet<_> =
        state.latest_msgs.values().cloned().collect();
    assert_eq!(
        justifications.len(),
        4,
        "`create_msg_and_update_sender` justifies every `latest_msgs` entry — three validators plus \
         genesis — which is the parent set the gate is asked about"
    );
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
        "all three validators are live at the tip — the refusal below is not a retired validator, and \
         there is no stopped one anywhere in this fixture"
    );

    let finalizer = Finalizer::new(&state.msg_map);
    let (_parent, fringe, why) =
        liveness::calculate_finalization_detailed(&finalizer, &justifications, &bonds);
    match why {
        Some(NoAdvance::Support {
            supporting,
            total,
            full_partitions,
            candidates,
        }) => {
            assert_eq!(
                (supporting, total, full_partitions),
                (0, 250, 0),
                "no candidate is a full partition, so the numerator is zero before any quorum is \
                 consulted — this is the campaign's line, and it is not a stake shortfall"
            );
            assert!(
                candidates > 0,
                "and the support map is not empty: the refusal is `0 full partitions`, not a map that \
                 never got built"
            );
        }
        other => panic!(
            "the derivation was expected to refuse with Support and did not: {other:?} (fringe \
             advanced: {})",
            fringe.is_some()
        ),
    }

    // **The separator.** Same three validators, same stakes, a fork-shaped DAG — and it publishes.
    let st2: DagMessageState<BlockHash, Validator> = DagMessageState::empty();
    let genesis2 = st2.create_message(id(255, 0), h(0), g, s(0), bonds.clone(), &BTreeSet::new());
    let mut state2 = st2.insert_msg(&genesis2);
    let mut previous: Vec<_> = Vec::new();
    for height in 1..=20 {
        let parents: std::collections::BTreeSet<_> = if previous.is_empty() {
            [genesis2.clone()].into_iter().collect()
        } else {
            previous.iter().cloned().collect()
        };
        let mut layer = std::collections::BTreeSet::new();
        for (i, v) in vs.iter().enumerate() {
            let m = state2.create_message(
                id(i as u8, height),
                h(height),
                v.clone(),
                s(height),
                bonds.clone(),
                &parents,
            );
            state2 = state2.insert_msg(&m);
            layer.insert(m);
        }
        previous = layer.iter().cloned().collect();
    }
    let justifications2: std::collections::BTreeSet<_> = previous.iter().cloned().collect();
    let finalizer2 = Finalizer::new(&state2.msg_map);
    let (_p2, fringe2, why2) =
        liveness::calculate_finalization_detailed(&finalizer2, &justifications2, &bonds);
    assert!(
        fringe2.is_some(),
        "the same validators at the same stakes on a fork-shaped DAG publish a fringe — so the variable \
         is the parent set the proposer chooses, not the stake split and not a departed validator (got \
         {why2:?})"
    );
}

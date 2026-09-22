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

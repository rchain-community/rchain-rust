//! Tuplespace-level reporting events (port of `trace/TuplespaceEvent.scala`).
//!
//! A [`TuplespaceEvent`] pairs an incoming operation with the operation it matched (if any), and
//! classifies each operation by polarity (send/receive) and cardinality (linear/non-linear/peek).

use std::collections::BTreeSet;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;

use crate::trace::event::{Comm, Consume, Produce};

/// The polarity of a tuplespace operation (port of `Polarity`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Polarity {
    Send,
    Receive,
}

/// The cardinality of a tuplespace operation (port of `Cardinality`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cardinality {
    Linear,
    NonLinear,
    Peek,
}

/// A single tuplespace operation (port of `TuplespaceOperation`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuplespaceOperation {
    pub polarity: Polarity,
    pub cardinality: Cardinality,
    pub event_hash: Blake2b256Hash,
}

/// An incoming operation paired with the operation it matched (port of `TuplespaceEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuplespaceEvent {
    pub incoming: TuplespaceOperation,
    pub matched: Option<TuplespaceOperation>,
}

fn to_operation_produce(produce: &Produce) -> TuplespaceOperation {
    TuplespaceOperation {
        polarity: Polarity::Send,
        cardinality: if produce.persistent {
            Cardinality::NonLinear
        } else {
            Cardinality::Linear
        },
        event_hash: produce.hash,
    }
}

fn to_operation_consume(consume: &Consume, peeks: bool) -> TuplespaceOperation {
    TuplespaceOperation {
        polarity: Polarity::Receive,
        cardinality: if consume.persistent {
            Cardinality::NonLinear
        } else if peeks {
            Cardinality::Peek
        } else {
            Cardinality::Linear
        },
        event_hash: consume.hash,
    }
}

impl TuplespaceEvent {
    /// Build an event from a produce (port of `TuplespaceEvent.from(produce)`).
    pub fn from_produce(produce: &Produce) -> (Blake2b256Hash, TuplespaceEvent) {
        (
            produce.channels_hash,
            TuplespaceEvent {
                incoming: to_operation_produce(produce),
                matched: None,
            },
        )
    }

    /// Build an event from a single-channel consume (port of `TuplespaceEvent.from(consume)`).
    pub fn from_consume(consume: &Consume) -> Option<(Blake2b256Hash, TuplespaceEvent)> {
        if let [single_channel_hash] = consume.channels_hashes.as_slice() {
            Some((
                *single_channel_hash,
                TuplespaceEvent {
                    incoming: to_operation_consume(consume, false),
                    matched: None,
                },
            ))
        } else {
            None
        }
    }

    /// Build an event from a single-produce COMM (port of `TuplespaceEvent.from(comm, incoming)`).
    pub fn from_comm(
        comm: &Comm,
        incoming_consumes: &BTreeSet<Consume>,
    ) -> Option<(Blake2b256Hash, TuplespaceEvent)> {
        if comm.produces.len() != 1 {
            return None;
        }
        let produce = &comm.produces[0];
        let produce_op = to_operation_produce(produce);
        let consume_op = to_operation_consume(&comm.consume, !comm.peeks.is_empty());

        let peek_initiated = comm.times_repeated.get(produce).copied().unwrap_or(0) != 0;
        let incoming = if incoming_consumes.contains(&comm.consume) && !peek_initiated {
            consume_op.clone()
        } else {
            produce_op.clone()
        };
        let matched = if incoming == produce_op {
            consume_op
        } else {
            produce_op
        };
        Some((
            produce.channels_hash,
            TuplespaceEvent {
                incoming,
                matched: Some(matched),
            },
        ))
    }

    /// Whether this event was left unsatisfied (port of `TuplespaceEventOps.unsatisfied`).
    pub fn unsatisfied(&self) -> bool {
        match self.incoming.cardinality {
            Cardinality::Peek => self.matched.is_none(),
            Cardinality::Linear => self
                .matched
                .as_ref()
                .map_or(true, |m| m.cardinality == Cardinality::Peek),
            Cardinality::NonLinear => self
                .matched
                .as_ref()
                .map_or(true, |m| m.cardinality != Cardinality::NonLinear),
        }
    }

    /// Whether this event conflicts with another (port of `TuplespaceEventOps.conflicts`).
    pub fn conflicts(&self, other: &TuplespaceEvent) -> bool {
        if self.incoming.polarity == other.incoming.polarity {
            let both_peeks = self.incoming.cardinality == Cardinality::Peek
                && other.incoming.cardinality == Cardinality::Peek;
            let both_matched_same_non_persistent = match (&self.matched, &other.matched) {
                (Some(this_matched), Some(other_matched)) => Some(
                    this_matched == other_matched
                        && other_matched.cardinality != Cardinality::NonLinear,
                ),
                _ => None,
            };
            if both_peeks {
                false
            } else {
                both_matched_same_non_persistent.unwrap_or(false)
            }
        } else {
            self.unsatisfied() && other.unsatisfied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::trace::event::Comm;

    fn produce(byte: u8, persistent: bool) -> Produce {
        let mut p = Produce::apply(&format!("chan{byte}"), &format!("datum{byte}"), persistent);
        // Distinct hashes per byte so `unsatisfied`/`conflicts` can tell events apart.
        p.hash = Blake2b256Hash::from_bytes([byte; 32]);
        p.channels_hash = Blake2b256Hash::from_bytes([byte; 32]);
        p
    }

    fn consume(byte: u8, persistent: bool) -> Consume {
        let mut c = Consume::apply(
            &[format!("chan{byte}")],
            &[format!("pat{byte}")],
            &format!("cont{byte}"),
            persistent,
        );
        c.hash = Blake2b256Hash::from_bytes([byte; 32]);
        c.channels_hashes = vec![Blake2b256Hash::from_bytes([byte; 32])];
        c
    }

    /// A COMM built structurally: `from_comm` reads only these fields, so no `Comm::apply`
    /// bookkeeping is needed to test the event mapping.
    fn comm(produce_byte: u8, consume_byte: u8, peeks: &[usize]) -> Comm {
        // `times_repeated` is the produce-counter map: a peeked produce has been copied at least
        // once, which is exactly what `from_comm` reads to decide whether the peek initiated.
        let mut times_repeated = BTreeMap::new();
        if !peeks.is_empty() {
            times_repeated.insert(produce(produce_byte, false), 1usize);
        }
        Comm {
            consume: consume(consume_byte, false),
            produces: vec![produce(produce_byte, false)],
            peeks: peeks.iter().copied().collect(),
            times_repeated,
        }
    }

    /// A produce is a **send** whose cardinality is linear or non-linear by its persistence, keyed by
    /// its channels hash; it matches nothing.
    #[test]
    fn a_produce_becomes_a_send_with_the_right_cardinality() {
        let (key, event) = TuplespaceEvent::from_produce(&produce(1, false));
        assert_eq!(key, produce(1, false).channels_hash);
        assert_eq!(event.incoming.polarity, Polarity::Send);
        assert_eq!(event.incoming.cardinality, Cardinality::Linear);
        assert_eq!(event.matched, None);

        let (_, persistent) = TuplespaceEvent::from_produce(&produce(2, true));
        assert_eq!(persistent.incoming.cardinality, Cardinality::NonLinear);
    }

    /// A consume is a **receive**; a *multi-channel* consume has no single key, so it yields no event
    /// (the reporter only reports single-channel operations).
    #[test]
    fn a_consume_becomes_a_receive_and_a_multi_channel_one_yields_nothing() {
        let (key, event) =
            TuplespaceEvent::from_consume(&consume(1, false)).expect("single channel");
        assert_eq!(key, consume(1, false).channels_hashes[0]);
        assert_eq!(event.incoming.polarity, Polarity::Receive);
        assert_eq!(event.incoming.cardinality, Cardinality::Linear);

        let mut two = consume(2, false);
        two.channels_hashes
            .push(Blake2b256Hash::from_bytes([9u8; 32]));
        assert_eq!(TuplespaceEvent::from_consume(&two), None);
    }

    /// **A COMM with more than one produce has no single key** and is not reported; with one produce
    /// the event is keyed by that produce's channels hash and carries both sides.
    #[test]
    fn a_comm_with_one_produce_is_reported_and_a_multi_produce_one_is_not() {
        let c = comm(1, 2, &[]);
        let (key, event) = TuplespaceEvent::from_comm(&c, &BTreeSet::new()).expect("one produce");
        assert_eq!(key, produce(1, false).channels_hash);
        let matched = event
            .matched
            .clone()
            .expect("a COMM always carries the other side");
        assert_eq!(
            event.incoming.polarity,
            Polarity::Send,
            "no incoming consume: the produce leads"
        );
        assert_eq!(
            matched.polarity,
            Polarity::Receive,
            "…and the consume is what it matched"
        );

        let mut two = comm(1, 2, &[]);
        two.produces.push(produce(3, false));
        assert_eq!(TuplespaceEvent::from_comm(&two, &BTreeSet::new()), None);
    }

    /// A **peek-initiated** COMM reports the produce as incoming (the peek did not consume), while a
    /// consume-initiated one reports the consume.
    #[test]
    fn the_incoming_side_depends_on_who_initiated() {
        let c = comm(1, 2, &[]);
        let mut consume_initiated: BTreeSet<crate::trace::event::Consume> = BTreeSet::new();
        consume_initiated.insert(comm(1, 2, &[]).consume);
        let (_, event) = TuplespaceEvent::from_comm(&c, &consume_initiated).expect("event");
        assert_eq!(event.incoming.polarity, Polarity::Receive);

        // No incoming consume recorded: the produce is the incoming side.
        let (_, event) = TuplespaceEvent::from_comm(&c, &BTreeSet::new()).expect("event");
        assert_eq!(event.incoming.polarity, Polarity::Send);

        // A peek is `times_repeated != 0` for its produce: the produce stays incoming even though the
        // consume is in the incoming set.
        let peeking = comm(1, 2, &[0]);
        let mut with_consume: BTreeSet<crate::trace::event::Consume> = BTreeSet::new();
        with_consume.insert(comm(1, 2, &[]).consume);
        let (_, event) = TuplespaceEvent::from_comm(&peeking, &with_consume).expect("event");
        assert_eq!(
            event.incoming.polarity,
            Polarity::Send,
            "a peek copies, it does not consume"
        );
    }

    /// `unsatisfied` (Law 9's merge signal): a linear operation is satisfied only by a non-peek match,
    /// a peek is satisfied by *any* match (it never consumes), and an unmatched event is unsatisfied.
    #[test]
    fn unsatisfied_distinguishes_linear_peek_and_non_linear() {
        let op = |polarity: &Polarity, cardinality: &Cardinality| TuplespaceOperation {
            polarity: polarity.clone(),
            cardinality: cardinality.clone(),
            event_hash: Blake2b256Hash::from_bytes([0u8; 32]),
        };
        let ev = |incoming: TuplespaceOperation, matched: Option<TuplespaceOperation>| {
            TuplespaceEvent { incoming, matched }
        };

        let linear = Cardinality::Linear;
        let peek = Cardinality::Peek;
        let non_linear = Cardinality::NonLinear;
        let send = Polarity::Send;
        let recv = Polarity::Receive;

        // Linear: satisfied by a linear or non-linear match, unsatisfied by a peek match or none.
        assert!(!ev(op(&send, &linear), Some(op(&recv, &linear))).unsatisfied());
        assert!(!ev(op(&send, &linear), Some(op(&recv, &non_linear))).unsatisfied());
        assert!(ev(op(&send, &linear), Some(op(&recv, &peek))).unsatisfied());
        assert!(ev(op(&send, &linear), None).unsatisfied());
        // Peek: satisfied by any match at all.
        assert!(!ev(op(&send, &peek), Some(op(&recv, &peek))).unsatisfied());
        assert!(ev(op(&send, &peek), None).unsatisfied());
        // Non-linear: satisfied only by a non-linear match.
        assert!(!ev(op(&recv, &non_linear), Some(op(&send, &non_linear))).unsatisfied());
        assert!(ev(op(&recv, &non_linear), Some(op(&send, &linear))).unsatisfied());
        assert!(ev(op(&recv, &non_linear), None).unsatisfied());
    }

    /// `conflicts` is the pairing the merge uses: two operations of the **same polarity** conflict
    /// only when they matched the same non-persistent partner (two peeks never do); two of *opposite*
    /// polarity conflict when both are unsatisfied (they are competing for the same datum).
    #[test]
    fn conflicts_covers_the_same_polarity_and_opposite_polarity_cases() {
        let op = |polarity: &Polarity, cardinality: &Cardinality, byte: u8| TuplespaceOperation {
            polarity: polarity.clone(),
            cardinality: cardinality.clone(),
            event_hash: Blake2b256Hash::from_bytes([byte; 32]),
        };
        let send = Polarity::Send;
        let recv = Polarity::Receive;
        let linear = Cardinality::Linear;
        let peek = Cardinality::Peek;

        // Same polarity, same matched partner, non-persistent partner -> conflict.
        let shared = op(&recv, &linear, 7);
        let a = TuplespaceEvent {
            incoming: op(&send, &linear, 1),
            matched: Some(shared.clone()),
        };
        let b = TuplespaceEvent {
            incoming: op(&send, &linear, 2),
            matched: Some(shared.clone()),
        };
        assert!(
            a.conflicts(&b),
            "both matched the same non-persistent partner"
        );

        // Same polarity, *different* partners -> no conflict.
        let c = TuplespaceEvent {
            incoming: op(&send, &linear, 3),
            matched: Some(op(&recv, &linear, 8)),
        };
        assert!(!a.conflicts(&c));

        // Same polarity, both peeks -> never a conflict (a peek consumes nothing).
        let p1 = TuplespaceEvent {
            incoming: op(&send, &peek, 4),
            matched: Some(shared.clone()),
        };
        let p2 = TuplespaceEvent {
            incoming: op(&send, &peek, 5),
            matched: Some(shared.clone()),
        };
        assert!(!p1.conflicts(&p2));

        // Opposite polarity, both unsatisfied -> conflict; one satisfied -> none.
        let unsatisfied = TuplespaceEvent {
            incoming: op(&send, &linear, 6),
            matched: None,
        };
        // Two unsatisfied operations of *opposite* polarity compete for the same datum: each is
        // looking for a partner the other could have been.
        let competing = TuplespaceEvent {
            incoming: op(&recv, &linear, 9),
            matched: None,
        };
        assert!(
            unsatisfied.conflicts(&competing),
            "a waiting send and a waiting receive compete"
        );
        assert!(
            competing.conflicts(&unsatisfied),
            "…and the relation is symmetric"
        );

        // A *satisfied* receive is not competing with anything: `unsatisfied()` is false, so the
        // opposite-polarity arm cannot fire.
        let satisfied_receive = TuplespaceEvent {
            incoming: op(&recv, &linear, 11),
            matched: Some(op(&send, &linear, 12)),
        };
        assert!(!unsatisfied.conflicts(&satisfied_receive));
        assert!(
            !unsatisfied.conflicts(&unsatisfied),
            "same polarity: the matched-partner rule applies, not the unsatisfied one"
        );
    }
}

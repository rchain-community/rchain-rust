//! Transforming reporting events into more readable forms (port of `ReportingTransformer.scala`).

use crate::reporting_rspace::{ReportingComm, ReportingConsume, ReportingEvent, ReportingProduce};

/// Transforms [`ReportingEvent`]s into some target type `E` (port of `ReportingTransformer`).
pub trait ReportingTransformer<C, P, A, K, E> {
    fn serialize_consume(&self, rc: &ReportingConsume<C, P, K>) -> E;
    fn serialize_produce(&self, rp: &ReportingProduce<C, A>) -> E;
    fn serialize_comm(&self, rc: &ReportingComm<C, P, A, K>) -> E;

    fn transform_event(&self, re: &ReportingEvent<C, P, A, K>) -> E {
        match re {
            ReportingEvent::Comm(comm) => self.serialize_comm(comm),
            ReportingEvent::Consume(cons) => self.serialize_consume(cons),
            ReportingEvent::Produce(prod) => self.serialize_produce(prod),
        }
    }
}

/// Stringified reporting events (port of `ReportingRhoStringTransformer.RhoEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RhoEvent {
    Comm(RhoComm),
    Produce(RhoProduce),
    Consume(RhoConsume),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RhoComm {
    pub consume: RhoConsume,
    pub produces: Vec<RhoProduce>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RhoProduce {
    pub channel: String,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RhoConsume {
    pub channels: String,
    pub patterns: String,
    pub continuation: String,
}

/// A [`ReportingTransformer`] that renders events as strings (port of
/// `ReportingEventStringTransformer`).
pub struct ReportingEventStringTransformer<C, P, A, K> {
    pub serialize_c: fn(&C) -> String,
    pub serialize_p: fn(&P) -> String,
    pub serialize_a: fn(&A) -> String,
    pub serialize_k: fn(&K) -> String,
}

impl<C, P, A, K> ReportingTransformer<C, P, A, K, RhoEvent>
    for ReportingEventStringTransformer<C, P, A, K>
{
    fn serialize_consume(&self, rc: &ReportingConsume<C, P, K>) -> RhoEvent {
        RhoEvent::Consume(self.consume_to_rho(rc))
    }

    fn serialize_produce(&self, rp: &ReportingProduce<C, A>) -> RhoEvent {
        RhoEvent::Produce(self.produce_to_rho(rp))
    }

    fn serialize_comm(&self, rc: &ReportingComm<C, P, A, K>) -> RhoEvent {
        let consume = self.consume_to_rho(&rc.consume);
        let produces = rc
            .produces
            .iter()
            .map(|rp| self.produce_to_rho(rp))
            .collect();
        RhoEvent::Comm(RhoComm { consume, produces })
    }
}

impl<C, P, A, K> ReportingEventStringTransformer<C, P, A, K> {
    fn consume_to_rho(&self, rc: &ReportingConsume<C, P, K>) -> RhoConsume {
        let k = (self.serialize_k)(&rc.continuation);
        let chs = rc
            .channels
            .iter()
            .map(self.serialize_c)
            .collect::<Vec<_>>()
            .join(";");
        let ps = rc
            .patterns
            .iter()
            .map(self.serialize_p)
            .collect::<Vec<_>>()
            .join(";");
        RhoConsume {
            channels: format!("[{}]", chs),
            patterns: format!("[{}]", ps),
            continuation: k,
        }
    }

    fn produce_to_rho(&self, rp: &ReportingProduce<C, A>) -> RhoProduce {
        RhoProduce {
            channel: (self.serialize_c)(&rp.channel),
            data: (self.serialize_a)(&rp.data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transformer over `String` payloads that shows which serializer produced which field: each
    /// function tags its input, so a dispatch that routed a produce to the consume serializer, or
    /// swapped two fields, is visible in the output rather than hidden by identical strings.
    fn transformer() -> ReportingEventStringTransformer<String, String, String, String> {
        ReportingEventStringTransformer {
            serialize_c: |c| format!("c:{c}"),
            serialize_p: |p| format!("p:{p}"),
            serialize_a: |a| format!("a:{a}"),
            serialize_k: |k| format!("k:{k}"),
        }
    }

    fn consume() -> ReportingConsume<String, String, String> {
        ReportingConsume {
            channels: vec!["ch1".to_string(), "ch2".to_string()],
            patterns: vec!["pat".to_string()],
            continuation: "body".to_string(),
            peeks: vec![1],
        }
    }

    fn produce() -> ReportingProduce<String, String> {
        ReportingProduce {
            channel: "ch1".to_string(),
            data: "datum".to_string(),
        }
    }

    /// `transform_event` dispatches on the variant: each of the three reaches its own serializer.
    /// The three arms are one line apart, so a copy-paste that sends a produce down the consume arm
    /// is exactly the mistake this pins.
    #[test]
    fn transform_event_dispatches_each_variant_to_its_own_serializer() {
        let t = transformer();

        assert_eq!(
            t.transform_event(&ReportingEvent::Consume(consume())),
            RhoEvent::Consume(RhoConsume {
                channels: "[c:ch1;c:ch2]".to_string(),
                patterns: "[p:pat]".to_string(),
                continuation: "k:body".to_string(),
            })
        );
        assert_eq!(
            t.transform_event(&ReportingEvent::Produce(produce())),
            RhoEvent::Produce(RhoProduce {
                channel: "c:ch1".to_string(),
                data: "a:datum".to_string(),
            })
        );
        assert_eq!(
            t.transform_event(&ReportingEvent::Comm(ReportingComm {
                consume: consume(),
                produces: vec![produce()],
            })),
            RhoEvent::Comm(RhoComm {
                consume: RhoConsume {
                    channels: "[c:ch1;c:ch2]".to_string(),
                    patterns: "[p:pat]".to_string(),
                    continuation: "k:body".to_string(),
                },
                produces: vec![RhoProduce {
                    channel: "c:ch1".to_string(),
                    data: "a:datum".to_string(),
                }],
            })
        );
    }

    /// The list formatting is the Scala's `ReportingRhoStringTransformer`: the channels and patterns
    /// are joined with `;` inside square brackets, and an **empty** list is `[]` rather than an empty
    /// string — a distinction a client parsing the report depends on.
    #[test]
    fn lists_are_bracketed_and_semicolon_joined_including_when_empty() {
        let t = transformer();
        let empty: ReportingConsume<String, String, String> = ReportingConsume {
            channels: Vec::new(),
            patterns: Vec::new(),
            continuation: "k".to_string(),
            peeks: Vec::new(),
        };
        assert_eq!(
            t.transform_event(&ReportingEvent::Consume(empty)),
            RhoEvent::Consume(RhoConsume {
                channels: "[]".to_string(),
                patterns: "[]".to_string(),
                continuation: "k:k".to_string(),
            })
        );

        // One element is not a special case, and a three-element list keeps its order.
        let many: ReportingConsume<String, String, String> = ReportingConsume {
            channels: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            patterns: vec!["x".to_string()],
            continuation: "cont".to_string(),
            peeks: vec![],
        };
        match t.transform_event(&ReportingEvent::Consume(many)) {
            RhoEvent::Consume(c) => {
                assert_eq!(c.channels, "[c:a;c:b;c:c]");
                assert_eq!(c.patterns, "[p:x]");
                assert_eq!(c.continuation, "k:cont");
            }
            other => panic!("expected a consume, got {other:?}"),
        }
    }

    /// The COMM keeps every produce, **in order**: a COMM that matched two data on one channel
    /// reports both, and a report that reordered or dropped one would misrepresent the match.
    #[test]
    fn a_comm_reports_all_of_its_produces_in_order() {
        let t = transformer();
        let comm = ReportingComm {
            consume: consume(),
            produces: vec![
                ReportingProduce {
                    channel: "ch2".to_string(),
                    data: "second".to_string(),
                },
                ReportingProduce {
                    channel: "ch1".to_string(),
                    data: "first".to_string(),
                },
            ],
        };
        match t.transform_event(&ReportingEvent::Comm(comm)) {
            RhoEvent::Comm(rho) => {
                assert_eq!(rho.produces.len(), 2, "no produce is dropped");
                assert_eq!(rho.produces[0].data, "a:second");
                assert_eq!(rho.produces[1].data, "a:first");
                assert_eq!(rho.produces[0].channel, "c:ch2");
            }
            other => panic!("expected a comm, got {other:?}"),
        }
    }
}

//! CapTP operations and descriptors.
//!
//! The shapes here are taken from the **reference implementation**, not the CapTP prose, for the
//! reason recorded as AUDIT C216: the prose and `ocapn-test-suite/utils/captp_types.py` disagree,
//! and the implementation is what every peer actually speaks. Concretely, that file builds
//!
//! * a descriptor as `syrup.Record(syrup.Symbol(<label>), [position])` — a record whose single
//!   argument is a plain integer, `<desc:export 0>`;
//! * `op:deliver` as `syrup.Record(syrup.Symbol("op:deliver"), [to, args, answer_position,
//!   resolve_me_desc])`, where an absent `answer_position` or `resolve_me_desc` is the Python
//!   `False` (the Syrup boolean `f`), and `to` is asserted to be a `desc:export` or `desc:answer`.
//!
//! The bootstrap object is exported at position 0 on every session, and — because descriptors are
//! written from the receiver's point of view — a peer addresses the *remote's* bootstrap as
//! `<desc:export 0>`, which the receiver resolves in its own **export** table.

use std::fmt;

use num_bigint::{BigInt, BigUint, Sign};

use crate::syrup::Value;

/// `op:deliver`'s record label.
pub const DELIVER_LABEL: &str = "op:deliver";
/// The four descriptor labels.
pub const IMPORT_OBJECT_LABEL: &str = "desc:import-object";
pub const IMPORT_PROMISE_LABEL: &str = "desc:import-promise";
pub const EXPORT_LABEL: &str = "desc:export";
pub const ANSWER_LABEL: &str = "desc:answer";
/// The position the bootstrap object is always exported at.
pub const BOOTSTRAP_POSITION: u64 = 0;

/// A reference to an object or promise within one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Desc {
    /// A position in the sender's import table — an object it is handing over.
    ImportObject(BigUint),
    /// A position in the sender's import table that names a promise.
    ImportPromise(BigUint),
    /// A position in the *receiver's* export table: "the object you exported at N". This is how a
    /// peer addresses another's bootstrap, as `Export(0)`.
    Export(BigUint),
    /// A position in the sender's table of answers to its own prior deliveries.
    Answer(BigUint),
}

impl Desc {
    pub fn position(&self) -> &BigUint {
        match self {
            Desc::ImportObject(p) | Desc::ImportPromise(p) | Desc::Export(p) | Desc::Answer(p) => p,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Desc::ImportObject(_) => IMPORT_OBJECT_LABEL,
            Desc::ImportPromise(_) => IMPORT_PROMISE_LABEL,
            Desc::Export(_) => EXPORT_LABEL,
            Desc::Answer(_) => ANSWER_LABEL,
        }
    }

    pub fn to_syrup(&self) -> Value {
        Value::Record(vec![
            Value::Symbol(self.label().to_string()),
            Value::Int(BigInt::from(self.position().clone())),
        ])
    }

    pub fn from_syrup(v: &Value) -> Result<Desc, CaptpError> {
        let Value::Record(fields) = v else {
            return Err(CaptpError::NotADescriptor);
        };
        let [label, pos] = fields.as_slice() else {
            return Err(CaptpError::NotADescriptor);
        };
        let Value::Symbol(label) = label else {
            return Err(CaptpError::NotADescriptor);
        };
        let pos = position_from_value(pos)?;
        match label.as_str() {
            IMPORT_OBJECT_LABEL => Ok(Desc::ImportObject(pos)),
            IMPORT_PROMISE_LABEL => Ok(Desc::ImportPromise(pos)),
            EXPORT_LABEL => Ok(Desc::Export(pos)),
            ANSWER_LABEL => Ok(Desc::Answer(pos)),
            _ => Err(CaptpError::NotADescriptor),
        }
    }
}

/// Whether a label names a descriptor rather than an operation.
pub fn is_descriptor_label(label: &str) -> bool {
    matches!(
        label,
        IMPORT_OBJECT_LABEL | IMPORT_PROMISE_LABEL | EXPORT_LABEL | ANSWER_LABEL
    )
}

/// A value that is a descriptor, as `(label, position)`.
///
/// Public because the bridge has to recognise one *inside a message's arguments* (C224 item 4): a
/// capability a peer passes is a descriptor, and which descriptor it is decides whether the node can
/// name the object on chain.
pub fn descriptor_of(v: &Value) -> Option<(String, BigUint)> {
    let Value::Record(fields) = v else {
        return None;
    };
    let [Value::Symbol(label), Value::Int(position)] = fields.as_slice() else {
        return None;
    };
    if !is_descriptor_label(label) || position.sign() == Sign::Minus {
        return None;
    }
    Some((label.clone(), position.magnitude().clone()))
}

/// `op:deliver` — "delivers a message to an object or promise".
#[derive(Debug, Clone, PartialEq)]
pub struct Deliver {
    /// Where the message goes. The reference asserts this is a `desc:export` or `desc:answer`.
    pub to: Desc,
    /// The message's arguments.
    pub args: Vec<Value>,
    /// `None` is the wire's `false`: no result is expected. A `Some` position is a **pipelining**
    /// peer asking us to remember what this delivery resolves to, so a later `<desc:answer N>`
    /// reaches it — the answer table, in `conn.rs`.
    pub answer_pos: Option<BigUint>,
    /// `None` is the wire's `false`. The reference constrains this to an import descriptor.
    pub resolve_me_desc: Option<Desc>,
}

impl Deliver {
    pub fn to_syrup(&self) -> Value {
        Value::Record(vec![
            Value::Symbol(DELIVER_LABEL.to_string()),
            self.to.to_syrup(),
            Value::List(self.args.clone()),
            optional_position(self.answer_pos.as_ref()),
            match &self.resolve_me_desc {
                Some(d) => d.to_syrup(),
                None => Value::Bool(false),
            },
        ])
    }

    pub fn from_syrup(v: &Value) -> Result<Deliver, CaptpError> {
        let Value::Record(fields) = v else {
            return Err(CaptpError::NotAnOp(DELIVER_LABEL));
        };
        let [label, to, args, answer, resolve] = fields.as_slice() else {
            return Err(CaptpError::NotAnOp(DELIVER_LABEL));
        };
        if !matches!(label, Value::Symbol(s) if s == DELIVER_LABEL) {
            return Err(CaptpError::NotAnOp(DELIVER_LABEL));
        }
        let to = Desc::from_syrup(to)?;
        if !matches!(to, Desc::Export(_) | Desc::Answer(_)) {
            return Err(CaptpError::BadField("to"));
        }
        let Value::List(args) = args else {
            return Err(CaptpError::BadField("args"));
        };
        let answer_pos = match answer {
            Value::Bool(false) => None,
            other => Some(position_from_value(other)?),
        };
        let resolve_me_desc = match resolve {
            Value::Bool(false) => None,
            other => {
                let d = Desc::from_syrup(other)?;
                if !matches!(d, Desc::ImportObject(_) | Desc::ImportPromise(_)) {
                    return Err(CaptpError::BadField("resolve-me-desc"));
                }
                Some(d)
            }
        };
        Ok(Deliver {
            to,
            args: args.clone(),
            answer_pos,
            resolve_me_desc,
        })
    }
}

/// `op:listen`'s record label.
pub const LISTEN_LABEL: &str = "op:listen";

/// `op:listen` — "request notification on a promise": deliver `[<fulfill …>]` or `[<break …>]` to
/// `resolve-me-desc` when the promise at `to` settles, with `wants-partial` when it should be told
/// of partial resolutions too.
///
/// The reference's `OpListen` carries **three** arguments — `to`, `resolve_me_desc`,
/// `wants_partial` — where the CapTP prose lists two. The implementation wins (AUDIT C216).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpListen {
    pub to: Desc,
    /// Where the notification goes.
    pub resolve_me_desc: Desc,
    pub wants_partial: bool,
}

impl OpListen {
    pub fn to_syrup(&self) -> Value {
        Value::Record(vec![
            Value::Symbol(LISTEN_LABEL.to_string()),
            self.to.to_syrup(),
            self.resolve_me_desc.to_syrup(),
            Value::Bool(self.wants_partial),
        ])
    }

    pub fn from_syrup(v: &Value) -> Result<OpListen, CaptpError> {
        let Value::Record(fields) = v else {
            return Err(CaptpError::NotAnOp(LISTEN_LABEL));
        };
        let [label, to, resolve_me, wants_partial] = fields.as_slice() else {
            return Err(CaptpError::NotAnOp(LISTEN_LABEL));
        };
        if !matches!(label, Value::Symbol(s) if s == LISTEN_LABEL) {
            return Err(CaptpError::NotAnOp(LISTEN_LABEL));
        }
        let Value::Bool(wants_partial) = wants_partial else {
            return Err(CaptpError::BadField("wants-partial"));
        };
        Ok(OpListen {
            to: Desc::from_syrup(to)?,
            resolve_me_desc: Desc::from_syrup(resolve_me)?,
            wants_partial: *wants_partial,
        })
    }
}

/// `op:gc-exports`'s record label.
pub const GC_EXPORTS_LABEL: &str = "op:gc-exports";
/// `op:gc-answers`'s record label.
pub const GC_ANSWERS_LABEL: &str = "op:gc-answers";

/// `op:gc-exports` — "tell the peer which of its exports we have released, and how many references
/// we held". The positions are the peer's export positions, and each wire delta is how many times
/// we received that reference since we last said so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpGcExports {
    pub positions: Vec<BigUint>,
    pub wire_deltas: Vec<BigUint>,
}

impl OpGcExports {
    pub fn to_syrup(&self) -> Value {
        Value::Record(vec![
            Value::Symbol(GC_EXPORTS_LABEL.to_string()),
            Value::List(self.positions.iter().cloned().map(position_value).collect()),
            Value::List(
                self.wire_deltas
                    .iter()
                    .cloned()
                    .map(position_value)
                    .collect(),
            ),
        ])
    }

    /// Read the peer's release of *our* exports (AUDIT C223).
    ///
    /// This side of the shape did not exist: the struct carried only `to_syrup`, so an inbound
    /// `op:gc-exports` could not be read at all and the session dropped it by label — a peer's
    /// explicit release was a no-op, and only the table caps bounded the export table. The deltas are
    /// accepted and not used: they are the peer's own reference accounting, and this side releases a
    /// position outright rather than counting down.
    pub fn from_syrup(v: &Value) -> Result<OpGcExports, String> {
        let Value::Record(fields) = v else {
            return Err("an op:gc-exports is a record".to_string());
        };
        let [label, positions, wire_deltas] = fields.as_slice() else {
            return Err("an op:gc-exports has two fields".to_string());
        };
        if !matches!(label, Value::Symbol(s) if s == GC_EXPORTS_LABEL) {
            return Err(format!("not a {GC_EXPORTS_LABEL} record: {label:?}"));
        }
        Ok(OpGcExports {
            positions: positions_from_syrup(positions, "positions")?,
            wire_deltas: positions_from_syrup(wire_deltas, "wire-deltas")?,
        })
    }
}

/// `op:gc-answers` — "tell the peer which of our answer positions we have released".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpGcAnswers {
    pub positions: Vec<BigUint>,
}

impl OpGcAnswers {
    pub fn to_syrup(&self) -> Value {
        Value::Record(vec![
            Value::Symbol(GC_ANSWERS_LABEL.to_string()),
            Value::List(self.positions.iter().cloned().map(position_value).collect()),
        ])
    }

    /// Read the peer's release of *our* answer positions (AUDIT C223) — the inbound half that did not
    /// exist, so that a release the peer asks for is honoured rather than dropped by label.
    pub fn from_syrup(v: &Value) -> Result<OpGcAnswers, String> {
        let Value::Record(fields) = v else {
            return Err("an op:gc-answers is a record".to_string());
        };
        let [label, positions] = fields.as_slice() else {
            return Err("an op:gc-answers has one field".to_string());
        };
        if !matches!(label, Value::Symbol(s) if s == GC_ANSWERS_LABEL) {
            return Err(format!("not a {GC_ANSWERS_LABEL} record: {label:?}"));
        }
        Ok(OpGcAnswers {
            positions: positions_from_syrup(positions, "positions")?,
        })
    }
}

/// A list of non-negative positions, as the gc shapes carry them.
fn positions_from_syrup(v: &Value, what: &str) -> Result<Vec<BigUint>, String> {
    let Value::List(items) = v else {
        return Err(format!("op:gc {what} is not a list"));
    };
    items
        .iter()
        .map(|item| match item {
            Value::Int(n) if n.sign() != num_bigint::Sign::Minus => Ok(n.magnitude().clone()),
            other => Err(format!(
                "op:gc {what} holds something that is not a position: {other:?}"
            )),
        })
        .collect()
}

fn position_value(n: BigUint) -> Value {
    Value::Int(BigInt::from(n))
}

fn optional_position(pos: Option<&BigUint>) -> Value {
    match pos {
        Some(p) => Value::Int(BigInt::from(p.clone())),
        None => Value::Bool(false),
    }
}

/// A position is a non-negative Syrup integer; a negative one has no meaning.
fn position_from_value(v: &Value) -> Result<BigUint, CaptpError> {
    match v {
        Value::Int(n) if n.sign() != Sign::Minus => Ok(n.magnitude().clone()),
        _ => Err(CaptpError::BadPosition),
    }
}

/// A CapTP value that is not the shape its label promises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptpError {
    /// Not a `<desc:…>` record, or a label no descriptor uses.
    NotADescriptor,
    /// Not the named operation's record.
    NotAnOp(&'static str),
    /// A field held the wrong Syrup type or the wrong descriptor kind; names the field.
    BadField(&'static str),
    /// A position was not a non-negative integer.
    BadPosition,
}

impl fmt::Display for CaptpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptpError::NotADescriptor => write!(f, "captp: not a `<desc:…>` record"),
            CaptpError::NotAnOp(op) => write!(f, "captp: not an `<{op} …>` record"),
            CaptpError::BadField(name) => write!(f, "captp: field {name:?} has the wrong type"),
            CaptpError::BadPosition => write!(f, "captp: position is not a non-negative integer"),
        }
    }
}

impl std::error::Error for CaptpError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(n: u64) -> BigUint {
        BigUint::from(n)
    }

    #[test]
    fn descriptor_kat() {
        assert_eq!(
            Desc::Export(pos(0)).to_syrup().to_bytes(),
            b"<11'desc:export0+>".to_vec()
        );
        assert_eq!(
            Desc::ImportObject(pos(17)).to_syrup().to_bytes(),
            b"<18'desc:import-object17+>".to_vec()
        );
        assert_eq!(
            Desc::Answer(pos(3)).to_syrup().to_bytes(),
            b"<11'desc:answer3+>".to_vec()
        );
    }

    #[test]
    fn descriptor_round_trips() {
        for d in [
            Desc::ImportObject(pos(0)),
            Desc::ImportPromise(pos(1)),
            Desc::Export(pos(2)),
            Desc::Answer(pos(3)),
        ] {
            assert_eq!(Desc::from_syrup(&d.to_syrup()).unwrap(), d);
        }
    }

    #[test]
    fn descriptor_refuses_the_wrong_shape() {
        assert_eq!(
            Desc::from_syrup(&Value::Symbol("desc:export".into())),
            Err(CaptpError::NotADescriptor)
        );
        assert_eq!(
            Desc::from_syrup(&Value::Record(vec![Value::Symbol("desc:nope".into())])),
            Err(CaptpError::NotADescriptor)
        );
        // A negative position is not a position.
        assert_eq!(
            Desc::from_syrup(&Value::Record(vec![
                Value::Symbol(EXPORT_LABEL.into()),
                Value::Int(BigInt::from(-1)),
            ])),
            Err(CaptpError::BadPosition)
        );
    }

    /// `<op:deliver <desc:export 0> ['fetch 12'swiss] false false>` in the reference's own example
    /// shape: a delivery to the remote bootstrap, no answer, no resolve-me.
    #[test]
    fn deliver_kat_to_bootstrap() {
        let d = Deliver {
            to: Desc::Export(pos(0)),
            args: vec![Value::Symbol("fetch".into()), Value::String("s".into())],
            answer_pos: None,
            resolve_me_desc: None,
        };
        assert_eq!(
            d.to_syrup().to_bytes(),
            b"<10'op:deliver<11'desc:export0+>[5'fetch1\"s]ff>".to_vec()
        );
        assert_eq!(Deliver::from_syrup(&d.to_syrup()).unwrap(), d);
    }

    #[test]
    fn deliver_round_trips_with_answer_and_resolve_me() {
        let d = Deliver {
            to: Desc::Answer(pos(3)),
            args: vec![Value::Symbol("drive".into())],
            answer_pos: Some(pos(4)),
            resolve_me_desc: Some(Desc::ImportObject(pos(17))),
        };
        let bytes = d.to_syrup().to_bytes();
        assert_eq!(
            Deliver::from_syrup(&Value::from_bytes(&bytes).unwrap()).unwrap(),
            d
        );
    }

    #[test]
    fn deliver_refuses_a_non_export_recipient() {
        // `to` must be an export or an answer; an import descriptor is not a recipient.
        let bad = Value::Record(vec![
            Value::Symbol(DELIVER_LABEL.into()),
            Desc::ImportObject(pos(1)).to_syrup(),
            Value::List(vec![]),
            Value::Bool(false),
            Value::Bool(false),
        ]);
        assert_eq!(Deliver::from_syrup(&bad), Err(CaptpError::BadField("to")));
    }

    #[test]
    fn deliver_refuses_an_export_as_resolve_me() {
        let bad = Value::Record(vec![
            Value::Symbol(DELIVER_LABEL.into()),
            Desc::Export(pos(0)).to_syrup(),
            Value::List(vec![]),
            Value::Bool(false),
            Desc::Export(pos(1)).to_syrup(),
        ]);
        assert_eq!(
            Deliver::from_syrup(&bad),
            Err(CaptpError::BadField("resolve-me-desc"))
        );
    }

    #[test]
    fn deliver_refuses_a_non_op_record() {
        assert_eq!(
            Deliver::from_syrup(&Value::List(vec![])),
            Err(CaptpError::NotAnOp(DELIVER_LABEL))
        );
        assert_eq!(
            Deliver::from_syrup(&Value::Record(vec![Value::Symbol("op:listen".into())])),
            Err(CaptpError::NotAnOp(DELIVER_LABEL))
        );
    }
}

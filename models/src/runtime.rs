//! Runtime wrapper types used by the evaluator (port of the protobuf `TaggedContinuation`,
//! `ParWithRandom`, `ListParWithRandom`, `BindPattern` messages).

use rchain_crypto::hash::blake2b512_random::Blake2b512Random;

use crate::ast::Var;
use crate::sorted::SortedProc;

/// A continuation: either rholang code or a reference to built-in code (port of `TaggedContinuation`).
#[derive(Clone, Debug)]
pub enum TaggedContinuation {
    ParBody(ParWithRandom),
    ScalaBodyRef(i64),
    Empty,
}

/// Rholang code plus the state of a split random generator (port of `ParWithRandom`).
#[derive(Clone, Debug)]
pub struct ParWithRandom {
    pub body: SortedProc,
    pub random_state: Blake2b512Random,
}

/// A list of canonically-sorted `Par`s plus a split random state (port of `ListParWithRandom`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListParWithRandom {
    pub pars: Vec<SortedProc>,
    pub random_state: Blake2b512Random,
}

/// A bound receive pattern (port of `BindPattern`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindPattern {
    pub patterns: Vec<SortedProc>,
    pub remainder: Option<Var>,
    pub free_count: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sorted::Sorted;

    /// The empty term: these tests are about the *field names*, not the terms, so the simplest
    /// canonically-sorted `Proc` is enough (the parser is exercised elsewhere).
    fn empty_proc() -> SortedProc {
        Sorted::new(Default::default())
    }

    /// The RSpace tuple types are a serde wire format, and the field names are camelCase
    /// (`randomState`, `freeCount`) because the format is shared with the Scala node's JSON
    /// encoders. A rename would be a wire break, so the shape is pinned here.
    #[test]
    fn the_tuple_types_serialize_with_camel_case_field_names() {
        let lpw = ListParWithRandom {
            pars: vec![empty_proc()],
            random_state: Blake2b512Random::default_random(),
        };
        assert_eq!(
            serde_json::to_value(&lpw).unwrap(),
            serde_json::json!({ "pars": [serde_json::to_value(empty_proc()).unwrap()], "randomState": null })
        );

        let pattern = BindPattern {
            patterns: vec![empty_proc()],
            remainder: None,
            free_count: 2,
        };
        let json = serde_json::to_value(&pattern).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "patterns": [serde_json::to_value(empty_proc()).unwrap()],
                "remainder": null,
                "freeCount": 2,
            })
        );
    }

    /// The random state is deliberately **not** carried (`null` out, a *fresh* generator back in),
    /// matching Scala's `encodeBlake2b512Random`/`decodeDummyBlake2b512Random`. The replay path
    /// re-derives its randomness from the recorded trace, so a tuple that claimed to carry a
    /// generator would be lying — and, because the replacement is `defaultRandom`, two decodes of
    /// the same bytes do not agree. Both halves are pinned: a "fix" that made the round trip
    /// faithful, or that made it deterministic, would change consensus behaviour.
    #[test]
    fn the_random_state_is_not_carried_through_serde() {
        let source = ListParWithRandom {
            pars: vec![empty_proc()],
            random_state: Blake2b512Random::from_init(&[7u8; 32]),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"randomState\":null"), "{json}");

        let back: ListParWithRandom = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pars, source.pars, "the terms survive the round trip");
        assert_ne!(
            back.random_state, source.random_state,
            "the encoded generator is not restored"
        );
        let again: ListParWithRandom = serde_json::from_str(&json).unwrap();
        assert_ne!(
            back.random_state, again.random_state,
            "each decode draws a fresh generator (`decodeDummyBlake2b512Random`)"
        );
    }

    /// `BindPattern`'s `remainder` distinguishes a `...`-terminated bind from an exhaustive one,
    /// and it survives the round trip — the one optional in these types that is *not* dropped.
    #[test]
    fn a_remainder_survives_the_round_trip() {
        let pattern = BindPattern {
            patterns: vec![empty_proc()],
            remainder: Some(Var::Wildcard),
            free_count: 0,
        };
        let json = serde_json::to_string(&pattern).unwrap();
        let back: BindPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pattern);
        assert_eq!(back.remainder, Some(Var::Wildcard));
    }
}

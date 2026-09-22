//! `Par` ⇄ `RhoExpr` mapping (port of the `RhoExpr` tree in `api/WebApi.scala`).
//!
//! `RhoExpr` is the JSON-ish representation of rholang data the web API exposes. `expr_from_par`
//! converts a protobuf `Par` to it and `rho_expr_to_par` converts back.
//!
//! **The wire shape is the reference's, not this module's** (AUDIT C38). The reference document
//! (`legacy/docs/rnode-api/rnode-openapi.json`, and the generated `rnode-openapi-schema.ts` beside
//! it) is what a client generates its types from, and it says every arm wraps its payload in a field
//! named `data`:
//!
//! ```text
//! ExprInt:  { ExprInt:  { data: number } }
//! ExprList: { ExprList: { data: RhoExpr[] } }
//! ExprMap:  { ExprMap:  { data: { [key: string]: RhoExpr } } }   -- an *object*, not pairs
//! ExprUnforg: { ExprUnforg: { data: RhoUnforg } }
//! UnforgPrivate: { data: string }
//! ```
//!
//! This module's `#[derive]`d representation emitted `{"ExprInt":42}`, `{"ExprMap":[["k",v]]}` and
//! `{"ExprUnforg":{"UnforgPrivate":"hex"}}` — three divergences from a client's expectations on every
//! value the node reports, which is the reply the client never managed to read. The enum below stays
//! as the internal form; `RhoExprWire`/`RhoUnforgWire` are the contract, and the manual
//! `Serialize`/`Deserialize` impls are the one place the two meet.

use std::collections::BTreeMap;

use rchain_models::ast::{Bundle, Expr, GUnforgeable, Par};
use rchain_models::rholang::RhoType::{
    RhoBoolean, RhoByteArray, RhoDeployId, RhoDeployerId, RhoList, RhoMap, RhoName, RhoNumber,
    RhoSet, RhoString, RhoTupleN, RhoUri,
};
use rchain_shared::base16;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Rholang terms interesting for translation to JSON (port of `RhoExpr`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RhoExpr {
    ExprPar(Vec<RhoExpr>),
    ExprTuple(Vec<RhoExpr>),
    ExprList(Vec<RhoExpr>),
    ExprSet(Vec<RhoExpr>),
    /// Key/value pairs, kept sorted. On the wire this is a JSON **object** (the reference's shape),
    /// which is also what law 1's canonical order wants.
    ExprMap(Vec<(String, RhoExpr)>),
    ExprBool(bool),
    ExprInt(i64),
    ExprString(String),
    ExprUri(String),
    ExprBytes(String),
    ExprUnforg(RhoUnforg),
}

/// An unforgeable name (port of `RhoUnforg`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RhoUnforg {
    UnforgPrivate(String),
    UnforgDeploy(String),
    UnforgDeployer(String),
}

/// The reference's payload wrapper: `{"data": …}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Data<T> {
    pub data: T,
}

impl<T> From<T> for Data<T> {
    fn from(data: T) -> Self {
        Data { data }
    }
}

/// The reference's `RhoExpr` encoding, arm for arm.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RhoExprWire {
    ExprPar(Data<Vec<RhoExprWire>>),
    ExprTuple(Data<Vec<RhoExprWire>>),
    ExprList(Data<Vec<RhoExprWire>>),
    ExprSet(Data<Vec<RhoExprWire>>),
    ExprMap(Data<BTreeMap<String, RhoExprWire>>),
    ExprBool(Data<bool>),
    ExprInt(Data<i64>),
    ExprString(Data<String>),
    ExprUri(Data<String>),
    ExprBytes(Data<String>),
    ExprUnforg(Data<RhoUnforgWire>),
}

/// The reference's `RhoUnforg` encoding: a tagged union whose payloads are themselves `{"data": …}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RhoUnforgWire {
    UnforgPrivate(Data<String>),
    UnforgDeploy(Data<String>),
    UnforgDeployer(Data<String>),
}

impl From<RhoUnforg> for RhoUnforgWire {
    fn from(u: RhoUnforg) -> Self {
        match u {
            RhoUnforg::UnforgPrivate(s) => RhoUnforgWire::UnforgPrivate(s.into()),
            RhoUnforg::UnforgDeploy(s) => RhoUnforgWire::UnforgDeploy(s.into()),
            RhoUnforg::UnforgDeployer(s) => RhoUnforgWire::UnforgDeployer(s.into()),
        }
    }
}

impl From<RhoUnforgWire> for RhoUnforg {
    fn from(u: RhoUnforgWire) -> Self {
        match u {
            RhoUnforgWire::UnforgPrivate(d) => RhoUnforg::UnforgPrivate(d.data),
            RhoUnforgWire::UnforgDeploy(d) => RhoUnforg::UnforgDeploy(d.data),
            RhoUnforgWire::UnforgDeployer(d) => RhoUnforg::UnforgDeployer(d.data),
        }
    }
}

impl From<RhoExpr> for RhoExprWire {
    fn from(e: RhoExpr) -> Self {
        let many = |es: Vec<RhoExpr>| Data {
            data: es.into_iter().map(RhoExprWire::from).collect::<Vec<_>>(),
        };
        match e {
            RhoExpr::ExprPar(es) => RhoExprWire::ExprPar(many(es)),
            RhoExpr::ExprTuple(es) => RhoExprWire::ExprTuple(many(es)),
            RhoExpr::ExprList(es) => RhoExprWire::ExprList(many(es)),
            RhoExpr::ExprSet(es) => RhoExprWire::ExprSet(many(es)),
            RhoExpr::ExprMap(kvs) => RhoExprWire::ExprMap(Data {
                // A `BTreeMap`, so the object's keys are emitted in law 1's canonical order.
                data: kvs
                    .into_iter()
                    .map(|(k, v)| (k, RhoExprWire::from(v)))
                    .collect(),
            }),
            RhoExpr::ExprBool(b) => RhoExprWire::ExprBool(b.into()),
            RhoExpr::ExprInt(n) => RhoExprWire::ExprInt(n.into()),
            RhoExpr::ExprString(s) => RhoExprWire::ExprString(s.into()),
            RhoExpr::ExprUri(s) => RhoExprWire::ExprUri(s.into()),
            RhoExpr::ExprBytes(s) => RhoExprWire::ExprBytes(s.into()),
            RhoExpr::ExprUnforg(u) => RhoExprWire::ExprUnforg(Data { data: u.into() }),
        }
    }
}

impl From<RhoExprWire> for RhoExpr {
    fn from(e: RhoExprWire) -> Self {
        let many = |es: Vec<RhoExprWire>| es.into_iter().map(RhoExpr::from).collect::<Vec<_>>();
        match e {
            RhoExprWire::ExprPar(d) => RhoExpr::ExprPar(many(d.data)),
            RhoExprWire::ExprTuple(d) => RhoExpr::ExprTuple(many(d.data)),
            RhoExprWire::ExprList(d) => RhoExpr::ExprList(many(d.data)),
            RhoExprWire::ExprSet(d) => RhoExpr::ExprSet(many(d.data)),
            RhoExprWire::ExprMap(d) => RhoExpr::ExprMap(
                d.data
                    .into_iter()
                    .map(|(k, v)| (k, RhoExpr::from(v)))
                    .collect(),
            ),
            RhoExprWire::ExprBool(d) => RhoExpr::ExprBool(d.data),
            RhoExprWire::ExprInt(d) => RhoExpr::ExprInt(d.data),
            RhoExprWire::ExprString(d) => RhoExpr::ExprString(d.data),
            RhoExprWire::ExprUri(d) => RhoExpr::ExprUri(d.data),
            RhoExprWire::ExprBytes(d) => RhoExpr::ExprBytes(d.data),
            RhoExprWire::ExprUnforg(d) => RhoExpr::ExprUnforg(d.data.into()),
        }
    }
}

impl Serialize for RhoExpr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        RhoExprWire::from(self.clone()).serialize(s)
    }
}

impl<'de> Deserialize<'de> for RhoExpr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(RhoExprWire::deserialize(d)?.into())
    }
}

impl Serialize for RhoUnforg {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        RhoUnforgWire::from(self.clone()).serialize(s)
    }
}

impl<'de> Deserialize<'de> for RhoUnforg {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(RhoUnforgWire::deserialize(d)?.into())
    }
}

/// Convert a `Par` to a `RhoExpr` (port of `exprFromParProto`).
pub fn expr_from_par(par: &Par) -> Option<RhoExpr> {
    let mut exprs = Vec::new();
    for e in &par.exprs {
        if let Some(r) = expr_from_expr(e) {
            exprs.push(r);
        }
    }
    for u in &par.unforgeables {
        if let Some(r) = unforg_from_proto(u) {
            exprs.push(r);
        }
    }
    for b in &par.bundles {
        if let Some(r) = expr_from_bundle(b) {
            exprs.push(r);
        }
    }

    // Implements the semantic of Par with Unit: P | Nil ==> P.
    match exprs.len() {
        1 => exprs.into_iter().next(),
        0 => None,
        _ => Some(RhoExpr::ExprPar(exprs)),
    }
}

fn expr_from_expr(exp: &Expr) -> Option<RhoExpr> {
    match exp {
        Expr::GBool(b) => Some(RhoExpr::ExprBool(*b)),
        Expr::GInt(i) => Some(RhoExpr::ExprInt(*i)),
        Expr::GString(s) => Some(RhoExpr::ExprString(s.clone())),
        Expr::GUri(u) => Some(RhoExpr::ExprUri(u.clone())),
        Expr::GByteArray(bs) => Some(RhoExpr::ExprBytes(base16::encode(bs))),
        Expr::ETuple(t) => Some(RhoExpr::ExprTuple(
            t.ps.iter().filter_map(expr_from_par).collect(),
        )),
        Expr::EList(l) => Some(RhoExpr::ExprList(
            l.ps.iter().filter_map(expr_from_par).collect(),
        )),
        Expr::ESet(s) => Some(RhoExpr::ExprSet(
            s.ps.iter().filter_map(expr_from_par).collect(),
        )),
        Expr::EMap(m) => {
            let mut fields = Vec::new();
            for (k, v) in &m.kvs {
                let Some(key_expr) = expr_from_par(k) else {
                    continue;
                };
                let Some(key) = key_to_string(&key_expr) else {
                    continue;
                };
                let Some(value) = expr_from_par(v) else {
                    continue;
                };
                fields.push((key, value));
            }
            Some(RhoExpr::ExprMap(fields))
        }
        _ => None,
    }
}

fn unforg_from_proto(un: &GUnforgeable) -> Option<RhoExpr> {
    match un {
        GUnforgeable::GPrivate(g) => Some(RhoExpr::ExprUnforg(RhoUnforg::UnforgPrivate(
            base16::encode(&g.id),
        ))),
        GUnforgeable::GDeployId(d) => Some(RhoExpr::ExprUnforg(RhoUnforg::UnforgDeploy(
            base16::encode(&d.sig),
        ))),
        GUnforgeable::GDeployerId(d) => Some(RhoExpr::ExprUnforg(RhoUnforg::UnforgDeployer(
            base16::encode(&d.public_key),
        ))),
        _ => None,
    }
}

fn expr_from_bundle(b: &Bundle) -> Option<RhoExpr> {
    expr_from_par(&b.body)
}

/// Stringify a map key `RhoExpr` (port of the `keyExpr match` in `exprFromExprProto`).
fn key_to_string(key: &RhoExpr) -> Option<String> {
    match key {
        RhoExpr::ExprString(s) => Some(s.clone()),
        RhoExpr::ExprInt(n) => Some(n.to_string()),
        RhoExpr::ExprBool(b) => Some(b.to_string()),
        RhoExpr::ExprUri(u) => Some(u.clone()),
        RhoExpr::ExprUnforg(u) => match u {
            RhoUnforg::UnforgPrivate(hex)
            | RhoUnforg::UnforgDeploy(hex)
            | RhoUnforg::UnforgDeployer(hex) => Some(hex.clone()),
        },
        RhoExpr::ExprBytes(hex) => Some(hex.clone()),
        _ => None,
    }
}

/// Convert a `RhoExpr` back to a `Par` (port of `rhoExprToParProto`). Hex-encoded leaves are
/// validated (failing on non-hex input) rather than silently corrupted.
pub fn rho_expr_to_par(exp: &RhoExpr) -> Result<Par, String> {
    match exp {
        RhoExpr::ExprPar(data) => {
            let pars: Result<Vec<Par>, String> = data.iter().map(rho_expr_to_par).collect();
            Ok(pars?
                .into_iter()
                .fold(Par::default(), |acc, p| acc.par_merge(&p)))
        }
        RhoExpr::ExprTuple(data) => {
            let pars: Result<Vec<Par>, String> = data.iter().map(rho_expr_to_par).collect();
            Ok(RhoTupleN::apply(pars?))
        }
        RhoExpr::ExprList(data) => {
            let pars: Result<Vec<Par>, String> = data.iter().map(rho_expr_to_par).collect();
            Ok(RhoList::apply(pars?))
        }
        RhoExpr::ExprSet(data) => {
            let pars: Result<Vec<Par>, String> = data.iter().map(rho_expr_to_par).collect();
            Ok(RhoSet::apply(pars?))
        }
        RhoExpr::ExprMap(data) => {
            let kvs: Result<Vec<(String, Par)>, String> = data
                .iter()
                .map(|(k, v)| rho_expr_to_par(v).map(|p| (k.clone(), p)))
                .collect();
            Ok(RhoMap::apply(
                kvs?.into_iter()
                    .map(|(k, v)| (RhoString::apply(k), v))
                    .collect(),
            ))
        }
        RhoExpr::ExprBool(b) => Ok(RhoBoolean::apply(*b)),
        RhoExpr::ExprInt(i) => Ok(RhoNumber::apply(*i)),
        RhoExpr::ExprString(s) => Ok(RhoString::apply(s.clone())),
        RhoExpr::ExprUri(u) => Ok(RhoUri::apply(u.clone())),
        RhoExpr::ExprBytes(hex) => Ok(RhoByteArray::apply(base16::try_decode(hex)?)),
        RhoExpr::ExprUnforg(u) => unforg_to_par(u),
    }
}

/// Convert a `RhoUnforg` to a `Par` (port of `unforgToParProto`), validating the hex-encoded name.
pub fn unforg_to_par(unforg: &RhoUnforg) -> Result<Par, String> {
    match unforg {
        RhoUnforg::UnforgPrivate(name) => Ok(RhoName::apply_bytes(base16::try_decode(name)?)),
        RhoUnforg::UnforgDeploy(name) => Ok(RhoDeployId::apply(base16::try_decode(name)?)),
        RhoUnforg::UnforgDeployer(name) => Ok(RhoDeployerId::apply(base16::try_decode(name)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(e: &RhoExpr) {
        assert_eq!(expr_from_par(&rho_expr_to_par(e).unwrap()), Some(e.clone()));
    }

    #[test]
    fn terminal_exprs_round_trip() {
        round_trip(&RhoExpr::ExprBool(true));
        round_trip(&RhoExpr::ExprBool(false));
        round_trip(&RhoExpr::ExprInt(42));
        round_trip(&RhoExpr::ExprInt(-7));
        round_trip(&RhoExpr::ExprString("hello".to_string()));
        round_trip(&RhoExpr::ExprUri("rho:id:123".to_string()));
        round_trip(&RhoExpr::ExprBytes("deadbeef".to_string()));
        round_trip(&RhoExpr::ExprUnforg(RhoUnforg::UnforgPrivate(
            "abcd1234".to_string(),
        )));
        round_trip(&RhoExpr::ExprUnforg(RhoUnforg::UnforgDeploy(
            "deadbeef".to_string(),
        )));
        round_trip(&RhoExpr::ExprUnforg(RhoUnforg::UnforgDeployer(
            "00112233".to_string(),
        )));
    }

    #[test]
    fn collection_exprs_round_trip() {
        round_trip(&RhoExpr::ExprTuple(vec![
            RhoExpr::ExprInt(1),
            RhoExpr::ExprBool(false),
        ]));
        round_trip(&RhoExpr::ExprList(vec![
            RhoExpr::ExprString("a".to_string()),
            RhoExpr::ExprInt(2),
        ]));
        round_trip(&RhoExpr::ExprSet(vec![
            RhoExpr::ExprInt(1),
            RhoExpr::ExprInt(2),
        ]));
        round_trip(&RhoExpr::ExprMap(vec![(
            "key".to_string(),
            RhoExpr::ExprInt(1),
        )]));
        round_trip(&RhoExpr::ExprPar(vec![
            RhoExpr::ExprBool(true),
            RhoExpr::ExprInt(1),
            RhoExpr::ExprString("s".to_string()),
            RhoExpr::ExprUri("u".to_string()),
            RhoExpr::ExprBytes("beef".to_string()),
            RhoExpr::ExprUnforg(RhoUnforg::UnforgPrivate("abcd".to_string())),
        ]));
    }

    #[test]
    fn non_json_exprs_map_to_none() {
        // An arithmetic expression has no RhoExpr representation.
        let p = Par {
            exprs: vec![Expr::EPlus(Box::default(), Box::default())],
            ..Default::default()
        };
        assert_eq!(expr_from_par(&p), None);
    }

    /// **The wire shape is the reference document's, arm for arm** (AUDIT C38). Every string here is
    /// the shape `legacy/docs/rnode-api/rnode-openapi-schema.ts` declares — the file a client
    /// generates its types from — so this test is the one that would have caught the divergence:
    /// the port emitted `{"ExprInt":42}`, `{"ExprMap":[["k",…]]}` and a flat unforgeable, and no
    /// client built on the reference API could read any of it.
    #[test]
    fn the_wire_shape_is_the_reference_documents() {
        let cases: Vec<(RhoExpr, &str)> = vec![
            (RhoExpr::ExprInt(42), r#"{"ExprInt":{"data":42}}"#),
            (RhoExpr::ExprBool(true), r#"{"ExprBool":{"data":true}}"#),
            (
                RhoExpr::ExprString("s".to_string()),
                r#"{"ExprString":{"data":"s"}}"#,
            ),
            (
                RhoExpr::ExprUri("rho:id:x".to_string()),
                r#"{"ExprUri":{"data":"rho:id:x"}}"#,
            ),
            (
                RhoExpr::ExprBytes("deadbeef".to_string()),
                r#"{"ExprBytes":{"data":"deadbeef"}}"#,
            ),
            (
                RhoExpr::ExprList(vec![RhoExpr::ExprInt(1)]),
                r#"{"ExprList":{"data":[{"ExprInt":{"data":1}}]}}"#,
            ),
            (
                RhoExpr::ExprTuple(vec![RhoExpr::ExprInt(1)]),
                r#"{"ExprTuple":{"data":[{"ExprInt":{"data":1}}]}}"#,
            ),
            (
                RhoExpr::ExprSet(vec![RhoExpr::ExprInt(1)]),
                r#"{"ExprSet":{"data":[{"ExprInt":{"data":1}}]}}"#,
            ),
            (
                RhoExpr::ExprPar(vec![RhoExpr::ExprInt(1)]),
                r#"{"ExprPar":{"data":[{"ExprInt":{"data":1}}]}}"#,
            ),
            // A map is a JSON **object** on the wire — the reference's `{ [key: string]: RhoExpr }` —
            // so its keys come out in the object's canonical (sorted) order, which is what law 1
            // wants; the pairs here are written sorted because that is the order the round-trip
            // reproduces. (The node's own maps reach this point from a `Par` whose exprs law 1 has
            // already sorted, so the wire order is canonical in practice too.)
            (
                RhoExpr::ExprMap(vec![
                    ("a".to_string(), RhoExpr::ExprInt(1)),
                    ("b".to_string(), RhoExpr::ExprInt(2)),
                ]),
                r#"{"ExprMap":{"data":{"a":{"ExprInt":{"data":1}},"b":{"ExprInt":{"data":2}}}}}"#,
            ),
            (
                RhoExpr::ExprUnforg(RhoUnforg::UnforgPrivate("ab".to_string())),
                r#"{"ExprUnforg":{"data":{"UnforgPrivate":{"data":"ab"}}}}"#,
            ),
        ];
        for (expr, want) in cases {
            let got = serde_json::to_string(&expr).expect("the encode serializes");
            assert_eq!(got, want, "wire shape of {expr:?}");
            let back: RhoExpr = serde_json::from_str(want).expect("and it decodes");
            assert_eq!(back, expr, "and it round-trips: {want}");
        }
    }
}

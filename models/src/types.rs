//! The type-system layer over the ρ-calculus (the Calculus-of-Constructions hardening).
//!
//! Mirrors `spec/Rchain/Ty.lean` (and `Rho.lean`). This module gives the port the hard type
//! discipline of [`spec/TYPE-SYSTEM.md`]: the two language sorts (`PSort`), the structural
//! name-vs-process classification, the `Closed` well-formedness refinement (Law 6), and the
//! de Bruijn context judgment (`varSort`). It is a hardening of the port — no behavior change.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ast::{
    Bundle, Connective, Expr, GUnforgeable, Match, MatchCase, New, Par, Receive, ReceiveBind, Send,
    Sort, Var,
};

/// The two syntactic sorts (mirrors `Ty.lean`'s `inductive PSort | proc | name`): a term is used in
/// *process* position or *name* position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PSort {
    Proc,
    Name,
}

/// A de Bruijn context: the sort of each level in scope (mirrors `Ty.lean`'s `Ctx := List PSort`).
pub type Ctx = Vec<PSort>;

/// The sort of a variable occurrence (mirrors `varSort`): a bound level is looked up in the context;
/// a free variable or wildcard has no local sort.
pub fn var_sort(ctx: &Ctx, v: &Var) -> Option<PSort> {
    match v {
        Var::BoundVar(l) if *l >= 0 => ctx.get(*l as usize).copied(),
        Var::BoundVar(_) | Var::FreeVar(_) | Var::Wildcard | Var::Empty => None,
    }
}

/// A *pure name* (mirrors `isPureName`): a `Par` with no process constructors at the top (empty
/// `sends`/`receives`/`news`/`matches`). These occur in name position: `Nil`, ground/expressions,
/// bundles, unforgeables, connectives.
pub fn is_pure_name(p: &Par) -> bool {
    p.sends.is_empty() && p.receives.is_empty() && p.news.is_empty() && p.matches.is_empty()
}

/// The structural sort classification (mirrors `classify`): a pure name is a `Name`, otherwise `Proc`.
pub fn classify(p: &Par) -> PSort {
    if is_pure_name(p) {
        PSort::Name
    } else {
        PSort::Proc
    }
}

/// Count the distinct free variables of a term (Law 5, `BindsAtMostOnce`).
///
/// Mirrors the normalizer's `FreeMap::count_no_wildcards` but works directly on a built `Par`: it
/// collects every `FreeVar(level)` occurrence, deduplicated by level, and returns the count. Used to
/// derive the `free_count` of hand-built system-contract patterns instead of hardcoding it.
///
/// The `remainder` binding (`ReceiveBind`/`EList`/`ParSet`/`ParMap`) is *not* counted here — callers
/// with a `remainder` must add 1. `New.injections` is not walked (mirrors `well_scoped_par`).
pub fn count_free_vars<S: Sort>(p: &Par<S>) -> i32 {
    i32::from(count_free_vars_refined(p))
}

/// [`count_free_vars`] as the carrier it is: a count of distinct levels a walk found, so the sign is
/// impossible and the only bound is width ([`FreeCount::from_len`] states it). The two spellings
/// walk the same occurrence set — the `i32` form is kept because `casper`'s runtime builders assign
/// it to `BindPattern.free_count`, an `i32` field the carrier does not cover yet (AUDIT C52).
pub fn count_free_vars_refined<S: Sort>(p: &Par<S>) -> FreeCount {
    let mut levels = BTreeSet::new();
    collect_free_vars_par(p, &mut |l| {
        levels.insert(l);
    });
    FreeCount::from_len(levels.len())
}

/// **Law 5's predicate**: no free level occurs twice in `p`, so matching `p` binds each level at most
/// once. The port's counterpart of the model's `linear` (`spec/Rchain/Match.lean:396-397`), which the
/// matcher's entry conjoins exactly as `spatialMatch` does
/// (`spatialMatchCore … && linear pattern`, `spec/Rchain/Match.lean:402`).
///
/// Two deliberate agreements with the model, and one deliberate difference:
///
/// - **The same walk as [`count_free_vars`]**, so both count the same occurrences — the levels a
///   `FreeVar` names, wildcards and bound variables contributing none.
/// - **A `remainder` binder is not counted** (`EList`/`ParSet`/`ParMap`), which is `count_free_vars`'s
///   convention and the model's too (`freeLevelsOfPar`'s `| .elist ps _ => …` ignores the field). It
///   is *not* a widening of the law: a binder and a remainder sharing a spelling is refused by the
///   normalizer (`normalizer.rs`'s reuse checks) before a matcher runs, and the model has no clause
///   for a remainder at all.
/// - **The walk is total over the port's `Par`** — sends, receives, news, matches, bundles,
///   connectives — where the model's `freeLevelsOfPar` walks only `exprs`, because the model's clauses
///   cover only expression-shaped pars and a level inside a send "cannot be accepted, so the linearity
///   check never has to see inside it". The port's clauses *do* match sends and receives, so its check
///   has to; on the model's own domain (`modelledPar`, the tie's) the two walks see the same levels.
///
/// This exists because the port's matcher enforced linearity in exactly one place — the aggregation
/// path (`aggregate_updates`) — while its element-pair (`fold_match`) and conjunction (`ConnAnd`)
/// paths bound with a plain insert and overwrote silently. Measured 2026-09-24: `@[v0, v0]` against
/// `[1, 1]`, both connective, **matched** in the port and is `false` in the model (AUDIT C42).
pub fn linear<S: Sort>(p: &Par<S>) -> bool {
    let mut levels: Vec<i32> = Vec::new();
    collect_free_vars_par(p, &mut |l| levels.push(l));
    levels.sort_unstable();
    levels.windows(2).all(|w| w[0] != w[1])
}

fn collect_free_vars_par<S: Sort>(p: &Par<S>, out: &mut impl FnMut(i32)) {
    for s in &p.sends {
        collect_free_vars_par(&s.chan, out);
        for d in &s.data {
            collect_free_vars_par(d, out);
        }
    }
    for r in &p.receives {
        for b in &r.binds {
            for pat in &b.patterns {
                collect_free_vars_par(pat, out);
            }
            collect_free_vars_par(&b.source, out);
        }
        collect_free_vars_par(&r.body, out);
    }
    for n in &p.news {
        collect_free_vars_par(&n.p, out);
    }
    for e in &p.exprs {
        collect_free_vars_expr(e, out);
    }
    for m in &p.matches {
        collect_free_vars_par(&m.target, out);
        for c in &m.cases {
            collect_free_vars_par(&c.pattern, out);
            collect_free_vars_par(&c.source, out);
        }
    }
    // `GUnforgeable` carries no variables.
    for b in &p.bundles {
        collect_free_vars_par(&b.body, out);
    }
    for c in &p.connectives {
        collect_free_vars_connective(c, out);
    }
}

fn collect_free_vars_var(v: &Var, out: &mut impl FnMut(i32)) {
    if let Var::FreeVar(l) = v {
        out(*l);
    }
}

fn collect_free_vars_expr(e: &Expr, out: &mut impl FnMut(i32)) {
    match e {
        Expr::GBool(_)
        | Expr::GInt(_)
        | Expr::GBigInt(_)
        | Expr::GString(_)
        | Expr::GUri(_)
        | Expr::GByteArray(_) => {}
        Expr::EVar(v) => collect_free_vars_var(v, out),
        Expr::ENot(p) | Expr::ENeg(p) => collect_free_vars_par(p, out),
        Expr::EMult(p, q)
        | Expr::EDiv(p, q)
        | Expr::EMod(p, q)
        | Expr::EPlus(p, q)
        | Expr::EMinus(p, q)
        | Expr::ELt(p, q)
        | Expr::ELte(p, q)
        | Expr::EGt(p, q)
        | Expr::EGte(p, q)
        | Expr::EEq(p, q)
        | Expr::ENeq(p, q)
        | Expr::EAnd(p, q)
        | Expr::EOr(p, q)
        | Expr::EShortAnd(p, q)
        | Expr::EShortOr(p, q)
        | Expr::EMatches(p, q)
        | Expr::EPercentPercent(p, q)
        | Expr::EPlusPlus(p, q)
        | Expr::EMinusMinus(p, q) => {
            collect_free_vars_par(p, out);
            collect_free_vars_par(q, out);
        }
        Expr::EList(el) => {
            for p in &el.ps {
                collect_free_vars_par(p, out);
            }
        }
        Expr::ETuple(et) => {
            for p in &et.ps {
                collect_free_vars_par(p, out);
            }
        }
        Expr::ESet(set) => {
            for p in &set.ps {
                collect_free_vars_par(p, out);
            }
        }
        Expr::EMap(map) => {
            for (k, v) in &map.kvs {
                collect_free_vars_par(k, out);
                collect_free_vars_par(v, out);
            }
        }
        Expr::EMethod(em) => {
            collect_free_vars_par(&em.target, out);
            for a in &em.arguments {
                collect_free_vars_par(a, out);
            }
        }
    }
}

fn collect_free_vars_connective(c: &Connective, out: &mut impl FnMut(i32)) {
    match c {
        Connective::ConnAnd(cb) | Connective::ConnOr(cb) => {
            for p in &cb.ps {
                collect_free_vars_par(p, out);
            }
        }
        Connective::ConnNot(p) => collect_free_vars_par(p, out),
        Connective::VarRef(_)
        | Connective::ConnBool(_)
        | Connective::ConnInt(_)
        | Connective::ConnBigInt(_)
        | Connective::ConnString(_)
        | Connective::ConnUri(_)
        | Connective::ConnByteArray(_)
        | Connective::Empty => {}
    }
}

/// Validated sort construction: a `Par` is a `Name` iff it is a pure name (the structural sort).
/// The `quote` re-marking carries the sort thereafter; the invariant is checked exactly once, here.
impl TryFrom<Par> for crate::ast::Name {
    type Error = String;
    fn try_from(p: Par) -> Result<Self, Self::Error> {
        if is_pure_name(&p) {
            Ok(p.quote())
        } else {
            Err("term is not a pure name (has a top-level send/receive/new/match)".to_string())
        }
    }
}

/// One-way boundary discharge: a `Name` re-enters the general `Par` by `eval` (the reflective `*`;
/// the flat record is unchanged).
impl From<crate::ast::Name> for Par {
    fn from(n: crate::ast::Name) -> Par {
        n.eval()
    }
}

// --- Closedness (Law 6): no free variables ------------------------------------------------
//
// A term is *closed* when it has no free variables in evaluation position. A `FreeVar` in *pattern*
// position (the binders of `ReceiveBind.patterns` / `MatchCase.pattern`) is a binder, not a free
// variable, so the `pattern` flag distinguishes the two positions as the check recurses.

fn closed_var(v: &Var, pattern: bool) -> bool {
    match v {
        Var::FreeVar(_) => pattern,
        Var::BoundVar(_) | Var::Wildcard | Var::Empty => true,
    }
}

fn closed_par<S: Sort>(p: &Par<S>, pattern: bool) -> bool {
    p.sends.iter().all(|s| closed_send(s, pattern))
        && p.receives.iter().all(|r| closed_receive(r, pattern))
        && p.news.iter().all(|n| closed_new(n, pattern))
        && p.exprs.iter().all(|e| closed_expr(e, pattern))
        && p.matches.iter().all(|m| closed_match(m, pattern))
        && p.unforgeables.iter().all(closed_unforgeable)
        && p.bundles.iter().all(|b| closed_bundle(b, pattern))
        && p.connectives.iter().all(|c| closed_connective(c, pattern))
}

fn closed_send(s: &Send, pattern: bool) -> bool {
    closed_par(&s.chan, pattern) && s.data.iter().all(|d| closed_par(d, pattern))
}

fn closed_receive_bind(rb: &ReceiveBind, pattern: bool) -> bool {
    rb.patterns.iter().all(|p| closed_par(p, true)) && closed_par(&rb.source, pattern)
}

fn closed_receive(r: &Receive, pattern: bool) -> bool {
    r.binds.iter().all(|b| closed_receive_bind(b, pattern)) && closed_par(&r.body, pattern)
}

fn closed_new(n: &New, pattern: bool) -> bool {
    closed_par(&n.p, pattern)
}

fn closed_match_case(mc: &MatchCase, pattern: bool) -> bool {
    closed_par(&mc.pattern, true) && closed_par(&mc.source, pattern)
}

fn closed_match(m: &Match, pattern: bool) -> bool {
    closed_par(&m.target, pattern) && m.cases.iter().all(|c| closed_match_case(c, pattern))
}

fn closed_expr(e: &Expr, pattern: bool) -> bool {
    match e {
        Expr::GBool(_)
        | Expr::GInt(_)
        | Expr::GBigInt(_)
        | Expr::GString(_)
        | Expr::GUri(_)
        | Expr::GByteArray(_) => true,
        Expr::EVar(v) => closed_var(v, pattern),
        Expr::ENot(p) | Expr::ENeg(p) => closed_par(p, pattern),
        Expr::EMult(p, q)
        | Expr::EDiv(p, q)
        | Expr::EMod(p, q)
        | Expr::EPlus(p, q)
        | Expr::EMinus(p, q)
        | Expr::ELt(p, q)
        | Expr::ELte(p, q)
        | Expr::EGt(p, q)
        | Expr::EGte(p, q)
        | Expr::EEq(p, q)
        | Expr::ENeq(p, q)
        | Expr::EAnd(p, q)
        | Expr::EOr(p, q)
        | Expr::EShortAnd(p, q)
        | Expr::EShortOr(p, q)
        | Expr::EMatches(p, q)
        | Expr::EPercentPercent(p, q)
        | Expr::EPlusPlus(p, q)
        | Expr::EMinusMinus(p, q) => closed_par(p, pattern) && closed_par(q, pattern),
        Expr::EList(el) => el.ps.iter().all(|p| closed_par(p, pattern)),
        Expr::ETuple(et) => et.ps.iter().all(|p| closed_par(p, pattern)),
        Expr::ESet(set) => set.ps.iter().all(|p| closed_par(p, pattern)),
        Expr::EMap(map) => map
            .kvs
            .iter()
            .all(|(k, v)| closed_par(k, pattern) && closed_par(v, pattern)),
        Expr::EMethod(em) => {
            closed_par(&em.target, pattern) && em.arguments.iter().all(|p| closed_par(p, pattern))
        }
    }
}

fn closed_bundle(b: &Bundle, pattern: bool) -> bool {
    closed_par(&b.body, pattern)
}

fn closed_unforgeable(_: &GUnforgeable) -> bool {
    true
}

fn closed_connective(c: &Connective, pattern: bool) -> bool {
    match c {
        Connective::ConnAnd(cb) | Connective::ConnOr(cb) => {
            cb.ps.iter().all(|p| closed_par(p, pattern))
        }
        Connective::ConnNot(p) => closed_par(p, pattern),
        Connective::VarRef(_)
        | Connective::ConnBool(_)
        | Connective::ConnInt(_)
        | Connective::ConnBigInt(_)
        | Connective::ConnString(_)
        | Connective::ConnUri(_)
        | Connective::ConnByteArray(_)
        | Connective::Empty => true,
    }
}

/// `is_closed p` — the process has no free variables (Law 6). Decidable, and preserved by
/// composition, `≡`, and canonicalization (mirrors `Ty.lean`'s `closed` / `Closed`).
pub fn is_closed(p: &Par) -> bool {
    closed_par(p, false)
}

/// A closed process — the well-formedness refinement that makes the interpreter's partiality
/// impossible (the Rust spelling of `TotalOn`/the totality invariant in `TYPE-SYSTEM.md` §1.6).
///
/// Constructed only via [`Closed::new`], which is a declared partiality boundary: it returns `None`
/// for a term with free variables rather than panicking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed(Par);

impl Closed {
    /// Validate that `par` is closed. `None` is the declared boundary for an open (free-variable)
    /// term.
    pub fn new(par: Par) -> Option<Closed> {
        if is_closed(&par) {
            Some(Closed(par))
        } else {
            None
        }
    }
}

/// One-way boundary discharge: a closed process re-enters the general `Par` (the proof is dropped
/// at the boundary).
impl From<Closed> for Par {
    fn from(c: Closed) -> Par {
        c.0
    }
}

// --- Well-scopedness (the variable half of the judgment) ---------------------------------
//
// `BoundVar` uses de Bruijn *levels*: `BoundVar(l)` is the `l`-th binder from the outermost, so a
// sub-term inside `n` binders is checked under a context of depth `depth + n`. `FreeVar`/wildcard/
// empty carry no scope requirement.

fn well_scoped_var(depth: usize, v: &Var) -> bool {
    match v {
        Var::BoundVar(l) => *l >= 0 && (*l as usize) < depth,
        Var::FreeVar(_) | Var::Wildcard | Var::Empty => true,
    }
}

fn well_scoped_par<S: Sort>(depth: usize, p: &Par<S>) -> bool {
    p.sends.iter().all(|s| well_scoped_send(depth, s))
        && p.receives.iter().all(|r| well_scoped_receive(depth, r))
        && p.news.iter().all(|n| well_scoped_new(depth, n))
        && p.exprs.iter().all(|e| well_scoped_expr(depth, e))
        && p.matches.iter().all(|m| well_scoped_match(depth, m))
        && p.bundles.iter().all(|b| well_scoped_par(depth, &b.body))
        && p.connectives
            .iter()
            .all(|c| well_scoped_connective(depth, c))
}

fn well_scoped_send(depth: usize, s: &Send) -> bool {
    well_scoped_par(depth, &s.chan) && s.data.iter().all(|d| well_scoped_par(depth, d))
}

fn well_scoped_receive_bind(depth: usize, rb: &ReceiveBind) -> bool {
    rb.patterns.iter().all(|p| well_scoped_par(depth, p))
        && well_scoped_par(depth, &rb.source)
        && rb
            .remainder
            .as_ref()
            .map(|v| well_scoped_var(depth, v))
            .unwrap_or(true)
}

fn well_scoped_receive(depth: usize, r: &Receive) -> bool {
    r.binds.iter().all(|b| well_scoped_receive_bind(depth, b))
        && well_scoped_par(depth + i32::from(r.bind_count) as usize, &r.body)
}

fn well_scoped_new(depth: usize, n: &New) -> bool {
    well_scoped_par(depth + i32::from(n.bind_count) as usize, &n.p)
}

fn well_scoped_match_case(depth: usize, mc: &MatchCase) -> bool {
    well_scoped_par(depth, &mc.pattern)
        && well_scoped_par(depth + i32::from(mc.free_count) as usize, &mc.source)
}

fn well_scoped_match(depth: usize, m: &Match) -> bool {
    well_scoped_par(depth, &m.target) && m.cases.iter().all(|c| well_scoped_match_case(depth, c))
}

fn well_scoped_expr(depth: usize, e: &Expr) -> bool {
    match e {
        Expr::GBool(_)
        | Expr::GInt(_)
        | Expr::GBigInt(_)
        | Expr::GString(_)
        | Expr::GUri(_)
        | Expr::GByteArray(_) => true,
        Expr::EVar(v) => well_scoped_var(depth, v),
        Expr::ENot(p) | Expr::ENeg(p) => well_scoped_par(depth, p),
        Expr::EMult(p, q)
        | Expr::EDiv(p, q)
        | Expr::EMod(p, q)
        | Expr::EPlus(p, q)
        | Expr::EMinus(p, q)
        | Expr::ELt(p, q)
        | Expr::ELte(p, q)
        | Expr::EGt(p, q)
        | Expr::EGte(p, q)
        | Expr::EEq(p, q)
        | Expr::ENeq(p, q)
        | Expr::EAnd(p, q)
        | Expr::EOr(p, q)
        | Expr::EShortAnd(p, q)
        | Expr::EShortOr(p, q)
        | Expr::EMatches(p, q)
        | Expr::EPercentPercent(p, q)
        | Expr::EPlusPlus(p, q)
        | Expr::EMinusMinus(p, q) => well_scoped_par(depth, p) && well_scoped_par(depth, q),
        Expr::EList(el) => el.ps.iter().all(|p| well_scoped_par(depth, p)),
        Expr::ETuple(et) => et.ps.iter().all(|p| well_scoped_par(depth, p)),
        Expr::ESet(set) => set.ps.iter().all(|p| well_scoped_par(depth, p)),
        Expr::EMap(map) => map
            .kvs
            .iter()
            .all(|(k, v)| well_scoped_par(depth, k) && well_scoped_par(depth, v)),
        Expr::EMethod(em) => {
            well_scoped_par(depth, &em.target)
                && em.arguments.iter().all(|p| well_scoped_par(depth, p))
        }
    }
}

fn well_scoped_connective(depth: usize, c: &Connective) -> bool {
    match c {
        Connective::ConnAnd(cb) | Connective::ConnOr(cb) => {
            cb.ps.iter().all(|p| well_scoped_par(depth, p))
        }
        Connective::ConnNot(p) => well_scoped_par(depth, p),
        Connective::VarRef(_)
        | Connective::ConnBool(_)
        | Connective::ConnInt(_)
        | Connective::ConnBigInt(_)
        | Connective::ConnString(_)
        | Connective::ConnUri(_)
        | Connective::ConnByteArray(_)
        | Connective::Empty => true,
    }
}

/// `well_scoped Γ t` — every bound level of `t` is within `Γ` (the variable half of the typing
/// judgment). The outer context `Γ` supplies the initial depth; binders introduced by `t` (`new`/
/// `for`/`match`) extend it as the check recurses.
pub fn well_scoped(ctx: &Ctx, p: &Par) -> bool {
    well_scoped_par(ctx.len(), p)
}

/// A well-scoped process under a de Bruijn context `Γ` — the variable half of the typing judgment
/// (`WellScoped Γ t`). Constructed only via [`WellScoped::new`], which is a declared partiality
/// boundary: it returns `None` for a term with an out-of-scope bound level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WellScoped {
    ctx: Ctx,
    par: Par,
}

impl WellScoped {
    /// Validate that `par` is well-scoped under `ctx`. `None` is the declared boundary for an
    /// out-of-scope bound variable.
    pub fn new(ctx: Ctx, par: Par) -> Option<WellScoped> {
        if well_scoped(&ctx, &par) {
            Some(WellScoped { ctx, par })
        } else {
            None
        }
    }

    /// The context the term is scoped under.
    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }
}

/// One-way boundary discharge: a well-scoped process re-enters the general `Par` (the proof is
/// dropped at the boundary).
impl From<WellScoped> for Par {
    fn from(w: WellScoped) -> Par {
        w.par
    }
}

// --- BindsAtMostOnce (Law 5): the free-variable count of a pattern ----------------------

/// The number of free variables a pattern binds (Law 5, `BindsAtMostOnce`): a non-negative count,
/// carried by the `free_count` fields of `ReceiveBind`/`MatchCase`. The normalizer computes it as the
/// number of *distinct* free variables in the pattern, so each is bound at most once.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(try_from = "i32", into = "i32")]
pub struct FreeCount(i32);

impl TryFrom<i32> for FreeCount {
    type Error = String;
    fn try_from(n: i32) -> Result<Self, Self::Error> {
        FreeCount::new(n).ok_or_else(|| format!("negative free-count: {n}"))
    }
}

impl FreeCount {
    /// The empty-pattern count.
    pub const ZERO: FreeCount = FreeCount(0);

    /// The single-binding count — one level, as in `for (x <- c)`.
    pub const ONE: FreeCount = FreeCount(1);

    /// Validated construction — the declared partiality boundary for a negative count.
    pub fn new(n: i32) -> Option<FreeCount> {
        if n >= 0 {
            Some(FreeCount(n))
        } else {
            None
        }
    }

    /// Total construction from a count of things that *exist*: a `BTreeSet`'s length
    /// ([`count_free_vars_refined`]), a level counter, a sum of validated counts.
    ///
    /// A `usize` cannot be negative, so the only way out of the domain is width, and reaching
    /// `i32::MAX` takes 2³¹ distinct free levels — 2³¹ variable nodes, tens of GiB, before the count
    /// is even formed. Saturation rather than wrapping, which is this tree's rule for exactly this
    /// shape (`shared/src/refined.rs`'s `BlockHeight`/`SeqNum` `Add`, with the same reasoning): a
    /// wrap would produce a *negative* "non-negative" count, the one value the carrier exists to
    /// exclude, where saturation keeps the value in the domain and monotone in its input.
    ///
    /// Successor of `from_nonneg`, deleted 2026-09-24 (AUDIT C52): that one took a raw `i32` and
    /// asserted `n >= 0` in a `debug_assert!` — compiled out in release, so the invalid count was
    /// carried silently in exactly the builds that matter. Taking the count of a collection makes the
    /// sign impossible instead of asserted.
    pub fn from_len(n: usize) -> FreeCount {
        FreeCount(i32::try_from(n).unwrap_or(i32::MAX))
    }
}

/// The carrier's own addition, which preserves the invariant: both operands are non-negative, so the
/// saturated sum is too ([`FreeCount::from_len`] carries the width argument, and `BlockHeight`'s
/// `Add` makes the same choice for the same reason).
impl std::ops::Add for FreeCount {
    type Output = FreeCount;
    fn add(self, rhs: FreeCount) -> FreeCount {
        FreeCount(self.0.saturating_add(rhs.0))
    }
}

/// One-way boundary discharge: the raw count (`i32`) is used at the range/arithmetic boundaries
/// (e.g. `0..free_count`, the wire codec).
impl From<FreeCount> for i32 {
    fn from(f: FreeCount) -> i32 {
        f.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g_int(i: i64) -> Par {
        Par {
            exprs: vec![Expr::GInt(i)],
            ..Default::default()
        }
    }

    fn free_var(l: i32) -> Par {
        Par {
            exprs: vec![Expr::EVar(Box::new(Var::FreeVar(l)))],
            ..Default::default()
        }
    }

    #[test]
    fn classify_nil_is_name() {
        assert_eq!(classify(&Par::default()), PSort::Name);
    }

    #[test]
    fn classify_process_is_proc() {
        let send = Par {
            sends: vec![Send::default()],
            ..Default::default()
        };
        assert_eq!(classify(&send), PSort::Proc);
    }

    #[test]
    fn classify_ground_expr_is_name() {
        assert_eq!(classify(&g_int(7)), PSort::Name);
    }

    #[test]
    fn count_free_vars_counts_distinct_free_variables() {
        let nil: Par = Par::default();
        assert_eq!(count_free_vars(&nil), 0);
        assert_eq!(count_free_vars(&free_var(0)), 1);
        // Duplicate level counts once (the "bind at most once" invariant).
        let dup: Par = Par {
            exprs: vec![
                Expr::EVar(Box::new(Var::FreeVar(0))),
                Expr::EVar(Box::new(Var::FreeVar(0))),
            ],
            ..Default::default()
        };
        assert_eq!(count_free_vars(&dup), 1);
        // Distinct levels count twice.
        let two: Par = Par {
            exprs: vec![
                Expr::EVar(Box::new(Var::FreeVar(0))),
                Expr::EVar(Box::new(Var::FreeVar(1))),
            ],
            ..Default::default()
        };
        assert_eq!(count_free_vars(&two), 2);
        // Wildcards and bound vars are not free vars.
        let wild: Par = Par {
            exprs: vec![Expr::EVar(Box::new(Var::Wildcard))],
            ..Default::default()
        };
        assert_eq!(count_free_vars(&wild), 0);
        let bound: Par = Par {
            exprs: vec![Expr::EVar(Box::new(Var::BoundVar(0)))],
            ..Default::default()
        };
        assert_eq!(count_free_vars(&bound), 0);
    }

    #[test]
    fn var_sort_looks_up_bound_level() {
        let ctx = vec![PSort::Name, PSort::Proc];
        assert_eq!(var_sort(&ctx, &Var::BoundVar(0)), Some(PSort::Name));
        assert_eq!(var_sort(&ctx, &Var::BoundVar(1)), Some(PSort::Proc));
        assert_eq!(var_sort(&ctx, &Var::BoundVar(2)), None);
    }

    #[test]
    fn var_sort_free_and_wildcard_are_none() {
        let ctx = vec![PSort::Name];
        assert_eq!(var_sort(&ctx, &Var::FreeVar(0)), None);
        assert_eq!(var_sort(&ctx, &Var::Wildcard), None);
        assert_eq!(var_sort(&ctx, &Var::Empty), None);
    }

    #[test]
    fn nil_is_closed() {
        assert!(Closed::new(Par::default()).is_some());
    }

    #[test]
    fn free_variable_is_not_closed() {
        assert!(Closed::new(free_var(0)).is_none());
    }

    #[test]
    fn bound_variable_is_closed() {
        let bound = Par {
            exprs: vec![Expr::EVar(Box::new(Var::BoundVar(0)))],
            ..Default::default()
        };
        assert!(is_closed(&bound));
    }

    #[test]
    fn par_merge_preserves_closedness() {
        assert!(is_closed(&g_int(1).par_merge(&g_int(2))));
        assert!(!is_closed(&free_var(0).par_merge(&g_int(2))));
    }

    fn bound_var(l: i32) -> Par {
        Par {
            exprs: vec![Expr::EVar(Box::new(Var::BoundVar(l)))],
            ..Default::default()
        }
    }

    #[test]
    fn well_scoped_bound_level_within_ctx() {
        let ctx = vec![PSort::Name, PSort::Proc];
        assert!(well_scoped(&ctx, &bound_var(0)));
        assert!(well_scoped(&ctx, &bound_var(1)));
        assert!(!well_scoped(&ctx, &bound_var(2)));
        assert!(!well_scoped(&ctx, &bound_var(-1)));
    }

    #[test]
    fn well_scoped_free_and_wildcard_are_always_in_scope() {
        let ctx: Ctx = vec![];
        assert!(well_scoped(&ctx, &free_var(0)));
        assert!(well_scoped(&ctx, &Par::default()));
    }

    #[test]
    fn well_scoped_newtype_validates() {
        let ctx = vec![PSort::Proc];
        assert!(WellScoped::new(ctx.clone(), bound_var(0)).is_some());
        assert!(WellScoped::new(ctx.clone(), bound_var(1)).is_none());
    }

    #[test]
    fn well_scoped_discharges_to_par() {
        let ctx = vec![PSort::Proc];
        let ws = WellScoped::new(ctx, bound_var(0)).unwrap();
        let p: Par = ws.into();
        assert_eq!(p, bound_var(0));
    }

    #[test]
    fn well_scoped_accounts_for_internal_binders() {
        // `new x in { x }` — the body's `BoundVar(0)` references the `new` binder.
        let program = Par {
            news: vec![New {
                bind_count: FreeCount::ONE,
                p: Box::new(Par {
                    exprs: vec![Expr::EVar(Box::new(Var::BoundVar(0)))],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(well_scoped(&vec![], &program));
    }

    #[test]
    fn free_count_rejects_negative() {
        assert!(FreeCount::new(-1).is_none());
        assert!(FreeCount::new(0).is_some());
        assert_eq!(i32::from(FreeCount::new(3).unwrap()), 3);
    }

    /// Every way into the carrier is non-negative — the class-level statement `from_nonneg` broke
    /// (U1 site 5). Its `debug_assert!(n >= 0)` is compiled out in release, so a negative count
    /// crossed the boundary silently in exactly the builds that matter; the total constructor now
    /// takes a `usize` ([`FreeCount::from_len`]), where the sign cannot be written.
    #[test]
    fn every_free_count_constructor_stays_in_the_domain() {
        for n in [-1, 0, 1, i32::MIN, i32::MAX] {
            if let Some(fc) = FreeCount::new(n) {
                assert!(i32::from(fc) >= 0);
            }
            if let Ok(fc) = FreeCount::try_from(n) {
                assert!(i32::from(fc) >= 0);
            }
        }
        for n in [0usize, 1, usize::MAX] {
            assert!(i32::from(FreeCount::from_len(n)) >= 0);
        }
        assert!(i32::from(FreeCount::ONE) >= 0);
        assert!(i32::from(FreeCount::ZERO) >= 0);
    }

    /// The total constructor's only escape is width, and it saturates rather than wrapping — the
    /// house rule for this shape (`shared/src/refined.rs`'s `BlockHeight`/`SeqNum`): a wrap would
    /// yield a *negative* "non-negative" count.
    #[test]
    fn free_count_from_len_saturates_and_add_preserves_the_invariant() {
        assert_eq!(i32::from(FreeCount::from_len(5)), 5);
        assert_eq!(i32::from(FreeCount::from_len(0)), 0);
        assert_eq!(i32::from(FreeCount::from_len(usize::MAX)), i32::MAX);
        assert_eq!(
            i32::from(FreeCount::from_len(2) + FreeCount::from_len(3)),
            5
        );
        assert_eq!(
            i32::from(FreeCount::from_len(usize::MAX) + FreeCount::from_len(usize::MAX)),
            i32::MAX,
            "a saturated sum stays in the domain"
        );
    }

    #[test]
    fn receive_pattern_binder_is_closed() {
        // `for (x <- ch) { Nil }` has no free variables: the pattern `x` is a binder.
        let recv = Par {
            receives: vec![Receive {
                binds: vec![ReceiveBind {
                    patterns: vec![Par {
                        exprs: vec![Expr::EVar(Box::new(Var::FreeVar(0)))],
                        ..Default::default()
                    }],
                    source: Box::new(Par::default()),
                    remainder: None,
                    free_count: FreeCount::ONE,
                }],
                body: Box::new(Par::default()),
                persistent: false,
                peek: false,
                bind_count: FreeCount::ONE,
                locally_free: Default::default(),
                connective_used: false,
            }],
            ..Default::default()
        };
        assert!(is_closed(&recv));
    }

    #[test]
    fn match_pattern_binder_is_closed() {
        // `match x { y => Nil }` has no free variables: the case pattern `y` is a binder.
        let m = Par {
            matches: vec![Match {
                target: Box::new(g_int(1).quote()),
                cases: vec![MatchCase {
                    pattern: Box::new(
                        Par {
                            exprs: vec![Expr::EVar(Box::new(Var::FreeVar(0)))],
                            ..Default::default()
                        }
                        .quote(),
                    ),
                    source: Box::new(Par::default()),
                    free_count: FreeCount::ONE,
                }],
                locally_free: Default::default(),
                connective_used: false,
            }],
            ..Default::default()
        };
        assert!(is_closed(&m));
    }

    #[test]
    fn evaluation_free_var_is_still_not_closed() {
        // A free variable in a send's data (evaluation position) is genuinely free.
        let send = Par {
            sends: vec![Send {
                data: vec![Par {
                    exprs: vec![Expr::EVar(Box::new(Var::FreeVar(0)))],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(!is_closed(&send));
    }
}

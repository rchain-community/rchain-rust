//! Pretty-printer for rholang terms (port of `interpreter/PrettyPrinter.scala`).
//!
//! The Scala `Coeval` laziness and the `PRETTY_PRINTER_OUTPUT_TRIM_AFTER` cap are dropped; strings
//! are built eagerly. Variable naming (`rotate`/`increment`) and the de Bruijn level shifts are
//! preserved faithfully.

use rchain_models::ast::{
    Bundle, Connective, Expr, GUnforgeable, Match, MatchCase, New, Par, Receive, Send, Sort, Var,
};
use rchain_models::par_ops::is_nil;
use rchain_models::sorter::{sort_pairs, sort_pars};
use rchain_shared::base16;

const INDENT: &str = "  ";

/// Wrap a rendered sub-expression in parentheses unless it is already parenthesised or a bare
/// integer (port of `StringOps.wrapWithBraces`).
fn wrap_with_braces(expr: &str) -> String {
    if expr.parse::<i64>().is_ok() {
        expr.to_string()
    } else if expr.starts_with('(') && expr.ends_with(')') {
        expr.to_string()
    } else {
        format!("({expr})")
    }
}

#[derive(Clone, Debug)]
pub struct PrettyPrinter {
    free_shift: i32,
    bound_shift: i32,
    news_shift_indices: Vec<i32>,
    free_id: String,
    base_id: String,
    rotation: i32,
    max_var_count: i32,
    is_building_channel: bool,
}

impl Default for PrettyPrinter {
    fn default() -> Self {
        PrettyPrinter {
            free_shift: 0,
            bound_shift: 0,
            news_shift_indices: Vec::new(),
            free_id: "free".to_string(),
            base_id: "a".to_string(),
            rotation: 23,
            max_var_count: 128,
            is_building_channel: false,
        }
    }
}

impl PrettyPrinter {
    pub fn new() -> Self {
        PrettyPrinter::default()
    }

    fn bound_id(&self) -> String {
        rotate(&self.base_id, self.rotation)
    }

    fn set_base_id(&self) -> String {
        increment(&self.base_id)
    }

    fn is_new_var(&self, level: i32) -> bool {
        self.news_shift_indices
            .contains(&(self.bound_shift - level - 1))
    }

    /// Top-level: render a `Par`.
    pub fn build_string<S: Sort>(&self, p: &Par<S>) -> String {
        self.build_par(p, 0)
    }

    pub fn build_expr(&self, e: &Expr) -> String {
        self.build_expr_inner(e)
    }

    pub fn build_var(&self, v: &Var) -> String {
        self.build_var_inner(v)
    }

    pub fn build_unforgeable(&self, u: &GUnforgeable) -> String {
        self.build_unforgeable_inner(u)
    }

    pub fn build_channel(&self, p: &Par) -> String {
        self.build_channel_inner(p, 0)
    }

    fn with(&self, f: impl FnOnce(&mut PrettyPrinter)) -> PrettyPrinter {
        let mut c = self.clone();
        f(&mut c);
        c
    }

    // --- term dispatch -------------------------------------------------

    fn build_par<S: Sort>(&self, p: &Par<S>, indent: i32) -> String {
        if is_nil(p) {
            return "Nil".to_string();
        }
        let mut groups: Vec<(&'static str, Vec<String>)> = Vec::new();
        if !p.bundles.is_empty() {
            groups.push((
                "bundles",
                p.bundles
                    .iter()
                    .map(|b| self.build_bundle(b, indent))
                    .collect(),
            ));
        }
        if !p.sends.is_empty() {
            groups.push((
                "sends",
                p.sends.iter().map(|s| self.build_send(s, indent)).collect(),
            ));
        }
        if !p.receives.is_empty() {
            groups.push((
                "receives",
                p.receives
                    .iter()
                    .map(|r| self.build_receive(r, indent))
                    .collect(),
            ));
        }
        if !p.news.is_empty() {
            groups.push((
                "news",
                p.news.iter().map(|n| self.build_new(n, indent)).collect(),
            ));
        }
        if !p.exprs.is_empty() {
            groups.push((
                "exprs",
                p.exprs.iter().map(|e| self.build_expr_inner(e)).collect(),
            ));
        }
        if !p.matches.is_empty() {
            groups.push((
                "matches",
                p.matches
                    .iter()
                    .map(|m| self.build_match(m, indent))
                    .collect(),
            ));
        }
        if !p.unforgeables.is_empty() {
            groups.push((
                "unforgeables",
                p.unforgeables
                    .iter()
                    .map(|u| self.build_unforgeable_inner(u))
                    .collect(),
            ));
        }
        if !p.connectives.is_empty() {
            groups.push((
                "connectives",
                p.connectives
                    .iter()
                    .map(|c| self.build_connective(c))
                    .collect(),
            ));
        }

        // The separator is emitted **once per item, before it** (`intersperse`), and the
        // between-groups separator once per non-empty group — the shape the Scala `buildStringM(Par)`
        // uses (`PrettyPrinter.scala:288-302`, where `prevNonEmpty` is the *group* accumulator). A
        // version that set the "previous item was printed" flag inside the item loop emitted two
        // separators between the first two items of a group (`a |\n |\nb`), which is not parsable
        // rholang — the round-trip test below caught it.
        let mut out = String::new();
        let mut prev_group = false;
        for (_, items) in groups {
            if items.is_empty() {
                continue;
            }
            if prev_group {
                out.push_str(" |\n");
                out.push_str(&INDENT.repeat(indent as usize));
            }
            for (i, item) in items.iter().enumerate() {
                out.push_str(item);
                if i != items.len() - 1 {
                    out.push_str(" |\n");
                    out.push_str(&INDENT.repeat(indent as usize));
                }
            }
            prev_group = true;
        }
        out
    }

    fn build_send(&self, s: &Send, indent: i32) -> String {
        let chan = self.build_channel_inner(&s.chan, indent);
        let data = build_seq(
            &s.data
                .iter()
                .map(|p| self.build_string(p))
                .collect::<Vec<_>>(),
        );
        let op = if s.persistent { "!!(" } else { "!(" };
        format!("{chan}{op}{data})")
    }

    fn build_receive(&self, r: &Receive, indent: i32) -> String {
        let mut total_free = 0i32;
        let mut binds_string = String::new();
        for (i, bind) in r.binds.iter().enumerate() {
            let printer = self.with(|c| {
                c.free_shift = self.bound_shift + total_free;
                c.bound_shift = 0;
                c.free_id = self.bound_id();
                c.base_id = self.set_base_id();
            });
            let bind_string = printer.build_pattern(&bind.patterns);
            let arrow = if r.persistent {
                " <= "
            } else if r.peek {
                " <<- "
            } else {
                " <- "
            };
            binds_string.push_str(&bind_string);
            binds_string.push_str(arrow);
            binds_string.push_str(&self.build_channel_inner(&bind.source, indent));
            if i != r.binds.len() - 1 {
                binds_string.push_str("  & ");
            }
            total_free += i32::from(bind.free_count);
        }

        let body_printer = self.with(|c| c.bound_shift += total_free);
        let body_str = body_printer.build_par(&r.body, indent + 1);
        if !body_str.is_empty() {
            format!(
                "for( {binds_string} ) {{\n{}{body_str}\n{}}}",
                INDENT.repeat((indent + 1) as usize),
                INDENT.repeat(indent as usize)
            )
        } else {
            format!("for( {binds_string} ) {{{body_str}}}")
        }
    }

    fn build_bundle(&self, b: &Bundle, indent: i32) -> String {
        // The Scala `BundleOps.showInstance` is `"%-8s".format(s"bundle$sign")`: the keyword is part
        // of the rendering and the whole prefix is **padded to eight columns**, so a bundle prints as
        // `bundle0 { … }` (with one space from the padding) and a read-write bundle as `bundle  { … }`
        // (`BundleReadWrite ::= "bundle"` — the signless spelling is in the grammar). Printing only
        // the sign produced `0{ … }`, which does not re-parse; the round-trip test below caught it.
        let sign = if b.read_flag && b.write_flag {
            ""
        } else if b.read_flag && !b.write_flag {
            "-"
        } else if !b.read_flag && b.write_flag {
            "+"
        } else {
            "0"
        };
        let prefix = format!("{:<8}", format!("bundle{sign}"));
        format!(
            "{prefix}{{ {}{} }}",
            INDENT.repeat((indent + 1) as usize),
            self.build_par(&b.body, indent + 1)
        )
    }

    fn build_new(&self, n: &New, indent: i32) -> String {
        let introduced: Vec<i32> = (0..n.bind_count).map(|i| i + self.bound_shift).collect();
        let variables = self.build_variables(n.bind_count);
        let body = self.with(|c| {
            c.bound_shift += n.bind_count;
            c.news_shift_indices.extend(introduced.iter().copied());
        });
        format!(
            "new {variables} in {{\n{}{}\n{}}}",
            INDENT.repeat((indent + 1) as usize),
            body.build_par(&n.p, indent + 1),
            INDENT.repeat(indent as usize)
        )
    }

    fn build_match(&self, m: &Match, indent: i32) -> String {
        let mut out = format!("match {} {{\n", self.build_string(&m.target));
        for (i, case) in m.cases.iter().enumerate() {
            out.push_str(&INDENT.repeat((indent + 1) as usize));
            out.push_str(&self.build_match_case(case, indent + 1));
            if i != m.cases.len() - 1 {
                out.push('\n');
            }
        }
        out.push_str(&format!("\n{}}}", INDENT.repeat(indent as usize)));
        out
    }

    fn build_connective(&self, c: &Connective) -> String {
        match c {
            Connective::ConnAnd(body) => format!(
                "{{{}}}",
                body.ps
                    .iter()
                    .map(|p| self.build_string(p))
                    .collect::<Vec<_>>()
                    .join(" /\\ ")
            ),
            Connective::ConnOr(body) => format!(
                "{{{}}}",
                body.ps
                    .iter()
                    .map(|p| self.build_string(p))
                    .collect::<Vec<_>>()
                    .join(" \\/ ")
            ),
            Connective::ConnNot(p) => format!("~{{{}}}", self.build_string(p)),
            Connective::VarRef(v) => format!("={}{}", self.free_id, self.free_shift - v.index - 1),
            Connective::ConnBool(_) => "Bool".to_string(),
            Connective::ConnInt(_) => "Int".to_string(),
            Connective::ConnBigInt(_) => "BigInt".to_string(),
            Connective::ConnString(_) => "String".to_string(),
            Connective::ConnUri(_) => "Uri".to_string(),
            Connective::ConnByteArray(_) => "ByteArray".to_string(),
            Connective::Empty => String::new(),
        }
    }

    // --- expr / var / unforgeable --------------------------------------

    fn build_expr_inner(&self, e: &Expr) -> String {
        match e {
            Expr::ENeg(p) => format!("-{}", wrap_with_braces(&self.build_string(p))),
            Expr::ENot(p) => format!("~{}", wrap_with_braces(&self.build_string(p))),
            Expr::EMult(p1, p2) => wrap_with_braces(&format!(
                "{} * {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EDiv(p1, p2) => wrap_with_braces(&format!(
                "{} / {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EMod(p1, p2) => wrap_with_braces(&format!(
                "{} % {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EPercentPercent(p1, p2) => wrap_with_braces(&format!(
                "{} %% {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EPlus(p1, p2) => wrap_with_braces(&format!(
                "{} + {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EPlusPlus(p1, p2) => wrap_with_braces(&format!(
                "{} ++ {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EMinus(p1, p2) => wrap_with_braces(&format!(
                "{} - {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EMinusMinus(p1, p2) => wrap_with_braces(&format!(
                "{} -- {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EAnd(p1, p2) => wrap_with_braces(&format!(
                "{} and {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EOr(p1, p2) => wrap_with_braces(&format!(
                "{} or {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EShortAnd(p1, p2) => wrap_with_braces(&format!(
                "{} && {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EShortOr(p1, p2) => wrap_with_braces(&format!(
                "{} || {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EEq(p1, p2) => wrap_with_braces(&format!(
                "{} == {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::ENeq(p1, p2) => wrap_with_braces(&format!(
                "{} != {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EGt(p1, p2) => wrap_with_braces(&format!(
                "{} > {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EGte(p1, p2) => wrap_with_braces(&format!(
                "{} >= {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::ELt(p1, p2) => wrap_with_braces(&format!(
                "{} < {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::ELte(p1, p2) => wrap_with_braces(&format!(
                "{} <= {}",
                self.build_string(p1),
                self.build_string(p2)
            )),
            Expr::EMatches(target, pattern) => wrap_with_braces(&format!(
                "{} matches {}",
                self.build_string(target),
                self.build_string(pattern)
            )),
            Expr::EList(list) => {
                let seq = build_seq(
                    &list
                        .ps
                        .iter()
                        .map(|p| self.build_string(p))
                        .collect::<Vec<_>>(),
                );
                format!("[{seq}{}]", self.build_remainder(&list.remainder))
            }
            Expr::ETuple(tuple) => {
                let seq = build_seq(
                    &tuple
                        .ps
                        .iter()
                        .map(|p| self.build_string(p))
                        .collect::<Vec<_>>(),
                );
                format!("({seq})")
            }
            Expr::ESet(set) => {
                let seq = build_seq(
                    &sort_pars(set.ps.clone())
                        .iter()
                        .map(|p| self.build_string(p))
                        .collect::<Vec<_>>(),
                );
                format!("Set({seq}{})", self.build_remainder(&set.remainder))
            }
            Expr::EMap(map) => {
                let pairs = sort_pairs(map.kvs.clone());
                let body = pairs
                    .iter()
                    .enumerate()
                    .map(|(i, (k, v))| {
                        let sep = if i != pairs.len() - 1 { ", " } else { "" };
                        format!("{} : {}{}", self.build_string(k), self.build_string(v), sep)
                    })
                    .collect::<String>();
                format!("{{{body}{}}}", self.build_remainder(&map.remainder))
            }
            Expr::EVar(v) => self.build_var_inner(v),
            Expr::GBool(b) => b.to_string(),
            Expr::GInt(i) => i.to_string(),
            Expr::GBigInt(bi) => format!("BigInt({bi})"),
            Expr::GString(s) => format!("\"{s}\""),
            Expr::GUri(u) => format!("`{u}`"),
            Expr::GByteArray(bs) => base16::encode(bs),
            Expr::EMethod(m) => {
                let args = m
                    .arguments
                    .iter()
                    .map(|p| self.build_string(p))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "({}).{}({args})",
                    self.build_string(&m.target),
                    m.method_name
                )
            }
        }
    }

    fn build_var_inner(&self, v: &Var) -> String {
        match v {
            Var::FreeVar(level) => format!("{}{}", self.free_id, self.free_shift + level),
            Var::BoundVar(level) => {
                let star = if self.is_new_var(*level) && !self.is_building_channel {
                    "*"
                } else {
                    ""
                };
                format!("{star}{}{}", self.bound_id(), self.bound_shift - level - 1)
            }
            Var::Wildcard => "_".to_string(),
            Var::Empty => "@Nil".to_string(),
        }
    }

    fn build_unforgeable_inner(&self, u: &GUnforgeable) -> String {
        match u {
            GUnforgeable::GPrivate(p) => format!("Unforgeable(0x{})", base16::encode(&p.id)),
            GUnforgeable::GDeployId(id) => format!("DeployId(0x{})", base16::encode(&id.sig)),
            GUnforgeable::GDeployerId(id) => {
                format!("DeployerId(0x{})", base16::encode(&id.public_key))
            }
            GUnforgeable::GSysAuthToken => "GSysAuthTokenBody()".to_string(),
            GUnforgeable::Empty => "Nil".to_string(),
        }
    }

    // --- channel / pattern helpers -------------------------------------

    fn build_channel_inner<S: Sort>(&self, p: &Par<S>, indent: i32) -> String {
        let printer = self.with(|c| c.is_building_channel = true);
        let rendered = printer.build_par(p, indent);
        let b = if rendered.len() > 60 {
            rendered
        } else {
            rendered.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        if self.is_bound_new(p) {
            b
        } else {
            format!("@{{{b}}}")
        }
    }

    fn is_bound_new<S: Sort>(&self, p: &Par<S>) -> bool {
        if let [Expr::EVar(v)] = p.exprs.as_slice() {
            if let Var::BoundVar(level) = v.as_ref() {
                return self.is_new_var(*level);
            }
        }
        false
    }

    fn build_remainder(&self, remainder: &Option<Box<Var>>) -> String {
        match remainder {
            Some(v) => format!("...{}", self.build_var_inner(v)),
            None => String::new(),
        }
    }

    fn build_variables(&self, bind_count: i32) -> String {
        let n = self.max_var_count.min(bind_count);
        (0..n)
            .map(|i| format!("{}{}", self.bound_id(), self.bound_shift + i))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn build_pattern<S: Sort>(&self, patterns: &[Par<S>]) -> String {
        patterns
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let s = self.build_channel_inner(p, 0);
                if i != patterns.len() - 1 {
                    format!("{s}, ")
                } else {
                    s
                }
            })
            .collect()
    }

    fn build_match_case(&self, case: &MatchCase, indent: i32) -> String {
        let pattern_printer = self.with(|c| {
            c.free_shift = self.bound_shift;
            c.bound_shift = 0;
            c.free_id = self.bound_id();
            c.base_id = self.set_base_id();
        });
        let pattern = pattern_printer.build_string(&case.pattern);
        let source = self
            .with(|c| c.bound_shift += i32::from(case.free_count))
            .build_par(&case.source, indent + 1);
        format!(
            "{pattern} => {{\n{}{source}\n{}}}",
            INDENT.repeat((indent + 1) as usize),
            INDENT.repeat(indent as usize)
        )
    }
}

fn build_seq(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if i != items.len() - 1 {
                format!("{s}, ")
            } else {
                s.clone()
            }
        })
        .collect()
}

/// Increment a base-id string (`a` → `b`, `z` → `aa`, `az` → `ba`).
fn increment(id: &str) -> String {
    let last = id.chars().last().unwrap_or('a');
    let new_id = increment_char(last).to_string();
    if new_id == "a" {
        if id.chars().count() > 1 {
            format!("{}{}", increment(&id[..id.len() - 1]), new_id)
        } else {
            "aa".to_string()
        }
    } else {
        format!("{}{}", &id[..id.len() - 1], new_id)
    }
}

fn increment_char(c: char) -> char {
    (((c as i32 + 1 - 97).rem_euclid(26)) + 97) as u8 as char
}

fn rotate(id: &str, rotation: i32) -> String {
    id.chars()
        .map(|c| (((c as i32 + rotation - 97).rem_euclid(26)) + 97) as u8 as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_models::ast::EList;

    fn expr(e: Expr) -> Par {
        rchain_models::par_ops::from_expr(e)
    }

    #[test]
    fn prints_ground_terms() {
        let p = PrettyPrinter::new();
        assert_eq!(p.build_expr(&Expr::GInt(42)), "42");
        assert_eq!(p.build_expr(&Expr::GString("hi".to_string())), "\"hi\"");
        assert_eq!(p.build_expr(&Expr::GBool(true)), "true");
        assert_eq!(
            p.build_expr(&Expr::GUri("rho:io:stdout".to_string())),
            "`rho:io:stdout`"
        );
    }

    #[test]
    fn prints_arithmetic() {
        let p = PrettyPrinter::new();
        let e = Expr::EPlus(Box::new(expr(Expr::GInt(1))), Box::new(expr(Expr::GInt(2))));
        assert_eq!(p.build_expr(&e), "(1 + 2)");
    }

    #[test]
    fn prints_list() {
        let p = PrettyPrinter::new();
        let list = Expr::EList(EList {
            ps: vec![expr(Expr::GInt(1)), expr(Expr::GInt(2))],
            ..Default::default()
        });
        assert_eq!(p.build_expr(&list), "[1, 2]");
    }

    #[test]
    fn increments_ids() {
        assert_eq!(increment("a"), "b");
        assert_eq!(increment("z"), "aa");
        assert_eq!(increment("az"), "ba");
    }
}

#[cfg(test)]
mod printer_tests {
    use super::*;

    use rchain_models::par_ops::from_expr;

    /// `wrapWithBraces` wraps a sub-expression in parentheses **unless** it is already a bare integer
    /// or already parenthesised. The bare-integer case is why `1 + 2` prints without redundant
    /// brackets, and the already-parenthesised case is what keeps a nested print stable.
    #[test]
    fn wrap_with_braces_leaves_integers_and_parenthesised_subexpressions_alone() {
        assert_eq!(wrap_with_braces("42"), "42");
        assert_eq!(
            wrap_with_braces("-7"),
            "-7",
            "a negative integer is still an integer"
        );
        assert_eq!(wrap_with_braces("(1 + 2)"), "(1 + 2)");
        assert_eq!(wrap_with_braces("1 + 2"), "(1 + 2)");
        assert_eq!(wrap_with_braces("a"), "(a)");
        // Only a *bare* integer is exempt: a number with a suffix or a decimal point is not.
        assert_eq!(wrap_with_braces("1n"), "(1n)");
        assert_eq!(wrap_with_braces("1.0"), "(1.0)");
    }

    /// The empty process prints as `Nil`; a ground value prints as itself.
    #[test]
    fn nil_and_grounds_print_plainly() {
        let printer = PrettyPrinter::new();
        assert_eq!(
            printer.build_string(&Par::<rchain_models::ast::ProcSort>::default()),
            "Nil"
        );
        assert_eq!(printer.build_string(&from_expr(Expr::GInt(42))), "42");
        assert_eq!(
            printer.build_string(&from_expr(Expr::GString("hi".to_string()))),
            "\"hi\""
        );
        assert_eq!(printer.build_string(&from_expr(Expr::GBool(true))), "true");
    }

    /// **The round trip is the printer's real contract**: printing a parsed term and parsing the
    /// result must give back the same term. An assertion on the exact text would pin a formatting
    /// choice; this pins *fidelity*, across every construct the printer handles — sends (persistent
    /// and not), receives, `new`, `match`, collections, connectives, bundles and arithmetic.
    #[test]
    fn printing_and_reparsing_is_the_identity() {
        let corpus = [
            // Grounds and collections.
            "42",
            "\"str\"",
            "true",
            "[1, 2, 3]",
            "(1, 2)",
            "Set(1, 2)",
            "{\"k\": 1}",
            "[]",
            // Sends and receives.
            "@\"c\"!(1)",
            "@\"c\"!!(1)",
            "@\"c\"!(1, 2)",
            "for (@x <- @\"c\") { Nil }",
            "for (@x <= @\"c\") { Nil }",
            "for (@x <- @\"a\" & @y <- @\"b\") { Nil }",
            // Binding and control.
            "new x in { x!(1) }",
            "new x, y in { x!(*y) }",
            "contract @\"c\"(@x) = { Nil }",
            "if (true) { Nil } else { Nil }",
            "match 1 { 0 => Nil _ => Nil }",
            // Arithmetic and connectives.
            "1 + 2 * 3",
            "1 - 2",
            "-1",
            "1 < 2",
            "1 == 2",
            "true and false",
            "true or false",
            "\"a\" ++ \"b\"",
            "[1] ++ [2]",
            "1 % 2",
            "1 / 2",
            // Parallel composition and references.
            "@\"a\"!(1) | @\"b\"!(2)",
            "bundle0 { Nil }",
            "bundle+ { Nil }",
        ];

        // The printer renders a *normalized* term, so the corpus is closed terms (the normalizer
        // rejects a globally free variable) and the comparison is between two normalized pars.
        for src in corpus {
            let first: Par = crate::normalizer::source_to_adt(src)
                .unwrap_or_else(|e| panic!("the corpus term {src:?} must parse: {e}"))
                .into();
            let printed = PrettyPrinter::new().build_string(&first);
            let second: Par = crate::normalizer::source_to_adt(&printed)
                .unwrap_or_else(|e| {
                    panic!("printing {src:?} produced {printed:?}, which does not parse: {e}")
                })
                .into();
            assert_eq!(
                second, first,
                "printing {src:?} produced {printed:?}, which reparses differently"
            );
        }
    }

    /// A bound variable is printed with a **generated name**, not its de Bruijn index: the printer
    /// walks `new`s and receives and emits `a`, `b`, … so a printed term is re-parsable source rather
    /// than internal indices. The free variables get the `free` prefix.
    #[test]
    fn variables_are_printed_with_generated_names() {
        let printer = PrettyPrinter::new();
        let term: Par = crate::normalizer::source_to_adt("new x in { x!(1) }")
            .expect("parse")
            .into();
        let printed = printer.build_string(&term);
        assert!(printed.contains("new "), "{printed}");
        assert!(
            !printed.contains("BoundVar"),
            "indices must not leak: {printed}"
        );

        // A free variable prints with the `free` prefix (the Scala `freeId`).
        let free = printer.build_string(&from_expr(Expr::EVar(Box::new(Var::FreeVar(0)))));
        assert!(free.starts_with("free"), "{free}");
        // A bound variable with no binder in scope prints as a *generated* name (`x-1`), which is
        // re-parsable as a variable — never as its internal index.
        let bound = printer.build_string(&from_expr(Expr::EVar(Box::new(Var::BoundVar(0)))));
        assert_eq!(bound, "x-1", "{bound}");
        assert!(crate::parser::parse(&bound).is_ok(), "{bound} must reparse");
    }

    /// A channel is printed as a **quoted name** (`@`), which is what makes the output re-parsable in
    /// name position — a send whose channel lost its `@` would reparse as a variable reference.
    #[test]
    fn a_channel_is_printed_as_a_quoted_name() {
        let term: Par = crate::normalizer::source_to_adt("@\"c\"!(1)")
            .expect("parse")
            .into();
        let printed = PrettyPrinter::new().build_string(&term);
        // A ground channel is printed **braced** (`@{"c"}`), which is what keeps the output
        // unambiguous when re-parsed (the braces are a group around the same ground name).
        assert!(printed.contains("@{\"c\"}"), "{printed}");
        assert!(printed.contains("!("), "{printed}");
    }

    /// **Two faithful printer warts, both inherited from the Scala `PrettyPrinter`.** Each renders a
    /// term as something the grammar does not read back as the same term, so a printed term must not
    /// be pasted into the REPL as source:
    ///
    /// * a **one-element tuple** `(1,)` renders `(1)` — the trailing comma is dropped
    ///   (`PrettyPrinter.scala:117`: `"(" buildSeq ")"`), and `(1)` is the *group* `1`;
    /// * `not x` renders `~(x)` (`PrettyPrinter.scala:75`: `"~" buildString(p).wrapWithBraces`), and
    ///   `~` is the *process* negation whose operand is parsed below the parenthesised-expression
    ///   level, so `~(x)` is a syntax error, not a parenthesised operand.
    ///
    /// Both are recorded in `spec/AUDIT.md` §16 and pinned here, so making the output re-parsable is a
    /// deliberate change with a test to update rather than an accident.
    #[test]
    fn the_documented_warts_print_what_the_grammar_cannot_read_back() {
        // The one-element tuple.
        let tuple: Par = crate::normalizer::source_to_adt("(1,)")
            .expect("parse")
            .into();
        let printed = PrettyPrinter::new().build_string(&tuple);
        assert_eq!(printed, "(1)", "the trailing comma is dropped, as in Scala");
        let reparsed: Par = crate::normalizer::source_to_adt(&printed)
            .expect("the printed form still parses")
            .into();
        assert_ne!(
            reparsed, tuple,
            "the round trip is not the identity for a one-element tuple"
        );

        // The boolean negation.
        let negated: Par = crate::normalizer::source_to_adt("not true")
            .expect("parse")
            .into();
        let printed = PrettyPrinter::new().build_string(&negated);
        assert_eq!(
            printed, "~(true)",
            "`not` is printed as the tilde, as in Scala"
        );
        assert!(
            crate::normalizer::source_to_adt(&printed).is_err(),
            "…and the printed form is not valid rholang: {printed}"
        );
    }
}

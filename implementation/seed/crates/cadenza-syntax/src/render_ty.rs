//! Render a decoded Ty-payload arena subtree to its type NAME — the canonical, host-agnostic type-name
//! surface for the sidecar wire (`cdz exports "name : type"`, `cdz type`, editor hover).
//!
//! The sidecar responses carry the FULL structured `Ty` payload on the wire (the shape `rcdzc`'s
//! `eval::encode_ty` builds), not a pre-rendered string (operator: "binary AST is the data exchange
//! format"). A host command must render that decoded payload to a clean type NAME. `cdz` has NO `rcdzc`
//! dependency, so it cannot call `rcdzc::ty::Ty::render_name`; this module is the canonical renderer over
//! the decoded `cadenza_ast` subtree, living in the printer's home crate.
//!
//! PARITY: [`render_ty_name`] renders byte-IDENTICALLY to `rcdzc` `Ty::render_name` (ty.rs) for a
//! monomorphic type, and [`render_ty_scheme`] to `Scheme::render_scheme` (its `render_named_vars` +
//! first-encounter `a`,`b`,`c`… var lettering) for a polymorphic one. Those functions are the source of
//! truth for the exact spelling (`Int64` not `(Int 64)`, `(-> A B)` curried, `Option a`, …).
//!
//! The payload grammar (head-name keyed; `encode_ty`):
//! - bare `Name` leaf: `Bool`/`Unit`/`String`/`Char`/`Symbol`/`BigInt`/`Rational`/`Bytes`/`Type`/`Any`.
//! - `(Int W)` / `(UInt W)` / `(Float W)` — `W` a width `Int` leaf.
//! - `(-> P R)` — a function (curried: nested `->` for multi-arg).
//! - `(Tuple E…)`, `(Record (: name T)…)` (fields pre-sorted), `(List E)`, `(Set E)`, `(Map K V)`.
//! - `(Sum NAME <decl> arg…)` — the `<decl>` child (index 2) is an INTERNAL arena occurrence id and is
//!   HIDDEN; render the nominal name, applied to args when generic.
//! - `(Nominal NAME <decl> (args…) INNER)` — identity is the NAME; the `<decl>`, `(args…)`, and `INNER`
//!   are hidden (`render_name` renders only the declared name).
//! - `(Qty INNER (unit (base NAME EXP)… [(scale N D)]))`, `(Cont RESUME ANSWER)`.
//! - `(Var N)` — a type-variable number: `_` in [`render_ty_name`], a stable letter in [`render_ty_scheme`].
//!
//! TOTAL: an unknown head or a malformed/short subtree renders to a defined fallback (its raw shape, else
//! `?`), NEVER a panic — these feed editor hover on possibly-incomplete programs. A DEPTH GUARD (mirroring
//! `render_name`'s `MAX_RENDER_DEPTH`) truncates an explosively-deep type with `…`.

use crate::ast::{Arenas, Struct, StructId};
use std::collections::BTreeMap;

/// The recursion cap — mirrors `rcdzc` `Ty::render_name`'s `MAX_RENDER_DEPTH` (24): a diagnostic never
/// needs deeper, and it keeps the renderer total on a pathologically deep decoded arena.
const MAX_RENDER_DEPTH: u32 = 24;

/// Render a monomorphic Ty payload rooted at `root` to its type name. A `(Var N)` renders as `_` (parity
/// with `Ty::render_name`, which collapses every unsolved var to the `_` placeholder).
pub fn render_ty_name(a: &Arenas, root: StructId) -> String {
    render(a, root, 0, None)
}

/// Render a Ty payload rooted at `root` as a SCHEME: each DISTINCT `(Var N)` gets a stable letter
/// (`a`, `b`, `c`, …, then `a1`, `b1`, … past 26) in FIRST-ENCOUNTER order, so a reader sees which vars
/// are the same quantified variable and which differ (parity with `Scheme::render_scheme`). Non-`Var`
/// nodes render exactly as [`render_ty_name`].
pub fn render_ty_scheme(a: &Arenas, root: StructId) -> String {
    // First-encounter order of DISTINCT var numbers, walking the same structure `render` visits — this
    // mirrors `Ty::collect_free_vars` (which visits `Sum`/`Nominal` ARGS but not a `Nominal`'s inner, and
    // a `Qty`'s inner but not its unit), so the letters line up with what `render` emits.
    let mut order: Vec<String> = Vec::new();
    collect_vars(a, root, 0, &mut order);
    let mut names = BTreeMap::new();
    for (i, v) in order.iter().enumerate() {
        let letter = (b'a' + (i % 26) as u8) as char;
        let suffix = i / 26;
        let name = if suffix == 0 {
            letter.to_string()
        } else {
            format!("{letter}{suffix}")
        };
        names.insert(v.clone(), name);
    }
    render(a, root, 0, Some(&names))
}

/// The width of an `(Int W)`/`(UInt W)`/`(Float W)` head — the decimal string of its width `Int` leaf,
/// or `None` if the child is missing / not an integer (a malformed payload).
fn width_str(a: &Arenas, id: StructId) -> Option<String> {
    a.as_int(id).map(|v| v.to_decimal_string())
}

/// The decimal string of a `(Var N)`'s number child (the map key), or `None` if malformed.
fn var_num(a: &Arenas, kids: &[StructId]) -> Option<String> {
    kids.get(1)
        .and_then(|&n| a.as_int(n))
        .map(|v| v.to_decimal_string())
}

/// Recurse the payload, emitting the type name. `vars = Some(map)` renders a `(Var N)` as its letter
/// (scheme mode); `None` renders it as `_` (monomorphic mode).
fn render(a: &Arenas, id: StructId, depth: u32, vars: Option<&BTreeMap<String, String>>) -> String {
    if depth >= MAX_RENDER_DEPTH {
        return "…".to_string();
    }
    let kids = match a.get(id) {
        // A bare atom is a monomorphic type name (`Bool`, `String`, …). An unnamed atom is malformed → `?`.
        Struct::Atom(_) => return a.as_name(id).unwrap_or("?").to_string(),
        Struct::List(kids) => kids.clone(),
    };
    let Some(head) = kids.first().and_then(|&h| a.as_name(h)) else {
        // A list whose head is not a name (or an empty list) is not a well-formed type payload.
        return generic(a, &kids, depth, vars);
    };
    let d = depth + 1;
    match head {
        "Int" | "UInt" | "Float" => match kids.get(1).and_then(|&w| width_str(a, w)) {
            Some(w) => format!("{head}{w}"),
            None => generic(a, &kids, depth, vars),
        },
        "->" if kids.len() == 3 => format!(
            "(-> {} {})",
            render(a, kids[1], d, vars),
            render(a, kids[2], d, vars)
        ),
        "Cont" if kids.len() == 3 => format!(
            "(Cont {} {})",
            render(a, kids[1], d, vars),
            render(a, kids[2], d, vars)
        ),
        "Tuple" => {
            let mut s = String::from("(Tuple");
            for &e in &kids[1..] {
                s.push(' ');
                s.push_str(&render(a, e, d, vars));
            }
            s.push(')');
            s
        }
        "Record" => {
            let mut s = String::from("(Record");
            for &field in &kids[1..] {
                // Each field is a `(: name T)` ascription node; render it back to that surface. A
                // malformed field falls through to the generic rendering of the whole record.
                match a.get(field) {
                    Struct::List(f) if f.len() == 3 => {
                        let name = a.as_name(f[1]).unwrap_or("?");
                        s.push_str(&format!(" (: {} {})", name, render(a, f[2], d, vars)));
                    }
                    _ => return generic(a, &kids, depth, vars),
                }
            }
            s.push(')');
            s
        }
        "List" | "Set" if kids.len() == 2 => {
            format!("({head} {})", render(a, kids[1], d, vars))
        }
        "Map" if kids.len() == 3 => format!(
            "(Map {} {})",
            render(a, kids[1], d, vars),
            render(a, kids[2], d, vars)
        ),
        // A sum: the nominal NAME, applied to its type ARGS when generic. Child index 2 (`<decl>`) is an
        // internal arena occurrence id and is HIDDEN. `args = kids[3..]`.
        "Sum" if kids.len() >= 3 => {
            let name = a.as_name(kids[1]).unwrap_or("<sum>");
            if kids.len() == 3 {
                name.to_string()
            } else {
                let mut s = format!("({name}");
                for &arg in &kids[3..] {
                    s.push(' ');
                    s.push_str(&render(a, arg, d, vars));
                }
                s.push(')');
                s
            }
        }
        // A nominal renders as its declared NAME only (its identity is the name; `<decl>`/`(args…)`/`INNER`
        // are hidden, matching `render_name`).
        "Nominal" if kids.len() >= 2 => a.as_name(kids[1]).unwrap_or("<nominal>").to_string(),
        // A type variable: its letter (scheme) or `_` (monomorphic).
        "Var" => match (vars, var_num(a, &kids)) {
            (Some(map), Some(n)) => map.get(&n).cloned().unwrap_or_else(|| "_".to_string()),
            _ => "_".to_string(),
        },
        // `Qty` and any unrecognized head fall back to a total, structure-preserving rendering. NOTE:
        // `Qty`'s byte-parity with `render_name` (which renders the unit via `Unit::render`) is a
        // follow-up increment; the generic form here stays total + honors the var map for any inner type.
        _ => generic(a, &kids, depth, vars),
    }
}

/// A total, structure-preserving fallback for an unrecognized/deferred head or a malformed subtree:
/// `(head child…)`, recursing children through [`render`] so an inner type still renders (and honors the
/// var map). Never panics; an empty list is `?`.
fn generic(
    a: &Arenas,
    kids: &[StructId],
    depth: u32,
    vars: Option<&BTreeMap<String, String>>,
) -> String {
    if kids.is_empty() {
        return "?".to_string();
    }
    let mut s = String::from("(");
    for (i, &c) in kids.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        // The head prints as its bare name; children recurse (a child that is itself a name atom prints
        // bare via `render`'s atom arm).
        s.push_str(&render(a, c, depth + 1, vars));
    }
    s.push(')');
    s
}

/// Collect DISTINCT `(Var N)` numbers in first-encounter order, visiting the SAME children `render`
/// counts toward the scheme — mirrors `rcdzc` `Ty::collect_free_vars` so the letter assignment matches
/// what `render` emits. In particular: `Sum`/`Nominal` visit their type ARGS (never a `Nominal`'s inner),
/// `Qty` visits its inner (never its unit), and the `Int`/`UInt`/`Float` width children carry no vars.
fn collect_vars(a: &Arenas, id: StructId, depth: u32, order: &mut Vec<String>) {
    if depth >= MAX_RENDER_DEPTH {
        return;
    }
    let Struct::List(kids) = a.get(id) else {
        return;
    };
    let kids = kids.clone();
    let head = kids.first().and_then(|&h| a.as_name(h));
    let d = depth + 1;
    match head {
        Some("Var") => {
            if let Some(n) = var_num(a, &kids)
                && !order.contains(&n)
            {
                order.push(n);
            }
        }
        Some("Int") | Some("UInt") | Some("Float") => {}
        // A sum's type ARGS (`kids[3..]`) — skip the hidden `<decl>` (index 2). A nominal's ARGS live in
        // the `(args …)` group at index 3 (skip its `args` head); its inner is NOT visited.
        Some("Sum") => {
            for &c in kids.get(3..).unwrap_or(&[]) {
                collect_vars(a, c, d, order);
            }
        }
        Some("Nominal") => {
            if let Some(&args_group) = kids.get(3)
                && let Struct::List(ag) = a.get(args_group)
            {
                for &c in ag.clone().get(1..).unwrap_or(&[]) {
                    collect_vars(a, c, d, order);
                }
            }
        }
        // A quantity's INNER type only (index 1); the unit carries no type vars.
        Some("Qty") => {
            if let Some(&inner) = kids.get(1) {
                collect_vars(a, inner, d, order);
            }
        }
        // Everything else (`->`, `Cont`, `Tuple`, `List`/`Set`, `Map`, `Record` fields) recurses its
        // non-head children — a `Record` field `(: name T)` recurses into `name` (a no-op atom) and `T`.
        _ => {
            for &c in kids.get(1..).unwrap_or(&[]) {
                collect_vars(a, c, d, order);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Builder, IntValue, Leaf, Radix};

    /// A tiny builder harness. Each constructor binds its children to temporaries first — a single
    /// expression `b.l(vec![b.n(…), …])` would borrow `b` mutably twice.
    struct B(Builder);
    impl B {
        fn new() -> B {
            B(Builder::new())
        }
        fn n(&mut self, s: &str) -> StructId {
            self.0.atom_leaf(Leaf::Name(s.into()))
        }
        fn i(&mut self, v: i64) -> StructId {
            self.0.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(v),
                radix: Radix::Dec,
            })
        }
        fn l(&mut self, kids: Vec<StructId>) -> StructId {
            self.0.list(kids)
        }
        // (Head <width>) — Int/UInt/Float.
        fn width_ty(&mut self, head: &str, w: i64) -> StructId {
            let h = self.n(head);
            let ww = self.i(w);
            self.l(vec![h, ww])
        }
        // (Var N)
        fn var(&mut self, n: i64) -> StructId {
            let h = self.n("Var");
            let nn = self.i(n);
            self.l(vec![h, nn])
        }
        // (: name T)
        fn field(&mut self, name: &str, t: StructId) -> StructId {
            let c = self.n(":");
            let nm = self.n(name);
            self.l(vec![c, nm, t])
        }
        fn name_of(self, root: StructId) -> String {
            let a = self.0.finish(root);
            render_ty_name(&a, a.root)
        }
        fn scheme_of(self, root: StructId) -> String {
            let a = self.0.finish(root);
            render_ty_scheme(&a, a.root)
        }
    }

    #[test]
    fn monomorphic_scalars_render_bare() {
        for name in [
            "Bool", "Unit", "String", "Char", "Symbol", "BigInt", "Rational", "Bytes", "Type",
            "Any",
        ] {
            let mut b = B::new();
            let r = b.n(name);
            assert_eq!(b.name_of(r), name, "{name} renders bare");
        }
    }

    #[test]
    fn int_uint_float_widths() {
        let mut b = B::new();
        let r = b.width_ty("Int", 64);
        assert_eq!(b.name_of(r), "Int64");
        let mut b = B::new();
        let r = b.width_ty("UInt", 32);
        assert_eq!(b.name_of(r), "UInt32");
        let mut b = B::new();
        let r = b.width_ty("Float", 64);
        assert_eq!(b.name_of(r), "Float64");
    }

    #[test]
    fn arrows_curried() {
        // (-> Int64 Int64)
        let mut b = B::new();
        let p = b.width_ty("Int", 64);
        let q = b.width_ty("Int", 64);
        let arr = b.n("->");
        let r = b.l(vec![arr, p, q]);
        assert_eq!(b.name_of(r), "(-> Int64 Int64)");
        // curried (-> Int64 (-> Int64 Bool))
        let mut b = B::new();
        let p1 = b.width_ty("Int", 64);
        let p2 = b.width_ty("Int", 64);
        let bl = b.n("Bool");
        let a1 = b.n("->");
        let inner = b.l(vec![a1, p2, bl]);
        let a2 = b.n("->");
        let r = b.l(vec![a2, p1, inner]);
        assert_eq!(b.name_of(r), "(-> Int64 (-> Int64 Bool))");
    }

    #[test]
    fn tuple_record_list_set_map() {
        // (Tuple Int64 Bool)
        let mut b = B::new();
        let e1 = b.width_ty("Int", 64);
        let e2 = b.n("Bool");
        let h = b.n("Tuple");
        let r = b.l(vec![h, e1, e2]);
        assert_eq!(b.name_of(r), "(Tuple Int64 Bool)");
        // (Record (: a Int64) (: b Bool)) — pre-sorted fields
        let mut b = B::new();
        let ta = b.width_ty("Int", 64);
        let fa = b.field("a", ta);
        let tb = b.n("Bool");
        let fb = b.field("b", tb);
        let h = b.n("Record");
        let r = b.l(vec![h, fa, fb]);
        assert_eq!(b.name_of(r), "(Record (: a Int64) (: b Bool))");
        // (List Int64)
        let mut b = B::new();
        let e = b.width_ty("Int", 64);
        let h = b.n("List");
        let r = b.l(vec![h, e]);
        assert_eq!(b.name_of(r), "(List Int64)");
        // (Set Int64)
        let mut b = B::new();
        let e = b.width_ty("Int", 64);
        let h = b.n("Set");
        let r = b.l(vec![h, e]);
        assert_eq!(b.name_of(r), "(Set Int64)");
        // (Map Int64 Bool)
        let mut b = B::new();
        let k = b.width_ty("Int", 64);
        let v = b.n("Bool");
        let h = b.n("Map");
        let r = b.l(vec![h, k, v]);
        assert_eq!(b.name_of(r), "(Map Int64 Bool)");
    }

    #[test]
    fn sum_mono_and_generic_hides_decl() {
        // monomorphic: (Sum Sign <3>) -> "Sign"
        let mut b = B::new();
        let h = b.n("Sum");
        let nm = b.n("Sign");
        let d = b.i(3);
        let r = b.l(vec![h, nm, d]);
        assert_eq!(b.name_of(r), "Sign");
        // generic mono render: (Sum Option <7> (Var 0)) -> "(Option _)"
        let mut b = B::new();
        let h = b.n("Sum");
        let nm = b.n("Option");
        let d = b.i(7);
        let v = b.var(0);
        let r = b.l(vec![h, nm, d, v]);
        assert_eq!(b.name_of(r), "(Option _)");
        // generic scheme render: -> "(Option a)"
        let mut b = B::new();
        let h = b.n("Sum");
        let nm = b.n("Option");
        let d = b.i(7);
        let v = b.var(0);
        let r = b.l(vec![h, nm, d, v]);
        assert_eq!(b.scheme_of(r), "(Option a)");
        // two-arg: (Sum Result <9> (Var 0) (Var 1)) -> "(Result a b)"
        let mut b = B::new();
        let h = b.n("Sum");
        let nm = b.n("Result");
        let d = b.i(9);
        let v0 = b.var(0);
        let v1 = b.var(1);
        let r = b.l(vec![h, nm, d, v0, v1]);
        assert_eq!(b.scheme_of(r), "(Result a b)");
    }

    #[test]
    fn nominal_renders_name_only() {
        // (Nominal UserId <5> (args) Int64) -> "UserId"
        let mut b = B::new();
        let ah = b.n("args");
        let args = b.l(vec![ah]);
        let inner = b.width_ty("Int", 64);
        let h = b.n("Nominal");
        let nm = b.n("UserId");
        let d = b.i(5);
        let r = b.l(vec![h, nm, d, args, inner]);
        assert_eq!(b.name_of(r), "UserId");
    }

    #[test]
    fn var_lettering_and_tie_structure() {
        // bare (Var 0): mono "_", scheme "a"
        let mut b = B::new();
        let r = b.var(0);
        assert_eq!(b.name_of(r), "_");
        let mut b = B::new();
        let r = b.var(0);
        assert_eq!(b.scheme_of(r), "a");
        // (-> (Var 0) (-> (Var 1) (Var 0))) -> "(-> a (-> b a))" (same N -> same letter)
        let mut b = B::new();
        let a0 = b.var(0);
        let b1 = b.var(1);
        let a0b = b.var(0);
        let ai = b.n("->");
        let inner = b.l(vec![ai, b1, a0b]);
        let ao = b.n("->");
        let r = b.l(vec![ao, a0, inner]);
        assert_eq!(b.scheme_of(r), "(-> a (-> b a))");
    }

    #[test]
    fn nominal_arg_var_consumes_a_letter_but_is_not_rendered() {
        // Parity w/ render_scheme + collect_free_vars: a var ONLY in a Nominal's args is COLLECTED
        // (consumes a letter) but the Nominal renders as just its name, so a sibling var shifts.
        // (-> (Nominal N <5> (args (Var 0)) Unit) (Var 1)) -> "(-> N b)".
        let mut b = B::new();
        let v0 = b.var(0);
        let ah = b.n("args");
        let args = b.l(vec![ah, v0]);
        let unit = b.n("Unit");
        let nh = b.n("Nominal");
        let nnm = b.n("N");
        let nd = b.i(5);
        let nom = b.l(vec![nh, nnm, nd, args, unit]);
        let v1 = b.var(1);
        let ao = b.n("->");
        let r = b.l(vec![ao, nom, v1]);
        assert_eq!(b.scheme_of(r), "(-> N b)");
    }

    #[test]
    fn totality_unknown_head_and_malformed() {
        // Unknown head -> generic structure-preserving fallback, never panics.
        let mut b = B::new();
        let h = b.n("Weird");
        let bl = b.n("Bool");
        let r = b.l(vec![h, bl]);
        assert_eq!(b.name_of(r), "(Weird Bool)");
        // A list head that is itself a list (not a name) -> generic.
        let mut b = B::new();
        let inner = b.width_ty("Int", 64);
        let r = b.l(vec![inner]);
        assert_eq!(b.name_of(r), "(Int64)");
    }
}

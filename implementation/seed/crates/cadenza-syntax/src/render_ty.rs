//! Render a decoded Ty-payload arena subtree to its type NAME — the canonical, host-agnostic type-name
//! surface for the sidecar wire (`cdz exports "name : type"`, `cdz type`, editor hover).
//!
//! The sidecar responses carry the FULL structured `Ty` payload on the wire (the shape `rcdzc`'s
//! `eval::encode_ty` builds), not a pre-rendered string (operator: "binary AST is the data exchange
//! format"). A host command must render that decoded payload to a clean type NAME. `cdz` has NO `rcdzc`
//! dependency, so it cannot call `rcdzc::ty::Ty::render_name`; this module is the canonical renderer over
//! the decoded `cadenza_ast` subtree, living in the printer's home crate.
//!
//! ARCHITECTURE (operator seq-41, option C — "not a formatter"): a Ty name is NOT produced by a bespoke
//! per-arm String formatter, and NOT by raw-structural sexpr-print of the wire payload (which would leak
//! the internal decl-occurrence ids). Instead the rendering is a TWO-STEP: [`transform`] TRANSLATES the
//! decoded wire payload into a SURFACE type-syntax AST (fresh `cadenza_ast` nodes — the shape a reader
//! would build for `Int64` / `(-> Int64 Int64)` / `(Option a)`), and then the CANONICAL s-expression
//! printer ([`crate::sexpr::print_from`]) renders those nodes to text. One shared printer, no per-crate
//! format code, no decl-id leak. `print_from` (single-line) — NOT the pretty printer — so a wide type
//! stays one line, byte-identical to the historical output the sidecar consumers pin.
//!
//! PARITY: [`render_ty`] renders byte-IDENTICALLY to `rcdzc` `Ty::render_name` (ty.rs) for a monomorphic
//! type, and [`render_ty_scheme`] to `Scheme::render_scheme` (its `render_named_vars` + first-encounter
//! `a`,`b`,`c`… var lettering) for a polymorphic one. Those functions are the source of truth for the
//! exact spelling (`Int64` not `(Int 64)`, `(-> A B)` curried, `Option a`, …). ONE deliberate lead: a
//! GENERIC `Nominal` renders its type args here (`(Box a)`, the host query/hover/exports surface —
//! generic-nominal-args canonical, #7380), while `render_name`/`render_scheme` (the rcdzc-INTERNAL
//! diagnostic-message surface) still collapse it to the bare `Box`. That diagnostic-surface follow is
//! routed to its owner; the two surfaces have different audiences (author-facing type query vs compile
//! error text), so they need not flip in lockstep.
//!
//! The payload grammar (head-name keyed; `encode_ty`) and its surface translation:
//! - bare `Name` leaf: `Bool`/`Unit`/`String`/`Char`/`Symbol`/`BigInt`/`Rational`/`Bytes`/`Type`/`Any`
//!   → the same `Name` atom.
//! - `(Int W)` / `(UInt W)` / `(Float W)` — `W` a width `Int` leaf → a single `Name` atom `IntW`/`UIntW`/
//!   `FloatW` (the width is FOLDED into the name, matching `Ty::render_name`'s `Int64` spelling).
//! - `(-> P R)` → `(-> P' R')` (curried: nested `->` for multi-arg), `(Cont R A)` → `(Cont R' A')`.
//! - `(Tuple E…)` → `(Tuple E'…)`, `(Record (: name T)…)` (fields pre-sorted) → `(Record (: name T')…)`,
//!   `(List E)` → `(List E')`, `(Set E)` → `(Set E')`, `(Map K V)` → `(Map K' V')`.
//! - `(Sum NAME <decl> arg…)` — the `<decl>` child (index 2) is an INTERNAL arena occurrence id and is
//!   HIDDEN → `NAME` (monomorphic) or `(NAME arg'…)` (generic).
//! - `(Nominal NAME <decl> (args…) INNER)` — identity is the NAME + instantiation; the `<decl>` and `INNER`
//!   are hidden → `NAME` (monomorphic, empty `(args)`) or `(NAME arg'…)` (generic), mirroring `Sum` and the
//!   generic-nominal-args canonical (#7380).
//! - `(Var N)` — a type-variable number → `_` in [`render_ty`], a stable letter in [`render_ty_scheme`].
//! - `(Qty INNER (unit (base NAME EXP)… [(scale N D)]))` → `(Qty <inner> <unit>)` with the unit in
//!   `Unit::render`'s canonical written form (`Unit.one` / `(Unit.base #"n")` / `(Unit.^ … k)` / left-nested
//!   `(Unit.* …)`); the `(scale …)` item is round-trip fidelity only, not part of the name. See
//!   [`unit_surface`].
//!
//! TOTAL: an unknown head or a malformed/short subtree translates to a defined fallback surface node (its
//! raw shape recursed, else `?`), NEVER a panic — these feed editor hover on possibly-incomplete programs.
//! A DEPTH GUARD (mirroring `render_name`'s `MAX_RENDER_DEPTH`) truncates an explosively-deep type with `…`.

use crate::ast::{Arenas, Builder, Leaf, Radix, Struct, StructId};
use std::collections::BTreeMap;

/// The recursion cap — mirrors `rcdzc` `Ty::render_name`'s `MAX_RENDER_DEPTH` (24): a diagnostic never
/// needs deeper, and it keeps the renderer total on a pathologically deep decoded arena.
const MAX_RENDER_DEPTH: u32 = 24;

/// Render a monomorphic Ty payload rooted at `root` to its type name. A `(Var N)` renders as `_` (parity
/// with `Ty::render_name`, which collapses every unsolved var to the `_` placeholder). The rendering is
/// the transform-then-print path (see the module docs): translate the wire payload to a surface type
/// AST, then print it with the canonical single-line s-expr printer.
pub fn render_ty(a: &Arenas, root: StructId) -> String {
    let mut b = Builder::new();
    let node = transform(a, root, 0, None, &mut b);
    let surface = b.finish(node);
    crate::sexpr::print_from(&surface, surface.root)
}

/// Render a Ty payload rooted at `root` as a SCHEME: each DISTINCT `(Var N)` gets a stable letter
/// (`a`, `b`, `c`, …, then `a1`, `b1`, … past 26) in FIRST-ENCOUNTER order, so a reader sees which vars
/// are the same quantified variable and which differ (parity with `Scheme::render_scheme`). Non-`Var`
/// nodes render exactly as [`render_ty`]. Same transform-then-print path, with the var letter map threaded.
pub fn render_ty_scheme(a: &Arenas, root: StructId) -> String {
    // First-encounter order of DISTINCT var numbers, walking the same structure `transform` visits — this
    // mirrors `Ty::collect_free_vars` (which visits `Sum`/`Nominal` ARGS but not a `Nominal`'s inner, and
    // a `Qty`'s inner but not its unit), so the letters line up with what `transform` emits.
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
    let mut b = Builder::new();
    let node = transform(a, root, 0, Some(&names), &mut b);
    let surface = b.finish(node);
    crate::sexpr::print_from(&surface, surface.root)
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

/// Build a fresh surface `Name` atom in `b`.
fn name(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Name(s.into()))
}

/// Translate the wire payload node `id` (in the DECODED arena `a`) into a fresh SURFACE type-syntax node
/// in the builder `b`, returning its id. `vars = Some(map)` translates a `(Var N)` to its letter (scheme
/// mode); `None` translates it to `_` (monomorphic mode). Never panics; every unrecognized/malformed shape
/// falls to the total [`generic`] translation.
fn transform(
    a: &Arenas,
    id: StructId,
    depth: u32,
    vars: Option<&BTreeMap<String, String>>,
    b: &mut Builder,
) -> StructId {
    if depth >= MAX_RENDER_DEPTH {
        return name(b, "…");
    }
    let kids = match a.get(id) {
        // A bare atom: a type name (`Bool`, `String`, …) — or, in a generic/deferred position, any other
        // leaf (a width `Int`, a unit exponent). Copy the leaf VERBATIM so a name prints bare and a value
        // prints its literal; this stays total (no unnamed-atom `?` placeholder is needed — every leaf has
        // a print form).
        Struct::Atom(l) => return b.atom_leaf(a.leaf(*l).clone()),
        Struct::List(kids) => kids.clone(),
    };
    let Some(head) = kids.first().and_then(|&h| a.as_name(h)) else {
        // A list whose head is not a name (or an empty list) is not a well-formed type payload.
        return generic(a, &kids, depth, vars, b);
    };
    let d = depth + 1;
    match head {
        // The width is FOLDED into a single name atom (`Int64`, `UInt32`, `Float64`) — the `Ty::render_name`
        // spelling. A missing/non-integer width falls to the generic structure.
        "Int" | "UInt" | "Float" => match kids.get(1).and_then(|&w| width_str(a, w)) {
            Some(w) => name(b, &format!("{head}{w}")),
            None => generic(a, &kids, depth, vars, b),
        },
        "->" if kids.len() == 3 => {
            let h = name(b, "->");
            let p = transform(a, kids[1], d, vars, b);
            let r = transform(a, kids[2], d, vars, b);
            b.list(vec![h, p, r])
        }
        "Cont" if kids.len() == 3 => {
            let h = name(b, "Cont");
            let resume = transform(a, kids[1], d, vars, b);
            let answer = transform(a, kids[2], d, vars, b);
            b.list(vec![h, resume, answer])
        }
        "Tuple" => {
            let mut items = vec![name(b, "Tuple")];
            for &e in &kids[1..] {
                items.push(transform(a, e, d, vars, b));
            }
            b.list(items)
        }
        "Record" => {
            let mut items = vec![name(b, "Record")];
            for &field in &kids[1..] {
                // Each field is a `(: name T)` ascription node; translate it back to that surface node. A
                // malformed field falls through to the generic translation of the whole record.
                match a.get(field) {
                    Struct::List(f) if f.len() == 3 => {
                        let fname = a.as_name(f[1]).unwrap_or("?").to_string();
                        let colon = name(b, ":");
                        let fn_atom = name(b, &fname);
                        let fty = transform(a, f[2], d, vars, b);
                        items.push(b.list(vec![colon, fn_atom, fty]));
                    }
                    _ => return generic(a, &kids, depth, vars, b),
                }
            }
            b.list(items)
        }
        "List" | "Set" if kids.len() == 2 => {
            let h = name(b, head);
            let e = transform(a, kids[1], d, vars, b);
            b.list(vec![h, e])
        }
        "Map" if kids.len() == 3 => {
            let h = name(b, "Map");
            let k = transform(a, kids[1], d, vars, b);
            let v = transform(a, kids[2], d, vars, b);
            b.list(vec![h, k, v])
        }
        // A sum: the nominal NAME, applied to its type ARGS when generic. Child index 2 (`<decl>`) is an
        // internal arena occurrence id and is HIDDEN. `args = kids[3..]`.
        "Sum" if kids.len() >= 3 => {
            let sum_name = a.as_name(kids[1]).unwrap_or("<sum>").to_string();
            if kids.len() == 3 {
                name(b, &sum_name)
            } else {
                let mut items = vec![name(b, &sum_name)];
                for &arg in &kids[3..] {
                    items.push(transform(a, arg, d, vars, b));
                }
                b.list(items)
            }
        }
        // A nominal renders as its declared NAME, applied to its type ARGS when generic — mirroring the
        // `Sum` arm and the generic-nominal-args canonical (#7380 closed it for the `type_ast` emitter;
        // this is the host QUERY/hover/exports surface, all of which render through here). The `<decl>`
        // (index 2) and `INNER` (index 4) stay hidden — identity is the NAME + instantiation. The args
        // live in the `(args …)` group at index 3 (`encode_ty` writes them there; `collect_vars` already
        // reserves their scheme letters, so showing them is strictly more consistent than the old bare
        // render, which dropped a visible `(Box a)` down to `Box`). A monomorphic nominal has an EMPTY
        // `(args)` group → the bare NAME (unchanged). A missing/malformed args group falls back to bare.
        "Nominal" if kids.len() >= 2 => {
            let n = a.as_name(kids[1]).unwrap_or("<nominal>").to_string();
            let args: Vec<StructId> = match kids.get(3).map(|&g| a.get(g)) {
                Some(Struct::List(ag))
                    if ag.first().and_then(|&h| a.as_name(h)) == Some("args") =>
                {
                    ag[1..].to_vec()
                }
                _ => Vec::new(),
            };
            if args.is_empty() {
                name(b, &n)
            } else {
                let mut items = vec![name(b, &n)];
                for arg in args {
                    items.push(transform(a, arg, d, vars, b));
                }
                b.list(items)
            }
        }
        // A type variable: its letter (scheme) or `_` (monomorphic).
        "Var" => match (vars, var_num(a, &kids)) {
            (Some(map), Some(n)) => {
                let letter = map.get(&n).cloned().unwrap_or_else(|| "_".to_string());
                name(b, &letter)
            }
            _ => name(b, "_"),
        },
        // A quantity: `(Qty <inner'> <unit'>)` — byte-parity with `Ty::render_name`'s `Ty::Qty` arm (which
        // renders `(Qty <inner> <unit>)` with the unit in `Unit::render`'s canonical written form). The
        // inner numeric type translates ordinarily; the `(unit …)` payload node translates via
        // [`unit_surface`]. A malformed unit node falls through to the generic translation.
        "Qty" if kids.len() == 3 => match unit_surface(a, kids[2], b) {
            Some(unit_node) => {
                let h = name(b, "Qty");
                let inner = transform(a, kids[1], d, vars, b);
                b.list(vec![h, inner, unit_node])
            }
            None => generic(a, &kids, depth, vars, b),
        },
        // Any unrecognized head falls back to a total, structure-preserving surface node (honoring the var
        // map for any inner type).
        _ => generic(a, &kids, depth, vars, b),
    }
}

/// A total, structure-preserving fallback for an unrecognized/deferred head or a malformed subtree:
/// translate to `(child'…)`, recursing children through [`transform`] so an inner type still translates
/// (and honors the var map). Never panics; an empty list becomes the `?` name.
fn generic(
    a: &Arenas,
    kids: &[StructId],
    depth: u32,
    vars: Option<&BTreeMap<String, String>>,
    b: &mut Builder,
) -> StructId {
    if kids.is_empty() {
        return name(b, "?");
    }
    let mut items = Vec::with_capacity(kids.len());
    for &c in kids {
        // The head recurses too: a name-atom head translates to its bare name node (prints bare); a child
        // that is itself a compound recurses.
        items.push(transform(a, c, depth + 1, vars, b));
    }
    b.list(items)
}

/// Translate a `(unit (base NAME EXP)… [(scale N D)])` payload node into the canonical written unit
/// SURFACE node, byte-parity with `rcdzc`'s `Unit::render`: the dimensionless (empty) unit is the atom
/// `Unit.one`; a base to the first power is `(Unit.base #"name")`; a base to power `k` is
/// `(Unit.^ (Unit.base #"name") k)`; a product of several is a left-nested `(Unit.* …)`. The base factors
/// are already in the sorted order `encode_ty` wrote (the `BTreeMap` order `Unit::render` also walks), so
/// the factor order matches. The `(scale N D)` item (present only for a non-reference-scale unit) is NOT
/// part of the type-NAME surface (`Unit::render` reads only the base→exponent map — a scaled base like
/// `kilometer` carries its prefix in the NAME, not a `Unit.prefix`/quotient node, which are the VALUE
/// surface's `render_value_form`, not this one) and is skipped. The base name is a `Sym` leaf (prints
/// `#"name"`); `Unit.base`/`Unit.^`/`Unit.*` are bare `Name` atoms (the printer leaves the dotted word
/// verbatim). Returns `None` if the node is not a well-formed `(unit …)` list so the caller falls back to
/// the generic translation.
fn unit_surface(a: &Arenas, id: StructId, b: &mut Builder) -> Option<StructId> {
    let Struct::List(kids) = a.get(id) else {
        return None;
    };
    let kids = kids.clone();
    if kids.first().and_then(|&h| a.as_name(h)) != Some("unit") {
        return None;
    }
    let mut factors: Vec<StructId> = Vec::new();
    for &child in &kids[1..] {
        let Struct::List(item) = a.get(child) else {
            return None;
        };
        let item = item.clone();
        match item.first().and_then(|&h| a.as_name(h)) {
            // `(base NAME EXP)` — one dimension factor.
            Some("base") if item.len() == 3 => {
                let bname = a.as_name(item[1])?.to_string();
                let exp = a.as_int(item[2])?.clone();
                // (Unit.base #"name")
                let ub = name(b, "Unit.base");
                let sym = b.atom_leaf(Leaf::Sym(bname.into()));
                let base_node = b.list(vec![ub, sym]);
                let factor = if exp.to_decimal_string() == "1" {
                    base_node
                } else {
                    // (Unit.^ (Unit.base #"name") k)
                    let uexp = name(b, "Unit.^");
                    let k = b.atom_leaf(Leaf::Int {
                        value: exp,
                        radix: Radix::Dec,
                    });
                    b.list(vec![uexp, base_node, k])
                };
                factors.push(factor);
            }
            // `(scale N D)` — carried for round-trip fidelity, not part of the type-name surface.
            Some("scale") => {}
            _ => return None,
        }
    }
    // The dimensionless unit is `Unit.one`; otherwise a left-nested `(Unit.* …)` product of the factors.
    let Some((&first, rest)) = factors.split_first() else {
        return Some(name(b, "Unit.one"));
    };
    let mut acc = first;
    for &f in rest {
        let umul = name(b, "Unit.*");
        acc = b.list(vec![umul, acc, f]);
    }
    Some(acc)
}

/// Collect DISTINCT `(Var N)` numbers in first-encounter order, visiting the SAME children `transform`
/// counts toward the scheme — mirrors `rcdzc` `Ty::collect_free_vars` so the letter assignment matches
/// what `transform` emits. In particular: `Sum`/`Nominal` visit their type ARGS (never a `Nominal`'s
/// inner), `Qty` visits its inner (never its unit), and the `Int`/`UInt`/`Float` width children carry no
/// vars.
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
            render_ty(&a, a.root)
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
    fn nominal_mono_bare_generic_shows_args() {
        // MONOMORPHIC: (Nominal UserId <5> (args) Int64) -> "UserId" (empty args group -> bare name).
        let mut b = B::new();
        let ah = b.n("args");
        let args = b.l(vec![ah]);
        let inner = b.width_ty("Int", 64);
        let h = b.n("Nominal");
        let nm = b.n("UserId");
        let d = b.i(5);
        let r = b.l(vec![h, nm, d, args, inner]);
        assert_eq!(b.name_of(r), "UserId");
        // GENERIC (mono render): (Nominal Box <7> (args (Var 0)) Int64) -> "(Box _)" — the type arg is
        // now rendered (generic-nominal-args canonical, #7380), the `<decl>`/`INNER` stay hidden.
        let mut b = B::new();
        let v = b.var(0);
        let ah = b.n("args");
        let args = b.l(vec![ah, v]);
        let inner = b.width_ty("Int", 64);
        let h = b.n("Nominal");
        let nm = b.n("Box");
        let d = b.i(7);
        let r = b.l(vec![h, nm, d, args, inner]);
        assert_eq!(b.name_of(r), "(Box _)");
        // GENERIC (scheme render): same payload -> "(Box a)".
        let mut b = B::new();
        let v = b.var(0);
        let ah = b.n("args");
        let args = b.l(vec![ah, v]);
        let inner = b.width_ty("Int", 64);
        let h = b.n("Nominal");
        let nm = b.n("Box");
        let d = b.i(7);
        let r = b.l(vec![h, nm, d, args, inner]);
        assert_eq!(b.scheme_of(r), "(Box a)");
        // A LIST of a generic nominal — breaker's rg1 shape: (List (Nominal Box <7> (args (Var 0)) Int64))
        // scheme -> "(List (Box a))" (was the dropped-arg bug "(List Box)").
        let mut b = B::new();
        let v = b.var(0);
        let ah = b.n("args");
        let args = b.l(vec![ah, v]);
        let inner = b.width_ty("Int", 64);
        let nh = b.n("Nominal");
        let nm = b.n("Box");
        let d = b.i(7);
        let nom = b.l(vec![nh, nm, d, args, inner]);
        let lh = b.n("List");
        let r = b.l(vec![lh, nom]);
        assert_eq!(b.scheme_of(r), "(List (Box a))");
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
    fn nominal_arg_var_is_collected_and_rendered_in_order() {
        // A var in a Nominal's args is COLLECTED by collect_vars (first-encounter order) AND now RENDERED
        // in the `(NAME arg…)` form, so the letters line up with position: the Nominal's arg is `a`, the
        // sibling is `b`. (-> (Nominal N <5> (args (Var 0)) Unit) (Var 1)) -> "(-> (N a) b)".
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
        assert_eq!(b.scheme_of(r), "(-> (N a) b)");
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

    // (base NAME EXP)
    fn base(b: &mut B, name: &str, exp: i64) -> StructId {
        let h = b.n("base");
        let nm = b.n(name);
        let e = b.i(exp);
        b.l(vec![h, nm, e])
    }

    #[test]
    fn qty_unit_render_parity() {
        // increment-2: Qty renders `(Qty <inner> <unit>)` with the unit in Unit::render's written form.
        // (Qty Float64 (unit (base meter 1))) -> "(Qty Float64 (Unit.base #\"meter\"))"
        let mut b = B::new();
        let inner = b.width_ty("Float", 64);
        let m = base(&mut b, "meter", 1);
        let uh = b.n("unit");
        let unit = b.l(vec![uh, m]);
        let qh = b.n("Qty");
        let r = b.l(vec![qh, inner, unit]);
        assert_eq!(b.name_of(r), "(Qty Float64 (Unit.base #\"meter\"))");
        // exponent > 1: (base meter 2) -> "(Unit.^ (Unit.base #\"meter\") 2)"
        let mut b = B::new();
        let inner = b.width_ty("Float", 64);
        let m = base(&mut b, "meter", 2);
        let uh = b.n("unit");
        let unit = b.l(vec![uh, m]);
        let qh = b.n("Qty");
        let r = b.l(vec![qh, inner, unit]);
        assert_eq!(
            b.name_of(r),
            "(Qty Float64 (Unit.^ (Unit.base #\"meter\") 2))"
        );
        // dimensionless: (unit) -> "Unit.one"
        let mut b = B::new();
        let inner = b.width_ty("Float", 64);
        let uh = b.n("unit");
        let unit = b.l(vec![uh]);
        let qh = b.n("Qty");
        let r = b.l(vec![qh, inner, unit]);
        assert_eq!(b.name_of(r), "(Qty Float64 Unit.one)");
        // product (left-nested): meter * second (both exp 1)
        let mut b = B::new();
        let inner = b.width_ty("Float", 64);
        let m = base(&mut b, "meter", 1);
        let s = base(&mut b, "second", 1);
        let uh = b.n("unit");
        let unit = b.l(vec![uh, m, s]);
        let qh = b.n("Qty");
        let r = b.l(vec![qh, inner, unit]);
        assert_eq!(
            b.name_of(r),
            "(Qty Float64 (Unit.* (Unit.base #\"meter\") (Unit.base #\"second\")))"
        );
        // velocity: meter^1 * second^-1 — negative exponent uses (Unit.^ … -1) (TYPE surface, not the
        // value surface's quotient).
        let mut b = B::new();
        let inner = b.width_ty("Float", 64);
        let m = base(&mut b, "meter", 1);
        let s = base(&mut b, "second", -1);
        let uh = b.n("unit");
        let unit = b.l(vec![uh, m, s]);
        let qh = b.n("Qty");
        let r = b.l(vec![qh, inner, unit]);
        assert_eq!(
            b.name_of(r),
            "(Qty Float64 (Unit.* (Unit.base #\"meter\") (Unit.^ (Unit.base #\"second\") -1)))"
        );
    }

    #[test]
    fn qty_scale_item_skipped_and_inner_var_honored() {
        // A non-reference-scale unit encodes a trailing (scale N D); it is NOT part of the type name.
        // (Qty Int64 (unit (base kilometer 1) (scale 1000 1))) -> "(Qty Int64 (Unit.base #\"kilometer\"))"
        let mut b = B::new();
        let inner = b.width_ty("Int", 64);
        let km = base(&mut b, "kilometer", 1);
        let sh = b.n("scale");
        let sn = b.i(1000);
        let sd = b.i(1);
        let scale = b.l(vec![sh, sn, sd]);
        let uh = b.n("unit");
        let unit = b.l(vec![uh, km, scale]);
        let qh = b.n("Qty");
        let r = b.l(vec![qh, inner, unit]);
        assert_eq!(b.name_of(r), "(Qty Int64 (Unit.base #\"kilometer\"))");
        // inner var: (Qty (Var 0) (unit (base meter 1))) — mono "_", scheme "a" (unit carries no vars).
        let mut b = B::new();
        let v = b.var(0);
        let m = base(&mut b, "meter", 1);
        let uh = b.n("unit");
        let unit = b.l(vec![uh, m]);
        let qh = b.n("Qty");
        let r = b.l(vec![qh, v, unit]);
        assert_eq!(b.name_of(r), "(Qty _ (Unit.base #\"meter\"))");
        let mut b = B::new();
        let v = b.var(0);
        let m = base(&mut b, "meter", 1);
        let uh = b.n("unit");
        let unit = b.l(vec![uh, m]);
        let qh = b.n("Qty");
        let r = b.l(vec![qh, v, unit]);
        assert_eq!(b.scheme_of(r), "(Qty a (Unit.base #\"meter\"))");
    }

    #[test]
    fn qty_malformed_unit_falls_to_generic() {
        // A Qty whose unit node is not a well-formed `(unit …)` renders via the generic translation
        // (total, never panics).
        let mut b = B::new();
        let inner = b.width_ty("Float", 64);
        let bogus = b.n("Bogus");
        let qh = b.n("Qty");
        let r = b.l(vec![qh, inner, bogus]);
        assert_eq!(b.name_of(r), "(Qty Float64 Bogus)");
    }

    #[test]
    fn empty_and_malformed_stay_total() {
        // An empty list payload -> the `?` fallback (total, never a panic or a bare `()`).
        let mut b = B::new();
        let r = b.l(vec![]);
        assert_eq!(b.name_of(r), "?");
    }

    #[test]
    fn nested_compounds_and_scheme_lettering() {
        // (List (Sum Option <7> (Var 0))) scheme -> "(List (Option a))" — recursion + var-lettering
        // compose through nesting. (A fresh builder per render: `name_of`/`scheme_of` consume `self`.)
        let mut b = B::new();
        let v = b.var(0);
        let sh = b.n("Sum");
        let onm = b.n("Option");
        let od = b.i(7);
        let opt = b.l(vec![sh, onm, od, v]);
        let lh = b.n("List");
        let r = b.l(vec![lh, opt]);
        assert_eq!(b.scheme_of(r), "(List (Option a))");
        let mut b = B::new();
        let v = b.var(0);
        let sh = b.n("Sum");
        let onm = b.n("Option");
        let od = b.i(7);
        let opt = b.l(vec![sh, onm, od, v]);
        let lh = b.n("List");
        let r = b.l(vec![lh, opt]);
        assert_eq!(b.name_of(r), "(List (Option _))");
        // (Map (Tuple Int64 Bool) (Set Char)) — a compound key + compound value, no vars.
        let mut b = B::new();
        let ti = b.width_ty("Int", 64);
        let tb = b.n("Bool");
        let th = b.n("Tuple");
        let tup = b.l(vec![th, ti, tb]);
        let ch = b.n("Char");
        let seth = b.n("Set");
        let set = b.l(vec![seth, ch]);
        let mh = b.n("Map");
        let r = b.l(vec![mh, tup, set]);
        assert_eq!(b.name_of(r), "(Map (Tuple Int64 Bool) (Set Char))");
        // A record whose field type is itself compound: (Record (: xs (List (Var 0)))) scheme.
        let mut b = B::new();
        let v = b.var(0);
        let lh = b.n("List");
        let list = b.l(vec![lh, v]);
        let field = b.field("xs", list);
        let rh = b.n("Record");
        let r = b.l(vec![rh, field]);
        assert_eq!(b.scheme_of(r), "(Record (: xs (List a)))");
    }

    #[test]
    fn depth_guard_truncates_deep_type_without_panic() {
        // A type nested deeper than MAX_RENDER_DEPTH (24) must TRUNCATE with `…` rather than panic or
        // recurse unbounded — the totality guard that protects editor hover on a pathological arena.
        // Build (List (List (… 40 deep … Int64 …))).
        let mut b = B::new();
        let mut node = b.width_ty("Int", 64);
        for _ in 0..40 {
            let lh = b.n("List");
            node = b.l(vec![lh, node]);
        }
        let out = b.name_of(node);
        assert!(
            out.contains('…'),
            "a >MAX_RENDER_DEPTH type must truncate with `…`, got: {out}"
        );
        // And the guard must fire at the SAME depth in scheme mode (via collect_vars + transform).
        let mut b = B::new();
        let mut node = b.var(0);
        for _ in 0..40 {
            let lh = b.n("List");
            node = b.l(vec![lh, node]);
        }
        let out = b.scheme_of(node);
        assert!(
            out.contains('…'),
            "scheme mode must also truncate, got: {out}"
        );
    }
}

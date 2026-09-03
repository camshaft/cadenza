//! rcdzc-side VALUE-DOC emit — the operator seq-210 parser-elimination.
//!
//! Generates, per export, a Ty-GUIDED `pub fn __cdz_doc_<export>() -> String` body that builds the
//! self-describing `(: <value> <type>)` codec doc via `cadenza_ast::Builder` + `codec::encode` (the SAME
//! wire cdz-run's `value_codec` emits, decoded by the harness's canonical `render_binary` — render-ty #7424)
//! and returns the `CDZDOC:<hex>` marker string. The gate driver calls this (flag-gated `CDZ_VALUE_DOC`)
//! instead of cdz-rust-render's type-note-driven `cdz_render_at` string walk: the value walk moves HERE and
//! consults the `Ty` DIRECTLY (no sexpr note re-parse), which is what lets us delete `parse_head_type` /
//! `cdz_render_at` / `rust_call_arg`.
//!
//! A compound VALUE head is a `Leaf::Ctor(CompoundCtor::…)` (via `Builder::compound`), so it renders the
//! canonical `#tuple(…)` / `#record(…)` / `#list(…)` / `#set(…)` / `#map(…)` — MATCHING cdz-run's `render_val`,
//! the corpus expected outputs, and the grader's `canonical_output_value` (all of which use the `#`-ctor form;
//! a bare `Name "tuple"` head would render the DIVERGENT `(tuple …)` and red every compound case on the flip).
//! A record/map ENTRY is a `Builder::field_pair` `(= key value)`. The TYPE head stays a `Name` (`(Tuple …)` —
//! types have no `#` form). A SUM value head is the bare variant `Name` (`(Some 5)`), matching cdz-run.
//!
//! Node shapes (verified via `cdz convert`):
//!   `(: 42 Int64)`             → List[ Name ":", Int 42, Name "Int64" ]
//!   `(: #tuple(1 2) …)`        → the value is List[ Ctor(Tuple), <e0>, <e1>… ]; the type List[ Name "Tuple", <T0>… ]
//!   `(: true Bool)`            → Bool leaf, type Name "Bool"
//!
//! WIP (built incrementally, per concierge): covers Int / Bool / Float / String / Symbol / Bytes / Char /
//! Tuple / Record / List / Set / Map (incl. a DIRECT-float key/element via the `__CdzF` `.get()` unwrap), a
//! QTY (any unit — base / power / product / `Unit./` quotient, `(Qty.of <mag> <unit>)`), and a bare-head SUM
//! (Option / Result / a user sum) — nullary, single-payload, multi-field (flattened by declared arity), AND
//! QUALIFIED-head (`<Type>.<Variant>` dotted head when a variant name collides with a prelude binding). A
//! COMPOUND-float (tuple-with-float) key/element + a RECURSIVE sum (needs a runtime helper fn to terminate)
//! are follow-up increments (each a `doc_value_node` + `doc_type_node` arm). An uncovered shape
//! DECLINES (never a miscompile) — the driver keeps `cdz_render_at` for it until covered, so partial coverage
//! is safe.

use crate::db::Db;
use crate::diag::Reject;
use crate::ty::Ty;

/// The body of `__cdz_doc_<export>` for a result of type `result_ty`, invoked as `call_expr` (e.g.
/// `main()`). Builds the `(: value type)` doc and returns `CDZDOC:<hex>`. `Err` (a shape not yet covered)
/// → the caller emits no `__cdz_doc` and the driver falls back to `cdz_render_at` (safe).
pub(super) fn emit_result_doc(
    db: &mut Db,
    result_ty: &Ty,
    call_expr: &str,
) -> Result<String, Reject> {
    let mut out = String::new();
    let mut ctr = 0usize;
    out.push_str("    let mut __b = cadenza_ast::ast::Builder::new();\n");
    out.push_str(&format!("    let __r = {call_expr};\n"));
    let vnode = doc_value_node(db, result_ty, "__r", &mut out, &mut ctr)?;
    let tnode = doc_type_node(db, result_ty, &mut out, &mut ctr)?;
    out.push_str("    let __colon = __b.name(\":\");\n");
    out.push_str(&format!(
        "    let __root = __b.list(vec![__colon, {vnode}, {tnode}]);\n"
    ));
    out.push_str("    let __bytes = cadenza_ast::codec::encode(&__b.finish(__root));\n");
    // Hex-encode with the `CDZDOC:` marker (matching cdz_rust_run::value_doc::interpret_run_stdout).
    out.push_str(
        "    let mut __s = String::from(\"CDZDOC:\");\n\
         \x20   const __H: &[u8] = b\"0123456789abcdef\";\n\
         \x20   for __x in &__bytes { __s.push(__H[(__x >> 4) as usize] as char); __s.push(__H[(__x & 15) as usize] as char); }\n\
         \x20   __s\n",
    );
    Ok(out)
}

fn fresh(ctr: &mut usize) -> String {
    let n = *ctr;
    *ctr += 1;
    format!("__n{n}")
}

/// Emit `let`-bindings building the UNIT sub-AST for a `Qty`'s unit — the SAME shape `lower::unit_value_ast`
/// bakes (which is what the wasm boundary encodes for BOTH the `(Qty.of …)` value and the `(Qty …)` type, so
/// one builder serves both). Forms: `Unit.one` (dimensionless / an empty factor list); `(Unit.base #"name")`
/// (a base at power 1); `(Unit.^ base k)` (a base at power k, k the POSITIVE exponent); a left-nested
/// `(Unit.* a b)` PRODUCT of several factors; and a `(Unit./ num den)` QUOTIENT when there are negative
/// exponents (the denominator's exponents made positive). Heads are bare `Name`s (printed verbatim →
/// sugared); a base name rides a raw `Leaf::Sym` (the printer escapes for `#"…"`). Mirrors `unit_value_ast`
/// exactly (base-name order = the `BTreeMap`'s sorted `entries()`), so the render byte-matches the wasm gate.
fn doc_unit_node(
    unit: &crate::ty::Unit,
    out: &mut String,
    ctr: &mut usize,
) -> Result<String, Reject> {
    // One base factor at a positive exponent: `(Unit.base #"name")`, or `(Unit.^ (Unit.base #"name") k)`.
    fn factor(name: &str, exp: i64, out: &mut String, ctr: &mut usize) -> String {
        let bh = fresh(ctr);
        out.push_str(&format!("    let {bh} = __b.name(\"Unit.base\");\n"));
        let sy = fresh(ctr);
        out.push_str(&format!(
            "    let {sy} = __b.atom_leaf(cadenza_ast::ast::Leaf::Sym({name:?}.into()));\n"
        ));
        let base = fresh(ctr);
        out.push_str(&format!("    let {base} = __b.list(vec![{bh}, {sy}]);\n"));
        if exp == 1 {
            base
        } else {
            let ph = fresh(ctr);
            out.push_str(&format!("    let {ph} = __b.name(\"Unit.^\");\n"));
            let n = fresh(ctr);
            out.push_str(&format!(
                "    let {n} = __b.atom_leaf(cadenza_ast::ast::Leaf::Int {{ value: cadenza_ast::ast::IntValue::from_i64({exp}), radix: cadenza_ast::ast::Radix::Dec }});\n"
            ));
            let f = fresh(ctr);
            out.push_str(&format!(
                "    let {f} = __b.list(vec![{ph}, {base}, {n}]);\n"
            ));
            f
        }
    }
    // Left-nested product `(Unit.* (Unit.* f0 f1) f2)…`; an empty factor list is the dimensionless `Unit.one`.
    fn product(factors: &[(String, i64)], out: &mut String, ctr: &mut usize) -> String {
        let Some(((n0, e0), rest)) = factors.split_first() else {
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.name(\"Unit.one\");\n"));
            return v;
        };
        let mut acc = factor(n0, *e0, out, ctr);
        for (name, exp) in rest {
            let f = factor(name, *exp, out, ctr);
            let mh = fresh(ctr);
            out.push_str(&format!("    let {mh} = __b.name(\"Unit.*\");\n"));
            let m = fresh(ctr);
            out.push_str(&format!(
                "    let {m} = __b.list(vec![{mh}, {acc}, {f}]);\n"
            ));
            acc = m;
        }
        acc
    }
    let entries: Vec<(String, i64)> = unit.entries().map(|(n, e)| (n.clone(), *e)).collect();
    // Split into positive (numerator) and negative (denominator, exponents made positive) factors, in the
    // `BTreeMap`'s sorted base-name order (matching `unit_value_ast`).
    let num: Vec<(String, i64)> = entries
        .iter()
        .filter(|(_, e)| *e > 0)
        .map(|(n, e)| (n.clone(), *e))
        .collect();
    let den: Vec<(String, i64)> = entries
        .iter()
        .filter(|(_, e)| *e < 0)
        .map(|(n, e)| (n.clone(), -*e))
        .collect();
    if den.is_empty() {
        // All positive (or empty → `Unit.one`) — a plain product / single factor / the identity.
        return Ok(product(&num, out, ctr));
    }
    // A quotient `(Unit./ numerator denominator)` — the derived-unit surface.
    let numerator = product(&num, out, ctr);
    let denominator = product(&den, out, ctr);
    let dh = fresh(ctr);
    out.push_str(&format!("    let {dh} = __b.name(\"Unit./\");\n"));
    let v = fresh(ctr);
    out.push_str(&format!(
        "    let {v} = __b.list(vec![{dh}, {numerator}, {denominator}]);\n"
    ));
    Ok(v)
}

/// Emit `let`-bindings (into `out`) building the VALUE node for `val_expr` (of Cadenza type `ty`); return
/// the final node's Rust variable. Field access is by-value (`(expr).i`) — disjoint tuple/record fields are
/// partial moves, fine for a single linear walk. A NOMINAL/QTY is transparent (walk the erased inner).
fn doc_value_node(
    db: &mut Db,
    ty: &Ty,
    val_expr: &str,
    out: &mut String,
    ctr: &mut usize,
) -> Result<String, Reject> {
    // A QTY must be handled BEFORE `strip_nominal_and_qty` erases it — its canonical value form is
    // `(Qty.of <magnitude> <unit>)` (the wasm `value_codec` shape), NOT the bare magnitude the strip would
    // leave (which would silently render `(: 5.0 Float64)` instead of `(: (Qty.of 5.0 (Unit.base #"meter"))
    // (Qty …))`). The `Qty.of` head is a bare `Name` (printed verbatim → sugared, matching `unit_value_ast`);
    // the magnitude walks the erased inner (the Rust value IS the bare f64/i64/…, since a Qty adds nothing at
    // run time — §156); the unit is a `doc_unit_node` sub-AST.
    if let Ty::Qty { inner, unit } = ty.strip_nominal() {
        let inner = (**inner).clone();
        let unit = unit.clone();
        let unit_node = doc_unit_node(&unit, out, ctr)?;
        let head = fresh(ctr);
        out.push_str(&format!("    let {head} = __b.name(\"Qty.of\");\n"));
        // DISPLAY-SCALE the magnitude to the dimension's REFERENCE unit before rendering: a stored quantity
        // displays with its magnitude scaled to the reference (`5 kilometer` → `5000.0 meter`), a DISPLAY
        // concern that does NOT alter the stored value (`Qty.value` still returns 5.0). The prefix/named-unit
        // factor rides `unit.scale()` (num/den); the dimension rides `unit.entries()`, which `doc_unit_node`
        // already renders as the reference (`(Unit.base #"meter")`). Mirror the wasm path
        // (`lower::value_form::const_value_ast_scaled`): a Float rounds via an f64 multiply. A non-(1/1) scale
        // on a NON-Float inner declines to the `cdz_render_at` fallback (which scales Int truncating /
        // Rational exact) — value_doc covers the Float display-scale here. A reference/base unit (scale 1/1)
        // is byte-neutral (render the magnitude as-is), so a base-unit Qty is unchanged.
        let (num, den) = unit.scale();
        let scaled_expr: String = if num == 1 && den == 1 {
            val_expr.to_string()
        } else if let Ty::Float(ft) = &inner {
            let scaled =
                format!("((({val_expr}) as f64) * ({num}i128 as f64) / ({den}i128 as f64))");
            if ft.ground_width() == 32 {
                format!("({scaled} as f32)")
            } else {
                scaled
            }
        } else {
            return Err(Reject::decline(
                "value-doc: non-Float prefixed Qty magnitude scaling — cdz_render_at fallback",
            ));
        };
        let mag = doc_value_node(db, &inner, &scaled_expr, out, ctr)?;
        let v = fresh(ctr);
        out.push_str(&format!(
            "    let {v} = __b.list(vec![{head}, {mag}, {unit_node}]);\n"
        ));
        return Ok(v);
    }
    match ty.strip_nominal_and_qty() {
        // An integer leaf → an `Int` atom (its runtime value is the i64-slot magnitude; `from_i64` is exact
        // for a signed Int64, the only width covered so far — an unsigned/narrow width is a later increment).
        Ty::Int(it) => {
            // SIGNEDNESS matters for the wire value: a SIGNED int widens via `as i64` (sign-extends — a
            // negative narrow int stays negative); an UNSIGNED int widens via `as u128` (zero-extends → the
            // correct unsigned decimal). `from_i64` on a `u64 as i64` would two's-complement a high-bit-set
            // UInt64 to a NEGATIVE i64 (`2^63 → i64::MIN`, `u64::MAX → -1`) — the value-doc UInt64 bug
            // v-cdz-smith found; `from_u128((v) as u128)` renders the unsigned decimal, matching wasm.
            let value_ctor = if it.ground_signed() {
                format!("cadenza_ast::ast::IntValue::from_i64(({val_expr}) as i64)")
            } else {
                format!("cadenza_ast::ast::IntValue::from_u128(({val_expr}) as u128)")
            };
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.atom_leaf(cadenza_ast::ast::Leaf::Int {{ value: {value_ctor}, radix: cadenza_ast::ast::Radix::Dec }});\n"
            ));
            Ok(v)
        }
        // A bool → a `Bool` atom.
        Ty::Bool => {
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.atom_leaf(cadenza_ast::ast::Leaf::Bool({val_expr}));\n"
            ));
            Ok(v)
        }
        // A string → a `Leaf::Str` (renders `"…"`, the escape codec in the printer). Rust value is a
        // `String`; `.as_str().into()` copies into the `Arc<str>` the leaf holds.
        Ty::String => {
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.atom_leaf(cadenza_ast::ast::Leaf::Str(({val_expr}).as_str().into()));\n"
            ));
            Ok(v)
        }
        // A symbol → `(Symbol.of "…")` — the CONSTRUCTOR form the wasm gate / corpus record for a Symbol
        // RESULT (`(output (: (Symbol.of "hot") Symbol))`, spec/semantics/17-symbols.sexp), NOT the bare
        // `#"…"` `Leaf::Sym` literal (that is cdz-run's `render_val_typed` form, a DIFFERENT non-gate path).
        // `Symbol.of` is a bare `Name` head (sugared); the argument is the symbol text as a `Leaf::Str`
        // (`"…"`, NOT `#"…"`). Rust rep is a `String` (Symbol erases to String).
        Ty::Symbol => {
            // Head = the STRUCTURAL member node `(. Symbol of)` (`Leaf::Member`), NOT a flat `Name
            // "Symbol.of"`. The wasm value_codec builds — and cdz-run RENDERS — a Symbol result UNSUGARED as
            // `((. Symbol of) "…")` (verified: `cdz run` prints `((. Symbol of) "escaping-symbol")`, the
            // const path uses `lower::member_access`, structural). A flat `Name "Symbol.of"` is a
            // structurally-DIFFERENT binary-AST → cdz-smith rust-vs-wasm fuzz false-mismatch (Symbol is in the
            // fuzz grammar since #7732; v-runtime/breaker co-diagnosed). `Builder::member` takes two nodes, so
            // bind recv + key first (two `__b` borrows can't overlap in one call, E0499). (NOTE: Qty.of and
            // the Unit.* heads render FLAT in the value_codec — `cdz run` prints `(Qty.of 5.0 (Unit.base …))`
            // — so ONLY the Symbol head is structural; do NOT "generalize" this to Qty/Unit.)
            let recv = fresh(ctr);
            out.push_str(&format!("    let {recv} = __b.name(\"Symbol\");\n"));
            let key = fresh(ctr);
            out.push_str(&format!("    let {key} = __b.name(\"of\");\n"));
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.member({recv}, {key});\n"));
            let s = fresh(ctr);
            out.push_str(&format!(
                "    let {s} = __b.atom_leaf(cadenza_ast::ast::Leaf::Str(({val_expr}).as_str().into()));\n"
            ));
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list(vec![{head}, {s}]);\n"));
            Ok(v)
        }
        // Bytes → a `Leaf::Bytes` (renders `b\"…\"`). Rust value is a `Vec<u8>`; `.into()` moves it into the
        // `Arc<[u8]>` the leaf holds.
        Ty::Bytes => {
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.atom_leaf(cadenza_ast::ast::Leaf::Bytes(({val_expr}).into()));\n"
            ));
            Ok(v)
        }
        // A char → a `Leaf::Char` (renders `#\\c`). Rust value is a `char` (Copy) — no clone/borrow needed.
        Ty::Char => {
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.atom_leaf(cadenza_ast::ast::Leaf::Char({val_expr}));\n"
            ));
            Ok(v)
        }
        // A tuple → `#tuple(<e0> <e1> …)`: a `Leaf::Ctor(CompoundCtor::Tuple)` head (via `Builder::compound`)
        // then each element's node (walked at `.i`). The Ctor head renders `#tuple(…)` — the canonical form
        // cdz-run's `render_val`, the corpus, and the grader all use (NOT a `Name "tuple"` head, which renders
        // the divergent bare `(tuple …)`).
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            let mut kids = Vec::with_capacity(elems.len());
            for (i, e) in elems.iter().enumerate() {
                kids.push(doc_value_node(
                    db,
                    e,
                    &format!("({val_expr}).{i}"),
                    out,
                    ctr,
                )?);
            }
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.compound(cadenza_ast::ast::CompoundCtor::Tuple, &[{}]);\n",
                kids.join(", ")
            ));
            Ok(v)
        }
        // A record → `#record((= <f0> <v0>) (= <f1> <v1>) …)`: a `Leaf::Ctor(CompoundCtor::Record)` head (via
        // `Builder::compound`) then, per field (in BTreeMap = SORTED-key order, matching the emitted tuple's
        // `.i`), a `(= <field> <value>)` entry built by `Builder::field_pair` (the `=` marker is a
        // `Leaf::FieldPair`). The Ctor head renders `#record(…)` — the canonical form cdz-run/corpus/grader use.
        Ty::Record(fields) => {
            let fields: Vec<(String, Ty)> = fields
                .iter()
                .map(|(k, t)| (k.name.to_string(), t.clone()))
                .collect();
            let mut kids = Vec::with_capacity(fields.len());
            for (i, (fname, fty)) in fields.iter().enumerate() {
                let fnn = fresh(ctr);
                out.push_str(&format!("    let {fnn} = __b.name({fname:?});\n"));
                let fv = doc_value_node(db, fty, &format!("({val_expr}).{i}"), out, ctr)?;
                let pair = fresh(ctr);
                out.push_str(&format!("    let {pair} = __b.field_pair({fnn}, {fv});\n"));
                kids.push(pair);
            }
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.compound(cadenza_ast::ast::CompoundCtor::Record, &[{}]);\n",
                kids.join(", ")
            ));
            Ok(v)
        }
        // A FLOAT → a `Leaf::Float` (finite) / `Leaf::FloatNan` / `Leaf::FloatInf` at RUNTIME, EXACTLY the
        // canonical `value_codec` / cdz-run `float_atom` disposition: NaN → `FloatNan` (`nan`), ±inf →
        // `FloatInf { negative }` (`inf`/`-inf`), finite → `Decimal::from_f{32,64}` (the 3-codec-identical
        // shortest decimal). A `Float32` uses `from_f32` on its OWN f32 (NOT the f32→f64 PROMOTION — operator
        // ruling #7554: the promoted shortest is a different number); a `Float64` uses `from_f64`. The Rust
        // value here is a bare `f32`/`f64` (the `__CdzF64`/`__CdzF32` ord-key wrapper only wraps a Set/Map
        // key/element — a later increment unwraps via `.get()`). The TYPE node is the bare `FloatN` name (the
        // `doc_type_node` leaf arm's `render_name`), so only the VALUE arm is needed here.
        Ty::Float(ft) => {
            let from = if ft.ground_width() == 32 {
                "from_f32"
            } else {
                "from_f64"
            };
            let f = fresh(ctr);
            out.push_str(&format!("    let {f} = {val_expr};\n"));
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = if {f}.is_nan() {{ __b.atom_leaf(cadenza_ast::ast::Leaf::FloatNan) }} \
                 else if {f}.is_infinite() {{ __b.atom_leaf(cadenza_ast::ast::Leaf::FloatInf {{ negative: {f}.is_sign_negative() }}) }} \
                 else {{ __b.atom_leaf(cadenza_ast::ast::Leaf::Float(cadenza_ast::ast::Decimal::{from}({f}).expect(\"a finite float has a Decimal\"))) }};\n"
            ));
            Ok(v)
        }
        // A LIST → `(list <e0> <e1> …)`: a `Name "list"` head then each element's value-node, appended at
        // RUNTIME by consuming the `Vec<T>` (`for __e in __r`) so an arbitrary-length list works. The head
        // + kids vec + iterator binder all use FRESH names (a nested list `(list (list …) …)` else shadows
        // the outer `__kids`, so its inner `push` would target the wrong vec). The element walk emits its
        // bindings into the LOOP-BODY buffer (re-run per element) and returns the built-node var to push.
        Ty::List(elem) => {
            let elem = (**elem).clone();
            let kids = fresh(ctr);
            let iter = fresh(ctr);
            out.push_str(&format!("    let mut {kids} = Vec::new();\n"));
            let mut body = String::new();
            let enode = doc_value_node(db, &elem, &iter, &mut body, ctr)?;
            out.push_str(&format!(
                "    for {iter} in ({val_expr}) {{\n{body}        {kids}.push({enode});\n    }}\n"
            ));
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.compound(cadenza_ast::ast::CompoundCtor::List, &{kids});\n"
            ));
            Ok(v)
        }
        // A SET → `(set e1 e2 …)` (ctor word `set`): a `Name "set"` head then each element's value-node in
        // CANONICAL order. The Rust rep is a `BTreeSet<T>`, whose iteration IS sorted-by-`Ord` = the canonical
        // key-value order the wasm `value_codec` emits (both sort by the element's canonical scalar) — so a
        // plain consuming `for __e in __r` yields the elements already in the right order (no re-sort). Fresh
        // head/kids/iter binders per level (nested set shadowing). A DIRECT FLOAT element is stored as the
        // `__CdzF{N}` ord-key wrapper (bare `f{N}` is not `Ord`), so its float is read via `.get()` (Copy
        // `self`) before the Float value-node walks it; a non-float Ord element is the bare value. A
        // COMPOUND-containing-float element (a tuple-with-float, wrapped PER-POSITION) is a follow-up — decline
        // (it is neither a direct float nor `ty_is_ord`).
        Ty::Set(elem) => {
            let elem = (**elem).clone();
            let kids = fresh(ctr);
            let iter = fresh(ctr);
            let elem_val = if matches!(elem, Ty::Float(_)) {
                format!("{iter}.get()")
            } else if crate::backend::rust::types::ty_is_ord(db, &elem) {
                iter.clone()
            } else {
                return Err(Reject::decline(
                    "value-doc: compound-float Set element not covered (needs per-position __CdzF unwrap)",
                ));
            };
            out.push_str(&format!("    let mut {kids} = Vec::new();\n"));
            let mut body = String::new();
            let enode = doc_value_node(db, &elem, &elem_val, &mut body, ctr)?;
            out.push_str(&format!(
                "    for {iter} in ({val_expr}) {{\n{body}        {kids}.push({enode});\n    }}\n"
            ));
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.compound(cadenza_ast::ast::CompoundCtor::Set, &{kids});\n"
            ));
            Ok(v)
        }
        // A MAP → `(map (= k1 v1) (= k2 v2) …)` (ctor word `map`): a `Name "map"` head then, per entry in
        // CANONICAL KEY order, a `List[FieldPair, <key-node>, <value-node>]` (the `=` marker is `Leaf::FieldPair`,
        // as records use). The Rust rep is a `BTreeMap<K, V>`, whose consuming `for (__k, __v) in __r` yields
        // entries sorted by `K`'s `Ord` = the canonical key order. Fresh binders per level. A DIRECT FLOAT KEY
        // is the `__CdzF{N}` wrapper, unwrapped via `.get()` (only the KEY position wraps — a float VALUE stays
        // a bare `f{N}`, so `{kv}.1` walks unchanged). A COMPOUND-float key is a follow-up — decline.
        Ty::Map(k, val_ty) => {
            let kty = (**k).clone();
            let vty = (**val_ty).clone();
            let kids = fresh(ctr);
            let kv = fresh(ctr);
            let entry = fresh(ctr);
            let key_val = if matches!(kty, Ty::Float(_)) {
                format!("{kv}.0.get()")
            } else if crate::backend::rust::types::ty_is_ord(db, &kty) {
                format!("{kv}.0")
            } else {
                return Err(Reject::decline(
                    "value-doc: compound-float Map key not covered (needs per-position __CdzF unwrap)",
                ));
            };
            out.push_str(&format!("    let mut {kids} = Vec::new();\n"));
            let mut body = String::new();
            let knode = doc_value_node(db, &kty, &key_val, &mut body, ctr)?;
            let vnode = doc_value_node(db, &vty, &format!("{kv}.1"), &mut body, ctr)?;
            out.push_str(&format!(
                "    for {kv} in ({val_expr}) {{\n{body}        let {entry} = __b.field_pair({knode}, {vnode});\n        {kids}.push({entry});\n    }}\n"
            ));
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.compound(cadenza_ast::ast::CompoundCtor::Map, &{kids});\n"
            ));
            Ok(v)
        }
        // A SUM (Option / Result / a user sum) → a `match` over the emitted enum. Each variant arm builds
        // the canonical `(<Variant> <payload…>)` node the wasm `value_codec` emits (value_codec.rs §Sum): a
        // `Name <variant>` head then, for a NULLARY variant, the bare `unit` atom (`(None unit)`); for a
        // SINGLE-payload variant, the payload's own value-node (`(Some 5)`). Head is the BARE variant name.
        //
        // Covers nullary `(V unit)`, single-payload `(V p)`, and MULTI-field `(V p0 p1 …)` (flattened, driven
        // by the declared arity) bare-head variants. A QUALIFIED-head sum (`sum_needs_qualified_heads` — a
        // variant name shadowed by a type-ctor/module, so the head must render `((. Ast Str) …)`) and a
        // RECURSIVE sum (a self-referential/`Box`ed payload needing a helper fn to terminate) each DECLINE —
        // follow-up increments. A decline is never a miscompile (the driver keeps `cdz_render_at`).
        Ty::Sum { decl, .. } => {
            let decl_occ = *decl;
            let sum_ty = ty.strip_nominal_and_qty().clone();
            // A QUALIFIED-head sum (`sum_needs_qualified_heads` — a variant name shadowed by a prelude
            // type-ctor/module, e.g. reflection `Ast`) renders each head as the TYPE-QUALIFIED dotted name
            // `<Type>.<Variant>` (a single `Name` atom the canonical printer sugars from `(. Type Variant)` —
            // verified `(T.List 5)` / `(Ast.Str "x")` round-trip), so a bare `Str` head can't resolve to the
            // colliding prelude binding. A non-qualified sum keeps the bare variant name.
            let qualified = crate::lower::sum_needs_qualified_heads(db, decl_occ);
            let type_name = if qualified {
                db.type_decl_by_occ(decl_occ)
                    .map(|t| t.name.to_string())
                    .ok_or_else(|| {
                        Reject::decline("value-doc: qualified sum has no declaration name")
                    })?
            } else {
                String::new()
            };
            let variant_count = db
                .type_decl_by_occ(decl_occ)
                .map(|t| t.variants.len())
                .ok_or_else(|| Reject::decline("value-doc: sum has no declaration"))?;
            let mut arms = String::new();
            for disc in 0..variant_count as u32 {
                // The BARE Cadenza variant name (the node head) and the Rust `<Enum>::<Variant>` match path.
                let vname = db
                    .type_decl_by_occ(decl_occ)
                    .and_then(|t| t.variants.get(disc as usize).map(|v| v.name.to_string()))
                    .ok_or_else(|| Reject::decline("value-doc: sum variant name not found"))?;
                // The node HEAD name: the bare variant for a normal sum, or the `<Type>.<Variant>` dotted
                // qualified name when the sum needs qualified heads.
                let head_name = if qualified {
                    format!("{type_name}.{vname}")
                } else {
                    vname.clone()
                };
                let path = super::expr::sum_variant_path_of_ty(db, &sum_ty, disc)?;
                match super::expr::variant_payload_ty(db, &sum_ty, disc) {
                    // Nullary → `(<Variant> unit)`: head atom then the canonical `unit` name atom.
                    None => {
                        arms.push_str(&format!(
                            "        {path} => {{ let __hv = __b.name({head_name:?}); let __u = __b.name(\"unit\"); __b.list(vec![__hv, __u]) }}\n"
                        ));
                    }
                    Some(payload_ty) => {
                        // A RECURSIVE sum declines: building the value-doc node walks the payload's own
                        // value-node, so a self-referential payload makes `doc_value_node` recurse forever
                        // (no helper-fn/terminator emitted yet). `variant_is_recursive` catches a BY-VALUE
                        // self-reference (`(Node Ast Ast)`), but the walk ALSO recurses through a CONTAINER
                        // payload — `(type Ast (Lit Int64) (Node (List Ast)))`: `doc_value_node(List Ast)` →
                        // `doc_value_node(Ast)` → this Sum arm → `(List Ast)` → … an INFINITE HANG at
                        // COMPILE time (v-nix (C)-gate "compile timeout", the Ast-sum crash). `reaches_decl`
                        // (behind `variant_is_recursive`) deliberately SKIPS List/Map/Set (the by-value Box
                        // logic — a `Vec`/`BTree` pointer breaks the SIZE cycle, so the enum emits fine), so
                        // it misses this; `mentions_decl` (which DOES follow containers) is the right guard
                        // for the value-doc WALK — decline when the payload reaches the decl through ANY
                        // position, container included. (Same List-recursion-vs-by-value distinction as the
                        // rose-tree newtype fix #7913.)
                        if super::enums::variant_is_recursive(db, &sum_ty, disc)
                            || super::enums::mentions_decl(&payload_ty, *decl)
                        {
                            return Err(Reject::decline(
                                "value-doc: recursive sum (self-referential payload, incl. through a \
                                 List/Map/Set) not covered by the rust value-doc emit — the doc walk \
                                 would recurse without a terminator",
                            ));
                        }
                        // The DECLARED arity distinguishes a MULTI-field variant `(V a b)` (arity ≥ 2 — the
                        // Rust enum binds ONE tuple `V((A,B))` which the codec FLATTENS as `(V p0 p1 …)`) from a
                        // SINGLE field `(V T)` (arity 1 — `(V <payload>)`, even when T is itself a tuple type →
                        // `(V (tuple …))`). `variant_payload_ty` returns a `Tuple` for BOTH, so match on ARITY,
                        // not on the payload being a tuple.
                        let arity = super::expr::variant_arity_of_ty(db, &sum_ty, disc);
                        let pbind = fresh(ctr);
                        let mut armbuf = String::new();
                        if arity >= 2 {
                            // Multi-field → FLATTEN: the bound `__p` is the tuple `(T0, T1, …)`; splice each
                            // element `(__p).i`'s node directly under the variant head (`(V p0 p1 …)`), NOT a
                            // nested `(tuple …)`.
                            let elems = match payload_ty.strip_nominal_and_qty() {
                                Ty::Tuple(es) if es.len() == arity => es.clone(),
                                _ => {
                                    return Err(Reject::decline(
                                        "value-doc: multi-field variant payload is not a matching tuple",
                                    ));
                                }
                            };
                            let mut enodes = Vec::with_capacity(arity);
                            for (i, e) in elems.iter().enumerate() {
                                enodes.push(doc_value_node(
                                    db,
                                    e,
                                    &format!("({pbind}).{i}"),
                                    &mut armbuf,
                                    ctr,
                                )?);
                            }
                            arms.push_str(&format!(
                                "        {path}({pbind}) => {{\n{armbuf}            let __hv = __b.name({head_name:?}); __b.list(vec![__hv, {}])\n        }}\n",
                                enodes.join(", ")
                            ));
                        } else {
                            // Single payload → `(<Variant> <payload-node>)`. The arm binds the payload by value
                            // (moving it out of the owned enum) and walks it into a per-arm buffer.
                            let pnode = doc_value_node(db, &payload_ty, &pbind, &mut armbuf, ctr)?;
                            arms.push_str(&format!(
                                "        {path}({pbind}) => {{\n{armbuf}            let __hv = __b.name({head_name:?}); __b.list(vec![__hv, {pnode}])\n        }}\n"
                            ));
                        }
                    }
                }
            }
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = match ({val_expr}) {{\n{arms}    }};\n"
            ));
            Ok(v)
        }
        _ => Err(Reject::decline(
            "value-doc: result shape not covered by the rust value-doc emit",
        )),
    }
}

/// Emit `let`-bindings building the TYPE node for `ty`; return the final node's Rust variable. A LEAF type
/// (Int64/Bool/…) is a `Name` atom of its `render_name`; a Tuple is `(Tuple <T0> …)`.
fn doc_type_node(
    db: &mut Db,
    ty: &Ty,
    out: &mut String,
    ctr: &mut usize,
) -> Result<String, Reject> {
    // A QTY TYPE → `(Qty <inner-type> <unit>)` (the `render_name` shape) — handled BEFORE the strip erases
    // the Qty. Same `doc_unit_node` as the value side (positive-exponent units render identically in the
    // value `(Qty.of …)` and the type `(Qty …)`; a derived unit declines there, so this is reached only for
    // a coverable unit).
    if let Ty::Qty { inner, unit } = ty.strip_nominal() {
        let inner = (**inner).clone();
        let unit = unit.clone();
        let head = fresh(ctr);
        out.push_str(&format!("    let {head} = __b.name(\"Qty\");\n"));
        let it = doc_type_node(db, &inner, out, ctr)?;
        let unit_node = doc_unit_node(&unit, out, ctr)?;
        let v = fresh(ctr);
        out.push_str(&format!(
            "    let {v} = __b.list(vec![{head}, {it}, {unit_node}]);\n"
        ));
        return Ok(v);
    }
    // A NOMINAL newtype TYPE → its DECLARED name, NOT the erased structural inner. The BOUNDARY renders a
    // nominal value's TYPE as the nominal name (`(: 5 UserId)`, `(: #list(1 2) Names)`, `(: 5 (Box Int64))`
    // for a generic nominal) — even though the VALUE crosses transparently as the erased inner (§156, which
    // is why `doc_value_node` strips the nominal). wasm's value_codec renders the nominal name here, so
    // value-doc must too or the `(: value type)` diverges from wasm for EVERY nominal-newtype return (and
    // for a nominal nested inside a compound type — `(List UserId)` — which this recursion also covers).
    // (A `Nominal(Qty …)` was already handled by the Qty branch above, so reaching here means the stripped
    // inner is not a Qty.) Monomorphic → bare `Name`; generic → `(<Name> <T…>)` (mirrors the Sum arm).
    if let Ty::Nominal { decl, args, .. } = ty {
        let args = args.clone();
        let name = {
            let ncx = db.name_ctx();
            ncx.name_of(*decl).unwrap_or("<nominal>").to_string()
        };
        if args.is_empty() {
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.name({name:?});\n"));
            return Ok(v);
        }
        let head = fresh(ctr);
        out.push_str(&format!("    let {head} = __b.name({name:?});\n"));
        let mut kids = vec![head];
        for a in args.iter() {
            kids.push(doc_type_node(db, a, out, ctr)?);
        }
        let v = fresh(ctr);
        out.push_str(&format!(
            "    let {v} = __b.list(vec![{}]);\n",
            kids.join(", ")
        ));
        return Ok(v);
    }
    match ty.strip_nominal_and_qty() {
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"Tuple\");\n"));
            let mut kids = vec![head];
            for e in elems.iter() {
                kids.push(doc_type_node(db, e, out, ctr)?);
            }
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.list(vec![{}]);\n",
                kids.join(", ")
            ));
            Ok(v)
        }
        // A record TYPE → `(Record (: <f0> <T0>) (: <f1> <T1>) …)`: a `Name "Record"` head then, per field
        // (sorted-key order), the canonical ASCRIPTION node `List[Name ":", Name <field>, <type-node>]`
        // (matching `render_name` + the corpus `(Record (: x Int64) …)` form — NOT a bare `[field, ty]`).
        Ty::Record(fields) => {
            // A record TYPE-annotation crosses in the BOUNDARY value-doc form `(record (name Type) …)` — a
            // LOWERCASE `record` head + BARE `(name Type)` field pairs — the shape `lower::value_form::
            // type_node_of` (the escaping-value type-annotation baker, value_form.rs:525) builds + the wasm
            // value_codec doc carries. It is NOT the render_ty SURFACE type spelling `(Record (: name Type))`
            // (uppercase + `:` ascriptions), which is for error/query display — emitting that here is a
            // structurally-different binary-AST than the wasm value-doc → rust-vs-wasm split (v-runtime/breaker,
            // static-source confirmed). Mirror type_node_of exactly: lowercase head, bare pairs.
            let fields: Vec<(String, Ty)> = fields
                .iter()
                .map(|(k, t)| (k.name.to_string(), t.clone()))
                .collect();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"record\");\n"));
            let mut kids = vec![head];
            for (fname, fty) in fields.iter() {
                let fnn = fresh(ctr);
                out.push_str(&format!("    let {fnn} = __b.name({fname:?});\n"));
                let ft = doc_type_node(db, fty, out, ctr)?;
                let pair = fresh(ctr);
                out.push_str(&format!("    let {pair} = __b.list(vec![{fnn}, {ft}]);\n"));
                kids.push(pair);
            }
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.list(vec![{}]);\n",
                kids.join(", ")
            ));
            Ok(v)
        }
        // A LIST TYPE → `(List <elem>)` (the `render_name` shape): a `Name "List"` head then the element
        // type-node.
        Ty::List(elem) => {
            let elem = (**elem).clone();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"List\");\n"));
            let et = doc_type_node(db, &elem, out, ctr)?;
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list(vec![{head}, {et}]);\n"));
            Ok(v)
        }
        // A SET TYPE → `(Set <elem>)`; a MAP TYPE → `(Map <key> <value>)` (the `render_name` shapes).
        Ty::Set(elem) => {
            let elem = (**elem).clone();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"Set\");\n"));
            let et = doc_type_node(db, &elem, out, ctr)?;
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list(vec![{head}, {et}]);\n"));
            Ok(v)
        }
        Ty::Map(k, val_ty) => {
            let kty = (**k).clone();
            let vty = (**val_ty).clone();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"Map\");\n"));
            let kt = doc_type_node(db, &kty, out, ctr)?;
            let vt = doc_type_node(db, &vty, out, ctr)?;
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.list(vec![{head}, {kt}, {vt}]);\n"
            ));
            Ok(v)
        }
        // A SUM TYPE → its NOMINAL name applied to type ARGS (the `render_name` shape): a MONOMORPHIC sum
        // (`args` empty) is the bare `Name <name>` (`Sign`); a GENERIC sum is `(<Name> <T…>)` — `(Option
        // Int64)`, `(Result Int64 Bool)` — built structurally so the head + args render as a list (NOT the
        // whole `render_name` string in one atom, which would render `(Option Int64)` as a quoted leaf).
        Ty::Sum { decl, args } => {
            let args = args.clone();
            let name = {
                let ncx = db.name_ctx();
                ncx.name_of(*decl).unwrap_or("<sum>").to_string()
            };
            if args.is_empty() {
                let v = fresh(ctr);
                out.push_str(&format!("    let {v} = __b.name({name:?});\n"));
                Ok(v)
            } else {
                let head = fresh(ctr);
                out.push_str(&format!("    let {head} = __b.name({name:?});\n"));
                let mut kids = vec![head];
                for a in args.iter() {
                    kids.push(doc_type_node(db, a, out, ctr)?);
                }
                let v = fresh(ctr);
                out.push_str(&format!(
                    "    let {v} = __b.list(vec![{}]);\n",
                    kids.join(", ")
                ));
                Ok(v)
            }
        }
        // A leaf type — its `render_name` is the bare name (`Int64`, `Bool`, …); one `Name` atom. `{name:?}`
        // quotes it as a Rust string literal.
        leaf => {
            let name = {
                let ncx = db.name_ctx();
                leaf.render_name(&ncx)
            };
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.name({name:?});\n"));
            Ok(v)
        }
    }
}

#[cfg(test)]
mod qty_display_scale_tests {
    //! Pin the value-doc Qty display-scale EMIT at the dev-gate level. The `5 kilometer` → `5000.0
    //! meter` reference-scale render is protected end-to-end by the units corpus
    //! (`spec/semantics/18-units-of-measure.sexp`, `pass` on `.gate-baseline-rust`), but that only runs
    //! in pr-sync's full battery. This module guards the exact emit arm above in ~ms — the path took
    //! TWO false-alarm regression reports (#7824 fix, then a stale-binary alarm), so a fast local
    //! witness that fails the instant the scale multiply is dropped is worth pinning.
    use super::*;
    use crate::ty::{FloatTy, IntTy, Unit};

    // A minimal loadable program → a real `Db` to thread through `emit_result_doc`. The Float
    // display-scale arm consults only the `Ty` + `Unit` (never the Db), but the signature requires one.
    fn db() -> Db {
        Db::load(crate::testkit::parse("(do (def (main) 0) (export main))"))
    }

    fn qty(inner: Ty, unit: Unit) -> Ty {
        Ty::Qty {
            inner: Box::new(inner),
            unit,
        }
    }

    #[test]
    fn kilo_prefixed_float64_qty_emits_the_x1000_reference_scale_multiply() {
        let mut db = db();
        let ty = qty(
            Ty::Float(FloatTy::f64()),
            Unit::base("meter").scaled(1000, 1).unwrap(),
        );
        let body = emit_result_doc(&mut db, &ty, "main()").expect(
            "value-doc covers the Float display-scale, so a kilo-meter f64 Qty must render",
        );
        assert!(
            body.contains("* (1000i128 as f64) / (1i128 as f64)"),
            "expected the ×1000 kilo display-scale multiply in the emitted body, got:\n{body}"
        );
    }

    #[test]
    fn milli_prefixed_float64_qty_emits_the_div1000_reference_scale_multiply() {
        let mut db = db();
        let ty = qty(
            Ty::Float(FloatTy::f64()),
            Unit::base("meter").scaled(1, 1000).unwrap(),
        );
        let body =
            emit_result_doc(&mut db, &ty, "main()").expect("a milli-meter f64 Qty must render");
        assert!(
            body.contains("* (1i128 as f64) / (1000i128 as f64)"),
            "expected the /1000 milli display-scale multiply in the emitted body, got:\n{body}"
        );
    }

    #[test]
    fn base_scale_1_float64_qty_emits_no_scaling_multiply() {
        let mut db = db();
        let ty = qty(Ty::Float(FloatTy::f64()), Unit::base("meter"));
        let body =
            emit_result_doc(&mut db, &ty, "main()").expect("a base-meter f64 Qty must render");
        // A reference/base unit (scale 1/1) is byte-neutral: the magnitude renders AS-IS, no multiply.
        assert!(
            !body.contains("i128 as f64"),
            "a scale-1 base-unit Qty must render its magnitude UNSCALED (no display-scale multiply), got:\n{body}"
        );
    }

    #[test]
    fn kilo_prefixed_float32_qty_scales_in_f64_then_narrows_to_f32() {
        let mut db = db();
        let ty = qty(
            Ty::Float(FloatTy::fixed(32)),
            Unit::base("meter").scaled(1000, 1).unwrap(),
        );
        let body =
            emit_result_doc(&mut db, &ty, "main()").expect("a kilo-meter f32 Qty must render");
        assert!(
            body.contains("* (1000i128 as f64) / (1i128 as f64)") && body.contains("as f32)"),
            "a Float32 prefixed Qty must scale in f64 then narrow to f32, got:\n{body}"
        );
    }

    #[test]
    fn prefixed_int_qty_declines_to_the_render_fallback() {
        // The value-doc Qty arm covers only the FLOAT display-scale; a non-(1/1) scale on an INTEGER
        // inner declines to `cdz_render_at` (which scales the integer truncating). Pin the decline so a
        // future change can't silently start emitting an unscaled/wrong integer magnitude here.
        let mut db = db();
        let ty = qty(
            Ty::Int(IntTy::i64()),
            Unit::base("meter").scaled(1000, 1).unwrap(),
        );
        assert!(
            emit_result_doc(&mut db, &ty, "main()").is_err(),
            "a prefixed INT Qty must DECLINE (the integer display-scale lives in the cdz_render_at fallback)"
        );
    }
}

#[cfg(test)]
mod set_map_float_key_tests {
    //! Pin the `__CdzF{N}` ord-key UNWRAP in the Set-element / Map-key value-doc arms. A direct float
    //! Set element or Map KEY is stored as the `__CdzF{N}` ord-key wrapper (a bare `f{N}` is not `Ord`,
    //! so a `BTreeSet`/`BTreeMap` cannot hold it), and the value walk must read the float back via
    //! `.get()` before the Float value-node renders it. The subtle invariant is the ASYMMETRY: only the
    //! KEY/element position wraps — a Map VALUE float stays a bare `f{N}` and must NOT be `.get()`-unwrapped.
    //! A refactor that dropped the `.get()` on a key would emit `__CdzF64` where an `f64` is expected (a
    //! rustc type error caught only in pr-sync's full battery); one that ADDED `.get()` to a value would
    //! not compile either. Pin both directions at the ms level. (An Ord non-float element/key reads bare.)
    use super::*;
    use crate::ty::{FloatTy, IntTy};

    fn db() -> Db {
        Db::load(crate::testkit::parse("(do (def (main) 0) (export main))"))
    }
    fn f64() -> Ty {
        Ty::Float(FloatTy::f64())
    }
    fn int() -> Ty {
        Ty::Int(IntTy::i64())
    }

    #[test]
    fn a_set_of_direct_floats_reads_each_element_via_get() {
        let mut db = db();
        let body = emit_result_doc(&mut db, &Ty::Set(Box::new(f64())), "main()")
            .expect("a Set<f64> value-doc must render (direct-float element covered)");
        assert!(
            body.contains(".get()"),
            "a direct-float Set element must be read out of its __CdzF ord-key wrapper via `.get()`, got:\n{body}"
        );
    }

    #[test]
    fn a_set_of_ord_scalars_reads_each_element_bare() {
        let mut db = db();
        let body = emit_result_doc(&mut db, &Ty::Set(Box::new(int())), "main()")
            .expect("a Set<i64> value-doc must render");
        assert!(
            !body.contains(".get()"),
            "an Ord (non-float) Set element is the bare value — no __CdzF unwrap, got:\n{body}"
        );
    }

    #[test]
    fn a_map_with_a_direct_float_key_unwraps_the_key_via_get() {
        let mut db = db();
        let body = emit_result_doc(
            &mut db,
            &Ty::Map(Box::new(f64()), Box::new(int())),
            "main()",
        )
        .expect("a Map<f64,i64> value-doc must render (direct-float key covered)");
        assert!(
            body.contains(".0.get()"),
            "a direct-float Map KEY must unwrap its __CdzF ord-key wrapper via `.0.get()`, got:\n{body}"
        );
    }

    #[test]
    fn a_map_float_value_stays_bare_only_the_key_wraps() {
        // The ASYMMETRY: key i64 (Ord) reads `.0` bare; value f64 stays a bare `f{N}` and walks `.1`
        // directly — NEITHER position unwraps, so no `.get()` appears. Guards against a refactor that
        // wrongly treats a float VALUE like a float key.
        let mut db = db();
        let body = emit_result_doc(
            &mut db,
            &Ty::Map(Box::new(int()), Box::new(f64())),
            "main()",
        )
        .expect("a Map<i64,f64> value-doc must render");
        assert!(
            !body.contains(".get()"),
            "a Map float VALUE stays a bare f{{N}} (only the KEY position wraps) — no `.get()` unwrap, got:\n{body}"
        );
    }
}

#[cfg(test)]
mod float_render_width_tests {
    //! Pin the WIDTH-DEPENDENT float codec: a `Float32` renders via `Decimal::from_f32` on its OWN f32,
    //! NOT the f32→f64 promotion (operator ruling #7554 — the promoted shortest decimal is a DIFFERENT
    //! number, e.g. 0.1_f32 promoted to f64 prints extra digits). A `Float64` (and a still-deferred float,
    //! which grounds to Float64) renders via `from_f64`. A refactor that used one codec for both widths
    //! would silently change every Float32 rendered value — a value MISCOMPILE the corpus catches only in
    //! pr-sync's full battery, so pin the width→codec mapping here at the ms level.
    use super::*;
    use crate::ty::FloatTy;

    fn db() -> Db {
        Db::load(crate::testkit::parse("(do (def (main) 0) (export main))"))
    }

    #[test]
    fn a_float32_renders_via_from_f32_not_the_f64_promotion() {
        let mut db = db();
        let body = emit_result_doc(&mut db, &Ty::Float(FloatTy::fixed(32)), "main()")
            .expect("a Float32 result must render");
        assert!(
            body.contains("from_f32(") && !body.contains("from_f64("),
            "a Float32 must render via `from_f32` on its own f32 (NOT the f64 promotion, ruling #7554), got:\n{body}"
        );
    }

    #[test]
    fn a_float64_renders_via_from_f64() {
        let mut db = db();
        let body = emit_result_doc(&mut db, &Ty::Float(FloatTy::f64()), "main()")
            .expect("a Float64 result must render");
        assert!(
            body.contains("from_f64(") && !body.contains("from_f32("),
            "a Float64 must render via `from_f64`, got:\n{body}"
        );
    }

    #[test]
    fn a_deferred_float_grounds_to_f64() {
        // A bare-literal float whose width never got constrained grounds to Float64 (`ground_width`
        // default) — so it takes the `from_f64` codec, not a "no width" panic.
        let mut db = db();
        let body = emit_result_doc(&mut db, &Ty::Float(FloatTy::deferred()), "main()")
            .expect("a deferred-width float grounds to Float64 and renders");
        assert!(
            body.contains("from_f64(") && !body.contains("from_f32("),
            "a deferred float grounds to Float64 → `from_f64`, got:\n{body}"
        );
    }

    #[test]
    fn a_float_render_handles_nan_and_inf_dispositions() {
        // The finite path uses the width codec above; NaN → `FloatNan`, ±inf → `FloatInf` — pin that the
        // arm emits BOTH non-finite branches (a canonical value_codec disposition, backend-parity).
        let mut db = db();
        let body = emit_result_doc(&mut db, &Ty::Float(FloatTy::f64()), "main()")
            .expect("a Float64 result must render");
        assert!(
            body.contains("FloatNan") && body.contains("FloatInf"),
            "the float arm must emit the NaN and ±inf non-finite dispositions, got:\n{body}"
        );
    }
}

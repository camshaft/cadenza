//! rcdzc-side VALUE-DOC emit — the operator seq-210 parser-elimination.
//!
//! Generates, per export, a Ty-GUIDED `pub fn __cdz_doc_<export>() -> String` body that builds the
//! self-describing `(: <value> <type>)` codec doc via `cadenza_ast::Builder` + `codec::encode` (the SAME
//! wire cdz-run's `value_codec` emits, decoded by the harness's canonical `render_binary` — render-ty #7424)
//! and returns the `CDZDOC:<hex>` marker string. The gate driver calls this (flag-gated `CDZ_VALUE_DOC`)
//! instead of cdz-rust-render's type-note-driven `cdz_render_at` string walk: the value walk moves HERE and
//! consults the `Ty` DIRECTLY (no sexpr note re-parse), which is what lets us delete `parse_head_type` /
//! `cdz_render_at` / `rust_call_arg`. A value tuple then renders `(tuple …)` (canonical), NOT `#tuple` —
//! closing op-seq-283 by construction.
//!
//! Node shapes (verified via `cdz convert -t debug`):
//!   `(: 42 Int64)`             → List[ Name ":", Int 42, Name "Int64" ]
//!   `(: (tuple 1 2) …)`        → the value is List[ Name "tuple", <e0>, <e1>… ]; the type List[ Name "Tuple", <T0>… ]
//!   `(: true Bool)`            → Bool leaf, type Name "Bool"
//!
//! WIP (built incrementally, per concierge): covers Int / Bool / Tuple. Record / Option / Result / List /
//! Sum / Set / Map / Float / String / Bytes / Qty are follow-up increments (each a `doc_value_node` +
//! `doc_type_node` arm). An uncovered shape DECLINES (never a miscompile) — the driver keeps `cdz_render_at`
//! for it until covered, so partial coverage is safe.

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
    match ty.strip_nominal_and_qty() {
        // An integer leaf → an `Int` atom (its runtime value is the i64-slot magnitude; `from_i64` is exact
        // for a signed Int64, the only width covered so far — an unsigned/narrow width is a later increment).
        Ty::Int(_) => {
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.atom_leaf(cadenza_ast::ast::Leaf::Int {{ value: cadenza_ast::ast::IntValue::from_i64(({val_expr}) as i64), radix: cadenza_ast::ast::Radix::Dec }});\n"
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
        // A tuple → `(tuple <e0> <e1> …)`: a `Name "tuple"` head then each element's node (walked at `.i`).
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"tuple\");\n"));
            let mut kids = vec![head];
            for (i, e) in elems.iter().enumerate() {
                kids.push(doc_value_node(db, e, &format!("({val_expr}).{i}"), out, ctr)?);
            }
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list(vec![{}]);\n", kids.join(", ")));
            Ok(v)
        }
        // A record → `(record (= <f0> <v0>) (= <f1> <v1>) …)`: a `Name "record"` head then, per field (in
        // BTreeMap = SORTED-key order, matching the emitted tuple's `.i`), a `List[FieldPair, Name <field>,
        // <value-node>]` (the `=` marker is a `Leaf::FieldPair`).
        Ty::Record(fields) => {
            let fields: Vec<(String, Ty)> =
                fields.iter().map(|(k, t)| (k.name.to_string(), t.clone())).collect();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"record\");\n"));
            let mut kids = vec![head];
            for (i, (fname, fty)) in fields.iter().enumerate() {
                let fp = fresh(ctr);
                out.push_str(&format!(
                    "    let {fp} = __b.atom_leaf(cadenza_ast::ast::Leaf::FieldPair);\n"
                ));
                let fnn = fresh(ctr);
                out.push_str(&format!("    let {fnn} = __b.name({fname:?});\n"));
                let fv = doc_value_node(db, fty, &format!("({val_expr}).{i}"), out, ctr)?;
                let pair = fresh(ctr);
                out.push_str(&format!("    let {pair} = __b.list(vec![{fp}, {fnn}, {fv}]);\n"));
                kids.push(pair);
            }
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list(vec![{}]);\n", kids.join(", ")));
            Ok(v)
        }
        _ => Err(Reject::decline(
            "value-doc: result shape not yet covered by the rust value-doc emit (WIP)",
        )),
    }
}

/// Emit `let`-bindings building the TYPE node for `ty`; return the final node's Rust variable. A LEAF type
/// (Int64/Bool/…) is a `Name` atom of its `render_name`; a Tuple is `(Tuple <T0> …)`.
fn doc_type_node(db: &mut Db, ty: &Ty, out: &mut String, ctr: &mut usize) -> Result<String, Reject> {
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
            out.push_str(&format!("    let {v} = __b.list(vec![{}]);\n", kids.join(", ")));
            Ok(v)
        }
        // A record TYPE → `(Record (: <f0> <T0>) (: <f1> <T1>) …)`: a `Name "Record"` head then, per field
        // (sorted-key order), the canonical ASCRIPTION node `List[Name ":", Name <field>, <type-node>]`
        // (matching `render_name` + the corpus `(Record (: x Int64) …)` form — NOT a bare `[field, ty]`).
        Ty::Record(fields) => {
            let fields: Vec<(String, Ty)> =
                fields.iter().map(|(k, t)| (k.name.to_string(), t.clone())).collect();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"Record\");\n"));
            let mut kids = vec![head];
            for (fname, fty) in fields.iter() {
                let colon = fresh(ctr);
                out.push_str(&format!("    let {colon} = __b.name(\":\");\n"));
                let fnn = fresh(ctr);
                out.push_str(&format!("    let {fnn} = __b.name({fname:?});\n"));
                let ft = doc_type_node(db, fty, out, ctr)?;
                let pair = fresh(ctr);
                out.push_str(&format!("    let {pair} = __b.list(vec![{colon}, {fnn}, {ft}]);\n"));
                kids.push(pair);
            }
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list(vec![{}]);\n", kids.join(", ")));
            Ok(v)
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

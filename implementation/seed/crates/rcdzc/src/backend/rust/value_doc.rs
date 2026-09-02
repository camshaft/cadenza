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
//! WIP (built incrementally, per concierge): covers Int / Bool / Float / String / Symbol / Bytes / Char /
//! Tuple / Record / List / Set / Map (incl. a DIRECT-float key/element via the `__CdzF` `.get()` unwrap) and
//! a bare-head SUM (Option / Result / a user sum) — nullary, single-payload, AND multi-field (flattened by
//! declared arity). Qty, a COMPOUND-float (tuple-with-float) key/element, plus the harder sum shapes
//! (qualified-head, recursive) are follow-up increments (each a `doc_value_node` + `doc_type_node` arm). An
//! uncovered shape DECLINES (never a miscompile) — the driver keeps `cdz_render_at` for it until covered, so
//! partial coverage is safe.

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
        // A string → a `Leaf::Str` (renders `"…"`, the escape codec in the printer). Rust value is a
        // `String`; `.as_str().into()` copies into the `Arc<str>` the leaf holds.
        Ty::String => {
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.atom_leaf(cadenza_ast::ast::Leaf::Str(({val_expr}).as_str().into()));\n"
            ));
            Ok(v)
        }
        // A symbol → a `Leaf::Sym` (renders `#\"…\"`) — the guest-result `Symbol` disambiguation cdz-run's
        // typed render makes (`Leaf::Sym` vs `Leaf::Str`), here driven DIRECTLY off the `Ty`. Rust rep is a
        // `String` (Symbol erases to String), same `.as_str().into()` copy.
        Ty::Symbol => {
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.atom_leaf(cadenza_ast::ast::Leaf::Sym(({val_expr}).as_str().into()));\n"
            ));
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
        // A tuple → `(tuple <e0> <e1> …)`: a `Name "tuple"` head then each element's node (walked at `.i`).
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            let head = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"tuple\");\n"));
            let mut kids = vec![head];
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
                "    let {v} = __b.list(vec![{}]);\n",
                kids.join(", ")
            ));
            Ok(v)
        }
        // A record → `(record (= <f0> <v0>) (= <f1> <v1>) …)`: a `Name "record"` head then, per field (in
        // BTreeMap = SORTED-key order, matching the emitted tuple's `.i`), a `List[FieldPair, Name <field>,
        // <value-node>]` (the `=` marker is a `Leaf::FieldPair`).
        Ty::Record(fields) => {
            let fields: Vec<(String, Ty)> = fields
                .iter()
                .map(|(k, t)| (k.name.to_string(), t.clone()))
                .collect();
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
                out.push_str(&format!(
                    "    let {pair} = __b.list(vec![{fp}, {fnn}, {fv}]);\n"
                ));
                kids.push(pair);
            }
            let v = fresh(ctr);
            out.push_str(&format!(
                "    let {v} = __b.list(vec![{}]);\n",
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
            let head = fresh(ctr);
            let kids = fresh(ctr);
            let iter = fresh(ctr);
            out.push_str(&format!("    let {head} = __b.name(\"list\");\n"));
            out.push_str(&format!("    let mut {kids} = vec![{head}];\n"));
            let mut body = String::new();
            let enode = doc_value_node(db, &elem, &iter, &mut body, ctr)?;
            out.push_str(&format!(
                "    for {iter} in ({val_expr}) {{\n{body}        {kids}.push({enode});\n    }}\n"
            ));
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list({kids});\n"));
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
            let head = fresh(ctr);
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
            out.push_str(&format!("    let {head} = __b.name(\"set\");\n"));
            out.push_str(&format!("    let mut {kids} = vec![{head}];\n"));
            let mut body = String::new();
            let enode = doc_value_node(db, &elem, &elem_val, &mut body, ctr)?;
            out.push_str(&format!(
                "    for {iter} in ({val_expr}) {{\n{body}        {kids}.push({enode});\n    }}\n"
            ));
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list({kids});\n"));
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
            let head = fresh(ctr);
            let kids = fresh(ctr);
            let kv = fresh(ctr);
            let fp = fresh(ctr);
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
            out.push_str(&format!("    let {head} = __b.name(\"map\");\n"));
            out.push_str(&format!("    let mut {kids} = vec![{head}];\n"));
            let mut body = String::new();
            let knode = doc_value_node(db, &kty, &key_val, &mut body, ctr)?;
            let vnode = doc_value_node(db, &vty, &format!("{kv}.1"), &mut body, ctr)?;
            out.push_str(&format!(
                "    for {kv} in ({val_expr}) {{\n{body}        let {fp} = __b.atom_leaf(cadenza_ast::ast::Leaf::FieldPair);\n        let {entry} = __b.list(vec![{fp}, {knode}, {vnode}]);\n        {kids}.push({entry});\n    }}\n"
            ));
            let v = fresh(ctr);
            out.push_str(&format!("    let {v} = __b.list({kids});\n"));
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
            // A qualified-head sum needs the `(. Type Variant)` head form — not covered yet.
            if crate::lower::sum_needs_qualified_heads(db, decl_occ) {
                return Err(Reject::decline(
                    "value-doc: qualified-head sum not covered by the rust value-doc emit",
                ));
            }
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
                let path = super::expr::sum_variant_path_of_ty(db, &sum_ty, disc)?;
                match super::expr::variant_payload_ty(db, &sum_ty, disc) {
                    // Nullary → `(<Variant> unit)`: head atom then the canonical `unit` name atom.
                    None => {
                        arms.push_str(&format!(
                            "        {path} => {{ let __hv = __b.name({vname:?}); let __u = __b.name(\"unit\"); __b.list(vec![__hv, __u]) }}\n"
                        ));
                    }
                    Some(payload_ty) => {
                        // A recursive variant boxes its payload (needs a helper to terminate) — decline.
                        if super::enums::variant_is_recursive(db, &sum_ty, disc) {
                            return Err(Reject::decline(
                                "value-doc: recursive sum not covered by the rust value-doc emit",
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
                                "        {path}({pbind}) => {{\n{armbuf}            let __hv = __b.name({vname:?}); __b.list(vec![__hv, {}])\n        }}\n",
                                enodes.join(", ")
                            ));
                        } else {
                            // Single payload → `(<Variant> <payload-node>)`. The arm binds the payload by value
                            // (moving it out of the owned enum) and walks it into a per-arm buffer.
                            let pnode = doc_value_node(db, &payload_ty, &pbind, &mut armbuf, ctr)?;
                            arms.push_str(&format!(
                                "        {path}({pbind}) => {{\n{armbuf}            let __hv = __b.name({vname:?}); __b.list(vec![__hv, {pnode}])\n        }}\n"
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
            let fields: Vec<(String, Ty)> = fields
                .iter()
                .map(|(k, t)| (k.name.to_string(), t.clone()))
                .collect();
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
                out.push_str(&format!(
                    "    let {pair} = __b.list(vec![{colon}, {fnn}, {ft}]);\n"
                ));
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

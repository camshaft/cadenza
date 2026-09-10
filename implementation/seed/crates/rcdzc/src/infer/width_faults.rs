//! Integer/float WIDTH-FAULT diagnostics — the CDZ0302 literal-out-of-width / ill-formed-width /
//! unbound-width / out-of-range reject + fix + message builders (`literal_width_fault`,
//! `ill_formed_int_width_*`, `nested_*_width*`, `width_fault_against_ty`, `int_out_of_range_*`,
//! `int_width_range`, ...). Extracted verbatim from `infer.rs` to keep it under
//! `xtask_support::MAX_SOURCE_BYTES` (512 KiB); pure code move, behavior-neutral. `use super::*` brings
//! the parent `infer` module items (Db, Ty, StructId, Reject, NameCtx, type_of, ...) into scope.
//! Private fns are `pub(super)` so the parent's `use width_faults::*;` re-imports them (call sites in
//! infer.rs unchanged); the cross-module `pub(crate)` message/fix fns keep that visibility and are
//! re-exported by the parent so `crate::infer::<fn>` paths still resolve.
use super::*;

/// The CDZ0302 fault if the value at `value` is an integer LITERAL that does not fit the NARROW integer
/// type the type-expression `ty_expr` denotes, else `None`. The literal analogue of "Annotations
/// Constrain" (numeric-model.md §A Bare Integer Literal Is Grounded By Its Annotation, Subject To A Range
/// Check): a bare literal has no intrinsic width, so an annotation FIXES its type subject only to a range
/// check — a literal outside the width is REJECTED, never truncated. Shared by the value annotation
/// `(: value T)` and the annotated LET BINDER `((: name T) value)` so both range-check identically. Only
/// a literal + a fixed-width integer type can fault here; a non-literal value's agreement is a separate
/// unify (CDZ0203), and a deferred/Var width imposes no bound.
pub(super) fn literal_width_fault(
    db: &mut Db,
    value: StructId,
    ty_expr: StructId,
) -> Option<Reject> {
    let annot_ty = crate::eval::typeval_of(db, ty_expr)?;
    // A FLOAT literal annotated to a NARROWER float width it cannot hold — `(: 1.0e300 Float32)`. The
    // value is finite as `Float64` (the literal's default) but overflows `Float32` to `±inf`, a value with
    // no written form (numeric-model.md §A Floating-Point Literal That Denotes No Representable Value Is
    // Malformed) — the float analogue of an out-of-range integer literal (CDZ0302). Only `Float32` is
    // narrow enough to catch here (a `Float64` overflow is a malformed bare literal, caught earlier); a
    // non-literal value imposes no compile-time bound. `is_finite_f64` guards the default `Float64`; this
    // is its `Float32` sibling, promised by that method's own doc-comment ("`(: 1e40 Float32)` … caught at
    // the annotation").
    if let Ty::Float(ft) = &annot_ty
        && ft.ground_width() == 32
        && let crate::ast::Struct::Atom(lid) = db.ast.get(value)
        && let crate::ast::Leaf::Float(dec) = db.ast.leaf(*lid).clone()
        && !dec.fits_f32()
    {
        // The mechanical repair: retype the annotation to `Float64` — the wider float holds the value (a
        // `Float64` is the literal's own default, so it is finite there), the float twin of the integer
        // width-widen / BigInt fix. Rewrite the whole `ty_expr` so either spelling — a bare `Float32` or a
        // `(Float 32)` compound — becomes the bare `Float64`. Heuristic (the author may instead have meant a
        // smaller literal), but the retype clears the overflow in one shot and type-checks.
        return Some(
            Reject::coded(
                Code::IntOutOfRange,
                "float literal does not fit the annotated type Float32 (it overflows the Float32 \
                 range to infinity — the largest finite Float32 is about 3.4e38)",
            )
            .at(ty_expr)
            .with_fix(Fix::replace_heuristic(ty_expr, "Float64")),
        );
    }
    let Ty::Int(it) = &annot_ty else { return None };
    let crate::ty::Width::Fixed(w) = it.width else {
        return None;
    };
    // The SENTINEL width 0 (a `reduce_ctor` clamp of an out-of-range/malformed width like `(UInt 65)`) is
    // an ill-formed TYPE, not a literal-range problem — reporting "literal does not fit UInt0" misleads
    // (it names the clamped sentinel and blames the literal). The ill-formed-width check
    // (`out_of_range_int_width`, applied at the param/value annotation) reports the REAL fault naming the
    // written width, so skip the literal-fit report here and let that fire.
    if w == 0 {
        return None;
    }
    // A value that ALREADY has a distinct numeric type — an EXPLICITLY-SUFFIXED literal `999N`
    // (`Ty::BigInt`) / `1R` (`Ty::Rational`) — is NOT a bare literal being grounded by the annotation: it
    // carries its own type, so `(: 999N Int64)` (or passing `999N` to an `Int64` parameter) is a genuine
    // type MISMATCH (BigInt ≠ Int64), reported by the CDZ0203 unify. The width fit-check sees through the
    // suffix's `(: 999 BigInt)` desugar to the inner `Resolved::Int` and would ALSO fire CDZ0302 ("literal
    // does not fit Int64") — double-reporting the same slip with a misleading second framing (a BigInt is
    // the wrong TYPE, not an out-of-range Int64 literal). Skip it; the mismatch path is the correct, sole
    // diagnostic. A BARE literal (no suffix) still range-checks: it types as the `Int64` default, so it is
    // a grounding, exactly as before.
    if matches!(type_of(db, value), Ty::BigInt | Ty::Rational) {
        return None;
    }
    // The bound value's CONSTANT integer value, if it has one: a bare literal `200`, OR a value that
    // FOLDS to a constant (`(+ 100 100)` → 200) — the same computed constants the value annotation
    // `(: (+ 100 100) Int8)` range-checks. `core_of` performs the fold; a runtime value (a param, a call
    // result) does not fold to `ConstInt` and imposes no compile-time bound (it is checked by its own
    // type / traps at run time).
    let v = match resolved_of(db, value) {
        Resolved::Int(v) => v,
        _ => match crate::lower::core_of(db, value) {
            crate::core::Core::ConstInt(v) => v,
            _ => return None,
        },
    };
    if !v.fits_width(it.ground_signed(), w) {
        return Some(int_out_of_range_reject(
            &annot_ty,
            it.ground_signed(),
            w,
            &v,
            ty_expr,
            &db.name_ctx(),
        ));
    }
    None
}

/// The CDZ0302 message for an ILL-FORMED integer width (`crate::eval::IntWidthFault`), shared by the value-
/// and parameter-annotation checks so both phrase it identically. An OVER-CEILING/zero width names the
/// WRITTEN width (`(UInt 65)` → "`UInt65` is not a valid integer type …"); a MALFORMED (negative /
/// non-natural) width has NO width number to name, so it states the constraint the width violated (a width
/// must be a compile-time NATURAL in 1..=64) — the case that used to slip past `cdz check` entirely.
pub(crate) fn ill_formed_int_width_message(fault: &crate::eval::IntWidthFault) -> String {
    match *fault {
        crate::eval::IntWidthFault::OverCeiling { signed, width } => format!(
            "`{}{width}` is not a valid integer type: a width must be in 1..=64 (a fixed-size integer \
             wider than 64 bits is reserved to the big-integer layer, and 0 is not a width)",
            if signed { "Int" } else { "UInt" }
        ),
        crate::eval::IntWidthFault::Malformed { signed } => format!(
            "an integer type's width must be a compile-time natural number in 1..=64 — this `{}` type's \
             width is not a natural number (a negative, fractional, or non-numeric width is not a width)",
            if signed { "Int" } else { "UInt" }
        ),
    }
}

/// The CDZ0302 REPAIR for an ill-formed integer width at `pos` — the actionable half of
/// [`ill_formed_int_width_message`] (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
/// A Fix). Only the OVER-CEILING case (`(UInt 65)`, `(Int 128)` — a fixed width strictly greater than 64)
/// has a single confident target: the message itself says such a width is "reserved to the big-integer
/// layer", so the repair is the unbounded `BigInt`, which holds any magnitude — the type-level twin of the
/// literal-range fix's `BigInt` continuation (`int_out_of_range_reject`). Every other ill-formed width has
/// NO single correct target — a `0` width or a `Malformed` (negative/non-numeric) width could mean the
/// author dropped or mistyped the number, so guessing one would be a false suggestion (worse than none, per
/// the `suggest` module) — those carry the message alone. Heuristic: the author may instead have meant a
/// specific in-range width, but `BigInt` clears the fault in one shot and always type-checks.
pub(crate) fn ill_formed_int_width_fix(
    fault: &crate::eval::IntWidthFault,
    pos: StructId,
) -> Option<Fix> {
    match *fault {
        crate::eval::IntWidthFault::OverCeiling { width, .. } if width > 64 => {
            Some(Fix::replace_heuristic(pos, "BigInt"))
        }
        _ => None,
    }
}

/// The FIRST ill-formed integer width ANYWHERE in the type-expression `ty_expr` — the top-level type OR a
/// NESTED type-argument position (`(Option (UInt 65))`, `(List (Int -8))`, `(Tuple Int8 (Int -8))`,
/// `(Map (Int -8) v)`). A top-level `int_width_fault` catches `(: 5 (Int -8))`, but a width nested in a
/// compound annotation reduces to a valid-looking `Ty` (the ctor clamps the bad width to sentinel `Int0`),
/// so the top-level check + `typeval_of` both wave it through — it slipped past `cdz check` while the emit
/// path caught it (a check-vs-emit gap). Recurse every type-ctor ARGUMENT position (the tail elements of a
/// `(head arg…)` type form; the head `List`/`Option`/`Map`/`->`/`Int`/… is the ctor, not a nested type)
/// and return the first arg that is itself an ill-formed-width integer type. Reuses `eval::int_width_fault`
/// per position, so the message + code match the top-level check exactly. A record `(Record (f T)…)` field
/// TYPE is a tail element of its `(f T)` pair — descended too (skipping the label). Returns `(pos, fault)`.
pub(super) fn nested_ill_formed_int_width(
    db: &mut Db,
    ty_expr: StructId,
) -> Option<(StructId, crate::eval::IntWidthFault)> {
    // This position itself — an `(Int W)`/`(UInt W)` with an ill-formed width.
    if let Some(fault) = crate::eval::int_width_fault(db, ty_expr) {
        return Some((ty_expr, fault));
    }
    // Otherwise descend its type-argument positions. A type form is `(head arg…)`; the head is the ctor
    // (a name/prim), never a nested type, so skip child 0. A `(name Type)` record-field pair's TYPE is its
    // second child (skip the label at child 0 via the same skip-first rule, recursively).
    let crate::ast::Struct::List(children) = db.ast.get(ty_expr) else {
        return None;
    };
    for &child in children.clone().iter().skip(1) {
        if let Some(found) = nested_ill_formed_int_width(db, child) {
            return Some(found);
        }
    }
    None
}

/// The `(Float W)` companion of [`nested_ill_formed_int_width`]: the position of an ill-formed float width
/// (outside the admitted IEEE set `{32,64}`, or non-natural) at `ty_expr` OR nested in one of its
/// type-argument positions (`(List (Float 8))`, `(Option (Float 16))`, a record field). `None` if every
/// float width in the type expression is admitted. Same skip-first descent as the integer helper (child 0
/// of a `(head arg…)` form is the ctor, never a nested type). Every ill-formed float width shares one
/// message, so this returns only the POSITION (to anchor the reject); the message is a constant.
pub(super) fn nested_ill_formed_float_width(db: &mut Db, ty_expr: StructId) -> Option<StructId> {
    if crate::eval::is_ill_formed_float_width(db, ty_expr) {
        return Some(ty_expr);
    }
    let crate::ast::Struct::List(children) = db.ast.get(ty_expr) else {
        return None;
    };
    for &child in children.clone().iter().skip(1) {
        if let Some(found) = nested_ill_formed_float_width(db, child) {
            return Some(found);
        }
    }
    None
}

pub(crate) const FLOAT_WIDTH_MESSAGE: &str =
    "a floating-point width must be one of the admitted IEEE widths (32 or 64)";

/// The UNBOUND-WIDTH companion of [`nested_ill_formed_int_width`]/[`nested_ill_formed_float_width`]: the
/// position of a width constructor `(Int W)`/`(UInt W)`/`(Float W)` whose width argument `W` is an UNBOUND
/// NAME (`(: a (Int hello))`), at `ty_expr` OR nested in one of its type-argument positions (`(List (Int
/// hello))`). `None` when no width position holds an unbound name. An unbound name in a WIDTH slot is not a
/// type (so the nested-type-var walk skips it) and reads as a non-constant width (so `int_width_fault`
/// waves it through as if it were a bound width variable), which let it slip past `cdz check` silently —
/// this closes that gap. Same skip-first descent as the sibling width walkers (child 0 of a `(head arg…)`
/// form is the ctor, never a nested type). A BOUND width variable (`(Int a)` with `a` a `Type`/width param)
/// is valid and returns `None` — `eval::unbound_width_arg` distinguishes the two by the arg's resolution.
pub(super) fn nested_unbound_width(
    db: &mut Db,
    ty_expr: StructId,
) -> Option<(StructId, &'static str)> {
    if let Some(found) = crate::eval::unbound_width_arg(db, ty_expr) {
        return Some(found);
    }
    let crate::ast::Struct::List(children) = db.ast.get(ty_expr) else {
        return None;
    };
    for &child in children.clone().iter().skip(1) {
        if let Some(found) = nested_unbound_width(db, child) {
            return Some(found);
        }
    }
    None
}

/// The CDZ0101 message for an UNBOUND NAME in a width position — `(: a (Int hello))` / `(Float hi)`. Names
/// the specific mistake (a width is a compile-time integer literal, not a name) and the repair: write the
/// literal, or the sized type directly. `example` is a ctor-appropriate sized type (`Int64`/`UInt64`/
/// `Float64`), so a `Float` width names a float example rather than the misleading `Int64`. The
/// width-position analogue of the lowercase-type-var guidance, but a width is not a type, so the fix is a
/// literal like `64`, not "leave the parameter unannotated".
pub(crate) fn unbound_width_message(name: &str, example: &str) -> String {
    format!(
        "unbound name `{name}` — a width must be a compile-time integer literal like `64`, not a name \
         (write the width literal, or the sized type `{example}` directly)"
    )
}

/// The CDZ0302 REPAIR for an ill-formed FLOAT width at `pos` — the actionable half of
/// [`FLOAT_WIDTH_MESSAGE`] (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix),
/// the float twin of [`ill_formed_int_width_fix`]. A CONCRETE natural width outside the admitted IEEE set
/// `{32, 64}` snaps to the nearest admitted width: a below-32 width (`(Float 8)`, `(Float 16)`) retypes to
/// `Float32`, and any wider non-admitted width (`(Float 48)`, `(Float 128)`) retypes to `Float64` — the
/// widest admitted precision (32 itself is admitted, so it never reaches here). A MALFORMED (negative /
/// non-numeric) width has NO width
/// number and no single confident target, so it carries the message alone (a false suggestion is worse
/// than none). Heuristic: the author may have meant a specific admitted width, but the snap clears the
/// fault in one shot and type-checks. `db` reads the concrete width off the annotation via
/// `eval::out_of_set_float_width`.
pub(crate) fn ill_formed_float_width_fix(db: &mut Db, pos: StructId) -> Option<Fix> {
    let w = crate::eval::out_of_set_float_width(db, pos)?;
    // A ZERO width (`(Float 0)`) reads as a dropped/mistyped number with no confident target — like the
    // integer twin, it stays message-only (a false suggestion is worse than none).
    if w == 0 {
        return None;
    }
    let target = if w < 32 { "Float32" } else { "Float64" };
    Some(Fix::replace_heuristic(pos, target))
}

/// The RUNTIME-WIDTH companion of [`nested_ill_formed_int_width`]/[`nested_ill_formed_float_width`]: the
/// position of a width-indexed numeric type `(Int n)`/`(UInt n)`/`(Float n)` whose width is RUNTIME DATA
/// (a parameter/ref) at `ty_expr` OR nested in one of its type-argument positions (`(List (Int n))`,
/// `(Option (Float n))`, a record field). `None` if no width in the type expression is runtime data. Same
/// skip-first descent as the ill-formed-width helpers. `is_runtime_width_type` (eval.rs) checks only the
/// TOP-LEVEL ctor, so a runtime width NESTED in a compound slipped past `cdz check` (rc=0) AND compiled —
/// a runtime value determining a type, which the type system forbids (numeric-model.md §An Integer/
/// Floating-Point Type Is Indexed By A Compile-Time Width). This closes that nested gap.
pub(crate) fn nested_runtime_width_type(db: &mut Db, ty_expr: StructId) -> Option<StructId> {
    if crate::eval::is_runtime_width_type(db, ty_expr) {
        return Some(ty_expr);
    }
    let crate::ast::Struct::List(children) = db.ast.get(ty_expr) else {
        return None;
    };
    for &child in children.clone().iter().skip(1) {
        if let Some(found) = nested_runtime_width_type(db, child) {
            return Some(found);
        }
    }
    None
}

/// The CDZ0302 out-of-range range-check EXTENDED through a COMPOUND value's payload/elements. The scalar
/// `literal_width_fault` above catches a top-level `(: 999 Int8)`, but a NESTED narrow-width literal — the
/// payload of `(: (Some 999) (Option Int8))`, an element of `(: (tuple 999) (Tuple Int8))`, a list element
/// of `(: (list 999) (List Int8))` — slipped through: the annotation's `Int8` propagates into the outer
/// value's type but the literal itself stays a deferred `Int64` (its own `type_of` reads `Int64`), so the
/// scalar fit-check never fires and `cdz check` ACCEPTED a value the declared type cannot hold (the emit
/// path DID catch it — a check-vs-emit gap). This walks the ANNOTATION's expected `Ty` (from `typeval_of`)
/// paired with the value's payload/element NODES, so each nested literal is fit-checked against the width
/// the annotation gives it — the range-check analogue of the annotation-descends-into-compound-payload
/// type check. Descends Sum (single-payload variant), Tuple, and List; a non-compound / mismatched shape
/// (reported by the ordinary type check) or a runtime (non-literal) payload adds nothing. Returns the FIRST
/// out-of-range nested literal's reject (anchored at that literal, so `cdz fix` targets it).
pub(super) fn nested_literal_width_faults(
    db: &mut Db,
    value: StructId,
    ty_expr: StructId,
) -> Option<Reject> {
    let expected = crate::eval::typeval_of(db, ty_expr)?;
    nested_literal_width_faults_against(db, value, &expected)
}

/// The `&Ty`-typed core of [`nested_literal_width_faults`] — takes the already-resolved expected `Ty`
/// instead of an annotation NODE, so a collection builder-chain arm (a `Map.insert`/`Set.insert` operand)
/// can RECURSE into the operand collection against the SAME `Ty::Map`/`Ty::Set` without a type-expr node
/// to re-resolve. The public entry point resolves `ty_expr` once and delegates here.
pub(super) fn nested_literal_width_faults_against(
    db: &mut Db,
    value: StructId,
    expected: &Ty,
) -> Option<Reject> {
    match expected {
        // A NARROW-INT annotation on a non-literal that `literal_width_fault` could not check directly — a
        // runtime `(if c 10000 0)` / `(match …)` annotated `(: … UInt8)`: the value is neither a
        // `Resolved::Int` nor a folding constant, so the scalar check above found nothing, yet each live
        // branch of the conditional carries the annotation's narrow width. Route it through
        // `width_fault_against_ty` (which descends a runtime `if` into both branches + range-checks the
        // narrow int). A bare out-of-range literal in a branch (`(: (if c 10000 0) UInt8)`) then rejects at
        // `check` as the emit path already does — closing the same check-vs-emit gap the compound arms close.
        Ty::Int(_) => width_fault_against_ty(db, value, expected),
        // A narrow `Float32` annotation on a non-literal `literal_width_fault` could not check directly — a
        // runtime `(if c 1.0e300 0.0)` / `(match …)` annotated `(: … Float32)`. Route it through
        // `width_fault_against_ty` (which descends the conditional's branches + applies the Float32-overflow
        // check to each branch literal). Without this, an overflowing branch literal slipped `cdz check`
        // while the emit path produced an INVALID module — the float sibling of the narrow-int gap.
        Ty::Float(_) => width_fault_against_ty(db, value, expected),
        // A single-payload variant `(Some 999)` : `(Option Int8)` — drill the payload arg against the
        // payload type at this sum's instantiation. (A multi-payload variant boxes its payloads as a tuple;
        // its single ctor arg is that tuple, handled by the Tuple arm once drilled — kept simple here to the
        // single-payload numeric case, the common one.)
        Ty::Sum { .. } => {
            let Resolved::Apply { head, args } = resolved_of(db, value) else {
                return None;
            };
            if crate::eval::variant_disc_of(db, head).is_none() || args.len() != 1 {
                return None;
            }
            let want = payload_ty_at_instantiation(db, head, expected)?;
            width_fault_against_ty(db, args[0], &want)
        }
        // A user-declared NOMINAL type — a newtype `(type W (W Int8))` (`inner` = the payload type) or a
        // multi-payload `(type P (P Int8 Int64))` (`inner` = a `Tuple` of the payloads). Its constructor
        // `(W 999)` / `(P 999 5)` resolves as `Apply(ctor, [payload args])`; without this arm a bare
        // over-range payload literal escaped the fit-check → wasm SILENTLY TRUNCATED it (999 → -25; rust
        // E0308) — the nominal face of the Option/Record/Map cases. Descend each ctor arg against the
        // matching `inner` type: a Tuple `inner` zips positionally (multi-payload), else the single arg
        // against `inner`. (A user MULTI-VARIANT sum is `Ty::Sum` and takes the Sum arm above.)
        Ty::Nominal { inner, .. } => {
            let Resolved::Apply { head, args } = resolved_of(db, value) else {
                return None;
            };
            crate::eval::variant_disc_of(db, head)?;
            match &**inner {
                Ty::Tuple(elem_tys) => elem_tys
                    .iter()
                    .zip(args.iter())
                    .find_map(|(t, &a)| width_fault_against_ty(db, a, t)),
                single => args
                    .first()
                    .and_then(|&a| width_fault_against_ty(db, a, single)),
            }
        }
        // A tuple `(tuple 999 …)` : `(Tuple Int8 …)` — each element against its element type.
        Ty::Tuple(elem_tys) => {
            let elems = positional_value_nodes(db, value, crate::resolved::Prim::TupleNew)?;
            elem_tys
                .iter()
                .zip(elems.iter())
                .find_map(|(t, &e)| width_fault_against_ty(db, e, t))
        }
        // A list `(list 999 …)` : `(List Int8)` — each element against the element type (homogeneous).
        Ty::List(elem_ty) => {
            let elems = positional_value_nodes(db, value, crate::resolved::Prim::ListNew)?;
            let elem_ty = (**elem_ty).clone();
            elems
                .iter()
                .find_map(|&e| width_fault_against_ty(db, e, &elem_ty))
        }
        // A record `(record (x 999) …)` : `(Record (: x Int8) …)` — each field value against its DECLARED
        // field type. Without this a bare `999` in an `Int8` field escaped the fit-check → wasm silently
        // TRUNCATED it (999 → -25) while rust rejected E0308 (a backend-divergent SILENT MISCOMPILE, the
        // record face of the Option/Tuple payload cases). The record value's fields are keyed by symbol
        // (`Resolved::Record`); pair each declared field type with its value node by name.
        Ty::Record(field_tys) => {
            // A record literal resolves either as a folded `Resolved::Record` OR (the common case) an
            // `Apply(RecordNew, [(key value)…])` name-alias — read the field value nodes by symbol from
            // whichever shape, like `positional_value_nodes` unifies the Tuple/List Apply cases.
            let fields = match resolved_of(db, value) {
                Resolved::Record { fields } => (*fields).clone(),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::RecordNew) =>
                {
                    crate::resolve::read_record_fields(db, &args).ok()?
                }
                _ => return None,
            };
            field_tys.iter().find_map(|(sym, t)| {
                fields
                    .get(sym)
                    .and_then(|&v| width_fault_against_ty(db, v, t))
            })
        }
        // A map `(map (k v) …)` : `(Map Int8 Int64)` — each KEY literal against the key type + each VALUE
        // literal against the value type. Both positions escaped the fit-check: `(: (map (1 999)) (Map
        // Int64 Int8))` silently TRUNCATED the value (999 → -25 on lookup), and `(: (map (999 1)) (Map Int8
        // Int64))` accepted an out-of-range key. Descend the entry key/value nodes (paired, in order) each
        // against its declared side. The map value's entries are `(key value)` occurrence pairs.
        Ty::Map(key_ty, val_ty) => {
            let (key_ty, val_ty) = ((**key_ty).clone(), (**val_ty).clone());
            // A map literal resolves as a folded `Resolved::Map { entries }` OR an `Apply(MapNew, [(k v)…])`
            // name-alias; read the `(key value)` node pairs from whichever shape (each Apply arg is a
            // two-element `(key value)` list, as `resolve_map` reads them). A map BUILT by a `Map.insert`
            // chain (`Apply(MapInsert, [map, key, val])`, bottoming at `Map.empty`) is NOT a literal — its
            // key/value literals escaped this check entirely, so an out-of-range literal fed through
            // `(Map.insert Map.empty k 200) : (Map Int64 Int8)` compiled clean AND ran to a truncated -56
            // (a silent miscompile — the builder-chain face of the map-literal case). Walk the insert chain
            // too: range-check this insert's key/value args, then recurse into the operand map.
            match resolved_of(db, value) {
                Resolved::Map { entries } => entries.to_vec().iter().find_map(|&(k, v)| {
                    width_fault_against_ty(db, k, &key_ty)
                        .or_else(|| width_fault_against_ty(db, v, &val_ty))
                }),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::MapNew) =>
                {
                    // Each `MapNew` arg is a map ENTRY. Read `(key, value)` from the native `(= k v)`
                    // FieldPair leaf (M2, what the reader emits for a `#map`/`(map (= k v))` entry), the
                    // transitional name-head `(= k v)`, OR the legacy 2-element `(k v)` pair. Before this
                    // only the 2-element pair was read, so a native FieldPair entry `(map (= 1 999))` fed
                    // through the name-alias `MapNew` path was skipped → its out-of-range value/key literal
                    // escaped CDZ0302 and silently truncated (the map face of the native-leaf descent gap).
                    args.iter()
                        .filter_map(|&entry| {
                            db.ast
                                .field_pair_parts(entry)
                                .or_else(|| db.ast.field_pair(entry))
                                .or_else(|| match db.ast.get(entry) {
                                    crate::ast::Struct::List(items) if items.len() == 2 => {
                                        Some((items[0], items[1]))
                                    }
                                    _ => None,
                                })
                        })
                        .collect::<Vec<_>>()
                        .iter()
                        .find_map(|&(k, v)| {
                            width_fault_against_ty(db, k, &key_ty)
                                .or_else(|| width_fault_against_ty(db, v, &val_ty))
                        })
                }
                // `(Map.insert <map> <key> <val>)` — check this entry's key + value, then recurse the
                // operand map (`Map.empty` bottoms out as a non-insert with no literal → None).
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::MapInsert)
                        && args.len() == 3 =>
                {
                    width_fault_against_ty(db, args[1], &key_ty)
                        .or_else(|| width_fault_against_ty(db, args[2], &val_ty))
                        .or_else(|| nested_literal_width_faults_against(db, args[0], expected))
                }
                _ => None,
            }
        }
        // A set BUILT by `Set.of (list …)` or a `Set.insert` chain : `(Set Int8)` — each element literal
        // against the element type. Previously there was NO `Ty::Set` arm at all, so an out-of-range set
        // element escaped the fit-check on both `check` and `emit` (the set face of the map builder-chain
        // silent miscompile). `Set.of list` (single list arg) descends the list elements; `Set.insert set
        // elem` checks the inserted element then recurses the operand set (`Set.empty` bottoms out).
        Ty::Set(elem_ty) => {
            let elem_ty = (**elem_ty).clone();
            match resolved_of(db, value) {
                // A native `#set(e…)` / `("set" e…)` LITERAL resolves to `Resolved::Set { elems }` (the
                // first-class set ctor). Before this arm it fell through to `_ => None`, so an out-of-range
                // set-literal element `(: #set(200) (Set Int8))` escaped CDZ0302 and silently truncated
                // (the set-literal face of the native-leaf descent gap; the `Set.of`/`Set.insert` builder
                // chains below were already covered).
                Resolved::Set { elems } => elems
                    .to_vec()
                    .iter()
                    .find_map(|&e| width_fault_against_ty(db, e, &elem_ty)),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::SetOf)
                        && args.len() == 1 =>
                {
                    positional_value_nodes(db, args[0], crate::resolved::Prim::ListNew)?
                        .iter()
                        .find_map(|&e| width_fault_against_ty(db, e, &elem_ty))
                }
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::SetInsert)
                        && args.len() == 2 =>
                {
                    width_fault_against_ty(db, args[1], &elem_ty)
                        .or_else(|| nested_literal_width_faults_against(db, args[0], expected))
                }
                _ => None,
            }
        }
        // A quantity `(Qty.of 300 kilometer)` : `(Qty UInt8 meter)` — drill the MAGNITUDE against the
        // annotation's INNER numeric type. A quantity annotation checks the dimension (not the scale, so
        // km may be annotated at meter), but it STILL grounds + range-checks the inner numeric type exactly
        // as a bare `(: 300 UInt8)` does — otherwise a quantity-wrapped literal slips its width entirely
        // (the annotation's `Ty::Qty` arm in `type_of` keeps the expression's own type to avoid the scale
        // rebrand, so the inner width never grounds/checks; this restores the check at the same choke point
        // the compound-payload cases use). The magnitude is the `Qty.of` value occurrence; range-check it
        // against the annotation's `inner`. This covers both a same-unit and a same-dimension different-
        // scale annotation uniformly. `Unit.in`'s bare-number result is not a `Ty::Qty` and is unaffected.
        Ty::Qty { inner, .. } => {
            let magnitude = crate::eval::qty_value_occ(db, value)?;
            width_fault_against_ty(db, magnitude, inner)
        }
        _ => None,
    }
}

/// Range-check the value node `value` (a literal, a folded constant, or a compound to recurse into)
/// against the EXPECTED type `want` — the `Ty`-typed core of the nested width check. A narrow-integer
/// `want` fit-checks a constant `value` (the same fold `literal_width_fault` runs); any other `want`
/// recurses through the compound if `value` is one. Returns the first out-of-range literal's reject.
pub(super) fn width_fault_against_ty(db: &mut Db, value: StructId, want: &Ty) -> Option<Reject> {
    // A RUNTIME `(Option.expect s "…")` / `(Result.expect …)` in a narrow-width context: `expect` PROJECTS
    // the sum's payload, so the annotation's `want` is the payload type — descend into the sum argument
    // against `Option<want>` (the payload arg substituted to `want`). Without this, `(: (Option.expect (if c
    // (Some 10000) None) "x") UInt8)` was a SILENT MISCOMPILE: a CONSTANT `(Some 10000)` FOLDS (`expect`
    // reduces to the payload `10000`, caught by the scalar check below), but a RUNTIME sum (here an `if`
    // returning `Some`) does NOT fold → the value stays a runtime `SumExpect` call, no literal to check, and
    // EMIT truncated `10000` to `16` (a `wrap` on the projected payload) with NO diagnostic on either side.
    // Rebuilding the sum's type with `want` as its payload arg + descending routes the `if`-branch `(Some
    // 10000)` through the Sum arm of `nested_width_fault_by_ty`, which range-checks `10000` against `want`.
    if let Resolved::Apply { head, args } = resolved_of(db, value)
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::SumExpect)
        && let Some(&sum_arg) = args.first()
        && let Ty::Sum {
            decl,
            args: sum_args,
        } = type_of(db, sum_arg)
        && !sum_args.is_empty()
    {
        // `expect` projects the present variant's payload, whose type is the sum's FIRST type arg — the
        // `Some a` of `Option a` (1 arg) AND the `Ok a` of `Result a e` (2 args, payload is arg 0). So
        // substitute `want` for arg 0 only, leaving any others (Result's error type) as-is.
        let mut new_args: Vec<Ty> = sum_args.iter().cloned().collect();
        new_args[0] = want.clone();
        let payload_sum = Ty::Sum {
            decl,
            args: std::rc::Rc::from(new_args),
        };
        return width_fault_against_ty(db, sum_arg, &payload_sum);
    }
    // A RUNTIME conditional `(if c a b)` in a narrow-width context: the WHOLE `if` carries the expected
    // type `want`, so BOTH of its live branches must fit `want` — each branch is a value the annotation's
    // width applies to. Without this a bare out-of-range literal in a branch (`(: (if c 10000 0) UInt8)`,
    // or the same reaching a narrow PARAMETER) slipped through `cdz check` because a runtime `if` folds to
    // neither a `Resolved::Int` nor a `Core::ConstInt` (the narrow-int block below then reads `v = None`
    // and returns), while the EMIT path DID reject it (CDZ0302) — a check-vs-emit gap. Descend into each
    // branch here so `check` catches it at the same choke point the compound-payload cases use. A CONSTANT-
    // condition `if` is NOT descended: `core_of` folds it to its taken branch (handled by the constant path
    // below), so a `Core::If` result marks a genuine runtime conditional with both branches live — checking
    // both is sound (neither is dead), whereas descending a folded `if` would falsely reject a dead untaken
    // out-of-range branch.
    if matches!(
        crate::lower::core_of(db, value),
        crate::core::Core::If { .. }
    ) && let Resolved::If { then_, else_, .. } = resolved_of(db, value)
    {
        return width_fault_against_ty(db, then_, want)
            .or_else(|| width_fault_against_ty(db, else_, want));
    }
    // A RUNTIME `(match s (p0 b0) …)` in a narrow-width context — the same rule as the `if` above, one
    // body per arm: the whole `match` carries `want`, so EVERY arm body must fit it, and a bare
    // out-of-range literal in any arm (`(: (match n (0 10000) (_ 0)) UInt8)`, or reaching a narrow param)
    // slipped `cdz check` while emit rejected CDZ0302. A `Core::Match` after `core_of` marks a genuine
    // RUNTIME match (all arms live) — a CONSTANT-scrutinee match folds to its selected arm (handled by the
    // constant path below), so a folded-away non-selected out-of-range arm is not falsely rejected. Descend
    // each arm's BODY against `want` (a pattern binder in the body is fine — the width check reads the
    // body's constant leaves, exactly as an if-branch).
    if matches!(
        crate::lower::core_of(db, value),
        crate::core::Core::Match { .. }
    ) && let Resolved::Match { arms, .. } = resolved_of(db, value)
    {
        return arms
            .iter()
            .find_map(|&(_pattern, body)| width_fault_against_ty(db, body, want));
    }
    // A FLOAT literal that overflows a narrow `Float32` `want` — the float analogue of the narrow-int
    // block below, reached here (not only by `literal_width_fault`'s direct-literal check) so it fires
    // through the runtime `if`/`match` descent above: `(: (if c 1.0e300 0.0) Float32)` PASSED `cdz
    // check` while the emit path produced an INVALID module (the branch literal is `±inf` in Float32, a
    // malformed value with no written form). Only `Float32` is narrow enough to overflow a finite `Float64`
    // literal; the retype-to-`Float64` fix is not offered here (no `ty_expr` for a nested/branch position,
    // like the nested-int case). Reuses `dec.fits_f32()` — the same predicate `literal_width_fault` runs.
    if let Ty::Float(ft) = want
        && ft.ground_width() == 32
    {
        // The overflowing float `Decimal`, whether `value` is a DIRECT float-literal atom OR a value that
        // FOLDS to a constant float — a CONST-condition `(if true 1.0e300 0.5)` reduces via `core_of` to
        // `Core::ConstFloat(1.0e300)`, materializing the malformed `inf` that a runtime `if` (handled by the
        // descent above) would reject; without reading the fold, the const-fold path slipped `check` and
        // COMPILED + ran to `inf`. (A runtime `if` is a `Core::If`, taken by the descent arm above, not here.)
        let dec = match db.ast.get(value) {
            crate::ast::Struct::Atom(lid) => match db.ast.leaf(*lid).clone() {
                crate::ast::Leaf::Float(dec) => Some(dec),
                _ => None,
            },
            _ => match crate::lower::core_of(db, value) {
                crate::core::Core::ConstFloat(dec) => Some(dec),
                _ => None,
            },
        };
        if let Some(dec) = dec
            && !dec.fits_f32()
        {
            return Some(
                Reject::coded(
                    Code::IntOutOfRange,
                    "float literal does not fit the annotated type Float32 (it overflows the Float32 \
                     range to infinity — the largest finite Float32 is about 3.4e38)",
                )
                .at(value),
            );
        }
    }
    if let Ty::Int(it) = want
        && let crate::ty::Width::Fixed(w) = it.width
        && w != 0
        && !matches!(type_of(db, value), Ty::BigInt | Ty::Rational)
    {
        let v = match resolved_of(db, value) {
            Resolved::Int(v) => Some(v),
            _ => match crate::lower::core_of(db, value) {
                crate::core::Core::ConstInt(v) => Some(v),
                _ => None,
            },
        };
        if let Some(v) = v
            && !v.fits_width(it.ground_signed(), w)
        {
            // Anchor at the offending literal node. The width came from a SOLVED element/payload `Ty` (an
            // enclosing compound annotation, or — via the list arms — a SIBLING literal's annotation), NOT a
            // written sub-annotation on THIS literal, so there is no type-node to retype. `int_out_of_range_reject`
            // would attach `Fix::replace_heuristic(<literal>, "Int16")` — rewriting the VALUE `-41` into a TYPE
            // name (`(list (: 1 UInt64) Int8)`), a source-corrupting suggestion. So build the reject WITHOUT a
            // fix: the message names the valid range, which is the actionable fact (the direct-annotation
            // callers `(: v T)` / `((: name T) v)` DO have a type-node and keep their retype fix). See the
            // `int_out_of_range_reject` doc.
            return Some(
                Reject::coded(
                    Code::IntOutOfRange,
                    int_out_of_range_message(want, it.ground_signed(), w, &db.name_ctx()),
                )
                .at(value),
            );
        }
        return None;
    }
    // Not a narrow int `want` — recurse if the value is itself a compound whose expected shape is `want`.
    // (A nested `(Some (tuple 999))` : `(Option (Tuple Int8))` descends Sum → Tuple.)
    nested_width_fault_by_ty(db, value, want)
}

/// The `Ty`-driven twin of `nested_literal_width_faults`'s descent (which is `ty_expr`-driven at the top):
/// descend a compound `value` against an already-solved expected `Ty`. Sum/Tuple/List, single-payload.
pub(super) fn nested_width_fault_by_ty(db: &mut Db, value: StructId, want: &Ty) -> Option<Reject> {
    match want {
        Ty::Sum { .. } => {
            let Resolved::Apply { head, args } = resolved_of(db, value) else {
                return None;
            };
            if crate::eval::variant_disc_of(db, head).is_none() || args.len() != 1 {
                return None;
            }
            let inner = payload_ty_at_instantiation(db, head, want)?;
            width_fault_against_ty(db, args[0], &inner)
        }
        Ty::Tuple(elem_tys) => {
            let elems = positional_value_nodes(db, value, crate::resolved::Prim::TupleNew)?;
            elem_tys
                .iter()
                .zip(elems.iter())
                .find_map(|(t, &e)| width_fault_against_ty(db, e, t))
        }
        Ty::List(elem_ty) => {
            let elems = positional_value_nodes(db, value, crate::resolved::Prim::ListNew)?;
            let elem_ty = (**elem_ty).clone();
            elems
                .iter()
                .find_map(|&e| width_fault_against_ty(db, e, &elem_ty))
        }
        // A RECORD value against a `(Record …)` expected type — each declared field's type applies to its
        // value node, keyed by symbol. The `Ty`-driven twin of the `ty_expr`-driven Record arm in
        // `nested_literal_width_faults_against`: without it a narrow FIELD literal fed through a compound
        // op ARGUMENT — `(Send.put (record (small 999) (big 5)))` for `(-> (Record (small UInt8) …) …)` —
        // escaped the fit-check (the tuple/list element arms above already reach their elements, but a
        // record row was not descended), so `999` inhabited the `UInt8` field and the arm OBSERVED it
        // (breaker nc-t3, the record face of the nw-class op-arg soundness gap). Read the field value nodes
        // by symbol from whichever record shape (a folded `Resolved::Record` or a `RecordNew` name-alias),
        // exactly as the `ty_expr` descent does.
        Ty::Record(field_tys) => {
            let fields = match resolved_of(db, value) {
                Resolved::Record { fields } => (*fields).clone(),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::RecordNew) =>
                {
                    crate::resolve::read_record_fields(db, &args).ok()?
                }
                _ => return None,
            };
            field_tys.iter().find_map(|(sym, t)| {
                fields
                    .get(sym)
                    .and_then(|&v| width_fault_against_ty(db, v, t))
            })
        }
        // A MAP value against a `(Map K V)` expected type — each entry key literal against `K`, each value
        // literal against `V`. The `Ty`-driven twin of the `ty_expr` Map arm; the map face of the same
        // compound-op-argument gap (a `(-> (Map … UInt8) …)` op arg with an out-of-range value literal).
        Ty::Map(key_ty, val_ty) => {
            let (key_ty, val_ty) = ((**key_ty).clone(), (**val_ty).clone());
            match resolved_of(db, value) {
                Resolved::Map { entries } => entries.to_vec().iter().find_map(|&(k, v)| {
                    width_fault_against_ty(db, k, &key_ty)
                        .or_else(|| width_fault_against_ty(db, v, &val_ty))
                }),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::MapNew) =>
                {
                    // Read `(key, value)` from a native `(= k v)` FieldPair entry (M2) as well as the legacy
                    // 2-element `(k v)` pair — see the `ty_expr`-driven twin in
                    // `nested_literal_width_faults_against`.
                    args.iter()
                        .filter_map(|&entry| {
                            db.ast
                                .field_pair_parts(entry)
                                .or_else(|| db.ast.field_pair(entry))
                                .or_else(|| match db.ast.get(entry) {
                                    crate::ast::Struct::List(items) if items.len() == 2 => {
                                        Some((items[0], items[1]))
                                    }
                                    _ => None,
                                })
                        })
                        .collect::<Vec<_>>()
                        .iter()
                        .find_map(|&(k, v)| {
                            width_fault_against_ty(db, k, &key_ty)
                                .or_else(|| width_fault_against_ty(db, v, &val_ty))
                        })
                }
                _ => None,
            }
        }
        // A SET value against a `(Set E)` expected type — each element literal against `E`. The `Ty`-driven
        // twin of the `ty_expr` Set arm; without it a native `#set(e…)` literal (`Resolved::Set`) fed through
        // a compound-op ARGUMENT with an out-of-range element escaped the fit-check (the set face of the
        // compound-op-argument narrow-width soundness gap, mirroring the Map/Record arms above).
        Ty::Set(elem_ty) => {
            let elem_ty = (**elem_ty).clone();
            match resolved_of(db, value) {
                Resolved::Set { elems } => elems
                    .to_vec()
                    .iter()
                    .find_map(|&e| width_fault_against_ty(db, e, &elem_ty)),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::SetOf)
                        && args.len() == 1 =>
                {
                    positional_value_nodes(db, args[0], crate::resolved::Prim::ListNew)?
                        .iter()
                        .find_map(|&e| width_fault_against_ty(db, e, &elem_ty))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The CDZ0302 reject for an integer literal `v` that overflows the `(signed, width)` type `annot_ty`,
/// carrying — when possible — a retype fix: replace the annotation `ty_expr` with the SMALLEST aliased
/// width ({8,16,32,64}) that DOES fit `v`, the rustc-style "value doesn't fit; use a type that holds it"
/// repair (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). Two shapes.
/// SAME-SIGNEDNESS WIDEN: a magnitude too large for the width (`(: 999 Int8)` → `Int16`, `(: 70000
/// UInt8)` → `UInt32`) takes the smallest wider width of the SAME sign. SIGN FLIP: a NEGATIVE literal in
/// an UNSIGNED type (`(: -5 UInt8)` → `Int8`) takes the smallest SIGNED width holding `v` — no unsigned
/// type can EVER hold a negative value, so the fit is UNAMBIGUOUS (rustc makes exactly this suggestion);
/// this is NOT a speculative signedness guess, since a negative literal has no unsigned reading, so the
/// signed type is forced, not chosen.
/// A value beyond `Int64`/`UInt64` (no aliased width fits) retypes to `BigInt` — the unbounded integer
/// type holds any magnitude. Replacing the whole `ty_expr` rewrites either spelling — a bare `Int8` or a
/// `(Int 8)` compound — to the bare `Int16` (or `BigInt`).
/// Heuristic: the retype clears the range fault, but whether the author meant a wider/signed type (vs. a
/// different literal) is theirs to confirm. Shared by both CDZ0302 literal-range sites (the value
/// annotation `(: v T)` and the let-binder/param `((: name T) v)`), so both carry the fix.
///
/// WARNING: `ty_expr` MUST be a written TYPE node (the annotation being retyped) — the fix replaces its spelling
/// with a type name. NEVER pass a VALUE node here: a literal whose width came from a solved/inferred `Ty`
/// (a nested compound payload, or a sibling literal's annotation) has no type-node to retype, and
/// rewriting the literal into a type name corrupts the source. Those sites build the reject directly
/// (message + `.at(value)`, no fix) — see `width_fault_against_ty`'s narrow-int arm.
pub(super) fn int_out_of_range_reject(
    annot_ty: &Ty,
    signed: bool,
    w: u32,
    v: &crate::ast::IntValue,
    ty_expr: StructId,
    ncx: &NameCtx,
) -> Reject {
    let reject = Reject::coded(
        Code::IntOutOfRange,
        int_out_of_range_message(annot_ty, signed, w, ncx),
    );
    // A NEGATIVE literal annotated with an UNSIGNED type cannot fit ANY unsigned width — the value is
    // negative, so only a SIGNED type reads it. Offer the smallest signed width that holds it (forced, not
    // guessed). Otherwise widen within the SAME signedness (the ordinary magnitude-too-large case).
    let (fix_signed, search_from) = if !signed && v.negative {
        (true, 0) // any signed width may fit; search all aliased widths
    } else {
        (signed, w) // widen: strictly larger widths of the same sign
    };
    match crate::ty::ALIASED_INT_WIDTHS
        .iter()
        .copied()
        .filter(|&aw| aw > search_from)
        .find(|&aw| v.fits_width(fix_signed, aw))
    {
        Some(fit) => {
            let stem = if fix_signed { "Int" } else { "UInt" };
            reject.with_fix(Fix::replace_heuristic(ty_expr, format!("{stem}{fit}")))
        }
        // No fixed width (8/16/32/64) holds `v` — the literal overflows even `Int64`/`UInt64`. The UNBOUNDED
        // integer type `BigInt` holds ANY magnitude (a literal grounds to it losslessly), so it is the
        // forced retype when no aliased width fits — the rustc-gold "use a type that holds it" repair
        // continued past the fixed widths. Heuristic (the author may instead have meant a different literal),
        // but the retype clears the range fault in one shot and always type-checks.
        None => reject.with_fix(Fix::replace_heuristic(ty_expr, "BigInt")),
    }
}

/// The CDZ0302 message for an integer literal that overflows the annotated type: names the type and,
/// when the width is a well-formed one whose range renders exactly, appends `(the valid range is
/// min..=max)`. A malformed width (no exact range) falls back to the type-name-only message.
pub(super) fn int_out_of_range_message(
    annot_ty: &Ty,
    signed: bool,
    w: u32,
    ncx: &NameCtx,
) -> String {
    match int_width_range(signed, w) {
        Some(range) => format!(
            "integer literal does not fit the annotated type {} (the valid range is {range})",
            annot_ty.render_name(ncx),
        ),
        None => format!(
            "integer literal does not fit the annotated type {}",
            annot_ty.render_name(ncx)
        ),
    }
}

/// The inclusive value range a `(signed, width)` integer type holds, rendered `min..=max` (rustc's
/// "the range is `-128..=127`" phrasing) — a signed N-bit holds `-(2^(N-1)) ..= 2^(N-1) - 1`, an
/// unsigned N-bit `0 ..= 2^N - 1`. Names the concrete bounds a CDZ0302 out-of-range literal missed, so
/// the message says WHICH range rather than only the type name. Returns `None` for a width the `i128`/
/// `u128` arithmetic can't hold exactly (`w == 0`, or `> 127` signed / `> 128` unsigned — only a
/// MALFORMED width, since a well-formed integer type is `1..=64`); the caller then omits the range
/// clause rather than the helper panicking on a shift overflow.
pub(crate) fn int_width_range(signed: bool, w: u32) -> Option<String> {
    if w == 0 {
        return None;
    }
    if signed {
        if w > 127 {
            return None;
        }
        let max = (1i128 << (w - 1)) - 1;
        let min = -(1i128 << (w - 1));
        Some(format!("{min}..={max}"))
    } else {
        if w > 128 {
            return None;
        }
        // `1u128 << 128` overflows; `w == 128` max is `u128::MAX` directly.
        let max = if w == 128 {
            u128::MAX
        } else {
            (1u128 << w) - 1
        };
        Some(format!("0..={max}"))
    }
}

//! `infer::construct` — type construction/application (`apply_type`) and the expected-vs-actual
//! mismatch DIAGNOSTICS + coercion-FIX builders, extracted verbatim from `infer.rs` to bring the
//! parent module under the source-size limit. Behavior + API unchanged: items keep their original
//! visibility (pub / pub(crate)); formerly-private helpers become pub(crate) and ALL are re-exported
//! into `infer` via `pub use construct::*`, so every `crate::infer::<item>` path resolves exactly as
//! before. The block uses `use super::*` to see infer's other private helpers.

use super::*;

pub(crate) fn apply_type(db: &mut Db, head: StructId, args: &[StructId]) -> Ty {
    // CASE-OF-CASE (matches `lower`): a head that reduces to a runtime `if` — `((if c a b) args…)` —
    // types as the `if` of the two branch applications. Each branch's lambda applies (β-reduces) to a
    // concrete result type, so the application's type is that (`Int64`), NOT `Ty::Fn` (the naive type
    // of the `if`, which would then have no machine representation at the boundary). Type each branch
    // applied to the same args and JOIN — an `if`'s two branches must agree, so either branch's type is
    // the result; take the then-branch's (the else must unify with it, checked at the `if`'s own node).
    if let Some((_cond, then_head, else_head)) = crate::eval::reduce_to_if(db, head) {
        let then_ty = apply_type(db, then_head, args);
        if !matches!(then_ty, Ty::Any) {
            return then_ty;
        }
        // The then-branch didn't determine a type (recursive/undetermined); fall back to the else.
        return apply_type(db, else_head, args);
    }
    // A LAMBDA head β-reduces; the application's type is the reduced body's type. The reduction runs
    // under the recursion guard (keyed by the lambda body), so a recursive call declines to `Any`
    // rather than diverging — matching lowering. (For C's corpus every function call folds this way;
    // a scheme-based typing of a lambda head arrives with a def's inferred scheme.)
    if crate::eval::lambda_body(db, head).is_some() {
        trace!(target: "rcdzc::infer", head = head.0, args = args.len(), "apply: β-reduce lambda head for its type");
        let reduced_ty = match db.enter_reduction() {
            Some(mut guard) => {
                let g = guard.db();
                match crate::eval::apply_lambda(g, head, args) {
                    Ok(Some(reduced)) => Some(type_of(g, reduced)),
                    _ => None, // recursive / partial — β-reduction can't type it; try the scheme below.
                }
            }
            None => None, // depth limit (recursive) — try the scheme below.
        };
        if let Some(t) = reduced_ty {
            return t;
        }
        // β-reduction couldn't type it (a RECURSIVE callee). Type the call by the callee's DEF SCHEME
        // instead — an annotated recursive def has a determined signature (`def_scheme`), so applying
        // it to the args yields the real result type (Int64 for `(sum-to 3)`) rather than `Any`. This
        // is what lets a recursive call's RESULT flow to a machine type (a wasm return valtype). A
        // callee with no determined scheme (unannotated, needs the connected solve) stays `Any`.
        if let Some(callee) = callee_def_index_for_infer(db, head)
            && let Some(scheme) = def_scheme(db, callee)
        {
            trace!(target: "rcdzc::infer", head = head.0, callee, "apply: recursive call typed by def_scheme");
            return apply_scheme_to_args(db, &scheme, args);
        }
        return Ty::Any; // recursive with an undetermined signature — fault reported elsewhere.
    }
    // `(Int64.of b)` / `(UInt N).of b` where `b : BigInt` — the CHECKED NARROWING from the unbounded
    // integer back to a fixed width (`options/numeric-model/explicit-checked.md`: `Int64.of` converts a
    // `BigInt` back, trapping when out of range). `CheckedOf`'s prelude scheme source is `(Int a)`, which
    // does NOT accept a `BigInt` — so a dedicated arm handles a `BigInt` source: the result is the
    // conversion op's TARGET type (this application's own solved type, an `Ty::Int`), exactly as the
    // fixed-width→fixed-width `of` result is. The lower fold already range-checks the constant (`fits_width`
    // on the unbounded `IntValue`) and rejects an out-of-range one CDZ0302, source-type-agnostically.
    // Only when the source really is a `BigInt`; a fixed-width source stays on the scheme path below.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::CheckedOf)
        && args.len() == 1
        && matches!(type_of(db, args[0]), Ty::BigInt)
    {
        // The result is the conversion op's TARGET width — the RESULT of its `∀a. (Int a) → TARGET`
        // scheme (TARGET is baked into this module's `of` field). Instantiate the head's scheme and peel
        // the arrow's result; the source `(Int a)` is ignored (a `BigInt` source, not unified here). A
        // head without a scheme (malformed) falls through to `Any`.
        let mut fresh = Fresh::new();
        if let Some(scheme) = crate::eval::scheme_of(db, head, &mut fresh)
            && let Ty::Fn(_, result) = crate::unify::instantiate(&scheme, &mut fresh)
        {
            return *result;
        }
        return Ty::Any;
    }
    // `Qty.of x u` — attach a compile-time unit to a numeric value. Its result type is `(Qty T u)` where
    // `T` is the VALUE argument's type and `u` is the VALUE of the second argument (a compile-time unit,
    // read by `eval::unit_of` — NOT an HM-unified variable, exactly as `Prim::Wrap` reads its target
    // width off the solved type rather than the scheme). Checked before the scheme-peeling loop because
    // the unit is not expressible as a static `(meta t)` arrow. If the unit does not reduce (a malformed
    // unit expression), fall through to the generic path (which yields `Any`, faulted elsewhere).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyOf) && args.len() == 2
    {
        let inner = type_of(db, args[0]);
        if let Some(unit) = crate::eval::unit_of(db, args[1]) {
            return Ty::Qty {
                inner: Box::new(inner),
                unit,
            };
        }
    }
    // Each `Record.*` row operation below computes a NEW closed `Ty::Record` from the OPERANDS' record
    // shapes alone (project keeps the named subset, without drops them, merge unions, extend/with insert)
    // — it never mutates an operand type and never leaves a row variable open in the result, so the
    // reshaped record is a fresh value with a statically-fixed field set and the emitted component carries
    // no runtime field-set computation. A shape change happens ONLY through such an explicit `Record.*`
    // operation — inference never widens or narrows a record's field set, so a reshape is always written.
    //= spec/capabilities/type-system.md#a-record-row-is-reshaped-only-through-an-explicit-operation-yielding-a-new-value
    //# A program MUST be able to derive a new record from existing records by an explicit row operation — restricting to named fields, dropping named fields, or combining two records — rather than by an implicit widening or narrowing that inference introduces, so that a shape change is always something the program wrote.
    //= spec/capabilities/type-system.md#a-record-row-is-reshaped-only-through-an-explicit-operation-yielding-a-new-value
    //# A record row operation MUST yield a new record value and MUST NOT alter the operand records, consistent with the immutable value heap, so that reshaping a record is the derivation of a new value with a new shape and not a mutation of an existing one.
    //= spec/capabilities/type-system.md#a-record-row-is-reshaped-only-through-an-explicit-operation-yielding-a-new-value
    //# The shape of a record row operation's result MUST be determined statically from the operands' shapes, so that the emitted component carries a concrete closed record shape and the operation introduces no runtime field set.
    //
    // `Record.project r (a c)` — narrow `r` to the named fields. The result is a NEW closed record type
    // whose fields are EXACTLY the named ones, each carrying the type it had in `r` (`type-system.md` §A
    // Record Is Restricted To A Named Set Of Its Fields). The second operand is a LITERAL field-name list
    // `(a c)` (labels via `record_op_labels`, not an evaluated value — not HM-unified, like `Qty.of`'s
    // unit / `Wrap`'s width). A named field ABSENT from `r`'s record type has no type to carry — it is
    // the CDZ0212 rejection (reported by `check_application`); here it is simply dropped from the result
    // shape so the value column stays a sane record. A non-record operand → `Any` (faulted elsewhere).
    //= spec/capabilities/type-system.md#a-record-is-restricted-to-a-named-set-of-its-fields
    //# A program MUST be able to project a record onto a stated set of field names, yielding a record whose fields are exactly those names bound to the values the operand holds for them, so that narrowing a record to a sub-shape is an explicit operation rather than an overloaded equality.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordProject)
        && args.len() == 2
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some(labels) = crate::resolve::record_op_labels(db, args[1])
    {
        let mut kept = std::collections::BTreeMap::new();
        for label in &labels {
            if let Some(ty) = fields.get(label) {
                kept.insert(label.clone(), ty.clone());
            }
        }
        return Ty::Record(std::rc::Rc::new(kept));
    }
    // `Record.without r (b)` — `r` MINUS the named fields (the complement of `project`). The result is a
    // NEW record type keeping every field of `r` whose label is NOT named. Same literal field-name list;
    // an absent named field is CDZ0212 (`check_application`), not reflected in the shape.
    //= spec/capabilities/type-system.md#a-record-is-reduced-by-dropping-a-named-set-of-its-fields
    //# A program MUST be able to derive a record that drops a stated set of field names from an operand record, yielding a record whose fields are exactly the operand's remaining fields, so that removing a field is the complement of projecting the fields kept.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordWithout)
        && args.len() == 2
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some(labels) = crate::resolve::record_op_labels(db, args[1])
    {
        let drop: std::collections::BTreeSet<_> = labels.into_iter().collect();
        let kept: std::collections::BTreeMap<_, _> = fields
            .iter()
            .filter(|(k, _)| !drop.contains(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return Ty::Record(std::rc::Rc::new(kept));
    }
    // `Record.merge a b` — the UNION of two records' fields. The result is a NEW record type with every
    // field of BOTH operands. The field sets MUST be disjoint (a shared name is CDZ0211, reported by
    // `check_application`); the type here is the union regardless (a shared field's fault fires there, and
    // last-writer here keeps the shape sane). A non-record operand → the generic path (Any).
    //= spec/capabilities/type-system.md#two-records-are-combined-only-when-their-field-sets-are-disjoint
    //# A program MUST be able to combine two records into one whose field set is the union of the operands' field sets, each field bound to the value its source record holds, so that merging records is the row analogue of forming a record from two groups of fields.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordMerge)
        && args.len() == 2
        && let (Ty::Record(a), Ty::Record(b)) = (type_of(db, args[0]), type_of(db, args[1]))
    {
        let mut union: std::collections::BTreeMap<_, _> =
            a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in b.iter() {
            union.insert(k.clone(), v.clone());
        }
        return Ty::Record(std::rc::Rc::new(union));
    }
    // `Record.extend r (z v)` / `Record.with r (z v)` — ADD (extend) or REPLACE (with) field `z` with the
    // VALUE `v`'s type. Both yield a NEW record type = `r`'s fields with `z ↦ typeof(v)` inserted (an
    // insert covers both: for `extend` z is new, for `with` it overwrites the old entry with the possibly-
    // DIFFERENT new type — §A Field Is Added To Or Replaced …, 'a new value of a possibly different type').
    // The presence/absence fault (extend→CDZ0211 if present, with→CDZ0212 if absent) is `check_application`'s;
    // the shape here is the same insert for both. The second operand is a `(name value)` pair read by
    // `record_op_pair`; `v` IS an evaluated value (its type is `typeof(v)`), unlike a label list.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A program MUST be able to derive a record that adds a field absent from an operand record, and a combination that adds a field the operand already contains MUST be rejected at compile time with the machine-readable code for a field that is already present, so that adding a field never silently overwrites an existing one.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A program MUST be able to derive a record that replaces a field present in an operand record with a new value of a possibly different type, so that updating a field is an explicit operation distinct from adding one and the replacement's type is whatever the new value holds.
    // THREE-operand form (DESIGN-record-update-syntax.md): `(Record.with r #z v)` /
    // `(Record.extend r #z v)` — a record, a `#symbol` field LABEL (`read_key` reads the label statically,
    // NOT as a `Ty::Symbol` value — the row-op field name stays compile-time), and the VALUE `v` (an
    // ordinary expression, its type is `typeof(v)`). Result = the record with field `z` inserted/replaced.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A program MUST be able to derive a record that adds a field absent from an operand record, and a combination that adds a field the operand already contains MUST be rejected at compile time with the machine-readable code for a field that is already present, so that adding a field never silently overwrites an existing one.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A program MUST be able to derive a record that replaces a field present in an operand record with a new value of a possibly different type, so that updating a field is an explicit operation distinct from adding one and the replacement's type is whatever the new value holds.
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(crate::resolved::Prim::RecordExtend | crate::resolved::Prim::RecordWith)
    ) && args.len() == 3
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some(label) = crate::resolve::read_label(db, args[1])
    {
        let mut out: std::collections::BTreeMap<_, _> =
            fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        out.insert(label, type_of(db, args[2]));
        return Ty::Record(std::rc::Rc::new(out));
    }
    // `Record.pop r z` — yields `(tuple (. r z) (r without z))`: the field's value paired with the record
    // of the remaining fields (`type-system.md` §A Record Is Reduced By Dropping A Named Set Of Its
    // Fields). Result type `(Tuple <typeof field z> (Record <r minus z>))`. The absent-field CDZ0212 is
    // `check_application`'s; here an absent field would leave the tuple's first element `Any` (faulted
    // there). The second operand is a BARE field NAME (a label via `read_key`).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordPop)
        && args.len() == 2
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some(label) = crate::resolve::read_label(db, args[1])
    {
        let field_ty = fields.get(&label).cloned().unwrap_or(Ty::Any);
        let rest: std::collections::BTreeMap<_, _> = fields
            .iter()
            .filter(|(k, _)| **k != label)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let rest_ty = Ty::Record(std::rc::Rc::new(rest));
        return Ty::Tuple(std::rc::Rc::from([field_ty, rest_ty]));
    }
    // Each `Tuple.*` positional row op below derives a NEW `Ty::Tuple` (or `Ty::Unit` for the empty
    // prefix) from the OPERANDS' element types — cat sums the arities, split-at/pop partition — so the
    // result arity is fixed statically from the operand arities, an operand tuple is never mutated, and
    // the emitted component carries a concrete tuple shape rather than a runtime-length tuple.
    //= spec/capabilities/type-system.md#a-tuple-is-reshaped-positionally-by-an-explicit-operation-yielding-a-new-value
    //# A program MUST be able to derive a new tuple from existing tuples by an explicit positional operation — concatenating two tuples or splitting one at a stated position — rather than by an implicit change of arity, consistent with a tuple being a fixed-size positional value whose length is part of its type.
    //= spec/capabilities/type-system.md#a-tuple-is-reshaped-positionally-by-an-explicit-operation-yielding-a-new-value
    //# A tuple positional operation MUST yield a new tuple value and MUST NOT alter the operand tuples, consistent with the immutable value heap, so that reshaping a tuple is the derivation of a new value and not a mutation.
    //= spec/capabilities/type-system.md#a-tuple-is-reshaped-positionally-by-an-explicit-operation-yielding-a-new-value
    //# The arity of a tuple positional operation's result MUST be determined statically from the operands' arities, so that the emitted component carries a concrete tuple shape and the operation introduces no runtime-length tuple.
    //
    // `Tuple.concat a b` — the concatenation of two tuples' element types (`type-system.md` §Two Tuples Are
    // Concatenated Into One Of Their Combined Length). Result arity = the sum; each element keeps its
    // source position's type. A non-tuple operand → the generic path (Any).
    //= spec/capabilities/type-system.md#two-tuples-are-concatenated-into-one-of-their-combined-length
    //# A program MUST be able to concatenate two tuples into one whose elements are the first tuple's elements in order followed by the second tuple's elements in order, so that its arity is the sum of the operands' arities and each element keeps the type of its source position.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TupleCat)
        && args.len() == 2
        && let (Ty::Tuple(a), Ty::Tuple(b)) = (type_of(db, args[0]), type_of(db, args[1]))
    {
        let cat: Vec<Ty> = a.iter().chain(b.iter()).cloned().collect();
        return Ty::Tuple(cat.into());
    }
    // `Tuple.split-at t k` — split at compile-time position `k` into a PAIR `(prefix suffix)`
    // (`type-system.md` §A Tuple Is Split At A Position Into A Prefix And A Suffix). The result type is
    // `(Tuple <prefix-tuple> <suffix-tuple>)`; `k=0` makes the empty prefix the UNIT value (the empty
    // tuple IS unit), so the prefix type is `Ty::Unit`, not a zero-arity tuple. `k` out of `0..=arity` is
    // CDZ0201 (`check_application`); here an out-of-range k falls through to the generic path (Any).
    //= spec/capabilities/type-system.md#a-tuple-is-split-at-a-position-into-a-prefix-and-a-suffix
    //# A program MUST be able to split a tuple at a stated position into a pair of tuples — a prefix holding the elements before the position and a suffix holding the elements from the position onward — so that partitioning a tuple positionally is an explicit operation yielding both parts.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TupleSplitAt)
        && args.len() == 2
        && let Ty::Tuple(elems) = type_of(db, args[0])
        && let Resolved::Int(k) = resolved_of(db, args[1])
        && let Some(k) = k.to_i64()
        && (0..=elems.len() as i64).contains(&k)
    {
        let k = k as usize;
        let prefix = tuple_or_unit(&elems[..k]);
        let suffix = tuple_or_unit(&elems[k..]);
        return Ty::Tuple(std::rc::Rc::from([prefix, suffix]));
    }
    // `Tuple.remove t` — element 0 off: `(tuple (. t 0) <rest>)`. Result `(Tuple <e0> (Tuple <rest…>))`. A
    // non-tuple or empty-tuple operand falls through (Any); the arity-≥1 requirement is `check_application`'s.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TuplePop)
        && args.len() == 1
        && let Ty::Tuple(elems) = type_of(db, args[0])
        && !elems.is_empty()
    {
        let rest = tuple_or_unit(&elems[1..]);
        return Ty::Tuple(std::rc::Rc::from([elems[0].clone(), rest]));
    }
    // `Tuple.size t` — the tuple's arity as an `Int64` (a compile-time-known count; `lower` folds it to a
    // constant Int). A non-tuple operand falls through to the generic path (→ Any, faulted elsewhere).
    //= spec/capabilities/type-system.md#the-arity-of-a-tuple-positional-operation-s-result-must-be-determined-statically
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TupleSize)
        && args.len() == 1
        && matches!(type_of(db, args[0]), Ty::Tuple(_))
    {
        return Ty::int64();
    }
    // `Type.of e` — compile-time type reflection. The application itself has type `Type` (it IS a
    // type-value, like `(Qty T u)` / `(-> A B)` in type position); the type-value it REDUCES to (via
    // `eval::typeval_of` → the `reduce_ctor` arm) is `e`'s inferred type, consumed only in a type
    // position (an annotation, further type-level computation). A `Type` value is erased before the
    // boundary, so exporting one is rejected downstream ("a type value has no runtime form").
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TypeOf)
        && args.len() == 1
    {
        return Ty::Type;
    }
    // `Type.eq a b` — compile-time type equality. Its result is an ordinary `Bool` (the type COMPARISON
    // is compile-time — `lower` folds it to a constant — but the produced value is a runtime `Bool`, so
    // it flows into an `if`/`and`/… and branches on types). The arguments are type-values, read (not
    // HM-unified) by `lower`; a non-type argument is faulted where it fails to reduce there.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TypeEq)
        && args.len() == 2
    {
        return Ty::Bool;
    }
    // `Type.ast e` / `Type.ast-generic e` — compile-time type→AST reflection. The application's type is the
    // built-in `Ast` prelude sum (the value it folds to, a `(type …)` decl reflected via `Ast.*` ctors),
    // exactly as a quote / `Ast.module` types as `Ast`. A non-concrete argument declines at the LOWER fold
    // (not here — the type is `Ast` regardless of whether the fold succeeds).
    if args.len() == 1
        && matches!(
            crate::eval::meta_apply_of(db, head),
            Some(crate::resolved::Prim::TypeAst { .. })
        )
    {
        return super::ast_sum_ty(db).unwrap_or(Ty::Any);
    }
    // `Qty.value q` — recover the underlying numeric value, DISCARDING the unit. Its result is the
    // quantity's INNER type; a non-quantity argument yields `Any` (faulted elsewhere).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyValue)
        && args.len() == 1
    {
        return match type_of(db, args[0]) {
            Ty::Qty { inner, .. } => *inner,
            _ => Ty::Any,
        };
    }
    // `Qty.pow q n` — the result unit is q's unit raised to the `n`th power (`Unit::pow`, composing
    // exponents + scale exactly as `Unit.^`); the inner numeric type is unchanged. `n` is a compile-time
    // Int literal read off arg1 (not an HM variable, like `Unit.^`'s power / `Qty.of`'s unit). A
    // negative `n` is left to `check_application`/lower to reject; here we still shape the type (the unit
    // map handles a negative power fine) so downstream sees a sane `Ty::Qty`. A non-quantity arg0 or a
    // non-literal exponent falls through to the generic path (→ Any, faulted elsewhere). The body is a
    // separate (never-inlined) helper so it keeps its `Ty::Qty` destructure (a `Box` + an inline `Unit`
    // map) out of `apply_type`'s frame, which is on the deep `type_of`↔`apply_type` recursion.
    if args.len() == 2
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyPow)
        && let Some(ty) = qty_pow_type(db, args)
    {
        return ty;
    }
    // `Unit.in target q` — explicit conversion that UNWRAPS to a bare dimensionless number
    // (DESIGN-quantity-reference-normalized-unwrap.md §1b): it converts q's magnitude into the target
    // unit's scale AND strips the quantity wrapper, so `(Unit.in meter (Qty.of 3.0 km))` : `Float64`
    // (the *number* of meters, 3000.0), NOT `(Qty Float64 meter)`. `as`/`in` is the deliberate EXIT
    // from the units world — the result is an ordinary number, no longer dimension-checked. The target
    // is still required to share q's DIMENSION (`check_application`, CDZ0501); here we fill the
    // value-column type with q's INNER numeric type. If the target unit doesn't reduce, or q isn't a
    // quantity, fall through (→ Any, faulted elsewhere).
    //= spec/capabilities/units-of-measure.md#an-explicit-conversion-unwraps-to-a-bare-number
    //# An explicit conversion of a quantity into a chosen unit — the `as`/`in` operation — MUST yield the dimensionless number counting how many of the chosen unit the quantity is, with the quantity wrapper removed, so that a conversion is the deliberate exit from the dimensional layer rather than a re-expression that stays dimensioned.
    //= spec/capabilities/units-of-measure.md#an-explicit-conversion-unwraps-to-a-bare-number
    //# The result of an explicit conversion MUST be an ordinary number of the quantity's underlying numeric type, subject to ordinary numeric rules and no longer dimension-checked, so that once a program has asked "how many of this unit is it?" it holds the answer as a plain number and may combine it freely.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::UnitIn)
        && args.len() == 2
        && let (Some(_target), Ty::Qty { inner, .. }) =
            (crate::eval::unit_of(db, args[0]), type_of(db, args[1]))
    {
        return *inner;
    }
    // (Prefix negation `(- e)` is deprecated: arity-1 `Sub` is no longer typed as its operand's type —
    // it CURRIES like `(+ 1)` via the generic binary scheme below, so its result type is the curried
    // arrow `(-> T T)`. `Num.neg`/`T.neg` (`Prim::Neg`) are the negation replacement.)
    // A binary OPERATOR applied to QUANTITIES — the dimensional result type (units-of-measure.md §How
    // Arithmetic Composes Dimensions). Only engages when an operand is a `Ty::Qty` (two bare numbers take
    // the ordinary scheme path). `+`/`-` keep the shared unit (result `(Qty T u)`); `*`/`/` COMPOSE units
    // by the group product/quotient (result `(Qty T (u_a·u_b))` / `(Qty T (u_a/u_b))` — a `Unit.one`
    // result renders dimensionless); comparisons yield `Bool`. The unit COMPOSITION is why this is not an
    // HM scheme — a unit multiplies, it does not unify. A dimensional/​numeric FAULT is reported by
    // `check_application`; here we only fill the value-column TYPE. (When the units disagree for `+`/`-`,
    // the fault fires there and this type is unused; we still return the lhs quantity so the shape stays
    // sane.)
    if args.len() == 2
        && let Some(prim) = crate::eval::meta_apply_of(db, head)
    {
        {
            let a = type_of(db, args[0]);
            let b = type_of(db, args[1]);
            // PROVISIONAL-OPERAND DEFER: an arith operand that is `Any` WHILE A SCHEME SOLVE IS IN FLIGHT is
            // a not-yet-resolved SELF-CALL result — the recursion guard returns `Any` for a self-call typed
            // inside its own def's body during that def's scheme solve. Committing the arith result NOW
            // (the numeric arms below all fail to match `Any`, so it falls to the generic `∀a.(Int a)`
            // scheme → a DEFERRED `Int`) FREEZES the wrong type: once the def's result grounds (e.g. a
            // sibling `(Leaf n)` arm fixes it `BigInt`), a clean re-solve types the self-calls `BigInt` and
            // `+` over two BigInts is `BigInt` — but the frozen deferred-`Int` already conflicts with the
            // `BigInt` arm → a spurious CDZ0203 "arms differ: BigInt vs Int64" (`(+ (s a) (s b))` with two
            // self-calls and no anchoring literal). Return `Any` here so the node is NOT cached (the
            // `type_of` memo skips `Any`) and RE-solves cleanly once the operands' real types settle. Only
            // under `solving_schemes` (a re-grounding fixpoint); outside a solve an `Any` operand is a real
            // fault the generic path reports. EITHER-side provisional defers (the guard is `a || b` Any):
            // this block PRECEDES the numeric arms, so even `(+ 1N (f …))` — one anchoring literal + one
            // provisional self-call — defers here rather than committing to the literal's width, which is
            // correct: the `Any` operand's real type is not yet known (it may ground to `BigInt`/`Float`,
            // and `+` over a `BigInt` sibling is `BigInt`, not the literal's `Int`), so no width can be
            // soundly committed until the self-call resolves. A clean re-solve then routes it through the
            // numeric arms below with both operands' real types. (Deferring on either-Any, not both-Any, is
            // the safe choice — never commit an arith width while an operand is an unresolved self-call.)
            if !db.solving_schemes.is_empty()
                && (matches!(a, Ty::Any) || matches!(b, Ty::Any))
                && matches!(
                    prim,
                    crate::resolved::Prim::Add
                        | crate::resolved::Prim::Sub
                        | crate::resolved::Prim::Mul
                        | crate::resolved::Prim::Div
                        | crate::resolved::Prim::Rem
                )
            {
                return Ty::Any;
            }
            // A QUANTITY operand takes the dimensional arm below, NOT the bare-numeric arms here — even
            // when its SIBLING is a bare `Float`/`BigInt`/`Rational`. `(* (Qty Float64 meter) 3.0)` must
            // stay `(Qty Float64 meter)` (a bare number scales, contributing `Unit.one`), not collapse to
            // the bare `Float64` the Float arm would return by matching the bare sibling. So gate the three
            // bare-numeric arms on `!any_qty`, mirroring `check_application`'s `is_multiplicative && any_qty`
            // ordering (the quantity check runs first there too). Without this the unit was silently dropped
            // and `Qty.value` of the result declined ("no machine representation").
            let any_qty = matches!(a, Ty::Qty { .. }) || matches!(b, Ty::Qty { .. });
            // A `+`/`-`/`*`/`/` over BIGINT operands is `BigInt` — the unbounded arithmetic, NOT the
            // fixed-width int scheme (whose `∀a. (Int a) → …` would reject a `BigInt` operand). `lower`
            // routes it to the runtime `bigint-*` op (or folds a constant). A `BigInt`/fixed mix is
            // rejected in `check_application` (CDZ0301), so if one operand is `BigInt` the well-typed
            // case has both — return `BigInt`. (Comparison over BigInt is `Bool`, via the generic path.)
            if matches!(
                prim,
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
                    | crate::resolved::Prim::Rem
            ) && !any_qty
                && (matches!(a, Ty::BigInt) || matches!(b, Ty::BigInt))
            {
                return Ty::BigInt;
            }
            // A `+`/`-`/`*`/`/` over RATIONAL operands is `Rational` — the exact arithmetic, NOT the
            // fixed-width int scheme (whose `∀a. (Int a) → …` rejects a `Rational` operand). `lower` folds
            // a constant pair (normalized over `IntValue` bignum) or (later) emits the runtime op. A
            // `Rational`/integer mix is rejected in `check_application` (CDZ0301), so if one operand is
            // `Rational` the well-typed case has both — return `Rational`. (`%` is NOT a rational op —
            // exact division is total, there is no remainder; a `%` over rationals falls through and its
            // scheme-unify rejects it. Comparison over Rational is `Bool`, via the generic path.)
            if matches!(
                prim,
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
            ) && !any_qty
                && (matches!(a, Ty::Rational) || matches!(b, Ty::Rational))
            {
                return Ty::Rational;
            }
            // A `+`/`-`/`*`/`/` over FLOAT operands is that float type — the SAME arithmetic operator as
            // the integer case, dispatched here on a `Ty::Float` operand (there is no distinct `+.`). Its
            // `∀a. (Int a) → …` scheme does NOT accept a `Float`, so the generic scheme-unify would reject
            // it; type it directly instead, and `lower` remaps to `Prim::FAdd`… + `lower_float_arith`. A
            // `Float`/integer mix is rejected in `check_application` (CDZ0301), so if one operand is
            // `Float` the well-typed case has both — return the float type (the concrete-width operand, so
            // a `Float32` op stays `Float32`; a deferred literal grounds to `Float64`). Comparison over
            // floats is `Bool` via the generic path; `%`/bitwise/shift have no float form and fall through
            // to the scheme (which rejects a float operand — those stay integer-only).
            if matches!(
                prim,
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
            ) && !any_qty
                && (matches!(a, Ty::Float(_)) || matches!(b, Ty::Float(_)))
            {
                // Prefer whichever operand fixed the width (a concrete `Float32`/`Float64` over a deferred
                // literal), mirroring the `join` width preference — so `(+ x 1.0)` with `x : Float32`
                // stays `Float32` rather than the literal's deferred width.
                return match (&a, &b) {
                    (Ty::Float(fa), _) if fa.width_is_fixed() => a.clone(),
                    (_, Ty::Float(fb)) if fb.width_is_fixed() => b.clone(),
                    (Ty::Float(_), _) => a.clone(),
                    (_, Ty::Float(_)) => b.clone(),
                    _ => Ty::float(),
                };
            }
            let a_qty = matches!(a, Ty::Qty { .. });
            let b_qty = matches!(b, Ty::Qty { .. });
            if a_qty || b_qty {
                match prim {
                    crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Rem => {
                        // `+`/`-`/`%` on same-dimension quantities KEEP the shared unit (a remainder is
                        // same-in/same-out like addition: `7m % 3m = 1m`), unlike `*`/`/` which compose
                        // units. The result unit: when both operands are at the SAME unit (scale), keep it (the
                        // common Layer-1 case — no conversion); when they share a dimension but DIFFER in
                        // scale (`meter` + `kilometer`), the result is the dimension's REFERENCE unit (the
                        // deterministic common unit each operand converts to — units-of-measure.md
                        // §Combining Units Of One Dimension Is Well-Formed). The inner numeric type is the
                        // lhs quantity's (both share it; the fault check enforces agreement). The choice is
                        // a pure function of the two operand units (equal → that unit; differ → the
                        // reference), independent of evaluation order, so the result is reproducible.
                        //= spec/capabilities/units-of-measure.md#combining-units-of-one-dimension-is-well-formed
                        //# The result unit of a combination of same-dimension quantities MUST be a deterministic function of the operands' units, so that the result is reproducible rather than dependent on evaluation order.
                        if let (
                            Ty::Qty {
                                inner: ia,
                                unit: ua,
                            },
                            Ty::Qty {
                                inner: ib,
                                unit: ub,
                            },
                        ) = (&a, &b)
                        {
                            let unit = if ua == ub {
                                ua.clone()
                            } else {
                                ua.at_reference()
                            };
                            // The result magnitude type takes the WIDER of the two operand inners — the
                            // effective machine width (`ground_width`: a fixed width is itself, a DEFERRED
                            // width is the default Int64) — so `+` is COMMUTATIVE in the magnitude width.
                            // Taking the lhs inner raw reconciled ASYMMETRICALLY: a DEFERRED call-result
                            // magnitude (default Int64) + a fixed NARROW sibling, narrow-FIRST, forced the
                            // result to the narrow width → a sum that overflows it SPURIOUSLY rejected CDZ0304,
                            // while the swapped order kept the wider default and compiled (breaker #8287).
                            // Picking the wider effective width is order-independent → the deferred magnitude
                            // keeps its default rather than narrowing to the fixed sibling, both orders. The
                            // widen emit is already realized (the call-result-first order compiles today). Two
                            // DIFFERENT FIXED widths still disagree and are rejected CDZ0301 by the
                            // operand-agreement fault check (no silent promotion), symmetric in both orders —
                            // this arm only chooses the RESULT type, which is moot for a rejected program.
                            let result_inner = match (ia.as_ref(), ib.as_ref()) {
                                (Ty::Int(x), Ty::Int(y)) => {
                                    if x.ground_width() >= y.ground_width() {
                                        (**ia).clone()
                                    } else {
                                        (**ib).clone()
                                    }
                                }
                                _ => ia.join(ib),
                            };
                            return Ty::Qty {
                                inner: Box::new(result_inner),
                                unit,
                            };
                        }
                        if let Ty::Qty { inner, unit } = &a {
                            return Ty::Qty {
                                inner: Box::new((**inner).clone()),
                                unit: unit.clone(),
                            };
                        }
                        return b;
                    }
                    crate::resolved::Prim::Mul | crate::resolved::Prim::Div => {
                        // Compose the units. A bare-number operand contributes `Unit.one` (scaling keeps
                        // the other's dimension — `(* (Qty 2 meter) 3)` stays meter).
                        let ua = match &a {
                            Ty::Qty { unit, .. } => unit.clone(),
                            _ => crate::ty::Unit::one(),
                        };
                        let ub = match &b {
                            Ty::Qty { unit, .. } => unit.clone(),
                            _ => crate::ty::Unit::one(),
                        };
                        let unit = if matches!(prim, crate::resolved::Prim::Mul) {
                            ua.mul(&ub)
                        } else {
                            ua.div(&ub)
                        };
                        // The inner numeric type is a quantity operand's inner (both share it).
                        let inner = match (&a, &b) {
                            (Ty::Qty { inner, .. }, _) | (_, Ty::Qty { inner, .. }) => {
                                (**inner).clone()
                            }
                            _ => Ty::Any,
                        };
                        return Ty::Qty {
                            inner: Box::new(inner),
                            unit,
                        };
                    }
                    crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Eq => return Ty::Bool,
                    _ => {}
                }
            }
        }
    }
    // A compound-VALUE constructor (the `tuple`/`record`/`list` alias) applied — its type is the compound
    // of the argument types, even at ZERO arguments (an empty `(list)` / `(tuple)` is a valid empty
    // compound, NOT the ctor record). This is checked BEFORE the zero-arg identity short-circuit below so
    // `(list)` types as `List Any` rather than as the `list` ctor record's type. (The TupleNew/RecordNew/
    // ListNew arms are the ones that can be nullary; a scheme-typed head falls through.)
    // SetNew is HERE too (the `(set …)` alias as an ARGUMENT stays `Apply{SetNew}` rather than reader-
    // flipping to `Resolved::Set`): without it a `(set 1 2)` argument fell through to `Ty::Any`, so its
    // element type was never checked and `(Set.contains (set 1 2) true)` accepted a Bool on a `Set Int64`
    // (SOUNDNESS false-accept, v-cdz-smith --typegen T1.32). Routes to `compound_ctor_type`'s `SetNew` arm
    // = `Set <join elems>`, exactly like its `ListNew` sibling.
    if let Some(
        prim @ (crate::resolved::Prim::TupleNew
        | crate::resolved::Prim::RecordNew
        | crate::resolved::Prim::ListNew
        | crate::resolved::Prim::SetNew),
    ) = crate::eval::meta_apply_of(db, head)
    {
        return compound_ctor_type(db, prim, args);
    }
    // The COMPILER-INTERNAL `ast-splice-lift` intrinsic (`(intrinsic "ast-splice-lift") args`) — its head
    // resolves DIRECTLY to a prim (no `(meta apply)` module record), so read it via `prim_of`. Result is
    // `(List Ast)` (`compound_ctor_type`'s `AstSpliceLift` arm). Only the quasiquote-splice desugar emits
    // it, never user surface.
    if crate::eval::prim_of(db, head) == Some(crate::resolved::Prim::AstSpliceLift) {
        return compound_ctor_type(db, crate::resolved::Prim::AstSpliceLift, args);
    }
    // The COMPILER-INTERNAL `ast-lift` intrinsic (`(intrinsic "ast-lift") e`) — same direct-prim head as
    // `ast-splice-lift`. Result is `Ast` (`compound_ctor_type`'s `AstLift` arm) whatever the operand's
    // type. Only the quasiquote desugar emits it around a runtime active-unquote operand.
    if crate::eval::prim_of(db, head) == Some(crate::resolved::Prim::AstLift) {
        return compound_ctor_type(db, crate::resolved::Prim::AstLift, args);
    }
    // The `map` VALUE-constructor alias applied — `(map (k v) …)` written as a bare NAME head. Its `args`
    // are the ENTRY-PAIR nodes (each a two-element `(key value)` list), NOT curried arguments — so type
    // it as `Map <join keys> <join values>` DIRECTLY (like the `list` alias types `List <join elems>`),
    // never peeling the pairs as a curried application (which would wrongly type `(a 1)` as applying `a`).
    // An empty `(map)` is `Map Any Any`. Homogeneity is `type_errors`' job; this fills the value column.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::MapNew) {
        let mut key_ty = Ty::Any;
        let mut val_ty = Ty::Any;
        for &entry in args {
            // Read the entry's `(key, value)` children — a map entry is structure, not an application
            // (`(a 1)` is the pair a↦1, not `a` applied to `1`). Read via the shared field-pair readers so
            // the canonical `(= key value)` FieldPair (native leaf, `=`-headed list) types its key/value
            // like the legacy bare `(key value)` 2-element pair — seq-276: the `(map (= k v))` name-alias
            // must type identically to `#map((= k v))` (this apply_type arm is the raw map-type consumer;
            // without the `field_pair` read a `(= k v)` entry was skipped → `(Map Any Any)` → a spurious
            // CDZ0203 "not fully determined"). Mirrors `map_entry_nodes` / `resolve_map`.
            if let Some((k, v)) = db
                .ast
                .field_pair_parts(entry)
                .or_else(|| db.ast.field_pair(entry))
                .or_else(|| match db.ast.get(entry) {
                    crate::ast::Struct::List(items) if items.len() == 2 => {
                        Some((items[0], items[1]))
                    }
                    _ => None,
                })
            {
                key_ty = key_ty.join(&type_of(db, k));
                val_ty = val_ty.join(&type_of(db, v));
            }
            // A malformed entry (not a pair) — the fault is reported elsewhere; the map's key/value stay
            // whatever the well-formed entries determined.
        }
        return Ty::Map(Box::new(key_ty), Box::new(val_ty));
    }
    // A NULLARY PERFORM `(E.op)` — an effect operation applied to no argument. Its `(meta t)` scheme is
    // `(-> Unit result)` (a `Unit`-domain op whose unit argument is elided in the corpus surface), so the
    // performance's type is `result`, NOT the op record's type (which the zero-arg identity short-circuit
    // below would wrongly return). Checked before that short-circuit, only for an effect operation, so an
    // ordinary nullary def `(g)` is unaffected. (A non-nullary op reaches the scheme-peeling loop below.)
    if crate::eval::effect_op_of(db, head).is_some() {
        // A REIFIED ASYNC WORLD-EFFECT perform in a reducer-world compile types as the effect-request
        // RECORD (schema-hash phase-1a), NOT the op-sig result. The discriminator (agreed with
        // v-rust-backend, matching its reify fork in `lower`): in a reducer-world compile
        // (`db.wit_world.is_some()`), a HOST-DELEGATED perform (`perform_host_target` = Some — its
        // entrypoint `(host …)` block is its CDZ0401 home) whose effect is NOT a declared world IMPORT
        // (`!is_world_import_op`) is an ASYNC world-effect returned in the effect-list → it reifies. A
        // world-IMPORT op (`is_world_import_op` TRUE — e.g. `kv.get`/`kv.delete`, the sole `kv` import) is
        // SYNCHRONOUS: its result is consumed inline, so it keeps its op-sig result type (the fall-through
        // arms below) — reifying it would break the host-fused kv reducers. Polarity is REIFY-WHEN-FALSE:
        // the world declares only `kv` as an import, so `Model.request`/`Emit.send`/`Tool.run` are
        // `is_world_import_op` FALSE and reify, while `kv.*` is TRUE and stays synchronous. Generic-
        // compiler-clean: reads the declared world imports, zero hard-coded capability vocabulary. A
        // NON-reducer compile (`wit_world` None) or a homeless perform (`perform_host_target` None → the
        // CDZ0401 no-home decline) never reaches the record type.
        if db.wit_world.is_some()
            && let Some((effect, op, _result)) = crate::effects::perform_host_target(db, head, head)
            && !crate::wit_world::is_world_import_op(db.wit_world.as_deref(), &effect, &op)
        {
            // has_target mirrors the reify fork in `lower`: a target-having effect (its op carries an
            // `@resource` marker) reifies the dest to a `target` field, so the record type gains it (ruling A).
            let effect_decl = crate::eval::effect_op_of(db, head).map(|(decl, _)| decl);
            let has_target = effect_decl
                .and_then(|decl| {
                    let op_idx = crate::eval::effect_op_of(db, head).map(|(_, i)| i)?;
                    db.effect_decl_by_occ(decl)
                        .and_then(|e| e.ops.get(op_idx as usize))
                        .and_then(|o| o.resource)
                })
                .is_some();
            // has_descriptor mirrors the reify EMIT: the record type gains `schema_descriptor: Bytes` iff the
            // reify emits it — i.e. iff the effect's descriptor builds. Computed via the SAME
            // `lower::effect_has_schema_descriptor` the emit gates on, so the typed shape can't drift from the
            // emitted shape (the phase-3 bug: emit 4-field, type 3-field → dropped field → schema_hash None).
            let has_descriptor = effect_decl
                .is_some_and(|decl| crate::lower::effect_has_schema_descriptor(db, decl));
            if let Some(record) = world_effect_request_ty(db, has_target, has_descriptor) {
                return record;
            }
        }
        let mut fresh = Fresh::new();
        if let Some(scheme) = crate::eval::scheme_of(db, head, &mut fresh) {
            let cur = crate::unify::instantiate(&scheme, &mut fresh);
            if let Ty::Fn(param, result) = &cur
                && (args.is_empty() && matches!(**param, Ty::Unit))
            {
                return (**result).clone();
            }
        }
    }
    // A ZERO-ARGUMENT application `(g)` with a non-lambda head is the head value — applying to no
    // arguments is the identity (a nullary def `(def (g) 7)` called). Its type is the head's type.
    // Mirrors the same short-circuit in `lower`, so `(g)` types and lowers as its body value. A
    // RECURSIVE nullary call (`(def (f) (f))`) has no normal form — type it `Any` (the fault is
    // reported by the collection side) rather than recursing into its own body without end.
    if args.is_empty() {
        if let Some(body) = crate::eval::lambda_body_of_nullary(db, head)
            && crate::eval::is_recursive(db, body)
        {
            return Ty::Any;
        }
        return type_of(db, head);
    }
    // A NULLARY variant CONSTRUCTOR applied to the unit value — `(None unit)` / `(Nil ())` — constructs
    // the sum (core-semantics.md §Construction MUST Be Via Application). Its ctor `(meta t)` is the bare
    // sum (no arrow — `variant_payload_type` is `None`), so the "peel a curried parameter per arg" loop
    // below would find a non-function and yield `Any`. The application's type IS the sum (the ctor's own
    // `(meta t)`); the unit argument is the payload, not a curried parameter. This is what lets a
    // `(match (None unit) …)` scrutinee type as the sum and route to the sum matcher.
    if crate::eval::variant_disc_of(db, head).is_some()
        && crate::eval::variant_payload_type(db, head).is_none()
    {
        return type_of(db, head);
    }
    let mut fresh = Fresh::new();
    let scheme = match crate::eval::scheme_of(db, head, &mut fresh) {
        Some(s) => s,
        // No `(meta t)` scheme. A RUNTIME FUNCTION VALUE head whose type is directly a `Ty::Fn` — a
        // closure bound out of a compound by a match binder (`(match t ((T.Mk f) (f 5)))`, where `f`
        // reads the `T.Mk` payload arrow), or a function-typed parameter — has no prelude scheme (a
        // `SumPayload`/`Proj`/`Param` binder is not a prelude entry), but its OWN type carries the arrow.
        // Peel one arrow per argument to get the result: `f : (-> Int64 Int64)` applied to one arg is
        // `Int64`. Without this the application typed `Any`, leaving the enclosing function's return type
        // non-machine ("function return type has no machine representation"). A prelude sum like `Some`
        // resolved WITHOUT this because its ctor scheme threads the payload type; a USER sum's
        // payload-bound closure relies on this arm. (Applied to more args than the arrow takes stops at
        // the non-function tail — a fault reported elsewhere.)
        None => {
            let head_ty = type_of(db, head);
            if matches!(head_ty, Ty::Fn(_, _)) {
                let mut cur = head_ty;
                for _ in args {
                    match cur {
                        Ty::Fn(_, result) => cur = *result,
                        _ => break,
                    }
                }
                return cur;
            }
            trace!(target: "rcdzc::infer", head = head.0, "apply: head has no (meta t) scheme → Any");
            return Ty::Any;
        }
    };
    trace!(target: "rcdzc::infer", head = head.0, scheme = %scheme.ty.render_name(&db.name_ctx()), args = args.len(), "apply: instantiate head scheme");
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut subst = Subst::new();
    for &arg in args {
        // Peel one curried parameter: `cur` must be a function type; unify the arg into its parameter.
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                // FRESHEN the argument's free variables past the head's instantiation counter before
                // unifying: `type_of` types the arg with its OWN private `Fresh` (from 0), so an
                // under-constrained arg (a bare nullary variant `(None) : Option ?0`) shares variable
                // numbers with the head's instantiation (`Some : (-> ?0 (Option ?0))`). Without freshening
                // the two `?0`s alias and `?0 = Option ?0` trips the occurs-check, spuriously rejecting a
                // well-typed `(Some (None))`. Freshening makes them disjoint (`?0 = Option ?1`).
                let arg_ty = type_of(db, arg);
                let at = freshen_arg(db, &arg_ty, &mut fresh);
                // A unify failure here is a real type fault; ignore it for the VALUE (reported by
                // `type_errors`) and continue with the declared result so the shape stays sane.
                let _ = crate::unify::unify(&mut subst, &param, &at, &db.name_ctx());
                cur = *result;
            }
            // Applied to more args than it takes — not a function; the fault is reported elsewhere.
            _ => return Ty::Any,
        }
    }
    // A NULLARY PERFORM: an effect operation declared `(-> Unit T)` performed as `(E.op)` (no argument —
    // the `unit` domain is elided, the corpus surface). After the given args are peeled, if the head is
    // an effect operation and `cur` is still `(-> Unit result)`, the elided unit is the implicit argument
    // — so the performance's type is `result`, not the un-applied arrow. Without this a nullary perform
    // types as its arrow (a `Type`), and using it as a value (`(+ (E.op) 1)`) faults "unify Int64 with
    // Type". (Only for an effect op — an ordinary partial application keeps its arrow type.)
    let applied = subst.apply(&cur);
    if crate::eval::effect_op_of(db, head).is_some()
        && let Ty::Fn(param, result) = &applied
        && matches!(**param, Ty::Unit)
    {
        return subst.apply(result);
    }
    // `Value.decode : ∀a. Bytes → (Option a)` — its result `a` is UNCONSTRAINED by the argument (a
    // `Bytes`), so per-node bottom-up `type_of` leaves the application `(Option ?a)` with `?a` free. The
    // target `a` is fixed by the CALL-SITE EXPECTED TYPE — an enclosing `(: (Value.decode …) (Option T))`
    // annotation. But an annotation is the node's PARENT: its `type_of` arm unifies `annot_ty` with this
    // node's type in a LOCAL subst and returns the grounded type for the ANNOTATION node — it does NOT
    // thread the `a := T` binding back into THIS node's memoized `type_of`. `lower_value_decode` reads
    // `type_of(decode-node)` to build the shape descriptor, so an ungrounded `?a` there DECLINES
    // ("target type is unsolved") even though the annotation names it. So when this application is
    // `Value.decode` and its result is still `(Option <free>)`, CLIMB to the enclosing annotation (the same
    // parent-context grounding `literal_binop_context_ty` does for a deferred integer width) and unify the
    // annotation's type into the result. Keyed on the PRIM, not a name — generic-compiler-clean; no other
    // op needs this (an arg-grounded result is already concrete). An UNANNOTATED decode stays `(Option ?a)`
    // and declines at lower (the honest needs-annotation contract).
    // `Type.try-as : ∀a b. a → (Option b)` grounds `b` from the enclosing annotation identically —
    // its target is fixed by the CALL-SITE EXPECTED TYPE (`DESIGN-variable-arity-functions.md` §5), not
    // by the argument, so it shares Value.decode's parent-climb (both leave `(Option <free>)` per-node).
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(crate::resolved::Prim::ValueDecode | crate::resolved::Prim::TryAsType)
    ) && matches!(&applied, Ty::Sum { args, .. } if args.first().is_some_and(has_free_var))
        && let Some(expected) = annotation_context_ty(db, head)
    {
        let mut gsubst = Subst::new();
        if crate::unify::unify(&mut gsubst, &applied, &expected, &db.name_ctx()).is_ok() {
            return gsubst.apply(&applied);
        }
    }
    applied
}

/// Whether `ty` contains an unsolved type VARIABLE anywhere (a free `Ty::Var`). Used to detect a
/// `Value.decode` result `(Option ?a)` whose target is not yet grounded, so `type_of` climbs to the
/// annotation to fix it before `lower_value_decode` reads the node's type for its descriptor.
pub(crate) fn has_free_var(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) => true,
        Ty::Sum { args, .. } | Ty::Nominal { args, .. } => args.iter().any(has_free_var),
        Ty::List(e) => has_free_var(e),
        Ty::Tuple(es) => es.iter().any(has_free_var),
        Ty::Map(k, v) => has_free_var(k) || has_free_var(v),
        Ty::Record(fs) => fs.values().any(has_free_var),
        _ => false,
    }
}

/// The type an enclosing `(: <this> <Type>)` annotation grounds `id` to — climb PARENTS to the nearest
/// `Resolved::Annot` whose annotated `expr` is on the path from `id`, and reduce its `ty_expr` to a type
/// value. `None` if no annotation encloses `id` (then a `Value.decode` there stays ungrounded and declines
/// at lower — the honest needs-annotation contract). Mirrors `literal_binop_context_ty`'s parent-climb for
/// a deferred integer width — the shared "consult the context because per-node `type_of` can't thread it
/// back" path. Starts at `head` (the decode op node), whose first parent is the application node, whose
/// parent is the annotation; a non-annotation parent chain simply finds no annotation and returns `None`.
pub(crate) fn annotation_context_ty(db: &mut Db, id: StructId) -> Option<crate::ty::Ty> {
    let mut child = id;
    loop {
        let parent = db.parent_of(child)?;
        if let Resolved::Annot { expr, ty_expr } = resolved_of(db, parent) {
            // Only the annotation whose annotated expression is (an ancestor-or-self on the path from) THIS
            // node grounds it — `(: (Value.decode bs) (Option T))` has `expr` == the decode application node.
            if expr == child {
                return crate::eval::typeval_of(db, ty_expr);
            }
            return None;
        }
        // An annotated let-binder `((: <pat> T) <this>)` grounds its initializer exactly as a direct
        // `(: <this> T)` annotation does — the author named the type either way, so the idiomatic
        // `(let (((: p (Option T)) (Value.decode bs))) …)` must ground the decode identically to
        // `(: (Value.decode bs) (Option T))`. When `parent` is the binding PAIR whose SECOND element is
        // this node, the declared type on its `(: <pat> T)` LHS is the grounding type.
        if let Some(ty) = let_binder_annotation_ty(db, parent, child) {
            return Some(ty);
        }
        child = parent;
    }
}

/// The declared type of an annotated let-binder whose INITIALIZER is `init` — i.e. `pair` is the binding
/// pair `((: <pat> T) init)` (a two-element list whose second element is `init`) sitting in a `let`'s
/// bindings-list, with an `(: <pat> T)` LHS that reduces to a type value `T`. `None` for a bare-name
/// binding, a non-annotated LHS, or a `pair` that isn't a let binding at all. Shares the shape guard with
/// `annotated_let_binder_ty` (pair-in-bindings-list, `:`-form LHS), but grounds UNCONDITIONALLY — there is
/// no "initializer inferred type" to contradict here (the initializer is a `Value.decode` whose target is
/// still a free var, which is exactly what needs grounding), so the disagreement check that helper makes
/// (to suppress a contradictory body-use cascade) does not apply.
pub(crate) fn let_binder_annotation_ty(
    db: &mut Db,
    pair: StructId,
    init: StructId,
) -> Option<crate::ty::Ty> {
    let kv = match db.ast.get(pair) {
        crate::ast::Struct::List(kv) if kv.len() == 2 && kv[1] == init => kv.clone(),
        _ => return None,
    };
    let bindings_occ = db.parent_of(pair)?;
    crate::resolve::let_of_bindings_list(db, bindings_occ)?;
    let ann = db.ast.as_form(kv[0], ":").map(<[_]>::to_vec)?;
    if ann.len() != 2 {
        return None;
    }
    crate::eval::typeval_of(db, ann[1])
}

/// Apply a def SCHEME to `args`, returning the result type — instantiate it with fresh variables, then
/// peel one curried parameter per argument (unifying the arg's type into the parameter). The result is
/// the instantiated return type after substitution. Used to type a RECURSIVE call by its callee's
/// signature (which β-reduction can't type). Mirrors the operator-scheme application in `apply_type`.
pub(crate) fn apply_scheme_to_args(db: &mut Db, scheme: &Scheme, args: &[StructId]) -> Ty {
    let generic = !scheme.ty_vars.is_empty();
    // Type each argument up front so a GENERIC scheme can seed its instantiation counter PAST every
    // variable the args carry (below). A MONOMORPHIC scheme (no `ty_vars`) is unaffected by this.
    let arg_tys: Vec<Ty> = args.iter().map(|&a| type_of(db, a)).collect();

    // For a GENERIC scheme (recursive-generic monomorphization), the connection between the callee's
    // result variable and its parameter variable MUST survive so a caller that THREADS its own generic
    // param through this call (`(def (wrap m y) … (idr 2 y))` — `wrap`'s result is `idr`'s result is
    // `y`'s type) gets a result type equal to that param var, not a disconnected fresh one. The old path
    // `freshen_free`s each arg to dodge an occurs-check collision between the arg's from-0 vars and the
    // scheme's from-0 instantiation — but freshening a bare param-var arg is exactly what SEVERS the
    // connection (a three-level generic chain then decoupled result from param → "looped function result
    // has no machine rep"). Instead, seed the scheme's instantiation counter ABOVE every ty-var the args
    // mention, so the scheme's fresh vars cannot collide with an arg's var and NO freshening is needed —
    // the arg's canonical param var flows through the unify untouched. A generic def scheme has EMPTY
    // width/sign var lists (`compute_def_scheme`), so only ty-vars can collide, and `collect_free_vars`
    // finds them. A MONOMORPHIC scheme keeps the exact old freshen path (byte-identical).
    let mut fresh = Fresh::new();
    if generic {
        let mut arg_vars = Vec::new();
        for t in &arg_tys {
            t.collect_free_vars(&mut arg_vars);
        }
        if let Some(&max) = arg_vars.iter().max() {
            fresh.reserve(max + 1); // instantiate above every arg var → no collision without freshening
        }
    }
    let mut cur = crate::unify::instantiate(scheme, &mut fresh);
    let mut subst = Subst::new();
    for (i, at_raw) in arg_tys.into_iter().enumerate() {
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                // A MONOMORPHIC scheme freshens the arg (dodges the from-0 occurs-check collision, see
                // `apply_type`); a GENERIC scheme skips it (the seeded counter already avoids collision)
                // so a threaded param var stays connected to the callee's result var.
                let at = if generic {
                    at_raw
                } else {
                    crate::unify::freshen_free(&at_raw, &mut fresh)
                };
                // A CLOSURE argument — `(fn (s) s)` identity, `(fn (x) (tuple x x))` aggregate — types
                // bottom-up with an `Any` at every unannotated-param position (`(-> Any Any)`, `(-> Any
                // (Tuple Any Any))`). The DOMAIN is pinned by a sibling arg (`gmap`'s `it` element), and
                // once it is concrete we RE-SOLVE the closure body UNDER that domain
                // (`solved_lambda_arrow_under`) to recover the closure's fully-CONCRETE arrow — tying the
                // callee's result var (`gmap`'s `b`, hence its `Iter b` result) to the closure's real
                // result. Unify THAT recovered arrow, NOT the bottom-up `at`:
                //
                // Unifying `at` first POISONS `b` for an AGGREGATE result. A SCALAR-`Any` result
                // (`(-> Any Any)`) leaves `b` free — `unify(?b, Any)` hits the `Any`-poison arm
                // (`unify.rs`), no bind — so the recovery below could refine it. But a COMPOUND result
                // (`(Tuple Any Any)`) is NOT bare `Any`: `unify(?b, (Tuple Any Any))` binds `?b :=
                // (Tuple Any Any)`, and then the recovery's re-unify against `(Tuple Int64 Int64)` cannot
                // refine the inner `Any`s (Any-absorbs). So the OUTER call node types `(GIter (Tuple Any
                // Any))`, the tuple elements ground to `Unit`, and a consumer specialized off THIS node
                // type (`count(gmap …)`) takes `GIter<((),())>` while `gmap` itself specializes
                // `GIter<(i64,i64)>` → rust E0308 (wasm erases the element, so it is a rust-visible
                // miscompile). Unifying the recovered CONCRETE arrow instead binds `b` to `(Tuple Int64
                // Int64)` directly. The recovered arrow's domain equals the pinned domain `at` would
                // supply, so skipping `at` here loses nothing. Falls back to `at` when recovery can't fire
                // (domain not yet concrete, non-lambda arg, or an unrecoverable body) — byte-identical to
                // the prior path for every non-recovered shape.
                let mut tied_via_recovery = false;
                if at.has_any()
                    && let Some(node) = args.get(i).copied()
                    && let (Some(lam_params), Some(lam_body)) = (
                        crate::eval::lambda_params_of(db, node),
                        crate::eval::lambda_body(db, node),
                    )
                {
                    let pinned = subst.apply(&param);
                    if let Ty::Fn(dom, _) = &pinned
                        && !matches!(**dom, Ty::Any)
                        && !dom.has_free_var()
                        && let Some(solved) =
                            solved_lambda_arrow_under(db, &lam_params, lam_body, &pinned)
                        && !solved.has_any()
                    {
                        let _ = crate::unify::unify(&mut subst, &param, &solved, &db.name_ctx());
                        tied_via_recovery = true;
                    }
                }
                if !tied_via_recovery {
                    let _ = crate::unify::unify(&mut subst, &param, &at, &db.name_ctx());
                }
                cur = *result;
            }
            _ => return Ty::Any,
        }
    }
    subst.apply(&cur)
}

/// The instantiated PARAMETER-ARROW type at position `pos` when `callee` is applied to `args` — the arrow
/// the callee's scheme requires of the argument at `pos`, with the OTHER (determined) sibling arguments
/// unified in so a scheme type variable shared across parameters is pinned. Used by `type_specialize` to
/// recover a bare CLOSURE argument's expected arrow at a call site: a recursive-generic transformer
/// `gmap : (Iter a) → (a → b) → (Iter b)` applied to a concrete `it : Iter String` pins `f`'s expected
/// arrow to `(-> String ?b)`, so an IDENTITY closure `(fn (s) s)` — whose body cannot type `s` bottom-up —
/// solves `s : String` from that domain and its identity body then determines `b = String`. The closure's
/// OWN argument (at `pos`) is NOT unified in (its `(-> Any …)` type would re-introduce the hole the
/// recovery exists to fill); every other DETERMINED argument is. Returns `None` when `callee` has no
/// scheme, the arrow runs out before `pos`, or the recovered slot is not itself an arrow (nothing to
/// recover). The domain MAY be concrete while the result stays a free var (the closure body solves it).
pub(crate) fn instantiated_param_arrow(
    db: &mut Db,
    callee: usize,
    args: &[StructId],
    pos: usize,
) -> Option<Ty> {
    let scheme = def_scheme(db, callee)?;
    let arg_tys: Vec<Ty> = args.iter().map(|&a| type_of(db, a)).collect();
    let generic = !scheme.ty_vars.is_empty();
    let mut fresh = Fresh::new();
    if generic {
        let mut arg_vars = Vec::new();
        for t in &arg_tys {
            t.collect_free_vars(&mut arg_vars);
        }
        if let Some(&max) = arg_vars.iter().max() {
            fresh.reserve(max + 1);
        }
    }
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut subst = Subst::new();
    let mut recovered: Option<Ty> = None;
    for (i, at_raw) in arg_tys.into_iter().enumerate() {
        let applied = subst.apply(&cur);
        let Ty::Fn(param, result) = applied else {
            break;
        };
        if i == pos {
            // The closure's own slot — capture the expected arrow AFTER the sibling unifications, but do
            // NOT unify the closure's bare `(-> Any …)` type in (it would re-introduce the hole).
            recovered = Some(*param);
        } else {
            let at = if generic {
                at_raw
            } else {
                crate::unify::freshen_free(&at_raw, &mut fresh)
            };
            // Only a DETERMINED sibling pins the scheme's shared vars; an undetermined one adds nothing.
            if !matches!(at, Ty::Any) && !at.has_free_var() {
                let _ = crate::unify::unify(&mut subst, &param, &at, &db.name_ctx());
            }
        }
        cur = *result;
    }
    let arrow = subst.apply(&recovered?);
    matches!(arrow, Ty::Fn(_, _)).then_some(arrow)
}

/// The fully-solved arrow of a bare lambda `(params, body)` when its parameters' EXPECTED types are known
/// from the call context (the `expected` arrow, e.g. `(-> String ?b)` recovered by
/// [`instantiated_param_arrow`]). Binds each param to the corresponding expected DOMAIN before typing the
/// body, so a pass-through body (`(fn (s) s)` — identity) whose result the bottom-up solve cannot pin
/// takes its result FROM the domain: `s : String` ⇒ body `s : String` ⇒ arrow `(-> String String)`. For a
/// param the expected arrow does not determine (a hole at that slot), fall back to the body-only solve
/// (`solve_lambda_param_ty`), so a genuinely-unconstrained param still declines rather than inventing a
/// type. `None` if the counts do not line up as an arrow. This is the closure-body result←domain flow the
/// plain `solved_lambda_arrow` omits (it solves each param from the body ALONE).
pub fn solved_lambda_arrow_under(
    db: &mut Db,
    params: &[StructId],
    body: StructId,
    expected: &Ty,
) -> Option<Ty> {
    // Peel the expected arrow into per-parameter expected domains (as many as it provides).
    let mut expected_doms: Vec<Ty> = Vec::new();
    let mut cur = expected.clone();
    for _ in 0..params.len() {
        match cur {
            Ty::Fn(p, r) => {
                expected_doms.push(*p);
                cur = *r;
            }
            _ => break,
        }
    }
    // Bind each param to its expected domain (when concrete) in a local subst, so typing the body reads
    // the param at that type. A param whose expected domain is a hole is left to the body-only solve.
    let mut subst = Subst::new();
    // ALSO seed each param's concrete expected domain into `db.param_types` (save + restore). The var-subst
    // above only rewrites `Ty::Var`s, but a BARE lambda param types as `Ty::Any` (not a var — `type_of`'s
    // Lambda arm), which `subst.apply` cannot touch. So an AGGREGATE body over the param — `(fn (x) (tuple
    // x x))`, `(fn (x) (Cons x Nil))` — types its element positions at `Any` and stays `(Tuple Any Any)`
    // even after the subst, so the recovered arrow keeps an `Any` hole and `type_specialize` DECLINES (the
    // gmap `int→tuple` + `string→string` CDZ0201 — `cdz check` green, `cdz test` red). Seeding
    // `db.param_types[x] = Int64` makes `type_of(x)` return the domain DIRECTLY, so the tuple types `(Tuple
    // Int64 Int64)`. Restored below so the transient seeding never leaks into another def's solve.
    let mut seeded: Vec<(StructId, Option<Ty>)> = Vec::new();
    for (i, &p) in params.iter().enumerate() {
        let occ = crate::eval::param_name_occ(db, p);
        if let Some(dom) = expected_doms.get(i)
            && !matches!(dom, Ty::Any)
            && !dom.has_free_var()
        {
            let pv = type_of(db, occ);
            // Unify the param's own (var/Any) type with the expected domain, so a body reference to it
            // reads the concrete domain through the subst.
            let _ = crate::unify::unify(&mut subst, &pv, dom, &db.name_ctx());
            // Seed the concrete domain so an aggregate body reads the param's type directly (not `Any`).
            // Only when the param is currently a HOLE (`Any`/var) — an already-concrete param is untouched.
            if matches!(pv, Ty::Any) || pv.has_free_var() {
                seeded.push((occ, db.param_types.insert(occ, dom.clone())));
            }
        }
    }
    // Type the body, then read the result + each param through the subst (a pass-through body's result is
    // now the bound domain). Curry right-to-left, exactly like `solved_lambda_arrow`.
    let result = subst.apply(&type_of(db, body));
    for (occ, prev) in seeded {
        match prev {
            Some(t) => db.param_types.insert(occ, t),
            None => db.param_types.remove(&occ),
        };
    }
    Some(
        params
            .iter()
            .enumerate()
            .rev()
            .fold(result, |acc, (i, &p)| {
                let occ = crate::eval::param_name_occ(db, p);
                let pt = match expected_doms.get(i) {
                    Some(dom) if !matches!(dom, Ty::Any) && !dom.has_free_var() => dom.clone(),
                    _ => match type_of(db, occ) {
                        Ty::Any => solve_lambda_param_ty(db, occ, body),
                        t => t,
                    },
                };
                Ty::Fn(Box::new(pt), Box::new(acc))
            }),
    )
}

/// Which built-in fallible sum a type is, for the `?`/`try` operator (`DESIGN-try-operator-rcdzc.md`).
/// `Option` short-circuits with `None`; `Result` short-circuits with `Err`. Identified STRUCTURALLY by
/// the declaration's variant NAMES (`Some`/`None`, `Ok`/`Err`) — never by the type's spelled name — so
/// the built-in prelude sums are recognized without a hard-coded name key (the no-keys-outside-the-prelude
/// rule; the same variant-name scan `option_discs`/`result_discs` do in `lower`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FallibleKind {
    /// A `(type Option (Some a) None)` — `?` unwraps `Some a`, short-circuits on `None`.
    Option,
    /// A `(type Result (Ok a) (Err b))` — `?` unwraps `Ok a`, short-circuits carrying `Err b`.
    Result,
}

/// Classify a solved type as a built-in fallible sum (`Option`/`Result`) for the `?` operator, returning
/// its kind together with its SUCCESS PAYLOAD type (the `a` a `?` yields: `Some`'s / `Ok`'s payload) — for
/// `Result` also the error type (`Err`'s payload). `None` when `ty` is not one of the two built-in
/// fallible sums (a non-sum, a user sum, or an `Option`/`Result` whose args are not yet solved to the
/// arity its variant set implies). Recognizes the sum by its declaration's variant names, then reads the
/// payload types off the positional `Ty::Sum::args` (params are in first-appearance order, so `Option`'s
/// `a` and `Result`'s `Ok`/`Err` payloads are `args[0]`/`args[1]`).
pub(crate) fn fallible_shape(db: &Db, ty: &Ty) -> Option<(FallibleKind, Ty, Option<Ty>)> {
    let Ty::Sum { decl, args, .. } = ty else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(*decl)?;
    let names: Vec<&str> = decl_ref.variants.iter().map(|v| v.name.as_str()).collect();
    let has = |n: &str| names.contains(&n);
    if names.len() == 2 && has("Some") && has("None") {
        let payload = args.first().cloned().unwrap_or(Ty::Any);
        Some((FallibleKind::Option, payload, None))
    } else if names.len() == 2 && has("Ok") && has("Err") {
        let ok = args.first().cloned().unwrap_or(Ty::Any);
        let err = args.get(1).cloned();
        Some((FallibleKind::Result, ok, err))
    } else {
        None
    }
}

/// The FALLIBLE BOUNDARY a `?`/`try` at `try_node` short-circuits to (`DESIGN-try-operator-rcdzc.md` §4
/// v1: the enclosing FUNCTION). Walks up the ancestor chain from `try_node` to the nearest enclosing
/// function BODY — a top-level/module/do-local def body (`def_index_by_body`) or a `(fn params body)`
/// lambda body (the ancestor that is child position 1 of a `fn` form) — and returns THAT BODY's type,
/// which is the boundary type `T_B` (Cadenza declares a return type by ASCRIBING the body `(: body T)`,
/// so the body's type IS the function's result type). `None` if the `?` is not inside any function body.
///
/// No circularity: `type_of(Try)` reads only its OPERAND's type (never the boundary), so demanding the
/// body's type from inside a `?` under it does not re-enter this `?`'s own type. The walk stops at the
/// FIRST enclosing function so a `?` in a helper targets the helper (Rust-identical), not an outer caller.
pub(crate) fn enclosing_boundary_ty(db: &mut Db, try_node: StructId) -> Option<Ty> {
    let mut cur = try_node;
    while let Some(parent) = db.parent_of(cur) {
        // `cur` is a def body (top-level, module-member, or do-local) — its type is the boundary.
        if db.def_index_by_body(cur).is_some() {
            return Some(type_of(db, cur));
        }
        // `cur` is a `(fn params body)` lambda's BODY — position 1 of a `fn` form. Its type is the
        // lambda's result, the boundary for a `?` in the lambda body.
        if db.ast.head_name(parent) == Some("fn")
            && let crate::ast::Struct::List(kids) = db.ast.get(parent)
            && kids.get(1) == Some(&cur)
        {
            return Some(type_of(db, cur));
        }
        cur = parent;
    }
    None
}

/// The PAYLOAD type of a variant `variant_head` at the scrutinee's instantiation `scrut_ty` — the type
/// a match-arm payload binder takes. The ctor's scheme is `∀params. payload… → Sum params`;
/// instantiate it, unify its RESULT (`Sum ?params`) against `scrut_ty` (a `Ty::Sum{args}` — the solved
/// scrutinee type) to solve the fresh params from the scrutinee's concrete args, then return the
/// (substituted) FIRST payload. So `(match (s : Option Int64) ((Some x) …))` reads `Some`'s payload as
/// `Int64`, not a free var. For a MONOMORPHIC ctor the scheme has no vars and the result unify is a
/// no-op, so the payload is the declared type directly. `None` if the ctor has no payload (a nullary
/// variant — no binding) or its scheme is not a single-payload arrow.
pub(crate) fn payload_ty_at_instantiation(
    db: &mut Db,
    variant_head: StructId,
    scrut_ty: &Ty,
) -> Option<Ty> {
    let mut fresh = Fresh::new();
    // SEED the instantiation counter PAST every free variable the SCRUTINEE type carries, so the ctor
    // scheme's fresh vars cannot COLLIDE with them. `scrut_ty` is not always ground: a recursive call's
    // result is typed by `apply_scheme_to_args`, which instantiates the callee's scheme from 0 — so a
    // `Result A ?0` scrutinee (an error slot no branch of the recursive callee fixes) carries `?0`. The
    // ctor scheme also instantiates from 0 (`Err : ∀a b. b → Result a b` → `?1 → Result ?0 ?1`), so its
    // `?0`/`?1` alias the scrutinee's `?0`: unifying `Result ?0 ?1` against `Result A ?0` first solves
    // `?0 = A`, then `?1 = ?0 = A` — binding `et` to `A` instead of the scrutinee's error var, which then
    // read the whole match as `Result A A` (a spurious CDZ0203 "if branches differ", and downstream a
    // backend "no local slot" decline). Reserving above the scrutinee's vars keeps the two var domains
    // DISJOINT, so `Err`'s payload solves to the scrutinee's own error var. A GROUND scrutinee has no free
    // var and reserves nothing (byte-identical to the old from-0 path). Mirrors `apply_scheme_to_args`,
    // which seeds the same way to keep a recursive-generic result var connected.
    {
        let mut scrut_vars = Vec::new();
        scrut_ty.collect_free_vars(&mut scrut_vars);
        if let Some(&max) = scrut_vars.iter().max() {
            fresh.reserve(max + 1);
        }
    }
    let scheme = crate::eval::scheme_of(db, variant_head, &mut fresh)?;
    let inst = crate::unify::instantiate(&scheme, &mut fresh);
    // The ctor type is a curried arrow `payload… → result`. Peel EVERY arrow: a single-payload variant
    // has one (`p → Sum`); a MULTI-payload variant curries (`p0 → p1 → Sum`) and its payloads are boxed
    // as ONE tuple handle (the same representation `(Cons (tuple p0 p1))` builds), so its bound payload
    // TYPE is the `Ty::Tuple(p0, p1, …)` that the `[Payload, Elem(i)]` access path indexes into.
    let mut payloads = Vec::new();
    let mut cur = inst;
    let result = loop {
        match cur {
            Ty::Fn(p, r) => {
                payloads.push(*p);
                cur = *r;
            }
            other => break other,
        }
    };
    if payloads.is_empty() {
        return None; // nullary (bare sum) — no payload to bind
    }
    // Solve the scheme's params from the scrutinee's concrete instantiation: unify the ctor's RESULT
    // (`Sum ?a`) against the scrutinee type (`Sum Int64`).
    let mut subst = Subst::new();
    let _ = crate::unify::unify(&mut subst, &result, scrut_ty, &db.name_ctx());
    let payload = if payloads.len() == 1 {
        payloads.pop().unwrap()
    } else {
        Ty::Tuple(payloads.into())
    };
    Some(subst.apply(&payload))
}

/// The integer TEXT a FLOAT LITERAL denotes when it is integer-valued AND fits the `expected` integer
/// type — the replacement for the mirror of the `of-int` coercion: an `Int`-expected position given a
/// float literal (`(+ 2 2.0)`). A reader-normalized `Decimal` is integer-valued exactly when its base-10
/// `exponent` is ≥ 0 (its significand carries no trailing zeros — `2.0` → `{[2], exp:0}`, `300.0` →
/// `{[3], exp:2}`, but `2.5` → `{[25], exp:-1}`). We reconstruct the integer as `significand · 10^exp`,
/// then REJECT it unless the value fits `expected`'s `(signed, width)` — so the suggested `2.0` → `2`
/// type-checks in ONE shot (the D7/D9 one-shot rule: never suggest a spelling that just cascades to the
/// next mismatch, here a CDZ0302 out-of-range). No prelude float→int conversion exists, so a NON-integer
/// float (`2.5`) or an out-of-range one gets `None` — an honest "no clean fix" rather than a wrong one
/// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). Returns the minus-signed
/// decimal text (`-0.0` → `0`, since the integer zero has no sign).
pub(crate) fn integer_text_of_float_literal(
    d: &crate::ast::Decimal,
    expected: crate::ty::IntTy,
) -> Option<String> {
    // Only a non-negative exponent is a whole number (a normalized `Decimal` has no trailing-zero
    // significand, so a fractional value keeps a negative exponent).
    if d.exponent < 0 {
        return None;
    }
    // Base-256 significand → base-10 digit string (Horner, little-endian digits printed reversed), the
    // same conversion `Decimal::to_f64_bits` performs. An empty significand is zero.
    let mut digits: Vec<u8> = vec![0];
    for &byte in &d.significand {
        let mut carry = byte as u32;
        for dig in digits.iter_mut() {
            let v = (*dig as u32) * 256 + carry;
            *dig = (v % 10) as u8;
            carry = v / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    let mut s: String = digits.iter().rev().map(|d| (b'0' + d) as char).collect();
    let trimmed = s.trim_start_matches('0');
    s = if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    };
    // Multiply by 10^exponent = append that many zeros (unless the value is zero).
    if s != "0" {
        for _ in 0..d.exponent {
            s.push('0');
        }
    }
    // Build the value to range-check it against the expected width. Parse the decimal digit string into a
    // big-endian magnitude (Horner: acc = acc*10 + digit), then apply the sign — the inverse of the digit
    // build above, reused so the width check sees the exact integer.
    let mut mag: Vec<u8> = Vec::new(); // little-endian base-256 during build
    for ch in s.bytes() {
        let mut carry = (ch - b'0') as u32;
        for byte in mag.iter_mut() {
            let v = (*byte as u32) * 10 + carry;
            *byte = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            mag.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    while mag.last() == Some(&0) {
        mag.pop();
    }
    mag.reverse();
    let negative = d.negative && !mag.is_empty(); // zero is never negative
    let value = crate::ast::IntValue {
        negative,
        magnitude: mag,
    };
    if !value.fits_width(expected.ground_signed(), expected.ground_width()) {
        return None;
    }
    Some(if negative { format!("-{s}") } else { s })
}

/// A homogeneity mismatch between an INTEGER LITERAL element and a FLOAT type has the SAME one-shot
/// repair the annotation site's `(: 3 Float64)` does: rewrite the integer literal `n` as a FLOAT
/// literal `n.0`, so a `(list 1 2.0)` / `(list 1.0 2)` unifies at the float type rather than staying a
/// no-silent-promotion reject. `elem`'s type is `elem_ty`; `other_ty` is the type it clashed with. The
/// fix applies exactly when the clashing pair is {integer literal, float}: `elem` is a bare integer
/// LITERAL (an atom Int leaf — a computed integer expression cannot be retyped in one edit) whose type
/// is `Ty::Int`, and `other_ty` is `Ty::Float`. Returns a `ReplaceNode` fix editing the literal token
/// (`1` → `1.0`), or `None` when the pair is not this shape. Mirrors the annotation-site literal-retype
/// (`Some((format!("{n}.0"), …))`) so the two sites suggest the identical rewrite.
pub(crate) fn float_literal_retype_fix(
    db: &mut Db,
    elem: StructId,
    elem_ty: &Ty,
    other_ty: &Ty,
) -> Option<crate::diag::Fix> {
    if !(matches!(elem_ty, Ty::Int(_)) && matches!(other_ty, Ty::Float(_))) {
        return None;
    }
    let crate::ast::Struct::Atom(lid) = db.ast.get(elem) else {
        return None;
    };
    let crate::ast::Leaf::Int { value, .. } = db.ast.leaf(*lid).clone() else {
        return None;
    };
    let n = value.to_i128()?;
    Some(crate::diag::Fix::replace_heuristic(elem, format!("{n}.0")))
}

/// The sole variant NAME of an ERASABLE SINGLE-VARIANT NEWTYPE `ty` (a `Ty::Nominal` whose declaration
/// has exactly one variant) — the constructor to UNWRAP it by, `(match v ((<Name> n) n))`. The INVERSE of
/// `wrap_variant_for`: where that names the ctor to WRAP a bare value into a newtype, this names the ctor
/// to UNWRAP a newtype back to its underlying value (the repair for a nominal-boundary comparison
/// `(= u 5)` — unwrap `u` to compare the `Int64` inside). `None` unless `ty` is a nominal whose decl has
/// EXACTLY ONE variant (a multi-variant sum has no single unwrap; a non-nominal has nothing to unwrap).
pub(crate) fn newtype_unwrap_variant(db: &mut Db, ty: &Ty) -> Option<String> {
    let decl = match ty {
        Ty::Nominal { decl, .. } => *decl,
        _ => return None,
    };
    let t = db.type_decl_by_occ(decl)?;
    match t.variants.as_slice() {
        [only] => Some(only.name.clone()),
        _ => None,
    }
}

/// If `expected` is a SUM type with a SINGLE-payload variant whose payload type (at `expected`'s
/// instantiation) agrees with `actual`, the variant's constructor NAME — the "try wrapping the
/// expression in `Some`" suggestion (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
/// A Fix). Derives the name GENERICALLY from the sum's declaration (which variant's payload fits),
/// never a hard-coded `Some`/`Ok` (the no-keys-outside-the-prelude rule). `None` when `expected` is not
/// a sum, or no variant's single payload matches — so a wrap is offered only when it would actually
/// resolve the mismatch. When TWO variants could each wrap the value — `(Result Int64 Int64)` given an
/// `Int64`, where both `Ok` and `Err` fit — the choice is AMBIGUOUS and `None` is returned rather than
/// guess one; a forced (unique) match is required. Variants are scanned in declaration order → deterministic.
pub(crate) fn wrap_variant_for(db: &mut Db, expected: &Ty, actual: &Ty) -> Option<String> {
    // Both a boxed `Ty::Sum` and an erased single-variant `Ty::Nominal` newtype offer the wrap: `(: n
    // Box)` for `(type Box (Wrap Int64))` suggests `(Wrap n)` exactly as `(: n Option)` suggests
    // `(Some n)` — the ctor is derived from the declaration either way.
    let decl = nominal_or_sum_decl(expected)?;
    // Snapshot (name, ctor) pairs first — `payload_ty_at_instantiation` borrows `db` mutably, so we
    // cannot hold a `&TypeDecl` across the calls.
    let variants: Vec<(String, StructId)> = db
        .type_decl_by_occ(decl)?
        .variants
        .iter()
        .filter_map(|v| v.ctor.map(|c| (v.name.clone(), c)))
        .collect();
    let mut hit: Option<String> = None;
    for (name, ctor) in variants {
        // A single-payload variant whose payload agrees with `actual` → wrapping the value in it yields
        // the expected sum. (A multi-payload variant's payload is a tuple, which a bare value never
        // matches, so it is naturally excluded.)
        if let Some(payload) = payload_ty_at_instantiation(db, ctor, expected)
            && payload.agrees_with(actual)
        {
            if hit.is_some() {
                return None; // two variants could each wrap it — ambiguous, don't guess
            }
            hit = Some(name);
        }
    }
    hit
}

/// An actionable message TAIL (no fix) when the mismatch is the common "used an optional value directly"
/// shape: `actual` is `(Option T)` and `expected` is exactly its payload `T` — a fallible read (`List.at`,
/// `String.at`, `from-bytes`) whose optional result was used where the bare payload was wanted. An
/// `(Option T)` has NO total unwrap (it is eliminated only by matching its `None` case, which is the
/// author's decision), so there is no mechanical fix — but the diagnostic can still say HOW to fix it
/// rather than only naming two types. `None` unless `actual` is `(Option <expected>)` (so a genuine
/// unrelated mismatch — `Int64` vs `String` — is untouched). Detects Option by the built-in sum's `name`
/// plus a single type argument (the `(Option a)` prelude shape), payload compared by `agrees_with` so a
/// deferred/`Any` payload still matches. An honest "match it" route where no one-shot spelling exists —
/// the diagnostic-carries-a-route-to-a-fix rule of `spec/capabilities/diagnostics.md`.
/// The `(module, member)` names of an application head that is a `(. Module member)` MEMBER ACCESS with
/// BOTH parts bare names — `(. List push)` → `("List", "push")`. Used to name a prelude operation in a
/// wrong-argument-type diagnostic ("`List.push` expects an argument of type …"), instead of the generic
/// unify mismatch. `None` when the head is not that shape (a bare operator atom `+`, a user-fn name, a
/// computed head) — those take their own diagnostic paths, so this fires only for a member-op call.
pub(crate) fn member_op_head_name(db: &Db, head: StructId) -> Option<(String, String)> {
    let tail = db.ast.as_form(head, ".")?;
    let module = db.ast.as_name(*tail.first()?)?.to_string();
    let member = db.ast.as_name(*tail.get(1)?)?.to_string();
    Some((module, member))
}

/// The source NAME of an application head that is a bare VARIANT CONSTRUCTOR — `(Mk 1 2)` → `"Mk"`. Used
/// to name the constructor in an OVER-APPLICATION diagnostic ("`Mk` takes 2 arguments, but 3 were given")
/// so a bare ctor reads as well as the member-access spelling `(. P Mk)` does (which `member_op_head_name`
/// already covers). `None` when the head is not a bare-name variant constructor — an ordinary function, an
/// operator, or a member-access head (that path names itself). Reads the head's source name first, then
/// confirms it constructs a variant via `variant_disc_of` (the same predicate the wrong-type-payload
/// branch uses) — GENERIC, no hard-coded ctor list (`no-keys-outside-the-prelude`).
pub(crate) fn variant_ctor_head_name(db: &mut Db, head: StructId) -> Option<String> {
    let name = db.ast.as_name(head)?.to_string();
    crate::eval::variant_disc_of(db, head).map(|_| name)
}

/// The callee's source NAME for a lambda-application head, when it is a plain bare-name function
/// reference — `(h true)` → `"h"`. Used to name the function in a wrong-typed-argument diagnostic
/// ("argument to `h` …"). `None` for an anonymous lambda applied directly (`((fn (x) …) 5)`), a
/// member-access head, or any non-name head — those callers fall back to the un-named phrasing. Reads the
/// head's source spelling (`db.ast.as_name`), so it works for a top-level def and a scoped local alike.
pub(crate) fn callee_head_name(db: &Db, head: StructId) -> Option<String> {
    db.ast.as_name(head).map(str::to_string)
}

/// Whether the application head names a NAMED definition — a top-level `(def (f …) …)`, a lambda-valued
/// `(def f (fn …))`, or a module member reached by name — as opposed to an INLINE / anonymous `(fn …)`
/// literal applied in place. A named def's body is INDEPENDENTLY collected by `compile::collect_faults`
/// (so a call site must NOT re-report its body faults — the baseline-subtraction de-duplicates them); an
/// inline lambda's body is checked ONLY at its β-reduction call site, so its faults must NOT be
/// subtracted. Follows a `Ref`/`Member` chain to the def the way `callee_def_index_for_infer` does, but
/// answers "is this a named def?" via the head resolving to a `Lambda` reached THROUGH a name — robust for
/// a lambda-valued def whose registered body node differs from the head's resolved inner body (which is
/// exactly what makes `callee_def_index_for_infer`'s `def_index_by_body` miss it).
pub(crate) fn named_callee_head(db: &mut Db, head: StructId) -> bool {
    // A direct name application `(helper x)` — the head IS a name occurrence.
    if db.ast.as_name(head).is_some() {
        return true;
    }
    // A `Ref`/`Member` chain to a named def (`(m.f x)`, or a name that resolved to a Ref). An inline
    // `(fn …)` head resolves straight to `Lambda` with no intervening name → false.
    match crate::resolve::resolved_of(db, head) {
        crate::resolved::Resolved::Ref { value } if value != head => named_callee_head(db, value),
        crate::resolved::Resolved::Member { .. } => true,
        _ => false,
    }
}

/// The wrong-typed-CALL-ARGUMENT message: the argument's type does not satisfy the parameter's declared
/// type at a call site. Framed as an ARGUMENT ("this argument is a Bool, but …"), NOT an annotation — the
/// author wrote no annotation — the same phrasing the synthesized-parameter-annotation path (M106) uses,
/// so a referenced-param arg (reported by that path) and an UNREFERENCED-param / recursive-callee arg
/// (reported here at the call-site unify, step 1) read identically instead of the raw "type mismatch: X
/// and Y must be the same type here" the unify produced. When the callee + parameter NAMES are known
/// (a bare-name function with a named parameter) it names them — "argument to `h`'s parameter `a`" —
/// which the synthesized-annotation path cannot (it has only the annotation node). `expected` is the
/// parameter's declared type, `actual` the argument's type; `tail` carries an optional structural hint.
pub(crate) fn call_argument_mismatch_message(
    callee: Option<&str>,
    param: Option<&str>,
    expected: &Ty,
    actual: &Ty,
    tail: &str,
    ncx: &NameCtx,
) -> String {
    let subject = match (callee, param) {
        (Some(f), Some(p)) => format!("the argument for `{f}`'s parameter `{p}`"),
        (Some(f), None) => format!("the argument to `{f}`"),
        _ => "this argument".to_string(),
    };
    format!(
        "{subject} is {}, but a value of type {} is expected here{tail}",
        actual.render_with_article(ncx),
        expected.render_name(ncx),
    )
}

pub(crate) fn option_payload_mismatch_hint(
    ncx: &NameCtx,
    expected: &Ty,
    actual: &Ty,
) -> Option<String> {
    if let Ty::Sum { decl, args } = actual
        && ncx.name_of(*decl) == Some("Option")
        && let [payload] = &args[..]
        && payload.agrees_with(expected)
    {
        return Some(
            " — the value is optional; match it to handle the absent (`None`) case, \
             e.g. `(match v ((Some x) …) ((None) …))`"
                .to_string(),
        );
    }
    None
}

/// Drill through two SAME-SHAPE nested compounds (records with the same field set, tuples of the same
/// arity) to the DEEPEST single leaf where the types actually differ, returning the relative access PATH
/// to that leaf and the leaf's expected-vs-actual types. `(Record (a (Record (b Int64))))` vs `(… (b
/// Bool))` drills to `("a.b", Int64, Bool)` so a caller can say "field `a.b` should be Int64, but this one
/// is Bool" instead of re-rendering the whole differing sub-record. Segments are dotted — a record field
/// contributes its name, a tuple position its 0-based index (`pt.1` = field `pt`, element 1) — matching
/// the member-access spelling. `None` when the two types are NOT further drillable at this level: a scalar
/// (or other) leaf, a field-SET / arity difference, or a cross-kind clash — in those cases the caller
/// keeps its own single-level phrasing (naming the immediate member + rendering the two sub-types, whose
/// difference the render then shows). Terminates because each step descends into a strictly smaller
/// structural sub-type (a `Ty::Nominal`/collection is a non-drillable leaf, so a recursive nominal stops).
pub(crate) fn deep_leaf_delta<'a>(want: &'a Ty, got: &'a Ty) -> Option<(String, &'a Ty, &'a Ty)> {
    match (want, got) {
        (Ty::Record(w), Ty::Record(g)) => {
            // Only drill a SAME field-set record; a field-set difference is not a single-leaf type diff
            // (the caller renders the sub-record, whose missing/extra field the render shows).
            if w.len() != g.len() || !w.keys().all(|k| g.contains_key(k)) {
                return None;
            }
            let (k, wt) = w
                .iter()
                .find(|(k, wt)| g.get(k).is_some_and(|gt| !wt.agrees_with(gt)))?;
            let gt = &g[k];
            Some(match deep_leaf_delta(wt, gt) {
                Some((sub, lw, lg)) => (format!("{}.{sub}", k.name), lw, lg),
                None => (k.name.to_string(), wt, gt),
            })
        }
        (Ty::Tuple(w), Ty::Tuple(g)) => {
            if w.len() != g.len() {
                return None; // an arity difference is reported at this level, not drilled
            }
            let (i, (wt, gt)) = w
                .iter()
                .zip(g.iter())
                .enumerate()
                .find(|(_, (wt, gt))| !wt.agrees_with(gt))?;
            Some(match deep_leaf_delta(wt, gt) {
                Some((sub, lw, lg)) => (format!("{i}.{sub}"), lw, lg),
                None => (i.to_string(), wt, gt),
            })
        }
        _ => None,
    }
}

/// A coercion FIX for a numeric leaf buried inside a directly-written COMPOUND value whose annotated type
/// differs only at that leaf — `(record (x 5))` annotated `(Record (x Float64))` retypes the inner `5` →
/// `5.0`, `(tuple 1 2)` vs `(Tuple Int64 Float64)` retypes the `2`, `(list 5)` vs `(List Float64)` retypes
/// the `5`. The structural-delta hint NAMES the leaf (`field \`x\` should be Float64 …`); this gives it the
/// same one-shot repair a bare `(: 5 Float64)` gets, anchored at the INNER value node. Drills the VALUE
/// expression (`expr`) in lockstep with the type delta (record field / tuple position / list element),
/// mirroring `deep_leaf_delta`'s type walk, to reach the leaf value node, then defers to
/// `numeric_text_coercion_fix` (the same helper the bare annotation uses). `None` unless the value is a
/// directly-written compound literal whose single differing leaf has a numeric coercion — a value bound to
/// a name / returned from a call has no inner literal to edit, and a non-numeric leaf (Bool vs Int) has no
/// coercion (its structural-delta message stands alone).
pub(crate) fn compound_inner_coercion_fix(
    db: &mut Db,
    expr: StructId,
    expected: &Ty,
    actual: &Ty,
) -> Option<Fix> {
    match (expected, actual) {
        (Ty::Record(w), Ty::Record(g))
            if w.len() == g.len() && w.keys().all(|k| g.contains_key(k)) =>
        {
            // Find the first field (sorted key order) whose type differs, then the matching value node in
            // the written record literal.
            let (key, wt) = w
                .iter()
                .find(|(k, wt)| g.get(k).is_some_and(|gt| !wt.agrees_with(gt)))?;
            let (gt, wt) = (g[key].clone(), (*wt).clone());
            let value_node = *record_value_nodes(db, expr)?.get(key)?;
            compound_inner_coercion_fix(db, value_node, &wt, &gt)
                .or_else(|| numeric_text_coercion_fix(db, &wt, &gt, value_node))
        }
        (Ty::Tuple(w), Ty::Tuple(g)) if w.len() == g.len() => {
            let (i, (wt, gt)) = w
                .iter()
                .zip(g.iter())
                .enumerate()
                .find(|(_, (wt, gt))| !wt.agrees_with(gt))?;
            let (gt, wt) = (gt.clone(), wt.clone());
            let value_node =
                *positional_value_nodes(db, expr, crate::resolved::Prim::TupleNew)?.get(i)?;
            compound_inner_coercion_fix(db, value_node, &wt, &gt)
                .or_else(|| numeric_text_coercion_fix(db, &wt, &gt, value_node))
        }
        (Ty::List(we), Ty::List(ge)) if !we.agrees_with(ge) => {
            // A list is homogeneous — retype the FIRST element whose inner numeric a coercion bridges (the
            // fix an agent applies element-by-element; the message names the element axis).
            let (we, ge) = ((**we).clone(), (**ge).clone());
            positional_value_nodes(db, expr, crate::resolved::Prim::ListNew)?
                .iter()
                .find_map(|&e| {
                    compound_inner_coercion_fix(db, e, &we, &ge)
                        .or_else(|| numeric_text_coercion_fix(db, &we, &ge, e))
                })
        }
        (Ty::Sum { decl: wd, .. }, Ty::Sum { decl: gd, .. }) if wd == gd => {
            // A variant-constructed value whose payload's inner numeric differs from the annotated sum's
            // payload — `(Some 5)` vs `(Option Float64)`, `(Ok 5)` vs `(Result Float64 String)`. The value
            // is a ctor application `(Some 5)` = `Apply{head=Some, args=[5]}`; drill its payload
            // argument(s) against the EXPECTED payload type at the annotated sum's instantiation
            // (`payload_ty_at_instantiation`). Same `decl` (same sum) so the variant + payload align; the
            // per-arg coercion is the sum-payload twin of the collection-element fix. (A DECLARED sum
            // applied directly — `(Mk 5)` — already coerces via the variant-ctor-payload branch; this
            // covers the ANNOTATION/arg-mismatch site, incl. the prelude `Option`/`Result`.)
            let Resolved::Apply { head, args } = resolved_of(db, expr) else {
                return None;
            };
            // A SINGLE-payload variant construction — `(Some 5)`, `(Ok 5)`, `(Mk 5)` — whose one argument
            // IS the payload. `payload_ty_at_instantiation` gives that payload's type at the EXPECTED sum
            // (`Float64` for `(Option Float64)`); coerce the argument against it. A multi-payload variant
            // (its payload is a tuple) is left alone — the single-arg shape is the common numeric case and
            // keeps the type-arg↔argument alignment unambiguous.
            if crate::eval::variant_disc_of(db, head).is_none() || args.len() != 1 {
                return None;
            }
            let want = payload_ty_at_instantiation(db, head, expected)?;
            let arg = args[0];
            let got = type_of(db, arg);
            compound_inner_coercion_fix(db, arg, &want, &got)
                .or_else(|| numeric_text_coercion_fix(db, &want, &got, arg))
        }
        (Ty::Map(wk, wv), Ty::Map(gk, gv)) => {
            // A map is homogeneous on each axis. Retype the FIRST entry's KEY (if the key axis differs) or
            // VALUE (if the value axis differs) whose inner numeric a coercion bridges — mirroring the
            // collection-axis hint (`collection_element_mismatch_hint` reports KEY before VALUE). The
            // entries are `(key-occ, value-occ)` pairs of the written `(map (k v) …)` literal.
            let key_diff = !wk.agrees_with(gk);
            let (wt, gt) = if key_diff {
                ((**wk).clone(), (**gk).clone())
            } else if !wv.agrees_with(gv) {
                ((**wv).clone(), (**gv).clone())
            } else {
                return None;
            };
            map_entry_nodes(db, expr)?.iter().find_map(|&(k, v)| {
                let node = if key_diff { k } else { v };
                compound_inner_coercion_fix(db, node, &wt, &gt)
                    .or_else(|| numeric_text_coercion_fix(db, &wt, &gt, node))
            })
        }
        _ => None,
    }
}

/// The ordered `(key-node, value-node)` entries of a directly-written MAP literal `expr` — both the
/// `Resolved::Map` primitive form and the `map` NAME-alias application (`Apply` of `Prim::MapNew`, whose
/// args are the `(k v)` entry-pair lists). `None` when `expr` is not a written map literal. Lets
/// `compound_inner_coercion_fix` reach a map key/value leaf regardless of which map spelling was used.
pub(crate) fn map_entry_nodes(db: &mut Db, expr: StructId) -> Option<Vec<(StructId, StructId)>> {
    match resolved_of(db, expr) {
        Resolved::Map { entries } => Some(entries.to_vec()),
        Resolved::Apply { head, args }
            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::MapNew) =>
        {
            // Each arg is a map ENTRY: the native `(= k v)` FieldPair leaf (M2, what `#map`/`(map (= k v))`
            // emit), the transitional name-head `(= k v)`, or the legacy 2-element `(k v)` pair — mirror
            // `resolve_map`. Before this, a native FieldPair entry (3-element) failed the 2-element read →
            // the whole `.collect()` returned `None` → `map_entry_nodes` yielded no entries → the map's
            // key/value TYPE stayed `Any`, so the value/key-type HOMOGENEITY + duplicate-key checks never
            // ran → a two-different-value-types / duplicate-key map silently type-checked (miscompile).
            args.iter()
                .map(|&pair| {
                    db.ast
                        .field_pair_parts(pair)
                        .or_else(|| db.ast.field_pair(pair))
                        .or_else(|| match db.ast.get(pair) {
                            crate::ast::Struct::List(kids) => match kids.as_slice() {
                                [k, v] => Some((*k, *v)),
                                _ => None,
                            },
                            _ => None,
                        })
                })
                .collect()
        }
        _ => None,
    }
}

/// The `label → value-node` map of a directly-written RECORD literal `expr` — both the `{}`/bare-string
/// primitive form (`Resolved::Record`) and the `record` NAME-alias application (`Resolved::Apply` whose
/// `(meta apply)` is `Prim::RecordNew`, read via the shared `read_record_fields`). `None` when `expr` is
/// not a written record literal (a value bound to a name / returned from a call has no field-value nodes
/// to edit). Lets `compound_inner_coercion_fix` reach the inner leaf regardless of which record spelling
/// the author used.
pub(crate) fn record_value_nodes(
    db: &mut Db,
    expr: StructId,
) -> Option<std::collections::BTreeMap<crate::resolved::Symbol, StructId>> {
    match resolved_of(db, expr) {
        Resolved::Record { fields } => Some((*fields).clone()),
        Resolved::Apply { head, args }
            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordNew) =>
        {
            crate::resolve::read_record_fields(db, &args).ok()
        }
        _ => None,
    }
}

/// A one-shot RENAME fix for a MISSPELLED record-literal field: when the `actual` record value carries a
/// field whose name is a plausible typo of a field the `expected` record type requires but the value is
/// MISSING, rewrite that field's KEY to the expected name (`(record … (yy 2))` where `(y Int64)` was
/// wanted → replace the key `yy` with `y`). The record-literal twin of the member-access
/// `no_field_reject` typo fix — a wrong field NAME in a CONSTRUCTED record is exactly the same slip as a
/// wrong field name in a projection, so it earns the same mechanical repair. `None` unless both are
/// records, the value literal is written inline (its key nodes are editable), and there is a confident
/// single typo pairing (a missing expected field that is `suggest::nearest` to an extra supplied one).
pub(crate) fn record_field_typo_fix(
    db: &mut Db,
    expected: &Ty,
    actual: &Ty,
    arg: StructId,
) -> Option<Fix> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    // The fields the value SUPPLIES that the type has no place for (candidate typos) and the fields the
    // type REQUIRES that the value is missing (candidate intended names).
    let extra: Vec<&str> = got
        .keys()
        .filter(|k| !want.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    let missing: Vec<&str> = want
        .keys()
        .filter(|k| !got.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    // A CONFIDENT single pairing at THIS level: exactly one extra field that is the nearest typo of some
    // missing one. (More than one extra/missing is not a single mechanical rename — the field-set-diff
    // message still guides; we just don't auto-fix an ambiguous multi-field slip.)
    if let [typo] = extra.as_slice()
        && let Some(intended) = crate::diag::suggest::nearest(typo, missing.iter().copied())
        // Find the KEY occurrence of the typo'd field in the WRITTEN record literal — the `k` in a `(k v)`
        // entry — so the fix rewrites exactly that token. `None` if the value is not an inline literal (a
        // name-bound record has no editable key node), matching the honest-no-fix rule.
        && let Some(key_occ) = record_field_key_occ(db, arg, typo)
    {
        return Some(Fix::replace_heuristic(key_occ, intended));
    }
    // No typo at THIS level. If the field SETS agree, a typo may live inside a shared field whose want/got
    // are BOTH records that differ — DRILL into that field's value literal and recurse (the field-typo twin
    // of `compound_inner_coercion_fix`'s nested-leaf drill, so `(record (inner (record (fooo 1))))` against
    // `(Record (inner (Record (foo Int64))))` renames the deep `inner.fooo`→`foo`). Only when the field sets
    // are identical (no top-level extra/missing) — otherwise the top-level set diff is the real fault.
    if extra.is_empty() && missing.is_empty() {
        for (k, wt) in want.iter() {
            let gt = got.get(k)?;
            if let (Ty::Record(_), Ty::Record(_)) = (wt, gt)
                && !wt.agrees_with(gt)
                && let Some(sub_arg) = record_field_value_occ(db, arg, &k.name)
                && let Some(fix) = record_field_typo_fix(db, wt, gt, sub_arg)
            {
                return Some(fix);
            }
        }
    }
    None
}

/// The (key-node, value-node) of a record-literal field ENTRY, handling both the canonical
/// `(= name value)` ascription triple (RV2, Phase B — key = child 1, value = child 2) and the legacy
/// `(name value)` pair. `None` if `entry` is neither shape. A record VALUE field carries the `=` head;
/// a record PATTERN field stays a bare pair (patterns were not migrated), and both are read here.
pub(crate) fn record_field_kv(db: &Db, entry: StructId) -> Option<(StructId, StructId)> {
    match db.ast.get(entry) {
        crate::ast::Struct::List(kv) if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") => {
            Some((kv[1], kv[2]))
        }
        crate::ast::Struct::List(kv) if kv.len() == 2 => Some((kv[0], kv[1])),
        _ => None,
    }
}

/// The ENTRY node (the `(= k v)` / `(k v)` field) named `field` in a WRITTEN record literal `expr` — the
/// whole field, so a delete fix can remove it as a unit. `None` if `expr` is not an inline record literal
/// (in the RAW AST) or has no such field. Same raw-AST discipline as [`record_field_key_occ`].
pub(crate) fn record_field_entry_occ(db: &Db, expr: StructId, field: &str) -> Option<StructId> {
    let entries = db.ast.compound_form_of(expr, CompoundCtor::Record)?;
    for &entry in entries {
        if let Some((key_id, _)) = record_field_kv(db, entry)
            && let Some(sym) = crate::resolve::read_key(db, key_id)
            && &*sym.name == field
        {
            return Some(entry);
        }
    }
    None
}

/// The WRITTEN record-literal FORM node of `expr` (the `(record …)` list itself), if `expr` is an inline
/// record literal in the RAW AST — the node an `InsertArms` fix appends new `(field value)` entries to.
/// `None` for a name-bound / call-result record (no source form to edit).
pub(crate) fn record_literal_form(db: &Db, expr: StructId) -> Option<StructId> {
    // The native `#record(…)` ctor-leaf head, the reserved-symbol head `("record" …)`, and the bare `record`
    // name-alias all spell the same record form; `compound_form_of` recognizes all three, so the add-missing-
    // fields fix appends to a native record literal too (M3 native-recognition parity).
    if db
        .ast
        .compound_form_of(expr, CompoundCtor::Record)
        .is_some()
    {
        Some(expr)
    } else {
        None
    }
}

/// A one-shot ADD-MISSING-FIELDS fix for a record literal that is MISSING fields its expected type
/// requires: append a `(field (trap "TODO"))` entry per missing field to the written `(record …)` form
/// (`(record (x 1))` against `(Record (x Int64) (y Int64))` → `(record (x 1) (y (trap "TODO")))`). The
/// construction analogue of the "add the missing match arms" fix: the SHAPE is right (the field set now
/// matches, clearing CDZ0203) and `trap : ∀a. String → a` inhabits ANY field type so the placeholder never
/// introduces a fresh type error — but the VALUES are placeholders the author fills, so it is heuristic
/// (`--verify-fixes` upgrades it, since applying it clears the fault). Requires BOTH records, the value
/// written inline (its form is editable), and a NON-EMPTY missing set with NO extra field (a pure omission
/// — a value that also carries an extra field is a rename candidate `record_field_typo_fix` handles first,
/// or a genuinely different shape, not a clean "you forgot these" add).
pub(crate) fn record_field_add_fix(
    db: &mut Db,
    expected: &Ty,
    actual: &Ty,
    arg: StructId,
) -> Option<Fix> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    let missing: Vec<&crate::resolved::Symbol> =
        want.keys().filter(|k| !got.contains_key(*k)).collect();
    let extra = got.keys().any(|k| !want.contains_key(k));
    // A pure omission — some required field absent, and no supplied field the type lacks (that would be a
    // rename or a genuinely different shape, not a clean "add the fields you forgot").
    if missing.is_empty() || extra {
        return None;
    }
    let form = record_literal_form(db, arg)?;
    // Synthesize each missing field as the canonical `(= name value)` ascription triple (RV2, Phase B),
    // so the inserted entry matches the value-record field shape the printer/resolver expect.
    let arms: Vec<String> = missing
        .iter()
        .map(|k| format!("(= {} (trap \"TODO\"))", k.name))
        .collect();
    Some(Fix::insert_arms_heuristic(form, arms))
}

/// A one-shot DELETE-EXTRA-FIELD fix for a record literal carrying a field its expected type has no place
/// for: remove that `(field value)` entry from the written `(record …)` form (`(record (x 1) (y 2))`
/// against `(Record (x Int64))` → `(record (x 1))`). The construction analogue of the "remove the unused
/// element" delete fix. Requires BOTH records, the value written inline, and EXACTLY ONE extra field with
/// NO missing field (a single clean surplus — an extra alongside a missing one is a rename candidate
/// `record_field_typo_fix` handles first; multiple extras are not one mechanical delete).
pub(crate) fn record_field_delete_fix(
    db: &mut Db,
    expected: &Ty,
    actual: &Ty,
    arg: StructId,
) -> Option<Fix> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    let extra: Vec<&str> = got
        .keys()
        .filter(|k| !want.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    let missing = want.keys().any(|k| !got.contains_key(k));
    if let [surplus] = extra.as_slice()
        && !missing
        && let Some(entry) = record_field_entry_occ(db, arg, surplus)
    {
        return Some(Fix::delete_heuristic(
            entry,
            format!("remove the field `{surplus}` — the expected record has no such field"),
        ));
    }
    None
}

/// The WRITTEN tuple-literal FORM node of `expr` (the `(tuple …)` list itself), if `expr` is an inline
/// tuple literal in the RAW AST — the node an `InsertArms` fix appends new element forms to. `None` for a
/// name-bound / call-result tuple (no source form to edit). Mirrors [`record_literal_form`]; a tuple is
/// spelled by either the `tuple` NAME alias or the `"tuple"` string-literal primitive head.
pub(crate) fn tuple_literal_form(db: &Db, expr: StructId) -> Option<StructId> {
    if db.ast.compound_form_of(expr, CompoundCtor::Tuple).is_some() {
        Some(expr)
    } else {
        None
    }
}

/// A one-shot ADD-MISSING-ELEMENTS fix for a tuple literal with too FEW elements: append a `(trap "TODO")`
/// placeholder per missing trailing position to the written `(tuple …)` form (`(tuple 1 2)` against
/// `(Tuple Int64 Int64 Int64)` → `(tuple 1 2 (trap "TODO"))`). The tuple analogue of `record_field_add_fix`:
/// `trap : ∀a. String → a` inhabits any element type, so the placeholder clears the arity fault in one shot
/// (`--verify-fixes` upgrades it). Requires BOTH tuples, the value written inline, and the value having
/// STRICTLY FEWER elements — a value with MORE is the delete case; equal arity is a per-position type
/// mismatch (its own message), not an arity fix.
pub(crate) fn tuple_element_add_fix(
    db: &mut Db,
    expected: &Ty,
    actual: &Ty,
    arg: StructId,
) -> Option<Fix> {
    let (Ty::Tuple(want), Ty::Tuple(got)) = (expected, actual) else {
        return None;
    };
    if got.len() >= want.len() {
        return None;
    }
    let form = tuple_literal_form(db, arg)?;
    let arms: Vec<String> = (got.len()..want.len())
        .map(|_| "(trap \"TODO\")".to_string())
        .collect();
    Some(Fix::insert_arms_heuristic(form, arms))
}

/// A one-shot DELETE-SURPLUS-ELEMENT fix for a tuple literal with EXACTLY ONE too many elements: remove the
/// trailing surplus element from the written `(tuple …)` form (`(tuple 1 2 3)` against `(Tuple Int64
/// Int64)` → `(tuple 1 2)`). The tuple analogue of `record_field_delete_fix`. Gated to a SINGLE surplus
/// (exactly one over) — one clean mechanical delete; two-or-more surplus is not one edit (the message still
/// names the arity), and a value with FEWER elements is the add case.
pub(crate) fn tuple_element_delete_fix(
    db: &mut Db,
    expected: &Ty,
    actual: &Ty,
    arg: StructId,
) -> Option<Fix> {
    let (Ty::Tuple(want), Ty::Tuple(got)) = (expected, actual) else {
        return None;
    };
    if got.len() != want.len() + 1 {
        return None;
    }
    let elems = positional_value_nodes(db, arg, crate::resolved::Prim::TupleNew)?;
    // The surplus is the trailing element (position `want.len()`), the one past the expected arity.
    let surplus = *elems.get(want.len())?;
    Some(Fix::delete_heuristic(
        surplus,
        "remove the extra tuple element — the expected tuple has fewer positions",
    ))
}

/// The VALUE occurrence (`v` in a `(k v)` entry) of the field named `field` in a WRITTEN record literal
/// `expr` — the companion of [`record_field_key_occ`] that returns the field's value node, so a nested
/// typo fix can recurse into a sub-record literal. `None` if `expr` is not an inline record literal or has
/// no such field.
pub(crate) fn record_field_value_occ(db: &Db, expr: StructId, field: &str) -> Option<StructId> {
    let entries = db.ast.compound_form_of(expr, CompoundCtor::Record)?;
    for &entry in entries {
        if let Some((key_id, val_id)) = record_field_kv(db, entry)
            && let Some(sym) = crate::resolve::read_key(db, key_id)
            && &*sym.name == field
        {
            return Some(val_id);
        }
    }
    None
}

/// The KEY occurrence (`k` in a `(k v)` entry) of the field named `field` in a WRITTEN record literal
/// `expr` — `(record (a 1) (b 2))`. `None` if `expr` is not an inline record literal (in the RAW AST) or
/// has no such field. Reads the RAW `db.ast` structure of `expr` (NOT `resolved_of`, whose `RecordNew`
/// args are RE-NODED entries that do not map back to the source key tokens the fix must edit) — the same
/// raw-AST discipline `no_field_reject` uses to target the exact `(. operand key)` key token.
pub(crate) fn record_field_key_occ(db: &Db, expr: StructId, field: &str) -> Option<StructId> {
    // The `(key value)` entry list is the tail of a `(record …)` form (both the reserved-symbol head and a
    // bare `record` name-alias spell the same shape here).
    let entries = db.ast.compound_form_of(expr, CompoundCtor::Record)?;
    for &entry in entries {
        if let Some((key_id, _)) = record_field_kv(db, entry)
            && let Some(sym) = crate::resolve::read_key(db, key_id)
            && &*sym.name == field
        {
            return Some(key_id);
        }
    }
    None
}

/// The ordered element value-nodes of a directly-written TUPLE (`prim = TupleNew`) or LIST (`ListNew`)
/// literal `expr` — both the `Resolved::Tuple`/`List` primitive form and the `tuple`/`list` NAME-alias
/// application (`Resolved::Apply` of the matching `(meta apply)` prim, whose `args` ARE the elements).
/// `None` when `expr` is not the requested written literal kind.
pub(crate) fn positional_value_nodes(
    db: &mut Db,
    expr: StructId,
    prim: crate::resolved::Prim,
) -> Option<Vec<StructId>> {
    match resolved_of(db, expr) {
        Resolved::Tuple { elems } if prim == crate::resolved::Prim::TupleNew => {
            Some(elems.to_vec())
        }
        Resolved::List { elems } if prim == crate::resolved::Prim::ListNew => Some(elems.to_vec()),
        Resolved::Apply { head, args } if crate::eval::meta_apply_of(db, head) == Some(prim) => {
            Some(args.to_vec())
        }
        _ => None,
    }
}

/// An actionable message TAIL when `expected` and `actual` are BOTH records that differ. Two shapes,
/// each pointing at the SPECIFIC difference instead of leaving the reader to diff two full record renders:
///  • a FIELD-SET difference — the value is MISSING a field the type requires, and/or carries an EXTRA one
///    the type has no place for (rustc's "missing field `y`" / "no field `z`"); OR
///  • a same-field-set PER-FIELD TYPE difference — the field names all match but some field's TYPE differs
///    (`(x Int64)` vs `(x Bool)`), named as "field `x` should be Int64, but this one is Bool" (rustc's
///    "expected `Int64`, found `Bool`" anchored on the field). Buried in a 3-field render otherwise — the
///    reader must diff `(Record (x Int64) (y Int64) (z Int64))` against `(… (y Bool) …)` to spot `y`.
/// `None` unless both are records AND some difference is found. Field names are compared as SETS (the
/// `BTreeMap` keys) and the differing-type fields are visited in sorted key order, so the hint is
/// deterministic and order-independent. A field-set difference takes precedence (a record with both a
/// wrong field-set AND a type mismatch on a shared field is first told which fields to add/remove).
pub(crate) fn record_field_diff_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    let missing: Vec<&str> = want
        .keys()
        .filter(|k| !got.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    let extra: Vec<&str> = got
        .keys()
        .filter(|k| !want.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    if missing.is_empty() && extra.is_empty() {
        // Same field-name set — look for the FIRST field (sorted key order) whose type differs and name
        // it. `agrees_with` is the same relation `unify` uses, so we only flag a genuine clash (a deferred
        // `Var`/`Any` field agrees and is skipped). Naming one field is enough to point the fix; the full
        // render still carries the complete picture for a multi-field clash. When the differing field is
        // itself a same-shape nested compound, DRILL to the deepest scalar leaf so the hint reads "field
        // `a.b.c` should be Int64, but this one is Bool" instead of re-rendering the whole sub-record.
        let culprit = want
            .iter()
            .find(|(k, wt)| got.get(k).is_some_and(|gt| !wt.agrees_with(gt)));
        return culprit.map(|(k, wt)| {
            let gt = &got[k];
            let (path, lw, lg) = match deep_leaf_delta(wt, gt) {
                Some((sub, lw, lg)) => (format!("{}.{sub}", k.name), lw, lg),
                None => (k.name.to_string(), wt, gt),
            };
            format!(
                " — field `{path}` should be {}, but this one is {}",
                lw.render_name(ncx),
                lg.render_name(ncx)
            )
        });
    }
    let quote = |names: &[&str]| {
        names
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts = Vec::new();
    if !missing.is_empty() {
        let plural = if missing.len() == 1 { "" } else { "s" };
        parts.push(format!("missing field{plural} {}", quote(&missing)));
    }
    if !extra.is_empty() {
        let plural = if extra.len() == 1 { "" } else { "s" };
        parts.push(format!(
            "no such field{plural} {} on the expected record",
            quote(&extra)
        ));
    }
    Some(format!(" — {}", parts.join("; ")))
}

/// An actionable message TAIL when `expected` and `actual` are BOTH tuples that differ. Two shapes,
/// mirroring the record hint, each pointing at the SPECIFIC difference:
///  • DIFFERENT ARITY — the value has more or fewer elements than the type ("expected a tuple with 3
///    elements, but this one has 2", rustc's arity message); OR
///  • same-arity PER-POSITION TYPE difference — a `(Tuple Int64 Bool)` where `(Tuple Int64 Int64)` is
///    wanted, named as "element 1 should be Int64, but this one is Bool" (0-indexed, matching the tuple
///    projection `(. t 1)` and the out-of-range "tuple index N" message). Buried in the full render
///    otherwise for a wide tuple.
/// `None` unless both are tuples AND some difference is found. Positions are visited left-to-right, so the
/// hint is deterministic (the first differing position). `agrees_with` (the relation `unify` uses) gates
/// the per-position check, so a deferred `Var`/`Any` position is skipped.
pub(crate) fn tuple_arity_mismatch_hint(
    expected: &Ty,
    actual: &Ty,
    ncx: &NameCtx,
) -> Option<String> {
    let (Ty::Tuple(want), Ty::Tuple(got)) = (expected, actual) else {
        return None;
    };
    if want.len() != got.len() {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        return Some(format!(
            " — expected a tuple with {} element{}, but this one has {}",
            want.len(),
            plural(want.len()),
            got.len(),
        ));
    }
    // Same arity — name the FIRST position whose type differs (0-indexed). When that position is itself a
    // same-shape nested compound, DRILL to the deepest scalar leaf ("element 0.x should be …") rather than
    // re-rendering the whole sub-compound.
    want.iter()
        .zip(got.iter())
        .enumerate()
        .find(|(_, (wt, gt))| !wt.agrees_with(gt))
        .map(|(i, (wt, gt))| {
            let (path, lw, lg) = match deep_leaf_delta(wt, gt) {
                Some((sub, lw, lg)) => (format!("{i}.{sub}"), lw, lg),
                None => (i.to_string(), wt, gt),
            };
            format!(
                " — element {path} should be {}, but this one is {}",
                lw.render_name(ncx),
                lg.render_name(ncx)
            )
        })
}

/// An actionable message TAIL when `expected` and `actual` are the SAME collection KIND (both `List`, both
/// `Map`, or both `Set`) but an ELEMENT / KEY / VALUE type differs — the collection analogue of the
/// record/tuple per-member hint. Naming both full types (`(Map String Int64)` vs `(Map Int64 Int64)`)
/// makes the reader diff two renders to see it is the KEY axis that differs; this names the axis and its
/// expected-vs-actual type ("its keys should be String, but these are Int64"). For a `Map` the KEY is
/// checked before the VALUE (report the leftmost differing axis, deterministic). `agrees_with` (the
/// relation `unify` uses) gates each axis, so a deferred `Var`/`Any` element is skipped. `None` unless
/// both are the same collection kind AND some axis genuinely differs. Tail only — the repair is retyping
/// the offending element(s), which the author must supply, so no mechanical fix (like the record/tuple
/// per-member hints).
pub(crate) fn collection_element_mismatch_hint(
    expected: &Ty,
    actual: &Ty,
    ncx: &NameCtx,
) -> Option<String> {
    let axis = |role: &str, want: &Ty, got: &Ty, plural: &str| {
        (!want.agrees_with(got)).then(|| {
            format!(
                " — its {role} should be {}, but {plural} {}",
                want.render_name(ncx),
                got.render_name(ncx)
            )
        })
    };
    match (expected, actual) {
        (Ty::List(want), Ty::List(got)) => axis("elements", want, got, "these are"),
        (Ty::Set(want), Ty::Set(got)) => axis("elements", want, got, "these are"),
        (Ty::Map(wk, wv), Ty::Map(gk, gv)) => {
            // KEY axis first, then VALUE — report the leftmost differing axis.
            axis("keys", wk, gk, "these are").or_else(|| axis("values", wv, gv, "these are"))
        }
        _ => None,
    }
}

/// An actionable message TAIL when `expected` and `actual` are the SAME sum type (same `decl` + variant
/// set) whose TYPE PARAMETER differs — `(Option Float64)` vs `(Option Int64)`, `(Result Float64 String)`
/// vs `(Result Int64 String)`. Names the differing payload axis (" — its payload should be Float64, but
/// this one is Int64") instead of leaving the reader to diff two full `(Option …)` renders, the sum twin
/// of the collection-axis hint. `None` unless both are the same sum with the same arg count and a genuine
/// arg difference (a DIFFERENT sum — `Option` vs `Result` — is unrelated, the generic path). `agrees_with`
/// gates each arg. Reports the first (leftmost) differing type argument.
pub(crate) fn sum_payload_mismatch_hint(
    expected: &Ty,
    actual: &Ty,
    ncx: &NameCtx,
) -> Option<String> {
    let (
        Ty::Sum {
            decl: wd, args: wa, ..
        },
        Ty::Sum {
            decl: gd, args: ga, ..
        },
    ) = (expected, actual)
    else {
        return None;
    };
    if wd != gd || wa.len() != ga.len() {
        return None;
    }
    wa.iter().zip(ga.iter()).find_map(|(w, g)| {
        (!w.agrees_with(g)).then(|| {
            format!(
                " — its payload should be {}, but this one is {}",
                w.render_name(ncx),
                g.render_name(ncx)
            )
        })
    })
}

/// An actionable message TAIL when `expected` and `actual` are BOTH FUNCTION types (`Ty::Fn`) that differ
/// — the function analogue of the record/tuple/sum per-member hints. A `Ty::Fn` is CURRIED (`p0 → p1 →
/// result`), so naming two full arrow renders (`(-> Int64 (-> Int64 Int64))` vs `(-> Int64 Int64)`) makes
/// the reader unwind the curry to see what actually differs. Peel both arrow chains and name the SPECIFIC
/// difference, in the order a caller most cares about:
///  • DIFFERENT ARITY — the two take a different number of arguments ("expected a function taking 2
///    arguments, but this one takes 1", rustc's arrow-arity message); OR
///  • same-arity RESULT-type difference — the parameter types agree but the RESULT differs (`(-> Int64
///    Bool)` vs `(-> Int64 Int64)`), named "its result should be Bool, but this one returns Int64".
/// A same-arity PARAMETER-type difference is deliberately NOT named here: the generic arg-unify already
/// descends into the first differing parameter position and reports it directly (`(-> Bool …)` vs `(-> Int64
/// …)` surfaces as an ordinary "Int64 vs Bool" at the parameter), so a param-axis hint here would duplicate
/// it. `None` unless both are functions AND they differ in arity or result. `agrees_with` (the relation
/// `unify` uses) gates the result check. Tail only — the repair (add/drop a parameter, or change the return
/// expression) is the author's, so no mechanical fix, like the sibling per-member hints.
pub(crate) fn fn_signature_delta_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    if !matches!(expected, Ty::Fn(..)) || !matches!(actual, Ty::Fn(..)) {
        return None;
    }
    // Peel the curried arrows: collect the parameter count and the ultimate result of each.
    let peel = |mut t: &Ty| {
        let mut arity = 0usize;
        while let Ty::Fn(_, r) = t {
            arity += 1;
            t = r;
        }
        (arity, t.clone())
    };
    let ((want_arity, want_result), (got_arity, got_result)) = (peel(expected), peel(actual));
    if want_arity != got_arity {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        return Some(format!(
            " — expected a function taking {} argument{}, but this one takes {}",
            want_arity,
            plural(want_arity),
            got_arity,
        ));
    }
    (!want_result.agrees_with(&got_result)).then(|| {
        format!(
            " — its result should be {}, but this one returns {}",
            want_result.render_name(ncx),
            got_result.render_name(ncx)
        )
    })
}

/// The combined per-member structural-delta TAIL for two SAME-KIND compound types that differ inside — a
/// record field-set / per-field type, a tuple arity / per-position type, or a collection element/key/value
/// type. This bundles the three per-member hints (`record_field_diff_hint`, `tuple_arity_mismatch_hint`,
/// `collection_element_mismatch_hint`) behind one call so the JOIN sites — a list literal, an `if`'s
/// branches, a `match`'s arms — can point at the SPECIFIC differing sub-part exactly as the annotation-
/// mismatch site does, instead of rendering two whole compound types the reader must diff. The `first`
/// argument is the type the join is unifying against (the first element / then-branch / first arm); the
/// hints phrase it as "should be <first>, but this one is <other>", which reads correctly for a join (the
/// first occurrence sets the type the rest must match). `None` when the two types are not the same
/// structured kind or agree on every member. No mechanical fix (the repair is retyping a value the author
/// supplies), matching the per-member hints it composes.
///
/// This is how a compound-type rejection names the MINIMAL conflict rather than an arbitrary casualty: it
/// points at the one differing field / position / element axis (the constraint that actually failed),
/// leaving the agreeing members out of the blame, instead of rendering two whole compound types that share
/// most of their structure.
//= spec/capabilities/type-system.md#a-type-rejection-reports-the-minimal-conflict-at-both-sites
//# A rejection for a failed unification MUST report the minimal unsatisfiable set of constraints rather than the first constraint that failed, so that the diagnostic names the actual conflict and not an arbitrary casualty of it.
pub(crate) fn structural_delta_hint(first: &Ty, other: &Ty, ncx: &NameCtx) -> Option<String> {
    record_field_diff_hint(first, other, ncx)
        .or_else(|| tuple_arity_mismatch_hint(first, other, ncx))
        .or_else(|| collection_element_mismatch_hint(first, other, ncx))
        .or_else(|| sum_payload_mismatch_hint(first, other, ncx))
        .or_else(|| fn_signature_delta_hint(first, other, ncx))
        .or_else(|| qty_scale_mismatch_hint(first, other))
        .or_else(|| same_name_distinct_type_hint(first, other, ncx))
}

/// A message TAIL for two `(Qty T u)` types that are the SAME DIMENSION but at DIFFERENT SCALES — the
/// units-specific reason two quantities fail to unify at a join (`if`/`match`/`list`). `(Qty Int64 km)`
/// and `(Qty Int64 m)` are distinct types (their `Ty::Qty.unit` carries the scale — km is 1000/1, m is
/// 1/1), so a join rejects them, but BOTH `render_name()` to `(Qty Int64 (Unit.base #"meter"))` (the
/// render shows the reference-unit NAME and drops the scale). Without this the generic
/// `same_name_distinct_type_hint` fires and blames a "declaration shadows a built-in" — flat wrong for a
/// scale clash. Name the REAL cause and the repair: the branches carry the same dimension at different
/// units, so convert one to the other's unit (`in`/`as`) to make the join well-typed — a quantity join
/// does NOT auto-normalize (the no-silent-promotion rule; a conversion is explicit). Fires ONLY for two
/// same-dimension different-scale quantities; a genuine cross-DIMENSION clash (meter vs second) renders
/// distinguishably and needs no tail, and a same-scale pair unifies (no mismatch to explain).
pub(crate) fn qty_scale_mismatch_hint(first: &Ty, other: &Ty) -> Option<String> {
    let (Ty::Qty { unit: ua, .. }, Ty::Qty { unit: ub, .. }) = (first, other) else {
        return None;
    };
    // Same dimension, different scale — the case the generic same-name hint would misdiagnose. A different
    // dimension renders distinguishably (the units differ by name), so leave it to the plain mismatch.
    if !ua.same_dimension(ub) || ua.scale() == ub.scale() {
        return None;
    }
    Some(
        " — these are the SAME dimension at DIFFERENT units (their scales to the reference differ); a \
         quantity join does not auto-convert, so convert one branch to the other's unit with `in`/`as` \
         to make them the same type"
            .to_string(),
    )
}

/// A message TAIL for two DISTINCT types that RENDER to the SAME name — "an Int64, but a value of type
/// Int64 is expected here" reads as a contradiction. This happens when a user type SHADOWS a prelude type
/// name: `(type Int64 (A))` binds `Int64` to a user sum, so `(f 5)` to a `(: x Int64)` parameter passes the
/// prelude `Int64` literal where the user's `Int64` sum is expected — two genuinely different types printed
/// identically. Name that the names collide because a declaration shadows a built-in, so the reader sees the
/// mismatch is real (not a compiler confusion) and knows to rename the shadowing type. Fires ONLY when the
/// rendered names are equal AND the types are NOT equal (a real, same-name clash); a normal mismatch (two
/// different names) renders unambiguously and needs no tail. No mechanical fix — the repair (rename the
/// shadowing declaration, or the reference) is the author's choice. Guards against a Var/Any side (which can
/// still unify — not a settled clash) so this only speaks for two CONCRETE same-named-but-distinct types.
pub(crate) fn same_name_distinct_type_hint(
    first: &Ty,
    other: &Ty,
    ncx: &NameCtx,
) -> Option<String> {
    if matches!(first, Ty::Var(_) | Ty::Any) || matches!(other, Ty::Var(_) | Ty::Any) {
        return None;
    }
    if first == other || first.render_name(ncx) != other.render_name(ncx) {
        return None;
    }
    Some(
        " — these are two DIFFERENT types printed with the same name (a declaration shadows a \
         built-in of that name); rename the shadowing type so the two are distinguishable"
            .to_string(),
    )
}

/// The message TAIL for a PEER-JOIN type clash — two `if` branches, two `match` arm bodies, two `list`
/// elements — that must share one type but do not. A peer clash is SYMMETRIC (neither side is the fixed
/// "expected" type, unlike an annotation/argument mismatch), so this first tries the structural-delta
/// hints (record field-set / tuple arity / collection axis / sum payload — themselves order-tolerant),
/// then the two DIRECTIONAL hints the annotation site carried but the join sites did not:
/// `option_payload_mismatch_hint` (one side is `(Option T)`, the other its payload `T` — "match it to
/// handle `None`") and `fn_not_applied_hint` (one side is an unapplied `(-> … T)` whose full application
/// yields the other side — "apply it to N more arguments") — each tried in BOTH orderings, so whichever
/// branch/arm/element is the Option wrapper or the unfinished call gets named. This brings the join sites
/// to fix-parity with the annotation site's hint chain. Tail only, no mechanical `Fix`: a peer clash's
/// repair (match the Option, finish the call, or retype a branch) is the author's choice, and the join
/// sites keep their own one-shot INT-LITERAL→FLOAT retype fix for the numeric case.
pub(crate) fn peer_type_delta_hint(first: &Ty, other: &Ty, ncx: &NameCtx) -> Option<String> {
    structural_delta_hint(first, other, ncx)
        // Two same-dimension DIFFERENT-scale quantities (`(list (Qty … km) (Qty … m))`) render to the SAME
        // name (`render_name` drops the scale), so the generic "must share one type: (Qty … meter) and
        // (Qty … meter)" reads as a contradiction. Route the peer join through the scale-mismatch hint (as
        // the `if`/`match` join sites already do at their own chain) so the list-element / peer clash names
        // the real cause (same dimension, different units — convert with `in`/`as`) rather than two
        // identical-looking types.
        .or_else(|| qty_scale_mismatch_hint(first, other))
        .or_else(|| option_payload_mismatch_hint(ncx, first, other))
        .or_else(|| option_payload_mismatch_hint(ncx, other, first))
        .or_else(|| fn_not_applied_hint(first, other, ncx))
        .or_else(|| fn_not_applied_hint(other, first, ncx))
}

/// An actionable message TAIL when `actual` is a FUNCTION value used where a NON-function `expected` is
/// required — the "forgot to call it" slip. A partial application `(h 1)` (h takes 2) or a bare function
/// name `h` used as a value has type `(-> … …)`; the generic "type mismatch: Int64 and (-> Int64 Int64)"
/// reads like an internal clash and never says the value is simply an UNAPPLIED function. This names the
/// slip and how to fix it — SUPPLY the missing arguments (rustc's "you might have forgotten to call this
/// function"). Fires ONLY when applying the function to its remaining N argument(s) would yield the
/// expected type (the fully-applied result `agrees_with` expected) — then "just apply it" is the true
/// repair. If the fully-applied result would STILL differ (a deeper mismatch), there is no "call it"
/// story, so no hint (honest-no-fix). No mechanical `Fix` — we cannot know WHICH argument values the
/// author meant — so this is a tail only, like `option_payload_mismatch_hint`. `expected` must be a
/// DEFINITE non-function (not a `Var`/`Any` that could still unify with the arrow, and not a `Fn` — a
/// fn-vs-fn signature mismatch is a real clash reported on its own terms, not a missing application).
pub(crate) fn fn_not_applied_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    if !matches!(actual, Ty::Fn(..)) || matches!(expected, Ty::Fn(..) | Ty::Var(_) | Ty::Any) {
        return None;
    }
    // Peel the curried arrows: how many arguments remain, and what the fully-applied result would be.
    let mut result = actual;
    let mut remaining = 0usize;
    while let Ty::Fn(_, r) = result {
        remaining += 1;
        result = r;
    }
    if !result.agrees_with(expected) {
        return None; // supplying the args would not produce the expected type — no "just call it" fix
    }
    let plural = if remaining == 1 { "" } else { "s" };
    Some(format!(
        " — the value is a function that hasn't been fully applied; apply it to {remaining} more \
         argument{plural} to get {}",
        expected.render_with_article(ncx)
    ))
}

/// A suggest-NEGATION message TAIL when `node` is an arity-1 subtraction `(- e)` used where a NON-function
/// value is expected. Since the operator removed unary-`-`-as-negation, `(- e)` is a PARTIALLY-applied
/// binary subtraction that CURRIES to `(-> T T)` (like `(+ 1)`), NOT negation — so a user who wrote `-e`
/// expecting to negate `e` gets the generic "unapplied function" clash, which never names the likely
/// intent. Detect the arity-1-`Prim::Sub` shape (the ONE case where "you probably meant to negate" is the
/// right story) and point at the real negation operators `Num.neg` / `<T>.neg`. Node-aware (the type-only
/// `fn_not_applied_hint` cannot tell a partial subtraction from any other `(-> T T)`), so it is a distinct
/// tail wired at the value-position mismatch sites that carry the offending node. `None` for anything else
/// (a genuine partial like `(+ 1)`, a bare fn name, `String.slice s` — those keep the plain
/// `fn_not_applied_hint`). Tail only, no mechanical fix (whether the author wants `Num.neg e` or `(- a e)`
/// is their intent to resolve).
pub(crate) fn suggest_neg_hint(db: &mut Db, node: StructId) -> Option<String> {
    let Resolved::Apply { head, args } = resolved_of(db, node) else {
        return None;
    };
    if args.len() != 1 || crate::eval::meta_apply_of(db, head) != Some(crate::resolved::Prim::Sub) {
        return None;
    }
    Some(
        " — this is `(- e)`, a partially-applied subtraction (a function `(-> T T)`), not negation; \
         to NEGATE a value use `Num.neg` (or the per-type `<T>.neg`, e.g. `Int64.neg`)"
            .to_string(),
    )
}

/// The `(prefix, suffix, verb)` of a prelude CONVERSION that turns a value of type `actual` into the
/// `expected` type in ONE shot — the coercion-wrap repair for a mismatch the numeric model / text model
/// has a total conversion for. Today: `String` where `Bytes` is wanted → `(String.to-bytes …)` (the total
/// UTF-8 encode, `collections-and-text.md`). NOT the reverse (`Bytes → String` is `from-bytes : Bytes →
/// (Option String)`, FALLIBLE — no one-shot wrap that type-checks, so it stays unfixed rather than suggest
/// a spelling that just cascades to an Option mismatch). The wrap text is a member-access spelling the
/// resolver handles generically — the same shape as the `(Float64.of-int …)` / `(Int64.of …)` coercions,
/// NOT a hard-coded name lookup (`no-keys-outside-the-prelude`). `None` when no total conversion applies.
pub(crate) fn total_conversion_wrap(
    expected: &Ty,
    actual: &Ty,
) -> Option<(String, String, String)> {
    match (expected, actual) {
        (Ty::Bytes, Ty::String) => Some((
            "(String.to-bytes ".to_string(),
            ")".to_string(),
            "encode the string to its UTF-8 bytes with `String.to-bytes`".to_string(),
        )),
        // A fixed-width integer where a BigInt is expected — `(+ (BigInt.of 5) 3)`. `BigInt.of : ∀a.
        // (Int a) → BigInt` is TOTAL (every fixed int fits the unbounded BigInt), so wrapping the int
        // operand repairs the numeric mix in one shot — the BigInt twin of the int-width `.of` coercion.
        (Ty::BigInt, Ty::Int(_)) => Some((
            "(BigInt.of ".to_string(),
            ")".to_string(),
            "convert the integer to a BigInt with `BigInt.of`".to_string(),
        )),
        // A fixed-width integer where a Rational is expected — `(+ r 1)`, `r : Rational`. `Rational.of-int
        // : ∀a. (Int a) → Rational` is TOTAL (the whole rational `n/1`), so wrapping the int operand
        // repairs the mix in one shot — the Rational twin.
        (Ty::Rational, Ty::Int(_)) => Some((
            "(Rational.of-int ".to_string(),
            ")".to_string(),
            "convert the integer to a Rational with `Rational.of-int`".to_string(),
        )),
        // A Char where Int64 is expected — `(+ #\a 1)`. `Char.to-int : Char → Int64` is TOTAL (a char's
        // scalar value), so wrap it. Only when the expected width is the Int64 `Char.to-int` yields (a
        // narrower target would still mismatch after the wrap — leave that to the author).
        (Ty::Int(exp), Ty::Char) if exp.ground_signed() && exp.ground_width() == 64 => Some((
            "(Char.to-int ".to_string(),
            ")".to_string(),
            "convert the char to its Int64 scalar value with `Char.to-int`".to_string(),
        )),
        // A Char where a FLOAT is expected — `(+ #\a 1.0)`, `(< #\a 1.0)`. `Char.to-int` yields Int64, and
        // Cadenza never implicitly promotes Int64 → Float, so a bare `Char.to-int` re-fails; the TWO-STEP
        // `(Float{W}.of-int (Char.to-int …))` is the working repair (W = the expected float's width, so the
        // result matches the sibling's exact type). The float twin of the Char→Int64 wrap above.
        (Ty::Float(exp), Ty::Char) => {
            let module = format!("Float{}", exp.ground_width());
            Some((
                format!("({module}.of-int (Char.to-int "),
                "))".to_string(),
                format!(
                    "convert the char to its Int64 scalar value then to a float with `{module}.of-int`"
                ),
            ))
        }
        // A Char where a BIGINT is expected — `(+ #\a (BigInt.of 5))`. `Char.to-int` yields Int64 and
        // `BigInt.of : ∀a. (Int a) → BigInt` lifts it; a bare `Char.to-int` re-fails (Int64 vs BigInt, no
        // implicit promotion), so the working repair is the TWO-STEP `(BigInt.of (Char.to-int …))` — the
        // BigInt twin of the Char→Float wrap.
        (Ty::BigInt, Ty::Char) => Some((
            "(BigInt.of (Char.to-int ".to_string(),
            "))".to_string(),
            "convert the char to its Int64 scalar value then to a BigInt with `BigInt.of`"
                .to_string(),
        )),
        // A Char where a RATIONAL is expected — `(+ #\a (Rational.of-int 5))`. `Rational.of-int : ∀a. (Int
        // a) → Rational` lifts the `Char.to-int` scalar; a bare `Char.to-int` re-fails (Int64 vs Rational),
        // so the working repair is the TWO-STEP `(Rational.of-int (Char.to-int …))` — the Rational twin.
        (Ty::Rational, Ty::Char) => Some((
            "(Rational.of-int (Char.to-int ".to_string(),
            "))".to_string(),
            "convert the char to its Int64 scalar value then to a Rational with `Rational.of-int`"
                .to_string(),
        )),
        // A Symbol where a String is expected — `Symbol.to-string : Symbol → String` is TOTAL (recovers
        // the symbol's underlying content), so wrap it. The text-model twin of the String→Bytes case.
        (Ty::String, Ty::Symbol) => Some((
            "(Symbol.to-string ".to_string(),
            ")".to_string(),
            "recover the symbol's content string with `Symbol.to-string`".to_string(),
        )),
        _ => None,
    }
}

/// The `(prefix, suffix, verb)` of the conversion `(<Expected>.of …)` when a NUMERIC value of one
/// width/precision is supplied where a DIFFERENT one of the SAME numeric kind is `expected` — the
/// numeric-model coercion wrap (`06-numeric-model.sexp` — `(+ 2 (Int64.of 1))`). Covers BOTH an INTEGER
/// width mismatch (`Int8`/`Int64`) and a FLOAT precision mismatch (`Float32`/`Float64`); shared by every
/// site the same mismatch surfaces — an operator/ctor argument, a value/param `(: … T)` annotation, and an
/// annotated let-binder. GATED to an ALIASED expected width/precision (int {8,16,32,64}, float {32,64}) —
/// only those render to a BOUND type name, so `(<Expected>.of …)` resolves; a non-aliased `(Int 48)` would
/// suggest an unbound `Int48.of`, worse than no fix. The wrap text is a member-access spelling the resolver
/// handles generically, not a hard-coded name. Heuristic: which value to convert is the author's intent —
/// and the int `.of` is CHECKED (traps out of range) while the float `.of` is TOTAL (widen exact, narrow
/// rounds), reflected in the verb. `None` unless both types are the same numeric kind at an aliased width.
/// The SPELLING of an integer width type's MODULE — the thing a `.wrap`/`.of` conversion is a member of —
/// that actually RESOLVES for `it`'s width. An ALIASED width ({8,16,32,64}) has a pre-installed BOUND
/// module NAME (`UInt8`, `Int64`, `prelude::install`), so a bare `UInt8` resolves; a NON-aliased width
/// (`(UInt 4)` — a bit-field's own type) has NO bound name (`render_name` produces `UInt4`, an UNBOUND
/// identifier), so the only spelling that resolves is the type-CONSTRUCTOR applied form `(UInt 4)`. Returns
/// the resolvable spelling either way — a bound name for an aliased width, the `(UInt N)` / `(Int N)` form
/// otherwise — so a diagnostic that suggests `<module>.wrap` never names an identifier that is not in scope
/// (PR #377 review: the raw `render_name()` suggested `UInt4.wrap`, an unbound name). `signed`/`width` are
/// read via the grounding accessors (a fixed segment width type is always concrete here).
pub(crate) fn width_module_spelling(it: &crate::ty::IntTy) -> String {
    let signed = it.ground_signed();
    let width = it.ground_width();
    let stem = if signed { "Int" } else { "UInt" };
    if crate::ty::ALIASED_INT_WIDTHS.contains(&width) {
        // An aliased width has a bound module name — `Int8`, `UInt64`.
        format!("{stem}{width}")
    } else {
        // A non-aliased width has no bound name; the type-constructor form is the resolvable spelling.
        format!("({stem} {width})")
    }
}

/// A `.wrap`/`.of` conversion spelled off a width MODULE spelling. A BOUND module name takes the ordinary
/// dotted member `UInt8.wrap`; the type-CONSTRUCTOR form `(UInt 4)` cannot take a trailing `.op` token
/// (`(UInt 4).wrap` does not lex as one member access), so it uses the explicit member form `(. (UInt 4)
/// wrap)` — both of which the resolver accepts (verified: `((. (UInt 4) wrap) n)` type-checks). Keyed off
/// the leading `(` of the constructor form.
pub(crate) fn width_conversion_spelling(module: &str, op: &str) -> String {
    if module.starts_with('(') {
        format!("(. {module} {op})")
    } else {
        format!("{module}.{op}")
    }
}

pub(crate) fn int_coercion_wrap(
    expected: &Ty,
    actual: &Ty,
    ncx: &NameCtx,
) -> Option<(String, String, String)> {
    let (kind_matches, aliased, checked) = match (expected, actual) {
        (Ty::Int(exp), Ty::Int(_)) => (
            true,
            crate::ty::ALIASED_INT_WIDTHS.contains(&exp.ground_width()),
            true,
        ),
        (Ty::Float(exp), Ty::Float(_)) => (
            true,
            crate::ty::ADMITTED_FLOAT_WIDTHS.contains(&exp.ground_width()),
            false,
        ),
        _ => (false, false, false),
    };
    if kind_matches && aliased {
        let n = expected.render_name(ncx);
        let verb = if checked {
            format!("convert to {n} with `{n}.of` (checked)")
        } else {
            format!("convert to {n} with `{n}.of`")
        };
        return Some((format!("({n}.of "), ")".to_string(), verb));
    }
    None
}

/// The coercion [`Fix`] that repairs a NUMERIC/TEXT mismatch between an `expected` type and the value
/// `arg` (of type `actual`), or `None` when no total one-shot conversion applies. Consolidates the
/// coercions the argument-unify chain offers — int-LITERAL→float retype (`3`→`3.0`, preferred over the
/// wrap), int→float (`of-int`, width-aware), int-width (`.of`), int-valued-float-literal drop
/// (`3.0`→`3`), and String→Bytes (`to-bytes`) — so EVERY site the same mismatch surfaces (an operator/ctor
/// argument, a VARIANT-CONSTRUCTOR PAYLOAD, an annotated LET-BINDER) offers the identical repair. Does NOT
/// include the sum-wrap ("wrap in `Some`", `wrap_variant_for`) — that is a distinct structural repair a
/// caller adds separately. Every fix is heuristic (`.of` is checked, retype/drop-`.0`/to-bytes are intent
/// guesses); the wrap text is a resolver-generic member-access spelling, not a hard-coded name.
pub(crate) fn numeric_text_coercion_fix(
    db: &mut Db,
    expected: &Ty,
    actual: &Ty,
    arg: StructId,
) -> Option<Fix> {
    // integer LITERAL where a float is expected → RETYPE it to a float literal (`3` → `3.0`), the same
    // one-shot repair the value-annotation site gives `(: 3 Float64)`. Preferred over the `of-int` WRAP
    // below: a literal has no reason to round-trip through a runtime conversion — just write it as a float.
    // (A non-literal int expression has no float spelling, so it falls through to the `of-int` wrap.)
    if let Ty::Float(_) = expected
        && let crate::ast::Struct::Atom(lid) = db.ast.get(arg)
        && let crate::ast::Leaf::Int { value, .. } = db.ast.leaf(*lid).clone()
        && let Some(n) = value.to_i128()
    {
        return Some(Fix::replace_heuristic(arg, format!("{n}.0")));
    }
    // int → float: `(<Float>.of-int …)`, widening a narrower int to Int64 first (`of-int : Int64 → Float`).
    if let (Ty::Float(_), Ty::Int(actual_int)) = (expected, actual) {
        let float_name = expected.render_name(&db.name_ctx());
        let (prefix, suffix, verb) =
            if actual_int.ground_width() == 64 && actual_int.ground_signed() {
                (
                    format!("({float_name}.of-int "),
                    ")".to_string(),
                    format!("convert the integer to {float_name} with `{float_name}.of-int`"),
                )
            } else {
                (
                    format!("({float_name}.of-int (Int64.of "),
                    "))".to_string(),
                    format!("convert to {float_name} with `{float_name}.of-int (Int64.of …)`"),
                )
            };
        return Some(Fix::wrap_heuristic(arg, prefix, suffix, verb));
    }
    // two integers / two floats of different width → `(<Expected>.of …)`.
    if let Some((prefix, suffix, verb)) = int_coercion_wrap(expected, actual, &db.name_ctx()) {
        return Some(Fix::wrap_heuristic(arg, prefix, suffix, verb));
    }
    // integer-valued float LITERAL where an int is expected → drop the `.0`.
    if let Ty::Int(exp) = expected
        && let crate::ast::Struct::Atom(lid) = db.ast.get(arg)
        && let crate::ast::Leaf::Float(dec) = db.ast.leaf(*lid).clone()
        && let Some(int_text) = integer_text_of_float_literal(&dec, *exp)
    {
        return Some(Fix::replace_heuristic(arg, int_text));
    }
    // a total prelude conversion (`String` → `Bytes` via `String.to-bytes`).
    if let Some((prefix, suffix, verb)) = total_conversion_wrap(expected, actual) {
        return Some(Fix::wrap_heuristic(arg, prefix, suffix, verb));
    }
    None
}

/// The coercion [`Fix`] that repairs a BARE NUMBER passed where a QUANTITY is expected — a plain `Int`/
/// `Float` `arg` against a `(Qty inner unit)` `expected` — by wrapping it in `(Qty.of <n> <unit>)` with the
/// unit read from the EXPECTED quantity type (`Unit::render` is the re-parseable `(Unit.base #"…")`
/// surface). This is the ARGUMENT-position twin of the dimensional-mismatch site's `Qty.of` wrap (which
/// offers the same repair for `(+ q 5)`): the same "give the bare number the required unit" fix wherever a
/// bare number meets a quantity. The bare number's INNER numeric type must match the quantity's inner type
/// (`Qty.of` grounds the value but does not convert its numeric type — an `Int64` bare into an `(Qty Int64
/// …)` param; a numeric mix would still fault after the wrap, so decline it and let the numeric-coercion
/// path speak). HEURISTIC — the author may have meant the magnitude of an existing quantity, but supplying
/// the required unit is the direct resolution. `None` unless `expected` is a `Qty` and `actual` a bare
/// matching-inner scalar.
pub(crate) fn qty_coercion_fix(expected: &Ty, actual: &Ty, arg: StructId) -> Option<Fix> {
    let Ty::Qty { inner, unit } = expected else {
        return None;
    };
    // The bare value must already be the quantity's inner numeric type — `Qty.of` grounds it to the unit,
    // it does not also convert Int↔Float / widen (that would still mismatch after the wrap).
    if !actual.agrees_with(inner) || matches!(actual, Ty::Qty { .. }) {
        return None;
    }
    let unit_src = unit.render();
    Some(Fix::wrap_heuristic(
        arg,
        "(Qty.of ",
        format!(" {unit_src})"),
        format!("give the number the required unit: `(Qty.of … {unit_src})`"),
    ))
}

/// The UNDERLYING structural type of the nominal NEWTYPE declared at `decl`, or `None` if `decl` is not
/// an erasable newtype (so it stays an ordinary boxed `Ty::Sum`). A newtype is a SINGLE-variant sum
/// whose runtime box is erased — the realization of `type-system.md §Nominal Is An Orthogonal Modifier`
/// (the tag "adds nothing to the value's runtime representation", §156). The returned type is the
/// nominal's `inner` (a caller wraps it as `Ty::Nominal { decl, name, inner }`); the underlying shape:
///  - 0 payloads (`(type Marker (The))`) → `Ty::Unit`,
///  - 1 payload  (`(type UserId (Mk Int64))`) → that payload's type,
///  - n payloads (`(type Point (Mk Int64 Int64))`) → `Ty::Tuple([payload tys…])` — the same shape a
///    multi-payload variant already boxes, so the erased value IS that tuple handle.
///
/// A declaration is an erasable newtype iff ALL of:
///  1. it has EXACTLY ONE variant (a multi-variant sum needs the discriminant → stays boxed),
///  2. it is MONOMORPHIC (no type parameters — a generic single-variant sum's erasure is a follow-up;
///     until then it stays boxed, which is still correct), AND
///  3. it is NON-RECURSIVE — the inner type does not transitively reach `decl` itself (via
///     `reaches_decl`, which follows every nested sum/nominal into its variants' payloads, catching a
///     direct self-reference `(type Stream (More (Tuple Int64 Stream)))` AND mutual recursion `(type A
///     (Mk B)) (type B (Mk A))`, load-order-independent). A recursive newtype's erased type is INFINITE
///     (`μX. …`), which `Ty::Nominal{inner}` cannot represent, so it stays boxed. A newtype whose inner is
///     a sum that does NOT cycle back (`(type Cached (Mk (Option Value)))`) DOES erase: its `Option` box
///     is genuine, but the outer `Mk` box (disc always 0) is pure overhead — removing it avoids the
///     double-box. (This replaced a blunt "contains ANY sum" guard that conservatively boxed every
///     newtype-over-a-sum, recursive or not.)
///
/// **Why this decodes payloads DIRECTLY (not via `scheme_of`).** It is precomputed once at load into
/// `Db::newtype_inner`, which `decode_ty` then consults to normalize a `Sum` into a `Nominal`. Using the
/// CACHED `scheme_of` here would cache the ctor's result type as `Sum` (the pre-normalization value),
/// and that stale scheme would then make `(Mk 42)` type as `Sum` even after `decode_ty` starts returning
/// `Nominal` — a representation split. Decoding the payload occurrences directly (via `typeval_of`,
/// uncached for types) avoids seeding any scheme with a soon-to-be-stale result, and the sum-free
/// restriction makes the result independent of which other decls are already cached.
pub(crate) fn newtype_underlying(db: &mut Db, decl: StructId) -> Option<Ty> {
    let (payload_count, payloads, params) = {
        let td = db.type_decl_by_occ(decl)?;
        // An OPEN sum (`(type T (Wrap Int64) .. r)`) is NEVER a newtype, even with a single NAMED variant:
        // the row variable stands for variants not named, so a value's discriminant is NOT statically the
        // sole variant — the sole constructor pattern does NOT cover it, and a match needs an open-tail
        // `_` arm (`type-system.md §206`). Erasing it to a `Ty::Nominal` would make the sole-ctor pattern
        // irrefutable (no discriminant to test), silently skipping the exhaustiveness check that requires
        // the `_`. Keep it a boxed `Ty::Sum` so `lower::build_tree`'s open-sum arm runs. (A CLOSED
        // single-variant sum erases as before — the default.)
        if td.open_tail.is_some() {
            return None;
        }
        // (1) exactly one variant. GENERICS are now IN scope — a generic newtype's template carries its
        // params as `Ty::Var(i)` (positional), substituted per-instantiation at `decode_ty`.
        if td.variants.len() != 1 {
            return None;
        }
        let v = &td.variants[0];
        (v.payloads.len(), v.payloads.clone(), td.params.clone())
    };
    // A NULLARY single variant is a nominal Unit (`(type Marker (The))`).
    if payload_count == 0 {
        return Some(Ty::Unit);
    }
    // Decode each payload TYPE occurrence directly to a template `Ty` (no `scheme_of` — see the doc
    // comment), mapping a declaration PARAM name to `Ty::Var(its index)`. A single payload's type IS the
    // underlying template; multiple payloads box as one tuple (the shape a multi-payload variant already
    // builds), so the underlying type is their `Ty::Tuple`.
    let mut tys = Vec::with_capacity(payload_count);
    for p in payloads {
        tys.push(decode_payload_template(db, p, &params)?);
    }
    let inner = if tys.len() == 1 {
        tys.pop().unwrap()
    } else {
        Ty::Tuple(tys.into())
    };
    // A RECURSIVE newtype ERASES TOO (Phase 2). Its inner's self-reference decodes to a finite
    // `Ty::Sum { decl }` LEAF (the μ-binder — `Ty::Sum` holds only the decl, not inline variants), so the
    // inner `(Option (Tuple Int64 Ty::Sum{Lst}))` is finite, NOT infinite. The old "infinite type" fear
    // was wrong. The equirecursive equality problem (a recursive nominal's inner diverging by derivation
    // path) is dissolved by Phase 1: `Ty::Nominal` is compared by `decl + args`, never `inner`. So there
    // is NO recursion guard — every single-variant sum erases; a recursive one's inner is a finite
    // machine-rep hint whose `Ty::Sum` back-edge terminates every reader.
    Some(inner)
}

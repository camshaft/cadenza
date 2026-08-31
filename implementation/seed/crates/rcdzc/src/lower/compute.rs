//! `lower::compute` — the core lowering dispatcher, split out of `lower.rs`. `compute` is the
//! per-node recursive lowering: given a resolved node's `StructId` it produces its A-normal `Core`
//! form, dispatching every construct (literals, `if`, `do`/`Seq`, `let`, `match`, calls, ctors,
//! reflection intrinsics, numeric/quantity ops, …) to the specialized lowerings that live in `lower`
//! and its sibling modules. It is invoked once, by `core_of`, and reaches every helper across the
//! module tree via `use super::*` (the parent glob-re-exports each sibling).

use super::*;

pub(super) fn compute(db: &mut Db, id: StructId) -> Core {
    // A `(do S… tail)` block whose NON-FINAL statements reach a HOST CALL lowers to a `Core::Seq` — the
    // side-effecting statements must be EMITTED (their host call crosses the boundary), then the tail is
    // the block's value. A `do` resolves to a `Ref` to its last form (`resolve_do`), which would DROP the
    // intermediates; intercept here for the effectful case so the calls are not lost. A `do` whose
    // intermediates are all PURE keeps the `Ref{last}` fold (the intermediates contribute nothing), so
    // this only fires when a non-final statement genuinely reaches a host call. Each sequenced statement
    // is a def-free value form (a do-local `(def …)` is a binding, not a statement — resolved by name).
    //
    // `Core::Seq { stmts, tail }` emits the statements in written order then the tail, and the block's
    // value is the tail (the last form) — and an earlier statement's host call is emitted before a later
    // statement's, so host effects observe the written order:
    //= spec/capabilities/core-semantics.md#a-sequencing-block-evaluates-its-forms-in-order
    //# A sequencing block MUST evaluate each of its forms in the order they are written.
    //= spec/capabilities/core-semantics.md#a-sequencing-block-evaluates-its-forms-in-order
    //# A sequencing block MUST evaluate to the value of its last form.
    //= spec/capabilities/core-semantics.md#a-sequencing-block-evaluates-its-forms-in-order
    //# A host call a form in a sequencing block makes MUST be observed before a host call made by a later form in the same block.
    if db.ast.head_name(id) == Some("do")
        && let Some(forms) = db.ast.as_form(id, "do")
        && let Some((&tail, stmts)) = forms.split_last()
    {
        // VALUE do-defs `(def x V)` that a body reference should KEEP as a runtime binding — the do-block
        // twin of a `let` binding. A do-local value def resolves to `Resolved::Ref { value: V }` (exactly a
        // `let`'s `(x V)` shape, `do_def_binds` / `last_binder_named`) but the do-block otherwise DROPS the
        // def from the Seq below and COPY-PROPAGATES `V` UNCONDITIONALLY at every reference. For a
        // multi-use HEAP value that both BORROWS (a `String.byte-len out` cond) and ESCAPES (a returned
        // if-arm `out`), copy-propagation re-emits the SAME producer at both sites, and the reclaim gate
        // frees the handle after the cond-borrow → the escaping arm returns a FREED handle (a wasm UAF:
        // wrong value, FINDING#20). A `let`-bound `out` avoids this — it routes through `should_keep_binding`
        // (count≥2 → kept → `LocalRef` → Borrowed → ONE retain/drop). So run the SAME keep-analysis over the
        // do-region for each value do-def: a kept one wraps the do-result in a `Core::Let { (V,V)… }` so its
        // refs lower to `LocalRef`; a single-use one is UNTOUCHED (copy-propagates + drops as sole owner —
        // keeping the net-0 reclaim pins honest, the case v-memory-safety's emit-tier guard over-suppressed).
        // Reuses `lower_let` verbatim (keep + dead-binding reclaim), keyed self-referentially on `V` (the
        // occurrence a reference resolves to). A FUNCTION do-def `(def (f p…) …)` has no `do_value_def_value`
        // and stays the by-name global path. Done BEFORE the Seq check so a kept value-def composes with
        // host-ordered statements (the `Core::Let` body IS the Seq/tail).
        let value_defs: Vec<StructId> = stmts
            .iter()
            .copied()
            .filter(|&f| db.ast.head_name(f) == Some("def"))
            .filter_map(|f| crate::resolve::do_value_def_value(db, f))
            .collect();
        let non_def_stmts: Vec<StructId> = stmts
            .iter()
            .copied()
            .filter(|&f| db.ast.head_name(f) != Some("def"))
            .collect();
        // The do-block's VALUE (before value-def keeping): a `Core::Seq` if a non-final statement is an
        // observable host call, else just the tail (the ordinary `Ref{last}` fold).
        let needs_seq = non_def_stmts
            .iter()
            .any(|&s| subtree_reaches_host_call(db, s));
        let do_value_node = if needs_seq {
            let seq_ty = crate::infer::type_of(db, tail);
            synth_core(
                db,
                Core::Seq {
                    stmts: non_def_stmts.into(),
                    tail,
                },
                seq_ty,
            )
        } else {
            tail
        };
        if !value_defs.is_empty() {
            // Route the value do-defs through `lower_let`'s keep-analysis. Self-keyed `(V, V)`: a body
            // reference to the do-def resolves to `Ref { value: V }`, so `V` is both the count key and the
            // kept-binding key — `lower_let` keeps only the multi-use ones and lowers their refs to
            // `LocalRef`. (A do-block with only single-use/const value-defs keeps nothing → `lower_let`
            // returns the body's core unchanged, byte-identical to the pre-fix copy-propagation.)
            let bindings: Vec<(StructId, StructId)> = value_defs.iter().map(|&v| (v, v)).collect();
            return lower_let(db, id, &bindings, do_value_node);
        }
        // No value do-defs — the plain Seq (host-ordered); else fall through to the tail's ordinary fold.
        if needs_seq {
            return core_of(db, do_value_node);
        }
    }
    match resolved_of(db, id) {
        // A bare integer literal in a `(pragma default-fraction Rational)` module grounds to the exact
        // rational `v/1` (matching the `Ty::Rational` `infer` gave it) — the lowering analogue of the
        // `default-fraction` default. Reuses `rational_from_literal` (the annotation path's grounder), so
        // `default-fraction` and an explicit `(: v Rational)` fold identically. A literal not in the map
        // (no pragma, or `<T>` not Rational) stays `ConstInt`.
        Resolved::Int(v) => default_fraction_rational(db, id).unwrap_or(Core::ConstInt(v)),
        // A rational LITERAL (`3/2` / `#rational(3 2)`) — fold its two integer children through the SAME
        // constant-fold `(Rational.of n d)` uses, so the literal and the builtin produce an identical
        // normalized `Core::ConstRational` (gcd-reduce, sign on the numerator).
        Resolved::Rational { num, den } => {
            crate::lower::arith_fold::lower_rational_of(db, num, den)
        }
        Resolved::Bool(b) => Core::ConstBool(b),
        Resolved::Str(s) => Core::ConstStr(s.into()),
        // A symbol literal (`#"meter"`) shares the constant-string REP — its identity is its text — so it
        // lowers to `Core::ConstStr` exactly like a `Symbol.of` on a constant string. Only the static type
        // (`Ty::Symbol`) differs, so `=` folds via the shared constant-string equality.
        Resolved::SymbolConst(s) => Core::ConstStr(s.into()),
        // A char literal (`#\a`) folds to its `Core::ConstChar` — a `Ty::Char` value. Constant
        // equality/ordering compare by scalar value; crossing the boundary as a char value is a later
        // increment (a char at the boundary declines).
        Resolved::Char(c) => Core::ConstChar(c),
        // A byte-string literal `b"…"` lowers to a `Core::BytesOf` of its bytes — each a fresh `UInt8`
        // `Leaf::Int` synthesized into the arena (the SAME shape `(Bytes.of (list …))` and
        // `String.to-bytes` build), so it bakes at escape, compares/slices/concats as a constant, and
        // renders back `b"…"`. No runtime op for a constant.
        Resolved::Bytes(bs) => {
            let elems: Vec<StructId> = bs
                .iter()
                .map(|&byte| {
                    db.push_atom(crate::ast::Leaf::Int {
                        value: IntValue::from_i64(byte as i64),
                        radix: crate::ast::Radix::Dec,
                    })
                })
                .collect();
            Core::BytesOf {
                elems: elems.into(),
            }
        }
        // A `(bin …)` construction in value position → the assembled byte sequence. On all-constant
        // segments it FOLDS to a `Core::BytesOf` of the emitted bytes (bakes at escape, compares/slices as
        // a constant — the same shape `b"…"`/`String.to-bytes` build); a runtime segment value takes the
        // runtime path (BN4). See `lower_bin_build`.
        Resolved::Bin { segs } => lower_bin_build(db, id, &segs),
        // A `bin` PATTERN binder reference — decode the bound segment's value FROM THE SCRUTINEE. On a
        // constant scrutinee (a visible `Core::BytesOf`) this const-folds to the decoded `ConstInt` /
        // `Core::BytesOf`; a runtime scrutinee is the BN4 cursor read (declines for now). See
        // `decode_bin_field`.
        Resolved::BinField {
            scrutinee,
            segs,
            seg_index,
        } => decode_bin_field(db, scrutinee, &segs, seg_index),
        // A MAP PATTERN binder reference — read FROM THE SCRUTINEE by key. Over a constant `Core::MapNew`
        // scrutinee (the corpus shape): a VALUE binder (`key = Some k`) folds to the entry's value at `k`;
        // the REST binder (`key = None`) folds to a `Core::MapNew` with the named keys removed. A RUNTIME
        // scrutinee reads at run time (`lower_map_field_runtime`: a value binder emits `Map.lookup`, a rest
        // binder a `Map.remove` chain), reached under the presence-test dispatch. See `lower_map_field`.
        Resolved::MapField {
            scrutinee,
            path,
            key,
            named,
            value_steps,
            value_heads,
        } => lower_map_field(
            db,
            id,
            scrutinee,
            &path,
            key,
            &named,
            &value_steps,
            &value_heads,
        ),
        // A RECORD sub-pattern binder NESTED inside a tuple/list/variant match pattern (`(match t ((tuple
        // (record (x a)) c) …))` binds `a` to field `x` of the record at tuple-slot 0). Reach the nested
        // RECORD by walking `path` from the scrutinee, then read field `key`. A record is laid out at run
        // time as a flat array in SORTED field order, read by `arr-get` at the field's sorted SLOT — exactly
        // what a `Core::SumPayload` `Elem(slot)` step emits — so the runtime read is a `SumPayload` whose
        // path is `path` extended by `Elem(<field slot>)` (the sum-payload analogue of the top-level record
        // arm's `Member`→`Proj` fold). Over a CONSTANT compound scrutinee `fold_sum_path` reaches the nested
        // `Core::Record`, and the field folds to its value directly by NAME (no slot, order-independent). The
        // field slot / record type is read at the path end via the scrutinee's solved type.
        Resolved::RecordField {
            scrutinee,
            path,
            key,
            sub_path,
            heads,
        } => {
            // CONSTANT FOLD: reach the nested record Core (a constant scrutinee folds its `Elem`/`Payload`
            // steps to the inner `Core::Record`); the field folds to its value `fv` by name. Then fold the
            // `sub_path` descent BELOW the field over `fv` (§235 deeper-nesting binder) — an EMPTY sub_path
            // (bare-binder field) yields `fv` directly. A runtime field value (sub_path fold `None`) falls
            // through to the runtime walk below.
            if let Some(Core::Record { fields }) = fold_sum_path(db, scrutinee, &path)
                && let Some(&fv) = fields.get(&key)
            {
                if sub_path.is_empty() {
                    return core_of(db, fv);
                }
                if let Some(c) = fold_sum_path(db, fv, &sub_path) {
                    return c;
                }
            }
            // RUNTIME: the field's SORTED slot in the record type at the path end (the same slot
            // `runtime_member_index` computes for a top-level member read). A record is a flat array read
            // by `arr-get` at the slot, so the field read is an `Elem(slot)` step; the `sub_path` descent
            // BELOW the field appends more `Elem`/`Payload` steps the `SumPayload` walker already handles.
            // Walk = `path ++ [Elem(slot)] ++ sub_path`.
            let rec_ty = crate::infer::record_field_at_path(db, scrutinee, &path, &heads);
            let crate::ty::Ty::Record(rec_fields) = rec_ty else {
                return Core::Poison(Reject::unsupported(
                    "a nested record match binder over a scrutinee whose nested value is not a record \
                     (or is a variant-nested record) is not supported",
                ));
            };
            let Some(slot) = rec_fields.keys().position(|k| *k == key) else {
                return Core::Poison(Reject::decline(
                    "a nested record match binder's field is absent from the record type (arm mis-selected)",
                ));
            };
            let mut walk = path.to_vec();
            walk.push(crate::core::PathStep::Elem(slot));
            walk.extend(sub_path.iter().copied());
            Core::SumPayload {
                scrutinee,
                path: walk.into(),
            }
        }
        // A RECORD REST binder — the residual record of the scrutinee's fields MINUS the `named` ones.
        Resolved::RecordRest { scrutinee, named } => {
            let named_syms: std::collections::BTreeSet<crate::resolved::Symbol> = named
                .iter()
                .filter_map(|&k| crate::resolve::read_key(db, k))
                .collect();
            // CONSTANT FOLD: a const record scrutinee folds (via the empty path) to a `Core::Record`; the
            // residual is that record with the named fields removed — itself a constant `Core::Record` (a
            // record's field set is static, so this is a fixed gather, the record twin of the map-rest fold).
            if let Some(Core::Record { fields }) = fold_sum_path(db, scrutinee, &[]) {
                let residual: std::collections::BTreeMap<crate::resolved::Symbol, StructId> =
                    fields
                        .iter()
                        .filter(|(k, _)| !named_syms.contains(*k))
                        .map(|(k, &v)| (k.clone(), v))
                        .collect();
                return Core::Record {
                    fields: std::rc::Rc::new(residual),
                };
            }
            // RUNTIME: building the residual record from an OPAQUE runtime record (a field-subset gather) is
            // not yet lowered — decline gracefully (slice 1: the const/inline-structural case folds above;
            // the runtime residual-record construction is a follow-up slice). Never a miscompile.
            Core::Poison(Reject::unsupported(
                "a runtime record rest binder (residual-record construction) is not supported (a constant/inline record-rest is)",
            ))
        }
        // A SET REST binder reaching lowering here is a FALLBACK: the set-matcher desugar
        // (`desugar_runtime_set_match`) rewrites a set-rest arm into a `Set.remove` residual binding BEFORE
        // this point, so a bare `SetRest` core is only reached if that desugar did not fire — decline
        // gracefully (never a miscompile). The residual-set VALUE construction is the desugar's slice;
        // this variant carries only the binder's TYPE (`infer` → `(Set E)`).
        Resolved::SetRest { .. } => Core::Poison(Reject::unsupported(
            "a set rest binder's residual set is built by the set-matcher desugar; a bare set-rest here is unsupported",
        )),
        // A FLOAT literal folds to its exact `Core::ConstFloat` — a `Ty::Float` value. This lets float
        // EQUALITY fold (two constants compared by canonical value). It still cannot cross the boundary
        // as a value or be an arithmetic operand (no f64 machine path yet) — those sites decline where
        // they consume it; the CONSTANT itself is now a real core value.
        Resolved::Float(d) => default_fraction_rational(db, id).unwrap_or(Core::ConstFloat(d)),
        Resolved::Unit => Core::Unit,
        // A name is its bound value's core. If that value is a KEPT `let` binding (a multi-use runtime
        // computation the enclosing `let` named once — see `lower_let`), this reference reads the
        // shared slot: `Core::LocalRef`. Otherwise the binding was copy-propagated / erased, so the
        // name IS its value's core — follow the ref (the ordinary case; a single-use or constant
        // binding leaves no `Let`).
        Resolved::Ref { value } => {
            if db.kept_bindings.contains(&value) {
                trace!(target: "rcdzc::lower", node = id.0, binder = value.0, "ref → local (kept multi-use binding)");
                Core::LocalRef { binder: value }
            } else {
                core_of(db, value)
            }
        }
        // A type annotation ERASES to its expression's core — `(: e T)` runs exactly as `e` (the
        // annotation's force is entirely on inference; it has no runtime trace). The ONE exception is a
        // numeric LITERAL annotated `Rational`: the annotation is what GROUNDS the literal to the exact
        // rational (`(: 5 Rational)` = 5/1, `(: 0.5 Rational)` = 1/2), so it must fold to a
        // `Core::ConstRational` here rather than pass through as the inner `ConstInt`/`ConstFloat` (which
        // would carry the wrong value type). Inference already grants the grounding (no CDZ0203).
        // `(const e)` — the FORCE-EVAL / const-DEMAND block (operator-requested). It REQUIRES `e` to reduce
        // to a compile-time constant. TWO fold paths, in order:
        //   1. the ORDINARY fold `core_of(e)` — everything the compiler already const-folds outside a block
        //      (Float/Int arithmetic, a constant record/tuple/list/Bytes, `Ast.encode`/`Blake3.of`, …). The
        //      block must NEVER be STRICTER than the plain fold: `(const (+ 1.5 2.0))` folds exactly like
        //      `(+ 1.5 2.0)`. This is what the general value-interpreter (`const_eval`) does NOT carry (no
        //      Float/Char/Map value), so trying it FIRST is what fixes the false CDZ0201 on those classes.
        //   2. else the general const-EVALUATOR on `e` directly — the block IS the demand signal, so it
        //      bypasses the recursive-call activation gate (`has_const_foldable_param`): a total recursion /
        //      composition over compile-time-known data folds WITHOUT threading `const` params through its
        //      callees (`const(contract-id(Ast.module))` folds though the helpers declare no const params).
        //      A taken `trap` surfaces its CDZ0304 message via `CVal::Trap`.
        // If NEITHER yields a constant, REJECT (CDZ0201): the block ASSERTS compile-time evaluability, so a
        // residual runtime value is an authoring error, not a silent pass-through to a runtime computation.
        Resolved::ConstBlock { expr } => {
            let ordinary = core_of(db, expr);
            if core_is_const_value(db, &ordinary) {
                return ordinary;
            }
            // A PROVABLE ConstTrap (CDZ0304) from the ordinary fold is FAIL-LOUD — surface it directly, so the
            // whole-expression `const_eval` below cannot mask a proven trap by taking a different reduction.
            if let Core::Poison(r) = &ordinary
                && r.code == Some(Code::ConstTrap)
            {
                return Core::Poison(r.clone());
            }
            // Otherwise try whole-expression `const_eval`. core_of's PIECEWISE lowering can DECLINE (a non-trap
            // poison) where the whole expression folds — e.g. a recursion threading a `Set`/`Map` ACCUMULATOR
            // returns an intermediate `CVal::Set`/`CVal::Map` that `cval_to_core` won't MATERIALIZE (the
            // query-only soundness guard), so the piecewise fold of the recursive call declines, yet
            // `const_eval` evaluates the whole `(const (… query (grow …)))` fine, flowing the collection
            // through the query as a `CVal`. So try `const_eval` whether `ordinary` is a (non-trap) decline
            // poison OR a non-constant runtime `Core`. A `const_eval`-discovered trap (`CVal::Trap`) still
            // surfaces as its fail-loud ConstTrap core via the `matches!` below.
            let mut budget: u64 = 1_000_000;
            if let Some(cv) = const_eval(db, expr, &CEnv::default(), &mut budget)
                && let Some(core) = cval_to_core(db, &cv)
                && (core_is_const_value(db, &core) || matches!(cv, CVal::Trap(_)))
            {
                core
            } else if let Core::Poison(r) = ordinary {
                // `const_eval` could not fold it either — surface the ORIGINAL decline (more specific than the
                // generic const-block message; e.g. a genuinely-runtime operand's own reason).
                Core::Poison(r)
            } else {
                Core::Poison(
                    Reject::coded(
                        Code::Malformed,
                        "`const` block requires a compile-time constant: this expression does not \
                         fully fold at compile time (it depends on runtime data, or the evaluator \
                         cannot reduce it at compile time)",
                    )
                    .at(id),
                )
            }
        }
        Resolved::Annot { expr, ty_expr } => {
            if matches!(
                crate::eval::typeval_of(db, ty_expr),
                Some(crate::ty::Ty::Rational)
            ) && let Some(folded) = rational_from_literal(db, expr)
            {
                folded
            } else {
                core_of(db, expr)
            }
        }
        // A sum-variant pattern's payload binder — read the scrutinee's payload. If the scrutinee is a
        // CONSTANT sum (`Core::SumNew` with a single payload), FOLD to that payload's core directly — a
        // constant `(match (Some 5) ((Some x) x))` yields the constant `5`, no heap build/read (the sum
        // analogue of a constant tuple projection folding). Otherwise it is a runtime read:
        // `sum-payload(scrutinee)` then unbox by the payload's solved type. The disc is not needed
        // (control is already in the matched arm).
        Resolved::SumPayload {
            scrutinee,
            steps,
            heads,
        } => {
            // FOLD when the whole path lands in constant `Core::SumNew` payloads — a constant `(match
            // (Some 5) ((Some x) x))` yields `5`, no heap read (extends to nesting: `(Some (Some 5))`
            // through `[Payload, Payload]` folds to `5`). Otherwise emit a runtime `Core::SumPayload`
            // that walks the path.
            if let Some(folded) = fold_sum_path(db, scrutinee, &steps) {
                folded
            } else {
                // A `Payload` step over a NOMINAL NEWTYPE is a runtime no-op (the box is erased), so it
                // emits no `sum-payload` — DROP it from the path the backend walks. `erase_nominal_steps`
                // walks the scrutinee type + heads and keeps only the real (boxed-sum / tuple) steps, so
                // the existing backend (wasm + rust) needs no nominal awareness: an empty path reads the
                // scrutinee value directly (`(Mk n)` binds `n` to the whole erased value). This is HOW a
                // program strips the name tag — a `(Mk n)` destructure is the explicit ask that yields the
                // underlying structural value — and because the `Payload` step erases to nothing, the
                // stripped value IS the same runtime value the nominal already was: a compile-time
                // reinterpretation, not a copy or conversion.
                //= spec/capabilities/type-system.md#a-nominal-value-is-convertible-to-its-underlying-structural-value
                //# A program MUST be able to strip a nominal type's name tag to obtain the underlying structural value, so that a value declared nominal can be compared or used structurally when the program explicitly asks for it rather than silently.
                //= spec/capabilities/type-system.md#a-nominal-value-is-convertible-to-its-underlying-structural-value
                //# The stripped structural value MUST be the same value the nominal value already is at runtime, so that removing the tag is a compile-time reinterpretation and not a copy or conversion of the value.
                let path = erase_nominal_steps(db, scrutinee, &steps, &heads);
                Core::SumPayload {
                    scrutinee,
                    path: path.into(),
                }
            }
        }
        // A `let` — A-NORMALIZE its bindings: a binding whose value is a runtime computation used more
        // than once is NAMED (a `Core::Let` binding, computed once, read by `LocalRef`); a single-use
        // or constant binding is copy-propagated / erased (its references follow through to its value).
        // So naming adds no cost and the emitted bytes are unchanged for a program with no multi-use
        // runtime binding (`reference-compiler.md` §The Core Representation Is In A-Normal Form).
        Resolved::Let { bindings, body } => lower_let(db, id, &bindings, body),
        // A NULLARY VARIANT used as a value (`None`) — its ctor record carries `(meta variant)` and its
        // type is the sum (no payload arrow). It constructs `sum-new(disc, unit)` with no payloads. A
        // PAYLOAD variant record used WITHOUT being applied (`Some` bare) is a function value with no
        // runtime form yet — decline (a variant constructor is applied to construct; a bare partial
        // application needs closures). This is checked before the plain-record arm so a variant is not
        // lowered as a data record of its meta fields.
        Resolved::Record { .. } if crate::eval::variant_disc_of(db, id).is_some() => {
            match crate::infer::type_of(db, id) {
                // Nullary variant value — its type is the sum directly.
                crate::ty::Ty::Sum { .. } => {
                    let disc = crate::eval::variant_disc_of(db, id).unwrap_or(0);
                    Core::SumNew {
                        disc,
                        payloads: Vec::new().into(),
                    }
                }
                // A nullary NEWTYPE value (a bare single-variant nullary ctor, `(type Marker (The))` used
                // as `The`) — erased to its underlying Unit (no box, no disc). The node's type is
                // `Ty::Nominal { inner: Unit }`, which occupies no runtime slot, exactly as `Core::Unit`.
                crate::ty::Ty::Nominal { .. } => Core::Unit,
                // A payload variant used BARE as a first-class VALUE (`W.Mk` stored in a tuple/list, or
                // returned) — ETA-EXPAND it to a runtime closure `(fn (p…) (W.Mk p…))` lifted like any
                // lambda value, so a constructor is a first-class function even when it cannot be inlined
                // away. (The inline-fold path — a bare ctor applied via an inlined HOF — is handled earlier
                // by `ctor_spine`; this is the genuine-runtime-closure case.) Falls back to declining if the
                // ctor scheme or a payload/result type has no machine representation.
                _ => eta_ctor_closure(db, id).unwrap_or_else(|| {
                    Core::Poison(Reject::decline(
                        "a variant constructor with payloads must be applied to its arguments",
                    ))
                }),
            }
        }
        // `Ast.module` used as a VALUE — an operator record whose `(meta apply)` is `Prim::ReflectModule`:
        // the type-directed self-reflection intrinsic. Fills the ENCLOSING module's own AST as an `Ast`
        // value HERE at lowering — there is no runtime "reflect the current module", so it always folds to
        // a constant. Reflected from the module's pre-resolve SOURCE snapshot (`link_inputs` captured it
        // before `Db::load` mutated the live arena), keyed by `file_of(id)` — this occurrence's file; a
        // single-file program has no linkage (`file_of` → `None`) so its snapshot is index 0.
        // `arenas_to_ast_value` walks that raw arena into a `Core::SumNew` `Ast` tree WITHOUT the resolve
        // pass (the appended nodes would otherwise be unresolved), byte-identical to what `quote`/`__ast__`
        // reflect over the same module. Declines cleanly (never a miscompile) when the built-in `Ast` sum is
        // unavailable, no snapshot exists, or the module has a leaf with no `Ast` variant (Char/Symbol).
        // Checked before the plain-record arm so it is not lowered as a data record of its meta fields.
        Resolved::Record { .. }
            if crate::eval::meta_apply_of(db, id) == Some(crate::resolved::Prim::ReflectModule) =>
        {
            // A directly-folded reflect op-record (rare — `Ast.module` is normally reached via member
            // access, handled at the `Resolved::Member` arm below with the ACCESS SITE's file). The
            // op-record is a shared, fileless built-in (the built-in `Ast` record's `module` field, the
            // SAME node for every occurrence), so here fall back to `file_of(id)` (a genuine occurrence has
            // one; a β-copy has none → file 0).
            reflect_module_value(db, db.file_of(id).unwrap_or(0))
        }
        // `Map.empty` used as a VALUE — an operator record whose `(meta apply)` is `Prim::MapEmpty`.
        // Lowers to an empty `Core::MapNew` (built on the CHAMP heap via `map-empty`). Its key/value types
        // are read off the node's solved type `Ty::Map(k, v)` (unified against its use — an empty map's
        // key/value are unconstrained until a `Map.insert`/comparison fixes them; no entries to box, so
        // `Any` is harmless here). Checked before the plain-record arm so it is not lowered as a data
        // record of its meta fields.
        Resolved::Record { .. }
            if crate::eval::meta_apply_of(db, id) == Some(crate::resolved::Prim::MapEmpty) =>
        {
            let (key_ty, val_ty) = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Map(k, v) => (*k, *v),
                _ => (crate::ty::Ty::Any, crate::ty::Ty::Any),
            };
            Core::MapNew {
                entries: Vec::new().into(),
                key_ty,
                val_ty,
            }
        }
        // A record value — kept as a compound; folds away only when a member reads a field of it.
        // `Resolved::Record.fields` and `Core::Record.fields` are BOTH `Rc<BTreeMap<…>>`, so SHARE the
        // map by an Rc clone (a refcount bump) — no O(fields) copy at all, and `Core::Record`'s own
        // per-read clone is likewise O(1).
        Resolved::Record { fields } => {
            let vals: Vec<StructId> = fields.values().copied().collect();
            if let Some(r) = reduction_bound_element(db, &vals) {
                return Core::Poison(r);
            }
            Core::Record { fields }
        }
        // Member access FOLDS: reduce the operand to a record (following refs, reducing a ctor
        // application) and lower the field's value directly, so `(. (record (x 1)) x)` and `(. (Int
        // 64) max)` both fold to the field's value with no record built. The one projection, via the
        // evaluator. A non-record operand or an absent field is a poison so a mis-projection never
        // emits a wrong value.
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(value) => {
                // `Ast.module` reflects the DEFINING module — the file of the ACCESS SITE `id` (the
                // `(. Ast module)` occurrence), NOT the shared, fileless built-in reflect op-record
                // `value` (folding which via `core_of` would default to file 0 and reflect the WRONG
                // module — the operator-confirmed use-site late-binding bug). `Resolved::Ref` folds a
                // value/nullary-def body IN PLACE (no copy, above), so a cross-module reference to
                // `def m = Ast.module` reaches this arm with `id` at the DEFINING file's occurrence, and
                // `file_of(id)` is that module. (A β-reduced/copied access site has no file → file 0.)
                if crate::eval::meta_apply_of(db, value)
                    == Some(crate::resolved::Prim::ReflectModule)
                {
                    return reflect_module_value(db, db.file_of(id).unwrap_or(0));
                }
                core_of(db, value)
            }
            // ANCHOR AT THE MEMBER NODE (`id`), symmetric with `infer::no_field_reject` (which stamps its
            // copy at the same member node): the ONE absent-field defect is reported by both the infer
            // check and this emit fold, and anchoring both at the member node lets `dedup_faults` collapse
            // them by (code, node). Without the explicit `.at(id)`, this poison reaches
            // `collect_reached_poisons` UNANCHORED and gets stamped at whatever ENCLOSING node it is reached
            // through — the redundant `((. r k))` apply wrapper, or an outer `(f (. r k))` call — a
            // DIFFERENT node than infer's member-node copy, so the two slip through as the SAME CDZ0201
            // printed twice. (A NESTED `(. (. r k) k)` still yields two, correctly: two DISTINCT member
            // nodes, each its own field read.)
            crate::eval::Member::NoField => {
                // The operand reduced to a CONCRETE record lacking `key`, but its declared TYPE (annotation-
                // wins, see `infer`'s `Resolved::Ref`) DOES carry `key`: an annotated let-binder whose
                // initializer contradicts its annotation, already reported once at the binder (CDZ0203). Emit
                // the runtime field read at the DECLARED slot rather than a second poison — mirrors the infer
                // check's cascade suppression, so the two emit paths stay symmetric and no duplicate CDZ0201
                // surfaces. (The program is already hard-rejected at the binder; this only avoids a
                // contradictory downstream diagnostic.)
                if let Some(index) = crate::eval::runtime_member_index(db, operand, &key) {
                    trace!(target: "rcdzc::lower", node = id.0, operand = operand.0, key = %key.name, index, "member access folds to the DECLARED slot (annotated-binder mismatch already reported)");
                    return Core::Proj { operand, index };
                }
                // RENAMED-OP (CDZ0603): mirror `infer::no_field_reject`'s retired-collection-op arm so
                // this emit copy carries the SAME code + node + fix as the infer copy — `dedup_faults`
                // then collapses the two by (code, node). Without this the emit copy would be a bare
                // CDZ0201 and the two codes wouldn't collapse (double report).
                if let Some(module) = db.ast.as_name(operand)
                    && let Some(new_name) = crate::infer::retired_collection_op(module, &key.name)
                {
                    let key_occ = db.ast.as_form(id, ".").and_then(|t| t.get(1).copied());
                    let reject = Reject::coded(
                        Code::RenamedOp,
                        format!(
                            "`{module}.{}` was renamed to `{module}.{new_name}`; write `(. {module} {new_name})`",
                            key.name
                        ),
                    )
                    .at(id);
                    return match key_occ {
                        Some(occ) => {
                            Core::Poison(reject.with_fix(crate::diag::Fix::replace_verified(
                                occ,
                                new_name,
                                format!("rename to `{new_name}`"),
                            )))
                        }
                        None => Core::Poison(reject),
                    };
                }
                // Match `infer::no_field_reject`'s category-aware subject/word (effect/module/type/record)
                // so the two copies share the `has no <word> \`key\`` dedup core and `dedup_faults`
                // collapses them (keeping the infer copy's did-you-mean fix). This includes the CODE: a
                // genuine RECORD-value field absence is CDZ0212 (AbsentField, the `Record.project` twin),
                // everything else CDZ0201 — flipped off the SAME `member_word` as infer's copy, so the two
                // copies keep equal codes and `dedup_faults` still collapses them (a code mismatch would
                // double-report the fixless tier-2 case).
                let (subject, member_word) = crate::infer::member_category(db, operand, &key.name);
                let code = if member_word == "field" {
                    Code::AbsentField
                } else {
                    Code::Malformed
                };
                Core::Poison(
                    Reject::coded(
                        code,
                        format!("{subject} has no {member_word} `{}`", key.name),
                    )
                    .at(id),
                )
            }
            // The operand did not reduce to a compile-time-visible record. MEMBER-INTO-IF: if it is an
            // `(if c R S)` whose BOTH branches are visible records carrying the field →
            // `(if c R.key S.key)`, pushing the member read into each branch. The record analogue of the
            // tuple `PROJECTION-INTO-IF` (a record built through an `if` was OPAQUE to `member_value`, so
            // it stayed a runtime heap value — `arr-alloc` + per-field box/set + `arr-get`/unbox, purely
            // to read one field back). Reuses the EXISTING field-value occurrences as the branches (no
            // ast synthesis, no re-resolution — each keeps its resolved scope); the un-read sibling
            // fields drop exactly as a visible-record member fold drops them, and `c` is evaluated either
            // way so its trap is preserved. `member_value` on each branch reduces it to its record and
            // projects `key` (by name — order-independent); a branch missing the field, or a kept
            // multi-use `if`-binding (`reduce_to_if` stops there), declines this and falls through to the
            // runtime read below.
            crate::eval::Member::NotRecord => {
                // A projection off a const-param fn call whose body RECURSES (a self-reflected contract
                // descriptor `contract(Ast.module).id`) does not reduce to a record via the ordinary
                // evaluator — the recursion declines, so `member_value` misses it. The general
                // const-evaluator DOES fold it (operand → constant record → project the field), so try it
                // before emitting a runtime projection (which would build a record with `Ast`-heap fields and
                // fail to emit). Reached only after `member_value` already failed, so ordinary records are
                // unaffected; `const_eval` falls through (`None`) on any non-constant. (This is the projection
                // analogue of the `Resolved::Apply` `Err`-arm const-fold — a record-returning const fn is
                // consumed by a Member, not a bare call.)
                {
                    let mut budget: u64 = 1_000_000;
                    if let Some(cv) = const_eval(db, id, &CEnv::default(), &mut budget)
                        && let Some(core) = cval_to_core(db, &cv)
                        && (core_is_const_value(db, &core) || matches!(cv, CVal::Trap(_)))
                    {
                        return core;
                    }
                }
                if let Some((cond, then_, else_)) = crate::eval::reduce_to_if(db, operand)
                    && let crate::eval::Member::Field(tf) =
                        crate::eval::member_value(db, then_, &key)
                    && let crate::eval::Member::Field(ef) =
                        crate::eval::member_value(db, else_, &key)
                {
                    trace!(target: "rcdzc::fold", node = id.0, key = %key.name, "member read pushed into an if of records (no heap build)");
                    Core::If {
                        cond,
                        then_: tf,
                        else_: ef,
                    }
                } else {
                    match crate::eval::runtime_member_index(db, operand, &key) {
                        Some(index) => {
                            trace!(target: "rcdzc::lower", node = id.0, operand = operand.0, key = %key.name, index, "member access on a runtime record → arr-get at the field's sorted index");
                            Core::Proj { operand, index }
                        }
                        None => match core_of(db, operand) {
                            Core::Poison(r) => Core::Poison(r),
                            _ => Core::Poison(Reject::coded(
                                Code::Malformed,
                                "member access requires a record",
                            )),
                        },
                    }
                }
            }
        },
        // A tuple literal — kept as a compound. Like a record, it folds away only when a projection
        // reads a visible element of it; a tuple that survives (constructed from runtime operands, or a
        // constant tuple that escapes) is a `Core::Tuple` the backend builds on the heap.
        Resolved::Tuple { elems } => {
            if let Some(r) = reduction_bound_element(db, &elems) {
                return Core::Poison(r);
            }
            // `Resolved::Tuple.elems` is already `Rc<[StructId]>`, so pass the shared slice straight
            // through to `Core::Tuple` — a move (no copy), and Core clones become refcount bumps.
            Core::Tuple { elems }
        }
        // A list literal — a `Core::ListNew` the backend builds on the persistent `vec-*` heap. (Unlike a
        // tuple, a list has no projection-fold: `List.len`/`List.at` are operations, not a static index.)
        Resolved::List { elems } => {
            if let Some(r) = reduction_bound_element(db, &elems) {
                return Core::Poison(r);
            }
            // `Resolved::List.elems` is already `Rc<[StructId]>` — pass the shared slice through.
            Core::ListNew { elems }
        }
        // A SET literal `("set" e…)` — a first-class tagged construction (operator: pulled through the
        // compiler) that lowers to the SAME `Core::SetOf` `Set.of (list …)` produces, so the set VALUE
        // still renders `(Set.of (list …sorted))`. Unlike `Set.of` over a runtime list, the elements are
        // STATICALLY enumerated here, so a runtime element VALUE is fine (the backend `set-insert`s + dedups
        // it); constant elements dedup at compile via `build_const_set`.
        Resolved::Set { elems } => {
            for &e in elems.iter() {
                if let Core::Poison(r) = core_of(db, e) {
                    return Core::Poison(r);
                }
            }
            let Some(elem_ty) = (match crate::infer::type_of(db, id) {
                crate::ty::Ty::Set(e) => Some(*e),
                _ => None,
            }) else {
                return Core::Poison(Reject::decline("set literal is not a solved set type"));
            };
            build_const_set(db, id, &elems, elem_ty)
        }
        // A map literal `(map (k v) …)` — a `Core::MapNew` the backend builds on the persistent CHAMP
        // `map-*` heap (`map-empty` + a `map-insert` per entry, in source order). The key/value types come
        // from the node's own solved `Ty::Map` (fully determined by unification — key/value homogeneity is
        // enforced in `type_errors`). A poison key/value propagates. Keys are VALUE occurrences (the
        // resolver stored them as such), so a computed key `(+ 2 3)` lowers its expression normally, and a
        // bound name keys by its value — no per-entry const-folding here yet (M3 adds the constant-map fold
        // for equality/render; the runtime build via `map-insert` is already order-canonical by CHAMP).
        Resolved::Map { entries } => {
            for &(k, v) in entries.iter() {
                if let Core::Poison(r) = core_of(db, k) {
                    return Core::Poison(r);
                }
                if let Core::Poison(r) = core_of(db, v) {
                    return Core::Poison(r);
                }
            }
            let (key_ty, val_ty) = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Map(k, v) => (*k, *v),
                _ => (crate::ty::Ty::Any, crate::ty::Ty::Any),
            };
            Core::MapNew {
                entries,
                key_ty,
                val_ty,
            }
        }
        // A tuple PROJECTION `(. t N)`. FOLD when the operand reduces to a compile-time-visible tuple:
        // lower the element's core directly (no heap, like a record member fold). Otherwise the operand
        // is a RUNTIME tuple (a parameter, a kept `let` binding) — emit a `Core::Proj` the backend lowers
        // to `arr-get`. An out-of-arity index is impossible here (rejected in `type_errors` before
        // selection); defensively, a projection past a visible tuple's arity poisons.
        Resolved::Proj { operand, index } => {
            // The operand is (transitively, through nested projections) a CAPTURED binding of an
            // enclosing closure — its base occurrence reads the env cell via `db.captured_ref`. Do NOT
            // fold the projection to the tuple's ELEMENT: reducing through the captured `let`-bound tuple
            // `(tuple n 7)` would inline the element `n` — an enclosing param that is NOT itself captured
            // — lowering it to a slot-less `Core::Param` in the lifted closure body (the captured-tuple-
            // projection ICE, hcx1; and its nested `(. (. a 0) 0)` face). Instead emit a runtime
            // `Core::Proj` whose operand lowers to `Core::Captured`/a nested runtime `Core::Proj`, reading
            // element `index` from the captured tuple env cell at runtime. (A non-captured operand still
            // folds below, unchanged.)
            if proj_operand_reaches_capture(db, operand) {
                return Core::Proj { operand, index };
            }
            match crate::eval::reduce_to_tuple_elems(db, operand) {
                Some(elems) => match elems.get(index) {
                    Some(&elem) => {
                        trace!(target: "rcdzc::fold", node = id.0, index, "tuple projection folds to a visible element");
                        core_of(db, elem)
                    }
                    None => Core::Poison(Reject::coded(
                        Code::Malformed,
                        format!("tuple index {index} is out of range"),
                    )),
                },
                None => {
                    // PROJECTION-INTO-IF: `(. (if c T E) i)` where BOTH branches are visible tuples of
                    // matching arity → `(if c T[i] E[i])`, pushing the projection into each branch. This
                    // reuses the EXISTING element occurrences as the `if`'s branches (no ast synthesis,
                    // no re-resolution — each keeps its resolved scope), so a tuple built through an `if`
                    // never reaches the heap when it is only projected: the two branch tuples fold away
                    // (their un-projected siblings drop exactly as a plain tuple projection drops them),
                    // leaving one `if` over the two selected elements. `c` is evaluated either way, so any
                    // trap in it is preserved. An out-of-arity index is impossible here (rejected in
                    // `type_errors`); defensively it poisons like the visible-tuple case.
                    if let Some((cond, te, ee)) = crate::eval::reduce_to_if_of_tuples(db, operand) {
                        match (te.get(index), ee.get(index)) {
                            (Some(&then_), Some(&else_)) => {
                                trace!(target: "rcdzc::fold", node = id.0, index, "projection pushed into an if of tuples (no heap build)");
                                Core::If { cond, then_, else_ }
                            }
                            _ => Core::Poison(Reject::coded(
                                Code::Malformed,
                                format!("tuple index {index} is out of range"),
                            )),
                        }
                    } else if let Core::Tuple { elems } = core_of(db, operand) {
                        // The RESOLVED fold (`reduce_to_tuple_elems`) sees through a `(tuple …)` literal
                        // but NOT an operand whose tuple is produced by a tuple OPERATION — `Tuple.split-at`
                        // / `Tuple.remove`, which `lower_tuple_split_at`/`lower_tuple_pop` FOLD to a constant
                        // `Core::Tuple` but which resolve as a `Prim` application, not a `Resolved::Tuple`.
                        // Fold the projection through that constant tuple's CORE, exactly as the literal
                        // path does: `(. (Tuple.split-at (tuple 10 20) 0) 1)` → element 1 = the suffix tuple
                        // `(tuple 10 20)`, with NO heap build. This is what makes `Tuple.split-at` at the
                        // k=0 / k=arity boundary — whose empty side is a `Unit` element — usable: the
                        // constant fold reaches the same representation the byte-identical literal `(tuple
                        // unit (tuple 10 20))` does, instead of a runtime `Core::Proj` whose `Unit` element
                        // hits the not-yet-built value-heap path. Only fires when the resolved fold failed
                        // AND the operand still lowered to a constant tuple (a runtime tuple's `core_of` is
                        // not `Core::Tuple`, so it correctly stays a runtime `Core::Proj` below).
                        match elems.get(index) {
                            Some(&elem) => {
                                trace!(target: "rcdzc::fold", node = id.0, index, "tuple projection folds through a constant-tuple operation result (no heap build)");
                                core_of(db, elem)
                            }
                            None => Core::Poison(Reject::coded(
                                Code::Malformed,
                                format!("tuple index {index} is out of range"),
                            )),
                        }
                    } else {
                        // GUARD: the operand must be a TUPLE (or an as-yet-`Any`/`Var` uninstantiated param,
                        // checked when a concrete tuple flows in at the call site). A CONCRETE NON-TUPLE here
                        // is an ill-typed projection that escaped `collect_node`'s CDZ0201 check (infer.rs
                        // ~13110): `collect_node` does NOT fault-check an UNCALLED inline closure body (it
                        // relies on the β-reduction call site, which never happens for a closure merely STORED,
                        // e.g. `(list (fn (v0) …))`). Such a closure's param is body-SOLVED to a scalar (seeded
                        // into `db.param_types` in `lower_lambda_value`, so `type_of` here reads it), and
                        // lowering a runtime `Core::Proj` (an `arr-get`) on that scalar slot emitted INVALID
                        // WASM (fuzzer bucket `(fn (v0) (+ v0 (. v0 0)))` stored uncalled). Decline cleanly —
                        // the same CDZ0201 the top-level twin `(def (v0) (+ v0 (. v0 0)))` gets. An `Any`/`Var`
                        // operand (`(fn (t) (. t 0))`, or a closure passed to a generic HOF) still lowers, so
                        // those are unaffected.
                        let ot = crate::infer::type_of(db, operand);
                        if !matches!(
                            ot,
                            crate::ty::Ty::Tuple(_) | crate::ty::Ty::Any | crate::ty::Ty::Var(_)
                        ) && !matches!(resolved_of(db, operand), Resolved::Poison(_))
                        {
                            trace!(target: "rcdzc::lower", node = id.0, operand = operand.0, index, "tuple projection on a concrete non-tuple declines (escaped collect_node — uncalled inline closure body)");
                            Core::Poison(Reject::coded(
                                Code::Malformed,
                                format!(
                                    "tuple projection requires a tuple, found {}",
                                    ot.render_name(&db.name_ctx())
                                ),
                            ))
                        } else {
                            trace!(target: "rcdzc::lower", node = id.0, operand = operand.0, index, "tuple projection stays runtime (operand is a runtime tuple)");
                            Core::Proj { operand, index }
                        }
                    }
                }
            }
        }
        // An `if`. FOLD when the condition reduces to a compile-time-constant boolean: the branch the
        // condition selects IS the result, so lower it directly and drop the `if`. This is dead-branch
        // elimination on a proven-constant condition — the untaken branch NEVER executes at run time.
        // WARNING: WHAT MAY BE DROPPED from the untaken branch mirrors the reachability model
        // (`compile::collect_reached_poisons`, which does NOT descend an `if`'s branches): a RUNTIME TRAP
        // shielded by an untaken branch is not a build failure, so a `ConstTrap` (CDZ0304) untaken branch
        // folds away (`(if (< 1 2) 7 (% 5 0))` → 7 — the div-by-zero is unreachable). But a NON-TRAP
        // poison — an ill-FORMED untaken branch (an unbound name, a type mismatch, an unsupported
        // literal like a float, whose branch also DISAGREES in type with the taken one, e.g.
        // `(if true 1 3.5)`) — is a static well-formedness fault the program must be REJECTED for,
        // reachability notwithstanding. So keep the `Core::If` when the untaken branch is a non-trap
        // poison, letting that fault surface; fold otherwise. A runtime condition stays a `Core::If`.
        Resolved::If { cond, then_, else_ } => {
            // NEGATED-CONDITION BRANCH SWAP: `(if (not c) t e)` ≡ `(if c e t)` — drop the negation by
            // swapping the branches. The `not` (an `i32.eqz`) is pure and `c` is evaluated either way (so
            // its trap, if any, is preserved), and the two forms select the same branch for every `c`. If
            // `c`'s core is `Core::Not { operand }`, re-drive the fold with `operand` as the condition and
            // the branches swapped — reusing the EXISTING `operand`/branch occurrences (no synthesis). A
            // `(not (not c))` unwinds one layer per swap and the inner `Not` fold cancels the rest.
            let (cond, then_, else_) = match core_of(db, cond) {
                Core::Not { operand } => (operand, else_, then_),
                _ => (cond, then_, else_),
            };
            // CONDITIONAL CONSTANT PROPAGATION on a REPEATED condition (runtime `c` only). Within the
            // THEN-branch `c` is known TRUE, within the ELSE-branch FALSE — so a branch that is ITSELF
            // `(if c' A B)` with `c'` EQUIVALENT to `c` (a syntactically-equal PURE condition; with no
            // mutation it re-evaluates identically) is redundant: take `A` in the then-branch, `B` in the
            // else-branch. Rewrite the branch to that inner arm, REUSING its existing occurrence (no
            // synthesis), so the folds below see the simplified branches (`(if c (if c A B) E)` →
            // `(if c A E)`, collapsing further if that leaves identical branches). Only a RUNTIME `c` is
            // rewritten: for a CONSTANT `c` the untaken branch is dead and the `ConstBool` arm's
            // untaken-illformed check must see the ORIGINAL branch (skip the rewrite), and a poison `c`
            // propagates. The inner `if`'s DROPPED arm may hide a runtime trap — unreachable under `c`, so
            // dropping it mirrors the reachability model (as the constant-condition fold drops a
            // `ConstTrap` untaken branch). `core_equiv`'s pure-core matching guarantees `c'` carries no
            // new effect (params/locals/consts/arith/compare/convert only).
            let (then_, else_) =
                if matches!(core_of(db, cond), Core::ConstBool(_) | Core::Poison(_)) {
                    (then_, else_)
                } else {
                    (
                        collapse_repeated_cond(db, cond, then_, true).unwrap_or(then_),
                        collapse_repeated_cond(db, cond, else_, false).unwrap_or(else_),
                    )
                };
            match core_of(db, cond) {
                Core::ConstBool(b) => {
                    let (taken, dropped) = if b { (then_, else_) } else { (else_, then_) };
                    let untaken_is_illformed = matches!(
                        core_of(db, dropped),
                        Core::Poison(r) if r.code != Some(Code::ConstTrap)
                    );
                    if untaken_is_illformed {
                        Core::If { cond, then_, else_ }
                    } else {
                        trace!(target: "rcdzc::lower", node = id.0, taken = b, "if with a constant condition folds to the taken branch");
                        core_of(db, taken)
                    }
                }
                // A condition that is a poison propagates (the ill-formed condition is the fault).
                Core::Poison(r) => Core::Poison(r),
                // A runtime condition. If BOTH branches are the SAME value (`(if c x x)`, or two branches
                // that FOLD to the same core — e.g. `(if c (+ x 0) x)` after the identity fold), the `if`
                // computes that value regardless, so it collapses to the branch — BUT only when the
                // condition is TRAP-FREE: the condition is still evaluated at run time, so if it could trap
                // (a call, a checked op) that trap must be preserved (keep the `if`). A trap-free condition
                // (a param/local, a comparison, a bitwise op) has no observable effect to keep.
                _ if core_equiv(db, then_, else_) && is_trap_free(db, cond) => {
                    trace!(target: "rcdzc::lower", node = id.0, "if with identical branches folds to the branch (trap-free condition)");
                    core_of(db, then_)
                }
                // BOOLEAN COERCION: `(if c true false)` is just `c` — the `if` computes the condition's own
                // value. `c` is a `Bool` (an `if` condition must be), and it is evaluated on BOTH branches of
                // the original, so returning it drops the `if` with no change (including any trap in `c`,
                // which still fires — `c` is unconditionally evaluated here just as it was as the condition).
                _ if matches!(core_of(db, then_), Core::ConstBool(true))
                    && matches!(core_of(db, else_), Core::ConstBool(false)) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "if c true false folds to the condition c");
                    core_of(db, cond)
                }
                // BOOLEAN NEGATION: `(if c false true)` is `!c`. `c` is unconditionally evaluated (as the
                // condition), so negating its value drops the `if` with no other change (any trap in `c`
                // still fires). A runtime `c` becomes `Core::Not{c}` (emitted as `i32.eqz`); a constant `c`
                // would already have folded via the `ConstBool` arm above, so here `c` is a runtime bool.
                _ if matches!(core_of(db, then_), Core::ConstBool(false))
                    && matches!(core_of(db, else_), Core::ConstBool(true)) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "if c false true folds to the negation !c");
                    Core::Not { operand: cond }
                }
                // IF-ENCODED CONNECTIVE: `(if c a false)` IS `(and c a)` and `(if c true b)` IS `(or c b)` —
                // an `if` with ONE boolean-constant branch is exactly a short-circuit connective (same
                // evaluation order, same trap behavior: `c` always runs, the other branch runs only on the
                // deciding polarity). Rerouting through `fold_short_circuit` unlocks the WHOLE boolean-algebra
                // fold family (subsumption/absorption/complement/comparison-pair) for if-encoded booleans —
                // e.g. `(if (> x 5) (> x 3) false)` collapses to `(> x 5)` — and is a strict emit improvement
                // (branchless `i32.and`/`i32.or` vs a `select`/`if` block). The kept condition `c` preserves
                // any trap (it is the always-evaluated `lhs`). Only fires for a RUNTIME `c` (a constant `c`
                // folded in the `ConstBool` arm above) with the OTHER branch a runtime bool (a both-constant
                // `if` was caught by the coercion/negation/identical-branch arms above). `then_`/`else_` are
                // the post-swap occurrences, reused directly (no synthesis). VETOED when the branch that
                // would become the connective's guarded `rhs` holds a TAIL CALL (`tail_positions_have_call`):
                // the loop transform only threads tail calls through `if`/`let`/`match`, not `and`/`or`, so
                // burying a tail-recursive call in a connective would defeat tail-loop conversion (a bigger
                // win than a branchless boolean) — e.g. `(if (= n 0) true (odd (- n 1)))` MUST stay an `if`.
                _ if matches!(core_of(db, else_), Core::ConstBool(false))
                    && !tail_positions_have_call(db, then_) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "(if c a false) → (and c a)");
                    fold_short_circuit(db, cond, then_, true)
                }
                _ if matches!(core_of(db, then_), Core::ConstBool(true))
                    && !tail_positions_have_call(db, else_) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "(if c true b) → (or c b)");
                    fold_short_circuit(db, cond, else_, false)
                }
                // The OTHER two if-with-one-boolean-constant patterns, where the constant is in the position
                // that flips the connective's condition to `(not c)`:
                //   `(if c a true)`  IS `(or (not c) a)`  — else is `true` (result is true unless c holds
                //       and a fails), so `(not c)` short-circuits the `or` to true when c is false.
                //   `(if c false b)` IS `(and (not c) b)` — then is `false` (result is false unless c fails
                //       and b holds), so `(not c)` short-circuits the `and` to false when c is true.
                // Same soundness as the two above: `(not c)` is the always-evaluated short-circuit LHS (c's
                // trap is preserved; `not` is total), and the runtime branch is the guarded RHS, evaluated
                // on exactly the original's deciding polarity. The negated condition is synthesized and
                // routed through `fold_short_circuit`, so `(not c)` folds (`(not (> x 10))`→`(<= x 10)`) and
                // the whole thing joins the boolean-algebra fold family — e.g. `(if (> x 10) (< x 5) true)`
                // → `(or (<= x 10) (< x 5))` → `(<= x 10)` (subsumption). Same tail-call veto on the guarded
                // runtime branch.
                _ if matches!(core_of(db, else_), Core::ConstBool(true))
                    && !tail_positions_have_call(db, then_) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "(if c a true) → (or (not c) a)");
                    let not_c = synth_core(db, Core::Not { operand: cond }, crate::ty::Ty::Bool);
                    fold_short_circuit(db, not_c, then_, false)
                }
                _ if matches!(core_of(db, then_), Core::ConstBool(false))
                    && !tail_positions_have_call(db, else_) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "(if c false b) → (and (not c) b)");
                    let not_c = synth_core(db, Core::Not { operand: cond }, crate::ty::Ty::Bool);
                    fold_short_circuit(db, not_c, else_, true)
                }
                // IF-TOWER FLATTENING (shared-arm condition combination). Two nested `if`s that share an
                // arm collapse to ONE `if` on a COMBINED condition, replacing a nested branch with a single
                // (backend-selectable-branchless) decision:
                //   `(if c1 x (if c2 x y))` → `(if (or c1 c2) x y)`  — the THEN arm `x` is shared (taken
                //       when c1, OR when !c1 && c2 — i.e. `c1 || c2`).
                //   `(if c1 (if c2 x y) y)` → `(if (and c1 c2) x y)` — the ELSE arm `y` is shared (`x` taken
                //       only when c1 && c2).
                // SHORT-CIRCUIT ORDER PRESERVED: `or`/`and` evaluate `c1` then `c2` (c2 only on the
                // deciding polarity), exactly as the nested `if` did — so a trap/effect in `c2` fires under
                // the same conditions. The shared arm and the surviving inner arm stay in `if`-branch
                // (guarded) positions, so trapping branches keep their shielding AND the tail-loop transform
                // is unaffected (no call is moved into a connective — only the two CONDITIONS combine).
                // `reduce_to_if` sees through refs/annotations/non-recursive calls to the inner `if`; the
                // combined condition is synthesized (`fold_short_circuit` folds it — `(or (> x 5) (> x 3))`
                // etc.) and the inner arms are reused by occurrence. A constant `c1` was handled above.
                _ if let Some((c2, t2, e2)) = crate::eval::reduce_to_if(db, else_)
                    && core_equiv(db, then_, t2) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "(if c1 x (if c2 x y)) → (if (or c1 c2) x y)");
                    let combined = fold_short_circuit(db, cond, c2, false); // (or c1 c2)
                    let cid = synth_core(db, combined, crate::ty::Ty::Bool);
                    Core::If {
                        cond: cid,
                        then_,
                        else_: e2,
                    }
                }
                _ if let Some((c2, t2, e2)) = crate::eval::reduce_to_if(db, then_)
                    && core_equiv(db, else_, e2) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, "(if c1 (if c2 x y) y) → (if (and c1 c2) x y)");
                    let combined = fold_short_circuit(db, cond, c2, true); // (and c1 c2)
                    let cid = synth_core(db, combined, crate::ty::Ty::Bool);
                    Core::If {
                        cond: cid,
                        then_: t2,
                        else_,
                    }
                }
                // COMMON-CONSTRUCTOR HOIST: both arms build the same `SumNew`/`Tuple` (same disc + arity),
                // so build it ONCE and push each differing payload into its own `(if c pᵢ qᵢ)` — one heap
                // build emitted instead of two duplicated ones. See `hoist_common_ctor` for soundness.
                _ if let Some(core) = hoist_common_ctor(db, cond, then_, else_) => {
                    trace!(target: "rcdzc::lower", node = id.0, "(if c (K …p) (K …q)) → (K …(if c pᵢ qᵢ)) — common-constructor hoist");
                    core
                }
                // COMMON-OPERATOR HOIST: both arms apply the same arith/convert op, so apply it ONCE over
                // the selected operands — `(if c (+ a 1) (+ b 1))` → `(+ (if c a b) 1)`, one checked add +
                // guard instead of two. See `hoist_common_arith` for soundness.
                _ if let Some(core) = hoist_common_arith(db, cond, then_, else_) => {
                    trace!(target: "rcdzc::lower", node = id.0, "(if c (op …p) (op …q)) → (op …(if c pᵢ qᵢ)) — common-operator hoist");
                    core
                }
                // A plain runtime `if`. Both branches are CONDITIONALLY reached (guarded by `cond`), so a
                // branch that const-folded to a provable ConstTrap DEMOTES to a runtime `Core::Trap` — it traps
                // only when taken, not at compile time (cn02 / operator ruling). An unconditional trap on the
                // strict spine is not here, so it still surfaces CDZ0304.
                _ => {
                    let then_ = demote_conditional_trap(db, then_);
                    let else_ = demote_conditional_trap(db, else_);
                    Core::If { cond, then_, else_ }
                }
            }
        }
        // A SHORT-CIRCUITING connective. Delegated to `fold_short_circuit`, which also serves the
        // `(if c a false)`→`(and c a)` / `(if c true b)`→`(or c b)` rewrites above (an if-encoded
        // connective routes through the SAME boolean-algebra fold family).
        Resolved::And { lhs, rhs, is_and } => fold_short_circuit(db, lhs, rhs, is_and),
        // Negation: fold a constant, `(not (not x))` → x (double negation), else `Core::Not` (i32.eqz).
        Resolved::Not { operand } => match core_of(db, operand) {
            Core::ConstBool(b) => Core::ConstBool(!b),
            // Double negation: the operand is itself a `Not` — the two cancel, so the result is the INNER
            // operand's core. `not` is total (no trap, no effect), so cancelling the pair changes nothing.
            Core::Not { operand: inner } => core_of(db, inner),
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Not { operand },
        },
        // `(try e)` — the fallible short-circuit operator (`DESIGN-try-operator-rcdzc.md` §3.2/§4 v1). The
        // enclosing FUNCTION body IS the boundary (v1): a `?`'s failure arm's value flows out as the
        // function's value — there is no separate boundary-block node for v1 (§4). Two constant folds:
        //   * CONSTANT SUCCESS (`Some x`/`Ok x`) → the payload; the happy path never short-circuits (BRICK 2a).
        //   * CONSTANT FAILURE (`None`/`Err`) → `Core::Break { value }`; the failure short-circuits the
        //     boundary, and `lower_let`/the strict-spine propagate the `Break` up to the boundary body's
        //     value (BRICK 3a). A `Break` on the UNCONDITIONAL strict spine folds the enclosing `let`/spine
        //     to the failure value; a conditional/runtime failure needs the real block/br (a later brick).
        // A `?` on a RUNTIME operand still declines (never a miscompile — §An Unsupported Construct Is
        // Declined). The operand is walked either way so its own core faults surface.
        Resolved::Try { operand } => match core_of(db, operand) {
            Core::Poison(r) => Core::Poison(r),
            // A constant success variant: `(try (Some x))` / `(try (Ok x))` folds to the payload. The
            // success disc is read off the operand's solved Option/Result type by variant NAME
            // (`option_discs`/`result_discs`), never assumed positionally.
            Core::SumNew { disc, payloads }
                if success_disc_of(db, operand) == Some(disc) && payloads.len() == 1 =>
            {
                core_of(db, payloads[0])
            }
            // A constant FAILURE variant: `(try (None))` / `(try (Err r))` — the `?` short-circuits the
            // enclosing boundary, so the whole `(try e)` becomes a `Core::Break` carrying the FAILURE value
            // (the operand itself, e.g. `None`/`Err r`, which is `T_B`-typed — it IS the boundary result).
            // `lower_let` folds a `let` whose init is this `Break` to the break value (the failure flows out
            // as the boundary body's value); a `Break` reaching the backend un-folded declines (BRICK 3b).
            Core::SumNew { disc, .. } if success_disc_of(db, operand) == Some(disc) => {
                // Success disc but arity ≠ 1 — a malformed success (should not reach here); decline.
                Core::Poison(Reject::decline(
                    "the `?`/`try` operator requires a single-payload success operand",
                ))
            }
            Core::SumNew { .. } => Core::Break { value: operand },
            // The boundary break for a RUNTIME operand is a later brick; until then the message names
            // the current limit (seq-280: the "later" intent stays here in the comment).
            _ => Core::Poison(Reject::decline(
                "the `?`/`try` operator lowers only a constant operand",
            )),
        },
        // A match over a scalar scrutinee — FOLD when the scrutinee is a constant (select the arm whose
        // probe it satisfies), else emit a `Core::Match` the backend lowers to a probe chain.
        Resolved::Match { scrutinee, arms } => lower_match(db, scrutinee, &arms),
        // `nan` — the canonical NaN Float VALUE (a bare prim naming a value, not an operation). Lowers to
        // `Core::ConstFloatNan`; folds in `=` by the canonical byte form.
        Resolved::Prim(Prim::FloatNan) => Core::ConstFloatNan,
        // `Infinity` — the positive-infinity Float VALUE (a bare prim naming a value). Lowers to
        // `Core::ConstFloatInf`; folds in `=`/ordering by IEEE (`+∞ = +∞`, `+∞ > every finite`).
        Resolved::Prim(Prim::FloatInf) => Core::ConstFloatInf,
        // A bare built-in operation value that is not applied has no runtime form yet (no closures) —
        // it declines. Applying it is what lowers.
        Resolved::Prim(_) => Core::Poison(Reject::unsupported(crate::diag::PRIM_AS_VALUE_DECLINE)),
        // Application — the ONE path, dispatched by the head value's `(meta apply)` primitive. An
        // arithmetic prim folds (below); a type-constructor prim reduces via the evaluator to a built
        // value (a module / type-value), which is then lowered — a member projection off it folds, a
        // bare type/module used at runtime declines at the erasure fence.
        Resolved::Apply { head, args } => {
            // A PERFORM that reaches lowering directly — no enclosing handler discharged it (a handled
            // perform is REDUCED AWAY by `effects::reduce_handle` before its body is lowered, so it never
            // reaches here) and no host delegation routed it (E2). Whether this is an ERROR depends on
            // CONTEXT: an unhandled perform reached from an ENTRYPOINT escapes ungranted (CDZ0401 — the
            // "no home" check, reported at the export level in `compile.rs`), but a perform in a LIBRARY
            // function's body is fine — its home is whatever handler/delegation encloses its CALLERS (the
            // cross-function inline trigger resolves it there). So here — the standalone lowering of an
            // arbitrary def body — a bare perform is a DECLINE, not a coded reject: a library def that
            // performs an effect stays well-formed, while the entrypoint-level check catches a genuinely
            // ungranted escape. (Reported cleanly rather than leaking the op's `(intrinsic perform)` marker
            // as an "unknown intrinsic".)
            if crate::eval::effect_op_of(db, head).is_some() {
                // A perform DELEGATED to the host by an enclosing `(host (E…) …)` lowers to a HOST CALL —
                // the operation is a component-level import the boundary resolves (E2). If no enclosing
                // `host` delegates this effect, the perform is unhandled here: a DECLINE (a library def
                // performing an effect whose home is its callers), and the entrypoint `check_no_home`
                // reports a genuine ungranted escape as CDZ0401.
                if let Some((effect, op, result)) =
                    crate::effects::perform_host_target(db, id, head)
                {
                    // SCHEMA-HASH PHASE-1a FORK (sync-vs-async perform discriminator) — fires ONLY in a
                    // REDUCER-WORLD compile (`db.wit_world` present): the target world declares the reducer's
                    // synchronous IMPORTS (kv). A host-delegated perform whose op is a func member of an
                    // import interface has a synchronous backing import → stays a `Core::HostCall` (the kv
                    // path). A perform whose op is NOT an import member is a WORLD-effect (Model/Tool/Emit)
                    // with no synchronous result — by the fold-purity contract its result defers to a later
                    // `apply`, so it REIFIES into the returned effect-list as an effect-request `Core::Record`
                    // (v-ah ruling: apply returns the effect-list; the reify is capability-blind — all args
                    // ride the payload, no target split). With NO target world (a PLAIN module's `(host (E)
                    // (E.op …))` — no reducer world), there is no import/world split: the perform is the
                    // ordinary host-delegated `Core::HostCall` exactly as before (the reify is a reducer-world
                    // concept). Reads the DECLARED world contract (`is_world_import_op`), zero hard-coded
                    // capability vocabulary (GENERIC-COMPILER-clean).
                    let args_vec = args.to_vec();
                    let world = db.wit_world.clone();
                    if let Some(world_bytes) = &world
                        && !crate::wit_world::is_world_import_op(Some(world_bytes), &effect, &op)
                    {
                        // The op's `@resource`-marked param index (`OpDecl.resource`) — the SEC-F1
                        // destination arg the reify routes to the `target` wire field (ruling A: the dest is a
                        // runtime value, not dropped). Resolved from the perform's `(effect-op <decl> <idx>)`
                        // channel; `None` for a target-free op (which reifies to a 3-field record).
                        let effect_op = crate::eval::effect_op_of(db, head);
                        let resource = effect_op.and_then(|(decl, op_idx)| {
                            db.effect_decl_by_occ(decl)
                                .and_then(|e| e.ops.get(op_idx as usize))
                                .and_then(|o| o.resource)
                        });
                        // The effect DECLARATION occurrence — for the phase-3 schema_descriptor bake (the
                        // descriptor is per-effect, so the op index is irrelevant here).
                        let effect_decl = effect_op.map(|(decl, _)| decl);
                        trace!(target: "rcdzc::lower", node = id.0, %effect, %op, ?resource, "apply: reducer-world NON-import world-effect → reify to effect-request record (async)");
                        return reify_effect_to_tuple(
                            db,
                            &effect,
                            &args_vec,
                            resource,
                            effect_decl,
                        );
                    }
                    trace!(target: "rcdzc::lower", node = id.0, %effect, %op, "apply: host-delegated perform → Core::HostCall (sync import / plain host-delegation)");
                    return Core::HostCall {
                        effect: effect.into(),
                        op: op.into(),
                        args: args_vec.into(),
                        result,
                    };
                }
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: unhandled perform at standalone lowering → decline (entrypoint check reports CDZ0401)");
                return Core::Poison(Reject::decline(crate::diag::NO_HOME_STANDALONE_DECLINE));
            }
            // (The old cross-component `Core::ExternCall` producer was REMOVED in U4 — a peer op is now an
            // effect bound to a peer, so it flows through the perform → `Core::HostCall` path above and the
            // backend routes a peer-bound effect's `HostCall` to the peer envelope.)
            // CASE-OF-CASE (commuting conversion): a head that reduces to a runtime `if` —
            // `((if c a b) args…)` — pushes the application into each branch: `(if c (a args…)
            // (b args…))`. A runtime-branch-SELECTED function (`(if b (fn …) (fn …))` applied) then
            // has each branch's lambda β-reduce in place, so the whole thing folds with NO closure
            // value surviving to run time. Sound because `if` branches are pure values (evaluating the
            // application in the taken branch is what the original did) and only ONE branch runs. Built
            // by synthesizing the two branch applications (head = each branch, same args) and an `if`
            // over the same condition, then lowering that — the ordinary `Resolved::If` fold handles a
            // constant condition / identical branches. Guarded on a NON-constant reduction target too
            // (a constant `if` already folds its head to a single branch upstream, but this is
            // harmless there). Checked before the lambda-head path since an `if` head is not a lambda.
            if let Some((cond, then_head, else_head)) = crate::eval::reduce_to_if(db, head) {
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: case-of-case — push the application into each if branch");
                // An application `(head arg…)` is a plain list with the head first, so each branch
                // application is `push_list([branch_head, args…])`; `(if cond then_app else_app)` is a
                // list headed by the `if` name. Lowering the rewritten `if` runs the ordinary fold.
                let then_app = {
                    let mut v = vec![then_head];
                    v.extend_from_slice(&args);
                    db.push_list(v)
                };
                let else_app = {
                    let mut v = vec![else_head];
                    v.extend_from_slice(&args);
                    db.push_list(v)
                };
                let if_head = db.push_name("if");
                let rewritten = db.push_list(vec![if_head, cond, then_app, else_app]);
                return core_of(db, rewritten);
            }
            // CASE-OF-MATCH (the `match` analogue of case-of-case): a head that reduces to a runtime
            // `match` — `((match c (pat0 f0) (pat1 f1)…) args…)` — pushes the application into each arm
            // BODY: `(match c (pat0 (f0 args…)) (pat1 (f1 args…))…)`. A match whose arms return CLOSURES
            // (`(match c ((C.A n) (fn (x) (+ x n))) …)`) then has each arm's lambda β-reduce in place
            // against the args, so the whole thing folds with no closure value surviving — exactly as the
            // `if` case does for `(if c (fn …) (fn …))`. Sound because only ONE arm runs (evaluating the
            // application in the taken arm is what the original did), and the arm PATTERN nodes are reused
            // verbatim so their binders stay in scope for the rewritten body `(f args…)`. Rebuilt from the
            // match form's AST (`(match scrutinee arm…)`, each arm `(pat body)`), then lowered through the
            // ordinary `Resolved::Match` path. Checked before the lambda-head path (a match head is not a
            // lambda) and after case-of-case (an `if` is not a match).
            if let Some(match_form) = crate::eval::reduce_to_match(db, head)
                && let Some(mtail) = db.ast.as_form(match_form, "match").map(<[_]>::to_vec)
                && let [scrutinee, arm_occs @ ..] = mtail.as_slice()
            {
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: case-of-match — push the application into each arm body");
                let scrutinee = *scrutinee;
                let mut new_arms: Vec<StructId> = Vec::with_capacity(arm_occs.len());
                let mut ok = true;
                for &arm in arm_occs {
                    // Each arm is `(pattern body)`; rewrite to `(pattern (body args…))`.
                    let (pat, body) = match db.ast.get(arm) {
                        crate::ast::Struct::List(kv) if kv.len() == 2 => (kv[0], kv[1]),
                        _ => {
                            ok = false;
                            break;
                        }
                    };
                    let body_app = {
                        let mut v = vec![body];
                        v.extend_from_slice(&args);
                        db.push_list(v)
                    };
                    new_arms.push(db.push_list(vec![pat, body_app]));
                }
                if ok {
                    let match_head = db.push_name("match");
                    let mut items = vec![match_head, scrutinee];
                    items.extend(new_arms);
                    let rewritten = db.push_list(items);
                    // Resolve the rewritten subtree against its NEW positions before lowering: each arm's
                    // rewritten body `(f args…)` and the pattern binders it references must re-resolve
                    // against the re-parented arm, exactly as `apply_lambda` pins an argument subtree
                    // before splicing. Without this a payload binder a closure arm captures (`(fn (x) (+ x
                    // n))` capturing the arm's `n`) kept a stale/absent resolution and reported `n` unbound.
                    crate::resolve::resolve_subtree(db, rewritten);
                    return core_of(db, rewritten);
                }
            }
            // A CURRIED CONSTRUCTOR SPINE — `((Pair 3) 4)`. A sum constructor is single-arity, so the
            // nested-parens surface is the SAME construction as the flat `(Pair 3 4)` (core-semantics.md
            // §A Sum Type Constructor Is A Single-Arity Function; §Functions Are Single-Arity). The flat
            // form has a bare `(. Sum V)` head and takes the `Some(Prim::SumNew)` path below directly;
            // this handles the case where the head is ITSELF an `Apply` of an under-applied constructor
            // (which otherwise reaches the `None` "not applyable" arm, since a half-applied ctor value has
            // no `(meta apply)`). `ctor_spine` peels the nested heads to the bottom variant constructor and
            // gathers every payload left-to-right; when the count reaches the variant's full payload arity,
            // build it exactly as the flat form does (`lower_sum_new`). A spine that stops SHORT of arity —
            // a genuinely partial constructor bound/returned as a first-class value — is left to fall
            // through (it needs a runtime closure, a later increment), and an OVER-applied spine likewise
            // falls through to the existing arity diagnostics. Checked before the runtime-closure/lambda
            // paths since a ctor spine matches none of those (the bottom head is a constructor record).
            // Only engage for genuine NESTING — the immediate head is itself an `Apply` (`((Pair 3) 4)`)
            // or a `Ref` to a partial ctor (`(let ((g (Pair 3))) (g 4))`). A FLAT `(Pair 3 4)` has the
            // bare ctor record as its head; it keeps its established `Some(Prim::SumNew)` path below
            // (byte-identical output), so this diverts nothing that already worked.
            if crate::eval::variant_disc_of(db, head).is_none()
                && let Some((ctor, all_args)) = ctor_spine(db, id)
                && crate::eval::variant_payload_arity(db, ctor) == Some(all_args.len())
                && !all_args.is_empty()
            {
                trace!(target: "rcdzc::lower", node = id.0, head = ctor.0, n_args = all_args.len(), "apply: curried constructor spine → flat sum construction");
                return lower_sum_new(db, id, ctor, &all_args);
            }
            // A RUNTIME CLOSURE APPLICATION: the head is a runtime FUNCTION VALUE that does NOT reduce to
            // a compile-time lambda and is NOT a known constructor/operator/type-builder — a
            // function-typed PARAMETER `g` applied inside a body (`(g n)` / `(g a b)`), or a runtime-held
            // closure. It cannot β-reduce (its value is unknown at compile time), so it applies via
            // `call_indirect`: lower to `Core::CallClosure`. The head must be a `Resolved::Param` (the
            // only runtime function-value source); a sum-variant constructor (`Ok`, whose type is also an
            // arrow), an operator prim, a type builder, and a named def all have their own paths
            // (constructors build, prims fold, defs β-reduce/inline) and must NOT be diverted here — so
            // this is gated on the head being a bare parameter, not merely on its type being `Ty::Fn`.
            // A multi-arg application `(g a b)` is a FULL-arity call of a multi-param closure (all args
            // pushed to one `call_indirect`). CURRIED SYNTAX — `((g n) 1)` — is the SAME full-arity call
            // written with nested parens: the head `(g n)` is ITSELF an application of the runtime fn `g`,
            // so the whole spine is `g` applied to `[n, 1]`. `runtime_fn_spine` peels the nested `Apply`
            // heads and gathers every argument left-to-right, reaching the ONE runtime fn value at the
            // bottom; the accumulated args go to a single `call_indirect` (`closure_type_index` peels
            // `args.len()` arrows off the closure's curried type to match the lifted lambda). A genuine
            // PARTIAL application — a spine that stops SHORT of full arity, e.g. `(g n)` bound and returned
            // — still declines at select (no lifted lambda's arity matches the short arg list), since it
            // would need to build an intermediate closure. Checked before the lambda-reduction path.
            if let Some((fn_head, all_args)) = runtime_fn_spine(db, id) {
                if all_args.is_empty() {
                    return Core::Poison(Reject::decline(
                        "a runtime closure applied to no arguments",
                    ));
                }
                // A genuine PARTIAL application of a runtime closure — the gathered arg count is FEWER than
                // the closure's curried arity — has no runtime form yet: it would need a residual closure
                // (capturing the supplied args + awaiting the rest), which the emit does NOT build. Left to
                // emit an under-arity `Core::CallClosure`, it produced an INVALID module (the residual's
                // machine rep disagrees with a later `call_indirect` — v-effects' `(let ((g (f 3))) (g 4))`
                // over a boxed 2-param closure: `(f 3)` is a 1-of-2 partial that mis-emitted, func-N wasm
                // 'expected i64 found i32'). DECLINE cleanly instead of emitting an invalid module (a
                // miscompile). The closure's ARITY is the number of arrows its type peels — `(-> a (-> b c))`
                // is arity 2; `all_args.len()` short of that is a partial. A FULL application (args == arity)
                // and an OVER-application (a curried spine gathering ≥ arity, handled by the flatten) keep
                // building the `CallClosure`. This is the clean-decline stopgap (v-effects co-owned); the
                // genuine chained-residual-closure lift is a later capability. `type_of(fn_head)` is the
                // closure's curried arrow (`check` grounds it — `check` is clean on this repro, so the arity
                // read is sound); a non-arrow / unresolved head has arrow-count 0 and is not treated as a
                // partial (unchanged).
                let closure_arity = {
                    let mut ty = crate::infer::type_of(db, fn_head);
                    let mut n = 0usize;
                    while let crate::ty::Ty::Fn(_, r) = ty {
                        n += 1;
                        ty = *r;
                    }
                    n
                };
                if closure_arity > 0 && all_args.len() < closure_arity {
                    // RESIDUAL-CLOSURE LIFT: eta-abstract the missing params into a synthesized lambda
                    // `(fn (__eta…) (fn_head supplied… __eta…))` and lift it — the closure + supplied args
                    // capture into the residual closure's env, and its body is a FULL application (→ a valid
                    // `CallClosure`), so the machine reps agree (no under-arity `CallClosure` miscompile).
                    if let Some(core) = partial_closure_eta_closure(db, fn_head, &all_args) {
                        trace!(target: "rcdzc::lower", node = id.0, head = fn_head.0, n_args = all_args.len(), arity = closure_arity, "apply: PARTIAL application of a runtime closure → residual-closure lift");
                        return core;
                    }
                    // Synthesis could not classify the eta-lambda (an unresolved/degenerate head) — decline
                    // cleanly rather than emit an under-arity call (still reject-don't-miscompile).
                    trace!(target: "rcdzc::lower", node = id.0, head = fn_head.0, n_args = all_args.len(), arity = closure_arity, "apply: PARTIAL application of a runtime closure → decline (residual-closure synthesis failed)");
                    return Core::Poison(Reject::unsupported(
                        "a partial application of a runtime closure (fewer arguments than its arity) \
                         is not supported — apply it to all its arguments, or wrap the remaining \
                         ones in an explicit lambda",
                    ));
                }
                trace!(target: "rcdzc::lower", node = id.0, head = fn_head.0, n_args = all_args.len(), "apply: runtime closure application (spine-flattened) → Core::CallClosure");
                return Core::CallClosure {
                    closure: fn_head,
                    args: all_args.into(),
                };
            }
            // An `inline-never` def is emitted as ONE real wasm function and CALLED, never β-reduced
            // (Addendum 4 — the author's inline-policy marker). Route it to `emit_call_or_specialize`
            // BEFORE the inline path below: that shared path also SPECIALIZES a generic / `const`-param
            // callee (so an `inline-never` generic still monomorphizes per type, and an `inline-never`
            // `const`-dict def still erases the dict — "avoid the inline but keep polymorphism"). Only for a
            // named top-level def whose body is in `db.inline_never` (`callee_def_index` resolves the head;
            // a non-def / computed head is not markable and falls through to β-reduce as usual).
            if !db.inline_never.is_empty()
                && let Some(callee) = callee_def_index(db, head)
                && db.defs[callee]
                    .body
                    .is_some_and(|b| db.inline_never.contains(&b))
            {
                // MANDATORY-INLINE guard: an `inline-never` call whose result is compile-time-DEMANDED (it
                // feeds a `const` param / a type position / a constant fold) cannot be emitted as a runtime
                // call without breaking that demand — reject rather than miscompile. Detected structurally
                // by whether the call β-reduces to a compile-time VALUE the caller needs: if the reduced
                // result is a pure constant the surrounding context would fold, honoring `inline-never`
                // would strand it. Conservative form: emit the call; if a later pass needed the constant it
                // would already have folded pre-lowering. (A dedicated demand check is a future refinement;
                // today `inline-never` on a truly const-demanded call is rare and the emit is still sound —
                // the value crosses as a runtime call result.) Emit the call/specialization:
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, callee, "apply: inline-never def → emit_call_or_specialize (no inline)");
                return emit_call_or_specialize(db, head, callee, &args);
            }
            // COST HEURISTIC (Addendum 4): the UNANNOTATED default is always-inline, but a LARGE
            // (≥ INLINE_COST_THRESHOLD nodes), MULTIPLY-CALLED (≥ INLINE_MIN_CALLERS) def whose call has a
            // RUNTIME-CAPTURING argument is emitted ONCE and called instead — duplicating a big body at
            // every site is the clear waste. `should_emit_once_by_cost` proves the emit is SOUND (the
            // runtime-capturing arg means the result can never be compile-time-demanded, so the
            // mandatory-inline invariant is untouched) and excludes generic/`const` callees (those are
            // specializations the shared path owns). `@inline-never`/`@inline-always` were handled above, so
            // this governs only the default. Routes through the SAME `emit_call_or_specialize` — so a def
            // the heuristic emits-once is byte-identical to one an author marked `@inline-never`.
            if let Some(callee) = callee_def_index(db, head)
                && !db.defs[callee]
                    .body
                    .is_some_and(|b| db.inline_always.contains(&b))
                && should_emit_once_by_cost(db, callee, &args)
            {
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, callee, "apply: cost heuristic → emit_call_or_specialize (emit once)");
                return emit_call_or_specialize(db, head, callee, &args);
            }
            // PARTIAL APPLICATION OF A PROJECTED/STORED LAMBDA (PA1a): `((. t 0) 3)` where `(. t 0)`
            // projects a multi-param lambda out of a compound and is applied through CURRIED syntax to fewer
            // args than its arity — the inner `(. t 0) 3` has a RESULT that is still a function. β-reducing
            // it yields the RESIDUAL inner lambda, whose params then dangle when the enclosing `(… 4)`
            // inlines it ("parameter reference has no local slot" / a materialized-cell stack imbalance).
            // Instead, collapse the WHOLE curried spine to ONE `Core::CallClosure` on the PROJECTED closure:
            // the projected element is materialized (its cell read) + applied at the full gathered arity via
            // `call_indirect`. A DIRECT full application of a projected lambda (`((. r f) 10)` — head IS the
            // projection, one level, result a non-fn value) is NOT matched, so a capturing-single closure
            // keeps FOLDING inline (the `a_capturing_closure_in_a_let_bound_compound_projected_and_applied_folds`
            // guard + 09-functions 316/337/349). Gated on the SYNTACTIC head being a projection (a genuinely
            // STORED fn — `syntactic_head` peels Apply heads WITHOUT folding to a lambda, unlike `apply_spine`).
            if is_curried_application_of_projected_fn(db, head) {
                let closure = syntactic_head(db, id);
                let spine_args = syntactic_spine_args(db, id);
                if !spine_args.is_empty() {
                    trace!(target: "rcdzc::lower", node = id.0, head = closure.0, n_args = spine_args.len(), "apply: curried application of a projected lambda → Core::CallClosure on the projected closure");
                    return Core::CallClosure {
                        closure,
                        args: spine_args.into(),
                    };
                }
            }
            // A LAMBDA head β-reduces (substitute args for params) and the reduced body lowers — this
            // is how a user function call folds/monomorphizes: `((fn (x) (+ x 1)) 5)` reduces to
            // `(+ 5 1)` → `6`, with no function value emitted. The reduction runs UNDER a guard keyed
            // by the lambda's body, so a recursive call (which re-enters the same body while lowering
            // the reduced result) is detected and DECLINES rather than inlining without end.
            if crate::eval::lambda_body(db, head).is_some() {
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: β-reduce lambda head and lower the result");
                // Reduce and lower under a depth guard: a terminating fold bottoms out; a recursive
                // function inlines past the bound and DECLINES rather than diverging.
                match db.enter_reduction() {
                    Some(mut guard) => {
                        let g = guard.db();
                        return match crate::eval::apply_lambda(g, head, &args) {
                            Ok(Some(reduced)) => {
                                // RECORD THE INLINE: this call β-reduced (folded the callee body into the
                                // site, no `Core::Call`, no emitted function). If the head names a top-level
                                // def, mark it inlined so the `Instantiations` query can report the def's
                                // disposition. `callee_def_index` is `None` for an anonymous `fn` / a
                                // let-bound lambda / a computed head (nothing to attribute) — exactly the
                                // cases to skip. An inline leaves no other trace, so it must be recorded here.
                                if let Some(callee) = callee_def_index(g, head) {
                                    g.inlined.insert(callee);
                                }
                                // ANCHOR the reduced body under the call site before lowering it. `apply_lambda`
                                // returns a subtree whose ROOT parent is `None` (a fresh β-reduce copy of the
                                // callee body). Usually harmless — a reduced body that is the substituted arg
                                // itself (`(def (f n) n)` → `k`) is main's own already-resolved occurrence. But
                                // when the callee body is a `handle` (`(def (run s0) (handle St s0 …))`), the
                                // reduced handle is a fresh orphan, and `core_of`'s Handle arm re-parents the
                                // FOLD under that orphan handle node — so a fold result referencing the seed
                                // (`(handle St k … (St.get))` with an identity arm → the seed `k` = the caller's
                                // param) has its chain ascend fold → orphan-handle → None and never reaches the
                                // caller's scope → a spurious CDZ0101 "unbound k". Parenting `reduced` under the
                                // call site `id` (whose own chain reaches the enclosing def) gives the reduced
                                // subtree — and every reparent-under-handle-site anchored within it — a live path
                                // to the caller's binders. (A reduced body that is a bare already-resolved
                                // occurrence is unaffected: re-parenting only sets the root's parent slot.)
                                if g.parent_of(reduced).is_none()
                                    && matches!(
                                        crate::resolve::resolved_of(g, reduced),
                                        Resolved::Handle { .. }
                                    )
                                {
                                    g.reparent(reduced, Some(id), g.child_ix_of(id) as u32);
                                }
                                core_of(g, reduced)
                            }
                            Ok(None) => unreachable!("lambda_body implies a lambda head"),
                            // The reduction declined. If it declined because the callee is RECURSIVE
                            // (can't inline to a normal form), emit a real `Core::Call` to it instead —
                            // provided the callee is a top-level def whose signature is DETERMINED
                            // (`def_scheme` — an annotated recursive def types by absorption, no fixpoint
                            // needed). An unannotated/undetermined callee still declines (its signature
                            // needs the connected solve, a later step). Any other decline propagates.
                            Err(msg) => {
                                // RECURSIVE-CONST-FOLD (P2): a recursive call INTO a callee that declares a
                                // `const` parameter, with ALL arguments compile-time constants, fully UNROLLS
                                // at compile time rather than emitting a runtime call. The `const` param is
                                // the const-DEMAND signal (the author/type-system marked the collection
                                // compile-time), so this fires ONLY on genuine const folds — NOT on ordinary
                                // recursive-generic producers / RRB builders / dictionary consumers (no
                                // `const` param), which still emit a runtime `Core::Call` below. Do ONE
                                // β-level here (bypassing the recursion decline), then `core_of` the reduced
                                // body: it folds the const-scrutinee match + arithmetic, and the residual
                                // self-call re-enters this Apply-fold with its now-const-folded arguments →
                                // the next level → the base arm (a constant). The descent runs under THIS
                                // `enter_reduction` guard, so a NON-shrinking recursion exhausts the
                                // `REDUCE_NODE_BUDGET` → `core_of` yields a `Poison` → we fall through to the
                                // runtime-call/decline path — it can neither hang nor blow up.
                                //
                                // GATE (CHEAP, no `core_of`): a `const` parameter whose declared TYPE is a
                                // shape a TOTAL recursion SHRINKS to a base case — a `(List …)` a
                                // `(list h .. t)` fold recurses over, OR a bare-NAME type: a SCALAR
                                // (`Int64`/`Bool`/`Float64`/`String`/`Bytes`/`Char`/`Symbol`) counted down
                                // (`(dec n)` → `(dec (- n 1))`), or a user SUM (`Ast`) peeled by structural
                                // recursion (`(unwrap form)` → `(unwrap (child form 2))`). This structurally
                                // distinguishes a genuine bounded fold (accept, try to unroll) from a const
                                // RECORD/dictionary a counter-driven recursion passes UNCHANGED — a dict
                                // consumer has a `const` param too, but its type is a `(Record …)`/`(Map …)`/
                                // `(Set …)`/`(Tuple …)` compound FORM that does not shrink, does not fold, and
                                // folding its args to TEST would waste the reduction budget. So the gate
                                // accepts `(List …)` OR any bare NAME and rejects every OTHER form. Reading the
                                // `(: name TYPE)` annotation costs nothing, so the expensive `is_const_value`/
                                // unroll below runs ONLY for a foldable shape. Combined with the fully-folded-
                                // constant check, only a TERMINATING const fold is accepted; everything else
                                // emits the runtime call as before. Broadening beyond `(List …)` (was
                                // const-list-only) is what lets a recursive helper with a const scalar/sum
                                // param — called PER ELEMENT inside a recursive build (`rebuild`-of-`unwrap`) —
                                // fold, closing the NESTED-recursion gap of the general const-fold.
                                let has_const_foldable_param = callee_def_index(g, head)
                                    .is_some_and(|callee| {
                                        let params = g.defs[callee].params.clone();
                                        params.iter().any(|&p| {
                                            g.const_params
                                                .contains(&crate::eval::param_name_occ(g, p))
                                                && g.ast
                                                    .as_form(p, ":")
                                                    .and_then(|t| t.get(1).copied())
                                                    .is_some_and(|ty| {
                                                        // A const param whose declared TYPE is a bare NAME
                                                        // (a scalar / nullary sum) OR a SHRINKING type-
                                                        // constructor application `(<Name> …)` activates the
                                                        // recursive const-fold: a total recursion PEELS a
                                                        // `(List …)` / `(Option …)` / `(Result …)` / user sum
                                                        // toward a base case, and the evaluator now carries
                                                        // those values. EXCLUDE the non-shrinking PRODUCT/
                                                        // DICTIONARY forms — `(Record …)` / `(Tuple …)` /
                                                        // `(Map …)` / `(Set …)` — which a counter-driven
                                                        // recursion passes UNCHANGED (an ad-hoc-polymorphism
                                                        // dictionary consumer): activating the fold there
                                                        // does not terminate on the shape, wastes the budget,
                                                        // and preempts the intended runtime inline+erase.
                                                        // Also EXCLUDE a FUNCTION type `(-> …)`: a `const`
                                                        // arrow param is a higher-order CLOSURE, folded via
                                                        // the closure-arg binding (triggered by a sibling
                                                        // data-typed const param), NOT this data-recursion
                                                        // gate — activating here would fold a DERIVED-closure
                                                        // re-pass that must stay a CDZ0201 reject. (Broadening
                                                        // beyond the former List-or-bare-name is what folds
                                                        // `(Option Int64)` recursion — closing the breaker
                                                        // wasm-hang; it is NOT a fully-open gate.)
                                                        let head_ok = |xs: &[StructId]| {
                                                            let Some(n) = xs
                                                                .first()
                                                                .and_then(|&h| g.ast.as_name(h))
                                                            else {
                                                                return false;
                                                            };
                                                            match n {
                                                                // A pure-DATA `(Record …)` const param is
                                                                // admitted: it can SHRINK (a counter rebuilt
                                                                // each step) and the interpreter folds
                                                                // records. A record with a FUNCTION field
                                                                // `(-> …)` is EXCLUDED — that is the ad-hoc-
                                                                // polymorphism DICTIONARY a recursive consumer
                                                                // passes UNCHANGED and inlines+erases at
                                                                // runtime; folding it would preempt that
                                                                // erasure. Distinguish CHEAPLY by the field
                                                                // TYPES: a dict has an arrow-typed field, a
                                                                // data record does not. A field is written
                                                                // `(name type)` OR `(: name type)` — the TYPE
                                                                // is the LAST child in BOTH spellings, so read
                                                                // `last()` (reading index 1 picks the NAME of
                                                                // the colon form and misses its arrow — the
                                                                // dict-consumer shape uses `(: op (-> …))`).
                                                                "Record" => xs[1..].iter().all(|&field| {
                                                                    match g.ast.get(field) {
                                                                        crate::ast::Struct::List(f) => f
                                                                            .last()
                                                                            .map(|&ty| {
                                                                                g.ast.head_name(ty)
                                                                                    != Some("->")
                                                                            })
                                                                            .unwrap_or(true),
                                                                        _ => true,
                                                                    }
                                                                }),
                                                                // `(Map …)`/`(Set …)` const params stay
                                                                // excluded (CHAMP-iteration-order-sensitive
                                                                // collections). A bare `(-> …)` is a higher-
                                                                // order closure param, folded via the
                                                                // closure-arg binding, not this data gate.
                                                                "Map" | "Set" | "->" => false,
                                                                _ => true,
                                                            }
                                                        };
                                                        g.ast.as_name(ty).is_some()
                                                            || matches!(g.ast.get(ty),
                                                                crate::ast::Struct::List(xs)
                                                                if head_ok(xs))
                                                    })
                                        })
                                    });
                                // GENERAL CONST-EVALUATION (P2, DESIGN-general-const-eval.md): interpret the
                                // total function applied to compile-time-constant arguments to a constant
                                // VALUE. Tried BEFORE the unroll-and-refold because it COMPOSES natively (a
                                // recursion consuming another recursion's const result; a let-bound nested-
                                // recursion result carried through a filter) where the refold cannot. Same
                                // activation gate (const-param demand + all-args-const) and a step budget;
                                // falls through to the unroll on any value it cannot yet evaluate (Stage a:
                                // scalars + lists), so it is a COMPLETENESS gain on the accept path, never a
                                // miscompile. A wrong value would be caught by the corpus gate; incompleteness
                                // is safe by construction (the unroll / runtime-call path still runs).
                                // GENERAL CONST-EVALUATION (P2): a recursive call INTO a callee that declares a
                                // `const` parameter, with ALL arguments compile-time constants, is evaluated to
                                // a constant VALUE by the general const-evaluator (`const_eval`) and folded in
                                // place — the reflected-source, nested-recursion, and composition cases the
                                // earlier unroll-and-refold could not do (a value-domain interpreter composes
                                // natively; `DESIGN-general-const-eval.md`). The `const` param is the
                                // const-DEMAND signal (author/type-system marked the collection compile-time),
                                // so this fires ONLY on genuine const folds — NOT on ordinary recursive-generic
                                // producers / RRB builders / dictionary consumers (no `const` param), which
                                // still emit a runtime `Core::Call` below. `const_eval` is budget-bounded (a
                                // non-terminating fold declines, never hangs) and, on any value it cannot
                                // evaluate, returns `None` → the runtime-call / decline path runs — so this is a
                                // COMPLETENESS gain on the accept path, never a miscompile.
                                // An argument qualifies if it is a const VALUE or a LAMBDA LITERAL — a
                                // lambda is compile-time-known (a closure over consts) and const_eval binds
                                // it as a `CVal::Closure` so a HIGHER-ORDER callee (`List.map`-style: a
                                // `const f` parameter applied per element) const-folds. A lambda is not a
                                // `is_const_value` (its core is a function, not a constant), so it must be
                                // admitted explicitly here or the higher-order fold never activates.
                                if has_const_foldable_param
                                    && args.iter().all(|&a| {
                                        is_const_value(g, a)
                                            || crate::eval::lambda_params_of(g, a).is_some()
                                    })
                                {
                                    let mut budget: u64 = 1_000_000;
                                    if let Some(cv) = const_eval_apply(
                                        g,
                                        id,
                                        head,
                                        &args,
                                        &CEnv::default(),
                                        &mut budget,
                                    ) && let Some(core) = cval_to_core(g, &cv)
                                        && (core_is_const_value(g, &core)
                                            || matches!(cv, CVal::Trap(_)))
                                    {
                                        // A fully-folded constant, OR a taken const-fold trap (its
                                        // `Core::Poison(ConstTrap, msg)` surfaces the trap message as the
                                        // compile error — a const-executed trap is fail-loud, not a decline).
                                        return core;
                                    }
                                }
                                lower_recursive_call_or_decline(g, head, &args, msg)
                            }
                        };
                    }
                    None => {
                        // The REDUCTION-depth limit was hit — not (necessarily) a recursive callee, just
                        // a call chain nested deeper than the inliner reduces (`REDUCE_DEPTH_LIMIT`). A
                        // finite deep chain is a resource-limit DECLINE, not a miscompile; name it
                        // accurately (the old "recursive function" wording misdescribed a plain deep
                        // nest, which since inlining became linear is now reachable on a well-formed
                        // program). This does NOT route through `lower_recursive_call_or_decline` (that is
                        // only for an `is_recursive`-origin decline), so the wording is free to be exact.
                        // A resource-limit rejection — the "declined at a bound, not crashed" class, coded
                        // CDZ0999 like the unproductive-recursion decline. Reached either by a call chain
                        // nested past `REDUCE_DEPTH_LIMIT`, or by the TOTAL-work budget (`REDUCE_NODE_BUDGET`)
                        // that `enter_reduction` enforces to stop an explosively-growing (non-normalizing)
                        // term — a self-applying lambda whose reduction would otherwise hang the compiler.
                        trace!(target: "rcdzc::lower", node = id.0, "apply: reduction limit hit → decline (resource limit, CDZ0999)");
                        return Core::Poison(Reject::coded(
                            Code::RecursionBound,
                            "an expression does not reduce to a value within the compiler's reduction limits (a call chain nested too deeply, or a non-terminating / explosively-growing reduction)",
                        ));
                    }
                }
            }
            // A ZERO-ARGUMENT application `(g)` whose head is not a lambda. Applying a value to no
            // arguments is the identity — the application IS the head value. This is how a NULLARY def
            // is called: `(def (g) 7)` resolves `g` to its body value (so a bare `g` is 7), and `(g)`
            // is that same value. (A nullary LAMBDA `((fn () 7))` took the β-reduce branch above, so it
            // is already handled; only a non-lambda head reaches here.) Without this, `(g)` fell through
            // to `meta_apply_of` — which, finding no `(meta apply)` on the scalar 7, rejected it as
            // "value is not applyable", breaking every nullary-function call.
            // An EMPTY compound-VALUE constructor — `(list)` / `(tuple)` / `(record)` / `(map)` written
            // with the alias name at zero args — BUILDS the empty compound, it is NOT the ctor value.
            // Route it through `reduce_ctor` (which rewrites `(map)` → `("map")` → the symbol form) before
            // the zero-arg identity short-circuit below (which would return the ctor record and then
            // decline it as a bare built-in value). A NON-empty alias application reaches `reduce_ctor` via
            // the `Some(prim)` arm; this is only the nullary case the short-circuit would otherwise capture.
            if args.is_empty()
                && matches!(
                    crate::eval::meta_apply_of(db, head),
                    Some(Prim::TupleNew | Prim::RecordNew | Prim::ListNew | Prim::MapNew)
                )
            {
                let prim = crate::eval::meta_apply_of(db, head).unwrap();
                return match crate::eval::reduce_ctor(db, prim, id, &args) {
                    Ok(built) => core_of(db, built),
                    Err(msg) => Core::Poison(Reject::decline(msg)),
                };
            }
            if args.is_empty() {
                // A BINARY OPERATOR applied to ZERO operands — `(=)` / `(+)` — is a malformed application,
                // NOT the operator used as a value: the operator DEMANDS its operands (`(+ 1)` already
                // rejects "+ takes exactly 2 operands"; `(+)` is the same arity error at zero). Reject
                // CDZ0201 rather than fall through to `core_of(head)`, which would decline the bare
                // operator value as "needs runtime closures" (a to-do, not the well-formedness error it
                // is — 07-type-system "a bare equality/arithmetic keyword is rejected, not a crash").
                if let Some(prim) = crate::eval::meta_apply_of(db, head)
                    && (prim.is_binop() || matches!(prim, Prim::Compare))
                {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: binary operator with no operands (CDZ0201)");
                    return Core::Poison(Reject::coded(
                        Code::Malformed,
                        format!("{} takes exactly 2 operands", intrinsic_name(prim)),
                    ));
                }
                // A UNARY (payload-carrying) VARIANT CONSTRUCTOR applied to ZERO arguments — `(Some)` —
                // is UNDER-application, the low-arity mirror of the over-application `(Some 1 2)`: a sum
                // constructor produces its value only when applied to its payload argument
                // (core-semantics.md §A Sum Type Constructor Is A Single-Arity Function). Reject CDZ0201
                // rather than fall through to `core_of(head)`, which would decline the bare partial
                // application ("needs closures") — a to-do, not the well-formedness error `(Some)` is
                // (09-functions "under-applying a unary constructor is a type error, not a fabricated unit
                // payload"). A NULLARY variant `(None)` has NO payload type, so it is NOT under-applied —
                // it constructs its value here (falls through to `core_of(head)`), preserving the valid
                // bare-nullary-construction path.
                if crate::eval::variant_disc_of(db, head).is_some()
                    && let Some(payload) = crate::eval::variant_payload_type(db, head)
                {
                    trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: unary variant ctor under-applied (CDZ0201)");
                    // NAME the constructor + its payload type (`` `Wrap` needs its payload argument — it
                    // carries an Int64 ``) instead of the anonymous "a variant constructor with a payload
                    // …", so the reader sees WHICH ctor and WHAT to supply — the under-application twin of
                    // the "`Mk` takes N arguments, but M were given" over-application message. The name
                    // reads off the head: a bare `Wrap` / a member `(. Sum Wrap)` → `Wrap`. When the payload
                    // type is UNRESOLVED (a generic ctor whose parameter no use has fixed — `(Some)` :
                    // `∀a. a`, rendering as `_`), the "it carries `_`" clause reads as noise, so OMIT it and
                    // just say the payload is needed.
                    let carries = if payload.has_free_var() || matches!(payload, crate::ty::Ty::Any)
                    {
                        String::new()
                    } else {
                        format!(
                            " — it carries {}",
                            payload.render_with_article(&db.name_ctx())
                        )
                    };
                    let msg = match ctor_head_display_name(db, head) {
                        Some(name) => format!(
                            "`{name}` needs its payload argument{carries}; apply it, e.g. `({name} <value>)`"
                        ),
                        None => {
                            format!("this variant constructor needs its payload argument{carries}")
                        }
                    };
                    return Core::Poison(Reject::coded(Code::Malformed, msg).at(head));
                }
                // A RECURSIVE nullary call (`(def (f) (f))`) cannot fold to a normal form — following
                // the head would re-enter the same body without end. Decline it exactly as a recursive
                // parameterized call declines (a nullary def has no runtime-function form yet, so there
                // is no `Core::Call` to emit — it declines rather than diverging). `is_recursive` reads
                // the callee body reached through the nullary def's `Ref` (see `eval::callee_body`).
                if let Some(body) = crate::eval::lambda_body_of_nullary(db, head)
                    && crate::eval::is_recursive(db, body)
                {
                    // BEFORE declining: a nullary def is `is_recursive` if its body merely CALLS a recursive
                    // helper — even when the def itself is not self-recursive and const-FOLDS. A self-reflected
                    // descriptor accessor `def id() = contract(Ast.module).id` (where `contract` recurses over
                    // the module forms) is exactly this: not unproductive, it evaluates to a constant. Try the
                    // general const-evaluator on the body; a genuinely unproductive nullary self-recursion
                    // (`(def (f) (f))`) yields no constant and falls through to the CDZ0999 decline.
                    let mut budget: u64 = 1_000_000;
                    if let Some(cv) = const_eval(db, body, &CEnv::default(), &mut budget)
                        && let Some(core) = cval_to_core(db, &cv)
                        && (core_is_const_value(db, &core) || matches!(cv, CVal::Trap(_)))
                    {
                        return core;
                    }
                    // A NULLARY self-recursion has no parameter to vary, so it can never reduce to a value
                    // (following it re-enters the same body without end) AND has no runtime-function form to
                    // specialize — a genuinely UNPRODUCTIVE recursion, not a not-yet-built gap. This is the
                    // robustness case (`self-hosting-and-bootstrap.md` §An Unsupported Construct Is Declined,
                    // Not Miscompiled): the compiler stops at the recursion bound and declines with the
                    // reserved CDZ0999 code — "declined, not crashed" — rather than aborting on a native
                    // stack overflow. A PARAMETERIZED recursive call is DIFFERENT (it runtime-specializes,
                    // or declines codeless if that isn't built yet — a plain Todo); only the unproductive
                    // nullary shape is coded here.
                    trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: unproductive nullary recursion → CDZ0999");
                    return Core::Poison(Reject::coded(
                        Code::RecursionBound,
                        "an unproductive self-recursion cannot be reduced to a value (declined at the recursion bound)",
                    ));
                }
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: zero-argument application is its head value");
                return core_of(db, head);
            }
            // The applied operation: a value record's `(meta apply)` prim (the usual `List.len`/`+`/…
            // path), OR — for a COMPILER-INTERNAL intrinsic emitted directly as a prim head (no module
            // record), e.g. `(intrinsic "ast-splice-lift")` from the quasiquote-splice desugar — the
            // head's own directly-resolved prim. (A bare prim used as a VALUE, not applied, still declines
            // at the non-Apply `Resolved::Prim` arm; here it is genuinely applied to `args`.)
            match crate::eval::meta_apply_of(db, head).or_else(|| crate::eval::prim_of(db, head)) {
                // UNARY NEGATION `(- e)` — the arity-1 subtraction (the ML prefix `-<expr>`). Negation is
                // `0 - e` at the OPERAND's numeric type: synthesize a typed zero and delegate to the SAME
                // binary-subtraction machinery, so every numeric type is covered with no new op — an
                // `Int N` gets the checked `x == MIN` overflow trap (the exact behaviour the `x * -1 →
                // (- 0 x)` strength reduction already relies on), a `Float`/`Rational`/`BigInt`/`Qty` its
                // own arithmetic. `infer` already typed `(- e)` as e's type and rejected a non-numeric
                // operand; here it can only be numeric (or an `Any` that faulted elsewhere → decline).
                Some(Prim::Sub) if args.len() == 1 => {
                    trace!(target: "rcdzc::lower", node = id.0, "apply: unary negation (- e) → 0 - e at the operand's type");
                    lower_negate(db, id, args[0])
                }
                // A PARTIALLY-applied binary OPERATOR — `(+ 1)`, `(< 3)`, `(* 2)` — CURRIES to a first-class
                // function of the remaining operand (operator ruling: "operators should curry"). Synthesize
                // the equivalent lambda `(fn (b) (op supplied b))` and lower it as a value — the same shape
                // the user could write by hand. `(- e)` is UNARY NEGATION (the arm above), not a curried
                // subtraction, so Sub with one operand never reaches here. A non-numeric/undetermined
                // operand makes `partial_binop_eta` return `None` → falls through to the arity fault below,
                // so a genuine malformed `(+ x)` (x unfixed) still reports rather than synthesizing a
                // broken closure.
                Some(prim)
                    if args.len() == 1
                        && prim != Prim::Sub
                        && (prim.is_binop() || prim.is_float_arith())
                        && let Some(c) = partial_binop_eta(db, head, args[0]) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: partial binary operator → curried closure (fn (b) (op supplied b))");
                    c
                }
                // MIXED-UNIT COMBINE: `+`/`-`/comparison on two quantities of the SAME dimension but
                // DIFFERENT scale (`1 km + 500 m`, `1 KiB + 1 kB`). Each operand converts to the
                // dimension's REFERENCE unit by its exact scale (`value * num / den` in the inner type T),
                // then the plain op runs there (units-of-measure.md §Combining Units Of One Dimension Is
                // Well-Formed / §A Unit Conversion Is The Arithmetic The Source Denotes). Handles the
                // CONSTANT case by folding the conversion (the demonstrable slice); a runtime mixed-unit
                // operand declines (the emitted scale-multiply on a runtime value is a later increment).
                Some(
                    prim @ (Prim::Add
                    | Prim::Sub
                    | Prim::Lt
                    | Prim::Gt
                    | Prim::Le
                    | Prim::Ge
                    | Prim::Eq),
                ) if args.len() == 2 && quantity_scales_differ(db, &args) => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: mixed-unit combine (convert to reference)");
                    lower_quantity_combine(db, id, prim, args[0], args[1])
                }
                // SAME-UNIT quantity COMPARISON: two quantities of the SAME unit (same dimension AND scale)
                // compared with `< > <= >= =`. The units are identical, so no conversion is needed — the
                // comparison is exactly the erased inner magnitudes' comparison. A CONSTANT pair already
                // folds below (both reach as `Core::ConstInt`/etc.), and same-unit ARITHMETIC `+`/`-` already
                // runs via the inner-type arith path — but a RUNTIME comparison (a `q` bound from a
                // `Map.lookup`/`List.at` Option arm, or a parameter) fell to the generic `is_scalar` gate,
                // which does NOT peel `Ty::Qty`, so `(< q …)` declined "comparison of a compound value needs
                // a heap walk". Rewrite it as `(op (Qty.value a) (Qty.value b))` — the explicit unwrap that
                // erases both units to their bare inner numerics — and re-lower, so it takes the ordinary
                // scalar/float/bigint/rational comparison path over the inners. GUARDED on `same-unit`: a
                // DIFFERENT-scale pair is handled by the `quantity_scales_differ` arm ABOVE (which CONVERTS
                // first), so this never compares raw across scales (verified: a `km` vs `m` param routes to
                // conversion, not here — the scale now survives the type round-trip after the encode_ty fix).
                // Same-DIMENSION is required — a cross-dimension pair is CDZ0501 in `check_application`.
                Some(prim @ (Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq))
                    if args.len() == 2 && quantity_same_unit_pair(db, &args) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: same-unit quantity comparison → compare erased magnitudes");
                    let lv = qty_magnitude_occ(db, args[0]);
                    let rv = qty_magnitude_occ(db, args[1]);
                    let head = db.push_name(intrinsic_name(prim));
                    let app = db.push_list(vec![head, lv, rv]);
                    core_of(db, app)
                }
                // A quantity over a FLOAT magnitude combined with `+`/`-`/`*`/`/` runs the INNER numeric
                // type's operation — the plain `T` op (units-of-measure.md §A Unit Conversion Is The
                // Arithmetic The Source Denotes: the running arithmetic is the plain `T` operation on erased
                // values). For a Float-inner quantity that is FLOAT arithmetic, so map the integer arith
                // prim to its float counterpart and route to `lower_float_arith` (the operands erase to their
                // inner floats, so the fold/emit is over `Core::ConstFloat`). A quantity's `+`/`*` is thus
                // polymorphic over the inner numeric, unlike the bare int-only `+` (which rejects a float).
                Some(prim @ (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div))
                    if quantity_inner_is_float(db, id, &args) =>
                {
                    let fprim = match prim {
                        Prim::Add => Prim::FAdd,
                        Prim::Sub => Prim::FSub,
                        Prim::Mul => Prim::FMul,
                        _ => Prim::FDiv,
                    };
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: quantity float arithmetic (inner Float)");
                    lower_float_arith(db, id, fprim, &args)
                }
                // A quantity over a RATIONAL magnitude combined with `+`/`-`/`*`/`/` runs EXACT RATIONAL
                // arithmetic on the erased inner magnitudes (the `Qty.of` operands lower to their inner
                // value's core — a `Core::ConstRational`), the rational analogue of the Float-inner arm.
                // The dimensional/unit result is recovered from the SOLVED `Ty::Qty` by the value renderer
                // (`const_value_ast`'s Qty arm), so this only computes the magnitude. `*`/`/` compose the
                // unit; `+`/`-` require the same unit (dimensional check in `infer`).
                Some(prim @ (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div))
                    if quantity_inner_is_rational(db, id, &args) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: quantity rational arithmetic (inner Rational)");
                    lower_rational_arith(db, prim, args[0], args[1])
                }
                // A quantity over a BIGINT magnitude combined with `+`/`-`/`*`/`/` runs the UNBOUNDED
                // bigint arithmetic on the erased inner handles — the bigint analogue of the Float/Rational
                // inner arms. A BigInt inner is a heap HANDLE (i32), so it MUST route to `lower_bigint_arith`
                // (the runtime `bigint-*` ops / constant fold); the default integer path would treat the
                // handle as an i64 fixnum → an i32/i64 miscompile (invalid wasm). Checked before the
                // `bigint_operand` arm below because that reads the operand's type as `Ty::BigInt`, which a
                // `(Qty BigInt u)` is NOT (its type is `Ty::Qty { inner: BigInt }`).
                Some(prim @ (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div))
                    if quantity_inner_is_bigint(db, id, &args) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: quantity bigint arithmetic (inner BigInt)");
                    lower_bigint_arith(db, prim, args[0], args[1])
                }
                // A `+`/`-`/`*`/`/` over BIGINT operands — the unbounded arithmetic. A constant pair folds
                // exactly via `num-bigint` (the value never overflows — the point of the type); a runtime
                // operand emits the runtime `bigint-add`/`-sub`/`-mul`/`-div` (B3b). Checked before the
                // generic int-arith path (which would range-check/trap against a fixed width — wrong for an
                // unbounded BigInt). Dispatch on the OPERAND type being `Ty::BigInt`, like the float arm.
                Some(prim @ (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Rem))
                    if args.len() == 2 && bigint_operand(db, &args) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: BigInt arithmetic");
                    lower_bigint_arith(db, prim, args[0], args[1])
                }
                // A `+`/`-`/`*`/`/` over RATIONAL operands — exact rational arithmetic. A constant pair
                // folds to a NORMALIZED `Core::ConstRational` (cross-multiply + gcd-reduce over `IntValue`
                // bignum); a runtime operand declines until the runtime rational compound. Checked before
                // the generic int-arith path (which would treat the operands as fixed-width ints). `%` is
                // NOT a rational op (exact division is total — no remainder), so it is excluded here and
                // falls through to the scheme, which rejects it.
                Some(prim @ (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div))
                    if args.len() == 2 && rational_operand(db, &args) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: Rational arithmetic");
                    lower_rational_arith(db, prim, args[0], args[1])
                }
                // A `+`/`-`/`*`/`/` over FLOAT operands — floating-point arithmetic written with the ONE
                // arithmetic operator (there is no distinct `+.`). Remap the integer prim to its float
                // counterpart and route to `lower_float_arith` (fold two constant floats at the solved
                // width, else emit the machine `f64.add`…), exactly as the Qty-inner-float arm does.
                // Checked before the generic int-arith path (which would range-check against a fixed
                // width — wrong for a float). Dispatch on the OPERAND type being `Ty::Float`, like the
                // BigInt/Rational arms; a float/int mix never reaches here (rejected CDZ0301 in
                // `check_application`). `%`/bit-ops/shift are integer-only and fall through to `is_arith`.
                Some(prim @ (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div))
                    if args.len() == 2 && float_operand(db, &args) =>
                {
                    let fprim = match prim {
                        Prim::Add => Prim::FAdd,
                        Prim::Sub => Prim::FSub,
                        Prim::Mul => Prim::FMul,
                        _ => Prim::FDiv,
                    };
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: Float arithmetic (operand is Float)");
                    lower_float_arith(db, id, fprim, &args)
                }
                Some(prim) if prim.is_arith() => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: arithmetic prim");
                    lower_arith(db, id, prim, &args)
                }
                // A FLOAT arithmetic prim (`+.`/`-.`/`*.`/`/.`) — fold two constant floats, else decline
                // (runtime float operands emit the machine op in F4).
                Some(prim) if prim.is_float_arith() => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: float arithmetic prim");
                    lower_float_arith(db, id, prim, &args)
                }
                // `Float64.of-int` / `Float32.of-int` — the explicit INT→FLOAT conversion. Fold a
                // constant integer to a `Core::ConstFloat` at the target width, else emit a runtime
                // `f{64,32}.convert_i64_s`.
                Some(Prim::FloatOfInt) => lower_float_of_int(db, id, &args),
                // `Float64.of` / `Float32.of` — the explicit FLOAT-WIDTH conversion. Fold a constant
                // float (round at the target width), else emit a runtime demote/promote.
                Some(Prim::FloatOf) => lower_float_of(db, id, &args),
                // `compare` — the three-way comparison, yielding an `Ordering` sum (Less/Equal/Greater).
                // FOLD a constant scalar/string pair to the matching variant; a compound/runtime operand
                // declines (as the comparison prims do).
                Some(Prim::Compare) if args.len() == 2 => lower_compare(db, id, args[0], args[1]),
                Some(prim) if prim.is_comparison() => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: comparison prim");
                    lower_comparison(db, prim, &args)
                }
                Some(prim) if prim.is_conversion() => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: conversion prim");
                    lower_conversion(db, id, prim, &args)
                }
                // `Qty.of x u` — attach a compile-time unit. The unit is CHECKED THEN ERASED
                // (units-of-measure.md §Dimensions Are Checked Then Erased), so lowering is the value
                // argument's lowering UNCHANGED — `(Qty.of 5.0 meter)` and the bare `5.0` produce the
                // identical core (byte-identical emitted value). The unit lives only in the solved type.
                Some(Prim::QtyOf) if args.len() == 2 => {
                    trace!(target: "rcdzc::lower", node = id.0, "apply: Qty.of erases to its value argument");
                    core_of(db, args[0])
                }
                // `Qty.value q` — recover the numeric value, discarding the unit. Since a quantity ALREADY
                // erases to its inner value, this is likewise the argument's lowering unchanged (the
                // explicit exit from the dimensional layer is a no-op at runtime).
                Some(Prim::QtyValue) if args.len() == 1 => {
                    trace!(target: "rcdzc::lower", node = id.0, "apply: Qty.value erases to its argument");
                    core_of(db, args[0])
                }
                // `Qty.pow q n` — raise the erased magnitude to the `n`th power (the unit is a
                // compile-time concern handled by the solved type). Erases to `value * value * … ` (`n`
                // factors) over the inner numeric type; `n = 0` is the dimensionless literal `1`. A
                // negative exponent declines (needs a reciprocal). Folds when the magnitude is constant.
                Some(Prim::QtyPow) if args.len() == 2 => {
                    trace!(target: "rcdzc::lower", node = id.0, "apply: Qty.pow repeated multiply");
                    lower_qty_pow(db, args[0], args[1])
                }
                // `Type.eq a b` — compile-time type equality FOLDS to a constant `Bool`. Reduce each
                // argument to its `Ty` (a type-value — a `(Type.of e)` result or a written type) and
                // compare with `Ty`'s exact structural `==`. A constant result means `(if (Type.eq …) …)`
                // selects its branch at compile time. A non-type argument declines (an ill-formed
                // operand). A compile-time COMPARISON producing a runtime `Bool`; no `Type` value survives.
                Some(Prim::TypeEq) if args.len() == 2 => {
                    trace!(target: "rcdzc::lower", node = id.0, "apply: Type.eq compile-time type equality");
                    match (
                        crate::eval::typeval_of(db, args[0]),
                        crate::eval::typeval_of(db, args[1]),
                    ) {
                        (Some(a), Some(b)) => Core::ConstBool(a == b),
                        _ => Core::Poison(Reject::decline(
                            "Type.eq requires two type-values (each a Type.of result or a type)",
                        )),
                    }
                }
                // `Unit.in target q` — EXPLICIT conversion. Convert q's erased magnitude from its unit to
                // the TARGET by `value * (q.scale / target.scale)` in the inner type T (a no-op when the
                // units are already equal). Folds the constant case; a runtime operand declines.
                Some(Prim::UnitIn) if args.len() == 2 => {
                    trace!(target: "rcdzc::lower", node = id.0, "apply: Unit.in explicit conversion");
                    lower_unit_in(db, args[0], args[1])
                }
                // A sum VARIANT CONSTRUCTOR applied — `(Option.Some 5)`. The discriminant is read off
                // the head's `(meta variant)` channel (the value the shared `sum-new` prim needs); the
                // args are the payloads. Build `Core::SumNew{disc, payloads}` the backend lowers to
                // `sum-new(disc, payload)`.
                Some(Prim::SumNew) => {
                    trace!(target: "rcdzc::lower", node = id.0, "apply: sum variant constructor");
                    lower_sum_new(db, id, head, &args)
                }
                // `List.len` applied to a list — FOLD when the operand is a compile-time-visible list
                // literal (its length is statically known), else emit `Core::ListLen` (the runtime
                // `Record.project r (a c)` — narrow a record to the named fields. FOLD over a
                // compile-time-visible `Core::Record`: build a NEW `Core::Record` holding only the named
                // fields, each carrying the operand's own value occurrence (the value heap is immutable,
                // so the result shares the operand's field values — `type-system.md` §A Record Row
                // Operation Yields A New Value). The second operand is a LITERAL field-name list `(a c)`
                // (labels via `record_op_labels`, NOT an evaluated value). A named field absent from the
                // record is the CDZ0212 `infer` reports; here the fold simply omits it (the reject denies
                // the build, so this core is never emitted). A poison operand / non-record / non-constant
                // record declines (the runtime row op is a later increment).
                Some(Prim::RecordProject) if args.len() == 2 => {
                    lower_record_project(db, id, args[0], args[1], false)
                }
                Some(Prim::RecordWithout) if args.len() == 2 => {
                    lower_record_project(db, id, args[0], args[1], true)
                }
                Some(Prim::RecordMerge) if args.len() == 2 => {
                    lower_record_merge(db, id, args[0], args[1])
                }
                // `Record.extend r #z v` / `Record.with r #z v` (3-operand, DESIGN-record-update-syntax.md)
                // — both INSERT field `z ↦ v` into a constant `Core::Record` (extend adds an absent field,
                // with replaces a present one; the presence/absence CDZ0211/0212 is `infer`'s, so the fold
                // is the same insert). The field LABEL is the `#symbol` 2nd operand (`read_label`); the
                // value `v` is the 3rd operand (its value occurrence carries into the field).
                Some(Prim::RecordExtend | Prim::RecordWith) if args.len() == 3 => {
                    lower_record_insert(db, id, args[0], args[1], args[2])
                }
                // `Record.pop r z` — `(tuple (. r z) (r without z))`: the popped field's value paired with
                // the remaining record. Folds a constant `Core::Record` to a `Core::Tuple`.
                Some(Prim::RecordPop) if args.len() == 2 => {
                    lower_record_pop(db, id, args[0], args[1])
                }
                // `Tuple.concat a b` — concatenate two constant `Core::Tuple`s (elements of `a` then `b`).
                Some(Prim::TupleCat) if args.len() == 2 => {
                    lower_tuple_cat(db, id, args[0], args[1])
                }
                // `Tuple.split-at t k` — `(tuple <prefix> <suffix>)` at compile-time literal `k`.
                Some(Prim::TupleSplitAt) if args.len() == 2 => {
                    lower_tuple_split_at(db, id, args[0], args[1])
                }
                // `Tuple.remove t` — `(tuple (. t 0) <rest>)`.
                Some(Prim::TuplePop) if args.len() == 1 => lower_tuple_pop(db, id, args[0]),
                // `vec-len`). One operand: the list.
                Some(Prim::ListLen) if args.len() == 1 => {
                    let operand = args[0];
                    if let Core::Poison(r) = core_of(db, operand) {
                        Core::Poison(r)
                    } else if let Some(elems) = const_list_elems(db, operand)
                        && elems.iter().all(|&e| is_trap_free(db, e))
                    {
                        // A compile-time-visible constant list, inline (`(list 1 2 3)`) or `let`-bound (a
                        // kept multi-use binding lowers the reference to a `LocalRef`, which the helper
                        // follows to the bound literal). `List.len` = the spine ARITY, fixed at construction,
                        // so the len folds to the element count regardless of how the value is bound (sound —
                        // only `Set.len`/`Map.size` are unsafe, since dedup can shrink them). Folds only the
                        // LEN read; the binding is not erased (a list used elsewhere is still built).
                        //
                        // TRAP-PRESERVATION (same `is_trap_free` discipline as the `x * 0`/`x & 0`
                        // annihilators): folding to the constant arity DISCARDS the element VALUES — computing
                        // the length from the spine structure without evaluating the constructions. So it may
                        // fire ONLY when every element construction is provably TRAP-FREE. A trapping element
                        // (e.g. `(Rational.of 3 d)` with a RUNTIME denominator `d` that may be 0 → a
                        // zero-denominator trap) is NOT trap-free, so the fold declines and the runtime
                        // `Core::ListLen` is emitted instead — which evaluates the list construction, so the
                        // trapping element still traps (the fold must drop the element's VALUE, not its
                        // EVALUATION). Without this guard `List.len (list _ (Rational.of 3 d))` at d=0 ran to
                        // the length instead of trapping (breaker/corpus-bugfix).
                        trace!(target: "rcdzc::fold", node = id.0, len = elems.len(), "List.len folds to a constant list arity");
                        Core::ConstInt(IntValue::from_i64(elems.len() as i64))
                    } else {
                        Core::ListLen { operand }
                    }
                }
                // `List.push` / `List.concat` — runtime `vec-push`/`vec-concat`. A poison operand
                // propagates; otherwise emit the runtime op (no constant fold — a persistent push/concat
                // builds a new heap value, not worth folding a constant spine here).
                Some(Prim::ListPush) if args.len() == 2 => {
                    match (core_of(db, args[0]), core_of(db, args[1])) {
                        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
                        // FOLD a compile-time-visible list literal + an element into ONE `Core::ListNew`
                        // with the element APPENDED — a constant list (bakes at escape / folds through
                        // `List.at`/`len`), exactly as a written `(list …)`. The pushed element's own
                        // occurrence (`args[1]`) carries over regardless of whether IT is constant.
                        (Core::ListNew { elems: a }, _) => {
                            let mut a = a.to_vec();
                            a.push(args[1]);
                            trace!(target: "rcdzc::fold", node = id.0, len = a.len(), "List.push folds onto a constant list");
                            Core::ListNew { elems: a.into() }
                        }
                        // A runtime list — the persistent `vec-push` on the heap.
                        _ => Core::ListPush {
                            list: args[0],
                            elem: args[1],
                        },
                    }
                }
                // `List.prepend` — insert an element at the FRONT (receiver-first: args = [list, elem]).
                // Lowers to the dedicated `Core::ListPrepend` (runtime `vec-prepend` op, the front-growth twin
                // of `vec-push`), REPLACING the old `concat(list-new(elem), list)` path — which invoked the full
                // RRB merge per prepend and leaked the superseded front-spine (~17 cells/prepend). A poison
                // operand propagates. A compile-time-visible list literal FOLDS: the element is INSERTED AT THE
                // FRONT of the constant `Core::ListNew` (mirroring the `ListConcat` fold of `[elem] ++ xs`), so a
                // constant prepend bakes / folds through `List.at`/`len` exactly as a written `(list …)`.
                // `List.prepend` — the body is in an `#[inline(never)]` helper so its locals (a `Vec`, the
                // `synth_core` scratch) stay OFF `core_of`'s per-frame stack: `core_of` is the recursive
                // lowering hub every nested node walks through, and a `DESCENT_DEPTH_LIMIT`-deep input (the
                // `a_sum_payload_wrapped_self_application` decline test) overflows the spawned compile thread's
                // stack if the hub's frame grows (the printer-arm-locals-bloat family).
                Some(Prim::ListPrepend) if args.len() == 2 => {
                    lower_list_prepend(db, id, args[0], args[1])
                }
                // `ast-splice-lift` — the quasiquote-splice lift `(List a) → (List Ast)`: wrap each element
                // in the `Ast` leaf its constant kind denotes (Int64→`Ast.Int`, Float64→`Ast.Float`,
                // Bool→`Ast.Bool`, String→`Ast.Str`). FOLD a constant list literal into a `Core::ListNew`
                // of leaf `Core::SumNew` nodes (discs read by name). A runtime list operand (no visible
                // `ListNew`) declines — the runtime map is a later increment. A non-scalar element (a
                // nested list, a char, a NaN float) declines (a wrong lift would corrupt the value).
                Some(Prim::AstSpliceLift) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::Poison(r) => Core::Poison(r),
                    Core::ListNew { elems } => match lower_ast_splice_lift(db, id, &elems) {
                        Some(core) => core,
                        None => Core::Poison(Reject::unsupported(
                            "an active `,@` splice needs a compile-time-constant list of scalar \
                                 (Int64/Float64/Bool/String) or Ast elements (the runtime splice map \
                                 is not supported)",
                        )),
                    },
                    _ => Core::Poison(Reject::unsupported(
                        "an active `,@` splice needs a compile-time-constant list (the runtime \
                         splice map is not supported)",
                    )),
                },
                // `ast-lift` — the runtime active-unquote lift (`∀a. a → Ast`): wrap the operand's value in
                // the `Ast` leaf its INFERRED type denotes. IDENTITY when the operand is already `Ast` (the
                // primary case — `(quasiquote (+ (unquote sub) 1))` with `sub : Ast`), else wrap in
                // `Ast.Int`/`Ast.Bool`/`Ast.Str` by the operand's scalar type. Works at RUNTIME (unlike the
                // splice fold) — that is the whole point. A type with no `Ast` leaf declines.
                Some(Prim::AstLift) if args.len() == 1 => lower_ast_lift(db, args[0]),
                // `Ast.encode` — serialize an AST value to its canonical bytes (`ast-encoding.md` §The
                // Encoding Is A Bijection). FOLD a compile-time-visible `Ast` value (a `Core::SumNew` tree
                // of the Int/Name/List variants) to a `Core::BytesOf` of the canonical byte form; a runtime
                // Ast declines (the runtime serializer is a later increment). Equal trees fold to identical
                // bytes — the byte-identity the corpus asserts.
                Some(Prim::AstEncode) if args.len() == 1 => lower_ast_encode(db, id, args[0]),
                // `Ast.decode` — the TOTAL inverse: parse canonical bytes back to `(Ok ast)` / `(Err …)`.
                // FOLD a compile-time-visible `Core::BytesOf`: a well-formed WHOLE encoding → `(Ok <SumNew
                // tree>)`, anything else (ill-formed, truncated, or valid-prefix-plus-trailing) → `(Err
                // unit)`, NEVER a trap. A runtime Bytes declines (the runtime deserializer is a later
                // increment).
                Some(Prim::AstDecode) if args.len() == 1 => lower_ast_decode(db, id, args[0]),
                // `Blake3.of` — the blake3 content hash `Bytes → Bytes`. FOLD a compile-time-visible
                // `Bytes` (a `Core::ConstBytes` / a `Core::BytesOf` of constants) to the `Core::ConstBytes`
                // of its `blake3::hash`; a runtime `Bytes` declines (the runtime lowering to heap op 91 is
                // a later increment). Byte-identical to that op — both call the one `blake3` crate (§9).
                Some(Prim::Blake3Of) if args.len() == 1 => lower_blake3_of(db, id, args[0]),
                // `print` — render a compile-time-visible `Ast` value to its canonical re-readable TEXT
                // (`Core::ConstStr`). The text analogue of `Ast.encode`; a runtime Ast declines.
                Some(Prim::Print) if args.len() == 1 => lower_print(db, args[0]),
                // `read` — the inverse: parse a compile-time-visible `Core::ConstStr` as one s-expression
                // and reify it into the `Ast` `Core::SumNew` tree it denotes. A runtime String declines.
                Some(Prim::Read) if args.len() == 1 => lower_read(db, args[0]),
                Some(Prim::ListConcat) if args.len() == 2 => {
                    match (core_of(db, args[0]), core_of(db, args[1])) {
                        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
                        // FOLD two compile-time-visible list literals into ONE merged `Core::ListNew`
                        // (the elements of the left followed by those of the right) — a constant list
                        // that bakes at escape / folds through `List.at`/`len`, exactly as a written
                        // `(list …)` does. `List Int64` concat `List Int64` → `List Int64`; the element
                        // occurrences carry over unchanged (they keep their own types).
                        (Core::ListNew { elems: a }, Core::ListNew { elems: b }) => {
                            let mut a = a.to_vec();
                            a.extend(b.iter().copied());
                            trace!(target: "rcdzc::fold", node = id.0, len = a.len(), "List.concat folds two constant lists");
                            Core::ListNew { elems: a.into() }
                        }
                        // A runtime list operand — the persistent `vec-concat` on the heap.
                        _ => Core::ListConcat {
                            lhs: args[0],
                            rhs: args[1],
                        },
                    }
                }
                // `List.update` — replace the element at an index (runtime `vec-update`). Three args:
                // the list, the Int64 index, the replacement element. Any poison operand propagates.
                Some(Prim::ListUpdate) if args.len() == 3 => {
                    match (
                        core_of(db, args[0]),
                        core_of(db, args[1]),
                        core_of(db, args[2]),
                    ) {
                        (Core::Poison(r), _, _)
                        | (_, Core::Poison(r), _)
                        | (_, _, Core::Poison(r)) => Core::Poison(r),
                        // FOLD a constant list literal + a constant index: an IN-RANGE index (`0 <= i <
                        // len`) replaces that element (a new `Core::ListNew` with the slot swapped for the
                        // replacement's occurrence — a constant list that escapes/folds). An OUT-OF-RANGE
                        // index (negative or `>= len`) is a PROVABLE TRAP — the runtime `vec-update` traps
                        // OOB, so the compiler proves it and FAILS the build (CDZ0304), never ships a
                        // trapping component (numeric-model.md §A Constant Operation With No Value Is
                        // Rejected At Compile Time). The replacement element's own occurrence carries over.
                        (Core::ListNew { elems: a }, Core::ConstInt(i), _) => match i.to_i64() {
                            Some(n) if n >= 0 && (n as usize) < a.len() => {
                                let mut a = a.to_vec();
                                a[n as usize] = args[2];
                                trace!(target: "rcdzc::fold", node = id.0, index = n, "List.update folds (in-range constant index)");
                                Core::ListNew { elems: a.into() }
                            }
                            _ => {
                                trace!(target: "rcdzc::fold", node = id.0, "List.update out-of-range constant index → CDZ0304");
                                Core::Poison(Reject::coded(
                                    Code::ConstTrap,
                                    "List.update index is out of bounds (a constant out-of-range update traps)",
                                ))
                            }
                        },
                        // A runtime list or index — the persistent `vec-update` on the heap.
                        _ => Core::ListUpdate {
                            list: args[0],
                            index: args[1],
                            elem: args[2],
                        },
                    }
                }
                // `List.at` — the FALLIBLE indexed read `(List a) → Int64 → (Option a)`. FOLD when the
                // list is a compile-time-visible literal AND the index is a constant: an in-range index
                // yields `(Some elem)` (the element's own core), an out-of-range one (negative, or `>=`
                // arity) yields `None` — both built as a `Core::SumNew` of the result Option's variant
                // discriminants, so a constant `List.at` renders through the ordinary sum escape/fold with
                // no heap read. Otherwise emit the runtime `Core::ListAt` (a bounds-checked `vec-get`).
                Some(Prim::ListAt) if args.len() == 2 => lower_list_at(db, id, args[0], args[1]),
                // `Bytes.of` — construct a byte sequence from a list of `Int64` in `0..=255`. When the
                // operand is a compile-time-visible list literal, RANGE-CHECK each element now (a `< 0`
                // or `> 255` value is a compile-time trap, CDZ0304 — matching the runtime `bytes-set`
                // guard) and emit a `Core::BytesOf` carrying the element occurrences (the backend bakes
                // it / builds it on the rope heap). A runtime list source is a later increment (declines
                // cleanly for now — only a visible literal folds). One operand: the list.
                Some(Prim::BytesOf) if args.len() == 1 => lower_bytes_of(db, id, args[0]),
                // `Bytes.len` — FOLD when the operand is a compile-time-visible `Bytes.of` (its byte
                // count is statically known), else emit the runtime `Core::BytesLen` (`bytes-len`). One
                // operand: the bytes. Mirrors `List.len`.
                Some(Prim::BytesLen) if args.len() == 1 => {
                    let operand = args[0];
                    match core_of(db, operand) {
                        Core::BytesOf { elems } => {
                            trace!(target: "rcdzc::fold", node = id.0, len = elems.len(), "Bytes.len folds to a constant (visible Bytes.of literal)");
                            Core::ConstInt(IntValue::from_i64(elems.len() as i64))
                        }
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::BytesLen { operand },
                    }
                }
                // `String.scalar-len` / `String.byte-len` — FOLD on a constant string to its scalar (char)
                // count / UTF-8 byte count respectively (`collections-and-text.md` §A String Offers Both
                // A Scalar Length And A Byte Length). No escape: the result is an `Int64`. A runtime
                // string declines (the byte-rope length op arrives with the runtime string heap).
                Some(prim @ (Prim::StrScalarLen | Prim::StrByteLen)) if args.len() == 1 => {
                    match core_of(db, args[0]) {
                        Core::ConstStr(s) => {
                            let n = match prim {
                                Prim::StrScalarLen => s.chars().count(),
                                _ => s.len(), // UTF-8 byte length
                            };
                            trace!(target: "rcdzc::fold", node = id.0, ?prim, len = n, "String length folds to a constant");
                            Core::ConstInt(IntValue::from_i64(n as i64))
                        }
                        Core::Poison(r) => Core::Poison(r),
                        // A RUNTIME string length. The `type_of` probe + its `Ty` temporary are kept OUT of
                        // this (huge, per-descent-level recursive) `compute` frame via an `#[inline(never)]`
                        // helper — inlining them here grew `compute`'s stack frame enough to overflow the
                        // compile thread on a deeply-nested reduction (the 1024-deep poison-chain tests). See
                        // `lower_runtime_str_len`.
                        _ => lower_runtime_str_len(db, prim, args[0], id),
                    }
                }
                // `Char.to-int` — the TOTAL scalar-value read `Char → Int64`. FOLD a constant char to a
                // `Core::ConstInt` of its Unicode scalar value (`c as u32`). A RUNTIME char (Char-rep 1/N)
                // is an i32 code-point slot, so it emits `Core::CharToInt` — a zero-extend to the i64
                // `Int64` result (the char's slot IS its scalar value; the widen is the whole op).
                Some(Prim::CharToInt) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::ConstChar(c) => {
                        trace!(target: "rcdzc::fold", node = id.0, "Char.to-int folds to the scalar value");
                        Core::ConstInt(IntValue::from_i64(c as u32 as i64))
                    }
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::CharToInt { operand: args[0] },
                },
                // `Char.from-int` — the FALLIBLE conversion `Int64 → (Option Char)`. FOLD a constant int
                // to `(Some #\c)` when it is a Unicode scalar value, `(None unit)` for a surrogate /
                // out-of-range integer (`collections-and-text.md` §A Char Converts To And From An Integer
                // Totally). Never traps.
                Some(Prim::CharFromInt) if args.len() == 1 => lower_char_from_int(db, id, args[0]),
                // `Value.encode v` (R2) — the in-fold binary-AST value-form encode `∀a. a → Bytes`, TOTAL.
                // Emits `Core::ValueEncode { value, desc }` — the runtime `value-encode(v, desc)` op — with
                // `desc` the framed `(: value Type)` shape descriptor built from `v`'s type
                // (`sum_shape_descriptor`, the same self-describing descriptor the escape path uses, so
                // `Value.decode` round-trips it). The op walks the value handle (constant OR runtime — a
                // constant value still sits boxed on the heap) to its document, so no separate const-fold is
                // needed for correctness (a const-fold to baked bytes is a later optimization). Declines if
                // the value's type has no value-form descriptor (a function / type-value — no encodable
                // value-form; matches the boundary's decline domain).
                Some(Prim::ValueEncode) if args.len() == 1 => {
                    let vty = crate::infer::type_of(db, args[0]);
                    match sum_shape_descriptor(db, &vty) {
                        Some(desc) => Core::ValueEncode {
                            value: args[0],
                            desc: std::rc::Rc::from(desc.as_slice()),
                        },
                        None => Core::Poison(Reject::decline(
                            "Value.encode on a value whose type has no binary-AST value-form descriptor \
                             (a function / type-value has no encodable value-form)",
                        )),
                    }
                }
                // `Value.decode b` (R2) — the inverse `∀a. Bytes → (Option a)`, PARTIAL. The target type `a`
                // is the node's solved type peeled from `(Option a)`. Emits `Core::ValueDecode { bytes, desc,
                // disc_some, disc_none }` — the runtime `value-decode(b, desc)` op — with `desc` the same
                // framed descriptor for `a`. (A constant-bytes fold is a later refinement; the runtime path
                // is the common case.) Declines if the target type is unresolved (typing already declines an
                // unsolved `a` at the decode node, so this is defensive) or has no value-form descriptor.
                Some(Prim::ValueDecode) if args.len() == 1 => lower_value_decode(db, id, args[0]),
                // `Symbol.of` — intern a String into a Symbol (`String → Symbol`). A CONSTANT string folds
                // to a constant symbol, which shares the underlying `Core::ConstStr` REP (identity is
                // content-derived); only the static TYPE differs (`Ty::Symbol`, off this node's solved
                // type). So the fold is the identity on the `ConstStr` — `(= (Symbol.of "a") (Symbol.of
                // "a"))` then folds via `const_compound_eq(ConstStr, ConstStr)`. A RUNTIME string is
                // interned by CANONICALIZING its byte-rope to a flat leaf (`Core::StrToBytes` = the runtime
                // `bytes-compact`): a Symbol IS a String byte-leaf at run time (the value heap is tagless —
                // there is NO `Shape::Sym` and NO integer intern table; a Symbol renders through `Shape::Str`
                // and compares via `champ_eq` over its physical bytes, exactly like a String). So the
                // compacted handle IS a valid runtime Symbol value, and two symbols of equal content compare
                // equal because both are canonical flat leaves — that IS interning under a by-content
                // representation, no `str-intern` op or symbol table needed. The node's solved `Ty::Symbol`
                // carries the Symbol typing for rendering/equality; the rep is shared with String, exactly as
                // `String.to-bytes` reuses `bytes-compact` to retag a runtime string as Bytes. Frozen hash
                // unchanged (reuses an existing runtime op).
                Some(Prim::SymbolOf) if args.len() == 1 => match core_of(db, args[0]) {
                    c @ Core::ConstStr(_) => c,
                    Core::Poison(r) => Core::Poison(r),
                    _ if matches!(crate::infer::type_of(db, args[0]), crate::ty::Ty::String) => {
                        trace!(target: "rcdzc::lower", "Symbol.of on a runtime string → NFC-normalize then compact its byte-rope to a canonical Symbol leaf");
                        // A Symbol's identity is its interned CONTENT, and a String's content is its
                        // NFC-normalized form (FINDING #23) — so a symbol interned from a DECOMPOSED runtime
                        // string (`Symbol.of (String.concat "e" "<combining-acute>")`) must compose to the
                        // same leaf as its composed literal twin `#"é"`, or the two symbols wrongly compare
                        // unequal (violating the 17-symbols content-identity MUST). NFC-normalize the string
                        // BEFORE the byte-compact: wrap the operand in `Core::NfcNormalize`, then `StrToBytes`
                        // flattens the (now canonical) leaf. Already-NFC input (the common case) → the op is a
                        // no-op passthrough, so a plain ASCII symbol is unaffected.
                        let normalized = synth_core(
                            db,
                            Core::NfcNormalize { string: args[0] },
                            crate::ty::Ty::String,
                        );
                        Core::StrToBytes { string: normalized }
                    }
                    _ => runtime_string_op_decline(
                        db,
                        args[0],
                        "Symbol.of needs a String operand (a runtime symbol interns by canonicalizing its bytes)",
                    ),
                },
                // `BigInt.of x` — the EXACT widening from a fixed-width integer to `BigInt`. A CONSTANT
                // source folds to the SAME `Core::ConstInt` node retyped `Ty::BigInt` (its `IntValue` is
                // already `num-bigint`-backed and unbounded — the value is unchanged, only the static type
                // widens), exactly as `Symbol.of` keeps its `Core::ConstStr`. A RUNTIME source emits
                // `bigint-of-i64` (B3b) — the value's i64 slot widened into a BigInt heap leaf.
                Some(Prim::BigIntOf) if args.len() == 1 => match core_of(db, args[0]) {
                    c @ Core::ConstInt(_) => c,
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::BigIntOfI64 { value: args[0] },
                },
                // `Rational.of n d` — CONSTRUCT an exact rational. A CONSTANT numerator/denominator pair
                // folds to a NORMALIZED `Core::ConstRational` (gcd-reduce, sign on the numerator, denom
                // strictly positive); a ZERO denominator TRAPS ("rational with zero denominator"). A
                // runtime operand emits `Core::RationalOfInts` (the runtime `rational-of` op, R3b).
                Some(Prim::RationalOf) if args.len() == 2 => {
                    lower_rational_of(db, args[0], args[1])
                }
                // `Rational.of-int n` — the whole rational `n/1`. A constant folds; a RUNTIME int emits
                // `Core::RationalOfIntWiden` (widen n + the constant 1 to BigInt, then `rational-of`, R3b).
                Some(Prim::RationalOfInt) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::ConstInt(n) => normalized_rational(n, crate::ast::IntValue::from_i64(1)),
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::RationalOfIntWiden { value: args[0] },
                },
                // `Rational.value r` — the identity that just names `r`'s type. Folds to its operand.
                Some(Prim::RationalValue) if args.len() == 1 => core_of(db, args[0]),
                // `Rational.numerator r` / `Rational.denominator r` — read the numerator / denominator as a
                // `BigInt`. A CONSTANT `Core::ConstRational(n, d)` (already normalized: lowest terms, sign on
                // the numerator, denominator > 0) folds to the constant BigInt of `n` / `d` — a
                // `Core::ConstInt` retyped `Ty::BigInt` (its `IntValue` can exceed i64; the same const-BigInt
                // shape `BigInt.of` leaves). A RUNTIME Rational emits `Core::RationalNum` / `RationalDen` (the
                // runtime `rational-num` / `rational-den` ops, which borrow the Rational + return a fresh
                // BigInt handle). A poison operand propagates.
                Some(Prim::RationalNum) if args.len() == 1 => match core_of(db, args[0]) {
                    // A `Core::ConstInt` at this node (typed `Ty::BigInt` by inference — the op's result
                    // type) is the constant-BigInt shape, exactly as `BigInt.of` leaves. Its `IntValue` can
                    // exceed i64; the emit choke-point materializes it as a BigInt leaf.
                    Core::ConstRational(n, _) => Core::ConstInt(n),
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::RationalNum { operand: args[0] },
                },
                Some(Prim::RationalDen) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::ConstRational(_, d) => Core::ConstInt(d),
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::RationalDen { operand: args[0] },
                },
                // `Rational.truncate r` — the integer part TOWARD ZERO, narrowed to `Int64`. A DERIVATION
                // (no runtime op): a CONSTANT `Core::ConstRational(n, d)` (normalized, `d > 0`) folds to the
                // truncating quotient `n / d` (`IntValue::divmod` truncates toward zero, remainder takes the
                // dividend's sign — exactly the toward-zero integer part). A RUNTIME Rational synthesizes
                // `(let ((__r r)) (Int64.of (/ ((. Rational numerator) __r) ((. Rational denominator) __r))))`
                // and lowers THAT — `numerator`/`denominator` (→ BigInt), BigInt `/` (truncating), then the
                // checked `Int64.of` narrowing (TRAPS on overflow). All existing prims → hash-neutral.
                Some(Prim::RationalTruncate) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::ConstRational(n, d) => match n.divmod(&d) {
                        // `Rational.truncate` narrows the integer part to Int64. The quotient `q` is an
                        // arbitrary-precision `IntValue`, so it can EXCEED Int64 even when every source
                        // literal fits (here `3 * Int64.max`): the runtime twin's checked `Int64.of` TRAPS
                        // on that overflow, so the const fold must reject it AT THE FOLD (a compile-provable
                        // trap), NOT emit an out-of-width `Core::ConstInt(q)` — that beyond-i64 literal trips
                        // the backend's downstream width check as a span-less CDZ0302 ("integer literal does
                        // not fit its width"), which misattributes the overflow to a source literal (none is
                        // out of width) and gives no node to anchor to (adv-60). Rejecting here with
                        // `ConstTrap` (CDZ0304) + this node's span matches every other compile-provable const
                        // overflow (e.g. `(+ Int64.max 1)`) and surfaces in `cdz check`.
                        Some((q, _rem)) if q.to_i64().is_some() => Core::ConstInt(q),
                        Some(_) => Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "constant Rational.truncate traps: the integer part overflows Int64 \
                             — compute in a wider type, or check the value fits",
                        )),
                        // A `ConstRational` is normalized (`Rational.of` traps a zero denominator at
                        // construction), so `d > 0` and `divmod` is `Some`; this arm is defensive.
                        None => Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "rational with zero denominator has no integer part",
                        )),
                    },
                    Core::Poison(r) => Core::Poison(r),
                    _ => lower_rational_truncate(db, args[0]),
                },
                // `Rational.floor r` (toward −∞) / `Rational.ceil r` (toward +∞) — `truncate` adjusted by ±1
                // off the remainder sign. A CONSTANT `Core::ConstRational(n, d)` folds: `divmod` gives the
                // toward-zero quotient `q` + remainder `rem` (dividend-signed); floor subtracts 1 when the
                // value is NEGATIVE with a nonzero remainder (`n < 0 ∧ rem ≠ 0` — the toward-zero `q` is one
                // too HIGH for a floor), ceil adds 1 when POSITIVE with a nonzero remainder. A RUNTIME
                // Rational synthesizes the derivation subtree (a remainder-sign conditional over truncate).
                Some(Prim::RationalFloor) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::ConstRational(n, d) => match n.divmod(&d) {
                        Some((q, rem)) => {
                            let v = if n.negative && !rem.is_zero() {
                                q.sub(&crate::ast::IntValue::from_i64(1))
                            } else {
                                q
                            };
                            // Narrows to Int64 — reject a beyond-i64 integer part at the fold (CDZ0304),
                            // not as a span-less downstream width error (adv-60; see `RationalTruncate`).
                            if v.to_i64().is_some() {
                                Core::ConstInt(v)
                            } else {
                                Core::Poison(Reject::coded(
                                    Code::ConstTrap,
                                    "constant Rational.floor traps: the integer part overflows Int64 \
                                     — compute in a wider type, or check the value fits",
                                ))
                            }
                        }
                        None => Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "rational with zero denominator has no integer part",
                        )),
                    },
                    Core::Poison(r) => Core::Poison(r),
                    _ => lower_rational_floor_ceil(db, args[0], /* is_floor */ true),
                },
                Some(Prim::RationalCeil) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::ConstRational(n, d) => match n.divmod(&d) {
                        Some((q, rem)) => {
                            let v = if !n.negative && !rem.is_zero() {
                                q.add(&crate::ast::IntValue::from_i64(1))
                            } else {
                                q
                            };
                            // Narrows to Int64 — reject a beyond-i64 integer part at the fold (adv-60).
                            if v.to_i64().is_some() {
                                Core::ConstInt(v)
                            } else {
                                Core::Poison(Reject::coded(
                                    Code::ConstTrap,
                                    "constant Rational.ceil traps: the integer part overflows Int64 \
                                     — compute in a wider type, or check the value fits",
                                ))
                            }
                        }
                        None => Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "rational with zero denominator has no integer part",
                        )),
                    },
                    Core::Poison(r) => Core::Poison(r),
                    _ => lower_rational_floor_ceil(db, args[0], /* is_floor */ false),
                },
                // `Rational.round r` — NEAREST integer, ties HALF-AWAY-FROM-ZERO. A CONSTANT
                // `Core::ConstRational(n, d)` folds: the toward-zero quotient `q` (+ remainder `rem`,
                // dividend-signed) adjusted AWAY from zero (by the sign of `n`) when `2·|rem| ≥ d` — the
                // fractional part is ≥ ½ (`≥` gives the half-away tie: at exactly ½, `2·|rem| = d`, round
                // away). A RUNTIME Rational synthesizes the derivation subtree (the same tie test over the
                // truncate). `d > 0` (normalized), so the compare is well-defined.
                Some(Prim::RationalRound) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::ConstRational(n, d) => match n.divmod(&d) {
                        Some((q, rem)) => {
                            // `|rem|` — the remainder's magnitude (rem is dividend-signed).
                            let abs_rem = crate::ast::IntValue {
                                negative: false,
                                magnitude: rem.magnitude.clone(),
                            };
                            let two = crate::ast::IntValue::from_i64(2);
                            // Tie test `2·|rem| ≥ d` — round away from zero (by the sign of `n`) when true.
                            let v = if two.mul(&abs_rem).cmp(&d) != std::cmp::Ordering::Less {
                                let one = crate::ast::IntValue::from_i64(1);
                                if n.negative { q.sub(&one) } else { q.add(&one) }
                            } else {
                                q
                            };
                            // Narrows to Int64 — reject a beyond-i64 rounded value at the fold (adv-60).
                            if v.to_i64().is_some() {
                                Core::ConstInt(v)
                            } else {
                                Core::Poison(Reject::coded(
                                    Code::ConstTrap,
                                    "constant Rational.round traps: the rounded value overflows Int64 \
                                     — compute in a wider type, or check the value fits",
                                ))
                            }
                        }
                        None => Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "rational with zero denominator has no integer part",
                        )),
                    },
                    Core::Poison(r) => Core::Poison(r),
                    _ => lower_rational_round(db, args[0]),
                },
                // `Symbol.to-string` — recover a Symbol's content String (`Symbol → String`, the inverse of
                // `Symbol.of`). A constant symbol IS its `Core::ConstStr`, so this folds to that same node
                // retyped `String` (the node's solved type); the rep is unchanged. A RUNTIME symbol is a
                // byte-leaf identical to a String (tagless heap; see `Symbol.of`), so recovering the String
                // is the same CANONICALIZING retag — `Core::StrToBytes` (`bytes-compact`) flattens the
                // handle to a canonical leaf the node's `Ty::String` renders/compares as a String. Guarded
                // on a genuine `Ty::Symbol` operand (a non-symbol is a type error `infer` already coded).
                // Frozen hash unchanged.
                Some(Prim::SymbolToString) if args.len() == 1 => match core_of(db, args[0]) {
                    c @ Core::ConstStr(_) => c,
                    Core::Poison(r) => Core::Poison(r),
                    _ if matches!(crate::infer::type_of(db, args[0]), crate::ty::Ty::Symbol) => {
                        trace!(target: "rcdzc::lower", "Symbol.to-string on a runtime symbol → compact its bytes to a canonical String leaf");
                        Core::StrToBytes { string: args[0] }
                    }
                    _ => Core::Poison(Reject::decline(
                        "Symbol.to-string needs a Symbol operand (its runtime read recovers the byte leaf)",
                    )),
                },
                // `Bytes.at` — the FALLIBLE indexed read `Bytes → Int64 → (Option Int64)`. Mirrors
                // `List.at`: FOLD a visible `Bytes.of` indexed by a constant (in-range → `(Some byte)`,
                // out-of-range/negative → `None`), else emit the runtime `Core::BytesAt`.
                Some(Prim::BytesAt) if args.len() == 2 => lower_bytes_at(db, id, args[0], args[1]),
                // `Bytes.concat` — append two byte sequences. FOLD a constant pair to a single
                // `Core::BytesOf` (its bytes are the concatenation); else emit runtime `Core::BytesConcat`.
                Some(Prim::BytesConcat) if args.len() == 2 => {
                    lower_bytes_concat(db, args[0], args[1])
                }
                // `Bytes.slice` — the FALLIBLE sub-range read. FOLD a constant `Bytes.of` + constant
                // start/len (in range → `(Some (Bytes.of <slice>))`, out → `None`), else `Core::BytesSlice`.
                Some(Prim::BytesSlice) if args.len() == 3 => {
                    lower_bytes_slice(db, id, args[0], args[1], args[2])
                }
                // `Bytes.compact` — content-equal, storage-independent. On a constant it is the identity
                // (same bytes); a runtime value emits `Core::BytesCompact`.
                Some(Prim::BytesCompact) if args.len() == 1 => {
                    let operand = args[0];
                    match core_of(db, operand) {
                        // A constant `Bytes.of` compacts to itself (content-equal); no runtime op.
                        c @ Core::BytesOf { .. } => c,
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::BytesCompact { operand },
                    }
                }
                // `String.at` — the FALLIBLE scalar-indexed read. FOLD a constant string + constant index
                // to `(Some "<char>")` in range / `None` out (by Unicode SCALAR position, not byte). A
                // runtime string declines (the byte-rope read is a later increment).
                Some(Prim::StrAt) if args.len() == 2 => lower_str_at(db, id, args[0], args[1]),
                // `String.scalar-at` — the FALLIBLE read of the CHAR at a scalar position. FOLD a constant
                // string + constant index to `(Some #\c)` in range / `(None unit)` out (by Unicode SCALAR
                // position, not byte). The char-typed companion of `String.at`. A runtime string declines.
                Some(Prim::StrScalarAt) if args.len() == 2 => {
                    lower_str_scalar_at(db, id, args[0], args[1])
                }
                // `String.slice` — the FALLIBLE sub-range read by SCALAR offsets `[start, end)`. FOLD a
                // constant string + constant bounds to `(Some "<substr>")` in range / `None` out (reversed,
                // over-long, or negative). A runtime string declines (the byte-rope slice is a later
                // increment).
                Some(Prim::StrSlice) if args.len() == 3 => {
                    lower_str_slice(db, id, args[0], args[1], args[2])
                }
                // `String.to-bytes` — the UTF-8 encoding. FOLD a constant string to a `Core::BytesOf` of
                // its UTF-8 bytes; a runtime string declines.
                Some(Prim::StrToBytes) if args.len() == 1 => lower_str_to_bytes(db, args[0]),
                // `String.from-bytes` — the TOTAL UTF-8 decode → `(Option String)`. FOLD a constant Bytes
                // via strict UTF-8; a runtime Bytes declines.
                Some(Prim::StrFromBytes) if args.len() == 1 => {
                    lower_str_from_bytes(db, id, args[0])
                }
                // `Option.expect` / `Result.expect` — the unwrap-or-trap accessor. `args[0]` is the sum,
                // `args[1]` the message (dropped — the wasm trap is textless). FOLD a constant PRESENT
                // variant to its payload; a runtime sum emits `Core::SumExpect` (disc probe → payload /
                // trap).
                Some(Prim::SumExpect) if args.len() == 2 => lower_sum_expect(db, id, args[0]),
                // `(trap "message")` — the diverging primitive. Its message argument is DROPPED (the wasm
                // trap carries no text) and it lowers to the unconditional `Core::Trap` (an `unreachable`).
                // A malformed argument in the message still surfaces its own fault: descend for it, and if
                // the message poisoned, propagate THAT (an unbound name in the message is the reported
                // fault, not the trap). Arity is exactly one (the scheme is `String → a`).
                Some(Prim::Trap) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::Trap,
                },
                // `Int64.checked-add` / `checked-mul` — the FALLIBLE arithmetic. FOLD a constant operand
                // pair to `(Some result)` in range / `(None unit)` on overflow; a runtime operand is a
                // later increment (declines cleanly).
                Some(prim @ (Prim::CheckedAdd | Prim::CheckedSub | Prim::CheckedMul))
                    if args.len() == 2 =>
                {
                    lower_checked_arith(db, id, prim, args[0], args[1])
                }
                // `Int64.wrapping-add` / `wrapping-mul` — two's-complement wraparound, NEVER trapping. FOLD
                // a constant pair via `wrapping_*`; a runtime operand emits `Core::Arith` (which for a
                // wrapping prim selects the RAW machine op, no overflow guard).
                Some(prim @ (Prim::WrappingAdd | Prim::WrappingSub | Prim::WrappingMul))
                    if args.len() == 2 =>
                {
                    lower_wrapping_arith(db, id, prim, args[0], args[1])
                }
                // `String.concat` — the TOTAL binary join. FOLD two constant strings to their
                // concatenation (the result is another constant `String`). The value form is always NFC,
                // and NFC is NOT closed under concatenation in general (a combining mark starting the RIGHT
                // operand can compose with the base char ending the LEFT one). The reader already NFC-
                // normalizes each `ConstStr`, and concatenation of two ALL-ASCII strings is trivially NFC
                // (ASCII carries no combining marks) — so fold that case, which the compiler's own error-
                // message/name assembly (and every corpus concat case) lives in. A concat where either
                // operand has a non-ASCII scalar DECLINES: re-normalizing the join would need Unicode
                // tables, and the pure compiler core carries no value deps (that arrives with the runtime
                // byte-rope join). A runtime operand likewise declines.
                Some(Prim::StrConcat) if args.len() == 2 => {
                    match (core_of(db, args[0]), core_of(db, args[1])) {
                        (Core::ConstStr(a), Core::ConstStr(b)) if a.is_ascii() && b.is_ascii() => {
                            trace!(target: "rcdzc::fold", node = id.0, "String.concat folds two constant ASCII strings");
                            Core::ConstStr(format!("{a}{b}").into())
                        }
                        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
                        // A RUNTIME string concatenation: a String value IS a flat UTF-8 byte leaf (an i32
                        // heap handle), and a UTF-8 join is byte concatenation (no re-normalization for
                        // this increment — the operands are already well-formed UTF-8), so it is exactly
                        // `bytes-concat` over the two byte handles — `Core::BytesConcat`, the runtime op
                        // already working for `Bytes.concat`, producing a fresh joined byte leaf (a String
                        // handle). Guarded on BOTH operands being definite Strings (defensive; the
                        // `StrConcat` scheme already constrains them).
                        _ if matches!(
                            crate::infer::type_of(db, args[0]),
                            crate::ty::Ty::String
                        ) && matches!(
                            crate::infer::type_of(db, args[1]),
                            crate::ty::Ty::String
                        ) =>
                        {
                            trace!(target: "rcdzc::lower", node = id.0, "String.concat on runtime strings → NFC-normalize(bytes-concat over their byte leaves)");
                            // A runtime String join is a raw `bytes-concat` of the two byte leaves — but a
                            // String's identity is its NFC-NORMALIZED contents (collections-and-text.md
                            // L33-34/L53-54 MUSTs; FINDING #23), and a decomposed sequence assembled here
                            // (`concat "e" "<combining-acute>"`) would otherwise be stored un-normalized →
                            // wrong length + unequal to / unfindable-as-key-against its composed literal twin.
                            // So wrap the concat in `Core::NfcNormalize`, which emits the `str-nfc-normalize`
                            // runtime op to canonicalize the joined result. The op is a no-op (same handle) for
                            // already-NFC text (the ASCII/pre-composed common case), so this is near-free there.
                            let concat = synth_core(
                                db,
                                Core::BytesConcat {
                                    lhs: args[0],
                                    rhs: args[1],
                                },
                                crate::ty::Ty::String,
                            );
                            Core::NfcNormalize { string: concat }
                        }
                        _ => {
                            // Defer to the authoritative CDZ0203 when an operand is a definite non-String
                            // (`(String.concat "a" 5)` — the fault is the type, not const-ASCII folding);
                            // a genuine runtime-String operand keeps the honest const-ASCII-only note.
                            let arg = if matches!(
                                crate::infer::type_of(db, args[0]),
                                crate::ty::Ty::String
                            ) {
                                args[1]
                            } else {
                                args[0]
                            };
                            runtime_string_op_decline(
                                db,
                                arg,
                                "a string concatenation is only folded for constant ASCII operands (the \
                                 normalizing byte-rope join arrives with the runtime string heap)",
                            )
                        }
                    }
                }
                // `Map.insert` — add-or-replace `key ↦ val`, returning the new map. For M1 the map operand
                // is a RUNTIME map (built inline or a parameter); emit `Core::MapInsert` carrying the
                // solved key/value types (for the box ops). A poison operand propagates.
                Some(Prim::MapInsert) if args.len() == 3 => lower_map_insert(db, id, &args),
                // `Map.lookup` — the FALLIBLE keyed read `(Map k v) → k → (Option v)`. Emit the runtime
                // `Core::MapLookup` (a NULL-or-handle test → `Some`/`None`). The result Option's discs are
                // read off the result type; the value type off the map operand.
                Some(Prim::MapLookup) if args.len() == 2 => {
                    lower_map_lookup(db, id, args[0], args[1])
                }
                // `Map.remove` — drop a key's association, returning the new map. Emit `Core::MapRemove`.
                Some(Prim::MapRemove) if args.len() == 2 => lower_map_remove(db, args[0], args[1]),
                // `Map.size` — the count of distinct keys, an `Int64`. Emit the runtime `Core::MapSize`.
                Some(Prim::MapSize) if args.len() == 1 => {
                    let map = args[0];
                    match core_of(db, map) {
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::MapSize { map },
                    }
                }
                Some(Prim::MapToList) if args.len() == 1 => lower_map_to_list(db, args[0]),
                // `Set.of` — construct a set from a LIST of elements (dedup). Emit `Core::SetOf` carrying
                // the element occurrences + the solved element type; a constant list folds to a canonical
                // set. `Set.contains`/`len`/`insert`/`remove` + the algebra ops each lower to their runtime
                // `Core::Set*` (folding a constant operand). The element type comes from the RESULT node's
                // solved `Ty::Set` (fully determined by unification, even for a bare `(Set.of (list))`).
                Some(Prim::SetOf) if args.len() == 1 => lower_set_of(db, id, args[0]),
                Some(Prim::SetContains) if args.len() == 2 => {
                    lower_set_contains(db, args[0], args[1])
                }
                Some(Prim::SetToList) if args.len() == 1 => lower_set_to_list(db, args[0]),
                Some(Prim::SetLen) if args.len() == 1 => {
                    let set = args[0];
                    match core_of(db, set) {
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::SetLen { set },
                    }
                }
                Some(prim @ (Prim::SetInsert | Prim::SetRemove)) if args.len() == 2 => {
                    lower_set_insert_remove(db, prim, args[0], args[1])
                }
                Some(prim @ (Prim::SetUnion | Prim::SetIntersection | Prim::SetDifference))
                    if args.len() == 2 =>
                {
                    lower_set_algebra(db, prim, args[0], args[1])
                }
                // `Map.swap` / `Map.take` — the value-yielding forms — reduce (via `reduce_ctor`) to the
                // synthesized tuple `(tuple (Map.lookup m k) (Map.insert/remove m k v))`, then lower that.
                // Going through `reduce_ctor` (not a direct build) means `reduce_to_tuple_elems` reduces
                // them the SAME way, so a `(. (Map.swap …) 0)` projection folds to just the lookup — the
                // corpus shape — dropping the unused new map with no heap build. Falls into the `Some(prim)`
                // constructor catch-all below (which calls `reduce_ctor`); no dedicated arm needed here.
                // Every other constructor prim — including the compound-VALUE constructors `TupleNew`/
                // `RecordNew` reached via the shadowable `tuple`/`record` alias names — reduces via
                // `reduce_ctor`, which rewrites `(tuple a b)` → the symbol-headed `((,) a b)` (and
                // `(record …)` → `({} …)`). Lowering the reduced node then goes through the ORDINARY
                // `Resolved::Tuple`/`Record` path — so a constant compound FOLDS (a projection reads the
                // element with no heap) exactly as a symbol-written one does, with no value-ctor special
                // case here. (A type constructor like `(Int 64)` reduces to its module the same way.)
                Some(prim) => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: constructor prim");
                    match crate::eval::reduce_ctor(db, prim, id, &args) {
                        Ok(built) => core_of(db, built),
                        // A NON-constructor OPERATION prim (`list-at`, `map-insert`, …) reaches `reduce_ctor`
                        // ONLY here, when its full-arity arm above did not match — the operation was applied
                        // to the WRONG NUMBER of arguments. `reduce_ctor` cannot build it, returning the
                        // internal `NOT_A_CTOR_PRIM` sentinel; surfacing that verbatim leaked
                        // `error: not a type constructor` for a plain `(. List at l)` (a partial application,
                        // missing the index). Rewrite it into an HONEST decline naming the operation and its
                        // shape: a partial application of a built-in operation is a genuine not-yet-built
                        // construct (it needs a runtime closure), NOT a type-constructor error. An
                        // OVER-application ALSO lands here, but `infer` already reports it as the coded
                        // CDZ0203 "applied N arguments to a function of arity M"; this decline is the weaker
                        // sibling (a Todo), so the coded reject remains the primary "no".
                        Err(msg) if msg == crate::eval::NOT_A_CTOR_PRIM => {
                            let named = op_member_name(db, head)
                                .map(|n| format!("`{n}`"))
                                .unwrap_or_else(|| "a built-in operation".to_string());
                            trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: operation applied at the wrong arity → honest decline");
                            // Arity-neutral wording: this fires on BOTH an under-application (`(List.at l)`,
                            // missing the index — the common case, which would need a runtime closure) and an
                            // over-application (`(Map.size m x)` — already the coded CDZ0203, this is its
                            // weaker Todo sibling). Both are "applied at the wrong arity".
                            Core::Poison(Reject::declined(
                                crate::diag::DeclineId::PrimAsValueNeedsClosure,
                                format!(
                                    "{named} is applied at the wrong arity — a built-in operation must be \
                                     applied to exactly its arguments (a partial application, which would \
                                     require a synthesized runtime closure, is not supported)"
                                ),
                            ))
                        }
                        Err(msg) => {
                            trace!(target: "rcdzc::lower", node = id.0, %msg, "apply: constructor declined");
                            Core::Poison(Reject::decline(msg))
                        }
                    }
                }
                // Not applyable. If the head itself is a poison (e.g. an unbound name), propagate THAT
                // root cause — an unbound head is a scope error, not merely "not applyable".
                None => match core_of(db, head) {
                    Core::Poison(r) => Core::Poison(r),
                    _ => {
                        trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: head is not applyable (decline)");
                        Core::Poison(Reject::decline(crate::diag::NOT_APPLYABLE_DECLINE))
                    }
                },
            }
        }
        Resolved::Poison(r) => Core::Poison(r),
        // A parameter reference is a RUNTIME value — its value is unknown at compile time, so it lowers
        // to a `Core::Param` the backend reads as a `local.get` of the parameter's slot. (A parameter
        // only reaches lowering when its function body is emitted STANDALONE — an exported function; at
        // a constant call site the param is substituted by the fold and never lowered as a param.)
        Resolved::Param { binder } => Core::Param { binder },
        // A TYPE VALUE is compile-time-only — no runtime core form (the erasure fence forbids one
        // reaching runtime), so lowering it as a runtime value declines.
        Resolved::TypeVal(_) => {
            Core::Poison(Reject::decline(crate::diag::TYPE_VALUE_NO_RUNTIME_DECLINE))
        }
        // A LAMBDA that survives to lowering as a RUNTIME value (it could not be β-reduced away — it is
        // passed to a recursive callee, or stored in a runtime cell). LIFT it to a standalone function
        // and produce a `Core::Closure` naming its table slot. Only a NO-CAPTURE (combinator) lambda
        // lifts in this increment; a lambda with free variables declines (captures are a later step).
        Resolved::Lambda { params, body } => lower_lambda_value(db, id, &params, body),
        // A `handle` is REDUCED AWAY (E1c): resolve each enclosed perform to its concrete arm and rewrite
        // the tail-resumptive case to plain code — the perform becomes the arm's resume value, the
        // next-state threads forward (`DESIGN-effects-rcdzc.md` §4.1). `reduce_handle` produces a
        // rewritten BODY occurrence, which we then lower by the ordinary path (so `select` sees only
        // plain `Core`). A case the tail path cannot serve (a non-tail/absent resume, a cross-function or
        // recursive perform) makes `reduce_handle` return `None` → DECLINE (a Todo, never a miscompile).
        Resolved::Handle { init, arms, body } => {
            match crate::effects::reduce_handle(db, init, &arms, body) {
                Some(rewritten) => {
                    // The rewritten body is a synthesized subtree with root parent `None` (`push_list`).
                    // Graft it UNDER the original `handle` node so a FREE variable inside it — e.g. an
                    // enclosing function's parameter used directly in the handle body (`(handle … (+ x
                    // (E.op)))`) — resolves up the original lexical chain instead of hitting CDZ0101. We
                    // parent to the `handle` node ITSELF (not its parent): the scope walk from a free name
                    // then ascends rewritten → handle → …, and a binder form above (a `def`/`fn`/`let`)
                    // recognizes the handle as the child it ascended from (its recorded body slot), so its
                    // `from == body_occ` param check still fires. Re-parenting to the handle's parent would
                    // instead present the rewritten node as the child, which that check would reject. (A
                    // perform's own binders were already substituted by the fold; only free names need this.)
                    db.reparent(rewritten, Some(id), db.child_ix_of(id) as u32);
                    // (A) CASE2 (#5321): mark the reduced-handle subtree as a handler region so `lower_let`'s
                    // strict-heap-ctor decompose SKIPS a dead ctor whose `let` lands here — the handler
                    // tail/#st-drop lowering reads `do` forms not `Core::Seq`, so the decompose's Seq wrapper
                    // would perturb it (olc1/cst1/sga1). Marked BEFORE the body's descendants are lowered.
                    crate::effects::mark_handler_region(db, rewritten);
                    // ESCAPED-CLOSURE LEAK GUARD (diagnostic quality). The rewritten body may STILL carry a
                    // discharged-op perform lexically inside a LIVE lambda — the fold could not route it
                    // because the closure ESCAPED its reach (stored in a collection, extracted via `List.at`
                    // + `match ((Some f) …)`, applied through the slot; `subtree_performs` treats a lambda
                    // VALUE as pure, since a closure body performs only when APPLIED, and the fold cannot
                    // trace the application back through the collection slot). That lambda lifts to a
                    // STANDALONE function lowered with NO handle frame, so its perform reaches the
                    // standalone-perform arm below and would surface the MISLEADING `NO_HOME_STANDALONE_DECLINE`
                    // ("performed with no enclosing handler here") — even though this handle LEXICALLY
                    // encloses it. Decline with the honest `HANDLER_NOT_REDUCIBLE_DECLINE` todo instead of
                    // lowering a body that lies about the home (breaker's diagnostic-quality finding, routed
                    // by corpus-bugfix 2026-07-28; same discipline as the guard-perform decline — a
                    // not-yet-routed path must say "not yet reducible", NEVER a false "no home"). SAFE
                    // reject, honest MESSAGE. Checked AFTER `reparent` so the walk's `resolved_of` sees the
                    // grafted lexical chain (a pre-graft walk would cache free names as unbound → spurious
                    // CDZ0101), and only at the OUTER lowering fold (never an intermediate recursive one).
                    if crate::effects::reduced_body_leaks_escaped_perform(db, rewritten, &arms) {
                        // ESCAPED-CLOSURE-LEAK RECOVERY (cx5d). The leak is a discharged perform inside a
                        // closure passed to a NON-recursive ONE-SHOT helper that APPLIES it out of the fold's
                        // reach. β-inline that helper in the ORIGINAL body so the closure is applied INLINE,
                        // re-run `reduce_handle`, and use the result ONLY if it no longer leaks — turning the
                        // decline into a fold. SAFE: a case that folds today never leaks, so never reaches
                        // here; the inline is one-shot-gated (perform runs once) and re-checked (used only if
                        // it now folds clean), so no value or non-leaking case changes.
                        if let Some(inlined) =
                            crate::effects::inline_escaped_one_shot_perform_call(db, body, &arms)
                            && let Some(rw2) =
                                crate::effects::reduce_handle(db, init, &arms, inlined)
                        {
                            db.reparent(rw2, Some(id), db.child_ix_of(id) as u32);
                            crate::effects::mark_handler_region(db, rw2);
                            if !crate::effects::reduced_body_leaks_escaped_perform(db, rw2, &arms) {
                                // Commit the recovery ONLY if it folds to a POISON-FREE core. The inline
                                // rewrite can rebuild a `let` whose binding-init held the helper call (a
                                // let-bound helper-call answer, `(let ((a (ap …))) a)`), and that rebuild
                                // can drop the let-binder's identity so the body reference re-resolves
                                // UNBOUND — a `core_of` that is a CDZ0101 `Poison` on a WELL-FORMED program
                                // (breaker iso-b). Surfacing that would be a wrong-diagnostic REJECTION; the
                                // recovery's contract is to be used only when it folds CLEAN, so a
                                // poison result must fall back to the honest `HANDLER_NOT_REDUCIBLE` todo
                                // (the same state the un-recovered leak would decline to). A genuinely
                                // folding recovery (cx5d/cx6/cx9…) yields a non-poison core, unchanged.
                                let c2 = core_of(db, rw2);
                                if !matches!(c2, Core::Poison(_)) {
                                    return c2;
                                }
                            }
                        }
                        return Core::Poison(Reject::unsupported(
                            crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE,
                        ));
                    }
                    core_of(db, rewritten)
                }
                None => {
                    // `reduce_handle` failed. When the handle's EFFECT NAME is UNBOUND (`handle Nope …`),
                    // every arm op `(. Nope op)` projects an unbound name — a CDZ0101 already reported
                    // authoritatively at the name — and the fold could never run, so the generic "not yet
                    // reducible" decline here is a SHADOW of that CDZ0101 (a second `error:` for one root
                    // cause). Detect it by lowering an arm op whose `(meta effect-op)` is absent and
                    // checking its poison is a CDZ0101; if so, propagate THAT poison (it dedups against the
                    // anchored unbound-name copy) so `handle Nope …` reports ONE error carrying the
                    // did-you-mean fix. An UNDECLARED op on a KNOWN effect (`gett` on `E`) is left to its
                    // CDZ0403 (whose decline M2's `dedup_faults` already suppresses) — lowering that arm op
                    // would surface the weaker raw member-access CDZ0201 instead. A handle whose arms all
                    // resolve but still can't fold (a real cross-function / non-tail resume) keeps the
                    // honest decline (the corpus expects it).
                    let unbound_arm_op = arms.iter().map(|a| a.op).find(|&op| {
                        crate::eval::effect_op_of(db, op).is_none()
                            && matches!(
                                core_of(db, op),
                                Core::Poison(ref r) if r.code == Some(crate::diag::Code::Unbound)
                            )
                    });
                    match unbound_arm_op {
                        Some(op) => core_of(db, op),
                        None => Core::Poison(Reject::unsupported(
                            crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE,
                        )),
                    }
                }
            }
        }
        // A `(host (E…) body)` DELEGATES its listed effects to the component boundary (an entrypoint's
        // routing decision). The delegation itself carries no runtime value — its VALUE is the body's
        // value — so lower the BODY; a perform of a delegated effect inside it becomes a `Core::HostCall`
        // (the perform arm resolves the enclosing `host` via `perform_host_target`). The manifest
        // contribution (the escaping effect row) is handled at serialization.
        Resolved::Host { body, .. } => core_of(db, body),
        Resolved::Resume { .. } => Core::Poison(Reject::unsupported(
            crate::diag::RESUME_NOT_REDUCIBLE_DECLINE,
        )),
    }
}

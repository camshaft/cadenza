//! `lower` — the query that fills the core column: for a node's `StructId`, its A-normal [`Core`]
//! form.
//!
//! One concern: lowering the resolved tree to the A-normal core. [`core_of`] reads a node's resolved
//! form (via [`crate::resolve::resolved_of`]) and produces its core form, memoizing into `db.core`;
//! it is the ONLY module that fills that column. Where a lowering decision needs the solved type it
//! READS it (via [`crate::infer::type_of`]) rather than recomputing one (`reference-compiler.md`
//! §One Pass Owns One Concern — architecture, not duvet-cited). Lowering DOES branch on the solved
//! type: a comparison folds constants but stays runtime for a scalar parameter, a match classifies
//! its scrutinee, and a runtime compound's value form is templated off its `Ty`.
//!
//! Lowering is mostly a structural map — a literal → a `Const*`, an `if` → a core `If` on the same
//! child ids (lowered on their own demand) — with three constructs that name intermediate values:
//! [`lower_let`] A-normalizes a multi-use runtime `let` into `Core::Let`/`LocalRef` (a single-use or
//! constant `let` is erased by copy-propagation), a lambda application β-reduces and lowers its result
//! (so a non-recursive call monomorphizes away), and a recursive call whose callee has a determined
//! signature lowers to a real `Core::Call`. The core's own fresh-id space is still unneeded: every
//! binding it introduces is keyed by an existing source occurrence.

use crate::arena::Slot;
use crate::ast::{IntValue, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Fix, Reject};
use crate::resolve::resolved_of;
use crate::resolved::{Prim, Resolved};
use tracing::trace;

/// A LAMBDA-LIFTED closure — a `(fn (param…) body)` that survived lowering as a runtime value and was
/// hoisted to a standalone wasm function. Its position in `db.lifted` is its funcref-TABLE slot. Every
/// closure is UNIFORMLY an `(env, param…) -> result` function (so a function-typed parameter can hold a
/// capturing OR a non-capturing closure interchangeably): the closure CELL (`box-int(code)` + captures)
/// is passed as the env at local slot 0, and the lambda's own parameters at slots 1.. . A body reference
/// to a lambda parameter is a `Core::Param`; a reference to a CAPTURED free variable is a
/// `Core::Captured { index }` that reads the env cell (recorded per-occurrence in `db.captured_ref`).
/// The param + result machine types come from the lambda's body. A MULTI-parameter lambda is supported
/// only when applied at FULL arity (a `(g a b)` application → one `call_indirect` with all args); a
/// partial application of a runtime multi-param closure (runtime currying) still declines.
/// `DESIGN-runtime-closures-rcdzc.md` §3.
#[derive(Clone, PartialEq, Debug)]
pub struct LiftedLambda {
    /// The lambda's `body` occurrence — the identity that dedups a lambda lifted more than once, and
    /// what the backend selects as the function body.
    pub body: StructId,
    /// The lambda's parameters, in order — each `(binder occurrence, solved machine type)`. They occupy
    /// wasm local slots `1..1+params.len()` (slot 0 is the env cell).
    pub params: Vec<(StructId, crate::ty::Ty)>,
    /// The result's solved machine type.
    pub ret_ty: crate::ty::Ty,
    /// The captured free variables, in cell order (cell index `1 + position`). Each is the binder
    /// occurrence of the enclosing binding the lambda body references. Empty for a combinator.
    pub captures: Vec<StructId>,
}

/// The core (A-normal) form of the node at `id`, filling the column on demand (memoized). Reads the
/// resolved form; children stay ids, lowered on their own demand.
/// If any of `elems` lowers to a REDUCTION-BOUND poison (`Code::RecursionBound`), return that poison so a
/// compound construction (tuple / list / record value) built around it COLLAPSES to the poison rather than
/// surviving as a giant `Core::Tuple`/`ListNew`/`Record`. A non-normalizing self-application in a compound
/// slot — `((fn v (tuple (v v) 1)) (fn v (tuple (v v) 1)))` — β-reduces (bounded by `REDUCE_NODE_BUDGET`)
/// into a memoized core chain thousands of `Core::Tuple` levels deep, bottoming out in that poison; without
/// this collapse, every downstream structural walk over the core tree (`layout::collect_call_callees`,
/// `compile::collect_reached_poisons`, emit/serialize) would descend the whole chain in native recursion
/// and OVERFLOW THE COMPILER'S STACK on a small valid-to-parse program. Collapsing at the source — the ONE
/// place the compound is built — stops that at every consumer at once, not walk-by-walk.
///
/// Only `RecursionBound` (a resource-limit poison — the reduction could not finish) collapses; a `ConstTrap`
/// element does NOT, preserving dead-code elimination (`(. (tuple 42 (/ 100 0)) 0)` projects away its
/// trapping element and warns CDZ0305, never rejecting). A `RecursionBound` element is not DCE-eligible: the
/// compiler did not PROVE the computation traps, it ran out of budget, so the enclosing compound genuinely
/// cannot be built and the poison must propagate.
fn reduction_bound_element(db: &mut Db, elems: &[StructId]) -> Option<Reject> {
    for &e in elems {
        if let Core::Poison(r) = core_of(db, e)
            && r.code == Some(Code::RecursionBound)
        {
            return Some(r);
        }
    }
    None
}

pub fn core_of(db: &mut Db, id: StructId) -> Core {
    if let Slot::Filled(c) = db.core.get(id) {
        trace!(target: "rcdzc::lower", node = id.0, "memo hit");
        return c.clone();
    }
    // A CAPTURED-reference occurrence (a body reference to a free variable of a lifted lambda) reads the
    // env cell — `Core::Captured` — rather than following the ref to its (out-of-scope) binding. Recorded
    // by `lower_lambda_value` when the lambda was lifted; keyed by this reference occurrence. Checked
    // before the ordinary resolved dispatch so the ref is not followed. (Not memoized into the column
    // here — it is filled below like any node — but the map lookup is O(1) and the value is stable.)
    if let Some(&(index, ref ty)) = db.captured_ref.get(&id) {
        let c = Core::Captured {
            index,
            ty: ty.clone(),
        };
        db.core.fill(id, c.clone());
        return c;
    }
    // Recursive-descent DEPTH GUARD. `compute` re-enters `core_of` for a node's sub-expressions, so a
    // pathologically deep nest (`(+ 1 (+ 1 …))` thousands deep) or an unproductive self-recursion a
    // nullary call re-enters (`(def (f) (f))`) would recurse until the native stack overflows and the
    // PROCESS ABORTS. Past `LOWER_DEPTH_LIMIT` decline (a resource-limit poison) instead — a compiler
    // must never crash on well-formed input, only decline or complete (decline-don't-miscompile). This
    // result is NOT memoized: the same node lowered from a shallower context (below the limit) must
    // still get its real core, so the decline is specific to this over-deep demand, not the node.
    if db.descent_depth >= crate::db::DESCENT_DEPTH_LIMIT {
        trace!(target: "rcdzc::lower", node = id.0, "lowering depth limit hit → decline (resource limit)");
        return Core::Poison(Reject::decline(
            "expression nests too deeply to compile (a recursion/resource limit was reached)",
        ));
    }
    db.descent_depth += 1;
    let c = compute(db, id);
    db.descent_depth -= 1;
    trace!(target: "rcdzc::lower", node = id.0, core = ?c, "lowered");
    db.core.fill(id, c.clone());
    c
}

/// Lower one node's resolved form to its core form. Records fold: a bare name is its bound value's
/// core, a `let` is its body's core, and a member projection is the FIELD'S core read directly — so a
/// record used only to read a field leaves no runtime trace (it folds to the projected scalar). A
/// record used as a runtime value survives as `Core::Record` (which declines at select until the
/// value heap exists). This is the one compile-time reduction tier acting through lowering
/// (`reference-compiler.md` §A Construct Whose Value Is Fully Determined At Compile Time).
/// Whether the core form at `id` reaches a `Core::HostCall` (directly or nested) — a bounded structural
/// walk over the core tree. Used by the `do`-sequencing lowering to decide whether a non-final statement
/// has a host-call side effect that must be emitted (rather than dropped by the ordinary `Ref{last}` fold).
fn subtree_reaches_host_call(db: &mut Db, id: StructId) -> bool {
    if matches!(core_of(db, id), Core::HostCall { .. }) {
        return true;
    }
    match db.ast.get(id).clone() {
        crate::ast::Struct::List(children) => {
            children.iter().any(|&c| subtree_reaches_host_call(db, c))
        }
        crate::ast::Struct::Atom(_) => false,
    }
}

fn compute(db: &mut Db, id: StructId) -> Core {
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
        let stmts: Vec<StructId> = stmts
            .iter()
            .copied()
            .filter(|&f| db.ast.head_name(f) != Some("def"))
            .collect();
        // Only build a Seq if some non-final statement reaches a host call (else the ordinary Ref{last}
        // fold is correct + cheaper). `subtree_reaches_host_call` walks the statement's core.
        if stmts.iter().any(|&s| subtree_reaches_host_call(db, s)) {
            return Core::Seq { stmts, tail };
        }
    }
    match resolved_of(db, id) {
        Resolved::Int(v) => Core::ConstInt(v),
        Resolved::Bool(b) => Core::ConstBool(b),
        Resolved::Str(s) => Core::ConstStr(s),
        // A symbol literal (`#"meter"`) shares the constant-string REP — its identity is its text — so it
        // lowers to `Core::ConstStr` exactly like a `Symbol.of` on a constant string. Only the static type
        // (`Ty::Symbol`) differs, so `=` folds via the shared constant-string equality.
        Resolved::SymbolConst(s) => Core::ConstStr(s),
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
            Core::BytesOf { elems }
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
        // the REST binder (`key = None`) folds to a `Core::MapNew` with the named keys removed. A runtime
        // scrutinee declines (the runtime key-directed matcher is a later increment). See `lower_map_field`.
        Resolved::MapField {
            scrutinee,
            key,
            named,
        } => lower_map_field(db, id, scrutinee, key, &named),
        // A FLOAT literal folds to its exact `Core::ConstFloat` — a `Ty::Float` value. This lets float
        // EQUALITY fold (two constants compared by canonical value). It still cannot cross the boundary
        // as a value or be an arithmetic operand (no f64 machine path yet) — those sites decline where
        // they consume it; the CONSTANT itself is now a real core value.
        Resolved::Float(d) => Core::ConstFloat(d),
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
        // annotation's force is entirely on inference; it has no runtime trace).
        Resolved::Annot { expr, .. } => core_of(db, expr),
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
                // scrutinee value directly (`(Mk n)` binds `n` to the whole erased value).
                let path = erase_nominal_steps(db, scrutinee, &steps, &heads);
                Core::SumPayload { scrutinee, path }
            }
        }
        // A `let` — A-NORMALIZE its bindings: a binding whose value is a runtime computation used more
        // than once is NAMED (a `Core::Let` binding, computed once, read by `LocalRef`); a single-use
        // or constant binding is copy-propagated / erased (its references follow through to its value).
        // So naming adds no cost and the emitted bytes are unchanged for a program with no multi-use
        // runtime binding (`reference-compiler.md` §The Core Representation Is In A-Normal Form).
        Resolved::Let { bindings, body } => lower_let(db, &bindings, body),
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
                        payloads: Vec::new(),
                    }
                }
                // A nullary NEWTYPE value (a bare single-variant nullary ctor, `(type Marker (The))` used
                // as `The`) — erased to its underlying Unit (no box, no disc). The node's type is
                // `Ty::Nominal { inner: Unit }`, which occupies no runtime slot, exactly as `Core::Unit`.
                crate::ty::Ty::Nominal { .. } => Core::Unit,
                // A payload variant used bare is a partial application (a function value).
                _ => Core::Poison(Reject::decline(
                    "a variant constructor with payloads must be applied to its arguments",
                )),
            }
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
                entries: Vec::new(),
                key_ty,
                val_ty,
            }
        }
        // A record value — kept as a compound; folds away only when a member reads a field of it.
        // `Resolved::Record.fields` and `Core::Record.fields` are BOTH `Arc<BTreeMap<…>>`, so SHARE the
        // map by an Arc clone (a refcount bump) — no O(fields) copy at all, and `Core::Record`'s own
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
            crate::eval::Member::Field(value) => core_of(db, value),
            // ANCHOR AT THE MEMBER NODE (`id`), symmetric with `infer::no_field_reject` (which stamps its
            // copy at the same member node): the ONE absent-field defect is reported by both the infer
            // check and this emit fold, and anchoring both at the member node lets `dedup_faults` collapse
            // them by (code, node). Without the explicit `.at(id)`, this poison reaches
            // `collect_reached_poisons` UNANCHORED and gets stamped at whatever ENCLOSING node it is reached
            // through — the redundant `((. r k))` apply wrapper, or an outer `(f (. r k))` call — a
            // DIFFERENT node than infer's member-node copy, so the two slip through as the SAME CDZ0201
            // printed twice. (A NESTED `(. (. r k) k)` still yields two, correctly: two DISTINCT member
            // nodes, each its own field read.)
            crate::eval::Member::NoField => Core::Poison(
                Reject::coded(
                    Code::Malformed,
                    format!("{}`{}`", crate::diag::NO_FIELD_PREFIX, key.name),
                )
                .at(id),
            ),
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
            Core::Tuple {
                elems: elems.to_vec(),
            }
        }
        // A list literal — a `Core::ListNew` the backend builds on the persistent `vec-*` heap. (Unlike a
        // tuple, a list has no projection-fold: `List.len`/`List.at` are operations, not a static index.)
        Resolved::List { elems } => {
            if let Some(r) = reduction_bound_element(db, &elems) {
                return Core::Poison(r);
            }
            Core::ListNew {
                elems: elems.to_vec(),
            }
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
                entries: entries.to_vec(),
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
                        // / `Tuple.pop`, which `lower_tuple_split_at`/`lower_tuple_pop` FOLD to a constant
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
                        trace!(target: "rcdzc::lower", node = id.0, operand = operand.0, index, "tuple projection stays runtime (operand is a runtime tuple)");
                        Core::Proj { operand, index }
                    }
                }
            }
        }
        // An `if`. FOLD when the condition reduces to a compile-time-constant boolean: the branch the
        // condition selects IS the result, so lower it directly and drop the `if`. This is dead-branch
        // elimination on a proven-constant condition — the untaken branch NEVER executes at run time.
        // ⚠ WHAT MAY BE DROPPED from the untaken branch mirrors the reachability model
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
                _ => Core::If { cond, then_, else_ },
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
        // A match over a scalar scrutinee — FOLD when the scrutinee is a constant (select the arm whose
        // probe it satisfies), else emit a `Core::Match` the backend lowers to a probe chain.
        Resolved::Match { scrutinee, arms } => lower_match(db, scrutinee, &arms),
        // `nan` — the canonical NaN Float VALUE (a bare prim naming a value, not an operation). Lowers to
        // `Core::ConstFloatNan`; folds in `=` by the canonical byte form.
        Resolved::Prim(Prim::FloatNan) => Core::ConstFloatNan,
        // A bare built-in operation value that is not applied has no runtime form yet (no closures) —
        // it declines. Applying it is what lowers.
        Resolved::Prim(_) => Core::Poison(Reject::decline(crate::diag::PRIM_AS_VALUE_DECLINE)),
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
                    trace!(target: "rcdzc::lower", node = id.0, %effect, %op, "apply: host-delegated perform → Core::HostCall");
                    return Core::HostCall {
                        effect,
                        op,
                        args: args.to_vec(),
                        result,
                    };
                }
                trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: unhandled perform at standalone lowering → decline (entrypoint check reports CDZ0401)");
                return Core::Poison(Reject::decline(crate::diag::NO_HOME_STANDALONE_DECLINE));
            }
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
                return lower_sum_new(db, ctor, &all_args);
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
                trace!(target: "rcdzc::lower", node = id.0, head = fn_head.0, n_args = all_args.len(), "apply: runtime closure application (spine-flattened) → Core::CallClosure");
                return Core::CallClosure {
                    closure: fn_head,
                    args: all_args,
                };
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
                            Ok(Some(reduced)) => core_of(g, reduced),
                            Ok(None) => unreachable!("lambda_body implies a lambda head"),
                            // The reduction declined. If it declined because the callee is RECURSIVE
                            // (can't inline to a normal form), emit a real `Core::Call` to it instead —
                            // provided the callee is a top-level def whose signature is DETERMINED
                            // (`def_scheme` — an annotated recursive def types by absorption, no fixpoint
                            // needed). An unannotated/undetermined callee still declines (its signature
                            // needs the connected solve, a later step). Any other decline propagates.
                            Err(msg) => lower_recursive_call_or_decline(g, head, &args, msg),
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
                    && crate::eval::variant_payload_type(db, head).is_some()
                {
                    trace!(target: "rcdzc::lower", node = id.0, head = head.0, "apply: unary variant ctor under-applied (CDZ0201)");
                    return Core::Poison(Reject::coded(
                        Code::Malformed,
                        "a variant constructor with a payload must be applied to its argument",
                    ));
                }
                // A RECURSIVE nullary call (`(def (f) (f))`) cannot fold to a normal form — following
                // the head would re-enter the same body without end. Decline it exactly as a recursive
                // parameterized call declines (a nullary def has no runtime-function form yet, so there
                // is no `Core::Call` to emit — it declines rather than diverging). `is_recursive` reads
                // the callee body reached through the nullary def's `Ref` (see `eval::callee_body`).
                if let Some(body) = crate::eval::lambda_body_of_nullary(db, head)
                    && crate::eval::is_recursive(db, body)
                {
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
            match crate::eval::meta_apply_of(db, head) {
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
                // A `+`/`-`/`*`/`/` over BIGINT operands — the unbounded arithmetic. A constant pair folds
                // exactly via `num-bigint` (the value never overflows — the point of the type); a runtime
                // operand emits the runtime `bigint-add`/`-sub`/`-mul`/`-div` (B3b). Checked before the
                // generic int-arith path (which would range-check/trap against a fixed width — wrong for an
                // unbounded BigInt). Dispatch on the OPERAND type being `Ty::BigInt`, like the float arm.
                Some(prim @ (Prim::Add | Prim::Sub | Prim::Mul | Prim::Div))
                    if args.len() == 2 && bigint_operand(db, &args) =>
                {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: BigInt arithmetic");
                    lower_bigint_arith(db, prim, args[0], args[1])
                }
                Some(prim) if prim.is_arith() => {
                    trace!(target: "rcdzc::lower", node = id.0, ?prim, "apply: arithmetic prim");
                    lower_arith(db, prim, &args)
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
                    lower_sum_new(db, head, &args)
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
                // `Record.extend r (z v)` / `Record.with r (z v)` — both INSERT field `z ↦ v` into a
                // constant `Core::Record` (extend adds an absent field, with replaces a present one; the
                // presence/absence CDZ0211/0212 is `infer`'s, so the fold is the same insert). The `(z v)`
                // pair's value occurrence carries into the field.
                Some(Prim::RecordExtend | Prim::RecordWith) if args.len() == 2 => {
                    lower_record_insert(db, id, args[0], args[1])
                }
                // `Record.pop r z` — `(tuple (. r z) (r without z))`: the popped field's value paired with
                // the remaining record. Folds a constant `Core::Record` to a `Core::Tuple`.
                Some(Prim::RecordPop) if args.len() == 2 => {
                    lower_record_pop(db, id, args[0], args[1])
                }
                // `Tuple.cat a b` — concatenate two constant `Core::Tuple`s (elements of `a` then `b`).
                Some(Prim::TupleCat) if args.len() == 2 => {
                    lower_tuple_cat(db, id, args[0], args[1])
                }
                // `Tuple.split-at t k` — `(tuple <prefix> <suffix>)` at compile-time literal `k`.
                Some(Prim::TupleSplitAt) if args.len() == 2 => {
                    lower_tuple_split_at(db, id, args[0], args[1])
                }
                // `Tuple.pop t` — `(tuple (. t 0) <rest>)`.
                Some(Prim::TuplePop) if args.len() == 1 => lower_tuple_pop(db, id, args[0]),
                // `vec-len`). One operand: the list.
                Some(Prim::ListLen) if args.len() == 1 => {
                    let operand = args[0];
                    match core_of(db, operand) {
                        Core::ListNew { elems } => {
                            trace!(target: "rcdzc::fold", node = id.0, len = elems.len(), "List.len folds to a constant (visible list literal)");
                            Core::ConstInt(IntValue::from_i64(elems.len() as i64))
                        }
                        Core::Poison(r) => Core::Poison(r),
                        _ => Core::ListLen { operand },
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
                        (Core::ListNew { elems: mut a }, _) => {
                            a.push(args[1]);
                            trace!(target: "rcdzc::fold", node = id.0, len = a.len(), "List.push folds onto a constant list");
                            Core::ListNew { elems: a }
                        }
                        // A runtime list — the persistent `vec-push` on the heap.
                        _ => Core::ListPush {
                            list: args[0],
                            elem: args[1],
                        },
                    }
                }
                Some(Prim::ListConcat) if args.len() == 2 => {
                    match (core_of(db, args[0]), core_of(db, args[1])) {
                        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
                        // FOLD two compile-time-visible list literals into ONE merged `Core::ListNew`
                        // (the elements of the left followed by those of the right) — a constant list
                        // that bakes at escape / folds through `List.at`/`len`, exactly as a written
                        // `(list …)` does. `List Int64` concat `List Int64` → `List Int64`; the element
                        // occurrences carry over unchanged (they keep their own types).
                        (Core::ListNew { elems: mut a }, Core::ListNew { elems: b }) => {
                            a.extend(b);
                            trace!(target: "rcdzc::fold", node = id.0, len = a.len(), "List.concat folds two constant lists");
                            Core::ListNew { elems: a }
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
                        (Core::ListNew { elems: mut a }, Core::ConstInt(i), _) => {
                            match i.to_i64() {
                                Some(n) if n >= 0 && (n as usize) < a.len() => {
                                    a[n as usize] = args[2];
                                    trace!(target: "rcdzc::fold", node = id.0, index = n, "List.update folds (in-range constant index)");
                                    Core::ListNew { elems: a }
                                }
                                _ => {
                                    trace!(target: "rcdzc::fold", node = id.0, "List.update out-of-range constant index → CDZ0304");
                                    Core::Poison(Reject::coded(
                                        Code::ConstTrap,
                                        "List.update index is out of bounds (a constant out-of-range update traps)",
                                    ))
                                }
                            }
                        }
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
                        // A RUNTIME string: its `byte-len` is the byte count of its underlying leaf. A
                        // runtime String value IS a flat UTF-8 byte leaf (an i32 heap handle — the same rep
                        // `str-new` would give, built via `bytes-alloc`/`bytes-set`), so its byte length is
                        // exactly `bytes-len` over that handle — `Core::BytesLen`, the runtime op already
                        // working for `Bytes.len`. (`scalar-len` is the UTF-8 SCALAR count, which needs a
                        // decoding walk over the bytes, not a leaf-length read, so it still declines here.)
                        _ if matches!(prim, Prim::StrByteLen)
                            && matches!(
                                crate::infer::type_of(db, args[0]),
                                crate::ty::Ty::String
                            ) =>
                        {
                            trace!(target: "rcdzc::lower", node = id.0, "String.byte-len on a runtime string → bytes-len over its byte leaf");
                            Core::BytesLen { operand: args[0] }
                        }
                        _ => Core::Poison(Reject::decline(
                            "a runtime string's scalar length needs a UTF-8 decoding walk (not yet built; byte-len works)",
                        )),
                    }
                }
                // `Char.to-int` — the TOTAL scalar-value read `Char → Int64`. FOLD a constant char to a
                // `Core::ConstInt` of its Unicode scalar value (`c as u32`). A runtime char has no machine
                // rep this increment, so a non-constant operand declines.
                Some(Prim::CharToInt) if args.len() == 1 => match core_of(db, args[0]) {
                    Core::ConstChar(c) => {
                        trace!(target: "rcdzc::fold", node = id.0, "Char.to-int folds to the scalar value");
                        Core::ConstInt(IntValue::from_i64(c as u32 as i64))
                    }
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::Poison(Reject::decline(
                        "Char.to-int on a runtime char is not yet computed (constant chars only)",
                    )),
                },
                // `Char.from-int` — the FALLIBLE conversion `Int64 → (Option Char)`. FOLD a constant int
                // to `(Some #\c)` when it is a Unicode scalar value, `(None unit)` for a surrogate /
                // out-of-range integer (`collections-and-text.md` §A Char Converts To And From An Integer
                // Totally). Never traps.
                Some(Prim::CharFromInt) if args.len() == 1 => lower_char_from_int(db, id, args[0]),
                // `Symbol.of` — intern a String into a Symbol (`String → Symbol`). A CONSTANT string folds
                // to a constant symbol, which shares the underlying `Core::ConstStr` REP (identity is
                // content-derived); only the static TYPE differs (`Ty::Symbol`, off this node's solved
                // type). So the fold is the identity on the `ConstStr` — `(= (Symbol.of "a") (Symbol.of
                // "a"))` then folds via `const_compound_eq(ConstStr, ConstStr)`. A runtime string interns
                // at run time (a later increment) — declines here.
                Some(Prim::SymbolOf) if args.len() == 1 => match core_of(db, args[0]) {
                    c @ Core::ConstStr(_) => c,
                    Core::Poison(r) => Core::Poison(r),
                    _ => runtime_string_op_decline(
                        db,
                        args[0],
                        "Symbol.of on a runtime string is not yet interned (constant strings only)",
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
                // `Symbol.to-string` — recover a Symbol's content String (`Symbol → String`, the inverse of
                // `Symbol.of`). A constant symbol IS its `Core::ConstStr`, so this folds to that same node
                // retyped `String` (the node's solved type); the rep is unchanged.
                Some(Prim::SymbolToString) if args.len() == 1 => match core_of(db, args[0]) {
                    c @ Core::ConstStr(_) => c,
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::Poison(Reject::decline(
                        "Symbol.to-string on a runtime symbol is not yet computed (constant symbols only)",
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
                Some(prim @ (Prim::CheckedAdd | Prim::CheckedMul)) if args.len() == 2 => {
                    lower_checked_arith(db, id, prim, args[0], args[1])
                }
                // `Int64.wrapping-add` / `wrapping-mul` — two's-complement wraparound, NEVER trapping. FOLD
                // a constant pair via `wrapping_*`; a runtime operand emits `Core::Arith` (which for a
                // wrapping prim selects the RAW machine op, no overflow guard).
                Some(prim @ (Prim::WrappingAdd | Prim::WrappingMul)) if args.len() == 2 => {
                    lower_wrapping_arith(db, prim, args[0], args[1])
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
                            Core::ConstStr(format!("{a}{b}"))
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
                            trace!(target: "rcdzc::lower", node = id.0, "String.concat on runtime strings → bytes-concat over their byte leaves");
                            Core::BytesConcat {
                                lhs: args[0],
                                rhs: args[1],
                            }
                        }
                        _ => Core::Poison(Reject::decline(
                            "a string concatenation is only folded for constant ASCII operands (the \
                             normalizing byte-rope join arrives with the runtime string heap)",
                        )),
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
                // `Set.of` — construct a set from a LIST of elements (dedup). Emit `Core::SetOf` carrying
                // the element occurrences + the solved element type; a constant list folds to a canonical
                // set. `Set.contains`/`len`/`insert`/`remove` + the algebra ops each lower to their runtime
                // `Core::Set*` (folding a constant operand). The element type comes from the RESULT node's
                // solved `Ty::Set` (fully determined by unification, even for a bare `(Set.of (list))`).
                Some(Prim::SetOf) if args.len() == 1 => lower_set_of(db, id, args[0]),
                Some(Prim::SetContains) if args.len() == 2 => {
                    lower_set_contains(db, args[0], args[1])
                }
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
                            Core::Poison(Reject::decline(format!(
                                "{named} is applied at the wrong arity — a built-in operation must be \
                                 applied to exactly its arguments (a partial application, which would need \
                                 a runtime closure, is not yet built)"
                            )))
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
                        None => Core::Poison(Reject::decline(
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
        Resolved::Resume { .. } => Core::Poison(Reject::decline(
            "resume outside a lowered handler arm is not yet realized",
        )),
    }
}

/// A-normalize a `let`: decide, per binding, whether to NAME its value (keep it as a `Core::Let`
/// binding computed once) or to COPY-PROPAGATE / erase it (let each reference follow through to the
/// value's core). A binding is KEPT iff its value is a runtime computation (not a compile-time
/// constant that folds away) AND its name is referenced MORE THAN ONCE in what follows — the case
/// where following through would recompute the value at each use. Every other binding — used at most
/// once, or constant — is propagated, the admin-redex elimination that keeps naming free
/// (`reference-compiler.md` §The Core Representation Is In A-Normal Form ¶3), so a program with no
/// multi-use runtime binding lowers exactly as before and its emitted bytes are unchanged.
///
/// The kept bindings are recorded in `db.kept_bindings` (keyed by the initializer occurrence a
/// reference resolves to) BEFORE the body is lowered, so a `Resolved::Ref` to a kept binding lowers
/// to a `Core::LocalRef` reading the shared slot. The result is a `Core::Let { bindings, body }` when
/// any binding is kept, or just the body's core when none is (no residual `let`).
fn lower_let(db: &mut Db, bindings: &[(StructId, StructId)], body: StructId) -> Core {
    // The `(binder-name-occ, init-occ)` pairs; a reference resolves to the INIT occurrence, so that is
    // what the body's `Ref`s point at and what a kept binding is keyed by.
    let mut kept: Vec<(StructId, StructId)> = Vec::new();
    for (k, &(_name_occ, init)) in bindings.iter().enumerate() {
        // A binding's SCOPE (the positions its name is visible in) is the LATER sibling initializers
        // plus the body — `let*` sequential scoping. Count uses across that whole continuation so a
        // binding referenced only by a later initializer is still named.
        let mut continuation: Vec<StructId> = bindings[k + 1..].iter().map(|&(_, v)| v).collect();
        continuation.push(body);
        if should_keep_binding(db, init, &continuation) {
            // Record the keep BEFORE lowering the body/later inits — their references to this init read
            // `db.kept_bindings` to decide `LocalRef` vs follow-through.
            db.kept_bindings.insert(init);
            kept.push((init, init));
        }
    }
    // The body's core (its references to kept bindings now lower to `LocalRef`).
    if kept.is_empty() {
        // Nothing named — the ordinary erase: the `let`'s value is its body's value.
        return core_of(db, body);
    }
    trace!(target: "rcdzc::lower", body = body.0, kept = kept.len(), "let: A-normalized (named multi-use runtime bindings)");
    Core::Let {
        bindings: kept,
        body,
    }
}

/// The non-exhaustiveness fault of the match form `id`, if it has one — for the WELL-FORMEDNESS pass
/// (`compile::collect_faults` via `infer::collect_node`) to surface a CDZ0210 over EVERY match, not only
/// the ones the emit path lowers. `cdz check` runs `type_errors` on every def body but the reached-poison
/// (lowering) walk only on nullary EXPORTED bodies, so a non-exhaustive match on a function PARAMETER
/// (`(def (f (: c Color)) (match c …))`) — the common case — was silently missed by `check`/`--json`/`fix`
/// and an UNCALLED function's non-exhaustive match escaped emission entirely (dead, never laid out). This
/// closes both gaps by lowering just the match: exhaustiveness is a STRUCTURAL, value-independent verdict
/// (`build_tree` decides it from the scrutinee's TYPE and the arm patterns, before any constant fold), so
/// an unsubstituted parameter scrutinee gives the correct answer.
///
/// Returns ONLY a `Code::NonExhaustive` reject. A DECLINE (a not-yet-lowerable match — a runtime list, an
/// unsupported nested pattern) is dropped: those are not reported by `check` today, and surfacing them
/// here from a standalone (un-β-reduced) lowering would raise false alarms a call-site fold resolves. Any
/// OTHER coded reject (a shape/type fault in a pattern, CDZ0201/0203) is already produced by the
/// surrounding `collect_node` walk, so it is not re-raised here. SAFE to call during `check`: β-reduction
/// at a call site copies the callee body into FRESH nodes (`eval::beta_reduce`), so it never reads this
/// match node's own memoized core — pre-filling the slot for the unsubstituted body cannot corrupt an
/// emitted call site.
pub fn match_nonexhaustive_fault(db: &mut Db, id: StructId) -> Option<Reject> {
    match core_of(db, id) {
        Core::Poison(r) if r.code == Some(Code::NonExhaustive) => Some(r),
        _ => None,
    }
}

/// The CODED pattern-well-formedness fault of a `(match …)` — a mistyped variant pattern head
/// (`((C.Gren) …)` on `(type C Red Green)` → CDZ0201 "record has no field `Gren` — did you mean `Green`?"
/// carrying a replace fix on the key), a foreign-sum variant (CDZ0203), a payload-arity mismatch, or a
/// non-linear binder (CDZ0102). Surfaced for `type_errors` so `cdz check` catches it in EVERY body, not
/// only the nullary-EXPORTED ones the emit-path lowering walk (`collect_reached_poisons`) reaches — the
/// same "check missed what only lowering produced" hole `match_nonexhaustive_fault` closes for
/// exhaustiveness. Like a mistyped variant in VALUE position, a variant typo in a pattern is a
/// well-formedness fault independent of the function's parameter values, so surfacing it over an
/// unreached parameterized body is not a false alarm (the scrutinee's TYPE — the source of the variant
/// set — comes from its annotation, not a runtime value).
///
/// Returns the poison ONLY when it is a CODED pattern fault that is NOT the non-exhaustiveness CDZ0210
/// (which `match_nonexhaustive_fault` already reports — filter it here to avoid a double report). A
/// not-yet-lowerable DECLINE (an unbuilt compound scrutinee, an unsolved-`Any` scrutinee type) is
/// UNCODED, so it yields `None` and this adds no false alarm — the exact conservatism the exhaustiveness
/// accessor uses.
pub fn match_pattern_fault(db: &mut Db, id: StructId) -> Option<Reject> {
    match core_of(db, id) {
        Core::Poison(r) if r.code.is_some() && r.code != Some(Code::NonExhaustive) => Some(r),
        _ => None,
    }
}

/// Lower a `(match scrutinee (pattern body)…)` over a SCALAR scrutinee. Each pattern classifies to a
/// [`Probe`] (an integer/boolean literal, a binder, or the wildcard `_`); a pattern that is none of
/// these declines (sum/tuple/record patterns walk the value heap — a separate path). If the scrutinee
/// FOLDS to a constant, select the first arm whose probe it satisfies and lower THAT arm's body (no
/// runtime match — like the const `if` fold). Otherwise the scrutinee is a runtime scalar: emit a
/// `Core::Match` the backend lowers to a probe chain.
///
//= spec/capabilities/core-semantics.md#matching-is-exhaustive-or-rejected
//# A match whose patterns do not cover every value of the scrutinee's type MUST be a compile-time error.
///
/// The arms are tried TOP-TO-BOTTOM: a constant scrutinee selects the FIRST arm whose probe it satisfies
/// and lowers only that arm's body, and a runtime scrutinee emits a probe chain that takes the first
/// matching arm — first-match-wins, as the corpus defines.
//= spec/capabilities/core-semantics.md#matching-is-exhaustive-or-rejected
//# A match MUST evaluate the branch of the first pattern that matches the scrutinee, as defined by the corpus.
///
/// A wildcard/binder tail covers the rest, and for an OPEN type (an integer) it is the only cover — no
/// finite literal set exhausts the integers. A FINITE type is exhausted by its literals instead: a Bool
/// scrutinee covered by both a `true` arm and a `false` arm needs no wildcard. A match that covers
/// neither way is rejected (CDZ0210), not compiled to a fallthrough with no defined value.
/// Whether `ty` is the EMPTY SUM — a `Ty::Sum` whose declaration has ZERO variants (an uninhabited
/// `Void`/`Never`, `type-system.md §Never Is The Empty Sum`). Such a type has no value, so a zero-arm
/// match on it is vacuously exhaustive. Reads the sum's declaration by its `decl` occurrence and checks
/// the variant count; a non-sum (or a sum with variants) is `false`.
fn is_empty_sum_ty(db: &mut Db, ty: &crate::ty::Ty) -> bool {
    if let crate::ty::Ty::Sum { decl, .. } = ty.strip_nominal() {
        return db
            .type_decl_by_occ(*decl)
            .is_some_and(|d| d.variants.is_empty());
    }
    false
}

fn lower_match(db: &mut Db, scrutinee: StructId, arms: &[(StructId, StructId)]) -> Core {
    // A ZERO-ARM match is the DEGENERATE base case of exhaustiveness: it is well-formed ONLY when the
    // scrutinee is UNINHABITED (`Never` — a diverging expression), for which no arm is needed to cover
    // every variant, there being none (`type-system.md §Never Is The Empty Sum`, 4th sentence). The
    // scrutinee still evaluates (its divergence IS the match's outcome), so lower it: a scrutinee that
    // provably DIVERGES lowers to `Core::Trap` / a poison — return that (the match diverges through the
    // scrutinee, exactly as `(match (never-returns))` traps). A zero-arm match on an INHABITED scrutinee
    // is genuinely non-exhaustive (`Code::NonExhaustive`), NOT the malformed "no arms" it was before.
    if arms.is_empty() {
        // The scrutinee's TYPE is the EMPTY SUM (zero variants) — an uninhabited `Void`/`Never`, e.g. a
        // parameter `(: v Void)`. `type-system.md §Never Is The Empty Sum` (4th sentence): "A match on a
        // scrutinee of the empty sum type MUST be exhaustive with zero arms." No value of that type can
        // exist, so the match is unreachable: emit `Core::Trap` (the scrutinee still evaluates — reading
        // it is itself the divergence — but no arm is needed). Distinct from the diverging-EXPRESSION case
        // below (a scrutinee that folds to a trap): here the scrutinee's static TYPE proves uninhabited.
        let scrut_ty = crate::infer::type_of(db, scrutinee);
        if is_empty_sum_ty(db, &scrut_ty) {
            return Core::Trap;
        }
        return match core_of(db, scrutinee) {
            c @ (Core::Trap | Core::Poison(_)) => c,
            _ => Core::Poison(Reject::coded(
                Code::NonExhaustive,
                "a zero-arm match is exhaustive only on an uninhabited (Never) scrutinee; this scrutinee has values a case must cover",
            )),
        };
    }
    // A COMPOUND scrutinee — a SUM, a TUPLE, or a RECORD — is matched by the DECISION TREE, not the
    // scalar-probe path. A sum dispatches on the discriminant; a tuple has no discriminant, so its match
    // is a chain of `Elem`-path binders / literal tests; a RECORD has neither a discriminant NOR a
    // sanctioned destructuring pattern (a record is read by `(. r field)` projection, not pattern-matched
    // field-by-field — `core-semantics.md §Patterns Compose` lists tuple + constructor patterns, NOT
    // record patterns), so a record match's only patterns are a bare BINDER (binds the whole record) or a
    // WILDCARD — a degenerate match the tree folds to the first covering arm. All go through
    // `lower_match_sum` (the shared decision-tree builder); a scalar scrutinee falls through to the
    // scalar-probe path below.
    if let crate::ty::Ty::Sum { .. }
    | crate::ty::Ty::Nominal { .. }
    | crate::ty::Ty::Tuple(_)
    | crate::ty::Ty::Record(_) = crate::infer::type_of(db, scrutinee)
    {
        return lower_match_sum(db, scrutinee, arms);
    }
    // A BYTES scrutinee whose arms use `(bin …)` binary patterns → the binary matcher (BN3, const
    // scrutinee). A scalar-only match over Bytes (only bare-binder/`_` arms) is NOT here — it has no
    // `(bin …)` arm, so it falls through to the scalar path (a whole-value binder / wildcard).
    if matches!(crate::infer::type_of(db, scrutinee), crate::ty::Ty::Bytes)
        && arms
            .iter()
            .any(|&(pat, _)| db.ast.head_name(pat) == Some("bin"))
    {
        return lower_match_bin(db, scrutinee, arms);
    }
    // A LIST scrutinee is deconstructed by ELEMENT patterns (`core-semantics.md` §A List Is Deconstructed
    // By Element Patterns With An Optional Rest). This increment folds a CONSTANT list (`Core::ListNew`)
    // against FIXED-ARITY patterns `(list a b)` / `(list)`: the known length selects the matching arm; its
    // element binders read the constant elements via `SumPayload` `Elem` folds. A REST pattern
    // `(list x .. rest)` or a RUNTIME list scrutinee declines (later increments).
    if matches!(crate::infer::type_of(db, scrutinee), crate::ty::Ty::List(_)) {
        return lower_match_list(db, scrutinee, arms);
    }
    // A MAP scrutinee whose arms use `(map …)` key-directed patterns → the map matcher (ask-61). A map
    // pattern `(map (k p) … .. rest)` matches when the map HAS key `k` (bound to a value matching `p`),
    // binding `rest` to the remaining map — a QUERY, not a structural shape. This increment folds a
    // CONSTANT `Core::MapNew` scrutinee (the corpus shape). A scalar-only match over a map (only bare-
    // binder/`_` arms — no `(map …)` pattern) falls through to the scalar path (a whole-value binder).
    if matches!(
        crate::infer::type_of(db, scrutinee),
        crate::ty::Ty::Map(_, _)
    ) && arms.iter().any(|&(pat, _)| {
        db.ast.head_ctor(pat) == Some("map") || db.ast.head_name(pat) == Some("map")
    }) {
        return lower_match_map(db, scrutinee, arms);
    }
    // Classify each arm into a probe + optional GUARD + body. An arm's pattern may be a GUARDED pattern
    // `(guard <inner-pat> <cond>)` — the inner pattern gives the probe, `<cond>` the guard (a boolean the
    // arm's binder is in scope for, resolve Case 5). A pattern that is not a scalar literal, binder,
    // wildcard, or such a guarded pattern declines the whole match (a compound needs a heap walk).
    let mut probes: Vec<(crate::core::Probe, Option<StructId>, StructId)> = Vec::new();
    for &(pat, body) in arms {
        let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
            // `(guard <inner-pat> <cond>)` — a guarded pattern.
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            Some(_) => {
                return Core::Poison(Reject::coded(
                    Code::Malformed,
                    "a guarded pattern must be (guard <pattern> <cond>)",
                ));
            }
            None => (pat, None),
        };
        match classify_probe(db, inner_pat) {
            Some(p) => probes.push((p, guard, body)),
            None => {
                return Core::Poison(Reject::decline(
                    "a match pattern that is not a scalar literal or `_` is not yet supported",
                ));
            }
        }
    }
    // WELL-FORMEDNESS (checked STRUCTURALLY, before any fold — a constant scrutinee does not excuse a
    // type-mismatched pattern or a non-exhaustive match; a match is well-formed or not regardless of
    // what the scrutinee happens to be):
    let scrut_ty = crate::infer::type_of(db, scrutinee);
    //  (1) each LITERAL pattern's type must agree with the scrutinee's — a bool pattern against an
    //      integer scrutinee (or vice-versa) is a shape/type error (CDZ0201), not a never-matching arm.
    for (probe, _, _) in &probes {
        let pat_ty = match probe {
            crate::core::Probe::Int(_) => Some(crate::ty::Ty::int()),
            crate::core::Probe::Bool(_) => Some(crate::ty::Ty::Bool),
            crate::core::Probe::Str(_) => Some(crate::ty::Ty::String),
            // A `ListLen` probe never arises in the SCALAR match path (it comes from a list PAYLOAD
            // sub-pattern in the sum decision tree, not `classify_probe`); no scalar-type check applies.
            crate::core::Probe::ListLen { .. } => None,
            crate::core::Probe::Wild => None,
        };
        if let Some(pt) = pat_ty
            && !pt.agrees_with(&scrut_ty)
        {
            return Core::Poison(Reject::coded(
                Code::Malformed,
                format!(
                    "match pattern type {} does not match scrutinee type {}",
                    pt.render_name(),
                    scrut_ty.render_name()
                ),
            ));
        }
    }
    //  (2) exhaustiveness: a scalar match must cover every value of the scrutinee's type. A wildcard
    //      tail covers the rest, and for an OPEN type (Int64) that is the ONLY way — no finite literal
    //      set exhausts the integers. But a FINITE type is exhausted by its literals: a Bool scrutinee
    //      covered by BOTH a `true` arm and a `false` arm needs no wildcard (the two values are the
    //      whole type — `core-semantics.md` §Matching Is Exhaustive Or Rejected). This holds EVEN when
    //      the scrutinee is a constant that hits an arm — well-formedness is independent of the value.
    //      A GUARDED arm does NOT count toward exhaustiveness — its guard may be false, so it covers no
    //      value unconditionally (`core-semantics.md` §Matching Is Exhaustive Or Rejected: "A guard does
    //      NOT count toward exhaustiveness"). So only UNGUARDED arms contribute coverage below.
    let has_wild = probes
        .iter()
        .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Wild));
    // A Bool scrutinee's two literals exhaust it. (A definitely-Bool or still-open `Any` scrutinee whose
    // arms are Bool literals — a bare parameter matched with `true`/`false` — is matching over Bool; a
    // definitely-Int scrutinee with a Bool probe already faulted in step (1) and never reaches here.)
    let bool_true = probes
        .iter()
        .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Bool(true)));
    let bool_false = probes
        .iter()
        .any(|(p, g, _)| g.is_none() && matches!(p, crate::core::Probe::Bool(false)));
    let bool_exhaustive = scrut_ty.agrees_with(&crate::ty::Ty::Bool) && bool_true && bool_false;
    if !has_wild && !bool_exhaustive {
        // Name what is uncovered + carry an "add the covering arm" fix (the missing bool literal, or a
        // wildcard for an open scalar) — the scalar twin of the sum add-arms fix.
        return Core::Poison(non_exhaustive_scalar_reject(
            db, scrutinee, &scrut_ty, bool_true, bool_false,
        ));
    }

    // Well-formed. FOLD if the scrutinee is a compile-time constant: select the first arm whose probe
    // it satisfies AND whose guard (if any) folds to `true` (no runtime match, like the const `if` fold).
    // A guard is folded via `core_of` — the arm's binder resolves to the constant scrutinee (Case 5), so
    // `(< x 0)` over a constant `x` folds to a `ConstBool`. If a matched arm's guard does NOT fold to a
    // constant bool (its guard reads a runtime value), the fold ABORTS to the runtime probe chain (we
    // cannot decide the arm at compile time). A guard that folds `false` skips the arm to the next.
    let scrut_core = core_of(db, scrutinee);
    if let Core::Poison(r) = scrut_core {
        return Core::Poison(r);
    }
    let const_scrut = match &scrut_core {
        Core::ConstInt(v) => Some(GuardFoldScrut::Int(v.clone())),
        Core::ConstBool(b) => Some(GuardFoldScrut::Bool(*b)),
        Core::ConstStr(s) => Some(GuardFoldScrut::Str(s.clone())),
        _ => None,
    };
    if let Some(sc) = const_scrut {
        let mut foldable = true;
        for (probe, guard, body) in &probes {
            let probe_hit = match &sc {
                GuardFoldScrut::Int(v) => probe_matches_int(probe, v),
                GuardFoldScrut::Bool(b) => probe_matches_bool(probe, *b),
                GuardFoldScrut::Str(s) => probe_matches_str(probe, s),
            };
            if !probe_hit {
                continue; // this arm's pattern doesn't match the constant — try the next
            }
            match guard {
                None => {
                    trace!(target: "rcdzc::fold", "match folds to a selected arm (constant scrutinee)");
                    return core_of(db, *body);
                }
                Some(g) => match core_of(db, *g) {
                    Core::ConstBool(true) => {
                        trace!(target: "rcdzc::fold", "match folds to a guarded arm (guard holds over a constant)");
                        return core_of(db, *body);
                    }
                    Core::ConstBool(false) => continue, // guard fails → fall through to the next arm
                    _ => {
                        // The guard did not fold to a constant bool (it reads a runtime value). We cannot
                        // decide this arm at compile time even though the scrutinee is constant → abort
                        // the fold and emit the runtime probe chain below.
                        foldable = false;
                        break;
                    }
                },
            }
        }
        if foldable {
            // Every matched arm's guard folded false and no unguarded arm covered — unreachable, because
            // exhaustiveness requires an unguarded wildcard/literal cover (checked above).
            return Core::Poison(Reject::decline(
                "match: no arm matched a constant (unreachable)",
            ));
        }
    }
    // Runtime scalar scrutinee — it must BE a scalar (a compound needs a heap walk, later).
    if !is_scalar(db, scrutinee) {
        return Core::Poison(Reject::decline(
            "matching a compound value needs a heap walk (not yet built)",
        ));
    }
    // ALL-SAME-BODY COLLAPSE: if every arm is UNGUARDED and all their bodies lower to the SAME core, the
    // match computes that value for every scrutinee — so it collapses to the body, dropping the probe
    // chain (the match analogue of `(if c x x)` → `x`). Guarded arms are excluded: a guard may fail, so
    // its arm does not unconditionally yield its body — the choice is then observable and the chain must
    // stay. Sound ONLY when the scrutinee is TRAP-FREE: the discriminant is otherwise unused after the
    // collapse, but the scrutinee was evaluated to drive the (now-gone) probes, so a scrutinee that could
    // trap must still be evaluated (keep the chain). `core_equiv` is the same conservative pure-core
    // equality the `if`-identical-branches fold uses; a binder arm's body `core_of` reads the scrutinee
    // (Case 5), so `(match a (n n))`'s arm equals `core_of(a)` and only collapses when every arm agrees.
    if probes.iter().all(|(_, guard, _)| guard.is_none())
        && let Some((_, _, first_body)) = probes.first()
        && probes[1..]
            .iter()
            .all(|(_, _, body)| core_equiv(db, *body, *first_body))
        && is_trap_free(db, scrutinee)
    {
        trace!(target: "rcdzc::fold", scrutinee = scrutinee.0, "match with all arms yielding the same value collapses to the body (trap-free scrutinee)");
        return core_of(db, *first_body);
    }
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, arms = probes.len(), "match stays runtime (scalar scrutinee → probe chain)");
    Core::Match {
        scrutinee,
        arms: probes
            .into_iter()
            .map(|(probe, guard, body)| crate::core::MatchArm { probe, guard, body })
            .collect(),
    }
}

/// Lower a `(match scrutinee (list-pattern body)…)` over a LIST scrutinee — this increment folds a
/// COMPILE-TIME-CONSTANT scrutinee against FIXED-ARITY element patterns. Its `Core::ListNew` gives the
/// length; select the FIRST arm whose pattern matches (a `(list p0 … pk)` of arity k matches length k; a
/// bare binder / `_` matches any) and lower that arm's body — the body's element binders resolve to
/// `SumPayload` `Elem(i)` reads that FOLD against the constant list (resolve Case 6l). A REST pattern
/// `(list x .. rest)` or a RUNTIME list scrutinee declines (later increments). A well-formed match must
/// cover every length — a bare binder / `_` catch-all — else CDZ0210.
fn lower_match_list(db: &mut Db, scrutinee: StructId, arms: &[(StructId, StructId)]) -> Core {
    enum Arm {
        Fixed(usize, StructId), // a fixed-arity `(list …)` of this exact arity
        Rest(usize, StructId), // a rest `(list p0 … p_{k-1} .. rest)` — matches length ≥ k (lead = k)
        Wild(StructId),        // a bare binder / `_` — matches any length
    }
    let mut classified: Vec<Arm> = Vec::with_capacity(arms.len());
    for &(pat, body) in arms {
        if db.ast.as_name(pat).is_some() {
            classified.push(Arm::Wild(body));
            continue;
        }
        match db
            .ast
            .as_ctor_form(pat, "list")
            .or_else(|| db.ast.as_form(pat, "list"))
        {
            Some(es) => {
                // Split at a `..` marker: `lead` leading binders, then (for a rest pattern) the rest
                // binder name. A rest pattern needs EXACTLY one binder after `..`.
                match es.iter().position(|&e| db.ast.as_name(e) == Some("..")) {
                    Some(i) => {
                        if i + 2 != es.len() {
                            return Core::Poison(Reject::coded(
                                Code::Malformed,
                                "a list rest pattern is `(list p… .. rest)` — exactly one binder after `..`",
                            ));
                        }
                        // Each leading element must be a bare name binder / `_` (a nested/literal element
                        // pattern is a later increment). The leading binders read via `SumPayload`; the
                        // rest binder is a sublist (bound in the body — resolve's list-pattern case; over a
                        // constant scrutinee, the tail folds when referenced).
                        if es[..i].iter().all(|&e| db.ast.as_name(e).is_some()) {
                            classified.push(Arm::Rest(i, body));
                        } else {
                            return Core::Poison(Reject::decline(
                                "a list element sub-pattern that is not a binder is not yet supported",
                            ));
                        }
                    }
                    None => {
                        // Fixed arity: each element must be a bare name binder / `_`.
                        if es.iter().all(|&e| db.ast.as_name(e).is_some()) {
                            classified.push(Arm::Fixed(es.len(), body));
                        } else {
                            return Core::Poison(Reject::decline(
                                "a list element sub-pattern that is not a binder is not yet supported",
                            ));
                        }
                    }
                }
            }
            None => {
                return Core::Poison(Reject::decline(
                    "a list match arm that is not an element pattern or a binder is not yet supported",
                ));
            }
        }
    }
    // WELL-FORMEDNESS: a list is OPEN (any length), so the arms must JOINTLY cover every length n ≥ 0.
    // A `Wild` / `Rest(k)` covers the infinite tail [k, ∞) (a `Wild` = `Rest(0)`); a `Fixed(k)` covers the
    // single length {k}. Let `m` be the SMALLEST tail-start among all catch-all arms (0 if any `Wild`).
    // If there is no `Wild`/`Rest` arm at all, no arm covers the infinite tail → non-exhaustive. Otherwise
    // lengths [m, ∞) are covered by that arm; the finite prefix 0..m must be covered by `Fixed` arms (no
    // `Rest(j)` with j < m exists, since m is the minimum). Else CDZ0210.
    let tail_start = classified.iter().filter_map(|a| match a {
        Arm::Wild(_) => Some(0),
        Arm::Rest(k, _) => Some(*k),
        Arm::Fixed(_, _) => None,
    });
    let Some(m) = tail_start.min() else {
        // NO catch-all arm → no arm covers the infinite tail. The mechanical repair is a WILDCARD `_` arm
        // (covers every remaining length), the list analogue of the scalar add-wildcard fix — bodied with
        // a diverging `(trap "TODO")` so it type-checks whatever the other arms return. Anchored at the
        // `(match …)` form (parent of the scrutinee); no parent → the bare reject.
        let reject = Reject::coded(
            Code::NonExhaustive,
            "a list match must cover every length (end in a `_`, a whole-list binder, or a `(list .. rest)` arm)",
        );
        return Core::Poison(match db.parent_of(scrutinee) {
            Some(match_form) => reject.with_fix(Fix::insert_arms_heuristic(
                match_form,
                vec!["(_ (trap \"TODO\"))".to_string()],
            )),
            None => reject,
        });
    };
    // Every length in 0..m must have a matching `Fixed` arm.
    let missing: Vec<usize> = (0..m)
        .filter(|&n| {
            !classified
                .iter()
                .any(|a| matches!(a, Arm::Fixed(k, _) if *k == n))
        })
        .collect();
    if !missing.is_empty() {
        // A `Rest(m)`/`Wild` covers `[m, ∞)`, but a shorter length `n < m` has no `Fixed(n)` arm. The
        // repair is to add exactly those missing-length arms — `(list _ _ … n underscores) (trap "TODO")`
        // (a length-0 gap is the empty `((list) (trap "TODO"))`). Underscore elements (the matcher only
        // needs the ARITY covered; the author renames as needed), diverging body. Fixed arms must precede
        // the catch-all to be reachable, but `insert_arms` appends AFTER the last arm — which is where the
        // `Rest`/`Wild` sits, so the inserted fixed arm would be dead. Still, applying it makes the match
        // exhaustive (the appended arm's length is now covered by SOME arm — the appended one shadows
        // nothing since the earlier catch-all already matched); `--verify-fixes` confirms it clears the
        // CDZ0210. (WHERE to place it for reachability is the author's call — the fix resolves the gap.)
        let arms: Vec<String> = missing
            .iter()
            .map(|&n| {
                let unders = vec!["_"; n].join(" ");
                let pat = if n == 0 {
                    "(list)".to_string()
                } else {
                    format!("(list {unders})")
                };
                format!("({pat} (trap \"TODO\"))")
            })
            .collect();
        let reject = Reject::coded(
            Code::NonExhaustive,
            "a list match must cover every length (a rest pattern leaves shorter lengths uncovered)",
        );
        return Core::Poison(match db.parent_of(scrutinee) {
            Some(match_form) => reject.with_fix(Fix::insert_arms_heuristic(match_form, arms)),
            None => reject,
        });
    }
    // A CONSTANT scrutinee FOLDS: the length selects the arm; the body's element binders read the
    // constant elements via their `SumPayload` `Elem`/`RestFrom` folds, so lowering the SELECTED body is
    // all that is needed (no β-substitution).
    match core_of(db, scrutinee) {
        Core::ListNew { elems } => {
            let n = elems.len();
            for arm in &classified {
                let (matches, body) = match arm {
                    Arm::Fixed(k, body) => (*k == n, *body),
                    Arm::Rest(lead, body) => (n >= *lead, *body), // rest: length ≥ leading count
                    Arm::Wild(body) => (true, *body),
                };
                if matches {
                    return core_of(db, body);
                }
            }
            Core::Poison(Reject::decline(
                "list match: no arm matched the constant list (unreachable — a catch-all was required)",
            ))
        }
        Core::Poison(r) => Core::Poison(r),
        // A RUNTIME list scrutinee — emit `Core::MatchList`, which dispatches on `vec-len` at run time.
        // Each arm's length condition drives the dispatch; the leading element binders + rest binder read
        // the runtime list on their own (`SumPayload` `Elem`/`RestFrom` → `vec-get`/`vec-split`).
        _ => {
            let match_arms: Vec<crate::core::ListArm> = classified
                .iter()
                .map(|arm| {
                    let (cond, body) = match arm {
                        Arm::Fixed(k, body) => (crate::core::ListArmCond::LenEq(*k), *body),
                        Arm::Rest(lead, body) => (crate::core::ListArmCond::LenGe(*lead), *body),
                        Arm::Wild(body) => (crate::core::ListArmCond::Any, *body),
                    };
                    crate::core::ListArm { cond, body }
                })
                .collect();
            Core::MatchList {
                scrutinee,
                arms: match_arms,
            }
        }
    }
}

/// Lower a match over a MAP scrutinee by KEY-DIRECTED patterns (ask-61, core-semantics.md §A Map Is
/// Matched By Key-Directed Patterns). A `(map (k p) …)` arm matches when the map HAS every named key `k`
/// (each bound to a value the body reads via a `MapField`); a bare binder / `_` is a catch-all. This
/// increment folds a CONSTANT `Core::MapNew` scrutinee: a named key is present iff some entry's key is
/// `const_compound_eq` to it, so the first arm whose keys are all present is selected and its body
/// lowered (the body's `MapField` binders then fold — the value at each key, the rest map). A map's key
/// set is UNBOUNDED, so a `(map …)` arm covers no shape — the match needs a catch-all (else CDZ0210).
/// A runtime map scrutinee, or a key-sub-pattern that is not a bare binder, declines (later increments).
fn lower_match_map(db: &mut Db, scrutinee: StructId, arms: &[(StructId, StructId)]) -> Core {
    // The constant scrutinee's entries (the corpus shape: an inline `Map.insert` chain / `(map …)`).
    let entries = match core_of(db, scrutinee) {
        Core::MapNew { entries, .. } => entries,
        Core::Poison(r) => return Core::Poison(r),
        _ => {
            return Core::Poison(Reject::decline(
                "matching a runtime map by key-directed patterns needs the runtime map matcher (constant maps only)",
            ));
        }
    };
    // A key is present in the constant map iff some entry's key compares equal to it (by value).
    let key_present = |db: &mut Db, k: StructId, es: &[(StructId, StructId)]| -> bool {
        es.iter()
            .any(|&(ek, _)| const_compound_eq(db, ek, k) == Some(true))
    };
    // WELL-FORMEDNESS: a `(map …)` arm covers no shape (a map's key set is unbounded), so the arms must
    // include a CATCH-ALL (a bare binder / `_`) or the match is non-exhaustive (CDZ0210).
    let has_catch_all = arms.iter().any(|&(pat, _)| db.ast.as_name(pat).is_some());
    if !has_catch_all {
        return Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a map match must end in a catch-all (`_` or a whole-map binder) — a map's key set is unbounded",
        ));
    }
    for &(pat, body) in arms {
        // A bare binder / `_` is a catch-all — it always matches.
        if db.ast.as_name(pat).is_some() {
            return core_of(db, body);
        }
        // A `(map (k p) … .. rest)` pattern: matches iff EVERY named key is present in the constant map.
        let Some((pat_entries, _rest)) = crate::resolve::map_pattern_of(db, pat) else {
            return Core::Poison(Reject::decline(
                "a map match arm that is not a `(map …)` pattern or a binder is not yet supported",
            ));
        };
        // Each key sub-pattern must be a bare-binder value (`p`) — a nested value pattern is a later
        // increment. The KEY itself is a value expression (a literal/scoped name), evaluated for presence.
        if !pat_entries
            .iter()
            .all(|&(_, v)| db.ast.as_name(v).is_some())
        {
            return Core::Poison(Reject::decline(
                "a map pattern value sub-pattern that is not a binder is not yet supported",
            ));
        }
        let all_present = pat_entries
            .iter()
            .all(|&(k, _)| key_present(db, k, &entries));
        if all_present {
            // This arm matches — its body's `MapField` binders fold against the constant scrutinee.
            return core_of(db, body);
        }
        // Else fall through to the next arm (the key-directed pattern is a genuine presence test).
    }
    Core::Poison(Reject::decline(
        "map match: no arm matched the constant map (unreachable — a catch-all was required)",
    ))
}

/// A constant scrutinee value for the guarded-match fold — an integer or a boolean.
enum GuardFoldScrut {
    Int(IntValue),
    Bool(bool),
    Str(String),
}

/// Walk a constant-value path from `root` down `steps`, returning the leaf's core if EVERY step lands
/// in a compile-time-constant compound (`Core::SumNew` payloads / `Core::Tuple` elements). This folds a
/// nested payload binder over a constant scrutinee — `(match (Some (Some 5)) ((Some (Some y)) y))`
/// through `[Payload, Payload]` yields the constant `5`, no heap read. `None` if any step hits a runtime
/// value (then the binder emits a runtime `Core::SumPayload` walk).
/// Drop the `Payload` steps that fall over a NOMINAL NEWTYPE sub-value — each is a runtime no-op (the box
/// is erased, so the value already IS its underlying value; `core-semantics.md §156`). The remaining
/// steps are the REAL heap accesses (a boxed sum's `sum-payload`, a tuple's `arr-get`) the backend walks,
/// so the emit path needs no nominal awareness. Walks the scrutinee's type in lockstep with the steps —
/// exactly as `type_of(SumPayload)` does — using `heads` to instantiate a boxed-sum `Payload`. A
/// nominal `Payload` unwraps to `inner` and is DROPPED; every other step is KEPT and advances the type.
fn erase_nominal_steps(
    db: &mut Db,
    scrutinee: StructId,
    steps: &[crate::core::PathStep],
    heads: &[StructId],
) -> Vec<crate::core::PathStep> {
    use crate::core::PathStep;
    let mut cur = crate::infer::type_of(db, scrutinee);
    let mut heads_it = heads.iter();
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        match step {
            PathStep::Payload => {
                if let crate::ty::Ty::Nominal { inner, .. } = &cur {
                    // Nominal unwrap — a no-op step. Advance the type to `inner`, DROP the step.
                    cur = (**inner).clone();
                } else {
                    // A real boxed-sum payload read — KEEP it, advance the type via the variant head.
                    let head = heads_it.next().copied();
                    out.push(*step);
                    cur = head
                        .and_then(|h| crate::infer::payload_ty_at_instantiation(db, h, &cur))
                        .unwrap_or(crate::ty::Ty::Any);
                    continue;
                }
            }
            PathStep::Elem(i) => {
                out.push(*step);
                cur = match &cur {
                    crate::ty::Ty::Tuple(elems) => {
                        elems.get(*i).cloned().unwrap_or(crate::ty::Ty::Any)
                    }
                    crate::ty::Ty::List(elem) => (**elem).clone(),
                    _ => crate::ty::Ty::Any,
                };
            }
            PathStep::RestFrom(_) => {
                // The rest sublist has the SAME type as the list scrutinee (`(List elem)`) — a tail of a
                // list is still a list of its element type.
                out.push(*step);
                // `cur` stays the list type (unchanged); a non-list here is a fault reported elsewhere.
            }
        }
    }
    out
}

fn fold_sum_path(db: &mut Db, root: StructId, steps: &[crate::core::PathStep]) -> Option<Core> {
    use crate::core::PathStep;
    let mut cur = root;
    // A TYPE cursor tracked ALONGSIDE `cur`, peeled one nominal layer per erased `Payload` step. Tracking
    // the peeled type — rather than re-reading `type_of(cur)` each step — is essential when a newtype WRAPS
    // A SUM (`(type W (V (Result …)))`): the newtype is erased, so `cur` stays the SAME node and its raw
    // type reads `Ty::Nominal` for EVERY step; re-reading it consumed the inner sum's `Payload` as a SECOND
    // nominal no-op and folded a payload binder to the WHOLE wrapper (a miscompile — `n` in `(W.V (Ok n))`
    // became the whole `Result`). The peeled cursor fires the nominal skip exactly once per layer, so the
    // inner sum's `Payload` then descends the sum (constant) or correctly declines the fold (runtime).
    let mut ty = crate::infer::type_of(db, root);
    for step in steps {
        // A `Payload` step over a NOMINAL NEWTYPE sub-value is a no-op: the box is erased, so the newtype
        // construction lowered its payload core DIRECTLY at `cur` (no `Core::SumNew` to descend). PEEL one
        // nominal layer off the type cursor and leave `cur` unchanged (a following `Payload` reads a wrapped
        // sum, a following `Elem` reads a multi-payload newtype's tuple).
        if matches!(step, PathStep::Payload)
            && let crate::ty::Ty::Nominal { inner, .. } = &ty
        {
            ty = (**inner).clone();
            continue;
        }
        cur = match (step, core_of(db, cur)) {
            (PathStep::Payload, Core::SumNew { payloads, .. }) if payloads.len() == 1 => {
                payloads[0]
            }
            (PathStep::Elem(i), Core::Tuple { elems }) => *elems.get(*i)?,
            // A list-pattern element binder reads position `i` of a CONSTANT list — the same `Elem` step a
            // tuple element uses, over a `Core::ListNew`. A runtime list has no `Core::ListNew` here.
            (PathStep::Elem(i), Core::ListNew { elems }) => *elems.get(*i)?,
            // A list-pattern REST binder over a CONSTANT list folds to a fresh `Core::ListNew` of the tail
            // elements (from index `k`) — a synthesized node so the tail sublist is itself constant.
            (PathStep::RestFrom(k), Core::ListNew { elems }) => {
                let tail: Vec<StructId> = elems.iter().skip(*k).copied().collect();
                return Some(Core::ListNew { elems: tail });
            }
            _ => return None,
        };
        // Re-sync the type cursor to the descended node (its own type — a nested newtype's inner peels on
        // the next `Payload`, a tuple element's type drives a following step).
        ty = crate::infer::type_of(db, cur);
    }
    Some(core_of(db, cur))
}

/// Lower a match over a SUM scrutinee to a DECISION TREE (Maranget). Dispatch on the variant
/// DISCRIMINANT at each level; a NESTED pattern shares its outer probe and splits on the inner
/// discriminant, so `(Some (Some x))`, `(Some None)`, `None` test the outer `Some` tag ONCE and only
/// then the inner tag — two tag checks on the deep path, not a linear re-probe per arm
/// (`type-system.md §Patterns Compose`). Exhaustiveness (`type-system.md §A Match Is Exhaustive Against
/// The Sum Type's Variant Set`) is checked at EACH switch: every variant covered OR a default arm; else
/// CDZ0210. A constant sum FOLDS to the selected body (like a scalar match); a runtime sum emits a
/// `Core::MatchSum` tree. A payload binder resolves to a `SumPayload` on its own (resolve Case 6), so an
/// arm carries only its discriminant + continuation.
//= spec/capabilities/type-system.md#a-match-is-exhaustive-against-the-sum-type-s-variant-set
//# The exhaustiveness rule governing a match MUST be checked against the scrutinee sum type's variant set, so that a match covering fewer than all variants is a compile-time rejection determined by that variant set rather than a runtime outcome.
/// Lower a `match` over a BYTES scrutinee whose arms include `(bin …)` binary patterns (BN3, constant
/// scrutinee). Each arm is either a `(bin <seg>…)` pattern or a CATCH-ALL (a bare binder / `_`). A `bin`
/// arm MATCHES iff the segment automaton (`bin_match_decode`) consumes the whole scrutinee AND every
/// LITERAL-slot segment's decoded value equals the literal (a magic-number/tag probe); its binder slots
/// bind via `BinField` (resolve Case B) — so the arm body needs no per-binder threading here. A match
/// with NO catch-all and only `bin` arms is NON-EXHAUSTIVE (a `bin` pattern never covers every byte
/// sequence — empty input, wrong length, an unequal literal all fail) → CDZ0210, exactly like a sum
/// missing a variant. On a CONSTANT scrutinee, select the first matching arm and lower its body; a
/// runtime scrutinee declines (the BN4 cursor automaton).
fn lower_match_bin(db: &mut Db, scrutinee: StructId, arms: &[(StructId, StructId)]) -> Core {
    if let Core::Poison(r) = core_of(db, scrutinee) {
        return Core::Poison(r);
    }
    // Classify arms. A `(bin …)` arm carries its parsed segments; a bare-name/`_` arm is a catch-all.
    // (A guarded bin pattern is a later refinement — decline if seen so we never mis-select.)
    enum BinArm {
        Bin(Vec<crate::resolved::Segment>, StructId), // segments, body
        CatchAll(StructId),                           // body (bare binder or `_`)
    }
    let mut classified: Vec<BinArm> = Vec::with_capacity(arms.len());
    for &(pat, body) in arms {
        if db.ast.head_name(pat) == Some("bin") {
            match crate::resolve::resolved_of(db, pat) {
                crate::resolved::Resolved::Bin { segs } => classified.push(BinArm::Bin(segs, body)),
                crate::resolved::Resolved::Poison(r) => return Core::Poison(r),
                _ => {
                    return Core::Poison(Reject::decline(
                        "a bin pattern did not resolve to segments",
                    ));
                }
            }
        } else if db.ast.as_name(pat).is_some() {
            // A bare name (binder) or `_` — a catch-all binding the whole scrutinee.
            classified.push(BinArm::CatchAll(body));
        } else {
            // A literal / other pattern against a Bytes scrutinee — not supported here; decline.
            return Core::Poison(Reject::decline(
                "a match over Bytes mixes a bin pattern with an unsupported pattern",
            ));
        }
    }
    // Exhaustiveness: a `bin` pattern never covers every byte sequence, so a match with no catch-all is
    // non-exhaustive (CDZ0210) — the same rule as a sum missing a variant.
    let has_catch_all = classified.iter().any(|a| matches!(a, BinArm::CatchAll(_)));
    if !has_catch_all {
        return Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a match over Bytes with only bin patterns and no catch-all is non-exhaustive",
        ));
    }
    // A CONSTANT scrutinee → select the first matching arm at compile time.
    let Some(raw) = bin_const_scrutinee(db, scrutinee) else {
        // RUNTIME scrutinee → build a runtime decision: an if-chain over per-arm predicates. Only for arms
        // whose `(bin …)` is ALL fixed-width int segments (a runtime bits/bytes/dependent segment is a
        // later slice); such an arm's predicate is `bytes-len == total_width & (each literal segment read
        // == its literal)`, and its binders read via `BinIntRead` (resolve Case B → decode_bin_field
        // runtime). The arms are processed in order into a nested `if`, tail = the catch-all body.
        //
        // Build from the LAST arm backward: `acc` starts at the catch-all body's occurrence, and each
        // preceding `(bin …)` arm wraps it as `(if <predicate> <arm-body> <acc>)`. A synthesized `if`
        // node's core is pre-filled so it lowers directly (no re-resolution).
        // MATERIALIZE the scrutinee ONCE: it is read many times (each arm's length probe + literal probes
        // + the matched arm's binder reads), so recomputing the `BinBuild` per read would both re-run the
        // construction AND clash scratch slots. Mark it a KEPT binding and read it through a `LocalRef`, so
        // it evaluates once into a slot and every read is a `local.get`. The whole match is wrapped in a
        // `Core::Let { (scrutinee, scrutinee), if-chain }` below.
        db.kept_bindings.insert(scrutinee);
        let scrut_ref = synth_core(
            db,
            Core::LocalRef { binder: scrutinee },
            crate::ty::Ty::Bytes,
        );
        let mut acc: Option<StructId> = None; // the else-tail so far (an occurrence)
        // Walk arms in REVERSE so the first arm ends up outermost (first-match order).
        for arm in classified.iter().rev() {
            match arm {
                BinArm::CatchAll(body) => {
                    // A catch-all resets the tail to its body (a later bin arm before it is unreachable in
                    // first-match order, but we keep the structure simple — the catch-all is normally last).
                    acc = Some(*body);
                }
                BinArm::Bin(segs, body) => {
                    // Handled at runtime: fixed-width INT segments, plus (optionally) a FINAL UNSIZED
                    // `(bytes rest)` — a header + a variable-length tail (static offsets throughout). A
                    // bit-field, or a dependent-size `(bytes b n)` (dynamic offset), is a later slice.
                    let ok = segs.iter().enumerate().all(|(i, s)| match &s.kind {
                        crate::resolved::SegKind::Int { .. } => true,
                        // A final unsized bytes segment is the LAST segment with no dependent size.
                        crate::resolved::SegKind::Bytes { size: None } => i + 1 == segs.len(),
                        _ => false,
                    });
                    if !ok {
                        return Core::Poison(Reject::decline(
                            "a runtime bin match with a bit-field or dependent-size segment is not yet lowered",
                        ));
                    }
                    let Some(else_body) = acc else {
                        // A bin arm with no following catch-all: exhaustiveness already required a
                        // catch-all, so this is unreachable — decline defensively.
                        return Core::Poison(Reject::decline(
                            "a runtime bin match arm has no fallthrough (unreachable)",
                        ));
                    };
                    // The predicate reads the scrutinee through the materialized `scrut_ref`.
                    let pred = match build_bin_arm_predicate(db, scrut_ref, segs) {
                        Ok(p) => p,
                        Err(r) => return Core::Poison(r),
                    };
                    acc = Some(synth_if(db, pred, *body, else_body));
                }
            }
        }
        let Some(root) = acc else {
            return Core::Poison(Reject::decline("a runtime bin match has no arms"));
        };
        // Wrap in a `let` that materializes the scrutinee once (keyed by its own occurrence — the same
        // occurrence the `scrut_ref` + each arm body's `BinField` read resolve their `LocalRef` to).
        return Core::Let {
            bindings: vec![(scrutinee, scrutinee)],
            body: root,
        };
    };
    for arm in &classified {
        match arm {
            BinArm::CatchAll(body) => return core_of(db, *body),
            BinArm::Bin(segs, body) => {
                // A segment BN3 can't decide (a dependent-size `(bytes b n)`) → we cannot know whether
                // this arm matches, so we must NOT silently skip it to a later arm (that would MISCOMPILE
                // a case whose dependent arm should match). `bin_match_decode` handles dependent-size
                // `(bytes body n)` now (BN4), decoding `n` from an earlier segment; a genuine non-match
                // (overrun / leftover / dependent-size overrun) returns `None` → fall to the next arm.
                let Some(decoded) = bin_match_decode(db, &raw, segs) else {
                    continue;
                };
                // Every LITERAL-slot segment must equal its decoded value (a magic-number / tag probe).
                // A binder slot (a bare name) is bound, not tested.
                let mut all_literals_match = true;
                for (seg, dec) in segs.iter().zip(decoded.iter()) {
                    // A slot is a literal probe iff it is NOT a bare name. Read its constant value.
                    if db.ast.as_name(seg.slot).is_some() {
                        continue; // a binder — no probe
                    }
                    match (core_of(db, seg.slot), dec) {
                        (Core::ConstInt(lit), BinDecoded::Int(got)) => {
                            if !lit.eq_value(&IntValue::from_i64(*got)) {
                                all_literals_match = false;
                                break;
                            }
                        }
                        // A non-constant / non-int literal slot can't be decided here — abort the fold.
                        _ => {
                            return Core::Poison(Reject::decline(
                                "a bin pattern literal segment is not a constant integer",
                            ));
                        }
                    }
                }
                if all_literals_match {
                    return core_of(db, *body);
                }
            }
        }
    }
    // A catch-all is guaranteed present (checked above), so some arm always matches — unreachable.
    Core::Poison(Reject::decline(
        "bin match: no arm matched (unreachable — catch-all present)",
    ))
}

fn lower_match_sum(db: &mut Db, scrutinee: StructId, arms: &[(StructId, StructId)]) -> Core {
    // The scrutinee must be a COMPOUND the decision tree matches — a SUM (its type gives the root variant
    // set to switch on), a TUPLE (no discriminant; `Elem`-path binders/lit-tests), or a RECORD (no
    // discriminant and no destructure pattern — only a whole-value binder/wildcard arm). A poisoned
    // scrutinee propagates its poison; anything else is a decline (the caller routes only these here).
    let scrut_ty = crate::infer::type_of(db, scrutinee);
    if !matches!(
        scrut_ty,
        crate::ty::Ty::Sum { .. }
            | crate::ty::Ty::Nominal { .. }
            | crate::ty::Ty::Tuple(_)
            | crate::ty::Ty::Record(_)
    ) {
        if let Core::Poison(r) = core_of(db, scrutinee) {
            return Core::Poison(r);
        }
        return Core::Poison(Reject::decline(
            "compound match scrutinee is not a sum, tuple, or record",
        ));
    }
    // Build the initial pattern MATRIX: one row per arm, each a `(constraints, body)` where a constraint
    // is `(path, disc)` — "the sub-value at `path` must have discriminant `disc`". A row's constraints
    // start from its top-level pattern (path `[]`) and may nest. A malformed/unsupported pattern declines
    // the whole match (a heap walk / literal-in-sum is a later increment), never a silent match.
    let mut rows: Vec<MatchRow> = Vec::new();
    for &(pat, body) in arms {
        // Peel a `(guard <inner-pattern> <cond>)` wrapper: the arm's discriminant constraints come from
        // the inner pattern, and `<cond>` is carried as the row's guard (gated at the leaf in `build_tree`).
        let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        // LINEARITY: a pattern is a BINDER POSITION and must bind each name at most once (core-semantics.md
        // §Patterns Compose: "A pattern MUST bind each name at most once … rather than silently shadowing").
        // `(tuple x x)` / `(Some (tuple x x))` binds `x` twice — CDZ0102, the same non-linear-binder error a
        // repeated `def` parameter gets — not a last-wins shadow that makes the first binder's payload
        // unreachable. Checked across the WHOLE arm pattern (nested sub-patterns included).
        if let Err(r) = check_pattern_linear(db, inner_pat) {
            return Core::Poison(r);
        }
        let mut lit_tests = Vec::new();
        match pattern_constraints(db, inner_pat, &scrut_ty, Vec::new(), &mut lit_tests) {
            Ok(constraints) => rows.push(MatchRow {
                constraints,
                lit_tests,
                body,
                guard,
            }),
            Err(r) => return Core::Poison(r),
        }
    }
    // Compile the matrix into a decision tree rooted at the scrutinee (path `[]`, type `scrut_ty`).
    let mut path_types: std::collections::HashMap<Vec<crate::core::PathStep>, crate::ty::Ty> =
        std::collections::HashMap::new();
    path_types.insert(Vec::new(), scrut_ty);
    match build_tree(db, scrutinee, &rows, &path_types) {
        // The whole match reduces to one body (a top-level catch-all, or a fully constant-folded tree).
        Ok(crate::core::SumCont::Leaf(body)) => core_of(db, body),
        // Otherwise the root is a Switch (the usual case) — or a Guarded, when a disc-fold collapsed the
        // root switch to the selected variant's guarded arm. Either way the backend emits it through the
        // uniform `emit_sum_cont`, so carry the root continuation directly.
        Ok(root) => Core::MatchSum {
            scrutinee,
            root: Box::new(root),
        },
        Err(r) => Core::Poison(r),
    }
}

/// One row of the pattern matrix: the discriminant CONSTRAINTS this arm imposes (each a `(path, disc)`),
/// and the arm's body. An empty constraint set is a catch-all (a bare binder / `_` top-level pattern) —
/// it matches regardless of any discriminant. Constraints are ordered outer-to-inner (a shorter path
/// first), which is the order the tree tests them.
#[derive(Clone)]
struct MatchRow {
    constraints: Vec<(Vec<crate::core::PathStep>, u32)>,
    /// LITERAL tests the arm imposes on payload sub-values: each `(path, probe)` requires the scalar at
    /// `path` to equal the literal. A `(Some 0)` pattern adds `([Payload], Int(0))`. Like a guard, a
    /// literal test does NOT count toward exhaustiveness (it may not match — it needs a same-variant
    /// binder/wildcard fall-through), and it is gated once the discriminant constraints are satisfied.
    lit_tests: Vec<(Vec<crate::core::PathStep>, crate::core::Probe)>,
    body: StructId,
    /// A match-arm GUARD `(guard <pattern> <cond>)` — the boolean `<cond>` the arm additionally requires.
    /// `None` for an unguarded arm. Once every discriminant constraint is satisfied (the row reaches a
    /// leaf position in `build_tree`), a guarded row emits `if cond then body else <fall-through>` and
    /// does NOT count toward exhaustiveness; an unguarded row is an unconditional leaf.
    guard: Option<StructId>,
}

/// Reject a match-arm pattern that binds the same name more than once (CDZ0102) — a pattern is a BINDER
/// POSITION and must be LINEAR. Walks the whole pattern collecting BINDER names (a bare non-`_` name that
/// is NOT a variant constructor of a sum in scope, NOR a literal), and faults the second occurrence,
/// anchored there. A `_` binds nothing (may repeat); a variant name (`Some`, `E.Lit`) is a constructor,
/// not a binder; a literal is a value, not a binder. Recurses into tuple/variant sub-patterns and peels a
/// `(guard …)` wrapper — so linearity holds across the WHOLE composed pattern, a name in two sub-patterns
/// faulting exactly as one appearing twice in a flat pattern. (A non-deduping walk — unlike resolve's
/// binder lookups it must SEE every occurrence to catch the repeat.)
///
//= spec/capabilities/core-semantics.md#bindings-introduced-by-a-pattern-are-scoped-to-its-branch
//# A pattern MUST bind each name at most once; a pattern that binds the same name more than once MUST be a compile-time error (`CDZ0102`), so that a pattern is linear rather than silently shadowing an earlier binder or imposing a hidden equality constraint.
//= spec/capabilities/core-semantics.md#patterns-compose
//# A pattern MUST admit any pattern in each of its binder positions, so that a constructor pattern's binder and a tuple pattern's element MAY themselves be a wildcard, a name, a tuple pattern, or a constructor pattern, matched recursively to any depth.
//= spec/capabilities/core-semantics.md#patterns-compose
//# A composed pattern MUST bind the union of its sub-patterns' bindings, matched recursively, and MUST remain linear across the whole pattern, so that a name appearing in more than one sub-pattern is the same `CDZ0102` error as one appearing twice in a flat pattern.
fn check_pattern_linear(db: &mut Db, pat: StructId) -> Result<(), Reject> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_pattern_binders(db, pat, &mut seen)
}

/// Validate a pattern in a BINDING position — a `let` binder, a `def`/`fn` parameter — where there is NO
/// alternative arm, so the pattern MUST be irrefutable. `value_ty` is the type of the value being bound (a
/// `let` initializer's type, or a parameter's solved type), used for the shape/arity check; pass `Ty::Any`
/// when it is not yet solved (the permissive treatment a projection of `Any` gets — no shape check, only
/// classification+linearity).
///
/// A binding pattern IS a single-arm match, so an ill-formed one gets the code the desugared match would.
/// A REFUTABLE pattern (a multi-variant constructor, a literal, a length-constrained list pattern) is
/// CDZ0210 (non-exhaustive — the other cases are uncovered and there is no fall-through arm). A
/// SHAPE-INCOMPATIBLE pattern (a wrong-arity tuple, a tuple pattern vs a non-tuple value) is CDZ0201. A
/// NON-LINEAR pattern (a binder repeated, flat or nested) is CDZ0102 (via `check_pattern_linear`).
///
//= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
//# A binding position — a `let` binder, a function or `fn` parameter — MUST accept an irrefutable pattern in place of a bare name, binding the names the pattern introduces to the corresponding sub-values of the bound value, exactly as the same pattern would in a single match arm over that value. A bare name and a wildcard are the trivial irrefutable patterns; a tuple pattern whose every element is itself irrefutable is irrefutable, matched recursively to any depth in the sense of *Patterns Compose*. A destructuring parameter MUST NOT change the function's arity — the parameter occupies one argument position and names its parts, so `(def (f (tuple a b)) …)` remains a single-argument function.
//= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
//# A binding position has no alternative arm, so its pattern MUST be irrefutable — it MUST match every value of the bound value's type.
//= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
//# A refutable pattern in a binding position — a constructor pattern of a multi-variant sum, a literal, or a length-constrained list pattern, none of which matches every value of its type — MUST be a compile-time error (`CDZ0210`), the same non-exhaustiveness the equivalent single-arm match would raise under *Matching Is Exhaustive Or Rejected*.
//= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
//# A pattern whose shape cannot match the bound value's type at all — a tuple pattern of the wrong arity, or a tuple pattern against a non-tuple value — MUST be a compile-time error (`CDZ0201`), and a non-linear binding pattern MUST be the same `CDZ0102` error as in any other pattern position.
///
/// A pattern that is irrefutable in principle but not-yet-supported (a record pattern, a single-variant
/// user sum, any list pattern) DECLINES (reject-don't-miscompile — a later increment accepts it), NOT a
/// coded reject. The classifier consults the PRELUDE (a variant's owning sum + variant count), never a
/// head-string scan, so `None` is a constructor (not a binder) and a single-variant sum is told from a
/// multi-variant one.
///
/// A bare name / `_` is the trivial irrefutable pattern — Ok with no work (the common, hot binding).
pub(crate) fn check_binding_pattern(
    db: &mut Db,
    pat: StructId,
    value_ty: &crate::ty::Ty,
) -> Result<(), Reject> {
    // An ANNOTATED binding pattern `(: <pat> <Type>)` (type-system.md §Annotations Constrain, Never
    // Contradict): the annotation constrains the bound value's type and the inner `<pat>` is the real
    // binder. Peel it — check the annotation type AGREES with the value's type (a contradiction is
    // CDZ0203, `(: x Bool) = 5`), then recurse on `<pat>` so the inner pattern's own well-formedness
    // (irrefutable / linear / right shape) is still checked. A generic/deferred value type (`Any`, an
    // unsolved var) agrees with any annotation — the annotation grounds it, no contradiction.
    //= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
    //# A binding pattern MAY carry a type annotation `(: <pattern> <Type>)`, which constrains the bound value's type while the inner pattern binds its names, in accordance with *Annotations Constrain, Never Contradict* (`type-system.md`): the annotation participates in inference as an added constraint, and a value whose type cannot satisfy it MUST be a compile-time error (`CDZ0203`), exactly as a value annotation `(: <expression> <Type>)` is.
    if let Some(ann) = db.ast.as_form(pat, ":")
        && ann.len() == 2
    {
        let inner = ann[0];
        let ty_expr = ann[1];
        if let Some(annot_ty) = crate::eval::typeval_of(db, ty_expr)
            && !value_ty.agrees_with(&annot_ty)
        {
            return Err(Reject::coded(
                Code::TypeMismatch,
                format!(
                    "a binder annotated {} is bound to a value of type {}",
                    annot_ty.render_name(),
                    value_ty.render_name()
                ),
            )
            .at(pat));
        }
        // The annotation may REFINE the value type (a deferred literal grounded to the annotated width),
        // so validate the inner pattern against the annotation type when it is more specific than the
        // value type, else the value type.
        let refined = crate::eval::typeval_of(db, ty_expr).unwrap_or_else(|| value_ty.clone());
        let inner_ty = if matches!(value_ty, crate::ty::Ty::Any) {
            refined
        } else {
            value_ty.clone()
        };
        return check_binding_pattern(db, inner, &inner_ty);
    }
    // A bare name (a binder) or `_` (wildcard) — trivially irrefutable, the common case.
    if let Some(name) = db.ast.as_name(pat) {
        // A bare name that resolves to a NULLARY constructor (`None`) is a refutable ctor, not a binder.
        if name != "_" && crate::eval::variant_disc_of(db, pat).is_some() {
            return classify_binding_ctor(db, pat, value_ty);
        }
        return Ok(());
    }
    // A literal `0` / `true` / `"s"` matches ONE value of its type — refutable, CDZ0210.
    if matches!(
        crate::resolve::resolved_of(db, pat),
        crate::resolved::Resolved::Int(_)
            | crate::resolved::Resolved::Bool(_)
            | crate::resolved::Resolved::Str(_)
            | crate::resolved::Resolved::Float(_)
            | crate::resolved::Resolved::Bytes(_)
    ) {
        return Err(Reject::coded(
            Code::NonExhaustive,
            "a literal pattern is refutable — it matches one value, not every value of its type, so it \
             cannot appear in a binding position",
        )
        .at(pat));
    }
    // A compound pattern `(head arg…)`. A `tuple` head is the one accepted destructuring shape in
    // Increment A; a constructor head is classified by variant count; a record/list head declines.
    //
    // This is where a tuple is DECONSTRUCTED by pattern matching: `(tuple a b)` in pattern position binds
    // its positional elements to `a`/`b` (each element sub-pattern recursed below), so a tuple's elements
    // are reachable by destructuring, not only by positional projection.
    //= spec/capabilities/core-semantics.md#a-tuple-is-a-fixed-size-positional-product
    //# A tuple MUST be deconstructible by pattern matching, so that `(tuple a b)` in pattern position binds the elements.
    if is_tuple_pattern(db, pat) {
        // Linearity across the WHOLE pattern (CDZ0102).
        check_pattern_linear(db, pat)?;
        let elems: Vec<StructId> = db
            .ast
            .as_form(pat, "tuple")
            .or_else(|| db.ast.as_ctor_form(pat, "tuple"))
            .unwrap_or(&[])
            .to_vec();
        // A binding position is IRREFUTABLE: each tuple ELEMENT sub-pattern must itself be irrefutable.
        // Recurse `check_binding_pattern` into each element with the element's own type, so a literal
        // element (int/bool/STRING/float) → CDZ0210, a multi-variant-ctor element → CDZ0210, a
        // single-variant/record/list element → DECLINE, and a bare-binder / nested-irrefutable-tuple
        // element → Ok — exactly the classification the TOP-LEVEL binder gets, at any nesting depth. The
        // BUG was that the tuple case called the MATCH-ARM collector `pattern_constraints` (where a
        // literal element is a runtime probe and a variant element a discriminant test — both legitimate
        // in a `match` arm) and then DISCARDED its result with a plain `Ok(())`, so a refutable
        // `(tuple 0 b)` / `(tuple (Some x) b)` binder slipped through and ran, silently dropping the
        // refutable sub-pattern. Recursing FIRST (before the arity check below) also gives a nested string/
        // float literal the same CDZ0210 the top-level binder emits, rather than the codeless
        // "malformed sum match pattern" decline `pattern_constraints`' atom fall-through produced.
        //
        // Element types from the value type when it is a matching-arity tuple; else `Any` (the permissive
        // treatment for an unsolved/`Any` or wrong-arity payload — a genuine arity mismatch is faulted
        // CDZ0201 by `pattern_constraints` below; classifying the elements against `Any` first is
        // harmless, since refutability is a property of the pattern shape, not the value type).
        let elem_tys: Vec<crate::ty::Ty> = match value_ty {
            crate::ty::Ty::Tuple(ts) if ts.len() == elems.len() => ts.to_vec(),
            _ => vec![crate::ty::Ty::Any; elems.len()],
        };
        for (i, &elem) in elems.iter().enumerate() {
            check_binding_pattern(db, elem, &elem_tys[i])?;
        }
        // Shape/arity against the value's type (CDZ0201) + nested-literal-TYPE agreement — reusing the
        // match-arm machinery verbatim. Runs AFTER the element refutability check so a refutable element's
        // CDZ0210 wins over this collector's shape decline; a well-shaped irrefutable pattern passes both.
        let mut lit_tests = Vec::new();
        pattern_constraints(db, pat, value_ty, Vec::new(), &mut lit_tests)?;
        return Ok(());
    }
    // A `(record …)` binding pattern is irrefutable in principle but a later increment — DECLINE.
    if db.ast.as_form(pat, "record").is_some() {
        return Err(Reject::decline(
            "a record binding pattern is not yet supported (Increment B)",
        ));
    }
    // A `(list …)` binding pattern (length-constrained → refutable; rest-binder → irrefutable) is out of
    // scope with all list patterns — DECLINE (not reject), so the irrefutable form is not mis-rejected.
    if db.ast.as_form(pat, "list").is_some() {
        return Err(Reject::decline(
            "a list binding pattern is not yet supported",
        ));
    }
    // Otherwise a constructor-headed pattern `(Some x)` / `((. Sum V) x)` — classify by variant count.
    classify_binding_ctor(db, pat, value_ty)
}

/// Classify a CONSTRUCTOR-headed binding pattern (`(Some x)`, bare `None`, `((. Sum V) x)`): a
/// SINGLE-variant sum is irrefutable but a later increment (DECLINE); a MULTI-variant sum is refutable
/// (the other variants are uncovered) → CDZ0210. The head is resolved against the prelude
/// (`variant_owner_decl` → the owning sum's declaration → its variant count), never a head-string scan.
/// A head that is not a constructor at all is a shape error (CDZ0201).
fn classify_binding_ctor(
    db: &mut Db,
    pat: StructId,
    value_ty: &crate::ty::Ty,
) -> Result<(), Reject> {
    // The constructor head: a bare name / member `(. Sum V)` used as a whole pattern, or a `(head arg…)`
    // application's head.
    let head = match db.ast.get(pat) {
        crate::ast::Struct::Atom(_) => pat,
        crate::ast::Struct::List(children) => match children.first().copied() {
            // A bare member `(. Sum V)` used as a whole pattern — the ctor is the pattern itself.
            Some(first) if db.ast.as_name(first) == Some(".") => pat,
            Some(first) => first,
            None => {
                return Err(Reject::coded(Code::Malformed, "an empty binding pattern").at(pat));
            }
        },
    };
    let Some(decl) = crate::eval::variant_owner_decl(db, head) else {
        // Not a constructor — a shape error (a head that is neither tuple/record/list nor a ctor).
        return Err(Reject::coded(
            Code::Malformed,
            "a binding pattern head is not a tuple, record, or constructor",
        )
        .at(pat));
    };
    let variant_count = db
        .type_decl_by_occ(decl)
        .map(|d| d.variants.len())
        .unwrap_or(0);
    if variant_count == 1 {
        // A single-variant sum's sole constructor ALWAYS matches — the pattern is IRREFUTABLE, so it is a
        // valid binding position (`(let (((Id.Mk n) v)) …)`). Its payload sub-patterns must themselves be
        // irrefutable, exactly as a tuple pattern's elements are: recurse `check_binding_pattern` into each
        // payload arg at the payload's type (a literal payload → CDZ0210, a bare binder / nested tuple →
        // Ok). The payload TYPE is the variant's payload at this instantiation; `pattern_constraints` then
        // checks the shape/arity (CDZ0201) + linearity, reusing the match-arm machinery, so the binder
        // references (which resolve to a `SumPayload` reading the payload — resolve `last_binder_named`'s
        // ctor case) read a well-formed pattern. A nullary single-variant sum (`(type Marker (The))`) has
        // no payload arg to bind — nothing to recurse, trivially irrefutable.
        check_pattern_linear(db, pat)?;
        let args: Vec<StructId> = match db.ast.get(pat) {
            crate::ast::Struct::List(children) => match children.first().copied() {
                // A bare member `(. Sum V)` used whole — no payload args in the pattern.
                Some(first) if db.ast.as_name(first) == Some(".") => Vec::new(),
                _ => children[1..].to_vec(),
            },
            _ => Vec::new(),
        };
        // Each payload arg's type — the variant's payload types at the value's instantiation. A single
        // payload IS the underlying type; multiple payloads box as one tuple (matched positionally). Use
        // the value type's payload when resolvable, else `Any` (permissive — arity/shape faults below).
        for &arg in &args {
            // A payload arg is validated for irrefutability against `Any` (refutability is a property of
            // the pattern shape, not the value type — the tuple case does the same for its elements).
            check_binding_pattern(db, arg, &crate::ty::Ty::Any)?;
        }
        // Shape/arity + nested-literal-type agreement, reusing the match-arm collector (CDZ0201 on a
        // wrong-arity payload). Runs after the per-arg irrefutability check, exactly as the tuple case.
        let mut lit_tests = Vec::new();
        pattern_constraints(db, pat, value_ty, Vec::new(), &mut lit_tests)?;
        return Ok(());
    }
    // A multi-variant constructor is refutable — the other variants are uncovered, and there is no
    // alternative arm. CDZ0210, the non-exhaustive-single-arm-match code.
    Err(Reject::coded(
        Code::NonExhaustive,
        "a multi-variant constructor pattern is refutable — the other variants are uncovered, so it \
         cannot appear in a binding position (only in a `match` arm)",
    )
    .at(pat))
}

/// The recursive walk behind [`check_pattern_linear`]: insert each binder name into `seen`, faulting a
/// repeat. See that function for the binder-vs-ctor-vs-literal classification.
fn collect_pattern_binders(
    db: &mut Db,
    pat: StructId,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), Reject> {
    // Peel a guard wrapper — the binder-carrying pattern is the inner one.
    if let Some(g) = db.ast.as_form(pat, "guard")
        && g.len() == 2
    {
        return collect_pattern_binders(db, g[0], seen);
    }
    // A bare atom: a literal binds nothing; a `_` binds nothing; any OTHER bare name is a binder UNLESS it
    // is a nullary variant constructor (`None`, `Sign.Neg`) — a ctor is not a binder. `variant_disc_of`
    // recognizes a ctor value; a name that is not one is a binder.
    if let crate::ast::Struct::Atom(_) = db.ast.get(pat) {
        if matches!(
            crate::resolve::resolved_of(db, pat),
            crate::resolved::Resolved::Int(_) | crate::resolved::Resolved::Bool(_)
        ) {
            return Ok(()); // a literal is not a binder
        }
        if let Some(name) = db.ast.as_name(pat).map(|s| s.to_string()) {
            if name == "_" {
                return Ok(());
            }
            // A bare name that resolves to a variant constructor is a ctor, not a binder.
            if crate::eval::variant_disc_of(db, pat).is_some() {
                return Ok(());
            }
            if !seen.insert(name.clone()) {
                // RENAME the repeated binder to a fresh non-colliding name (`a` → `a2`), making the pattern
                // linear (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). Fresh
                // relative to the binders already seen in this pattern, so it collides with none. Heuristic:
                // the rename clears the hard error; the fresh binder is then unused until the author uses it
                // (two same-named binders were likely meant to be distinct values, or an equality the pattern
                // language does not express). Anchored at the repeated binder occurrence.
                let fresh = crate::diag::suggest::fresh_suffixed_name(&name, seen);
                return Err(Reject::coded(
                    Code::NonLinearBinder,
                    format!("pattern binds `{name}` more than once (a pattern must be linear)"),
                )
                .at(pat)
                .with_fix(Fix::replace_heuristic(pat, fresh)));
            }
        }
        return Ok(());
    }
    // A compound pattern `(head arg…)` — a variant `(Some p)`, a tuple `(tuple p…)`, or a member `(. S V)`
    // (a nullary ctor, no binders). The head is a ctor/`tuple`/`.` — not a binder; recurse into the args.
    if let crate::ast::Struct::List(children) = db.ast.get(pat) {
        let children = children.clone();
        // A `(. Sum V)` member pattern is a nullary-ctor reference — no binder args.
        if children.first().and_then(|&h| db.ast.as_name(h)) == Some(".") {
            return Ok(());
        }
        // Skip the head (a ctor / `tuple` alias); recurse each argument sub-pattern.
        for &arg in children.iter().skip(1) {
            collect_pattern_binders(db, arg, seen)?;
        }
    }
    Ok(())
}

/// Collect the discriminant constraints a PATTERN imposes on the sub-value at `path` (of type `ty`),
/// appending `(deeper-path, disc)` per variant test. A bare NAME is a binder/wildcard — NO constraint
/// (it matches any value; its binding is resolved independently). A variant pattern `(V arg…)` / bare
/// nullary `V` adds `(path, disc(V))` and recurses into its single payload arg at `path + [Payload]`
/// (a multi-payload variant's payload is a tuple — the arg descends through `Elem` in a later increment).
/// A variant name is distinguished from a binder by RESOLVING it against `ty`'s variant set: `None`
/// against `Option` is the nullary variant (a constraint), `x` is a binder (none). Errs (declines) on a
/// pattern this increment does not compile — a tuple/record destructure, a literal, a wrong-arity ctor.
///
/// A nullary variant pattern (`None`) and a unary+ one (`(Some x)`) are handled by the SAME arm — each
/// adds its discriminant test and descends into one payload position — so the matcher never branches on
/// a constructor's arity: every constructor pattern is treated uniformly as a single-arity application.
//= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
//# The pattern matcher MUST NOT special-case "nullary" vs "unary+" constructors by arity.
//= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
//# The pattern matcher MUST handle all constructor patterns uniformly as single-arity applications.
//= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
//# A pattern matching a sum type constructor MUST have the form `(Ctor binder)` in all cases: `(Some x)`, `(None _)`, `(Sign.Zero _)`.
/// Enrich the propagated "record has no field `Q`" poison of a MATCH-PATTERN head `(. Sum Q)` — where `Q`
/// is not a variant of the scrutinee sum — with a "did you mean?" over the sum's VARIANT NAMES, plus a
/// replace fix on the mistyped key. The pattern-position twin of `infer::no_field_reject`'s value-position
/// suggestion: `core_of(head)` (the member fold) emits the bare coded message; here — where the scrutinee
/// sum type `ty` is in hand — we can name the nearest variant. `ty` is the scrutinee's type (a `Ty::Sum`
/// when this fires; a non-sum leaves the bare `reject` untouched). Returns the enriched (or original)
/// reject. Deterministic — `suggest::nearest` over the declaration-ordered variant set.
fn enrich_pattern_head_suggestion(
    db: &mut Db,
    head: StructId,
    ty: &crate::ty::Ty,
    reject: Reject,
) -> Reject {
    // The scrutinee sum's declaration occurrence — the key of its (memoized) variant candidate set.
    let crate::ty::Ty::Sum { decl, .. } = ty else {
        return reject;
    };
    let decl = *decl;
    // The mistyped key: the second child of the `(. Sum Q)` head; its name is what to match + rewrite.
    let Some(key_occ) = db.ast.as_form(head, ".").and_then(|t| t.get(1).copied()) else {
        return reject;
    };
    let Some(key) = db.ast.as_name(key_occ).map(str::to_string) else {
        return reject;
    };
    // The nearest variant of `decl` to `key`, MEMOIZED per (decl, key): the variant-name clone + edit-
    // distance scan is O(variants), and a WIDE sum matched with a stale variant from N sites re-ran it each
    // → O(N²). Keyed by (decl, key), it is computed once per distinct query.
    let candidate = if let Some(hit) = db.variant_suggest_winner.get(&(decl, key.clone())) {
        hit.clone()
    } else {
        let names: Vec<String> = match db.type_decl_by_occ(decl) {
            Some(t) => t.variants.iter().map(|v| v.name.clone()).collect(),
            None => return reject,
        };
        let winner = crate::diag::suggest::nearest(&key, &names);
        db.variant_suggest_winner
            .insert((decl, key.clone()), winner.clone());
        winner
    };
    let Some(candidate) = candidate else {
        return reject; // no near variant — keep the bare "record has no field" message
    };
    // Append the suggestion to the bare message and carry a replace fix on the key occurrence — mirroring
    // `infer::no_field_reject`'s value-position enrichment.
    let message = format!("{} — did you mean `{candidate}`?", reject.message);
    Reject { message, ..reject }.with_fix(Fix::replace_heuristic(key_occ, candidate))
}

fn pattern_constraints(
    db: &mut Db,
    pat: StructId,
    ty: &crate::ty::Ty,
    path: Vec<crate::core::PathStep>,
    lit_tests: &mut Vec<(Vec<crate::core::PathStep>, crate::core::Probe)>,
) -> Result<Vec<(Vec<crate::core::PathStep>, u32)>, Reject> {
    // A GUARDED pattern `(guard <inner-pattern> <cond>)` contributes the INNER pattern's discriminant
    // constraints (the guard itself is not a discriminant test — it is carried on the `MatchRow` by
    // `lower_match_sum` and gated at the leaf in `build_tree`). Descend into the inner pattern so a
    // `(guard (Some x) …)` still constrains `[]` to the `Some` disc + binds `x` at `[Payload]`.
    if let Some(g) = db.ast.as_form(pat, "guard") {
        if g.len() != 2 {
            return Err(Reject::coded(
                Code::Malformed,
                "a guarded pattern must be (guard <pattern> <cond>)",
            ));
        }
        return pattern_constraints(db, g[0], ty, path, lit_tests);
    }
    // A LITERAL payload sub-pattern — an integer or boolean atom, NOT a name. `(Some 0)` matches `Some`
    // carrying exactly `0`: the literal refines the match (`core-semantics.md §Pattern Matching`, "nested
    // patterns can combine constructors and literals"). It imposes NO discriminant constraint (a scalar
    // has no variant tag); it adds a LITERAL TEST `(path, probe)` — the sub-value at `path` must EQUAL
    // the literal — gated (like a guard) once the enclosing discriminant is satisfied, with a same-variant
    // fall-through for the non-matching case. The literal's TYPE must AGREE with the sub-value's type at
    // this position: `(tuple true b)` against `(tuple 1 2)` puts a `Bool` literal where the element is
    // `Int64` — a literal-pattern-type mismatch (CDZ0201, core-semantics.md §Equality Is Structural),
    // checked HERE (nested), exactly as the top-level `(match 5 (true 1))` case is, so a nested wrong-type
    // literal does not slip past as a runtime non-match. (`ty` is `Any` for an unsolved position — no
    // check, the not-yet-constrained treatment a projection of `Any` gets.)
    let probe = match crate::resolve::resolved_of(db, pat) {
        crate::resolved::Resolved::Int(v) => {
            Some((crate::core::Probe::Int(v), crate::ty::Ty::int()))
        }
        crate::resolved::Resolved::Bool(b) => {
            Some((crate::core::Probe::Bool(b), crate::ty::Ty::Bool))
        }
        // A STRING-literal payload sub-pattern — `(Ast.Name "+")` matches an `Ast.Name` carrying exactly
        // "+". Like the Int/Bool literal, it imposes no discriminant, adds a `Probe::Str` lit-test gated
        // at the leaf, and folds against a constant `Core::ConstStr` (a runtime String payload declines
        // at `build_lit_test`, like the scalar string match). Enables the quote-pattern literal head
        // (`` `(+ …) `` → `(Ast.Name "+")`), matched by string equality.
        crate::resolved::Resolved::Str(s) => {
            Some((crate::core::Probe::Str(s), crate::ty::Ty::String))
        }
        _ => None,
    };
    if let Some((probe, lit_ty)) = probe {
        if !matches!(ty, crate::ty::Ty::Any) && !lit_ty.agrees_with(ty) {
            return Err(Reject::coded(
                Code::Malformed,
                format!(
                    "a {} literal pattern does not match the {} sub-value it is matched against",
                    lit_ty.render_name(),
                    ty.render_name()
                ),
            ));
        }
        lit_tests.push((path, probe));
        return Ok(Vec::new());
    }
    // A bare NAME: either a NULLARY VARIANT of this sum (`None`) or a binder/wildcard. Resolve it against
    // the sum's variant set — a name that IS a variant contributes that discriminant (no payload to
    // recurse into); any other bare name binds and contributes nothing.
    if let Some(name) = db.ast.as_name(pat) {
        let name = name.to_string();
        if name != "_"
            && let Some(disc) = variant_disc_by_name(db, ty, &name)
        {
            return Ok(vec![(path, disc)]);
        }
        return Ok(Vec::new()); // a binder / wildcard — no constraint
    }
    // A TUPLE pattern `(tuple p0 p1…)` at `path` — a variant's tuple PAYLOAD, destructured positionally
    // (core-semantics.md §Patterns Compose: a tagged value carrying a tuple is one nested pattern). A
    // tuple has no discriminant, so it imposes NO constraint of its own; each element sub-pattern
    // descends at `path + [Elem(i)]`, of the tuple element's type. (Reached only inside a variant
    // payload — the top-level scrutinee is a sum, so `pattern_constraints` is entered on a variant.)
    if is_tuple_pattern(db, pat) {
        let elems: Vec<StructId> = db
            .ast
            .as_form(pat, "tuple")
            .or_else(|| db.ast.as_ctor_form(pat, "tuple"))
            .unwrap_or(&[])
            .to_vec();
        // The payload MUST be a tuple, and the pattern's ARITY must match it — a tuple pattern against a
        // non-tuple payload, or one naming the wrong number of elements (`(tuple a b c)` against a
        // 2-tuple), is an ill-typed destructure the compiler REJECTS (CDZ0201), never a silent match on a
        // wrong shape. (type-system.md: two tuples agree only when their arities are identical.)
        let elem_tys: &[crate::ty::Ty] = match ty {
            crate::ty::Ty::Tuple(ts) if ts.len() == elems.len() => ts,
            // `Any` payload (an unsolved/unknown type) can't be arity-checked here — descend permissively
            // (each element `Any`), the same not-yet-constrained treatment a projection of an `Any` gets.
            crate::ty::Ty::Any => {
                let mut out = Vec::new();
                for (i, &elem) in elems.iter().enumerate() {
                    let mut deeper = path.clone();
                    deeper.push(crate::core::PathStep::Elem(i));
                    out.extend(pattern_constraints(
                        db,
                        elem,
                        &crate::ty::Ty::Any,
                        deeper,
                        lit_tests,
                    )?);
                }
                return Ok(out);
            }
            _ => {
                return Err(Reject::coded(
                    Code::Malformed,
                    format!(
                        "a tuple pattern of {} element(s) does not match the payload type {}",
                        elems.len(),
                        ty.render_name()
                    ),
                ));
            }
        };
        let mut out = Vec::new();
        for (i, &elem) in elems.iter().enumerate() {
            let mut deeper = path.clone();
            deeper.push(crate::core::PathStep::Elem(i));
            out.extend(pattern_constraints(
                db,
                elem,
                &elem_tys[i],
                deeper,
                lit_tests,
            )?);
        }
        return Ok(out);
    }
    // A LIST pattern `(list p0 p1…)` at `path` — a variant's LIST payload, destructured element-by-element
    // (`metaprogramming.md` quote patterns desugar `` `(+ ,a ,b) `` to `(Ast.List (list (Ast.Name "+") a
    // b))`, whose `(list …)` payload sub-pattern this handles; also a user `(W.Wrap (list a b))`). A list
    // has a RUNTIME length, so the pattern imposes a `ListLen` test (like a literal test — gated once the
    // discriminant constraints hold, folded against a constant list); each LEADING element sub-pattern
    // descends at `path + [Elem(i)]`, of the list's element type. A trailing `.. rest` makes the length
    // test AT-LEAST-`lead` and binds the tail — the rest binder resolves independently via `RestFrom(lead)`
    // (`resolve::find_binder_in_list`), so it needs no constraint here. SCOPE: the CONSTANT-scrutinee fold
    // only — a runtime list payload's `ListLen`/element reads decline (`build_lit_test`).
    if is_list_pattern(db, pat) {
        let raw: Vec<StructId> = db
            .ast
            .as_form(pat, "list")
            .or_else(|| db.ast.as_ctor_form(pat, "list"))
            .unwrap_or(&[])
            .to_vec();
        // Split off a trailing `.. rest`: a `..` MARKER followed by exactly one binder as the final two
        // elements. `lead` = the fixed leading element patterns; `has_rest` = a tail-binding rest pattern.
        let dotdot = raw.iter().position(|&e| db.ast.as_name(e) == Some(".."));
        let (leads, has_rest): (&[StructId], bool) = match dotdot {
            Some(k) if k + 2 == raw.len() => (&raw[..k], true), // `(list p… .. rest)` — well-formed
            Some(_) => {
                // A `..` that is not the second-to-last element is malformed (a rest binds the whole tail,
                // so it must be final). CDZ0201 — the same shape rule a top-level list pattern enforces.
                return Err(Reject::coded(
                    Code::Malformed,
                    "a list rest pattern `.. rest` must be the final element",
                ));
            }
            None => (&raw[..], false),
        };
        let elem_ty = match ty {
            crate::ty::Ty::List(e) => (**e).clone(),
            crate::ty::Ty::Any => crate::ty::Ty::Any,
            _ => {
                return Err(Reject::coded(
                    Code::Malformed,
                    format!(
                        "a list pattern does not match the payload type {}",
                        ty.render_name()
                    ),
                ));
            }
        };
        // The LENGTH test — exactly `leads.len()` for a fixed pattern, AT LEAST `leads.len()` when a
        // `.. rest` binds the tail. Gated like a literal test (folded against a constant `Core::ListNew`);
        // a mismatch falls through.
        lit_tests.push((
            path.clone(),
            crate::core::Probe::ListLen {
                len: leads.len(),
                at_least: has_rest,
            },
        ));
        let mut out = Vec::new();
        for (i, &elem) in leads.iter().enumerate() {
            let mut deeper = path.clone();
            deeper.push(crate::core::PathStep::Elem(i));
            out.extend(pattern_constraints(db, elem, &elem_ty, deeper, lit_tests)?);
        }
        return Ok(out);
    }
    // A compound pattern. Its head is the variant CONSTRUCTOR — a member `(. Sum V)` or a bare variant
    // name — and the remaining children are payload sub-patterns.
    let (head, args): (StructId, Vec<StructId>) = match db.ast.get(pat) {
        crate::ast::Struct::List(children) => match children.first().copied() {
            // A bare member `(. Sum V)` used as a whole pattern — the ctor, no payload args.
            Some(first) if db.ast.as_name(first) == Some(".") => (pat, Vec::new()),
            Some(first) => (first, children[1..].to_vec()),
            None => return Err(Reject::decline("an empty sum match pattern")),
        },
        crate::ast::Struct::Atom(_) => {
            return Err(Reject::decline("a malformed sum match pattern"));
        }
    };
    // A BARE variant-name head that COLLIDES with a prelude entry (`(Int n)` on `(type T (Int Int64))`,
    // `(Some n)` on a user `(type T (Some …))`) resolves — via scope→def→PRELUDE — to the prelude `Int`
    // type constructor / Option `Some`, NOT this sum's variant, so the ctor check below would reject a
    // well-formed pattern (CDZ0203). The SCRUTINEE's type is known here, so its variant set disambiguates:
    // if the bare head names a variant of THIS sum/nominal, resolve it to that variant's CACHED ctor
    // occurrence (the same node the qualified `T.Int` form uses, which already carries the right `(meta t)`
    // scheme + `(meta variant)` disc) and use THAT as the head. This gives the bare form the same
    // local-variant precedence the qualified form has — the residual of the variant-shadows-prelude fix
    // (`9f326a2d` repaired TYPE/MODULE positions; this repairs the CONSTRUCT/PATTERN head). A NON-colliding
    // bare name already resolves to its own variant, so `variant_disc_by_name` finding it and re-reading
    // the SAME cached ctor is a harmless no-op; a bare name that is NOT a variant (a typo) is left for the
    // existing ctor check to reject.
    let head = 'remap: {
        let Some(name) = db.ast.as_name(head).map(str::to_string) else {
            break 'remap head;
        };
        if name == "." {
            break 'remap head;
        }
        // The scrutinee's declaration — a boxed `Ty::Sum` OR a single-variant `Ty::Nominal` newtype (a
        // `(type T (Int Int64))` erases to a nominal, whose sole variant is still reached by name).
        let decl = match ty {
            crate::ty::Ty::Sum { decl, .. } | crate::ty::Ty::Nominal { decl, .. } => *decl,
            _ => break 'remap head,
        };
        // The cached ctor of the variant of THIS declaration named `name` (if any). Resolving to it gives
        // the bare form the local-variant precedence the qualified `T.<name>` already has.
        match db
            .type_decl_by_occ(decl)
            .and_then(|t| t.variants.iter().find(|v| v.name == name))
            .and_then(|v| v.ctor)
        {
            Some(ctor) => ctor,
            None => head,
        }
    };
    // A NOMINAL NEWTYPE scrutinee — the sole constructor `(Mk arg…)` imposes NO discriminant constraint
    // (a newtype has no runtime disc; its one variant always matches), but its payload binders DO
    // destructure. The ctor must belong to THIS newtype's declaration (a `(Other x)` pattern over a
    // `UserId` scrutinee is a type error, CDZ0203 — same as the boxed-sum check below). The payload
    // descends at `path + [Payload]`, which `erase_nominal_steps` later drops as a no-op; the payload
    // type is the nominal's `inner` (single payload) or its tuple elements (multi-payload struct).
    if let crate::ty::Ty::Nominal {
        decl: scrut_decl,
        inner,
        ..
    } = ty
    {
        if crate::eval::variant_owner_decl(db, head) != Some(*scrut_decl) {
            return Err(Reject::coded(
                Code::TypeMismatch,
                format!(
                    "this constructor pattern is not the constructor of the matched type {}",
                    ty.render_name()
                ),
            ));
        }
        let inner = (**inner).clone();
        return match args.len() {
            // A bare `(Mk)` / member `(. T Mk)` with no payload arg — nothing to bind (a unit newtype).
            0 => Ok(Vec::new()),
            // `(Mk n)` — bind the single payload at `[Payload]` (erased later), typed as `inner`.
            1 => {
                let mut deeper = path;
                deeper.push(crate::core::PathStep::Payload);
                pattern_constraints(db, args[0], &inner, deeper, lit_tests)
            }
            // `(Mk a b …)` over a multi-payload struct — the payload is `inner` = `Ty::Tuple`; each arg
            // destructures an element at `[Payload, Elem(i)]` (the `Payload` erases, the `Elem` reads the
            // tuple handle). Arity is checked against the tuple below via the shared descent.
            _ => {
                let elem_tys: Vec<crate::ty::Ty> = match &inner {
                    crate::ty::Ty::Tuple(ts) if ts.len() == args.len() => ts.to_vec(),
                    crate::ty::Ty::Tuple(ts) => {
                        return Err(Reject::coded(
                            Code::Malformed,
                            format!(
                                "this constructor pattern binds {} payload(s), but the newtype carries {}",
                                args.len(),
                                ts.len()
                            ),
                        ));
                    }
                    _ => {
                        return Err(Reject::coded(
                            Code::Malformed,
                            format!(
                                "this constructor pattern binds {} payloads, but the newtype's payload is {}",
                                args.len(),
                                inner.render_name()
                            ),
                        ));
                    }
                };
                let mut payload_path = path;
                payload_path.push(crate::core::PathStep::Payload);
                let mut out = Vec::new();
                for (i, (&arg, elem_ty)) in args.iter().zip(elem_tys.iter()).enumerate() {
                    let mut deeper = payload_path.clone();
                    deeper.push(crate::core::PathStep::Elem(i));
                    out.extend(pattern_constraints(db, arg, elem_ty, deeper, lit_tests)?);
                }
                Ok(out)
            }
        };
    }
    let Some(disc) = crate::eval::variant_disc_of(db, head) else {
        // The head names no variant. A `(. Sum Q)` head where `Q` is not a variant of the sum
        // (`((V.Q) …)` on a `(type V (A …) (B))`) lowers as a MEMBER ACCESS that already carries the
        // precise coded fault — `CDZ0201: record has no field \`Q\`` (a sum record's variants ARE its
        // fields), the SAME code the value position `(V.Q)` gets. Propagate that coded poison rather than
        // the generic UNCODED "not a variant constructor" decline, so a mistyped variant in a match
        // pattern NAMES the offending variant and is graded a rejection (not a to-do).
        if let Core::Poison(reject) = core_of(db, head)
            && reject.code.is_some()
        {
            // ENRICH with a "did you mean?" over the SCRUTINEE sum's variant names — the pattern-position
            // twin of `infer::no_field_reject`'s value-position suggestion. `core_of(head)` (a member fold)
            // emits the BARE `record has no field \`Q\``; here we know the scrutinee's sum type, so we can
            // name the nearest variant (`((V.Alph) …)` on `(type V (Alpha) (Beta))` → "did you mean
            // `Alpha`?") + carry a replace fix on the mistyped key occurrence, exactly as the value site.
            return Err(enrich_pattern_head_suggestion(db, head, ty, reject));
        }
        return Err(Reject::decline(
            "a sum match pattern head is not a variant constructor",
        ));
    };
    // TYPE-CHECK the pattern's constructor against the SCRUTINEE's sum type: the variant must belong to
    // the sum being matched, not merely be SOME sum's variant with the right name. A `Some`/`U.A` pattern
    // over a `T` scrutinee resolves to a valid discriminant of Option/U, but that variant is not T's — a
    // type confusion that would bind the payload under the wrong type (a wrong VALUE, or an INVALID WASM
    // component when the payload widths differ). Sum identity is by DECLARATION OCCURRENCE (`ty.rs`
    // §Two sums are the SAME type iff their `decl` agree), so compare the pattern ctor's owning `decl`
    // against the scrutinee `ty`'s `decl` — a mismatch is CDZ0203, the same type error `(: 5 Bool)` gets.
    // (A bare nullary-variant name took the `variant_disc_by_name` path above, which is already scoped to
    // this sum's declaration, so only a COMPOUND ctor pattern reaches here needing the check.)
    if let crate::ty::Ty::Sum {
        decl: scrut_decl, ..
    } = ty
        && crate::eval::variant_owner_decl(db, head) != Some(*scrut_decl)
    {
        return Err(Reject::coded(
            Code::TypeMismatch,
            format!(
                "this variant pattern is not a variant of the matched type {}",
                ty.render_name()
            ),
        ));
    }
    let mut out = vec![(path.clone(), disc)];
    // Recurse into the payload. A single-payload variant `(Some p)` descends into `p` at `path +
    // [Payload]`; the payload's TYPE is the variant's payload type at this instantiation, so a nested
    // variant name there resolves against the right sum. A NULLARY variant pattern `(None)`/bare `None`
    // has no payload arg — nothing to recurse.
    match args.len() {
        0 => {}
        1 => {
            let payload_ty = crate::infer::payload_ty_at_instantiation(db, head, ty)
                .unwrap_or(crate::ty::Ty::Any);
            let mut deeper = path;
            deeper.push(crate::core::PathStep::Payload);
            let sub = pattern_constraints(db, args[0], &payload_ty, deeper, lit_tests)?;
            out.extend(sub);
        }
        // A MULTI-PAYLOAD variant pattern `(Cons h t)` is sugar for the single-tuple-payload form `(Cons
        // (tuple h t))`: the payloads are boxed as ONE tuple handle (`lower_sum_new` / the `SumNew`
        // backend), so `payload_ty_at_instantiation` reports the payload as a `Ty::Tuple`, and each arg
        // destructures a tuple ELEMENT at `path + [Payload, Elem(i)]` — exactly the descent the explicit
        // `(tuple …)` payload pattern takes.
        _ => {
            let payload_ty = crate::infer::payload_ty_at_instantiation(db, head, ty)
                .unwrap_or(crate::ty::Ty::Any);
            // The pattern's payload ARITY must match the variant's declared payload count — `(Mk a b c)`
            // against a 2-payload `Mk` names a nonexistent third element (it would read past the payload
            // tuple and bind `c` under an `Any`/wrong type — a wrong value, or invalid wasm). REJECT it
            // (CDZ0201), the same arity check the explicit `(tuple …)` payload pattern enforces above. An
            // `Any` payload (unsolved) can't be arity-checked — descend permissively (each `Any`).
            let elem_tys: Vec<crate::ty::Ty> = match &payload_ty {
                crate::ty::Ty::Tuple(ts) if ts.len() == args.len() => ts.to_vec(),
                crate::ty::Ty::Tuple(ts) => {
                    return Err(Reject::coded(
                        Code::Malformed,
                        format!(
                            "this variant pattern binds {} payload(s), but the variant carries {}",
                            args.len(),
                            ts.len()
                        ),
                    ));
                }
                crate::ty::Ty::Any => vec![crate::ty::Ty::Any; args.len()],
                // A non-tuple payload type under a multi-arg pattern is an arity error too (a single-payload
                // variant matched with several binders).
                _ => {
                    return Err(Reject::coded(
                        Code::Malformed,
                        format!(
                            "this variant pattern binds {} payloads, but the variant's payload is {}",
                            args.len(),
                            payload_ty.render_name()
                        ),
                    ));
                }
            };
            let mut payload_path = path;
            payload_path.push(crate::core::PathStep::Payload);
            for (i, (&arg, elem_ty)) in args.iter().zip(elem_tys.iter()).enumerate() {
                let mut deeper = payload_path.clone();
                deeper.push(crate::core::PathStep::Elem(i));
                let sub = pattern_constraints(db, arg, elem_ty, deeper, lit_tests)?;
                out.extend(sub);
            }
        }
    }
    Ok(out)
}

/// Whether `id` is a tuple PATTERN `(tuple …)` — a `tuple` NAME head (the alias the reader keeps in a
/// pattern) or the `"tuple"` string-literal primitive. Mirrors `resolve::is_tuple_pattern` (kept local
/// so lower does not depend on resolve's private helpers).
fn is_tuple_pattern(db: &Db, id: StructId) -> bool {
    db.ast.as_form(id, "tuple").is_some() || db.ast.head_ctor(id) == Some("tuple")
}

/// Whether `id` is a list PATTERN `(list p0 p1…)` — a `list` NAME head (the shadowable alias the reader
/// keeps) or the `"list"` string-literal primitive. Routes a variant's list payload into element-by-
/// element descent (`pattern_constraints`'s list arm), the list analogue of [`is_tuple_pattern`].
fn is_list_pattern(db: &Db, id: StructId) -> bool {
    db.ast.as_form(id, "list").is_some() || db.ast.head_ctor(id) == Some("list")
}

/// The element occurrences of `id` when it is a tuple CONSTRUCTOR expression — the symbol-headed
/// `Resolved::Tuple { elems }` or the `tuple` NAME-alias application (`Prim::TupleNew`). `None` for a
/// non-tuple. Used by `type_at_path` to type a tuple-scrutinee's element from the constructor directly,
/// bypassing the aggregate `type_of` that reads a recursive-call element as `Any`.
fn tuple_constructor_elems(db: &mut Db, id: StructId) -> Option<Vec<StructId>> {
    match resolved_of(db, id) {
        Resolved::Tuple { elems } => Some(elems.to_vec()),
        Resolved::Apply { head, args }
            if crate::eval::meta_apply_of(db, head) == Some(Prim::TupleNew) =>
        {
            Some(args.to_vec())
        }
        _ => None,
    }
}

/// The CDZ0210 non-exhaustive-sum-match rejection, enriched with the MISSING variants and a structural
/// "add the missing arms" fix (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A
/// Fix — the match analogue of rustc's `error[E0004]: … patterns not covered` + its "add arms"
/// suggestion). `decl` is the scrutinee sum's declaration occurrence; `tested` the discriminants the
/// arms already cover; `scrutinee` the match's scrutinee node (its parent IS the `(match …)` form the
/// insert targets). The fix is Heuristic — the arm SHAPES cover the gap (applying makes the match
/// exhaustive), but their BODIES are `(trap "TODO: …")` placeholders the author fills.
fn non_exhaustive_sum_reject(
    db: &Db,
    decl: StructId,
    tested: &[u32],
    scrutinee: StructId,
) -> Reject {
    let generic = "a sum match must cover every variant or end in a wildcard `_` (non-exhaustive)";
    let Some(t) = db.type_decl_by_occ(decl) else {
        return Reject::coded(Code::NonExhaustive, generic);
    };
    // The variants whose discriminant no arm tested, in declaration order (a deterministic list).
    let missing: Vec<&crate::db::Variant> = t
        .variants
        .iter()
        .enumerate()
        .filter(|(i, _)| !tested.contains(&(*i as u32)))
        .map(|(_, v)| v)
        .collect();
    if missing.is_empty() {
        return Reject::coded(Code::NonExhaustive, generic);
    }
    // Name the missing variants in the message (rustc "patterns `X` and `Y` not covered").
    let names: Vec<String> = missing.iter().map(|v| format!("`{}`", v.name)).collect();
    let message = format!(
        "non-exhaustive match: pattern{} {} not covered",
        if missing.len() == 1 { "" } else { "s" },
        join_and(&names),
    );
    // One arm per missing variant. A nullary variant → `(Name <body>)`; a payload variant → bind each
    // payload with a fresh `_`-prefixed name so the arm is well-formed AND does not itself warn unused:
    // `((Some _p0) <body>)`. The body is `(trap "TODO: <variant>")` — a DIVERGING placeholder the author
    // replaces. `trap : ∀a. String → a`, so it type-checks in ANY arm whatever the sibling arms' result
    // type is; a bare `unit` body cascaded to a CDZ0203 "match arms differ: T vs Unit" the moment the
    // other arms were not Unit-typed (trading one fault for another — a fix must resolve in ONE shot,
    // `spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). The message names the
    // variant so the author sees which case is stubbed.
    let arms: Vec<String> = missing
        .iter()
        .map(|v| {
            if v.payloads.is_empty() {
                format!("({} (trap \"TODO: {}\"))", v.name, v.name)
            } else {
                let binders: Vec<String> =
                    (0..v.payloads.len()).map(|i| format!("_p{i}")).collect();
                format!(
                    "(({} {}) (trap \"TODO: {}\"))",
                    v.name,
                    binders.join(" "),
                    v.name
                )
            }
        })
        .collect();
    // The `(match …)` form is the scrutinee's parent — the list the arms append into.
    match db.parent_of(scrutinee) {
        Some(match_form) => Reject::coded(Code::NonExhaustive, message)
            .with_fix(Fix::insert_arms_heuristic(match_form, arms)),
        None => Reject::coded(Code::NonExhaustive, message),
    }
}

/// Join names as `a`, `a and b`, or `a, b, and c` — the English list a "not covered" message reads
/// naturally with (matching rustc's phrasing).
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// The CDZ0210 non-exhaustive-SCALAR-match rejection, enriched with an "add the covering arm" fix (the
/// scalar analogue of `non_exhaustive_sum_reject` — `spec/capabilities/diagnostics.md` §A Diagnostic
/// Carries A Route To A Fix). A BOOL scrutinee missing a literal (`bool_true`/`bool_false` = whether
/// each is covered by an unguarded arm) is a FINITE gap: name + insert exactly the missing
/// `(true (trap …))` / `(false (trap …))` arm, like a missing sum variant. Any OTHER scalar (an open
/// Int/String, or a Bool with neither literal) is closed only by a wildcard: insert `(_ (trap …))`. The
/// arm bodies are `(trap "TODO: …")` — a DIVERGING placeholder (`trap : ∀a. String → a`) that type-checks
/// in ANY arm whatever the sibling arms return; a bare `unit` body cascaded to a CDZ0203 "match arms
/// differ: T vs Unit" the moment the other arms were not Unit-typed. Heuristic (the author fills the
/// body). Anchored at the `(match …)` form (parent of the scrutinee); falls back to the plain reject (no
/// fix) if that parent is absent.
fn non_exhaustive_scalar_reject(
    db: &Db,
    scrutinee: StructId,
    scrut_ty: &crate::ty::Ty,
    bool_true: bool,
    bool_false: bool,
) -> Reject {
    // A Bool scrutinee with exactly one literal covered → the missing one is a KNOWN, finite gap.
    let is_bool = scrut_ty.agrees_with(&crate::ty::Ty::Bool);
    let (message, arms) = if is_bool && (bool_true ^ bool_false) {
        let missing = if bool_true { "false" } else { "true" };
        (
            format!("non-exhaustive match: `{missing}` is not covered"),
            vec![format!("({missing} (trap \"TODO: {missing}\"))")],
        )
    } else {
        // An open scalar (or a Bool with neither literal) — only a wildcard closes it.
        (
            "non-exhaustive match: add a wildcard `_` arm to cover the remaining values"
                .to_string(),
            vec!["(_ (trap \"TODO\"))".to_string()],
        )
    };
    match db.parent_of(scrutinee) {
        Some(match_form) => Reject::coded(Code::NonExhaustive, message)
            .with_fix(Fix::insert_arms_heuristic(match_form, arms)),
        None => Reject::coded(Code::NonExhaustive, message),
    }
}

/// The discriminant of the variant named `name` in the sum `ty`, or `None` if `ty` is not a sum or has
/// no such variant. This is what distinguishes a bare NULLARY-VARIANT pattern (`None` against `Option`)
/// from a binder (`x`) — the name is looked up in the scrutinee sum's own declaration (occurrence-keyed,
/// so a same-named variant in another sum does not leak in).
fn variant_disc_by_name(db: &mut Db, ty: &crate::ty::Ty, name: &str) -> Option<u32> {
    let decl = match ty {
        crate::ty::Ty::Sum { decl, .. } => *decl,
        _ => return None,
    };
    let t = db.type_decl_by_occ(decl)?;
    t.variants
        .iter()
        .position(|v| v.name == name)
        .map(|i| i as u32)
}

/// A map from an access PATH to the solved TYPE of the sub-value there — populated as the tree descends
/// (the root `[]` maps to the scrutinee type; entering a variant arm at `switch_path` extends it with
/// that variant's payload type at `switch_path + [Payload]`). Keyed per-branch (not global), because the
/// SAME path under different parent variants has different types (`Result`'s `[Payload]` is `a` in the
/// `Ok` arm, `e` in the `Err` arm) — a global map would collide; a branch-local one is always consistent.
type PathTypes = std::collections::HashMap<Vec<crate::core::PathStep>, crate::ty::Ty>;

/// Compile a pattern MATRIX (`rows`) into a decision-tree CONTINUATION for the value at `scrutinee`. If
/// the FIRST row is a catch-all (no constraints), it matches unconditionally → its body is the leaf (later
/// rows unreachable). Otherwise switch on the discriminant at the SHALLOWEST path any row constrains:
/// gather the discs tested there in source order, and for each build a specialized sub-matrix — rows
/// constraining that path with this disc (constraint removed) PLUS rows not constraining it (they match
/// any disc, flowing into every arm) — then recurse. A default arm (`disc: None`) covers the rows that
/// don't constrain the switch path. Exhaustiveness is checked at EACH switch (every variant tested, or a
/// default). A constant sub-value FOLDS to the matching arm's continuation (no runtime switch).
fn build_tree(
    db: &mut Db,
    scrutinee: StructId,
    rows: &[MatchRow],
    path_types: &PathTypes,
) -> Result<crate::core::SumCont, Reject> {
    // The FIRST row whose discriminant constraints are all satisfied (empty) is at a LEAF position. If it
    // is UNGUARDED it matches unconditionally → its body is the leaf (later rows unreachable). If it is
    // GUARDED, it fires only when its guard holds; on a false guard control FALLS THROUGH to the rest of
    // this sub-matrix (`build_tree` of the remaining rows) — the per-variant fall-through a guarded arm
    // needs. A guarded leaf does NOT terminate the matrix, so the fall-through must independently be
    // exhaustive (an unguarded arm of the same variant, or the default, below it).
    match rows.first() {
        None => {
            return Err(Reject::coded(
                Code::NonExhaustive,
                "a sum match must cover every variant or end in a wildcard `_` (non-exhaustive)",
            ));
        }
        // A row whose discriminant constraints are all satisfied but that still carries LITERAL TESTS is
        // at a leaf gated by those tests: `(Some 0)` reaches here (after the `Some` switch) with a pending
        // `([Payload], Int(0))`. Emit a `LitTest` — test the sub-value at `path` against the literal; on a
        // match, CONTINUE with that test dropped (further lit-tests / the guard / the body); on a MISMATCH,
        // FALL THROUGH to the remaining rows (the same-variant binding arm `(Some k)`), exactly as a guard
        // threads its `else`. A literal test does NOT count toward exhaustiveness — the fall-through must
        // cover the variant. FOLD when the tested sub-value is a compile-time constant (a constant
        // scrutinee): a matching literal drops the test, a non-matching one skips to the fall-through
        // WITHOUT emitting the body — the constant-match half of corpus "nested patterns with literals".
        Some(row) if row.constraints.is_empty() && !row.lit_tests.is_empty() => {
            let (lit_path, probe) = row.lit_tests[0].clone();
            // The row with this first literal test consumed (its other tests / guard / body remain).
            let mut matched_row = row.clone();
            matched_row.lit_tests.remove(0);
            let mut matched_rows = vec![matched_row];
            matched_rows.extend_from_slice(&rows[1..]);
            // FOLD against a constant sub-value.
            if let Some(c) = const_at_path(db, scrutinee, &lit_path) {
                let hit = match (&probe, &c) {
                    (crate::core::Probe::Int(v), Core::ConstInt(cv)) => v.eq_value(cv),
                    (crate::core::Probe::Bool(b), Core::ConstBool(cb)) => b == cb,
                    // A string-literal payload test folds against a constant `Core::ConstStr` by value
                    // equality (both NFC-normalized by the reader) — `(Ast.Name "+")` matches an
                    // `Ast.Name` carrying "+". A runtime string payload has no `ConstStr` → declines below.
                    (crate::core::Probe::Str(s), Core::ConstStr(cs)) => s == cs,
                    // A LIST length test folds against a CONSTANT list: an exact test needs `== len`, a
                    // rest (`at_least`) test needs `>= len` (the tail binds the surplus). (A runtime list
                    // has no `ListNew` here → the runtime-test arm below, which declines.)
                    (crate::core::Probe::ListLen { len, at_least }, Core::ListNew { elems }) => {
                        if *at_least {
                            elems.len() >= *len
                        } else {
                            elems.len() == *len
                        }
                    }
                    // A non-constant / type-mismatched sub-value can't fold — emit the runtime test.
                    _ => {
                        return build_lit_test(
                            db,
                            scrutinee,
                            lit_path,
                            probe,
                            &matched_rows,
                            &rows[1..],
                            path_types,
                        );
                    }
                };
                if hit {
                    return build_tree(db, scrutinee, &matched_rows, path_types);
                } else {
                    return build_tree(db, scrutinee, &rows[1..], path_types);
                }
            }
            return build_lit_test(
                db,
                scrutinee,
                lit_path,
                probe,
                &matched_rows,
                &rows[1..],
                path_types,
            );
        }
        Some(row) if row.constraints.is_empty() && row.guard.is_none() => {
            return Ok(crate::core::SumCont::Leaf(row.body));
        }
        Some(row) if row.constraints.is_empty() => {
            // A GUARDED leaf: `if guard then body else <fall-through over the remaining rows>`.
            let cond = row.guard.expect("matched the guarded arm");
            let body = row.body;
            // FOLD the guard when it is a compile-time-constant bool (a constant scrutinee makes its
            // payload binders constant, so `(> x 0)` over `x = 0` folds to `false`). A true guard SELECTS
            // the body directly; a false guard SKIPS to the fall-through tree — WITHOUT lowering the body.
            // This shields a body that would TRAP when folded (`(/ 10 x)` at `x = 0` → CDZ0304) from being
            // evaluated when its guard is false: the guard short-circuits the body exactly as `and`/`or`
            // and `if` shield an untaken branch (core-semantics.md §Boolean Connectives Short-Circuit).
            // Without this fold, a false-guarded arm's trapping body raised a SPURIOUS CDZ0304 for an arm
            // that never runs. A guard reading a RUNTIME value does not fold → the runtime `Guarded` cont.
            match core_of(db, cond) {
                Core::ConstBool(true) => {
                    // The guard folds TRUE, so this arm fires and its body is the value. But a guarded arm
                    // does NOT count toward exhaustiveness (core-semantics.md §Matching Is Exhaustive Or
                    // Rejected: "a guarded arm may be false, so it covers no variant"), and the match must
                    // be well-formed AS WRITTEN — a non-exhaustive match is CDZ0210 regardless of whether a
                    // constant scrutinee happens to satisfy a guard. So verify the fall-through `rows[1..]`
                    // still forms an exhaustive cover BEFORE folding to the body: `build_tree` on it
                    // surfaces CDZ0210 if the variant is otherwise uncovered (a bare `((guard (Some x) …)
                    // (None -1))` — `Some` covered ONLY by the guarded arm — must reject, matching the
                    // standalone-emitted body). The check's RESULT is discarded (we still fold to `body`
                    // when the scrutinee satisfies the guard); only its error propagates. This keeps the
                    // fold consistent with the runtime `Guarded` path below, which builds `els` (and thus
                    // checks the fall-through) unconditionally.
                    let _ = build_tree(db, scrutinee, &rows[1..], path_types)?;
                    return Ok(crate::core::SumCont::Leaf(body));
                }
                Core::ConstBool(false) => return build_tree(db, scrutinee, &rows[1..], path_types),
                _ => {}
            }
            let els = build_tree(db, scrutinee, &rows[1..], path_types)?;
            return Ok(crate::core::SumCont::Guarded {
                cond,
                body,
                els: Box::new(els),
            });
        }
        _ => {}
    }
    // Pick the SWITCH path — the shallowest path any row constrains (outer patterns first, so the outer
    // probe is shared). Its TYPE gives the variant set for exhaustiveness + recursion. Read from
    // `path_types` (populated as sum-variant arms descend), else COMPUTE it by walking the path from the
    // scrutinee's own type — a `Ty::Tuple` element indexes at `Elem(i)`, so a sum nested in a TUPLE element
    // (`(match (tuple a b) ((tuple (E.Lit x) …)…))`, switch path `[Elem(0)]`) resolves even though no
    // sum-payload descent seeded it. (`path_types` still wins where present — a variant payload's
    // instantiated type is more precise than a raw type-walk.)
    let switch_path = shallowest_path(rows);
    let sub_ty = match path_types.get(&switch_path) {
        Some(t) => t.clone(),
        // Not seeded exactly: try a raw type-walk from the scrutinee, then (for a path that descends
        // through a boxed-sum `Payload` a raw walk can't cross) walk the SUFFIX from the longest seeded
        // PREFIX in `path_types` — a list-element switch `[Payload, Elem(1)]` resolves from the seeded
        // `[Payload]` = `(List Ast)` even though the raw `Payload` walk over the boxed sum returns None.
        None => match type_at_path(db, scrutinee, &switch_path)
            .or_else(|| type_from_seeded_prefix(path_types, &switch_path))
        {
            Some(t) => t,
            None => {
                return Err(Reject::decline(
                    "compound match switch path has no solved type",
                ));
            }
        },
    };
    let (decl, variant_count) = match &sub_ty {
        crate::ty::Ty::Sum { decl, .. } => match db.type_decl_by_occ(*decl) {
            Some(t) => (*decl, t.variants.len()),
            None => return Err(Reject::decline("sum match sub-value has no declaration")),
        },
        _ => {
            return Err(Reject::decline(
                "sum match dispatches on a non-sum sub-value",
            ));
        }
    };
    // Partition the matrix by the disc each row tests at `switch_path` in ONE pass (was one O(N) scan per
    // arm via `specialize` → O(N²) over N arms; the `tested.contains` loop was O(N²) too). Each row either
    // tests `switch_path` with some disc `d` (it belongs ONLY to arm `d`, with that now-satisfied
    // constraint dropped) or does NOT test it (a DEFAULT row — it flows into EVERY arm AND the default
    // arm, unchanged). Rows keep their source index so an arm's sub-matrix preserves source order (arm
    // priority = first-matching-row) when disc rows and default rows interleave.
    let mut tested: Vec<u32> = Vec::new();
    let mut disc_rows: crate::fxhash::FxHashMap<u32, Vec<(usize, MatchRow)>> = Default::default();
    let mut default_rows: Vec<(usize, MatchRow)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        match row.constraints.iter().find(|(p, _)| *p == switch_path) {
            Some((_, d)) => {
                let d = *d;
                let bucket = disc_rows.entry(d).or_insert_with(|| {
                    tested.push(d);
                    Vec::new()
                });
                bucket.push((
                    i,
                    MatchRow {
                        // Drop the now-satisfied `switch_path` constraint (control is in this arm).
                        constraints: row
                            .constraints
                            .iter()
                            .filter(|(p, _)| *p != switch_path)
                            .cloned()
                            .collect(),
                        lit_tests: row.lit_tests.clone(),
                        body: row.body,
                        guard: row.guard,
                    },
                ));
            }
            None => default_rows.push((
                i,
                MatchRow {
                    constraints: row.constraints.clone(),
                    lit_tests: row.lit_tests.clone(),
                    body: row.body,
                    guard: row.guard,
                },
            )),
        }
    }
    // The switched sub-value's STATICALLY-KNOWN discriminant, if any — a `SumNew` core at `switch_path`
    // has a fixed disc EVEN when its payload is a runtime value (`(Some n)` is `SumNew{Some, [n]}`: the
    // `Some` tag is known, only `n` is runtime). It drives the FOLD below (pick the known arm, no runtime
    // switch). It does NOT relax exhaustiveness: `core-semantics.md §Matching Is Exhaustive Or Rejected`
    // (corpus 02 "a sum match missing a variant is non-exhaustive EVEN when the scrutinee is the covered
    // one") makes exhaustiveness a property of the ARM SET against the TYPE's variant set, never of which
    // variant the scrutinee holds — a value-driven shortcut that skips the check because the constant hit
    // a present arm is exactly what that case forbids.
    let known_disc = match const_at_path(db, scrutinee, &switch_path) {
        Some(Core::SumNew { disc, .. }) => Some(disc),
        _ => None,
    };
    // Exhaustiveness: every variant tested, or a default (wildcard/binder) present — else CDZ0210. Against
    // the TYPE's variant set, independent of `known_disc` (see above).
    let has_default = !default_rows.is_empty();
    if !has_default && tested.len() < variant_count {
        // Name the missing variants + carry an "add the missing arms" fix — but ONLY at the ROOT switch
        // (`switch_path` empty): there the missing-variant arms append directly to the `(match …)` form
        // and are well-formed top-level patterns. A NESTED non-exhaustive (a gap inside a payload
        // pattern) would need arms shaped to the nesting, which the flat append cannot express, so it
        // keeps the enriched message but no fix (the `db.parent_of(scrutinee)` there is not the match).
        if switch_path.is_empty() {
            return Err(non_exhaustive_sum_reject(db, decl, &tested, scrutinee));
        }
        return Err(Reject::coded(
            Code::NonExhaustive,
            "a sum match must cover every variant or end in a wildcard `_` (non-exhaustive)",
        ));
    }
    // One arm per tested discriminant, then the default arm (if any). Each arm's sub-matrix merges its
    // disc rows with the default rows by source index (both already ascending), recursing under a
    // `path_types` extended with THIS variant's payload type at `switch_path+[Payload]`.
    let mut sum_arms: Vec<crate::core::SumArm> = Vec::new();
    for &d in &tested {
        let own = disc_rows.remove(&d).unwrap_or_default();
        let sub_rows = merge_rows(own, &default_rows);
        let child_types = extend_path_types(db, path_types, &switch_path, &sub_ty, decl, d);
        let cont = build_tree(db, scrutinee, &sub_rows, &child_types)?;
        sum_arms.push(crate::core::SumArm {
            disc: Some(d),
            cont,
        });
    }
    if has_default {
        // The default arm switches on nothing new at `switch_path` — its rows only reach paths they
        // already constrain (all in `path_types`), so no extension is needed.
        let sub_rows: Vec<MatchRow> = default_rows.into_iter().map(|(_, r)| r).collect();
        let cont = build_tree(db, scrutinee, &sub_rows, path_types)?;
        sum_arms.push(crate::core::SumArm { disc: None, cont });
    }
    // FOLD when the switched sub-value's discriminant is STATICALLY KNOWN (a `SumNew` core — its tag is
    // fixed even if its payload is runtime): pick the matching arm's continuation directly, no runtime
    // disc switch. `(match (Some n) …)` folds to the `Some` arm (whose body may still test the runtime
    // payload `n` via a `LitTest`). A scrutinee whose disc is NOT known keeps the runtime `Switch`.
    if let Some(disc) = known_disc {
        for arm in &sum_arms {
            if arm.disc.is_none() || arm.disc == Some(disc) {
                trace!(target: "rcdzc::fold", "sum match folds to a selected arm (known discriminant)");
                return Ok(arm.cont.clone());
            }
        }
    }
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, depth = switch_path.len(), arms = sum_arms.len(), "sum switch (decision-tree node)");
    Ok(crate::core::SumCont::Switch {
        path: switch_path,
        arms: sum_arms,
    })
}

/// Build a runtime `SumCont::LitTest` node: test the sub-value at `lit_path` against `probe`; on a match
/// continue with `matched_rows` (this arm with the test consumed, then the rest of the sub-matrix), on a
/// mismatch fall through to `else_rows`. Both sub-trees are compiled by `build_tree`. Split out of
/// `build_tree` so the constant-fold path (a matching/non-matching constant sub-value) and the runtime
/// path share one construction; the `then_`/`els` recursion is what lets several literal tests on one arm
/// nest and a fall-through reach the same-variant binding arm.
fn build_lit_test(
    db: &mut Db,
    scrutinee: StructId,
    lit_path: Vec<crate::core::PathStep>,
    probe: crate::core::Probe,
    matched_rows: &[MatchRow],
    else_rows: &[MatchRow],
    path_types: &PathTypes,
) -> Result<crate::core::SumCont, Reject> {
    // A `ListLen` or `Str` probe that did NOT fold (the payload is a RUNTIME value, not a constant
    // `Core::ListNew`/`Core::ConstStr`) needs a runtime length/string test the backends don't emit — the
    // runtime list/string matcher, not this increment. Decline so the match is a Todo (never emitted to
    // the backend). The CONSTANT case folded in `build_tree` and never reaches here.
    if matches!(
        probe,
        crate::core::Probe::ListLen { .. } | crate::core::Probe::Str(_)
    ) {
        return Err(Reject::decline(
            "a list/string pattern over a runtime payload is not yet supported (only a constant folds)",
        ));
    }
    let then_ = build_tree(db, scrutinee, matched_rows, path_types)?;
    let els = build_tree(db, scrutinee, else_rows, path_types)?;
    Ok(crate::core::SumCont::LitTest {
        path: lit_path,
        probe,
        then_: Box::new(then_),
        els: Box::new(els),
    })
}

/// The solved TYPE of the sub-value at `path` from `scrutinee`, computed by walking the scrutinee's own
/// type: an `Elem(i)` step indexes a `Ty::Tuple`'s i-th element; a `Payload` step descends a sum
/// variant's payload (via the head recorded... but a raw type-walk cannot know WHICH variant a `Payload`
/// step refers to, so `Payload` is only resolvable through `extend_path_types`' instantiation — this
/// fallback handles the `Elem`-only paths a TUPLE-scrutinee match produces, where `path_types` was not
/// seeded). Returns `None` for a `Payload` step (deferred to `path_types`) or an out-of-range/ill-typed
/// index. Used as the fallback when `path_types` has no entry — a sum nested in a tuple element.
fn type_at_path(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
) -> Option<crate::ty::Ty> {
    // A LEADING `Elem(i)` over a scrutinee that is a TUPLE CONSTRUCTOR — `(match (tuple (fold a) (fold b))
    // …)` — types element `i` DIRECTLY from the constructor rather than from the tuple's aggregate
    // `type_of`. `type_of((tuple (fold a) (fold b)))` types each element in AGGREGATE, where a RECURSIVE
    // call `(fold a)` (during `fold`'s own lowering) reads `Any` (the recursion guard), giving `(Tuple Any
    // Any)` → a non-sum decline at the switch. Typing the element occurrence on its OWN reaches
    // `apply_type`'s recursive-callee `def_scheme` fallback (`fold : E → E`), so `Elem(0)` resolves to `E`.
    // Only the leading `Elem` steps are peeled structurally; the rest fall through to the type-walk below.
    let mut cur = if let Some(&crate::core::PathStep::Elem(i)) = path.first()
        && let Some(elems) = tuple_constructor_elems(db, scrutinee)
        && let Some(&elem_occ) = elems.get(i)
    {
        // Descend the remaining path from this element occurrence (recurse, so a NESTED tuple constructor
        // element resolves too), then RETURN — the leading `Elem(i)` is consumed.
        return type_at_path(db, elem_occ, &path[1..]);
    } else {
        crate::infer::type_of(db, scrutinee)
    };
    for step in path {
        cur = match step {
            crate::core::PathStep::Elem(i) => match &cur {
                crate::ty::Ty::Tuple(elems) => elems.get(*i)?.clone(),
                // A LIST element (a `(list …)` payload sub-pattern) — every element has the list's one
                // element type (homogeneous), so `Elem(i)` over a `Ty::List(e)` is `e` for any `i`.
                crate::ty::Ty::List(elem) => (**elem).clone(),
                _ => return None,
            },
            // A rest sublist is the same `List` type as its scrutinee.
            crate::core::PathStep::RestFrom(_) => match &cur {
                crate::ty::Ty::List(_) => cur.clone(),
                _ => return None,
            },
            crate::core::PathStep::Payload => match &cur {
                // A `Payload` step over a NOMINAL NEWTYPE UNWRAPS the tag to its underlying type (a
                // runtime no-op). A newtype imposes NO discriminant constraint, so its `Payload` step is
                // NOT seeded in `path_types` by a variant descent — a raw type-walk must resolve it here,
                // or a switch on a sub-value INSIDE an erased newtype's payload (`(Outer.Wrap (tuple
                // (Inner.A v) k))` — switch path `[Payload, Elem(0)]` on `Inner`) has no solved type.
                crate::ty::Ty::Nominal { inner, .. } => (**inner).clone(),
                // A BOXED-sum `Payload` step's target type needs the variant instantiation
                // (`extend_path_types` seeds it in `path_types`); a raw type-walk cannot supply it.
                _ => return None,
            },
        };
    }
    Some(cur)
}

/// Resolve `path`'s type by walking its SUFFIX from the longest PREFIX present in `path_types`. Used
/// when a raw scrutinee type-walk can't cross a boxed-sum `Payload` step but `path_types` seeded a
/// prefix (e.g. `[Payload]` = a variant's payload type): the remaining `Elem` steps then walk the seeded
/// type structurally. Only `Elem` suffix steps are walked (over a `Tuple`/`List`); a further `Payload`
/// in the suffix is a nested boxed sum a plain type-walk can't cross → `None` (declines, as before).
fn type_from_seeded_prefix(
    path_types: &PathTypes,
    path: &[crate::core::PathStep],
) -> Option<crate::ty::Ty> {
    // Longest seeded prefix (try the full path down to the empty prefix).
    for cut in (0..path.len()).rev() {
        if let Some(base) = path_types.get(&path[..cut].to_vec()) {
            let mut cur = base.clone();
            for step in &path[cut..] {
                cur = match step {
                    crate::core::PathStep::Elem(i) => match &cur {
                        crate::ty::Ty::Tuple(elems) => elems.get(*i)?.clone(),
                        crate::ty::Ty::List(elem) => (**elem).clone(),
                        _ => return None,
                    },
                    // A rest sublist `.. rest` has the SAME `List` type as its scrutinee (the tail is a
                    // list of the same element type).
                    crate::core::PathStep::RestFrom(_) => match &cur {
                        crate::ty::Ty::List(_) => cur.clone(),
                        _ => return None,
                    },
                    // A `Payload` in the suffix crosses a nested boxed sum — a plain type-walk can't
                    // supply its instantiation, so decline (the same limit `type_at_path` has).
                    crate::core::PathStep::Payload => return None,
                };
            }
            return Some(cur);
        }
    }
    None
}

/// Extend `path_types` for the arm switching on variant `disc` at `switch_path` (a sum of type `sub_ty`,
/// declaration `decl`): the sub-value at `switch_path + [Payload]` has the type of THAT variant's payload
/// at `sub_ty`'s instantiation. Read via the variant's constructor record (its `(meta t)` scheme unified
/// against `sub_ty`), so a generic sum's payload is instantiated (`Ok`'s payload in `Result Int Str` is
/// `Int`). A nullary variant has no payload — no extension. The map is CLONED so sibling arms don't share.
fn extend_path_types(
    db: &mut Db,
    path_types: &PathTypes,
    switch_path: &[crate::core::PathStep],
    sub_ty: &crate::ty::Ty,
    decl: StructId,
    disc: u32,
) -> PathTypes {
    let mut out = path_types.clone();
    // The variant's constructor occurrence — via the synthesized sum record's variant field, which
    // carries the `(meta t)` scheme `payload_ty_at_instantiation` reads. (The declaration name occurrence
    // does not resolve to a scheme; the synthesized ctor field does.)
    // The variant's constructor occurrence — cached on the variant at synthesis time (O(1)), rather than
    // re-scanning the sum record's variant fields by name per arm (that was O(V) per arm → O(V²) overall).
    let ctor = db
        .type_decl_by_occ(decl)
        .and_then(|t| t.variants.get(disc as usize))
        .and_then(|v| v.ctor);
    if let Some(ctor) = ctor
        && let Some(payload_ty) = crate::infer::payload_ty_at_instantiation(db, ctor, sub_ty)
    {
        let mut child = switch_path.to_vec();
        child.push(crate::core::PathStep::Payload);
        // A MULTI-payload variant's payload is a `Ty::Tuple` (its payloads boxed as one tuple handle);
        // also register each tuple ELEMENT's type at `switch_path + [Payload, Elem(i)]` so a nested switch
        // (a variant pattern in a payload position — `(Cons h (Cons h2 rest))`) resolves its sub-value's
        // type. A single-payload variant's payload is registered at `[Payload]` alone, unchanged.
        if let crate::ty::Ty::Tuple(elems) = &payload_ty {
            for (i, elem_ty) in elems.iter().enumerate() {
                let mut elem_path = child.clone();
                elem_path.push(crate::core::PathStep::Elem(i));
                out.insert(elem_path, elem_ty.clone());
            }
        }
        out.insert(child, payload_ty);
    }
    out
}

/// The shallowest (shortest, then by `path_cmp`) path any row constrains — the switch site.
fn shallowest_path(rows: &[MatchRow]) -> Vec<crate::core::PathStep> {
    rows.iter()
        .flat_map(|r| r.constraints.iter().map(|(p, _)| p.clone()))
        .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| path_cmp(a, b)))
        .unwrap_or_default()
}

/// A total order on paths for a deterministic switch choice (Payload < Elem < RestFrom, each by index).
/// `RestFrom` never appears in a SUM decision-tree switch path (only a list-rest binder's own path, which
/// does not go through `MatchRow`), but the ordering is total so the comparator stays well-defined.
fn path_cmp(a: &[crate::core::PathStep], b: &[crate::core::PathStep]) -> std::cmp::Ordering {
    use crate::core::PathStep;
    // A rank + inner index gives a total order across all three step kinds in one comparison.
    fn key(s: &PathStep) -> (u8, usize) {
        match s {
            PathStep::Payload => (0, 0),
            PathStep::Elem(i) => (1, *i),
            PathStep::RestFrom(k) => (2, *k),
        }
    }
    for (x, y) in a.iter().zip(b.iter()) {
        let o = key(x).cmp(&key(y));
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

/// Merge an arm's OWN disc rows with the shared DEFAULT rows into one sub-matrix, preserving SOURCE order
/// (arm priority = first-matching-row). Both inputs are `(source_index, row)` already ascending by index
/// (the partition in `build_tree` pushed them in row order), so this is a linear two-way merge — no sort.
/// A default row is cloned into each arm it flows into; `own` rows are moved (each belongs to one arm).
fn merge_rows(own: Vec<(usize, MatchRow)>, defaults: &[(usize, MatchRow)]) -> Vec<MatchRow> {
    let mut out = Vec::with_capacity(own.len() + defaults.len());
    let mut oi = own.into_iter().peekable();
    let mut di = defaults.iter().peekable();
    loop {
        match (oi.peek(), di.peek()) {
            (Some((oidx, _)), Some((didx, _))) => {
                if oidx <= didx {
                    out.push(oi.next().unwrap().1);
                } else {
                    out.push(di.next().unwrap().1.clone());
                }
            }
            (Some(_), None) => out.push(oi.next().unwrap().1),
            (None, Some(_)) => out.push(di.next().unwrap().1.clone()),
            (None, None) => break,
        }
    }
    out
}

/// The compile-time-constant `Core` at `path` from `scrutinee`, if every step lands in a constant
/// compound (`SumNew` payload / `Tuple` element) — else `None` (a runtime step). Drives the constant fold
/// at each switch. Mirrors `fold_sum_path` but starts from an occurrence and returns the leaf core.
fn const_at_path(db: &mut Db, scrutinee: StructId, path: &[crate::core::PathStep]) -> Option<Core> {
    use crate::core::PathStep;
    let mut cur = scrutinee;
    for step in path {
        // A `Payload` step over a NOMINAL NEWTYPE is a no-op (the box is erased; the underlying value IS
        // `cur`) — see `fold_sum_path`. Leave `cur` unchanged and continue.
        if matches!(step, PathStep::Payload)
            && matches!(
                crate::infer::type_of(db, cur),
                crate::ty::Ty::Nominal { .. }
            )
        {
            continue;
        }
        cur = match (step, core_of(db, cur)) {
            (PathStep::Payload, Core::SumNew { payloads, .. }) if payloads.len() == 1 => {
                payloads[0]
            }
            (PathStep::Elem(i), Core::Tuple { elems }) => *elems.get(*i)?,
            // A list-pattern element binder reads position `i` of a CONSTANT list — the same `Elem` step a
            // tuple element uses, over a `Core::ListNew`. A runtime list has no `Core::ListNew` here.
            (PathStep::Elem(i), Core::ListNew { elems }) => *elems.get(*i)?,
            // A list-pattern REST binder over a CONSTANT list folds to a fresh `Core::ListNew` of the tail
            // elements (from index `k`) — a synthesized node so the tail sublist is itself constant.
            (PathStep::RestFrom(k), Core::ListNew { elems }) => {
                let tail: Vec<StructId> = elems.iter().skip(*k).copied().collect();
                return Some(Core::ListNew { elems: tail });
            }
            _ => return None,
        };
    }
    Some(core_of(db, cur))
}

/// Classify a match PATTERN occurrence into a [`Probe`], or `None` if it is not a Stage-3 scalar
/// pattern. An integer/boolean literal is a literal probe; a bare NAME (the wildcard `_`, or a BINDER
/// like `k`) always matches — a `Wild` probe. A binder differs from `_` only in scope: a reference to
/// it in the arm body resolves to the scrutinee (handled by `resolve`'s Case 5), so the PROBE is
/// identical (always matches, exhaustive tail). (A constructor / tuple / record pattern is a later
/// increment — it returns `None` here; with no sums yet, every bare name in a scalar match is a binder.)
fn classify_probe(db: &mut Db, pat: StructId) -> Option<crate::core::Probe> {
    // A bare name — the wildcard `_` OR a binder — always matches. Detected structurally (before
    // resolving, which would look the name up / poison it); the binding is a scope concern, not a probe.
    if db.ast.as_name(pat).is_some() {
        return Some(crate::core::Probe::Wild);
    }
    match resolved_of(db, pat) {
        Resolved::Int(v) => Some(crate::core::Probe::Int(v)),
        Resolved::Bool(b) => Some(crate::core::Probe::Bool(b)),
        // A STRING-literal pattern (`("hello" …)`). Only the constant-scrutinee fold uses it (a runtime
        // string match declines — `is_scalar` is Int/Bool); it is classified here so a match on a
        // constant string selects its arm.
        Resolved::Str(s) => Some(crate::core::Probe::Str(s)),
        _ => None,
    }
}

/// Whether a probe matches a constant integer scrutinee (for the fold). A `Wild` matches anything. The
/// literal comparison is BY VALUE (`eq_value`) — a folded `0` (empty magnitude) and a literal `0`
/// (`[0]`) denote the same integer, so struct `==` would wrongly miss (the parity-dispatch bug).
fn probe_matches_int(probe: &crate::core::Probe, v: &IntValue) -> bool {
    match probe {
        crate::core::Probe::Int(p) => p.eq_value(v),
        crate::core::Probe::Wild => true,
        crate::core::Probe::Bool(_)
        | crate::core::Probe::Str(_)
        | crate::core::Probe::ListLen { .. } => false,
    }
}

/// Whether a probe matches a constant boolean scrutinee (for the fold). A `Wild` matches anything.
fn probe_matches_bool(probe: &crate::core::Probe, b: bool) -> bool {
    match probe {
        crate::core::Probe::Bool(p) => *p == b,
        crate::core::Probe::Wild => true,
        crate::core::Probe::Int(_)
        | crate::core::Probe::Str(_)
        | crate::core::Probe::ListLen { .. } => false,
    }
}

/// Whether a probe matches a constant string scrutinee (for the fold). A `Wild` matches anything; a
/// string literal matches by VALUE equality (the `ConstStr` scrutinee and pattern are both already NFC-
/// normalized by the reader, so `==` is exact — the same basis as the constant `String` equality fold).
fn probe_matches_str(probe: &crate::core::Probe, s: &str) -> bool {
    match probe {
        crate::core::Probe::Str(p) => p == s,
        crate::core::Probe::Wild => true,
        crate::core::Probe::Int(_)
        | crate::core::Probe::Bool(_)
        | crate::core::Probe::ListLen { .. } => false,
    }
}

/// Whether an application HEAD is a RUNTIME function-value source that must apply via `call_indirect`
/// (a `Core::CallClosure`), rather than β-reduce at compile time — a `Param`, or a PATTERN BINDER reading
/// a runtime value out of a compound (a sum-variant payload `(match t ((T.Mk f) (f x)))`, or a
/// tuple/record element `(match t ((tuple f _) (f x)))`, which resolve to `SumPayload`/`Proj`). A
/// `Param` is always runtime. A `SumPayload`/`Proj` is runtime ONLY when the fold cannot reach the stored
/// lambda: over a CONSTANT compound the projection β-reduces to the lambda (`lambda_body` sees it) and
/// must fold — the runtime path is taken solely when `lambda_body` is `None` (a genuinely heap-held
/// closure). So this is checked AFTER the lambda-reduction attempt would have fired for a foldable head.
fn head_is_runtime_fn_value(db: &mut Db, id: StructId) -> bool {
    // A CAPTURED free variable that is a fn value — a lifted closure body applies a closure it CAPTURED
    // (`(fn (x) (f x))` where `f` is captured from an enclosing scope). Inside the lifted body `f` is a
    // runtime closure HANDLE read from the env cell (`Core::Captured`), NOT the compile-time lambda it was
    // defined from — so it must apply via `call_indirect`, not β-reduce. Checked FIRST: without this the
    // `Ref` arm below follows `f` through to its original `(fn …)` definition and reports NOT-runtime, so
    // `(f x)` mis-lowered — `f`'s handle was read as a scalar and ADDED to `x` instead of called (a
    // miscompile of a closure that captures another capturing closure).
    if db.captured_ref.contains_key(&id) {
        return true;
    }
    match resolved_of(db, id) {
        Resolved::Param { .. } => true,
        Resolved::Ref { value } => head_is_runtime_fn_value(db, value),
        // A payload/element binder — runtime iff the fold can't reduce it to a lambda (a constant compound
        // folds through the projection; a runtime one does not, so its stored closure applies indirect).
        Resolved::SumPayload { .. } | Resolved::Proj { .. } => {
            crate::eval::lambda_body(db, id).is_none()
        }
        // A record-field projection `(. rec f)` whose field TYPE is a function — the record-field analogue
        // of `Proj`'s tuple-element. When `rec` is a RUNTIME record (e.g. bound as a sum-match payload,
        // `(match h ((H.M rec) ((. rec f) x)))`) the fn field cannot fold to its lambda, so it is a runtime
        // closure handle read from the value heap and applied via `call_indirect`. FOUR gates keep this
        // from diverting things that already work: (1) it carries NO `(meta apply)` — a prelude OPERATION
        // reached by member syntax (`(. Bytes at)`, `Map.insert`) is an operator/type-builder with its own
        // prim path, NOT a runtime closure (diverting them broke every `Bytes.at`/`List.at`/… op); (2) it
        // is NOT a variant constructor — `(. Shape Rect)` is a `Member` of arrow type reached by its
        // `Prim::SumNew` path; (3) the field type is `Ty::Fn` — an ordinary DATA field read (`rec.n`) stays
        // on its folding path; (4) it does NOT reduce to a lambda — a constant record's fn field folds and
        // β-reduces. Only a genuine fn-typed field of a RUNTIME record (no prim, no ctor, no fold) is a
        // runtime closure handle.
        Resolved::Member { .. } => {
            crate::eval::meta_apply_of(db, id).is_none()
                && crate::eval::variant_disc_of(db, id).is_none()
                && matches!(crate::infer::type_of(db, id), crate::ty::Ty::Fn(_, _))
                && crate::eval::lambda_body(db, id).is_none()
        }
        _ => false,
    }
}

/// Peel a CURRIED APPLICATION SPINE `(((f a) b) c)` into its ultimate RUNTIME FUNCTION-VALUE head and
/// the full left-to-right argument list `[a, b, c]`. The curried surface `((g n) 1)` is the SAME
/// full-arity application as `(g n 1)` — each nested `Apply` head contributes its own arguments, and the
/// bottom head is the one runtime fn value they all apply to. Returns `Some((f, args))` iff that bottom
/// head is a runtime function value (a fn-typed param / heap-held closure — `head_is_runtime_fn_value`)
/// of arrow type; `None` if the head is anything else (a lambda that β-reduces, a constructor, a prim, a
/// def), so those keep their own lowering paths. The accumulated args feed ONE `call_indirect`; if their
/// count doesn't match a lifted lambda's arity (a genuine partial or over-application) it declines at
/// select rather than fabricating an intermediate closure.
fn runtime_fn_spine(db: &mut Db, id: StructId) -> Option<(StructId, Vec<StructId>)> {
    match resolved_of(db, id) {
        Resolved::Apply { head, args } => {
            // A nested application head — recurse and PREPEND the deeper spine's args (they bind to the
            // function's leading parameters), then this level's args.
            if let Some((fn_head, mut spine_args)) = runtime_fn_spine(db, head) {
                spine_args.extend_from_slice(&args);
                return Some((fn_head, spine_args));
            }
            // Bottom of the spine: the head is a direct runtime fn value applied to this level's args.
            if head_is_runtime_fn_value(db, head)
                && matches!(crate::infer::type_of(db, head), crate::ty::Ty::Fn(_, _))
            {
                return Some((head, args.to_vec()));
            }
            None
        }
        _ => None,
    }
}

/// Peel a CURRIED CONSTRUCTOR SPINE `((V a) b)` into the variant-constructor head `V` and the full
/// left-to-right payload list `[a, b]`. A sum constructor is a single-arity function (core-semantics.md
/// §A Sum Type Constructor Is A Single-Arity Function; §Functions Are Single-Arity: `(f a b)` desugars to
/// `((f a) b)`), so a multi-payload variant written curried — `((Pair 3) 4)` — is the SAME construction
/// as the flat `(Pair 3 4)`: each nested `Apply` head contributes its own arguments and the bottom head
/// is the one variant constructor they all apply to. Returns `Some((ctor, args))` iff the bottom head is
/// a variant constructor (`variant_disc_of` is `Some`) reached through at least one nested `Apply` (a
/// FLAT `(Pair 3 4)` has a non-`Apply` head and takes the ordinary `lower_sum_new` path directly — this
/// is only for the nested-parens surface). `None` for any other head, so lambdas/prims/defs keep their
/// paths. The caller checks the gathered count against the variant's payload arity before building.
fn ctor_spine(db: &mut Db, id: StructId) -> Option<(StructId, Vec<StructId>)> {
    // FUEL bounds the peel. The Ref-follow + lambda-reduction below can chain (a partial ctor bound to a
    // ref, itself the reduction of a helper); more importantly a RECURSIVE nullary def `(def (f) (f))`
    // has its head-ref point back to the SAME `(f)` apply, so an unfueled follow-and-recurse cycles
    // forever (a stack overflow). A real constructor spine is bounded by the variant's payload arity (a
    // handful); 64 is far above any genuine spine and stops the pathological cycle with a clean `None`.
    ctor_spine_fueled(db, id, 64)
}

fn ctor_spine_fueled(db: &mut Db, id: StructId, fuel: u32) -> Option<(StructId, Vec<StructId>)> {
    if fuel == 0 {
        return None;
    }
    // Follow a `let`/`def` REF to the value it binds, so a partial constructor stashed in a binding —
    // `(let ((g (Pair 3))) (g 4))`, where the head `g` refs the half-applied `(Pair 3)` — flattens the
    // same as the inline `((Pair 3) 4)`. Without this, the ref head is neither an `Apply` (so no deeper
    // spine) nor a constructor record (so no bottom), and the partial ctor reaches "not applyable". A ref
    // that cycles back to `id` (a recursive nullary self-call) is caught by the fuel bound above.
    let node = match resolved_of(db, id) {
        Resolved::Ref { value } => value,
        _ => id,
    };
    let Resolved::Apply { head, args } = resolved_of(db, node) else {
        return None;
    };
    // A nested application head — recurse to the bottom constructor and PREPEND the deeper spine's args
    // (they bind to the constructor's leading payloads), then this level's args.
    if let Some((ctor, mut spine_args)) = ctor_spine_fueled(db, head, fuel - 1) {
        spine_args.extend_from_slice(&args);
        return Some((ctor, spine_args));
    }
    // Bottom of the spine: the head is a variant constructor applied to this level's args. Reached either
    // through the recursive arm above (the OUTER `Apply`'s head is the inner `Apply`) or directly when a
    // followed ref lands on a half-applied constructor `(Pair 3)`. A genuine FLAT construction never lands
    // here — its own head is the bare `(. Sum V)` record, so `ctor_spine` on it returns `None` at the
    // `let…else` (a record is not an `Apply`) and the flat `Some(Prim::SumNew)` path builds it.
    if crate::eval::variant_disc_of(db, head).is_some() {
        return Some((head, args.to_vec()));
    }
    // The head is a LAMBDA (a def / `fn`) applied to this level's args, and that application REDUCES to a
    // constructor — `((mk1 3) 4)` where the head `(mk1 3)` applies the helper `(def (mk1 x) (Pair x))` to
    // `3`, reducing to `(Pair 3)`. β-reduce `head` over `args` under the depth guard, then peel the
    // reduced spine (the reduced `(Pair 3)` yields the bottom ctor + `[3]`); the caller's outer level
    // appends its own `[4]`. `args` are CONSUMED by the reduction, so they are NOT re-appended here. Only
    // attempted when the head is a lambda (`lambda_body` is `Some`) — a runtime-closure/prim head keeps its
    // path. `apply_lambda` DECLINES a recursive callee (returns `Err`), and the `enter_reduction` guard
    // plus the fuel bound stop a deep/cyclic chain from inlining without end.
    if crate::eval::lambda_body(db, head).is_some()
        && let Some(mut guard) = db.enter_reduction()
    {
        let g = guard.db();
        if let Ok(Some(reduced)) = crate::eval::apply_lambda(g, head, &args)
            && reduced != head
        {
            return ctor_spine_fueled(g, reduced, fuel - 1);
        }
    }
    None
}

/// Lower a `(fn (param…) body)` that survives as a RUNTIME value — LAMBDA-LIFT it to a standalone
/// function and produce a `Core::Closure` naming its funcref-table slot + capture set. Single-parameter
/// only (the curried surface reduces multi-param application via partial application upstream). The
/// lambda's FREE VARIABLES are captured BY VALUE into the closure cell; each capturing reference in the
/// body is recorded (`db.captured_ref`) so lowering that reference in the lifted body reads the env cell
/// (`Core::Captured`) rather than following through to the (out-of-scope) binding. The param + result
/// machine types come from the lambda's body (`type_of`).
fn lower_lambda_value(db: &mut Db, id: StructId, params: &[StructId], body: StructId) -> Core {
    // At least one parameter (a nullary lambda value has no use here). A multi-parameter lambda IS
    // supported — it lifts to an `(env, p1, …, pn) -> result` function and is applied at FULL arity via
    // one `call_indirect` (see the `Core::CallClosure` lowering). A PARTIAL application of a runtime
    // multi-param closure (runtime currying) still declines at the application site, not here.
    if params.is_empty() {
        return Core::Poison(Reject::decline(
            crate::diag::NULLARY_LAMBDA_NO_CLOSURE_DECLINE,
        ));
    }
    let param_occs: Vec<StructId> = params
        .iter()
        .map(|&p| crate::eval::param_name_occ(db, p))
        .collect();
    // Collect the ORDERED, DISTINCT capture set — the enclosing-binding occurrences the body references
    // (other than ANY of its own params / top-level defs / the prelude), first-reference order. Each
    // capturing REFERENCE occurrence is recorded → its capture index, so the lifted body reads it from
    // the env.
    let mut captures: Vec<StructId> = Vec::new();
    let mut capture_refs: Vec<(StructId, usize)> = Vec::new();
    let ok = collect_captures(db, body, &param_occs, id, &mut captures, &mut capture_refs);
    if !ok {
        return Core::Poison(Reject::decline(
            "a closure captures a value with no runtime representation (not yet built)",
        ));
    }
    // The lambda's EXPECTED arrow from its CONTEXT (a variant-payload position `(T.Susp (fn …))`, a
    // built-in `Some`/`Ok` payload, or an annotation) — the "thread the use-site arrow back" the bottom-up
    // `type_of` omits. `None` when the context declares no arrow (a HOF call site the call's own unify
    // covers, or a genuinely unconstrained position). Its `Ty::Fn(P0, P1, … R)` gives per-parameter
    // expected types and the result type to fall back on when body-solving leaves them `Any`.
    let expected = crate::infer::expected_arrow_for_lambda(db, id);
    // Peel the expected arrow into a per-parameter type list + the final result type: an N-param lambda's
    // expected arrow is curried `P0 → P1 → … → R`. `expected_param_tys[i]` is param `i`'s expected type;
    // `expected_ret` is `R` after peeling all N params.
    let (expected_param_tys, expected_ret) = {
        let mut ptys: Vec<crate::ty::Ty> = Vec::new();
        let mut cur = expected.clone();
        for _ in 0..param_occs.len() {
            match cur {
                Some(crate::ty::Ty::Fn(p, r)) => {
                    ptys.push(*p);
                    cur = Some(*r);
                }
                _ => {
                    cur = None;
                    break;
                }
            }
        }
        (ptys, cur)
    };
    // Each parameter's solved machine type. A bare `(fn (x) …)` types `Any` at its own occurrence
    // (inference does not thread the use-site arrow back), so SOLVE it from the body's uses
    // (`solve_lambda_param_ty`, the lambda analogue of the recursive-def A2 solve), then fall back to the
    // EXPECTED arrow's parameter type (its storage context). A param neither the body nor the context
    // constrains to a machine type declines below (no invented width).
    let mut param_tys: Vec<(StructId, crate::ty::Ty)> = Vec::new();
    for (i, &p) in param_occs.iter().enumerate() {
        let mut pt = match crate::infer::type_of(db, p) {
            crate::ty::Ty::Any => crate::infer::solve_lambda_param_ty(db, p, body),
            t => t,
        };
        if matches!(pt, crate::ty::Ty::Any)
            && let Some(ep) = expected_param_tys.get(i)
            && !matches!(ep, crate::ty::Ty::Any)
        {
            pt = ep.clone();
        }
        if crate::backend::wasm::lir::valtype_of(&pt).is_none() {
            return Core::Poison(Reject::decline(crate::diag::CLOSURE_PARAM_NO_REPR_DECLINE));
        }
        // RECORD the solved param type so the LIFTED BODY's own `type_of(p)` reads it — otherwise the
        // body computes `p`'s type bottom-up as `Any` (it has no annotation and no def entry), and a use
        // like `(C.A p)` that boxes `p` into a sum/tuple payload declines "element of type Any". `type_of`
        // reads a `Param` from `db.param_types`, so seeding it here threads the closure's solved parameter
        // type into the body. Insert only a DETERMINED type, and only if absent, so a genuinely-annotated
        // or already-solved param is never overwritten.
        if !matches!(pt, crate::ty::Ty::Any) {
            db.param_types.entry(p).or_insert_with(|| pt.clone());
        }
        param_tys.push((p, pt));
    }
    // The RESULT type is the body's; if that is `Any` (a body that returns e.g. a sum whose payload
    // depends on an as-yet-unpinned param), fall back to the expected arrow's result type.
    let ret_ty = match crate::infer::type_of(db, body) {
        crate::ty::Ty::Any => expected_ret.clone().unwrap_or(crate::ty::Ty::Any),
        t => t,
    };
    if crate::backend::wasm::lir::valtype_of(&ret_ty).is_none() {
        return Core::Poison(Reject::decline(crate::diag::CLOSURE_RESULT_NO_REPR_DECLINE));
    }
    // Every captured value must have a machine representation too (it is boxed into the cell).
    for &cap in &captures {
        if crate::backend::wasm::lir::valtype_of(&crate::infer::type_of(db, cap)).is_none() {
            return Core::Poison(Reject::decline(
                crate::diag::CLOSURE_CAPTURE_NO_REPR_DECLINE,
            ));
        }
    }
    // Record each capturing reference → its capture index + type, so `core_of` on that reference (when
    // the LIFTED body is lowered) produces a `Core::Captured` reading the env cell. Keyed by the
    // reference OCCURRENCE (unique per use), so it never collides with an ordinary ref elsewhere.
    for (ref_occ, index) in capture_refs {
        let ty = crate::infer::type_of(db, captures[index]);
        db.captured_ref.insert(ref_occ, (index, ty));
    }
    // Register the lift (dedup by body occurrence); its position in `db.lifted` is its table slot.
    let code = db.lift_lambda(crate::lower::LiftedLambda {
        body,
        params: param_tys,
        ret_ty,
        captures: captures.clone(),
    });
    trace!(target: "rcdzc::lower", node = id.0, body = body.0, code, n_params = params.len(), n_captures = captures.len(), "lift lambda → Core::Closure");
    Core::Closure { code, captures }
}

/// Collect the lambda body's FREE-VARIABLE capture set into `captures` (ordered, distinct, first-use
/// order) and record each capturing REFERENCE occurrence → its capture index into `capture_refs`.
/// Returns `false` if a capture cannot be represented (a decline). A reference is a capture iff it
/// resolves to a USER-program binding (a `let` init / an enclosing parameter) lexically OUTSIDE the
/// lambda `lam_id`; a reference to the lambda's own `param`, a top-level def, or a prelude name is NOT a
/// capture. The captured VALUE identity is the binding occurrence the reference resolves to (so two
/// references to the same free variable share ONE capture slot).
fn collect_captures(
    db: &mut Db,
    node: StructId,
    params: &[StructId],
    lam_id: StructId,
    captures: &mut Vec<StructId>,
    capture_refs: &mut Vec<(StructId, usize)>,
) -> bool {
    match resolved_of(db, node) {
        // A bare parameter USE: `Resolved::Param { binder }`. One of the lambda's OWN params → not a
        // capture; an enclosing param → a capture keyed by that binder.
        Resolved::Param { binder } => {
            if params.contains(&binder) {
                return true;
            }
            // DEGENERATE SELF-CAPTURE — the reference occurrence IS its own binder (`binder == node`).
            // A legitimate capture reference and its binder are DISTINCT occurrences (the binder sits in a
            // param list / `let`; the use sits in the body). They coincide only as a copy artifact: when a
            // lambda ARGUMENT is `resolve_subtree`-pinned at a call site and that lambda is later itself
            // copied (its enclosing def inlined), a pinned OWN-param body reference is shared into the copy
            // while the param LIST is copied fresh — leaving a body occurrence that resolves to the
            // ORIGINAL param binder, now orphaned (no slot at the build site). Emitting it produced an
            // invalid module (a bare env-read / `local.get` with no backing local). A sound α-renaming fix
            // to the copy machinery is a separate, larger change; until then DECLINE rather than miscompile
            // (reject-don't-miscompile). This does NOT hit a genuine capture (distinct binder/use nodes).
            if binder == node {
                return false;
            }
            record_capture(binder, node, captures, capture_refs);
            return true;
        }
        Resolved::Ref { value } => {
            // A top-level def is global — never captured.
            if db.def_index_by_body(value).is_some() {
                return true;
            }
            // The ref's target may be a PARAMETER — including a SYNTHESIZED one, produced when a recursive
            // callee is specialized (its fn-typed argument `g` threads through as a fresh param binder).
            // Such a target is NOT a user node, so the `is_user_node` bailout below would wrongly treat it
            // as a global and skip the capture — leaving the reference to lower as a bare `Core::Param`
            // with no slot in the lifted body (an invalid module). Classify by the RESOLVED target: a
            // param that is not one of the lambda's OWN params is an enclosing binding to capture, keyed by
            // that param binder (so two references to the same threaded `g` share one capture slot).
            if let Resolved::Param { binder } = resolved_of(db, value) {
                if params.contains(&binder) {
                    return true; // the lambda's own parameter, reached through a ref — not a capture.
                }
                record_capture(binder, node, captures, capture_refs);
                return true;
            }
            // A non-param synthesized ref (a prelude name, a reduced constant) is global — not captured.
            if !db.is_user_node(value) {
                return true;
            }
            if !db.is_within(value, lam_id) {
                // A USER binding outside the lambda — a capture. Its identity is `value` (the binding
                // occurrence); this reference occurrence (`node`) reads it from the env.
                record_capture(value, node, captures, capture_refs);
                return true;
            }
            // Within the lambda (its params, a nested let) — recurse through the target for a nested
            // capture (e.g. a `let`-local whose init references a free variable).
            return collect_captures(db, value, params, lam_id, captures, capture_refs);
        }
        _ => {}
    }
    // A NESTED LAMBDA `(fn (inner-params) inner-body)` in the body — its OWN params are bound WITHIN it,
    // so they are neither captures of the outer lambda nor self-captures; only its FREE variables (which,
    // if bound outside the OUTER lambda, are the outer's captures too) matter. Descend into the inner
    // BODY with the inner params ADDED to the excluded set, and SKIP the inner param list (whose binder
    // occurrences would otherwise trip the `binder == node` self-capture guard, spuriously declining any
    // lifted lambda containing a nested lambda). This keeps a nested applied lambda `((fn (y) …) x)`
    // analyzable so it β-reduces at lowering rather than declining here.
    if let Some(tail) = db.ast.as_form(node, "fn")
        && let (Some(&inner_params_occ), Some(&inner_body)) = (tail.first(), tail.get(1))
        && let crate::ast::Struct::List(inner_params) = db.ast.get(inner_params_occ)
    {
        let inner_param_occs: Vec<StructId> = inner_params
            .clone()
            .iter()
            .map(|&p| crate::eval::param_name_occ(db, p))
            .collect();
        let mut combined = params.to_vec();
        combined.extend_from_slice(&inner_param_occs);
        return collect_captures(db, inner_body, &combined, lam_id, captures, capture_refs);
    }
    // Descend into the AST children (a form's operands, an if's branches, a `let`'s bindings).
    match db.ast.get(node) {
        crate::ast::Struct::List(children) => {
            let children: Vec<StructId> = children.clone();
            children
                .iter()
                .all(|&c| collect_captures(db, c, params, lam_id, captures, capture_refs))
        }
        crate::ast::Struct::Atom(_) => true,
    }
}

/// Record a captured binding: assign `binder` a capture slot (first-use order, deduped) and map the
/// capturing reference occurrence `ref_occ` to that slot.
fn record_capture(
    binder: StructId,
    ref_occ: StructId,
    captures: &mut Vec<StructId>,
    capture_refs: &mut Vec<(StructId, usize)>,
) {
    let index = captures
        .iter()
        .position(|&b| b == binder)
        .unwrap_or_else(|| {
            captures.push(binder);
            captures.len() - 1
        });
    capture_refs.push((ref_occ, index));
}

/// A lambda application whose β-reduction DECLINED with `msg`: emit a runtime `Core::Call` if it
/// declined because the callee is a RECURSIVE top-level def with a DETERMINED signature; otherwise
/// propagate the decline. This is the ONE place a recursive call becomes a real wasm call instead of an
/// unbounded inline. A non-recursive decline (a partial application, a bad head) is NOT a call — its
/// message is passed through unchanged.
fn lower_recursive_call_or_decline(
    db: &mut Db,
    head: StructId,
    args: &[StructId],
    msg: String,
) -> Core {
    // A REDUCTION-BUDGET decline (a non-normalizing / explosively-growing term — a self-applying lambda
    // whose reduction the total-work budget stopped) is a resource-limit rejection, the SAME "declined at
    // a bound, not crashed" class as the unproductive-recursion CDZ0999. Code it so, so it is a diagnosed
    // reject rather than a bare uncoded decline (the compiler stops and reports, never hangs).
    if msg.contains("reduction budget") {
        return Core::Poison(Reject::coded(Code::RecursionBound, msg));
    }
    // Only a RECURSION decline becomes a call; every other decline (partial application, over-arity)
    // propagates as-is. The recursion decline is the one `apply_lambda` raises via `is_recursive`.
    let is_recursion_decline = msg.contains("recursive function needs runtime specialization");
    if !is_recursion_decline {
        return Core::Poison(Reject::decline(msg));
    }
    // Resolve the head to the top-level def it names. Only a NAMED top-level def can be emitted as a
    // standalone wasm function (its index is stable in the layout); a computed/anonymous recursive head
    // has no such identity, so it still declines.
    let callee = match callee_def_index(db, head) {
        Some(d) => d,
        None => return Core::Poison(Reject::decline(msg)),
    };
    // The callee must have a DETERMINED signature to be emitted (its params need machine valtypes). An
    // annotated recursive def qualifies (types by absorption); an unannotated one is solved by the
    // connected parameter solve (`solve_recursive_params`, A2) — it stays undetermined only when no use
    // in the body constrains a parameter (it grounds to `Any`), in which case the call still declines.
    if crate::infer::def_scheme(db, callee).is_none() {
        trace!(target: "rcdzc::lower", head = head.0, callee, "recursive call: callee signature undetermined → decline (A2)");
        return Core::Poison(Reject::decline(
            "a recursive function with an unannotated parameter is not yet inferred (annotate its parameters)",
        ));
    }
    trace!(target: "rcdzc::lower", head = head.0, callee, args = args.len(), "recursive call → Core::Call");
    Core::Call {
        callee,
        args: args.to_vec(),
    }
}

/// The `db.defs` index of the top-level def an application head names, if any — following a `Ref` to a
/// `Lambda` whose body matches a def's body occurrence. Returns `None` for a head that is not a named
/// top-level def (a `let`-bound lambda, a computed head).
fn callee_def_index(db: &mut Db, head: StructId) -> Option<usize> {
    // The head resolves to a `Lambda { body }` for a named function (a top-level def, or a MODULE MEMBER
    // via Case R) or a `Ref` chain to one; match its body back to the def index. A `Member` projection
    // `(. m f)` reduces to the field lambda via `member_value` (WITHOUT the general `lambda_of`
    // β-reduction, which would inline a deep non-recursive chain — an exponential cost on the hot lower
    // path). So a recursive MODULE MEMBER called through the projection chain finds its registered
    // internal def (`modules::register_callable`), exactly as a bare-named recursive def finds its
    // top-level def, at no cost to an ordinary call.
    match resolved_of(db, head) {
        Resolved::Lambda { body, .. } => db.def_index_by_body(body),
        Resolved::Ref { value } => callee_def_index(db, value),
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(v) => callee_def_index(db, v),
            _ => None,
        },
        _ => None,
    }
}

/// Whether a `let` binding whose initializer is `init` should be KEPT as a named `Core::Let` binding
/// rather than copy-propagated. Kept iff (1) its value is a RUNTIME computation — its core is not a
/// constant/atom that folds away, so following it through would re-emit the computation — AND (2) its
/// name is used MORE THAN ONCE across `scope` (the later sibling initializers and the body — naming
/// pays for itself only when it avoids a recompute). A constant, a single-use binding, or a poison is
/// propagated (byte-neutral).
fn should_keep_binding(db: &mut Db, init: StructId, scope: &[StructId]) -> bool {
    // A LAMBDA-valued binding is NEVER kept as a runtime `let` slot — it is copy-propagated so its
    // applications fold (β-reduce) at each use. Short-circuit HERE, before `is_runtime_computation` calls
    // `core_of(init)` — which for a lambda runs `lower_lambda_value`, LIFTING it speculatively and (for a
    // capturing lambda) polluting `db.captured_ref` with the body's capturing-reference occurrences. Those
    // occurrences are SHARED with the fold's reduced body (`(g 5)` → `(+ 5 k)` reuses the original `k`
    // occurrence), so the stale `captured_ref` entry would then make the FOLDED `k` lower to a
    // `Core::Captured` env-read in the ENCLOSING scope (where there is no env) — a miscompile (reading an
    // uninitialized local). Checking `resolved_of` (not `core_of`) avoids triggering the lift. A lambda
    // that genuinely escapes to runtime is lifted at its USE site, not kept here.
    if matches!(resolved_of(db, init), Resolved::Lambda { .. }) {
        return false;
    }
    // A binding whose value REDUCES to a lambda — a function returning a closure, `(mk n)` returning
    // `(fn (x) (+ n x))` — is the SAME hazard one syntactic step removed: it is not a `Resolved::Lambda`
    // (it is an `Apply`), so it slips past the check above, but `is_runtime_computation`'s `core_of` below
    // would still LIFT it (`lower_lambda_value`) and pollute `db.captured_ref` with the returned closure's
    // captured occurrences. Those occurrences are SHARED with the fold's reduced body — `(let ((f (mk n)))
    // (f 3))` copy-propagates `f` and β-reduces `((mk n) 3)` to `(+ n 3)`, reusing the ORIGINAL `n`
    // occurrence — so the stale `captured_ref` entry makes the FOLDED `n` lower to a `Core::Captured`
    // env-read in the ENCLOSING scope (which has no env) → invalid wasm ("expected i32, found i64"). Detect
    // it with `lambda_body` (which reduces but does NOT lower, so it does not pollute) and propagate, so the
    // application folds inline exactly as the (working) `((mk n) 3)` and HOF-argument forms do.
    if crate::eval::lambda_body(db, init).is_some() {
        return false;
    }
    // A value that folds to a constant / atom leaves no computation to share — always propagate.
    if !is_runtime_computation(db, init) {
        return false;
    }
    // A COMPOUND (tuple/record) binding that is ONLY ever PROJECTED — never used as a whole value —
    // need not be built on the heap at all: each projection folds straight through to the element's own
    // computation (a param `local.get`, a nested op, …), which is far cheaper than an `arr-alloc` +
    // per-field `box`/`arr-set` + `arr-get`/`get` + `drop` round-trip. Keeping it would build a heap
    // value only to read it back (or, when the projections fold, to drop it dead). So a projection-only
    // compound is NOT kept — it folds. A compound that ESCAPES as a whole (returned, passed as an arg,
    // nested into another compound) genuinely needs materialization and IS kept. (A non-compound
    // runtime binding — a shared scalar computation — keeps the multi-use rule below: naming avoids a
    // recompute.)
    if is_compound_value(db, init) && !binding_escapes_whole(db, init, scope) {
        return false;
    }
    // Count references to this binding across its scope. Naming is worth it only at >= 2 uses.
    let mut n = 0;
    for &region in scope {
        n += uses_in(db, region, init);
    }
    n >= 2
}

/// Whether the node at `init` lowers to a COMPOUND heap value — a tuple or a record. These are the
/// values whose only-projected form folds through rather than being built on the heap.
fn is_compound_value(db: &mut Db, init: StructId) -> bool {
    matches!(core_of(db, init), Core::Tuple { .. } | Core::Record { .. })
}

/// Whether the binding `init` is used as a WHOLE VALUE anywhere in `scope` — i.e. referenced in any
/// position OTHER than as the operand of a projection (`(. c i)` / `(. c field)`). A whole-value use
/// (returned as the body's result, passed as a call argument, nested as an element of another compound,
/// annotated, …) means the compound must actually exist at run time, so it is materialized on the heap.
/// If every reference is a projection, the compound never needs to exist — each field read folds to the
/// element directly — so this returns `false` and the binding is not kept. Mirrors the value-flow
/// discipline `binding_escapes` uses in selection for Perceus drops, at the resolved layer.
fn binding_escapes_whole(db: &mut Db, init: StructId, scope: &[StructId]) -> bool {
    scope
        .iter()
        .any(|&region| ref_escapes_whole(db, region, init))
}

/// Whether a reference to `init` appears as a WHOLE-VALUE use within `node` (not merely as a projection
/// operand). Recurses every sub-position; at a projection `(. operand i)`, a reference that IS the
/// `operand` is a projection (does not escape), but the operand is still recursed in case it nests a
/// whole-value use deeper (e.g. `(. (f c) 0)` uses `c` wholly as `f`'s argument).
fn ref_escapes_whole(db: &mut Db, node: StructId, init: StructId) -> bool {
    match resolved_of(db, node) {
        // A bare reference to the binding, in a non-projection position → a whole-value use.
        Resolved::Ref { value } => value == init,
        // A projection: if its operand is a DIRECT ref to `init`, that is a projection use (does not
        // escape). Otherwise recurse the operand (it may nest a whole-value use).
        Resolved::Proj { operand, .. } | Resolved::Member { operand, .. } => {
            if matches!(resolved_of(db, operand), Resolved::Ref { value } if value == init) {
                false
            } else {
                ref_escapes_whole(db, operand, init)
            }
        }
        Resolved::If { cond, then_, else_ } => {
            ref_escapes_whole(db, cond, init)
                || ref_escapes_whole(db, then_, init)
                || ref_escapes_whole(db, else_, init)
        }
        Resolved::And { lhs, rhs, .. } => {
            ref_escapes_whole(db, lhs, init) || ref_escapes_whole(db, rhs, init)
        }
        Resolved::Not { operand } => ref_escapes_whole(db, operand, init),
        Resolved::Let { bindings, body } => {
            bindings
                .iter()
                .any(|(_, v)| ref_escapes_whole(db, *v, init))
                || ref_escapes_whole(db, body, init)
        }
        Resolved::Record { fields } => fields.values().any(|&v| ref_escapes_whole(db, v, init)),
        Resolved::Tuple { elems } | Resolved::List { elems } => {
            elems.iter().any(|&e| ref_escapes_whole(db, e, init))
        }
        // A map literal uses each entry's key AND value as a whole value (both consumed into the map).
        Resolved::Map { entries } => entries
            .iter()
            .any(|&(k, v)| ref_escapes_whole(db, k, init) || ref_escapes_whole(db, v, init)),
        // A `(bin …)` construction uses each segment's value slot (and dependent size) as a whole value.
        Resolved::Bin { segs } => segs.iter().any(|s| {
            ref_escapes_whole(db, s.slot, init)
                || matches!(&s.kind, crate::resolved::SegKind::Bytes { size: Some(n) } if ref_escapes_whole(db, *n, init))
                || matches!(&s.kind, crate::resolved::SegKind::Utf8 { size } if ref_escapes_whole(db, *size, init))
        }),
        Resolved::Annot { expr, .. } => ref_escapes_whole(db, expr, init),
        Resolved::Apply { head, args } => {
            ref_escapes_whole(db, head, init)
                || args.iter().any(|&a| ref_escapes_whole(db, a, init))
        }
        Resolved::Match { scrutinee, arms } => {
            ref_escapes_whole(db, scrutinee, init)
                || arms.iter().any(|(_, b)| ref_escapes_whole(db, *b, init))
        }
        // Effect control forms: a reference to `init` as a whole value can appear in a handler's init,
        // any arm body, a resumption's value/next-state, or the handled/delegated body — recurse each.
        Resolved::Handle {
            init: seed,
            arms,
            body,
        } => {
            ref_escapes_whole(db, seed, init)
                || arms.iter().any(|a| ref_escapes_whole(db, a.body, init))
                || ref_escapes_whole(db, body, init)
        }
        Resolved::Resume { value, next_state } => {
            ref_escapes_whole(db, value, init) || ref_escapes_whole(db, next_state, init)
        }
        Resolved::Host { body, .. } => ref_escapes_whole(db, body, init),
        // A `SumPayload`/`BinField` reads a PIECE of the scrutinee (a payload / a decoded segment), not
        // the whole value — like a projection operand, it is not a whole-value escape of `init`.
        Resolved::SumPayload { .. }
        | Resolved::BinField { .. }
        | Resolved::MapField { .. }
        | Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::SymbolConst(_)
        | Resolved::Bytes(_)
        | Resolved::Char(_)
        | Resolved::Float(_)
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. }
        | Resolved::Poison(_) => false,
    }
}

/// Whether the node at `init` lowers to a RUNTIME COMPUTATION — a core form that emits instructions
/// (arithmetic, comparison, conversion, a conditional, a runtime record), as opposed to a constant, a
/// unit, a bare local/param read, or a poison, which are free to duplicate. Reads the value's core
/// (the fold has already run, so a constant-folding binding reports `false` here).
fn is_runtime_computation(db: &mut Db, init: StructId) -> bool {
    let core = core_of(db, init);
    // A STATIC (fully-constant) tuple is NOT a runtime computation — keeping it would force a per-call
    // heap build (`arr-alloc`), pure waste for a value that never varies (`value-heap-runtime.md` §2d:
    // a static compound must not pay per-call construction). Leaving it UNKEPT lets each projection fold
    // straight through to the constant element (`reduce_to_tuple_elems` follows an unkept binding) — so
    // a constant tuple that is only projected emits ZERO heap ops, better than build-once. (A tuple with
    // a RUNTIME element genuinely allocates and IS kept — the H2b round-trip. The build-once-GLOBAL path
    // for a constant tuple that ESCAPES as a value activates with the first escape path — the renderer.)
    if matches!(core, Core::Tuple { .. }) && is_constant_compound(db, init) {
        return false;
    }
    matches!(
        core,
        Core::Arith { .. }
            | Core::Compare { .. }
            | Core::Convert { .. }
            | Core::If { .. }
            | Core::Record { .. }
            // A tuple with a runtime element constructs a heap value (an allocation), and a projection
            // reads one — both are runtime computations worth naming when used more than once. Keeping a
            // multi-use runtime tuple as a `Core::Let` binding is ALSO what makes its projection stay
            // runtime (the binding is opaque to the fold via `reduce_to_tuple_elems`) — the H2b round-trip.
            | Core::Tuple { .. }
            | Core::Proj { .. }
            // A list construction (`vec-empty` + a `vec-push` per element) is a genuine runtime
            // computation — an allocation per element — so a `let`-bound list used more than once is
            // worth NAMING (built once, the handle read by each use) rather than rebuilt at every use.
            // Unlike a tuple, a list has NO fold-through projection (a runtime-indexed `List.at` can't
            // fold to an element the way `(. t 0)` does), so `is_compound_value` deliberately does NOT
            // list `ListNew` — a list binding is always a whole-value use and simply keeps under the
            // >= 2-use rule below. (A single-use list still inlines: `n < 2`.)
            | Core::ListNew { .. }
    )
}

/// Whether the node at `id` lowers to a fully COMPILE-TIME-CONSTANT compound (or scalar): a constant
/// scalar/unit, or a `Core::Tuple` all of whose elements are themselves constant (recursively). This is
/// the classification that routes a STATIC compound away from per-call construction (§2d): a constant
/// tuple has no runtime-varying part, so it need never be built at run time — its projections fold, and
/// (once an escape path exists) its materialization is a build-once global rather than a per-call alloc.
pub fn is_constant_compound(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_) | Core::ConstBool(_) | Core::Unit => true,
        Core::Tuple { elems } => elems.iter().all(|&e| is_constant_compound(db, e)),
        Core::Record { fields } => fields.values().all(|&v| is_constant_compound(db, v)),
        _ => false,
    }
}

/// The CANONICAL BINARY VALUE FORM of a fully-constant compound at `id` — the bytes the resource escape
/// path's `encode()` returns (`DESIGN-value-heap-rcdzc.md` §3a; `contracts/deterministic-value-form.md`).
/// Reconstructs the s-expression `(: <value> <type>)` as ordinary AST (the value from the constant core,
/// the type from the solved `type_of`) and encodes it with the shared codec — the SAME bytes the corpus
/// value form uses, so the host decodes + pretty-prints them to the recorded text. Returns `None` if the
/// node is not a compile-time-constant compound (a runtime compound's bytes are built by the real
/// handle-walking encoder, R2 — this constant path is R1's proof that the resource+`encode()`+decode
/// pipeline crosses correctly before the walk exists). The type is baked as constant bytes (the runtime
/// is name-free); this does NO in-wasm formatting — it is a compile-time serialization.
pub fn constant_value_form(db: &mut Db, id: StructId) -> Option<Vec<u8>> {
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    let value = const_value_ast(db, &mut b, id)?;
    let ty = crate::infer::type_of(db, id);
    let type_ast = type_ast(&mut b, &ty)?;
    let root = b.list(vec![colon, value, type_ast]);
    Some(crate::codec::encode(&b.finish(root)))
}

/// A RUNTIME leaf hole in a value-form byte template: the byte OFFSET in the template where the leaf's
/// runtime value is written, the WALK PATH of `arr-get` indices from the root heap handle to the leaf,
/// and its KIND (how many bytes / which encoding). The template bakes everything static (structure,
/// names, type nodes, kind/len framing); at run time `encode()` walks each hole's path and writes the
/// value. (`DESIGN-value-heap-rcdzc.md` §3a R2 — the runtime compound escape.)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeLeaf {
    /// Byte offset in the template where the runtime value is written.
    pub offset: usize,
    /// `arr-get` indices from the root handle down to this leaf (empty = the root is itself the leaf).
    pub path: Vec<u32>,
    /// How the leaf's runtime value fills its hole.
    pub kind: LeafFill,
    /// Whether the walk starts by recovering the SUM PAYLOAD: when this leaf lives inside a sum
    /// variant's payload, the walker first calls `sum-payload(rep)` to get the payload handle, THEN
    /// applies `path`. A single-payload variant leaves `path` empty (the payload handle IS the boxed
    /// leaf); a multi-payload variant's `path` indexes into the payload tuple. `false` for a plain
    /// tuple/record leaf (the walk starts at the root handle directly). Set on the per-variant templates
    /// a [`SumFormTemplate`] holds.
    pub via_sum_payload: bool,
}

/// How a runtime leaf's value fills its template hole.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeafFill {
    /// A boxed integer: read `get-int` (s64), write 8 big-endian magnitude bytes at `offset` (the
    /// template reserves an 8-byte magnitude with `len=8`; a non-minimal magnitude decodes fine because
    /// `BigInt::from_bytes_be` normalizes leading zeros). A NEGATIVE value also flips the kind byte at
    /// `offset - 2` from `INT_POS_DEC` to `INT_NEG_DEC` and writes the ABSOLUTE magnitude.
    Int,
    /// A boxed boolean: read `get-bool`, write the kind byte at `offset` — `9` (true) or `8` (false).
    Bool,
}

/// The value-form byte TEMPLATE for a runtime compound of type `ty`: the codec bytes with every leaf's
/// value left as a placeholder, plus the [`RuntimeLeaf`] holes to fill at run time. Everything static —
/// the `(: value type)` structure, the `tuple`/`record` heads + field names, the whole TYPE node, and
/// each leaf's kind/len framing — is baked; only the leaf VALUES are holes. `encode()` copies this
/// template into linear memory, walks each hole's heap path, and writes the value (R2). `None` if the
/// type has no value-form surface (a function/type-value). Every leaf is treated as a runtime hole
/// (walked from the handle), so a mixed const/runtime compound needs no special-casing — a constant
/// element still sits boxed on the heap and reads back the same.
pub fn runtime_value_form_template(ty: &crate::ty::Ty) -> Option<ValueFormTemplate> {
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    // Build the value AST with PLACEHOLDER leaves, recording each leaf's walk path + kind as we go.
    let mut leaves: Vec<PendingLeaf> = Vec::new();
    let value = template_value_ast(&mut b, ty, &mut Vec::new(), &mut leaves)?;
    let type_ast = type_ast(&mut b, ty)?;
    let root = b.list(vec![colon, value, type_ast]);
    let arenas = b.finish(root);
    let bytes = crate::codec::encode(&arenas);
    // Locate each placeholder leaf's byte offset in the encoded LEAF POOL (leaves are encoded in order
    // right after the 8-byte header + leaf-count LEB). Walk the pool, tracking offsets; a leaf that was
    // recorded as runtime (by its LeafId) gets its hole offset resolved here.
    let holes = resolve_leaf_offsets(&bytes, &arenas, &leaves)?;
    Some(ValueFormTemplate {
        bytes,
        leaves: holes,
    })
}

/// The two STATIC halves of a runtime `Bytes` value form, for the looping `encode()` walker (L2b).
/// The value form of `(: <bytes> Bytes)` is `PREFIX · <LEB len> · <n raw bytes> · SUFFIX`, where ONLY
/// the leaf's length-LEB and payload are runtime — the prefix (header … the `KIND_BYTES` tag) and the
/// suffix (the `Bytes` type-name leaf + the whole struct table + root) are byte-identical regardless of
/// `n` (verified across n = 0 / 3 / 130). So the walker writes `prefix`, then the runtime LEB of
/// `bytes-len`, then copies the bytes, then `suffix` — no fixed-size template. `DESIGN-runtime-bytes-
/// escape-walker.md`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuntimeBytesForm {
    /// Bytes to write verbatim BEFORE the runtime length+payload — the header through the KIND_BYTES tag.
    pub prefix: Vec<u8>,
    /// Bytes to write verbatim AFTER the runtime payload — the type-name leaf + struct table + root.
    pub suffix: Vec<u8>,
}

/// Compute the [`RuntimeBytesForm`] for `Ty::Bytes` — build the ZERO-length Bytes value form (`…0b 00
/// <suffix>`) and split it at the leaf's length byte: `prefix` = everything up to and INCLUDING the
/// `KIND_BYTES` tag, `suffix` = everything AFTER the `00` length byte. A runtime walker fills the gap
/// with `<LEB n><n bytes>`. `None` if the encoded form does not have the expected `0b 00` shape (a
/// codec change) — the escape then declines rather than emit a wrong walker.
pub fn runtime_bytes_form(db: &mut Db) -> Option<RuntimeBytesForm> {
    runtime_leaf_form(db, false)
}

/// The runtime STRING escape form — `(: "" String)` with an empty `Leaf::Str`, split at the `KIND_STR`
/// tag. A runtime String is a UTF-8 byte-rope leaf (the same heap rep as Bytes — `String.concat` is
/// `bytes-concat`), so it escapes through the SAME looping walker (`emit_runtime_bytes_resource`); only
/// the value-form framing differs (`(: "…" String)` vs `(: b"…" Bytes)`). The walker's `bytes-len`/
/// `bytes-get` read the same leaf either way; the payload bytes ARE the UTF-8 the codec decodes back to a
/// `Leaf::Str`, so `cdz-run` renders `(: "…" String)`.
pub fn runtime_string_form(db: &mut Db) -> Option<RuntimeBytesForm> {
    runtime_leaf_form(db, true)
}

/// Shared builder for the runtime Bytes/String escape form: encode `(: <empty-leaf> <TypeName>)` and split
/// at the leaf's tag so the walker can splice the runtime `LEB(len) · payload` between the static prefix
/// (header … tag) and suffix (the `<TypeName>` + struct framing). `is_string` selects a `Leaf::Str`/
/// `"String"`/`KIND_STR` split vs a `Leaf::Bytes`/`"Bytes"`/`KIND_BYTES` one — the ONLY difference between
/// a runtime String and a runtime Bytes escape (both are UTF-8/byte leaves on the rope heap).
fn runtime_leaf_form(db: &mut Db, is_string: bool) -> Option<RuntimeBytesForm> {
    let _ = db; // (kept for signature symmetry with the other form builders; not needed here)
    const KIND_BYTES: u8 = 11;
    const KIND_STR: u8 = 7;
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    let (empty, ty_name, kind) = if is_string {
        (
            b.atom_leaf(crate::ast::Leaf::Str(String::new())),
            b.name("String"),
            KIND_STR,
        )
    } else {
        (
            b.atom_leaf(crate::ast::Leaf::Bytes(Vec::new())),
            b.name("Bytes"),
            KIND_BYTES,
        )
    };
    let root = b.list(vec![colon, empty, ty_name]);
    let arenas = b.finish(root);
    let encoded = crate::codec::encode(&arenas);
    // Find the leaf's KIND tag IMMEDIATELY followed by its `0x00` length byte (the empty leaf). `":"` and
    // the type name are NAME leaves (`0x0a …`), so the only `<kind> 00` pair is the empty payload leaf.
    let pos = encoded.windows(2).position(|w| w == [kind, 0x00])?;
    let prefix = encoded[..=pos].to_vec();
    let suffix = encoded[pos + 2..].to_vec();
    Some(RuntimeBytesForm { prefix, suffix })
}

/// A value-form template: the byte buffer (placeholders in the leaf values) + the runtime holes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValueFormTemplate {
    pub bytes: Vec<u8>,
    pub leaves: Vec<RuntimeLeaf>,
}

/// The escape template for a SUM result — one complete value-form template per variant (its rendered
/// `(: (VariantName payload…) SumType)` bytes + holes), indexed by DISCRIMINANT. Unlike a tuple/record
/// (one static shape, one template), a sum renders DIFFERENTLY per variant (`(Some 5)` vs `(None unit)`
/// — different name, different payload), so the walker must switch on the runtime discriminant
/// (`sum-disc`) and emit the matching variant's template. Each variant's payload leaves carry
/// `via_sum_payload` (they are reached through `sum-payload` first). `type-system.md §A Match Is
/// Exhaustive Against The Sum Type's Variant Set` — the variant set is closed, so the switch is total.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SumFormTemplate {
    /// One template per variant, in DISCRIMINANT (declaration) order — `variants[disc]` renders the
    /// value with that discriminant.
    pub variants: Vec<ValueFormTemplate>,
}

/// Build the [`SumFormTemplate`] for a `Ty::Sum` result: one value-form template per variant. Each
/// variant's template renders `(: (VariantName payload…) SumType)` with the payload leaves left as
/// holes reached through `sum-payload`. A NULLARY variant renders `(VariantName unit)` (no holes) — the
/// corpus form (`(None unit)`). A SINGLE-payload variant renders `(VariantName <scalar-hole>)`, the
/// hole reached directly off the payload handle (`via_sum_payload`, empty `path`). A MULTI-payload
/// variant renders `(VariantName p0 p1 …)`, the holes reached by `arr-get` into the payload tuple. A
/// payload whose type has no value-form surface (a function/nested-sum for now) makes the whole thing
/// `None` — the escape declines. Needs `db` to read the variant names + payload types from
/// `db.type_decls` (found by the sum's `decl` occurrence).
pub fn sum_form_template(db: &mut Db, ty: &crate::ty::Ty) -> Option<SumFormTemplate> {
    let crate::ty::Ty::Sum { decl, args, .. } = ty else {
        return None;
    };
    // Recover the variant set + the declaration's type PARAMS from the declaration occurrence. A generic
    // sum's payload occurrences mention the params (a lowercase `a`); the instantiation's `args` are the
    // concrete types to substitute for them, positionally.
    let decl_ref = db.type_decl_by_occ(*decl)?;
    let params = decl_ref.params.clone();
    // Clone the shape out so we can reduce payload types with `&mut db` below.
    let variants: Vec<(String, Vec<StructId>)> = decl_ref
        .variants
        .iter()
        .map(|v| (v.name.clone(), v.payloads.clone()))
        .collect();
    let mut out = Vec::with_capacity(variants.len());
    for (disc, (_, payload_occs)) in variants.iter().enumerate() {
        // Reduce each payload TYPE occurrence to a `Ty` AT THE INSTANTIATION: a payload that IS a type
        // parameter (a bare name in `params`) becomes the corresponding concrete `arg`; any other
        // payload reduces normally (`typeval_of`). This is what makes a generic `Option Int64` escape
        // with its `Some` payload templated as `Int64` rather than the unresolvable param `a`.
        let mut payload_tys = Vec::with_capacity(payload_occs.len());
        for &p in payload_occs {
            let pty = match db.ast.as_name(p) {
                Some(n) if params.iter().any(|q| q == n) => {
                    let idx = params.iter().position(|q| q == n).unwrap();
                    args.get(idx).cloned()?
                }
                _ => crate::eval::typeval_of(db, p)?,
            };
            payload_tys.push(pty);
        }
        // The variant HEAD — BARE (`Some`, `Cons`) normally, QUALIFIED `(. Ast List)` when the sum has a
        // variant name a prelude entry shadows (see `variant_head_ast`) — so the runtime walker writes the
        // same head the constant bake does.
        out.push(variant_form_template(
            db,
            *decl,
            disc as u32,
            &payload_tys,
            ty,
        )?);
    }
    Some(SumFormTemplate { variants: out })
}

// ─── Shape descriptor (for the runtime `value-encode` op) ────────────────────────────────────────
//
// The compiler-baked descriptor the runtime `value-encode` walker reads to render a RUNTIME value —
// including a self-referential (recursive) sum, which has no fixed hole-template. A descriptor is a
// TABLE of shapes + a root index; a child position references another entry by index, so a recursive
// type closes as a finite cycle (a `Ref` back to the sum's entry). Wire format is documented on the
// runtime side (`cdz-runtime` value-encode note); this is the ENCODER, kept in lock-step (a drift is
// caught by the runtime's byte-exact `value_encode_form_matches_the_codec` cross-check + the corpus).
//
// Shape tags: 0 Int, 1 Bool, 2 Float, 3 Str, 4 Bytes, 5 Unit, 6 Tuple[n][idx…], 7 List[idx],
// 8 Record[n](name,idx)…, 9 Sum[n](head,idx)…, 10 Named(name,idx), 11 Ref(idx), 12 Set[idx],
// 13 Map(k,v), 14 Float32, 15 Framed(head,[arg…],idx), 16 Spread[n][idx…] (a multi-payload variant's
// tuple payload, rendered FLAT under the variant head).

/// Build the shape descriptor bytes for a `Ty::Sum` result, wrapped in the outer `(: <value> <Type>)`
/// frame — the input to the runtime `value-encode` op. Handles a RECURSIVE sum: a sum decl already in
/// the table is referenced by index (`Ref`), closing the cycle. `None` if any payload type has no
/// renderable shape yet (a Float/Str/Bytes payload — a later slice; the escape then declines cleanly).
pub fn sum_shape_descriptor(db: &mut Db, ty: &crate::ty::Ty) -> Option<Vec<u8>> {
    let mut builder = ShapeTableBuilder::default();
    match ty {
        // A boxed sum. A MONOMORPHIC sum (`args: []`) wraps in `Named(<type name>, …)` — the bare-name
        // `(: <value> <Type>)` frame (`(: (Neg unit) Sign)`). A GENERIC sum (`args` non-empty) must render
        // its type ARGUMENTS too (`(: (Some "é") (Option String))`, NOT the bare `(: … Option)`), so it
        // wraps in a PARAMETRIC `Framed(<type-node>, …)` frame built from the full type (`type_node_of`
        // renders `(Option String)`), exactly as a `List`/`Map`/`Set` does. Without this a generic sum
        // result dropped its type args at the boundary.
        crate::ty::Ty::Sum { name, args, .. } => {
            let inner = builder.shape_of(db, ty)?;
            if args.is_empty() {
                let named = builder.push(ShapeNode::Named(name.clone(), inner));
                Some(builder.encode(named))
            } else {
                let type_node = type_node_of(ty)?;
                let framed = builder.push(ShapeNode::Framed(type_node, inner));
                Some(builder.encode(framed))
            }
        }
        // A NOMINAL newtype (a recursive one that escapes): its `shape_of` ALREADY produces a
        // `Named(<type name>, …)` root (the erased-tag frame), so encode it directly — wrapping again
        // would double-name it. This is what carries the recursive newtype's OWN name to the host
        // (`(: … Lst)`), where routing on the stripped inner sum would have named it `Option`.
        crate::ty::Ty::Nominal { .. } => {
            let root = builder.shape_of(db, ty)?;
            Some(builder.encode(root))
        }
        // A LIST/SET/MAP result: build the value shape, then wrap in a PARAMETRIC `Framed(<type-node>, …)`
        // frame so the value form renders `(: (list …) (List <elem>))` etc. — the element/key/value types
        // OBSERVABLE, matching the constant-collection value form. The type node is built RECURSIVELY from
        // the full type (`type_node_of`), so a nested element type crosses too: `(List (List Int64))`,
        // `(Map Int64 (Set Int64))`, `(Set (Tuple Int64 Int64))`. The inner VALUE shape (`shape_of`) already
        // recurses over nested collections, so the walker renders them; only the type node needed lifting.
        crate::ty::Ty::List(_) | crate::ty::Ty::Set(_) | crate::ty::Ty::Map(_, _) => {
            let type_node = type_node_of(ty)?;
            let inner = builder.shape_of(db, ty)?;
            let framed = builder.push(ShapeNode::Framed(type_node, inner));
            Some(builder.encode(framed))
        }
        // A TUPLE/RECORD result whose value shape is renderable but which contains a VARIABLE-length element
        // (a list/map/set, or a sum) — `runtime_value_form_template` returns `None` for it (no fixed-size
        // static template), so it escapes via the same runtime `value-encode` walker as a collection. Wrap in
        // a PARAMETRIC `Framed(<type-node>, …)` frame so the value form renders `(: (tuple …) (Tuple …))` /
        // `(: (record …) (Record …))` with the element/field types observable. `shape_of` already recurses
        // over the nested elements (a list element loops, a nested sum switches on its disc). A fixed-shape
        // tuple/record (all scalar/byte/fixed-compound elements) still takes the cheaper static-template path
        // (`runtime_value_form_template`), which the caller tries FIRST — this descriptor path is the fallback
        // for the variable-shape case only.
        crate::ty::Ty::Tuple(_) | crate::ty::Ty::Record(_) => {
            let type_node = type_node_of(ty)?;
            let inner = builder.shape_of(db, ty)?;
            let framed = builder.push(ShapeNode::Framed(type_node, inner));
            Some(builder.encode(framed))
        }
        _ => None,
    }
}

/// The RECURSIVE type node for a `Framed` frame's type position — mirrors `Ty::render_name` structurally
/// so the runtime's `render_type_node` reproduces the same written type. A leaf (a scalar/nominal/nullary
/// sum) is a bare-name node with no children; a parametric type (`List`/`Set`/`Map`/`Tuple`/`Record`/a
/// generic sum) is a head plus child type nodes, nested to any depth. `None` for a type that never appears
/// as an escaping collection element (Fn/Qty/Var/Any/Type) — the escape declines rather than misrender it.
fn type_node_of(ty: &crate::ty::Ty) -> Option<TypeNode> {
    use crate::ty::Ty;
    let leaf = |s: String| TypeNode {
        head: s,
        children: vec![],
    };
    Some(match ty {
        Ty::Int(_)
        | Ty::Bool
        | Ty::Unit
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Float(_)
        | Ty::Bytes => leaf(ty.render_name()),
        Ty::List(e) => TypeNode {
            head: "List".to_string(),
            children: vec![type_node_of(e)?],
        },
        Ty::Set(e) => TypeNode {
            head: "Set".to_string(),
            children: vec![type_node_of(e)?],
        },
        Ty::Map(k, v) => TypeNode {
            head: "Map".to_string(),
            children: vec![type_node_of(k)?, type_node_of(v)?],
        },
        Ty::Tuple(elems) => {
            let mut children = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                children.push(type_node_of(e)?);
            }
            TypeNode {
                head: "Tuple".to_string(),
                children,
            }
        }
        // A record renders as `(record (name Type) …)`: each field is itself a node `(name <type>)` — head
        // = the field name, one child = the field's type node. `render_type_node` reproduces `(name Type)`.
        Ty::Record(fields) => {
            let mut children = Vec::with_capacity(fields.len());
            for (k, t) in fields.iter() {
                children.push(TypeNode {
                    head: k.name.clone(),
                    children: vec![type_node_of(t)?],
                });
            }
            TypeNode {
                head: "record".to_string(),
                children,
            }
        }
        // A monomorphic sum renders as its bare name; a generic sum as `(Name arg…)`.
        Ty::Sum { name, args, .. } => {
            if args.is_empty() {
                leaf(name.clone())
            } else {
                let mut children = Vec::with_capacity(args.len());
                for a in args {
                    children.push(type_node_of(a)?);
                }
                TypeNode {
                    head: name.clone(),
                    children,
                }
            }
        }
        Ty::Nominal { name, .. } => leaf(name.clone()),
        // Qty/Fn/Var/Any/Type: not an escaping collection element/arg — decline rather than misrender.
        _ => return None,
    })
}

/// A shape-table entry (indices reference other entries — recursion closes via `Ref`).
enum ShapeNode {
    Int,
    /// An arbitrary-precision integer (a runtime `BigInt`). The runtime (descriptor tag 17) reads it via
    /// `unbox_bigint` and renders the SAME `KIND_INT` leaf as `Int` — the codec leaf is already
    /// arbitrary-width (sign + big-endian magnitude), so no new wire KIND is needed, only the shape tag.
    BigInt,
    Bool,
    Float,
    Float32,
    Str,
    Bytes,
    Unit,
    Tuple(Vec<u32>),
    List(u32),
    Record(Vec<(String, u32)>),
    Sum(Vec<(String, u32)>),
    Named(String, u32),
    Ref(u32),
    Set(u32),
    Map(u32, u32),
    /// A `(: <value> <type-node>)` frame — an arbitrary (possibly NESTED) type node. The runtime
    /// `value-encode` decodes this as descriptor tag 15 and renders the type node RECURSIVELY, so a runtime
    /// collection crosses as `(: (list …) (List Int64))` — or, with nesting, `(: … (List (List Int64)))`.
    Framed(TypeNode, u32),
    /// A MULTI-payload variant's payload — a tuple handle at run time whose elements render FLATTENED
    /// under the variant head (`(Cons h t)`, NOT `(Cons (tuple h t))`). The runtime (descriptor tag 16)
    /// reads it exactly like a `Tuple` (each element via `arr-get`) but renders the elements DIRECTLY as
    /// the variant's children rather than wrapping them in a `tuple` form. Only a `Sum` variant references
    /// a `Spread` (it is the multi-payload variant payload); a genuine tuple VALUE stays a `Tuple`.
    Spread(Vec<u32>),
}

/// A compile-time-built TYPE node for a `Framed` frame's type position, written to the descriptor wire as
/// `[ head ][ n_children ]( TypeNode )*n` and rebuilt+rendered by the runtime's `render_type_node`. A leaf
/// (a scalar/nominal) has no children; `(List Int64)` = head `List`, one child `Int64`; nests arbitrarily.
struct TypeNode {
    head: String,
    children: Vec<TypeNode>,
}

/// Builds the shape table, memoizing each `Ty::Sum` by its declaration occurrence so a recursive
/// reference reuses the same entry (a `Ref`) rather than expanding forever.
#[derive(Default)]
struct ShapeTableBuilder {
    table: Vec<ShapeNode>,
    /// sum decl → its table index (filled BEFORE the variants are built, so a self-reference resolves).
    sums: std::collections::HashMap<StructId, u32>,
}

impl ShapeTableBuilder {
    fn push(&mut self, node: ShapeNode) -> u32 {
        self.table.push(node);
        (self.table.len() - 1) as u32
    }

    /// The table index of a shape for `ty`, building it (and its sub-shapes) if new. A `Ty::Sum` already
    /// in progress returns a `Ref` to its (reserved) entry, closing recursion. `None` for an
    /// unrenderable leaf type (Float/Str/Bytes — a later slice).
    fn shape_of(&mut self, db: &mut Db, ty: &crate::ty::Ty) -> Option<u32> {
        use crate::ty::Ty;
        Some(match ty {
            Ty::Int(_) => self.push(ShapeNode::Int),
            // A runtime BigInt (arbitrary precision) escapes as `ShapeNode::BigInt` — the runtime reads it
            // via `unbox_bigint` and renders the same arbitrary-width `KIND_INT` leaf as a fixed-width int.
            Ty::BigInt => self.push(ShapeNode::BigInt),
            Ty::Bool => self.push(ShapeNode::Bool),
            // A FLOAT payload is renderable at BOTH widths: `box_op_ty`/`get_op_ty` box it via its width's
            // leaf (`box-float`/`box-float32`), and the runtime `value-encode` renders a KIND_FLOAT decimal
            // — Float64 from the f64, Float32 from its OWN 4-byte leaf (the f32's shortest decimal).
            Ty::Float(ft) if ft.ground_width() == 64 => self.push(ShapeNode::Float),
            Ty::Float(ft) if ft.ground_width() == 32 => self.push(ShapeNode::Float32),
            Ty::String => self.push(ShapeNode::Str),
            Ty::Bytes => self.push(ShapeNode::Bytes),
            Ty::Unit => self.push(ShapeNode::Unit),
            Ty::Tuple(elems) => {
                if elems.is_empty() {
                    return Some(self.push(ShapeNode::Unit));
                }
                let mut idxs = Vec::with_capacity(elems.len());
                for e in elems.iter() {
                    idxs.push(self.shape_of(db, e)?);
                }
                self.push(ShapeNode::Tuple(idxs))
            }
            Ty::List(elem) => {
                let e = self.shape_of(db, elem)?;
                self.push(ShapeNode::List(e))
            }
            // A SET renders `(Set.of (list …))` with elements in CANONICAL key-VALUE order. The runtime
            // value-encode sorts by the element's scalar value, matching `const_key_order` — which only
            // orders SCALAR keys (Int/Bool/Unit/String). So admit a set only over such an element (a
            // nested-compound element has no canonical scalar order → decline, as the const escape does).
            Ty::Set(elem)
                if matches!(
                    elem.strip_nominal(),
                    Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::String
                ) =>
            {
                let e = self.shape_of(db, elem)?;
                self.push(ShapeNode::Set(e))
            }
            // A MAP renders `(map (k1 v1) …)` with entries in CANONICAL KEY order. The runtime sorts by
            // the KEY's scalar value (matching `const_key_order`), so admit a map only over a SCALAR key
            // (Int/Bool/Unit/String); the VALUE may be any encodable shape (the walk recurses on it). A
            // nested-compound key has no canonical scalar order → decline, as the const map escape does.
            Ty::Map(key, val)
                if matches!(
                    key.strip_nominal(),
                    Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::String
                ) =>
            {
                let k = self.shape_of(db, key)?;
                let v = self.shape_of(db, val)?;
                self.push(ShapeNode::Map(k, v))
            }
            Ty::Record(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for (name, t) in fields.iter() {
                    let idx = self.shape_of(db, t)?;
                    out.push((name.name.clone(), idx));
                }
                self.push(ShapeNode::Record(out))
            }
            Ty::Sum { decl, .. } => {
                // Already building/built this sum → a Ref to its reserved entry (closes recursion).
                if let Some(&existing) = self.sums.get(decl) {
                    return Some(self.push(ShapeNode::Ref(existing)));
                }
                // Reserve THIS sum's entry index BEFORE building the variants (a variant payload that
                // references the sum resolves to this index). Fill it in place once the variants are built.
                let self_ix = self.push(ShapeNode::Unit); // placeholder, overwritten below
                self.sums.insert(*decl, self_ix);
                let variants = sum_variant_payload_types(db, ty)?;
                let mut out = Vec::with_capacity(variants.len());
                for (head, payload_tys) in variants {
                    // A variant's payload shape: no payload → Unit; one → that type; MANY → a SPREAD.
                    // A MULTI-payload variant `(Cons Int64 L)` boxes its payloads as a tuple handle at run
                    // time, but its CANONICAL value form is FLAT — `(Cons h t)`, matching both the surface
                    // construction `(L.Cons h t)` and the non-recursive `sum_form_template` render
                    // (`(Variant p0 p1 …)`). So it uses `Spread` (the runtime renders the variant head
                    // followed by each tuple ELEMENT, no `tuple` wrapper) — NOT `Tuple` (which would render
                    // `(Cons (tuple h t))`, exposing the internal boxing). A SINGLE tuple-typed payload
                    // `(Cons (Tuple Int64 L))` is a genuine one-payload variant whose payload IS a tuple, so
                    // it takes the `1 =>` arm and renders `(Cons (tuple h t))` correctly.
                    let payload_ix = match payload_tys.len() {
                        0 => self.push(ShapeNode::Unit),
                        1 => self.shape_of(db, &payload_tys[0])?,
                        _ => {
                            let mut idxs = Vec::with_capacity(payload_tys.len());
                            for pt in &payload_tys {
                                idxs.push(self.shape_of(db, pt)?);
                            }
                            self.push(ShapeNode::Spread(idxs))
                        }
                    };
                    out.push((head, payload_ix));
                }
                self.table[self_ix as usize] = ShapeNode::Sum(out);
                self_ix
            }
            // A NOMINAL newtype is ERASED at run time — the value IS its underlying value — so its shape
            // is its inner's shape, wrapped in `Named(<type name>, …)` so the host renders `(: <underlying>
            // <TypeName>)`. Recursion closes on the nominal's OWN `decl` (a RECURSIVE newtype's inner
            // re-references it): reserve the entry keyed by `decl` BEFORE building the inner (a
            // self-reference resolves to a `Ref`), then fill it. The inner's `Ty::Sum{decl}` back-edge (the
            // erased-newtype μ-binder) resolves to this same reserved entry via `sums`, so the shape table
            // is finite. Reuses `Named`, which the runtime `value-encode` walker already renders.
            Ty::Nominal {
                decl, name, inner, ..
            } => {
                if let Some(&existing) = self.sums.get(decl) {
                    return Some(self.push(ShapeNode::Ref(existing)));
                }
                let self_ix = self.push(ShapeNode::Unit); // placeholder, filled below
                self.sums.insert(*decl, self_ix);
                let inner_ix = self.shape_of(db, inner)?;
                self.table[self_ix as usize] = ShapeNode::Named(name.clone(), inner_ix);
                self_ix
            }
            // Float payload rendering is a later slice — decline (the escape falls through). Str/Bytes are
            // supported (→ `ShapeNode::Str`/`Bytes`, above); the runtime `value-encode` renders their leaves.
            _ => return None,
        })
    }

    /// Serialize the table + root to the descriptor wire format (all counts/lengths unsigned LEB128).
    fn encode(&self, root: u32) -> Vec<u8> {
        fn leb(out: &mut Vec<u8>, mut v: u64) {
            loop {
                let mut b = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    b |= 0x80;
                }
                out.push(b);
                if v == 0 {
                    break;
                }
            }
        }
        fn name(out: &mut Vec<u8>, s: &str) {
            leb(out, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        let mut d = Vec::new();
        leb(&mut d, self.table.len() as u64);
        for node in &self.table {
            match node {
                ShapeNode::Int => d.push(0),
                ShapeNode::BigInt => d.push(17), // matches the runtime `decode_shape` tag 17 = BigInt
                ShapeNode::Bool => d.push(1),
                ShapeNode::Float => d.push(2), // matches the runtime `decode_shape` tag 2 = Float
                ShapeNode::Float32 => d.push(14), // matches the runtime `decode_shape` tag 14 = Float32
                ShapeNode::Str => d.push(3),      // matches the runtime `decode_shape` tag 3 = Str
                ShapeNode::Bytes => d.push(4), // matches the runtime `decode_shape` tag 4 = Bytes
                ShapeNode::Unit => d.push(5),
                ShapeNode::Tuple(idxs) => {
                    d.push(6);
                    leb(&mut d, idxs.len() as u64);
                    for &i in idxs {
                        leb(&mut d, i as u64);
                    }
                }
                ShapeNode::List(i) => {
                    d.push(7);
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Record(fields) => {
                    d.push(8);
                    leb(&mut d, fields.len() as u64);
                    for (k, i) in fields {
                        name(&mut d, k);
                        leb(&mut d, *i as u64);
                    }
                }
                ShapeNode::Sum(variants) => {
                    d.push(9);
                    leb(&mut d, variants.len() as u64);
                    for (h, i) in variants {
                        name(&mut d, h);
                        leb(&mut d, *i as u64);
                    }
                }
                ShapeNode::Named(n, i) => {
                    d.push(10);
                    name(&mut d, n);
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Ref(i) => {
                    d.push(11);
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Set(i) => {
                    d.push(12); // matches the runtime `decode_shape` tag 12 = Set
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Map(k, v) => {
                    d.push(13); // matches the runtime `decode_shape` tag 13 = Map
                    leb(&mut d, *k as u64);
                    leb(&mut d, *v as u64);
                }
                ShapeNode::Framed(type_node, i) => {
                    // recursive TypeNode wire: [ head ][ n_children ]( TypeNode )*n — the runtime's
                    // `decode_type_node` mirrors this depth-first walk.
                    fn write_type_node(out: &mut Vec<u8>, tn: &TypeNode) {
                        fn leb(out: &mut Vec<u8>, mut v: u64) {
                            loop {
                                let mut b = (v & 0x7f) as u8;
                                v >>= 7;
                                if v != 0 {
                                    b |= 0x80;
                                }
                                out.push(b);
                                if v == 0 {
                                    break;
                                }
                            }
                        }
                        leb(out, tn.head.len() as u64);
                        out.extend_from_slice(tn.head.as_bytes());
                        leb(out, tn.children.len() as u64);
                        for c in &tn.children {
                            write_type_node(out, c);
                        }
                    }
                    d.push(15); // matches the runtime `decode_shape` tag 15 = Framed (14 = Float32)
                    write_type_node(&mut d, type_node);
                    leb(&mut d, *i as u64);
                }
                ShapeNode::Spread(idxs) => {
                    d.push(16); // matches the runtime `decode_shape` tag 16 = Spread
                    leb(&mut d, idxs.len() as u64);
                    for &i in idxs {
                        leb(&mut d, i as u64);
                    }
                }
            }
        }
        leb(&mut d, root as u64);
        d
    }
}

/// The variants of a `Ty::Sum` as `(head-name, payload-types)` pairs at this instantiation — the head
/// spelled as the runtime template writes it (a BARE variant name; the value form renders variants bare,
/// e.g. `(Cons …)`, `(None unit)`). Mirrors `sum_form_template`'s variant/payload recovery.
fn sum_variant_payload_types(
    db: &mut Db,
    ty: &crate::ty::Ty,
) -> Option<Vec<(String, Vec<crate::ty::Ty>)>> {
    let crate::ty::Ty::Sum { decl, args, .. } = ty else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(*decl)?;
    let params = decl_ref.params.clone();
    let variants: Vec<(String, Vec<StructId>)> = decl_ref
        .variants
        .iter()
        .map(|v| (v.name.clone(), v.payloads.clone()))
        .collect();
    let mut out = Vec::with_capacity(variants.len());
    for (head, payload_occs) in variants {
        let mut payload_tys = Vec::with_capacity(payload_occs.len());
        for &p in &payload_occs {
            let pty = match db.ast.as_name(p) {
                Some(n) if params.iter().any(|q| q == n) => {
                    let idx = params.iter().position(|q| q == n).unwrap();
                    args.get(idx).cloned()?
                }
                _ => crate::eval::typeval_of(db, p)?,
            };
            payload_tys.push(pty);
        }
        out.push((head, payload_tys));
    }
    Some(out)
}

/// One variant's value-form template: `(: <variant-head> payload…) SumType)`, payload leaves as holes
/// reached via `sum-payload`. Arity shapes the value + the hole paths (see [`sum_form_template`]). The
/// variant HEAD is built by [`variant_head_ast`] (bare normally, qualified `(. Type Variant)` when the
/// sum has a prelude-shadowed variant), so the runtime template writes the identical head the constant
/// bake does.
fn variant_form_template(
    db: &mut Db,
    decl: StructId,
    disc: u32,
    payloads: &[crate::ty::Ty],
    sum_ty: &crate::ty::Ty,
) -> Option<ValueFormTemplate> {
    let mut b = crate::ast::Builder::new();
    let colon = b.name(":");
    let mut leaves: Vec<PendingLeaf> = Vec::new();
    // The VALUE: `(<variant-head> payload…)`.
    let value = {
        let head = variant_head_ast(db, &mut b, decl, disc)?;
        let mut children = vec![head];
        match payloads.len() {
            // Nullary: `(VariantName unit)` — the corpus form (`(None unit)`), no holes.
            0 => {
                children.push(b.name("unit"));
            }
            // Single payload: reached DIRECTLY off the payload handle — `via_sum_payload`, empty path.
            1 => {
                let mut path = Vec::new();
                children.push(template_value_ast_flagged(
                    &mut b,
                    &payloads[0],
                    &mut path,
                    &mut leaves,
                    true,
                )?);
            }
            // Multiple payloads: the payload is a tuple handle — `arr-get(i)` into it, `via_sum_payload`.
            _ => {
                for (i, pty) in payloads.iter().enumerate() {
                    let mut path = vec![i as u32];
                    children.push(template_value_ast_flagged(
                        &mut b,
                        pty,
                        &mut path,
                        &mut leaves,
                        true,
                    )?);
                }
            }
        }
        b.list(children)
    };
    // The TYPE node — the sum's full type surface: a bare `Sign` for a monomorphic sum, `(Option
    // Int64)` for a generic instantiation (`type_ast`'s `Ty::Sum` arm renders both from the solved
    // type). So `(: (Some 5) (Option Int64))` — the corpus parameterized form.
    let type_node = type_ast(&mut b, sum_ty)?;
    let root = b.list(vec![colon, value, type_node]);
    let arenas = b.finish(root);
    let bytes = crate::codec::encode(&arenas);
    let holes = resolve_leaf_offsets(&bytes, &arenas, &leaves)?;
    Some(ValueFormTemplate {
        bytes,
        leaves: holes,
    })
}

/// A leaf recorded during template construction, before its byte offset is resolved: its arena `LeafId`
/// (to locate it in the encoded pool) plus the runtime info the hole carries.
struct PendingLeaf {
    leaf_id: crate::ast::LeafId,
    path: Vec<u32>,
    kind: LeafFill,
    /// Whether this leaf is reached through `sum-payload` first (a sum variant payload leaf) — carried
    /// onto the resolved [`RuntimeLeaf`]. `false` for a plain tuple/record leaf.
    via_sum_payload: bool,
}

/// Build the VALUE s-expression for a type with PLACEHOLDER leaves, recording each scalar leaf's walk
/// `path` (the `arr-get` indices to reach it) and kind. A tuple/record recurses, pushing the positional
/// index onto the path; a scalar emits a placeholder atom and records a `PendingLeaf`. `None` for a type
/// with no value surface.
fn template_value_ast(
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
    path: &mut Vec<u32>,
    out: &mut Vec<PendingLeaf>,
) -> Option<StructId> {
    template_value_ast_flagged(b, ty, path, out, false)
}

/// The core of [`template_value_ast`] with the `via_sum_payload` flag threaded onto each recorded leaf
/// — set when building a sum VARIANT PAYLOAD's sub-template (the leaves are reached through
/// `sum-payload` first). The flat tuple/record path passes `false`.
fn template_value_ast_flagged(
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
    path: &mut Vec<u32>,
    out: &mut Vec<PendingLeaf>,
    via_sum_payload: bool,
) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    use crate::ty::Ty;
    match ty {
        Ty::Int(_) => {
            // Placeholder: a positive zero with a FIXED 8-byte magnitude, so the template reserves an
            // 8-byte hole (len=8) the runtime overwrites with the leaf's big-endian magnitude (a
            // non-minimal magnitude decodes fine — `BigInt::from_bytes_be` drops leading zeros). Pushed
            // NON-deduped (`leaf_unique`) so this occurrence has its OWN pool entry and hence its own
            // byte offset — two equal placeholders must not collapse to one hole.
            let leaf_id = b.leaf_unique(Leaf::Int {
                value: crate::ast::IntValue {
                    negative: false,
                    magnitude: vec![0u8; 8],
                },
                radix: Radix::Dec,
            });
            let atom = b.atom(leaf_id);
            out.push(PendingLeaf {
                leaf_id,
                path: path.clone(),
                kind: LeafFill::Int,
                via_sum_payload,
            });
            Some(atom)
        }
        Ty::Bool => {
            // Placeholder `false`; the runtime overwrites the kind byte (8=false / 9=true). Pushed
            // NON-deduped so each bool occurrence has its own pool entry + offset.
            let leaf_id = b.leaf_unique(Leaf::Bool(false));
            let atom = b.atom(leaf_id);
            out.push(PendingLeaf {
                leaf_id,
                path: path.clone(),
                kind: LeafFill::Bool,
                via_sum_payload,
            });
            Some(atom)
        }
        Ty::Tuple(elems) => {
            let head = b.name("tuple");
            let mut children = vec![head];
            for (i, e) in elems.iter().enumerate() {
                path.push(i as u32);
                children.push(template_value_ast_flagged(
                    b,
                    e,
                    path,
                    out,
                    via_sum_payload,
                )?);
                path.pop();
            }
            Some(b.list(children))
        }
        Ty::Record(fields) => {
            let head = b.name("record");
            let mut children = vec![head];
            // A record is a positional heap array in canonical (sorted) field order — the same order the
            // BTreeMap iterates, so the `arr-get` index is the field's position in that order.
            for (i, (name, t)) in fields.iter().enumerate() {
                let fname = b.name(&name.name);
                path.push(i as u32);
                let fval = template_value_ast_flagged(b, t, path, out, via_sum_payload)?;
                path.pop();
                children.push(b.list(vec![fname, fval]));
            }
            Some(b.list(children))
        }
        _ => None,
    }
}

/// Resolve each pending leaf's BYTE OFFSET in the encoded template. Re-encodes the leaf pool the same
/// way `codec::encode` does (header + count, then each leaf), tracking the running offset; when a leaf's
/// `LeafId` matches a pending runtime leaf, its hole offset is the magnitude position (Int: after the
/// kind + len bytes) or the kind-byte position (Bool). Returns the resolved holes in the pending order.
fn resolve_leaf_offsets(
    bytes: &[u8],
    arenas: &crate::ast::Arenas,
    pending: &[PendingLeaf],
) -> Option<Vec<RuntimeLeaf>> {
    // Offset walk mirrors `codec::encode`: 8-byte header, then a LEB128 leaf-count, then each leaf.
    let mut off = 8usize;
    off += leb_len(arenas.leaves.len() as u64);
    // Map each LeafId → (magnitude offset for Int, kind-byte offset for Bool).
    let mut leaf_off: std::collections::HashMap<u32, (usize, LeafFill)> =
        std::collections::HashMap::new();
    for (i, leaf) in arenas.leaves.iter().enumerate() {
        let kind_off = off;
        match leaf {
            crate::ast::Leaf::Int { value, .. } => {
                // kind byte (1) + len LEB + magnitude.
                let len = value.magnitude.len();
                let mag_off = off + 1 + leb_len(len as u64);
                leaf_off.insert(i as u32, (mag_off, LeafFill::Int));
                off = mag_off + len;
            }
            crate::ast::Leaf::Bool(_) => {
                leaf_off.insert(i as u32, (kind_off, LeafFill::Bool));
                off += 1;
            }
            crate::ast::Leaf::Name(n) => {
                off += 1 + leb_len(n.len() as u64) + n.len();
            }
            crate::ast::Leaf::Str(s) => {
                off += 1 + leb_len(s.len() as u64) + s.len();
            }
            // A symbol leaf encodes like a Str/Name (kind byte + len LEB + utf8 bytes) and is compile-
            // time-only (a unit erases before the boundary — a symbol never reaches a runtime value
            // form), so advance past it with no runtime hole.
            crate::ast::Leaf::Sym(s) => {
                off += 1 + leb_len(s.len() as u64) + s.len();
            }
            // A bytes leaf is a fully-baked constant (no runtime hole) — advance past it like a Str
            // (kind byte + len LEB + the raw bytes).
            crate::ast::Leaf::Bytes(bs) => {
                off += 1 + leb_len(bs.len() as u64) + bs.len();
            }
            crate::ast::Leaf::Float(_) => return None, // floats not yet in the runtime escape
            // A char leaf encodes like a Str (kind byte + len LEB + utf8 bytes); a char does not yet
            // cross the boundary in the runtime escape, so advance past it (no runtime hole).
            crate::ast::Leaf::Char(c) => {
                off += 1 + leb_len(c.len_utf8() as u64) + c.len_utf8();
            }
            // A bad-escape / bad-char marker is a POISON — it never reaches a constant value form
            // (resolving it rejects CDZ0001/CDZ0002 before any escape emission), so a runtime template
            // over it is meaningless.
            crate::ast::Leaf::BadEscape(_) | crate::ast::Leaf::BadChar(_) => return None,
        }
    }
    let _ = bytes;
    let mut holes = Vec::with_capacity(pending.len());
    for p in pending {
        let (offset, _) = leaf_off.get(&p.leaf_id.0)?;
        holes.push(RuntimeLeaf {
            offset: *offset,
            path: p.path.clone(),
            kind: p.kind,
            via_sum_payload: p.via_sum_payload,
        });
    }
    Some(holes)
}

/// The number of bytes the unsigned LEB128 encoding of `n` occupies (matches `encode::uleb128`).
fn leb_len(mut n: u64) -> usize {
    let mut c = 1;
    while n >= 0x80 {
        n >>= 7;
        c += 1;
    }
    c
}

/// Build the variant HEAD s-expression for variant `disc` of the sum declared at `decl`, as it appears
/// in an observed value's canonical form: the variant's BARE NAME atom — `Some`, `Sm`, `Cons`, `Pos`. A
/// variant renders the SAME whether its sum is BUILT-IN (Option/Result) or USER-declared: the value form
/// of a variant does not depend on where its sum was declared (the built-in-vs-user split that rendered a
/// user variant as the member-access `(. Type Variant)` while a built-in rendered bare was an
/// inconsistency — a rendered VALUE should be a variant name, not a projection expression). The rendered
/// value is always annotated with its sum type (`(: (Sm 42) Opt)`), which disambiguates a bare variant
/// name shared across sums (sum identity is by declaration occurrence, carried by the annotation). `None`
/// if the disc is out of range (a compiler bug). Shared by the constant-escape bake and the
/// runtime-escape template so both write the identical head.
fn variant_head_ast(
    db: &mut Db,
    b: &mut crate::ast::Builder,
    decl: StructId,
    disc: u32,
) -> Option<StructId> {
    let t = db.type_decl_by_occ(decl)?;
    let tname = t.name.clone();
    let vname = t.variants.get(disc as usize)?.name.clone();
    // A variant head normally renders BARE (`Some`, `Cons`, `Neg`) — the value reads back because the
    // bare name resolves to that variant. But when a variant name is SHADOWED by a prelude entry that is
    // NOT a variant ctor (`Ast.Int`/`Ast.List` — `Int` is the integer type ctor, `List` the list
    // module), a bare head would read back as that other binding, not the variant, so the value form
    // would not round-trip. Such a sum renders EVERY head QUALIFIED `(. Type Variant)` (a consistent
    // per-sum spelling, so mixed variants don't split): the member access resolves unambiguously to the
    // variant. This is the render-side twin of the load-time `variant_ctor_index` prelude-collision skip
    // (`db.rs`) — the same rule (don't let a colliding variant name masquerade as its prelude binding),
    // applied to the escaping VALUE FORM. `Some`/`None` are in the prelude too, but bound to their OWN
    // variant ctors, so they round-trip bare and are NOT qualified.
    if sum_needs_qualified_heads(db, decl) {
        let dot = b.name(".");
        let ty_name = b.name(tname);
        let var_name = b.name(vname);
        return Some(b.list(vec![dot, ty_name, var_name]));
    }
    Some(b.name(vname))
}

/// Whether the sum declared at `decl` must render its variant heads QUALIFIED (see [`variant_head_ast`]):
/// true iff ANY variant name is bound in the prelude to something that is NOT a variant ctor (a type
/// ctor, a module, a value). A per-sum property (not per-variant) so every head of the sum spells the
/// same way. A variant whose prelude binding IS a variant ctor (`Some`/`None`/`Ok`/`Err`) round-trips
/// bare, so it does not force qualification; a variant name absent from the prelude (`Cons`, `Neg`)
/// likewise resolves bare to its own ctor.
fn sum_needs_qualified_heads(db: &mut Db, decl: StructId) -> bool {
    let Some(t) = db.type_decl_by_occ(decl) else {
        return false;
    };
    let names: Vec<String> = t.variants.iter().map(|v| v.name.clone()).collect();
    names
        .iter()
        .any(|name| match db.prelude.get(name).copied() {
            // Bound in the prelude to a non-variant-ctor (no `(meta variant)`) → bare would resolve to
            // that OTHER binding, so the whole sum must qualify.
            Some(occ) => crate::eval::variant_disc_of(db, occ).is_none(),
            // Not a prelude name → bare resolves to this sum's own variant ctor; no qualification needed.
            None => false,
        })
}

/// Reconstruct the VALUE s-expression of a constant node into `b`: a scalar → its literal atom; a
/// `Core::Tuple` → `(tuple <elem>…)`; a `Core::Record` → `(record (<name> <value>)…)` in canonical field
/// order. `None` if the node is not a constant the escape path can bake.
fn const_value_ast(db: &mut Db, b: &mut crate::ast::Builder, id: StructId) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    // A QUANTITY value renders its CONSTRUCTION form `(Qty.of <inner-value> <unit>)` — the unit is a
    // compile-time value that erased from the core (a `Qty.of` node lowers to its inner value's core),
    // so it is recovered from the SOLVED TYPE `Ty::Qty` and re-materialized as source structure. This is
    // checked FIRST (before the core match) because the erased core is a bare scalar (`ConstFloat`),
    // which would otherwise render as the bare number, losing the quantity the corpus records.
    if let crate::ty::Ty::Qty { inner, unit } = crate::infer::type_of(db, id) {
        // The inner VALUE: the erased core IS the inner value (Qty.of erases to it), so render it at the
        // inner type by recursing on the SAME node with the quantity peeled — build a synthetic inner
        // render by matching the core directly (the node's core is the inner numeric's core).
        let inner_val = const_value_ast_at(db, b, id, &inner)?;
        let unit_ast = unit_value_ast(b, &unit);
        // `((. Qty of) <value> <unit>)` — the member-access form the reader normalizes `(Qty.of …)` to
        // (a dotted name `Qty.of` desugars to `(. Qty of)`), so the baked value re-reads/re-prints to
        // the SAME canonical shape the corpus records.
        let qty_of = member_access(b, "Qty", "of");
        return Some(b.list(vec![qty_of, inner_val, unit_ast]));
    }
    // A SYMBOL value renders its CONSTRUCTION form `((. Symbol of) "text")` (17-symbols "a symbol is
    // constructed from a string"), NOT the bare string its `Core::ConstStr` rep would otherwise render.
    // A symbol shares the constant-string rep (its identity is its text — see `Resolved::SymbolConst`),
    // so the core is a `ConstStr`; recover the SYMBOL surface from the SOLVED TYPE `Ty::Symbol` and
    // re-materialize the `Symbol.of` construction as source, exactly as `Ty::Qty` recovers `Qty.of`.
    // Checked FIRST (before the core match) since the erased core would otherwise render as a bare String.
    if matches!(crate::infer::type_of(db, id), crate::ty::Ty::Symbol)
        && let Core::ConstStr(s) = core_of(db, id)
    {
        let symbol_of = member_access(b, "Symbol", "of");
        let text = b.atom_leaf(Leaf::Str(s));
        return Some(b.list(vec![symbol_of, text]));
    }
    match core_of(db, id) {
        Core::ConstInt(v) => Some(b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })),
        // A constant float bakes as its exact decimal leaf — the codec encodes it (KIND_FLOAT), and the
        // host reader renders it back. A quantity over a Float64 magnitude reaches here through
        // `const_value_ast_at` (the inner-value render of `Qty.of`).
        Core::ConstFloat(d) => Some(b.atom_leaf(Leaf::Float(d))),
        Core::ConstBool(x) => Some(b.atom_leaf(Leaf::Bool(x))),
        // A constant string bakes as its `"…"` leaf — the codec encodes it (KIND_STR: len + UTF-8
        // bytes), and the host reader lifts it back to a string value.
        Core::ConstStr(s) => Some(b.atom_leaf(Leaf::Str(s))),
        // A constant char bakes as its `#\c` leaf — the codec encodes it (KIND_CHAR), and the host reader
        // renders it `#\c`. This lets a constant `(Some #\a)` (a `Char.from-int` fold) cross the boundary.
        Core::ConstChar(c) => Some(b.atom_leaf(Leaf::Char(c))),
        // The unit value bakes as the `unit` name leaf — ONE canonical byte form, distinct from every
        // other value's form (no other value renders as the bare `unit` name), so a program that produces
        // only its emitted events still has a serializable normal-termination value.
        //= spec/contracts/deterministic-value-form.md#the-unit-value-has-a-canonical-byte-form
        //# The unit value MUST have exactly one canonical byte encoding, so that a program that produces no value other than its emitted events has a serializable normal-termination value.
        //= spec/contracts/deterministic-value-form.md#the-unit-value-has-a-canonical-byte-form
        //# The canonical byte encoding of the unit value MUST be distinct from that of every other value, consistent with structural equality treating the unit value as equal only to itself.
        Core::Unit => Some(b.name("unit")),
        Core::Tuple { elems } => {
            let head = b.name("tuple");
            let mut children = vec![head];
            for e in elems {
                children.push(const_value_ast(db, b, e)?);
            }
            Some(b.list(children))
        }
        Core::Record { fields } => {
            let head = b.name("record");
            let mut children = vec![head];
            // Canonical (sorted) field order — a `BTreeMap` iterates sorted, matching the type render.
            for (name, &v) in fields.iter() {
                let fname = b.name(name.name.clone());
                let fval = const_value_ast(db, b, v)?;
                children.push(b.list(vec![fname, fval]));
            }
            Some(b.list(children))
        }
        // A CONSTANT list literal renders `(list e1 e2 …)` — its length is statically known (unlike a
        // grown/runtime list), so its bytes bake exactly like a constant tuple's. Each element is a
        // constant in turn (a non-constant element makes the whole value non-constant, so `core_of` would
        // not be a `ListNew` of constants and this returns `None`, declining the escape). A list is an
        // ORDERED aggregate — the render walks `elems` in order, so the canonical form preserves element
        // order (unlike the map/set render, which sorts an unordered aggregate).
        //= spec/contracts/deterministic-value-form.md#ordering-of-aggregate-members-is-fixed
        //# The canonical encoding of an ordered aggregate MUST preserve its element order.
        Core::ListNew { elems } => {
            let head = b.name("list");
            let mut children = vec![head];
            for e in elems {
                children.push(const_value_ast(db, b, e)?);
            }
            Some(b.list(children))
        }
        // A CONSTANT map value — `(map (k1 v1) (k2 v2) …)` — its entries rendered in CANONICAL KEY ORDER,
        // independent of insertion order and DISTINGUISHABLE from a record (`map` head, `(key value)`
        // pairs). The constant map already has each key at most once (the `Map.insert` fold replaced by
        // key value); sort the entries by their canonical KEY order (`const_key_order`), then render each
        // pair. A non-constant key/value makes an entry non-constant → `None`, declining the escape (a
        // genuinely runtime map's escape is the deferred looping walker). This is the constant-escape (R1)
        // companion of the map value — a fully-constant map crosses by baked bytes here.
        //= spec/contracts/deterministic-value-form.md#ordering-of-aggregate-members-is-fixed
        //# The canonical encoding of an unordered aggregate MUST place its members in a fixed order derived from the members themselves, not from the order in which they were inserted or discovered.
        //= spec/capabilities/collections-and-text.md#a-map-renders-as-its-entries-in-canonical-key-order
        //# A map's canonical form MUST present its entries as key-value pairs in the deterministic order of *Map Iteration Is Deterministic*, so that two equal maps have identical canonical forms regardless of the order their entries were added.
        //= spec/capabilities/collections-and-text.md#a-map-renders-as-its-entries-in-canonical-key-order
        //# The canonical form MUST be distinguishable from a record's, so that a map and a record are never confused by their rendered form even when they carry the same keys and values (a map's keys are values of one key type; a record's field names are fixed compile-time labels).
        Core::MapNew { entries, .. } => {
            let mut sorted: Vec<(StructId, StructId)> = entries.clone();
            // Sort by canonical key order. A key that is not orderable-as-a-constant declines the whole
            // escape (the runtime walker path is deferred), so a failed comparison bails to `None`.
            let mut orderable = true;
            sorted.sort_by(|a, b| {
                const_key_order(db, a.0, b.0).unwrap_or_else(|| {
                    orderable = false;
                    std::cmp::Ordering::Equal
                })
            });
            if !orderable {
                return None;
            }
            let head = b.name("map");
            let mut children = vec![head];
            for (k, v) in sorted {
                let kv = const_value_ast(db, b, k)?;
                let vv = const_value_ast(db, b, v)?;
                children.push(b.list(vec![kv, vv]));
            }
            Some(b.list(children))
        }
        // A CONSTANT set value — `(Set.of (list e1 e2 …))` — its elements rendered in CANONICAL (sorted)
        // ORDER inside a `(list …)`, wrapped in a `(Set.of …)` form (collections-and-text.md §A Set …
        // canonical written form is `(Set.of (list …))`). The constant set already has each element at
        // most once (the `Set.of`/insert folds dedup by value); sort by `const_key_order` (reused — an
        // element orders exactly like a map key). A non-orderable element declines the escape.
        Core::SetOf { elems, .. } => {
            let mut sorted: Vec<StructId> = elems.clone();
            let mut orderable = true;
            sorted.sort_by(|&x, &y| {
                const_key_order(db, x, y).unwrap_or_else(|| {
                    orderable = false;
                    std::cmp::Ordering::Equal
                })
            });
            if !orderable {
                return None;
            }
            let list_head = b.name("list");
            let mut list_children = vec![list_head];
            for e in sorted {
                list_children.push(const_value_ast(db, b, e)?);
            }
            let inner_list = b.list(list_children);
            // `(Set.of <list>)` — a member-access `(. Set of)` applied to the list. The value form the
            // corpus records is the `Set.of (list …)` application, so build it as `((. Set of) <list>)`.
            let set_mod = b.name("Set");
            let of_key = b.name("of");
            let dot = b.name(".");
            let set_of = b.list(vec![dot, set_mod, of_key]);
            Some(b.list(vec![set_of, inner_list]))
        }
        // A CONSTANT sum value — `(Some 5)`, `(None unit)`, `(Some (Some 5))`. Its canonical form is
        // `(VariantName payload…)` with the variant TAG present (`deterministic-value-form.md`;
        // core-semantics.md §A Constructor Applied To An Argument Is A Sum Value). This holds regardless
        // of what the payload IS — a scalar, a tuple, or ANOTHER sum value — so a NESTED constant sum
        // (`(Some (Some 5))`) bakes recursively, both variant tags present. This is the constant-escape
        // (R1) companion of `sum_form_template`'s runtime walker: a fully-constant sum crosses by baked
        // bytes here, so it never needs the per-variant runtime template (which cannot express a nested
        // sum's variable-length inner shape). The variant NAME is recovered from the disc against this
        // node's solved sum type (its declaration's variant set); a nullary variant carries `unit`.
        Core::SumNew { disc, payloads } => {
            let ty = crate::infer::type_of(db, id);
            let crate::ty::Ty::Sum { decl, .. } = ty else {
                return None; // a SumNew whose solved type is not a sum is a compiler bug — decline
            };
            let head = variant_head_ast(db, b, decl, disc)?;
            let mut children = vec![head];
            match payloads.len() {
                // Nullary variant: `(VariantName unit)` — the corpus form (`(None unit)`).
                0 => children.push(b.name("unit")),
                // Single payload (the canonical variant shape — one payload type, a scalar / tuple /
                // nested sum): render it recursively.
                1 => children.push(const_value_ast(db, b, payloads[0])?),
                // Multiple application arguments (a `(V.Both a b)` multi-arg surface) — not a canonical
                // single-payload form; the escape declines rather than guess a rendering.
                _ => return None,
            }
            Some(b.list(children))
        }
        // A constant `Bytes.of` → a `Leaf::Bytes` value node (rendered `b"…"` by the host). Each element
        // is a constant Int in `0..=255` (range-checked at `lower_bytes_of`); collect the raw bytes. A
        // non-constant element would have declined at `lower_bytes_of` (no `Core::BytesOf` built), so
        // every element here folds to a `ConstInt` in range.
        Core::BytesOf { elems } => {
            let mut raw = Vec::with_capacity(elems.len());
            for e in elems {
                match core_of(db, e) {
                    Core::ConstInt(v) => {
                        raw.push(v.to_i64().filter(|n| (0..=255).contains(n))? as u8)
                    }
                    _ => return None,
                }
            }
            Some(b.atom_leaf(Leaf::Bytes(raw)))
        }
        _ => None,
    }
}

/// Render the constant value at `id` treating it AS having type `expect` — the quantity-inner helper.
/// A `Qty.of` node erases (in `core_of`) to its inner value's core, so rendering the INNER value means
/// rendering that same core, but WITHOUT re-triggering the `Ty::Qty` branch of `const_value_ast` (which
/// reads `type_of(id)` = the whole quantity type). `expect` is the inner numeric type; for a scalar
/// (int/float/bool) the value form is the same as `const_value_ast`'s scalar arms, so match the core
/// directly here. (A quantity over a COMPOUND inner type is not a Layer-1 case — the numeric core is
/// scalar — so a non-scalar inner declines the escape by `None`.)
fn const_value_ast_at(
    db: &mut Db,
    b: &mut crate::ast::Builder,
    id: StructId,
    expect: &crate::ty::Ty,
) -> Option<StructId> {
    use crate::ast::{Leaf, Radix};
    let _ = expect; // the inner is a scalar in Layer 1; the core discriminates directly
    match core_of(db, id) {
        Core::ConstInt(v) => Some(b.atom_leaf(Leaf::Int {
            value: v,
            radix: Radix::Dec,
        })),
        Core::ConstFloat(d) => Some(b.atom_leaf(Leaf::Float(d))),
        Core::ConstBool(x) => Some(b.atom_leaf(Leaf::Bool(x))),
        // A non-scalar inner value is not a Layer-1 quantity magnitude — decline the escape.
        _ => None,
    }
}

/// Materialize a compile-time `Unit` value as SOURCE structure — the `<unit>` position of a rendered
/// `(Qty.of <value> <unit>)` and of a `(Qty T <unit>)` type. The dimensionless unit renders `Unit.one`;
/// a single base to the first power renders `((. Unit base) #"name")`; a base to a power `(Unit.^ …
/// k)`; a product of positive factors a left-nested `(Unit.* …)`; and — crucially — a unit with
/// NEGATIVE exponents renders as a QUOTIENT `(Unit./ <numerator> <denominator>)`, the surface the corpus
/// records for a derived unit (`(Unit./ meter second)` for a velocity, NOT `(Unit.* meter (Unit.^ second
/// -1))`). The numerator is the positive-exponent factors (`Unit.one` if none); the denominator the
/// negative-exponent factors with their exponents made positive. Uses the `#"name"` SYMBOL leaf per base
/// so the rendered unit re-reads to the same `Unit`. `Unit.base` is member access; `Unit.^`/`Unit.*`/
/// `Unit./` stay BARE names (their segment is not alphabetic, so the reader does not desugar them).
fn unit_value_ast(b: &mut crate::ast::Builder, unit: &crate::ty::Unit) -> StructId {
    use crate::ast::Leaf;
    let entries: Vec<(String, i64)> = unit.entries().map(|(n, e)| (n.clone(), *e)).collect();
    if entries.is_empty() {
        // `Unit.one` — the dimensionless unit, the member-access form `(. Unit one)`.
        return member_access(b, "Unit", "one");
    }
    // One base factor at a (positive) exponent: `((. Unit base) #"name")` or `(Unit.^ … k)`.
    fn factor(b: &mut crate::ast::Builder, name: &str, exp: i64) -> StructId {
        let base_head = member_access(b, "Unit", "base");
        let sym = b.atom_leaf(Leaf::Sym(name.to_string()));
        let base = b.list(vec![base_head, sym]);
        if exp == 1 {
            base
        } else {
            let pow_head = b.name("Unit.^");
            let n = b.atom_leaf(Leaf::Int {
                value: crate::ast::IntValue::from_i64(exp),
                radix: crate::ast::Radix::Dec,
            });
            b.list(vec![pow_head, base, n])
        }
    }
    // Left-nested product of a factor list, or `Unit.one` when empty.
    fn product(b: &mut crate::ast::Builder, factors: &[(String, i64)]) -> StructId {
        if factors.is_empty() {
            return member_access(b, "Unit", "one");
        }
        let mut acc = factor(b, &factors[0].0, factors[0].1);
        for (name, exp) in &factors[1..] {
            let f = factor(b, name, *exp);
            let mul_head = b.name("Unit.*");
            acc = b.list(vec![mul_head, acc, f]);
        }
        acc
    }
    // Split into positive (numerator) and negative (denominator, exponents made positive) factors.
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
        // All positive — a plain product (or a single factor).
        return product(b, &num);
    }
    // A quotient `(Unit./ numerator denominator)` — the derived-unit surface.
    let numerator = product(b, &num);
    let denominator = product(b, &den);
    let div_head = b.name("Unit./");
    b.list(vec![div_head, numerator, denominator])
}

/// Build a member-access form `(. <operand-name> <key>)` — the canonical shape the reader normalizes a
/// dotted name `Operand.key` to (an alphabetic-segment postfix desugar). Used to bake a unit/quantity
/// value form so it re-reads to the same tree the corpus records (`(. Qty of)`, `(. Unit base)`).
fn member_access(b: &mut crate::ast::Builder, operand: &str, key: &str) -> StructId {
    let dot = b.name(".");
    let op = b.name(operand.to_string());
    let k = b.name(key.to_string());
    b.list(vec![dot, op, k])
}

/// Reconstruct a TYPE s-expression into `b`, matching `Ty::render_name`'s surface exactly so the host
/// prints the recorded type: `Int64`/`UInt8`/… as a name atom, `Bool`/`Unit` likewise, a tuple as
/// `(Tuple T…)`, a record as `(record (name T)…)`. `None` for a type with no value-form surface (a
/// function/type-value/unsolved variable can never be a runtime value crossing the boundary).
fn type_ast(b: &mut crate::ast::Builder, ty: &crate::ty::Ty) -> Option<StructId> {
    use crate::ty::Ty;
    match ty {
        // A scalar's type surface is its name atom. `String`/`Char`/`Symbol`/`BigInt` are monomorphic
        // named types too, so their surface is the bare `String`/`Char`/`Symbol`/`BigInt` atom
        // (`render_name`).
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::String | Ty::Char | Ty::Symbol | Ty::BigInt => {
            Some(b.name(ty.render_name()))
        }
        // A sum's type surface: the bare NAME for a monomorphic sum (`(: (Neg unit) Sign)`), or the
        // STRUCTURED application `(Option Int64)` for a generic instantiation — a `(NAME arg…)` list, so
        // the args round-trip as separate nodes (not one spaced-out name atom). Matches `render_name`'s
        // surface but built as real structure so the codec + host reader see the parameterized type.
        Ty::Sum { name, args, .. } => {
            if args.is_empty() {
                Some(b.name(name.clone()))
            } else {
                let head = b.name(name.clone());
                let mut children = vec![head];
                for a in args {
                    children.push(type_ast(b, a)?);
                }
                Some(b.list(children))
            }
        }
        Ty::Tuple(elems) => {
            let head = b.name("Tuple");
            let mut children = vec![head];
            for t in elems.iter() {
                children.push(type_ast(b, t)?);
            }
            Some(b.list(children))
        }
        Ty::Record(fields) => {
            // The TYPE head is capitalized `Record` (like `Tuple`); the VALUE head is lowercase `record`
            // (see `const_value_ast`). The corpus writes `(Record (a Int64) …)` for the type.
            let head = b.name("Record");
            let mut children = vec![head];
            for (name, t) in fields.iter() {
                let fname = b.name(name.name.clone());
                let fty = type_ast(b, t)?;
                children.push(b.list(vec![fname, fty]));
            }
            Some(b.list(children))
        }
        // A list's type surface is `(List Elem)` — matches `render_name`.
        Ty::List(elem) => {
            let head = b.name("List");
            let ety = type_ast(b, elem)?;
            Some(b.list(vec![head, ety]))
        }
        // A map's type surface is `(Map Key Value)` — matches `render_name` (key first).
        Ty::Map(k, v) => {
            let head = b.name("Map");
            let kty = type_ast(b, k)?;
            let vty = type_ast(b, v)?;
            Some(b.list(vec![head, kty, vty]))
        }
        // A set's type surface is `(Set Elem)` — one element type parameter (matches `render_name`).
        Ty::Set(elem) => {
            let head = b.name("Set");
            let ety = type_ast(b, elem)?;
            Some(b.list(vec![head, ety]))
        }
        // A bytes value's type surface is the bare name `Bytes` (a leaf, like a scalar) — matches
        // `render_name`; its VALUE renders `b"…"` (built in `const_value_ast` / the escape walker).
        Ty::Bytes => Some(b.name("Bytes".to_string())),
        // A still-free type variable in an escaping value's type has NO defined serialization — a bare
        // `(None)` : `Option ?0` or an empty `(list)` : `List ?0` whose payload/element nothing pins. It
        // is NOT rendered (no honest concrete surface exists): returning `None` here makes
        // `constant_value_form`/`sum_form_template` decline, so the escape falls through to the
        // AMBIGUOUS-TYPE guard in `backend/wasm/mod.rs` (`has_free_var` → CDZ0203, "annotate it") rather
        // than crossing with an invented type. type-system.md §An Escaping Value MUST Have A Fully
        // Determined Type; corpus 07 "an escaped value with an unresolved payload type is rejected".
        // A float's type surface is its aliased width name `Float32`/`Float64` (a leaf, like a scalar) —
        // matches `render_name`; its VALUE renders as the float literal. Needed when a float is NESTED in
        // a compound value form (`(tuple 1.0 2.0)`) whose type annotation the escape bakes.
        Ty::Float(ft) => Some(b.name(format!("Float{}", ft.ground_width()))),
        // A quantity's type surface is `(Qty <inner> <unit>)` — matches `render_name`. The inner type is
        // built recursively; the unit is built as REAL structure by `unit_value_ast` (`Unit.one`,
        // `(Unit.base #"n")`, `(Unit.^ (Unit.base #"n") k)`, left-nested `(Unit.* …)`), so the rendered
        // type re-reads to the same unit (a name atom carrying parens would not round-trip). The corpus
        // surface `(Qty Float64 (Unit.base #"meter"))`.
        Ty::Qty { inner, unit } => {
            let head = b.name("Qty");
            let ity = type_ast(b, inner)?;
            let uty = unit_value_ast(b, unit);
            Some(b.list(vec![head, ity, uty]))
        }
        // A nominal's type surface is its declared NAME atom (`(: (Mk 42) UserId)`) — its identity is
        // the name, not its underlying shape (like a monomorphic sum). The value itself renders as the
        // underlying value form (built by the value walker, which sees through the tag).
        Ty::Nominal { name, .. } => Some(b.name(name.clone())),
        // A function/type-value has no boundary value form, so no type surface. A program that would
        // escape one declines before reaching the escape.
        Ty::Fn(_, _) | Ty::Type | Ty::Var(_) | Ty::Any => None,
    }
}

/// Conditional-constant-propagation helper: if `branch` reduces to an inner `(if c' A B)` whose
/// condition `c'` is EQUIVALENT to the enclosing `cond` (via `core_equiv` — a pure-core structural
/// match), return the occurrence of the arm the enclosing branch's known truth of `cond` selects — `A`
/// when `cond_is_true` (the then-branch, where `cond` holds), `B` otherwise (the else-branch, where it
/// does not). Also handles the NEGATED case: when `c'` is the boolean negation of `cond` (`(not cond)`,
/// or `cond` is `(not c')`), the known truth of `cond` implies the OPPOSITE truth of `c'`, so the FLIPPED
/// arm is selected — `(if c A (if (not c) B D))` takes `B` in the else-branch (where `c` is false, so
/// `(not c)` is true). `None` if `branch` is not such a nested `if` (leave it unchanged). The returned
/// occurrence is REUSED as-is (no synthesis); it was resolved in the same scope, so lowering it in the
/// branch's place is sound. `reduce_to_if` chases refs/annotations and stops at a kept multi-use binding,
/// so a `let`-named inner `if` is not peeled (its value lives in a slot). Only the DIRECT nested `if` is
/// collapsed here; deeper propagation happens because the rewritten branch re-lowers and can collapse
/// again.
fn collapse_repeated_cond(
    db: &mut Db,
    cond: StructId,
    branch: StructId,
    cond_is_true: bool,
) -> Option<StructId> {
    let (inner_cond, inner_then, inner_else) = crate::eval::reduce_to_if(db, branch)?;
    if core_equiv(db, cond, inner_cond) {
        // `c'` == `cond`: same truth → `cond_is_true` picks the inner then, else the inner else.
        Some(if cond_is_true { inner_then } else { inner_else })
    } else if is_negation_of(db, cond, inner_cond) {
        // `c'` == `!cond`: OPPOSITE truth → flip which arm survives.
        Some(if cond_is_true { inner_else } else { inner_then })
    } else {
        None
    }
}

/// Whether the cores at `a` and `b` are boolean NEGATIONS of each other — one is `Core::Not { operand }`
/// with `operand` `core_equiv` to the other. Both orders are tried (`a` is `(not b)` or `b` is `(not a)`).
/// `not` is total and pure, and `core_equiv` matches only pure cores, so a matched pair is two pure
/// booleans of exactly opposite truth. Used by `collapse_repeated_cond` to propagate a known condition
/// into a nested `if` guarded by that condition's negation.
fn is_negation_of(db: &mut Db, a: StructId, b: StructId) -> bool {
    let one_way = |db: &mut Db, x: StructId, y: StructId| -> bool {
        matches!(core_of(db, x), Core::Not { operand } if core_equiv(db, operand, y))
    };
    one_way(db, a, b) || one_way(db, b, a)
}

/// The number of times the binding whose initializer is `init` is REFERENCED within the resolved
/// subtree rooted at `node` — a use is a `Resolved::Ref { value: init }` (the identity a reference to
/// the binding resolves to). Walks the resolved tree structurally without lowering; a nested `let`
/// that SHADOWS the name rebinds references below it to a different init, so those do not count (they
/// resolve to the inner binding's occurrence, not `init`). Bounded by the subtree size.
fn uses_in(db: &mut Db, node: StructId, init: StructId) -> u32 {
    match resolved_of(db, node) {
        Resolved::Ref { value } => {
            if value == init {
                1
            } else {
                // A ref to ANOTHER binding — but its value may itself reference `init` (e.g. a later
                // `let` binding's initializer). Do not descend through the ref target here: the walk
                // over the enclosing structure already visits every initializer/body position, so
                // counting the ref itself (0 for a different binding) avoids double-counting.
                0
            }
        }
        Resolved::If { cond, then_, else_ } => {
            uses_in(db, cond, init) + uses_in(db, then_, init) + uses_in(db, else_, init)
        }
        Resolved::And { lhs, rhs, .. } => uses_in(db, lhs, init) + uses_in(db, rhs, init),
        Resolved::Not { operand } => uses_in(db, operand, init),
        Resolved::Let { bindings, body } => {
            let mut n = 0;
            for (_, value) in &bindings {
                n += uses_in(db, *value, init);
            }
            n + uses_in(db, body, init)
        }
        Resolved::Record { fields } => {
            let mut n = 0;
            for value in fields.values() {
                n += uses_in(db, *value, init);
            }
            n
        }
        Resolved::Member { operand, .. } => uses_in(db, operand, init),
        Resolved::Bin { segs } => {
            let mut n = 0;
            for s in segs.iter() {
                n += uses_in(db, s.slot, init);
                match &s.kind {
                    crate::resolved::SegKind::Bytes { size: Some(sz) } => {
                        n += uses_in(db, *sz, init)
                    }
                    crate::resolved::SegKind::Utf8 { size } => n += uses_in(db, *size, init),
                    _ => {}
                }
            }
            n
        }
        Resolved::Tuple { elems } | Resolved::List { elems } => {
            let mut n = 0;
            for &e in elems.iter() {
                n += uses_in(db, e, init);
            }
            n
        }
        Resolved::Map { entries } => {
            let mut n = 0;
            for &(k, v) in entries.iter() {
                n += uses_in(db, k, init) + uses_in(db, v, init);
            }
            n
        }
        Resolved::Proj { operand, .. } => uses_in(db, operand, init),
        Resolved::Annot { expr, .. } => uses_in(db, expr, init),
        Resolved::Apply { head, args } => {
            let mut n = uses_in(db, head, init);
            for a in args.iter() {
                n += uses_in(db, *a, init);
            }
            n
        }
        // A match: the scrutinee and every arm body may reference the binding. (A literal pattern is a
        // constant, not a reference.) The scrutinee runs once; each arm body is a distinct use position.
        Resolved::Match { scrutinee, arms } => {
            let mut n = uses_in(db, scrutinee, init);
            for (_, body) in &arms {
                n += uses_in(db, *body, init);
            }
            n
        }
        // A `SumPayload`/`BinField`/`MapField` reads the scrutinee at run time (a payload / a decoded
        // segment / a keyed lookup); if the scrutinee is `init`, that is a use of the binding.
        Resolved::SumPayload { scrutinee, .. }
        | Resolved::BinField { scrutinee, .. }
        | Resolved::MapField { scrutinee, .. } => usize::from(scrutinee == init) as u32,
        // Effect control forms: the binding may be referenced in a handler's init, any arm body, a
        // resumption's value/next-state, or the handled/delegated body — count each position.
        Resolved::Handle {
            init: seed,
            arms,
            body,
        } => {
            let mut n = uses_in(db, seed, init);
            for arm in arms.iter() {
                n += uses_in(db, arm.body, init);
            }
            n + uses_in(db, body, init)
        }
        Resolved::Resume { value, next_state } => {
            uses_in(db, value, init) + uses_in(db, next_state, init)
        }
        Resolved::Host { body, .. } => uses_in(db, body, init),
        // Leaves and non-referencing forms contribute nothing.
        Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::SymbolConst(_)
        | Resolved::Bytes(_)
        | Resolved::Char(_)
        | Resolved::Float(_)
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. }
        | Resolved::Poison(_) => 0,
    }
}

/// Lower an ARITHMETIC application: FOLD it when its operands fold to constants — evaluate at compile
/// time with a CHECKED operation, so a provable overflow is a build error (CDZ0304 poison) rather than
/// a shipped runtime trap (`reference-compiler.md` §A Compile-Provable Trap Fails The Build). An
/// operand that is not a constant stays a runtime `Arith` (its wasm op selected from the solved width
/// at selection); a poison operand propagates.
/// Whether the arithmetic application at `id` is over QUANTITIES whose inner numeric type is a FLOAT —
/// so `+`/`-`/`*`/`/` must run the float operation on the erased inner values (a quantity's operator is
/// polymorphic over its inner numeric). Reads the node's solved type: `+`/`-`/comparison keep the
/// operands' unit so the RESULT is `(Qty Float …)`; `*`/`/` compose units so the result is still a
/// quantity — either way a `Ty::Qty { inner: Float }` result marks the float case. Falls back to the
/// first operand's type when the result is not itself a quantity (a `Qty.value`-peeled position).
fn quantity_inner_is_float(db: &mut Db, id: StructId, args: &[StructId]) -> bool {
    let is_qty_float = |t: &crate::ty::Ty| matches!(t, crate::ty::Ty::Qty { inner, .. } if matches!(**inner, crate::ty::Ty::Float(_)));
    if is_qty_float(&crate::infer::type_of(db, id)) {
        return true;
    }
    // The result may not be a quantity (a comparison yields Bool); check the first operand.
    args.first()
        .map(|&a| is_qty_float(&crate::infer::type_of(db, a)))
        .unwrap_or(false)
}

/// Whether the two operands are quantities of the SAME dimension at DIFFERENT scales — a mixed-unit
/// combine that must convert to the reference (`1 km + 500 m`). `false` when either is not a quantity,
/// they differ in dimension (that is CDZ0501, reported in `infer`), or the scales are equal (the common
/// same-unit case — no conversion, the ordinary arith path). Reads the operands' solved units.
fn quantity_scales_differ(db: &mut Db, args: &[StructId]) -> bool {
    let (a, b) = (
        crate::infer::type_of(db, args[0]),
        crate::infer::type_of(db, args[1]),
    );
    match (&a, &b) {
        (crate::ty::Ty::Qty { unit: ua, .. }, crate::ty::Ty::Qty { unit: ub, .. }) => {
            ua.same_dimension(ub) && ua.scale() != ub.scale()
        }
        _ => false,
    }
}

/// Lower a MIXED-UNIT combine `(op a b)` where `a` and `b` are quantities of one dimension at different
/// scales: convert EACH operand to the dimension's REFERENCE unit by its exact scale (`value * num /
/// den` in the inner type T), then apply `op` at the reference. Folds the CONSTANT case — the operands
/// erase to a `Core::ConstInt`/`ConstFloat`, each scaled exactly (Int) or by round-to-nearest (Float),
/// per spec §48 ("los[es] precision only where the underlying numeric type is itself inexact"). A
/// non-constant operand DECLINES (the runtime scale-multiply is a later increment). `+`/`-` fold to the
/// converted numeric (rendered back as `(Qty <sum> <reference-unit>)` by the value form); a comparison
/// folds to a `ConstBool`.
fn lower_quantity_combine(
    db: &mut Db,
    id: StructId,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    // Each operand's scale to the reference (num/den) — read off its solved unit.
    let scale_of = |db: &mut Db, arg: StructId| -> Option<(i128, i128)> {
        match crate::infer::type_of(db, arg) {
            crate::ty::Ty::Qty { unit, .. } => Some(unit.scale()),
            _ => None,
        }
    };
    let (ln, ld) = match scale_of(db, lhs) {
        Some(s) => s,
        None => return Core::Poison(Reject::decline("mixed-unit combine: non-quantity operand")),
    };
    let (rn, rd) = match scale_of(db, rhs) {
        Some(s) => s,
        None => return Core::Poison(Reject::decline("mixed-unit combine: non-quantity operand")),
    };
    // The inner numeric type decides how conversion + the op run.
    let inner_is_float = matches!(
        crate::infer::type_of(db, lhs),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Float(_))
    );
    let lc = core_of(db, lhs);
    let rc = core_of(db, rhs);
    if inner_is_float {
        // FLOAT: convert each to the reference by `v * num / den` (rounding), then run the op.
        if let (Some(lv), Some(rv)) = (float_of_core(&lc), float_of_core(&rc)) {
            // CONSTANT operands — fold exactly at compile time.
            let l = lv * (ln as f64) / (ld as f64);
            let r = rv * (rn as f64) / (rd as f64);
            return fold_float_combine(op, l, r);
        }
        // RUNTIME operand(s) — synthesize the scale conversion as real float arithmetic and lower it.
        return lower_runtime_combine(db, op, lhs, (ln, ld), rhs, (rn, rd), true);
    }
    // INT (and other exact inner): convert each by `v * num / den` over i128 (exact; truncates on a
    // non-whole ratio, per opting into integer math).
    if let (Some(lv), Some(rv)) = (int_of_core(&lc), int_of_core(&rc)) {
        // CONSTANT operands — fold.
        let conv = |v: i128, n: i128, d: i128| -> Option<i128> { v.checked_mul(n).map(|x| x / d) };
        let (l, r) = match (conv(lv, ln, ld), conv(rv, rn, rd)) {
            (Some(l), Some(r)) => (l, r),
            _ => {
                return Core::Poison(Reject::coded(
                    Code::ConstTrap,
                    "mixed-unit conversion overflows the machine range",
                ));
            }
        };
        let _ = id;
        return fold_int_combine(op, l, r);
    }
    // RUNTIME operand(s) — synthesize the scale conversion as real integer arithmetic and lower it.
    lower_runtime_combine(db, op, lhs, (ln, ld), rhs, (rn, rd), false)
}

/// The runtime path of a mixed-unit combine: synthesize `(op (convert lhs) (convert rhs))` as ordinary
/// arithmetic over the operands' ERASED magnitudes and lower it — the scale multiply the source denotes
/// by naming two units, emitted as real code (units-of-measure.md §A Unit Conversion Is The Arithmetic
/// The Source Denotes: "the scale multiply reaches the emitted component only when a magnitude is a
/// runtime value"). Each operand converts to the reference by `value * num / den` (float: `*.`/`/.`;
/// int: `*`/`/`), built from the quantity's value occurrence + synthesized constant factors. A
/// non-`Qty.of` operand (no reusable value occurrence) declines.
#[allow(clippy::too_many_arguments)]
fn lower_runtime_combine(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    (ln, ld): (i128, i128),
    rhs: StructId,
    (rn, rd): (i128, i128),
    is_float: bool,
) -> Core {
    let lconv = match convert_operand_ast(db, lhs, ln, ld, is_float) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::decline(
                "runtime mixed-unit combine over a non-Qty.of operand (not yet emitted)",
            ));
        }
    };
    let rconv = match convert_operand_ast(db, rhs, rn, rd, is_float) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::decline(
                "runtime mixed-unit combine over a non-Qty.of operand (not yet emitted)",
            ));
        }
    };
    // Build `(op-name lconv rconv)` with the ORDINARY numeric operator (float ops for a float inner) so
    // it lowers through the ordinary arith/comparison path — the converted operands are bare numerics.
    let op_name = combine_op_name(op, is_float);
    let head = db.push_name(op_name);
    let app = db.push_list(vec![head, lconv, rconv]);
    core_of(db, app)
}

/// Synthesize an arena node for a quantity operand's magnitude CONVERTED to the reference: `value * num
/// / den`, using the ordinary numeric operators (float `*.`/`/.` for a float inner, int `*`/`/`
/// otherwise). `value` is the quantity's `Qty.of` value occurrence (reused, not re-synthesized). When
/// the scale is 1/1 the value passes through unconverted. `None` if the operand has no reusable value
/// occurrence (not a literal `Qty.of`).
fn convert_operand_ast(
    db: &mut Db,
    operand: StructId,
    num: i128,
    den: i128,
    is_float: bool,
) -> Option<StructId> {
    let value = crate::eval::qty_value_occ(db, operand)?;
    // Scale 1/1 — no conversion, use the value as-is.
    if num == 1 && den == 1 {
        return Some(value);
    }
    let (mul, div) = if is_float { ("*.", "/.") } else { ("*", "/") };
    // `(* value num)` — multiply by the scale numerator (a `num.0` float literal for a float inner).
    let mut node = value;
    if num != 1 {
        let n_lit = num_literal(db, num, is_float);
        let mul_head = db.push_name(mul);
        node = db.push_list(vec![mul_head, node, n_lit]);
    }
    // `(/ … den)` — divide by the denominator.
    if den != 1 {
        let d_lit = num_literal(db, den, is_float);
        let div_head = db.push_name(div);
        node = db.push_list(vec![div_head, node, d_lit]);
    }
    Some(node)
}

/// A synthesized numeric literal node for a machine integer `v` — a float decimal `v.0` when `is_float`,
/// else an integer literal. Used for the constant scale factors a runtime conversion multiplies by.
fn num_literal(db: &mut Db, v: i128, is_float: bool) -> StructId {
    if is_float {
        // Build the exact decimal for `v` (a whole number, always finite).
        match crate::ast::Decimal::from_f64(v as f64) {
            Some(d) => db.push_atom(crate::ast::Leaf::Float(d)),
            // Unreachable for a whole scale factor; fall back to an integer literal.
            None => db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i128(v),
                radix: crate::ast::Radix::Dec,
            }),
        }
    } else {
        db.push_atom(crate::ast::Leaf::Int {
            value: IntValue::from_i128(v),
            radix: crate::ast::Radix::Dec,
        })
    }
}

/// The ordinary numeric operator NAME for a mixed-unit combine `op` at the inner type — the float
/// operators (`+.`/`-.`/`<`/…) for a float inner, the integer ones otherwise. Comparisons share one
/// spelling across inners (they are polymorphic over the operand type).
fn combine_op_name(op: Prim, is_float: bool) -> &'static str {
    match op {
        Prim::Add if is_float => "+.",
        Prim::Add => "+",
        Prim::Sub if is_float => "-.",
        Prim::Sub => "-",
        Prim::Lt => "<",
        Prim::Gt => ">",
        Prim::Le => "<=",
        Prim::Ge => ">=",
        Prim::Eq => "=",
        // Only additive/comparison ops reach a mixed-unit combine.
        _ => "+",
    }
}

/// The `f64` a constant float/int core holds (a quantity's erased inner), for the float conversion fold.
fn float_of_core(c: &Core) -> Option<f64> {
    match c {
        Core::ConstFloat(d) => Some(f64::from_bits(d.to_f64_bits())),
        _ => None,
    }
}

/// The `i128` a constant int core holds (a quantity's erased inner), for the integer conversion fold.
fn int_of_core(c: &Core) -> Option<i128> {
    match c {
        Core::ConstInt(v) => v.to_i128(),
        _ => None,
    }
}

/// Lower `(Unit.in target q)` — convert q's erased magnitude from its unit to `target` by
/// `value * (q.scale / target.scale)` in the inner type T (Float rounds, Int exact/truncates). A no-op
/// when the scales are equal. Folds the constant case; a runtime magnitude declines (the emitted runtime
/// scale-multiply is a later increment). The dimensional check (target vs q dimension) is
/// `check_application`'s (CDZ0501); here q is assumed same-dimension.
///
/// The conversion is exactly the SCALE ARITHMETIC the source denotes by naming the two units — the ratio
/// of the operand's scale to the target's, nothing the dimensional layer adds. A constant magnitude is
/// converted at compile time (folded to a `ConstFloat`/`ConstInt`, no runtime arithmetic), and the result
/// is a BARE numeric core — the `Ty::Qty` dimension is erased whether or not the scale multiply survives.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# A conversion between two units of one dimension MUST be the scale arithmetic the source denotes by naming those units, not additional arithmetic the dimensional layer introduces, so that the emitted arithmetic is what the program means rather than an overhead the check imposes.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# A unit conversion whose operands are compile-time constants MUST be computed at compile time, so that a conversion between constant quantities contributes no runtime arithmetic.
//= spec/capabilities/units-of-measure.md#a-unit-conversion-is-the-arithmetic-the-source-denotes
//# The dimension a quantity carries MUST be erased whether or not a scale conversion is emitted, so that the type-level dimensional information never survives into the component even when the scale arithmetic does.
fn lower_unit_in(db: &mut Db, target: StructId, q: StructId) -> Core {
    // q's scale to the reference (read off its solved unit); the target's from `unit_of`.
    let (qn, qd) = match crate::infer::type_of(db, q) {
        crate::ty::Ty::Qty { unit, .. } => unit.scale(),
        _ => return Core::Poison(Reject::decline("Unit.in of a non-quantity")),
    };
    let (tn, td) = match crate::eval::unit_of(db, target) {
        Some(u) => u.scale(),
        None => return Core::Poison(Reject::decline("Unit.in target is not a unit")),
    };
    // The conversion factor is `q.scale / target.scale` = `(qn/qd) / (tn/td)` = `(qn*td) / (qd*tn)`.
    let inner_is_float = matches!(
        crate::infer::type_of(db, q),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Float(_))
    );
    // The conversion factor is `q.scale / target.scale` = `(qn*td) / (qd*tn)` — one combined ratio.
    let num = match qn.checked_mul(td) {
        Some(n) => n,
        None => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            ));
        }
    };
    let den = match qd.checked_mul(tn) {
        Some(d) if d != 0 => d,
        _ => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            ));
        }
    };
    let qc = core_of(db, q);
    if inner_is_float {
        if let Some(v) = float_of_core(&qc) {
            // CONSTANT float magnitude — fold the conversion.
            let converted = v * (num as f64) / (den as f64);
            return match crate::ast::Decimal::from_f64(converted) {
                Some(d) => Core::ConstFloat(d),
                None => Core::Poison(Reject::decline("Unit.in float result has no finite form")),
            };
        }
    } else if let Some(v) = int_of_core(&qc) {
        // CONSTANT int magnitude — fold `v * num / den` (exact/truncating).
        return match v.checked_mul(num) {
            Some(scaled) => Core::ConstInt(IntValue::from_i128(scaled / den)),
            None => Core::Poison(Reject::coded(
                Code::ConstTrap,
                "Unit.in conversion overflows",
            )),
        };
    }
    // RUNTIME magnitude — synthesize `value * num / den` as real arithmetic over q's value occurrence
    // and lower it (the same scale-multiply the constant path folds, emitted as code).
    match convert_operand_ast(db, q, num, den, inner_is_float) {
        Some(node) => core_of(db, node),
        None => Core::Poison(Reject::decline(
            "Unit.in over a runtime non-Qty.of magnitude (not yet emitted)",
        )),
    }
}

/// Lower `(Qty.pow q n)` — raise q's erased magnitude to the `n`th power over the inner numeric type.
/// The unit is a compile-time concern (the solved `Ty::Qty` already carries `unit^n`); at runtime this
/// is just `value * value * … ` (`|n|` factors), synthesized as ordinary arithmetic over q's value
/// occurrence and re-lowered (so the constant case FOLDS through the normal arith path and a runtime
/// magnitude emits the multiplies). `n = 0` is the dimensionless `1` (the multiplicative identity in the
/// inner type). A NEGATIVE `n` is the reciprocal `1 / value^|n|` (an inverse unit like a frequency
/// `second⁻¹`) — the division runs in the inner type, so Float divides and Int TRUNCATES (`1 / 8 = 0`),
/// the documented "precision loss only where the numeric type is itself inexact / integer truncates".
fn lower_qty_pow(db: &mut Db, q: StructId, exp: StructId) -> Core {
    let inner_is_float = matches!(
        crate::infer::type_of(db, q),
        crate::ty::Ty::Qty { inner, .. } if matches!(*inner, crate::ty::Ty::Float(_))
    );
    let n = match crate::resolve::resolved_of(db, exp) {
        crate::resolved::Resolved::Int(v) => match v.to_i64() {
            Some(n) => n,
            None => return Core::Poison(Reject::decline("Qty.pow exponent out of range")),
        },
        _ => {
            return Core::Poison(Reject::decline(
                "Qty.pow exponent is not a compile-time integer",
            ));
        }
    };
    // `n = 0` — the dimensionless identity `1` (`1.0` for a float inner).
    if n == 0 {
        let one = num_literal(db, 1, inner_is_float);
        return core_of(db, one);
    }
    let value = match crate::eval::qty_value_occ(db, q) {
        Some(v) => v,
        None => {
            return Core::Poison(Reject::decline(
                "Qty.pow over a non-Qty.of magnitude (not yet emitted)",
            ));
        }
    };
    // Build `value^|n|` = `(* (* … value value) value)` — `|n|` copies, left-nested — with the inner
    // type's multiply (`*.` float / `*` int).
    let mul = if inner_is_float { "*." } else { "*" };
    let mut node = value;
    for _ in 1..n.unsigned_abs() {
        let mul_head = db.push_name(mul);
        node = db.push_list(vec![mul_head, node, value]);
    }
    // A negative exponent is the reciprocal `1 / value^|n|`, dividing in the inner type (`/.` float / `/`
    // int); a positive one is the power itself. Lower the synthesized node through the ordinary arith
    // path (so the constant case folds and a runtime magnitude emits the multiplies/division).
    if n < 0 {
        let (one, div) = (
            num_literal(db, 1, inner_is_float),
            if inner_is_float { "/." } else { "/" },
        );
        let div_head = db.push_name(div);
        node = db.push_list(vec![div_head, one, node]);
    }
    core_of(db, node)
}

/// Apply `op` to two converted FLOAT reference values, producing the result core: `+`/`-` a
/// `ConstFloat`, a comparison a `ConstBool`.
fn fold_float_combine(op: Prim, l: f64, r: f64) -> Core {
    match op {
        Prim::Add | Prim::Sub => {
            let v = if matches!(op, Prim::Add) {
                l + r
            } else {
                l - r
            };
            match crate::ast::Decimal::from_f64(v) {
                Some(d) => Core::ConstFloat(d),
                None => Core::Poison(Reject::decline(
                    "mixed-unit float result has no finite form",
                )),
            }
        }
        Prim::Lt => Core::ConstBool(l < r),
        Prim::Gt => Core::ConstBool(l > r),
        Prim::Le => Core::ConstBool(l <= r),
        Prim::Ge => Core::ConstBool(l >= r),
        Prim::Eq => Core::ConstBool(l == r),
        _ => Core::Poison(Reject::decline("unexpected op in mixed-unit float combine")),
    }
}

/// Apply `op` to two converted INT reference values, producing the result core: `+`/`-` a `ConstInt`, a
/// comparison a `ConstBool`.
fn fold_int_combine(op: Prim, l: i128, r: i128) -> Core {
    let arith = |v: Option<i128>| match v {
        Some(n) => Core::ConstInt(IntValue::from_i128(n)),
        None => Core::Poison(Reject::coded(
            Code::ConstTrap,
            "mixed-unit result overflows",
        )),
    };
    match op {
        Prim::Add => arith(l.checked_add(r)),
        Prim::Sub => arith(l.checked_sub(r)),
        Prim::Lt => Core::ConstBool(l < r),
        Prim::Gt => Core::ConstBool(l > r),
        Prim::Le => Core::ConstBool(l <= r),
        Prim::Ge => Core::ConstBool(l >= r),
        Prim::Eq => Core::ConstBool(l == r),
        _ => Core::Poison(Reject::decline("unexpected op in mixed-unit int combine")),
    }
}

/// The wrong-arity CDZ0201 reject shared by the fixed-arity BINARY operators — integer arithmetic
/// (`lower_arith`), FLOAT arithmetic (`lower_float_arith`), and COMPARISON (`lower_comparison`). All three
/// take exactly 2 operands; an OVER-application (`(+ 1 2 3)`, `(< 1 2 3)`, `(+. 1.0 2.0 3.0)`) has a
/// mechanical repair: DELETE the first surplus operand (`args[2]`) — the fixpoint removes each extra until
/// exactly 2 remain. A TOO-FEW application (`(+ 1)`) has nothing to delete → no fix. Carrying the delete
/// fix on THIS authoritative CDZ0201 is what lets `dedup_faults` drop the sibling CDZ0203 over-application
/// (which anchors at the same surplus node), so a binary operator over-application reports ONCE, with the
/// fix — the parity `lower_arith` had but `lower_comparison`/`lower_float_arith` lacked (they double-reported).
fn binop_arity_reject(op: Prim, args: &[StructId]) -> Reject {
    let mut reject = Reject::coded(
        Code::Malformed,
        format!("{} takes exactly 2 operands", intrinsic_name(op)),
    );
    if args.len() > 2 {
        reject = reject.with_fix(crate::diag::Fix::delete_heuristic(
            args[2],
            "remove the extra operand",
        ));
    }
    reject
}

/// True iff either operand of a binary op has solved type `Ty::BigInt` — the signal to route `+`/`-`/
/// `*`/`/` to the runtime BigInt arithmetic instead of the fixed-width int fold. (A `BigInt`/fixed mix
/// never reaches lowering — `check_application` rejected it CDZ0301 — so if ONE operand is a BigInt the
/// other is too.)
fn bigint_operand(db: &mut Db, args: &[StructId]) -> bool {
    args.iter()
        .any(|&a| matches!(crate::infer::type_of(db, a), crate::ty::Ty::BigInt))
}

/// Lower a BigInt `+`/`-`/`*`/`/` to a runtime `Core::BigIntBinOp` (the runtime `bigint-*` op). Unlike
/// fixed-width arithmetic, this does NOT constant-fold: exact BigInt arithmetic needs compiler-side
/// bignum (rcdzc deliberately has no bignum crate — `IntValue` carries the value but not arithmetic), so
/// the unbounded arithmetic runs at RUN TIME via the runtime `Big` limb library (B3a). A poison operand
/// propagates. `div` traps on a zero divisor at run time (numeric-model — an unbounded range gives `n/0`
/// no value); the never-trapping add/sub/mul grow the magnitude as needed.
fn lower_bigint_arith(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let big_op = match op {
        Prim::Add => crate::core::BigIntOp::Add,
        Prim::Sub => crate::core::BigIntOp::Sub,
        Prim::Mul => crate::core::BigIntOp::Mul,
        Prim::Div => crate::core::BigIntOp::Div,
        _ => return Core::Poison(Reject::decline("not a BigInt arithmetic op")),
    };
    Core::BigIntBinOp {
        op: big_op,
        lhs,
        rhs,
    }
}

/// Lower a BigInt comparison `<`/`>`/`<=`/`>=`/`=` to either a constant `Bool` fold or a runtime
/// `Core::BigIntCmp` (the runtime `bigint-cmp` op + a fixed compare-with-zero). A CONSTANT pair (both
/// operands `Core::ConstInt` — the shape a folded `(BigInt.of <constant>)` leaves) folds when both values
/// fit `i128` (`to_i128` reads the exact value; every constant a program is likely to compare fits, and
/// the runtime op covers the rest), comparing at 128-bit precision. A poison operand propagates. Otherwise
/// (a runtime operand) emit `Core::BigIntCmp`; the emit borrows both operands and applies the operator's
/// signed compare against the three-way `-1`/`0`/`1` result.
fn lower_bigint_cmp(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Core {
    let lc = core_of(db, lhs);
    let rc = core_of(db, rhs);
    match (lc, rc) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // A constant BigInt pair — both carry the exact `IntValue`. Fold at 128-bit precision when both
        // fit; a value beyond i128 (astronomically large) falls through to the runtime op.
        (Core::ConstInt(a), Core::ConstInt(b)) => match (a.to_i128(), b.to_i128()) {
            (Some(x), Some(y)) => {
                let r = compare_ord(op, x.cmp(&y));
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant BigInt comparison (i128)");
                Core::ConstBool(r)
            }
            _ => Core::BigIntCmp { op, lhs, rhs },
        },
        _ => Core::BigIntCmp { op, lhs, rhs },
    }
}

fn lower_arith(db: &mut Db, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(binop_arity_reject(op, args));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::ConstInt(a), Core::ConstInt(b)) => fold_arith(op, a, b),
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // ALGEBRAIC IDENTITY: one operand is a constant whose value makes the op a NO-OP or a constant
        // result — the whole checked operation (and its overflow guard) is eliminated at lowering. Only
        // the identities that are SAFE at every width and never trap are applied (see `arith_identity`);
        // the RESULT keeps the op's solved type because the runtime operand shares it (binary-op
        // unification), and a `0`/`1` constant grounds to that width at selection.
        (lc, rc) => {
            if let Some(simplified) = arith_identity(db, op, args[0], &lc, args[1], &rc) {
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic identity simplified (op elided)");
                return simplified;
            }
            trace!(target: "rcdzc::lower", op = intrinsic_name(op), "arithmetic stays runtime (operand not constant)");
            Core::Arith {
                op,
                lhs: args[0],
                rhs: args[1],
            }
        }
    }
}

/// Lower a FLOAT arithmetic application (`+.`/`-.`/`*.`/`/.`). FOLDS two constant floats at the solved
/// float WIDTH (the `Decimal` operands round to the width's IEEE format, the op runs, the result rounds
/// back — round-to-nearest-even, the fixed deterministic mode); a non-constant operand DECLINES (runtime
/// float ops emit the machine `f64.add`/… in a later increment). Unlike integer arithmetic there is NO
/// checked-trap: an IEEE overflow yields an infinity — but a NON-FINITE result has no written value form
/// (the float-literal-overflow rule), so a fold to `±inf`/NaN DECLINES rather than producing a bad value.
fn lower_float_arith(db: &mut Db, id: StructId, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(binop_arity_reject(op, args));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::ConstFloat(a), Core::ConstFloat(b)) => {
            // The result WIDTH is the application's solved type (both operands unify to it). Fold at that
            // width: round each operand to the width's format, compute, round the result back.
            let width = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            let fold_at = |x: f64, y: f64| -> f64 {
                let r = match op {
                    Prim::FAdd => x + y,
                    Prim::FSub => x - y,
                    Prim::FMul => x * y,
                    Prim::FDiv => x / y,
                    _ => f64::NAN,
                };
                // A `Float32` result rounds through binary32 (`as f32 as f64`), the fixed narrower mode;
                // `Float64` computes directly. Both round-to-nearest-even (the IEEE default wasm uses).
                if width == 32 { r as f32 as f64 } else { r }
            };
            let (x, y) = if width == 32 {
                (
                    f64::from_bits(a.to_f64_bits()) as f32 as f64,
                    f64::from_bits(b.to_f64_bits()) as f32 as f64,
                )
            } else {
                (
                    f64::from_bits(a.to_f64_bits()),
                    f64::from_bits(b.to_f64_bits()),
                )
            };
            let result = fold_at(x, y);
            match crate::ast::Decimal::from_f64(result) {
                Some(d) => {
                    trace!(target: "rcdzc::lower", op = intrinsic_name(op), width, "folded constant float op");
                    Core::ConstFloat(d)
                }
                // A non-finite result (overflow → ±inf, 0.0/.0 → NaN) has no written value form — decline
                // rather than emit an unrepresentable constant (the float-literal-overflow discipline).
                None => Core::Poison(Reject::decline(
                    "a floating-point operation whose result is not finite has no value form yet",
                )),
            }
        }
        // A runtime float operand — emit the machine `f64.add`/`f32.add`/… at selection (the op's width
        // read off the solved type there, like the integer `Core::Arith`). Float ops never trap, so no
        // overflow guard — just the two operands + the machine op. A poison operand already returned above.
        _ => Core::Arith {
            op,
            lhs: args[0],
            rhs: args[1],
        },
    }
}

/// Lower a `Float64.of-int` / `Float32.of-int` — the TOTAL int→float conversion `Int64 → (Float N)`.
/// FOLD a constant integer to a `Core::ConstFloat` (the value as f64/f32, rounding to the nearest
/// representable float at the target width — total, never trapping); a runtime integer emits
/// `Core::Convert{op: FloatOfInt}` (select → `f{64,32}.convert_i64_s`). The target width is the node's
/// solved `Ty::Float`. No implicit promotion — the conversion is always written (numeric-model.md §A
/// Conversion Involving A Floating-Point Type Is Explicit).
fn lower_float_of_int(db: &mut Db, id: StructId, args: &[StructId]) -> Core {
    if args.len() != 1 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "of-int takes exactly 1 operand".to_string(),
        ));
    }
    let width = match crate::infer::type_of(db, id) {
        crate::ty::Ty::Float(ft) => ft.ground_width(),
        _ => {
            return Core::Poison(Reject::decline(
                "a float conversion target is not a definite float type",
            ));
        }
    };
    match core_of(db, args[0]) {
        Core::Poison(r) => Core::Poison(r),
        Core::ConstInt(v) => {
            // Fold: the integer's value as a float at the target width (round-to-nearest-even). A value
            // beyond the finite float range would round to ±inf — but an `Int64` is always finite in f64
            // (|Int64| < 2^63 ≪ f64 max), and f32 of an Int64 is finite too, so this never overflows.
            let Some(i) = v.to_i64() else {
                // A BigInt-magnitude constant (>i64) has no Int64 conversion source here — decline.
                return Core::Poison(Reject::decline(
                    "of-int of a value wider than Int64 is not yet supported",
                ));
            };
            let f = if width == 32 {
                i as f32 as f64
            } else {
                i as f64
            };
            match crate::ast::Decimal::from_f64(f) {
                Some(d) => {
                    trace!(target: "rcdzc::lower", width, "folded constant of-int to a float");
                    Core::ConstFloat(d)
                }
                None => Core::Poison(Reject::decline(
                    "a float conversion whose result is not finite has no value form",
                )),
            }
        }
        // A runtime integer operand — emit the machine int→float convert at selection (target width read
        // off the solved type there). Total, so no guard.
        _ => Core::Convert {
            op: Prim::FloatOfInt,
            operand: args[0],
        },
    }
}

/// Lower a `Float64.of` / `Float32.of` — the TOTAL float-WIDTH conversion `Float M → (Float N)` (promote
/// / demote / identity). FOLD a constant float by rounding the exact `Decimal` at the TARGET width
/// (this node's solved `Ty::Float`): a same-width or widening conversion is exact, a narrowing rounds to
/// nearest under the fixed mode. A runtime float emits `Core::Convert{op:FloatOf}` (select →
/// demote/promote/nothing). Total — a float always has an image at another float width.
fn lower_float_of(db: &mut Db, id: StructId, args: &[StructId]) -> Core {
    if args.len() != 1 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "of takes exactly 1 operand".to_string(),
        ));
    }
    let width = match crate::infer::type_of(db, id) {
        crate::ty::Ty::Float(ft) => ft.ground_width(),
        _ => {
            return Core::Poison(Reject::decline(
                "a float conversion target is not a definite float type",
            ));
        }
    };
    match core_of(db, args[0]) {
        Core::Poison(r) => Core::Poison(r),
        Core::ConstFloat(d) => {
            // Round the exact source value to the target width: `as f32 as f64` for a Float32 target
            // (narrowing rounds to nearest binary32), the f64 value unchanged for a Float64 target
            // (a promote/identity is exact). Rounding once at the target width matches the runtime op.
            let src = f64::from_bits(d.to_f64_bits());
            let rounded = if width == 32 { src as f32 as f64 } else { src };
            match crate::ast::Decimal::from_f64(rounded) {
                Some(nd) => {
                    trace!(target: "rcdzc::lower", width, "folded constant float-width conversion");
                    Core::ConstFloat(nd)
                }
                None => Core::Poison(Reject::decline(
                    "a float conversion whose result is not finite has no value form",
                )),
            }
        }
        // A runtime float operand — emit the machine demote/promote at selection (source + target widths
        // read off the solved types there). Total, no guard.
        _ => Core::Convert {
            op: Prim::FloatOf,
            operand: args[0],
        },
    }
}

/// Apply a SAFE algebraic identity to a runtime arithmetic op with ONE constant operand, returning the
/// simplified core (the runtime operand's own core, or a constant) — or `None` when no identity applies
/// and the op stays a runtime `Arith`. `lc`/`rc` are the already-lowered operand cores; `lhs`/`rhs`
/// their AST occurrences. Every identity here is exact at EVERY width and never CHANGES the value; the
/// PASSTHROUGH identities keep the runtime operand (so its own traps still fire), while the ANNIHILATOR
/// identities (`x*0`, `x&0` → `0`) DISCARD the operand and so are applied ONLY when the discarded
/// operand cannot trap (`is_trap_free`) — else eliding it would drop a defined trap (`(* (/ a b) 0)`
/// must still trap on `b==0`; `numeric-model.md`/§div traps are defined outcomes, not to be optimized
/// away). Applied identities:
///  - `x + 0` = `0 + x` = `x - 0` = `x` (adding/subtracting 0 never overflows; keeps x);
///  - `x * 1` = `1 * x` = `x` (keeps x); `x * 0` = `0 * x` = `0` (ONLY if x is trap-free — discards x);
///  - `x | 0` = `0 | x` = `x ^ 0` = `0 ^ x` = `x` (keeps x); `x & 0` = `0 & x` = `0` (trap-free x only);
///  - `x << 0` = `x >> 0` = `x` (a zero shift COUNT is a no-op — count is the RIGHT operand; keeps x).
///
/// Deliberately NOT applied HERE: `0 - x` (negation traps at MIN), `x & allbits` (all-ones is width-
/// dependent), `0 << x` / `0 >> x` (a non-constant count must still trap if out of range). NOTE: the
/// STRENGTH REDUCTION `x * 2^k → x << k` is not a value-identity (it rewrites the op, not elides it), so
/// it lives at the SELECTION tier (`emit`'s `Core::Arith` Mul arm → `emit_mul_pow2_as_shift`), where the
/// shift's cheaper round-trip overflow check replaces the mul's division-based one — sound because a
/// left shift is EXACT multiplication by a power of two with the SAME defined overflow-trap
/// (`numeric-model.md` §Overflow Is Defined for shifts).
fn arith_identity(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    lc: &Core,
    rhs: StructId,
    rc: &Core,
) -> Option<Core> {
    // A constant operand's value tested against a small literal (0 or 1), by value (magnitude-agnostic).
    let is =
        |c: &Core, k: i64| matches!(c, Core::ConstInt(v) if v.eq_value(&IntValue::from_i64(k)));
    let zero = || Core::ConstInt(IntValue::from_i64(0));
    match op {
        // `x + 0` / `0 + x` → x.
        Prim::Add if is(rc, 0) => Some(lc.clone()),
        Prim::Add if is(lc, 0) => Some(rc.clone()),
        // `x - 0` → x. (`0 - x` is negation — NOT an identity, would need a trap-checked negate.)
        Prim::Sub if is(rc, 0) => Some(lc.clone()),
        // `x * 1` / `1 * x` → x (keeps x).
        Prim::Mul if is(rc, 1) => Some(lc.clone()),
        Prim::Mul if is(lc, 1) => Some(rc.clone()),
        // `x * 0` / `0 * x` → 0 — DISCARDS x, so only when x cannot trap.
        Prim::Mul if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::Mul if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x * -1` / `-1 * x` → NEGATION `(- 0 x)` — a strength reduction. A full-width `* -1` keeps the
        // expensive `div_s` round-trip overflow guard (the constant-multiplier fast path EXCLUDES `-1`,
        // since its `MIN/-1` bound is not i64-representable), but negation `0 - x` has the SAME single
        // overflow — `x == MIN` (`-MIN` is unrepresentable) — emitted as one `eq` check (the negation fast
        // path in `emit_machine_overflow_guard`). Value- AND trap-identical: `x * -1 == -x`, both overflow
        // iff `x == MIN`. Rewrite to `(- 0 x)`, synthesizing the `0` (the `Leaf::Int` node-synth pattern);
        // `x` STAYS an operand (the subtrahend), so its OWN traps are preserved — no `is_trap_free` guard
        // needed (unlike `* 0`, which discards `x`). (A NARROW `* -1` already sheds the div via the
        // narrow-product-fits-slot path, so this mainly helps full width — but the rewrite is correct at
        // every width: `0 - x` narrows/range-checks exactly as the narrow `* -1` result does.)
        Prim::Mul if is(rc, -1) || is(lc, -1) => {
            let x = if is(rc, -1) { lhs } else { rhs };
            let z = db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i64(0),
                radix: crate::ast::Radix::Dec,
            });
            trace!(target: "rcdzc::fold", "x * -1 → negation (- 0 x)");
            Some(Core::Arith {
                op: Prim::Sub,
                lhs: z,
                rhs: x,
            })
        }
        // WRAPPING arithmetic has the SAME algebraic identities as checked `+`/`*` — the wrap is total,
        // so it never traps and the fold is value-identical (`a +% 0 = a`, `a *% 1 = a`, `a *% 0 = 0`).
        // The keeping folds preserve the surviving operand's traps; the annihilator `*% 0` DISCARDS the
        // other operand, so it too is guarded on trap-freedom (`(/ x 0) *% 0` must still trap).
        Prim::WrappingAdd if is(rc, 0) => Some(lc.clone()),
        Prim::WrappingAdd if is(lc, 0) => Some(rc.clone()),
        Prim::WrappingMul if is(rc, 1) => Some(lc.clone()),
        Prim::WrappingMul if is(lc, 1) => Some(rc.clone()),
        Prim::WrappingMul if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::WrappingMul if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x | 0` / `0 | x` / `x ^ 0` / `0 ^ x` → x.
        Prim::BitOr | Prim::BitXor if is(rc, 0) => Some(lc.clone()),
        Prim::BitOr | Prim::BitXor if is(lc, 0) => Some(rc.clone()),
        // `x & 0` / `0 & x` → 0 — DISCARDS x, so only when x cannot trap.
        Prim::BitAnd if is(rc, 0) && is_trap_free(db, lhs) => Some(zero()),
        Prim::BitAnd if is(lc, 0) && is_trap_free(db, rhs) => Some(zero()),
        // `x & M` / `M & x` → x when the constant `M` has ALL of `x`'s value bits set — a redundant mask.
        // An UNSIGNED width-N `x` lives in `[0, 2^N)`, so if `M`'s low N bits are all 1s the `&` cannot
        // clear anything (`x & M == x`). Restricted to UNSIGNED: a SIGNED value's slot high bits are sign
        // extension, which a mask WOULD clear (changing negatives), so `& fullmask` is not the identity
        // there. `x` keeps its own traps (the operand is returned). `x` is the value operand, `M` the
        // constant — `(& x M)` returns `lc` (x) when `rc` (M) masks x's whole width; symmetric for `(& M x)`.
        Prim::BitAnd if is_full_mask_for(db, lhs, rc) => Some(lc.clone()),
        Prim::BitAnd if is_full_mask_for(db, rhs, lc) => Some(rc.clone()),
        // OR-THEN-MASK ABSORPTION: `(& (| v C1) C2)` → `C2` when `C2 ⊆ C1` (`C2 & C1 == C2`). The inner OR
        // sets every bit of C1 (⊇ C2), the outer mask keeps only C2's bits — all of which are 1 — so the
        // result is exactly the constant C2, independent of `v`. `(& (| x 15) 15)` → 15. DISCARDS `v`, so
        // gated on `is_trap_free`. `C2` is the constant operand of the outer `&`; the inner `(| v C1)` is
        // the other. Both operand orders of the outer `&` are tried.
        Prim::BitAnd
            if let Core::ConstInt(c2) = rc
                && let Some(c2v) = c2.to_i64()
                && let Some(v) = or_then_mask_absorbs(db, lhs, c2v)
                && is_trap_free(db, v) =>
        {
            Some(rc.clone())
        }
        Prim::BitAnd
            if let Core::ConstInt(c2) = lc
                && let Some(c2v) = c2.to_i64()
                && let Some(v) = or_then_mask_absorbs(db, rhs, c2v)
                && is_trap_free(db, v) =>
        {
            Some(lc.clone())
        }
        // `x | M` / `M | x` → M when the constant `M` covers ALL of `x`'s value bits — the OR-SATURATION
        // dual of the `&`-mask elision. `x | M` sets every bit of M plus x's bits; if M already has all the
        // bits x could set (`is_full_mask_for`: x nonneg in `[0, 2^B)`, M's low B bits all 1), the OR adds
        // nothing NEW and the result is exactly M (`(| x 255)` with `x ∈ [0,255]` → 255). DISCARDS x (the
        // result is the constant M, not x), so — like `& 0`/`* 0` — only when x is TRAP-FREE (a trapping x
        // must still trap). Returns the CONSTANT operand's core (M). Same `is_full_mask_for` predicate as
        // the `&` fold, so it too fires on an emit-refined range via the emit-time sibling.
        Prim::BitOr if is_full_mask_for(db, lhs, rc) && is_trap_free(db, lhs) => Some(rc.clone()),
        Prim::BitOr if is_full_mask_for(db, rhs, lc) && is_trap_free(db, rhs) => Some(lc.clone()),
        // XOR CANCELLATION: `(^ (^ v w) w)` → `v` — the two XORs by the SAME `w` (constant OR runtime)
        // cancel (`w ^ w == 0`, and `v ^ 0 == v`). Handled BEFORE the nested-bitwise collapse so the
        // constant case produces `v` DIRECTLY rather than a residual `(^ v 0)` the collapse would leave
        // (which does not re-simplify). DISCARDS `w`, so gated on `is_trap_free(w)`. `v` stays, traps kept.
        Prim::BitXor
            if let Some((v, w)) = xor_cancels(db, lhs, rhs)
                && is_trap_free(db, w) =>
        {
            trace!(target: "rcdzc::fold", node = v.0, "XOR cancellation (^ (^ v w) w) → v");
            Some(core_of(db, v))
        }
        // IDEMPOTENT-BITWISE COLLAPSE: `(OP (OP v w) w)` → `(OP v w)` for `&`/`|` (idempotent: `w OP w == w`,
        // so re-applying `OP w` changes nothing), where the outer operand is `core_equiv` to `w` in the
        // inner op. Covers a RUNTIME `w` (`(| (| x y) y)` → `(| x y)`); the CONSTANT case already collapses
        // via `nested_bitwise_collapse` (`(| x (w|w))` = `(| x w)`). Unlike XOR-cancel, this KEEPS the inner
        // `(OP v w)` node — BOTH operands survive — so NO `is_trap_free` guard is needed (any trap in `v`/`w`
        // is still evaluated). Returns the inner node's core. `nested_shift_combine` — not `nested_bitwise_
        // collapse` — placement before it is fine (the collapse only fires on a CONSTANT operand, distinct
        // from this same-runtime-operand shape). `idempotent_bitwise_collapse` returns the inner node.
        Prim::BitAnd | Prim::BitOr
            if let Some(inner) = idempotent_bitwise_collapse(db, op, lhs, rhs) =>
        {
            trace!(target: "rcdzc::fold", node = inner.0, ?op, "idempotent bitwise (OP (OP v w) w) → (OP v w)");
            Some(core_of(db, inner))
        }
        // ABSORPTION LAW: `x & (x | y)` → `x` and `x | (x & y)` → `x` — a value combined with the DUAL op of
        // itself-with-anything absorbs to itself. The outer op is `&`/`|` and one operand is an inner op of
        // the DUAL kind (`| ` under `&`, `&` under `|`) that CONTAINS `x` (either side); the OTHER outer
        // operand is `x` (`core_equiv`). Result is `x`. DISCARDS the inner op's OTHER operand `y`, so gated
        // on `is_trap_free(y)` (a trapping `y` must still trap). `x` is returned so its own traps stay. Both
        // outer orders and both inner-operand positions are tried by `absorption_operand`.
        Prim::BitAnd | Prim::BitOr
            if let Some((x, y)) = absorption_operand(db, op, lhs, rhs)
                && is_trap_free(db, y) =>
        {
            trace!(target: "rcdzc::fold", node = x.0, ?op, "absorption law (x OP (x DUAL y)) → x");
            Some(core_of(db, x))
        }
        // NESTED-BITWISE COLLAPSE: `(OP (OP v C1) C2)` → `(OP v (C1 ⊙ C2))` for a TOTAL, ASSOCIATIVE
        // bitwise op — `&`/`|`/`^`. Two constant operations on the same value collapse to ONE by folding
        // the constants (`(& (& x 255) 15)`→`(& x 15)`, `(| (| x 5) 3)`→`(| x 7)`, `(^ (^ x 5) 3)`→`(^ x
        // 6)`); the `&` case's folded constant also enables downstream range folds. `v` keeps its own
        // traps (it stays the operand). Guarded on the shape (`nested_bitwise_collapse` returns `None`
        // when it does not apply) so the same-operand `& a a`/`| a a` fold below still fires. Verified
        // value-identical: each op is associative, so `(v OP C1) OP C2 == v OP (C1 ⊙ C2)`.
        Prim::BitAnd | Prim::BitOr | Prim::BitXor
            if let Some(folded) = nested_bitwise_collapse(db, op, lhs, lc, rhs, rc) =>
        {
            Some(folded)
        }
        // `x << 0` / `x >> 0` → x (a zero shift COUNT is a no-op; count is the right operand).
        Prim::Shl | Prim::Shr if is(rc, 0) => Some(lc.clone()),
        // NESTED SHIFT COLLAPSE: `(SH (SH v A) B)` → `(SH v (A+B))` for the SAME shift direction, A, B
        // constants, A+B < width. A RIGHT shift is TOTAL — shifting right by A then B drops the same low
        // A+B bits as one shift by A+B (both `>>ₛ` sign-fill and `>>ᵤ` zero-fill; the inner and outer `>>`
        // on the same-typed value are the same kind, so composing is exact). A LEFT shift is CHECKED (it
        // is exact `·2^count`, trapping on N-bit overflow) but STILL collapses trap-identically: magnitude
        // is MONOTONIC in the count, so `(v<<A)<<B` overflows on exactly the inputs `v<<(A+B)` does (inner
        // overflow ⟹ combined overflow, and combined overflow ⟹ the double's outer step overflows) — same
        // value `v·2^(A+B)` when neither traps, same trap set otherwise. Bounded by A+B < width for BOTH:
        // a combined count ≥ width is masked mod width by the machine op (wrong), and for `<<` it must also
        // TRAP as an out-of-range count — so only the in-range sum is faithful. `v` keeps its own traps (it
        // stays the operand). Guarded via `nested_shift_combine`.
        Prim::Shr | Prim::Shl if let Some(folded) = nested_shift_combine(db, op, lhs, rc) => {
            Some(folded)
        }
        // `(>>ᵤ x k)` → 0 when the LOGICAL right shift drops ALL of `x`'s significant bits — its provable
        // bit-bound `B <= k`. E.g. `(x & 15) >>ᵤ 4`: `x & 15` fits 4 bits, `>>ᵤ 4` shifts them all out → 0.
        // DISCARDS `x`, so gated on `is_trap_free` (a trapping operand's trap must survive). `k` must be a
        // valid IN-RANGE constant count (`< width`) — an out-of-range shift TRAPS rather than yielding 0,
        // so a too-large `k` is left for the runtime count-guard. `unsigned_value_bits` returns the bound
        // only for an unsigned logical-shift chain, so this never misfires on a signed `>>ₛ` (which
        // sign-extends, not zero-fills).
        Prim::Shr
            if is_trap_free(db, lhs)
                && let Core::ConstInt(k) = rc
                && let Some(k) = k.to_i64()
                && k >= 1
                && let Some(bits) = unsigned_value_bits(db, lhs)
                && (k as u32) < shift_width(db, lhs)
                && bits <= k as u32 =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, k, bits, "logical shift drops all significant bits → 0");
            Some(zero())
        }
        // `x / 1` → x (division by one is the identity; keeps x, so its own traps stay).
        Prim::Div if is(rc, 1) => Some(lc.clone()),
        // `x % 1` → 0 (every integer is divisible by 1) — DISCARDS x, so only when x cannot trap.
        Prim::Rem if is(rc, 1) && is_trap_free(db, lhs) => Some(zero()),
        // DIVIDEND-SMALLER-THAN-DIVISOR: when `x` is provably in `[0, C-1]` for a POSITIVE constant divisor
        // `C`, the truncating `x / C` is 0 and `x % C` is `x` — the divisor is too big to divide `x` even
        // once. `(/ (& x 7) 100)` → 0, `(% (& x 7) 100)` → `x & 7` (a masked/refined value modding by a
        // larger constant). Requires `x` NONNEGATIVE with a known upper bound `< C` (`value_range` lo ≥ 0,
        // hi < C) so truncation-toward-zero equals the mathematical result; a negative `x` (`-1 % 100 =
        // -1`, `-1 / 100 = 0`) is excluded for simplicity (the nonneg case is the masked/unsigned idiom).
        // The `/` DISCARDS `x` → gated on `is_trap_free`; the `%` KEEPS `x` (returns `lc`) so its traps
        // survive. `C ≥ 2` (the `/1`,`%1` identities above handle `C=1`; a constant `÷0` is a poison in
        // `lower` before here). Verified: for `0 ≤ x < C`, `x/C == 0` and `x%C == x`.
        Prim::Div
            if let Core::ConstInt(c) = rc
                && let Some(c) = c.to_i64()
                && c >= 2
                && dividend_below_divisor(db, lhs, c)
                && is_trap_free(db, lhs) =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, c, "dividend provably < divisor → x / C = 0");
            Some(zero())
        }
        Prim::Rem
            if let Core::ConstInt(c) = rc
                && let Some(c) = c.to_i64()
                && c >= 2
                && dividend_below_divisor(db, lhs, c) =>
        {
            trace!(target: "rcdzc::fold", node = lhs.0, c, "dividend provably < divisor → x % C = x");
            Some(lc.clone())
        }
        // NESTED-MODULO COLLAPSE: `(% (% v M) N)` → `(% v N)` when the outer divisor `N` DIVIDES the inner
        // `M` (`M % N == 0`), both positive constants. Since `M` is a multiple of `N`, reducing mod `M`
        // first then mod `N` gives the same residue as reducing mod `N` directly — for truncated (toward-
        // zero) division at every sign of `v` (`(x%100)%10 == x%10`, incl. negatives: `-25%100=-25`,
        // `-25%10=-5`, and `-25%10=-5` directly). One `rem` instead of two. `v` STAYS the operand of the
        // outer `% N`, so its own traps (and the outer `% N`'s ÷0 — impossible here, N≥2) are preserved; no
        // `is_trap_free` needed. Both divisors must be constants ≥ 2 and `N | M`.
        Prim::Rem
            if let Core::ConstInt(n) = rc
                && let Some(n) = n.to_i64()
                && n >= 2
                && let Core::Arith {
                    op: Prim::Rem,
                    lhs: v,
                    rhs: inner_div,
                } = core_of(db, lhs)
                && let Core::ConstInt(mm) = core_of(db, inner_div)
                && let Some(m) = mm.to_i64()
                && m >= 2
                && m % n == 0 =>
        {
            trace!(target: "rcdzc::fold", inner_m = m, outer_n = n, "nested modulo (% (% v M) N) → (% v N) (N | M)");
            Some(Core::Arith {
                op: Prim::Rem,
                lhs: v,
                rhs,
            })
        }

        // COMPLEMENT LAWS: `x & ~x` → 0 and `x | ~x` → -1 (all-ones), where `~x` is `(^ x -1)` (there is no
        // dedicated bit-NOT prim). A value AND its bitwise complement share NO set bit, so `&` is 0 and `|`
        // is every bit set. Both DISCARD `x` (the result does not depend on it), so gated on
        // `is_trap_free(x)` — a trapping `x` must still trap. `complement_pair` matches `v` against
        // `(^ v -1)` on either operand order.
        //
        // The `&` result 0 is valid at EVERY width/sign. But the `|` all-ones result is `-1` only for a
        // SIGNED type; an UNSIGNED width-N all-ones is `2^N − 1`, and a literal `-1` is OUT OF RANGE there
        // (`(: -1 UInt8)` is a CDZ0302 reject) — `arith_identity` has no width to synthesize `2^N−1`. So the
        // `|` fold is restricted to a SIGNED operand type, where `-1` IS the all-ones and representable;
        // an unsigned `x | ~x` keeps the runtime `or` (correct, just not folded).
        Prim::BitAnd if complement_pair(db, lhs, rhs).is_some_and(|v| is_trap_free(db, v)) => {
            trace!(target: "rcdzc::fold", "complement law x & ~x → 0");
            Some(zero())
        }
        Prim::BitOr
            if matches!(crate::infer::type_of(db, lhs), crate::ty::Ty::Int(it) if it.ground_signed())
                && complement_pair(db, lhs, rhs).is_some_and(|v| is_trap_free(db, v)) =>
        {
            trace!(target: "rcdzc::fold", "complement law x | ~x → -1 (all ones, signed)");
            Some(Core::ConstInt(IntValue::from_i64(-1)))
        }
        // SAME-OPERAND identities: the two operands are the SAME value (`core_equiv`), so the result is
        // determined regardless of that value. `core_equiv` matches only pure scalar cores, but the
        // operand may still be a checked op that TRAPS (`(- (/ a b) (/ a b))` — the `/` traps on b==0),
        // so a DISCARDING identity (`- a a → 0`, `^ a a → 0`) fires only when the operand is trap-free;
        // eliding a possibly-trapping operand would drop a defined trap. The KEEPING identities
        // (`& a a → a`, `| a a → a`) return the operand's own core, so its traps are preserved — always
        // safe. (`/ a a → 1` is NOT applied: `a == 0` traps ÷0, a defined outcome, so it is not an
        // identity.)
        Prim::Sub | Prim::BitXor if core_equiv(db, lhs, rhs) && is_trap_free(db, lhs) => {
            Some(zero())
        }
        Prim::BitAnd | Prim::BitOr if core_equiv(db, lhs, rhs) => Some(lc.clone()),
        _ => None,
    }
}

/// For an outer bitwise op with operands `(lhs, rhs)`, whether one operand is the bitwise COMPLEMENT of
/// the other — i.e. one is `v` and the other is `(^ v -1)` (`~v`). Returns the un-complemented value `v`
/// (so the caller can trap-check it, since the complement laws `x & ~x = 0` / `x | ~x = -1` DISCARD `x`).
/// Both operand orders are tried, and the `-1` may be on either side of the inner XOR. `None` otherwise.
fn complement_pair(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<StructId> {
    // Whether `maybe_not` is `(^ v -1)` for a `v` that is `core_equiv` to `other`.
    let is_not_of = |db: &mut Db, maybe_not: StructId, other: StructId| -> bool {
        let Core::Arith {
            op: Prim::BitXor,
            lhs: il,
            rhs: ir,
        } = core_of(db, maybe_not)
        else {
            return false;
        };
        let is_neg1 =
            |c: &Core| matches!(c, Core::ConstInt(v) if v.eq_value(&IntValue::from_i64(-1)));
        // `(^ v -1)` — the `-1` on the right (`v` = il) or left (`v` = ir), and `v` matches `other`.
        (is_neg1(&core_of(db, ir)) && core_equiv(db, il, other))
            || (is_neg1(&core_of(db, il)) && core_equiv(db, ir, other))
    };
    if is_not_of(db, rhs, lhs) {
        Some(lhs) // `(op v (^ v -1))`
    } else if is_not_of(db, lhs, rhs) {
        Some(rhs) // `(op (^ v -1) v)`
    } else {
        None
    }
}

/// Whether `lhs`/`rhs` are BOOLEAN complements — one is `v` and the other is `(not v)` (`Core::Not` whose
/// operand is `core_equiv` to the first). The `and`/`or` analogue of the bitwise `complement_pair`; drives
/// the boolean complement laws `(and a (not a))` → false, `(or a (not a))` → true. Both operand orders are
/// tried. (`Core::Not` is `lower`'s canonical boolean negation — a `(not (not a))` already cancelled in the
/// `Resolved::Not` fold, so a `Not` here wraps a non-`Not` operand.)
fn bool_complement_pair(db: &mut Db, lhs: StructId, rhs: StructId) -> bool {
    let is_not_of = |db: &mut Db, maybe_not: StructId, other: StructId| -> bool {
        matches!(core_of(db, maybe_not), Core::Not { operand } if core_equiv(db, operand, other))
    };
    is_not_of(db, rhs, lhs) || is_not_of(db, lhs, rhs)
}

/// Fold a SHORT-CIRCUIT CONNECTIVE `(and/or lhs rhs)` (the `is_and` flag selects) into its simplest core.
/// Shared by the `Resolved::And` arm AND the `(if c a false)`→`(and c a)` / `(if c true b)`→`(or c b)`
/// if-encoded-connective rewrites — an if-shaped connective routes through the SAME boolean-algebra fold
/// family (constant short-circuit, idempotence, absorption, complement, and the comparison-pair folds).
///
/// A constant LEFT operand short-circuits WITHOUT evaluating `rhs` (a trapping/ill-formed `rhs` is
/// shielded, exactly as an `if`'s unselected branch): `(and false _)`→false, `(and true rhs)`→rhs;
/// `(or true _)`→true, `(or false rhs)`→rhs. Otherwise `lhs` is the always-evaluated short-circuit
/// condition; the arms below simplify against a constant/structural `rhs`, and the fallthrough emits a
/// `Core::And` the backend lowers to `if lhs then/else <rhs|const>`.
fn fold_short_circuit(db: &mut Db, lhs: StructId, rhs: StructId, is_and: bool) -> Core {
    match core_of(db, lhs) {
        Core::ConstBool(b) => {
            // `and`: left decides when false (short-circuit to false), else the result is rhs.
            // `or`:  left decides when true  (short-circuit to true),  else the result is rhs.
            if b == is_and {
                core_of(db, rhs) // and-true → rhs ; or-false → rhs
            } else {
                Core::ConstBool(!is_and) // and-false → false ; or-true → true
            }
        }
        Core::Poison(r) => Core::Poison(r),
        // A constant RIGHT operand (the left is a non-constant runtime bool, ALWAYS evaluated — it is
        // the short-circuit condition). `(and p true)` / `(or p false)` → `p` (the neutral element,
        // keeps `p` so its effects/traps stay). `(and p false)` → `false` / `(or p true)` → `true`
        // (the ABSORBING element) — this DISCARDS `p`, so it is applied only when `p` is trap-free
        // (else `p`'s trap must still fire, so keep the `Core::And`). Mirrors the constant-left fold
        // above; completes the boolean-identity set. (Both-constant folded via the left arm already.)
        lc => match core_of(db, rhs) {
            Core::ConstBool(rb) if rb == is_and => lc, // and-true / or-false → p (neutral, keeps p)
            Core::ConstBool(_) if is_trap_free(db, lhs) => Core::ConstBool(!is_and), // absorbing
            // IDEMPOTENCE: `(and a a)` → `a` and `(or a a)` → `a` — a boolean combined with itself is
            // itself. The two operands are the SAME value (`core_equiv`), so the result is `a`. `lhs` is
            // the short-circuit condition, ALWAYS evaluated (and evaluated ONCE by returning its core),
            // so `a`'s own effects/traps are preserved regardless of the fold — no `is_trap_free` guard
            // needed (`lhs` runs exactly as it would as the condition; `rhs`, a re-evaluation of the
            // same pure value, is dropped). Mirrors the bitwise `(& a a)`/`(| a a)` same-operand fold.
            _ if core_equiv(db, lhs, rhs) => lc,
            // NESTED IDEMPOTENCE / ABSORPTION: `(and (and a b) a)` → `(and a b)` and `(or (or a b) a)` →
            // `(or a b)` — one operand is a nested SAME-connective `(and/or p q)` that already CONTAINS
            // the other operand (`p` or `q` is `core_equiv` to it), so re-conjoining/disjoining it is
            // redundant. Returns the nested node (all operands stay evaluated → trap-safe, like the
            // bitwise idempotent collapse c117). Only the SAME connective (`is_and` matches). Both outer
            // orders are tried by `bool_nested_idempotent`.
            _ if let Some(keep) = bool_nested_idempotent(db, lhs, rhs, is_and) => core_of(db, keep),
            // ABSORPTION LAW: `(and a (or a b))` → `a` and `(or a (and a b))` → `a` — a boolean combined
            // with the DUAL connective of itself-with-anything absorbs to itself (the short-circuit
            // analogue of the bitwise `x & (x|y)`→x / `x | (x&y)`→x fold, c118). One operand is an inner
            // `and`/`or` of the DUAL connective CONTAINING `x`; the other is `x`. Result is `x`. DISCARDS
            // the inner op's OTHER operand `y`, so gated on `is_trap_free(y)` — `y` is only conditionally
            // evaluated in the short-circuit original, so trap-freedom suffices to drop it. `x` is pure
            // (`core_equiv`) so returning it evaluates once with no trap. Both orders via
            // `bool_absorption_operand`.
            _ if let Some((x, y)) = bool_absorption_operand(db, lhs, rhs, is_and)
                && is_trap_free(db, y) =>
            {
                core_of(db, x)
            }
            // COMPLEMENT LAW: `(and a (not a))` → `false` and `(or a (not a))` → `true` — a boolean and
            // its negation are exhaustive+exclusive, so `and` is always false and `or` always true. The
            // boolean analogue of the bitwise `x & ~x`/`x | ~x` fold (c119). DISCARDS both operands (the
            // result is a constant), so gated on `is_trap_free(lhs)` — a trapping `a` must still trap
            // (`core_equiv` matches only pure cores, so `a` is pure anyway, but keep the guard explicit).
            // Both operand orders (`a`&`!a` / `!a`&`a`) are handled by `bool_complement_pair`.
            _ if bool_complement_pair(db, lhs, rhs) && is_trap_free(db, lhs) => {
                Core::ConstBool(!is_and) // and → false ; or → true
            }
            // COMPLEMENTARY-COMPARISON LAW: `(or (< a b) (>= a b))` → true, `(and (< a b) (>= a b))` →
            // false — two comparisons on the SAME operand PAIR whose operators are exact COMPLEMENTS
            // (`< `↔`>=`, `<=`↔`>`) partition the total order, so their `or` is exhaustive (always true)
            // and their `and` is exclusive (always false). A redundant range guard (`(or (< x c) (>= x
            // c))`). DISCARDS both operands, so gated on `is_trap_free` for each (a comparison is
            // trap-free iff its operands are; a `(< (/ a b) 5)` with a trapping `/` keeps the runtime
            // form). `complementary_comparisons` checks same-pair + complement-op.
            _ if complementary_comparisons(db, lhs, rhs)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::ConstBool(!is_and) // or → true ; and → false
            }
            // SUBSUMPTION: two comparisons on the SAME runtime operand `v` against constants with the
            // SAME operator (both `<`, both `<=`, both `>`, or both `>=`) — one implies the other, so
            // the redundant one drops. `and` keeps the STRONGER (tighter bound), `or` the WEAKER
            // (looser): `(and (< v 5) (< v 10))` → `(< v 5)`, `(or (< v 5) (< v 10))` → `(< v 10)`. The
            // kept comparison still evaluates `v` (its trap, if any, is preserved) — no operand is
            // dropped, only the redundant second bound. `subsuming_comparison` returns the occurrence to
            // keep (`lhs` or `rhs`).
            _ if let Some(keep) = subsuming_comparison(db, lhs, rhs, is_and) => core_of(db, keep),
            // COINCIDENT-POINT COLLAPSE: `(and (>= v c) (<= v c))` → `(= v c)` — two INCLUSIVE
            // opposite bounds pinning `v` to a single point ARE equality (`v>=c && v<=c ⟺ v==c`), so
            // three ops (`ge`+`le`+`and`) become one `eq`. Only under `and`; reuses the existing (proven
            // in-type) constant node, so no synthesis / no range guard. DISCARDS the second comparison,
            // so gated on `is_trap_free` for both (like the sibling disjoint/covering fold); the kept
            // `(= v c)` still evaluates `v`. Distinct from disjoint/covering (which folds `L>U` empty /
            // `L<=U+1` covering — the coincident `L==U` point is exactly what THIS fold handles).
            _ if is_and
                && let Some((v, c)) = coincident_point_eq(db, lhs, rhs)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::Compare {
                    op: Prim::Eq,
                    lhs: v,
                    rhs: c,
                }
            }
            // DISJOINT/COVERING INTERVAL: two comparisons on the SAME operand `v` vs constants forming
            // OPPOSITE-direction half-lines (one an upper bound `v ≤ U`, the other a lower bound `v ≥
            // L`). `and` (intersection `L ≤ v ≤ U`) is EMPTY iff `L > U` → `false`; `or` (union) COVERS
            // everything iff the half-lines touch/overlap (`L ≤ U+1`) → `true`. `(and (< x 5) (> x 10))`
            // → false, `(or (< x 5) (> x 3))` → true. Only the constant verdicts (a non-empty `and` /
            // gapped `or` is not a constant — kept). DISCARDS both operands, so gated on `is_trap_free`.
            _ if let Some(v) = disjoint_or_covering(db, lhs, rhs, is_and)
                && is_trap_free(db, lhs)
                && is_trap_free(db, rhs) =>
            {
                Core::ConstBool(v)
            }
            // EQUALITY-VS-RANGE: one operand is `(= x c)`, the other an ordering comparison `(cmp x k)`
            // on the SAME `x`. Whether `c` satisfies `(cmp c k)` (a compile-time test) decides:
            //   `and`: `sat` → `(= x c)` (the range is redundant given equality); `!sat` → `false`
            //          (equality contradicts the range). `(and (= x 5) (> x 0))` → `(= x 5)`,
            //          `(and (= x 5) (> x 100))` → false.
            //   `or`:  `sat` → `(cmp x k)` (equality is subsumed by the range it satisfies); `!sat` →
            //          keep both (not a constant — `x==c` adds one point outside the range).
            // Each DISCARDS one operand — gated on that operand's `is_trap_free`. `eq_vs_range` returns
            // `(eq_node, range_node, sat)`.
            _ if let Some((eq_node, range_node, sat)) = eq_vs_range(db, lhs, rhs) => {
                if is_and {
                    if sat && is_trap_free(db, range_node) {
                        core_of(db, eq_node) // range redundant → keep the equality
                    } else if !sat && is_trap_free(db, eq_node) && is_trap_free(db, range_node) {
                        Core::ConstBool(false) // contradiction
                    } else {
                        Core::And { lhs, rhs, is_and }
                    }
                } else if sat && is_trap_free(db, eq_node) {
                    core_of(db, range_node) // `or`: equality subsumed → keep the range
                } else {
                    Core::And { lhs, rhs, is_and }
                }
            }
            // REASSOCIATE TO EXPOSE A COMPARISON PAIR across a same-connective nested tree. The pairwise
            // comparison folds above only see the TWO DIRECT operands, so `(and (and (> x 0) (< x 100)) (> x
            // 5))` misses that `(> x 5)` subsumes the buried `(> x 0)`. When one operand is a same-connective
            // `(op P Q)` and the other is a comparison `C`, try folding `C` against `P` (and against `Q`) via
            // `fold_short_circuit`: if that pair COLLAPSES (to a constant or a single kept comparison — i.e.
            // NOT a plain two-operand `Core::And`), rebuild the tree with the collapsed result and the
            // remaining leaf. `(and (and (> x 0) (< x 100)) (> x 5))` → `(and (> x 5) (< x 100))`; a nested
            // COMPLEMENT `(and (and (< x y) …) (>= x y))` → false. SOUND only when every involved leaf (`C`,
            // `P`, `Q`) is TRAP-FREE: `and`/`or` is associative+commutative over pure booleans, so regrouping
            // and reordering is unobservable (no trap/effect order to preserve). `reassociate_comparison_pair`
            // returns the rebuilt `Core` or `None`.
            _ if let Some(folded) = reassociate_comparison_pair(db, lhs, rhs, is_and) => folded,
            _ => Core::And { lhs, rhs, is_and },
        },
    }
}

/// Reassociate a short-circuit `(op lhs rhs)` (connective `is_and`) to expose a COMPARISON PAIR that the
/// direct pairwise folds miss because it is split across a same-connective nested subtree. When one operand
/// is a nested `(op P Q)` (SAME connective) and the OTHER operand `C` is a comparison, this folds `C`
/// against `P` and against `Q` (via `fold_short_circuit`); if either pair COLLAPSES — the recursive fold
/// returns something OTHER than a plain two-operand `Core::And` of those same two nodes (a constant, or a
/// single subsuming comparison) — the whole tree is rebuilt as `(op collapsed remaining_leaf)`, dropping the
/// redundant comparison. `(and (and (> x 0) (< x 100)) (> x 5))` → `(and (> x 5) (< x 100))` (subsumption);
/// `(and (and (< x y) p) (>= x y))` → `false` (complement). Returns `None` when nothing collapses.
///
/// SOUNDNESS: fires ONLY when `C`, `P`, and `Q` are all TRAP-FREE. A short-circuit `and`/`or` over pure
/// (trap-free, effect-free) boolean operands is fully associative AND commutative — there is no evaluation
/// order or trap to preserve — so regrouping `(op (op P Q) C)` as `(op (op C P) Q)` and folding the exposed
/// `(op C P)` pair is behavior-identical. (A non-trap-free leaf could change WHICH branch's trap fires or
/// its order, so it is excluded — the tree stays as-is.) Both outer operand orders and both nested-operand
/// positions are tried.
fn reassociate_comparison_pair(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<Core> {
    // Try: `nested` = a same-connective `(op P Q)`, `c` = the other operand (a comparison). Fold `c`
    // against each nested leaf; on a genuine collapse, rebuild `(op collapsed other_leaf)`.
    let try_side = |db: &mut Db, nested: StructId, c: StructId| -> Option<Core> {
        // `c` must be a comparison, and trap-free (a discarding/regrouping fold requires purity).
        if !matches!(core_of(db, c), Core::Compare { .. }) || !is_trap_free(db, c) {
            return None;
        }
        let Core::And {
            lhs: p,
            rhs: q,
            is_and: nested_is_and,
        } = core_of(db, nested)
        else {
            return None;
        };
        if nested_is_and != is_and {
            return None; // must be the SAME connective to reassociate
        }
        // Every leaf must be trap-free so the reassociation is unobservable.
        if !is_trap_free(db, p) || !is_trap_free(db, q) {
            return None;
        }
        // Fold `c` against P, keeping Q; then against Q, keeping P. A genuine collapse = the recursive fold
        // did NOT return a plain `Core::And` re-pairing the same two nodes (that would be no progress).
        let collapsed = |db: &mut Db, pair_a: StructId, pair_b: StructId| -> Option<Core> {
            let folded = fold_short_circuit(db, pair_a, pair_b, is_and);
            match folded {
                // No progress: the pair stayed a two-operand `and`/`or`. (Any other shape — ConstBool, a
                // single Compare, a Not, an Eq — is a real collapse.)
                Core::And { .. } => None,
                other => Some(other),
            }
        };
        if let Some(folded) = collapsed(db, c, p) {
            // `(op (op P Q) C)` → `(op folded(C,P) Q)`.
            let fid = synth_core(db, folded, crate::ty::Ty::Bool);
            return Some(fold_short_circuit(db, fid, q, is_and));
        }
        if let Some(folded) = collapsed(db, c, q) {
            // → `(op folded(C,Q) P)`.
            let fid = synth_core(db, folded, crate::ty::Ty::Bool);
            return Some(fold_short_circuit(db, fid, p, is_and));
        }
        None
    };
    try_side(db, lhs, rhs).or_else(|| try_side(db, rhs, lhs))
}

/// NESTED IDEMPOTENCE for a short-circuit `and`/`or`: when one outer operand is a nested `Core::And` of the
/// SAME connective (`is_and`) that already CONTAINS the other outer operand (one of its sides is
/// `core_equiv` to it), the outer re-application is redundant — `(and (and a b) a)` == `(and a b)`. Returns
/// the NESTED node to keep (all its operands stay evaluated → trap-safe, no operand dropped). Both outer
/// operand orders and both nested-operand positions are tried. `None` when the shape does not match.
fn bool_nested_idempotent(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<StructId> {
    // `nested` is `(op p q)` with the SAME connective; `outer` must be `core_equiv` to `p` or `q`.
    let check = |db: &mut Db, nested: StructId, outer: StructId| -> Option<StructId> {
        let Core::And {
            lhs: p,
            rhs: q,
            is_and: nested_is_and,
        } = core_of(db, nested)
        else {
            return None;
        };
        if nested_is_and != is_and {
            return None;
        }
        (core_equiv(db, p, outer) || core_equiv(db, q, outer)).then_some(nested)
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// The SHORT-CIRCUIT BOOLEAN ABSORPTION LAW: `(and a (or a b))` → `a` and `(or a (and a b))` → `a` (either
/// outer order, `a` on either side of the inner op). A boolean combined with the DUAL connective of
/// itself-with-anything absorbs to itself — the boolean analogue of the bitwise `x & (x|y)`→x / `x | (x&y)`
/// →x fold (c118, `absorption_operand`). The outer connective is `is_and`; one operand must be an inner
/// `Core::And` of the DUAL connective (`or` under `and`, `and` under `or`) that CONTAINS `x` (either side);
/// the OTHER outer operand is `x` (`core_equiv`). Returns `(x, y)` — the whole expression absorbs to `x`,
/// discarding the inner op's OTHER operand `y`. `x` is pure (`core_equiv` matches only pure cores) so
/// returning it evaluates it once with no trap; `y` may be arbitrary, so the caller gates `is_trap_free(y)`
/// (in the short-circuit original `y` is only conditionally evaluated, so trap-freedom is SUFFICIENT to
/// drop it soundly). Both outer orders and both inner-operand positions are tried.
fn bool_absorption_operand(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<(StructId, StructId)> {
    // `inner` must be a `Core::And` of the DUAL connective; `outer_x` must be `core_equiv` to one operand.
    let check = |db: &mut Db, inner: StructId, outer_x: StructId| -> Option<(StructId, StructId)> {
        let Core::And {
            lhs: ip,
            rhs: iq,
            is_and: inner_is_and,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_is_and == is_and {
            return None; // must be the DUAL connective (`or` under `and`, `and` under `or`)
        }
        if core_equiv(db, ip, outer_x) {
            Some((outer_x, iq)) // x matched ip → y is iq
        } else if core_equiv(db, iq, outer_x) {
            Some((outer_x, ip)) // x matched iq → y is ip
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// Whether `lhs`/`rhs` are two comparisons on the SAME operand pair whose operators are exact COMPLEMENTS
/// over the total order — `< `/`>=` or `<=`/`>` — so together they partition every value: their `or` is
/// always TRUE (exhaustive) and their `and` always FALSE (disjoint). `(or (< a b) (>= a b))` → true,
/// `(and (< a b) (>= a b))` → false. Requires BOTH to be `Core::Compare` with `core_equiv` operand pairs
/// (same order — `< a b` complements `>= a b`, NOT `>= b a`) and complementary ops. `=`/`Compare` are not
/// ordering complements and never match. Drives the complementary-comparison fold (caller trap-guards).
fn complementary_comparisons(db: &mut Db, lhs: StructId, rhs: StructId) -> bool {
    let Core::Compare {
        op: lop,
        lhs: la,
        rhs: lb,
    } = core_of(db, lhs)
    else {
        return false;
    };
    let Core::Compare {
        op: rop,
        lhs: ra,
        rhs: rb,
    } = core_of(db, rhs)
    else {
        return false;
    };
    // Exact ordering complements: `<` ↔ `>=`, `<=` ↔ `>` (either assignment to lhs/rhs).
    let complement = matches!(
        (lop, rop),
        (Prim::Lt, Prim::Ge) | (Prim::Ge, Prim::Lt) | (Prim::Le, Prim::Gt) | (Prim::Gt, Prim::Le)
    );
    // Same operand pair in the SAME order (the operators already encode the direction).
    complement && core_equiv(db, la, ra) && core_equiv(db, lb, rb)
}

/// SUBSUMPTION between two comparisons on the SAME runtime operand `v` against CONSTANTS that form
/// SAME-DIRECTION half-lines (both upper bounds `v ≤ B`, or both lower `v ≥ B`) — one implies the other,
/// so `(and …)`/`(or …)` keeps just one. Returns the occurrence to KEEP (`lhs` or `rhs`), or `None` when
/// the pair is not two same-direction half-lines on the same `v`. `is_and` selects which survives: `and`
/// keeps the STRONGER (tighter) bound, `or` the WEAKER (looser).
///
/// Uses `comparison_halfline` to normalize each side to an INCLUSIVE bound (`v ≤ B` / `v ≥ B`), so MIXED
/// operators are handled uniformly — `(< v 5)` and `(<= v 4)` both normalize to `v ≤ 4` (and keeps either),
/// `(or (<= v 10) (< v 5))` → `v ≤ 10` (the looser). For two UPPER bounds the tighter is the SMALLER `B`;
/// for two LOWER bounds the tighter is the LARGER `B`. `comparison_halfline` already handles either operand
/// side (a mirrored `(< c v)` normalizes to a lower bound on `v`) and only the four ordering ops (`Eq`/
/// `Compare` are not half-lines, so a `(= x 5)`/`(= x 6)` pair returns `None` here — never mis-subsumed).
/// The kept comparison still evaluates `v`, so no trap drops. OPPOSITE-direction pairs are `None` here (the
/// disjoint/covering + coincident-point folds handle those).
fn subsuming_comparison(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    is_and: bool,
) -> Option<StructId> {
    let (lv, l_upper, lb) = comparison_halfline(db, lhs)?;
    let (rv, r_upper, rb) = comparison_halfline(db, rhs)?;
    // Same operand, same direction (both upper or both lower) — an opposite-direction pair is a range, not
    // a subsumption (handled by `disjoint_or_covering`/`coincident_point_eq`).
    if l_upper != r_upper || !core_equiv(db, lv, rv) {
        return None;
    }
    // For UPPER bounds `v ≤ B`, the tighter (stronger) is the SMALLER B; for LOWER bounds `v ≥ B`, the
    // LARGER B. `and` keeps the stronger, `or` the weaker.
    let lhs_stronger = if l_upper { lb <= rb } else { lb >= rb };
    let keep_lhs = if is_and { lhs_stronger } else { !lhs_stronger };
    Some(if keep_lhs { lhs } else { rhs })
}

/// Normalize a `Core::Compare` on a runtime operand `v` against a constant into an INCLUSIVE half-line
/// bound on `v`, as `(v, is_upper, bound)` — `is_upper` means `v <= bound`, else `v >= bound`. Handles all
/// four ops on either operand side (`(< v c)` → `v <= c-1`; `(> v c)` → `v >= c+1`; `(< c v)` = `v > c` →
/// `v >= c+1`; etc). Bound arithmetic is `i128` so `c±1` never overflows at the i64 extremes. `None` when
/// the node is not a comparison of a runtime value against a constant. Used by `disjoint_or_covering`.
fn comparison_halfline(db: &mut Db, id: StructId) -> Option<(StructId, bool, i128)> {
    let Core::Compare { op, lhs, rhs } = core_of(db, id) else {
        return None;
    };
    let as_int = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64().map(|v| v as i128),
        _ => None,
    };
    // `(op v c)` (v on the left) or `(op c v)` (v on the right, which flips the operator's sense).
    let (v, c, v_left) = match (as_int(db, rhs), as_int(db, lhs)) {
        (Some(c), _) => (lhs, c, true),
        (_, Some(c)) => (rhs, c, false),
        _ => return None,
    };
    // Effective operator with `v` on the left (`(op c v)` mirrors: `<`↔`>`, `<=`↔`>=`).
    let eff = if v_left {
        op
    } else {
        match op {
            Prim::Lt => Prim::Gt,
            Prim::Gt => Prim::Lt,
            Prim::Le => Prim::Ge,
            Prim::Ge => Prim::Le,
            other => other,
        }
    };
    // To an inclusive bound: `v < c` ⇒ `v <= c-1`; `v <= c` ⇒ `v <= c`; `v > c` ⇒ `v >= c+1`; `v >= c` ⇒
    // `v >= c`. (`=`/`Compare` are not half-lines.)
    match eff {
        Prim::Lt => Some((v, true, c - 1)),
        Prim::Le => Some((v, true, c)),
        Prim::Gt => Some((v, false, c + 1)),
        Prim::Ge => Some((v, false, c)),
        _ => None,
    }
}

/// For two comparisons forming OPPOSITE-direction half-lines on the SAME operand `v` — one `v <= U`, the
/// other `v >= L` — decide whether their `and`/`or` is a CONSTANT. `and` (intersection `L <= v <= U`) is
/// EMPTY iff `L > U` → `Some(false)`; `or` (union) COVERS every value iff the half-lines touch or overlap
/// (`L <= U + 1`) → `Some(true)`. `None` when the pair is not opposite half-lines on the same `v`, or the
/// intersection is non-empty (`and`) / the union has a gap (`or`) — those stay runtime. `(and (< x 5) (> x
/// 10))` → false; `(or (< x 5) (> x 3))` → true. All bound math is `i128` (no overflow at i64 extremes).
fn disjoint_or_covering(db: &mut Db, lhs: StructId, rhs: StructId, is_and: bool) -> Option<bool> {
    let (lv, l_upper, lb) = comparison_halfline(db, lhs)?;
    let (rv, r_upper, rb) = comparison_halfline(db, rhs)?;
    if l_upper == r_upper || !core_equiv(db, lv, rv) {
        return None; // need OPPOSITE directions on the SAME operand
    }
    // Order them: `u` = the upper bound `v <= U`, `l` = the lower bound `v >= L`.
    let (upper, lower) = if l_upper { (lb, rb) } else { (rb, lb) };
    if is_and {
        // Intersection `lower <= v <= upper` is empty iff `lower > upper`.
        (lower > upper).then_some(false)
    } else {
        // Union `v <= upper || v >= lower` covers all iff the pieces touch/overlap: `lower <= upper + 1`.
        (lower <= upper + 1).then_some(true)
    }
}

/// COINCIDENT-POINT COLLAPSE for `and`: `(and (>= v c) (<= v c))` (either operand order, `v` on either
/// side of each comparison) → `(= v c)`. Two INCLUSIVE opposite-direction bounds pinning `v` to a single
/// point ARE equality — `v >= c && v <= c ⟺ v == c` in any total order (sound for signed AND unsigned
/// integers alike; it is a pure order-theoretic fact, no sign assumption). Returns `(v, c_node)` to build
/// `Core::Compare { op: Eq, lhs: v, rhs: c_node }` — three ops (`ge` + `le` + `and`) collapse to one `eq`.
/// Restricted to the two INCLUSIVE ops (`>=`/`<=`) against the SAME i64 constant VALUE on both sides, and
/// REUSES an existing constant node (proven representable in `v`'s type — it typechecked against `v`), so
/// no constant is synthesized and no type-range guard is needed. The strictly-inclusive requirement also
/// keeps this distinct from the exclusive width-2 point `(and (> v (c-1)) (< v (c+1)))`, which would need a
/// synthesized `c` + a representability guard — deliberately left un-folded (conservative, no regression).
/// `None` unless the shape matches. DISCARDS the second comparison, so the caller gates on `is_trap_free`
/// for both operands (matches the sibling disjoint/covering fold); the kept `(= v c)` evaluates `v` once.
fn coincident_point_eq(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<(StructId, StructId)> {
    // From a `Core::Compare` on a runtime `v` against an i64 constant, return `(v, c_node, c_value, eff)`
    // where `eff` is the operator NORMALIZED to `v` on the left (`(op c v)` mirrors `<`↔`>`, `<=`↔`>=`).
    let bound_of = |db: &mut Db, id: StructId| -> Option<(StructId, StructId, i64, Prim)> {
        let Core::Compare { op, lhs: a, rhs: b } = core_of(db, id) else {
            return None;
        };
        let as_int = |db: &mut Db, id: StructId| match core_of(db, id) {
            Core::ConstInt(v) => v.to_i64(),
            _ => None,
        };
        // `(op v c)` (v on the left) or `(op c v)` (v on the right, which mirrors the operator).
        match (as_int(db, b), as_int(db, a)) {
            (Some(c), _) => Some((a, b, c, op)),
            (_, Some(c)) => Some((
                b,
                a,
                c,
                match op {
                    Prim::Lt => Prim::Gt,
                    Prim::Gt => Prim::Lt,
                    Prim::Le => Prim::Ge,
                    Prim::Ge => Prim::Le,
                    other => other,
                },
            )),
            _ => None,
        }
    };
    let (lv, lc_node, lc, leff) = bound_of(db, lhs)?;
    let (rv, _rc_node, rc, reff) = bound_of(db, rhs)?;
    // Same runtime operand, same constant VALUE, and the two effective ops are exactly `>=` and `<=`
    // (opposite INCLUSIVE bounds). Either assignment (`>= , <=` or `<= , >=`).
    if lc != rc || !core_equiv(db, lv, rv) {
        return None;
    }
    let inclusive_opposite = matches!((leff, reff), (Prim::Ge, Prim::Le) | (Prim::Le, Prim::Ge));
    if !inclusive_opposite {
        return None;
    }
    trace!(target: "rcdzc::fold", "coincident-point collapse (and (>= v c) (<= v c)) → (= v c)");
    // Reuse `lv` as the operand and lhs's constant node as `c` — both proven trap-free by the caller's gate.
    Some((lv, lc_node))
}

/// For two comparisons where one is an EQUALITY `(= x c)` and the other an ORDERING comparison `(cmp x k)`
/// on the SAME `x` (both constants), return `(eq_node, range_node, sat)` — `sat` = whether `c` satisfies
/// the range predicate `(cmp c k)`, computed at compile time. The caller decides the fold: for `and`, `sat`
/// keeps the equality (range redundant) / `!sat` is `false` (contradiction); for `or`, `sat` keeps the
/// range (equality subsumed). `None` unless exactly one side is a scalar `Eq` and the other a scalar
/// ordering comparison (`< > <= >=`), both on the SAME `x` (`core_equiv`) against i64 constants.
fn eq_vs_range(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<(StructId, StructId, bool)> {
    let as_const_i64 = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // Extract `(x, c)` from a `(= x c)` / `(= c x)` node (equality is symmetric).
    let eq_of = |db: &mut Db, id: StructId| -> Option<(StructId, i64)> {
        let Core::Compare {
            op: Prim::Eq,
            lhs: a,
            rhs: b,
        } = core_of(db, id)
        else {
            return None;
        };
        match (as_const_i64(db, b), as_const_i64(db, a)) {
            (Some(c), _) => Some((a, c)),
            (_, Some(c)) => Some((b, c)),
            _ => None,
        }
    };
    // Extract `(x, effective-op-with-x-on-left, k)` from an ordering comparison `(cmp x k)` / `(cmp k x)`.
    let range_of = |db: &mut Db, id: StructId| -> Option<(StructId, Prim, i64)> {
        let Core::Compare { op, lhs: a, rhs: b } = core_of(db, id) else {
            return None;
        };
        if !matches!(op, Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge) {
            return None;
        }
        match (as_const_i64(db, b), as_const_i64(db, a)) {
            (Some(k), _) => Some((a, op, k)), // `(op x k)`
            (_, Some(k)) => Some((
                b,
                match op {
                    // `(op k x)` mirrors to x on the left.
                    Prim::Lt => Prim::Gt,
                    Prim::Gt => Prim::Lt,
                    Prim::Le => Prim::Ge,
                    Prim::Ge => Prim::Le,
                    other => other,
                },
                k,
            )),
            _ => None,
        }
    };
    // Try both assignments (eq on the left or right).
    let (eq_node, range_node, ex, c, rx, rop, k) =
        if let (Some((ex, c)), Some((rx, rop, k))) = (eq_of(db, lhs), range_of(db, rhs)) {
            (lhs, rhs, ex, c, rx, rop, k)
        } else if let (Some((ex, c)), Some((rx, rop, k))) = (eq_of(db, rhs), range_of(db, lhs)) {
            (rhs, lhs, ex, c, rx, rop, k)
        } else {
            return None;
        };
    if !core_equiv(db, ex, rx) {
        return None; // same `x`
    }
    // Does the equality's value `c` satisfy the range predicate `(rop c k)`?
    let sat = compare_ord(rop, c.cmp(&k));
    Some((eq_node, range_node, sat))
}

/// The NESTED-BITWISE COLLAPSE for an outer TOTAL, ASSOCIATIVE bitwise op (`&`/`|`/`^`) whose operands
/// are `(lhs, rhs)` (cores `lc`/`rc`): when one operand is `(OP v C1)` — the SAME op — and the OTHER is a
/// constant `C2`, returns `(OP v (C1 ⊙ C2))` where `⊙` is that op's constant fold — one op instead of
/// two. `(& (& v C1) C2)` → `(& v (C1&C2))`, `(| (| v C1) C2)` → `(| v (C1|C2))`, `(^ (^ v C1) C2)` →
/// `(^ v (C1^C2))`. `None` when neither shape matches (so the caller's later folds still fire). All three
/// ops are TOTAL (never trap) and ASSOCIATIVE, so no trap is dropped and the value is identical; `v` stays
/// the operand (its own traps preserved). The folded constant is a fresh `Leaf::Int` atom, lowered lazily
/// to `Core::ConstInt` and grounded to the op width at selection. (NOT for `+`/`-`/`*`/`<<` — those are
/// CHECKED, so `(v OP C1) OP C2` traps differently from `v OP (C1⊙C2)`.)
fn nested_bitwise_collapse(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    lc: &Core,
    rhs: StructId,
    rc: &Core,
) -> Option<Core> {
    if !matches!(op, Prim::BitAnd | Prim::BitOr | Prim::BitXor) {
        return None;
    }
    let apply = |a: i64, b: i64| match op {
        Prim::BitAnd => a & b,
        Prim::BitOr => a | b,
        _ => a ^ b, // BitXor
    };
    // The `(v, C1)` of an inner `(OP v C1)` node with the SAME op, C1 a constant on either side.
    let nested_op_const = |db: &mut Db, inner: StructId| -> Option<(StructId, i64)> {
        let Core::Arith {
            op: inner_op,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != op {
            return None;
        }
        match (core_of(db, il), core_of(db, ir)) {
            (Core::ConstInt(c), _) => c.to_i64().map(|c| (ir, c)),
            (_, Core::ConstInt(c)) => c.to_i64().map(|c| (il, c)),
            _ => None,
        }
    };
    let combine = |db: &mut Db, inner: StructId, outer_c: i64| -> Option<Core> {
        let (v, inner_c) = nested_op_const(db, inner)?;
        let folded = apply(inner_c, outer_c);
        let fc = db.push_atom(crate::ast::Leaf::Int {
            value: IntValue::from_i64(folded),
            radix: crate::ast::Radix::Dec,
        });
        trace!(target: "rcdzc::fold", ?op, inner_c, outer_c, folded, "nested-bitwise collapse (OP (OP v C1) C2) → (OP v (C1⊙C2))");
        Some(Core::Arith {
            op,
            lhs: v,
            rhs: fc,
        })
    };
    // inner on the LEFT, constant C2 on the RIGHT.
    if let Core::ConstInt(c2) = rc
        && let Some(c2) = c2.to_i64()
        && let Some(folded) = combine(db, lhs, c2)
    {
        return Some(folded);
    }
    // constant C2 on the LEFT, inner on the RIGHT.
    if let Core::ConstInt(c2) = lc
        && let Some(c2) = c2.to_i64()
        && let Some(folded) = combine(db, rhs, c2)
    {
        return Some(folded);
    }
    None
}

/// XOR CANCELLATION for an outer `(^ lhs rhs)`: when one operand is `(^ v w)` and the OTHER is
/// `core_equiv` to `w`, the two XORs by `w` cancel — `(v ^ w) ^ w == v ^ (w ^ w) == v ^ 0 == v` (XOR is
/// associative/commutative and self-inverse). Returns `v`. Covers a CONSTANT `w` (`(^ (^ x 5) 5)` → x —
/// which `nested_bitwise_collapse` would leave as a residual `(^ x 0)`) AND a RUNTIME `w` (`(^ (^ x y) y)`
/// → x, the involution `nested_bitwise_collapse` cannot see). Both operand orders of the outer `^`, and
/// `w` on either side of the inner `^`, are tried. The result is `v` (its own traps preserved); `w` is
/// DISCARDED, so the caller gates on `is_trap_free(w)` — a trapping `w` (`(^ (^ v (/ a b)) (/ a b))` at
/// b==0) must still trap. Returns `(v, w)` so the caller can trap-check `w`.
fn xor_cancels(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<(StructId, StructId)> {
    // Try: `lhs` is the inner `(^ v w)`, `rhs` is the matching `w`.
    let check = |db: &mut Db, inner: StructId, outer_w: StructId| -> Option<(StructId, StructId)> {
        let Core::Arith {
            op: Prim::BitXor,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        // The outer operand `outer_w` must equal ONE side of the inner XOR; the OTHER side is `v`.
        if core_equiv(db, ir, outer_w) {
            Some((il, ir)) // (v, w)
        } else if core_equiv(db, il, outer_w) {
            Some((ir, il)) // (v, w) — inner XOR is commutative
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// IDEMPOTENT-BITWISE COLLAPSE for an outer `(op lhs rhs)` where `op` is `&` or `|`: when one operand is
/// an inner `(op v w)` (the SAME op) and the OTHER outer operand is `core_equiv` to `w`, return the inner
/// node — `(op (op v w) w) == (op v w)` because `&`/`|` are idempotent (`w op w == w`) and associative.
/// Covers a RUNTIME `w` the constant-folding `nested_bitwise_collapse` cannot (`(| (| x y) y)` → `(| x
/// y)`). Both outer orders and `w` on either side of the inner op are tried. The inner `(op v w)` node is
/// RETAINED, so both `v` and `w` are still evaluated — no trap is dropped (no `is_trap_free` needed).
fn idempotent_bitwise_collapse(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    // `inner` must be `(op v w)` with the SAME op; `outer_w` must match one of its operands.
    let check = |db: &mut Db, inner: StructId, outer_w: StructId| -> Option<StructId> {
        let Core::Arith {
            op: inner_op,
            lhs: il,
            rhs: ir,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != op {
            return None;
        }
        // The outer operand equals ONE side of the inner op → re-applying `op` by it is a no-op.
        if core_equiv(db, il, outer_w) || core_equiv(db, ir, outer_w) {
            Some(inner)
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// ABSORPTION LAW for an outer `(op lhs rhs)` where `op` is `&` or `|`: when one operand is an inner op of
/// the DUAL kind (`|` under `&`, `&` under `|`) that contains `x`, and the OTHER outer operand IS `x`
/// (`core_equiv`), the whole expression absorbs to `x` — `x & (x | y) == x`, `x | (x & y) == x`. Returns
/// `(x, y)` where `y` is the inner op's OTHER operand (the one absorbed away), so the caller can trap-check
/// `y` (it is DISCARDED). Both outer orders and both inner-operand positions are tried. `None` when the
/// shape does not match.
fn absorption_operand(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<(StructId, StructId)> {
    let dual = match op {
        Prim::BitAnd => Prim::BitOr,
        Prim::BitOr => Prim::BitAnd,
        _ => return None,
    };
    // `inner` must be `(dual p q)`; `outer_x` must equal one of `p`/`q` (that side is `x`, the other `y`).
    let check = |db: &mut Db, inner: StructId, outer_x: StructId| -> Option<(StructId, StructId)> {
        let Core::Arith {
            op: inner_op,
            lhs: ip,
            rhs: iq,
        } = core_of(db, inner)
        else {
            return None;
        };
        if inner_op != dual {
            return None;
        }
        if core_equiv(db, ip, outer_x) {
            Some((outer_x, iq)) // (x, y) — x matched ip, y is iq
        } else if core_equiv(db, iq, outer_x) {
            Some((outer_x, ip)) // (x, y) — x matched iq, y is ip
        } else {
            None
        }
    };
    check(db, lhs, rhs).or_else(|| check(db, rhs, lhs))
}

/// The NESTED SHIFT COLLAPSE for `(SH lhs rhs)` where `SH` is `Shr` OR `Shl`: when `lhs` is itself
/// `(SH v A)` — the SAME shift op — with a constant inner count A, and the outer count `rc` is a constant
/// B with `A + B < width`, returns `(SH v (A+B))` — one shift instead of two. `None` otherwise (so later
/// folds fire). For `>>`: the shift is total; inner and outer are the same kind (`>>ₛ`/`>>ᵤ`) on the
/// same-typed value, so composing drops the same low `A+B` bits as one shift by `A+B`. For `<<`: the
/// shift is CHECKED (exact `·2^count`, traps on N-bit overflow) but still collapses TRAP-IDENTICALLY —
/// magnitude is monotonic in the count, so `(v<<A)<<B` and `v<<(A+B)` overflow on exactly the same inputs
/// (inner overflow ⟹ combined; combined ⟹ the double's outer step) and agree on value otherwise. The
/// `A + B < width` bound is essential for BOTH: a combined count `≥ width` would be masked mod width by
/// the machine shift (`>>`) / must trap as an out-of-range count (`<<`), disagreeing with the double
/// shift. `v` keeps its traps (it stays the operand). The combined count `A+B` is a fresh `Leaf::Int`.
fn nested_shift_combine(db: &mut Db, op: Prim, lhs: StructId, rc: &Core) -> Option<Core> {
    // Only the two shift ops, and the inner must be the SAME op (a `<<` inside a `>>` composes bits
    // differently and does not collapse).
    if !matches!(op, Prim::Shr | Prim::Shl) {
        return None;
    }
    // Outer count B must be a constant ≥ 1 (0 is handled by the `SH 0` identity).
    let Core::ConstInt(b) = rc else { return None };
    let b = b.to_i64().filter(|&b| b >= 1)?;
    // `lhs` must be an inner shift by the SAME op with a constant count A ≥ 1.
    let Core::Arith {
        op: inner_op,
        lhs: v,
        rhs: inner_count,
    } = core_of(db, lhs)
    else {
        return None;
    };
    if inner_op != op {
        return None;
    }
    let Core::ConstInt(a) = core_of(db, inner_count) else {
        return None;
    };
    let a = a.to_i64().filter(|&a| a >= 1)?;
    // Sound ONLY when the combined count stays in range for the SHIFTED VALUE's width (both shifts share
    // it — binary-op unification). A `width` of 0 (deferred) fails the guard, so no fold.
    let width = shift_width(db, v) as i64;
    if width == 0 || a + b >= width {
        return None;
    }
    let fc = db.push_atom(crate::ast::Leaf::Int {
        value: IntValue::from_i64(a + b),
        radix: crate::ast::Radix::Dec,
    });
    trace!(target: "rcdzc::fold", ?op, a, b, sum = a + b, "nested shift collapse (SH (SH v A) B) → (SH v (A+B))");
    Some(Core::Arith {
        op,
        lhs: v,
        rhs: fc,
    })
}

/// OR-THEN-MASK ABSORPTION: for an outer `(& inner C2)` whose `inner` is `(| v C1)` (C1 a constant on
/// either side), return `C2` when `C2`'s set bits are a SUBSET of `C1`'s (`C2 & C1 == C2`) — because
/// `(v | C1) & C2` forces every bit of `C2` to 1 (they are all in `C1`, which the OR sets) and clears the
/// rest, so the result is exactly `C2`, regardless of `v`. `(& (| x 15) 15)` → 15, `(& (| x 255) 15)` →
/// 15. Returns the CONSTANT `C2` occurrence (the outer mask, `c2_occ`) to reuse as the folded value.
/// `None` when the shape does not match or `C2 ⊄ C1`. The fold DISCARDS `v`, so the caller gates it on
/// `is_trap_free(v)` — the returned `Some` reports `v` so the caller can check it.
fn or_then_mask_absorbs(db: &mut Db, inner: StructId, c2: i64) -> Option<StructId> {
    let Core::Arith {
        op: Prim::BitOr,
        lhs: il,
        rhs: ir,
    } = core_of(db, inner)
    else {
        return None;
    };
    // The inner OR's constant C1 (on either side); the other operand is `v`.
    let (v, c1) = match (core_of(db, il), core_of(db, ir)) {
        (Core::ConstInt(c), _) => (ir, c.to_i64()?),
        (_, Core::ConstInt(c)) => (il, c.to_i64()?),
        _ => return None,
    };
    // `C2 ⊆ C1` — every bit the outer mask keeps is one the inner OR already set to 1.
    if (c2 & c1) == c2 { Some(v) } else { None }
}

/// Whether masking the value at `val` with the constant `mask_core` is a NO-OP — i.e. `val & M == val`.
/// True iff `val`'s solved type is a resolved UNSIGNED integer of width `N` (`Sign::Fixed(false)` +
/// `Width::Fixed(N)`, `N < 64`) and the mask's low `N` bits are ALL set (`M & (2^N − 1) == 2^N − 1`). An
/// unsigned width-N value lives in `[0, 2^N)`, so a mask covering its whole range clears nothing. NOT
/// applied to signed types (the slot's high bits are sign extension a mask would wrongly clear) nor to
/// a 64-bit width (whose full mask `2^64−1` is not i64-representable here — and `& allbits` at 64 is a
/// separate case the `x & x` fold does not cover; skipped for simplicity, low value).
fn is_full_mask_for(db: &mut Db, val: StructId, mask_core: &Core) -> bool {
    let Core::ConstInt(m) = mask_core else {
        return false;
    };
    let Some(m) = m.to_i64() else {
        return false;
    };
    let Some(bits) = unsigned_value_bits(db, val) else {
        return false; // not a provably-nonnegative value with a known ≤63-bit range.
    };
    // `2^bits − 1` — all bits the value can possibly set. `bits` is `1..=63`; at `bits == 63` the shift
    // `1i64 << 63` is `i64::MIN`, so `− 1` would OVERFLOW in a checked build (a latent panic) — that case
    // is exactly `i64::MAX` (all 63 low bits, the whole nonneg i64 range), so special-case it.
    let low = if bits >= 63 {
        i64::MAX
    } else {
        (1i64 << bits) - 1
    };
    (m & low) == low
}

/// For a `BitAnd` at emit time, whether the constant mask on ONE side covers the WHOLE provable range of
/// the value on the other side — so `v & M == v` and the `&` is redundant. Returns the VALUE operand to
/// emit alone (`Some(v)`), or `None` when neither side is such a redundant mask. This is the EMIT-TIME
/// sibling of the `is_full_mask_for` lower fold: identical soundness (a nonneg `v ∈ [0, 2^B)` whose bits
/// `M` all covers), but it consults `value_range` HERE — where the flow-refinement stack is populated — so
/// it fires on a refined value the lower fold could not see (`(if (and (>= x 0) (< x 256)) (& x 255) …)`:
/// under the branch `x ∈ [0,255]`, `x & 255 == x`). Both operand orders are tried. The `&` is TOTAL, so
/// eliding it drops no trap; returning the value operand preserves its own evaluation (and any trap in it).
pub(crate) fn redundant_and_mask_value(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    let rc = core_of(db, rhs);
    if is_full_mask_for(db, lhs, &rc) {
        return Some(lhs); // `(& v M)` with M covering v's range → v
    }
    let lc = core_of(db, lhs);
    if is_full_mask_for(db, rhs, &lc) {
        return Some(rhs); // `(& M v)` → v
    }
    None
}

/// For a `BitOr` at emit time, whether the constant `M` on ONE side covers the WHOLE provable range of the
/// value `v` on the other side — so `v | M == M` (OR-SATURATION) and the `|` is redundant. Returns the
/// CONSTANT operand (`Some(M_occ)`) to emit alone, or `None`. The emit-time sibling of the `BitOr`
/// OR-saturation lower fold, firing on a flow-refined `v` the lower fold cannot see (`(if (and (>= x 0)
/// (< x 256)) (| x 255) …)` → `x | 255 == 255` under `x ∈ [0,255]`). DISCARDS `v` (the result is the
/// constant M), so the caller must first confirm `v` is TRAP-FREE — a trapping `v` must still trap. Both
/// operand orders tried; returns whichever operand is the covering constant.
pub(crate) fn redundant_or_mask_const(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
) -> Option<StructId> {
    // `(| v M)` — v on the left, constant M on the right covering v's range → M (rhs).
    let rc = core_of(db, rhs);
    if is_full_mask_for(db, lhs, &rc) && is_trap_free(db, lhs) {
        return Some(rhs);
    }
    // `(| M v)` — constant M on the left → M (lhs).
    let lc = core_of(db, lhs);
    if is_full_mask_for(db, rhs, &lc) && is_trap_free(db, rhs) {
        return Some(lhs);
    }
    None
}

/// Whether the dividend `val` is provably in `[0, divisor − 1]` for a positive `divisor` — so a truncating
/// `val / divisor` is `0` and `val % divisor` is `val` (the divisor is too large to divide `val` even
/// once). True iff `value_range(val)` is a NONNEGATIVE closed interval `[lo, hi]` with `lo >= 0` and
/// `hi < divisor`. Restricted to a nonnegative dividend: for `0 <= val < divisor`, both the mathematical
/// and the truncate-toward-zero results are exact (`val / divisor = 0`, `val % divisor = val`); a negative
/// dividend is excluded (its `value_range` lo is `< 0`, failing the check). `None`/unbounded range → false.
fn dividend_below_divisor(db: &mut Db, val: StructId, divisor: i64) -> bool {
    matches!(value_range(db, val), Some((lo, Some(hi))) if lo >= 0 && hi < divisor)
}

/// An upper bound (in `1..=63`) on the number of LOW bits a runtime value can set — i.e. the value is
/// provably in `[0, 2^B)`. Derived from `value_range`: a value whose range is `[0, hi]` (nonnegative,
/// `hi` a nonneg i64) fits `bits(hi)` bits. `None` when the value is not provably nonnegative or has no
/// i64 upper bound. Drives the mask-elision (`& fullmask`) and shift-out (`>>ᵤ` all bits) folds.
fn unsigned_value_bits(db: &mut Db, val: StructId) -> Option<u32> {
    match value_range(db, val) {
        // A nonnegative closed range `[0, hi]` → the significant-bit count of `hi` (≥ 1). `hi` is a
        // nonnegative i64, so `bits(hi) ∈ 1..=63`.
        Some((0, Some(hi))) if hi >= 0 => Some((64 - (hi as u64).leading_zeros()).max(1)),
        _ => None,
    }
}

/// The language WIDTH `N` of `val`'s resolved integer type — the range a shift COUNT is guarded to
/// `[0, N)`. Used by the shift-out-to-zero fold to confirm the constant count is IN-RANGE (an
/// out-of-range shift TRAPS, so it must NOT be folded to 0). `None` if the type is not a resolved
/// integer (a deferred width would guess the guard bound).
fn shift_width(db: &mut Db, val: StructId) -> u32 {
    match crate::infer::type_of(db, val) {
        crate::ty::Ty::Int(it) => match it.width {
            crate::ty::Width::Fixed(n) => n,
            _ => 0, // deferred/var — treat as 0 so the `k < width` guard fails (no fold).
        },
        _ => 0,
    }
}

/// Whether the node at `id` lowers to a core that CANNOT TRAP at run time — so discarding it (an
/// annihilator identity like `x * 0 → 0`) loses no defined trap. CONSERVATIVE: only a value with no
/// checked operation anywhere inside it. Trap-free = a leaf (constant/param/local/unit), a wrap
/// (total), or a bitwise op / conversion / projection over trap-free operands. NOT trap-free = `+`/
/// `-`/`*`/`<<`/`>>` (overflow/count guards), `/`/`%` (÷0, MIN/-1), a call (its body may trap), an
/// `if`/`match` (a branch may trap), a sum/tuple/record construct (may allocate/box — treated as
/// possibly-effecting here). Reads the operand's already-lowered core recursively.
pub(crate) fn is_trap_free(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::Unit
        | Core::Param { .. }
        | Core::LocalRef { .. } => true,
        // Bitwise ops are total; a comparison never traps — trap-free if their operands are.
        Core::Arith {
            op: Prim::BitAnd | Prim::BitOr | Prim::BitXor,
            lhs,
            rhs,
        }
        | Core::Compare { lhs, rhs, .. } => is_trap_free(db, lhs) && is_trap_free(db, rhs),
        // Boolean negation `not` is a single `i32.eqz` — total (never traps) — so trap-free if its operand
        // is. (Lets `(not a)` participate in a discarding fold, e.g. the boolean complement law
        // `(and (not a) a)` → false, whose `is_trap_free(lhs)` guard sees the `(not a)` lhs.)
        Core::Not { operand } => is_trap_free(db, operand),
        // `wrap` is total (never traps) — trap-free if its operand is.
        Core::Convert {
            op: Prim::Wrap,
            operand,
        } => is_trap_free(db, operand),
        // A COLLECTION COUNT (`List.len`/`Bytes.len`/`Map.size`/`Set.len`) is a TOTAL O(1) borrowing read —
        // it never traps — so it is trap-free when its collection operand is (a param/kept-local handle is;
        // a count of a trapping construction stays tied to that trap). This lets a length feed a discarding
        // fold (`(>= (List.len xs) 0)` → true drops the length) with its `[0, 2^32-1]` range from
        // `value_range`. The operand is the container handle for each.
        Core::ListLen { operand } | Core::BytesLen { operand } => is_trap_free(db, operand),
        Core::MapSize { map } => is_trap_free(db, map),
        Core::SetLen { set } => is_trap_free(db, set),
        // A RIGHT SHIFT by a CONSTANT in-range count (`0 <= k < width`) never traps: `>>` cannot overflow
        // (its magnitude only shrinks), and a valid constant count trips no count-guard. So it is trap-free
        // when its value operand is. (A `<<` is EXCLUDED — it is exact `·2^k` and can overflow the type, so
        // it is genuinely trapping even with a valid count. A RUNTIME count is also excluded — an
        // out-of-range count traps.) This lets a `(>> x k)` feed a discarding fold: `(< (>>ᵤ x 60) 20)` on
        // a UInt64 (range `[0,15]`) folds to `true` without keeping a bogus "shift might trap" compare.
        Core::Arith {
            op: Prim::Shr,
            lhs,
            rhs,
        } if matches!(core_of(db, rhs), Core::ConstInt(k)
                if k.to_i64().is_some_and(|k| k >= 0 && (k as u32) < shift_width(db, lhs))) =>
        {
            is_trap_free(db, lhs)
        }
        // A `/` or `%` by a CONSTANT divisor `C ∉ {0, -1}` never traps: `C != 0` rules out ÷0, and `C != -1`
        // rules out the sole signed-division overflow `MIN / -1`. So it is trap-free when its dividend is.
        // (`C == 0` is a constant-trap poison in `lower` before here; `C == -1` keeps the guard. A RUNTIME
        // divisor is excluded — it could be 0 or -1.) Lets `(< (% (& x 255) 10) 10)` fold to `true`.
        Core::Arith {
            op: Prim::Div | Prim::Rem,
            lhs,
            rhs,
        } if matches!(core_of(db, rhs), Core::ConstInt(c)
                if matches!(c.to_i64(), Some(v) if v != 0 && v != -1)) =>
        {
            is_trap_free(db, lhs)
        }
        // Everything else — checked arithmetic (+/-/*), a LEFT shift, a runtime-count/-divisor shift or
        // div/rem, calls, control flow, heap constructs, poison — is conservatively treated as possibly-
        // trapping.
        _ => false,
    }
}

/// Whether the core at `id` CONTAINS a runtime call (`Core::Call`, `CallClosure`, or `HostCall`) anywhere
/// in the positions the mutual-/self-recursion loop transform threads a TAIL call through — the node
/// itself, an `if`'s branches, a `let`'s body, a `match`'s arms. Used to VETO the `(if c a false)`→`(and c
/// a)` rewrite when the branch that would become the connective's guarded `rhs` holds a tail call: the loop
/// transform (`body_has_member_tail_call`) only follows `if`/`let`/`match` tail positions, NOT `and`/`or`,
/// so burying a tail-recursive call inside a connective would defeat tail-loop conversion (a far bigger win
/// than a branchless boolean). Conservative — descends only tail positions, matching the transform's reach;
/// a call in a non-tail operand is not a tail edge and would not be lost, but treating the whole branch as
/// call-bearing here is safe (it only forgoes the rewrite). NOT the same as `!is_trap_free`: a checked-arith
/// boolean branch (`(> (+ x 1) 5)`) is call-free, so the rewrite still fires and stays sound (its trap is
/// shielded in the connective's guarded rhs exactly as in the `if`'s branch).
fn tail_positions_have_call(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Call { .. } | Core::CallClosure { .. } | Core::HostCall { .. } => true,
        Core::If { then_, else_, .. } => {
            tail_positions_have_call(db, then_) || tail_positions_have_call(db, else_)
        }
        Core::Let { body, .. } => tail_positions_have_call(db, body),
        Core::Match { arms, .. } => arms.iter().any(|a| tail_positions_have_call(db, a.body)),
        _ => false,
    }
}

/// Whether the nodes at `a` and `b` lower to the STRUCTURALLY IDENTICAL core — the basis for folding an
/// `if` whose two branches are the same (`(if c x x)` → `x`). CONSERVATIVE: matches only PURE
/// deterministic scalar cores (const / param / local-ref leaves; arithmetic / comparison / conversion /
/// projection over recursively-equal operands), so any other core (a call, a nested `if`, a heap
/// construct) compares unequal and the `if` is left intact. Every matched kind is a value that reads the
/// same whichever branch produces it, so collapsing the two branches to one is behavior-preserving.
/// (This is the `lower`-column twin of `select::core_eq`, kept here because `lower` owns the core.)
fn core_equiv(db: &mut Db, a: StructId, b: StructId) -> bool {
    if a == b {
        return true;
    }
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => x.eq_value(&y),
        (Core::ConstBool(x), Core::ConstBool(y)) => x == y,
        (Core::Unit, Core::Unit) => true,
        (Core::Param { binder: x }, Core::Param { binder: y }) => x == y,
        (Core::LocalRef { binder: x }, Core::LocalRef { binder: y }) => x == y,
        (
            Core::Arith {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Arith {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        )
        | (
            Core::Compare {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Compare {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        ) => {
            // Base structural match: same operator, operands equal position-wise.
            let positional = ox == oy && core_equiv(db, lx, ly) && core_equiv(db, rx, ry);
            // COMMUTATIVITY of EQUALITY: `(= a b)` and `(= b a)` denote the identical boolean (equality is
            // symmetric), so accept the SWAPPED operand match too. Only `Eq` — `<`/`>`/`<=`/`>=` flip
            // direction when swapped, and this arm is shared with `Core::Arith` whose ops are never `Eq`, so
            // `ox == Eq` fires ONLY for a comparison. Guarded on both operands trap-free so the swap changes
            // no observable evaluation ORDER (a trapping operand's position could decide which trap fires
            // first; a pure operand's cannot). Lets `(and (= a b) (= b a))` fold to one `(= a b)` via the
            // idempotence path, which keys on `core_equiv`.
            positional
                || (ox == oy
                    && matches!(ox, Prim::Eq)
                    && is_trap_free(db, lx)
                    && is_trap_free(db, rx)
                    && core_equiv(db, lx, ry)
                    && core_equiv(db, rx, ly))
        }
        (
            Core::Convert {
                op: ox,
                operand: px,
            },
            Core::Convert {
                op: oy,
                operand: py,
            },
        ) => ox == oy && core_equiv(db, px, py),
        (
            Core::Proj {
                operand: px,
                index: ix,
            },
            Core::Proj {
                operand: py,
                index: iy,
            },
        ) => ix == iy && core_equiv(db, px, py),
        _ => false,
    }
}

/// The "not-yet-computed on a runtime string" DECLINE for a string operation whose `arg` did not fold
/// to a constant — BUT only when `arg` is actually a `String`. When `arg` is NOT a string (`(Symbol.of
/// 5)` — a type error `infer` already reports as CDZ0203), the "runtime string" wording is a lie that
/// shadows the real type error; emit a NEUTRAL decline instead so the coded CDZ0203 is the story the
/// reader sees. (A genuine runtime string keeps the precise `msg`, the honest "constant strings only"
/// increment note.)
fn runtime_string_op_decline(db: &mut Db, arg: crate::ast::StructId, msg: &str) -> Core {
    if matches!(crate::infer::type_of(db, arg), crate::ty::Ty::String) {
        Core::Poison(Reject::decline(msg.to_string()))
    } else {
        // Not a string — the type mismatch (CDZ0203) is the authoritative report; do not claim a
        // "runtime string" it is not. This decline is generic (a lowering can't proceed on an
        // ill-typed operand) and defers to the coded type error.
        Core::Poison(Reject::decline(
            "this operation's operand is not a string (see the type error above)",
        ))
    }
}

/// Fold a constant arithmetic operation with a CHECKED evaluation. Both operands are compile-time
/// constants; if the operation's defined outcome on them is a trap (an overflow the checked default
/// forbids, or an operand outside the machine range the fold evaluates over), the result is a poison
/// carrying CDZ0304 — the build fails rather than shipping a runtime trap. On success the result is a
/// `ConstInt`. The evaluation is over `i64` (the Stage default integer); a later width stage
/// generalizes the range the check tests to the operands' solved width.
fn fold_arith(op: Prim, a: IntValue, b: IntValue) -> Core {
    let (x, y) = match (a.to_i64(), b.to_i64()) {
        (Some(x), Some(y)) => (x, y),
        // An operand beyond the machine range the fold evaluates over — a provable width trap.
        _ => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "constant operand does not fit the integer width",
            ));
        }
    };
    // Each integer op evaluates over `i64` (the Stage default width) with the DEFINED numeric-model
    // semantics; `None` marks a provable trap the checked default forbids (`numeric-model.md` §Overflow
    // Is Defined). A later width stage generalizes the range/count the checks test to the solved width.
    let checked = match op {
        Prim::Add => x.checked_add(y),
        Prim::Sub => x.checked_sub(y),
        Prim::Mul => x.checked_mul(y),
        // Division truncates toward zero; traps on a zero divisor and on `MIN / -1` (Rust's
        // `checked_div` returns `None` for both — exactly the two defined traps).
        Prim::Div => x.checked_div(y),
        // Remainder takes the dividend's sign; traps on a zero divisor. `MIN % -1` is 0 (no overflow),
        // but Rust's `%` panics there — `checked_rem` returns `None`, so special-case it to 0.
        Prim::Rem => {
            if y == -1 {
                Some(0)
            } else {
                x.checked_rem(y)
            }
        }
        // A left shift is exact multiplication by `2^count`: it traps on an out-of-range count
        // (< 0 or ≥ width) AND on overflow past the width — NOT wasm's silent mask-and-wrap.
        Prim::Shl => checked_shl_i64(x, y),
        // Arithmetic (sign-extending) right shift; traps on an out-of-range count, never overflows.
        Prim::Shr => checked_shr_i64(x, y),
        // Bitwise operations are total on the two's-complement value — never trap.
        Prim::BitAnd => Some(x & y),
        Prim::BitOr => Some(x | y),
        Prim::BitXor => Some(x ^ y),
        // A non-integer-binary prim never reaches the fold (`lower_arith` is only called for an
        // `is_arith` prim), so these arms are unreachable in practice; decline rather than panic.
        Prim::Lt
        | Prim::Gt
        | Prim::Le
        | Prim::Ge
        | Prim::Eq
        | Prim::Compare
        | Prim::Wrap
        | Prim::CheckedOf
        | Prim::IntCtor
        | Prim::UIntCtor
        | Prim::FnCtor
        | Prim::TupleCtor
        | Prim::RecordCtor
        | Prim::BoolTy
        | Prim::UnitTy
        | Prim::SumNew
        | Prim::SumCtor
        | Prim::TupleNew
        | Prim::RecordNew
        | Prim::RecordProject
        | Prim::RecordWithout
        | Prim::RecordMerge
        | Prim::RecordExtend
        | Prim::RecordWith
        | Prim::RecordPop
        | Prim::TupleCat
        | Prim::TupleSplitAt
        | Prim::TuplePop
        | Prim::ListNew
        | Prim::ListLen
        | Prim::ListPush
        | Prim::ListConcat
        | Prim::ListUpdate
        | Prim::ListAt
        | Prim::ListCtor
        | Prim::BytesOf
        | Prim::BytesLen
        | Prim::BytesTy
        | Prim::StrScalarLen
        | Prim::StrByteLen
        | Prim::StrAt
        | Prim::StrScalarAt
        | Prim::StrConcat
        | Prim::StrSlice
        | Prim::StrToBytes
        | Prim::StrFromBytes
        | Prim::SumExpect
        | Prim::CheckedAdd
        | Prim::CheckedMul
        | Prim::WrappingAdd
        | Prim::WrappingMul
        | Prim::StringTy
        | Prim::BytesAt
        | Prim::BytesConcat
        | Prim::BytesSlice
        | Prim::BytesCompact
        // Float arithmetic is folded by `lower_float_arith` (an f64/f32 fold), not this integer fold.
        | Prim::FAdd
        | Prim::FSub
        | Prim::FMul
        | Prim::FDiv
        | Prim::FloatCtor
        | Prim::FloatOfInt
        | Prim::FloatOf
        | Prim::FloatNan
        | Prim::MapCtor
        | Prim::MapNew
        | Prim::MapEmpty
        | Prim::MapInsert
        | Prim::MapLookup
        | Prim::MapRemove
        | Prim::MapSize
        | Prim::MapSwap
        | Prim::MapTake
        | Prim::SetCtor
        | Prim::SetOf
        | Prim::SetContains
        | Prim::SetLen
        | Prim::SetInsert
        | Prim::SetRemove
        | Prim::SetUnion
        | Prim::SetIntersection
        | Prim::SetDifference
        | Prim::CharTy
        | Prim::CharToInt
        | Prim::CharFromInt
        | Prim::SymbolTy
        | Prim::SymbolOf
        | Prim::SymbolToString
        // `BigIntTy` is a ground type-value builder (bare `BigInt` in type position → `Ty::BigInt`),
        // and `BigIntOf` is the unary widening conversion (folds in its own arm above) — neither is an
        // integer BINARY operation, like `StringTy`/`SymbolTy`/`SymbolOf`.
        | Prim::BigIntTy
        | Prim::BigIntOf
        // The unit/quantity prims are compile-time unit builders / erasing quantity ops — never an
        // integer binary operation (a `Qty.of`/`Qty.value` lowers to its value argument, a unit builder
        // is reduced away by `eval`), so they never reach this integer fold.
        | Prim::UnitOne
        | Prim::UnitBase
        | Prim::UnitMul
        | Prim::UnitDiv
        | Prim::UnitPow
        | Prim::UnitPrefix
        | Prim::UnitOf
        | Prim::UnitDefine
        | Prim::UnitIn
        | Prim::QtyOf
        | Prim::QtyValue
        | Prim::QtyPow
        | Prim::QtyUnit
        | Prim::QtyCtor
        | Prim::TypeOf
        | Prim::TypeEq
        // `trap` is the diverging primitive (lowered to `Core::Trap`), never an integer binary operation.
        | Prim::Trap => {
            return Core::Poison(Reject::decline("not an integer binary operation"));
        }
    };
    match checked {
        Some(n) => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = n, "folded constant integer op");
            Core::ConstInt(IntValue::from_i64(n))
        }
        // A provable trap — the checked default traps, and the compiler can prove it, so the build
        // fails (CDZ0304) rather than emitting a component that traps (`numeric-model.md` §A Constant
        // Operation With No Value Is Rejected At Compile Time).
        None => {
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), "constant op traps → CDZ0304 (fails build)");
            Core::Poison(Reject::coded(
                Code::ConstTrap,
                format!(
                    "constant {} traps: {}",
                    intrinsic_name(op),
                    const_trap_cause(op, y),
                ),
            ))
        }
    }
}

/// The SPECIFIC cause of a provable constant-integer trap (CDZ0304) for op `op` with right operand `y`
/// — the compiler proved the trap, so it knows which of the three defined causes fired and names THAT
/// ("divide by zero" / "overflows Int64" / "shift count N out of range 0..64") rather than listing all
/// three. The evaluation is over the Stage-default `i64`, so overflow is against `Int64`; a later width
/// stage would name the solved width. Only called when the fold returned `None` (a genuine trap), so a
/// default arm is a defensive fallback, not a reachable case. (The left operand doesn't disambiguate a
/// cause — every trap is decided by the op and the divisor/shift-count `y`.)
fn const_trap_cause(op: Prim, y: i64) -> String {
    match op {
        Prim::Div | Prim::Rem if y == 0 => "divide by zero".to_string(),
        // The only non-zero-divisor `Div` trap is `Int64.min / -1` (the quotient overflows Int64).
        Prim::Div => "the quotient overflows Int64 (Int64.min / -1)".to_string(),
        Prim::Add | Prim::Sub | Prim::Mul => "the result overflows Int64".to_string(),
        Prim::Shl | Prim::Shr => {
            if !(0..64).contains(&y) {
                format!("shift count {y} is out of range 0..64")
            } else {
                // An in-range Shl whose exact result overflows the width.
                "the shifted result overflows Int64".to_string()
            }
        }
        _ => "the operation has no defined value on these operands".to_string(),
    }
}

/// A left shift as EXACT multiplication by `2^count`: `None` (a provable trap) if the count is outside
/// `0..64` or the exact result overflows `i64` — a left shift is not exempt from Overflow Is Defined,
/// so it traps like `*` rather than masking the count and wrapping (`numeric-model.md`).
fn checked_shl_i64(x: i64, count: i64) -> Option<i64> {
    if !(0..64).contains(&count) {
        return None;
    }
    // Multiply by 2^count and narrow to `i64`, `None` on overflow — the defined meaning of a left
    // shift. The product is computed in `i128` because the `2^count` factor is itself not always an
    // `i64`: `1i64 << 63` is `i64::MIN` (a NEGATIVE 2^63), so a signed factor miscomputes both
    // `1 << 63` (folds to `i64::MIN` instead of overflowing) and `-1 << 63` (overflows the signed
    // multiply instead of yielding `i64::MIN`). In `i128`, `2^count` (count < 64) and its product
    // with any `i64` both fit exactly, so the single `i64::try_from` fit-check is the whole rule.
    i64::try_from((x as i128) << count).ok()
}

/// An ARITHMETIC (sign-extending) right shift: `None` if the count is outside `0..64` (an out-of-range
/// count traps rather than masking). Never overflows. The signed shift preserves the sign bit, so
/// shifting a negative value right fills with ones (e.g. `-256 >> 7 = -2`).
fn checked_shr_i64(x: i64, count: i64) -> Option<i64> {
    if !(0..64).contains(&count) {
        return None;
    }
    Some(x >> count)
}

/// Lower a COMPARISON application (`< > <= >= =`). Folds two constant SCALARS (integers or booleans) to
/// a `ConstBool` — a total ordering on the scalar's value. A RUNTIME scalar operand (a function
/// parameter) becomes a `Core::Compare` the backend emits as a machine comparison. A COMPOUND operand
/// (a record/heap value) still declines — structural comparison over the value heap is a later stage.
/// The operator's type stays fully generic (`∀a. a → a → Bool`). A poison operand propagates.
/// The Less/Equal/Greater discriminants of the built-in `Ordering` sum (this node's solved result type),
/// read off the declaration by variant NAME (not baked) — the `Ordering` analogue of `option_discs`.
fn ordering_discs(db: &mut Db, id: StructId) -> Option<(u32, u32, u32)> {
    let crate::ty::Ty::Sum { decl, .. } = crate::infer::type_of(db, id) else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(decl)?;
    let (mut lt, mut eq, mut gt) = (None, None, None);
    for (i, v) in decl_ref.variants.iter().enumerate() {
        match v.name.as_str() {
            "Less" => lt = Some(i as u32),
            "Equal" => eq = Some(i as u32),
            "Greater" => gt = Some(i as u32),
            _ => {}
        }
    }
    Some((lt?, eq?, gt?))
}

/// Lower `(compare a b)` — the three-way comparison yielding an `Ordering` (core-semantics.md §A Total
/// Order Is Observed Through A Three-Way Comparison). FOLD a constant scalar/string operand pair to the
/// matching `Ordering` variant — `Less`/`Equal`/`Greater` by the operands' `cmp`, built as a NULLARY
/// `Core::SumNew` at the result Ordering's discs (`ordering_discs`), so it rides the ordinary sum
/// fold/escape/match exactly as a variant constructor does. A float pair compares by canonical value
/// (IEEE partial order; a NaN pair — unordered — declines). A compound or runtime operand declines (the
/// heap-walk / runtime three-way compare is a later stage), mirroring `lower_comparison`.
///
/// The order each scalar type offers is TOTAL and a deterministic function of the values: Int by numeric
/// value, Char by scalar value, String lexicographically, and Bool by `false < true` (Rust `bool::cmp`).
/// Every ordered pair folds to exactly one of Less/Equal/Greater, and the fold reads only the operand
/// values — no environment, order, or outside influence enters.
//= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
//# A type that offers an ordering MUST offer a total order over its values.
//= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
//# The ordering a type offers MUST be a deterministic function of the values compared.
//= spec/capabilities/core-semantics.md#ordering-where-offered-is-total
//# The Bool type MUST offer a total order in which false is less than true.
fn lower_compare(db: &mut Db, id: StructId, lhs: StructId, rhs: StructId) -> Core {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let Some((lt, eq, gt)) = ordering_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "compare result is not the built-in Ordering sum",
        ));
    };
    // The ordering of the two constant operands, or `None` to decline (non-constant / non-scalar / a NaN
    // pair / an operand beyond the fold's machine range).
    let ord = match (core_of(db, lhs), core_of(db, rhs)) {
        (Core::ConstInt(a), Core::ConstInt(b)) => match (a.to_i64(), b.to_i64()) {
            (Some(x), Some(y)) => Some(x.cmp(&y)),
            _ => None,
        },
        (Core::ConstBool(a), Core::ConstBool(b)) => Some(a.cmp(&b)),
        (Core::ConstStr(a), Core::ConstStr(b)) => Some(a.cmp(&b)),
        // Two chars order by scalar value (`compare #\a #\b` → Less).
        (Core::ConstChar(a), Core::ConstChar(b)) => Some((a as u32).cmp(&(b as u32))),
        (Core::ConstFloat(a), Core::ConstFloat(b)) => {
            f64::from_bits(a.to_f64_bits()).partial_cmp(&f64::from_bits(b.to_f64_bits()))
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => return Core::Poison(r),
        _ => None,
    };
    match ord {
        Some(o) => {
            let disc = match o {
                Less => lt,
                Equal => eq,
                Greater => gt,
            };
            trace!(target: "rcdzc::fold", node = id.0, ?o, "compare folds to an Ordering variant");
            Core::SumNew {
                disc,
                payloads: Vec::new(),
            }
        }
        None => Core::Poison(Reject::decline(
            "compare of a runtime/compound operand (or a NaN pair) is not yet computed (constant scalars only)",
        )),
    }
}

fn lower_comparison(db: &mut Db, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(binop_arity_reject(op, args));
    }
    // A comparison over BIGINT operands routes through the runtime `bigint-cmp` (B3c) — a `BigInt` has no
    // fixed machine slot, so `is_scalar` is false and the plain scalar-compare path below never fires. A
    // CONSTANT pair still folds (both reach as `Core::ConstInt` carrying the exact `IntValue`, compared by
    // `lower_bigint_cmp` at 128-bit precision where it fits, else the runtime op); a runtime operand emits
    // `Core::BigIntCmp` (`bigint-cmp` + a fixed compare-with-zero). A `BigInt`/fixed mix was rejected
    // CDZ0301 in `check_application`, so if one operand is BigInt the other is too. Checked before the
    // constant folds below (a BigInt `ConstInt`'s value can exceed i64, so the `to_i64` fold would decline).
    if bigint_operand(db, args) {
        return lower_bigint_cmp(db, op, args[0], args[1]);
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::ConstInt(a), Core::ConstInt(b)) => match (a.to_i64(), b.to_i64()) {
            (Some(x), Some(y)) => {
                let r = compare_ord(op, x.cmp(&y));
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant integer comparison");
                Core::ConstBool(r)
            }
            // An operand exceeds `i64` range — an UNSIGNED value at/above `2^63` (`UInt64.max = 2^64-1`
            // does not fit `i64`). Compare by the TRUE numeric value at 128-bit precision: `to_i128`
            // reads the exact value (any ≤128-bit operand, unsigned or signed), and comparing the true
            // values is correct for BOTH signednesses — an unsigned value is non-negative, a signed one
            // carries its sign, so a naive i64-bit-pattern compare (where `UInt64.max` looks like `-1`)
            // is avoided. This folds `(< (: 0 UInt64) (. UInt64 max))` → true (numeric-model.md §Unsigned
            // Comparison Orders By Magnitude). A value wider than 128 bits (a BigInt) still declines.
            _ => match (a.to_i128(), b.to_i128()) {
                (Some(x), Some(y)) => {
                    let r = compare_ord(op, x.cmp(&y));
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant integer comparison (i128, wide unsigned)");
                    Core::ConstBool(r)
                }
                _ => Core::Poison(Reject::decline(
                    "comparison of an integer beyond the machine width is not yet folded",
                )),
            },
        },
        (Core::ConstBool(a), Core::ConstBool(b)) => {
            let r = compare_ord(op, a.cmp(&b));
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant boolean comparison");
            Core::ConstBool(r)
        }
        // Two CONSTANT strings compare by their text (lexicographic by Unicode scalar values — the byte
        // order of NFC UTF-8, which the reader already normalized to). `(= "a" "a")` → true; ordering
        // comparisons (`<`) order by text. A constant fold, no heap: the string equality the compiler
        // needs for tag/name dispatch.
        (Core::ConstStr(a), Core::ConstStr(b)) => {
            let r = compare_ord(op, a.cmp(&b));
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant string comparison");
            Core::ConstBool(r)
        }
        // Two CONSTANT chars compare by SCALAR VALUE (`collections-and-text.md` §A Char Is A Single
        // Unicode Scalar Value: "a char's ordering MUST be the numeric order of its scalar value"), so `=`
        // is scalar equality and `<`/`>` order by code point — `(= #\a #\a)` → true, `(< #\a #\b)` → true
        // (97 < 98). A constant fold, no runtime (a char has no machine slot this increment).
        (Core::ConstChar(a), Core::ConstChar(b)) => {
            let r = compare_ord(op, (a as u32).cmp(&(b as u32)));
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant char comparison");
            Core::ConstBool(r)
        }
        // NaN under the CANONICAL BYTE FORM: every NaN shares one canonical byte form, so `(= nan nan)`
        // is TRUE (NOT IEEE `f64.eq`, which says nan≠nan) and `nan` is UNEQUAL to any finite float
        // (distinct byte forms). Ordering (`<`/`>`) against a NaN is undefined (unordered) — decline, as
        // the finite-pair path does for a NaN operand. Handled BEFORE the finite `ConstFloat` pair. (A
        // negative zero is likewise a distinct byte form from positive zero, so `(= -0.0 0.0)` is false —
        // the `ConstFloat` pair compares the canonical decimal, which carries the sign.)
        //= spec/capabilities/core-semantics.md#floating-point-equality-follows-the-canonical-byte-form
        //# A floating-point value MUST be equal to another floating-point value exactly when their canonical byte forms are identical, so that a negative zero is distinct from a positive zero and all not-a-number values are equal to one another.
        (Core::ConstFloatNan, Core::ConstFloatNan) => {
            if matches!(op, Prim::Eq) {
                trace!(target: "rcdzc::fold", "folded nan = nan → true (canonical byte form)");
                Core::ConstBool(true)
            } else {
                Core::Poison(Reject::decline(
                    "an ordering comparison with a NaN operand has no defined result",
                ))
            }
        }
        (Core::ConstFloatNan, Core::ConstFloat(_)) | (Core::ConstFloat(_), Core::ConstFloatNan) => {
            if matches!(op, Prim::Eq) {
                trace!(target: "rcdzc::fold", "folded nan = finite → false (distinct byte forms)");
                Core::ConstBool(false)
            } else {
                Core::Poison(Reject::decline(
                    "an ordering comparison with a NaN operand has no defined result",
                ))
            }
        }
        // Two CONSTANT floats compare by their canonical Float64 value (contracts/deterministic-value-
        // form.md #Numeric Values Serialize Deterministically — floats equal under structural equality
        // share a canonical form, distinct floats have distinct forms). EQUALITY (`=`) is by RAW BITS, so
        // `-0.0 ≠ 0.0` (distinct bit patterns → the canonical form distinguishes them) and a NaN is
        // unequal to itself. `1e19` and `1e20` round to different doubles → unequal. Ordering (`<`/`>`)
        // uses the IEEE partial order (`f64::partial_cmp`); an unordered pair (NaN) declines rather than
        // inventing a total order. Only the fold — no float runtime is needed for a Bool result.
        (Core::ConstFloat(a), Core::ConstFloat(b)) => {
            let (ba, bb) = (a.to_f64_bits(), b.to_f64_bits());
            if matches!(op, Prim::Eq) {
                let r = ba == bb;
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant float equality (by canonical bits)");
                Core::ConstBool(r)
            } else {
                match f64::from_bits(ba).partial_cmp(&f64::from_bits(bb)) {
                    Some(ord) => {
                        let r = compare_ord(op, ord);
                        trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded constant float comparison");
                        Core::ConstBool(r)
                    }
                    // An unordered pair (a NaN operand) has no defined `<`/`>` result — decline.
                    None => Core::Poison(Reject::decline(
                        "an ordering comparison with a NaN operand has no defined result",
                    )),
                }
            }
        }
        // Two UNIT values — there is exactly ONE unit value, so two units always compare EQUAL. Fold at
        // compile time to the ordering-`Equal` result for the operator (`= unit ()` → true, `< unit ()`
        // → false, `<= unit ()` → true). No heap walk and no runtime op: unit carries no data to
        // compare (it has no machine slot — `valtype_of(Ty::Unit)` is `None`), so `(= unit ())` is not a
        // "compound needs a heap walk" case but a trivial constant. (`unit` and `()` are the same value —
        // core-semantics.md #Unit And The Empty Tuple Are The Same Value.)
        (Core::Unit, Core::Unit) => {
            let r = compare_ord(op, std::cmp::Ordering::Equal);
            trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "folded unit comparison (two units are equal)");
            Core::ConstBool(r)
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // A non-constant operand: a runtime comparison IF both operands are scalars (integers or
        // booleans, which have a machine representation the backend can compare); a compound operand
        // still declines (heap-walk equality is a later stage).
        _ => {
            // CONSTANT COMPOUND EQUALITY folds STRUCTURALLY: two values are equal when they have the same
            // type and their contents are equal component-wise. Only for `=` (a total ordering `<`/`>`
            // over compounds is a later stage); only when BOTH operands are compile-time-visible constant
            // compounds (a `SumNew`/`Tuple`/`Record`/`ListNew`, recursively) — a runtime operand still
            // needs the heap walk (`value-eq`/`champ_eq`, deferred to the backend).
            //= spec/capabilities/core-semantics.md#equality-is-structural
            //# Two values MUST be equal when they have the same type and their contents are equal component-wise.
            // This component-wise fold agrees with the canonical byte form: two constant compounds are equal
            // exactly when their canonical forms coincide (a scalar leaf compares by its canonical value, a
            // nested compound recurses), so structural equality and byte-form identity never disagree.
            //= spec/capabilities/core-semantics.md#equality-is-structural
            //# Value equality MUST agree with the canonical byte form, so that two values are equal exactly when their canonical byte forms are identical.
            // `(= (Some 1) (Some 1))` → true, `(= (Some 1) (Some 2))` → false, `(= None None)` → true,
            // `(= (tuple 1 2) (tuple 1 2))` → true. A nested compound compares recursively (a payload/
            // element that is itself a compound). Returns `None` when either side is not a constant
            // compound → falls through to the scalar-runtime / decline below.
            if matches!(op, Prim::Eq)
                && let Some(eq) = const_compound_eq(db, args[0], args[1])
            {
                trace!(target: "rcdzc::fold", result = eq, "folded constant compound equality (structural)");
                return Core::ConstBool(eq);
            }
            // BOOL-INT EQUALITY: `(= (if c 1 0) K)` / `(= (if c 0 1) K)` with `K` ∈ {0,1} — the `if` is a
            // bool coerced to an int (0/1), so comparing it to 0/1 is just the condition or its negation:
            // `(if c 1 0) == 1` → `c`, `== 0` → `!c`; `(if c 0 1)` is the mirror. Folds away the whole
            // materialize-then-compare (`lt_s ; extend ; const 1 ; eq` → `lt_s`). Only for `Eq`, only a
            // 0/1 constant against a `(if c <1> <0>)`-shaped bool-int (either operand order). Reuses `c`'s
            // occurrence (no synthesis); `!c` is `Core::Not`. Verified value-identical over c ∈ {T,F}.
            if matches!(op, Prim::Eq)
                && let Some(folded) = fold_bool_int_eq(db, args[0], args[1])
            {
                trace!(target: "rcdzc::fold", "(= (if c 1 0) 0/1) folds to c / !c");
                return folded;
            }
            // BOOL-CONST EQUALITY: `(= c true)` → `c`, `(= c false)` → `!c` (either order) — a bool compared
            // to a bool literal is itself / its negation, dropping the `i32.const K ; i32.eq`.
            if matches!(op, Prim::Eq)
                && let Some(folded) = fold_bool_const_eq(db, args[0], args[1])
            {
                trace!(target: "rcdzc::fold", "(= c true/false) folds to c / !c");
                return folded;
            }
            if is_scalar(db, args[0]) && is_scalar(db, args[1]) {
                // SELF-COMPARISON: the two operands are the SAME value (`core_equiv`), so the ordering is
                // fixed regardless of what that value is — `x < x`/`x > x` → false, `x <= x`/`x >= x`/`x =
                // x` → true. Sound ONLY for a TOTAL order and a TRAP-FREE operand: `is_scalar` is Int/Bool
                // (a total order — Float, where `NaN < NaN` etc. is false and `NaN = NaN` is false, is NOT
                // scalar and never reaches here), and the operand must be trap-free since the fold DISCARDS
                // it — `(< (/ a b) (/ a b))` must still trap on b==0 (`core_equiv` matches pure cores, but a
                // matched pure compare/arith can still wrap a trapping `/`). `compare_ord` gives the result
                // for each operator at `Ordering::Equal`.
                if core_equiv(db, args[0], args[1]) && is_trap_free(db, args[0]) {
                    let r = compare_ord(op, std::cmp::Ordering::Equal);
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), result = r, "self-comparison folds to a constant (x is a total-order scalar)");
                    return Core::ConstBool(r);
                }
                // TYPE-BOUND simplification: a comparison of a runtime integer against a constant AT its
                // own type's min/max is (partly) decidable — `v < min`/`v > max` are unsatisfiable, `v >=
                // min`/`v <= max` are tautologies, and `v <= min`/`v >= max` rewrite to `v == bound` (the
                // backend selects `eqz` when the bound is 0). This subsumes the unsigned-vs-0 case (an
                // unsigned type's `min` is 0) and adds signed narrow (`Int8`'s `[-128,127]`) and full-width
                // bounds. Only fires when the runtime operand's type is fully resolved (Fixed sign+width).
                if let Some(folded) = fold_comparison_at_type_bound(db, op, args[0], args[1]) {
                    return folded;
                }
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "comparison stays runtime (scalar operands)");
                Core::Compare {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                }
            } else if matches!(op, Prim::Eq) && node_ty_is_enum_disc(db, args[0]) {
                // ENUM-DISCRIMINANT equality: both operands are bare discriminant i32s (an all-nullary
                // enum is represented as its discriminant, no heap box), so `=` is a plain `i32.eq` — NOT
                // a `value-eq` heap walk (which would misread a small discriminant as a tagged immediate
                // handle). Route to `Core::Compare`, whose backend emits `i32.eq` for an enum-disc operand
                // (`operand_int_ty` widths it as i32). Only equality — an enum has no order, so `<`/`>`
                // never reach here (they take the `is_scalar` path above, false for a sum, then decline).
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "enum-discriminant equality → i32.eq compare");
                Core::Compare {
                    op,
                    lhs: args[0],
                    rhs: args[1],
                }
            } else if matches!(op, Prim::Eq) && compound_eq_heap_walkable(db, args[0]) {
                // RUNTIME STRUCTURAL EQUALITY — a `=` on two COMPOUND heap values neither of which folded
                // (a sum/tuple/record built from a parameter or a recursive call). Emit a `value-eq`
                // runtime call (the tagless `champ_eq` walk): equal iff same shape + component-wise equal
                // (core-semantics.md §Equality Is Structural). Restricted to a value whose leaves are all
                // SCALAR (Int/Bool/Unit) — such a value is canonical BY CONSTRUCTION (no embedded RRB
                // vector / CHAMP map / Bytes rope, whose byte form is canonical only after a compaction
                // the compiler would first have to emit), so the walk's result is exact. A compound with a
                // collection/bytes leaf still declines (that canonicalization is a later increment). The
                // type checker already unified the two operands' types before lowering, so a single-side
                // walkability check suffices — both sides share the shape.
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "runtime structural equality → value-eq heap walk");
                Core::ValueEq {
                    lhs: args[0],
                    rhs: args[1],
                }
            } else {
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), "decline: comparison of a compound value needs a heap walk");
                Core::Poison(Reject::decline(
                    "comparison of a compound value needs a heap walk (not yet built)",
                ))
            }
        }
    }
}

/// Fold `(= <bool-int-if> K)` where `K` ∈ {0,1} and the other operand is a `(if c 1 0)` / `(if c 0 1)`
/// (a boolean coerced to an integer). Returns `c` when the `if`-value equals `K`, `!c` (`Core::Not`) when
/// it is the complement — dropping the materialize-then-compare. `None` when neither operand is a 0/1
/// constant, or the other is not a `(if c <one> <zero>)`-shaped bool-int, so the caller keeps the runtime
/// compare. Value-identical: `(if c 1 0)` is `1` iff `c` (so `== 1` is `c`, `== 0` is `!c`); `(if c 0 1)`
/// is the mirror. `c` is trap-free by construction (it was an `if` condition, a Bool) and is REUSED, so
/// no synthesis and no dropped trap. Only `Eq` (equality) — an ordering `<`/`>` against 0/1 is handled by
/// the range fold (`[0,1]` vs a bound).
fn fold_bool_int_eq(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<Core> {
    // (the bool-int `if` node, the 0/1 constant K) in either operand order.
    let as_const01 = |db: &mut Db, id: StructId| -> Option<i64> {
        match core_of(db, id) {
            Core::ConstInt(v) => v.to_i64().filter(|&k| k == 0 || k == 1),
            _ => None,
        }
    };
    let (if_node, k) = if let Some(k) = as_const01(db, rhs) {
        (lhs, k)
    } else if let Some(k) = as_const01(db, lhs) {
        (rhs, k)
    } else {
        return None;
    };
    // The `if` must be `(if c <then> <else>)` with the branches the integer constants 1 and 0.
    let Core::If { cond, then_, else_ } = core_of(db, if_node) else {
        return None;
    };
    let branch_val = |db: &mut Db, b: StructId| -> Option<i64> {
        match core_of(db, b) {
            Core::ConstInt(v) => v.to_i64().filter(|&x| x == 0 || x == 1),
            _ => None,
        }
    };
    let (t, e) = (branch_val(db, then_)?, branch_val(db, else_)?);
    // `then`/`else` must be the two distinct values {1,0}. The `if`-value equals `t` when `c`, else `e`.
    if !((t == 1 && e == 0) || (t == 0 && e == 1)) {
        return None;
    }
    // `(if c t e) == k`: true exactly when the SELECTED branch equals k. Since one branch is 1 and the
    // other 0, `== k` picks out the condition polarity: it holds under `c` iff `t == k`.
    // t==k → the value equals k precisely when c holds → fold to `c`; else → `!c`.
    if t == k {
        Some(core_of(db, cond)) // (= (if c 1 0) 1) → c ;  (= (if c 0 1) 0) → c
    } else {
        Some(Core::Not { operand: cond }) // (= (if c 1 0) 0) → !c ; (= (if c 0 1) 1) → !c
    }
}

/// Fold a BOOLEAN EQUALITY against a boolean LITERAL: `(= c true)` → `c`, `(= c false)` → `(not c)` (and
/// the mirrored operand order). A boolean compared to a constant boolean IS that boolean (compared to
/// `true`) or its negation (compared to `false`) — dropping the redundant `i32.const K ; i32.eq` the
/// runtime `Core::Compare` would emit. Returns the runtime operand's core (`== true`) or `Core::Not` of it
/// (`== false`); `None` unless exactly one operand is a constant `Bool` and the OTHER a runtime `Bool`
/// (a `ConstBool`/`ConstBool` pair already folded in the caller's earlier arm; a NON-`Bool` operand is not
/// this fold). The runtime operand is REUSED (its evaluation/traps preserved — no synthesis, no dropped
/// trap; the discarded operand is a constant, trivially trap-free). Only `Eq` — a `Bool` has no order, so
/// `<`/`>` never reach here (the caller routes them past `is_scalar` then declines for a non-total order).
fn fold_bool_const_eq(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<Core> {
    // One side a constant bool `k`, the other a RUNTIME bool `v` (not itself a constant — a const/const
    // pair folded earlier). `v` must be Bool-typed so the result is the operand as-is / negated.
    let as_const_bool = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstBool(b) => Some(b),
        _ => None,
    };
    let (v, k) = if let Some(k) = as_const_bool(db, rhs) {
        (lhs, k)
    } else if let Some(k) = as_const_bool(db, lhs) {
        (rhs, k)
    } else {
        return None;
    };
    // The runtime operand must be a Bool (its machine value is the 0/1 the fold returns directly). A
    // constant `v` would already have folded via the caller's `ConstBool`/`ConstBool` arm.
    if !matches!(crate::infer::type_of(db, v), crate::ty::Ty::Bool) {
        return None;
    }
    if k {
        Some(core_of(db, v)) // (= c true) → c
    } else {
        Some(Core::Not { operand: v }) // (= c false) → !c
    }
}

/// The inclusive bounds a resolved integer type occupies, as `(min, max)` where each is `Some` only if it
/// fits an `i64`. Returns `None` (skip the whole type) if the sign or width is not yet `Fixed` (a
/// deferred/variable operand must NOT be folded — its bounds are a guess, and a deferred sign grounds to
/// SIGNED where the range differs). Signed `N` holds `[-2^(N-1), 2^(N-1)-1]` (both fit i64 for `N <= 64`);
/// unsigned `N` holds `[0, 2^N - 1]` — the min `0` always fits, but the max `2^64 - 1` at `N == 64` does
/// NOT, so that bound is `None` (a comparison against it stays a runtime compare, out of i64 reach here).
fn resolved_int_bounds(it: crate::ty::IntTy) -> Option<(Option<i64>, Option<i64>)> {
    let crate::ty::Sign::Fixed(signed) = it.sign else {
        return None;
    };
    let crate::ty::Width::Fixed(w) = it.width else {
        return None;
    };
    if signed {
        let half = 1i64 << (w - 1); // w <= 64, so 2^(w-1) <= 2^63 fits i64 (as `1<<63` = i64::MIN's magnitude)
        Some((Some(half.wrapping_neg()), Some(half.wrapping_sub(1)))) // w=64: (i64::MIN, i64::MAX)
    } else if w >= 64 {
        Some((Some(0), None)) // unsigned 64: min 0 folds; max 2^64-1 is not i64-representable → None
    } else {
        // unsigned `w < 64`: max is `2^w − 1`, which fits i64. At `w == 63` the shift `1i64 << 63` is
        // `i64::MIN`, so `− 1` would OVERFLOW in a checked build — `2^63 − 1` IS `i64::MAX`, so use it
        // directly (a `UInt63`'s max); every `w ≤ 62` uses the shift.
        let max = if w == 63 { i64::MAX } else { (1i64 << w) - 1 };
        Some((Some(0), Some(max)))
    }
}

/// Simplify an ordering comparison of a runtime scalar against a constant that sits at (or beyond) the
/// operand's OWN type bound, exploiting the type's domain `[min, max]`:
///  - `v < min` → `false`, `v >= min` → `true`, `v <= min` → `v == min`;
///  - `v > max` → `false`, `v <= max` → `true`, `v >= max` → `v == max`.
///
/// This subsumes the unsigned-vs-0 case (`min = 0` for an unsigned type) and adds signed narrow bounds
/// (`Int8`'s `[-128,127]`) and full-width (`Int64`'s `[i64::MIN, i64::MAX]`). `v < max` / `v > min` are
/// NOT decidable (the value can be anywhere in range) and are left as the native compare. The `v == bound`
/// rewrite emits a `Core::Compare Eq`, which the backend selects to `eqz` when the bound is 0.
///
/// Returns `None` unless exactly one operand is a constant EQUAL to the other operand's resolved-type min
/// or max (a `Sign::Fixed` + `Width::Fixed` integer — a deferred/variable type is not folded). `Prim::Eq`
/// is excluded (equality against a bound is not a tautology and its `= 0` is already the `eqz` peephole).
fn fold_comparison_at_type_bound(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<Core> {
    let const_val = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // Identify (runtime operand `v`, its `(min, max)` range, the constant `c`, whether `v` is on the
    // LEFT). Exactly one side must be a constant and the OTHER a runtime value with a known range.
    let (v, (min, max), c, v_on_left) =
        if let (Some(c), Some(b)) = (const_val(db, rhs), value_range(db, lhs)) {
            (lhs, b, c, true) // `(op v c)`
        } else if let (Some(c), Some(b)) = (const_val(db, lhs), value_range(db, rhs)) {
            (rhs, b, c, false) // `(op c v)`
        } else {
            return None;
        };
    // EQUALITY against a value with a known range is DECIDABLE when `c` lies OUTSIDE `[min, max]` (no
    // value in the range equals `c` → `false`) or the range pins `v` to the single point `{c}` (`v` can
    // only be `c` → `true`). `=` is symmetric, so `v_on_left` is immaterial. This subsumes the classic
    // `(= (& x 15) 100)` → false (`x & 15 ∈ [0,15]`, `100` above). Discards `v`, so — like the ordering
    // rules below — only fires when `v` is TRAP-FREE (a trapping operand must keep its runtime compare so
    // the trap survives). The single-point `true` case rarely fires at lower time (a `[c,c]` range comes
    // from a `ConstInt`, already folded), but is sound and DOES fire via the refinement sibling
    // (`refined_comparison_const`, a match arm pinning the scrutinee to `{c}`).
    if matches!(op, Prim::Eq) {
        let outside = c < min || max.is_some_and(|m| c > m);
        let pinned = min == c && max == Some(c);
        if (outside || pinned) && is_trap_free(db, v) {
            trace!(target: "rcdzc::fold", node = v.0, c, min, ?max, result = pinned, "range-vs-constant equality is decidable — folds to a constant");
            return Some(Core::ConstBool(pinned));
        }
        return None;
    }
    // Normalize to the `(cmp v c)` sense — the LEFT-const forms are the mirror (`c < v` ≡ `v > c`).
    let cmp = match (op, v_on_left) {
        (Prim::Lt, true) | (Prim::Gt, false) => Prim::Lt,
        (Prim::Gt, true) | (Prim::Lt, false) => Prim::Gt,
        (Prim::Le, true) | (Prim::Ge, false) => Prim::Le,
        (Prim::Ge, true) | (Prim::Le, false) => Prim::Ge,
        _ => return None,
    };
    // The constant occurrence to reuse as the rhs of a rewritten `v == c` (keeps its width grounding).
    let c_occ = if v_on_left { rhs } else { lhs };
    // A tautology/unsatisfiable comparison folds to a CONSTANT — but that DISCARDS the runtime operand
    // `v`, so a TRAPPING operand (`(/ 10 z)` with z==0, an overflowing `(+ x x)`) would lose its trap and
    // the program would run to the folded bool instead of trapping. Only fold when `v` is TRAP-FREE; a
    // possibly-trapping operand keeps the runtime `Core::Compare` (returning `None`), which evaluates `v`
    // and traps exactly as a genuine comparison does. This mirrors the self-comparison fold's
    // `is_trap_free` guard above (which likewise refuses to drop a trapping operand). The `v == bound`
    // rewrite (`eq_bound`) is unaffected — it KEEPS `v` as an operand, so its trap is preserved.
    let operand_trap_free = is_trap_free(db, v);
    let const_bool = |r: bool, why: &str| {
        if !operand_trap_free {
            trace!(target: "rcdzc::fold", node = v.0, why, "range-vs-constant comparison is decidable but the operand may trap — keep the runtime compare to preserve the trap");
            return None;
        }
        trace!(target: "rcdzc::fold", node = v.0, why, "range-vs-constant comparison folds to a constant");
        Some(Core::ConstBool(r))
    };
    let eq_bound = |why: &str| {
        trace!(target: "rcdzc::fold", node = v.0, why, "range-vs-constant comparison folds to `== bound`");
        Some(Core::Compare {
            op: Prim::Eq,
            lhs: v,
            rhs: c_occ,
        })
    };
    // `v ∈ [min, max]` (`max` is `None` when the upper bound is not i64-representable — an unsigned-64
    // value, `[0, 2^64)`). A comparison against `c` is DECIDABLE when the whole range lies on one side;
    // the boundary cases (`c` exactly at `min`/`max`) collapse `<=`/`>=` to an equality test (which the
    // backend selects to `eqz` when the bound is 0). Rules verified exhaustively. A rule that references
    // `max` fires only when `max` is known.
    let above_max = |c: i64| max.is_some_and(|m| c > m);
    let at_or_above_max = |c: i64| max.is_some_and(|m| c >= m);
    let at_max = |c: i64| max == Some(c);
    match cmp {
        Prim::Lt if above_max(c) => const_bool(true, "v < c: c > max"), // every v < c
        Prim::Lt if c <= min => const_bool(false, "v < c: c <= min"),   // no v < c
        Prim::Ge if above_max(c) => const_bool(false, "v >= c: c > max"),
        Prim::Ge if c <= min => const_bool(true, "v >= c: c <= min"),
        Prim::Le if at_or_above_max(c) => const_bool(true, "v <= c: c >= max"),
        Prim::Le if c < min => const_bool(false, "v <= c: c < min"),
        Prim::Le if c == min => eq_bound("v <= min ⇔ v == min"),
        Prim::Gt if at_or_above_max(c) => const_bool(false, "v > c: c >= max"),
        Prim::Gt if c < min => const_bool(true, "v > c: c < min"),
        Prim::Ge if at_max(c) => eq_bound("v >= max ⇔ v == max"),
        // A constant strictly inside the range (and not at a collapsing boundary) — not decidable.
        _ => None,
    }
}

/// Decide an ORDERING comparison `(op a b)` purely from the two operands' inclusive ranges `[alo, ahi]`
/// and `[blo, bhi]` (`hi` is `None` when unbounded above — an unsigned-64 value). Returns `Some(true)`/
/// `Some(false)` only when the ranges are DISJOINT enough that the operator's result is the same for
/// every `(a, b)` in them; `None` when the ranges overlap so the result depends on the runtime values.
/// The range-vs-range companion of the constant-vs-range fold, consumed by `refined_comparison_const` at
/// emit time (where the flow-refinement stack lands two runtime vars in disjoint intervals) — e.g. under
/// `a ∈ [101,…]`, `b ∈ […,49]` ⟹ `a > b` always, `a < b` never. `Eq` is intentionally NOT decided here (an
/// ordering-only helper, mirroring the `Eq`-excludes structure of the constant folds). Reasoning
/// (for `<`): TRUE for all iff even the largest `a` is below the smallest `b` (`ahi < blo`); FALSE for all
/// iff even the smallest `a` is not below the largest `b` (`alo >= bhi`, needs both bounds). The other
/// operators are the standard rewrites of `<`. A missing bound (`None`) simply fails the guard that needs
/// it (leaving the comparison runtime), never fabricating a decision.
fn compare_disjoint_ranges(
    op: Prim,
    (alo, ahi): (i64, Option<i64>),
    (blo, bhi): (i64, Option<i64>),
) -> Option<bool> {
    // `a < b` for ALL pairs iff `max(a) < min(b)`; NEVER iff `min(a) >= max(b)`.
    let lt_always = || ahi.is_some_and(|ah| ah < blo);
    let lt_never = || bhi.is_some_and(|bh| alo >= bh);
    // `a > b` for ALL pairs iff `min(a) > max(b)`; NEVER iff `max(a) <= min(b)`.
    let gt_always = || bhi.is_some_and(|bh| alo > bh);
    let gt_never = || ahi.is_some_and(|ah| ah <= blo);
    match op {
        Prim::Lt if lt_always() => Some(true),
        Prim::Lt if lt_never() => Some(false),
        Prim::Ge if lt_always() => Some(false), // `>=` is `!(<)`
        Prim::Ge if lt_never() => Some(true),
        Prim::Gt if gt_always() => Some(true),
        Prim::Gt if gt_never() => Some(false),
        Prim::Le if gt_always() => Some(false), // `<=` is `!(>)`
        Prim::Le if gt_never() => Some(true),
        _ => None,
    }
}

/// EMIT-TIME comparison fold against the CURRENT flow-sensitive refinement (cycle 82's branch-range
/// stack). When one operand is a compile-time constant `c` and the other a runtime value whose refined
/// `value_range` lies ENTIRELY on one side of `c`, the comparison is decidable to a constant bool — a
/// redundant re-test the enclosing branch already established (`(if (> n 0) (if (> n 0) …) …)`, or an
/// IMPLIED test `(if (>= n 5) (if (> n 0) …) …)`). This is the sibling of [`fold_comparison_at_type_bound`]
/// but is called from the `Core::Compare` EMIT arm (not `lower`), because refinements are populated only
/// during emit — `lower` runs with the stack empty and cannot see branch facts. Returns only the
/// CONSTANT-bool verdicts (not the `== bound` collapse, which needs a synthesized node `lower` owns).
///
/// SOUND: the refined range is a fact the branch GUARANTEES; folding the comparison discards the runtime
/// operand, so — exactly as `fold_comparison_at_type_bound` — it only fires when that operand is
/// `is_trap_free` (a refined variable is a `Param`/`LocalRef`, trap-free), never dropping a trap.
pub(crate) fn refined_comparison_const(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Option<bool> {
    // Whether `id` is a variable currently carrying an active flow refinement — the emit-only fact that
    // `lower` could not see. At least one operand must be refined for a fold here to add value beyond the
    // const-fold tier's declared-type reasoning.
    let is_refined = |db: &mut Db, id: StructId| {
        matches!(
            core_of(db, id),
            Core::Param { binder } | Core::LocalRef { binder } if db.refined_range(binder).is_some()
        )
    };
    // RANGE-VS-RANGE (both operands runtime): when NEITHER operand is constant but their refined ranges
    // are disjoint enough to decide the ordering, fold to the constant — a flow-sensitive comparison
    // elimination (`(if (> a 100) (if (< b 50) (< b a) …))` → the inner `(< b a)` is always true, since
    // `a ∈ [101,…]` and `b ∈ […,49]`). This lives ONLY at emit time: at lower time, a disjoint positive
    // range needs either a constant operand (handled by the const-vs-range fold) or a checked arith op
    // (not trap-free, so the discarding fold declines) — the refinement stack is what makes two runtime
    // variables land in disjoint intervals. Only fires when at least one operand is genuinely REFINED and
    // BOTH are trap-free (the fold discards both). `Eq` is not handled here (an ordering-only helper,
    // matching the const-fold structure). Attempted BEFORE the const-operand path so a `(< a b)` where
    // both are refined vars is decided; a const operand has no refinement and falls through below.
    if !matches!(op, Prim::Eq)
        && let (Some(ra), Some(rb)) = (value_range(db, lhs), value_range(db, rhs))
        && let Some(r) = compare_disjoint_ranges(op, ra, rb)
        && (is_refined(db, lhs) || is_refined(db, rhs))
        && is_trap_free(db, lhs)
        && is_trap_free(db, rhs)
    {
        return Some(r);
    }
    let const_val = |db: &mut Db, id: StructId| match core_of(db, id) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // (runtime operand `v`, its `[min, max]` refined range, the constant `c`, whether `v` is on the left).
    let (v, (min, max), c, v_on_left) =
        if let (Some(c), Some(b)) = (const_val(db, rhs), value_range(db, lhs)) {
            (lhs, b, c, true)
        } else if let (Some(c), Some(b)) = (const_val(db, lhs), value_range(db, rhs)) {
            (rhs, b, c, false)
        } else {
            return None;
        };
    // Only a genuinely REFINED variable is interesting here: a bare declared-type range already folds in
    // `lower` (via `fold_comparison_at_type_bound`), so if this fires only on the type bound it is a
    // no-op the const-fold tier handled. Require the operand be a variable with an active refinement so
    // we add value only where `lower` could not see the branch fact.
    let refined = matches!(
        core_of(db, v),
        Core::Param { binder } | Core::LocalRef { binder } if db.refined_range(binder).is_some()
    );
    if !refined {
        return None;
    }
    // Discarding `v` must not drop a trap (a refined `Param`/`LocalRef` is trap-free, so this holds).
    if !is_trap_free(db, v) {
        return None;
    }
    // EQUALITY against the refined range: DECIDABLE when `c` lies OUTSIDE `[min, max]` (→ false) or the
    // range PINS `v` to `{c}` (→ true). The pin case is the payoff a match arm's exact-value refinement
    // enables — `refined_frame_for_match_arm` sets the scrutinee to `[c, c]`, so a redundant `(= n 5)`
    // inside the `(5 …)` arm folds to `true`; the outside case folds a re-test the enclosing branch's
    // interval already excludes (`(if (> n 10) (= n 3) …)` → `false`). `=` is symmetric — `v_on_left` is
    // immaterial. Sibling of the ordering rules below.
    if matches!(op, Prim::Eq) {
        let outside = c < min || max.is_some_and(|m| c > m);
        let pinned = min == c && max == Some(c);
        return if outside {
            Some(false)
        } else if pinned {
            Some(true)
        } else {
            None
        };
    }
    // Normalize to `(cmp v c)`.
    let cmp = match (op, v_on_left) {
        (Prim::Lt, true) | (Prim::Gt, false) => Prim::Lt,
        (Prim::Gt, true) | (Prim::Lt, false) => Prim::Gt,
        (Prim::Le, true) | (Prim::Ge, false) => Prim::Le,
        (Prim::Ge, true) | (Prim::Le, false) => Prim::Ge,
        _ => return None,
    };
    // `v ∈ [min, max]`. Decide only when the WHOLE range lies on one side of `c`.
    let above_max = |c: i64| max.is_some_and(|m| c > m); // c strictly above the range → v < c always
    let at_or_above_max = |c: i64| max.is_some_and(|m| c >= m);
    match cmp {
        Prim::Lt if above_max(c) => Some(true), // v < c: c > max → always
        Prim::Lt if c <= min => Some(false),    // v < c: c <= min → never
        Prim::Ge if above_max(c) => Some(false), // v >= c: c > max → never
        Prim::Ge if c <= min => Some(true),     // v >= c: c <= min → always
        Prim::Le if at_or_above_max(c) => Some(true), // v <= c: c >= max → always
        Prim::Le if c < min => Some(false),     // v <= c: c < min → never
        Prim::Gt if at_or_above_max(c) => Some(false), // v > c: c >= max → never
        Prim::Gt if c < min => Some(true),      // v > c: c < min → always
        _ => None,
    }
}

/// The inclusive range a runtime value provably occupies, as `(min, max)` where `min: i64` is always
/// known and `max: Option<i64>` is absent when the upper bound is not i64-representable (an unsigned-64
/// value spans `[0, 2^64)` — min 0, no i64 max). `None` when no bound is known at all. Prefers the DERIVED
/// range from `unsigned_value_bits` (a nonnegative value with a known significant-bit count `B` →
/// `[0, 2^B − 1]`, tighter than its type) and falls back to the value's declared-type bounds. Feeds the
/// range-vs-constant comparison fold.
fn value_range(db: &mut Db, id: StructId) -> Option<(i64, Option<i64>)> {
    // A CONSTANT's range is exactly itself — `[v, v]` (the tightest possible).
    if let Core::ConstInt(v) = core_of(db, id)
        && let Some(v) = v.to_i64()
    {
        return Some((v, Some(v)));
    }
    // A VARIABLE reference (a parameter or a kept `let`-binding) MAY carry a flow-sensitive REFINEMENT: a
    // range known to hold in the branch currently being emitted (`n : Int64` refined to `[2, MAX]` inside
    // the else-branch of `(< n 2)`). When present, INTERSECT it with the declared-type bounds — the
    // tightest sound range — so a guard-elision check sees the narrowed range and drops a dead overflow
    // guard (`(- n 1)` under `n ≥ 2` cannot underflow). Refinements are EMIT-ONLY (the const-fold callers
    // run with the stack empty, so this is a no-op there) and `value_range` is never memoized, so a
    // transient refinement cannot poison any cached result. When no refinement applies, falls through to
    // the ordinary arith/type range below.
    if let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, id)
        && let Some((rlo, rhi)) = db.refined_range(binder)
    {
        // Intersect with the declared-type bounds (a refinement only NARROWS a real value).
        let type_range = match crate::infer::type_of(db, id) {
            crate::ty::Ty::Int(it) => {
                resolved_int_bounds(it).and_then(|(lo, hi)| lo.map(|lo| (lo, hi)))
            }
            _ => None,
        };
        let (tlo, thi) = type_range.unwrap_or((i64::MIN, Some(i64::MAX)));
        let lo = rlo.max(tlo);
        let hi = match (rhi, thi) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, b) => b,
        };
        return Some((lo, hi));
    }
    // A kept `let`-binding reference (`Core::LocalRef { binder }`, no active refinement) carries the range
    // of its INITIALIZER — `binder` IS the initializer's occurrence (see `lower_let`). So a multi-use
    // masked binding `(let ((y (& x 255))) (+ y y))` propagates `[0,255]` through `y`, letting `(+ y y)`
    // shed its overflow guard exactly as the inlined `(+ (& x 255) (& x 255))` does. The initializer is a
    // distinct, earlier node (a binding never references itself), so the recursion bottoms out. No
    // refinement applies here (the block above returned early if one did).
    if let Core::LocalRef { binder } = core_of(db, id)
        && let Some(r) = value_range(db, binder)
    {
        return Some(r);
    }
    // An ARITHMETIC / BITWISE / SHIFT node's range PROPAGATES from its operands' ranges — this is the
    // dataflow layer that lets a bounded sub-expression bound its enclosing op (`(+ (& x 15) (& y 15))`
    // → [0,30], and a further `(+ … (& z 15))` → [0,45]). All interval math is in `i128` so endpoints
    // never wrap; the result is clamped back to `i64` range (a bound outside i64 becomes "unbounded" on
    // that side). A `None` operand range makes the whole node unbounded (falls through to the type).
    if let Core::Arith { op, lhs, rhs } = core_of(db, id)
        && let Some(r) = arith_range(db, op, lhs, rhs)
    {
        return Some(r);
    }
    // A CONDITIONAL's range is the UNION of its branches' ranges — the value IS one branch's value, so it
    // lies within `[min(lo), max(hi)]`. This bounds a bool-materialized `(if c 1 0)` to `[0,1]`, so a
    // downstream `(* bool C)` / `(+ bool C)` sheds its overflow guard (`bool*C ∈ [0,C]` can't overflow),
    // and any small-constant conditional (`(if c 10 20)` → [10,20]) bounds its consumer. Both branches
    // must have a known range (an unbounded branch makes the union unbounded → `None`). A `None` upper on
    // EITHER branch makes the union's upper `None` (unbounded above). Match arms union the same way.
    let branch_union = |db: &mut Db, branches: &[StructId]| -> Option<(i64, Option<i64>)> {
        let mut lo = i64::MAX;
        let mut hi: Option<i64> = Some(i64::MIN);
        for &b in branches {
            let (blo, bhi) = value_range(db, b)?;
            lo = lo.min(blo);
            hi = match (hi, bhi) {
                (Some(h), Some(bh)) => Some(h.max(bh)),
                _ => None, // an unbounded-above branch makes the union unbounded above
            };
        }
        Some((lo, hi))
    };
    match core_of(db, id) {
        Core::If { then_, else_, .. } => {
            if let Some(r) = branch_union(db, &[then_, else_]) {
                return Some(r);
            }
        }
        Core::Match { arms, .. } => {
            let bodies: Vec<StructId> = arms.iter().map(|a| a.body).collect();
            if !bodies.is_empty()
                && let Some(r) = branch_union(db, &bodies)
            {
                return Some(r);
            }
        }
        // A COLLECTION COUNT — `List.len`/`Bytes.len`/`Map.size`/`Set.len` — is a NON-NEGATIVE `Int64`
        // whose backend value is an i32 count zero-extended to i64 (`I64ExtendI32U`), so it lives in
        // `[0, 2^32 − 1]`. This lets the range folds fire on a length: `(>= (List.len xs) 0)` → true,
        // `(< (List.len xs) 0)` → false, a `(match (List.len xs) (-1 …) …)` drops the impossible arm, and
        // a length used as a mask/shift/dividend operand sheds guards. The `2^32-1` upper bound is the true
        // representable maximum (a count can't exceed the i32 the runtime returns); a tighter real cap
        // (heap size) is unknown here, so `2^32-1` is the sound envelope.
        Core::ListLen { .. }
        | Core::BytesLen { .. }
        | Core::MapSize { .. }
        | Core::SetLen { .. } => {
            return Some((0, Some(u32::MAX as i64)));
        }
        _ => {}
    }
    // Else the declared integer type's bounds. `min` must be i64-representable (it always is: signed MIN
    // and unsigned 0 both fit); `max` may be absent (unsigned-64).
    match crate::infer::type_of(db, id) {
        crate::ty::Ty::Int(it) => match resolved_int_bounds(it) {
            Some((Some(lo), hi)) => Some((lo, hi)),
            _ => None,
        },
        _ => None,
    }
}

/// The range of a `Core::Arith { op, lhs, rhs }` result, propagated from the operand ranges. `None` when
/// the op is not range-tracked or an operand's range is unknown (→ the node falls back to its type). All
/// arithmetic is `i128` (endpoints never wrap); each endpoint is clamped to `i64` (an out-of-i64 bound
/// becomes `None` on that side = "unbounded there"). Covers: `&` (bounded by the smaller nonneg operand),
/// `|`/`^` (bounded by `2^max(bits)` for nonneg operands), `<<`/`>>ᵤ` by a constant, `%` by a constant
/// divisor (`[-(|C|-1), |C|-1]`, tightened to `[0, |C|-1]` for a nonneg dividend), and `+`/`-`/`*`
/// interval arithmetic. Verified sound by exhaustive endpoint checks.
fn arith_range(db: &mut Db, op: Prim, lhs: StructId, rhs: StructId) -> Option<(i64, Option<i64>)> {
    let clamp = |lo: i128, hi: i128| -> (i64, Option<i64>) {
        let lo = lo.max(i64::MIN as i128) as i64;
        let hi = if hi <= i64::MAX as i128 {
            Some(hi as i64)
        } else {
            None
        };
        (lo, hi)
    };
    // Significant-bit count of a NON-NEGATIVE constant (`⌈log2(v+1)⌉`), for the bitwise-op bounds.
    let const_nonneg_bits = |db: &mut Db, o: StructId| -> Option<u32> {
        match core_of(db, o) {
            Core::ConstInt(v) => v
                .to_i64()
                .filter(|&x| x >= 0)
                .map(|x| 64 - (x as u64).leading_zeros()),
            _ => None,
        }
    };
    match op {
        // `v & M`: a nonneg operand (constant mask OR a value with a known `[0,hi]`) caps the result at
        // its significant-bit count; the AND fits the MIN of whatever bounds are known. A nonneg mask on
        // either side alone bounds it — `(& x:Int64 15)` → [0,15] regardless of `x`'s sign.
        Prim::BitAnd => {
            let bits = |db: &mut Db, o: StructId| -> Option<u32> {
                const_nonneg_bits(db, o).or_else(|| unsigned_value_bits(db, o))
            };
            let (a, b) = (bits(db, lhs), bits(db, rhs));
            let bound = a.into_iter().chain(b).min()?;
            Some((0, Some((1i64 << bound.min(62)) - 1)))
        }
        // `v | w` / `v ^ w`: a nonneg result sets no bit above `max` of the operands' bit counts. BOTH
        // operands must be provably nonnegative (a negative operand's high bits would set the sign bit).
        Prim::BitOr | Prim::BitXor => {
            let a = const_nonneg_bits(db, lhs).or_else(|| unsigned_value_bits(db, lhs))?;
            let b = const_nonneg_bits(db, rhs).or_else(|| unsigned_value_bits(db, rhs))?;
            Some((0, Some((1i64 << a.max(b).min(62)) - 1)))
        }
        // `v <<ᵤ k` (constant `k`): a nonneg `v ∈ [0,hi]` shifts to `[0, hi << k]` (in i128, clamped).
        Prim::Shl => {
            let (0, Some(hi)) = value_range(db, lhs)? else {
                return None;
            };
            let Core::ConstInt(k) = core_of(db, rhs) else {
                return None;
            };
            let k = k.to_i64().filter(|&k| (0..64).contains(&k))?;
            Some(clamp(0, (hi as i128) << k))
        }
        // `v >>ᵤ k` (constant `k`, LOGICAL — an unsigned/nonneg operand): a nonneg `v` shifted right by
        // `k` loses its low `k` bits. Two cases, both requiring `v` proven NONNEGATIVE (`value_range` lo
        // == 0; a signed `>>ₛ` sign-extends and is excluded):
        //   • a KNOWN finite `v ∈ [0, hi]` → `[0, hi >> k]`;
        //   • an UNBOUNDED-above nonneg `v` (a bare `UInt64`, whose max `2^64-1` is not i64-representable)
        //     → still bounded by the TYPE WIDTH: an unsigned width-`W` value is `< 2^W`, so `v >>ᵤ k <
        //     2^(W−k)` → `[0, 2^(W−k) − 1]`. This is what makes `(& (>> x 56) 255)` on a UInt64 drop its
        //     redundant mask (`x >>ᵤ 56 ∈ [0,255]` already fits the mask). `W − k` may be ≥ 64 for small
        //     `k` at width 64 → the bound is not i64-representable, so leave it unbounded (`None`).
        Prim::Shr => {
            let (0, hi_opt) = value_range(db, lhs)? else {
                return None;
            };
            let Core::ConstInt(k) = core_of(db, rhs) else {
                return None;
            };
            let k = k.to_i64().filter(|&k| (0..64).contains(&k))?;
            match hi_opt {
                Some(hi) => Some((0, Some(hi >> k))),
                None => {
                    // Unbounded-above nonneg: bound by the type width. `W − k` significant bits remain.
                    let crate::ty::Ty::Int(it) = crate::infer::type_of(db, lhs) else {
                        return None;
                    };
                    let crate::ty::Width::Fixed(w) = it.width else {
                        return None;
                    };
                    let bits = (w as i64) - k;
                    // `bits` in `1..=62` → `2^bits − 1` fits i64 and is computed by the shift. `bits == 63`
                    // is exactly `2^63 − 1 = i64::MAX` (the shift `1i64 << 63` is `i64::MIN`, so `− 1` would
                    // OVERFLOW in a checked build — a latent panic; handle it directly). `bits ≥ 64` or `≤ 0`
                    // → not i64-representable, so nonneg but no finite upper bound.
                    match bits {
                        1..=62 => Some((0, Some((1i64 << bits) - 1))),
                        63 => Some((0, Some(i64::MAX))),
                        _ => Some((0, None)), // still nonneg, but no finite i64 upper bound
                    }
                }
            }
        }
        // `v % C` (constant divisor `C`): the truncated-toward-zero remainder has magnitude `< |C|`, so
        // its range is `[-(|C|-1), |C|-1]` in general, tightened to `[0, |C|-1]` when the DIVIDEND is
        // provably non-negative (a nonneg dividend yields a nonneg remainder). `(% x 10)` → `[-9,9]`, and
        // `(% (& x 255) 10)` → `[0,9]`. Only a compile-time constant divisor (the common `% C` idiom); a
        // runtime divisor's range is unknown here. `C = 0` never reaches this (a constant `÷0`/`%0` is a
        // constant-trap poison in `lower`); `|C| = 1` folds to `0` (the `% 1` identity) before here.
        Prim::Rem => {
            let Core::ConstInt(c) = core_of(db, rhs) else {
                return None;
            };
            let d = c.to_i64().map(|c| c.unsigned_abs()).filter(|&d| d >= 1)?;
            let hi = (d - 1).min(i64::MAX as u64) as i64;
            let lo = if value_provably_nonneg(db, lhs) {
                0
            } else {
                -hi
            };
            Some((lo, Some(hi)))
        }
        // `+`/`-`/`*`: interval arithmetic over the operands' CLOSED ranges.
        Prim::Add | Prim::Sub | Prim::Mul => {
            let (alo, ahi) = closed_range(db, lhs)?;
            let (blo, bhi) = closed_range(db, rhs)?;
            let (alo, ahi, blo, bhi) = (alo as i128, ahi as i128, blo as i128, bhi as i128);
            let (lo, hi) = match op {
                Prim::Add => (alo + blo, ahi + bhi),
                Prim::Sub => (alo - bhi, ahi - blo),
                _ => {
                    let c = [alo * blo, alo * bhi, ahi * blo, ahi * bhi];
                    (*c.iter().min().unwrap(), *c.iter().max().unwrap())
                }
            };
            Some(clamp(lo, hi))
        }
        _ => None,
    }
}

/// A CLOSED range `[min, max]` (both bounds i64) for a value, or `None` if either bound is unknown — the
/// form INTERVAL ARITHMETIC needs. Wraps `value_range`, demanding a finite `max` (an unbounded value can
/// overflow any op, so no fit proof).
fn closed_range(db: &mut Db, id: StructId) -> Option<(i64, i64)> {
    match value_range(db, id) {
        Some((lo, Some(hi))) => Some((lo, hi)),
        _ => None,
    }
}

/// Whether a CHECKED `+`/`-`/`*` at `id` provably CANNOT overflow its result type — so its overflow guard
/// (and narrow range-check) can be elided at emit. Computes the exact result INTERVAL from the operands'
/// `closed_range`s (in `i128`, so the interval endpoints never wrap) and checks it lies within the result
/// type's `[min, max]`. The interval per op: `+` → `[alo+blo, ahi+bhi]`; `-` → `[alo−bhi, ahi−blo]`; `*` →
/// the min/max of the four corner products. A `None` operand range → `false` (unknown, keep the guard).
/// Verified sound by exhaustive endpoint check. Lets a masked/narrowed operand (`(+ (& x 15) (& y 15))`,
/// sum ≤ 30) shed its guard.
pub(crate) fn arith_provably_in_range(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    result: crate::ty::IntTy,
) -> bool {
    // The RESULT type's inclusive bounds come from `result` — the op's AUTHORITATIVE machine width/sign
    // (the caller's `Machine`, derived from the ARITH NODE's solved type), NOT from an operand's node
    // type. ⚠ An operand's node type can be misleading: a bare-literal-branch `if` (`(if c 1 0)`) or a
    // bare literal is still DEFERRED, and its default (Int64) is WIDER than the op when the true width
    // comes from CONTEXT — `(: (+ (if (< n 5) 100 0) 100) Int8)` is an Int8 op even though both operands'
    // nodes are deferred, and `(- 0 n)` at Int8 is Int8 though the `0` is deferred. Using the op's real
    // width is the only sound choice. (An unsigned-64 result has no i64 max → cannot prove a fit here.)
    let (Some(tmin), Some(tmax)) = (match resolved_int_bounds(result) {
        Some(b) => b,
        None => return false,
    }) else {
        return false;
    };
    let (Some((alo, ahi)), Some((blo, bhi))) = (closed_range(db, lhs), closed_range(db, rhs))
    else {
        return false;
    };
    let (alo, ahi, blo, bhi) = (alo as i128, ahi as i128, blo as i128, bhi as i128);
    let (rlo, rhi) = match op {
        Prim::Add => (alo + blo, ahi + bhi),
        Prim::Sub => (alo - bhi, ahi - blo),
        Prim::Mul => {
            let c = [alo * blo, alo * bhi, ahi * blo, ahi * bhi];
            (*c.iter().min().unwrap(), *c.iter().max().unwrap())
        }
        _ => return false,
    };
    rlo >= tmin as i128 && rhi <= tmax as i128
}

/// Whether `val << k` provably CANNOT overflow the type of `val` — so the shift's overflow round-trip
/// guard (and narrow range-check) can be elided. A left shift is exact multiplication by `2^k`, so it
/// overflows iff the interval `[vlo << k, vhi << k]` leaves the type's `[min, max]`. `<<` is monotone for
/// `k ≥ 0`, so checking both endpoints (in `i128`, never wrapping) suffices. Used by both the `<<` emit
/// and the `* 2^k → <<` strength-reduction emit — a masked/bounded operand (`(<< (& x 15) 2)` = `[0,60]`)
/// sheds its guard. `None` operand range → `false` (keep the guard). Verified sound by endpoint check.
pub(crate) fn shl_provably_in_range(db: &mut Db, val: StructId, k: u32) -> bool {
    let crate::ty::Ty::Int(it) = crate::infer::type_of(db, val) else {
        return false;
    };
    let (Some(tmin), Some(tmax)) = (match resolved_int_bounds(it) {
        Some(b) => b,
        None => return false,
    }) else {
        return false;
    };
    // CLEAR-LOW-BITS IDIOM: `(v >> k) << k` — a right shift by `k` then a left shift by the SAME `k` — just
    // CLEARS the low `k` bits of `v`, so the result `floor(v / 2^k) * 2^k` NEVER leaves `v`'s type, for
    // BOTH shift kinds. Signed (`>>ₛ`, floor toward −∞): `MIN` is a multiple of `2^k` (k ≤ 63), so the
    // rounded-down value stays ≥ `MIN` and ≤ 0 ≤ `MAX`. Unsigned (`>>ᵤ`): `q·2^k ≤ v`, stays in `[0, max]`.
    // The INTERVAL analysis below cannot prove this — a signed `v >>ₛ k` over a full-range `v` has no
    // finite range, so `closed_range` returns the type bounds and `[MIN<<k, MAX<<k]` spuriously overflows.
    // This structural recognizer sees the correlation the intervals lose. `val`'s type IS `v`'s type
    // (`>>` preserves width), so the fit is against the right bounds. The inner count must be the SAME
    // constant `k`.
    if let Core::Arith {
        op: Prim::Shr,
        rhs: inner_count,
        ..
    } = core_of(db, val)
        && let Core::ConstInt(ic) = core_of(db, inner_count)
        && ic.to_i64() == Some(k as i64)
    {
        return true;
    }
    let Some((vlo, vhi)) = closed_range(db, val) else {
        return false;
    };
    // `k < 128` guaranteed (a valid shift count is `< width ≤ 64`); the i128 shift never overflows for a
    // real operand interval, and the fit-check against the i64-representable type bounds catches any
    // out-of-type result.
    let (rlo, rhi) = ((vlo as i128) << k, (vhi as i128) << k);
    rlo >= tmin as i128 && rhi <= tmax as i128
}

/// The RUNTIME-COUNT companion of [`shl_provably_in_range`]: whether `(<< val count)` provably stays in
/// `val`'s type for a count whose range is only known at compile time (not a fixed constant) — the
/// masked-count idiom `(<< (& x 15) (& k 3))`, where `val ∈ [0,15]` and `count ∈ [0,7]` gives a max
/// `15 << 7 = 1920` that fits. Requires BOTH the value's `closed_range` `[vlo, vhi]` AND the count's
/// `value_range` `[clo, chi]` to be known, with a valid count range `0 <= clo <= chi < width` (an
/// out-of-range count is a genuine trap the guard must keep). `<<` is monotonic in the value for a fixed
/// nonneg count, so the result's bounding box is spanned by the four corners `{vlo,vhi} << {clo,chi}`
/// (a negative `vlo` shifts MORE negative as the count grows; a positive `vhi` shifts MORE positive);
/// the box fits iff its min ≥ tmin and max ≤ tmax. All in i128 (a real interval × count < width never
/// overflows i128). Returns false on any unknown bound (keep the overflow round-trip).
pub(crate) fn shl_provably_in_range_dynamic(db: &mut Db, val: StructId, count: StructId) -> bool {
    let crate::ty::Ty::Int(it) = crate::infer::type_of(db, val) else {
        return false;
    };
    let (Some(tmin), Some(tmax)) = (match resolved_int_bounds(it) {
        Some(b) => b,
        None => return false,
    }) else {
        return false;
    };
    let crate::ty::Width::Fixed(width) = it.width else {
        return false;
    };
    // The count must have a known, VALID range `[clo, chi]` with `0 <= clo` and `chi < width` — an
    // out-of-range count still traps (its guard is handled separately), so we only reason about the
    // in-range shift amounts here.
    let Some((clo, chi)) = value_range(db, count) else {
        return false;
    };
    let Some(chi) = chi else { return false };
    if clo < 0 || chi < 0 || chi >= width as i64 {
        return false;
    }
    let Some((vlo, vhi)) = closed_range(db, val) else {
        return false;
    };
    // `<<` by a fixed nonneg count is monotonic in the value AND (in magnitude) in the count, so the
    // result's bounding box corners are `{vlo, vhi} << {clo, chi}`.
    let corners = [
        (vlo as i128) << clo,
        (vlo as i128) << chi,
        (vhi as i128) << clo,
        (vhi as i128) << chi,
    ];
    let rlo = corners.iter().copied().min().unwrap();
    let rhi = corners.iter().copied().max().unwrap();
    rlo >= tmin as i128 && rhi <= tmax as i128
}

/// Whether the divisor at `id` could be `-1`. The narrow-signed-division range-check exists SOLELY for
/// the `MIN_N / -1` overflow (the only quotient that leaves the type); if the divisor provably is NOT
/// `-1`, that check is dead. Returns `true` (keep the check) unless the divisor's range EXCLUDES `-1` —
/// a constant `≠ -1`, or a value whose `value_range` does not straddle `-1` (e.g. an unsigned/nonneg
/// value, or a masked `(& y 7)` ∈ [0,7]). Conservative: an unknown range → `true`.
pub(crate) fn divisor_can_be_neg_one(db: &mut Db, id: StructId) -> bool {
    match value_range(db, id) {
        Some((lo, Some(hi))) => lo <= -1 && -1 <= hi, // -1 within [lo, hi]
        Some((lo, None)) => lo <= -1,                 // unbounded above; can reach -1 iff lo <= -1
        None => true,                                 // unknown → assume it can
    }
}

/// Whether the value at `id` is provably NON-NEGATIVE (its `value_range` lower bound is `≥ 0`). Consults
/// the same lattice as the guard-elision checks — so it sees a mask (`(& x 255)` ∈ [0,255]), an unsigned
/// type, AND a FLOW-SENSITIVE refinement (`x` under `(> x 0)`). Used by the signed `/`/`%` by a power of
/// two: a non-negative dividend truncates toward zero identically to a plain shift/mask, so the
/// round-toward-zero BIAS sequence (needed only to correct negatives) is DEAD. Conservative: an unknown
/// or possibly-negative range → `false` (keep the bias).
pub(crate) fn value_provably_nonneg(db: &mut Db, id: StructId) -> bool {
    matches!(value_range(db, id), Some((lo, _)) if lo >= 0)
}

/// Whether the value at `id` provably lies within the inclusive `[lo, hi]` — its `value_range` is known
/// AND fully contained. Consults the same lattice as the guard-elision checks (a mask, an unsigned type,
/// a flow-refinement). Used by `emit_wrap`'s truncation-elision: a `wrap` to width N is a no-op when the
/// operand already lies in the target's `[min_N, max_N]` (an unsigned target's `[0, 2^N-1]`), even when
/// the operand's TYPE is wider — `UInt8.wrap(& x 255)` needs no re-mask. Conservative: an unknown range,
/// or one that exceeds either bound, → `false` (keep the truncation).
pub(crate) fn value_range_within(db: &mut Db, id: StructId, lo: i64, hi: i64) -> bool {
    matches!(value_range(db, id), Some((vlo, Some(vhi))) if vlo >= lo && vhi <= hi)
}

/// Whether the value at `id` provably CANNOT equal the constant `c` — its `value_range` is known and `c`
/// lies strictly OUTSIDE it (`c < min` or `c > max`). Consults the same lattice as the guard/comparison
/// folds (a mask, an unsigned type, a flow-refinement). Used by the match probe chain to drop a DEAD arm
/// whose literal probe the scrutinee's range excludes (`(match (& x 7) (100 …) …)`: `x & 7 ∈ [0,7]`, so
/// the `100` arm can never match). Conservative: an unknown range, or one that contains `c`, → `false`
/// (keep the arm). A `None` upper bound (unsigned-64) only excludes on the low side.
pub(crate) fn value_excludes(db: &mut Db, id: StructId, c: i64) -> bool {
    match value_range(db, id) {
        Some((lo, hi)) => c < lo || hi.is_some_and(|h| c > h),
        None => false,
    }
}

/// Structurally compare two CONSTANT compound values at `a`/`b`, returning `Some(true/false)` if BOTH are
/// compile-time-visible constants (a `SumNew`/`Tuple`/`Record`/`ListNew`, or a scalar leaf), else `None`
/// (a runtime operand — the caller declines, deferring to the heap walk). Equality is STRUCTURAL
/// (`core-semantics.md §Equality Is Structural`): two values are equal iff same shape + component-wise
/// equal. A `SumNew` compares its discriminant then its payloads pairwise; a `Tuple`/`ListNew` its
/// elements pairwise (unequal length → not equal); a `Record` its fields (the field SET is fixed by the
/// type, so same-typed records share keys — compare each). Scalar leaves compare by value. Two DIFFERENT
/// compound KINDS (a tuple vs a sum) never fold here — the type checker rejects a cross-shape `=` before
/// lowering, so a kind mismatch reaching here is a compiler bug → `None` (decline).
pub(crate) fn const_compound_eq(db: &mut Db, a: StructId, b: StructId) -> Option<bool> {
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => Some(x.eq_value(&y)),
        (Core::ConstBool(x), Core::ConstBool(y)) => Some(x == y),
        (Core::ConstStr(x), Core::ConstStr(y)) => Some(x == y),
        // Two chars: equal iff their scalar values match.
        (Core::ConstChar(x), Core::ConstChar(y)) => Some(x == y),
        // Two floats: equal iff their canonical Float64 BITS match — so a nested `-0.0` is distinct from
        // `0.0` (`(= (tuple -0.0) (tuple 0.0))` → false). By-bits, NOT `f64` `==`, precisely so `-0.0`/
        // `0.0` differ — the structural byte-form rule.
        (Core::ConstFloat(x), Core::ConstFloat(y)) => Some(x.to_f64_bits() == y.to_f64_bits()),
        // A nested NaN under the canonical byte form: every NaN equals every NaN (`(= (tuple nan) (tuple
        // nan))` → true, `(= (Some nan) (Some nan))` → true), and `nan` is unequal to any finite float —
        // the SAME rule the scalar `=` fold applies, recursed through a compound.
        (Core::ConstFloatNan, Core::ConstFloatNan) => Some(true),
        (Core::ConstFloatNan, Core::ConstFloat(_)) | (Core::ConstFloat(_), Core::ConstFloatNan) => {
            Some(false)
        }
        (Core::Unit, Core::Unit) => Some(true),
        // Two sum values: equal iff same discriminant AND equal payloads (pairwise). A different disc is
        // not-equal WITHOUT comparing payloads (`(Some 1)` ≠ `None`). Same disc ⇒ same variant ⇒ same
        // payload arity (the type fixes it), so a pairwise payload compare is well-formed.
        (
            Core::SumNew {
                disc: da,
                payloads: pa,
            },
            Core::SumNew {
                disc: db_,
                payloads: pb,
            },
        ) => {
            if da != db_ {
                return Some(false);
            }
            if pa.len() != pb.len() {
                return Some(false);
            }
            for (&x, &y) in pa.iter().zip(pb.iter()) {
                if !const_compound_eq(db, x, y)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        (Core::Tuple { elems: ea }, Core::Tuple { elems: eb })
        | (Core::ListNew { elems: ea }, Core::ListNew { elems: eb })
        // Two constant byte sequences: equal iff the same bytes in the same order — the SAME
        // element-wise compare as a tuple/list, since a `Core::BytesOf`'s elements are constant `ConstInt`
        // bytes (`0..=255`, range-checked at `lower_bytes_of`). `(= (Bytes.of (list 1 2)) (Bytes.of (list
        // 1 2)))` → true; a different length or a differing byte → false. This is what folds the corpus's
        // `(= (Bytes.concat …) (Bytes.of …))` / `(= (Bytes.compact …) …)` witnesses (concat/compact
        // already fold to a constant `Core::BytesOf`, so both operands reach here constant).
        | (Core::BytesOf { elems: ea }, Core::BytesOf { elems: eb }) => {
            if ea.len() != eb.len() {
                return Some(false);
            }
            for (&x, &y) in ea.iter().zip(eb.iter()) {
                if !const_compound_eq(db, x, y)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        (Core::Record { fields: fa }, Core::Record { fields: fb }) => {
            if fa.len() != fb.len() {
                return Some(false);
            }
            // Same-typed records share the field SET; compare each field's value by key. A key present in
            // one but not the other (a shape mismatch the type checker would have caught) is not-equal.
            for (key, &va) in fa.iter() {
                match fb.get(key) {
                    Some(&vb) => {
                        if !const_compound_eq(db, va, vb)? {
                            return Some(false);
                        }
                    }
                    None => return Some(false),
                }
            }
            Some(true)
        }
        // Two constant SETS — equal iff same size AND every element of one is present in the other (by
        // VALUE, order-independent — a set is unordered; collections-and-text.md §A Set Is A Collection Of
        // Unique Elements: two sets are equal when they contain equal elements, independent of order). Both
        // are already dedup'd (the `Set.of`/insert folds), so equal size + one-way containment suffices.
        (Core::SetOf { elems: ea, .. }, Core::SetOf { elems: eb, .. }) => {
            if ea.len() != eb.len() {
                return Some(false);
            }
            for &x in &ea {
                if !set_has_const_elem(db, &eb, x) {
                    return Some(false);
                }
            }
            Some(true)
        }
        // Two constant MAPS — equal iff same size AND every entry `k ↦ v` of one has a KEY-EQUAL entry in
        // the other whose VALUE is equal (collections-and-text.md §Two Maps Are Equal When They Associate
        // The Same Keys With Equal Values — order-independent, by value, exactly like a set but with a
        // value per key). A map's KEY SET is runtime data, NOT its type, so two maps of DIFFERENT key sets
        // are the SAME `Map<K,V>` type and compare `false` here (not a type error) — this is what folds `(=
        // (map ("a" 1)) (map ("b" 2)))` → false AND lets a `(list (map …) (map …))` element-compare fold
        // (the recursion that was declining). Entries are already dedup'd (the `map` literal / insert folds
        // keep one entry per key), so equal size + one-way key-and-value containment suffices.
        (Core::MapNew { entries: ea, .. }, Core::MapNew { entries: eb, .. }) => {
            if ea.len() != eb.len() {
                return Some(false);
            }
            for &(ka, va) in &ea {
                // Find `ka` among `eb`'s keys (by value). Track whether any key comparison was AMBIGUOUS
                // (a non-const nested key → `None`): if the key is not found AND every comparison was a
                // definite `Some(false)`, the key is genuinely ABSENT, so — sizes being equal — the maps
                // differ (`false`); but if a comparison was ambiguous we cannot conclude absence, so decline.
                let mut value_at_key = None;
                let mut ambiguous_key = false;
                for &(kb, vb) in &eb {
                    match const_compound_eq(db, ka, kb) {
                        Some(true) => {
                            value_at_key = Some(vb);
                            break;
                        }
                        Some(false) => {}
                        None => ambiguous_key = true,
                    }
                }
                match value_at_key {
                    Some(vb) => match const_compound_eq(db, va, vb)? {
                        true => {}                        // key present, value equal — continue
                        false => return Some(false),      // key present, value differs
                    },
                    None if ambiguous_key => return None, // couldn't rule out the key being present
                    None => return Some(false),           // key genuinely absent → maps differ
                }
            }
            Some(true)
        }
        // Any other pairing includes a runtime operand (not a constant compound) — decline the fold.
        _ => None,
    }
}

/// The CANONICAL KEY ORDER between two CONSTANT map keys `a`/`b` — the deterministic order a map's
/// entries render in (collections-and-text.md §A Map Renders As Its Entries In Canonical Key Order). The
/// compiler owns the sort (the runtime iterates hash order; the canonical form sorts by the key's value).
/// Scalar keys order by VALUE: an integer by numeric value, a string lexicographically, a bool false<true,
/// unit is a singleton. `None` when a key is not a compile-time-orderable constant (a nested compound key,
/// or a runtime key) — the caller then declines the constant escape (the runtime walker is deferred).
fn const_key_order(db: &mut Db, a: StructId, b: StructId) -> Option<std::cmp::Ordering> {
    match (core_of(db, a), core_of(db, b)) {
        // Compare by numeric value. `to_i64` covers the ≤64-bit keys the corpus uses; a wider key
        // declines the constant sort (deferred to the runtime walker).
        (Core::ConstInt(x), Core::ConstInt(y)) => Some(x.to_i64()?.cmp(&y.to_i64()?)),
        (Core::ConstStr(x), Core::ConstStr(y)) => Some(x.cmp(&y)),
        (Core::ConstBool(x), Core::ConstBool(y)) => Some(x.cmp(&y)),
        (Core::Unit, Core::Unit) => Some(std::cmp::Ordering::Equal),
        // A nested-compound or runtime key has no compile-time canonical order here — decline.
        _ => None,
    }
}

/// Whether the operand at `id` has a type the runtime `value-eq` heap walk (`champ_eq`) compares
/// CORRECTLY — a compound whose leaves are all SCALAR (Int/Bool/Unit), reached through tuples, records,
/// and sum variants. Such a value is CANONICAL BY CONSTRUCTION: it holds no embedded RRB vector, CHAMP
/// map/set, or Bytes/String rope, each of whose byte form is canonical only AFTER a compaction the
/// compiler would have to emit first (`deterministic-value-form.md` §A Value Has One Canonical Byte
/// Form). Restricting the runtime `=` to this class keeps the walk EXACT without that extra machinery;
/// a compound carrying a collection/text leaf still declines (a later increment). Recurses structurally:
/// a sum is walkable iff EVERY variant's payload type is (read via `payload_ty_at_instantiation`, so a
/// generic sum's payload is checked at its actual instantiation). A cyclic type (a recursive sum whose
/// payload mentions itself — a cons list) terminates because the recursion is bounded by the DISTINCT
/// declaration occurrences visited, tracked in `seen`.
fn compound_eq_heap_walkable(db: &mut Db, id: StructId) -> bool {
    let ty = crate::infer::type_of(db, id);
    ty_heap_walkable(db, &ty, &mut Vec::new())
}

/// The type-level recursion behind [`compound_eq_heap_walkable`]. `seen` holds the sum declarations
/// currently on the recursion stack, so a recursive sum (`(type IntList (Cons (Tuple Int64 IntList))
/// Nil)`) does not loop: re-entering a decl already in progress is treated as walkable (its scalar-leaf
/// obligation is discharged by the OTHER variants / the outer visit — a purely self-referential cycle
/// carries no non-scalar leaf of its own).
fn ty_heap_walkable(db: &mut Db, ty: &crate::ty::Ty, seen: &mut Vec<StructId>) -> bool {
    use crate::ty::Ty;
    match ty {
        // Scalar leaves — the base case the walk compares directly (equal canonical raw bytes).
        Ty::Int(_) | Ty::Bool | Ty::Unit => true,
        // An UNCONSTRAINED type variable — a PHANTOM parameter no value in this comparison instantiates.
        // It arises for a SIBLING variant of a multi-parameter sum whose param the compared values do not
        // use: `(= (Ok 6) (Ok 6))` types the operand `Result Int64 ?b` (the `Err` parameter `b` is free —
        // no `Err` value exists here), so walking `Result`'s variants reaches `Err`'s payload `?b`. A bare
        // unconstrained var is SCALAR-SAFE for the walk: it can only ground to a phantom (unit-like) type,
        // NEVER to a concrete non-canonical leaf — a `List`/`Bytes`/`String` is a concrete `Ty` (it reaches
        // the arm below), and a var CONSTRAINED to a collection is substituted to that concrete `Ty` before
        // reaching here. Treating it as walkable admits only the genuinely-phantom case; rejecting it was
        // over-conservative and declined `(= (Ok x) (Ok 6))` though the compared `Ok` values ARE walkable.
        Ty::Var(_) => true,
        // A tuple/record — walkable iff every element/field is. (An empty tuple is unit, trivially so.)
        Ty::Tuple(elems) => {
            let elems: Vec<Ty> = elems.to_vec();
            elems.iter().all(|e| ty_heap_walkable(db, e, seen))
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().all(|v| ty_heap_walkable(db, v, seen))
        }
        // A sum — walkable iff every variant's payload type is. A recursive sum is broken by `seen`.
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return true;
            }
            seen.push(*decl);
            let Some(variant_count) = db.type_decl_by_occ(*decl).map(|t| t.variants.len()) else {
                seen.pop();
                return false;
            };
            let mut ok = true;
            for disc in 0..variant_count {
                let ctor = db
                    .type_decl_by_occ(*decl)
                    .and_then(|t| t.variants.get(disc))
                    .and_then(|v| v.ctor);
                // A NULLARY variant (no ctor arrow → no payload) carries only its discriminant — a scalar
                // leaf, walkable. A payload-carrying variant's payload type must itself be walkable.
                if let Some(ctor) = ctor
                    && let Some(payload_ty) =
                        crate::infer::payload_ty_at_instantiation(db, ctor, ty)
                    && !ty_heap_walkable(db, &payload_ty, seen)
                {
                    ok = false;
                    break;
                }
            }
            seen.pop();
            ok
        }
        // A MAP handle is CANONICAL by construction (the runtime CHAMP is order-independent — equal maps
        // are byte-identical under `champ_eq`/`champ_hash`), so two runtime maps compare correctly by the
        // `value-eq` structural walk — walkable iff its KEY and VALUE types are themselves walkable (their
        // canonical forms are stable). This is what makes map equality independent of insertion order AND
        // independent of const-fold-vs-runtime construction (both build the same canonical CHAMP handle),
        // and makes two maps with different key SETS compare `false` (well-typed, same `Map<K,V>` type) —
        // NOT a rejection. A key/value whose canonical form needs machinery not yet emitted (Bytes rope,
        // String, a nested collection) makes the map decline, deferring to that increment.
        Ty::Map(k, v) => {
            // An `Any` key/value is a DEFERRED type of an EMPTY map (`(map)` — no entries, so its key/
            // value never determined), a PHANTOM exactly like a `Var`: an empty map carries no key or
            // value to compare, so the walk never reaches a concrete non-canonical leaf through it. Treat
            // it as walkable (else `(= (map) (map (1 10)))` — comparing an empty map to a one-entry one,
            // which MUST yield `false` — would decline). A CONCRETE key/value (Int/String/…) is checked
            // normally; a genuinely non-canonical one (a nested collection, a Bytes rope) still declines.
            let (k, v) = ((**k).clone(), (**v).clone());
            let key_ok = matches!(k, Ty::Any) || ty_heap_walkable(db, &k, seen);
            let val_ok = matches!(v, Ty::Any) || ty_heap_walkable(db, &v, seen);
            key_ok && val_ok
        }
        // A SET handle is CANONICAL by construction (the same order-independent CHAMP as a map), so two
        // runtime sets compare correctly by the `value-eq` walk — walkable iff its ELEMENT type is. An
        // `Any` element is the deferred type of an EMPTY set (`(Set.of (list))` — no elements), a phantom
        // like a map's `Any` key/value, so treat it as walkable (else `(= (Set.of (list)) (Set.of (list
        // 1)))`, which MUST yield `false`, would decline). The map analogue, one axis instead of two.
        Ty::Set(elem) => {
            let elem = (**elem).clone();
            matches!(elem, Ty::Any) || ty_heap_walkable(db, &elem, seen)
        }
        // A STRING is a flat UTF-8 byte LEAF, CANONICAL by construction — every runtime String rep is a
        // flat leaf (`str-new`, or the compiler's `bytes-alloc`+`bytes-set` build, both `alloc(Vec::new(),
        // bytes)`), NEVER a rope: `String.concat` declines for a runtime/non-ASCII operand (only a constant
        // ASCII pair folds), so no non-canonical string reaches here. So two runtime strings (and two
        // String-keyed maps) compare correctly by `value-eq`'s raw-byte walk (`champ_eq`). The type checker
        // unifies both `=` operands to `String` before lowering, so a String is only ever compared against
        // a String (never a Bytes of the same bytes). CONTRAST `Ty::Bytes` below (NON-walkable): a
        // `Bytes.concat` DOES emit a non-canonical rope (`bytes-concat`), whose byte form is canonical only
        // after a `bytes-compact` the compiler would first have to emit — so a runtime Bytes still declines.
        Ty::String => true,
        // A SYMBOL is a nominal over a flat UTF-8 String leaf — canonical by construction (its identity
        // is content-derived), so two symbols compare correctly by the raw-byte walk, exactly like the
        // `String` it wraps. (This increment folds constant symbol equality; a runtime symbol reaching
        // `=` compares by the same walk.)
        Ty::Symbol => true,
        // A nominal — walkable iff its underlying value is (the tag is erased at run time, so the walk
        // compares the underlying values directly). A recursive nominal is impossible here (a recursive
        // single-variant sum is never erased — it stays a `Ty::Sum`, handled above).
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            ty_heap_walkable(db, &inner, seen)
        }
        // A quantity ERASES to its inner numeric type, so it is walkable iff that inner type is — a `(Qty
        // Int64 u)` compares by its erased `Int64`. (A quantity `=` folds at compile time in Layer 1, but
        // classifying it by the inner type keeps this correct if a runtime quantity ever reaches `=`.)
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_heap_walkable(db, &inner, seen)
        }
        // A collection / bytes-rope / char / float / function / type-value / unresolved leaf is NOT
        // walkable here (its canonical form needs machinery this increment does not emit, or it is not a
        // runtime value that reaches a compound equality — `Ty::Type`/`Ty::Any`/`Ty::Fn` never cross `=`).
        // A `Char` has no runtime machine rep yet (its equality folds at compile time). `Bytes` can be a
        // non-canonical rope (see the `String` note above), so it declines until `bytes-compact`-on-compare.
        // A `BigInt` will be canonical-byte-form-walkable once its runtime leaf exists (B3) — B0 adds the
        // type only and constructs none, so it declines here for now (a constant `BigInt` `=` folds in the
        // compiler at B1; a runtime `BigInt` `=` is wired with the runtime limb library).
        Ty::List(_)
        | Ty::Bytes
        | Ty::Char
        | Ty::BigInt
        | Ty::Float(_)
        | Ty::Fn(_, _)
        | Ty::Type
        | Ty::Any => false,
    }
}

/// Lower a CONVERSION application (`T.wrap`). Truncating: keeps the low `N` bits of the operand at the
/// TARGET width/signedness (`Prim::Wrap`), NEVER traps. The target type is the CONVERSION NODE's own
/// solved type (`type_of(db, id)`), read here so the fold and the runtime path agree on the width. A
/// constant operand FOLDS via `IntValue::wrap_to` to a `ConstInt` already at the target width; a runtime
/// operand becomes a `Core::Convert` the backend emits as a mask-and-reinterpret. A poison propagates.
/// Lower a sum variant CONSTRUCTOR application `(Option.Some 5)`. The discriminant is read off the
/// head's `(meta variant)` channel; the args are the payloads (an empty payload for a nullary variant,
/// which normally reaches here bare — handled in the `Resolved::Record` arm — but an explicit `(None)`
/// application is fine too). Produces `Core::SumNew` the backend builds as `sum-new(disc, payload)`.
fn lower_sum_new(db: &mut Db, head: StructId, args: &[StructId]) -> Core {
    let Some(disc) = crate::eval::variant_disc_of(db, head) else {
        return Core::Poison(Reject::decline(
            "a sum constructor has no discriminant metadata",
        ));
    };
    // NEWTYPE ERASURE: if the constructor's owning sum is an erasable NEWTYPE (a single-variant sum), the
    // value IS its payload — NO `sum-new` box, no discriminant (`type-system.md §156`, the tag adds
    // nothing to the runtime representation). Emit the payload directly: 0 payloads → unit, 1 → the
    // payload's core, n → the payload TUPLE (the same shape a multi-payload variant already boxes, now
    // the value itself). The result node's TYPE is `Ty::Nominal`, so `valtype_of` reads its underlying
    // slot — the erased core and the declared type agree.
    if let Some(decl) = crate::eval::variant_owner_decl(db, head)
        && db.newtype_inner.contains_key(&decl)
    {
        return match args.len() {
            0 => Core::Unit,
            1 => core_of(db, args[0]),
            _ => Core::Tuple {
                elems: args.to_vec(),
            },
        };
    }
    // A NULLARY variant is CONSTRUCTED by applying it to the unit value — `(None unit)` / `(Nil ())` —
    // the canonical form (core-semantics.md §Construction MUST Be Via Application: "(None unit)"; a
    // nullary variant carries unit). Its ctor `(meta t)` is the bare sum (no arrow → `variant_payload_type`
    // is `None`), so the single `unit` argument is NOT a payload — the payload of a nullary variant IS the
    // unit value, built as an empty array by the backend (`SumNew` with no payloads). Drop the unit arg so
    // it is not boxed as a spurious payload. (A bare `None` used as a value takes the no-arg path directly.)
    if crate::eval::variant_payload_type(db, head).is_none() && args.len() == 1 {
        // The argument must BE the unit value — a nullary variant applied to a non-unit is an arity error
        // the type-checker reports; here, lower it as the nullary construction (the type fault surfaces in
        // `type_errors`, and an over-payloaded nullary is caught there, not silently given a payload).
        return Core::SumNew {
            disc,
            payloads: Vec::new(),
        };
    }
    Core::SumNew {
        disc,
        payloads: args.to_vec(),
    }
}

/// The `(Some-discriminant, None-discriminant)` of the `Option` sum that is the type at `id` (a
/// `List.at`/fallible-access node's result). Reads the sum's declaration by its `decl` occurrence and
/// finds the `Some`/`None` variant positions (a variant's index in the decl IS its discriminant).
/// `None` if the type is not a two-variant `Some`/`None` sum — a fallible-access result is always the
/// built-in `Option`, so a non-Option here is a compiler bug and the caller declines.
fn option_discs(db: &mut Db, id: StructId) -> Option<(u32, u32)> {
    let crate::ty::Ty::Sum { decl, .. } = crate::infer::type_of(db, id) else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(decl)?;
    let mut some_disc = None;
    let mut none_disc = None;
    for (i, v) in decl_ref.variants.iter().enumerate() {
        match v.name.as_str() {
            "Some" => some_disc = Some(i as u32),
            "None" => none_disc = Some(i as u32),
            _ => {}
        }
    }
    Some((some_disc?, none_disc?))
}

/// Lower `(List.at list index)` — the fallible indexed read. FOLD when the `list` operand is a
/// compile-time-visible list literal AND the `index` folds to a constant: an in-range index (`0 <= i <
/// arity`) yields `(Some elem)` — a `Core::SumNew` of the element's core at the `Some` discriminant —
/// and an out-of-range index (negative or `>= arity`) yields `None` (`Core::SumNew` with no payloads at
/// the `None` discriminant). Both fold to the ordinary sum construction, so a constant `List.at` renders
/// through the sum escape/fold with no heap. Otherwise emit the runtime `Core::ListAt` (a bounds-checked
/// `vec-get`). A poison list/index propagates.
fn lower_list_at(db: &mut Db, id: StructId, list: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, list) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "List.at result is not the built-in Option sum",
        ));
    };
    // FOLD a constant list literal indexed by a constant integer.
    if let (Core::ListNew { elems }, Core::ConstInt(i)) = (core_of(db, list), core_of(db, index)) {
        // The index is a signed Int64; a negative value or one `>= arity` is out of bounds → `None`.
        match i.to_i64() {
            Some(n) if n >= 0 && (n as usize) < elems.len() => {
                trace!(target: "rcdzc::fold", node = id.0, index = n, "List.at folds to Some (in-bounds constant index)");
                return Core::SumNew {
                    disc: disc_some,
                    payloads: vec![elems[n as usize]],
                };
            }
            _ => {
                trace!(target: "rcdzc::fold", node = id.0, "List.at folds to None (out-of-bounds constant index)");
                return Core::SumNew {
                    disc: disc_none,
                    payloads: Vec::new(),
                };
            }
        }
    }
    // A runtime list or runtime index — emit the bounds-checked runtime read.
    Core::ListAt {
        list,
        index,
        disc_some,
        disc_none,
    }
}

/// The solved KEY and VALUE types of a map operand — `(k, v)` from its `Ty::Map(k, v)`. `None` if the
/// operand's type is not a map (a poison or an unsolved type), so the caller declines rather than
/// guessing a box op. Used by the `Map.*` lowerings to pick the key/value box ops.
fn map_kv_types(db: &mut Db, map: StructId) -> Option<(crate::ty::Ty, crate::ty::Ty)> {
    match crate::infer::type_of(db, map) {
        crate::ty::Ty::Map(k, v) => Some((*k, *v)),
        _ => None,
    }
}

/// The solved ELEMENT type of a set operand — `T` from its `Ty::Set(T)`. `None` if not a solved set type.
fn set_elem_type(db: &mut Db, set: StructId) -> Option<crate::ty::Ty> {
    match crate::infer::type_of(db, set) {
        crate::ty::Ty::Set(elem) => Some(*elem),
        _ => None,
    }
}

/// Whether the CONSTANT elements of a set (a `Core::SetOf`) contain one `const_compound_eq` to `elem`.
/// Used by the const folds (contains / insert-dedup). Both must be compile-time-visible constants.
fn set_has_const_elem(db: &mut Db, elems: &[StructId], elem: StructId) -> bool {
    elems
        .iter()
        .any(|&e| const_compound_eq(db, e, elem) == Some(true))
}

/// The canonical, HASHABLE identity of a SCALAR constant value at `id` — `None` for a compound (a
/// tuple/record/sum/list/…) or a runtime value. Reproduces `const_compound_eq`'s SCALAR-leaf equality
/// EXACTLY, so two scalar constants share a token iff `const_compound_eq` on them is `Some(true)`: an int
/// by its trimmed, sign-normalized magnitude (matching `IntValue::eq_value`), a float by canonical
/// `Float64` bits (so `-0.0` ≠ `0.0`), NaN a singleton distinct from every finite float, string/bool/char
/// by value, unit a singleton. This is the O(1) basis of the LINEAR set dedup: a scalar element hashes to
/// its token and dedups in one hash-set probe, replacing an O(elements²) pairwise `const_compound_eq`
/// scan that re-derived and deep-cloned each element's `Core` on every comparison.
#[derive(PartialEq, Eq, Hash)]
enum ScalarKey {
    Int { negative: bool, magnitude: Vec<u8> },
    Bool(bool),
    Str(String),
    Char(char),
    FloatBits(u64),
    FloatNan,
    Unit,
}

fn scalar_const_key(db: &mut Db, id: StructId) -> Option<ScalarKey> {
    match core_of(db, id) {
        Core::ConstInt(v) => {
            // Trim leading zero bytes + normalize a zero's sign — the SAME canonicalization
            // `IntValue::eq_value` applies, so equal tokens ⟺ `eq_value` true.
            let start = v.magnitude.iter().take_while(|&&b| b == 0).count();
            let magnitude = v.magnitude[start..].to_vec();
            let negative = !magnitude.is_empty() && v.negative;
            Some(ScalarKey::Int {
                negative,
                magnitude,
            })
        }
        Core::ConstBool(b) => Some(ScalarKey::Bool(b)),
        Core::ConstStr(s) => Some(ScalarKey::Str(s)),
        Core::ConstChar(c) => Some(ScalarKey::Char(c)),
        Core::ConstFloat(d) => Some(ScalarKey::FloatBits(d.to_f64_bits())),
        Core::ConstFloatNan => Some(ScalarKey::FloatNan),
        Core::Unit => Some(ScalarKey::Unit),
        // A compound (tuple/record/sum/list/set/map/bytes) or a runtime value has no scalar token — the
        // caller keeps it on the O(compounds²) pairwise path (compounds are rare + small).
        _ => None,
    }
}

/// Lower `(Set.of list)` — construct a set from a list, DEDUPLICATING. When the `list` operand is a
/// compile-time-visible `Core::ListNew`, fold to a canonical constant `Core::SetOf` (dropping later
/// duplicates by value — `const_compound_eq`), so it bakes/compares/renders as a constant set; a runtime
/// list emits `Core::SetOf` over the list's elements (a later increment reads a runtime list — declines
/// for now). The element type comes from the RESULT node's own solved `Ty::Set`. A poison propagates.
fn lower_set_of(db: &mut Db, id: StructId, list: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, list) {
        return Core::Poison(r);
    }
    let Some(elem_ty) = (match crate::infer::type_of(db, id) {
        crate::ty::Ty::Set(e) => Some(*e),
        _ => None,
    }) else {
        return Core::Poison(Reject::decline("Set.of result is not a solved set type"));
    };
    match core_of(db, list) {
        // A constant list → a canonical DEDUP'd constant set. Keep the FIRST occurrence of each element
        // value (order-independent equality means which copy is kept is unobservable; the render sorts).
        // LINEAR for scalar elements (the corpus shape): a scalar's canonical `ScalarKey` dedups in one
        // hash-set probe. A COMPOUND element (rare) has no scalar token, so it falls back to the pairwise
        // `const_compound_eq` against only the OTHER kept compounds (`compounds` list). This replaced an
        // O(elements²) `set_has_const_elem` scan over ALL kept elements (each comparison re-cloning a
        // `Core`) — a `(Set.of (list 0 1 … N))` of N distinct ints was quadratic (N=3200 spent ~82% of
        // the compile in `const_compound_eq`); the scalar fast path makes it linear.
        Core::ListNew { elems } => {
            let mut deduped: Vec<StructId> = Vec::with_capacity(elems.len());
            let mut seen_scalars: crate::fxhash::FxHashSet<ScalarKey> =
                crate::fxhash::FxHashSet::default();
            let mut compounds: Vec<StructId> = Vec::new();
            for &e in &elems {
                match scalar_const_key(db, e) {
                    Some(key) => {
                        if seen_scalars.insert(key) {
                            deduped.push(e);
                        }
                    }
                    // A compound element: dedup against the other kept compounds only (a scalar can never
                    // equal a compound, so cross-checking is unnecessary — `const_compound_eq` of two
                    // different kinds is `None`/`Some(false)`).
                    None => {
                        if !set_has_const_elem(db, &compounds, e) {
                            compounds.push(e);
                            deduped.push(e);
                        }
                    }
                }
            }
            trace!(target: "rcdzc::fold", node = id.0, elems = deduped.len(), "Set.of folds a constant list to a canonical set");
            Core::SetOf {
                elems: deduped,
                elem_ty,
            }
        }
        // A runtime list source — building a set from a runtime list needs a runtime dedup loop (a later
        // increment); decline cleanly (a constant list is the corpus shape).
        _ => Core::Poison(Reject::decline(
            "Set.of over a runtime list is not yet built (a constant list literal only)",
        )),
    }
}

/// Lower `(Set.contains set elem)` — the total membership predicate. FOLD a constant set (`Core::SetOf`)
/// with a constant element to `ConstBool` (by value, `const_compound_eq`), else emit the runtime
/// `Core::SetContains` (a `bool`). A poison propagates.
fn lower_set_contains(db: &mut Db, set: StructId, elem: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, set) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, elem) {
        return Core::Poison(r);
    }
    if let Core::SetOf { elems, .. } = core_of(db, set)
        && is_const_value(db, elem)
    {
        let present = set_has_const_elem(db, &elems, elem);
        trace!(target: "rcdzc::fold", result = present, "Set.contains folds against a constant set");
        return Core::ConstBool(present);
    }
    let Some(elem_ty) = set_elem_type(db, set) else {
        return Core::Poison(Reject::decline(
            "Set.contains operand is not a solved set type",
        ));
    };
    Core::SetContains { set, elem, elem_ty }
}

/// Lower `(Set.insert set elem)` / `(Set.remove set elem)`. FOLD onto a constant set (`Core::SetOf`) when
/// the element is constant: insert appends (no-op if already present, by value); remove drops the matching
/// element (no-op if absent). Else emit the runtime `Core::SetInsert`/`Core::SetRemove`. The element type
/// comes from the RESULT node's solved `Ty::Set`. A poison propagates.
fn lower_set_insert_remove(
    db: &mut Db,
    prim: crate::resolved::Prim,
    set: StructId,
    elem: StructId,
) -> Core {
    use crate::resolved::Prim;
    if let Core::Poison(r) = core_of(db, set) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, elem) {
        return Core::Poison(r);
    }
    let is_insert = prim == Prim::SetInsert;
    if let Core::SetOf { elems, elem_ty } = core_of(db, set)
        && is_const_value(db, elem)
    {
        let mut out: Vec<StructId> = elems.to_vec();
        if is_insert {
            if !set_has_const_elem(db, &out, elem) {
                out.push(elem); // add-if-absent (a present element is a no-op value)
            }
        } else {
            out.retain(|&e| const_compound_eq(db, e, elem) != Some(true)); // drop the matching element
        }
        trace!(target: "rcdzc::fold", elems = out.len(), insert = is_insert, "Set.insert/remove folds onto a constant set");
        return Core::SetOf {
            elems: out,
            elem_ty,
        };
    }
    let Some(elem_ty) = set_elem_type(db, set) else {
        return Core::Poison(Reject::decline(
            "Set.insert/remove operand is not a solved set type",
        ));
    };
    if is_insert {
        Core::SetInsert { set, elem, elem_ty }
    } else {
        Core::SetRemove { set, elem, elem_ty }
    }
}

/// Lower `(Set.union a b)` / `intersection` / `difference`. FOLD two constant sets (`Core::SetOf`) to a
/// canonical constant result set (by-value element algebra, `const_compound_eq`); else emit the runtime
/// `Core::SetAlgebra`. A poison propagates.
fn lower_set_algebra(
    db: &mut Db,
    prim: crate::resolved::Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    use crate::core::SetAlgebraOp;
    use crate::resolved::Prim;
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let op = match prim {
        Prim::SetUnion => SetAlgebraOp::Union,
        Prim::SetIntersection => SetAlgebraOp::Intersection,
        _ => SetAlgebraOp::Difference,
    };
    if let (Core::SetOf { elems: a, elem_ty }, Core::SetOf { elems: b, .. }) =
        (core_of(db, lhs), core_of(db, rhs))
    {
        let out: Vec<StructId> = match op {
            // union: a's elements, then b's elements not already present.
            SetAlgebraOp::Union => {
                let mut out = a.to_vec();
                for &e in &b {
                    if !set_has_const_elem(db, &out, e) {
                        out.push(e);
                    }
                }
                out
            }
            // intersection: a's elements that are also in b.
            SetAlgebraOp::Intersection => a
                .iter()
                .copied()
                .filter(|&e| set_has_const_elem(db, &b, e))
                .collect(),
            // difference: a's elements NOT in b.
            SetAlgebraOp::Difference => a
                .iter()
                .copied()
                .filter(|&e| !set_has_const_elem(db, &b, e))
                .collect(),
        };
        trace!(target: "rcdzc::fold", ?op, elems = out.len(), "set-algebra folds two constant sets");
        return Core::SetOf {
            elems: out,
            elem_ty,
        };
    }
    Core::SetAlgebra { op, lhs, rhs }
}

/// Lower `(Map.insert map key val)` — add-or-replace, returning the new map. For M1 this emits the
/// runtime `Core::MapInsert` on a runtime map operand (a constant-map fold is a later increment). The
/// key/value types come from the map operand's `Ty::Map` (they choose the box ops). A poison propagates.
fn lower_map_insert(db: &mut Db, id: StructId, args: &[StructId]) -> Core {
    let (map, key, val) = (args[0], args[1], args[2]);
    for &a in &[map, key, val] {
        if let Core::Poison(r) = core_of(db, a) {
            return Core::Poison(r);
        }
    }
    // The key/value types come from the INSERT NODE's own solved type `Map k v` (the RESULT map),
    // which unification has fully determined — NOT from the map OPERAND, whose isolated type may still
    // be `Map ?0 ?1` for a bare `Map.empty` (its key/value are solved only via this insert's arguments).
    let Some((key_ty, val_ty)) = map_kv_types(db, id) else {
        return Core::Poison(Reject::decline(
            "Map.insert result is not a solved map type",
        ));
    };
    // FOLD onto a compile-time-visible constant map when the KEY is a constant (its value need not be —
    // the value occurrence carries over regardless, exactly as `List.push` folds onto a constant list).
    // Add-or-REPLACE by key VALUE: an existing entry whose key is `const_compound_eq` to the new key has
    // its value replaced IN PLACE (preserving position — the each-key-at-most-once rule); otherwise the
    // entry is appended. The result is a constant `Core::MapNew` that bakes at escape / renders sorted /
    // compares by `value-eq`, so a chain `(Map.insert (Map.insert Map.empty 2 20) 1 10)` folds to one
    // canonical two-entry map. A runtime map operand or a runtime key stays a `Core::MapInsert` (the
    // persistent CHAMP op). Keys compared by VALUE (`const_compound_eq`), so two names bound to the same
    // value collapse here just as they do at run time.
    if let (Core::MapNew { entries, .. }, true) = (core_of(db, map), is_const_value(db, key)) {
        let mut merged = entries.clone();
        let mut replaced = false;
        for e in merged.iter_mut() {
            if const_compound_eq(db, e.0, key) == Some(true) {
                *e = (e.0, val); // replace the value at this key (keep the key occurrence + position)
                replaced = true;
                break;
            }
        }
        if !replaced {
            merged.push((key, val));
        }
        trace!(target: "rcdzc::fold", node = id.0, entries = merged.len(), "Map.insert folds onto a constant map");
        return Core::MapNew {
            entries: merged,
            key_ty,
            val_ty,
        };
    }
    Core::MapInsert {
        map,
        key,
        val,
        key_ty,
        val_ty,
    }
}

/// Lower a MAP PATTERN binder reference — read from the (constant) scrutinee by key. `key = Some(k)` is
/// a VALUE binder at key `k` → the entry's value core; `key = None` is the REST binder → a `Core::MapNew`
/// with the `named` keys removed. Only a CONSTANT `Core::MapNew` scrutinee folds (the corpus shape: an
/// inline `Map.insert` chain); a runtime scrutinee declines (the runtime key-directed matcher is a later
/// increment). The arm was already SELECTED by `lower_match_map` (which ran the same key-presence probe),
/// so a value binder's key IS present here; a defensive miss declines rather than miscompiling.
fn lower_map_field(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    key: Option<StructId>,
    named: &[StructId],
) -> Core {
    let Core::MapNew { entries, .. } = core_of(db, scrutinee) else {
        return Core::Poison(Reject::decline(
            "a map pattern over a runtime map scrutinee is not yet matched (constant map only)",
        ));
    };
    match key {
        // A VALUE binder — the value at key `k` (keys compared by value, `const_compound_eq`).
        Some(k) => {
            for (ek, ev) in entries.iter() {
                if const_compound_eq(db, *ek, k) == Some(true) {
                    return core_of(db, *ev);
                }
            }
            Core::Poison(Reject::decline(
                "a map pattern value binder's key is absent from the constant map (arm mis-selected)",
            ))
        }
        // The REST binder — the map with every `named` key removed. Its key/value types come from this
        // binder node's own solved `Ty::Map` (the scrutinee's map type). Build a fresh constant `MapNew`.
        None => {
            let (key_ty, val_ty) = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Map(k, v) => (*k, *v),
                _ => (crate::ty::Ty::Any, crate::ty::Ty::Any),
            };
            let rest: Vec<(StructId, StructId)> = entries
                .iter()
                .filter(|(ek, _)| {
                    !named
                        .iter()
                        .any(|&nk| const_compound_eq(db, *ek, nk) == Some(true))
                })
                .copied()
                .collect();
            Core::MapNew {
                entries: rest,
                key_ty,
                val_ty,
            }
        }
    }
}

/// Whether the node at `id` lowers to a compile-time CONSTANT value — a constant scalar/string/float/
/// unit, or a constant compound (`SumNew`/`Tuple`/`Record`/`ListNew`/`MapNew`) all of whose parts are
/// constant. Used to decide whether a `Map.insert` key can fold (a constant key merges into a constant
/// map; a runtime key keeps the persistent op). Mirrors the constant test `const_value_ast` performs.
fn is_const_value(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstFloat(_)
        | Core::Unit => true,
        Core::Tuple { elems } | Core::ListNew { elems } => {
            elems.iter().all(|&e| is_const_value(db, e))
        }
        Core::SumNew { payloads, .. } => payloads.iter().all(|&p| is_const_value(db, p)),
        Core::Record { fields } => fields.values().all(|&v| is_const_value(db, v)),
        Core::MapNew { entries, .. } => entries
            .iter()
            .all(|&(k, v)| is_const_value(db, k) && is_const_value(db, v)),
        Core::SetOf { elems, .. } => elems.iter().all(|&e| is_const_value(db, e)),
        _ => false,
    }
}

/// Lower `(Map.lookup map key)` — the fallible keyed read → `(Option v)`. Emits the runtime
/// `Core::MapLookup` (a NULL-or-handle test building `Some`/`None`). The result Option's discriminants
/// are read off the node's result type; the key/value types off the map operand. A poison propagates.
fn lower_map_lookup(db: &mut Db, id: StructId, map: StructId, key: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, map) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, key) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Map.lookup result is not the built-in Option sum",
        ));
    };
    let Some((key_ty, val_ty)) = map_kv_types(db, map) else {
        return Core::Poison(Reject::decline(
            "Map.lookup operand is not a solved map type",
        ));
    };
    Core::MapLookup {
        map,
        key,
        key_ty,
        val_ty,
        disc_some,
        disc_none,
    }
}

/// Lower `(Map.remove map key)` — drop a key's association, returning the new map. Emits the runtime
/// `Core::MapRemove`. The key type comes from the map operand's `Ty::Map` (for the box op). A poison
/// propagates.
fn lower_map_remove(db: &mut Db, map: StructId, key: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, map) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, key) {
        return Core::Poison(r);
    }
    let Some((key_ty, _)) = map_kv_types(db, map) else {
        return Core::Poison(Reject::decline(
            "Map.remove operand is not a solved map type",
        ));
    };
    Core::MapRemove { map, key, key_ty }
}

/// Lower `(String.at string index)` — the fallible SCALAR-indexed read. FOLD when both operands are
/// constant: index the string by UNICODE SCALAR position (`chars().nth`, NOT byte offset —
/// collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values), yielding `(Some
/// "<char>")` in range (the ONE-scalar string at that position, a fresh `Core::ConstStr` synthesized
/// into the arena) and `None` out (negative, or `>=` the scalar length). Builds a `Core::SumNew` at the
/// result Option's Some/None discriminants, so it rides the ordinary sum fold/escape/match — no string
/// heap. A runtime string declines (the byte-rope indexed read is a later increment). A poison
/// operand propagates.
/// Lower `(Char.from-int n)` — the FALLIBLE integer→char conversion `Int64 → (Option Char)`. FOLD a
/// constant integer: a value that IS a Unicode scalar (in `0..=0x10FFFF`, not a surrogate `0xD800..=
/// 0xDFFF`) → `(Some #\c)` (a fresh `Leaf::Char` payload, the shape `String.at` uses for its scalar);
/// a surrogate / out-of-range integer → `(None unit)`. Never traps (`collections-and-text.md` §A Char
/// Converts To And From An Integer Totally). A runtime operand declines (no runtime char rep yet); a
/// poison propagates. `char::from_u32` performs the exact scalar-validity test.
fn lower_char_from_int(db: &mut Db, id: StructId, n: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, n) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Char.from-int result is not the built-in Option sum",
        ));
    };
    match core_of(db, n) {
        Core::ConstInt(v) => {
            // A scalar iff the value fits u32 AND `char::from_u32` accepts it (excludes surrogates and
            // > U+10FFFF). A negative or > u32 value is trivially not a scalar → None.
            let scalar = v
                .to_i64()
                .and_then(|i| u32::try_from(i).ok())
                .and_then(char::from_u32);
            match scalar {
                Some(c) => {
                    trace!(target: "rcdzc::fold", node = id.0, "Char.from-int folds to Some (a valid scalar)");
                    let payload = db.push_atom(crate::ast::Leaf::Char(c));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload],
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, "Char.from-int folds to None (surrogate / out-of-range)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new(),
                    }
                }
            }
        }
        _ => Core::Poison(Reject::decline(
            "Char.from-int on a runtime integer is not yet computed (constant integers only)",
        )),
    }
}

fn lower_str_at(db: &mut Db, id: StructId, string: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, string) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.at result is not the built-in Option sum",
        ));
    };
    match (core_of(db, string), core_of(db, index)) {
        (Core::ConstStr(s), Core::ConstInt(i)) => {
            // Index by scalar value; a negative index or one at/beyond the scalar length is out of range.
            let scalar = i.to_i64().and_then(|n| {
                if n >= 0 {
                    s.chars().nth(n as usize)
                } else {
                    None
                }
            });
            match scalar {
                Some(c) => {
                    // The one-scalar string at that position — a fresh `Leaf::Str` node whose `core_of`
                    // is `Core::ConstStr`, used as the `Some` payload (the same shape `List.at` uses,
                    // but the element is synthesized here since a string has no element sub-nodes).
                    trace!(target: "rcdzc::fold", node = id.0, "String.at folds to Some (in-bounds constant scalar index)");
                    let payload = db.push_atom(crate::ast::Leaf::Str(c.to_string()));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload],
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, "String.at folds to None (out-of-range constant index)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new(),
                    }
                }
            }
        }
        // A RUNTIME string (or runtime index) — walk the UTF-8 byte buffer to the i-th scalar's byte span
        // and slice it (`Core::StrAt`). A String is a flat UTF-8 byte leaf, so the backend scans scalar
        // starts (a byte is a scalar START iff `(b & 0xC0) != 0x80`), skips `index` scalars, and slices the
        // scalar's byte span into the `Some` payload — matching the const `chars().nth`. Guarded on the
        // string operand being a definite `Ty::String` (the index is any integer).
        _ if matches!(crate::infer::type_of(db, string), crate::ty::Ty::String) => Core::StrAt {
            string,
            index,
            disc_some,
            disc_none,
        },
        _ => Core::Poison(Reject::decline(
            "String.at needs a String operand (its runtime read walks the UTF-8 buffer)",
        )),
    }
}

/// Lower `(String.scalar-at string index)` — the fallible read of the CHAR (single Unicode scalar) at a
/// scalar position `String → Int64 → (Option Char)`. The char-typed companion of `String.at`: identical
/// index logic (by Unicode SCALAR position, `chars().nth`, not byte), but the `Some` payload is a
/// `Leaf::Char` (the scalar itself), so the result is `(Option Char)` — folds to `(Some #\c)` in range /
/// `(None unit)` out (negative or at/beyond the scalar length). A runtime string declines; a poison
/// operand propagates.
fn lower_str_scalar_at(db: &mut Db, id: StructId, string: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, string) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.scalar-at result is not the built-in Option sum",
        ));
    };
    match (core_of(db, string), core_of(db, index)) {
        (Core::ConstStr(s), Core::ConstInt(i)) => {
            let scalar = i.to_i64().and_then(|n| {
                if n >= 0 {
                    s.chars().nth(n as usize)
                } else {
                    None
                }
            });
            match scalar {
                Some(c) => {
                    // The scalar at that position — a fresh `Leaf::Char` node (`core_of` = `Core::ConstChar`),
                    // the `Some` payload. Distinct from `String.at`, whose payload is a one-scalar `Leaf::Str`.
                    trace!(target: "rcdzc::fold", node = id.0, "String.scalar-at folds to Some (in-bounds constant scalar index)");
                    let payload = db.push_atom(crate::ast::Leaf::Char(c));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload],
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, "String.scalar-at folds to None (out-of-range constant index)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new(),
                    }
                }
            }
        }
        _ => Core::Poison(Reject::decline(
            "String.scalar-at on a runtime string is not yet computed (constant strings only)",
        )),
    }
}

/// Lower `(String.slice string start end)` — the fallible SCALAR sub-range read, half-open `[start,
/// end)`. FOLD when all three operands are constant: cut the string by UNICODE SCALAR position (`chars`,
/// NOT byte offset — collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values). The
/// range is well-defined only when `0 <= start <= end <= scalar-len`: then `(Some "<substr>")` (a fresh
/// `Core::ConstStr` of the selected scalars — `start == end` yields the empty string, present not None);
/// any bound outside that (reversed `end < start`, over-long `end > len`, or negative) yields `(None
/// unit)`. Builds a `Core::SumNew` at the result Option's discriminants, riding the ordinary sum
/// fold/escape/match — no string heap. A runtime string declines; a poison operand propagates.
fn lower_str_slice(
    db: &mut Db,
    id: StructId,
    string: StructId,
    start: StructId,
    end: StructId,
) -> Core {
    for operand in [string, start, end] {
        if let Core::Poison(r) = core_of(db, operand) {
            return Core::Poison(r);
        }
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.slice result is not the built-in Option sum",
        ));
    };
    match (core_of(db, string), core_of(db, start), core_of(db, end)) {
        (Core::ConstStr(s), Core::ConstInt(a), Core::ConstInt(b)) => {
            let scalars: Vec<char> = s.chars().collect();
            let len = scalars.len() as i64;
            // The range is valid iff `0 <= start <= end <= scalar-len` (signed — a negative bound is out
            // of range, NOT wrapped to a large unsigned offset). `start == end` is an in-range empty slice.
            match (a.to_i64(), b.to_i64()) {
                (Some(a), Some(b)) if a >= 0 && a <= b && b <= len => {
                    let sub: String = scalars[a as usize..b as usize].iter().collect();
                    trace!(target: "rcdzc::fold", node = id.0, "String.slice folds to Some (in-range constant bounds)");
                    let payload = db.push_atom(crate::ast::Leaf::Str(sub));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload],
                    }
                }
                _ => {
                    trace!(target: "rcdzc::fold", node = id.0, "String.slice folds to None (out-of-range constant bounds)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new(),
                    }
                }
            }
        }
        // A runtime string or runtime bound — the byte-rope slice is a later increment.
        _ => Core::Poison(Reject::decline(
            "String.slice on a runtime string is not yet computed (constant strings only)",
        )),
    }
}

/// Lower `(String.to-bytes s)` — the UTF-8 encoding `String → Bytes`. FOLD a constant string to a
/// `Core::BytesOf` whose elements are its UTF-8 bytes, each a fresh `UInt8` `Leaf::Int` synthesized into
/// the arena (the same shape `Bytes.of` of a byte-list builds — so it bakes at escape / consumes through
/// `Bytes.len`/`Bytes.at` identically, no string heap). A runtime string declines (the byte-rope
/// materialization arrives with the runtime string heap). A poison operand propagates.
fn lower_str_to_bytes(db: &mut Db, string: StructId) -> Core {
    match core_of(db, string) {
        Core::ConstStr(s) => {
            let elems: Vec<StructId> = s
                .as_bytes()
                .iter()
                .map(|&b| {
                    db.push_atom(crate::ast::Leaf::Int {
                        value: IntValue::from_i64(b as i64),
                        radix: crate::ast::Radix::Dec,
                    })
                })
                .collect();
            trace!(target: "rcdzc::fold", len = elems.len(), "String.to-bytes folds a constant string to its UTF-8 bytes");
            Core::BytesOf { elems }
        }
        Core::Poison(r) => Core::Poison(r),
        _ => Core::Poison(Reject::decline(
            "String.to-bytes of a runtime string is not yet computed (constant strings only)",
        )),
    }
}

/// Lower `(String.from-bytes b)` — the TOTAL UTF-8 decode `Bytes → (Option String)`. FOLD a
/// compile-time-visible constant `Bytes.of` (each element a constant `UInt8`) by strict UTF-8
/// (`std::str::from_utf8`, which rejects INVALID bytes, OVERLONG encodings, AND surrogate code points —
/// exactly the three failure modes the spec pins): well-formed → `(Some "<decoded>")` (a fresh
/// `Core::ConstStr` payload), ill-formed → `(None unit)` — built as a `Core::SumNew` at the result
/// Option's discs (`option_discs`, like `List.at`/`String.at`), riding the ordinary sum fold/escape/
/// match, no string heap. A runtime `Bytes` declines; a poison operand propagates. Never a trap — an
/// ill-formed sequence is DATA (`None`), the whole point of the total decode.
fn lower_str_from_bytes(db: &mut Db, id: StructId, bytes: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, bytes) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.from-bytes result is not the built-in Option sum",
        ));
    };
    // Collect the raw bytes of a compile-time-visible `Bytes.of`; a runtime Bytes declines.
    let Core::BytesOf { elems } = core_of(db, bytes) else {
        return Core::Poison(Reject::decline(
            "String.from-bytes of a runtime byte sequence is not yet computed (constant Bytes only)",
        ));
    };
    let mut raw = Vec::with_capacity(elems.len());
    for e in elems {
        match core_of(db, e) {
            Core::ConstInt(v) => match v.to_i64() {
                Some(n) if (0..=255).contains(&n) => raw.push(n as u8),
                // A non-UInt8 element can't occur in a well-formed `Bytes.of` (range-checked at build),
                // but be defensive — decline rather than mis-decode.
                _ => {
                    return Core::Poison(Reject::decline(
                        "String.from-bytes: a byte element is not a UInt8",
                    ));
                }
            },
            _ => {
                return Core::Poison(Reject::decline(
                    "String.from-bytes of a non-constant byte element is not yet computed",
                ));
            }
        }
    }
    // Strict UTF-8 decode: `from_utf8` yields the string iff every byte forms a shortest-form, non-
    // surrogate scalar sequence — the spec's well-formedness. Otherwise `None`.
    match std::str::from_utf8(&raw) {
        Ok(s) => {
            trace!(target: "rcdzc::fold", node = id.0, "String.from-bytes folds well-formed UTF-8 to Some");
            let payload = db.push_atom(crate::ast::Leaf::Str(s.to_string()));
            Core::SumNew {
                disc: disc_some,
                payloads: vec![payload],
            }
        }
        Err(_) => {
            trace!(target: "rcdzc::fold", node = id.0, "String.from-bytes folds ill-formed UTF-8 to None");
            Core::SumNew {
                disc: disc_none,
                payloads: Vec::new(),
            }
        }
    }
}

/// Lower `(Option.expect sum message)` / `(Result.expect sum message)` — the unwrap-or-trap accessor. The
/// PRESENT variant is discriminant 0 (`Some`/`Ok`, the sum's FIRST variant — the shape the `expect` field
/// is added for). FOLD a compile-time-visible PRESENT variant (`Core::SumNew{disc:0, payloads:[p]}`) to
/// its payload `p` (the message is discarded). A constant ABSENT variant is a PROVABLE trap; not folded
/// yet (declines cleanly — no corpus case exercises a constant absent expect, and a codeless decline
/// grades Todo, never a miscompile). A runtime sum emits `Core::SumExpect` (disc probe → payload / trap).
/// A poison sum propagates. `message` is not lowered — the wasm trap carries no text.
/// Lower `(Record.project r (a c))` — narrow `r` to the named fields. FOLD over a compile-time-visible
/// `Core::Record`: build a NEW `Core::Record` holding only the named fields, each carrying `r`'s own value
/// occurrence (the value heap is immutable, so the result SHARES `r`'s field values — `type-system.md` §A
/// Record Row Operation Yields A New Value). The second operand is a LITERAL field-name list `(a c)` (labels
/// via `record_op_labels`, NOT an evaluated value). A named field absent from `r` is the CDZ0212 `infer`
/// reports; here the fold simply omits it (the reject denies the build, so this core is never emitted). A
/// poison operand propagates; a non-record / non-constant record declines (the runtime row op is a later
/// increment). A malformed label list is CDZ0201.
fn lower_record_project(
    db: &mut Db,
    id: StructId,
    record: StructId,
    labels: StructId,
    drop: bool,
) -> Core {
    let Core::Record { fields } = core_of(db, record) else {
        return match core_of(db, record) {
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Poison(Reject::decline(
                "a record row operation over a runtime record is not yet built",
            )),
        };
    };
    let Some(labels) = crate::resolve::record_op_labels(db, labels) else {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "the second operand is a list of field names, e.g. `(a c)`",
        ));
    };
    // `project` KEEPS the named fields; `without` keeps every field NOT named (the complement). Each
    // result field carries the operand's own value occurrence (the immutable heap shares them).
    let named: std::collections::BTreeSet<_> = labels.iter().cloned().collect();
    let kept: std::collections::BTreeMap<_, _> = fields
        .iter()
        .filter(|(k, _)| named.contains(*k) != drop)
        .map(|(k, &v)| (k.clone(), v))
        .collect();
    trace!(target: "rcdzc::fold", node = id.0, n = kept.len(), drop, "record project/without folds a constant record to its result fields");
    Core::Record {
        fields: std::sync::Arc::new(kept),
    }
}

/// Lower `(Record.merge a b)` — the UNION of two records' fields (`type-system.md` §Two Records Are
/// Combined Only When Their Field Sets Are Disjoint). FOLD two constant `Core::Record`s to a new one
/// holding every field of both (each carrying its source's value occurrence). The disjointness CDZ0211 is
/// `infer`'s; here a shared field would be silently overwritten by `b`, but the reject denies the build so
/// this core is never emitted. A poison operand propagates; a non-constant/non-record operand declines.
fn lower_record_merge(db: &mut Db, id: StructId, a: StructId, b: StructId) -> Core {
    match (core_of(db, a), core_of(db, b)) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::Record { fields: fa }, Core::Record { fields: fb }) => {
            let mut union: std::collections::BTreeMap<_, _> =
                fa.iter().map(|(k, &v)| (k.clone(), v)).collect();
            for (k, &v) in fb.iter() {
                union.insert(k.clone(), v);
            }
            trace!(target: "rcdzc::fold", node = id.0, n = union.len(), "Record.merge folds two constant records to their union");
            Core::Record {
                fields: std::sync::Arc::new(union),
            }
        }
        _ => Core::Poison(Reject::decline(
            "Record.merge over a runtime record is not yet built",
        )),
    }
}

/// Lower `(Record.extend r (z v))` / `(Record.with r (z v))` — INSERT field `z ↦ v` into a constant
/// `Core::Record` (extend adds an absent field, with replaces a present one; the presence/absence
/// CDZ0211/0212 is `infer`'s, so the fold is one insert for both). The `(z v)` pair's value occurrence
/// carries into the new field. A poison operand propagates; a non-constant/non-record operand, or a
/// malformed pair, declines/rejects.
fn lower_record_insert(db: &mut Db, id: StructId, record: StructId, pair: StructId) -> Core {
    let Core::Record { fields } = core_of(db, record) else {
        return match core_of(db, record) {
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Poison(Reject::decline(
                "a record row operation over a runtime record is not yet built",
            )),
        };
    };
    let Some((label, value)) = crate::resolve::record_op_pair(db, pair) else {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "the second operand is a `(name value)` field pair, e.g. `(z 5)`",
        ));
    };
    let mut out: std::collections::BTreeMap<_, _> =
        fields.iter().map(|(k, &v)| (k.clone(), v)).collect();
    out.insert(label, value);
    trace!(target: "rcdzc::fold", node = id.0, n = out.len(), "Record.extend/with folds an insert into a constant record");
    Core::Record {
        fields: std::sync::Arc::new(out),
    }
}

/// Lower `(Record.pop r z)` — `(tuple (. r z) (r without z))`: the popped field's value paired with the
/// record of the remaining fields. Folds a constant `Core::Record` to a `Core::Tuple{elems: [value,
/// rest-record]}`. The absent-field CDZ0212 is `infer`'s (this fold assumes the field present — an absent
/// one leaves no value occurrence, so it declines defensively). A poison/non-constant operand
/// propagates/declines.
fn lower_record_pop(db: &mut Db, id: StructId, record: StructId, name: StructId) -> Core {
    let Core::Record { fields } = core_of(db, record) else {
        return match core_of(db, record) {
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Poison(Reject::decline(
                "Record.pop over a runtime record is not yet built",
            )),
        };
    };
    let Some(label) = crate::resolve::read_label(db, name) else {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "the second operand is a field name, e.g. `z`",
        ));
    };
    let Some(&value) = fields.get(&label) else {
        return Core::Poison(Reject::decline(
            "Record.pop of an absent field (reported CDZ0212 by inference)",
        ));
    };
    // The remaining record — every field EXCEPT the popped one, each carrying its value occurrence. It is
    // synthesized as its own occurrence (`synth_core`, `Core::Record` + its `Ty::Record`) so it can be the
    // tuple's second element (a `Core::Tuple`'s elements are node ids).
    let rest: std::collections::BTreeMap<_, _> = fields
        .iter()
        .filter(|(k, _)| **k != label)
        .map(|(k, &v)| (k.clone(), v))
        .collect();
    let rest_ty: std::collections::BTreeMap<_, _> = rest
        .keys()
        .map(|k| (k.clone(), crate::infer::type_of(db, rest[k])))
        .collect();
    let rest_record = synth_core(
        db,
        Core::Record {
            fields: std::sync::Arc::new(rest),
        },
        crate::ty::Ty::Record(std::sync::Arc::new(rest_ty)),
    );
    trace!(target: "rcdzc::fold", node = id.0, "Record.pop folds to a (value, remaining-record) tuple");
    Core::Tuple {
        elems: vec![value, rest_record],
    }
}

/// Lower `(Tuple.cat a b)` — concatenate two constant `Core::Tuple`s: the elements of `a` in order
/// followed by `b`'s (each element carrying its source occurrence). A poison operand propagates; a
/// non-constant/non-tuple operand declines (the runtime op is a later increment).
fn lower_tuple_cat(db: &mut Db, id: StructId, a: StructId, b: StructId) -> Core {
    match (core_of(db, a), core_of(db, b)) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::Tuple { elems: ea }, Core::Tuple { elems: eb }) => {
            let mut elems = ea;
            elems.extend(eb);
            trace!(target: "rcdzc::fold", node = id.0, n = elems.len(), "Tuple.cat folds two constant tuples");
            Core::Tuple { elems }
        }
        _ => Core::Poison(Reject::decline(
            "Tuple.cat over a runtime tuple is not yet built",
        )),
    }
}

/// Synthesize a tuple VALUE node from element occurrences — a `Core::Tuple` (or `Core::Unit` for the
/// empty tuple, the empty-tuple-is-unit convention) with its `Ty` filled, so it can be an element of an
/// enclosing tuple (whose elements are node ids). Mirrors `Record.pop`'s remaining-record synthesis.
fn synth_tuple(db: &mut Db, elems: Vec<StructId>) -> StructId {
    if elems.is_empty() {
        return synth_core(db, Core::Unit, crate::ty::Ty::Unit);
    }
    let tys: Vec<crate::ty::Ty> = elems
        .iter()
        .map(|&e| crate::infer::type_of(db, e))
        .collect();
    synth_core(db, Core::Tuple { elems }, crate::ty::Ty::Tuple(tys.into()))
}

/// Lower `(Tuple.split-at t k)` — split a constant `Core::Tuple` at compile-time literal `k` into the
/// PAIR `(tuple <prefix> <suffix>)`: a prefix tuple of the first `k` elements and a suffix tuple of the
/// rest, each synthesized as its own occurrence (`synth_tuple`; an empty side is `unit`). An out-of-range
/// or non-literal `k` is the CDZ0201 `infer` reports (this fold declines defensively). A poison / non-
/// constant tuple operand propagates/declines.
fn lower_tuple_split_at(db: &mut Db, id: StructId, tuple: StructId, pos: StructId) -> Core {
    let Core::Tuple { elems } = core_of(db, tuple) else {
        return match core_of(db, tuple) {
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Poison(Reject::decline(
                "Tuple.split-at over a runtime tuple is not yet built",
            )),
        };
    };
    let arity = elems.len() as i64;
    let k = match core_of(db, pos) {
        Core::ConstInt(v) => v.to_i64().filter(|&k| (0..=arity).contains(&k)),
        _ => None,
    };
    let Some(k) = k else {
        return Core::Poison(Reject::decline(
            "Tuple.split-at needs a compile-time position within the tuple's arity",
        ));
    };
    let k = k as usize;
    let prefix = synth_tuple(db, elems[..k].to_vec());
    let suffix = synth_tuple(db, elems[k..].to_vec());
    trace!(target: "rcdzc::fold", node = id.0, k, "Tuple.split-at folds to a (prefix, suffix) pair");
    Core::Tuple {
        elems: vec![prefix, suffix],
    }
}

/// Lower `(Tuple.pop t)` — element 0 off: `(tuple (. t 0) <rest>)`, the rest a synthesized tuple of the
/// remaining elements (`(Tuple.split-at t 1)` with the singleton prefix unwrapped). A poison / non-
/// constant / empty tuple operand propagates/declines.
fn lower_tuple_pop(db: &mut Db, id: StructId, tuple: StructId) -> Core {
    let Core::Tuple { elems } = core_of(db, tuple) else {
        return match core_of(db, tuple) {
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Poison(Reject::decline(
                "Tuple.pop over a runtime tuple is not yet built",
            )),
        };
    };
    let Some((&first, rest)) = elems.split_first() else {
        return Core::Poison(Reject::decline("Tuple.pop of an empty tuple"));
    };
    let rest_tuple = synth_tuple(db, rest.to_vec());
    trace!(target: "rcdzc::fold", node = id.0, "Tuple.pop folds to a (element0, rest) tuple");
    Core::Tuple {
        elems: vec![first, rest_tuple],
    }
}

fn lower_sum_expect(db: &mut Db, id: StructId, sum: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, sum) {
        return Core::Poison(r);
    }
    // The present variant is discriminant 0 (the sum's first variant). Confirm the scrutinee IS a sum.
    let crate::ty::Ty::Sum { .. } = crate::infer::type_of(db, sum) else {
        return Core::Poison(Reject::decline(
            "expect applies to an Option/Result sum value",
        ));
    };
    const DISC_PRESENT: u32 = 0;
    // FOLD a compile-time-visible present variant to its single payload.
    if let Core::SumNew { disc, payloads } = core_of(db, sum) {
        if disc == DISC_PRESENT && payloads.len() == 1 {
            trace!(target: "rcdzc::fold", node = id.0, "expect folds a constant present variant to its payload");
            return core_of(db, payloads[0]);
        }
        if disc != DISC_PRESENT {
            // A provably-ABSENT constant expect (`Option.expect None`, `Result.expect (Err …)`) — requiring
            // the value of a statically-known absent optional is a PROVABLE TRAP (core-semantics.md
            // §Requiring The Value Of An Optional Traps On Absence). Fold to `Core::Trap` (an `unreachable`)
            // — exactly what the runtime `Core::SumExpect` emits on its absent-disc branch, and the same
            // provable-trap lowering `T.of` out-of-range and a proven-overflow `*` fold to. The trap carries
            // no text (an `unreachable` has none), so a `(trap "m")` message-match still grades Todo — but
            // the OUTCOME is now the correct divergence rather than a decline.
            trace!(target: "rcdzc::fold", node = id.0, disc, "expect on a constant absent variant folds to a provable trap");
            return Core::Trap;
        }
    }
    // A runtime sum — probe the discriminant at run time, unwrap the payload or trap.
    Core::SumExpect {
        scrutinee: sum,
        disc_present: DISC_PRESENT,
    }
}

/// Lower `(Int64.checked-add a b)` / `(Int64.checked-mul a b)` — the FALLIBLE arithmetic companions of
/// the trapping `+`/`*`, returning `(Option T)`: `Some result` when it fits the width / `None` on
/// overflow (numeric-model.md §Overflow Is Defined). FOLD a constant operand pair via `i64` checked
/// arithmetic (the SAME `checked_add`/`checked_mul` `fold_arith` uses to prove the trapping op's overflow
/// — but here overflow yields `None`, not a build error): in range → `Core::SumNew{disc_some, [result]}`
/// (the result a fresh `Core::ConstInt` synthesized into the arena, the `Some` payload — the shape
/// `List.at`/`String.at` use); overflow → `Core::SumNew{disc_none, []}`. Both fold to the ordinary Option
/// construction, riding the sum fold/escape/match. A runtime operand is a later increment (declines
/// cleanly); a poison operand propagates.
fn lower_checked_arith(
    db: &mut Db,
    id: StructId,
    prim: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "checked-arithmetic result is not the built-in Option sum",
        ));
    };
    match (core_of(db, lhs), core_of(db, rhs)) {
        (Core::ConstInt(a), Core::ConstInt(b)) => {
            // Evaluate over `i64` (the Stage default width) — the same range the trapping fold uses. A
            // later width stage generalizes the overflow test to the solved width.
            let (Some(x), Some(y)) = (a.to_i64(), b.to_i64()) else {
                // An operand beyond the machine range — a later width stage handles it; decline for now.
                return Core::Poison(Reject::decline(
                    "checked arithmetic on an operand beyond the evaluated width is not yet folded",
                ));
            };
            let checked = match prim {
                Prim::CheckedAdd => x.checked_add(y),
                _ => x.checked_mul(y),
            };
            match checked {
                Some(n) => {
                    trace!(target: "rcdzc::fold", node = id.0, ?prim, result = n, "checked arithmetic folds to Some (in range)");
                    let payload = db.push_atom(crate::ast::Leaf::Int {
                        value: IntValue::from_i64(n),
                        radix: crate::ast::Radix::Dec,
                    });
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload],
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, ?prim, "checked arithmetic folds to None (overflow)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new(),
                    }
                }
            }
        }
        // A runtime operand — the overflow-detecting Some/None build is a later increment.
        _ => Core::Poison(Reject::decline(
            "checked arithmetic on a runtime operand is not yet computed (constant operands only)",
        )),
    }
}

/// Lower `(Int64.wrapping-add a b)` / `(Int64.wrapping-mul a b)` — two's-complement wraparound, NEVER
/// trapping (numeric-model.md §Overflow Is Defined — the modular value outcome). FOLD a constant operand
/// pair via `i64` `wrapping_add`/`wrapping_mul` (evaluated at the Stage default width; a later width stage
/// masks to the solved width). A runtime operand becomes a `Core::Arith` carrying the WRAPPING prim — the
/// backend selects the RAW machine `i64.add`/`i64.mul` (which already wraps), NOT the checked/trapping
/// path the `+`/`*` prims take. A poison operand propagates.
fn lower_wrapping_arith(db: &mut Db, prim: Prim, lhs: StructId, rhs: StructId) -> Core {
    let a = core_of(db, lhs);
    let b = core_of(db, rhs);
    match (a, b) {
        (Core::ConstInt(x), Core::ConstInt(y)) => {
            let (Some(x), Some(y)) = (x.to_i64(), y.to_i64()) else {
                return Core::Poison(Reject::decline(
                    "wrapping arithmetic on an operand beyond the evaluated width is not yet folded",
                ));
            };
            let n = match prim {
                Prim::WrappingAdd => x.wrapping_add(y),
                _ => x.wrapping_mul(y),
            };
            trace!(target: "rcdzc::fold", ?prim, result = n, "wrapping arithmetic folds to a constant");
            Core::ConstInt(IntValue::from_i64(n))
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // ALGEBRAIC IDENTITY: one operand is a constant making the wrapping op a no-op (`a +% 0`,
        // `a *% 1`) or a constant (`a *% 0 → 0`) — elide the op. Shares the checked-arith `arith_identity`
        // helper (which now handles the wrapping prims), so the two families stay in lockstep.
        (lc, rc) => {
            if let Some(simplified) = arith_identity(db, prim, lhs, &lc, rhs, &rc) {
                trace!(target: "rcdzc::lower", ?prim, "wrapping-arithmetic identity simplified (op elided)");
                return simplified;
            }
            // A runtime operand — the RAW (non-trapping) machine op, selected in the backend from this prim.
            Core::Arith { op: prim, lhs, rhs }
        }
    }
}

/// Lower `(Bytes.of list)` — construct a byte sequence from a list of `Int64` in `0..=255`. Folds only
/// a compile-time-visible `Core::ListNew` operand (a runtime list source is a later increment → declines
/// cleanly). Each element must fold to a constant in range: a value `< 0` or `> 255` is a compile-time
/// trap (CDZ0304, matching the runtime `bytes-set` guard — `numeric-model.md` §A Constant Operation With
/// No Value Is Rejected At Compile Time); a non-constant element declines (its `Bytes.of` can't be baked
/// yet). On success produces `Core::BytesOf { elems }` carrying the element occurrences — the backend
/// bakes/builds the sequence. A poison list propagates.
fn lower_bytes_of(db: &mut Db, id: StructId, list: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, list) {
        return Core::Poison(r);
    }
    let Core::ListNew { elems } = core_of(db, list) else {
        // A runtime list (a parameter, a push-built list) is a later increment — decline cleanly.
        return Core::Poison(Reject::decline(
            "Bytes.of of a runtime list is not yet supported (only a visible list literal)",
        ));
    };
    // Each element is a `UInt8` (the `Bytes.of : (List UInt8) → Bytes` scheme). A CONSTANT element
    // outside `0..=255` is not a UInt8 — reject it as an OUT-OF-RANGE WIDTH literal (CDZ0302), NOT a
    // runtime trap: under the UInt8 model an ill-typed byte cannot be constructed at all, and to truncate
    // a wider value into a byte the program writes `(UInt8.wrap n)` explicitly. (The list-element
    // width-check does not yet flow the UInt8 bound through `(list …)` unification on its own, so the
    // constant bound is enforced here — with the width code, matching the type story.) A RUNTIME element
    // (a `UInt8` param, or `(UInt8.wrap n)`) is IN RANGE BY ITS TYPE and passes through — `select` emits
    // its i32 value into `bytes-set`, so `(Bytes.of (list (UInt8.wrap n)))` builds a byte from a runtime
    // value (the LEB128 encoder). The `Core::BytesOf` is built either way; a CONSTANT one bakes at escape
    // (R1), a RUNTIME one builds on the rope heap + escapes via the looping walker (L2b).
    for &e in &elems {
        match core_of(db, e) {
            Core::Poison(r) => return Core::Poison(r),
            Core::ConstInt(v) => match v.to_i64() {
                Some(n) if (0..=255).contains(&n) => {}
                _ => {
                    trace!(target: "rcdzc::fold", node = id.0, "Bytes.of element is not a UInt8 → CDZ0302");
                    return Core::Poison(Reject::coded(
                        Code::IntOutOfRange,
                        "a byte must be a UInt8 (0..=255); truncate a wider value with UInt8.wrap",
                    ));
                }
            },
            // A runtime UInt8 element — in range by its type; `select` emits its value into `bytes-set`.
            _ => {}
        }
    }
    trace!(target: "rcdzc::lower", node = id.0, len = elems.len(), "Bytes.of → Core::BytesOf");
    Core::BytesOf { elems }
}

/// Lower `(Bytes.at bytes index)` — the fallible indexed byte read. FOLD when `bytes` is a visible
/// `Core::BytesOf` AND `index` folds to a constant: an in-range index (`0 <= i < len`) yields `(Some
/// byte)` — a `Core::SumNew` at the `Some` disc carrying the byte as a constant `Int64` — and an
/// out-of-range index (negative or `>= len`) yields `None`. Otherwise emit the runtime `Core::BytesAt`
/// (a bounds-checked `bytes-get`). Mirrors `lower_list_at`, but the element is always a byte → `Int64`.
fn lower_bytes_at(db: &mut Db, id: StructId, bytes: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, bytes) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Bytes.at result is not the built-in Option sum",
        ));
    };
    // FOLD a `Bytes.of` indexed by a constant integer. An OUT-OF-BOUNDS constant index folds to `None`
    // regardless of the elements (the length is statically known). An IN-BOUNDS index folds to `Some
    // <byte>` ONLY when that element is a compile-time CONSTANT: the `Some` payload is an `Int64`, and a
    // constant byte's core folds through that width, but a RUNTIME element occurrence is a `UInt8` (an i32
    // value) that would sit in the i64 `Some(Int64)` payload UN-WIDENED → invalid wasm ("expected i64,
    // found i32"). So a runtime-element in-bounds read falls through to the runtime `Core::BytesAt` below,
    // which reads the byte and zero-extends it to the payload's i64 width. (`Bytes.at (Bytes.of (list 5))
    // 0)` folds; `Bytes.at (Bytes.of (list n)) 0` with `n` runtime takes the runtime read.)
    if let (Core::BytesOf { elems }, Core::ConstInt(i)) = (core_of(db, bytes), core_of(db, index)) {
        match i.to_i64() {
            Some(n) if n >= 0 && (n as usize) < elems.len() => {
                if matches!(core_of(db, elems[n as usize]), Core::ConstInt(_)) {
                    trace!(target: "rcdzc::fold", node = id.0, index = n, "Bytes.at folds to Some (in-bounds constant index + constant element)");
                    return Core::SumNew {
                        disc: disc_some,
                        payloads: vec![elems[n as usize]],
                    };
                }
                // A runtime element at an in-bounds constant index — fall through to the runtime read
                // (which widens the byte to the Int64 payload); the constant fold would not widen it.
            }
            _ => {
                trace!(target: "rcdzc::fold", node = id.0, "Bytes.at folds to None (out-of-bounds constant index)");
                return Core::SumNew {
                    disc: disc_none,
                    payloads: Vec::new(),
                };
            }
        }
    }
    // A runtime bytes/element or runtime index — emit the bounds-checked runtime read.
    Core::BytesAt {
        bytes,
        index,
        disc_some,
        disc_none,
    }
}

/// Lower `(Bytes.concat a b)`. FOLD when BOTH operands are visible `Core::BytesOf` literals: the result
/// is a single `Core::BytesOf` whose elements are `a`'s then `b`'s (each already a range-checked constant
/// byte occurrence), so a constant concat bakes with no runtime op. Otherwise emit `Core::BytesConcat`. A
/// poison operand propagates.
fn lower_bytes_concat(db: &mut Db, lhs: StructId, rhs: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    if let (Core::BytesOf { elems: a }, Core::BytesOf { elems: b }) =
        (core_of(db, lhs), core_of(db, rhs))
    {
        let mut elems = a;
        elems.extend(b);
        trace!(target: "rcdzc::fold", len = elems.len(), "Bytes.concat folds two constant sequences");
        return Core::BytesOf { elems };
    }
    Core::BytesConcat { lhs, rhs }
}

/// Lower `(Bytes.slice bytes start len)` — the fallible sub-range read. Emits the runtime
/// `Core::BytesSlice`, which bounds-checks (`start >= 0`, `len >= 0`, `start + len <= bytes-len`) and
/// yields `Some(bytes-slice)` in range / `None` out — never trapping (the runtime `bytes-slice` traps on
/// OOB, so the emit guards first). A CONSTANT slice (`Bytes.of` sliced by constant `start`/`len`) FOLDS:
/// out-of-range → `None`; in-range → `Some(Bytes.of <sub-range>)`, a synthesized `Core::BytesOf` carrying
/// the selected element occurrences (its core + type PRE-FILLED so it lowers/types/escapes/compares like
/// any constant `Bytes.of` — same shape `String.slice`/`String.to-bytes` synthesize a folded payload). A
/// runtime bytes/start/len takes the runtime path; the runtime `Some(Bytes)` payload is a Bytes HANDLE
/// (no box). Mirrors `lower_bytes_at`, extended to the compound `Some` payload.
fn lower_bytes_slice(
    db: &mut Db,
    id: StructId,
    bytes: StructId,
    start: StructId,
    len: StructId,
) -> Core {
    for op in [bytes, start, len] {
        if let Core::Poison(r) = core_of(db, op) {
            return Core::Poison(r);
        }
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Bytes.slice result is not the built-in Option sum",
        ));
    };
    // A CONSTANT slice — a visible `Bytes.of` sliced by constant `start`/`len` — folds at compile time.
    if let (Core::BytesOf { elems }, Core::ConstInt(s), Core::ConstInt(l)) =
        (core_of(db, bytes), core_of(db, start), core_of(db, len))
    {
        let n = elems.len() as i128;
        match (s.to_i64(), l.to_i64()) {
            // In range (`start >= 0`, `len >= 0`, `start + len <= bytes-len`) → `Some(Bytes.of <sub>)`.
            // The payload is a synthesized node whose core is a `Core::BytesOf` of the selected element
            // occurrences (already range-checked constant bytes) — its `core`/`ty` are pre-filled so it
            // rides the ordinary constant-`Bytes.of` fold/escape/equality (both `core_of` and `type_of`
            // short-circuit on a filled memo slot), no runtime op. `start == len == 0` yields the empty
            // sequence (present, not None).
            (Some(s), Some(l)) if s >= 0 && l >= 0 && (s as i128) + (l as i128) <= n => {
                let sub: Vec<StructId> = elems[s as usize..(s + l) as usize].to_vec();
                // A fresh occurrence to carry the folded sub-sequence. Its leaf is a placeholder (a
                // `Leaf::Bytes` of the raw sub-bytes, purely so an inspected node is self-consistent);
                // the `core`/`ty` pre-fill below is authoritative — `core_of`/`type_of` short-circuit on
                // a filled slot, so the node never re-resolves through the leaf.
                let raw: Vec<u8> = elems[s as usize..(s + l) as usize]
                    .iter()
                    .filter_map(|&e| match core_of(db, e) {
                        Core::ConstInt(v) => v
                            .to_i64()
                            .filter(|n| (0..=255).contains(n))
                            .map(|n| n as u8),
                        _ => None,
                    })
                    .collect();
                let payload = db.push_atom(crate::ast::Leaf::Bytes(raw));
                db.core.fill(payload, Core::BytesOf { elems: sub });
                db.types.fill(payload, crate::ty::Ty::Bytes);
                trace!(target: "rcdzc::fold", node = id.0, start = s, len = l, "Bytes.slice folds to Some (in-range constant)");
                return Core::SumNew {
                    disc: disc_some,
                    payloads: vec![payload],
                };
            }
            // Provably out of range → `None`.
            _ => {
                trace!(target: "rcdzc::fold", node = id.0, "Bytes.slice folds to None (out-of-range constant)");
                return Core::SumNew {
                    disc: disc_none,
                    payloads: Vec::new(),
                };
            }
        }
    }
    Core::BytesSlice {
        bytes,
        start,
        len,
        disc_some,
        disc_none,
    }
}

/// Lower `(bin <segment>…)` in EXPRESSION position — construct a `Bytes`. Realizes the FIXED-WIDTH
/// INTEGER segments (`uNN`/`iNN`, big-endian, `le` modifier) and BIT-FIELDS (`bits v k`): a CONSTANT
/// segment folds to its encoded bytes, assembled across all segments into a single `Core::BytesOf` of
/// synthesized `UInt8` `Leaf::Int` elems — so a constant `(bin …)` bakes/compares/slices exactly like
/// `(Bytes.of (list …))`, no runtime op. An int emits its `w` two's-complement bytes (MSB-first, reversed
/// for `le`); a `bits k` shifts `k` bits MSB-first into a bit-accumulator that flushes whole bytes as
/// they close (the whole `bin` is byte-aligned — CDZ0220, checked in infer — so the accumulator is empty
/// at every int/bytes segment and at the end). A value OUT OF RANGE for its segment (`(u8 256)`, `(u8
/// -1)`, a `bits k` value ≥ 2^k) is a compile-provable trap (CDZ0304 — the build-fail companion of the
/// runtime "binary value does not fit segment" trap). `(bin)` (no segments) is the empty byte sequence.
/// A `bytes` splice, or a RUNTIME (non-constant) value, is not folded here yet — declines cleanly (BN4
/// dependent-bytes + the runtime path).
fn lower_bin_build(db: &mut Db, id: StructId, segs: &[crate::resolved::Segment]) -> Core {
    use crate::resolved::SegKind;
    // RUNTIME construction: if ANY segment's value is not a compile-time constant, the `bin` can't fold to
    // a baked `Core::BytesOf` — it builds at run time. This slice handles a `bin` of ONLY fixed-width
    // INTEGER segments (a runtime `bits`/`bytes` segment is a later increment); such a `bin` lowers to a
    // `Core::BinBuild` the backend emits (alloc + per-segment range-check-and-write). A constant segment
    // still range-checks here (a provable trap fails the build even alongside a runtime sibling). A `bin`
    // mixing a runtime value with a `bits`/`bytes` segment declines (not yet built).
    let any_runtime = segs.iter().any(|s| match &s.kind {
        // A runtime INT value (a param, not a `ConstInt`) — the segment builds at run time.
        SegKind::Int { .. } => !matches!(core_of(db, s.slot), Core::ConstInt(_) | Core::Poison(_)),
        // A `(bytes b)` splice whose `b` is not a compile-time-visible constant Bytes — spliced at run
        // time via `bytes-concat`. (`bin_const_scrutinee` = Some only for a visible `Core::BytesOf`.)
        SegKind::Bytes { .. } => bin_const_scrutinee(db, s.slot).is_none(),
        // A runtime bit-field value (a param, not a `ConstInt`) — the run packs at run time.
        SegKind::Bits { .. } => !matches!(core_of(db, s.slot), Core::ConstInt(_) | Core::Poison(_)),
        // A `utf8` segment is a PATTERN-only construct here — building a `(utf8 s n)` (splice a String's
        // bytes) is not yet lowered; route it to the const-build loop, which declines cleanly.
        SegKind::Utf8 { .. } => false,
    });
    if any_runtime {
        // Build the `bin` as a sequence of PIECES concatenated at run time (`Core::BytesConcat`): each
        // maximal RUN of fixed-width int segments is one `Core::BinBuild` piece, each maximal RUN of
        // bit-fields is one `Core::BinBitsBuild` piece (byte-aligned — CDZ0220 closes a `bits` run to a
        // whole byte before any int/bytes segment and at the end), and each `(bytes b)` SPLICE segment
        // contributes `b` directly. Composes headers/bit-flags with a runtime bytes body via `bytes-concat`.
        let mut pieces: Vec<StructId> = Vec::new();
        let mut int_run: Vec<crate::core::BinSeg> = Vec::new();
        let mut bits_run: Vec<crate::core::BinBitsField> = Vec::new();
        // Flush the current int-run as a `Core::BinBuild` piece (synthesized, so it emits standalone).
        let flush_ints =
            |db: &mut Db, run: &mut Vec<crate::core::BinSeg>, pieces: &mut Vec<StructId>| {
                if !run.is_empty() {
                    let piece = synth_core(
                        db,
                        Core::BinBuild {
                            segs: std::mem::take(run),
                        },
                        crate::ty::Ty::Bytes,
                    );
                    pieces.push(piece);
                }
            };
        // Flush the current bit-field run as a `Core::BinBitsBuild` piece (byte-aligned per CDZ0220).
        let flush_bits =
            |db: &mut Db, run: &mut Vec<crate::core::BinBitsField>, pieces: &mut Vec<StructId>| {
                if !run.is_empty() {
                    let piece = synth_core(
                        db,
                        Core::BinBitsBuild {
                            fields: std::mem::take(run),
                        },
                        crate::ty::Ty::Bytes,
                    );
                    pieces.push(piece);
                }
            };
        for seg in segs {
            match &seg.kind {
                SegKind::Int { width, signed } => {
                    if let Core::Poison(r) = core_of(db, seg.slot) {
                        return Core::Poison(r);
                    }
                    // An int segment is byte-aligned — close any open bit-field run first (order-preserving).
                    flush_bits(db, &mut bits_run, &mut pieces);
                    // A CONSTANT sibling still range-checks (a provable misfit fails the build).
                    if let Core::ConstInt(v) = core_of(db, seg.slot)
                        && !v.fits_width(*signed, (*width as u32) * 8)
                    {
                        return Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "binary value does not fit segment",
                        ));
                    }
                    int_run.push(crate::core::BinSeg {
                        width: *width,
                        signed: *signed,
                        little_endian: seg.little_endian,
                        value: seg.slot,
                    });
                }
                // A `(bits v k)` bit-field: close any open int-run first, then extend the bit-field run.
                // The run is byte-aligned as a whole (CDZ0220), so it flushes to a `Core::BinBitsBuild`.
                SegKind::Bits { k } => {
                    if let Core::Poison(r) = core_of(db, seg.slot) {
                        return Core::Poison(r);
                    }
                    let k = *k;
                    // `k` must be a usable runtime field width (the u64 pack accumulator carries ≤ 7 open
                    // bits between flushes, so `7 + k <= 64` keeps the `acc << k` shift lossless). A wider
                    // runtime bit-field declines (the constant path still handles k ≤ 63).
                    if k == 0 || k > 56 {
                        return Core::Poison(Reject::decline(
                            "a runtime bin bit-field wider than 56 bits is not yet built",
                        ));
                    }
                    // A CONSTANT bit-field sibling still range-checks (a k-bit UNSIGNED field; misfit → trap).
                    if let Core::ConstInt(v) = core_of(db, seg.slot)
                        && !v.fits_width(false, k)
                    {
                        return Core::Poison(Reject::coded(
                            Code::ConstTrap,
                            "binary value does not fit segment",
                        ));
                    }
                    flush_ints(db, &mut int_run, &mut pieces);
                    bits_run.push(crate::core::BinBitsField { k, value: seg.slot });
                }
                // A `(bytes b)` splice: flush both runs, then splice `b` (a Bytes value). A dependent
                // size `(bytes b n)` on CONSTRUCTION is a length constraint the const path checks; a
                // RUNTIME sized splice (a runtime `b`/`n`) is not checked yet — decline it.
                SegKind::Bytes { size } => {
                    if let Core::Poison(r) = core_of(db, seg.slot) {
                        return Core::Poison(r);
                    }
                    if size.is_some() {
                        return Core::Poison(Reject::decline(
                            "a runtime sized (bytes b n) construction is not yet built",
                        ));
                    }
                    if crate::infer::type_of(db, seg.slot) != crate::ty::Ty::Bytes {
                        return Core::Poison(Reject::decline(
                            "a bin bytes splice operand is not a Bytes value",
                        ));
                    }
                    flush_ints(db, &mut int_run, &mut pieces);
                    flush_bits(db, &mut bits_run, &mut pieces);
                    pieces.push(seg.slot);
                }
                // Constructing a `(utf8 s n)` segment (splice a String's bytes) is not yet lowered — the
                // `utf8` segment is currently pattern-only (`bin_match_decode`). Decline cleanly.
                SegKind::Utf8 { .. } => {
                    return Core::Poison(Reject::decline(
                        "constructing a utf8 bin segment is not yet built (utf8 is pattern-only)",
                    ));
                }
            }
        }
        flush_ints(db, &mut int_run, &mut pieces);
        flush_bits(db, &mut bits_run, &mut pieces);
        // Concatenate the pieces left-to-right. Zero pieces = the empty bin (empty Bytes); one piece is
        // itself; else fold to a chain of `Core::BytesConcat`.
        let mut iter = pieces.into_iter();
        let Some(first) = iter.next() else {
            return Core::BytesOf { elems: Vec::new() }; // (bin) with only… nothing — empty
        };
        let mut acc = first;
        for piece in iter {
            acc = synth_core(
                db,
                Core::BytesConcat {
                    lhs: acc,
                    rhs: piece,
                },
                crate::ty::Ty::Bytes,
            );
        }
        return core_of(db, acc);
    }
    let mut raw: Vec<u8> = Vec::new();
    // The open bit-accumulator between `bits` segments: `acc` holds `nbits` bits, MSB-first (the first
    // field's bits occupy the high end). Whole bytes are flushed to `raw` as soon as `nbits >= 8`.
    let mut acc: u64 = 0;
    let mut nbits: u32 = 0;
    for seg in segs {
        match &seg.kind {
            SegKind::Int { width, signed } => {
                let w = *width as u32;
                let bits = w * 8;
                match core_of(db, seg.slot) {
                    Core::Poison(r) => return Core::Poison(r),
                    Core::ConstInt(v) => {
                        // Range: the value must fit the segment's (signed, bits) width — else a provable
                        // trap (never truncate). `(u8 256)`/`(u8 -1)` fail here.
                        if !v.fits_width(*signed, bits) {
                            return Core::Poison(Reject::coded(
                                Code::ConstTrap,
                                "binary value does not fit segment",
                            ));
                        }
                        // The low `w` bytes of the value's two's-complement representation, big-endian
                        // (MSB first). `to_i64_bits` gives the 64-bit two's-complement pattern; for a
                        // signed negative this already has the right high bits within `w` (checked to fit).
                        let word = v.to_i64_bits() as u64;
                        let mut be: Vec<u8> = (0..w)
                            .rev()
                            .map(|i| ((word >> (i * 8)) & 0xff) as u8)
                            .collect();
                        if seg.little_endian {
                            be.reverse();
                        }
                        raw.extend(be);
                    }
                    // A runtime integer value — the runtime construction path (BN4). Decline for now.
                    _ => {
                        return Core::Poison(Reject::decline(
                            "a bin segment with a runtime value is not yet built (constant segments only)",
                        ));
                    }
                }
            }
            // A bit-field `(bits v k)`: the low `k` bits of `v`, packed MSB-first into the accumulator.
            // `v` must fit `k` UNSIGNED bits (`bits 2 1` — 2 needs two bits, has one — traps). k ≤ 63 keeps
            // `acc` (a u64) from overflowing between flushes (a whole `bin` is byte-aligned, so ≤ 7 bits
            // are ever carried across a segment; a single field ≤ 63 bits fits with room to flush).
            SegKind::Bits { k } => {
                let k = *k;
                match core_of(db, seg.slot) {
                    Core::Poison(r) => return Core::Poison(r),
                    Core::ConstInt(v) => {
                        // A bit-field is an unsigned k-bit value; out of range (or negative) → trap.
                        if k == 0 || k > 63 || !v.fits_width(false, k) {
                            return Core::Poison(Reject::coded(
                                Code::ConstTrap,
                                "binary value does not fit segment",
                            ));
                        }
                        let val = v.to_i64_bits() as u64 & ((1u64 << k) - 1);
                        acc = (acc << k) | val;
                        nbits += k;
                        // Flush every whole byte from the TOP of the accumulator (MSB-first).
                        while nbits >= 8 {
                            let shift = nbits - 8;
                            raw.push(((acc >> shift) & 0xff) as u8);
                            nbits -= 8;
                            acc &= (1u64 << nbits) - 1; // keep only the still-open low bits
                        }
                    }
                    _ => {
                        return Core::Poison(Reject::decline(
                            "a bin bit-field with a runtime value is not yet built (constant segments only)",
                        ));
                    }
                }
            }
            // A `(bytes b [n])` splice — append all of the constant byte sequence `b`. A dependent size
            // `n` (`(bytes b n)`) is a LENGTH CONSTRAINT on construction: `|b|` must equal `n`, else the
            // value does not fit its segment → trap (CDZ0304 for a constant). The whole `bin` is
            // byte-aligned (CDZ0220), so the accumulator is empty here.
            SegKind::Bytes { size } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at a bytes segment"
                );
                let Some(bytes) = bin_const_scrutinee(db, seg.slot) else {
                    if let Core::Poison(r) = core_of(db, seg.slot) {
                        return Core::Poison(r);
                    }
                    return Core::Poison(Reject::decline(
                        "a bin bytes segment with a runtime value is not yet built (constant only)",
                    ));
                };
                if let Some(n_occ) = size {
                    match core_of(db, *n_occ) {
                        Core::ConstInt(v) => {
                            if v.to_i64().filter(|n| *n >= 0) != Some(bytes.len() as i64) {
                                return Core::Poison(Reject::coded(
                                    Code::ConstTrap,
                                    "binary value does not fit segment",
                                ));
                            }
                        }
                        Core::Poison(r) => return Core::Poison(r),
                        _ => {
                            return Core::Poison(Reject::decline(
                                "a bin bytes segment size is not a compile-time constant (not yet built)",
                            ));
                        }
                    }
                }
                raw.extend(bytes);
            }
            // Constructing a `(utf8 s n)` segment (splice a String's UTF-8 bytes) is not yet lowered —
            // `utf8` is currently pattern-only (`bin_match_decode`). Decline cleanly.
            SegKind::Utf8 { .. } => {
                return Core::Poison(Reject::decline(
                    "constructing a utf8 bin segment is not yet built (utf8 is pattern-only)",
                ));
            }
        }
    }
    // A well-formed `bin` is byte-aligned, so no open bits remain here (CDZ0220 caught a mis-aligned one
    // in infer before this runs). Defensively: any residual open bits mean an ill-formed form slipped
    // through — decline rather than emit a wrong byte count.
    if nbits != 0 {
        return Core::Poison(Reject::coded(
            Code::IllFormedBinary,
            "a bin form's bit-fields must close to a whole number of bytes",
        ));
    }
    // Assemble the emitted bytes into a constant `Core::BytesOf` (synthesized UInt8 element leaves), the
    // same shape `b"…"`/`String.to-bytes` produce — so it rides the constant-Bytes fold/escape/equality.
    trace!(target: "rcdzc::lower", node = id.0, len = raw.len(), "bin construction folds to a constant Bytes");
    let elems: Vec<StructId> = raw
        .iter()
        .map(|&b| {
            db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i64(b as i64),
                radix: crate::ast::Radix::Dec,
            })
        })
        .collect();
    Core::BytesOf { elems }
}

/// The constant bytes of `scrutinee` if it reduces to a compile-time-visible `Core::BytesOf` (a `(bin
/// …)`/`(Bytes.of …)`/`b"…"` constant) — `None` for a runtime Bytes (a parameter, a concat result), which
/// takes the BN4 runtime cursor path. Each element must be a `ConstInt` in `0..=255`.
fn bin_const_scrutinee(db: &mut Db, scrutinee: StructId) -> Option<Vec<u8>> {
    let Core::BytesOf { elems } = core_of(db, scrutinee) else {
        return None;
    };
    let mut raw = Vec::with_capacity(elems.len());
    for e in elems {
        match core_of(db, e) {
            Core::ConstInt(v) => raw.push(v.to_i64().filter(|n| (0..=255).contains(n))? as u8),
            _ => return None,
        }
    }
    Some(raw)
}

/// One decoded `bin` segment against a concrete byte sequence: an integer (an `Int`/`Bits` segment's
/// value) or a byte RANGE `[start, end)` into the scrutinee (a `Bytes` segment). Used both to decide a
/// match (literal probes + whole-scrutinee close) and to bind a segment binder (`decode_bin_field`).
enum BinDecoded {
    Int(i64),
    ByteRange(usize, usize),
    /// A `utf8` segment's decoded string — the byte range validated as strict UTF-8 (its match already
    /// required well-formedness, so this is a real `String`). Kept alongside the range so a binder can
    /// bind the decoded `String` directly.
    Str(String),
}

/// Run a `bin` PATTERN's segment automaton over the concrete bytes `raw`, left-to-right. Returns each
/// segment's decoded value if the pattern MATCHES the WHOLE sequence, else `None` (a non-match: a
/// fixed-width segment overruns the input, a bit-field run does not close, a dependent size overruns the
/// remainder, or bytes are left unconsumed with no trailing unsized `(bytes …)`). Handles fixed-width
/// ints, bit-fields, a FINAL unsized `(bytes rest)`, and a DEPENDENT-size `(bytes body n)` (`n` names an
/// earlier INT segment binder — resolved to that segment's already-decoded value). The literal-vs-binder
/// distinction is the CALLER's (a literal slot must equal the decoded int); here we decode every
/// segment's raw value + enforce widths/consumption.
fn bin_match_decode(
    db: &Db,
    raw: &[u8],
    segs: &[crate::resolved::Segment],
) -> Option<Vec<BinDecoded>> {
    use crate::resolved::SegKind;
    let mut out: Vec<BinDecoded> = Vec::with_capacity(segs.len());
    let mut off: usize = 0; // byte offset
    let mut acc: u64 = 0; // open bit-accumulator (MSB-first) between bit-fields
    let mut nbits: u32 = 0;
    for (i, seg) in segs.iter().enumerate() {
        match &seg.kind {
            SegKind::Int { width, signed } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at an int segment"
                );
                let w = *width as usize;
                if off + w > raw.len() {
                    return None; // overrun → non-match
                }
                // Assemble big-endian (MSB first); `le` reverses the byte order.
                let mut val: u64 = 0;
                for j in 0..w {
                    let b = if seg.little_endian {
                        raw[off + (w - 1 - j)]
                    } else {
                        raw[off + j]
                    };
                    val = (val << 8) | b as u64;
                }
                // Sign-extend a signed segment from its top bit; zero-extend an unsigned one.
                let bits = (w as u32) * 8;
                let decoded = if *signed && bits < 64 && (val >> (bits - 1)) & 1 == 1 {
                    (val | !((1u64 << bits) - 1)) as i64
                } else {
                    val as i64
                };
                out.push(BinDecoded::Int(decoded));
                off += w;
            }
            SegKind::Bits { k } => {
                let k = *k;
                // Pull `k` bits MSB-first from the byte stream, refilling the accumulator a byte at a time.
                while nbits < k {
                    if off >= raw.len() {
                        return None; // overrun
                    }
                    acc = (acc << 8) | raw[off] as u64;
                    off += 1;
                    nbits += 8;
                }
                let shift = nbits - k;
                let field = (acc >> shift) & ((1u64 << k) - 1);
                acc &= (1u64 << shift) - 1;
                nbits -= k;
                out.push(BinDecoded::Int(field as i64));
            }
            SegKind::Bytes { size: None } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at a bytes segment"
                );
                // A FINAL unsized bytes binds the remainder. (Well-formedness in infer guarantees it is
                // the last segment.) BN3 handles the final-rest form; a non-final would have been CDZ0220.
                if i + 1 != segs.len() {
                    return None;
                }
                out.push(BinDecoded::ByteRange(off, raw.len()));
                off = raw.len();
            }
            // A DEPENDENT-size `(bytes body n)`: `n` names an EARLIER integer segment binder — resolve it
            // to that segment's already-decoded value, then bind exactly `n` bytes at the cursor. `n == 0`
            // is a valid empty bind; `n` overrunning the remainder is a NON-MATCH (fall through). The
            // whole `bin` is byte-aligned here (CDZ0220), so the cursor is on a byte boundary.
            SegKind::Bytes { size: Some(n_occ) } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at a bytes segment"
                );
                // `n_occ` is a name referencing an earlier segment's binder. Find that segment by name and
                // read its decoded Int. (A non-name / forward / non-int size reference can't be resolved
                // here → non-match, conservatively.)
                let size_name = db.ast.as_name(*n_occ)?;
                let bound = segs
                    .iter()
                    .take(i)
                    .position(|s| db.ast.as_name(s.slot) == Some(size_name))
                    .and_then(|idx| match out.get(idx) {
                        Some(BinDecoded::Int(v)) => Some(*v),
                        _ => None,
                    });
                let n = bound.filter(|v| *v >= 0)? as usize;
                if off + n > raw.len() {
                    return None; // the named size overruns the remaining bytes → non-match
                }
                out.push(BinDecoded::ByteRange(off, off + n));
                off += n;
            }
            // A UTF-8 string segment `(utf8 s n)`: read exactly `n` bytes (like a dependent `bytes`) then
            // DECODE them as strict UTF-8. Ill-formed bytes are a NON-MATCH (return `None`), never a trap —
            // exhaustiveness (a required catch-all) forces the caller to handle the bad case. `n` names an
            // earlier integer segment binder, resolved to its already-decoded value.
            SegKind::Utf8 { size } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at a utf8 segment"
                );
                let size_name = db.ast.as_name(*size)?;
                let bound = segs
                    .iter()
                    .take(i)
                    .position(|s| db.ast.as_name(s.slot) == Some(size_name))
                    .and_then(|idx| match out.get(idx) {
                        Some(BinDecoded::Int(v)) => Some(*v),
                        _ => None,
                    });
                let n = bound.filter(|v| *v >= 0)? as usize;
                if off + n > raw.len() {
                    return None; // the named size overruns the remaining bytes → non-match
                }
                // Strict UTF-8 validation (matches `str::from_utf8` — rejects invalid leads, overlong
                // forms, surrogates, and code points > U+10FFFF). Ill-formed → non-match.
                let s = core::str::from_utf8(&raw[off..off + n]).ok()?;
                out.push(BinDecoded::Str(s.to_string()));
                off += n;
            }
        }
    }
    // Whole-scrutinee accounting: after the last segment, any open bits or leftover bytes are a non-match
    // (a `bin` pattern matches the ENTIRE sequence — leftover needs a trailing unsized `(bytes rest)`).
    if nbits != 0 || off != raw.len() {
        return None;
    }
    Some(out)
}

/// Lower a `bin` PATTERN binder reference — decode the bound segment's value from the (constant)
/// scrutinee. On a visible `Core::BytesOf` scrutinee, run the segment automaton and return this segment's
/// decoded value: an `Int` → `Core::ConstInt`; a `Bytes` → a synthesized constant `Core::BytesOf` of the
/// bound byte range (its core/ty pre-filled, like the slice-fold payload). A runtime scrutinee, or a
/// pattern the automaton can't decode here (a dependent-size `(bytes b n)`), declines — BN4. The arm was
/// already SELECTED by `lower_match_bin` (which ran the same decode + the literal probes), so this decode
/// is on a byte sequence the pattern matched; a defensive `None` still declines rather than miscompiles.
fn decode_bin_field(
    db: &mut Db,
    scrutinee: StructId,
    segs: &[crate::resolved::Segment],
    seg_index: usize,
) -> Core {
    let Some(raw) = bin_const_scrutinee(db, scrutinee) else {
        // RUNTIME scrutinee — decode the segment directly from the runtime `Bytes` (the arm was already
        // selected by `lower_match_bin`'s runtime predicate, which guarded the length). Only a fixed-width
        // INTEGER segment at a STATIC offset is read this way; a bit-field or a (dependent) bytes segment
        // in a runtime match is a later slice.
        return decode_bin_field_runtime(db, scrutinee, segs, seg_index);
    };
    let Some(decoded) = bin_match_decode(db, &raw, segs) else {
        return Core::Poison(Reject::decline(
            "a bin pattern segment could not be decoded at compile time (dependent size / non-match)",
        ));
    };
    match decoded.get(seg_index) {
        Some(BinDecoded::Int(n)) => Core::ConstInt(IntValue::from_i64(*n)),
        // A `utf8` segment binds the decoded, already-validated string as a `Core::ConstStr` (typed
        // `Ty::String`) — the same rep a string literal lowers to, so it rides the constant path.
        Some(BinDecoded::Str(s)) => Core::ConstStr(s.clone()),
        Some(BinDecoded::ByteRange(s, e)) => {
            // A synthesized constant `Core::BytesOf` of the bound sub-range (same shape the Bytes.slice
            // fold produces): fresh UInt8 element leaves, core/ty pre-filled so it rides the constant path.
            let sub: Vec<StructId> = raw[*s..*e]
                .iter()
                .map(|&b| {
                    db.push_atom(crate::ast::Leaf::Int {
                        value: IntValue::from_i64(b as i64),
                        radix: crate::ast::Radix::Dec,
                    })
                })
                .collect();
            let payload = db.push_atom(crate::ast::Leaf::Bytes(raw[*s..*e].to_vec()));
            db.core.fill(payload, Core::BytesOf { elems: sub });
            db.types.fill(payload, crate::ty::Ty::Bytes);
            core_of(db, payload)
        }
        None => Core::Poison(Reject::decline(
            "a bin pattern segment index is out of range",
        )),
    }
}

/// The STATIC byte offset of segment `seg_index`, plus a flag for whether ALL preceding segments are
/// fixed-offset (byte-aligned int/`bits`). `None` if a preceding segment makes the offset dynamic (a
/// dependent-size `(bytes b n)`) or the pattern has a bit-field the runtime path does not handle yet.
/// The runtime matcher (fixed-offset int segments) uses this to place a `BinIntRead`.
fn bin_static_offset(segs: &[crate::resolved::Segment], seg_index: usize) -> Option<u32> {
    use crate::resolved::SegKind;
    let mut off: u32 = 0;
    for seg in segs.iter().take(seg_index) {
        match &seg.kind {
            SegKind::Int { width, .. } => off += *width as u32,
            // A bit-field / bytes / utf8 segment before the target makes the runtime read's offset
            // non-trivial (sub-byte cursor, or dynamic length) — not built yet.
            SegKind::Bits { .. } | SegKind::Bytes { .. } | SegKind::Utf8 { .. } => return None,
        }
    }
    Some(off)
}

/// Decode a `bin`-pattern INTEGER segment binder out of a RUNTIME `Bytes` scrutinee (a `BinIntRead` at
/// the segment's static offset). Only a fixed-width int segment at a fixed offset is supported; anything
/// else (a bit-field or bytes binder, or an offset made dynamic by a preceding dependent size) declines —
/// a later runtime-matching slice.
fn decode_bin_field_runtime(
    db: &mut Db,
    scrutinee: StructId,
    segs: &[crate::resolved::Segment],
    seg_index: usize,
) -> Core {
    use crate::resolved::SegKind;
    let Some(seg) = segs.get(seg_index) else {
        return Core::Poison(Reject::decline(
            "a bin pattern segment index is out of range",
        ));
    };
    match &seg.kind {
        SegKind::Int { width, signed } => match bin_static_offset(segs, seg_index) {
            Some(byte_offset) => {
                // `lower_match_bin` materialized the scrutinee as a KEPT binding, so read it through a
                // `LocalRef` (its own occurrence is the binding key) — NOT the raw scrutinee occurrence,
                // which would re-emit the `BinBuild` construction per binder read.
                let scrut_ref = synth_core(
                    db,
                    Core::LocalRef { binder: scrutinee },
                    crate::ty::Ty::Bytes,
                );
                Core::BinIntRead {
                    bytes: scrut_ref,
                    byte_offset,
                    width: *width,
                    signed: *signed,
                    little_endian: seg.little_endian,
                }
            }
            None => Core::Poison(Reject::decline(
                "a runtime bin segment after a variable-length segment is not yet decoded",
            )),
        },
        // A FINAL unsized `(bytes rest)` binder — the tail after the fixed prefix. Read it as
        // `bytes-slice(scrutinee, off, len-off)` via `Core::BinRestRead`; the offset is the static sum of
        // the preceding int widths (a dependent-size preceding segment would make it dynamic → decline).
        SegKind::Bytes { size: None } if seg_index + 1 == segs.len() => {
            match bin_static_offset(segs, seg_index) {
                Some(byte_offset) => {
                    let scrut_ref = synth_core(
                        db,
                        Core::LocalRef { binder: scrutinee },
                        crate::ty::Ty::Bytes,
                    );
                    Core::BinRestRead {
                        bytes: scrut_ref,
                        byte_offset,
                    }
                }
                None => Core::Poison(Reject::decline(
                    "a runtime bin rest binder after a variable-length segment is not yet decoded",
                )),
            }
        }
        SegKind::Bits { .. } | SegKind::Bytes { .. } | SegKind::Utf8 { .. } => {
            Core::Poison(Reject::decline(
                "a runtime bin bit-field / sized-bytes / utf8 binder is not yet decoded",
            ))
        }
    }
}

/// Synthesize a fresh node carrying `core` with solved type `ty` (its `core`/`ty` columns pre-filled, so
/// it lowers/types directly without re-resolution — the same trick `Bytes.slice`'s fold payload uses).
/// Used by the runtime bin matcher to build the `if`-chain + per-arm predicate out of `Core` directly,
/// and by select's equal-refined-branch collapse to materialize the shared constant.
pub(crate) fn synth_core(db: &mut Db, core: Core, ty: crate::ty::Ty) -> StructId {
    let id = db.push_atom(crate::ast::Leaf::Bytes(Vec::new())); // placeholder leaf; core/ty are authoritative
    db.core.fill(id, core);
    db.types.fill(id, ty);
    id
}

/// A synthesized `(if cond then_ else_)` occurrence (its `Core::If` + result type pre-filled). The result
/// type is the arm bodies' type (they agree — a well-typed match). Used to chain runtime bin-match arms.
fn synth_if(db: &mut Db, cond: StructId, then_: StructId, else_: StructId) -> StructId {
    let ty = crate::infer::type_of(db, then_);
    synth_core(db, Core::If { cond, then_, else_ }, ty)
}

/// Build the runtime PREDICATE for a `(bin …)` arm over a runtime `Bytes` `scrutinee`: a boolean `Core`
/// occurrence that holds exactly when the arm matches — `bytes-len(scrutinee) == total_width` AND, for
/// each LITERAL segment, `BinIntRead(that segment) == literal`. All segments are fixed-width ints (the
/// caller guarded that), so the total width is static and each segment's offset is static. A binder
/// segment adds no probe (it binds); a literal segment must be a constant int. Returns the predicate
/// occurrence, or a `Reject` if a literal slot is not a constant integer.
fn build_bin_arm_predicate(
    db: &mut Db,
    scrutinee: StructId,
    segs: &[crate::resolved::Segment],
) -> Result<StructId, Reject> {
    use crate::resolved::SegKind;
    let total: u32 = segs
        .iter()
        .map(|s| match &s.kind {
            SegKind::Int { width, .. } => *width as u32,
            _ => 0,
        })
        .sum();
    // Length probe. Whole-scrutinee accounting: a `bin` pattern with no trailing unsized bytes matches
    // only the EXACT fixed length (`bytes-len == total`); one ending in a final `(bytes rest)` matches any
    // length `>= total` (the fixed int prefix, with the rest absorbing the remainder). `BytesLen` yields
    // Int64; type both compare operands FIXED Int64 so the literal grounds to i64 (an i32-vs-i64 compare
    // is an invalid module).
    let has_final_rest = matches!(
        segs.last().map(|s| &s.kind),
        Some(SegKind::Bytes { size: None })
    );
    let len_node = synth_core(
        db,
        Core::BytesLen { operand: scrutinee },
        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
    );
    let total_node = synth_core(
        db,
        Core::ConstInt(IntValue::from_i64(total as i64)),
        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
    );
    let mut pred = synth_core(
        db,
        Core::Compare {
            op: if has_final_rest { Prim::Ge } else { Prim::Eq },
            lhs: len_node,
            rhs: total_node,
        },
        crate::ty::Ty::Bool,
    );
    // Per LITERAL segment: `BinIntRead(seg) == literal`. A binder slot (a bare name) adds no probe.
    let mut off: u32 = 0;
    for (i, seg) in segs.iter().enumerate() {
        let SegKind::Int { width, signed } = &seg.kind else {
            continue;
        };
        let w = *width as u32;
        if db.ast.as_name(seg.slot).is_none() {
            // A literal segment — its slot must be a constant integer.
            let lit = match core_of(db, seg.slot) {
                Core::ConstInt(v) => v,
                Core::Poison(r) => return Err(r),
                _ => {
                    return Err(Reject::decline(
                        "a runtime bin pattern literal segment is not a constant integer",
                    ));
                }
            };
            let _ = i;
            // `BinIntRead` always emits an i64; type it FIXED Int64 so the compare's `operand_int_ty`
            // picks width 64 and grounds the literal operand to i64 (not a default-narrow i32).
            let read = synth_core(
                db,
                Core::BinIntRead {
                    bytes: scrutinee,
                    byte_offset: off,
                    width: *width,
                    signed: *signed,
                    little_endian: seg.little_endian,
                },
                crate::ty::Ty::Int(crate::ty::IntTy::i64()),
            );
            let lit_node = synth_core(
                db,
                Core::ConstInt(lit),
                crate::ty::Ty::Int(crate::ty::IntTy::i64()),
            );
            let eq = synth_core(
                db,
                Core::Compare {
                    op: Prim::Eq,
                    lhs: read,
                    rhs: lit_node,
                },
                crate::ty::Ty::Bool,
            );
            pred = synth_core(
                db,
                Core::And {
                    lhs: pred,
                    rhs: eq,
                    is_and: true,
                },
                crate::ty::Ty::Bool,
            );
        }
        off += w;
    }
    Ok(pred)
}

fn lower_conversion(db: &mut Db, id: StructId, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 1 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            format!("{} takes exactly 1 operand", intrinsic_name(op)),
        ));
    }
    // The target width/signedness = the conversion node's solved type (an integer). If it is not an
    // integer type (an unresolved/absurd target), decline rather than guess.
    let target = match crate::infer::type_of(db, id) {
        crate::ty::Ty::Int(it) => it,
        _ => {
            return Core::Poison(Reject::decline(
                "a conversion target is not a definite integer type",
            ));
        }
    };
    let (signed, width) = (target.ground_signed(), target.ground_width());
    match core_of(db, args[0]) {
        Core::ConstInt(v) => match op {
            // `T.wrap` — truncate to the target width at arbitrary precision (total, never traps).
            Prim::Wrap => {
                let wrapped = v.wrap_to(signed, width);
                trace!(target: "rcdzc::fold", op = intrinsic_name(op), signed, width, "folded constant wrap");
                Core::ConstInt(wrapped)
            }
            // `T.of` — the CHECKED conversion: in range → the value UNCHANGED at the target type; out of
            // range → TRAP (numeric-model.md §A Conversion Between Integer Types Is Explicit). The value is
            // not altered when it fits (`(UInt8.of 200) = 200`), distinguishing it from the truncating
            // `wrap` (which would keep the low bits of an out-of-range value instead of trapping).
            _ => {
                if v.fits_width(signed, width) {
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), signed, width, "folded checked conversion (in range)");
                    Core::ConstInt(v)
                } else {
                    // A CONSTANT operand that provably exceeds the target range is a statically-ill-formed
                    // conversion — the compiler already knows at compile time it cannot succeed. Reject it
                    // CDZ0302 (integer does not fit the target width), consistent with `(: 128 Int8)` and
                    // the const-overflow arithmetic fold, rather than emitting a RUNTIME trap for a
                    // statically-impossible conversion. (A RUNTIME `T.of` whose value is unknown until run
                    // time still traps at run time — the branch below declines it to the checked emit; only
                    // a compile-time-KNOWN out-of-range constant is rejected up front here.)
                    let signed_word = if signed { "signed" } else { "unsigned" };
                    trace!(target: "rcdzc::fold", op = intrinsic_name(op), signed, width, "checked conversion of an out-of-range constant → CDZ0302 reject");
                    Core::Poison(Reject::coded(
                        Code::IntOutOfRange,
                        format!(
                            "integer does not fit the target type of the checked conversion \
                             ({signed_word} {width}-bit)"
                        ),
                    ))
                }
            }
        },
        Core::Poison(r) => Core::Poison(r),
        // A runtime operand: emit the mask-and-reinterpret at selection (the target is read off this
        // node's solved type there, the same `type_of(id)` used here).
        _ => {
            // `Int64.of b` / `(UInt N).of b` on a RUNTIME `BigInt` `b` — the checked narrowing back to a
            // fixed width, emitted as the runtime `bigint-to-i64-checked` (traps out of range at run time,
            // B3b). The runtime op checks the i64 range; a narrower target's over-range value is a runtime
            // concern the op will refine later (a constant over-range narrowing is already CDZ0302 at
            // compile time, B1). Checked before the generic runtime-of decline below.
            if matches!(op, Prim::CheckedOf)
                && matches!(crate::infer::type_of(db, args[0]), crate::ty::Ty::BigInt)
            {
                return Core::BigIntToI64 { operand: args[0] };
            }
            // `T.of` on a RUNTIME operand needs a range-check-then-trap emitted at select — not yet built,
            // so decline rather than emit a truncating `Convert` (that would be `wrap`'s semantics — a
            // MISCOMPILE for `of`, silently keeping the low bits where `of` must trap). No corpus case
            // exercises a runtime `T.of` (they all convert constants); a runtime one waits on the checked
            // emit (the `Core::CheckedArith` companion, task follow-up).
            if matches!(op, Prim::CheckedOf) {
                return Core::Poison(Reject::decline(
                    "a runtime checked integer conversion (T.of) is not yet emitted (convert a constant, or use T.wrap)",
                ));
            }
            if is_scalar(db, args[0]) {
                // WRAP COMPOSITION: `T.wrap(U.wrap(x))` where this outer target width `N` is ≤ the inner
                // wrap's target width `M` — the inner wrap keeps the low `M` bits, and the outer keeps the
                // low `N ≤ M`, which are UNCHANGED by the inner wrap. So the inner wrap is redundant:
                // `Int8.wrap(Int16.wrap x)` = `Int8.wrap x`. Reach past the inner Convert to ITS operand,
                // eliding one mask-and-reinterpret. Verified `wrap∘wrap == outer wrap` for N ≤ M, any
                // signedness. (`N > M` is NOT redundant — the inner narrowing already discarded bits the
                // wider outer wrap cannot recover, so it stays.)
                if matches!(op, Prim::Wrap)
                    && let Core::Convert {
                        op: Prim::Wrap,
                        operand: inner,
                    } = core_of(db, args[0])
                    && let crate::ty::Ty::Int(inner_ty) = crate::infer::type_of(db, args[0])
                    && inner_ty.ground_width() >= width
                {
                    trace!(target: "rcdzc::fold", node = id.0, outer = width, inner = inner_ty.ground_width(), "wrap∘wrap: inner wrap subsumed by the narrower/equal outer");
                    return Core::Convert { op, operand: inner };
                }
                trace!(target: "rcdzc::lower", op = intrinsic_name(op), signed, width, "conversion stays runtime (scalar operand)");
                Core::Convert {
                    op,
                    operand: args[0],
                }
            } else {
                Core::Poison(Reject::decline(
                    "a conversion of a non-scalar operand has no meaning",
                ))
            }
        }
    }
}

/// Whether the node at `id` has a SCALAR solved type — an integer or a boolean, which occupies a
/// machine slot the backend can compare or compute on directly (as opposed to a compound/heap value).
fn is_scalar(db: &mut Db, id: StructId) -> bool {
    matches!(
        crate::infer::type_of(db, id),
        crate::ty::Ty::Int(_) | crate::ty::Ty::Bool
    )
}

/// Whether the value at `id` has an ENUM-DISCRIMINANT type — a C-style enum the backend represents as a
/// bare discriminant `i32` (no heap box; `Db::is_enum_disc`). Used to route `=` on such a value to the
/// scalar `i32.eq` compare rather than a `value-eq` heap walk. Peels a nominal wrapper (a nominal-over-
/// enum shares the representation).
fn node_ty_is_enum_disc(db: &mut Db, id: StructId) -> bool {
    match crate::infer::type_of(db, id).strip_nominal() {
        crate::ty::Ty::Sum { decl, .. } => db.is_enum_disc(*decl),
        _ => false,
    }
}

/// The user-facing `Operand.key` spelling of an operation head that is a `(. Operand key)` member access
/// (`(. List at)` → `List.at`), for a wrong-arity diagnostic. Reads the two segment names off the raw
/// `.` form — the surface the author wrote — rather than the internal intrinsic name (`list-at`). `None`
/// when the head is not a two-segment member access (e.g. a bare alias), so the caller falls back to a
/// generic phrasing.
fn op_member_name(db: &Db, head: StructId) -> Option<String> {
    let tail = db.ast.as_form(head, ".")?;
    let [operand, key] = tail else { return None };
    let operand = db.ast.as_name(*operand)?;
    let key = db.ast.as_name(*key)?;
    Some(format!("{operand}.{key}"))
}

/// Reduce an `Ordering` to the boolean the comparison `op` asks of it — the one place the relational
/// prims map to their meaning, shared by every scalar the fold compares (integers and booleans agree
/// on the ordering; only the comparison of the ordering differs). Equality is `Ordering::Equal`.
fn compare_ord(op: Prim, ord: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        Prim::Lt => ord == Less,
        Prim::Gt => ord == Greater,
        Prim::Le => ord != Greater,
        Prim::Ge => ord != Less,
        Prim::Eq => ord == Equal,
        _ => false, // not a comparison — unreachable (only called for `is_comparison` prims).
    }
}

/// The source spelling of an intrinsic, for diagnostics.
fn intrinsic_name(op: Prim) -> &'static str {
    match op {
        Prim::Add => "+",
        Prim::Sub => "-",
        Prim::Mul => "*",
        Prim::Div => "/",
        Prim::Rem => "%",
        Prim::Shl => "<<",
        Prim::Shr => ">>",
        Prim::BitAnd => "&",
        Prim::BitOr => "|",
        Prim::BitXor => "^",
        Prim::Lt => "<",
        Prim::Gt => ">",
        Prim::Le => "<=",
        Prim::Ge => ">=",
        Prim::Eq => "=",
        Prim::Compare => "compare",
        Prim::Wrap => "wrap",
        Prim::CheckedOf => "of",
        Prim::IntCtor => "Int",
        Prim::UIntCtor => "UInt",
        Prim::FnCtor => "->",
        Prim::TupleCtor => "Tuple",
        Prim::RecordCtor => "Record",
        Prim::BoolTy => "Bool",
        Prim::UnitTy => "Unit",
        Prim::StringTy => "String",
        Prim::BigIntTy => "BigInt",
        Prim::BigIntOf => "bigint-of",
        Prim::CharTy => "Char",
        Prim::CharToInt => "char-to-int",
        Prim::CharFromInt => "char-from-int",
        Prim::SymbolTy => "Symbol",
        Prim::SymbolOf => "symbol-of",
        Prim::SymbolToString => "symbol-to-string",
        Prim::SumNew => "sum-new",
        Prim::SumCtor => "sum-ctor",
        Prim::TupleNew => "tuple-new",
        Prim::RecordNew => "record-new",
        Prim::RecordProject => "record-project",
        Prim::RecordWithout => "record-without",
        Prim::RecordMerge => "record-merge",
        Prim::RecordExtend => "record-extend",
        Prim::RecordWith => "record-with",
        Prim::RecordPop => "record-pop",
        Prim::TupleCat => "tuple-cat",
        Prim::TupleSplitAt => "tuple-split-at",
        Prim::TuplePop => "tuple-pop",
        Prim::ListNew => "list-new",
        Prim::ListLen => "list-len",
        Prim::ListPush => "list-push",
        Prim::ListConcat => "list-concat",
        Prim::ListUpdate => "list-update",
        Prim::ListAt => "list-at",
        Prim::ListCtor => "List",
        Prim::BytesOf => "bytes-of",
        Prim::BytesLen => "bytes-len",
        Prim::BytesTy => "bytes-ty",
        Prim::StrScalarLen => "str-scalar-len",
        Prim::StrByteLen => "str-byte-len",
        Prim::BytesAt => "bytes-at",
        Prim::BytesConcat => "bytes-concat",
        Prim::BytesSlice => "bytes-slice",
        Prim::BytesCompact => "bytes-compact",
        Prim::StrAt => "str-at",
        Prim::StrScalarAt => "str-scalar-at",
        Prim::StrConcat => "str-concat",
        Prim::StrSlice => "str-slice",
        Prim::StrToBytes => "str-to-bytes",
        Prim::StrFromBytes => "str-from-bytes",
        Prim::SumExpect => "sum-expect",
        Prim::Trap => "trap",
        Prim::CheckedAdd => "checked-add",
        Prim::CheckedMul => "checked-mul",
        Prim::WrappingAdd => "wrapping-add",
        Prim::WrappingMul => "wrapping-mul",
        Prim::FAdd => "+.",
        Prim::FSub => "-.",
        Prim::FMul => "*.",
        Prim::FDiv => "/.",
        Prim::FloatCtor => "Float",
        Prim::FloatOfInt => "of-int",
        Prim::FloatOf => "of",
        Prim::FloatNan => "nan",
        Prim::MapCtor => "Map",
        Prim::MapNew => "map-new",
        Prim::MapEmpty => "map-empty",
        Prim::MapInsert => "map-insert",
        Prim::MapLookup => "map-lookup",
        Prim::MapRemove => "map-remove",
        Prim::MapSize => "map-size",
        Prim::MapSwap => "map-swap",
        Prim::MapTake => "map-take",
        Prim::UnitOne => "unit-one",
        Prim::UnitBase => "unit-base",
        Prim::UnitMul => "unit-mul",
        Prim::UnitDiv => "unit-div",
        Prim::UnitPow => "unit-pow",
        Prim::UnitPrefix => "unit-prefix",
        Prim::UnitOf => "unit-of",
        Prim::UnitDefine => "unit-define",
        Prim::UnitIn => "unit-in",
        Prim::QtyOf => "qty-of",
        Prim::QtyValue => "qty-value",
        Prim::QtyPow => "qty-pow",
        Prim::QtyUnit => "qty-unit",
        Prim::QtyCtor => "Qty",
        Prim::TypeOf => "type-of",
        Prim::TypeEq => "type-eq",
        Prim::SetCtor => "Set",
        Prim::SetOf => "set-of",
        Prim::SetContains => "set-contains",
        Prim::SetLen => "set-len",
        Prim::SetInsert => "set-insert",
        Prim::SetRemove => "set-remove",
        Prim::SetUnion => "set-union",
        Prim::SetIntersection => "set-intersection",
        Prim::SetDifference => "set-difference",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::IntValue;
    use crate::testkit::{if_program, scalar_program};

    // ── R2 value-form TEMPLATE (the compile-time-computed byte template + runtime holes) ──────────
    //
    // A runtime compound's `encode()` copies the template into memory then fills each hole by walking
    // the heap handle. These tests SIMULATE that fill in Rust — build the template for a type, write
    // each hole from a Rust value model, decode with the shared codec, and assert the rendered text —
    // proving the template layout + hole offsets are right BEFORE any wasm emission depends on them.

    /// A tiny value model mirroring what the runtime holds: a nested tuple/record of ints and bools.
    #[derive(Clone)]
    enum V {
        Int(i64),
        Bool(bool),
        Tuple(Vec<V>),
        Record(Vec<V>), // fields in canonical (sorted) order — positional, like the heap array
    }

    /// Follow a hole's `arr-get` path from the root value to the leaf it addresses.
    fn walk<'a>(root: &'a V, path: &[u32]) -> &'a V {
        let mut v = root;
        for &i in path {
            v = match v {
                V::Tuple(es) | V::Record(es) => &es[i as usize],
                _ => panic!("path descends into a scalar"),
            };
        }
        v
    }

    /// Simulate `encode()`: fill the template's holes from `root`, returning the finished bytes. Int →
    /// 8 big-endian magnitude bytes at the hole (+ flip the kind byte to NEG for a negative); Bool →
    /// the kind byte (8/9).
    fn fill(tpl: &ValueFormTemplate, root: &V) -> Vec<u8> {
        let mut bytes = tpl.bytes.clone();
        for hole in &tpl.leaves {
            match (hole.kind, walk(root, &hole.path)) {
                (LeafFill::Int, V::Int(n)) => {
                    let mag = (n.unsigned_abs()).to_be_bytes(); // 8 bytes, big-endian
                    bytes[hole.offset..hole.offset + 8].copy_from_slice(&mag);
                    if *n < 0 {
                        // kind byte sits 2 bytes before the magnitude (kind + len=8 → len is one byte).
                        bytes[hole.offset - 2] = 3; // KIND_INT_NEG_DEC
                    }
                }
                (LeafFill::Bool, V::Bool(b)) => {
                    bytes[hole.offset] = if *b { 9 } else { 8 };
                }
                _ => panic!("hole kind / value mismatch"),
            }
        }
        bytes
    }

    /// Build a template for `ty`, fill it from `root`, decode + print — the value-form text the host
    /// would render.
    fn render(ty: &crate::ty::Ty, root: &V) -> String {
        let tpl = runtime_value_form_template(ty).expect("template");
        let bytes = fill(&tpl, root);
        let arenas = cadenza_syntax::codec::decode(&bytes).expect("decode filled template");
        cadenza_syntax::sexpr::print(&arenas).trim().to_string()
    }

    fn t_int() -> crate::ty::Ty {
        crate::ty::Ty::int64()
    }

    #[test]
    fn template_fills_a_flat_runtime_tuple() {
        let ty = crate::ty::Ty::Tuple(vec![t_int(), t_int()].into());
        assert_eq!(
            render(&ty, &V::Tuple(vec![V::Int(3), V::Int(1)])),
            "(: (tuple 3 1) (Tuple Int64 Int64))"
        );
        // Different runtime values reuse the SAME template — only the holes change.
        assert_eq!(
            render(&ty, &V::Tuple(vec![V::Int(4), V::Int(8)])),
            "(: (tuple 4 8) (Tuple Int64 Int64))"
        );
    }

    #[test]
    fn template_fills_a_mixed_and_negative_tuple() {
        let ty = crate::ty::Ty::Tuple(vec![t_int(), crate::ty::Ty::Bool].into());
        assert_eq!(
            render(&ty, &V::Tuple(vec![V::Int(0), V::Bool(true)])),
            "(: (tuple 0 true) (Tuple Int64 Bool))"
        );
        let ty2 = crate::ty::Ty::Tuple(vec![t_int(), t_int()].into());
        assert_eq!(
            render(&ty2, &V::Tuple(vec![V::Int(-5), V::Int(7)])),
            "(: (tuple -5 7) (Tuple Int64 Int64))"
        );
    }

    #[test]
    fn template_fills_a_three_element_and_nested_tuple() {
        let ty3 = crate::ty::Ty::Tuple(vec![t_int(), t_int(), t_int()].into());
        assert_eq!(
            render(&ty3, &V::Tuple(vec![V::Int(10), V::Int(11), V::Int(12)])),
            "(: (tuple 10 11 12) (Tuple Int64 Int64 Int64))"
        );
        let nested = crate::ty::Ty::Tuple(
            vec![t_int(), crate::ty::Ty::Tuple(vec![t_int(), t_int()].into())].into(),
        );
        assert_eq!(
            render(
                &nested,
                &V::Tuple(vec![V::Int(2), V::Tuple(vec![V::Int(2), V::Int(2)])])
            ),
            "(: (tuple 2 (tuple 2 2)) (Tuple Int64 (Tuple Int64 Int64)))"
        );
    }

    #[test]
    fn template_fills_a_runtime_record() {
        use crate::resolved::Symbol;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(Symbol::plain("a"), t_int());
        fields.insert(Symbol::plain("b"), t_int());
        let ty = crate::ty::Ty::Record(fields.into());
        // Fields in canonical (sorted) order a, b → positional [a, b].
        assert_eq!(
            render(&ty, &V::Record(vec![V::Int(3), V::Int(1)])),
            "(: (record (a 3) (b 1)) (Record (a Int64) (b Int64)))"
        );
    }

    #[test]
    fn lowers_a_literal_to_a_const() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        assert_eq!(
            core_of(&mut db, body),
            Core::ConstInt(IntValue::from_i64(42))
        );
    }

    #[test]
    fn an_if_with_a_constant_condition_folds_to_the_taken_branch() {
        // `if_program` is `(if false 1 2)` — a CONSTANT condition, so it folds to the else-branch (2),
        // NOT a residual `Core::If`. (A constant-condition `if` is dead-branch-eliminated in `lower`.)
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        assert_eq!(
            core_of(&mut db, if_node),
            Core::ConstInt(IntValue::from_i64(2)),
            "if false 1 2 folds to 2"
        );
    }

    #[test]
    fn a_const_if_folds_past_an_unreachable_trap_but_not_an_illformed_branch() {
        // A `ConstTrap` (CDZ0304) in the UNTAKEN branch is reachability-gated — the const-if folds past
        // it to the taken branch (the same rule `collect_reached_poisons` applies: a trap shielded by an
        // untaken branch is not a build failure). `(if (< 1 2) 7 (% 5 0))` → 7.
        let ast =
            crate::testkit::parse("(module m (def (main) (if (< 1 2) 7 (% 5 0))) (export main))");
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("main").unwrap()].body.unwrap();
        assert_eq!(
            core_of(&mut db, body),
            Core::ConstInt(IntValue::from_i64(7)),
            "a const-if folds past an unreachable ConstTrap untaken branch"
        );
        // But a NON-TRAP poison in the untaken branch (an unbound name) is an ill-formedness the program
        // must be rejected for — the const-if is KEPT (not folded) so the fault surfaces.
        let ast2 =
            crate::testkit::parse("(module m (def (main) (if (< 1 2) 7 nope)) (export main))");
        let mut db2 = Db::load(ast2);
        let body2 = db2.defs[db2.def_by_name("main").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db2, body2), Core::If { .. }),
            "a const-if with an ill-formed (unbound-name) untaken branch is NOT folded away"
        );
    }

    #[test]
    fn an_if_with_identical_branches_folds_to_the_branch() {
        // `(if p x x)` — both branches are the same value, so the `if` collapses to `x` (the condition
        // `p` is a param, trap-free, so evaluating it has no effect to preserve). Result: `Core::Param`
        // (the `x`), NOT a `Core::If`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: p Bool) (: x Int64)) (if p x x)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("f").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db, body), Core::Param { .. }),
            "an if with identical branches over a trap-free condition folds to the branch"
        );
    }

    #[test]
    fn if_true_false_folds_to_the_condition() {
        // `(if c true false)` is a boolean coercion no-op — it computes `c` itself. `(< a b)` is a
        // comparison, so the body folds to `Core::Compare`, NOT a `Core::If` wrapping two ConstBools.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) true false)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("f").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db, body), Core::Compare { .. }),
            "if c true false folds to the condition c"
        );
        // The dual `(if c false true)` is a NEGATION `!c` — it folds to `Core::Not { operand: c }` (the
        // backend emits `<c> ; i32.eqz`), NOT the bare condition (that would leave the result uninverted).
        let ast2 = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) false true)) (def (main) 0) (export main))",
        );
        let mut db2 = Db::load(ast2);
        let body2 = db2.defs[db2.def_by_name("f").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db2, body2), Core::Not { .. }),
            "if c false true folds to the negation !c"
        );
    }

    #[test]
    fn an_if_with_identical_branches_keeps_a_possibly_trapping_condition() {
        // `(if (g x) x x)` where `g` is a RECURSIVE call (possibly-trapping) — the branches are equal,
        // but the condition is NOT trap-free, so the `if` is KEPT to preserve the condition's evaluation
        // (and any trap). Result stays a `Core::If`.
        let ast = crate::testkit::parse(
            "(module m (def (g (: n Int64)) (if (= n 0) true (g (- n 1)))) \
               (def (f (: x Int64)) (if (g x) x x)) (export f))",
        );
        let mut db = Db::load(ast);
        let body = db.defs[db.def_by_name("f").unwrap()].body.unwrap();
        assert!(
            matches!(core_of(&mut db, body), Core::If { .. }),
            "identical branches do NOT fold away a possibly-trapping condition"
        );
    }

    #[test]
    fn lowers_a_runtime_if_referencing_its_child_ids() {
        // A RUNTIME condition (a bool parameter `p`) is NOT foldable, so it stays a `Core::If` carrying
        // its child occurrences: `(def (f (: p Bool)) (if p 1 2))`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: p Bool)) (if p 1 2)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let d = db.def_by_name("f").expect("def f");
        let body = db.defs[d].body.expect("body");
        match core_of(&mut db, body) {
            Core::If { then_, else_, .. } => {
                assert_eq!(
                    core_of(&mut db, then_),
                    Core::ConstInt(IntValue::from_i64(1))
                );
                assert_eq!(
                    core_of(&mut db, else_),
                    Core::ConstInt(IntValue::from_i64(2))
                );
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    // ── A-normalization: the keep-or-propagate decision at the core column ────────────────────────
    //
    // A `let` whose value is a RUNTIME computation used more than once is kept as a `Core::Let`
    // (named once); a single-use or constant binding is propagated (no residual `Let`). These inspect
    // the core form directly — the module's own concern — separate from the wasm behavior tests.

    /// Locate def `name`'s body occurrence (the root of the expression `core_of` is asked about).
    fn body_of(db: &mut Db, name: &str) -> StructId {
        let d = db.def_by_name(name).expect("def present");
        db.defs[d].body.expect("body")
    }

    #[test]
    fn a_multi_use_runtime_let_lowers_to_a_core_let() {
        // `(let ((s (+ a b))) (+ s s))` in a function body — `s` is a runtime add used twice, so the
        // body's core is a `Core::Let` naming `s`, with the binding keyed by `s`'s initializer.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (let ((s (+ a b))) (+ s s))) (export f))",
        );
        let mut db = Db::load(ast);
        let body = body_of(&mut db, "f");
        match core_of(&mut db, body) {
            Core::Let { bindings, .. } => {
                assert_eq!(bindings.len(), 1, "exactly one binding kept");
                // The kept binding's value lowers to a runtime arithmetic op (the `(+ a b)`).
                let (_, value) = bindings[0];
                assert!(matches!(core_of(&mut db, value), Core::Arith { .. }));
            }
            other => panic!("expected Core::Let, got {other:?}"),
        }
    }

    #[test]
    fn a_single_use_runtime_let_leaves_no_core_let() {
        // `(let ((s (+ a b))) (* s 2))` — `s` used ONCE, so it is copy-propagated: the body's core is
        // the `(* (+ a b) 2)` multiplication directly, with NO enclosing `Core::Let`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (let ((s (+ a b))) (* s 2))) (export f))",
        );
        let mut db = Db::load(ast);
        let body = body_of(&mut db, "f");
        assert!(
            matches!(core_of(&mut db, body), Core::Arith { op: Prim::Mul, .. }),
            "a single-use binding must propagate, leaving no Core::Let"
        );
    }

    #[test]
    fn a_multi_use_constant_let_folds_and_is_not_named() {
        // `(let ((k (+ 1 2))) (+ k k))` — `k` used twice but its value FOLDS to the constant 3, so
        // there is no runtime computation to share: the whole body folds to `ConstInt(6)`, no `Let`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64)) (let ((k (+ 1 2))) (+ k k))) (export f))",
        );
        let mut db = Db::load(ast);
        let body = body_of(&mut db, "f");
        assert_eq!(
            core_of(&mut db, body),
            Core::ConstInt(IntValue::from_i64(6)),
            "a constant binding folds; nothing is named"
        );
    }
}

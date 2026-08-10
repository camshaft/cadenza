//! DATA-TYPE INVARIANT ESTABLISH — Part 1: synthesize a TYPED predicate-check def per `@invariant` type.
//!
//! The DATA-level verification-family member (design §10). An `@invariant(PRED)` on a type `T` states a
//! property every value of `T` maintains, with the value bound to `self`. ESTABLISH is the obligation that
//! every constructor of `T` produces a value satisfying `PRED`.
//!
//! ## What THIS pass does (Part 1)
//!
//! For each type declared with an `@invariant` — `(@ (invariant PRED) (type T …))` — synthesize a checker:
//!
//! ```text
//! (def (__invariant_check_T (: self T)) <CHECK-BODY>)
//! ```
//!
//! appended to the top-level item list. Because it flows through the ordinary resolve/infer/lower like any
//! hand-written def (the `proptest_gen` synthesis discipline), the predicate is now RESOLVED + TYPE-CHECKED
//! with `self : T` in scope — which it never was as the stripped recorded predicate. It is also the CALLEE the
//! construct-site establish check (Part 2) will invoke.
//!
//! ## `self` binds the WHOLE value; a SINGLE-PAYLOAD NEWTYPE AUTO-UNWRAPS (ruling 2026-07-18)
//!
//! `self` is the whole value of `T`. For a SINGLE-VARIANT, SINGLE-PAYLOAD newtype `(type T (V U))`, a bare
//! scalar predicate `(>= self 0)` would hit the nominal boundary (`self : T` is not comparable to an `Int64`
//! literal — CDZ0202). So the checker AUTO-UNWRAPS: the CHECK-BODY is
//!
//! ```text
//! (match self (((. T V) __inv_u) PRED[self := __inv_u]))
//! ```
//!
//! — destructure the sole variant, binding its payload, and rewrite every `self` in PRED to that payload. This
//! realizes the design's "Percent-as-newtype-over-Int64: T IS the scalar, `(>= self 0)` typechecks" intent
//! faithfully within the nominal-newtype grammar — the author writes the bare form (or an accessor
//! `(> (len self) 0)`, which likewise needs the underlying `(List …)`), the checker transparently unwraps `self`
//! to the payload, so the nominal boundary never bites (and v-property-testing's generator keeps reading the
//! bare `self` shape from the raw AST, unaffected by this transform).
//!
//! **A predicate that ALREADY destructures `self` itself is NOT rewritten.** If PRED contains a `(match self …)`
//! (the author unwraps the whole value directly — `(match self (((. T V) v) …))`), rewriting `self` → the payload
//! would corrupt it (the inner match would try to destructure the payload, not the value). So the auto-unwrap
//! only applies when PRED does NOT self-destructure `self`; a self-destructuring PRED uses `self` (the whole
//! value) directly. Both forms are thus valid: bare/accessor (auto-unwrapped) and self-destructure (as-is).
//!
//! A type that is NOT a single-payload newtype (a multi-variant sum, a nullary/multi-payload variant, or a
//! record) does NOT auto-unwrap — the predicate uses `self` directly via an accessor / its own match. (Only the
//! single-payload newtype has the transparent-scalar reading the bare form needs.)
//!
//! The check-body's PRED is a COPY of the recorded node (the `@invariant` wrapper's PRED stays intact for
//! `strip_annotations` → `db.invariants`). Runs at load BEFORE `strip_annotations` (wrapper still present) +
//! `scan_top_level` (the synthesized def scans like hand-written source); all appended nodes are ordinary AST.
//!
//! ## Part 2 — the CHECKED CONSTRUCTOR (`__invariant_construct_T`)
//!
//! For a single-payload newtype this pass ALSO synthesizes the checked constructor the construct-site establish
//! enforcement routes to (design §10.2 — ESTABLISH: every construction of `T` must satisfy its invariant, else
//! TRAP; the (D) run-time path):
//!
//! ```text
//! (def (__invariant_construct_T (: __inv_p U))
//!   (let ((__inv_v ((. T V) __inv_p))) (if (__invariant_check_T __inv_v) __inv_v (trap "…"))))
//! ```
//!
//! It builds the value ONCE (`__inv_v : T`, properly typed — so `__invariant_check_T`'s `self : T` param and its
//! auto-unwrap match receive the right type) and yields it or traps. Because it flows through the ordinary
//! resolve/infer/lower, the newtype erasure of `__inv_v` to its payload is uniform across the value + check-call
//! arms (no erased-arg subtlety).
//!
//! It is WIRED at `lower_sum_new`: a single-payload construction `(T.V x)` of an `@invariant` newtype in user
//! code becomes a `Core::Call` to this def (instead of erasing straight to the payload), so EVERY construction
//! establishes the invariant at run time — the author writes the natural constructor, no call-site annotation.
//! `synthesize` returns the set of RAW-CONSTRUCTION ids (each `((. T V) __inv_p)` a construct-def contains);
//! `Db::load` stores it in `invariant_exempt_ctors`, and `lower_sum_new` EXEMPTS those ids from the divert —
//! they ARE the checked constructor's own construction, so re-routing them would recurse forever.
//!
//! ## Multi-variant sums — a checked constructor PER VARIANT
//!
//! A ≥2-variant sum is not a newtype (it BOXES as `Core::SumNew{disc, payloads}`, not erased), so it has no
//! single construct-def. Instead this pass synthesizes ONE checked constructor per variant, keyed by the
//! variant's DISCRIMINANT (declaration index) — `__invariant_construct_<T>__d<i>` — since the divert at the
//! boxed `Core::SumNew` path has the `disc` in hand:
//!
//! ```text
//! (def (__invariant_construct_Shape__d0 (: __inv_p0 Int64))
//!   (let ((__inv_v ((. Shape Circle) __inv_p0))) (if (__invariant_check_Shape __inv_v) __inv_v (trap))))
//! (def (__invariant_construct_Shape__d1 (: __inv_p0 Int64) (: __inv_p1 Int64))
//!   (let ((__inv_v ((. Shape Square) __inv_p0 __inv_p1))) (if (__invariant_check_Shape __inv_v) __inv_v (trap))))
//! ```
//!
//! Each `__inv_v : Shape` is the properly-typed BOXED sum, fed to the whole-value `__invariant_check_Shape`
//! (Part 1, no auto-unwrap — the predicate reads `self` directly via the author's match/accessor). Each inner raw
//! `((. T V) …)` is EXEMPT, so it builds a plain `Core::SumNew` when re-reached (no recursion).
//!
//! The per-variant path fires for ANY type that is not a sole-PAYLOAD newtype — so it also covers a
//! SINGLE-VARIANT MULTI-payload newtype `(type Range (Mk A B))`, whose sole variant (disc 0) gets
//! `__invariant_construct_Range__d0`. Such a type erases to a `Ty::Tuple` (not a single-payload value), so the
//! divert for it is in `lower_sum_new`'s tuple-erase arm (keyed `__d<disc>`), not the `args.len()==1` path.
//! A NULLARY variant also gets a (no-arg) construct-def — `(let ((__inv_v (T.V unit))) (if (check __inv_v)
//! __inv_v (trap)))` — diverted at `lower_sum_new`'s nullary-unit path, so an invariant that rejects a nullary
//! variant (making it uninhabitable) traps when it is constructed.
//!
//! With single/multi-payload newtypes, multi-variant sums, and nullary variants all covered, ESTABLISH is
//! complete across every variant shape. PRESERVE (design §10.2 — an op returning `T` maintains `I`) follows
//! for FREE under the (D) run-time tier: an op building its `T` result does so through the SAME checked
//! constructor, so a result violating the invariant traps at that construction — no separate machinery.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

fn name(ast: &mut Arenas, n: &str) -> StructId {
    push_atom(ast, Leaf::Name(n.into()))
}

/// The `@invariant` VALUE binder — the name the invariant predicate references to mean "the value being
/// checked" (`@invariant(>= self 0)`). RENAMED from `it` (operator ruling 2026-07-18: *"ret for ensures and
/// self for invariants makes sense"*): `it` was too collision-prone; `self` reads as "the value of this type"
/// and is the operator's chosen name for `@invariant` (DISTINCT from `@ensures`'s `ret` — each family member
/// gets the name that fits its meaning). Governs the checker's `(: self T)` param + the auto-unwrap match
/// scrutinee. (More collision-prone than a `__`-form, but the operator chose readability.)
pub(crate) const VALUE_BINDER: &str = "self";

/// The fresh payload-binder name the auto-unwrap introduces — chosen not to collide with a user name
/// (a `__`-prefixed name the reader would not write; and never `self`, which stays in scope for accessor forms).
const UNWRAP_BINDER: &str = "__inv_u";

/// The checked-constructor's payload PARAMETER binder — the value the raw construction consumes
/// (`(def (__invariant_construct_T (: __inv_p U)) …)`). A `__`-prefixed fresh name, no user collision.
const CONSTRUCT_PARAM: &str = "__inv_p";

/// The checked-constructor's local binding for the constructed value — built once, checked, then yielded or
/// trapped (`(let ((__inv_v ((. T V) __inv_p))) (if (__invariant_check_T __inv_v) __inv_v (trap)))`).
const CONSTRUCT_VALUE: &str = "__inv_v";

/// One `@invariant`-annotated type's synthesis plan, collected in a first scan and consumed in a second
/// (append-only) pass — see [`synthesize`].
struct Plan {
    /// The annotated type's name (`Percent`) — names both synthesized defs (`__invariant_check_Percent`,
    /// `__invariant_construct_Percent`) and the checker's `self : T` param + auto-unwrap ctor pattern.
    type_name: String,
    /// For a SINGLE-VARIANT, SINGLE-PAYLOAD newtype, its `(ctor-name, payload-type-occ)` — the ctor + payload
    /// type the auto-unwrap match and the checked constructor synthesize against. `None` for any other shape.
    sole_variant: Option<(String, StructId)>,
    /// EVERY variant `(ctor-name, [payload-type-occ…])` in declaration order (index = discriminant). Used to
    /// synthesize a per-variant checked constructor for a MULTI-variant sum (the `sole_variant` case has its
    /// own single-construct path; a nullary/empty-payload variant here gets no construct-def — its
    /// establish is a later injection point).
    variants: Vec<(String, Vec<StructId>)>,
    /// The recorded `@invariant` PREDICATE occurrence (over `self`), copied into the checker body.
    pred: StructId,
}

/// Deep-copy a predicate subtree, rewriting every bare-name occurrence of `from` to `to` (and re-pushing
/// other names as fresh occurrences so they resolve in the def's scope). Non-name leaves are shared.
/// `from`/`to` empty ⇒ a plain copy (no rewrite).
fn copy_rewrite(ast: &mut Arenas, node: StructId, from: &str, to: &str) -> StructId {
    match ast.get(node).clone() {
        Struct::Atom(lid) => {
            let leaf = ast.leaf(lid).clone();
            if let Leaf::Name(n) = &leaf {
                let n = if !from.is_empty() && n.as_ref() == from {
                    to.to_string()
                } else {
                    n.to_string()
                };
                return push_atom(ast, Leaf::Name(n.into()));
            }
            node
        }
        Struct::List(children) => {
            let copied: Vec<StructId> = children
                .iter()
                .map(|&c| copy_rewrite(ast, c, from, to))
                .collect();
            push_list(ast, copied)
        }
    }
}

/// True if `node` (a predicate subtree) contains a `(match self …)` whose scrutinee is the bare value binder
/// `self` — i.e. the author already destructures the whole value. When so, auto-unwrap must NOT rewrite
/// `self` → the payload (that would corrupt the author's match). A conservative syntactic scan.
fn self_destructures_value(ast: &Arenas, node: StructId) -> bool {
    if let Some(mtail) = ast.as_form(node, "match")
        && let Some(&scrut) = mtail.first()
        && ast.as_name(scrut) == Some(VALUE_BINDER)
    {
        return true;
    }
    if let Struct::List(children) = ast.get(node) {
        return children.iter().any(|&c| self_destructures_value(ast, c));
    }
    false
}

/// If `type_decl_tail` (the children of a `(type …)` AFTER the head, i.e. `[NAME, variant…]`) describes a
/// SINGLE-VARIANT, SINGLE-PAYLOAD newtype `(type T (V U))`, return `(V-name, U-type-occ)` — the sole variant's
/// ctor name and its single payload TYPE occurrence. Returns `None` for a nullary variant, a multi-payload
/// variant, a multi-variant sum, or a record. The payload occ is what the construct-def's parameter is typed
/// against (`(: __inv_p U)`); the name is what the auto-unwrap match + the construct-def ctor reference.
fn sole_newtype_variant(ast: &Arenas, type_decl_tail: &[StructId]) -> Option<(String, StructId)> {
    // `[NAME, variant]` — exactly one variant after the type name.
    let [_name, variant] = type_decl_tail else {
        return None;
    };
    // The variant is `(V U)` — a list whose head is the ctor name and with exactly ONE payload element.
    let Struct::List(items) = ast.get(*variant) else {
        return None;
    };
    if items.len() != 2 {
        return None; // nullary (`(V)`) or multi-payload (`(V A B)`) — not a single-payload newtype
    }
    ast.as_name(items[0]).map(|n| (n.to_string(), items[1]))
}

/// Enumerate EVERY variant of a `(type T variant…)` declaration: `[(ctor-name, [payload-type-occ…])…]` in
/// declaration order (so a variant's index IS its discriminant — `type-system.md`, the same order
/// `variant_disc_of` reads). A variant `(V P0 P1 …)` yields `("V", [P0, P1, …])`; a nullary `(V)` yields
/// `("V", [])`. Used to synthesize a per-variant checked constructor for a MULTI-variant sum (the sole-newtype
/// case has its own single-construct path). A malformed variant (not a headed list) is skipped.
fn all_variants(ast: &Arenas, type_decl_tail: &[StructId]) -> Vec<(String, Vec<StructId>)> {
    let mut out = Vec::new();
    for &variant in type_decl_tail.iter().skip(1) {
        let Struct::List(items) = ast.get(variant) else {
            continue;
        };
        let Some(&head) = items.first() else {
            continue;
        };
        if let Some(vname) = ast.as_name(head) {
            out.push((vname.to_string(), items[1..].to_vec()));
        }
    }
    out
}

/// Synthesize a `__invariant_check_<T>` def (Part 1) and, for a single-payload newtype, a
/// `__invariant_construct_<T>` def (Part 2) per `@invariant`-annotated type + append to the top-level items.
///
/// RETURNS the set of RAW-CONSTRUCTION node ids the construct-defs contain — the `((. T V) __inv_p)` inside
/// each `__invariant_construct_<T>`. These are the construct sites that must be EXEMPT from the establish
/// divert (`lower_sum_new`): they ARE the checked constructor's own construction, so re-routing them through
/// `__invariant_construct_<T>` would recurse forever. `Db::load` records this set in `invariant_exempt_ctors`
/// (append-only synthesis keeps these ids stable through the later load passes). A no-op — returning an empty
/// set — for a program with no `@invariant`. See the module docs.
pub(crate) fn synthesize(ast: &mut Arenas) -> crate::fxhash::FxHashSet<StructId> {
    let mut exempt = crate::fxhash::FxHashSet::default();
    let root = ast.root;
    let prefix_len = if ast.as_form(root, "do").is_some() {
        1
    } else if ast.as_form(root, "module").is_some() {
        2
    } else {
        return exempt;
    };
    let Struct::List(root_children) = ast.get(root).clone() else {
        return exempt;
    };
    if root_children.len() <= prefix_len {
        return exempt;
    }

    // Scan ORIGINAL nodes for `(@ (invariant PRED) (type T …))`. Collect (type-name, sole-newtype-variant?,
    // PRED). The sole-newtype-variant, when present, is `(ctor-name, payload-type-occ)` — the ctor + payload
    // type the construct-def synthesizes against. Bounded to the pre-pass length so the appended defs are not
    // re-scanned.
    let original_len = ast.structure.len() as u32;
    let mut plans: Vec<Plan> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        let Some(tail) = ast.as_form(id, "@") else {
            continue;
        };
        let (Some(&name_occ), Some(&inner)) = (tail.first(), tail.get(1)) else {
            continue;
        };
        let Some(&pred) = ast.as_form(name_occ, "invariant").and_then(|t| match t {
            [only] => Some(only),
            _ => None,
        }) else {
            continue;
        };
        let Some(type_tail) = ast.as_form(inner, "type").map(<[_]>::to_vec) else {
            continue;
        };
        // Name via the shared decoder (bare atom OR parenthesized `(Name a)` generic head), so an
        // `@invariant` on a generic type declared `(type (Box a) …)` is recognized, not skipped.
        let Some(type_name) = type_tail
            .first()
            .and_then(|&n| ast.type_decl_head_name(n))
            .map(str::to_string)
        else {
            continue;
        };
        let sole_variant = sole_newtype_variant(ast, &type_tail);
        let variants = all_variants(ast, &type_tail);
        plans.push(Plan {
            type_name,
            sole_variant,
            variants,
            pred,
        });
    }
    if plans.is_empty() {
        return exempt;
    }

    // Build the defs per plan: the CHECKER (`__invariant_check_T`, Part 1) and — for a single-payload newtype —
    // the CHECKED CONSTRUCTOR (`__invariant_construct_T`, Part 2), the callee the construct-site establish check
    // will route to.
    let mut new_defs: Vec<StructId> = Vec::with_capacity(plans.len());
    for Plan {
        type_name,
        sole_variant,
        variants,
        pred,
    } in plans
    {
        // CHECK-BODY: for a single-payload newtype whose predicate does NOT self-destructure `self`,
        // AUTO-UNWRAP — `(match self (((. T V) __inv_u) PRED[self:=__inv_u]))`. If PRED already destructures
        // `self` (or the type is not a single-payload newtype), use the predicate over `self` directly.
        let auto_unwrap = sole_variant
            .as_ref()
            .is_some_and(|_| !self_destructures_value(ast, pred));
        let check_body = match &sole_variant {
            Some((variant, _)) if auto_unwrap => {
                let body_pred = copy_rewrite(ast, pred, VALUE_BINDER, UNWRAP_BINDER);
                // pattern `(. T V)` — the qualified ctor; binds the payload to `__inv_u`.
                let ctor_pat = {
                    let dot = name(ast, ".");
                    let ty = name(ast, &type_name);
                    let v = name(ast, variant);
                    push_list(ast, vec![dot, ty, v])
                };
                let payload = name(ast, UNWRAP_BINDER);
                let full_pat = push_list(ast, vec![ctor_pat, payload]);
                let arm = push_list(ast, vec![full_pat, body_pred]);
                let match_head = name(ast, "match");
                let self_scrut = name(ast, VALUE_BINDER);
                push_list(ast, vec![match_head, self_scrut, arm])
            }
            // No auto-unwrap: `None` (not a single-payload newtype) OR a self-destructuring PRED (the author
            // already unwraps `self`). Use the predicate over `self` directly (a plain copy).
            _ => copy_rewrite(ast, pred, "", ""),
        };
        let sig = {
            let fn_name = name(ast, &format!("__invariant_check_{type_name}"));
            let param = {
                let colon = name(ast, ":");
                let value = name(ast, VALUE_BINDER);
                let ty = name(ast, &type_name);
                push_list(ast, vec![colon, value, ty])
            };
            push_list(ast, vec![fn_name, param])
        };
        let def_head = name(ast, "def");
        new_defs.push(push_list(ast, vec![def_head, sig, check_body]));

        // CHECKED CONSTRUCTOR (Part 2), single-payload newtype only for now: the callee the construct-site
        // establish check routes a `(T.V x)` construction to, so a value that VIOLATES the invariant TRAPS at
        // construction rather than escaping (the (D) run-time establish enforcement — design §10.2). Shape:
        //
        // ```text
        // (def (__invariant_construct_T (: __inv_p U))
        //   (let ((__inv_v ((. T V) __inv_p))) (if (__invariant_check_T __inv_v) __inv_v (trap "…"))))
        // ```
        //
        // It builds the value ONCE (`__inv_v : T`, a properly-typed nominal — so `__invariant_check_T`'s
        // `self : T` param + its auto-unwrap match are fed the right type), checks it, and yields it or traps.
        // Because it flows through the ordinary resolve/infer/lower like a hand-written def, the newtype
        // erasure of `__inv_v` back to its payload is uniform across the value + the check-call arms (no
        // erased-arg subtlety). WIRED into `lower_sum_new` in the follow-up sub-slice; harmless dead code
        // until then (an unreferenced def is not emitted). A non-newtype (multi-variant/record) invariant type
        // has no single ctor to wrap here — its checked-construct path is a later increment.
        if let Some((variant, payload_ty)) = &sole_variant {
            let payload_ty = *payload_ty;
            let construct_sig = {
                let fn_name = name(ast, &format!("__invariant_construct_{type_name}"));
                // `(: __inv_p U)` — the payload param, typed against the newtype's payload occ (copied so it
                // resolves in the def's own scope, like every other synthesized type reference here).
                let param = {
                    let colon = name(ast, ":");
                    let p = name(ast, CONSTRUCT_PARAM);
                    let ty = copy_rewrite(ast, payload_ty, "", "");
                    push_list(ast, vec![colon, p, ty])
                };
                push_list(ast, vec![fn_name, param])
            };
            // `((. T V) __inv_p)` — the raw construction (the very shape a source `(T.V x)` desugars to).
            let raw_construct = {
                let ctor = {
                    let dot = name(ast, ".");
                    let ty = name(ast, &type_name);
                    let v = name(ast, variant);
                    push_list(ast, vec![dot, ty, v])
                };
                let arg = name(ast, CONSTRUCT_PARAM);
                push_list(ast, vec![ctor, arg])
            };
            // EXEMPT this raw construction from the establish divert: it IS the checked constructor's own
            // `(T.V …)`, so routing it back through `__invariant_construct_T` would recurse forever. The id is
            // stable through the remaining (append-only) load passes, so `lower_sum_new` can test membership.
            exempt.insert(raw_construct);
            // `(if (__invariant_check_T __inv_v) __inv_v (trap "…"))`.
            let check_and_yield = {
                let call = {
                    let check_fn = name(ast, &format!("__invariant_check_{type_name}"));
                    let v = name(ast, CONSTRUCT_VALUE);
                    push_list(ast, vec![check_fn, v])
                };
                let yield_v = name(ast, CONSTRUCT_VALUE);
                let trap_call = {
                    let trap_head = name(ast, "trap");
                    let msg = push_atom(
                        ast,
                        Leaf::Str(format!(
                            "@invariant violated: a constructed `{type_name}` does not satisfy its invariant"
                        ).into()),
                    );
                    push_list(ast, vec![trap_head, msg])
                };
                let if_head = name(ast, "if");
                push_list(ast, vec![if_head, call, yield_v, trap_call])
            };
            // `(let ((__inv_v ((. T V) __inv_p))) <check-and-yield>)`.
            let construct_body = {
                let v_binder = name(ast, CONSTRUCT_VALUE);
                let bind = push_list(ast, vec![v_binder, raw_construct]);
                let bindings = push_list(ast, vec![bind]);
                let let_head = name(ast, "let");
                push_list(ast, vec![let_head, bindings, check_and_yield])
            };
            let def_head = name(ast, "def");
            new_defs.push(push_list(
                ast,
                vec![def_head, construct_sig, construct_body],
            ));
        }

        // MULTI-VARIANT CHECKED CONSTRUCTORS (Part 2, the ≥2-variant sum). A multi-variant sum is NOT a
        // newtype (not erased — it boxes as `Core::SumNew{disc, payloads}`), so the sole-newtype construct-def
        // above is not synthesized for it. Instead synthesize ONE checked constructor PER VARIANT, keyed by
        // the variant's DISCRIMINANT (its declaration index) — the divert at the boxed `Core::SumNew` path
        // has the `disc` in hand, so `__invariant_construct_<T>__d<i>` is the lookup key:
        //
        // ```text
        // (def (__invariant_construct_Shape__d0 (: __inv_p0 Int64))
        //   (let ((__inv_v ((. Shape Circle) __inv_p0))) (if (__invariant_check_Shape __inv_v) __inv_v (trap))))
        // (def (__invariant_construct_Shape__d1 (: __inv_p0 Int64) (: __inv_p1 Int64))
        //   (let ((__inv_v ((. Shape Square) __inv_p0 __inv_p1))) (if (__invariant_check_Shape __inv_v) __inv_v (trap))))
        // ```
        //
        // Each `__inv_v : Shape` is the properly-typed BOXED sum (or, for a single-variant multi-payload
        // newtype, a `Ty::Tuple`-erased value), fed to the whole-value `__invariant_check_Shape` (Part 1, no
        // auto-unwrap — the predicate reads `self` directly via the author's match/accessor). Each inner raw
        // `((. T V) …)` is EXEMPT (recorded), so it builds the plain value when re-reached (no recursion). Runs
        // for any type that did NOT get the sole-payload-newtype path (`sole_variant.is_none()`): a MULTI-variant
        // sum (per-variant, keyed by disc) AND a SINGLE-variant MULTI-payload newtype `(Mk A B)` (its one
        // variant, disc 0 — which erases to a `Ty::Tuple` and would otherwise construct with NO establish
        // check). A NULLARY variant (empty payload list) gets NO construct-def here: it constructs via the
        // nullary-unit path, a distinct injection point deferred to a later increment.
        if sole_variant.is_none() {
            for (disc, (variant, payload_tys)) in variants.iter().enumerate() {
                // `(: __inv_p0 T0) (: __inv_p1 T1) …` — one param per payload, typed against its occ. A NULLARY
                // variant has no payloads → an empty param list (a nullary construct-def).
                let params: Vec<StructId> = payload_tys
                    .iter()
                    .enumerate()
                    .map(|(i, &pty)| {
                        let colon = name(ast, ":");
                        let p = name(ast, &format!("{CONSTRUCT_PARAM}{i}"));
                        let ty = copy_rewrite(ast, pty, "", "");
                        push_list(ast, vec![colon, p, ty])
                    })
                    .collect();
                let construct_sig = {
                    let fn_name = name(ast, &format!("__invariant_construct_{type_name}__d{disc}"));
                    let mut sig = vec![fn_name];
                    sig.extend(params);
                    push_list(ast, sig)
                };
                // The raw construction: for a payload variant, `((. T V) __inv_p0 …)`; for a NULLARY variant,
                // `((. T V) unit)` — the canonical nullary construction form (core-semantics.md: "(None unit)").
                let raw_construct = {
                    let ctor = {
                        let dot = name(ast, ".");
                        let ty = name(ast, &type_name);
                        let v = name(ast, variant);
                        push_list(ast, vec![dot, ty, v])
                    };
                    let mut app = vec![ctor];
                    if payload_tys.is_empty() {
                        app.push(name(ast, "unit"));
                    } else {
                        for i in 0..payload_tys.len() {
                            app.push(name(ast, &format!("{CONSTRUCT_PARAM}{i}")));
                        }
                    }
                    push_list(ast, app)
                };
                exempt.insert(raw_construct);
                // `(if (__invariant_check_T __inv_v) __inv_v (trap "…"))`.
                let check_and_yield = {
                    let call = {
                        let check_fn = name(ast, &format!("__invariant_check_{type_name}"));
                        let v = name(ast, CONSTRUCT_VALUE);
                        push_list(ast, vec![check_fn, v])
                    };
                    let yield_v = name(ast, CONSTRUCT_VALUE);
                    let trap_call = {
                        let trap_head = name(ast, "trap");
                        let msg = push_atom(
                            ast,
                            Leaf::Str(format!(
                                "@invariant violated: a constructed `{type_name}` does not satisfy its invariant"
                            ).into()),
                        );
                        push_list(ast, vec![trap_head, msg])
                    };
                    let if_head = name(ast, "if");
                    push_list(ast, vec![if_head, call, yield_v, trap_call])
                };
                let construct_body = {
                    let v_binder = name(ast, CONSTRUCT_VALUE);
                    let bind = push_list(ast, vec![v_binder, raw_construct]);
                    let bindings = push_list(ast, vec![bind]);
                    let let_head = name(ast, "let");
                    push_list(ast, vec![let_head, bindings, check_and_yield])
                };
                let def_head = name(ast, "def");
                new_defs.push(push_list(
                    ast,
                    vec![def_head, construct_sig, construct_body],
                ));
            }
        }
    }

    // Rebuild the root, appending the checker defs after the existing items (ids stay stable — append-only).
    let mut children: Vec<StructId> = root_children;
    children.extend(new_defs);
    let new_root = push_list(ast, children);
    ast.root = new_root;
    exempt
}

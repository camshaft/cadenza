//! The evaluator — the ONE compile-time reduction of an application through `Meta.apply`.
//!
//! To apply a value to arguments there is a single rule (`prelude-and-resolution.md` §A Form Whose
//! Head Is Not A Grammar Name Is Dispatched By The Kind Of Value Its Head Resolves To): project the
//! head value's `(meta apply)`; if it names something applyable, use it; otherwise the value is "not
//! applyable". The applyable bottom is a native [`Prim`] — an arithmetic operation, or a type
//! constructor. This module owns that reduction so infer and lower share one path.
//!
//! A type constructor applied BUILDS a value by APPENDING arena nodes (`Db::push_*`): `(Int 64)`
//! reduces to a fresh width-64 integer MODULE record — a record whose `(meta t)` is the concrete
//! `Int64` type and whose fields (`max`/`min`/…) are that width's specialization. So `Int64` and
//! `(Int 64)` denote the same reduced module, and projecting a field of it (`Int64.max`) is the
//! ordinary member projection. The built module is a normal arena record, resolved/typed/lowered by
//! the ordinary demand.

use crate::ast::{IntValue, Leaf, StructId};
use crate::db::Db;
use crate::fxhash::FxHashMap as HashMap;
use crate::resolve::resolved_of;
use crate::resolved::{Prim, Resolved, Symbol};
use crate::ty::{IntTy, Scheme, Ty, Width};
use crate::unify::Fresh;
use tracing::trace;

/// Read a value's `(meta t)` — its TYPE — as a [`Scheme`], reducing the type-lambda it holds. An
/// operator's `Meta.t` is a compile-time lambda `(fn (a) (-> (Int a) (-> (Int a) (Int a))))`; this
/// binds each lambda parameter to a FRESH variable, reduces the body's type expression symbolically
/// (so `(Int a)` becomes an integer of that fresh WIDTH variable, shared across every `a`), and closes
/// the mentioned variables into the scheme. This is how `infer` reads an operator's type generically
/// as DATA — one rule for every operator, no per-operator Rust. `None` if the value has no `(meta t)`
/// or its type expression is malformed.
pub fn scheme_of(db: &mut Db, id: StructId, fresh: &mut Fresh) -> Option<Scheme> {
    let t = project_meta(db, id, "t")?;
    // MEMOIZE by the `(meta t)` NODE `t` — shared across every application of the same operator (all
    // `+` occurrences project the one prelude `+`'s type-lambda). Reducing that lambda (`type_in_env`)
    // is ~half the cost of typing operator-heavy code, and it is a pure function of `t`'s structure.
    // The cached scheme is built with CANONICAL numbering (a private `Fresh` from 0), independent of
    // the caller's `fresh` counter; every caller `instantiate`s the result, which renames the bound
    // variables to truly-fresh ones — so one shared canonical scheme serves all uses. (Without the
    // cache each of N operator applications re-reduced the same lambda: O(N) redundant work.)
    if let Some(cached) = db.scheme_cache.get(&t) {
        return cached.clone();
    }
    // Compute with a PRIVATE canonical counter (from 0), independent of the caller's `fresh`. The
    // caller's `fresh` is intentionally NOT advanced: every caller `instantiate`s the returned scheme
    // before using it, and `instantiate` renames ALL of the scheme's bound variables to fresh ones
    // from the caller's counter — so the canonical numbers 0..k never survive into a solved type and
    // cannot collide. (This is why one shared canonical scheme is correct for every use site.)
    let mut canon = Fresh::new();
    let scheme = scheme_of_uncached(db, t, &mut canon);
    db.scheme_cache.insert(t, scheme.clone());
    let _ = fresh; // caller's counter untouched — instantiate freshens the canonical vars
    scheme
}

/// Reduce the `(meta t)` node `t` to a [`Scheme`] using the supplied `fresh` for its variable
/// numbering — the uncached core of [`scheme_of`]. Split out so [`scheme_of`] can compute the cached
/// value with a private canonical counter.
fn scheme_of_uncached(db: &mut Db, t: StructId, fresh: &mut Fresh) -> Option<Scheme> {
    // The `(meta t)` value is either a bare type (a scheme with no quantified vars) or a type-lambda
    // whose parameters are the quantified variables.
    match resolved_of(db, t) {
        Resolved::Lambda { params, body } => {
            // Bind each parameter occurrence to a fresh variable; the same parameter used twice in the
            // body shares its variable (so both operands of `+` unify to one width).
            let mut env: HashMap<StructId, TyOrWidth> = HashMap::default();
            let mut ty_vars = Vec::new();
            let mut width_vars = Vec::new();
            let mut sign_vars = Vec::new();
            for &p in params.iter() {
                let n = fresh.var();
                // A parameter's kind (type var vs width var) is decided by HOW it is used; we record a
                // single fresh number and let the use site pick. Track it as both — the body reduction
                // uses it as a width inside `(Int _)` and as a type elsewhere. A PAIRED fresh sign
                // variable makes an integer-typed operand generic over signedness too: `(Int a)` in an
                // operator's type-lambda denotes an integer of width `a` AND this shared sign, so `+`
                // is `∀(width a, sign s_a). (Int^{s_a}_a) → (Int^{s_a}_a) → (Int^{s_a}_a)` — one
                // operator serving signed and unsigned (a UInt64+UInt64 grounds `s_a` unsigned; an
                // Int64/UInt64 mix is a sign conflict, CDZ0301). The sign is shared across the three
                // `(Int a)` occurrences exactly as the width is.
                let s = fresh.var();
                env.insert(p, TyOrWidth { num: n, sign: s });
                ty_vars.push(n);
                width_vars.push(n);
                sign_vars.push(s);
            }
            let ty = type_in_env(db, body, &env)?;
            trace!(target: "rcdzc::eval", node = t.0, scheme = %ty.render_name(), quantified = params.len(), "read (meta t) as a polymorphic scheme");
            // Only the vars actually mentioned matter; keep the lists (unused ones are harmless).
            Some(Scheme {
                ty_vars,
                width_vars,
                sign_vars,
                ty,
            })
        }
        // A non-lambda `(meta t)` is a monomorphic type.
        _ => {
            let scheme = typeval_of(db, t).map(Scheme::mono);
            if let Some(s) = &scheme {
                trace!(target: "rcdzc::eval", node = t.0, ty = %s.ty.render_name(), "read (meta t) as a monomorphic type");
            }
            scheme
        }
    }
}

/// A lambda parameter's fresh variables — used as a type variable OR a width variable depending on the
/// position it appears in (`(Int a)` → width; a bare `a` → type), with a PAIRED sign variable so an
/// integer operand of variable width is generic over signedness too. `num` serves the type/width axis;
/// `sign` is the sign variable used when the parameter is a variable width inside `(Int a)`.
#[derive(Clone, Copy)]
struct TyOrWidth {
    num: u32,
    sign: u32,
}

/// Reduce a type expression to a `Ty` under an environment binding lambda parameters to fresh
/// variables — the symbolic sibling of `typeval_of`. A parameter reference becomes its variable
/// (`Ty::Var`); `(Int a)` becomes an integer whose WIDTH is the parameter's variable; a ground type or
/// a fully-constant constructor reduces as usual.
fn type_in_env(db: &mut Db, id: StructId, env: &HashMap<StructId, TyOrWidth>) -> Option<Ty> {
    match resolved_of(db, id) {
        // A lambda parameter used as a bare type → its type variable.
        Resolved::Param { binder } => env.get(&binder).map(|v| Ty::Var(v.num)),
        // A bare parameter REFERENCE used as a type (`(-> a (-> a Bool))` — the comparison operand): it
        // resolves to a `Ref` to the parameter occurrence (a body-position name is looked up, not a
        // formal), so follow the ref; if it lands on a parameter in `env`, that is its type variable.
        // (This is the type-position sibling of `width_in_env` following a ref for `(Int a)`.) A ref to
        // anything else reduces concretely below.
        Resolved::Ref { value } if matches!(resolved_of(db, value), Resolved::Param { .. }) => {
            type_in_env(db, value, env)
        }
        // `(Int W)` / `(UInt W)` where W may be a parameter (a width variable) or a constant.
        Resolved::Apply { head, args } => {
            let prim = meta_apply_of(db, head)?;
            match prim {
                Prim::IntCtor | Prim::UIntCtor if args.len() == 1 => {
                    let width = width_in_env(db, args[0], env)?;
                    // When the width is a bound PARAMETER (a width variable), the operand is generic over
                    // signedness too — its sign is that parameter's PAIRED sign variable, so `(Int a)` in
                    // `+`'s type-lambda relates signed and unsigned alike. When the width is CONCRETE
                    // (`(Int 8)`, `(UInt 8)`), the constructor fixes the sign as written — a literal
                    // annotation's type is exactly what it says.
                    let sign = match sign_in_env(db, args[0], env) {
                        Some(s) => crate::ty::Sign::Var(s),
                        None => crate::ty::Sign::Fixed(matches!(prim, Prim::IntCtor)),
                    };
                    Some(Ty::Int(IntTy { sign, width }))
                }
                Prim::FnCtor if args.len() == 2 => {
                    let p = type_in_env(db, args[0], env)?;
                    let r = type_in_env(db, args[1], env)?;
                    Some(Ty::Fn(Box::new(p), Box::new(r)))
                }
                // A GENERIC SUM application `(Option a)` inside a type-lambda — each arg reduced under
                // the env (so `a` becomes its type variable), then `Ty::Sum{decl, args}`. This is what
                // makes a generic variant ctor's `(meta t)` = `(fn (a) (-> a (Option a)))` read as the
                // scheme `∀a. a → Option a`. The owning declaration is on the head's `(meta sum-decl)`.
                Prim::SumCtor => {
                    let decl_field = project_meta(db, head, "sum-decl")?;
                    let decl = match resolved_of(db, decl_field) {
                        Resolved::Int(v) => {
                            crate::ast::StructId(v.to_i64().and_then(|n| u32::try_from(n).ok())?)
                        }
                        _ => return None,
                    };
                    let name = db.type_decl_by_occ(decl)?.name.clone();
                    let mut arg_tys = Vec::with_capacity(args.len());
                    for &a in &args {
                        arg_tys.push(type_in_env(db, a, env)?);
                    }
                    Some(Ty::Sum {
                        decl,
                        name,
                        args: arg_tys,
                    })
                }
                _ => None,
            }
        }
        // A ref / ground record / built typeval with no free parameters — reduce concretely.
        _ => typeval_of(db, id),
    }
}

/// Reduce a width expression under the environment: a parameter → its width variable, a constant → a
/// fixed width.
fn width_in_env(db: &mut Db, id: StructId, env: &HashMap<StructId, TyOrWidth>) -> Option<Width> {
    match resolved_of(db, id) {
        Resolved::Param { binder } => env.get(&binder).map(|v| Width::Var(v.num)),
        Resolved::Int(v) => v
            .to_i64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Width::Fixed),
        Resolved::Ref { value } => width_in_env(db, value, env),
        _ => None,
    }
}

/// The PAIRED sign variable of a width expression that is a bound parameter, or `None` if the width is
/// concrete (a constant). When a width is a variable, its integer type is generic over signedness too;
/// when concrete, the constructor's own signedness applies. (Follows a `Ref` to reach the parameter,
/// mirroring `width_in_env`.)
fn sign_in_env(db: &mut Db, id: StructId, env: &HashMap<StructId, TyOrWidth>) -> Option<u32> {
    match resolved_of(db, id) {
        Resolved::Param { binder } => env.get(&binder).map(|v| v.sign),
        Resolved::Ref { value } => sign_in_env(db, value, env),
        _ => None,
    }
}

/// β-REDUCE a lambda application: build a copy of `body` in the arena with each of `params` replaced
/// by the corresponding `args`, returning the reduced body's occurrence. This is how applying a
/// function `((fn (x) (+ x 1)) 5)` reduces — substitute the argument for the parameter, then evaluate
/// the substituted body. It APPENDS nodes (like the type-constructor builders); the reduced body is an
/// ordinary subtree resolved/typed/lowered by demand. Substitution is capture-safe here because the
/// arguments are closed (they resolve in the caller's scope, and the copy carries fresh occurrences
/// whose parent is the copied structure — a param inside `args` refers to ITS binder, unaffected).
///
/// `arg_of` maps a parameter occurrence to the argument occurrence substituted for it. A body
/// occurrence that IS a reference to one of these params is replaced by its argument; every other node
/// is structurally copied (its children reduced in turn).
pub fn beta_reduce(db: &mut Db, body: StructId, arg_of: &HashMap<StructId, StructId>) -> StructId {
    // A name occurrence in a BINDER POSITION (the name slot of a `let` binding pair, or a match-arm
    // pattern binder) NAMES a binding — it is not a reference to be substituted. Its `resolved_of`
    // walks the scope and, when it shadows a parameter of the same name, reaches that param's binder
    // (which IS in `arg_of`), so the substitution branches below would replace the binding NAME with
    // the argument — turning `(let ((x true)) x)` under param `x` into `(let ((99 true)) x)`, whose
    // body `x` then finds no `x` binding and reports a spurious unbound name (a differently-typed
    // shadow additionally MISCOMPILED to an invalid component). A binder is copied structurally like
    // any other node, never substituted. (Only VALUE references — a bare use of the name — are.)
    if is_binder_occurrence(db, body) {
        let leaf = match db.ast.get(body) {
            crate::ast::Struct::Atom(lid) => db.ast.leaf(*lid).clone(),
            // A non-atom binder (an annotated `(: name T)` binder) copies structurally below.
            _ => return copy_structural(db, body, arg_of),
        };
        return db.push_atom(leaf);
    }
    // A reference to a substituted parameter → its argument. A parameter reference resolves to
    // `Ref { value: <param binder occ> }`; if that binder is one we're substituting, use the arg. The
    // ref is followed TRANSITIVELY: a MATCH-ARM BINDER `k` resolves (Case 5) to the scrutinee
    // occurrence, which itself resolves to the param binder — so `k` reaches the substituted argument
    // through the chain. (A chain that does not end at a substituted binder is not substituted.)
    if let Resolved::Ref { value } = resolved_of(db, body) {
        let mut target = value;
        loop {
            if let Some(&arg) = arg_of.get(&target) {
                return arg;
            }
            match resolved_of(db, target) {
                Resolved::Ref { value: next } => target = next,
                _ => break,
            }
        }
    }
    // The parameter OCCURRENCE itself (in the binder position, not a ref) is not substituted — only
    // references are; but a bare param used as a value resolves to `Param{binder}` — substitute it.
    if let Resolved::Param { binder } = resolved_of(db, body)
        && let Some(&arg) = arg_of.get(&binder)
    {
        return arg;
    }
    // Otherwise structurally copy. A CONSTANT atom (int/bool/float/string leaf) is self-contained — it
    // resolves to its own value regardless of scope, so SHARE it (cheap, no re-resolution needed). A
    // NAME atom, though, resolves by a SCOPE WALK: if it names a binding INSIDE this body (a `let`-local
    // like `x`), sharing the original node would keep its stale resolution to the ORIGINAL (pre-copy)
    // binding — but the copy re-parents everything, so a `let`-local's copied init differs from the
    // original. So COPY a name atom (a fresh occurrence of the same name leaf); `push_list` re-parents
    // it under the copied enclosing form, and its `resolved_of` re-runs against the COPIED scope,
    // binding it to the copied binding (`(def (g n) (let ((x (+ n 1))) (+ x x)))` — the body's `x` must
    // resolve to the COPY's substituted init `(+ arg 1)`, not the original `(+ n 1)`). A param reference
    // never reaches here (the substitution branches above return the arg first); a prelude/free name
    // re-resolves to the same global, harmlessly.
    copy_structural(db, body, arg_of)
}

/// Structurally copy `body` (no substitution at THIS node — the caller decided it is not a
/// substituted reference). A CONSTANT atom (int/bool/float/string leaf) is self-contained — it
/// resolves to its own value regardless of scope, so SHARE it (cheap, no re-resolution needed). A NAME
/// atom resolves by a SCOPE WALK: if it names a binding INSIDE this body (a `let`-local like `x`),
/// sharing the original node would keep its stale resolution to the ORIGINAL (pre-copy) binding — but
/// the copy re-parents everything, so a `let`-local's copied init differs from the original. So COPY a
/// name atom (a fresh occurrence of the same name leaf); `push_list` re-parents it under the copied
/// enclosing form, and its `resolved_of` re-runs against the COPIED scope, binding it to the copied
/// binding (`(def (g n) (let ((x (+ n 1))) (+ x x)))` — the body's `x` must resolve to the COPY's
/// substituted init `(+ arg 1)`, not the original `(+ n 1)`). A prelude/free name re-resolves to the
/// same global, harmlessly.
fn copy_structural(db: &mut Db, body: StructId, arg_of: &HashMap<StructId, StructId>) -> StructId {
    match db.ast.get(body).clone() {
        crate::ast::Struct::Atom(lid) => match db.ast.leaf(lid).clone() {
            crate::ast::Leaf::Name(_) => {
                let leaf = db.ast.leaf(lid).clone();
                db.push_atom(leaf)
            }
            _ => body, // a constant leaf — self-contained, share it.
        },
        crate::ast::Struct::List(children) => {
            let reduced: Vec<StructId> = children
                .iter()
                .map(|&c| beta_reduce(db, c, arg_of))
                .collect();
            db.push_list(reduced)
        }
    }
}

/// Whether `id` is a name occurrence in a BINDER POSITION — the name slot of a `let` binding pair, or a
/// match-arm pattern binder. Such an occurrence NAMES a binding; it is not a value reference, so
/// β-reduction must copy it structurally rather than substitute an argument for it (else a binder that
/// shadows a same-named parameter would be replaced by the argument, destroying the binding). Detected
/// structurally by the parent chain, so no name-specific knowledge is needed:
///
///  - `id` is the FIRST child of a `(name init)` pair whose grandparent is a `let`'s bindings-list.
///  - `id` is the FIRST child (the pattern) of a `(pattern body)` arm whose grandparent is a `match`.
fn is_binder_occurrence(db: &Db, id: StructId) -> bool {
    let Some(pair) = db.parent_of(id) else {
        return false;
    };
    // `id` must be the pattern/name slot — the pair's/arm's first child.
    let crate::ast::Struct::List(pair_children) = db.ast.get(pair) else {
        return false;
    };
    if pair_children.first() != Some(&id) {
        return false;
    }
    let Some(grandparent) = db.parent_of(pair) else {
        return false;
    };
    // A `let` binding pair: the grandparent is the bindings-LIST of a `let` — its own parent is a `let`
    // form whose FIRST tail element is that list (the bindings position, distinct from the body).
    if let Some(great) = db.parent_of(grandparent)
        && let Some(let_tail) = db.ast.as_form(great, "let")
        && let_tail.first() == Some(&grandparent)
    {
        return true;
    }
    // A `match` arm: the grandparent is the `match` form and `pair` is one of its ARMS (a tail element,
    // i.e. not the scrutinee at tail position 0).
    if let Some(tail) = db.ast.as_form(grandparent, "match")
        && tail.iter().skip(1).any(|&arm| arm == pair)
    {
        return true;
    }
    false
}

/// If `head` reduces to a lambda, apply it to `args` by β-reduction, returning the reduced body's
/// occurrence. Follows a `Ref` to the lambda (so a `let`-bound function applies too). Parameters match
/// positionally: exact arity β-reduces the whole body; MORE args than params reduces then applies the
/// remainder to the result (curried); FEWER args is a partial application, not yet supported (`Err`).
/// A non-lambda head is `Ok(None)` — the caller tries the `(meta apply)` primitive path instead.
pub fn apply_lambda(
    db: &mut Db,
    head: StructId,
    args: &[StructId],
) -> Result<Option<StructId>, String> {
    let (params, body) = match lambda_of(db, head) {
        Some(lam) => lam,
        None => return Ok(None),
    };
    // A RECURSIVE function is not β-reducible to a normal form at compile time — decline BEFORE
    // inlining (the static check, not the depth backstop, is what stops the exponential node blow-up on
    // a branching recursive body). This is the one place every lambda application funnels through (both
    // `infer` and `lower` reach a lambda head here, and currying recurses through here), so the one
    // check covers them all.
    if is_recursive(db, body) {
        trace!(target: "rcdzc::eval", body = body.0, "decline: recursive function (needs runtime specialization)");
        return Err(
            "a recursive function needs runtime specialization (not yet built)".to_string(),
        );
    }
    trace!(target: "rcdzc::eval", body = body.0, params = params.len(), args = args.len(), "β-reduce lambda application");
    if args.len() < params.len() {
        return Err("partial application of a function is not yet supported".to_string());
    }
    let mut arg_of: HashMap<StructId, StructId> = HashMap::default();
    for (p, a) in params.iter().zip(args.iter()) {
        // PIN each argument's meaning at the CALL SITE before reducing. `beta_reduce` splices the
        // shared argument occurrence into a freshly `push_list`-built copy of the body, and
        // `push_list` re-parents its children — which moves the argument's root out of the caller's
        // scope. Resolving the argument subtree now memoizes every node against its caller-side
        // position, so the later re-parent cannot change how a caller-bound name inside the argument
        // resolves (`(let ((k 10)) (inc k))` — `k` must stay bound to the caller's `let`). The body's
        // OWN names still resolve lazily after the copy, against the copied scope, as intended.
        crate::resolve::resolve_subtree(db, *a);
        // A body reference to a parameter binds to the parameter's NAME occurrence (resolve's
        // `binder_in` returns the name, seeing through a `(: name T)` annotated binder). So key the
        // substitution on the name occurrence too, not the raw signature child (which is the `(:…)`
        // node for an annotated param) — else the arg would never match the body's references.
        arg_of.insert(param_name_occ(db, *p), *a);
    }
    let reduced = beta_reduce(db, body, &arg_of);
    let rest = &args[params.len()..];
    if rest.is_empty() {
        Ok(Some(reduced))
    } else {
        match apply_lambda(db, reduced, rest)? {
            Some(r) => Ok(Some(r)),
            None => Err("applied more arguments than the function accepts".to_string()),
        }
    }
}

/// The NAME occurrence a parameter binds — seeing through a `(: name T)` annotated binder to `name`,
/// or the occurrence itself for a bare parameter. This is the identity a body reference to the
/// parameter resolves to (via resolve's `binder_in`), so β-substitution keys on it. Mirrors resolve's
/// `param_name` on the evaluator side (a param occurrence is either a name atom or a `(: name T)` list).
pub(crate) fn param_name_occ(db: &Db, param: StructId) -> StructId {
    if db.ast.as_name(param).is_some() {
        return param;
    }
    if let Some(tail) = db.ast.as_form(param, ":")
        && let Some(&name_occ) = tail.first()
    {
        return name_occ;
    }
    param
}

/// The body occurrence of the lambda `head` reduces to, if any — the stable per-function identity the
/// recursion guard keys on (so a recursive call, which re-enters the same body, is detected).
pub fn lambda_body(db: &mut Db, head: StructId) -> Option<StructId> {
    lambda_of(db, head).map(|(_, body)| body)
}

/// The def-BODY occurrence a NULLARY-def head refers to, if `head` names one — a nullary def resolves
/// its name to a `Ref` straight at its body (no `Lambda` wrapper). Returns that body so a caller can
/// ask `is_recursive` of it (a nullary self-call `(def (f) (f))` must be detected as recursive and
/// declined, not inlined without end). `None` for a head that is not a nullary-def reference.
pub fn lambda_body_of_nullary(db: &mut Db, head: StructId) -> Option<StructId> {
    match resolved_of(db, head) {
        Resolved::Ref { value } if db.def_index_by_body(value).is_some() => Some(value),
        Resolved::Ref { value } => lambda_body_of_nullary(db, value),
        _ => None,
    }
}

/// The PARAMETER NAME occurrences of the lambda `head` reduces to, in signature order — each the
/// identity a body reference resolves to and the node whose `type_of` is the parameter's declared
/// type (its annotation, or `Any` for a bare param). Used by infer to check each call argument against
/// its parameter's type at the call site, which β-reduction alone erases (the parameter's annotation
/// is dropped when its argument is substituted into the body).
pub fn lambda_params_of(db: &mut Db, head: StructId) -> Option<Vec<StructId>> {
    let (params, _) = lambda_of(db, head)?;
    Some(params.iter().map(|&p| param_name_occ(db, p)).collect())
}

/// Is the function whose body is `body` (transitively) RECURSIVE — does the static call graph contain
/// a cycle back to `body`? A recursive function cannot be β-reduced to a normal form at compile time
/// (it would inline without end, and one that branches — several self-calls per body, like a CBOR tree
/// reader — explodes EXPONENTIALLY in appended nodes long before any depth bound), so the evaluator
/// reads this BEFORE reducing a call and DECLINES a recursive one. The evaluator only monomorphizes by
/// reduction; a recursive call is instead emitted as a runtime `Core::Call` by [`crate::lower`] (when
/// the callee's signature is determined), so this decline is a hand-off to lowering, not an
/// unimplemented feature. This is the SOUND, PURELY STRUCTURAL recursion signal — a static property of
/// the resolved call graph, computed WITHOUT reducing (no `apply_lambda`, no arena growth), unlike the
/// body-on-the-stack set that false-positived on `(f (f v))`. Cached per body occurrence (the fixed
/// def/`let` structure makes the verdict stable), the same memoization `build_cache` uses.
pub fn is_recursive(db: &mut Db, body: StructId) -> bool {
    if let Some(&v) = db.recursive.get(&body) {
        return v;
    }
    // A cycle back to `body`: search the callee edges; `body` is recursive iff some path of ≥1 call
    // returns to it. The search is an ITERATIVE worklist DFS over the call graph (an explicit stack,
    // no native recursion over the potentially-unbounded graph), and it reuses the Db's pooled
    // buffers rather than allocating per call: take them out (freeing `db` for the walk), clear, use,
    // put back. `visited` bounds the walk to each reachable body once (finite — bodies are arena
    // nodes). Only THIS top-level query is cached; the bodies visited along the way answer "does C
    // reach `body`", not "is C recursive", so they are not memoized here.
    let mut visited = std::mem::take(&mut db.rec_visited);
    let mut worklist = std::mem::take(&mut db.rec_worklist);
    visited.clear();
    worklist.clear();

    // Seed the frontier with `body`'s direct callees; then expand each not-yet-seen callee. A frontier
    // entry equal to `body` closes the cycle. (`body` itself is never inserted into `visited`, so a
    // path returning to it is detected on pop.) Each node's edges come from the memoized `callees_of`,
    // so a body reached from many ancestors is walked (and `resolved_of`-cloned) only ONCE overall.
    ensure_callees(db, body);
    worklist.extend_from_slice(&db.callee_edges[&body]);
    let mut result = false;
    while let Some(node) = worklist.pop() {
        if node == body {
            result = true;
            break;
        }
        if visited.insert(node) {
            // Compute-then-read: `ensure_callees` fills the memo (needs `&mut db`), then the cached
            // slice is pushed into the local `worklist` (which was taken out of `db`, so borrowing
            // `db.callee_edges` immutably here does not conflict). No per-visit allocation.
            ensure_callees(db, node);
            worklist.extend_from_slice(&db.callee_edges[&node]);
        }
    }

    // Return the buffers for the next body's walk to reuse.
    db.rec_visited = visited;
    db.rec_worklist = worklist;
    db.recursive.insert(body, result);
    trace!(target: "rcdzc::eval", body = body.0, recursive = result, "recursion analysis");
    result
}

/// ENSURE `node`'s direct callee edges are computed and cached in `db.callee_edges`, MEMOIZED. The
/// edge set is a pure function of the fixed resolved structure, so it is computed once — via
/// [`collect_callees`] — and cached; the recursion DFS then reads the cached slice directly (no clone,
/// no re-walk). Computing the walk re-`resolved_of`s the subtree and clones a `Resolved` per node, so
/// doing it once per body — rather than once per ancestor that reaches the body — is the O(N²)→O(N)
/// churn removal. Returns nothing: the caller reads `db.callee_edges[&node]` after this returns.
fn ensure_callees(db: &mut Db, node: StructId) {
    if db.callee_edges.contains_key(&node) {
        return;
    }
    let mut edges = Vec::new();
    collect_callees(db, node, &mut edges);
    db.callee_edges.insert(node, edges);
}

/// Collect the callee bodies of the function whose body expression is (rooted at) `node` — a LOCAL,
/// NON-REDUCING syntactic walk of one function's body. For each application, the head's statically-known
/// callee body (via [`callee_body`], following refs to a lambda WITHOUT reducing) is one edge; the walk
/// descends into every sub-expression that runs when this body runs. It STOPS at a nested `fn`
/// boundary — a lambda defined inside this body is a SEPARATE call-graph node (its own calls are edges
/// from IT, explored only if IT is called), so descending into it would conflate two nodes. This keeps
/// the graph precise: over-reporting would decline a terminating fold, under-reporting would let a
/// recursive one reach the (slow, exponential) depth backstop.
///
/// This is the UNCACHED walk; callers in the recursion analysis go through [`callees_of`] to memoize
/// per node. (`collect_callees` recurses into sub-expressions of the SAME body, accumulating into
/// `out`; the memo keys only whole body/callee nodes the DFS visits, not every interior node.)
fn collect_callees(db: &mut Db, node: StructId, out: &mut Vec<StructId>) {
    match resolved_of(db, node) {
        Resolved::Apply { head, args } => {
            if let Some(cb) = callee_body(db, head) {
                out.push(cb);
            }
            // Descend into the head ONLY if it is itself an application (a computed head like
            // `((g x) y)`); a head that is a plain function reference is captured by `callee_body`
            // above, and descending into the lambda/ref it names would cross into another node.
            if matches!(resolved_of(db, head), Resolved::Apply { .. }) {
                collect_callees(db, head, out);
            }
            for a in args {
                collect_callees(db, a, out);
            }
        }
        Resolved::If { cond, then_, else_ } => {
            collect_callees(db, cond, out);
            collect_callees(db, then_, out);
            collect_callees(db, else_, out);
        }
        // A match's scrutinee and every arm BODY run when this body runs — a self-call inside an arm
        // (e.g. `sum-to`'s match base case) is a real edge, so descend into each. (A pattern is not
        // executed code — it is a probe — so it contributes no callee.)
        Resolved::Match { scrutinee, arms } => {
            collect_callees(db, scrutinee, out);
            for (_, body) in arms {
                collect_callees(db, body, out);
            }
        }
        Resolved::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_callees(db, value, out);
            }
            collect_callees(db, body, out);
        }
        Resolved::Record { fields } => {
            for (_, &value) in fields.iter() {
                collect_callees(db, value, out);
            }
        }
        Resolved::Member { operand, .. } => collect_callees(db, operand, out),
        // A tuple's elements and a projection's operand run when this body runs — a call inside them is
        // a real edge, so descend.
        Resolved::Tuple { elems } => {
            for &e in elems.iter() {
                collect_callees(db, e, out);
            }
        }
        Resolved::Proj { operand, .. } => collect_callees(db, operand, out),
        // An annotation is transparent — a call inside `(: (f x) T)` is a real edge of this body.
        Resolved::Annot { expr, .. } => collect_callees(db, expr, out),
        // A nested `fn` is a separate node — do NOT descend (its calls are its own edges). A bare ref,
        // a leaf, a prim, a param, a type value, a poison have no callees of THIS body.
        // A `SumPayload` reads the scrutinee's payload; the scrutinee's own callees are collected via the
        // enclosing match's scrutinee position, so a payload reference adds no edge of this body.
        Resolved::Lambda { .. }
        | Resolved::Ref { .. }
        | Resolved::SumPayload { .. }
        | Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Poison(_) => {}
    }
}

/// The statically-known callee body of an application head, if any — following a `Ref` to the lambda it
/// binds (a `let`-bound function, or a top-level def, which resolves straight to a `Lambda`), WITHOUT
/// reducing. `None` for a head whose callee cannot be known without evaluation (a computed head, a
/// primitive, a non-function) — such an edge is left to the depth backstop rather than forced here.
fn callee_body(db: &mut Db, head: StructId) -> Option<StructId> {
    match resolved_of(db, head) {
        Resolved::Lambda { body, .. } => Some(body),
        Resolved::Ref { value } => {
            // A NULLARY def resolves its name straight to its BODY occurrence (a value, not a `Lambda`
            // wrapper — that is what makes a bare `g` denote the body value). So a call `(g)` reaching
            // this `Ref` has its callee body AT `value`: recognize `value` as a def body and return it,
            // rather than recursing INTO it (which would classify the body's own head, missing the
            // self-call edge). Without this a nullary self-recursion `(def (f) (f))` is not detected as
            // recursive, so the call inlines without end. A `Ref` to anything else (a `let`-bound
            // function, a nested ref) still follows through.
            if db.def_index_by_body(value).is_some() {
                Some(value)
            } else {
                callee_body(db, value)
            }
        }
        _ => None,
    }
}

/// If the value at `id` reduces to a lambda, its parameters and body. Follows a `Ref` (a `let`-bound
/// function) AND reduces an `Apply` head (a function RETURNED by another application — `(adder 10)`
/// reduces to the inner `(fn (x) …)`, so `((adder 10) 5)` applies it). This is what makes curried and
/// returned functions work: the head of an application is itself evaluated to a lambda first.
fn lambda_of(db: &mut Db, id: StructId) -> Option<(std::sync::Arc<[StructId]>, StructId)> {
    match resolved_of(db, id) {
        Resolved::Lambda { params, body } => Some((params, body)),
        Resolved::Ref { value } => lambda_of(db, value),
        Resolved::Apply { head, args } => {
            // Reduce the inner application to a value, then see if THAT is a lambda. This reduction is
            // itself a β-reduction, so run it UNDER THE DEPTH GUARD: a self-referential nullary call
            // `(def (f) (f))` resolves `f` to a `Ref` at the body `(f)`, so reducing THIS application
            // re-enters `lambda_of` on the same body — an unbounded loop that would overflow the native
            // stack. Past `REDUCE_DEPTH_LIMIT` the guard denies entry and this yields `None` (not a
            // lambda / can't be reduced here), matching how a too-deep β-reduction declines elsewhere.
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            let reduced = apply_lambda(g, head, &args).ok().flatten()?;
            lambda_of(g, reduced)
        }
        _ => None,
    }
}

/// The `(meta apply)` primitive of the head value at `id`, if the head is applyable — project the
/// `apply` field (in the `meta` namespace) and, following a ref, read the `Prim` it holds. `None`
/// means "not applyable" (no `(meta apply)`, or it is not a primitive). This is the one dispatch step
/// every application takes.
pub fn meta_apply_of(db: &mut Db, id: StructId) -> Option<Prim> {
    let field = project_meta(db, id, "apply")?;
    prim_of(db, field)
}

/// The DISCRIMINANT of the sum variant the value at `id` is a constructor for, if it is one — read off
/// the record's `(meta variant)` channel (an integer literal the sum synthesis wrote). `None` for any
/// value that is not a variant constructor (no `(meta variant)`). This is what a variant application
/// reads at lowering to know which variant it builds (the analogue of a conversion reading its target
/// width off the solved type) — the discriminant lives in the metadata, not in the shared `SumNew`
/// prim. It also MARKS a record as a variant value/constructor, distinguishing a nullary variant
/// (`None` — a value) from a genuine type-value record (both carry `(meta t)`; only a variant carries
/// `(meta variant)`).
pub fn variant_disc_of(db: &mut Db, id: StructId) -> Option<u32> {
    let field = project_meta(db, id, "variant")?;
    match resolved_of(db, field) {
        Resolved::Int(v) => v.to_i64().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
}

/// The PAYLOAD type of the sum variant the value at `id` constructs — for a variant constructor
/// `(. Sum V)` whose `(meta t)` is `(-> payload Sum)`, the arrow's parameter `payload`. Read via the
/// ordinary `(meta t)` scheme, so it works for any single-payload variant. A NULLARY variant's type is
/// the bare sum (no arrow) → `None` (no payload to bind). A MULTI-payload variant's type is a curried
/// `(-> p0 (-> p1 Sum))` → the FIRST parameter here (Stage-5 binds a single scalar payload; a
/// multi-payload destructure is a later increment, so only the one-payload arrow is meaningful).
pub fn variant_payload_type(db: &mut Db, id: StructId) -> Option<crate::ty::Ty> {
    let mut fresh = Fresh::new();
    let scheme = scheme_of(db, id, &mut fresh)?;
    match scheme.ty {
        Ty::Fn(param, _) => Some(*param),
        _ => None,
    }
}

/// Project a meta-channel field named `key` from the record value at `id`, following a `Ref` to the
/// record. Returns the field's value occurrence, or `None` if the value is not a record or has no such
/// field. The one generic projection, restricted to the `meta` namespace.
pub fn project_meta(db: &mut Db, id: StructId, key: &str) -> Option<StructId> {
    let sym = Symbol {
        namespace: Some("meta".to_string()),
        name: key.to_string(),
    };
    project_field(db, id, &sym)
}

/// Project a field `key` from the record value at `id`, returning its value occurrence. The generic
/// record projection both member access and the meta channel use. It REDUCES the operand to a record
/// first: following a `Ref`, and reducing a type-constructor application (so `(. (Int 64) max)`
/// projects `max` off the module `(Int 64)` builds). `None` if the operand is not (reducible to) a
/// record, or has no such field.
pub fn project_field(db: &mut Db, id: StructId, key: &Symbol) -> Option<StructId> {
    let rec = reduce_to_record_id(db, id)?;
    match resolved_of(db, rec) {
        Resolved::Record { fields } => fields.get(key).copied(),
        _ => None,
    }
}

/// Reduce the value at `id` to the occurrence of a RECORD, if it reduces to one: following a `Ref`,
/// reducing a type-constructor application (`(Int 64)` → its module), and following a member
/// projection (a nested record). Returns the record's occurrence, or `None` if it does not reduce to a
/// record. This is the single "reduce to a record" step both projection and its checks use.
fn reduce_to_record_id(db: &mut Db, id: StructId) -> Option<StructId> {
    match resolved_of(db, id) {
        Resolved::Record { .. } => Some(id),
        Resolved::Ref { value } => reduce_to_record_id(db, value),
        Resolved::Apply { head, args } => {
            // Reducing the application to a record is a β-reduction — run it UNDER THE DEPTH GUARD. A
            // self-referential nullary call `(def (f) (f))` resolves `f` to a `Ref` at the body `(f)`,
            // and `meta_apply_of(head)` → `project_field` → `reduce_to_record_id` re-enters this same
            // application, an unbounded loop that would overflow the native stack. Past
            // `REDUCE_DEPTH_LIMIT` the guard denies entry and this yields `None` (does not reduce to a
            // record), matching how a too-deep β-reduction declines elsewhere.
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            // A USER-DEF / LAMBDA head β-reduces — a field read of a call-RETURNED record
            // `(. (mk a b) field)` folds through to the field's value, with NO heap build (the record
            // analogue of the tuple case above). A recursive callee declines here and the projection
            // stays a runtime `Core::Proj`/record read.
            if lambda_body(g, head).is_some() {
                return match apply_lambda(g, head, &args) {
                    Ok(Some(reduced)) => reduce_to_record_id(g, reduced),
                    _ => None,
                };
            }
            let prim = meta_apply_of(g, head)?;
            if prim.is_arith() {
                return None;
            }
            let built = reduce_ctor(g, prim, id, &args).ok()?;
            reduce_to_record_id(g, built)
        }
        // A member projection whose result is itself a record (a nested record chain).
        Resolved::Member { operand, key } => {
            let value = match member_value(db, operand, &key) {
                Member::Field(v) => v,
                _ => return None,
            };
            reduce_to_record_id(db, value)
        }
        _ => None,
    }
}

/// The outcome of a member projection over a reducible operand: the field's value occurrence, or a
/// precise fault (a non-record operand, or an absent field on a closed record).
pub enum Member {
    /// The field's value occurrence — project succeeded.
    Field(StructId),
    /// The operand is not a record (member access requires one) → CDZ0201.
    NotRecord,
    /// The operand is a record but has no field `key` (a closed record rejects) → CDZ0201.
    NoField,
}

/// Resolve a member access `(. operand key)` to its field's value occurrence, reducing the operand to
/// a record. Shared by `infer` (types the field's value) and `lower` (lowers it) so both agree on the
/// one projection. If the operand reduces to a record, the field is projected (`Field`) or reported
/// absent (`NoField`); otherwise `NotRecord`.
pub fn member_value(db: &mut Db, operand: StructId, key: &Symbol) -> Member {
    match reduce_to_record_id(db, operand) {
        Some(rec) => match resolved_of(db, rec) {
            Resolved::Record { fields } => match fields.get(key) {
                Some(&v) => {
                    trace!(target: "rcdzc::eval", operand = operand.0, key = %key.name, value = v.0, "project: field found");
                    Member::Field(v)
                }
                None => {
                    trace!(target: "rcdzc::eval", operand = operand.0, key = %key.name, "project: no such field");
                    Member::NoField
                }
            },
            _ => Member::NotRecord,
        },
        None => {
            trace!(target: "rcdzc::eval", operand = operand.0, key = %key.name, "project: operand is not a record");
            Member::NotRecord
        }
    }
}

/// The sorted-position index of field `key` in a RUNTIME record operand — one whose value does not
/// reduce to a compile-time-visible record (returned from a call, selected by an `if`) but whose TYPE
/// is a record carrying the field. `None` if the operand is not a record type, or is a record type
/// lacking the field. At run time a record IS a positional heap array in sorted-key order — the
/// `Ty::Record`/`Core::Record` `BTreeMap` order the backend's `arr-set`/`arr-get` share — so a field
/// read on such an operand lowers to a `Core::Proj` at this index, the same `arr-get` a tuple
/// projection uses. The symmetric counterpart of a tuple projection falling through to a runtime read.
pub fn runtime_member_index(db: &mut Db, operand: StructId, key: &Symbol) -> Option<usize> {
    match crate::infer::type_of(db, operand) {
        crate::ty::Ty::Record(fields) => fields.keys().position(|k| k == key),
        _ => None,
    }
}

/// Reduce the value at `id` to the element occurrences of a COMPILE-TIME-VISIBLE tuple, if it reduces
/// to one: a `(tuple …)` literal directly, or following a `Ref` to one. Returns the ordered element
/// occurrences. `None` if the operand is not (statically reducible to) a tuple — a runtime tuple (a
/// function parameter, a `Core::Let`-kept binding) is opaque here, so its projection stays a runtime
/// `Core::Proj` rather than folding. This is the tuple analogue of `reduce_to_record_id`, and it is what
/// makes a projection of a visible tuple fold (no heap) while a projection of a runtime tuple does not.
///
/// A `Ref` to a binding that was KEPT as a runtime `Core::Let` is NOT followed (the value lives in a
/// slot at run time, not visibly here) — so a multi-use `let`-bound tuple projects at run time. A
/// single-use / propagated binding's ref IS followed (it inlines to the tuple literal, which folds).
pub fn reduce_to_tuple_elems(db: &mut Db, id: StructId) -> Option<std::sync::Arc<[StructId]>> {
    match resolved_of(db, id) {
        Resolved::Tuple { elems } => Some(elems),
        Resolved::Ref { value } => {
            if db.kept_bindings.contains(&value) {
                // A kept multi-use runtime binding — opaque; its projection is a runtime read.
                None
            } else {
                reduce_to_tuple_elems(db, value)
            }
        }
        // A tuple that is itself an element projected from another compile-time-visible tuple.
        Resolved::Proj { operand, index } => {
            let elems = reduce_to_tuple_elems(db, operand)?;
            let elem = *elems.get(index)?;
            reduce_to_tuple_elems(db, elem)
        }
        Resolved::Annot { expr, .. } => reduce_to_tuple_elems(db, expr),
        // A `(tuple a b)` written with the shadowable alias name — reduce the application to the
        // symbol-headed primitive (`reduce_ctor` pins the arg scopes, then builds `((,) a b)`) and read
        // its elements, so a constant tuple built via the alias folds exactly as one written `((,) a b)`
        // does. Mirrors `reduce_to_record_id`'s `Apply` arm; run under the depth guard.
        Resolved::Apply { head, args } => {
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            // A USER-DEF / LAMBDA head β-reduces — inlining the (pure) call — so a projection of a
            // call-RETURNED tuple `(. (mk a b) i)` folds straight through to the element `a`/`b`, with
            // NO heap build. This mirrors the lambda branch `core_of`'s `Apply` arm takes when lowering
            // the whole call, so the keep-decision (a projection-only compound folds through) and the
            // fold agree: without this the call reduces to a `Core::Tuple` at lowering but the fold
            // could not see through it, so each projection re-lowered the body and REBUILT the tuple
            // (one `arr-alloc` per projection). A RECURSIVE callee declines here (`apply_lambda`'s
            // `is_recursive` guard) and the projection stays a runtime `Core::Proj` (the heap read).
            if lambda_body(g, head).is_some() {
                return match apply_lambda(g, head, &args) {
                    Ok(Some(reduced)) => reduce_to_tuple_elems(g, reduced),
                    _ => None,
                };
            }
            let prim = meta_apply_of(g, head)?;
            if prim.is_arith() {
                return None;
            }
            let built = reduce_ctor(g, prim, id, &args).ok()?;
            reduce_to_tuple_elems(g, built)
        }
        _ => None,
    }
}

/// The primitive the value at `id` denotes, following a `Ref` — `+` resolves to a `Ref` to its
/// `(intrinsic +)` node, which resolves to `Prim::Add`. `None` if not a primitive.
pub fn prim_of(db: &mut Db, id: StructId) -> Option<Prim> {
    match resolved_of(db, id) {
        Resolved::Prim(p) => Some(p),
        Resolved::Ref { value } => prim_of(db, value),
        _ => None,
    }
}

/// Reduce a TYPE-CONSTRUCTOR application: apply the constructor `prim` to `args`, returning the
/// occurrence its result reduces to (a built module record for `Int`/`UInt`; a `(typeval …)` for
/// `->`), or `Err` with a decline/reject message. Arithmetic prims are NOT handled here — they fold in
/// `lower`; this is the type-level reduction the evaluator performs.
///
/// `Int`/`UInt` require a single constant WIDTH argument and build the integer module for it. `->`
/// requires two type arguments and builds a function type-value. A wrong arity or a non-constant width
/// is a reject.
///
/// `origin` is the occurrence being reduced (the `(tuple …)`/`(record …)` application node) — a
/// STABLE, UNIQUE per-construction-site identity `push_list` assigns freshly. The value-constructor
/// build-once cache keys on it (see the `TupleNew`/`RecordNew` arm); the type-constructor arms key on
/// their argument VALUES and ignore it.
pub fn reduce_ctor(
    db: &mut Db,
    prim: Prim,
    origin: StructId,
    args: &[StructId],
) -> Result<StructId, String> {
    match prim {
        Prim::IntCtor | Prim::UIntCtor => {
            let signed = matches!(prim, Prim::IntCtor);
            if args.len() != 1 {
                return Err(format!(
                    "{} takes one width argument",
                    if signed { "Int" } else { "UInt" }
                ));
            }
            // A width the compiler cannot resolve to a compile-time natural reduces to the invalid
            // sentinel width 0, so the built module's type is `Ty::Int(Fixed(0))` — which
            // `int_bounds`/`fits_width` reject as CDZ0302 exactly as an explicit `(UInt 0)` is (the bad
            // width is rejected AT THE ANNOTATION, not silently dropped to the default Int64). This
            // covers BOTH a MALFORMED concrete width (negative/over-u32/non-integer) AND a NON-CONSTANT
            // one — a RUNTIME parameter, an unbound name, a non-constant computation: an integer type
            // MUST be indexed by a compile-time width (numeric-model.md §An Integer Type Is Indexed By A
            // Compile-Time Width), so a runtime value in width position is rejected, not accepted-and-
            // ignored. (A width VARIABLE `(Int a)` inside an operator's `(meta t)` scheme never reaches
            // here — it is read symbolically by `width_in_env` as `Width::Var`, not by `read_width`.)
            let width = match read_width(db, args[0]) {
                WidthRead::Fixed(w) => w,
                WidthRead::Malformed | WidthRead::NotConst => 0,
            };
            // Build once per (ctor, width): the first demand appends the module, every later demand —
            // repeated on this occurrence, or another `(Int 64)` elsewhere — returns the same node, so
            // the reduction is idempotent and the arena holds one module per width.
            let key = crate::db::BuildKey {
                prim,
                args: vec![width as i64],
            };
            if let Some(cached) = db.cached_build(&key) {
                trace!(target: "rcdzc::eval", signed, width, node = cached.0, "ctor (Int/UInt): build-cache hit");
                return Ok(cached);
            }
            let built = build_int_module(db, signed, width);
            db.cache_build(key, built);
            trace!(target: "rcdzc::eval", signed, width, node = built.0, "ctor (Int/UInt): built integer module");
            Ok(built)
        }
        Prim::FnCtor => {
            if args.len() != 2 {
                return Err("-> takes two type arguments".to_string());
            }
            let p =
                typeval_of(db, args[0]).ok_or_else(|| "-> parameter is not a type".to_string())?;
            let r = typeval_of(db, args[1]).ok_or_else(|| "-> result is not a type".to_string())?;
            let fn_ty = crate::ty::Ty::Fn(Box::new(p), Box::new(r));
            trace!(target: "rcdzc::eval", ty = %fn_ty.render_name(), "ctor (->): built function type-value");
            Ok(encode_typeval(db, &fn_ty))
        }
        // `Tuple` — VARIADIC over its element TYPES: reduce each arg to a `Ty`, build `Ty::Tuple`. Its
        // arity + element types ARE the type, so `(: e (Tuple T…))` checks the value's shape against it
        // (a wrong arity/element is CDZ0203 at the annotation, via `agrees_with`).
        Prim::TupleCtor => {
            let mut elems = Vec::with_capacity(args.len());
            for (i, &a) in args.iter().enumerate() {
                let t =
                    typeval_of(db, a).ok_or_else(|| format!("Tuple element {i} is not a type"))?;
                elems.push(t);
            }
            let tup = crate::ty::Ty::Tuple(elems.into());
            trace!(target: "rcdzc::eval", ty = %tup.render_name(), "ctor (Tuple): built tuple type-value");
            Ok(encode_typeval(db, &tup))
        }
        // `Record` — VARIADIC over `(name type)` field pairs: each arg is a raw `(name T)` list; read the
        // field name (first child) and reduce the type (second child) to a `Ty`. The field-name SET +
        // per-field types ARE the record type, so `(: e (Record (a T)…))` checks the value's field set and
        // field types against it (a wrong field type is CDZ0203 at the annotation, via `unify`). A
        // duplicate field name collapses in the `BTreeMap` — the same fixed-SET rule a record VALUE has.
        Prim::RecordCtor => {
            let mut fields: std::collections::BTreeMap<crate::resolved::Symbol, crate::ty::Ty> =
                std::collections::BTreeMap::new();
            for (i, &a) in args.iter().enumerate() {
                let pair = match db.ast.get(a).clone() {
                    crate::ast::Struct::List(children) if children.len() == 2 => children,
                    _ => return Err(format!("Record field {i} is not a (name type) pair")),
                };
                let name = db
                    .ast
                    .as_name(pair[0])
                    .ok_or_else(|| format!("Record field {i} has no name"))?
                    .to_string();
                let t = typeval_of(db, pair[1])
                    .ok_or_else(|| format!("Record field `{name}` is not a type"))?;
                fields.insert(crate::resolved::Symbol::plain(name), t);
            }
            let rec = crate::ty::Ty::Record(std::sync::Arc::new(fields));
            trace!(target: "rcdzc::eval", ty = %rec.render_name(), "ctor (Record): built record type-value");
            Ok(encode_typeval(db, &rec))
        }
        // The compound-VALUE constructors reached via the shadowable `tuple`/`record` alias names.
        // Reduce the application to the STRING-HEADED primitive form — `(tuple a b)` → `("tuple" a b)`,
        // `(record (x 1) …)` → `("record" (x 1) …)` — so it resolves to `Resolved::Tuple`/
        // `Resolved::Record` and every downstream machinery (the compile-time-visible fold in
        // `reduce_to_tuple_elems`/`reduce_to_record_id`, member projection, the host-escape walk,
        // `type_of`, `lower`) treats it IDENTICALLY to a value written with the string primitive.
        //
        // ⚠ `push_list` RE-PARENTS the arg subtrees under the new head, which would orphan a field/
        // element value like the runtime param `a` in `(record (x a) …)` from its lexical scope (a
        // spurious unbound name — the re-parenting hazard `apply_lambda` documents). So PIN each arg's
        // resolution first (`resolve_subtree` memoizes every node against its CURRENT scope), exactly as
        // β-reduction pins a call argument before splicing it; the later re-parent then cannot change
        // how any node inside an arg resolves.
        Prim::TupleNew | Prim::RecordNew => {
            // Build once per construction site. The built node is `(head-str arg…)` over the EXACT SAME
            // arg occurrences, so it is a pure function of the SITE being reduced — the same source
            // application demanded repeatedly (a `let`-bound tuple projected field-by-field: `(+ (. t 0)
            // (+ (. t 1) …))` re-reduces `t` at EVERY projection) must return the ONE built node, not
            // rebuild the whole N-element tuple each time. Without this the fold was O(N²): N projections
            // × an O(N) `push_list` rebuild (the shared-tuple projection chain).
            //
            // The key is O(1): the prim plus `origin` — the StructId of the application being reduced,
            // which `push_list` assigns FRESH per occurrence. NOT the argument ids: β-reduction SHARES a
            // constant-atom occurrence (`copy_structural` returns the original id for a literal), so in a
            // NESTED `(tuple n (tuple n n))` the outer and inner tuples share the same first-arg `n`
            // occurrence — keying on an argument would collide the inner build onto the outer and recurse
            // without end (a stack overflow). Keying on all N arg ids instead would re-introduce the
            // O(N²) as an O(N) key HASH per projection. `origin` is unique and O(1); it mirrors the
            // `Int`/`UInt` build-once cache (which keys on the width value, likewise unique per module).
            let key = crate::db::BuildKey {
                prim,
                args: vec![origin.0 as i64],
            };
            if let Some(cached) = db.cached_build(&key) {
                trace!(target: "rcdzc::eval", node = cached.0, "ctor (tuple/record value): build-cache hit");
                return Ok(cached);
            }
            for &a in args {
                crate::resolve::resolve_subtree(db, a);
            }
            let head = db.push_str(if matches!(prim, Prim::TupleNew) {
                "tuple"
            } else {
                "record"
            });
            let mut children = vec![head];
            children.extend_from_slice(args);
            let built = db.push_list(children);
            db.cache_build(key, built);
            Ok(built)
        }
        _ => Err("not a type constructor".to_string()),
    }
}

/// Reduce a GENERIC SUM type-constructor application `(Option Int64)` to its built sum type-value node
/// `(typeval (Sum NAME <decl> arg…))`. `head` is the applied sum record (`Option`), carrying the owning
/// declaration on its `(meta sum-decl)` channel; `args` are the type arguments, each reduced to a `Ty`.
/// The built `Ty::Sum { decl, name, args }` is encoded back to an arena `(typeval …)` node so it flows
/// through the ordinary type path. `None` if the head has no `(meta sum-decl)` (not a generic sum), an
/// arg is not a type, or the declaration is unknown. (The analogue of `reduce_ctor` for `TupleCtor`, but
/// keyed on the head's metadata rather than the prim alone — one prim serves every generic sum.)
pub fn reduce_sum_ctor(db: &mut Db, head: StructId, args: &[StructId]) -> Option<StructId> {
    let decl_field = project_meta(db, head, "sum-decl")?;
    let decl = match resolved_of(db, decl_field) {
        Resolved::Int(v) => crate::ast::StructId(v.to_i64().and_then(|n| u32::try_from(n).ok())?),
        _ => return None,
    };
    let name = db.type_decl_by_occ(decl)?.name.clone();
    let mut arg_tys = Vec::with_capacity(args.len());
    for &a in args {
        arg_tys.push(typeval_of(db, a)?);
    }
    let sum = crate::ty::Ty::Sum {
        decl,
        name,
        args: arg_tys,
    };
    trace!(target: "rcdzc::eval", ty = %sum.render_name(), "ctor (Sum): built generic sum type-value");
    Some(encode_typeval(db, &sum))
}

/// The type-VALUE the node at `id` reduces to, if any — a ground type (`Bool`/`Unit` via `(meta t)`),
/// a `(typeval …)` built node, or a type-constructor application reduced. This is how a type
/// expression yields a `Ty`. Returns `None` if the node is not a type.
pub fn typeval_of(db: &mut Db, id: StructId) -> Option<crate::ty::Ty> {
    match resolved_of(db, id) {
        // A ground-type record: read its `(meta t)`, which holds the type-value.
        Resolved::Record { .. } | Resolved::Ref { .. } => {
            // A VARIANT-CONSTRUCTOR record is NOT a type-value even though it carries a `(meta t)` — its
            // `(meta t)` is the constructor's own type (`(-> P Sum)` or, for a nullary variant, the sum
            // `Sum`), and a NULLARY variant used bare (`None`) is a VALUE of the sum, not the sum TYPE.
            // Its `(meta variant)` channel is the mark that distinguishes it: a record carrying it is a
            // sum value/constructor, so it does not reduce to a type here (a bare `None` types as the
            // sum via `type_of`, not as `Ty::Type`; `(: x Option)` still reduces `Option` — the SUM
            // record — because that record has no `(meta variant)`).
            if variant_disc_of(db, id).is_some() {
                return None;
            }
            // Either a `(meta t)` ground-type record, OR an already-built `(typeval …)` record.
            if let Some(t_field) = project_meta(db, id, "t") {
                return typeval_of(db, t_field);
            }
            None
        }
        // A native ground type prim (`Bool`/`Unit`) held in a `(meta t)` field.
        Resolved::Prim(p) => p.ground_type(),
        // A built `(typeval …)` node carries the type directly.
        Resolved::TypeVal(t) => Some(t),
        // A type-constructor application — reduce it, then read the built value's type.
        Resolved::Apply { head, args } => {
            let prim = meta_apply_of(db, head)?;
            if prim.is_arith() {
                return None;
            }
            // A GENERIC SUM application `(Option Int64)` — build `Ty::Sum{decl,args}` from the head's
            // `(meta sum-decl)` + the applied type args (keyed on the head's metadata, not the prim).
            if prim == Prim::SumCtor {
                let built = reduce_sum_ctor(db, head, &args)?;
                return typeval_of(db, built);
            }
            let built = reduce_ctor(db, prim, id, &args).ok()?;
            typeval_of(db, built)
        }
        _ => None,
    }
}

/// How a `(Int W)`/`(UInt W)` width argument reads. A concrete non-negative natural that fits `u32` is
/// `Fixed`. A concrete but NON-NATURAL width — a negative or over-`u32` integer, or a bool/float/type-
/// value in width position — is `Malformed`. A width that is not a concrete value at all — an unbound
/// name, a runtime parameter, or a non-constant computation — is `NotConst`. Both `Malformed` and
/// `NotConst` are INVALID widths: an integer type MUST be indexed by a compile-time natural
/// (numeric-model.md §An Integer Type Is Indexed By A Compile-Time Width), so the caller reduces either
/// to the sentinel width 0 → CDZ0302, rejecting the bad width at the annotation rather than dropping it.
/// (The variants stay distinct so a future caller could message them differently; `reduce_ctor` treats
/// both as reject.)
enum WidthRead {
    Fixed(u32),
    Malformed,
    NotConst,
}

/// Classify the width argument of `(Int W)`/`(UInt W)`. The old reader narrowed with `u32::try_from`
/// and returned `None` for anything that failed — which silently DROPPED a negative/non-natural width
/// (the annotation was ignored and the literal kept its default Int64) instead of rejecting it. This
/// distinguishes a MALFORMED concrete width (reject) from a NON-CONSTANT one (decline), so the caller
/// can reject the former with CDZ0302 exactly as `(UInt 0)` already is.
fn read_width(db: &mut Db, id: StructId) -> WidthRead {
    match resolved_of(db, id) {
        // A concrete integer literal: a non-negative natural that fits `u32` is the width. A negative
        // or over-`u32` literal is a malformed (non-natural) width.
        Resolved::Int(v) => match v.to_i64() {
            Some(n) if (0..=i64::from(u32::MAX)).contains(&n) => WidthRead::Fixed(n as u32),
            _ => WidthRead::Malformed,
        },
        Resolved::Ref { value } => read_width(db, value),
        // A concrete NON-integer value in width position (a Bool, a type-value, a type/module record)
        // is a non-natural width — reject, don't drop.
        Resolved::Bool(_) | Resolved::TypeVal(_) | Resolved::Record { .. } => WidthRead::Malformed,
        // A float LITERAL reaches here as a decline-poison (floats are unsupported), but a concrete
        // float in width position is still a non-natural width → malformed. Anything else (an unbound
        // name, a width variable, a non-constant computation) is not a concrete width → decline.
        _ => match db.ast.get(id) {
            crate::ast::Struct::Atom(lid)
                if matches!(db.ast.leaf(*lid), crate::ast::Leaf::Float(_)) =>
            {
                WidthRead::Malformed
            }
            _ => WidthRead::NotConst,
        },
    }
}

/// Whether the type expression at `id` is an integer type `(Int W)`/`(UInt W)` whose WIDTH is RUNTIME
/// DATA — a function PARAMETER (or a reference chain reaching one). Such a width puts a runtime value in
/// a type-determining position, which the type system forbids (a width must be a compile-time natural,
/// `numeric-model.md §An Integer Type Is Indexed By A Compile-Time Width`). Checked on the UN-INLINED
/// annotation body so `(def (mk n) (: 5 (UInt n)))` rejects even though a constant call site would fold
/// `n` — the width is non-dependent by declaration, not by how it is called. Only a genuine
/// runtime-value width is flagged; a constant width, an unbound name, or a not-yet-built thing is not
/// (those are handled elsewhere — a malformed width by `read_width`, an unbound name by scope).
pub fn is_runtime_width_type(db: &mut Db, id: StructId) -> bool {
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return false;
    };
    match meta_apply_of(db, head) {
        Some(Prim::IntCtor | Prim::UIntCtor) if args.len() == 1 => width_is_runtime(db, args[0]),
        _ => false,
    }
}

/// Whether a width expression resolves to a RUNTIME value — a function parameter (through any `Ref`
/// chain). A constant literal, an unbound name, or anything else is not runtime data.
fn width_is_runtime(db: &mut Db, id: StructId) -> bool {
    match resolved_of(db, id) {
        Resolved::Param { .. } => true,
        Resolved::Ref { value } => width_is_runtime(db, value),
        _ => false,
    }
}

/// Encode a `Ty` as a `(typeval …)` arena node the resolver decodes back to `Resolved::TypeVal`.
/// Compact and total over the type shapes the evaluator builds (integers, bool, unit, function).
pub fn encode_typeval(db: &mut Db, ty: &crate::ty::Ty) -> StructId {
    let head = db.push_name("typeval");
    let payload = encode_ty(db, ty);
    db.push_list(vec![head, payload])
}

/// Encode a `Ty` into an arena subtree (the payload of a `(typeval …)` node). The dual `decode_ty`
/// lives in `resolve` where a `(typeval …)` node is read.
fn encode_ty(db: &mut Db, ty: &crate::ty::Ty) -> StructId {
    use crate::ty::Ty;
    match ty {
        Ty::Bool => db.push_name("Bool"),
        Ty::Unit => db.push_name("Unit"),
        Ty::Int(it) => {
            let ctor = db.push_name(if it.ground_signed() { "Int" } else { "UInt" });
            let w = match it.width {
                Width::Fixed(w) => w as i64,
                // A deferred/var width in a built type grounds to the default for encoding.
                _ => crate::ty::DEFAULT_INT_WIDTH as i64,
            };
            let width = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(w),
                radix: crate::ast::Radix::Dec,
            });
            db.push_list(vec![ctor, width])
        }
        Ty::Fn(p, r) => {
            let arr = db.push_name("->");
            let pe = encode_ty(db, p);
            let re = encode_ty(db, r);
            db.push_list(vec![arr, pe, re])
        }
        // A tuple: `(Tuple <elem>…)` — the head then each element type encoded in order.
        Ty::Tuple(elems) => {
            let mut items = vec![db.push_name("Tuple")];
            for e in elems.iter() {
                items.push(encode_ty(db, e));
            }
            db.push_list(items)
        }
        // A record type-value: `(Record (name T)…)` in canonical (sorted) field order — the capitalized
        // `Record` head (the TYPE; the VALUE head is lowercase `record`), each field a `(name T)` pair.
        // Round-trips with `decode_ty`'s `"Record"` arm; the `BTreeMap` iteration IS the canonical order.
        Ty::Record(fields) => {
            let mut items = vec![db.push_name("Record")];
            for (name, t) in fields.iter() {
                let fname = db.push_name(&name.name);
                let fty = encode_ty(db, t);
                items.push(db.push_list(vec![fname, fty]));
            }
            db.push_list(items)
        }
        // A sum type-value: `(Sum <name> <decl> arg…)` — the nominal name (for rendering), the
        // declaration occurrence (the identity, an integer literal so it round-trips), and the type ARGS
        // (empty for a monomorphic sum). Round-trips with `resolve::decode_ty`'s `"Sum"` arm. This is the
        // shape `sums::synthesize` also builds for a sum record's `(meta t)`, so the two must agree.
        Ty::Sum { decl, name, args } => {
            let head = db.push_name("Sum");
            let nm = db.push_name(name);
            let d = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(decl.0 as i64),
                radix: crate::ast::Radix::Dec,
            });
            // `(Sum NAME <decl> arg…)` — the type ARGS follow, so a generic instantiation round-trips
            // (a monomorphic sum encodes no args, byte-identical to before).
            let mut items = vec![head, nm, d];
            for a in args {
                items.push(encode_ty(db, a));
            }
            db.push_list(items)
        }
        // Var/Any shouldn't reach a built type-value in Milestone A; encode Unit as a safe stub.
        _ => db.push_name("Unit"),
    }
}

/// Build the integer MODULE record for `(signed, width)` by appending arena nodes, returning its
/// occurrence. The module is a record whose `(meta t)` holds the concrete integer type-value and whose
/// fields carry the width's specialization: `max`/`min` as the width's bounds. (Operations like `+`
/// join it width-specialized in Milestone B; Milestone A pins the type + bounds so `(. (Int 64) max)`
/// folds through the ordinary projection.)
pub fn build_int_module(db: &mut Db, signed: bool, width: u32) -> StructId {
    let it = IntTy::fixed(signed, width);
    let ty = crate::ty::Ty::Int(it);

    let mut fields: Vec<StructId> = Vec::new();

    // `(meta t)` — the concrete integer type-value.
    let ty_node = encode_typeval(db, &ty);
    fields.push(meta_field(db, "t", ty_node));

    // `max`/`min` — the width's bounds, computed from the WIDTH PARAMETER (not a per-type table), at
    // arbitrary precision so `UInt64.max = 2^64 - 1` (which exceeds `i64`) is exact. Any width the
    // constructor admits (1..=64, including odd widths like `(UInt 7)`/`(UInt 24)`) yields that width's
    // bounds. A width the fold can't represent leaves the bound unrealized (declines when projected).
    match int_bounds(signed, width) {
        Some((max, min)) => {
            fields.push(named_int_field(db, "max", max, signed, width));
            fields.push(named_int_field(db, "min", min, signed, width));
        }
        None => {
            fields.push(unrealized_field(db, "max"));
            fields.push(unrealized_field(db, "min"));
        }
    }

    // `wrap` — the truncating conversion INTO this width, built identically to the prelude's named-width
    // modules (`prelude::wrap_field`) so `(Int N).wrap` and `Int8.wrap` denote the same operation. Its
    // `(meta t)` is `(fn (a) (-> (Int a) TARGET))` (generic source, this width as target); `(meta apply)`
    // the shared `wrap` intrinsic.
    fields.push(wrap_field(db, signed, width));

    // The record PRIMITIVE head is the STRING `"record"` (the ordinary NAME `record` is a shadowable
    // prelude alias). A compiler-synthesized module record builds directly with the string head so it
    // resolves structurally to `Resolved::Record`, independent of any user binding of the name `record`.
    let head = db.push_str("record");
    let mut children = vec![head];
    children.append(&mut fields);
    db.push_list(children)
}

/// The module's `wrap` field, appended on `&mut Db` — the constructor-side twin of `prelude::wrap_field`
/// (the two MUST build the same shape so `(Int N).wrap` and the named-width `Int8.wrap` are one
/// operation). `(wrap (record ((meta t) (fn (a) (-> (Int a) TARGET))) ((meta apply) (intrinsic wrap))))`.
fn wrap_field(db: &mut Db, signed: bool, width: u32) -> StructId {
    // `(fn (a) (-> (Int a) TARGET))`.
    let int_a = {
        let int = db.push_name("Int");
        let a = db.push_name("a");
        db.push_list(vec![int, a])
    };
    let target = {
        let ctor = db.push_name(if signed { "Int" } else { "UInt" });
        let w = db.push_atom(Leaf::Int {
            value: IntValue::from_i64(width as i64),
            radix: crate::ast::Radix::Dec,
        });
        db.push_list(vec![ctor, w])
    };
    let arr = db.push_name("->");
    let body = db.push_list(vec![arr, int_a, target]);
    let fn_head = db.push_name("fn");
    let a_param = db.push_name("a");
    let params = db.push_list(vec![a_param]);
    let lambda = db.push_list(vec![fn_head, params, body]);
    // `("record" ((meta t) lambda) ((meta apply) (intrinsic wrap)))` — the record primitive is the
    // string head `"record"`.
    let rec_head = db.push_str("record");
    let t_field = meta_field(db, "t", lambda);
    let prim = intrinsic_node(db, "wrap");
    let apply_field = meta_field(db, "apply", prim);
    let record = db.push_list(vec![rec_head, t_field, apply_field]);
    let k = db.push_name("wrap");
    db.push_list(vec![k, record])
}

/// The (max, min) bounds of a `(signed, width)` integer as arbitrary-precision [`IntValue`]s, computed
/// FROM THE WIDTH — a signed N-bit is `-(2^(N-1)) ..= 2^(N-1)-1`, an unsigned N-bit is `0 ..= 2^N-1`.
/// `None` only for a width the fold does not model (0, or > 64). Odd widths (`7`, `24`, `48`) are
/// ordinary — the bounds are a function of `width`, so nothing is hard-coded per named type. Shared
/// with the prelude's named-width modules so a named width and `(Int N)` share one bounds computation.
pub fn int_bounds(signed: bool, width: u32) -> Option<(IntValue, IntValue)> {
    if width == 0 || width > 64 {
        return None;
    }
    if signed {
        // max = 2^(width-1) - 1, min = -(2^(width-1)).
        let half = 1u128 << (width - 1);
        Some((IntValue::from_u128(half - 1), IntValue::from_neg_u128(half)))
    } else {
        // max = 2^width - 1, min = 0. (width==64 → 2^64-1, exact via u128.)
        let max = (1u128 << width) - 1;
        Some((IntValue::from_u128(max), IntValue::zero()))
    }
}

/// A `((meta KEY) VALUE)` meta field (appended). Mirrors the prelude's `meta_field` but on `&mut Db`.
fn meta_field(db: &mut Db, key: &str, value: StructId) -> StructId {
    let meta_head = db.push_name("meta");
    let key_name = db.push_name(key);
    let meta_key = db.push_list(vec![meta_head, key_name]);
    db.push_list(vec![meta_key, value])
}

/// An `(intrinsic NAME)` node (appended) — the arena form a native primitive value takes, mirroring
/// `prelude::intrinsic_node` on `&mut Db` (so a constructor-built module's `wrap` field carries the
/// same intrinsic the prelude's named-width modules do).
fn intrinsic_node(db: &mut Db, name: &str) -> StructId {
    let head = db.push_name("intrinsic");
    let who = db.push_name(name);
    db.push_list(vec![head, who])
}

/// A `(name (: INT (Int/UInt WIDTH)))` record field — an arbitrary-precision integer constant ANNOTATED
/// with the module's own width, so projecting the field (`UInt8.max`) yields a value that types at that
/// width (not the signed-64 a bare literal would default to). The annotation is the ordinary `(: e T)`
/// node; the literal grounds to `(signed, width)` through it. This is what carries `UInt8.max`'s type as
/// `UInt8` and `UInt64.max`'s value (2^64-1, > i64) at its true width.
fn named_int_field(db: &mut Db, name: &str, value: IntValue, signed: bool, width: u32) -> StructId {
    let k = db.push_name(name);
    let lit = db.push_atom(Leaf::Int {
        value,
        radix: crate::ast::Radix::Dec,
    });
    // The type expression `(Int width)` / `(UInt width)`.
    let ctor = db.push_name(if signed { "Int" } else { "UInt" });
    let w = db.push_atom(Leaf::Int {
        value: IntValue::from_i64(width as i64),
        radix: crate::ast::Radix::Dec,
    });
    let ty_expr = db.push_list(vec![ctor, w]);
    // `(: lit ty_expr)`.
    let colon = db.push_name(":");
    let annot = db.push_list(vec![colon, lit, ty_expr]);
    db.push_list(vec![k, annot])
}

/// A `(name (unrealized name))` field — the field exists but declines when projected.
fn unrealized_field(db: &mut Db, name: &str) -> StructId {
    let k = db.push_name(name);
    let head = db.push_name("unrealized");
    let who = db.push_name(name);
    let v = db.push_list(vec![head, who]);
    db.push_list(vec![k, v])
}

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
use crate::resolve::resolved_of;
use crate::resolved::{Prim, Resolved, Symbol};
use crate::ty::{IntTy, Scheme, Ty, Width};
use crate::unify::Fresh;
use std::collections::HashMap;

/// Read a value's `(meta t)` — its TYPE — as a [`Scheme`], reducing the type-lambda it holds. An
/// operator's `Meta.t` is a compile-time lambda `(fn (a) (-> (Int a) (-> (Int a) (Int a))))`; this
/// binds each lambda parameter to a FRESH variable, reduces the body's type expression symbolically
/// (so `(Int a)` becomes an integer of that fresh WIDTH variable, shared across every `a`), and closes
/// the mentioned variables into the scheme. This is how `infer` reads an operator's type generically
/// as DATA — one rule for every operator, no per-operator Rust. `None` if the value has no `(meta t)`
/// or its type expression is malformed.
pub fn scheme_of(db: &mut Db, id: StructId, fresh: &mut Fresh) -> Option<Scheme> {
    let t = project_meta(db, id, "t")?;
    // The `(meta t)` value is either a bare type (a scheme with no quantified vars) or a type-lambda
    // whose parameters are the quantified variables.
    match resolved_of(db, t) {
        Resolved::Lambda { params, body } => {
            // Bind each parameter occurrence to a fresh variable; the same parameter used twice in the
            // body shares its variable (so both operands of `+` unify to one width).
            let mut env: HashMap<StructId, TyOrWidth> = HashMap::new();
            let mut ty_vars = Vec::new();
            let mut width_vars = Vec::new();
            for &p in &params {
                let n = fresh.var();
                // A parameter's kind (type var vs width var) is decided by HOW it is used; we record a
                // single fresh number and let the use site pick. Track it as both — the body reduction
                // uses it as a width inside `(Int _)` and as a type elsewhere.
                env.insert(p, TyOrWidth { num: n });
                ty_vars.push(n);
                width_vars.push(n);
            }
            let ty = type_in_env(db, body, &env)?;
            // Only the vars actually mentioned matter; keep both lists (unused ones are harmless).
            Some(Scheme { ty_vars, width_vars, ty })
        }
        // A non-lambda `(meta t)` is a monomorphic type.
        _ => typeval_of(db, t).map(Scheme::mono),
    }
}

/// A lambda parameter's fresh variable number — used as a type variable OR a width variable depending
/// on the position it appears in (`(Int a)` → width; a bare `a` → type). One number serves both.
#[derive(Clone, Copy)]
struct TyOrWidth {
    num: u32,
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
                    let signed = matches!(prim, Prim::IntCtor);
                    let width = width_in_env(db, args[0], env)?;
                    Some(Ty::Int(IntTy { signed, width }))
                }
                Prim::FnCtor if args.len() == 2 => {
                    let p = type_in_env(db, args[0], env)?;
                    let r = type_in_env(db, args[1], env)?;
                    Some(Ty::Fn(Box::new(p), Box::new(r)))
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
        Resolved::Int(v) => v.to_i64().and_then(|n| u32::try_from(n).ok()).map(Width::Fixed),
        Resolved::Ref { value } => width_in_env(db, value, env),
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
pub fn beta_reduce(
    db: &mut Db,
    body: StructId,
    arg_of: &HashMap<StructId, StructId>,
) -> StructId {
    // A reference to a substituted parameter → its argument. A parameter reference resolves to
    // `Ref { value: <param binder occ> }`; if that binder is one we're substituting, use the arg.
    if let Resolved::Ref { value } = resolved_of(db, body) {
        if let Some(&arg) = arg_of.get(&value) {
            return arg;
        }
    }
    // The parameter OCCURRENCE itself (in the binder position, not a ref) is not substituted — only
    // references are; but a bare param used as a value resolves to `Param{binder}` — substitute it.
    if let Resolved::Param { binder } = resolved_of(db, body) {
        if let Some(&arg) = arg_of.get(&binder) {
            return arg;
        }
    }
    // Otherwise structurally copy: an atom is shared (it references only a leaf); a list is copied
    // with each child reduced.
    match db.ast.get(body).clone() {
        crate::ast::Struct::Atom(_) => body, // atoms are immutable and self-contained — reuse.
        crate::ast::Struct::List(children) => {
            let reduced: Vec<StructId> =
                children.iter().map(|&c| beta_reduce(db, c, arg_of)).collect();
            db.push_list(reduced)
        }
    }
}

/// If `head` reduces to a lambda, apply it to `args` by β-reduction, returning the reduced body's
/// occurrence. Follows a `Ref` to the lambda (so a `let`-bound function applies too). Parameters match
/// positionally: exact arity β-reduces the whole body; MORE args than params reduces then applies the
/// remainder to the result (curried); FEWER args is a partial application, not yet supported (`Err`).
/// A non-lambda head is `Ok(None)` — the caller tries the `(meta apply)` primitive path instead.
pub fn apply_lambda(db: &mut Db, head: StructId, args: &[StructId]) -> Result<Option<StructId>, String> {
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
        tracing::trace!(target: "rcdzc::eval", body = body.0, "decline: recursive function (needs runtime specialization)");
        return Err("a recursive function needs runtime specialization (not yet built)".to_string());
    }
    tracing::trace!(target: "rcdzc::eval", body = body.0, params = params.len(), args = args.len(), "β-reduce lambda application");
    if args.len() < params.len() {
        return Err("partial application of a function is not yet supported".to_string());
    }
    let mut arg_of: HashMap<StructId, StructId> = HashMap::new();
    for (p, a) in params.iter().zip(args.iter()) {
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
fn param_name_occ(db: &Db, param: StructId) -> StructId {
    if db.ast.as_name(param).is_some() {
        return param;
    }
    if let Some(tail) = db.ast.as_form(param, ":") {
        if let Some(&name_occ) = tail.first() {
            return name_occ;
        }
    }
    param
}

/// The body occurrence of the lambda `head` reduces to, if any — the stable per-function identity the
/// recursion guard keys on (so a recursive call, which re-enters the same body, is detected).
pub fn lambda_body(db: &mut Db, head: StructId) -> Option<StructId> {
    lambda_of(db, head).map(|(_, body)| body)
}

/// Is the function whose body is `body` (transitively) RECURSIVE — does the static call graph contain
/// a cycle back to `body`? A recursive function cannot be β-reduced to a normal form at compile time
/// (it would inline without end, and one that branches — several self-calls per body, like a CBOR tree
/// reader — explodes EXPONENTIALLY in appended nodes long before any depth bound), so the evaluator
/// reads this BEFORE reducing a call and DECLINES a recursive one (it needs runtime specialization,
/// not yet built). This is the SOUND, PURELY STRUCTURAL recursion signal — a static property of the
/// resolved call graph, computed WITHOUT reducing (no `apply_lambda`, no arena growth), unlike the
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
    // path returning to it is detected on pop.)
    collect_callees(db, body, &mut worklist);
    let mut result = false;
    while let Some(node) = worklist.pop() {
        if node == body {
            result = true;
            break;
        }
        if visited.insert(node) {
            collect_callees(db, node, &mut worklist);
        }
    }

    // Return the buffers for the next body's walk to reuse.
    db.rec_visited = visited;
    db.rec_worklist = worklist;
    db.recursive.insert(body, result);
    result
}

/// Collect the callee bodies of the function whose body expression is (rooted at) `node` — a LOCAL,
/// NON-REDUCING syntactic walk of one function's body. For each application, the head's statically-known
/// callee body (via [`callee_body`], following refs to a lambda WITHOUT reducing) is one edge; the walk
/// descends into every sub-expression that runs when this body runs. It STOPS at a nested `fn`
/// boundary — a lambda defined inside this body is a SEPARATE call-graph node (its own calls are edges
/// from IT, explored only if IT is called), so descending into it would conflate two nodes. This keeps
/// the graph precise: over-reporting would decline a terminating fold, under-reporting would let a
/// recursive one reach the (slow, exponential) depth backstop.
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
        Resolved::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_callees(db, value, out);
            }
            collect_callees(db, body, out);
        }
        Resolved::Record { fields } => {
            for (_, value) in fields {
                collect_callees(db, value, out);
            }
        }
        Resolved::Member { operand, .. } => collect_callees(db, operand, out),
        // An annotation is transparent — a call inside `(: (f x) T)` is a real edge of this body.
        Resolved::Annot { expr, .. } => collect_callees(db, expr, out),
        // A nested `fn` is a separate node — do NOT descend (its calls are its own edges). A bare ref,
        // a leaf, a prim, a param, a type value, a poison have no callees of THIS body.
        Resolved::Lambda { .. }
        | Resolved::Ref { .. }
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
        Resolved::Ref { value } => callee_body(db, value),
        _ => None,
    }
}

/// If the value at `id` reduces to a lambda, its parameters and body. Follows a `Ref` (a `let`-bound
/// function) AND reduces an `Apply` head (a function RETURNED by another application — `(adder 10)`
/// reduces to the inner `(fn (x) …)`, so `((adder 10) 5)` applies it). This is what makes curried and
/// returned functions work: the head of an application is itself evaluated to a lambda first.
fn lambda_of(db: &mut Db, id: StructId) -> Option<(Vec<StructId>, StructId)> {
    match resolved_of(db, id) {
        Resolved::Lambda { params, body } => Some((params, body)),
        Resolved::Ref { value } => lambda_of(db, value),
        Resolved::Apply { head, args } => {
            // Reduce the inner application to a value, then see if THAT is a lambda.
            let reduced = apply_lambda(db, head, &args).ok().flatten()?;
            lambda_of(db, reduced)
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

/// Project a meta-channel field named `key` from the record value at `id`, following a `Ref` to the
/// record. Returns the field's value occurrence, or `None` if the value is not a record or has no such
/// field. The one generic projection, restricted to the `meta` namespace.
pub fn project_meta(db: &mut Db, id: StructId, key: &str) -> Option<StructId> {
    let sym = Symbol { namespace: Some("meta".to_string()), name: key.to_string() };
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
            let prim = meta_apply_of(db, head)?;
            if prim.is_arith() {
                return None;
            }
            let built = reduce_ctor(db, prim, &args).ok()?;
            reduce_to_record_id(db, built)
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
                Some(&v) => Member::Field(v),
                None => Member::NoField,
            },
            _ => Member::NotRecord,
        },
        None => Member::NotRecord,
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
pub fn reduce_ctor(db: &mut Db, prim: Prim, args: &[StructId]) -> Result<StructId, String> {
    match prim {
        Prim::IntCtor | Prim::UIntCtor => {
            let signed = matches!(prim, Prim::IntCtor);
            if args.len() != 1 {
                return Err(format!("{} takes one width argument", if signed { "Int" } else { "UInt" }));
            }
            let width = const_width(db, args[0])
                .ok_or_else(|| "integer width must be a constant".to_string())?;
            // Build once per (ctor, width): the first demand appends the module, every later demand —
            // repeated on this occurrence, or another `(Int 64)` elsewhere — returns the same node, so
            // the reduction is idempotent and the arena holds one module per width.
            let key = crate::db::BuildKey { prim, args: vec![width as i64] };
            if let Some(cached) = db.cached_build(&key) {
                return Ok(cached);
            }
            let built = build_int_module(db, signed, width);
            db.cache_build(key, built);
            Ok(built)
        }
        Prim::FnCtor => {
            if args.len() != 2 {
                return Err("-> takes two type arguments".to_string());
            }
            let p = typeval_of(db, args[0]).ok_or_else(|| "-> parameter is not a type".to_string())?;
            let r = typeval_of(db, args[1]).ok_or_else(|| "-> result is not a type".to_string())?;
            let fn_ty = crate::ty::Ty::Fn(Box::new(p), Box::new(r));
            Ok(encode_typeval(db, &fn_ty))
        }
        _ => Err("not a type constructor".to_string()),
    }
}

/// The type-VALUE the node at `id` reduces to, if any — a ground type (`Bool`/`Unit` via `(meta t)`),
/// a `(typeval …)` built node, or a type-constructor application reduced. This is how a type
/// expression yields a `Ty`. Returns `None` if the node is not a type.
pub fn typeval_of(db: &mut Db, id: StructId) -> Option<crate::ty::Ty> {
    match resolved_of(db, id) {
        // A ground-type record: read its `(meta t)`, which holds the type-value.
        Resolved::Record { .. } | Resolved::Ref { .. } => {
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
            let built = reduce_ctor(db, prim, &args).ok()?;
            typeval_of(db, built)
        }
        _ => None,
    }
}

/// Read a constant width from the node at `id` — an integer literal's value narrowed to a `u32`. Used
/// for the width argument of `(Int W)`. `None` if not a constant that fits.
fn const_width(db: &mut Db, id: StructId) -> Option<u32> {
    match resolved_of(db, id) {
        Resolved::Int(v) => v.to_i64().and_then(|n| u32::try_from(n).ok()),
        Resolved::Ref { value } => const_width(db, value),
        _ => None,
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
            let ctor = db.push_name(if it.signed { "Int" } else { "UInt" });
            let w = match it.width {
                Width::Fixed(w) => w as i64,
                // A deferred/var width in a built type grounds to the default for encoding.
                _ => crate::ty::DEFAULT_INT_WIDTH as i64,
            };
            let width = db.push_atom(Leaf::Int { value: IntValue::from_i64(w), radix: crate::ast::Radix::Dec });
            db.push_list(vec![ctor, width])
        }
        Ty::Fn(p, r) => {
            let arr = db.push_name("->");
            let pe = encode_ty(db, p);
            let re = encode_ty(db, r);
            db.push_list(vec![arr, pe, re])
        }
        // Var/Any/Record shouldn't reach a built type-value in Milestone A; encode Unit as a safe stub.
        _ => db.push_name("Unit"),
    }
}

/// Build the integer MODULE record for `(signed, width)` by appending arena nodes, returning its
/// occurrence. The module is a record whose `(meta t)` holds the concrete integer type-value and whose
/// fields carry the width's specialization: `max`/`min` as the width's bounds. (Operations like `+`
/// join it width-specialized in Milestone B; Milestone A pins the type + bounds so `(. (Int 64) max)`
/// folds through the ordinary projection.)
pub fn build_int_module(db: &mut Db, signed: bool, width: u32) -> StructId {
    let it = IntTy { signed, width: Width::Fixed(width) };
    let ty = crate::ty::Ty::Int(it);

    let mut fields: Vec<StructId> = Vec::new();

    // `(meta t)` — the concrete integer type-value.
    let ty_node = encode_typeval(db, &ty);
    fields.push(meta_field(db, "t", ty_node));

    // `max`/`min` — the width's bounds (for the widths we can represent as an i64 constant). A width
    // beyond i64 leaves the bound unrealized (declines when projected) rather than emit a wrong value.
    if let Some((max, min)) = int_bounds(signed, width) {
        fields.push(named_int_field(db, "max", max));
        fields.push(named_int_field(db, "min", min));
    } else {
        fields.push(unrealized_field(db, "max"));
        fields.push(unrealized_field(db, "min"));
    }

    let head = db.push_name("record");
    let mut children = vec![head];
    children.append(&mut fields);
    db.push_list(children)
}

/// The (max, min) bounds of a `(signed, width)` integer as `i64`s, or `None` if a bound does not fit
/// `i64` (width ≥ 64 unsigned, or width > 64). Milestone A represents bounds as `i64` constants.
fn int_bounds(signed: bool, width: u32) -> Option<(i64, i64)> {
    if width == 0 || width > 64 {
        return None;
    }
    if signed {
        if width == 64 {
            Some((i64::MAX, i64::MIN))
        } else {
            let max = (1i64 << (width - 1)) - 1;
            let min = -(1i64 << (width - 1));
            Some((max, min))
        }
    } else {
        // Unsigned max is 2^width - 1, which does not fit i64 for width ≥ 64.
        if width >= 64 {
            None
        } else {
            Some(((1i64 << width) - 1, 0))
        }
    }
}

/// A `((meta KEY) VALUE)` meta field (appended). Mirrors the prelude's `meta_field` but on `&mut Db`.
fn meta_field(db: &mut Db, key: &str, value: StructId) -> StructId {
    let meta_head = db.push_name("meta");
    let key_name = db.push_name(key);
    let meta_key = db.push_list(vec![meta_head, key_name]);
    db.push_list(vec![meta_key, value])
}

/// A `(name INT)` record field holding an integer constant (appended).
fn named_int_field(db: &mut Db, name: &str, value: i64) -> StructId {
    let k = db.push_name(name);
    let v = db.push_atom(Leaf::Int { value: IntValue::from_i64(value), radix: crate::ast::Radix::Dec });
    db.push_list(vec![k, v])
}

/// A `(name (unrealized name))` field — the field exists but declines when projected.
fn unrealized_field(db: &mut Db, name: &str) -> StructId {
    let k = db.push_name(name);
    let head = db.push_name("unrealized");
    let who = db.push_name(name);
    let v = db.push_list(vec![head, who]);
    db.push_list(vec![k, v])
}

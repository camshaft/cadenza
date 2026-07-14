//! Render a `Core` node as a Rust expression — the Rust backend's "selection".
//!
//! This is the structured backend's analogue of the wasm backend's instruction selection, but instead
//! of flattening the core into a stack-machine sequence it prints the core's structure as Rust's own
//! (`backends-and-targets.md` §A Backend Linearizes The Core Only If Its Target Is Linear). Each core
//! form maps to a Rust expression: `If` → an `if/else`, `Match` → a `match`, `Let` → a block with
//! `let` bindings, `Call` → a function call, `Arith` → a checked operation, `Compare` → a comparison,
//! `Convert` (`.wrap`) → an `as` cast. The machine representation is a read-off of the solved type
//! (`reference-compiler.md` §A Value's Machine Representation Follows Its Solved Type At Selection) —
//! read via `type_of`, exactly as the wasm backend reads it.
//!
//! NUMERIC MODEL — the correctness heart. A Cadenza integer TRAPS on overflow (`numeric-model.md`
//! §Overflow Is Defined). Rust's native `iN`/`uN` are Cadenza's aliased widths with the SAME
//! wrapping-vs-checked distinction, so:
//!   - `+`/`-`/`*` emit `<lhs>.checked_add(<rhs>)` (etc.) unwrapped with a trap on `None` — the direct
//!     expression of the wasm backend's carry/borrow/round-trip guard-and-range-check recipe (that
//!     recipe existed to express checked arithmetic in the flat rung; `checked_*` IS it, at any width);
//!   - `/`/`%` emit `checked_div`/`checked_rem`, which return `None` on ÷0 AND on `MIN / -1` — exactly
//!     the two cases the numeric model traps (wasm traps these natively);
//!   - `&`/`|`/`^` are total → the plain Rust operator;
//!   - `.wrap` truncates via an `as` cast, which in Rust keeps the low bits and reinterprets at the
//!     target — bit-identical to `IntValue::wrap_to` (`numeric-model.md` §wrap never traps).
//!
//! The trap is a Rust `panic!` (an aborting trap), the native analogue of a wasm `unreachable`.
//!
//! A construct this scalar slice does not render — a runtime shift (whose out-of-range-count trap is
//! not yet expressed), a compound value, a poison — DECLINES, attributed to this target
//! (`backends-and-targets.md` §A Backend Inherits The Front's Decline Boundaries).

use super::Mode;
use super::types;
use crate::ast::{IntValue, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_of;
use crate::layout::Layout;
use crate::lower::core_of;
use crate::resolved::Prim;
use crate::ty::{IntTy, Sign, Ty, Width};
use std::collections::HashMap;

/// The environment a body is rendered in: a map from a binder occurrence (a parameter, or a kept
/// `let` binding) to the Rust identifier it reads as. A `Core::Param`/`Core::LocalRef` looks its
/// binder up here. Populated with the parameters at the export, and extended with each `let` binding.
type Env = HashMap<StructId, String>;

/// The rendering context threaded through every `emit` — the emission [`Mode`] (sync vs async/gas) and,
/// when the function being emitted is compiled as a tail `loop`, the [`LoopGroup`] describing how a tail
/// call to a group member iterates in place. A struct (not a bare `Mode`) so these ride along without
/// widening every helper's argument list; the callee-name/param lookups a `Core::Call` needs read `db`
/// directly, so the boundary `Layout` is not needed here (the caller passes it only to `emit_body`).
#[derive(Clone)]
pub struct Ctx<'a> {
    pub mode: Mode,
    /// The boundary layout — read by a `Core::Call` to derive the callee's UNIQUED Rust `fn` ident
    /// ([`super::fn_ident`]), so a call to a β-copied do-local worker names ITS copy, matching the copy's
    /// declaration (the two agree because both go through `fn_ident`, which suffixes a colliding def index).
    pub layout: &'a Layout,
    /// `Some` iff the function being emitted is compiled as a `loop` (it tail-calls into its own tail-
    /// recursion group). A tail call to a group member reassigns the shared parameter locals (+ the
    /// `which` state, for a mutual group) and `continue`s instead of recursing; every other tail
    /// position `break`s its value out of the loop. `None` = an ordinary body (no loop).
    pub loop_group: Option<&'a LoopGroup>,
    /// The sum-match payload bindings in scope — one per `(scrutinee, path)` a matched arm bound the
    /// payload of. A `Core::SumPayload { scrutinee, path }` in an arm body resolves to the identifier
    /// here (the arm's Rust `match` pattern bound the payload to it), instead of re-extracting it. Empty
    /// outside a sum match; extended (a fresh `Ctx`) per arm by `emit_sum_match`.
    pub sum_binds: Vec<SumBind>,
    /// The SOLVED TYPE of a sub-value at a switch/bind path, recorded as each match arm DESCENDS — the
    /// Rust-backend twin of `lower`'s `path_types`. A `Payload` step's target type depends on WHICH variant
    /// the enclosing arm entered (`(type W (A Int64) (V (Option Int64)))`: `V`'s payload is `Option Int64`,
    /// `A`'s is `Int64`), which the FLATTENED path cannot encode. An arm at disc `d` records its payload's
    /// type (`variant_payload_ty(subject, d)`) here, keyed by the bind path; a NESTED switch then resolves
    /// its subject type by lookup (longest-prefix match + walk the remaining `Elem`s) instead of
    /// re-deriving it variant-0-first. Empty at the root (the scrutinee's own type resolves directly).
    pub sum_path_types: Vec<(Vec<crate::core::PathStep>, Ty)>,
}

/// A payload bound by a sum-match arm's Rust pattern: the scrutinee occurrence + access path the
/// `Core::SumPayload` reads, and the Rust identifier the pattern bound it to. A `SumPayload` matching
/// `(scrutinee, path)` renders as this `name`. `boxed` when the bound payload field is a `Box<…>` (a
/// RECURSIVE variant's field) — a read of it derefs (`*name` for the whole payload, `(*name).i` for a
/// tuple element), the deref twin of the construct site's `Box::new`.
#[derive(Clone)]
pub struct SumBind {
    pub scrutinee: StructId,
    pub path: Vec<crate::core::PathStep>,
    pub name: String,
    pub boxed: bool,
}

/// Describes a tail-recursion group compiled as a shared `loop`. A group of ONE member is plain self-
/// tail-recursion; a group of MANY same-signature members that tail-call each other (a mutual-recursion
/// SCC) share ONE loop dispatched by a `which` state variable. Each member's body renders with its own
/// parameter binders mapped to the SHARED positional locals `__p0, __p1, …` (members may name their
/// params differently but share the signature), and a tail call to member `k` sets `which = k` +
/// reassigns the shared locals + `continue`s (a PARALLEL move: all args into temps before any store, so
/// an arg reading an old param value is correct).
pub struct LoopGroup {
    /// The group's members (`db.defs` indices). `members[0]` is the function being emitted — it enters
    /// the loop at `which = 0` (its own body runs first). A tail `Core::Call` to `members[k]` iterates
    /// with `which = k`. A single-member group is a self-loop (no `which` needed, but harmless).
    pub members: Vec<usize>,
    /// The shared parameter identifiers `__p0…__pN` (one per signature position). Emitted `let mut` and
    /// reassigned each iteration; a member body's own param names map to these.
    pub shared_params: Vec<String>,
    /// The group's result integer type, if integer — a bare-literal tail leaf (`break 0`) is grounded to
    /// it so every `break` yields the SAME Rust type (a `loop` requires all `break` values agree). `None`
    /// for a non-integer result (Bool/unit). All members share the signature, so one result type serves.
    pub result_it: Option<IntTy>,
}

impl LoopGroup {
    fn is_mutual(&self) -> bool {
        self.members.len() > 1
    }
}

/// Render a function body as the Rust expression that is its return value. Builds the initial
/// environment from the function's parameters (each binder → its emitted name), then renders the body
/// core. `self_def` is the function's own `db.defs` index — used to detect a SELF-tail-call and, when
/// the body has one, compile the whole body as a `loop` (bounded stack in sync mode; no `Box::pin` poll-
/// chain in async mode). Shared by the export and non-export paths (both pass their `(binder, type)`
/// parameter list). The result is a single expression (the function's tail expression), indented once.
pub fn emit_body(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: usize,
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    // The tail-recursion group this function belongs to, compiled as a shared `loop` (§keeps deep tail
    // recursion bounded — vital in async, where a `Box::pin` poll-chain would otherwise be as deep as
    // the recursion). Empty = no loop (an ordinary body). One member = self-recursion; many = a mutual
    // SCC dispatched by a `which` state.
    let members = if params.is_empty() {
        Vec::new()
    } else {
        loop_group(db, self_def)
    };
    if !members.is_empty() {
        return emit_loop_body(db, params, self_def, &members, layout, mode);
    }
    // No loop: render the body directly.
    let mut env: Env = HashMap::new();
    for (i, (binder, _)) in params.iter().enumerate() {
        env.insert(*binder, super::param_name(db, *binder, i));
    }
    let ctx = Ctx {
        mode,
        layout,
        loop_group: None,
        sum_binds: Vec::new(),
        sum_path_types: Vec::new(),
    };
    let expr = emit(db, body, &env, &ctx)?;
    Ok(format!("    {expr}"))
}

/// Emit a tail-recursion group's shared `loop` for the member being defined (`self_def`, which is
/// `members[0]`). The shared positional locals `__p0…` are initialized from this member's params; for a
/// MUTUAL group a `which` state selects the member body each iteration (this member enters at `which =
/// 0`). Each member's body renders in TAIL position with ITS param binders mapped to the shared locals.
fn emit_loop_body(
    db: &mut Db,
    params: &[(StructId, Ty)],
    self_def: usize,
    members: &[usize],
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    let shared_params: Vec<String> = (0..params.len()).map(|i| format!("__p{i}")).collect();
    let result_it = match type_of(db, db.defs[self_def].body.unwrap()) {
        Ty::Int(it) => Some(it),
        _ => None,
    };
    let group = LoopGroup {
        members: members.to_vec(),
        shared_params: shared_params.clone(),
        result_it,
    };
    let ctx = Ctx {
        mode,
        layout,
        loop_group: Some(&group),
        sum_binds: Vec::new(),
        sum_path_types: Vec::new(),
    };
    // Initialize the shared locals from THIS member's params (its param name → `__pi`), then the body.
    let mut init = String::new();
    for (i, (binder, _)) in params.iter().enumerate() {
        let pname = super::param_name(db, *binder, i);
        init.push_str(&format!("let mut __p{i} = {pname}; "));
    }
    if group.is_mutual() {
        // A mutual group dispatches on `which` (this member enters at 0 = its own body). The dispatch is
        // an if-chain over the members; each member body renders in tail position with its params mapped.
        let mut dispatch = String::new();
        for (k, &m) in members.iter().enumerate() {
            let body = db.defs[m]
                .body
                .ok_or_else(|| Reject::decline("a loop member has no body"))?;
            let env = member_env(db, m, &shared_params);
            let b = emit_tail(db, body, &env, &ctx)?;
            if k == 0 {
                dispatch.push_str(&format!("if which == 0 {{ {b} }}"));
            } else if k + 1 < members.len() {
                dispatch.push_str(&format!(" else if which == {k} {{ {b} }}"));
            } else {
                // Last member: the unconditional `else` (reached by elimination).
                dispatch.push_str(&format!(" else {{ {b} }}"));
            }
        }
        Ok(format!(
            "    let mut which: u32 = 0; {init}\n    loop {{\n        {dispatch}\n    }}"
        ))
    } else {
        // A single-member self-loop: no `which`, just this member's body in tail position.
        let env = member_env(db, self_def, &shared_params);
        let body = db.defs[self_def].body.unwrap();
        let b = emit_tail(db, body, &env, &ctx)?;
        Ok(format!("    {init}\n    loop {{\n        {b}\n    }}"))
    }
}

/// The rendering environment for loop member `m`: its own parameter binders mapped to the SHARED
/// positional locals `__p0…` (members may name their params differently but share the signature by
/// position). So a reference to member `m`'s parameter `i` reads `__pi`.
fn member_env(db: &mut Db, m: usize, shared_params: &[String]) -> Env {
    let mut env: Env = HashMap::new();
    let mparams = crate::layout::def_params(db, m);
    for (i, (binder, _)) in mparams.iter().enumerate() {
        if let Some(name) = shared_params.get(i) {
            env.insert(*binder, name.clone());
        }
    }
    env
}

/// Whether the function `self_def` is compiled as a tail `loop` (it belongs to a non-empty tail-
/// recursion group). `pub(super)` so `emit_signature` reads the SAME predicate to decide whether to
/// declare params `mut` (a looped function reassigns its params). Agrees with `emit_body` by calling the
/// same `loop_group`.
pub(super) fn body_loops(db: &mut Db, self_def: usize) -> bool {
    !loop_group(db, self_def).is_empty()
}

/// The tail-recursion group `self_def` belongs to — the members compiled into ONE shared `loop`, with
/// `self_def` FIRST (it enters the loop at its own `which = 0`). Empty = no loop. Mirrors the wasm
/// backend's `mutual_loop_group`:
///  - forward tail-reachability from `self_def`, staying within SAME-SIGNATURE defs (a differently-typed
///    tail callee can't share the loop's positional locals);
///  - keep only members that tail-reach BACK to `self_def` (the genuine SCC) — a one-way tail callee is
///    not part of the cycle and stays an ordinary (boxed) call;
///  - a single member is a loop ONLY if it actually self-tail-calls; else no loop (empty).
///
/// Members are `self_def` then the rest ascending, so the `which` discriminants are stable. Discriminants
/// are LOCAL to each member's own emitted loop (control never crosses between two members' loops — each
/// is a complete copy of the group), so `self`-first differing per member is fine.
fn loop_group(db: &mut Db, self_def: usize) -> Vec<usize> {
    let Some(self_sig) = sig_types(db, self_def) else {
        return Vec::new();
    };
    // Forward tail-reachability within the same signature.
    let mut reach: Vec<usize> = vec![self_def];
    let mut i = 0;
    while i < reach.len() {
        let d = reach[i];
        i += 1;
        if let Some(body) = db.defs[d].body {
            let mut callees = Vec::new();
            tail_callees(db, body, &mut callees);
            for c in callees {
                if !reach.contains(&c) && sig_types(db, c).as_ref() == Some(&self_sig) {
                    reach.push(c);
                }
            }
        }
    }
    // Keep the SCC: members that tail-reach back to `self_def` (plus `self_def` itself).
    let mut members: Vec<usize> = reach
        .iter()
        .copied()
        .filter(|&d| d == self_def || tail_reaches(db, d, self_def, &reach))
        .collect();
    members.sort_unstable();
    members.retain(|&d| d != self_def);
    members.insert(0, self_def);
    if members.len() == 1 {
        // A lone member loops only if it self-tail-calls.
        let body = match db.defs[self_def].body {
            Some(b) => b,
            None => return Vec::new(),
        };
        if body_has_tail_call_to(db, body, &members) {
            return members;
        }
        return Vec::new();
    }
    members
}

/// The signature of `def` as its parameter + result RUST types (the string forms), or `None` if any
/// type has no native mapping. Two defs share a loop only if these agree — they reassign the SAME shared
/// positional locals, so the widths must match position-for-position (and the result type must match so
/// every member's `break` yields one type).
fn sig_types(db: &mut Db, def: usize) -> Option<Vec<String>> {
    let params = crate::layout::def_params(db, def);
    let body = db.defs[def].body?;
    let mut sig = Vec::new();
    for (_, ty) in &params {
        sig.push(types::rust_type(ty)?);
    }
    // A sentinel separates params from result so `(u8)->u16` ≠ `(u8,u16)->()` etc.
    sig.push("->".to_string());
    sig.push(types::rust_type(&type_of(db, body))?);
    Some(sig)
}

/// The defs called in TAIL position from the body at `id` (an `if` branch, a `let` body, a `match` arm)
/// — the tail-recursion edges. A call in a NON-tail position (an operand) is NOT an edge. Mirrors
/// [`emit_tail`]'s propagation so the group and the emission agree.
fn tail_callees(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
    match core_of(db, id) {
        Core::Call { callee, .. } if !out.contains(&callee) => out.push(callee),
        Core::Call { .. } => {}
        Core::If { then_, else_, .. } => {
            tail_callees(db, then_, out);
            tail_callees(db, else_, out);
        }
        Core::Let { body, .. } => tail_callees(db, body, out),
        Core::Match { arms, .. } => {
            for a in arms {
                tail_callees(db, a.body, out);
            }
        }
        _ => {}
    }
}

/// Whether `from` tail-reaches `target` staying within `within` (a transitive closure over the tail
/// edges) — used to keep only the genuine SCC members (those that tail-cycle back to `self_def`).
fn tail_reaches(db: &mut Db, from: usize, target: usize, within: &[usize]) -> bool {
    let mut seen: Vec<usize> = vec![from];
    let mut i = 0;
    while i < seen.len() {
        let d = seen[i];
        i += 1;
        if let Some(body) = db.defs[d].body {
            let mut callees = Vec::new();
            tail_callees(db, body, &mut callees);
            for c in callees {
                if c == target {
                    return true;
                }
                if within.contains(&c) && !seen.contains(&c) {
                    seen.push(c);
                }
            }
        }
    }
    false
}

/// Whether the body at `id` makes a tail call to ANY member of `members` (the loop's iteration edge). A
/// call in a NON-tail position is not an edge. Mirrors [`emit_tail`]'s propagation.
fn body_has_tail_call_to(db: &mut Db, id: StructId, members: &[usize]) -> bool {
    match core_of(db, id) {
        Core::Call { callee, .. } => members.contains(&callee),
        Core::If { then_, else_, .. } => {
            body_has_tail_call_to(db, then_, members) || body_has_tail_call_to(db, else_, members)
        }
        Core::Let { body, .. } => body_has_tail_call_to(db, body, members),
        Core::Match { arms, .. } => arms
            .iter()
            .any(|a| body_has_tail_call_to(db, a.body, members)),
        _ => false,
    }
}

/// Render the node at `id` GROUNDED to the context integer type `it` — the width/signedness of the
/// construct the node sits in (an arithmetic/comparison op, an `if`/`match` result). A bare integer
/// LITERAL is width-polymorphic: its own `type_of` defaults to `Int64` (unification fixes the parent's
/// type from the definite operand but does NOT thread that width back onto the literal node), so a
/// literal operand of a narrow op would otherwise render `1u64 as i64` and produce a Rust type mismatch
/// against the narrow context (`u8::checked_add(i64)` → E0308). Grounding renders the literal at the
/// context width (`1u8`), exactly as the wasm backend's `emit_operand`/`emit_branch` normalize a bare
/// literal to the op/branch machine width. A NON-literal node already carries its own definite type, so
/// it emits unchanged.
fn emit_grounded(
    db: &mut Db,
    id: StructId,
    it: IntTy,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    if let Core::ConstInt(v) = core_of(db, id) {
        return emit_const_int_at(it, &v);
    }
    let rendered = emit(db, id, env, ctx)?;
    // WIDTH NORMALIZATION for a CONTROL-FLOW / non-literal operand. A bare literal is grounded above; but
    // an operand that is an `if`/`match` (or any node) whose BRANCHES are bare deferred-width literals is
    // solved at its OWN type — which defaults to Int64 — while the enclosing op wants the NARROW width
    // `it`. Emitting it unchanged renders an `i64` sub-expression where an `iN` is required (`(if … {
    // 100i64 } …).checked_add(100i8)` → rustc E0308). Reconcile HERE, at the consuming site, by wrapping
    // the operand down to the op's width with an `as <target>` cast — the native mirror of the wasm
    // backend's `i32.wrap_i64` narrow-value normalization. SOUND: a genuine fixed-width disagreement is a
    // type FAULT (CDZ0203) that aborts before emit, so a wider-than-`it` operand reaching here is a
    // deferred literal defaulted to Int64 whose low bits ARE its value; the cast truncates to `it` exactly
    // as the wasm wrap does, and the enclosing op's own overflow check then traps a true overflow (`(: (+
    // (if … 100 0) 100) Int8)` n=3 → 100+100=200 overflows Int8 → panics, matching wasm's trap). Only cast
    // when the operand's OWN solved integer type DIFFERS from `it` (same width → emit unchanged, no
    // redundant `as`); a non-integer operand emits unchanged.
    if let Ty::Int(op_it) = type_of(db, id)
        && (op_it.ground_signed(), op_it.ground_width()) != (it.ground_signed(), it.ground_width())
        && let Some(target) = types::rust_type(&Ty::Int(it))
    {
        // Parenthesize the rendered operand before the `as` so the cast binds to the WHOLE expression
        // regardless of its shape (an `if`/`match`/block would otherwise let `as` bind only to the last
        // sub-expression). `unused_parens` is allowed in the emitted header, so redundant parens are fine.
        return Ok(format!("(({rendered}) as {target})"));
    }
    Ok(rendered)
}

/// Render the node at `id` in TAIL position inside a self-loop (`ctx.self_loop` is `Some`) — the result
/// each path produces is the function's result. Tail-ness PROPAGATES through the result-producing
/// sub-positions (an `if`'s branches, a `match`'s arm bodies, a `let`'s body); the condition/scrutinee/
/// binding values are NOT tail (they are ordinary values, emitted via `emit`). At a tail LEAF:
///  - a SELF tail-call `f(a…)` becomes the parallel move `{ let (t…) = (a…); p0 = t0; …; continue }` —
///    all args computed into temps before any param is overwritten, then jump to the loop top;
///  - any other value `v` becomes `break v` (yielding the loop's — the function's — result).
///
/// Returns a Rust STATEMENT/expression usable as the loop body. Only called when `ctx.self_loop` is set.
fn emit_tail(db: &mut Db, id: StructId, env: &Env, ctx: &Ctx) -> Result<String, Reject> {
    let group = ctx
        .loop_group
        .expect("emit_tail is only called inside a loop group");
    match core_of(db, id) {
        // A tail call to a GROUP MEMBER iterates the loop: reassign the shared positional locals (+ the
        // `which` state, for a mutual group) and `continue`. A tail call to a NON-member is not a loop
        // edge — it falls to the `break <value>` leaf below (an ordinary boxed/awaited call in async).
        Core::Call { callee, args } if group.members.contains(&callee) => {
            // Ground each arg to the callee's param width, exactly as the ordinary call arm.
            let param_tys = crate::layout::def_params(db, callee);
            let mut rendered = Vec::new();
            for (i, &a) in args.iter().enumerate() {
                match param_tys.get(i).map(|(_, t)| t) {
                    Some(Ty::Int(it)) => rendered.push(emit_grounded(db, a, *it, env, ctx)?),
                    _ => rendered.push(emit(db, a, env, ctx)?),
                }
            }
            if rendered.len() != group.shared_params.len() {
                return Err(Reject::decline("tail-call arity mismatch"));
            }
            // Parallel move: bind all new values into temps, THEN assign each shared local — so an arg
            // that reads an old param value (`f(n-1, acc+n)`) sees the pre-iteration locals. For a mutual
            // group, also set `which` to the callee's member index (which member body runs next).
            let temps: Vec<String> = (0..rendered.len()).map(|i| format!("__t{i}")).collect();
            let binds = if rendered.is_empty() {
                String::new()
            } else {
                format!("let ({},) = ({},); ", temps.join(", "), rendered.join(", "))
            };
            let moves = group
                .shared_params
                .iter()
                .zip(&temps)
                .map(|(p, t)| format!("{p} = {t};"))
                .collect::<Vec<_>>()
                .join(" ");
            let set_which = if group.is_mutual() {
                let k = group.members.iter().position(|&m| m == callee).unwrap();
                format!("which = {k}; ")
            } else {
                String::new()
            };
            Ok(format!("{{ {binds}{set_which}{moves} continue; }}"))
        }
        // An `if` in tail position: both branches are tail; the condition is an ordinary value.
        Core::If { cond, then_, else_ } => {
            let c = emit(db, cond, env, ctx)?;
            let t = emit_tail(db, then_, env, ctx)?;
            let e = emit_tail(db, else_, env, ctx)?;
            Ok(format!("if {c} {{ {t} }} else {{ {e} }}"))
        }
        // A `let` in tail position: its bindings are ordinary values, its body is tail.
        Core::Let { bindings, body } => {
            let mut extended = env.clone();
            let mut lines = String::new();
            for (binder, value) in &bindings {
                let name = local_name(db, *binder, &extended);
                let v = emit(db, *value, &extended, ctx)?;
                lines.push_str(&format!("let {name} = {v}; "));
                extended.insert(*binder, name);
            }
            let b = emit_tail(db, body, &extended, ctx)?;
            Ok(format!("{{ {lines}{b} }}"))
        }
        // A `match` in tail position: each arm body is tail. (Delegates to the shared match emitter with
        // a tail flag so arm bodies go through `emit_tail`.)
        Core::Match { scrutinee, arms } => {
            emit_match_impl(db, id, scrutinee, &arms, env, ctx, true)
        }
        // Any other tail leaf: its value is the loop's result — `break` it out. A bare-literal leaf is
        // grounded to the function's result width so every `break` in the loop yields the same type.
        _ => {
            let v = match group.result_it {
                Some(it) => emit_grounded(db, id, it, env, ctx)?,
                None => emit(db, id, env, ctx)?,
            };
            Ok(format!("break {v};"))
        }
    }
}

/// Render the node at `id` as a Rust expression string. Exhaustive over `Core`; a form without a
/// scalar rendering declines. Reads the core + type columns on demand. The rendered expression is
/// parenthesized where needed so it composes as a sub-expression without precedence surprises.
fn emit(db: &mut Db, id: StructId, env: &Env, ctx: &Ctx) -> Result<String, Reject> {
    match core_of(db, id) {
        // An integer constant, written as its two's-complement BIT PATTERN in the unsigned type of its
        // width, then cast to the target type — the same bit-pattern emit the wasm backend does
        // (`to_i64_bits`/`to_i32_bits`). This one spelling covers a signed negative (`-128: Int8` =
        // `128u8 as i8`) and an unsigned value at/above the signed max (`UInt64.max` = `…u64`) alike.
        // The constant must FIT its width (checked here, CDZ0302 — a value that does not fit never
        // reaches a well-typed program, but selection re-checks rather than truncate silently).
        Core::ConstInt(v) => emit_const_int(db, id, &v),
        Core::ConstBool(b) => Ok(if b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        // Unit is Rust's `()`.
        Core::Unit => Ok("()".to_string()),
        // A parameter or kept-let reference — read the identifier its binder maps to. A binder with no
        // environment entry is a compiler bug (a ref whose binding was not brought into scope), so
        // decline rather than emit a dangling name.
        Core::Param { binder } | Core::LocalRef { binder } => env
            .get(&binder)
            .cloned()
            .ok_or_else(|| Reject::decline("reference has no bound Rust identifier")),
        // An `if` → Rust's `if cond { then } else { else }`. Rust's `if` is an expression, so it yields
        // the branch value directly — the structured target expresses the core's `If` as itself. Both
        // branches must produce the `if`'s RESULT type; a bare-literal branch is GROUNDED to that width
        // (via `emit_branch`) so a default-Int64 literal opposite a narrow branch does not mismatch the
        // block's type — the same reconciliation the wasm backend's `emit_branch` does.
        Core::If { cond, then_, else_ } => {
            let c = emit(db, cond, env, ctx)?;
            let t = emit_branch(db, then_, id, env, ctx)?;
            let e = emit_branch(db, else_, id, env, ctx)?;
            Ok(format!("if {c} {{ {t} }} else {{ {e} }}"))
        }
        // A short-circuiting boolean connective → Rust's own `&&`/`||`, which short-circuit with
        // exactly the core's semantics: `rhs` is evaluated ONLY on the non-short-circuiting branch, so
        // a trapping/effectful `rhs` is shielded just as the core's `if lhs then rhs else false`
        // (`and`) / `if lhs then true else rhs` (`or`) prescribes (core-semantics.md §Boolean
        // Connectives Short-Circuit). The structured target expresses the connective as itself.
        Core::And { lhs, rhs, is_and } => {
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            let op = if is_and { "&&" } else { "||" };
            Ok(format!("({l} {op} {r})"))
        }
        // An A-normal `let` sequence → a Rust block: each binding a `let name = value;`, then the body
        // as the block's tail expression. A kept binding names a runtime value used more than once, so
        // Rust computes it once and each `LocalRef` reads the binding — the same "name it once" the
        // core's `Let` encodes. No `drop` bookkeeping: Rust owns the value's lifetime (the native
        // strategy — `§A Backend … native aggregates`), so the Perceus dance the wasm backend does is
        // simply not needed here.
        Core::Let { bindings, body } => {
            let mut extended = env.clone();
            let mut lines = String::new();
            for (binder, value) in &bindings {
                let name = local_name(db, *binder, &extended);
                let v = emit(db, *value, &extended, ctx)?;
                lines.push_str(&format!("let {name} = {v}; "));
                extended.insert(*binder, name);
            }
            let b = emit(db, body, &extended, ctx)?;
            Ok(format!("{{ {lines}{b} }}"))
        }
        // A scalar `match` → Rust's `match`. Each arm renders `pattern => body`; a literal probe is the
        // literal pattern (written in the scrutinee's type), a wildcard/binder is `_`. `lower`
        // guaranteed exhaustiveness (a wildcard tail, or full Bool coverage), so the Rust match is
        // exhaustive too. The scrutinee is rendered once (Rust binds it), not re-tested per arm.
        Core::Match { scrutinee, arms } => emit_match(db, id, scrutinee, &arms, env, ctx),
        // A runtime comparison → the Rust comparison operator. Signedness/width are already baked into
        // the operands' Rust types (a `u32` compares unsigned, an `i8` signed), so the operator alone
        // is correct — no `_s`/`_u` variant selection like wasm needs. Both operands must share one
        // type; a bare-literal operand is GROUNDED to the comparison's integer type (the non-literal
        // side's width) so `(< a 5)` over a narrow `a` does not compare `u8 < i64` (Rust E0308).
        Core::Compare { op, lhs, rhs } => {
            let sym =
                compare_sym(op).ok_or_else(|| Reject::decline("not a comparison operator"))?;
            match operand_int_ty(db, lhs, rhs) {
                Some(it) => {
                    let l = emit_grounded(db, lhs, it, env, ctx)?;
                    let r = emit_grounded(db, rhs, it, env, ctx)?;
                    Ok(format!("({l} {sym} {r})"))
                }
                // A non-integer comparison (Bool operands) — no width to reconcile, emit as-is.
                None => {
                    let l = emit(db, lhs, env, ctx)?;
                    let r = emit(db, rhs, env, ctx)?;
                    Ok(format!("({l} {sym} {r})"))
                }
            }
        }
        // A runtime arithmetic op.
        Core::Arith { op, lhs, rhs } => emit_arith(db, id, op, lhs, rhs, env, ctx),
        // A float CONSTANT → a Rust float literal at the node's width. Emitted via `f64::from_bits`/
        // `f32::from_bits` of the canonical bit pattern so the EXACT value (incl. `-0.0`, a subnormal)
        // round-trips — a decimal spelling could lose a bit. The width is the node's solved type.
        Core::ConstFloat(d) => {
            let width = match type_of(db, id) {
                Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            if width == 32 {
                let bits = (f64::from_bits(d.to_f64_bits()) as f32).to_bits();
                Ok(format!("f32::from_bits({bits}u32)"))
            } else {
                Ok(format!("f64::from_bits({}u64)", d.to_f64_bits()))
            }
        }
        // A runtime `.wrap` conversion → an `as` cast to the target Rust type. Rust's `as` between
        // integers keeps the low bits and reinterprets at the target sign — bit-identical to
        // `IntValue::wrap_to`, and total (never panics), as `.wrap` requires.
        Core::Convert { op, operand } => match op {
            Prim::Wrap => {
                let dst = int_ty_of(db, id);
                let rty = types::rust_type(&Ty::Int(dst)).ok_or_else(|| {
                    Reject::decline("wrap target width has no native Rust representation")
                })?;
                let operand_s = emit(db, operand, env, ctx)?;
                Ok(format!("({operand_s} as {rty})"))
            }
            // A runtime int→float conversion `Float N.of-int` → an `as f64`/`as f32` cast (total,
            // round-to-nearest, matches the wasm `convert_i64_s`). The target width is the node's type.
            Prim::FloatOfInt => {
                let rty = types::rust_type(&type_of(db, id)).ok_or_else(|| {
                    Reject::decline("of-int target has no native Rust representation")
                })?;
                let operand_s = emit(db, operand, env, ctx)?;
                Ok(format!("({operand_s} as {rty})"))
            }
            // A runtime float-WIDTH conversion `Float N.of` → an `as f64`/`as f32` cast: Rust's `as`
            // between floats demotes with rounding (f64→f32) / promotes exactly (f32→f64) / is the
            // identity (same width) — matching the wasm demote/promote. Target width is the node's type.
            Prim::FloatOf => {
                let rty = types::rust_type(&type_of(db, id)).ok_or_else(|| {
                    Reject::decline("of target has no native Rust representation")
                })?;
                let operand_s = emit(db, operand, env, ctx)?;
                Ok(format!("({operand_s} as {rty})"))
            }
            _ => Err(Reject::decline("not a runtime conversion")),
        },
        // A boolean negation `!operand`.
        Core::Not { operand } => {
            let o = emit(db, operand, env, ctx)?;
            Ok(format!("(!{o})"))
        }
        // A runtime call → `callee(args…)`. The callee is a reachable definition every backend emits
        // (`layout::compute` closed the reachable set over `Core::Call`), rendered as its own `fn` (a
        // `pub fn` for an export, a private `fn` otherwise) — so a call names it by its source name,
        // whether or not it is exported. Each argument is GROUNDED to the callee's corresponding
        // parameter width: a bare literal arg (`(f 1)`) defaults to Int64 on its own, so a narrow
        // parameter would otherwise get an `i64` literal (the same width mismatch the operand fix
        // addressed) — read the callee's param types and ground each literal arg to its position's type.
        Core::Call { callee, args } => {
            let name = db.defs[callee].name.clone();
            if name.is_empty() {
                return Err(Reject::decline("call to an unnamed definition"));
            }
            let param_tys = crate::layout::def_params(db, callee);
            let mut rendered = Vec::new();
            for (i, &a) in args.iter().enumerate() {
                // Ground a literal arg to the callee's param type at this position; a non-literal arg,
                // or a position past the known params (arity is checked upstream), emits as-is.
                match param_tys.get(i).map(|(_, t)| t) {
                    Some(Ty::Int(it)) => rendered.push(emit_grounded(db, a, *it, env, ctx)?),
                    _ => rendered.push(emit(db, a, env, ctx)?),
                }
            }
            // The callee ident via `fn_ident` — the SAME uniqued name its declaration emits, so a call to a
            // β-copied do-local worker names ITS copy (`fac_7`), not a sibling copy's identically-named fn.
            let ident = super::fn_ident(db, ctx.layout, callee);
            if ctx.mode.is_async() {
                // Async/gas mode: thread `env` as the callee's first argument, and `Box::pin(…).await`
                // the call. The pin is what makes a RECURSIVE `async fn` well-sized (a recursive future
                // is otherwise infinite); a non-recursive call inlines upstream and never reaches here,
                // so this only ever wraps a genuine recursive/mutual call. `env` is passed by `&mut *env`
                // — a fresh reborrow per call so a later sibling call (two calls as operands of one op)
                // can borrow `env` again after this `.await` releases it.
                let args = if rendered.is_empty() {
                    "env".to_string()
                } else {
                    format!("env, {}", rendered.join(", "))
                };
                Ok(format!("Box::pin({ident}({args})).await"))
            } else {
                Ok(format!("{ident}({})", rendered.join(", ")))
            }
        }
        // A runtime TUPLE → Rust's native tuple literal `(e0, e1, …)`. Each element is rendered
        // recursively (a scalar or a nested tuple), so a tuple of scalars and a nested tuple both
        // compose directly. The native-aggregate value strategy: a Cadenza tuple IS a Rust tuple, no
        // heap handle (unlike the wasm backend's `arr-alloc`). A 1-tuple needs the trailing comma
        // `(e,)` to be a tuple rather than a parenthesized expression.
        Core::Tuple { elems } => {
            let mut parts = Vec::with_capacity(elems.len());
            for &e in &elems {
                parts.push(emit(db, e, env, ctx)?);
            }
            let trailing = if parts.len() == 1 { "," } else { "" };
            Ok(format!("({}{trailing})", parts.join(", ")))
        }
        // A runtime RECORD → a Rust tuple literal in SORTED FIELD-NAME order — the SAME representation
        // as a tuple (a record is structural/anonymous; at run time it IS a positional array in sorted
        // key order). The `fields` `BTreeMap` iterates sorted, so its VALUES in order are the tuple
        // elements; the field names are compile-time-only (they became positions), re-appearing only in
        // the boundary render. A field read is a `Core::Proj` at the sorted index (handled below), so
        // this only builds. (Nominal records → a named Rust struct is a future refinement.)
        Core::Record { fields } => {
            let mut parts = Vec::with_capacity(fields.len());
            // `fields` (a `BTreeMap`) iterates in sorted key order — its values in order are the tuple
            // elements, matching the sorted-field positions `Ty::Record`/`Core::Proj` use.
            for &v in fields.values() {
                parts.push(emit(db, v, env, ctx)?);
            }
            let trailing = if parts.len() == 1 { "," } else { "" };
            Ok(format!("({}{trailing})", parts.join(", ")))
        }
        // A runtime tuple/record PROJECTION `(. t i)` → Rust's tuple field access `(<operand>).index`.
        // The index is within the operand's static arity (checked before selection — for a record it is
        // the field's SORTED index, matching the `Core::Record` element order above), so it is always a
        // valid Rust field. Parenthesize the operand so a compound operand expression binds correctly.
        Core::Proj { operand, index } => {
            let t = emit(db, operand, env, ctx)?;
            Ok(format!("({t}).{index}"))
        }
        // A SUM VALUE CONSTRUCTION → the Rust enum variant `<Enum>::<Variant>(payloads…)`. The enum +
        // variant names come from the node's solved `Ty::Sum` declaration at the disc's index (the
        // discriminant IS the variant's declaration-order position). A nullary variant is the bare
        // `<Enum>::<Variant>` (no parens); a payload variant carries its args positionally — matching the
        // emitted `enum <Enum> { <Variant>(T…), … }`.
        Core::SumNew { disc, payloads } => {
            let path = sum_variant_path(db, id, disc)?;
            let mut args = Vec::with_capacity(payloads.len());
            for &p in &payloads {
                args.push(emit(db, p, env, ctx)?);
            }
            // A RECURSIVE variant's payload field is a `Box<…>` (the enum boxes it to stay finite), so its
            // payload value is wrapped `Box::new(…)` — the deref twin at the match site reads `*__pay`.
            // A non-recursive variant's field is unboxed. `wrap` applies the box exactly when the enum decl
            // did (`variant_is_recursive` is the shared predicate).
            let ty = type_of(db, id);
            let boxed = super::enums::variant_is_recursive(db, &ty, disc);
            let wrap = |payload: String| {
                if boxed {
                    format!("Box::new({payload})")
                } else {
                    payload
                }
            };
            match args.len() {
                // A nullary variant is the bare path (`None`, `Shape::Circle` with no payload).
                0 => Ok(path),
                // A one-payload variant carries its payload directly (`Some(x)`), boxed if recursive.
                1 => Ok(format!("{path}({})", wrap(args[0].clone()))),
                // A MULTI-payload variant carries ONE TUPLE (matching the enum decl's `V((T0, T1))` and the
                // core's single-`Ty::Tuple` payload model, which the match side reads as one indexed value).
                _ => Ok(format!("{path}({})", wrap(format!("({})", args.join(", "))))),
            }
        }
        // A poison reaching selection is a fault the collector surfaces before emission; reaching here
        // is a decline rather than emitted code (same as the wasm backend).
        Core::Poison(reject) => Err(reject),
        // A SUM MATCH → a Rust `match` on the scrutinee, dispatching on the variant. Each arm's
        // continuation is a leaf body or a nested switch (the decision tree). A payload BINDER in a body
        // is not bound in the arm pattern here — it resolves to a `Core::SumPayload` that re-extracts the
        // payload — so the arm pattern ignores the payload (`Enum::V { .. }` / `Enum::V(_)`).
        Core::MatchSum { scrutinee, root } => emit_sum_match(db, scrutinee, &root, env, ctx),
        // The SUB-VALUE of a sum scrutinee at a path, read by a variant pattern's binder. Rust binds in
        // the pattern, not by a separate accessor, so this re-matches the scrutinee to extract the
        // payload at `path`: `match <scrut> { <Enum>::<V>(p) => <walk path into p>, _ => unreachable!() }`.
        // Control is already in the matched arm (the disc was checked), so the `_` arm is unreachable.
        // Scrutinees here are pure (a param/local), so re-matching is cheap and observably identical.
        Core::SumPayload { scrutinee, path } => {
            emit_sum_payload(db, id, scrutinee, &path, env, ctx)
        }
        // `Option.expect`/`Result.expect` → `match <scrut> { <Enum>::<Present>(p) => p, _ => panic!() }`.
        // The present variant is `disc_present` (Some/Ok = 0); its single payload binds to a fresh name
        // and IS the expression's value; any other variant panics (the absent-variant trap — a Rust panic
        // is a Cadenza trap, matching the wasm `unreachable`). Scrutinee is pure (a param/local/call), so
        // matching it inline is sound.
        Core::SumExpect {
            scrutinee,
            disc_present,
        } => emit_sum_expect(db, scrutinee, disc_present, env, ctx),
        // `trap` → a Rust `panic!` (a Cadenza trap, matching the wasm `unreachable`). Rust's `panic!`
        // returns the never type `!`, which coerces to ANY expected type — the runtime counterpart of
        // `trap`'s `Never` unifying with any position. The message is dropped (the trap grades on halting,
        // not its text), so a fixed literal keeps the emit deterministic.
        Core::Trap => Ok("panic!(\"trap\")".to_string()),
        Core::ListNew { .. }
        | Core::ListLen { .. }
        | Core::ListPush { .. }
        | Core::ListConcat { .. }
        | Core::ListUpdate { .. }
        | Core::ListAt { .. }
        | Core::MatchList { .. }
        | Core::ConstStr(_)
        | Core::ConstChar(_)
        | Core::ConstFloatNan
        | Core::BytesOf { .. }
        | Core::BytesLen { .. }
        | Core::BytesAt { .. }
        | Core::StrAt { .. }
        | Core::BytesConcat { .. }
        | Core::BytesSlice { .. }
        | Core::BytesCompact { .. }
        // Runtime BigInt (a heap leaf + the runtime `bigint-*` ops) has no native Rust rendering yet —
        // the rust backend would need a Rust bignum runtime. Declines cleanly (a constant BigInt folds
        // and reaches this backend as a `Core::ConstInt`, which emits fine).
        | Core::BigIntOfI64 { .. }
        | Core::BigIntToI64 { .. }
        | Core::BigIntBinOp { .. }
        | Core::MapNew { .. }
        | Core::MapInsert { .. }
        | Core::MapLookup { .. }
        | Core::MapRemove { .. }
        | Core::MapSize { .. }
        | Core::SetOf { .. }
        | Core::SetContains { .. }
        | Core::SetInsert { .. }
        | Core::SetRemove { .. }
        | Core::SetLen { .. }
        | Core::SetAlgebra { .. }
        | Core::BinBuild { .. }
        | Core::BinBitsBuild { .. }
        | Core::BinIntRead { .. }
        | Core::BinRestRead { .. }
        | Core::Closure { .. }
        | Core::CallClosure { .. }
        | Core::Captured { .. }
        // A host call crosses the component boundary — the Rust backend emits no component imports, so it
        // declines (the wasm backend is the host-boundary target). A sequencing block only ever holds a
        // host-call statement today, so the Rust backend declines it too.
        | Core::HostCall { .. }
        | Core::Seq { .. }
        // Runtime structural equality is a value-heap walk — the Rust backend's scalar slice does not
        // emit heap ops, so it declines (decline-don't-miscompile), as it does every compound op.
        | Core::ValueEq { .. } => Err(Reject::decline(
            "the Rust backend does not yet render this compound value",
        )),
    }
}

/// Render an integer constant at the node's OWN solved type. Used only where the node stands in a
/// context that already fixes its width (a bare literal whose own `type_of` is definite). A literal
/// used as an OPERAND / BRANCH / ARM BODY of a construct is instead grounded to that construct's width
/// via [`emit_const_int_at`] — see [`emit_grounded`] — because a bare literal's own type is the default
/// (`Int64`), which unification does not thread the context width back onto.
fn emit_const_int(db: &mut Db, id: StructId, v: &IntValue) -> Result<String, Reject> {
    emit_const_int_at(int_ty_of(db, id), v)
}

/// Render an integer constant as `<bits><utype> as <target>` (or just `<bits><utype>` when the target
/// IS the unsigned bit type) at the GIVEN integer type `it` — the width/signedness of the CONTEXT the
/// literal appears in, not necessarily the literal's own defaulted type. Mirrors the wasm backend
/// (`emit_operand`/`emit_branch` ground a bare literal to the op/branch width): the value must fit that
/// width (else CDZ0302 — never truncate), and it is written as the two's-complement bit pattern so a
/// negative signed value and a large unsigned value share one spelling.
fn emit_const_int_at(it: IntTy, v: &IntValue) -> Result<String, Reject> {
    let signed = it.ground_signed();
    let width = it.ground_width();
    if !v.fits_width(signed, width) {
        return Err(Reject::coded(
            Code::IntOutOfRange,
            "integer literal does not fit its width",
        ));
    }
    let target = types::rust_type(&Ty::Int(it)).ok_or_else(|| {
        Reject::decline("integer literal width has no native Rust representation")
    })?;
    let ubits = types::unsigned_bits_type(it).ok_or_else(|| {
        Reject::decline("integer literal width has no native Rust representation")
    })?;
    // The unsigned bit pattern of the value at its width: the low `width` bits of its two's-complement
    // representation, as an unsigned magnitude. `wrap_to(false, width)` computes exactly that, and the
    // result is a non-negative `IntValue` whose decimal is the unsigned literal.
    let bits = v.wrap_to(false, width);
    let literal = int_value_decimal(&bits);
    if target == ubits {
        // The target is itself the unsigned bit type (a `UIntN`): write the literal directly.
        Ok(format!("{literal}{ubits}"))
    } else {
        // A signed (or otherwise reinterpreted) target: write the bit pattern in the unsigned type and
        // cast, so the sign is set from the bit pattern (`128u8 as i8` = -128), never a decimal minus.
        Ok(format!("({literal}{ubits} as {target})"))
    }
}

/// Render a runtime arithmetic op as a Rust expression, honoring the numeric model's traps:
///  - `+`/`-`/`*` → `<lhs>.checked_add(<rhs>).unwrap_or_else(|| <trap>)` — trap (panic) on overflow;
///  - `/`/`%` → `checked_div`/`checked_rem` — trap on ÷0 and `MIN / -1`;
///  - `&`/`|`/`^` → the total bitwise operator;
///  - `<<`/`>>` → a guarded block: count `>= N` traps; `<<` also round-trips to trap on overflow;
///    `>>` is arithmetic (signed) / logical (unsigned) via the value type's own `>>`.
fn emit_arith(
    db: &mut Db,
    id: StructId,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // A FLOAT arithmetic op (`+.`/`-.`/`*.`/`/.`) → the native Rust `+`/`-`/`*`/`/` on `f64`/`f32`. IEEE,
    // never traps (no `checked_*`/overflow panic, unlike the integer arith below) — matches the wasm
    // machine op. Both operands share the op's float type, so they emit as-is (no width grounding).
    if op.is_float_arith() {
        let sym = match op {
            Prim::FAdd => "+",
            Prim::FSub => "-",
            Prim::FMul => "*",
            Prim::FDiv => "/",
            _ => unreachable!("guarded by is_float_arith"),
        };
        let l = emit(db, lhs, env, ctx)?;
        let r = emit(db, rhs, env, ctx)?;
        return Ok(format!("({l} {sym} {r})"));
    }
    // Both operands share the OP's integer type (its result width == operand width). Ground a bare
    // literal operand to it so `(+ a 1)` over a narrow `a` emits `<narrow>::checked_add(1<narrow>)`,
    // not `checked_add((1u64 as i64))` (Rust E0308) — the analogue of the wasm backend's `emit_operand`.
    let it = int_ty_of(db, id);
    let l = emit_grounded(db, lhs, it, env, ctx)?;
    let r = emit_grounded(db, rhs, it, env, ctx)?;
    match op {
        Prim::Add | Prim::Sub | Prim::Mul => {
            let method = match op {
                Prim::Add => "checked_add",
                Prim::Sub => "checked_sub",
                Prim::Mul => "checked_mul",
                _ => unreachable!(),
            };
            // `checked_*` returns `None` exactly when the true result leaves the N-bit type — the
            // numeric model's overflow trap. Panic on `None` (an aborting trap, the native `unreachable`
            // analogue); the message names the op so a trap is legible.
            Ok(format!(
                "({l}).{method}({r}).unwrap_or_else(|| panic!(\"integer overflow in {}\"))",
                op_name(op),
            ))
        }
        Prim::Div => {
            // `checked_div` returns `None` on a zero divisor AND on `MIN / -1` — precisely the two cases
            // the numeric model traps for division (`MIN / -1` overflows: the quotient +2^(N-1) is out of
            // range). Panic on either, mirroring the wasm `i64.div_s` native trap.
            Ok(format!(
                "({l}).checked_div({r}).unwrap_or_else(|| panic!(\"{} by zero or overflow\"))",
                op_name(op),
            ))
        }
        Prim::Rem => {
            // `%` traps ONLY on a zero divisor — NOT on `MIN % -1`. `x % -1` is 0 for every x, including
            // `MIN % -1 = 0` (numeric-model.md §Modulo by -1 is always zero: modulo forms no quotient, so
            // it has no overflow — the check that makes `/` trap must NOT apply to `%`). Rust's
            // `checked_rem` WRONGLY returns `None` at `MIN % -1` (it conflates the remainder with the
            // division overflow), so it cannot be used here — it would panic where the value must be 0.
            // Guard only the zero divisor explicitly, then `wrapping_rem`, which yields 0 at `MIN % -1`
            // (it performs no overflow check), matching the wasm backend's `i64.rem_s`. Evaluate each
            // operand once into a block-local binding so a side-effecting operand runs exactly once.
            Ok(format!(
                "{{ let (l, r) = ({l}, {r}); \
                 if r == 0 {{ panic!(\"{} by zero\") }} else {{ l.wrapping_rem(r) }} }}",
                op_name(op),
            ))
        }
        Prim::BitAnd | Prim::BitOr | Prim::BitXor => {
            let sym = match op {
                Prim::BitAnd => "&",
                Prim::BitOr => "|",
                _ => "^",
            };
            Ok(format!("({l} {sym} {r})"))
        }
        // WRAPPING arithmetic → Rust's own `wrapping_add`/`wrapping_mul` — two's-complement wraparound,
        // never panics (the native mirror of the wasm backend's raw `i64.add`/`i64.mul`). `it` is the
        // aliased width N, so the operands are the N-bit type and the wrap is modulo 2^N.
        Prim::WrappingAdd | Prim::WrappingMul => {
            let method = if matches!(op, Prim::WrappingAdd) {
                "wrapping_add"
            } else {
                "wrapping_mul"
            };
            Ok(format!("({l}).{method}({r})"))
        }
        // A runtime shift, honoring the numeric model's trapping semantics exactly (mirroring the wasm
        // backend's `emit_shift` — `numeric-model.md` §A Shift Is Not Exempt From Overflow Is Defined):
        //   - COUNT GUARD: a count outside `0..N` traps. The count is read as `u32` and compared `>= N`,
        //     which catches BOTH a too-large count and a negative one (a negative read unsigned is huge);
        //   - `<<` is exact `*2^count`, so it TRAPS on overflow: shift, then round-trip `(r >> count)`
        //     must recover the value — Rust's `>>` is arithmetic for a signed type / logical for an
        //     unsigned one, so the inverse is exact and the check catches a dropped high bit;
        //   - `>>` is arithmetic (signed) / logical (unsigned) — Rust's native `>>` on the value's type
        //     already IS that, so the count guard is the only trap.
        // `it` is the op's aliased width N (a non-aliased width already declined at `rust_type`), so the
        // Rust value type IS the N-bit native type — no wider-slot round-trip like wasm needs. Emitted as
        // a block that binds the value + count once (so a computed operand is evaluated once) then guards.
        Prim::Shl | Prim::Shr => {
            let width = it.ground_width();
            let vty = types::rust_type(&Ty::Int(it)).ok_or_else(|| {
                Reject::decline("shift value width has no native Rust representation")
            })?;
            // The count expression: its own solved type (a shift count is not rigidly the value's type),
            // cast to u32 for the guard and the shift-count position.
            let count_it = int_ty_of(db, rhs);
            let count = emit_grounded(db, rhs, count_it, env, ctx)?;
            if matches!(op, Prim::Shr) {
                // `>>`: guard the count, then the native shift (arithmetic/logical by `vty`'s sign).
                Ok(format!(
                    "{{ let v: {vty} = {l}; let c = ({count}) as u32; \
                     if c >= {width} {{ panic!(\"shift count out of range\") }} v >> c }}"
                ))
            } else {
                // `<<`: guard the count, shift, then round-trip to detect an overflow (a dropped bit).
                Ok(format!(
                    "{{ let v: {vty} = {l}; let c = ({count}) as u32; \
                     if c >= {width} {{ panic!(\"shift count out of range\") }} \
                     let r = v << c; \
                     if (r >> c) != v {{ panic!(\"integer overflow in left shift\") }} r }}"
                ))
            }
        }
        _ => Err(Reject::decline(
            "not a runtime integer arithmetic operation",
        )),
    }
}

/// Render a scalar `match` as Rust's `match`. The scrutinee is rendered once (Rust binds it as the
/// matchee); each arm is `pattern [if guard] => body`. A literal probe becomes the literal pattern
/// written in the scrutinee's type; a wildcard OR a bare-name BINDER becomes `_` (a binder resolves to
/// the scrutinee occurrence in `resolve`, so a body reference to the binder already re-reads the
/// scrutinee — no Rust binding pattern is needed). A guarded arm emits Rust's own pattern guard `if
/// <cond>`, which Rust evaluates ONLY after the pattern matches and falls through on false — exactly
/// the core's guard semantics (short-circuit + fall-through), so no manual nesting is needed.
///
/// EXHAUSTIVENESS maps across: `lower` admits a runtime match only if it is exhaustive by its UNGUARDED
/// arms (a guard does not count — `numeric-model`/CDZ0210), which is Rust's rule too. An integer match
/// therefore carries an unguarded wildcard/binder arm → a Rust `_` catch-all; a Bool match carries
/// `true`+`false`. Arms AFTER an unguarded catch-all are unreachable in both models, so emission stops
/// at the first unguarded `_` (mirroring the wasm probe chain, and avoiding Rust's unreachable-arm
/// lint) — leaving a `match` Rust sees as exhaustive.
fn emit_match(
    db: &mut Db,
    match_id: StructId,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    emit_match_impl(db, match_id, scrutinee, arms, env, ctx, false)
}

/// Render a scalar `match`, with `tail` selecting whether the arm bodies are in TAIL position (inside a
/// self-loop): when `tail`, each arm body goes through [`emit_tail`] (a self-call iterates the loop, any
/// other value `break`s it); otherwise each arm body is an ordinary expression grounded to the match's
/// result width. The scrutinee, patterns, and guards are identical either way.
#[allow(clippy::too_many_arguments)]
fn emit_match_impl(
    db: &mut Db,
    match_id: StructId,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    env: &Env,
    ctx: &Ctx,
    tail: bool,
) -> Result<String, Reject> {
    // The match's RESULT integer type, if any — a bare-literal arm body is grounded to it so a
    // default-Int64 literal arm beside a narrow-width arm does not yield a mismatched type (Rust E0308),
    // the same reconciliation the wasm backend applies to a `ConstInt` arm body via `emit_operand`.
    let result_it = match type_of(db, match_id) {
        Ty::Int(it) => Some(it),
        _ => None,
    };
    let scrut = emit(db, scrutinee, env, ctx)?;
    let mut out = format!("match ({scrut}) {{ ");
    for arm in arms {
        let pat = match arm.probe {
            crate::core::Probe::Int(ref v) => int_pattern(db, scrutinee, v)?,
            crate::core::Probe::Bool(x) => (if x { "true" } else { "false" }).to_string(),
            // A string-literal probe only ever FOLDS (a constant scrutinee); a runtime string match
            // declines at `is_scalar` before a `Core::Match` is built, so no `Probe::Str` reaches a
            // runtime match emit on either backend.
            crate::core::Probe::Str(_) => {
                return Err(crate::diag::Reject::decline(
                    "a runtime string-literal match is not yet emitted",
                ));
            }
            // A `ListLen` probe only ever FOLDS (a constant list payload); a runtime list payload declines
            // at `build_lit_test` before a decision tree is emitted, so it never reaches a runtime match.
            crate::core::Probe::ListLen { .. } => {
                return Err(crate::diag::Reject::decline(
                    "a runtime list-pattern match is not yet emitted",
                ));
            }
            crate::core::Probe::Wild => "_".to_string(),
        };
        let guard = match arm.guard {
            Some(g) => format!(" if {}", emit(db, g, env, ctx)?),
            None => String::new(),
        };
        let b = if tail {
            // Tail arm: `emit_tail` produces `break v;` / a self-loop `continue` — a statement, so the
            // arm is `pat => { <stmt> }` (braces make a statement a valid match-arm body).
            format!("{{ {} }}", emit_tail(db, arm.body, env, ctx)?)
        } else {
            match result_it {
                Some(it) => emit_grounded(db, arm.body, it, env, ctx)?,
                None => emit(db, arm.body, env, ctx)?,
            }
        };
        out.push_str(&format!("{pat}{guard} => {b}, "));
        // An UNGUARDED wildcard is the unconditional catch-all — every later arm is unreachable (as in
        // `lower`/wasm). Stop here so the emitted `match` is exhaustive with no unreachable arm.
        if arm.guard.is_none() && matches!(arm.probe, crate::core::Probe::Wild) {
            break;
        }
    }
    out.push('}');
    Ok(out)
}

/// An integer literal PATTERN in the scrutinee's Rust type — the literal written so it matches a value
/// of that type. Uses the same bit-pattern spelling as a constant, but a pattern cannot contain an
/// `as` cast, so a value that would need reinterpretation (a signed negative, or an unsigned value
/// above the signed max) is written as its signed decimal / plain unsigned decimal directly.
fn int_pattern(db: &mut Db, scrutinee: StructId, v: &IntValue) -> Result<String, Reject> {
    let it = int_ty_of(db, scrutinee);
    let target = types::rust_type(&Ty::Int(it)).ok_or_else(|| {
        Reject::decline("match scrutinee width has no native Rust representation")
    })?;
    // A pattern is written as a plain decimal in the target type (`5i64`, `-1i8`). `int_value_decimal`
    // gives the signed decimal (with a leading `-` for a negative), which is a valid Rust integer
    // pattern for the signed target; for an unsigned target the value is non-negative so it is a plain
    // decimal. This is exact for every in-range value (range already checked at type time).
    Ok(format!("{}{target}", int_value_signed_decimal(v)))
}

/// The Rust identifier a `let` binding is emitted under — its source name, made a valid identifier,
/// de-collided against names already in scope by appending a numeric suffix. Determinism matters: the
/// body's `LocalRef` to this binding must resolve to the same identifier, so it is inserted into the
/// environment by the caller right after this returns.
fn local_name(db: &Db, binder: StructId, env: &Env) -> String {
    let base = db
        .ast
        .as_name(binder)
        .map(super::sanitize_ident)
        .unwrap_or_else(|| "tmp".to_string());
    // De-collide: if the base is already bound (a shadowing `let`, or a param of the same name), append
    // a suffix until unique. The binder occurrence is unique, so this always terminates.
    if !env.values().any(|n| n == &base) {
        return base;
    }
    let mut n = 1;
    loop {
        let cand = format!("{base}_{n}");
        if !env.values().any(|v| v == &cand) {
            return cand;
        }
        n += 1;
    }
}

/// The signed decimal string of an integer value (a leading `-` for a negative) — for a Rust literal
/// or pattern in a signed context.
fn int_value_signed_decimal(v: &IntValue) -> String {
    let mag = int_value_decimal(v);
    if v.negative && mag != "0" {
        format!("-{mag}")
    } else {
        mag
    }
}

/// The decimal string of an integer value's MAGNITUDE (unsigned, no sign) — the big-endian magnitude
/// bytes rendered in base 10. Empty magnitude is `0`. Done by repeated division so it needs no bignum
/// dependency (the value is arbitrary-precision; a width-bounded value here is small, but the routine
/// is general).
fn int_value_decimal(v: &IntValue) -> String {
    if v.magnitude.is_empty() || v.magnitude.iter().all(|&b| b == 0) {
        return "0".to_string();
    }
    // Repeatedly divide the big-endian magnitude by 10, collecting remainder digits.
    let mut digits = Vec::new();
    let mut cur = v.magnitude.clone();
    while !cur.iter().all(|&b| b == 0) {
        let mut rem: u16 = 0;
        for byte in cur.iter_mut() {
            let acc = (rem << 8) | (*byte as u16);
            *byte = (acc / 10) as u8;
            rem = acc % 10;
        }
        digits.push(b'0' + rem as u8);
    }
    digits.reverse();
    String::from_utf8(digits).expect("ascii digits")
}

/// The Rust comparison operator symbol for a comparison prim, or `None` for a non-comparison prim.
fn compare_sym(op: Prim) -> Option<&'static str> {
    Some(match op {
        Prim::Lt => "<",
        Prim::Gt => ">",
        Prim::Le => "<=",
        Prim::Ge => ">=",
        Prim::Eq => "==",
        _ => return None,
    })
}

/// A human op name for a trap panic message.
fn op_name(op: Prim) -> &'static str {
    match op {
        Prim::Add => "addition",
        Prim::Sub => "subtraction",
        Prim::Mul => "multiplication",
        Prim::Div => "division",
        Prim::Rem => "remainder",
        _ => "arithmetic",
    }
}

/// The integer type of the node at `id`, defaulting to `Int64` for a non-integer type — the same
/// read-off `select.rs` does (`int_ty_of`). A non-integer node never reaches an integer-typed emit
/// path, so the default is defensive.
fn int_ty_of(db: &mut Db, id: StructId) -> IntTy {
    match type_of(db, id) {
        Ty::Int(it) => it,
        _ => IntTy {
            sign: Sign::Fixed(true),
            width: Width::Fixed(crate::ty::DEFAULT_INT_WIDTH),
        },
    }
}

/// The Rust path `<Enum>::<Variant>` for the sum value at `id` (a `Ty::Sum`) whose runtime discriminant
/// is `disc`. The enum name is the sum's declared name (sanitized); the variant name is the declaration's
/// `disc`-th variant (the discriminant IS the declaration-order position). Both are `sum_ident`-sanitized
/// so they match the emitted `enum` declaration. Declines if the node is not a sum or the disc is out of
/// range (a compiler bug — a `SumNew` always carries a sum type + an in-range disc).
fn sum_variant_path(db: &mut Db, id: StructId, disc: u32) -> Result<String, Reject> {
    let ty = type_of(db, id);
    sum_variant_path_of_ty(db, &ty, disc)
}

/// The Rust `<Enum>::<Variant>` path for the `disc`-th variant of the sum TYPE `ty` — the type-keyed core
/// of [`sum_variant_path`] (which reads the type off a node). Split out so a nested switch can name a
/// variant of a sub-value's type. Declines if `ty` is not a sum, its enum is not representable (a
/// recursive/unrepresentable sum has no Rust type), or the disc is out of range.
fn sum_variant_path_of_ty(db: &mut Db, ty: &Ty, disc: u32) -> Result<String, Reject> {
    let decl_occ = match ty.strip_nominal() {
        Ty::Sum { decl, .. } => *decl,
        _ => return Err(Reject::decline("sum construction node is not a sum type")),
    };
    // The sum's enum must have EMITTED — a recursive/unrepresentable sum has no Rust type, so naming
    // `<Enum>::<Variant>` here would reference an undeclared type. This catches a construct/match of such
    // a sum ANYWHERE IN A BODY (not just a signature): the fold can inline a helper that builds a
    // non-representable sum as a discarded intermediate (`(. (tuple (NLit 5) 9) 1)` keeps only the Int64,
    // but still constructs `Node::NLit`), which the signature-level `sum_representable` guard cannot see.
    if !super::enums::sum_representable(db, ty) {
        return Err(Reject::decline(
            "a construct/match of a sum with no emitted Rust enum (recursive/unrepresentable)",
        ));
    }
    let decl = db
        .type_decl_by_occ(decl_occ)
        .ok_or_else(|| Reject::decline("sum declaration not found"))?;
    let enum_name = types::sum_ident(&decl.name);
    let variant = decl
        .variants
        .get(disc as usize)
        .ok_or_else(|| Reject::decline("sum discriminant out of range"))?;
    let vname = types::sum_ident(&variant.name);
    Ok(format!("{enum_name}::{vname}"))
}

/// The payload type of a sum's variant 0 (the shape a `Payload` path step descends into) — `None` for a
/// nullary or unresolvable variant. Substitutes the sum's actual type args into the variant's generic
/// payload (`Option Int64`'s `Some` payload is `Int64`, not `?0`). The rust-backend twin of the wasm
/// backend's `sum_single_payload_ty`; used by `ty_at_sum_path` to walk a nested switch's subject type.
fn sum_disc0_payload_ty(db: &mut Db, sum: &Ty) -> Option<Ty> {
    variant_payload_ty(db, sum, 0)
}

/// The payload type of a sum's variant `disc` at THIS instantiation — `None` for a nullary or
/// unresolvable variant. Generalizes [`sum_disc0_payload_ty`] to ANY discriminant: a nested switch on a
/// variant at disc ≥ 1 (`(type W (A Int64) (V (Option Int64)))` matched `(W.V (Some n))`) must read the
/// payload of the ACTUAL entered variant (`V` → `Option Int64`), not variant 0's (`A` → `Int64`). Reading
/// variant 0 unconditionally made a nested constructor match on a non-first variant resolve to the wrong
/// sub-value type and decline (`sum construction node is not a sum type`). Substitutes the sum's actual
/// type args (`Option a`'s `V` payload at `W Int64` → `Option Int64`).
fn variant_payload_ty(db: &mut Db, sum: &Ty, disc: u32) -> Option<Ty> {
    let stripped = sum.strip_nominal().clone();
    let Ty::Sum { decl, .. } = &stripped else {
        return None;
    };
    let ctor = {
        let td = db.type_decl_by_occ(*decl)?;
        td.variants.get(disc as usize)?.ctor?
    };
    crate::infer::payload_ty_at_instantiation(db, ctor, &stripped)
}

/// Emit a sum MATCH → a Rust `match` on the scrutinee. The `root` continuation is normally a
/// [`SumCont::Switch`] on the scrutinee's own discriminant (`path` empty); each `SumArm` becomes
/// `<Enum>::<Variant>(binder) => <cont>` (a nullary variant → `<Enum>::<Variant> => …`, no binding) and a
/// `disc: None` arm is the `_` default. The arm BINDS its variant's payload to a fresh identifier and
/// threads a `SumBind` (keyed by the scrutinee + the arm's path `[Payload]`) into the continuation's
/// `Ctx`, so a `Core::SumPayload` in the body resolves to that identifier. A LEAF continuation is the arm
/// body; a NESTED switch (the decision tree recursing into a deeper sub-value) and a GUARDED arm (a
/// sum-scrutinee guard) are DECLINED for now — the common single-level match (Option, a flat user sum)
/// lands first; nested constructor patterns and sum guards follow.
///
/// A disc-fold can collapse the root to a nested `Switch` on a deeper path (a statically-known scrutinee
/// discriminant), or to a `Guarded`/`Leaf` — those non-`Switch` roots are declined here (they need the
/// deeper-path/guard rendering not yet built); the reached-directly `Leaf` root already folds in `lower`.
fn emit_sum_match(
    db: &mut Db,
    scrutinee: StructId,
    root: &crate::core::SumCont,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // The root is normally a Switch on the scrutinee's own discriminant (path empty). A non-root-Switch
    // continuation (a Guarded arm or a bare Leaf as the ROOT) is a shape this slice does not yet render —
    // decline. A `Switch` root (empty or a disc-folded deeper path) recurses through `emit_sum_switch`.
    // A `Switch` root recurses through `emit_sum_switch`. A NON-Switch root arises when the disc-fold in
    // `lower` collapses the root `Switch` on a STATICALLY-KNOWN discriminant (a constant `SumNew`
    // scrutinee) to the selected arm's continuation — a `LitTest` (`(match (Cons x Nil) ((Cons 0 t) …))`
    // where the `Cons` tag is known but the payload `x` is runtime), a `Guarded`, or a bare `Leaf`. Those
    // continuations are exactly what `emit_sum_cont` renders (it reads a sub-value via `emit_sum_payload`,
    // which folds against the constant scrutinee's payload nodes), so route them there rather than
    // declining. Before this, a constant-disc recursive/literal match declined on Rust while wasm compiled
    // it — the last non-Switch-root gap.
    match root {
        crate::core::SumCont::Switch { path, arms } => {
            emit_sum_switch(db, scrutinee, path, arms, env, ctx)
        }
        crate::core::SumCont::Guarded { .. }
        | crate::core::SumCont::Leaf(_)
        | crate::core::SumCont::LitTest { .. } => emit_sum_cont(db, scrutinee, root, env, ctx),
    }
}

/// Emit a `Switch` on the sub-value of `scrutinee` at `sw_path` — a Rust `match` dispatching on each
/// arm's discriminant. The switched-on VALUE is the scrutinee itself for the root (`sw_path == []`) or the
/// payload the enclosing arm bound for a NESTED switch (`sw_path` reads it via `emit_sum_payload`, which
/// resolves the parent arm's `__pay` binding). Each arm binds its own payload (`__pay{i}` at `sw_path +
/// [Payload]`) and recurses on its continuation: a `Leaf` emits the body, a nested `Switch` emits an inner
/// `match` (a nested constructor pattern like `(Some (Ok n))` — the outer switches Some/None, the Some
/// arm's continuation switches Ok/Err of the payload). Guarded / literal-payload continuations are still
/// declined (a later slice). This is what lets a RUNTIME nested sum match render on the Rust backend, the
/// two-compiler companion of the wasm decision-tree walk.
fn emit_sum_switch(
    db: &mut Db,
    scrutinee: StructId,
    sw_path: &[crate::core::PathStep],
    arms: &[crate::core::SumArm],
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // ERASED-NEWTYPE ALIGNMENT: a `Payload` step over a NOMINAL newtype sub-value is a runtime no-op (the
    // tag erases; the value IS the inner). `lower` drops such steps from the BODY's `SumPayload` read
    // paths (`erase_nominal_steps`), but the decision-tree's switch/bind paths keep them, so a switch on a
    // sum WRAPPED in an erased newtype (`(type W (V (Result …)))` matched `(W.V (Result.Ok n))`) carries a
    // leading nominal `[Payload]` the erased body read does not — the bind (`sw_path+[Payload]`) and the
    // read (erased) then disagree by one step ("sum payload has no bound match arm"). Erase the switch
    // path the same way here so the subject reads the inner sum directly and every bind path this switch
    // mints aligns with the erased body reads. (wasm tolerates the raw path via its runtime-rep
    // coincidence; the Rust backend's path-keyed binds need the alignment.)
    let sw_path_owned = erase_nominal_switch_path(db, scrutinee, sw_path);
    let sw_path = &sw_path_owned[..];
    // The value this switch dispatches on: the scrutinee (root, empty path) or the sub-value at `sw_path`
    // (a nested switch — read the enclosing arm's payload binding). `emit_sum_payload` folds a constant
    // scrutinee or reads the bound `__pay` name.
    let subject = if sw_path.is_empty() {
        emit(db, scrutinee, env, ctx)?
    } else {
        emit_sum_payload(db, scrutinee, scrutinee, sw_path, env, ctx)?
    };
    // The SOLVED TYPE of the value this switch dispatches on. At the root (`sw_path == []`) it is the
    // scrutinee's own type; at a nested switch it is the sub-value type an ENCLOSING arm recorded in
    // `sum_path_types` when it descended into this variant (`variant_payload_ty` of the entered disc). A
    // recorded hint is authoritative — it carries which variant was entered, which the flattened path
    // cannot; only if none is recorded (the root, or a path with no hint) do we walk the type from the
    // scrutinee via `ty_at_sum_path` (which then falls back to the disc-0 payload for a `Payload` step).
    let subject_ty = lookup_sum_path_type(ctx, sw_path)
        .unwrap_or_else(|| ty_at_sum_path(db, scrutinee, sw_path));
    let mut out = format!("match {subject} {{ ");
    for (i, arm) in arms.iter().enumerate() {
        match arm.disc {
            Some(disc) => {
                // `<Enum>::<Variant>(binder) => cont`. The payload binder is a fresh `__pay_{path}_{i}` the
                // arm's `SumPayload { scrutinee, sw_path + [Payload] (…) }` resolves to; a nullary variant
                // binds nothing. The bind's path is FROM THE ROOT scrutinee (`sw_path + [Payload]`), so a
                // deeper `SumPayload` (a binder in this arm's body, or a nested switch's subject) resolves.
                let vpath = sum_variant_path_of_ty(db, &subject_ty, disc)?;
                let arity = variant_arity_of_ty(db, &subject_ty, disc);
                let (pat_tail, arm_ctx) = if arity == 0 {
                    (String::new(), ctx.clone())
                } else {
                    let name = format!("__pay_{}_{i}", sw_path.len());
                    let mut payload_path = sw_path.to_vec();
                    payload_path.push(crate::core::PathStep::Payload);
                    // A RECURSIVE variant's field is a `Box<…>` (the enum boxes it), so the bind is boxed —
                    // a read derefs. The switched variant's type is THIS switch's subject type.
                    let boxed = super::enums::variant_is_recursive(db, &subject_ty, disc);
                    let mut c = ctx.clone();
                    c.sum_binds.push(SumBind {
                        scrutinee,
                        path: payload_path.clone(),
                        name: name.clone(),
                        boxed,
                    });
                    // RECORD the entered variant's payload type at the bind path, so a NESTED switch on this
                    // arm's payload (a disc-≥1 variant carrying a sum) resolves its subject to the ACTUAL
                    // payload type, not variant-0's. This is what the flattened path alone cannot supply.
                    if let Some(pty) = variant_payload_ty(db, &subject_ty, disc) {
                        c.sum_path_types.push((payload_path, pty));
                    }
                    (format!("({name})"), c)
                };
                let cont = emit_sum_cont(db, scrutinee, &arm.cont, env, &arm_ctx)?;
                out.push_str(&format!("{vpath}{pat_tail} => {cont}, "));
            }
            None => {
                // The default (wildcard) tail. Its continuation is emitted in the OUTER ctx (no payload
                // bound — a wildcard arm binds nothing of the switched variant).
                let cont = emit_sum_cont(db, scrutinee, &arm.cont, env, ctx)?;
                out.push_str(&format!("_ => {cont}, "));
            }
        }
    }
    out.push('}');
    Ok(out)
}

/// Emit an arm's CONTINUATION as a Rust EXPRESSION:
///  - `Leaf` → the arm body;
///  - nested `Switch` → an inner `match` ([`emit_sum_switch`], a nested constructor pattern);
///  - `Guarded { cond, body, els }` → `if <cond> { <body> } else { <els-cont> }` — the variant already
///    matched (the enclosing switch bound its payload into `ctx`), so `cond`/`body` see the payload binder;
///    a false guard FALLS THROUGH to the `els` continuation (the rest of the sub-matrix), mirroring the
///    wasm backend's guarded `if`;
///  - `LitTest { path, probe, then_, els }` → `if (<sub-value at path> == <literal>) { <then-cont> } else
///    { <els-cont> }` — a payload-literal refinement (`(Some 0)`); the sub-value is read via
///    `emit_sum_payload` (folds a constant / reads the bound name), compared to the literal, and a mismatch
///    falls through to `els` (the binding arm). Both mirror the wasm `emit_sum_cont`'s desugar to an `if`.
fn emit_sum_cont(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    match cont {
        crate::core::SumCont::Leaf(b) => emit(db, *b, env, ctx),
        crate::core::SumCont::Switch { path, arms } => {
            emit_sum_switch(db, scrutinee, path, arms, env, ctx)
        }
        crate::core::SumCont::Guarded { cond, body, els } => {
            let c = emit(db, *cond, env, ctx)?;
            let then_ = emit(db, *body, env, ctx)?;
            let els = emit_sum_cont(db, scrutinee, els, env, ctx)?;
            Ok(format!("if {c} {{ {then_} }} else {{ {els} }}"))
        }
        crate::core::SumCont::LitTest {
            path,
            probe,
            then_,
            els,
        } => {
            let subject = emit_sum_payload(db, scrutinee, scrutinee, path, env, ctx)?;
            // The literal to compare against, in the sub-value's own type (`5i64`, `true`) so the Rust
            // comparison types. A string probe never reaches a RUNTIME test (it declines at `is_scalar`
            // before a decision tree is built), matching the scalar-match path.
            let lit = match probe {
                crate::core::Probe::Int(v) => {
                    // The sub-value's integer type gives the literal's suffix; a `Payload`/`Elem` path ends
                    // at an Int leaf. Prefer an arm-recorded path type (the entered-variant type, exact for
                    // a disc-≥1 payload), falling back to a scrutinee-rooted walk.
                    let sub = lookup_sum_path_type(ctx, path)
                        .unwrap_or_else(|| ty_at_sum_path(db, scrutinee, path));
                    let it = match sub {
                        Ty::Int(it) => it,
                        _ => IntTy {
                            sign: Sign::Fixed(true),
                            width: Width::Fixed(crate::ty::DEFAULT_INT_WIDTH),
                        },
                    };
                    let target = types::rust_type(&Ty::Int(it)).ok_or_else(|| {
                        Reject::decline("a literal-payload width has no native Rust representation")
                    })?;
                    format!("{}{target}", int_value_signed_decimal(v))
                }
                crate::core::Probe::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
                crate::core::Probe::Str(_)
                | crate::core::Probe::ListLen { .. }
                | crate::core::Probe::Wild => {
                    return Err(Reject::decline(
                        "a non-scalar literal-payload probe is not rendered by the Rust backend",
                    ));
                }
            };
            let then_ = emit_sum_cont(db, scrutinee, then_, env, ctx)?;
            let els = emit_sum_cont(db, scrutinee, els, env, ctx)?;
            Ok(format!(
                "if ({subject}) == {lit} {{ {then_} }} else {{ {els} }}"
            ))
        }
    }
}

/// The TYPE of the sub-value reached by walking `sw_path` from `scrutinee`'s type — a `Payload` step
/// descends a variant's payload (the sum's disc-0 single payload at its instantiation, or a nominal's
/// inner), an `Elem(i)` a tuple element. Returns `Ty::Any` on an unwalkable path (the caller then reads
/// arity 0 / declines). Enough for the nested-sum-switch subject: a nested switch's `path` ends at a sum
/// sub-value, so the walk reaches a `Ty::Sum` whose declaration names the variant.
/// Drop `Payload` steps that land on an ERASED NOMINAL newtype from a switch/bind path — the Rust-backend
/// twin of `lower::erase_nominal_steps` (which does this for the BODY's `SumPayload` read paths). A newtype
/// tag erases at runtime (the value IS the inner), so its `Payload` step is a no-op; keeping it in a switch
/// path would put the switch one level too shallow and mint bind paths one step deeper than the erased body
/// reads. Walking the scrutinee's TYPE, a `Payload` over a `Ty::Nominal` advances to its inner and is
/// dropped; a `Payload` over a real sum is KEPT (and the type advances through the sum's disc-0 payload,
/// enough to detect a nominal deeper in the path); an `Elem(i)` is kept (advancing through a tuple/list).
/// A boxed non-newtype path has no nominal `Payload` step, so it is returned unchanged (no regression).
fn erase_nominal_switch_path(
    db: &mut Db,
    scrutinee: StructId,
    sw_path: &[crate::core::PathStep],
) -> Vec<crate::core::PathStep> {
    let mut cur = type_of(db, scrutinee);
    let mut out = Vec::with_capacity(sw_path.len());
    for step in sw_path {
        match step {
            crate::core::PathStep::Payload => match &cur {
                // A nominal newtype's payload step erases — advance to the inner, drop the step.
                Ty::Nominal { inner, .. } => cur = (**inner).clone(),
                // A real (boxed-sum) payload step — keep it, advance through the sum's payload shape.
                _ => {
                    out.push(*step);
                    cur = sum_disc0_payload_ty(db, &cur).unwrap_or(Ty::Any);
                }
            },
            crate::core::PathStep::Elem(i) => {
                out.push(*step);
                cur = match cur.strip_nominal() {
                    Ty::Tuple(elems) => elems.get(*i).cloned().unwrap_or(Ty::Any),
                    Ty::List(elem) => (**elem).clone(),
                    _ => Ty::Any,
                };
            }
            // A list-rest step is not a nominal `Payload` (never erased) — keep it, type stays the list.
            crate::core::PathStep::RestFrom(_) => {
                out.push(*step);
                cur = match cur.strip_nominal() {
                    Ty::List(_) => cur.strip_nominal().clone(),
                    _ => Ty::Any,
                };
            }
        }
    }
    out
}

/// Resolve the solved type of the sub-value at `path` from the arm-recorded `sum_path_types` hints —
/// the entered-variant type an enclosing switch recorded when it descended. Longest-prefix match: find the
/// deepest recorded path that is a prefix of `path`, then walk the remaining `Elem`/nominal-`Payload`
/// steps from its type (a tuple-payload destructure). `None` if no recorded path is a prefix (the root, or
/// a genuinely un-hinted path) — the caller then falls back to a scrutinee-rooted type walk.
fn lookup_sum_path_type(ctx: &Ctx, path: &[crate::core::PathStep]) -> Option<Ty> {
    let (best_path, best_ty) = ctx
        .sum_path_types
        .iter()
        .filter(|(p, _)| path.starts_with(p))
        .max_by_key(|(p, _)| p.len())?;
    let rest = &path[best_path.len()..];
    let mut ty = best_ty.clone();
    for step in rest {
        ty = match step {
            crate::core::PathStep::Elem(i) => match ty.strip_nominal() {
                Ty::Tuple(elems) => elems.get(*i).cloned()?,
                Ty::List(elem) => (**elem).clone(),
                _ => return None,
            },
            // A nominal-newtype Payload peels a layer (a no-op); a sum Payload beyond a recorded hint only
            // arises through a nested switch, which records its OWN hint — so here it is not resolvable.
            crate::core::PathStep::Payload => match &ty {
                Ty::Nominal { inner, .. } => (**inner).clone(),
                _ => return None,
            },
            crate::core::PathStep::RestFrom(_) => return None,
        };
    }
    Some(ty)
}

/// The payload arity of variant `disc` of the sum TYPE `ty` — the type-keyed twin of
/// [`variant_payload_arity_at`], reading the arity off the (possibly hint-supplied) subject type rather
/// than re-walking from the scrutinee. `strip_nominal` first so an erased-newtype-wrapped sum reads the
/// inner sum's variant arity.
fn variant_arity_of_ty(db: &mut Db, ty: &Ty, disc: u32) -> usize {
    let decl_occ = match ty.strip_nominal() {
        Ty::Sum { decl, .. } => *decl,
        _ => return 0,
    };
    match db.type_decl_by_occ(decl_occ) {
        Some(decl) => decl
            .variants
            .get(disc as usize)
            .map(|v| v.payloads.len())
            .unwrap_or(0),
        None => 0,
    }
}

fn ty_at_sum_path(db: &mut Db, scrutinee: StructId, sw_path: &[crate::core::PathStep]) -> Ty {
    let mut ty = type_of(db, scrutinee);
    // A parallel CONSTANT-VALUE cursor: the `Core` node the sub-value currently is, when the scrutinee is
    // a compile-time-known value (a folded `SumNew`/`Tuple`). A `Payload` step over a `Ty::Sum` must
    // descend the ENTERED variant's payload — but the flattened path does not carry which discriminant the
    // enclosing arm selected. When the value is a constant `SumNew { disc }`, its `disc` IS the entered
    // variant, so read THAT variant's payload type (not variant 0's). This is what lets a nested match on a
    // variant at disc ≥ 1 (`(type W (A Int64) (V (Option Int64)))` matched `(W.V (Some n))`, folded to a
    // known `W.V`) resolve its inner switch's subject to `Option Int64` (V's payload), not `Int64` (A's).
    // A non-constant scrutinee falls back to variant 0 — a fully-runtime nested match on a disc-≥1 variant
    // is not reachable here (a sum-typed value can't cross the export boundary, so `f` folds or declines).
    let mut val: Option<Core> = Some(crate::lower::core_of(db, scrutinee));
    for step in sw_path {
        // The disc of the current constant value, if it is a `SumNew` — the entered variant at a `Payload`.
        let cur_disc = match &val {
            Some(Core::SumNew { disc, .. }) => Some(*disc),
            _ => None,
        };
        ty = match step {
            crate::core::PathStep::Payload => match ty.strip_nominal() {
                Ty::Sum { .. } => {
                    let disc = cur_disc.unwrap_or(0);
                    match variant_payload_ty(db, &ty, disc) {
                        Some(t) => t,
                        None => return Ty::Any,
                    }
                }
                Ty::Nominal { inner, .. } => (**inner).clone(),
                _ => return Ty::Any,
            },
            crate::core::PathStep::Elem(i) => match ty.strip_nominal() {
                Ty::Tuple(elems) => match elems.get(*i) {
                    Some(t) => t.clone(),
                    None => return Ty::Any,
                },
                Ty::List(elem) => (**elem).clone(),
                _ => return Ty::Any,
            },
            // A rest sublist keeps the list type (the Rust backend declines a runtime list match; total here).
            crate::core::PathStep::RestFrom(_) => match ty.strip_nominal() {
                Ty::List(_) => ty.clone(),
                _ => return Ty::Any,
            },
        };
        // Advance the value cursor alongside the type: a `Payload` enters a `SumNew`'s sole payload (a
        // multi-payload variant's payloads become the following `Elem`s), an `Elem` a `Tuple`/`SumNew`
        // element. Anything else drops the cursor to `None` (fall back to variant 0 / structural type).
        val = match (step, val.take()) {
            (crate::core::PathStep::Payload, Some(Core::SumNew { payloads, .. }))
                if payloads.len() == 1 =>
            {
                Some(crate::lower::core_of(db, payloads[0]))
            }
            (crate::core::PathStep::Elem(i), Some(Core::SumNew { payloads, .. })) => {
                payloads.get(*i).map(|&p| crate::lower::core_of(db, p))
            }
            (crate::core::PathStep::Elem(i), Some(Core::Tuple { elems })) => {
                elems.get(*i).map(|&e| crate::lower::core_of(db, e))
            }
            _ => None,
        };
    }
    ty
}

/// The payload ARITY of the `disc`-th variant of the sum the value at `id` has — how many payload types
/// the variant declares (0 = nullary). Read from the declaration's variant. A single-payload variant is
/// 1 (its payload may itself be a tuple); a multi-payload variant is its payload count. Used to decide
/// whether a match arm's pattern binds a payload (`(p)`) or not.
fn variant_payload_arity(db: &mut Db, id: StructId, disc: u32) -> usize {
    let decl_occ = match type_of(db, id) {
        Ty::Sum { decl, .. } => decl,
        _ => return 0,
    };
    match db.type_decl_by_occ(decl_occ) {
        Some(decl) => decl
            .variants
            .get(disc as usize)
            .map(|v| v.payloads.len())
            .unwrap_or(0),
        None => 0,
    }
}

/// Emit a `Core::SumPayload { scrutinee, path }` → the Rust identifier the enclosing sum-match arm bound
/// the payload to (looked up in `ctx.sum_binds` by `(scrutinee, path)`). A payload deeper than the
/// arm's direct payload — a `PathStep::Elem(i)` after the `Payload` (a tuple-payload destructure) —
/// reads a tuple field off that binding (`(<bound>).i`). Declines if no binding is in scope (a sum
/// pattern shape this slice does not yet render — e.g. a nested switch's payload).
/// Walk `path` through the CONSTANT value tree rooted at `root`, returning the single `Core` node it
/// selects — or `None` if the path lands between nodes (a multi-payload `Payload`, or a step over a
/// non-constant node). A `Payload` over a single-payload `SumNew` enters its sole payload (`(W.V x)` →
/// `x` — the disc-fold-flattened nested-match subject read, several `Payload`s deep); an `Elem(i)` indexes
/// a `SumNew`'s / `Tuple`'s / `ListNew`'s element. The value-tree twin of `lower::fold_sum_path`, but
/// returning the NODE (for `emit`) rather than its folded `Core`.
fn fold_const_sum_path(
    db: &mut Db,
    root: StructId,
    path: &[crate::core::PathStep],
) -> Option<StructId> {
    let mut cur = root;
    let mut i = 0;
    while i < path.len() {
        let step = &path[i];
        match (step, crate::lower::core_of(db, cur)) {
            (crate::core::PathStep::Payload, Core::SumNew { payloads, .. }) if payloads.len() == 1 => {
                cur = payloads[0];
                i += 1;
            }
            // A MULTI-payload variant's payload IS the tuple of its payloads (no single node). A following
            // `Elem(j)` indexes payload `j` DIRECTLY — consume BOTH steps. A bare `Payload` ending here has
            // no single node (`None` — the caller renders the payload tuple).
            (crate::core::PathStep::Payload, Core::SumNew { payloads, .. }) => {
                match path.get(i + 1) {
                    Some(crate::core::PathStep::Elem(j)) => {
                        cur = *payloads.get(*j)?;
                        i += 2;
                    }
                    _ => return None,
                }
            }
            (crate::core::PathStep::Elem(j), Core::Tuple { elems }) => {
                cur = *elems.get(*j)?;
                i += 1;
            }
            (crate::core::PathStep::Elem(j), Core::ListNew { elems }) => {
                cur = *elems.get(*j)?;
                i += 1;
            }
            // A `RestFrom`, or a step over a non-constant node.
            _ => return None,
        }
    }
    Some(cur)
}

fn emit_sum_payload(
    db: &mut Db,
    _id: StructId,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // CONSTANT-SCRUTINEE FOLD: the scrutinee is a compile-time `SumNew { disc, payloads }` (a constant
    // sum value, e.g. `(match (V.P 3 4) ((V.P a b) …))` where the front-end did NOT fold the match to the
    // arm body). Then the arm's payload binders read directly off the constant's payload NODES, no runtime
    // re-match: a `[Payload]` reads the sole payload (a single-payload variant) or the whole tuple (a
    // multi-payload variant — its payloads ARE the tuple), and a trailing `Elem(i)` reads payload `i`. This
    // is what lets a CONSTANT multi-payload match `(V.P a b)` render on Rust (the runtime-scrutinee case
    // already works via `emit_sum_match`'s binding, and a single-payload constant already folds in `lower`
    // to a bare value; only a constant MULTI-payload match reached here unresolved — the wasm backend emits
    // runtime `sum-payload`/`arr-get` reads of the constant, the Rust one folds to the payload node).
    if matches!(crate::lower::core_of(db, scrutinee), Core::SumNew { .. }) {
        // Walk the path through the CONSTANT value tree, descending each step into the node it selects. A
        // `Payload` over a single-payload `SumNew` enters its sole payload (a `(W.V …)` → the inner value —
        // this is what lets a disc-fold-FLATTENED nested match, whose single switch sits at a deep path like
        // `[Payload, Payload]` with NO enclosing binds, read its subject directly off the constant); a
        // `Payload` over a MULTI-payload variant yields the tuple of all payloads; an `Elem(i)` indexes a
        // tuple/multi-payload node. If the whole path resolves to a single node, emit it; a path that lands
        // "between" nodes (a multi-payload `Payload` not at the end) falls through to the bind lookup.
        if let Some(node) = fold_const_sum_path(db, scrutinee, path) {
            return emit(db, node, env, ctx);
        }
        // A `[…, Payload]` ending on a MULTI-payload variant is the tuple of its payloads (no single node).
        if let (Some((last, prefix)), _) = (path.split_last(), ()) {
            if matches!(last, crate::core::PathStep::Payload)
                && let Some(parent) = fold_const_sum_path(db, scrutinee, prefix)
                && let Core::SumNew { payloads, .. } = crate::lower::core_of(db, parent)
                && payloads.len() != 1
            {
                let mut parts = Vec::with_capacity(payloads.len());
                for &p in &payloads {
                    parts.push(emit(db, p, env, ctx)?);
                }
                return Ok(format!("({})", parts.join(", ")));
            }
        }
    }
    // The binding covers the arm's direct payload (path prefix `[Payload]`); any trailing `Elem(i)` steps
    // index into it (a tuple payload). Find a bind whose path is a prefix of `path`.
    for b in ctx.sum_binds.iter().rev() {
        if b.scrutinee == scrutinee && path.starts_with(&b.path) {
            let rest = &path[b.path.len()..];
            // A BOXED bind (a recursive variant's `Box<…>` field) is DEREFERENCED to reach the payload —
            // `(*name)` — the twin of the construct site's `Box::new`. An `Elem(i)` then indexes the
            // deref'd tuple (`(*name).i`); the whole payload is `(*name)`. A non-boxed bind reads `name`
            // directly (Rust auto-derefs a `Box` for a field access, but the explicit `*` is uniform and
            // correct whether the following step is a field index or the value itself).
            let mut expr = if b.boxed {
                format!("(*{})", b.name)
            } else {
                b.name.clone()
            };
            for step in rest {
                match step {
                    crate::core::PathStep::Elem(i) => expr = format!("({expr}).{i}"),
                    crate::core::PathStep::Payload => {
                        return Err(Reject::decline(
                            "a nested sum payload is not yet rendered by the Rust backend",
                        ));
                    }
                    crate::core::PathStep::RestFrom(_) => {
                        return Err(Reject::decline(
                            "a list rest binder is not yet rendered by the Rust backend",
                        ));
                    }
                }
            }
            return Ok(expr);
        }
    }
    Err(Reject::decline(
        "sum payload has no bound match arm (unsupported pattern shape)",
    ))
}

/// Emit `Option.expect`/`Result.expect` → `match <scrut> { <Enum>::<Present>(p) => p, _ => panic!("…") }`.
/// The present variant is `disc_present` (Some/Ok = 0), which carries exactly one payload (the shape the
/// `expect` field is added for); its binding IS the expression's value. Any other variant panics — a Rust
/// panic is a Cadenza trap, the native mirror of the wasm `unreachable` (core-semantics.md §Requiring The
/// Value Of An Optional Traps On Absence). The scrutinee is pure (param/local/call), so matching it inline
/// evaluates it once, observably as the wasm path's single materialization.
fn emit_sum_expect(
    db: &mut Db,
    scrutinee: StructId,
    disc_present: u32,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    let vpath = sum_variant_path(db, scrutinee, disc_present)?;
    if variant_payload_arity(db, scrutinee, disc_present) != 1 {
        return Err(Reject::decline(
            "expect's present variant does not carry exactly one payload",
        ));
    }
    let scrut = emit(db, scrutinee, env, ctx)?;
    // The payload binds to `__expect` and is the match's value directly (a single-payload present arm).
    Ok(format!(
        "match {scrut} {{ {vpath}(__expect) => __expect, _ => panic!(\"expect\") }}"
    ))
}

/// Emit an `if`/`match` branch producing the construct at `construct_id`'s RESULT type. When that
/// result is an integer, a bare-literal branch is GROUNDED to its width (via [`emit_grounded`]) so a
/// default-Int64 literal branch opposite a narrow branch does not mismatch the block's type; a
/// non-integer result (e.g. Bool branches) emits normally. Mirrors the wasm backend's `emit_branch`.
fn emit_branch(
    db: &mut Db,
    branch: StructId,
    construct_id: StructId,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    match type_of(db, construct_id) {
        Ty::Int(it) => emit_grounded(db, branch, it, env, ctx),
        _ => emit(db, branch, env, ctx),
    }
}

/// The shared integer type of a comparison's two operands — the width/signedness both must be rendered
/// at. A bare literal defaults to `Int64`, so the DEFINITE side (the non-literal operand) supplies the
/// real width: prefer whichever operand has a concrete `Ty::Int`. `None` when neither is an integer (a
/// Bool comparison — no width to reconcile, the operands emit as-is). Mirrors `select.rs`'s
/// `operand_int_ty`, but returns `None` for the non-integer case rather than a Bool-as-i32 stand-in
/// (Rust compares `bool` with `==` directly, needing no width).
fn operand_int_ty(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<IntTy> {
    // Prefer the operand whose type is NOT a bare defaulted literal: a param/computed operand carries
    // the real width, a literal defaults. Both concrete-and-equal is the common case; if one is a
    // literal (deferred width) the other pins the width through unify, so either read gives the same
    // ground width — but reading the non-literal side first is robust to the literal's default.
    let pick = |id: StructId, db: &mut Db| match type_of(db, id) {
        Ty::Int(it) => Some(it),
        _ => None,
    };
    // If lhs is a literal and rhs is definite (or vice versa), take the definite side.
    let lhs_lit = matches!(core_of(db, lhs), Core::ConstInt(_));
    if lhs_lit {
        pick(rhs, db).or_else(|| pick(lhs, db))
    } else {
        pick(lhs, db).or_else(|| pick(rhs, db))
    }
}

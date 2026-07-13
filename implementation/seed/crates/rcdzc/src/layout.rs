//! `layout` — the query that computes the program's boundary surface, target-neutrally.
//!
//! Above the backend seam sits the computation of *what the program presents at its boundary*: which
//! definitions are exported, under what (verbatim) names, with what solved parameter and result
//! types, and which definitions are reachable — computed ONCE and consumed by whichever backend runs
//! (`backends-and-targets.md` §The Boundary Layout Is Computed Once, Target-Neutrally, And Reused).
//!
//! The interface is read from each export's DECLARED signature, never inferred from a body
//! (`reference-compiler.md` §The Exported Interface Is The Declared Signature). An export's signature
//! is its solved parameter types (each `(name-occurrence, type)`) and result type, obtained by
//! demanding `type_of`/`def_scheme` (a lazy read of the type column). The export NAME crosses
//! verbatim; no export is recognized by name or body shape.
//!
//! Reachability lives here: an export drives which definitions are reached, and only reachable
//! definitions are emitted (dead code dropped once, target-neutrally). A runtime `Core::Call` reaches
//! its callee, so the reachable set is grown by a worklist over each reachable body's calls (not just
//! the exports — a recursive or helper def a call names is emitted too). This is the place that set,
//! the emission order, and each function's absolute index are fixed for every backend.

use crate::ast::StructId;
use crate::db::Db;
use crate::diag::Reject;
use crate::infer::type_of;
use crate::ty::Ty;
use tracing::trace;

/// One exported entry, resolved to a target-neutral boundary plan.
#[derive(Clone, PartialEq, Debug)]
pub struct ExportPlan {
    /// The name the entry crosses the boundary under — verbatim from the source.
    pub name: String,
    /// The definition (index into `db.defs`) this export names.
    pub def: usize,
    /// The AST occurrence of the definition body — the root the backend walks the core from.
    pub body: StructId,
    /// The parameters, in signature order — each the `(name-occurrence, solved-type)` the backend
    /// needs to assign a local slot and a boundary valtype (`select_function`). Empty for a nullary
    /// export. The name occurrence is what a body reference to the parameter binds to (seen through a
    /// `(: a T)` annotated binder), so it is the slot-map key.
    pub params: Vec<(StructId, Ty)>,
    /// The solved result type the entry returns.
    pub result: Ty,
}

/// The whole boundary layout: the exported entries (declaration order), the definitions reachable
/// from them in emission order, and each reachable definition's absolute wasm-function index.
#[derive(Clone, PartialEq, Debug)]
pub struct Layout {
    pub exports: Vec<ExportPlan>,
    /// Reachable definition indices in emission order (exported first, declaration order; then the
    /// rest). A body emitted at position `k` is DEFINED wasm func `import_base + k` — runtime imports
    /// occupy the function index space `0..import_base` ahead of every defined function.
    pub order: Vec<usize>,
    /// The number of runtime-op imports the program declares — the offset added to a defined
    /// function's emission position to get its absolute wasm function index. `0` for a program that
    /// imports nothing (a scalar program), which is then byte-identical to a runtime-free build.
    pub import_base: u32,
    /// `def → its position in `order``, the O(1) inverse of the `order` sequence. `abs` (called once
    /// per `Core::Call` during selection AND per export during serialization) needs a def's emission
    /// position; without this map it did an O(len) `order.position()` scan, making a call-heavy or
    /// export-heavy program O(N²) in the backend. Built once alongside `order` by [`Layout::new`], so
    /// it cannot drift; the backend's `import_base` reshuffle preserves it via [`Layout::with_import_base`].
    order_pos: std::collections::HashMap<usize, usize>,
    /// `def → its index in `exports``, for the def→ExportPlan lookup the emit loop does once per
    /// emitted function (an export's params come from its plan). Without it that was an O(exports)
    /// `exports.iter().find()` per func — O(N²) on a many-export program. `None`-absent means the def
    /// is not an export (an internal reachable callee), which reads its params via `def_params`.
    export_of_def: std::collections::HashMap<usize, usize>,
    /// The LAMBDA-LIFTED closures, in funcref-table-slot order (a copy of `db.lifted` at layout time).
    /// Each lifted lambda is emitted as a standalone wasm function AFTER the `order` def functions, so
    /// its wasm function index is `import_base + order.len() + its slot`. The funcref table's element `k`
    /// points at lifted lambda `k`'s function, so a `Core::Closure { code: k }` stored slot selects it
    /// through `call_indirect`. Empty for a program with no runtime closure (byte-identical to before).
    pub lifted: Vec<crate::lower::LiftedLambda>,
    /// Parallel to `lifted`: whether slot `k` is REACHED by a `Core::Closure` in an emitted body. An
    /// UNREACHED slot (a lambda demanded during type-checking / a fold that erased it) is emitted as an
    /// inert STUB and gets NO funcref-table element (never called), so a dead lift is neither unsound nor
    /// referenced. `true` for every slot of a program whose closures are all live.
    pub lifted_reached: Vec<bool>,
}

impl Layout {
    /// Assemble a `Layout` from its emission plan, deriving the two O(1) inverse indices (`order_pos`,
    /// `export_of_def`) so they can never drift from `order`/`exports`. The one way to build a `Layout`
    /// — `compute` and the backend's `import_base` reshuffle both go through it — so the indices are a
    /// maintained invariant, not a field a caller could forget or set inconsistently.
    pub fn new(exports: Vec<ExportPlan>, order: Vec<usize>, import_base: u32) -> Layout {
        Layout::with_lifted(exports, order, import_base, Vec::new(), Vec::new())
    }

    /// [`Layout::new`] plus the lambda-lifted closures (in table-slot order) + a parallel `reached` flag
    /// per slot — the emission plan when a program has runtime closures. The lifted functions emit after
    /// the `order` defs, so their wasm indices are `import_base + order.len() + slot`.
    pub fn with_lifted(
        exports: Vec<ExportPlan>,
        order: Vec<usize>,
        import_base: u32,
        lifted: Vec<crate::lower::LiftedLambda>,
        lifted_reached: Vec<bool>,
    ) -> Layout {
        let order_pos = order.iter().enumerate().map(|(k, &d)| (d, k)).collect();
        let export_of_def = exports
            .iter()
            .enumerate()
            .map(|(i, e)| (e.def, i))
            .collect();
        Layout {
            exports,
            order,
            import_base,
            order_pos,
            export_of_def,
            lifted,
            lifted_reached,
        }
    }

    /// A copy of this layout with a different `import_base` (the backend shifts the base once the
    /// runtime-import count is known). The inverse indices are unchanged by the shift, so they carry
    /// over without a rebuild.
    pub fn with_import_base(&self, import_base: u32) -> Layout {
        Layout {
            import_base,
            ..self.clone()
        }
    }

    /// The absolute wasm-function index of definition `def`, or `None` if it is not emitted. Imports
    /// occupy `0..import_base`, so a defined function's index is `import_base + its position in order`.
    /// O(1) via the `order_pos` index (the emission-position map built in `compute`).
    pub fn abs(&self, def: usize) -> Option<u32> {
        self.order_pos
            .get(&def)
            .map(|&k| self.import_base + k as u32)
    }

    /// The [`ExportPlan`] for definition `def`, or `None` if `def` is not an export — an O(1) lookup
    /// (via `export_of_def`) replacing an `exports.iter().find(|e| e.def == def)` scan.
    pub fn export_plan(&self, def: usize) -> Option<&ExportPlan> {
        self.export_of_def.get(&def).map(|&i| &self.exports[i])
    }

    /// The absolute wasm-function index of lambda-lifted closure `slot` — the lifted functions emit
    /// AFTER the `order` defs, so lifted `slot` is wasm func `import_base + order.len() + slot`. This is
    /// what the funcref-table element section points at for table slot `slot`.
    pub fn lifted_abs(&self, slot: usize) -> u32 {
        self.import_base + (self.order.len() + slot) as u32
    }

    /// The TYPE-section index of lambda-lifted closure `slot`'s functype — the functypes are laid
    /// imports first, then `order` defs, then lifted lambdas, so lifted `slot`'s type index is
    /// `import_count + order.len() + slot`. A `call_indirect` applying a closure of that signature
    /// references this type. (Structural functypes: any type index with the matching `(param)->result`
    /// signature validates; using the lifted function's own type keeps it exact.)
    pub fn lifted_type_index(&self, slot: usize, import_count: u32) -> u32 {
        import_count + (self.order.len() + slot) as u32
    }
}

/// Compute the boundary layout for the program in `db` — a query the "compile" request drives. Demands
/// each export's result type (a lazy `type_of`); touches only the exported/reachable definitions.
/// A program with no export is rejected: nothing is public, so there is nothing to emit.
pub fn compute(db: &mut Db) -> Result<Layout, Reject> {
    if db.exports.is_empty() {
        return Err(Reject::decline("no `(export …)`: nothing is public"));
    }

    // Resolve each export to a plan by its DECLARED signature. An export naming no definition declines.
    let mut exports: Vec<ExportPlan> = Vec::new();
    for i in 0..db.exports.len() {
        let name = db.exports[i].name.clone();
        let def = match db.exports[i].def {
            Some(d) => d,
            None => {
                return Err(Reject::decline(format!(
                    "export `{name}` names no definition"
                )));
            }
        };
        let body = match db.defs[def].body {
            Some(b) => b,
            None => {
                return Err(Reject::decline(format!(
                    "export `{name}`: definition has no body"
                )));
            }
        };
        // The parameters — each `(name-occurrence, solved-type)`. An exported parameter needs a
        // DEFINITE type: its type is solved by demanding `type_of` on its binder, which is the
        // annotation type for an annotated param and `Any` for an unannotated one. An unannotated
        // (ambiguous) parameter has no machine width, so it DECLINES asking for an annotation — the
        // no-implicit-width rule (the backend can't pick a width the program didn't ask for).
        let params = export_params(db, def, &name)?;
        // The result type is the entry body's solved type — a lazy read of the type column.
        let result = type_of(db, body);
        trace!(target: "rcdzc::layout", %name, def, params = params.len(), result = %result.render_name(), "export plan");
        exports.push(ExportPlan {
            name,
            def,
            body,
            params,
            result,
        });
    }

    // Emission order: exported definitions first (declaration order, deduplicated), then every
    // definition REACHABLE from them through a runtime `Core::Call` — a recursive callee, or a callee
    // that a recursive function reaches. A worklist closes the reachable set: for each def in `order`,
    // lower its body and append any `Core::Call` callee not already present. (Non-recursive calls
    // inline, so they add nothing here — only a `Core::Call` grows the set.)
    // `order` keeps the emission SEQUENCE (exports first, then reachable callees); `in_order` is the
    // O(1) membership check that goes with it. A plain `order.contains(&x)` here is an O(len) scan, and
    // it runs once per export AND once per discovered callee — O(N²) on a program with many exports or
    // a wide call fan-out (a 3200-export program spent ~all its layout time in these scans + the Vec
    // regrowth they drove). The set keeps each "already queued?" test O(1).
    let mut order: Vec<usize> = Vec::new();
    let mut in_order: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for e in &exports {
        if in_order.insert(e.def) {
            order.push(e.def);
        }
    }
    let mut i = 0;
    while i < order.len() {
        let def = order[i];
        if let Some(body) = db.defs[def].body {
            let mut callees = Vec::new();
            collect_call_callees(db, body, &mut callees);
            for c in callees {
                if in_order.insert(c) {
                    trace!(target: "rcdzc::layout", def = c, "reachable via a runtime call — added to emission order");
                    order.push(c);
                }
            }
        }
        i += 1;
    }

    // LAMBDA-LIFTED closures: lowering the def bodies above (via `collect_call_callees` → `core_of`)
    // registers each surviving `(fn …)` into `db.lifted` (a `Core::Closure` naming its table slot). But
    // `db.lifted` accumulates EVERY lambda `lower_lambda_value` touched — including one demanded during
    // type-checking / fold exploration that the final emitted code FOLDS AWAY (a constant closure applied
    // immediately). Emitting such a DEAD lift is both wasteful and unsound (its body may read captures
    // from an env no reachable `Core::Closure` ever builds). So collect only the lifted lambdas REACHED
    // by a `Core::Closure { code }` in an EMITTED body (the reachable defs' bodies), transitively (a
    // reached lambda's body may itself build a closure). This is the closure analogue of the
    // `Core::Call` reachability above.
    let mut reached_codes: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // Seed from every reachable def body.
    for &def in &order {
        if let Some(body) = db.defs[def].body {
            collect_closure_codes(db, body, &mut reached_codes);
        }
    }
    // Transitively close: a reached lambda's body may build further closures AND call further defs.
    let mut work: Vec<usize> = reached_codes.iter().copied().collect();
    while let Some(code) = work.pop() {
        let body = db.lifted[code].body;
        let mut more = std::collections::HashSet::new();
        collect_closure_codes(db, body, &mut more);
        for c in more {
            if reached_codes.insert(c) {
                work.push(c);
            }
        }
        // A reached lifted body's own `Core::Call` callees must be emitted too.
        let mut callees = Vec::new();
        collect_call_callees(db, body, &mut callees);
        for c in callees {
            if in_order.insert(c) {
                trace!(target: "rcdzc::layout", def = c, "reachable via a lifted closure body — added to emission order");
                order.push(c);
            }
        }
    }
    // The lifted set snapshotted in table-slot order. `reached` marks which slots a reachable
    // `Core::Closure` actually builds — an UNREACHED slot (a lambda demanded during type-checking / fold
    // exploration that the emitted code folds away) is emitted as an inert STUB and its table entry left
    // out (never called), so a dead lift is neither unsound nor referenced.
    let lifted = db.lifted.clone();
    let lifted_reached: Vec<bool> = (0..lifted.len())
        .map(|code| reached_codes.contains(&code))
        .collect();

    // `import_base` is 0 until a program uses a runtime op: the per-program runtime-import set is
    // computed by the backend when a `Core` compound op lowers to a heap call (value-heap H2). A
    // program that imports nothing keeps base 0 and is byte-identical to a runtime-free build.
    Ok(Layout::with_lifted(
        exports,
        order,
        0,
        lifted,
        lifted_reached,
    ))
}

/// Collect the funcref-table slots (`Core::Closure { code }`) a body BUILDS, into `out` — the closure
/// analogue of [`collect_call_callees`]. A closure value reaching a reachable body means its lifted
/// function is genuinely used (so it must be emitted with a real body + its table entry); a lambda in
/// `db.lifted` NOT reached this way was demanded only during type-checking / a fold that erased it, so it
/// is a dead lift. Descends every sub-position (both `if` branches, arm bodies, operands) like the call
/// walk. A `Core::CallClosure` dispatches dynamically (no static code), so it adds no slot itself.
fn collect_closure_codes(db: &mut Db, id: StructId, out: &mut std::collections::HashSet<usize>) {
    use crate::core::Core;
    match crate::lower::core_of(db, id) {
        Core::Closure { code, captures } => {
            out.insert(code);
            for c in captures {
                collect_closure_codes(db, c, out);
            }
        }
        Core::CallClosure { closure, args } => {
            collect_closure_codes(db, closure, out);
            for arg in args {
                collect_closure_codes(db, arg, out);
            }
        }
        Core::If { cond, then_, else_ } => {
            collect_closure_codes(db, cond, out);
            collect_closure_codes(db, then_, out);
            collect_closure_codes(db, else_, out);
        }
        Core::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_closure_codes(db, value, out);
            }
            collect_closure_codes(db, body, out);
        }
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs }
        | Core::And { lhs, rhs, .. }
        | Core::ListConcat { lhs, rhs }
        | Core::BytesConcat { lhs, rhs } => {
            collect_closure_codes(db, lhs, out);
            collect_closure_codes(db, rhs, out);
        }
        Core::ListPush { list, elem } => {
            collect_closure_codes(db, list, out);
            collect_closure_codes(db, elem, out);
        }
        Core::ListUpdate { list, index, elem } => {
            collect_closure_codes(db, list, out);
            collect_closure_codes(db, index, out);
            collect_closure_codes(db, elem, out);
        }
        Core::ListAt { list, index, .. } => {
            collect_closure_codes(db, list, out);
            collect_closure_codes(db, index, out);
        }
        Core::BytesAt { bytes, index, .. } => {
            collect_closure_codes(db, bytes, out);
            collect_closure_codes(db, index, out);
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            collect_closure_codes(db, bytes, out);
            collect_closure_codes(db, start, out);
            collect_closure_codes(db, len, out);
        }
        Core::BytesCompact { operand }
        | Core::Convert { operand, .. }
        | Core::Not { operand }
        | Core::ListLen { operand }
        | Core::BytesLen { operand } => collect_closure_codes(db, operand, out),
        Core::Call { args, .. } | Core::HostCall { args, .. } => {
            for a in args {
                collect_closure_codes(db, a, out);
            }
        }
        Core::Match { scrutinee, arms } => {
            collect_closure_codes(db, scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_closure_codes(db, g, out);
                }
                collect_closure_codes(db, arm.body, out);
            }
        }
        Core::Record { fields } => {
            for value in fields.values() {
                collect_closure_codes(db, *value, out);
            }
        }
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            for e in elems {
                collect_closure_codes(db, e, out);
            }
        }
        Core::Proj { operand, .. } => collect_closure_codes(db, operand, out),
        Core::SumNew { payloads, .. } => {
            for p in payloads {
                collect_closure_codes(db, p, out);
            }
        }
        Core::MatchSum { scrutinee, root } => {
            collect_closure_codes(db, scrutinee, out);
            collect_cont_closure_codes(db, &root, out);
        }
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => {
            collect_closure_codes(db, scrutinee, out)
        }
        // Leaves / references build no closure.
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstFloat(_)
        | Core::Unit
        | Core::Param { .. }
        | Core::Captured { .. }
        | Core::LocalRef { .. }
        | Core::Poison(_) => {}
    }
}

/// The closure-slot analogue of `collect_cont_callees` — walk a sum-match continuation for the closures
/// its arm bodies build.
fn collect_cont_closure_codes(
    db: &mut Db,
    cont: &crate::core::SumCont,
    out: &mut std::collections::HashSet<usize>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => collect_closure_codes(db, *body, out),
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_closure_codes(db, *cond, out);
            collect_closure_codes(db, *body, out);
            collect_cont_closure_codes(db, els, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_cont_closure_codes(db, then_, out);
            collect_cont_closure_codes(db, els, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                collect_cont_closure_codes(db, &arm.cont, out);
            }
        }
    }
}

/// Collect the `db.defs` indices a body CALLS at runtime — the `Core::Call` callees reached from the
/// core form at `id`, descending through every sub-position (both `if` branches are reachable code, so
/// a callee in either counts). Reads the core column on demand. A callee's OWN calls are found when it
/// is itself expanded from the worklist, so this walk does not recurse into a callee's body.
fn collect_call_callees(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
    match crate::lower::core_of(db, id) {
        crate::core::Core::Call { callee, args } => {
            if !out.contains(&callee) {
                out.push(callee);
            }
            for a in args {
                collect_call_callees(db, a, out);
            }
        }
        crate::core::Core::If { cond, then_, else_ } => {
            collect_call_callees(db, cond, out);
            collect_call_callees(db, then_, out);
            collect_call_callees(db, else_, out);
        }
        crate::core::Core::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_call_callees(db, value, out);
            }
            collect_call_callees(db, body, out);
        }
        crate::core::Core::Arith { lhs, rhs, .. }
        | crate::core::Core::Compare { lhs, rhs, .. }
        | crate::core::Core::ValueEq { lhs, rhs }
        | crate::core::Core::And { lhs, rhs, .. }
        | crate::core::Core::ListConcat { lhs, rhs } => {
            collect_call_callees(db, lhs, out);
            collect_call_callees(db, rhs, out);
        }
        crate::core::Core::ListPush { list, elem } => {
            collect_call_callees(db, list, out);
            collect_call_callees(db, elem, out);
        }
        crate::core::Core::ListUpdate { list, index, elem } => {
            collect_call_callees(db, list, out);
            collect_call_callees(db, index, out);
            collect_call_callees(db, elem, out);
        }
        crate::core::Core::ListAt { list, index, .. } => {
            collect_call_callees(db, list, out);
            collect_call_callees(db, index, out);
        }
        crate::core::Core::BytesAt { bytes, index, .. } => {
            collect_call_callees(db, bytes, out);
            collect_call_callees(db, index, out);
        }
        crate::core::Core::BytesConcat { lhs, rhs } => {
            collect_call_callees(db, lhs, out);
            collect_call_callees(db, rhs, out);
        }
        crate::core::Core::BytesSlice {
            bytes, start, len, ..
        } => {
            collect_call_callees(db, bytes, out);
            collect_call_callees(db, start, out);
            collect_call_callees(db, len, out);
        }
        crate::core::Core::BytesCompact { operand } => collect_call_callees(db, operand, out),
        crate::core::Core::Convert { operand, .. } | crate::core::Core::Not { operand } => {
            collect_call_callees(db, operand, out)
        }
        crate::core::Core::Match { scrutinee, arms } => {
            collect_call_callees(db, scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_call_callees(db, g, out);
                }
                collect_call_callees(db, arm.body, out);
            }
        }
        crate::core::Core::Record { fields } => {
            for value in fields.values() {
                collect_call_callees(db, *value, out);
            }
        }
        crate::core::Core::Tuple { elems }
        | crate::core::Core::ListNew { elems }
        | crate::core::Core::BytesOf { elems } => {
            for e in elems {
                collect_call_callees(db, e, out);
            }
        }
        crate::core::Core::Proj { operand, .. }
        | crate::core::Core::ListLen { operand }
        | crate::core::Core::BytesLen { operand } => collect_call_callees(db, operand, out),
        // A sum construction's payloads are unconditionally evaluated — descend for their calls.
        crate::core::Core::SumNew { payloads, .. } => {
            for p in payloads {
                collect_call_callees(db, p, out);
            }
        }
        // A sum match: the scrutinee + every arm's continuation are reachable code (a self-call in an arm
        // is a recursion edge, like an `if` branch). A nested switch's arms recurse. A sum-payload read
        // evaluates the scrutinee.
        crate::core::Core::MatchSum { scrutinee, root } => {
            collect_call_callees(db, scrutinee, out);
            collect_cont_callees(db, &root, out);
        }
        crate::core::Core::SumPayload { scrutinee, .. } => collect_call_callees(db, scrutinee, out),
        // `expect` evaluates its scrutinee (which may CALL — a `checked-add` composes here); the trap path
        // calls nothing.
        crate::core::Core::SumExpect { scrutinee, .. } => collect_call_callees(db, scrutinee, out),
        // A closure's captured values are unconditionally evaluated at construction — descend for their
        // calls. The lifted function's OWN body is reached via the lifted-def worklist (a lifted lambda is
        // a synthetic def added to the emission set separately), not here.
        crate::core::Core::Closure { captures, .. } => {
            for c in captures {
                collect_call_callees(db, c, out);
            }
        }
        // A closure application evaluates the closure value and its arguments; the callee is dynamic
        // (`call_indirect`), so no static callee to add — the lifted functions are already in the set.
        crate::core::Core::CallClosure { closure, args } => {
            collect_call_callees(db, closure, out);
            for arg in args {
                collect_call_callees(db, arg, out);
            }
        }
        // A host call dispatches to a component IMPORT (not a `db.defs` function), so no static callee to
        // add; its arguments may still reach callees.
        crate::core::Core::HostCall { args, .. } => {
            for arg in args {
                collect_call_callees(db, arg, out);
            }
        }
        // Leaves and references have no sub-calls (a `Captured` read is a heap read of the env cell).
        crate::core::Core::ConstInt(_)
        | crate::core::Core::ConstBool(_)
        | crate::core::Core::ConstStr(_)
        | crate::core::Core::ConstFloat(_)
        | crate::core::Core::Unit
        | crate::core::Core::Param { .. }
        | crate::core::Core::Captured { .. }
        | crate::core::Core::LocalRef { .. }
        | crate::core::Core::Poison(_) => {}
    }
}

/// Collect the callees reachable through a sum-match CONTINUATION — a leaf's body, or a nested switch's
/// arms (each recursing). Mirrors the `MatchSum` arm walk so a self-call at any tree depth is a recursion
/// edge (the `Payload`/`Elem` steps are heap reads, no calls).
fn collect_cont_callees(db: &mut Db, cont: &crate::core::SumCont, out: &mut Vec<usize>) {
    match cont {
        crate::core::SumCont::Leaf(body) => collect_call_callees(db, *body, out),
        // A guarded arm reaches callees through its guard cond, its body, AND the fall-through.
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_call_callees(db, *cond, out);
            collect_call_callees(db, *body, out);
            collect_cont_callees(db, els, out);
        }
        // A literal test reaches callees through both continuations (the `path` walk has no calls).
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_cont_callees(db, then_, out);
            collect_cont_callees(db, els, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                collect_cont_callees(db, &arm.cont, out);
            }
        }
    }
}

/// The parameters of definition `def` for INTERNAL emission — each `(name-occurrence, solved-type)`,
/// in signature order. Same as [`export_params`] but WITHOUT the boundary-representability decline: an
/// internal (non-exported) callee's parameters need only a CORE machine valtype (i32/i64), not a
/// component-boundary primitive, so a width that could not cross the boundary is still fine for a local
/// call. The name occurrence is the slot-map key (seen through a `(: a T)` annotated binder). Used by
/// the backend to select a reachable non-export function (a recursive callee) with its own local slots.
pub fn def_params(db: &mut Db, def: usize) -> Vec<(StructId, Ty)> {
    let sig_params = db.defs[def].params.clone();
    let mut out = Vec::new();
    for p in sig_params {
        let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => p,
        };
        let ty = type_of(db, binder);
        out.push((binder, ty));
    }
    out
}

/// The exported parameters of definition `def` — each `(name-occurrence, solved-type)`, in signature
/// order. The name occurrence is what a body reference binds to (through a `(: a T)` binder); its type
/// is solved by `type_of` on that occurrence (the annotation type, or `Any` if unannotated). An
/// exported parameter with NO definite scalar type (an unannotated/ambiguous one, whose type has no
/// machine representation) DECLINES asking for an annotation — the backend must not invent a width the
/// program did not write (`numeric-model.md` no implicit width; the operator's "ambiguous params
/// require annotations").
fn export_params(db: &mut Db, def: usize, name: &str) -> Result<Vec<(StructId, Ty)>, Reject> {
    let sig_params = db.defs[def].params.clone();
    let mut out = Vec::new();
    for p in sig_params {
        // The name occurrence — bare `a`, or the inner name of an annotated binder `(: a T)`.
        let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => p,
        };
        let ty = type_of(db, binder);
        // A parameter must have a machine representation to cross the boundary. `Any` (an unannotated
        // param whose type nothing fixed) has none — decline, pointing at the annotation it needs.
        if crate::backend::wasm::lir::valtype_of(&ty).is_none() {
            trace!(target: "rcdzc::layout", %name, binder = binder.0, ty = %ty.render_name(), "decline: exported parameter type is ambiguous (needs annotation)");
            return Err(Reject::decline(format!(
                "export `{name}`: parameter type is ambiguous — annotate it, e.g. `(: p Int64)`"
            ))
            .at(binder));
        }
        out.push((binder, ty));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::scalar_program;

    #[test]
    fn one_export_by_signature() {
        let (ast, _) = scalar_program();
        let mut db = Db::load(ast);
        let layout = compute(&mut db).expect("layout");
        assert_eq!(layout.exports.len(), 1);
        assert_eq!(layout.exports[0].name, "main");
        assert!(layout.exports[0].params.is_empty());
        assert!(layout.exports[0].result.agrees_with(&Ty::int64()));
        // The single exported definition is wasm func 0.
        assert_eq!(layout.order, vec![0]);
        assert_eq!(layout.abs(0), Some(0));
    }

    #[test]
    fn a_recursive_callee_is_reachable_past_the_exports() {
        // `main` (def 0, the export) calls `sum-to` (def 1) — a recursive callee reached by a runtime
        // `Core::Call`. Reachability must ADD `sum-to` to the emission order after the export, and its
        // absolute index (1) is what a `call` from `main` targets.
        let ast = crate::testkit::parse(
            "(module m (def (main) (sum-to 3)) (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = compute(&mut db).expect("layout");
        let main = db.def_by_name("main").expect("main");
        let sum_to = db.def_by_name("sum-to").expect("sum-to");
        // Both emitted; the export is first (index 0), the reachable callee second.
        assert_eq!(layout.order, vec![main, sum_to]);
        assert_eq!(layout.abs(main), Some(0));
        assert_eq!(layout.abs(sum_to), Some(1));
    }

    #[test]
    fn an_uncalled_def_is_not_reachable() {
        // A def neither exported nor called is dead — it does NOT enter the emission order.
        let ast = crate::testkit::parse(
            "(module m (def (main) 42) (def (unused (: n Int64)) (+ n 1)) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = compute(&mut db).expect("layout");
        let main = db.def_by_name("main").expect("main");
        assert_eq!(layout.order, vec![main]);
        assert_eq!(layout.abs(db.def_by_name("unused").unwrap()), None);
    }

    #[test]
    fn no_export_declines() {
        // A program with a def but no export presents nothing — layout declines.
        use crate::ast::{Builder, IntValue, Leaf, Radix};
        let mut b = Builder::new();
        let module = b.name("module");
        let m = b.name("m");
        let def = b.name("def");
        let sig = {
            let main = b.name("main");
            b.list(vec![main])
        };
        let body = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(42),
            radix: Radix::Dec,
        });
        let def_form = b.list(vec![def, sig, body]);
        let root = b.list(vec![module, m, def_form]);
        let mut db = Db::load(b.finish(root));
        assert!(compute(&mut db).is_err());
    }
}

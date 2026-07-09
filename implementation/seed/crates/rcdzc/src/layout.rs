//! The module layout — decides, ONCE (after inference, before selection), the component-boundary
//! surface (which functions are exported and under what boundary name / signature ABI) and where
//! each user function lands in the final wasm function-index space.
//!
//! This is what lets `Lir::Call` carry an ABSOLUTE index (no remap deferred to serialization, no
//! import-vs-user ambiguity): each user function's wasm index is fixed here, before selection.
//!
//! ## Multi-export, no single-entry assumption
//! A module has ONE OR MORE `(export …)` items (modules-and-namespaces.md §Visibility Is Explicit).
//! `Layout` carries EVERY export (never a `find(nullary)`-and-drop-the-rest shortcut), each with its
//! SIGNATURE — the ABI *is* the signature (`params`/`ret`), read generically at assembly, never a
//! name/body heuristic and never a pre-classified `Run`/`Compile` enum (that would re-encode a
//! particular signature one level up). The boundary NAME is the source name, VERBATIM — the compiler
//! never renames an export (a `main`→`run` rewrite would be exactly the name magic this design
//! forbids). A consumer wanting a conventional entry name exports the item under that name in source.
//!
//! ## Function-index base
//! A module that touches the value heap anywhere (constructs/projects a compound) imports the 42 heap
//! ops and defines 3 fixed helpers first, so its user functions start at `RT_FUNC_BASE` (=45). A
//! module that never touches the heap imports nothing; its user functions start at 0.

use crate::heap::RT_FUNC_BASE;
use crate::ir::{Mir, MirModule};
use crate::ty::Ty;

/// One component-boundary export: the user function it exposes (by module index), the boundary NAME
/// it is presented under, and its SIGNATURE (the whole ABI contract — the surface is derived from
/// `params`/`ret` at assembly, not from the name or body).
#[derive(Debug, Clone, PartialEq)]
pub struct ExportPlan {
    /// The boundary name (source name; a source `main` becomes `run`).
    pub name: String,
    /// The exported function's module index.
    pub func: usize,
    /// The exported function's parameter types (the ABI's inputs).
    pub params: Vec<Ty>,
    /// The exported function's return type (the ABI's output).
    pub ret: Ty,
}

/// The resolved layout: every export (each by signature), each user function's absolute wasm index,
/// the emission order (exported functions first, in declaration order, then the rest), and whether
/// the component imports the value-heap runtime.
pub struct Layout {
    /// Every `(export …)` item, in declaration order — the full boundary surface, nothing dropped.
    pub exports: Vec<ExportPlan>,
    /// `abs[i]` = the final wasm function index of user function `i`.
    pub abs: Vec<u32>,
    /// User function module-indices in emission order (exported first, declaration order; then the
    /// rest). Both assemblers emit bodies/types in this order, so `abs[order[k]] == base + k`.
    pub order: Vec<usize>,
    /// Whether the module touches the value heap (constructs/projects a compound) — it then imports
    /// the runtime and its user functions start at `RT_FUNC_BASE`. Consumed by the heap assembler,
    /// wired in the tuple slice.
    #[allow(dead_code)]
    pub imports_runtime: bool,
}

impl Layout {
    /// Compute the layout for a lowered module: carry every export by signature, decide whether the
    /// heap runtime is imported (any function touches a compound), and assign each user function its
    /// absolute wasm index (exported functions first).
    pub fn of(module: &MirModule) -> Result<Layout, String> {
        if module.exports.is_empty() {
            return Err("no `(export …)`: nothing is public".to_string());
        }

        // The ABI is the signature: carry each export with its function's params/ret. The boundary
        // name is the SOURCE name, VERBATIM — the compiler never renames an export (no `main`→`run`
        // magic; renaming a function behind the author's back is exactly the name/shape heuristic the
        // whole design forbids). A consumer that wants a conventional entry name is the consumer's
        // concern: it exports the item under that name in source.
        let exports: Vec<ExportPlan> = module
            .exports
            .iter()
            .map(|e| {
                let f = &module.funcs[e.func];
                ExportPlan {
                    name: e.name.clone(),
                    func: e.func,
                    params: f.params.clone(),
                    ret: f.ret.clone(),
                }
            })
            .collect();

        // Dead-code elimination: emit only functions REACHABLE from an export (following `Call` +
        // `FuncRef` through reachable bodies, after folding). A module's internal helpers and its
        // record value-def fold away at their use sites — the record function's body even holds a
        // `FuncRef` (not runtime-emittable), so a dead function is not merely wasted space but would
        // fail selection. Reachability drops them cleanly. (This is the DCE the fold's inlining needs.)
        let n = module.funcs.len();
        let mut reachable = vec![false; n];
        let mut stack: Vec<usize> = module.exports.iter().map(|e| e.func).collect();
        while let Some(f) = stack.pop() {
            if f >= n || reachable[f] {
                continue;
            }
            reachable[f] = true;
            let mut callees = Vec::new();
            collect_callees(&module.funcs[f].body, &mut callees);
            stack.extend(callees);
        }

        // The heap runtime is imported iff any REACHABLE function touches a compound value.
        let imports_runtime = (0..n).any(|i| reachable[i] && body_uses_heap(&module.funcs[i].body));

        // Emission order: exported functions first (declaration order), then the remaining REACHABLE
        // functions in module order. Exported functions get the low core-function indices the boundary
        // aliases reference; a single entry is core func 0 (byte-identical to the old compiler).
        let mut order: Vec<usize> = Vec::new();
        let mut seen = vec![false; n];
        for e in &module.exports {
            if !seen[e.func] {
                seen[e.func] = true;
                order.push(e.func);
            }
        }
        for i in 0..n {
            if reachable[i] && !seen[i] {
                order.push(i);
            }
        }

        // First defined-function index: 0 when nothing is imported; RT_FUNC_BASE past the imports +
        // fixed helpers when the heap runtime is imported.
        let base = if imports_runtime { RT_FUNC_BASE } else { 0 };
        let mut abs = vec![0u32; n];
        for (k, &orig) in order.iter().enumerate() {
            abs[orig] = base + k as u32;
        }
        Ok(Layout { exports, abs, order, imports_runtime })
    }
}

/// Collect the module-function indices a body references — a `Call` callee or a `FuncRef` value (which
/// an `Apply` turns into a call). Drives reachability DCE.
fn collect_callees(m: &Mir, out: &mut Vec<usize>) {
    match m {
        Mir::Call { func, args } => {
            out.push(*func);
            args.iter().for_each(|a| collect_callees(a, out));
        }
        Mir::FuncRef(func) => out.push(*func),
        Mir::Apply { func, args } => {
            collect_callees(func, out);
            args.iter().for_each(|a| collect_callees(a, out));
        }
        Mir::Tuple(elems) => elems.iter().for_each(|(_, e)| collect_callees(e, out)),
        Mir::List(elems) => elems.iter().for_each(|(_, e)| collect_callees(e, out)),
        Mir::Map(entries) => entries.iter().for_each(|((_, k), (_, v))| {
            collect_callees(k, out);
            collect_callees(v, out);
        }),
        Mir::Set(elems) => elems.iter().for_each(|(_, e)| collect_callees(e, out)),
        Mir::HeapOp { args, .. } => args.iter().for_each(|(_, e)| collect_callees(e, out)),
        Mir::Proj { operand, .. } => collect_callees(operand, out),
        Mir::Arith(_, a, b) | Mir::Bit(_, a, b) | Mir::Shift(_, a, b) => {
            collect_callees(a, out);
            collect_callees(b, out);
        }
        Mir::Cmp { a, b, .. } => {
            collect_callees(a, out);
            collect_callees(b, out);
        }
        Mir::If { cond, then_, else_, .. } => {
            collect_callees(cond, out);
            collect_callees(then_, out);
            collect_callees(else_, out);
        }
        Mir::Let { value, body, .. } => {
            collect_callees(value, out);
            collect_callees(body, out);
        }
        // A sum's payload + a match's scrutinee and arm bodies may reference functions.
        Mir::Sum { payload, .. } => collect_callees(payload, out),
        Mir::Match { scrutinee, arms, .. } => {
            collect_callees(scrutinee, out);
            arms.iter().for_each(|(_, b)| collect_callees(b, out));
        }
        // A lambda body can reference functions (a recursive closure) — descend so a closed FuncRef
        // keeps the callee reachable.
        Mir::Lambda { body, .. } => collect_callees(body, out),
        // An intrinsic / ctor / wildcard / trap / str-literal / type-value / type-ctor references no module function.
        Mir::Intrinsic(_) | Mir::Ctor { .. } | Mir::Wildcard | Mir::Trap(_)
        | Mir::Int(_) | Mir::Bool(_) | Mir::Str(_) | Mir::Unit | Mir::Local(_) | Mir::Error(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) => {}
    }
}

/// Whether a lowered function body touches the value heap (constructs or projects a compound), which
/// is what forces the runtime import + the `RT_FUNC_BASE` function-index offset.
fn body_uses_heap(m: &Mir) -> bool {
    match m {
        // A product construction, or ANY projection (a projection means an `arr-get` on a heap
        // handle), touches the value heap.
        Mir::Tuple(_) => true,
        // A list construction is a value-heap op (`vec-*`).
        Mir::List(_) => true,
        // A map/set construction is a value-heap op (CHAMP `map-*`/`set-*`).
        Mir::Map(_) | Mir::Set(_) => true,
        // A heap op (List.push / Map.* / Set.*) touches the value heap (CHAMP/vec himports).
        Mir::HeapOp { .. } => true,
        Mir::Proj { operand, .. } => {
            // The projection itself is a heap op; also recurse the operand (it built the product).
            let _ = operand;
            true
        }
        Mir::Call { args, .. } => args.iter().any(body_uses_heap),
        Mir::Arith(_, a, b) | Mir::Bit(_, a, b) | Mir::Shift(_, a, b) => {
            body_uses_heap(a) || body_uses_heap(b)
        }
        Mir::Cmp { a, b, .. } => body_uses_heap(a) || body_uses_heap(b),
        Mir::If { cond, then_, else_, .. } => {
            body_uses_heap(cond) || body_uses_heap(then_) || body_uses_heap(else_)
        }
        Mir::Let { value, body, .. } => body_uses_heap(value) || body_uses_heap(body),
        // A function value / application does not itself touch the heap (it folds to a `Call`, or
        // declines in `select`); an application's args might.
        Mir::Apply { func, args } => body_uses_heap(func) || args.iter().any(body_uses_heap),
        // A lambda body might touch the heap; check it.
        Mir::Lambda { body, .. } => body_uses_heap(body),
        // Constructing a sum (`sum-new`) and matching one (`sum-disc`/`sum-payload`) are value-heap ops.
        Mir::Sum { .. } => true,
        Mir::Match { .. } => true,
        // A string literal builds a `bytes-*` heap leaf.
        Mir::Str(_) => true,
        Mir::FuncRef(_) | Mir::Intrinsic(_) | Mir::Ctor { .. } | Mir::Wildcard | Mir::Trap(_)
        | Mir::Int(_) | Mir::Bool(_) | Mir::Unit | Mir::Local(_) | Mir::Error(_) | Mir::TypeVal(_) | Mir::TypeCtor(_) => false,
    }
}


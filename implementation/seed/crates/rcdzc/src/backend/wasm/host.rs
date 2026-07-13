//! Host-delegated effect operations at the component boundary (E2).
//!
//! A `(host (E…) …)` delegation routes its listed effects to the component boundary: each performed
//! operation is a component-level WIT function the host resolves (`capabilities-and-effects.md` §A Host
//! Import Is A Boundary Effect And The Manifest Is Its Row). The compiler emits the program's core
//! module importing one core function per host op, and the component envelope imports the declaring
//! effect as an INTERFACE (an instance-type declaring the op as a func), aliases the op out, lowers it,
//! and binds it to the program — the same 7-section shape the value-heap runtime import takes, but the
//! interface is named by the EFFECT (a dotted `E.op` is never a top-level extern — the component model
//! forbids the dot, so the boundary is `interface E { func op }`).
//!
//! This module owns the host-import SET: its descriptor, the `Ty → AbiValType` boundary mapping, and the
//! walk that collects every `Core::HostCall` a reachable body performs into a deterministic ordered set
//! (the parallel of `select::collect_used_ops` for runtime ops). `emit` fixes this set BEFORE selection,
//! so a `Core::HostCall` resolves to its position in it (a `Lir::CallHostImport(index)`), and the
//! serializer + envelope lay the imports in that order.
//!
//! SCOPE (E2h-2): the SCALAR boundary — a scalar/unit operation parameter and a scalar/unit result. A
//! string or compound parameter/result (the old seed's `HostString` (ptr,len) shape, and the resource
//! escape) is a later increment; such an op declines here (its `AbiValType` mapping returns `None`).

use crate::ast::StructId;
use crate::backend::wasm::runtime_abi::AbiValType;
use crate::core::Core;
use crate::db::Db;
use crate::lower::core_of;
use crate::ty::Ty;

/// One host-delegated operation the program performs — its declaring effect's NAME (the WIT interface),
/// the operation's NAME (the func in it), and its scalar boundary signature. Two operations are the same
/// import iff `(effect, op)` match; the SET is ordered (its position is the import's core-func index).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HostImport {
    /// The declaring effect's name — the WIT interface the op is imported through.
    pub effect: String,
    /// The operation's name — the func exported by the interface.
    pub op: String,
    /// The operation's boundary parameter types (scalar only in this increment). A `Unit` domain is
    /// ELIDED (a nullary op `(-> Unit R)` performed as `(E.op)` takes no boundary parameter).
    pub params: Vec<AbiValType>,
    /// The operation's boundary result type — `None` for a `Unit` result.
    pub result: Option<AbiValType>,
}

/// The component/boundary scalar ABI type a value of solved type `ty` crosses as, or `None` if it has no
/// SCALAR boundary form (unit, or a compound/string — the latter declines this increment). Mirrors
/// `comp_valtype_of`'s aliased-width mapping, but yields the backend's `AbiValType` (which carries both
/// the core and component bytes) rather than a raw byte.
pub fn abi_val_type(ty: &Ty) -> Option<AbiValType> {
    match ty {
        Ty::Bool => Some(AbiValType::Bool),
        // A 64-bit float crosses as `f64`; a narrower float width has no scalar host ABI here yet.
        Ty::Float(ft) if ft.ground_width() == 64 => Some(AbiValType::F64),
        Ty::Int(it) => match (it.ground_signed(), it.ground_width()) {
            // Only the aliased 32/64 widths have a scalar `AbiValType`; a narrower aliased width
            // (8/16) has a component primitive but the runtime-op ABI table only models u32/s64/bool/f64,
            // so this increment crosses 32- and 64-bit integers (the corpus host ops are `Int64`). A
            // narrower or non-aliased width declines (a later increment widens the ABI table).
            (false, 32) => Some(AbiValType::U32),
            (true, 64) => Some(AbiValType::S64),
            _ => None,
        },
        _ => None,
    }
}

/// Collect the host-import SET a reachable body performs — every distinct `(effect, op)` a
/// `Core::HostCall` names, in first-encountered order (deterministic: the same walk order every build).
/// `out` accumulates across all reachable bodies (the caller runs it over `layout.order`). A duplicate
/// `(effect, op)` is not re-added (the same op called twice is ONE import). Descends every sub-position
/// (both `if` branches, arm bodies, operands) so an op used only under a branch is still imported.
pub fn collect_host_imports(db: &mut Db, id: StructId, out: &mut Vec<HostImport>) {
    match core_of(db, id) {
        Core::HostCall {
            effect,
            op,
            args,
            result,
        } => {
            // The op's boundary signature — parameter types from the arg types, result from `result`. A
            // `Unit` arg/result is elided (no boundary slot). A non-scalar arg/result yields `None`; such
            // an op is still recorded (with whatever scalars map) but the envelope declines to lay a
            // non-scalar signature — the decline surfaces at assembly, not here.
            let mut params = Vec::new();
            for &a in &args {
                let at = crate::infer::type_of(db, a);
                if !matches!(at, Ty::Unit)
                    && let Some(v) = abi_val_type(&at)
                {
                    params.push(v);
                }
            }
            let result_abi = if matches!(result, Ty::Unit) {
                None
            } else {
                abi_val_type(&result)
            };
            let imp = HostImport {
                effect,
                op,
                params,
                result: result_abi,
            };
            if !out.iter().any(|h| h.effect == imp.effect && h.op == imp.op) {
                out.push(imp);
            }
            for a in args {
                collect_host_imports(db, a, out);
            }
        }
        _ => {
            // Descend structurally into every child — a host call may be nested anywhere.
            if let crate::ast::Struct::List(children) = db.ast.get(id).clone() {
                for c in children {
                    collect_host_imports(db, c, out);
                }
            }
        }
    }
}

/// The index of the host import for `(effect, op)` in the ordered set — the core-func index a
/// `Core::HostCall` lowers its call to. `None` if not in the set (a compiler bug — the set is collected
/// from the same `Core::HostCall` nodes selection emits).
pub fn host_import_index(imports: &[HostImport], effect: &str, op: &str) -> Option<usize> {
    imports
        .iter()
        .position(|h| h.effect == effect && h.op == op)
}

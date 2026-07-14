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

/// One boundary parameter of a host operation: a SCALAR (crosses as its component primitive / one core
/// slot) or a STRING (crosses as the component `string` / TWO core slots `(ptr, len)` read out of the
/// program's linear memory by the canonical ABI). A `Unit` domain contributes NO parameter (elided).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HostParam {
    Scalar(AbiValType),
    /// A `string` parameter — its component valtype is `string`; its core form is `(ptr: i32, len: i32)`.
    Str,
}

/// One host-delegated operation the program performs — its declaring effect's NAME (the WIT interface),
/// the operation's NAME (the func in it), and its boundary signature. Two operations are the same import
/// iff `(effect, op)` match; the SET is ordered (its position is the import's core-func index).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HostImport {
    /// The declaring effect's name — the WIT interface the op is imported through.
    pub effect: String,
    /// The operation's name — the func exported by the interface.
    pub op: String,
    /// The operation's boundary parameters (scalar or string). A `Unit` domain is ELIDED (a nullary op
    /// `(-> Unit R)` performed as `(E.op)` takes no boundary parameter).
    pub params: Vec<HostParam>,
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
///
/// This SET is the manifest and the import list at once: it is derived from the host ops the reachable
/// bodies actually REACH (the escaping delegated row, after nearer handlers interpose), so the imports
/// the envelope emits mirror it exactly — one import per reached host op, and none for an op no body
/// reaches. A program that delegates/reaches no host op collects the EMPTY set (an empty manifest = a
/// pure program). (The value-heap runtime interface is collected separately, not counted here — it is
/// the one import that is NOT a host capability and never appears in this manifest.)
//= spec/capabilities/capabilities-and-effects.md#the-value-heap-runtime-is-the-one-import-that-is-not-a-capability
//# An import of the value-heap runtime interface MUST NOT be a host capability and MUST NOT appear in the manifest, so that reaching the runtime is an internal linkage the compiler controls rather than an effect that escapes to the host, and capability-safety stays auditable as "every import other than the one well-known runtime interface is a capability the manifest enumerates."
//= spec/contracts/host-interface-binding.md#imports-mirror-the-manifest-exactly
//# The set of host operations a component imports MUST equal the set of capabilities its manifest enumerates.
//= constitution.md#iv-no-ambient-authority
//# A compiled component MUST import only the host operations enumerated in its capability manifest.
//= constitution.md#iv-no-ambient-authority
//# The compiler MUST NOT emit an import that the program's declared capabilities do not enumerate.
//= spec/contracts/host-interface-binding.md#imports-mirror-the-manifest-exactly
//# The compiler MUST NOT emit an import for a host operation the manifest does not enumerate.
//= spec/contracts/host-interface-binding.md#imports-mirror-the-manifest-exactly
//# The compiler MUST NOT emit a manifest entry for which no corresponding import is generated.
//= spec/contracts/host-interface-binding.md#the-manifest-is-a-projection-of-the-escaping-effect-row
//# A program's escaping effect row MUST equal the set of host functions it imports, where the escaping row is the union of the effects its entrypoints delegate to the host that no nearer handler discharges (capabilities-and-effects.md §A Host Import Is A Boundary Effect And The Manifest Is Its Row), so that the manifest is a projection of that delegated row rather than a separately-asserted list and an effect an enclosing handler fully interposes before a delegation generates no import.
//= spec/contracts/host-interface-binding.md#a-host-import-is-a-wit-typed-function-the-manifest-enumerates
//# A component's imports MUST be host functions declared in the WIT-shaped world it targets, each bound only when the manifest enumerates it.
pub fn collect_host_imports(db: &mut Db, id: StructId, out: &mut Vec<HostImport>) {
    match core_of(db, id) {
        Core::HostCall {
            effect,
            op,
            args,
            result,
        } => {
            // The op's boundary signature — parameter kinds from the arg types, result from `result`. A
            // `Unit` arg/result is elided (no boundary slot). A STRING arg is `HostParam::Str` (crosses as
            // `string`, core `(ptr,len)`); a scalar arg maps its `AbiValType`. A parameter whose type is
            // neither (a compound) makes the op undelegable — the envelope declines at assembly.
            //
            // The import carries a COMPLETE WIT-typed signature built from the op's own declared types —
            // parameters from the arg types, result from `result` — with NOTHING injected: no extra
            // parameter, no resume/continuation argument, no state, and no error/outcome arm the operation
            // did not itself declare. So a delegated `(op nm (-> P… R))` becomes the import `nm` whose
            // params are `P…` and whose result is `R` verbatim — the WIT import contract is exactly the
            // effect operation's type.
            //= spec/contracts/host-interface-binding.md#a-host-import-is-a-wit-typed-function-the-manifest-enumerates
            //# An imported host function MUST carry a complete WIT-typed signature — its parameter types, its result type, and its error type — sufficient for the compiler to emit that import into the component's world without consulting anything outside the program's source.
            //= spec/contracts/host-interface-binding.md#a-host-import-is-a-wit-typed-function-the-manifest-enumerates
            //# A host-delegated effect operation MUST appear as its declared signature verbatim: an operation `(op nm (-> P… R))` an entrypoint delegates MUST become the imported function `nm` whose parameters are `P…` and whose result is `R`, with the compiler injecting no additional parameter, no resume or continuation argument, no state, and no error or outcome arm the operation did not itself declare, so that the WIT import contract is exactly the effect operation's type and a host implements precisely what the program declared.
            let mut params = Vec::new();
            for &a in &args {
                let at = crate::infer::type_of(db, a);
                match &at {
                    Ty::Unit => {}
                    Ty::String => params.push(HostParam::Str),
                    _ => {
                        if let Some(v) = abi_val_type(&at) {
                            params.push(HostParam::Scalar(v));
                        }
                    }
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

/// Whether the host-import set has ANY string parameter — so the program's core module must EXPORT/IMPORT
/// a linear memory (the `(ptr,len)` a `string` lowers to is read out of it) and the envelope must thread a
/// shared-memory module + a Memory canon-option on each string op's lower. A scalar-only host set needs no
/// memory (byte-identical to the E2h-2 scalar shape).
pub fn set_needs_memory(imports: &[HostImport]) -> bool {
    imports
        .iter()
        .any(|h| h.params.iter().any(|p| matches!(p, HostParam::Str)))
}

/// The first host operation the subtree at `id` performs whose BOUNDARY SIGNATURE this increment cannot
/// yet emit — returns `Some((op, "result"|"argument", type-name))` for an HONEST feature-limitation
/// decline, or `None` when every reached host op is representable. A `Core::HostCall`'s result is emittable
/// when it is `Unit` or a scalar (`abi_val_type`); a NON-scalar non-Unit result (a `String`, a compound)
/// is NOT — a `String`/`list<u8>` result needs the memory + list-lifting envelope the closure-`Bytes`
/// path has but the plain host envelope does not (a later increment). An ARGUMENT is emittable when it is
/// `Unit`, a `String` (crosses `(ptr,len)`), or a scalar; a compound argument is likewise deferred.
/// Without this, an unrepresentable result silently collected `result: None` (indistinguishable from a
/// Unit result), then `select` hit the INTERNAL "not in the host-import set" path — a message documented
/// as "a compiler bug" surfacing for a valid-but-unsupported program. Diagnosing it here names the real
/// limitation instead. Walks the same positions as `collect_host_imports` (bounded by the AST).
/// Whether `ty` is UNDETERMINED — a top-level `Ty::Any` or a type carrying a free unification variable.
/// A synthesized `Core::HostCall` (a fold-forwarded perform) types its result `Ty::Any`, and an unresolved
/// operand types a var; neither is a real "unrepresentable boundary type" signal (selection resolves it),
/// so [`first_unrepresentable_host_op`] must not flag it. A genuinely-declared non-scalar (`Ty::String`, a
/// compound) is DETERMINED and still flagged.
fn ty_undetermined(ty: &Ty) -> bool {
    matches!(ty, Ty::Any) || ty.has_free_var()
}

pub fn first_unrepresentable_host_op(
    db: &mut Db,
    id: StructId,
) -> Option<(String, &'static str, String)> {
    if let Core::HostCall {
        op, args, result, ..
    } = core_of(db, id)
    {
        // The RESULT: emittable iff Unit or scalar. A DETERMINED non-scalar (a `String`, a compound) is
        // deferred → decline. An UNDETERMINED result (`Ty::Any` / a free var) is NOT flagged: a fold-
        // SYNTHESIZED `Core::HostCall` (a forwarded/interposed perform) types its result `Ty::Any`
        // (infer.rs), and its real emittability is decided when selection resolves it — flagging `Any` here
        // would falsely reject the working interpose-forward case. Only a genuinely-determined non-scalar
        // (e.g. a declared `(-> Unit String)` op) is the unsupported feature.
        if !matches!(result, Ty::Unit)
            && !ty_undetermined(&result)
            && abi_val_type(&result).is_none()
        {
            return Some((op, "result", result.render_name()));
        }
        // Each ARGUMENT: emittable iff Unit, String, or scalar. A DETERMINED compound argument is deferred;
        // an undetermined arg type (a synthesized node) is skipped for the same reason as the result.
        for &a in &args {
            let at = crate::infer::type_of(db, a);
            if !matches!(at, Ty::Unit | Ty::String)
                && !ty_undetermined(&at)
                && abi_val_type(&at).is_none()
            {
                return Some((op, "argument", at.render_name()));
            }
        }
        // Descend the args too (a host call may be nested in an arg).
        for a in args {
            if let Some(hit) = first_unrepresentable_host_op(db, a) {
                return Some(hit);
            }
        }
        return None;
    }
    if let crate::ast::Struct::List(children) = db.ast.get(id).clone() {
        for c in children {
            if let Some(hit) = first_unrepresentable_host_op(db, c) {
                return Some(hit);
            }
        }
    }
    None
}

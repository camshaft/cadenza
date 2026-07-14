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

/// The boundary `AbiValType` a value of type `ty` crosses as BETWEEN CADENZA PEERS over a SHARED runtime
/// (X5). A scalar crosses by its scalar rep (`abi_val_type`); a runtime-owned COMPOUND (a value with no
/// scalar rep but a `u32` heap handle in-guest — a tuple/record/sum/list/map/set/string/bytes/bigint/
/// rational) crosses as its opaque `u32` handle into the shared heap (component-abi.md §Cadenza Components
/// Composed Against A Shared Runtime Exchange Values As Handles). Unlike the HOST boundary (where a compound
/// has no representation — the host can't build a heap handle), a peer shares the runtime, so the handle is
/// meaningful on both sides. `None` only for `Unit` (elided) or a type with neither a scalar rep nor a heap
/// handle (a bare function — declines).
pub fn extern_abi_val_type(ty: &Ty) -> Option<AbiValType> {
    if let Some(v) = abi_val_type(ty) {
        return Some(v);
    }
    // A value the RUNTIME OWNS crosses as its opaque `u32` heap handle — the compound types that live on
    // the value heap (a tuple/record/sum/list/map/set + the byte-rope String/Bytes + the bignum BigInt/
    // Rational), and an erased nominal over one. NOT a narrow scalar (Int8 has no scalar `abi_val_type` yet
    // but is NOT a heap value). `Unit`/a bare function → None (declines).
    //
    // So a COMPOUND crosses between peers as an opaque handle INTO the one shared runtime instance (X5),
    // NOT marshaled into a component-model aggregate — no serialization, the shared runtime owns the value.
    // The handle is the SAME runtime handle a program exchanges with the runtime across its internal
    // boundary, interpretable only by that shared runtime (neither peer dereferences it).
    //= spec/contracts/component-abi.md#cadenza-components-composed-against-a-shared-runtime-exchange-values-as-handles
    //# Two or more separately-derived Cadenza components that a host composes against a single value-heap runtime instance MUST exchange a compound value that crosses between them as an opaque handle into that shared runtime, rather than by marshaling the value into a component-model aggregate at the crossing, so that a value passes between Cadenza components with no serialization and the shared runtime that owns the value is the one place its representation lives.
    //= spec/contracts/component-abi.md#cadenza-components-composed-against-a-shared-runtime-exchange-values-as-handles
    //# The opaque handle by which a compound value crosses MUST be interpretable only by the shared runtime — the same runtime handle a program exchanges with the runtime across its internal boundary (§A Runtime Value Crosses As An Opaque Handle) — so that a handle one component produces is a value the other accepts without either dereferencing it, and the concrete boundary form of that handle (a runtime handle valtype, or a well-known `value` resource type the runtime interface publishes) is fixed at the declared-default location rather than by this contract.
    if is_extern_heap_type(ty) {
        Some(AbiValType::U32)
    } else {
        None
    }
}

/// Whether `ty` is a value the shared runtime OWNS — its cross-peer boundary form is an opaque `u32`
/// handle into the shared heap (X5). The value-heap compound + byte-rope + bignum types; an erased
/// nominal reads through to its inner type.
fn is_extern_heap_type(ty: &Ty) -> bool {
    match ty {
        Ty::Tuple(_)
        | Ty::Record(_)
        | Ty::Sum { .. }
        | Ty::List(_)
        | Ty::Map(_, _)
        | Ty::Set(_)
        | Ty::Bytes
        | Ty::String
        | Ty::BigInt
        | Ty::Rational => true,
        Ty::Nominal { inner, .. } => is_extern_heap_type(inner),
        Ty::Qty { inner, .. } => is_extern_heap_type(inner),
        _ => false,
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
//= spec/capabilities/capabilities-and-effects.md#undeclared-capability-is-a-compile-time-error
//# The compiler MUST determine a program's required capabilities from the operations its entrypoints actually reach and delegate, rather than from a separately-asserted list that could understate them.
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
//= spec/contracts/host-interface-binding.md#the-manifest-is-a-projection-of-the-escaping-effect-row
//# A component that delegates no host function, or whose every otherwise-delegated effect a nearer handler discharges, MUST have an empty manifest, so that a program's purity is the empty row and is legible from an empty manifest, and a program whose every host operation is interposed by a handler is pure.
//= spec/contracts/host-interface-binding.md#a-host-import-is-a-wit-typed-function-the-manifest-enumerates
//# A component's imports MUST be host functions declared in the WIT-shaped world it targets, each bound only when the manifest enumerates it.
//= spec/capabilities/self-hosting-surface.md#host-calls-reach-the-host-through-the-manifest-s-capabilities
//# A compiled component MUST make the host calls its observable behavior records only through the host functions the program's manifest enumerates.
//= spec/capabilities/self-hosting-surface.md#host-calls-reach-the-host-through-the-manifest-s-capabilities
//# A compiled component MUST NOT make a host call through a host function the program's manifest does not enumerate.
//= spec/capabilities/self-hosting-surface.md#a-compiled-program-computes-its-behavior-without-ambient-authority
//# A program MUST NOT reach a host function outside the capabilities its manifest enumerates to compute its observable behavior, so that behavior is deterministic and capability-bound.
//= spec/contracts/build-tool-interface.md#the-tool-produces-a-component-a-manifest-and-diagnostics
//# The component the build tool produces MUST have imports that mirror the manifest it produces, as fixed by the host-interface-binding contract.
//= spec/capabilities/capabilities-and-effects.md#a-host-import-is-a-boundary-effect-and-the-manifest-is-its-row
//# A program's escaping effect row MUST equal the set of effects its entrypoints delegate to the host, so that a capability and a boundary effect are one concept and the manifest is a projection of the effects an entrypoint routes to the boundary rather than of every effect declared.
//= spec/capabilities/capabilities-and-effects.md#a-host-import-is-a-boundary-effect-and-the-manifest-is-its-row
//# Purity MUST be the empty effect row: an entrypoint that delegates no effect to the host MUST reach no effect that escapes and MUST run to normal termination without suspending, so that an entrypoint's determinism is legible from an empty delegation and an entrypoint whose every reached effect is handled in-program is pure.
//= spec/capabilities/capabilities-and-effects.md#an-effect-that-does-not-escape-is-discharged-by-a-handler
//# An effect discharged by an in-program handler MUST NOT appear in the program's manifest, so that only effects that escape to the host — those an entrypoint delegates and no nearer handler discharges — are capabilities.
// Because the set is derived by REACHABILITY from the exports (`collect_host_imports` runs over
// `layout.order`, itself a worklist grown from the exports), a linked dependency that no entrypoint
// reaches contributes no import — dependency resolution never enlarges the required-capability set beyond
// the union the entrypoints reach.
//= spec/capabilities/modules-and-namespaces.md#resolution-introduces-no-authority
//# The set of capabilities a program requires MUST NOT be enlarged by dependency resolution beyond the union its entrypoints delegate to the host, so that pulling in a dependency that declares or performs an effect grants no authority unless an entrypoint delegates that effect (capabilities-and-effects.md §The Program Manifest Is The Union Of Its Entrypoints' Delegations).
// A host op is the ONLY source of nondeterminism a program can reach, and every reached host op appears
// in this set — so a program's determinism is legible from its manifest, and the compiler grants no
// nondeterminism source the program did not delegate (it emits an import only for a reached, delegated op).
//= spec/contracts/host-interface-binding.md#the-manifest-makes-nondeterminism-legible
//# An operation whose result is a source of nondeterminism MUST be reachable only through a capability the manifest enumerates, so that a program's determinism is legible from its manifest.
//= spec/contracts/host-interface-binding.md#the-manifest-makes-nondeterminism-legible
//# The compiler MUST NOT grant a program a source of nondeterminism the program did not declare as a capability.
// This is the reachability-based enforcement of constitution III: a host op is the only nondeterminism
// source a program can reach, and one is imported only when a reached body delegates it — so the compiler
// never introduces a nondeterminism source the program did not obtain through a declared capability.
//= constitution.md#iii-the-compiler-introduces-no-undeclared-nondeterminism
//# The compiler MUST NOT introduce into a component a source of nondeterminism that the program did not obtain through a declared capability.
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
            //= spec/contracts/host-interface-binding.md#a-host-import-is-a-wit-typed-function-the-manifest-enumerates
            //# An operation whose declared result type is itself fallible MUST carry that fallibility in its own result type, which the program handles as an ordinary value, so that error handling is the program's declared contract rather than something the boundary adds to a delegated operation.
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

/// One CROSS-COMPONENT extern import a `Core::ExternCall` names (X4b) — the peer INTERFACE, the operation
/// NAME (the func the interface exports), and its boundary signature. Two calls are the same import iff
/// `(interface, op)` match; the SET is ordered (its position is the import's core-func index in the
/// `"peer"`-bound import block, laid AFTER the host + runtime imports). The peer analogue of [`HostImport`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExternImport {
    /// The peer interface the op is imported through (`cadenza:pkg/iface`).
    pub interface: String,
    /// The operation's name — the func the peer interface exports.
    pub op: String,
    /// The op's boundary parameters (scalar; a `Unit` domain is elided). X4b-3 scope: scalar/unit only
    /// (a `value`-handle param is X5); a param with no scalar ABI makes the call undelegable (declines).
    pub params: Vec<AbiValType>,
    /// The op's boundary result — `None` for a `Unit` result.
    pub result: Option<AbiValType>,
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
/// limitation instead. Walks the same positions as `collect_host_imports` (bounded by the AST). This is
/// the rejection for a host function whose declared signature the compiler cannot emit as a well-formed
/// WIT import — it declines rather than emitting a component whose import does not match the world it names.
//= spec/contracts/host-interface-binding.md#a-host-import-is-a-wit-typed-function-the-manifest-enumerates
//# The compiler MUST reject a program that imports a host function whose declared signature it cannot emit as a well-formed WIT import, rather than emit a component whose import does not match the world it names.
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

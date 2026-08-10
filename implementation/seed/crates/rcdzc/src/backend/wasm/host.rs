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
    /// A `list<u8>` (Cadenza `Bytes`) parameter — its component valtype is the shared `(list u8)` DEFINED
    /// type (referenced by index within the import instance-type, unlike `string`'s inline primitive), and
    /// its core form is `(ptr: i32, len: i32)` — IDENTICAL to `Str` at the core level (the guest copies the
    /// rope bytes into shared `mem` and passes `(ptr,len)`; the canon `Lower` reads them via `Memory(0)`,
    /// no realloc for an argument). Only the COMPONENT boundary type differs (list<u8> vs string). adv-62b's
    /// sibling: closes the wasm-vs-rust reverse-parity gap where a runtime Bytes host-arg declined.
    Bytes,
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
        // A char crosses as the component-model `char` primitive (a Unicode scalar), lowered to core i32.
        Ty::Char => Some(AbiValType::Char),
        // Each aliased float width crosses as its component primitive (`f64`/`f32`); a non-aliased width
        // (a deferred/unsolved float) has no boundary form and declines.
        Ty::Float(ft) => match ft.ground_width() {
            64 => Some(AbiValType::F64),
            32 => Some(AbiValType::F32),
            _ => None,
        },
        // Every ALIASED integer width crosses as its faithful component-model primitive (`s8`/`u8`/…/
        // `s64`/`u64`), lowered to the core i32 (width ≤ 32) or i64 (64) slot the canonical ABI uses — the
        // canonical lowering sign/zero-extends a narrow value into its i32 slot, which IS a narrow int's
        // in-guest representation, so a narrow result needs no extra guest-side conversion. A NON-aliased
        // width (`(UInt 48)`, a deferred/unsolved int) has no boundary primitive and declines.
        Ty::Int(it) => match (it.ground_signed(), it.ground_width()) {
            (true, 8) => Some(AbiValType::S8),
            (false, 8) => Some(AbiValType::U8),
            (true, 16) => Some(AbiValType::S16),
            (false, 16) => Some(AbiValType::U16),
            (true, 32) => Some(AbiValType::S32),
            (false, 32) => Some(AbiValType::U32),
            (true, 64) => Some(AbiValType::S64),
            (false, 64) => Some(AbiValType::U64),
            _ => None,
        },
        // A QUANTITY crosses as its INNER numeric type's boundary form. The unit is a COMPILE-TIME value,
        // ERASED before codegen (`Ty::Qty` has the SAME runtime rep as its inner — see lir.rs
        // `valtype_of`/`comp_valtype_of`), so `(Qty Int64 meter)` is an `Int64` at the boundary and
        // `(Qty Float64 meter)` an `f64`. The host supplies the magnitude as that inner scalar; the guest's
        // static `Ty::Qty` carries the unit, so no runtime reconstruction is needed — a wrong-DIMENSION host
        // value is inexpressible (the host has no unit channel; the unit is fixed guest-side by the declared
        // op type). This is the runtime-parameter `@param` Quantity host path (v-cad Length dimensions,
        // v-notebook). A Qty whose inner has no scalar boundary form (a Rational/BigInt inner — a heap value)
        // still declines here; the num/den pair for an exact-Rational Qty is a later increment.
        Ty::Qty { inner, .. } => abi_val_type(inner),
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
    // Rational), and an erased nominal over one. Every aliased SCALAR (incl. a narrow int) already returned
    // via `abi_val_type` above (it crosses by value, not as a handle). `Unit`/a bare function → None.
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
        // A Symbol is a String byte-leaf at run time (the tagless heap has no `Shape::Sym`; a Symbol is
        // represented + compared exactly as its content String — `box_op_ty`/`get_op_ty` in select.rs map
        // it to the String layout). So a Symbol a peer op takes/returns is ALREADY a runtime heap handle
        // and crosses the peer boundary as its opaque `u32` exactly like a String — no marshaling. Without
        // this a peer op declaring a `Symbol` declined at the boundary ("no component boundary form") while
        // the identical String op crossed; this brings the peer transport to the same String parity the
        // compound-element layout already has.
        | Ty::Symbol
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
// The host-interface-binding contract states the same exclusion for THIS projection: the runtime interface
// is not counted among host-function imports, so a component whose only import is that runtime interface
// still has an empty manifest (this walk emits an import only for a reached host op, never the runtime).
//= spec/contracts/host-interface-binding.md#the-manifest-is-a-projection-of-the-escaping-effect-row
//# The single, well-known value-heap runtime interface the compiler emits programs against MUST NOT be counted among a component's host-function imports for the purpose of this projection, so that a component whose only import is that runtime interface still has an empty manifest and every other import remains a host function the manifest enumerates (capabilities-and-effects.md §The Value-Heap Runtime Is The One Import That Is Not A Capability).
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
// This projection realizes the entrypoint→boundary delegation: an effect an enclosing handler discharges
// is folded away before it reaches a `Core::HostCall`, so it never enters this set (never the manifest);
// an effect an entrypoint delegates is reached as a `Core::HostCall` and emitted as an imported host
// function — the host is that effect's terminal discharger.
//= spec/capabilities/capabilities-and-effects.md#host-binding-is-a-routing-decision-made-at-the-entrypoint
//# An entrypoint MUST be able to delegate a set of effects to the host boundary, fixing that within the delegated computation those effects are discharged at the component boundary by an imported-function call the host resolves, so that the host is the *terminal* handler of a delegated effect and delegation is the boundary counterpart of an in-program handler.
//= spec/capabilities/capabilities-and-effects.md#host-binding-is-a-routing-decision-made-at-the-entrypoint
//# An effect an enclosing handler discharges MUST NOT appear in the manifest, and an effect an entrypoint delegates to the host MUST be enumerated in the program's manifest and reached there as a call to an imported host function, so that whether a given performance escapes is determined by the handlers dynamically enclosing it and the delegation enclosing it, and a delegated effect always has exactly one terminal discharger — the host.
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
//= spec/capabilities/self-hosting-surface.md#a-compiled-program-computes-its-behavior-without-ambient-authority
//# A program's observable behavior MUST be a function of its canonical representation, its inputs, and the responses to the host calls it makes alone, so that the same program on the same inputs and the same responses produces the same behavior wherever it runs.
//= spec/contracts/build-tool-interface.md#the-tool-produces-a-component-a-manifest-and-diagnostics
//# The component the build tool produces MUST have imports that mirror the manifest it produces, as fixed by the host-interface-binding contract.
//= spec/capabilities/capabilities-and-effects.md#a-host-import-is-a-boundary-effect-and-the-manifest-is-its-row
//# A program's escaping effect row MUST equal the set of effects its entrypoints delegate to the host, so that a capability and a boundary effect are one concept and the manifest is a projection of the effects an entrypoint routes to the boundary rather than of every effect declared.
//= spec/capabilities/capabilities-and-effects.md#a-host-import-is-a-boundary-effect-and-the-manifest-is-its-row
//# Purity MUST be the empty effect row: an entrypoint that delegates no effect to the host MUST reach no effect that escapes and MUST run to normal termination without suspending, so that an entrypoint's determinism is legible from an empty delegation and an entrypoint whose every reached effect is handled in-program is pure.
//= spec/capabilities/capabilities-and-effects.md#an-effect-that-does-not-escape-is-discharged-by-a-handler
//# An effect discharged by an in-program handler MUST NOT appear in the program's manifest, so that only effects that escape to the host — those an entrypoint delegates and no nearer handler discharges — are capabilities.
// The interposition twin: an effect an enclosing handler FULLY DISCHARGES (without re-performing it) never
// reaches this reachability walk as a `Core::HostCall` (the handler folded the perform away in `effects`),
// so it neither imports nor crosses the boundary — an entrypoint whose every otherwise-delegated effect is
// so interposed is pure with an empty manifest (the run-an-I/O-program-deterministically mechanism).
//= spec/capabilities/capabilities-and-effects.md#a-handler-may-interpose-on-an-effect-an-entrypoint-would-delegate
//# An effect that an enclosing handler fully discharges without re-performing it MUST NOT appear in the manifest and MUST NOT reach the boundary, so that an entrypoint whose every otherwise-delegated effect is interposed by a handler is pure with an empty manifest — the mechanism a test harness uses to run an I/O program as a deterministic one.
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
// Because the ONLY nondeterminism a run can reach is a host op in this manifest (every other operation is
// a pure deterministic function of its inputs), a run's observable behavior is fixed by its input plus the
// ordered responses the host gives those calls — the same input and the same responses in the same order
// reproduce the same host-call sequence and the same result.
//= spec/capabilities/capabilities-and-effects.md#a-run-is-a-deterministic-function-of-its-input-and-responses
//# A run's observable behavior MUST be a deterministic function of its input and the ordered responses to the host calls it makes, so that the same input and the same responses in the same order reproduce the same host-call sequence and the same result (constitution III).
// The TERMINAL CONDITION (whether a terminating run ends in a normal result or a trap) is part of that
// observable behavior, so it too is a deterministic function of input + ordered capability responses:
// nothing but a reached host op can vary it, and this walk proves those are exactly the manifest's ops.
//= spec/capabilities/core-semantics.md#a-program-that-terminates-ends-in-one-of-two-terminal-conditions
//# The terminal condition of a program run that terminates MUST be a deterministic function of its input and its declared capabilities' responses, so that whether a run terminates is a property of the environment that hosts it while the terminal condition of one that does is fixed by the program.
// This walk SURFACES the reached capabilities into the manifest and stops there: it applies NO
// permissibility judgement — every reached host op is enumerated regardless of whether some runtime's
// policy would allow it, and the compiler never refuses a program on such a policy ground (there is no
// allow/deny list here). Deciding which capabilities are permissible is the RUNTIME's concern, not the
// compiler's:
//= spec/contracts/host-interface-binding.md#policy-over-the-manifest-belongs-to-the-runtime
//# The compiler MUST surface a program's declared capabilities in its manifest without deciding which capabilities are permissible.
//= spec/contracts/host-interface-binding.md#policy-over-the-manifest-belongs-to-the-runtime
//# The compiler MUST NOT refuse a program solely because a capability it declares would be disallowed by a particular runtime's policy.
//
// This is a BACKEND-AGNOSTIC reachability walk of the lowered core (it only enumerates reached host ops;
// no wasm-emit specifics), so the RUST backend deliberately REUSES it (e.g. its closure-escapes-effect
// scan) rather than duplicating the descent. The `HostImport` it builds carries wasm-ABI shape, so a clean
// hoist to a shared `backend::host` module would need a walk/construction split — deferred as not worth the
// churn for a single reuse (v-rust-backend agreed); if a 2nd/3rd cross-backend reuse appears, do the split.
pub fn collect_host_imports(db: &mut Db, id: StructId, out: &mut Vec<HostImport>) {
    // WALK-DEPTH GUARD — the same bound `collect_call_callees` / `collect_closure_codes` hold (see
    // [`crate::db::WALK_DEPTH_LIMIT`]): this walk drives `core_of` at every node, and a non-normalizing
    // self-application in a sum-constructor payload materializes an unbounded `Core::SumNew` chain that
    // would overflow the native stack. Past the limit stop descending — a host call buried deeper belongs
    // to a program `collect_faults` rejects anyway, so a clipped set changes no ACCEPTED program.
    if db.walk_depth >= crate::db::WALK_DEPTH_LIMIT {
        return;
    }
    db.walk_depth += 1;
    collect_host_imports_at(db, id, out);
    db.walk_depth -= 1;
}

/// The CORE walk of [`collect_host_imports`] — descend the LOWERED core so a `HostCall` reached through
/// an INLINED helper (spliced into the caller's core by β-reduction, absent from the caller's AST) is
/// found. Mirrors [`crate::layout::collect_closure_codes`] arm-for-arm (exhaustive, NO wildcard, so a new
/// `Core` variant is a compile error here rather than a silently-dropped host call — the same discipline
/// the closure/callee walks hold). A `Core::Call` descends only its ARGS, not the callee's body: a
/// non-inlined callee is itself a `layout.order` entry whose body is walked by the caller loop, so
/// recursing into it here would be redundant (and, for a recursive callee, non-terminating). This is why
/// a reusable effect-performing helper (`assert-eq` performing `Test.fail`) now contributes its op to the
/// import set whether it inlines or emits — where the old AST walk saw only the un-inlined `(assert-eq …)`
/// application and missed the performed op entirely.
fn collect_host_imports_at(db: &mut Db, id: StructId, out: &mut Vec<HostImport>) {
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
            // An effect BOUND to a peer contract (`db.effect_bindings`, U2) crosses a COMPOUND arg/result
            // as its opaque `u32` heap handle over the shared runtime (U5), not by-value like a host op —
            // so a peer-bound op uses `extern_abi_val_type` (compound → `U32` handle). A genuine HOST op
            // keeps the scalar/string mapping (a compound has no host boundary form). `Unit` is elided.
            //
            // A SCALAR crossing to/from a peer still crosses by its component-model scalar representation
            // (`abi_val_type`), NOT as a handle — only a runtime-owned compound carries a handle. And the
            // peer op's boundary signature is the concrete `(-> P… R)` the effect operation declares
            // (monomorphic — no on-demand instantiation), the effects-unified successor of the removed
            // `(extern …)` surface (U4).
            //= spec/contracts/component-abi.md#cadenza-components-composed-against-a-shared-runtime-exchange-values-as-handles
            //# A scalar value that crosses between such components MUST cross by its component-model scalar representation and not as a handle, so that only a value the runtime owns is carried by handle and a scalar carries no runtime dependency.
            //= spec/contracts/component-abi.md#the-exchanged-signature-is-monomorphic
            //# A cross-component imported or exported signature by which components exchange values MUST be monomorphic, per §Generics Do Not Cross The Boundary, so that the exchanged interface names concrete types and a component binds a peer's export at a fixed instantiation the peer emitted rather than requesting an instantiation on demand.
            let peer_bound = db.effect_bindings.contains_key(&effect);
            let mut params = Vec::new();
            for &a in &args {
                let at = crate::infer::type_of(db, a);
                match &at {
                    Ty::Unit => {}
                    Ty::String if !peer_bound => params.push(HostParam::Str),
                    // A runtime `Bytes` arg crosses as `list<u8>` — the (ptr,len) shared-memory shape (same
                    // core form as String, distinct component type). Closes the wasm-vs-rust reverse-parity
                    // gap where a Bytes host-arg declined on wasm. A PEER-bound Bytes crosses as a heap
                    // handle (`extern_abi_val_type` in the `_` arm below), not this host-boundary list<u8>.
                    Ty::Bytes if !peer_bound => params.push(HostParam::Bytes),
                    _ => {
                        let v = if peer_bound {
                            extern_abi_val_type(&at)
                        } else {
                            abi_val_type(&at)
                        };
                        if let Some(v) = v {
                            params.push(HostParam::Scalar(v));
                        }
                    }
                }
            }
            let result_abi = if matches!(result, Ty::Unit) {
                None
            } else if peer_bound {
                extern_abi_val_type(&result)
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
        // A CALL descends only its ARGS (a host call may hide in an argument); the callee's own body is
        // walked when it is itself expanded from `layout.order`. A CallClosure likewise descends the
        // closure value + args.
        Core::Call { args, .. } => {
            for a in args {
                collect_host_imports(db, a, out);
            }
        }
        Core::CallClosure { closure, args } => {
            collect_host_imports(db, closure, out);
            for a in args {
                collect_host_imports(db, a, out);
            }
        }
        // A closure's CAPTURES are ordinary values built in the enclosing scope — a captured value may be a
        // host-call RESULT (`(let ((a (ask.ask))) (fn (x) (+ x a)))` captures the host call `a`), so the
        // captures must be walked or that host op is missed and the program declines. The closure's BODY is
        // walked separately (it emits as its own lifted function whose body the layout reaches).
        Core::Closure { captures, .. } => {
            for c in captures {
                collect_host_imports(db, c, out);
            }
        }
        Core::If { cond, then_, else_ } => {
            collect_host_imports(db, cond, out);
            collect_host_imports(db, then_, out);
            collect_host_imports(db, else_, out);
        }
        Core::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_host_imports(db, value, out);
            }
            collect_host_imports(db, body, out);
        }
        Core::Seq { stmts, tail } => {
            for s in stmts {
                collect_host_imports(db, s, out);
            }
            collect_host_imports(db, tail, out);
        }
        // A boundary block / break — descend into the body / break value to reach any host op inside.
        Core::Block { body, .. } => collect_host_imports(db, body, out),
        Core::Break { value } => collect_host_imports(db, value, out),
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. }
        | Core::ListConcat { lhs, rhs }
        | Core::BytesConcat { lhs, rhs }
        | Core::BigIntBinOp { lhs, rhs, .. }
        | Core::BigIntCmp { lhs, rhs, .. }
        | Core::RationalOfInts { num: lhs, den: rhs }
        | Core::RationalBinOp { lhs, rhs, .. }
        | Core::RationalCmp { lhs, rhs, .. } => {
            collect_host_imports(db, lhs, out);
            collect_host_imports(db, rhs, out);
        }
        Core::BigIntOfI64 { value } => collect_host_imports(db, value, out),
        Core::BigIntToI64 { operand } => collect_host_imports(db, operand, out),
        Core::RationalOfIntWiden { value } => collect_host_imports(db, value, out),
        Core::RationalNum { operand } | Core::RationalDen { operand } => {
            collect_host_imports(db, operand, out)
        }
        Core::ListPush { list, elem } => {
            collect_host_imports(db, list, out);
            collect_host_imports(db, elem, out);
        }
        Core::ListUpdate { list, index, elem } => {
            collect_host_imports(db, list, out);
            collect_host_imports(db, index, out);
            collect_host_imports(db, elem, out);
        }
        Core::ListAt { list, index, .. } => {
            collect_host_imports(db, list, out);
            collect_host_imports(db, index, out);
        }
        Core::MapNew { entries, .. } => {
            for (k, v) in entries {
                collect_host_imports(db, k, out);
                collect_host_imports(db, v, out);
            }
        }
        Core::MapInsert { map, key, val, .. } => {
            collect_host_imports(db, map, out);
            collect_host_imports(db, key, out);
            collect_host_imports(db, val, out);
        }
        Core::MapLookup { map, key, .. } | Core::MapRemove { map, key, .. } => {
            collect_host_imports(db, map, out);
            collect_host_imports(db, key, out);
        }
        Core::MapSize { map } => collect_host_imports(db, map, out),
        Core::SetOf { elems, .. } => {
            for &e in elems.iter() {
                collect_host_imports(db, e, out);
            }
        }
        Core::SetContains { set, elem, .. }
        | Core::SetInsert { set, elem, .. }
        | Core::SetRemove { set, elem, .. } => {
            collect_host_imports(db, set, out);
            collect_host_imports(db, elem, out);
        }
        Core::SetLen { set } => collect_host_imports(db, set, out),
        Core::SetToList { set, .. } => collect_host_imports(db, set, out),
        Core::MapToList { map, .. } => collect_host_imports(db, map, out),
        Core::SetAlgebra { lhs, rhs, .. } => {
            collect_host_imports(db, lhs, out);
            collect_host_imports(db, rhs, out);
        }
        Core::BytesAt { bytes, index, .. } => {
            collect_host_imports(db, bytes, out);
            collect_host_imports(db, index, out);
        }
        Core::StrAt { string, index, .. } => {
            collect_host_imports(db, string, out);
            collect_host_imports(db, index, out);
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            collect_host_imports(db, string, out);
            collect_host_imports(db, start, out);
            collect_host_imports(db, end, out);
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            collect_host_imports(db, bytes, out);
            collect_host_imports(db, start, out);
            collect_host_imports(db, len, out);
        }
        Core::BytesCompact { operand }
        | Core::StrFromBytes { bytes: operand, .. }
        | Core::StrToBytes { string: operand }
        | Core::NfcNormalize { string: operand }
        | Core::Convert { operand, .. }
        | Core::Not { operand }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand } => collect_host_imports(db, operand, out),
        Core::Match { scrutinee, arms } => {
            collect_host_imports(db, scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_host_imports(db, g, out);
                }
                collect_host_imports(db, arm.body, out);
            }
        }
        Core::Record { fields } => {
            for value in fields.values() {
                collect_host_imports(db, *value, out);
            }
        }
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            for &e in elems.iter() {
                collect_host_imports(db, e, out);
            }
        }
        Core::BinBuild { segs } => {
            for s in segs {
                collect_host_imports(db, s.value, out);
            }
        }
        Core::BinBitsBuild { fields } => {
            for f in fields {
                collect_host_imports(db, f.value, out);
            }
        }
        Core::BinIntRead {
            bytes, off_plus, ..
        }
        | Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            collect_host_imports(db, bytes, out);
            if let Some(op) = off_plus {
                collect_host_imports(db, op, out);
            }
        }
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            collect_host_imports(db, bytes, out);
            if let Some(op) = off_plus {
                collect_host_imports(db, op, out);
            }
            collect_host_imports(db, len, out);
        }
        Core::Proj { operand, .. } => collect_host_imports(db, operand, out),
        Core::SumNew { payloads, .. } => {
            for p in payloads {
                collect_host_imports(db, p, out);
            }
        }
        Core::MatchSum { scrutinee, root } => {
            collect_host_imports(db, scrutinee, out);
            collect_cont_host_imports(db, &root, out);
        }
        Core::MatchList { scrutinee, arms } => {
            collect_host_imports(db, scrutinee, out);
            for arm in &arms {
                collect_host_imports(db, arm.body, out);
            }
        }
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => {
            collect_host_imports(db, scrutinee, out)
        }
        // Leaves / references perform no host call.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::Unit
        | Core::Trap
        | Core::Param { .. }
        | Core::Captured { .. }
        | Core::LocalRef { .. }
        | Core::Poison(_) => {}
    }
}

/// Walk a sum-match continuation for the host calls its arm bodies perform — the host-import analogue of
/// `collect_cont_closure_codes`, so a `Test.fail`-style perform inside a `(match …)` arm over a sum is
/// found too.
fn collect_cont_host_imports(db: &mut Db, cont: &crate::core::SumCont, out: &mut Vec<HostImport>) {
    match cont {
        crate::core::SumCont::Leaf(body) => collect_host_imports(db, *body, out),
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_host_imports(db, *cond, out);
            collect_host_imports(db, *body, out);
            collect_cont_host_imports(db, els, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_cont_host_imports(db, then_, out);
            collect_cont_host_imports(db, els, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                collect_cont_host_imports(db, &arm.cont, out);
            }
        }
    }
}

/// One CROSS-COMPONENT extern import a PEER-BOUND effect names — the peer INTERFACE, the operation NAME
/// (the func the interface exports), and its boundary signature. Since U4 (extern→effects) an extern import
/// is derived from an escaping `Core::HostCall` whose effect is peer-bound (`db.effect_bindings`), retargeted
/// to the bound interface in [`emit`]; there is no separate `Core::ExternCall`. Two calls are the same import
/// iff `(interface, op)` match; the SET is ordered (its position is the import's core-func index in the
/// `"peer"`-bound import block, laid AFTER the host + runtime imports). The peer analogue of [`HostImport`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExternImport {
    /// The peer interface the op is imported through (`cadenza:pkg/iface`).
    pub interface: String,
    /// The operation's name — the func the peer interface exports.
    pub op: String,
    /// The op's boundary parameters. A scalar/unit param crosses by value; a runtime-owned COMPOUND crosses
    /// as its opaque `u32` handle over the shared runtime (`extern_abi_val_type`, U5). A `Unit` domain is
    /// elided; a param with no boundary ABI at all makes the call undelegable (declines upstream).
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
    // A Str OR Bytes param crosses as `(ptr,len)` read out of the program's linear memory, so either
    // requires the shared-memory core module + the canon `Lower`'s `Memory(0)` option.
    imports.iter().any(|h| {
        h.params
            .iter()
            .any(|p| matches!(p, HostParam::Str | HostParam::Bytes))
    })
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
        effect,
        op,
        args,
        result,
    } = core_of(db, id)
    {
        // A PEER-BOUND effect (`db.effect_bindings`) crosses a COMPOUND as its opaque runtime handle
        // (`extern_abi_val_type`, X5b), so its representable set is WIDER than a plain host op's — a
        // runtime-owned compound result/argument is emittable, not a decline. `abi_ok` picks the right
        // predicate for this call's surface: a peer-bound effect widens to the handle transport, a plain
        // host effect keeps the scalar-only boundary this increment emits.
        let peer_bound = db.effect_bindings.contains_key(&effect);
        let abi_ok = |ty: &Ty| {
            if peer_bound {
                extern_abi_val_type(ty).is_some()
            } else {
                abi_val_type(ty).is_some()
            }
        };
        // The RESULT: emittable iff Unit or (peer-bound) a handle-crossable value / (host) a scalar. A
        // DETERMINED unrepresentable result is deferred → decline. An UNDETERMINED result (`Ty::Any` / a
        // free var) is NOT flagged: a fold-SYNTHESIZED `Core::HostCall` (a forwarded/interposed perform)
        // types its result `Ty::Any` (infer.rs), and its real emittability is decided when selection
        // resolves it — flagging `Any` here would falsely reject the working interpose-forward case.
        if !matches!(result, Ty::Unit) && !ty_undetermined(&result) && !abi_ok(&result) {
            return Some((op, "result", result.render_name()));
        }
        // Each ARGUMENT: emittable iff Unit, String, Bytes, or (peer-bound) a handle-crossable value /
        // (host) a scalar. A `Bytes` arg now crosses as `list<u8>` at the host boundary (the `(ptr,len)`
        // shared-memory shape, same as String), so it is emittable — no longer a deferred compound. An
        // undetermined arg type (a synthesized node) is skipped for the same reason as the result.
        for &a in &args {
            let at = crate::infer::type_of(db, a);
            if !matches!(at, Ty::Unit | Ty::String | Ty::Bytes)
                && !ty_undetermined(&at)
                && !abi_ok(&at)
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

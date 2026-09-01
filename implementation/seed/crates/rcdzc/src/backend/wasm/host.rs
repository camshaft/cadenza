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
    /// A `record` parameter (shape d) — its component valtype is a `record` DEFINED type (tag `0x72`) the
    /// import instance-type declares, referenced by index (like [`Bytes`](HostParam::Bytes)'s `(list u8)`
    /// ref). Its core form FLATTENS to one slot run per field, in the HOST WIT record's DECLARATION field
    /// order — the fields are REORDERED (`reorder_record_fields_to_wit`, from the target world's op type) out
    /// of the guest's name-lex `Ty::Record` order, because the two differ (e.g. `message{contract, sender,
    /// payload, token}` vs name-lex `contract, payload, sender, token`) and the component-linker requires the
    /// import's record type to STRUCTURALLY match the host's (a name-lex order silently fails to instantiate).
    /// The guest decomposes the value-heap record field-by-field (`emit_record_arg_marshal` reads each WIT
    /// field's NAME-LEX cell index). Carries `(field-name, field-ABI)` per field (in WIT declaration order) so
    /// BOTH the core flatten and the component record type derive from it. A SCALAR field is one slot + an
    /// inline primitive valtype (any aliased width); a BYTES field crosses as `list<u8>` (2 slots + the `(list
    /// u8)` type); a NESTED record recurses (d3); a `result<list<u8>, enum>` field is a compound-in-record.
    Record(Vec<(String, RecordFieldAbi)>),
    /// An `enum` parameter — a payloadless Cadenza sum (every variant nullary, `db.is_enum_disc`). Its
    /// component valtype is an `enum` DEFINED type (tag `0x6d`) the import instance-type declares + EXPORTS
    /// (a nominal type an import func uses must be exported, like a [`Record`](HostParam::Record)), referenced
    /// by index. Its core form is ONE `i32` slot — the discriminant, which is EXACTLY a payloadless enum's
    /// in-guest representation (`ty_is_enum_disc` → a bare `i32.const disc`, no heap handle), so the guest
    /// passes the value directly (no marshal, unlike a Bytes/Record param). Carries the case names (kebab,
    /// DECLARATION = discriminant order — the same order the component `enum` type declares its cases, so the
    /// guest's raw disc IS the component enum's canonical discriminant). The edge-direction shape a
    /// `graph.neighbors(node, kind, dir)` op takes (`dir: enum`).
    Enum(Vec<String>),
    /// A `list<T>` (non-`Bytes`) parameter — e.g. `graph.set-edges`'s `targets: list<reducer-id>` =
    /// `list<list<u8>>`. Its component valtype is a `(list <elem>)` DEFINED type (referenced by index, like
    /// [`Bytes`](HostParam::Bytes)'s `(list u8)`); its core form is `(ptr: i32, count: i32)` — the guest
    /// marshals the value-heap `List` into a `count * stride(elem)` region of the shared `mem` (each element
    /// canonical-encoded at its stride offset), then passes `(region-ptr, count)`. Carries the ELEMENT's ABI
    /// ([`RecordFieldAbi`]) — a `Bytes` element copies its rope into `mem` elsewhere + writes `(ptr,len)` into
    /// the element slot (the shape `set-edges` needs); a scalar element writes its value inline. A `list<u8>`
    /// stays [`Bytes`](HostParam::Bytes) (its own `(ptr,len)` shape), not this. The arg-side analogue of the
    /// spilled result-list LIFT.
    List(Box<RecordFieldAbi>),
    /// A bare scalar-payload VARIANT param (the top-level position, not nested in a record/list) — crosses as
    /// a component `variant` DEFINED type, flattening (canonical variant flatten) to `(disc:i32, join(case
    /// payloads))`. Carries the cases (name, optional scalar payload valtype) in DECLARATION order (= the
    /// component discriminant order). The guest marshals it via `select::emit_variant_reg_flatten` (the same
    /// helper a `RecordFieldAbi::Variant` field uses); a mixed int/float payload is excluded by the detector.
    Variant(Vec<(String, Option<AbiValType>)>),
}

/// Whether a record-field ABI bottoms out at a `list<u8>` (`Bytes`) leaf — so a `list<T>` param carrying it
/// as its element needs the shared `(list u8)` DEFINED type (index 0) in the instance-type. A `Scalar` does
/// not; a `Bytes`/`Result` (its ok arm is `list<u8>`) does; a nested `Record` if any sub-field does.
pub fn record_field_abi_reaches_bytes(f: &RecordFieldAbi) -> bool {
    match f {
        RecordFieldAbi::Scalar(_) => false,
        RecordFieldAbi::Bytes | RecordFieldAbi::Result { .. } => true,
        RecordFieldAbi::Record(sub) => sub.iter().any(|(_, sf)| record_field_abi_reaches_bytes(sf)),
        // A `list<T>` reaches the shared `(list u8)` type iff its element does (a `list<list<u8>>` element is
        // itself `Bytes`; a `list<s64>` does not). Recurse into the element ABI.
        RecordFieldAbi::List(elem) => record_field_abi_reaches_bytes(elem),
        // A `tuple<…>` reaches `(list u8)` iff any element does.
        RecordFieldAbi::Tuple(elems) => elems.iter().any(record_field_abi_reaches_bytes),
        // An `option<T>` reaches `(list u8)` iff its payload does (option<bytes>); a scalar payload does not.
        RecordFieldAbi::Option(payload) => record_field_abi_reaches_bytes(payload),
        // A `variant` with only SCALAR payloads (this increment's scope) never reaches `(list u8)`.
        RecordFieldAbi::Variant(_) => false,
    }
}

/// Whether a record-field ABI MARSHALS INTO SHARED MEMORY — a `Bytes`/`Result` (rope copy), a `list<T>` (its
/// backing array + elements, for ANY element type — unlike [`record_field_abi_reaches_bytes`], a `list<s64>`
/// counts), or a nested `Record` with such a field. Distinct from reaches-bytes: it decides whether a
/// `HostParam::Record` forces the shared-memory core module + the canon `Lower`'s `Memory` option
/// (`set_needs_memory`), which a list-of-scalars field needs even though it never touches the `(list u8)` type.
pub fn record_field_abi_needs_memory(f: &RecordFieldAbi) -> bool {
    match f {
        RecordFieldAbi::Scalar(_) => false,
        RecordFieldAbi::Bytes | RecordFieldAbi::Result { .. } | RecordFieldAbi::List(_) => true,
        RecordFieldAbi::Record(sub) => sub.iter().any(|(_, sf)| record_field_abi_needs_memory(sf)),
        // A `tuple<…>` element/field is written into mem iff any element needs mem (a scalar-only tuple as a
        // record FIELD flattens inline with no mem; as a list element it's always in-mem, but the list itself
        // forces mem via the `List(_)` arm, so this only decides a record's tuple FIELD).
        RecordFieldAbi::Tuple(elems) => elems.iter().any(record_field_abi_needs_memory),
        // An `option<T>` field flattens to `(disc, flatten(payload))` — it needs mem iff its payload does
        // (option<bytes> copies a rope; an option<scalar> flattens with no mem).
        RecordFieldAbi::Option(payload) => record_field_abi_needs_memory(payload),
        // A `variant` with only SCALAR payloads flattens to `(disc, scalar)` core slots — no memory.
        RecordFieldAbi::Variant(_) => false,
    }
}

/// The boundary ABI of one shape-d record FIELD.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RecordFieldAbi {
    /// A NO-WRAP scalar field (Int64/UInt64/Bool/Float64/Float32) — one core slot (`core_byte`), an inline
    /// primitive component valtype (`comp_byte`). Its guest read is wrap-free (`get-int`/`get-bool`/`get-float`).
    Scalar(AbiValType),
    /// A `Bytes` (`list<u8>`) field — 2 core slots `(ptr,len)` (like a `Bytes` PARAM), and the component
    /// record field references the shared `(list u8)` DEFINED type. The guest copies the field's rope into
    /// shared `mem` and writes `(ptr,len)` (needs memory). The common reducer-envelope field shape
    /// (`contract`/`payload`/`token` are all `list<u8>`).
    Bytes,
    /// A NESTED `record` field (shape d3, the message envelope's `sender: origin`) — its own boundary fields,
    /// recursively. Its component type is another `record` DEFINED type (nominal → also EXPORTED from the
    /// instance-type), which the enclosing record's field references by the child's EXPORTED index. Its core
    /// form is the FLATTENING of its fields (a nested record does not spill — its fields flatten inline into
    /// the parent's flattened run). The guest marshals it by projecting the sub-record handle (`arr-get`) then
    /// RECURSING field-by-field.
    Record(Vec<(String, RecordFieldAbi)>),
    /// A `result<list<u8>, enum-or-variant>` field (the response envelope's `answer`) — a compound-in-record.
    /// The canonical ABI flattens a `result<T, E>` like a 2-case variant: `(disc:i32, join(flatten(ok),
    /// flatten(err)))`; with `ok = list<u8>` (`(ptr,len)`) and a PAYLOAD-LESS err (one `i32` disc) the join
    /// pads to `(disc:i32, i32, i32)` = 3 core slots. Its component type references a `result<list<u8>, err>`
    /// DEFINED type (whose err arm references the EXPORTED err defined type). `err_cases` are the err's case
    /// names (kebab, declaration = discriminant order). `err_is_variant` selects the err arm's component TYPE
    /// CONSTRUCTOR — a payload-less `variant` (`0x71`) when the host WIT declares `variant`, else an `enum`
    /// (`0x6d`): the two are DISTINCT component types (a `result<_, variant>` does NOT structurally match a
    /// `result<_, enum>`), so the err arm MUST follow the WIT declaration or the component-linker silently
    /// fails to instantiate — the same WIT-must-drive-the-emitted-type rule as the field ORDER. Set by
    /// [`reorder_record_fields_to_wit`] from the host WIT (the guest side, a payload-less `Sum`, cannot tell
    /// which the host declared). The guest lowers it by branching on the value-heap sum's disc: Ok → rope→mem
    /// copy `(ptr,len)`, Err → `(disc, 0)` — the marshal is identical for `variant`/`enum` (only the type differs).
    Result {
        err_cases: Vec<String>,
        err_is_variant: bool,
    },
    /// A `list<T>` field (or list ELEMENT — a `list<list<T>>`'s inner list) — 2 core slots `(ptr,count)` (like
    /// [`Bytes`](RecordFieldAbi::Bytes), count in place of len), and the component type is a `(list <elem>)`
    /// DEFINED type over the element's own ABI (recursively). The guest marshals it into shared `mem` — an
    /// outer element array + each element lowered after it (`select::emit_list_arg_marshal`, recursed). Makes a
    /// list element / a record's list field FIRST-CLASS: the element ABI is itself a `RecordFieldAbi`, so
    /// nesting is arbitrary-depth.
    List(Box<RecordFieldAbi>),
    /// A `tuple<…>` field (or list ELEMENT — a `list<tuple<…>>`'s element) — the POSITIONAL product. Its
    /// component type is a `(tuple <elem>…)` DEFINED type over the elements' own ABIs; as a list element it is
    /// written in place at its canonical layout (`select::emit_tuple_to_mem`), as a record field it flattens
    /// its elements inline (like a nested record). Element ABIs are themselves `RecordFieldAbi`, so arbitrary
    /// nesting composes.
    Tuple(Vec<RecordFieldAbi>),
    /// An `option<T>` field — a 2-case variant `{ none, some(T) }`. Its component type is an `(option <T>)`
    /// DEFINED type. As a record FIELD it flattens (canonical variant flatten) to `(disc:i32, flatten(T))` —
    /// the guest branches on the value-heap Option's discriminant: Some → `(1, payload)`, None → `(0, 0-pad)`.
    /// This increment carries a SCALAR payload only (`Box<Scalar>`); an `option<bytes>`/`option<compound>` is a
    /// later increment.
    Option(Box<RecordFieldAbi>),
    /// A general `variant { c0, c1(T1), … }` field (NOT option/result-shaped) — the case names in DECLARATION
    /// (= discriminant) order, each with an optional SCALAR payload. Its component type is a `variant` DEFINED
    /// type. As a record FIELD it flattens (canonical variant flatten) to `(disc:i32, join(payloads))`; this
    /// increment scopes the payload cases to a UNIFORM single SCALAR type, so the join is that one scalar slot —
    /// the guest branches on the value-heap sum's disc (Some payload → unbox; nullary → 0). A payload case with
    /// a `Bytes`/compound payload, or MIXED payload widths, is a later increment.
    Variant(Vec<(String, Option<AbiValType>)>),
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
    /// The operation's SCALAR boundary result — `None` for a `Unit` result OR for a SPILLED compound result
    /// (the latter carried by [`spilled_result`](HostImport::spilled_result)).
    pub result: Option<AbiValType>,
    /// The operation's result when it is a SPILLED COMPOUND — a result whose flattened core form is more than
    /// one value, so the canonical ABI returns it through a caller-provided retptr (NOT an i32) and the guest
    /// LIFTS the host-written bytes into a value-heap handle (`select::emit_result_lift`, the general
    /// WIT-type-driven lift). `Some(wit_ty)` carries the WIT result type — which drives the retptr size
    /// (`canonical_layout`), the guest lift, AND the component defined-type the import instance-type declares;
    /// `None` for a scalar/unit result. This single type-carrying field REPLACES the former three per-shape
    /// bool flags (`option<list<u8>>` / `list<tuple<list<u8>,list<u8>>>` / bare `list<u8>`): the shape is now
    /// read off the WIT type rather than a hardcoded flag, so a new spilled shape rides the same machinery
    /// rather than growing a fourth flag. Mutually exclusive with a scalar `result` (both `None`/`None` for a
    /// plain scalar or unit op). A host-boundary-only concept — a peer-bound op's compound crosses as an
    /// opaque `u32` handle over the shared runtime, never this canonical spilled marshal, so a peer op leaves
    /// this `None`.
    pub spilled_result: Option<Ty>,
    /// The operation's result when it is a payloadless `enum` returned BY VALUE — the kebab case names (in
    /// discriminant/declaration order, the same the component `enum` type declares). An enum flattens to ONE
    /// `i32` (the discriminant), so it is NOT spilled (no retptr, unlike [`spilled_result`]); its component
    /// result type is an `enum` DEFINED + EXPORTED type (referenced via the op's `result_cref`, like a spilled
    /// compound), while its CORE result is a bare `i32`. The guest uses the returned `i32` AS the enum value
    /// (a payloadless enum's in-guest rep is a bare `i32` discriminant), so NO lift/wrap is emitted — the
    /// symmetric result-side of [`HostParam::Enum`]. Mutually exclusive with `result`/`spilled_result`
    /// (all three `None` for a plain scalar/unit op). Host-boundary only (a peer-bound enum is a `u32` handle).
    pub enum_result: Option<Vec<String>>,
}

/// Whether `ty` is the built-in `Option<Bytes>` (guest `option<list<u8>>` at the host boundary) — a `Sum`
/// whose declaration has exactly the `Some`/`None` variants instantiated at a single `Bytes` payload. The
/// one compound host RESULT the host-fused bytes path lifts (S0); every other compound result still
/// declines. Reads through an erased nominal wrapper.
/// Whether `ty` is `result<list<u8>, enum>` — the response envelope's `answer` shape. Returns the err
/// enum's case names (kebab, declaration = discriminant order) if so, else `None`. A `Sum` whose decl has
/// exactly `Ok`/`Err` variants, instantiated at `[Bytes, err]` where `err` is a PAYLOAD-LESS enum (a `Sum`
/// all of whose variants are nullary). Reads through erased nominal wrappers.
pub fn result_bytes_enum(db: &mut Db, ty: &Ty) -> Option<Vec<String>> {
    use crate::backend::common::export_name::kebab_extern_name;
    let stripped = ty.strip_nominal();
    let Ty::Sum { decl, args } = stripped else {
        return None;
    };
    if args.len() != 2 || !matches!(args[0], Ty::Bytes) {
        return None;
    }
    // The decl must be the two-variant `Ok`/`Err` result type (scope the immutable Db borrow).
    {
        let d = db.type_decl_by_occ(*decl)?;
        if !(d.variants.len() == 2
            && d.variants.iter().any(|v| v.name == "Ok")
            && d.variants.iter().any(|v| v.name == "Err"))
        {
            return None;
        }
    }
    // The err arm (args[1]) must be a payload-less enum — a `Sum` whose every variant is nullary.
    let Ty::Sum { decl: err_decl, .. } = args[1].strip_nominal() else {
        return None;
    };
    let ed = db.type_decl_by_occ(*err_decl)?;
    if ed.variants.is_empty() || ed.variants.iter().any(|v| !v.payloads.is_empty()) {
        return None;
    }
    Some(
        ed.variants
            .iter()
            .map(|v| kebab_extern_name(&v.name))
            .collect(),
    )
}

/// The case list of a general `variant`-with-scalar-payload host boundary type — a `Sum` that is NOT
/// option-shaped ([`option_payload_ty`]) nor `result<Bytes,enum>` ([`result_bytes_enum`]), with AT LEAST ONE
/// payload case, every case nullary or a SINGLE scalar payload, and all payload cases sharing ONE
/// `AbiValType` (so the canonical variant flatten's payload join is that one scalar slot). Returns
/// `(kebab-case-name, Option<payload-scalar>)` per case in DECLARATION (= discriminant) order, else `None`
/// (a payloadless enum → the [`enum_cases`] path; a mixed-width / `Bytes` / compound / multi-payload variant
/// is a later increment). The general-variant analogue of [`enum_cases`], carrying the payloads.
pub fn variant_scalar_payload_cases(
    db: &mut Db,
    ty: &Ty,
) -> Option<Vec<(String, Option<AbiValType>)>> {
    use crate::backend::common::export_name::kebab_extern_name;
    let Ty::Sum { decl, .. } = ty.strip_nominal() else {
        return None;
    };
    // Option/result-shaped sums use their own arms (distinct component types) — never this general variant.
    if option_payload_ty(db, ty).is_some() || result_bytes_enum(db, ty).is_some() {
        return None;
    }
    let decl = *decl;
    // Snapshot (name, payload-count) per variant to release the immutable Db borrow before the &mut calls.
    let variants: Vec<(String, usize)> = {
        let d = db.type_decl_by_occ(decl)?;
        d.variants
            .iter()
            .map(|v| (kebab_extern_name(&v.name), v.payloads.len()))
            .collect()
    };
    let mut cases = Vec::with_capacity(variants.len());
    let mut join: Option<AbiValType> = None;
    let mut any_payload = false;
    for (disc, (name, n)) in variants.into_iter().enumerate() {
        match n {
            0 => cases.push((name, None)),
            1 => {
                let pty = crate::backend::wasm::select::variant_payload_ty_at(db, ty, disc as u32)?;
                let pv = abi_val_type(&pty)?; // a scalar payload only
                // Admit MIXED payload widths as long as they JOIN cleanly: all-integer (incl bool/char, which
                // share the i32 slot) join to the widest int slot, and a uniform float is fine. REJECT mixing
                // int with float, or f32 with f64 — those need the canonical reinterpret join lattice (a later
                // increment). The marshal computes the register join valtype + the memory max-natural width.
                let is_float = |a: AbiValType| matches!(a, AbiValType::F32 | AbiValType::F64);
                if let Some(prev) = join
                    && prev != pv
                    && (is_float(prev) || is_float(pv))
                {
                    return None;
                }
                join = Some(pv);
                any_payload = true;
                cases.push((name, Some(pv)));
            }
            _ => return None, // a multi-payload case → a later increment
        }
    }
    any_payload.then_some(cases)
}

/// RESULT-SIDE ONLY: whether `ty` is a variant each of whose cases is nullary or carries ONE `leaf_liftable`
/// payload — a SCALAR (as [`variant_scalar_payload_cases`]) OR a liftable COMPOUND (`list`/`Bytes`/`tuple`/
/// `record`/nested). Returns `(case-name, has-payload)` per case in declaration order (= the component disc
/// order). This is the RESULT-lift admission (the payload is read from the spilled retptr'd region by
/// `select::emit_variant_sum_lift`, which recurses `emit_result_lift` for a compound payload); it is DISTINCT
/// from `variant_scalar_payload_cases` (the ARG marshal's register-flatten path stays scalar-only — a compound
/// payload there would need an in-memory arg marshal, a later increment). Excludes option/result-shaped sums
/// (their own arms). Requires ≥1 payload case (an all-nullary sum is an `enum`, handled by value).
pub fn variant_liftable_payload_cases(db: &mut Db, ty: &Ty) -> Option<Vec<(String, bool)>> {
    use crate::backend::common::export_name::kebab_extern_name;
    let Ty::Sum { decl, .. } = ty.strip_nominal() else {
        return None;
    };
    if option_payload_ty(db, ty).is_some() || result_bytes_enum(db, ty).is_some() {
        return None;
    }
    let decl = *decl;
    let variants: Vec<(String, usize)> = {
        let d = db.type_decl_by_occ(decl)?;
        d.variants
            .iter()
            .map(|v| (kebab_extern_name(&v.name), v.payloads.len()))
            .collect()
    };
    let mut cases = Vec::with_capacity(variants.len());
    let mut any_payload = false;
    for (disc, (name, n)) in variants.into_iter().enumerate() {
        match n {
            0 => cases.push((name, false)),
            1 => {
                let pty = crate::backend::wasm::select::variant_payload_ty_at(db, ty, disc as u32)?;
                if !leaf_liftable(db, &pty) {
                    return None; // a non-liftable payload (e.g. a Set/Map/String) → not this increment
                }
                any_payload = true;
                cases.push((name, true));
            }
            _ => return None, // a multi-payload case → a later increment
        }
    }
    any_payload.then_some(cases)
}

/// The boundary ABI of a shape-d record FIELD, or `None` if the field has no boundary form yet. Supports a
/// NO-WRAP scalar (`Int64`/`UInt64`/`Bool`/`Float64`/`Float32` — the read needs no i64→i32 narrow), a
/// `Bytes` (`list<u8>`) field, a NESTED record (recurse), and a `result<list<u8>, enum>` field
/// ([`result_bytes_enum`], the answer-back envelope). A narrow-int/`Char`/`Qty`/`String`, or a not-yet-mapped
/// compound, is a LATER slice → `None`. The guard's admit set + the classifier's `HostParam::Record`
/// production stay in lockstep (no arity skew between the boundary sig and the args).
fn field_boundary_abi(db: &mut Db, ty: &Ty) -> Option<RecordFieldAbi> {
    // A SCALAR field of ANY aliased width crosses NATIVELY as one core slot + its inline component primitive
    // — bool, s8..s64 / u8..u64 (every int width, not just 64), char, f32/f64, and a `Qty` over any of those.
    // Read via `abi_val_type` (general over width), so a record host-arg field is no longer pinned to 64-bit
    // ints. A narrow int / char reads back with `get-int` + an i64→i32 narrow (`emit_record_arg_marshal`).
    if let Some(v) = abi_val_type(ty) {
        return Some(RecordFieldAbi::Scalar(v));
    }
    match ty {
        // A `Bytes` field crosses as `list<u8>` — 2 core slots, a `(list u8)`-type field ref (d2).
        Ty::Bytes => Some(RecordFieldAbi::Bytes),
        // A NESTED record field (d3) crosses if EVERY sub-field itself crosses — recurse (name-lex order).
        Ty::Record(sub) => {
            let sub = sub.clone(); // release the borrow of `ty` before the recursive `&mut db` calls
            let mut fields = Vec::with_capacity(sub.len());
            for (sym, fty) in sub.iter() {
                fields.push((sym.name.to_string(), field_boundary_abi(db, fty)?));
            }
            (!fields.is_empty()).then_some(RecordFieldAbi::Record(fields))
        }
        // A `list<T>` field/element (a `list<list<T>>`'s inner list, or a record's list field) crosses if its
        // ELEMENT crosses — recurse. Its component type is a `(list <elem>)` DEFINED type; core `(ptr,count)`.
        Ty::List(inner) => {
            let inner = (**inner).clone(); // release the borrow of `ty` before the recursive `&mut db` call
            field_boundary_abi(db, &inner).map(|e| RecordFieldAbi::List(Box::new(e)))
        }
        // A `tuple<…>` field/element crosses if EVERY element crosses — recurse (positional). Its component
        // type is a `(tuple <elem>…)` DEFINED type; a non-empty tuple only (an empty tuple has no fields).
        Ty::Tuple(elems) => {
            let elems = elems.to_vec(); // release the borrow of `ty` before the recursive `&mut db` calls
            if elems.is_empty() {
                return None;
            }
            let mut abis = Vec::with_capacity(elems.len());
            for e in &elems {
                abis.push(field_boundary_abi(db, e)?);
            }
            Some(RecordFieldAbi::Tuple(abis))
        }
        _ => {
            // An `option<T>` field: a 2-case variant `{ none, some(T) }` flattening to `(disc, flatten(T))`.
            // The payload crosses as a SCALAR (`(disc, scalar)`) or `Bytes` (`(disc, ptr, len)`, the Some arm
            // copies the rope) this increment; an option<compound> is a later slice (decline).
            if let Some(payload) = option_payload_ty(db, ty) {
                if let Some(pv) = abi_val_type(&payload) {
                    return Some(RecordFieldAbi::Option(Box::new(RecordFieldAbi::Scalar(pv))));
                }
                if matches!(payload.strip_nominal(), Ty::Bytes | Ty::String) {
                    return Some(RecordFieldAbi::Option(Box::new(RecordFieldAbi::Bytes)));
                }
                return None;
            }
            // A `result<list<u8>, enum-or-variant>` field (the answer-back envelope) — carries the err's case
            // names. `err_is_variant` defaults to `false` (enum) here — the guest `Sum` is payload-less and
            // cannot tell which constructor the host declared; `reorder_record_fields_to_wit` stamps it from WIT.
            if let Some(err_cases) = result_bytes_enum(db, ty) {
                return Some(RecordFieldAbi::Result {
                    err_cases,
                    err_is_variant: false,
                });
            }
            // A general `variant { c0, c1(scalar), … }` field (not option/result-shaped) with uniform scalar
            // payloads — the `variant` DEFINED type + the `(disc, payload)` canonical flatten.
            variant_scalar_payload_cases(db, ty).map(RecordFieldAbi::Variant)
        }
    }
}

/// The declared parameter WIT types of a host op (from the target world, DECLARATION order), or `None` if the
/// world/interface/op isn't found. A host op is imported through the interface whose FQ-name last segment
/// kebab-matches the effect (the same match `mod.rs` host_iface lookup + `is_world_import_op` use); the op is
/// the member whose kebab name matches. Used to emit a RECORD host-arg's fields in the WIT type's DECLARATION
/// order rather than the guest's name-lex `Ty::Record` order — the two differ (e.g. `message{contract, sender,
/// payload, token}` vs name-lex `contract, payload, sender, token`), and the component-linker requires the
/// import's record type to STRUCTURALLY match the host's, so a name-lex order silently fails to instantiate.
pub fn wit_op_param_types(
    db: &mut Db,
    effect: &str,
    op: &str,
) -> Option<Vec<crate::wit_world::WitType>> {
    use crate::backend::common::export_name::kebab_extern_name;
    let world_bytes = db.wit_world.clone()?;
    let arenas = crate::codec::decode(&world_bytes)?;
    let world = crate::wit_world::parse_target_world(&arenas, arenas.root)?;
    let ek = kebab_extern_name(effect);
    let iface = world
        .imports
        .iter()
        .find(|i| kebab_extern_name(i.name.rsplit('/').next().unwrap_or(&i.name)) == ek)?;
    let ok = kebab_extern_name(op);
    let member = iface
        .members
        .iter()
        .find(|m| kebab_extern_name(&m.name) == ok)?;
    Some(member.func.params.iter().map(|(_, t)| t.clone()).collect())
}

/// The declared RESULT WIT type of a host op (from the target world), or `None` if the world/interface/op
/// isn't found. The result-side analogue of [`wit_op_param_types`] — the AUTHORITATIVE host contract for a
/// spilled compound result's component type, so its err arm follows the host's `variant`-vs-`enum` CONSTRUCTOR
/// (the #3228 rule, result-side): `run.run`'s world result `result<payload, variant error>` must emit a
/// `variant` err arm, not the `enum` a guest-`Ty`-derived type ([`spilled_result_wit_type`]) would (a
/// `result<_, variant>` and a `result<_, enum>` are DISTINCT component types → a mismatch silently fails to
/// instantiate). For a STRUCTURAL result (`list`/`option`/`tuple`/`bytes`) this equals the guest-derived type,
/// so preferring it is byte-neutral there and corrective only for a nominal (variant/enum) arm.
pub fn wit_op_result_type(
    db: &mut Db,
    effect: &str,
    op: &str,
) -> Option<crate::wit_world::WitType> {
    use crate::backend::common::export_name::kebab_extern_name;
    let world_bytes = db.wit_world.clone()?;
    let arenas = crate::codec::decode(&world_bytes)?;
    let world = crate::wit_world::parse_target_world(&arenas, arenas.root)?;
    let ek = kebab_extern_name(effect);
    let iface = world
        .imports
        .iter()
        .find(|i| kebab_extern_name(i.name.rsplit('/').next().unwrap_or(&i.name)) == ek)?;
    let ok = kebab_extern_name(op);
    let member = iface
        .members
        .iter()
        .find(|m| kebab_extern_name(&m.name) == ok)?;
    Some(member.func.result.clone())
}

/// Reorder a name-lex list of record field ABIs to the WIT record type's DECLARATION field order, recursing
/// into a nested record field (its sub-fields reorder to the nested WIT record's order). A field whose name
/// isn't in `wit_fields` (or a `wit` that isn't a record) keeps the name-lex order unchanged (defensive — the
/// classifier + the world should agree by name). This makes the emitted component record type + core flatten
/// match the host WIT's field order.
pub fn reorder_record_fields_to_wit(
    name_lex: Vec<(String, RecordFieldAbi)>,
    wit: &crate::wit_world::WitType,
) -> Vec<(String, RecordFieldAbi)> {
    use crate::wit_world::WitType;
    let WitType::Record(wit_fields) = wit else {
        return name_lex;
    };
    let mut by_name: std::collections::HashMap<String, RecordFieldAbi> =
        name_lex.into_iter().collect();
    let mut out = Vec::with_capacity(wit_fields.len());
    for (fname, fwit) in wit_fields {
        let Some(abi) = by_name.remove(fname) else {
            continue;
        };
        // Recurse into a NESTED record field: reorder its sub-fields to the nested WIT record's order. For a
        // `result` field, stamp the err arm's component-type constructor from the WIT (`variant` vs `enum`) —
        // the emitted type MUST follow the host WIT (a `result<_, variant>` is a distinct component type from
        // a `result<_, enum>`; a name-lex/guest-default choice silently fails to instantiate).
        let abi = match abi {
            RecordFieldAbi::Record(sub) => {
                RecordFieldAbi::Record(reorder_record_fields_to_wit(sub, fwit))
            }
            RecordFieldAbi::Result { err_cases, .. } => {
                let err_is_variant = matches!(
                    fwit,
                    WitType::Result {
                        err: Some(e),
                        ..
                    } if matches!(e.as_ref(), WitType::Variant(_))
                );
                RecordFieldAbi::Result {
                    err_cases,
                    err_is_variant,
                }
            }
            other => other,
        };
        out.push((fname.clone(), abi));
    }
    // Any field the WIT didn't name (shouldn't happen) — append in name-lex order to stay total.
    for (n, abi) in by_name {
        out.push((n, abi));
    }
    out
}

/// Whether `ty` is a `record` whose EVERY field crosses at the boundary ([`field_boundary_abi`]) — the
/// shape-d record host-ARGUMENT the guest marshals field-by-field. Matches the BARE `Ty::Record` the
/// classifier keys on (a nominal-wrapped record, or a record with a not-yet-mapped field, yields false), so
/// the guard's admit set and the classifier's `HostParam::Record` production stay in lockstep.
pub fn is_boundary_record(db: &mut Db, ty: &Ty) -> bool {
    match ty {
        Ty::Record(fields) => {
            let fields = fields.clone(); // release the borrow of `ty` before the `&mut db` calls
            !fields.is_empty() && fields.values().all(|f| field_boundary_abi(db, f).is_some())
        }
        _ => false,
    }
}

/// Whether a shape-d record parameter type has ANY `Bytes` field — such a record needs shared linear memory
/// (the guest copies each byte field's rope into `mem`) AND the `(list u8)` defined type. Reads the bare
/// `Ty::Record` the classifier keys on.
pub fn record_has_bytes_field(ty: &Ty) -> bool {
    match ty {
        // Recurse into a NESTED record field (d3): a byte field ANYWHERE in the tree needs shared memory.
        Ty::Record(fields) => fields
            .values()
            .any(|f| matches!(f, Ty::Bytes) || record_has_bytes_field(f)),
        _ => false,
    }
}

/// Whether a record ARG has a `list<T>` FIELD anywhere in its tree (recursing into nested records) — a list
/// field marshals its backing array + elements into shared `mem`, so the arg needs the running scratch cursor
/// reserved just like a `Bytes` field. Complements [`record_has_bytes_field`] for the cursor-reservation gate.
pub fn record_has_list_field(ty: &Ty) -> bool {
    match ty.strip_nominal() {
        Ty::Record(fields) => fields
            .values()
            .any(|f| matches!(f.strip_nominal(), Ty::List(_)) || record_has_list_field(f)),
        _ => false,
    }
}

/// Whether a record ARG has a `tuple<…>` FIELD anywhere in its tree (recursing into nested records) — a tuple
/// field may carry a `Bytes` element whose rope is copied into shared `mem`, so the arg reserves the running
/// scratch cursor. Reserving for ANY tuple field (even scalar-only) is a harmless over-reservation (an unused
/// cursor slot). Complements [`record_has_bytes_field`]/[`record_has_list_field`] for the cursor gate.
pub fn record_has_tuple_field(ty: &Ty) -> bool {
    match ty.strip_nominal() {
        Ty::Record(fields) => fields
            .values()
            .any(|f| matches!(f.strip_nominal(), Ty::Tuple(_)) || record_has_tuple_field(f)),
        _ => false,
    }
}

/// Whether a record ARG has an `option<bytes>` FIELD anywhere in its tree (recursing into nested records) —
/// its Some arm copies the payload rope into shared `mem`, so the arg needs the running scratch cursor. An
/// `option<scalar>` does NOT (it flattens to core slots). Complements [`record_has_bytes_field`]/
/// [`record_has_list_field`] for the cursor-reservation gate (an option is a `Sum`, invisible to those).
pub fn record_has_option_bytes_field(db: &mut Db, ty: &Ty) -> bool {
    let Ty::Record(fields) = ty.strip_nominal() else {
        return false;
    };
    let fields = (**fields).clone(); // release the borrow of `ty` before the recursive `&mut db` calls
    fields.values().any(|f| {
        option_payload_ty(db, f)
            .is_some_and(|p| matches!(p.strip_nominal(), Ty::Bytes | Ty::String))
            || record_has_option_bytes_field(db, f)
    })
}

/// The payload type of an OPTION-SHAPED sum (`option<T>`) — a sum with exactly two variants, one nullary
/// and one single-payload, instantiated at a single type argument — else `None`. Returns `T` (the instantiated
/// payload = the sum's sole type argument). The general option classifier (superseding the former Bytes-only
/// `is_option_bytes`): an option result of ANY payload lifts through the same `emit_option_sum_lift` recursion + the same WIT
/// `option<T>` component type, so the boundary need not special-case the payload. Reads through erased nominals.
pub fn option_payload_ty(db: &mut Db, ty: &Ty) -> Option<Ty> {
    let Ty::Sum { decl, args } = ty.strip_nominal() else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    let payload = args[0].clone();
    let d = db.type_decl_by_occ(*decl)?;
    let single = d.variants.iter().filter(|v| v.payloads.len() == 1).count();
    let nullary = d.variants.iter().filter(|v| v.payloads.is_empty()).count();
    (d.variants.len() == 2 && single == 1 && nullary == 1).then_some(payload)
}

/// Whether `ty` is `List<Tuple<Bytes, Bytes>>` (guest `list<tuple<list<u8>,list<u8>>>` at the host
/// boundary) — the kv `prefix-scan` result shape: a list of (key, value) byte-pair tuples. The SECOND
/// compound host RESULT the host-fused bytes path will lift (after `option<T>`, [`option_payload_ty`]):
/// the host writes a spilled list of pairs into a caller-provided return area, which the guest lifts into
/// a value-heap `List<Tuple<Bytes,Bytes>>`. Purely structural (List/Tuple/Bytes — no decl lookup, unlike a
/// `Sum`), so it takes `&Ty` not `&mut Db`. Reads through erased nominal wrappers at each level.
pub fn is_list_byte_pairs(ty: &Ty) -> bool {
    let Ty::List(inner) = ty.strip_nominal() else {
        return false;
    };
    let Ty::Tuple(elems) = inner.strip_nominal() else {
        return false;
    };
    elems.len() == 2 && matches!(elems[0], Ty::Bytes) && matches!(elems[1], Ty::Bytes)
}

/// The case names of an `enum` host-boundary type — a PAYLOADLESS Cadenza sum (every variant nullary, so
/// `db.is_enum_disc`), returned as kebab-cased names in DECLARATION (= discriminant) order, else `None`. The
/// component `enum` type declares its cases in this order, so the guest's raw discriminant (a payloadless
/// enum is a bare `i32.const disc` at run time) IS the component enum's canonical discriminant — no
/// remapping. Reads through an erased nominal wrapper. The parameter analogue of the err-enum in
/// [`result_bytes_enum`].
pub fn enum_cases(db: &mut Db, ty: &Ty) -> Option<Vec<String>> {
    use crate::backend::common::export_name::kebab_extern_name;
    let Ty::Sum { decl, .. } = ty.strip_nominal() else {
        return None;
    };
    if !db.is_enum_disc(*decl) {
        return None;
    }
    let d = db.type_decl_by_occ(*decl)?;
    Some(
        d.variants
            .iter()
            .map(|v| kebab_extern_name(&v.name))
            .collect(),
    )
}

/// Whether a SPILLED-COMPOUND host result of type `ty` is one the general result path handles end-to-end —
/// the guest lift (`select::emit_result_lift`) AND the component defined-type emission (a Ty→WitType→CDef
/// tree of STRUCTURAL, anonymous-allowed component types). The recursion mirrors `emit_result_lift`'s wired
/// arms: a `list<u8>` (Bytes) / `string` leaf (both cross as a `(ptr,len)` spilled to the retptr and lift into
/// a value-heap byte-rope handle — identical layout, so `emit_result_lift`'s `Ty::Bytes | Ty::String` arm and
/// `ty_natural_wit`'s `string`→`WitType::String` mapping already handle both); a `List<T>` of a liftable
/// element (so `list<list<u8>>` = graph.neighbors, and `list<tuple<list<u8>,list<u8>>>` = kv.prefix-scan); a
/// `Tuple` of liftable fields; and an option-shaped sum over `Bytes` (`option<list<u8>>` = kv.get). A
/// `Record`/general-`Sum`/scalar leaf is NOT yet admitted here — a record/variant/enum component type must be
/// NAMED+exported (not anonymous), a later slice; a scalar is not spilled. This is the GENERAL admit predicate
/// that supersedes the three per-shape checks: a new structural shape composes without a new branch.
pub fn result_is_liftable(db: &mut Db, ty: &Ty) -> bool {
    match ty.strip_nominal() {
        // A `list<u8>` (Bytes) or a `string` — the same `(ptr,len)` spilled shape (a guest `String` is a
        // byte-rope handle, the same value-heap representation a `Bytes` result lifts into), so the two share
        // the leaf arm, `emit_result_lift`'s arm, and the shared retptr layout.
        Ty::Bytes | Ty::String => true,
        Ty::List(e) => {
            let e = (**e).clone();
            leaf_liftable(db, &e)
        }
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            !elems.is_empty() && elems.iter().all(|e| leaf_liftable(db, e))
        }
        // A `record { f: T… }` whose EVERY field is liftable — a host op returning a native record. The lift
        // (`emit_result_lift`'s `Ty::Record` arm) reads each field at its host WIT-DECLARATION offset and
        // arr-sets it to the field's name-lex value-heap slot (following the host's field ORDER, the result
        // side of the #3223 rule); `declare_result_lift_ops` declares its `arr-alloc`/`arr-set` + field ops,
        // and its component `record` DEFINED type is DEFINED + EXPORTED by the nominal-export-for-results path.
        Ty::Record(fields) => {
            let fields = fields.clone();
            !fields.is_empty() && fields.values().all(|f| leaf_liftable(db, f))
        }
        // A `result<list<u8>, enum>` (run.run's `result<payload, error>`): Ok = `Bytes`, Err = a PAYLOAD-LESS
        // enum. The lift (`emit_result_sum_lift`) reads the WIT-canonical disc (Ok=0/Err=1), copies the Bytes on
        // Ok, and rebuilds the guest's `Error` enum-disc on Err; the WIT type is `result<list<u8>, enum>`.
        _ if result_bytes_enum(db, ty).is_some() => true,
        // An option-shaped sum (`option<T>`) whose payload `T` is itself liftable — general over the payload
        // (not pinned to `Bytes`); the lift (`emit_option_sum_lift`) recurses the payload, the WIT type is
        // `option<wit(T)>`. So `option<list<u8>>`, `option<list<list<u8>>>`, `option<tuple<…>>` all lift.
        _ if option_payload_ty(db, ty).is_some_and(|p| leaf_liftable(db, &p)) => true,
        // A general VARIANT (N cases, each nullary or ONE liftable payload — scalar OR a liftable compound
        // like `list<u8>`/`list<T>`/`tuple`/`record`; NOT option/result-shaped, which took their own arms).
        // The lift (`select::emit_variant_sum_lift`) reads the disc + the selected case's payload from the
        // spilled retptr'd region (recursing `emit_result_lift` for a compound payload) and rebuilds the guest
        // Sum; its component `variant` DEFINED type comes from `spilled_result_wit_type` → `add_wit_type_deduped`.
        _ => variant_liftable_payload_cases(db, ty).is_some(),
    }
}

/// Whether `ty` is liftable as an ELEMENT/FIELD/PAYLOAD of a spilled compound result — a SCALAR leaf (which
/// `emit_result_lift` loads width-correct + boxes) OR itself a liftable compound ([`result_is_liftable`]).
/// The distinction from `result_is_liftable`: a bare SCALAR is NOT a spilled top-level result (it crosses by
/// value), but it IS a valid leaf of a `list`/`tuple`/`option`. `abi_val_type` recognizes exactly the scalar
/// leaves the lift boxes (bool / char / every aliased int width / f32 / f64, and a `Qty` over one).
fn leaf_liftable(db: &mut Db, ty: &Ty) -> bool {
    abi_val_type(ty).is_some() || result_is_liftable(db, ty)
}

/// The WIT type of a SPILLED-COMPOUND host result — the type that drives its component defined-type emission
/// (`wit_ctype::add_wit_type_deduped`). Structural results (`List`/`Tuple`/`Bytes`) map via
/// [`crate::wit_world::ty_natural_wit`]; the option-shaped sum (`option<list<u8>>`, which `ty_natural_wit`
/// declines as a `Ty::Sum`) maps to `option<list<u8>>`. `None` for a result with no such WIT type. Kept in
/// lockstep with [`result_is_liftable`]: every liftable result has a WIT type here.
pub fn spilled_result_wit_type(db: &mut Db, ty: &Ty) -> Option<crate::wit_world::WitType> {
    use crate::wit_world::WitType;
    if let Some(payload) = option_payload_ty(db, ty) {
        return Some(WitType::Option(Box::new(spilled_result_wit_type(
            db, &payload,
        )?)));
    }
    // A `result<list<u8>, enum>` (run.run): `result<list<u8>, enum{err-cases}>`. The err arm is emitted as an
    // `enum` here (from the guest's payload-less `Error` sum); a host WIT that declares the err arm as a
    // `variant` needs the WORLD result type threaded (the #3228 variant-vs-enum rule, result-side) — a
    // follow-up. For a WIT `enum error` this already host-links; emit+validate is constructor-agnostic.
    if let Some(err_cases) = result_bytes_enum(db, ty) {
        return Some(WitType::Result {
            ok: Some(Box::new(WitType::List(Box::new(WitType::U8)))),
            err: Some(Box::new(WitType::Enum(err_cases))),
        });
    }
    // A general VARIANT result → a `variant` WIT type: each case is `(name, Some(payload-wit))` for a payload
    // case or `(name, None)` for a nullary case, in declaration order (= the component disc order). The
    // payload's WIT comes from its own `ty_natural_wit` — a SCALAR or a liftable COMPOUND (`list<u8>` etc.).
    // The result-side twin of the bare-variant ARG's component type (`build_host_group`'s CDef::Variant).
    if let Some(cases) = variant_liftable_payload_cases(db, ty) {
        let mut wit_cases: Vec<(String, Option<WitType>)> = Vec::with_capacity(cases.len());
        for (i, (name, has_payload)) in cases.iter().enumerate() {
            let pw = if *has_payload {
                let pt = crate::backend::wasm::select::variant_payload_ty_at(db, ty, i as u32)?;
                Some(crate::wit_world::ty_natural_wit(&pt)?)
            } else {
                None
            };
            wit_cases.push((name.clone(), pw));
        }
        return Some(WitType::Variant(wit_cases));
    }
    crate::wit_world::ty_natural_wit(ty)
}

/// The component/boundary scalar ABI type a value of solved type `ty` crosses as, or `None` if it has no
/// SCALAR boundary form (unit, or a compound/string — the latter declines this increment). Mirrors
/// `comp_valtype_of`'s aliased-width mapping, but yields the backend's `AbiValType` (which carries both
/// the core and component bytes) rather than a raw byte.
/// Whether a `list<T>` ELEMENT type is marshalable as a host arg by `select::emit_list_arg_marshal`: a
/// `Bytes`/`String` (crosses as an inner `(ptr,len)`), a SCALAR (aliased-width int/char/float, written
/// inline), a NESTED `list` whose own element is marshalable (recursed to arbitrary depth), or a RECORD whose
/// every field is a scalar or `Bytes` (written in place at its canonical layout by `emit_record_to_mem`).
/// Kept in lockstep with the marshal's element arms so the representability gate admits exactly what the
/// marshal emits — a record with a nested-record/list field, or a tuple/variant element, declines here.
/// Whether a RECORD/TUPLE field of a `list<record|tuple>` ELEMENT is marshalable in place by
/// `select::emit_product_to_mem`: a scalar, a `Bytes`, or an `option<scalar>` (written via `emit_option_to_mem`
/// at the field's canonical offset). A nested record/list/tuple/option<bytes> field is a later slice.
fn product_field_marshalable(db: &mut Db, f: &Ty) -> bool {
    matches!(f.strip_nominal(), Ty::Bytes | Ty::String)
        || abi_val_type(f).is_some()
        || option_payload_ty(db, f).is_some_and(|p| abi_val_type(&p).is_some())
        // A general `variant<scalar>` field of a product element (`list<record{v: variant{…}, …}>` /
        // `list<tuple<variant, …>>`): written in place by `select::emit_variant_to_mem`. Detected after
        // option (option takes its own arm); this is the residual general scalar-payload variant.
        || variant_scalar_payload_cases(db, f).is_some()
}

pub fn list_elem_marshalable(db: &mut Db, ty: &Ty) -> bool {
    match ty.strip_nominal().clone() {
        Ty::Bytes | Ty::String => true,
        Ty::List(inner) => list_elem_marshalable(db, &inner),
        Ty::Record(fields) => {
            !fields.is_empty() && fields.values().all(|f| product_field_marshalable(db, f))
        }
        Ty::Tuple(elems) => {
            !elems.is_empty() && elems.iter().all(|e| product_field_marshalable(db, e))
        }
        // An `option<scalar>` element (`list<option<s64>>`): written in place at its canonical layout by
        // `select::emit_option_to_mem` (disc byte + the payload scalar). A scalar payload only this increment
        // (option<bytes>/compound element is a later slice — the option-to-mem writer would need a cursor).
        ref other if option_payload_ty(db, other).is_some_and(|p| abi_val_type(&p).is_some()) => {
            true
        }
        // A `variant<scalar>` element (`list<variant{a, b(s64), …}>`): written in place at its canonical
        // variant layout (disc + uniform scalar payload) by `select::emit_variant_to_mem`. Detected AFTER
        // option (option takes its own arm); this is the residual general scalar-payload variant. A mixed-
        // width / Bytes / compound variant payload is a later slice (the flatten join widens).
        ref other if variant_scalar_payload_cases(db, other).is_some() => true,
        ref other => abi_val_type(other).is_some(),
    }
}

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
    // SHARING-AWARE VISITED-SET (see [`Db::host_import_visited`], same class + soundness as
    // `collect_call_callees`'s `callee_visited`): a shared core DAG reached via several sub-positions would
    // otherwise be re-walked as a tree (O(K^depth) on a wide fan-out — the emit-walk re-descent). The
    // host-import SET is presence-only (the walk self-dedups by (effect,op)), so skipping an already-walked
    // node changes no output. Cleared at the top-level entry (`walk_depth == 0`) — a fresh per-entry set,
    // required because this walk runs PER-EXPORT and a stale set would drop a later root's imports. After
    // the depth guard: a depth-clipped node is still recorded, sound because the clip is accepted-neutral.
    if db.walk_depth == 0 {
        db.host_import_visited.clear();
    }
    if !db.host_import_visited.insert(id) {
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
            let peer_bound = db.effect_bindings.contains_key(&*effect);
            // The op's DECLARED param WIT types (declaration order) — used to reorder a RECORD arg's fields to
            // the host WIT's field order (the guest's name-lex `Ty::Record` order differs, and the linker needs
            // a structural match). Computed once; a non-record arg ignores it. `arg_i` indexes it (args ↔ WIT
            // params align 1:1 for the host ops that take a record — no Unit args interleaved).
            let wit_params = wit_op_param_types(db, &effect, &op);
            let mut params = Vec::new();
            for (arg_i, &a) in args.iter().enumerate() {
                let at = crate::infer::type_of(db, a);
                match &at {
                    Ty::Unit => {}
                    Ty::String if !peer_bound => params.push(HostParam::Str),
                    // A runtime `Bytes` arg crosses as `list<u8>` — the (ptr,len) shared-memory shape (same
                    // core form as String, distinct component type). Closes the wasm-vs-rust reverse-parity
                    // gap where a Bytes host-arg declined on wasm. A PEER-bound Bytes crosses as a heap
                    // handle (`extern_abi_val_type` in the `_` arm below), not this host-boundary list<u8>.
                    Ty::Bytes if !peer_bound => params.push(HostParam::Bytes),
                    // A RECORD arg with all-SCALAR fields (shape d, first slice) crosses NATIVELY: it
                    // FLATTENS to one core slot per field in NAME-LEX order (the `BTreeMap`'s canonical
                    // order, which is exactly the order the component `record` type declares its fields), and
                    // declares a component `record` DEFINED type. A field that is NOT a scalar (a
                    // `Bytes`/`String`/nested record) makes the whole record undelegable THIS increment —
                    // push nothing, leaving `params` short so the boundary guard (`first_unrepresentable_
                    // host_op`) declines the op (a Bytes/nested field is the d2/d3 slice). A PEER-bound record
                    // still crosses as a `u32` handle (the `_` arm below), not this native record.
                    Ty::Record(fields) if !peer_bound => {
                        let mut field_abis = Vec::with_capacity(fields.len());
                        let mut all_ok = !fields.is_empty();
                        for (sym, fty) in fields.iter() {
                            match field_boundary_abi(db, fty) {
                                Some(v) => field_abis.push((sym.name.to_string(), v)),
                                None => {
                                    all_ok = false;
                                    break;
                                }
                            }
                        }
                        if all_ok {
                            // Reorder the name-lex fields to the host WIT record's DECLARATION order (recursing
                            // into nested records), so the emitted component record type + core flatten match
                            // the host — a name-lex order silently fails the component-linker structural match.
                            let field_abis = match wit_params.as_ref().and_then(|ps| ps.get(arg_i)) {
                                Some(wit) => reorder_record_fields_to_wit(field_abis, wit),
                                None => field_abis,
                            };
                            params.push(HostParam::Record(field_abis));
                        }
                    }
                    // A `list<T>` (non-`Bytes`) arg (`graph.set-edges`'s `targets: list<reducer-id>`) crosses as
                    // a component `(list <elem>)` DEFINED type — core `(ptr, count)`. The guest marshals the
                    // value-heap `List` into the shared `mem`. Admitted when the ELEMENT crosses as a record
                    // field ABI (`field_boundary_abi` — Bytes / scalar / nested). A `Bytes` (`list<u8>`) arg is
                    // NOT this (it's `HostParam::Bytes`, its own `(ptr,len)`). A PEER-bound list is a `u32`
                    // handle (`_` arm). Checked before the scalar arm.
                    Ty::List(elem) if !peer_bound => {
                        if let Some(elem_abi) = field_boundary_abi(db, elem) {
                            params.push(HostParam::List(Box::new(elem_abi)));
                        }
                        // else: element not crossable → push nothing → the boundary guard declines the op.
                    }
                    // An ENUM arg (a payloadless Cadenza sum, `graph.neighbors`'s `dir`) crosses NATIVELY as a
                    // component `enum` DEFINED type: ONE `i32` core slot (the discriminant, which is a
                    // payloadless enum's in-guest rep — a bare `i32.const disc`), so the guest passes it with no
                    // marshal. A PEER-bound enum crosses as a `u32` handle (the `_` arm below), not this native
                    // enum. Checked BEFORE the scalar arm (a `Sum` has no `abi_val_type`, so the `_` arm would
                    // leave `params` short and decline).
                    _ if !peer_bound && enum_cases(db, &at).is_some() => {
                        params.push(HostParam::Enum(enum_cases(db, &at).unwrap()));
                    }
                    // A scalar-payload VARIANT arg (a Cadenza sum with scalar/nullary payload cases) crosses as
                    // a component `variant` DEFINED type — the canonical flatten join (disc + max-width
                    // payload). Checked BEFORE the scalar `_` arm (a Sum has no `abi_val_type`, so `_` would
                    // leave `params` short and decline). The composite-nested variant rides
                    // `RecordFieldAbi::Variant` / the list marshal; this is the top-level bare-variant param.
                    _ if !peer_bound && variant_scalar_payload_cases(db, &at).is_some() => {
                        params.push(HostParam::Variant(
                            variant_scalar_payload_cases(db, &at).unwrap(),
                        ));
                    }
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
            // A SPILLED COMPOUND host result — one whose flattened core form is >1 value, so the canonical
            // ABI returns it via a caller-provided retptr and the guest LIFTS it into a value-heap handle
            // (`select::emit_result_lift`). Admitted GENERALLY by `result_is_liftable` (any structural
            // list/tuple nesting of `list<u8>` + the `option<list<u8>>` shape) — e.g. `option<list<u8>>`
            // (kv.get), `list<tuple<list<u8>,list<u8>>>` (kv.prefix-scan), bare `list<u8>` (identity.id), and
            // `list<list<u8>>` (graph.neighbors) all ride the ONE recursion, no per-shape branch. Host-boundary
            // only (a peer-bound op's compound crosses as a `u32` handle over the shared runtime, never this
            // canonical spilled marshal), so a peer op leaves it `None`. Carrying the WIT type (not per-shape
            // bool flags) is what lets the retptr size / guest lift / component defined-type all derive from
            // ONE source and a new shape ride the same machinery.
            let spilled_result = if !peer_bound && result_is_liftable(db, &result) {
                Some(result.clone())
            } else {
                None
            };
            // A payloadless `enum` RESULT crosses BY VALUE (one i32 disc), NOT spilled — the symmetric
            // result-side of an enum ARG. Host-boundary only; disjoint from a spilled compound (an enum is
            // never `result_is_liftable`) and from a scalar (`abi_val_type` is `None` for a `Sum`).
            let enum_result = if !peer_bound && spilled_result.is_none() {
                enum_cases(db, &result)
            } else {
                None
            };
            let result_abi = if matches!(result, Ty::Unit)
                || spilled_result.is_some()
                || enum_result.is_some()
            {
                None
            } else if peer_bound {
                extern_abi_val_type(&result)
            } else {
                abi_val_type(&result)
            };
            let imp = HostImport {
                effect: effect.to_string(),
                op: op.to_string(),
                params,
                result: result_abi,
                spilled_result,
                enum_result,
            };
            if !out.iter().any(|h| h.effect == imp.effect && h.op == imp.op) {
                out.push(imp);
            }
            for &a in args.iter() {
                collect_host_imports(db, a, out);
            }
        }
        // A CALL descends only its ARGS (a host call may hide in an argument); the callee's own body is
        // walked when it is itself expanded from `layout.order`. A CallClosure likewise descends the
        // closure value + args.
        Core::Call { args, .. } => {
            for &a in args.iter() {
                collect_host_imports(db, a, out);
            }
        }
        Core::CallClosure { closure, args } => {
            collect_host_imports(db, closure, out);
            for &a in args.iter() {
                collect_host_imports(db, a, out);
            }
        }
        // A closure's CAPTURES are ordinary values built in the enclosing scope — a captured value may be a
        // host-call RESULT (`(let ((a (ask.ask))) (fn (x) (+ x a)))` captures the host call `a`), so the
        // captures must be walked or that host op is missed and the program declines. The closure's BODY is
        // walked separately (it emits as its own lifted function whose body the layout reaches).
        Core::Closure { captures, .. } => {
            for &c in captures.iter() {
                collect_host_imports(db, c, out);
            }
        }
        Core::If { cond, then_, else_ } => {
            collect_host_imports(db, cond, out);
            collect_host_imports(db, then_, out);
            collect_host_imports(db, else_, out);
        }
        Core::Let { bindings, body } => {
            for (_, value) in bindings.iter().copied() {
                collect_host_imports(db, value, out);
            }
            collect_host_imports(db, body, out);
        }
        Core::Seq { stmts, tail } => {
            for &s in stmts.iter() {
                collect_host_imports(db, s, out);
            }
            collect_host_imports(db, tail, out);
        }
        // A boundary block / break — descend into the body / break value to reach any host op inside.
        Core::Block { body, .. } => collect_host_imports(db, body, out),
        Core::Break { value } => collect_host_imports(db, value, out),
        // The abort VALUE is evaluated before the non-local branch; a HostCall inside it would otherwise be
        // missed → a missing host import → invalid module. Recurse into it; `handle_id` is a reference to
        // the target handle node, not an emitted subexpression.
        Core::HandleAbort { value, .. } => collect_host_imports(db, value, out),
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. }
        | Core::ListConcat { lhs, rhs }
        | Core::MapMerge { lhs, rhs }
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
        Core::CharToInt { operand } | Core::IntToCharChecked { operand, .. } => {
            collect_host_imports(db, operand, out)
        }
        Core::RationalOfIntWiden { value } => collect_host_imports(db, value, out),
        Core::RationalNum { operand } | Core::RationalDen { operand } => {
            collect_host_imports(db, operand, out)
        }
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
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
            for (k, v) in entries.iter().copied() {
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
        Core::StrScalarAt { operand, index, .. } => {
            collect_host_imports(db, operand, out);
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
        | Core::Blake3Of { operand }
        | Core::AstPrint { operand, .. }
        | Core::AstEncode { operand, .. }
        | Core::AstDecode { operand, .. }
        | Core::StrFromBytes { bytes: operand, .. }
        | Core::StrToBytes { string: operand }
        | Core::NfcNormalize { string: operand }
        | Core::Convert { operand, .. }
        | Core::Not { operand }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        // `Value.encode`/`decode` are `cadenza:runtime/heap` ops, not host imports; they contribute
        // no HostImport but their single value/bytes operand must still be walked for nested performs.
        | Core::ValueEncode { value: operand, .. }
        | Core::ValueDecode { bytes: operand, .. }
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
            for &p in payloads.iter() {
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
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Unit
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
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
    // A Str/Bytes param crosses as `(ptr,len)` read out of the program's linear memory, and a `list<T>` param
    // crosses as `(ptr,count)` (the guest marshals the list INTO the shared memory) — each requires the
    // shared-memory core module + the canon `Lower`'s `Memory(0)` option. A RECORD param that marshals into mem
    // (a `Bytes`/`Result`/`list<T>` field anywhere in its tree) needs it too — the earlier assumption that a
    // Record param always had a sibling Bytes/list forcing memory no longer holds (a `record{ids: list<s64>}`
    // arg's list field marshals its backing into mem with no sibling Bytes/list param).
    imports.iter().any(|h| {
        h.params.iter().any(|p| match p {
            HostParam::Str | HostParam::Bytes | HostParam::List(_) => true,
            HostParam::Record(fields) => {
                fields.iter().any(|(_, a)| record_field_abi_needs_memory(a))
            }
            _ => false,
        })
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
    allow_option_bytes: bool,
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
        let peer_bound = db.effect_bindings.contains_key(&*effect);
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
        // The host-fused bytes-provider path lifts a SPILLED COMPOUND result into a value-heap value via the
        // general `select::emit_result_lift` (mirror of `result_is_liftable`): `option<list<u8>>` (kv.get),
        // `list<tuple<list<u8>,list<u8>>>` (kv.prefix-scan), bare `list<u8>` (identity.id), `list<list<u8>>`
        // (graph.neighbors), and any list/tuple nesting of those. REPRESENTABLE when `allow_option_bytes` (the
        // flag = "this is the bytes-provider / typed-interface host path", not option-specific) and `!peer_bound`
        // (a peer op crosses its compound as a handle, never this canonical spilled marshal).
        let result_is_liftable_spilled =
            allow_option_bytes && !peer_bound && result_is_liftable(db, &result);
        // A payloadless `enum` result crosses BY VALUE (one i32 disc) — representable on the same reducer/
        // host-fused path the enum ARG + spilled compounds ride (gated on `allow_option_bytes` + `!peer_bound`,
        // matching where the enum result's component type is wired). NOT spilled (never `result_is_liftable`).
        let enum_result_by_value =
            allow_option_bytes && !peer_bound && enum_cases(db, &result).is_some();
        if !matches!(result, Ty::Unit)
            && !ty_undetermined(&result)
            && !abi_ok(&result)
            && !result_is_liftable_spilled
            && !enum_result_by_value
        {
            return Some((op.to_string(), "result", result.render_name(&db.name_ctx())));
        }
        // Each ARGUMENT: emittable iff Unit, String, Bytes, or (peer-bound) a handle-crossable value /
        // (host) a scalar. A `Bytes` arg now crosses as `list<u8>` at the host boundary (the `(ptr,len)`
        // shared-memory shape, same as String), so it is emittable — no longer a deferred compound. An
        // undetermined arg type (a synthesized node) is skipped for the same reason as the result.
        for &a in args.iter() {
            let at = crate::infer::type_of(db, a);
            // A shape-d all-scalar RECORD argument crosses NATIVELY (flattened per field) on the reducer
            // typed/host-fused path — gated on `allow_option_bytes` (which marks that path) and `!peer_bound`
            // (a peer record crosses as a `u32` handle, not this flatten), matching where the guest marshal +
            // the record instance-type are wired. Every OTHER path keeps declining a record arg.
            let arg_is_boundary_record =
                allow_option_bytes && !peer_bound && is_boundary_record(db, &at);
            // An ENUM arg (a payloadless sum, e.g. graph.neighbors' `dir`) crosses NATIVELY as a component
            // `enum` type + one i32 disc — representable on the same reducer/host-fused path (gated on
            // `allow_option_bytes` + `!peer_bound`, matching where the enum instance-type is wired).
            let arg_is_boundary_enum =
                allow_option_bytes && !peer_bound && enum_cases(db, &at).is_some();
            // A `list<T>` arg (`graph.set-edges`'s `targets: list<reducer-id>`) crosses as a `(list <elem>)`
            // component type — the guest marshals the value-heap `List` into shared `mem`
            // (`select::emit_list_arg_marshal`). The element must itself be marshalable: a `list<u8>` (Bytes,
            // = `list<list<u8>>`), a SCALAR (aliased-width int/char/float — written inline), OR a NESTED `list`
            // (recursed to arbitrary depth). Same reducer/host-fused gating; a record/tuple/variant element is a
            // later increment (declined here + at the marshal, in lockstep).
            let list_elem_ok = if let Ty::List(e) = at.strip_nominal() {
                let e = (**e).clone();
                list_elem_marshalable(db, &e)
            } else {
                false
            };
            let arg_is_boundary_list = allow_option_bytes && !peer_bound && list_elem_ok;
            // A scalar-payload VARIANT arg passed BARE (the top-level param position, not nested in a record/
            // list) crosses NATIVELY as a component `variant` DEFINED type — the canonical flatten join (disc +
            // max-width payload), the same marshal a record-field/list-element variant uses, now at the param
            // position. Same reducer/host-fused gating; a mixed int/float payload is excluded by the detector.
            let arg_is_boundary_variant = allow_option_bytes
                && !peer_bound
                && variant_scalar_payload_cases(db, &at).is_some();
            if !matches!(at, Ty::Unit | Ty::String | Ty::Bytes)
                && !ty_undetermined(&at)
                && !abi_ok(&at)
                && !arg_is_boundary_record
                && !arg_is_boundary_enum
                && !arg_is_boundary_list
                && !arg_is_boundary_variant
            {
                return Some((op.to_string(), "argument", at.render_name(&db.name_ctx())));
            }
        }
        // Descend the args too (a host call may be nested in an arg).
        for &a in args.iter() {
            if let Some(hit) = first_unrepresentable_host_op(db, a, allow_option_bytes) {
                return Some(hit);
            }
        }
        return None;
    }
    if let crate::ast::Struct::List(children) = db.ast.get(id).clone() {
        for c in children {
            if let Some(hit) = first_unrepresentable_host_op(db, c, allow_option_bytes) {
                return Some(hit);
            }
        }
    }
    None
}

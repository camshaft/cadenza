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
    /// A WIT `flags{…}` parameter — a bitset the guest models as a RECORD-of-bools (operator ruling: flags is a
    /// PRODUCT). Its component valtype is a `flags` DEFINED type (nominal, like [`Enum`](HostParam::Enum), laid
    /// and exported by `build_host_group`'s flags_params branch), flattening to `ceil(n/32)` `i32` bitset words
    /// (`wit_ctype::flatten` of `WitType::Flags`). The guest PACKS its bool fields into the word(s):
    /// `select::emit_flags_arg_pack` reads each bool field (`arr-get` then `get-bool`) and shifts it into the bit
    /// its label maps to. Carries `field_bits`, the `(guest name-lex slot, flags bit index)` per field, matched by
    /// NAME to the WIT label order (the inverse of `param_field`'s flags-UNPACK arm), and the kebab `labels`
    /// (for the `CDef::Flags` component type). Scoped to ≤32 labels (a single `i32` word) this increment.
    Flags {
        field_bits: Vec<(u32, u32)>,
        labels: Vec<String>,
    },
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
    /// A bare `variant{nullary…, bytes-case(s)}` param (the top-level position, not nested in a record/list) —
    /// a variant whose payload cases each carry a `Bytes`/`String` (`list<u8>`), the rest nullary. Crosses as
    /// the declared `variant` DEFINED type (laid STRUCTURALLY from the op's WIT via `add_wit_type_deduped` →
    /// `CDef::Variant`, so a `(list u8)` payload is expressed — the same structural path a `Result`/`List` param
    /// rides, NOT the scalar `Variant`'s nominal-`AbiValType` builder which cannot express a Bytes payload). Its
    /// core form flattens (canonical variant flatten) to `(disc:i32, ptr:i32, len:i32)` — the SAME 3-slot shape
    /// as [`Result`](HostParam::Result): on a Bytes case the guest copies the payload rope into `mem` at the
    /// running cursor and pushes `(disc, ptr, len)`; on a nullary case pushes `(disc, 0, 0)`. Carries the
    /// Bytes-payload cases' DISCRIMINANTS (declaration = component order). The guest marshals it via
    /// `select::emit_variant_bytes_arg_reg_flatten`. A mixed scalar+Bytes / compound payload variant is a later
    /// increment (`variant_bytes_payload_cases` requires every payload case to be `Bytes`/`String`).
    VariantBytes(Vec<i32>),
    /// A bare `variant{nullary…, list<scalar>-case(s)}` param — a variant whose payload cases each carry a
    /// `list<scalar>` (all sharing the same scalar element), the rest nullary. The `list` sibling of
    /// [`VariantBytes`](HostParam::VariantBytes): the SAME `(disc:i32, ptr:i32, count:i32)` 3-slot flatten, but on
    /// a list case the guest MARSHALS the payload list into `mem` at the running cursor via `emit_list_arg_marshal`
    /// (`vec-len`/`vec-get` + the scalar element) rather than a Bytes rope-copy; a nullary case pushes
    /// `(disc, 0, 0)`. Crosses as the declared `variant` DEFINED type (laid structurally from the WIT via
    /// `add_wit_type_deduped` → `CDef::Variant` with a `(list <elem>)` payload case). Carries the list-payload
    /// cases' DISCRIMINANTS (declaration = component order); the element type is re-derived from the arg `Ty`. The
    /// guest marshals it via `select::emit_variant_list_arg_reg_flatten`. A `list<compound>` element / mixed
    /// element types / a mixed scalar+Bytes+list payload set is a later increment.
    VariantList(Vec<i32>),
    /// A bare `variant{nullary…, one tuple-of-scalars case}` param — a variant with exactly one case carrying a
    /// `tuple` of scalars, the rest nullary. The PRODUCT-payload sibling of [`VariantBytes`](HostParam::VariantBytes)
    /// / [`VariantList`](HostParam::VariantList), but with a VARIABLE positional flatten `(disc:i32, e0, e1, …)` —
    /// the discriminant then the tuple's elements INLINE (element = component order) — rather than a fixed 3 slots;
    /// the register twin of [`ResultTuple`](HostParam::ResultTuple)'s Ok arm minus the err-disc (a nullary case
    /// zero-fills ALL payload slots). All-scalar → NO `mem`. Crosses as the declared `variant` DEFINED type (laid
    /// structurally from the WIT via `add_wit_type_deduped` → `CDef::Variant` with a `(tuple <e>…)` payload case).
    /// Carries the tuple case's DISCRIMINANT + the element ABIs (for the flatten slot widths). The guest marshals it
    /// via `select::emit_variant_tuple_arg_reg_flatten`. A compound/bytes tuple element, a second product case, or a
    /// record payload is a later increment.
    VariantTuple(i32, Vec<RecordFieldAbi>),
    /// A bare `variant{nullary…, one record-of-scalars case}` param — the RECORD sibling of
    /// [`VariantTuple`](HostParam::VariantTuple). Same VARIABLE positional flatten `(disc:i32, f0, f1, …)`, but the
    /// fields are WIT-REORDERED (guest field order is name-lex; the component + marshal use the host WIT's
    /// declaration order) exactly as [`ResultRecord`](HostParam::ResultRecord). All-scalar → NO `mem`. Crosses as
    /// the declared `variant` DEFINED type (laid structurally from the WIT via `add_wit_type_deduped` →
    /// `CDef::Variant` with a `(record …)` payload case). Carries the record case's DISCRIMINANT + the field
    /// (name, ABI) pairs already REORDERED to WIT declaration order (so `serialize` + the marshal agree). The guest
    /// marshals it via `select::emit_variant_record_arg_reg_flatten`. A compound field / second product case is a
    /// later increment.
    VariantRecord(i32, Vec<(String, RecordFieldAbi)>),
    /// A bare `variant{nullary…, scalar-case(s), bytes-case(s)}` param — the canonical HETEROGENEOUS tagged-union
    /// mixing at least one `Scalar` payload case with at least one `Bytes` payload case (rest nullary). Its core
    /// form is the canonical variant JOIN `[disc:i32] ++ position-wise-join(payload flattens)` — a `Scalar` case
    /// contributes one slot, a `Bytes` case `(i32 ptr, i32 len)`, joined slot-wise (mixed int widths → `i64`;
    /// [`variant_mixed_join_slots`]). The guest marshals it via `select::emit_variant_mixed_arg_reg_flatten`
    /// (branch per case: scalar → unbox into slot 0 coerced to the joined width, rest 0; bytes → rope-copy at the
    /// cursor → `(ptr,len)`; nullary → zero). Crosses as the declared `variant` DEFINED type (structural WIT).
    /// Carries each payload case's `(disc, kind)`. A list/tuple/record payload case is a later increment; an
    /// int↔float scalar mix is [`VariantScalarsMixed`](HostParam::VariantScalarsMixed).
    VariantMixed(Vec<(i32, VariantPayloadKind)>),
    /// A bare `variant{nullary…, scalar-case(s)}` param whose scalar payload cases MIX an integer with a float
    /// (or `f32` with `f64`) — the canonical REINTERPRET-JOIN tagged union the uniform-join
    /// [`Variant`](HostParam::Variant) declines. Its core form is the canonical variant flatten `(disc:i32,
    /// join)` where `join` is `wit_ctype::flatten_variant`'s reinterpret join of the payload slots (a same-width
    /// int/float → that int width, e.g. `join(s64,f64)=i64`; anything else → `i64`), always an INTEGER slot when
    /// a float is mixed in. The guest marshals it via `select::emit_variant_mixed_scalar_arg_reg_flatten` (branch
    /// per payload case: unbox with THAT case's read op, then COERCE the runtime value into the join slot — a
    /// float bit-reinterprets [`i64.reinterpret_f64`/`i32.reinterpret_f32`], a narrow int wraps/extends; a nullary
    /// case pushes the join-width zero). All-scalar → NO `mem`. Crosses as the declared `variant` DEFINED type via
    /// the SAME nominal-`AbiValType` builder as [`Variant`](HostParam::Variant) (`comp_byte` expresses f32/f64), so
    /// it carries the FULL case list `(name, Option<scalar>)` in declaration order and rides Variant's host_imports
    /// nominal path. A `Bytes`/list/tuple/record payload case is a different flavor (`VariantBytes`/`VariantMixed`/…).
    VariantScalarsMixed(Vec<(String, Option<AbiValType>)>),
    /// A bare `option<scalar>` param (the top-level position, not nested in a record/list) — crosses as the
    /// built-in WIT `option<T>` type (NOT a nominal `variant` DEFINED type; a `variant{none,some(T)}` substitute
    /// fails the structural component-link match against a host declaring `option`), referenced by a per-param
    /// structural `CRef` (like [`List`](HostParam::List)). Its core form flattens (canonical variant flatten) to
    /// `(disc:i32, payload)` — the SAME core shape as an `option<scalar>` record FIELD. Carries the payload's
    /// scalar ABI. The guest marshals it via `select::emit_option_reg_flatten` (the register twin of the
    /// `RecordFieldAbi::Option` field flatten), mapping the guest Option's some-disc to the WIT `option` some=1
    /// / none=0. An `option<compound>` (bytes/record) top-level arg is a later increment (declined — the
    /// classifier only pushes this for a scalar payload, leaving `params` short otherwise).
    Option(Box<RecordFieldAbi>),
    /// A bare `tuple<scalar…>` param (the top-level position, not nested in a record/list) — crosses as the
    /// built-in WIT `tuple<T…>` type (structural, anonymous-allowed), referenced by a per-param structural
    /// `CRef` (like [`List`](HostParam::List)/[`Option`](HostParam::Option)). Its core form flattens
    /// POSITIONALLY INLINE — one scalar core slot per element, in element (= declaration = component) order,
    /// no discriminant (unlike Option/Variant) — the SAME core shape as a `tuple<…>` record FIELD. Carries the
    /// elements' scalar ABIs. The guest marshals it via `select::emit_tuple_reg_flatten` (`arr-get i` +
    /// unbox per element). A `tuple` with a COMPOUND element (bytes/record/list) is a later increment
    /// (declined — the classifier only pushes this for an ALL-SCALAR tuple, leaving `params` short otherwise).
    Tuple(Vec<RecordFieldAbi>),
    /// A bare `result<list<u8>, enum>` param (the top-level position, not nested in a record/list) — crosses as
    /// the built-in WIT `result<list<u8>, err-enum>` type, referenced by a per-param structural `CRef` (like
    /// [`Option`](HostParam::Option)). Its core form flattens (canonical variant flatten) to `(disc:i32, i32,
    /// i32)` — the SAME core shape as a `result<list<u8>, enum>` record FIELD ([`RecordFieldAbi::Result`]): the
    /// discriminant then the join of the Ok arm `(ptr,len)` and the Err arm `(enum-disc, 0)`. Carries the err
    /// enum's case names (kebab, DECLARATION = discriminant order). The guest marshals it via a register flatten
    /// (the twin of the result record-FIELD arm): Ok copies the Bytes rope into `mem` at the running cursor +
    /// pushes `(ptr,len)`, Err pushes `(err-enum-disc, 0)`. A non-`Bytes` ok arm / a `variant` err arm is a later
    /// increment (declined — the classifier only pushes this for `result_bytes_enum`).
    Result(Vec<String>),
    /// A bare `result<scalar, enum>` param (the top-level position, not nested in a record/list) — crosses as
    /// the built-in WIT `result<ok-scalar, err-enum>` type, referenced by a per-param structural `CRef` (like
    /// [`Result`](HostParam::Result)). Its core form flattens (canonical variant flatten) to `(disc:i32, join)`
    /// — the discriminant then the reinterpret join of the Ok arm's scalar and the Err arm's enum discriminant
    /// (`join` is `i64` iff the Ok scalar is 64-bit, else `i32`; the `i32` err disc widens to fit). This is 2
    /// core slots, NOT the 3-slot `list<u8>` Ok shape ([`Result`](HostParam::Result)) — there is no rope, so it
    /// needs NO shared `mem`. Carries the Ok scalar's ABI and the err enum's case names (kebab, DECLARATION =
    /// discriminant order). The guest marshals it via `select::emit_result_scalar_arg_reg_flatten`: Ok unboxes
    /// the scalar payload, Err reads the err enum's disc, both into the shared join slot. A non-integer Ok
    /// (float — needs the reinterpret join lattice) / a `variant` err arm is a later increment (declined — the
    /// classifier only pushes this for `result_scalar_enum`, which admits integer-width Ok scalars).
    ResultScalar(AbiValType, Vec<String>),
    /// A bare `result<record-of-scalars, enum>` param (the top-level position, not nested) — crosses as the
    /// built-in WIT `result<record, err-enum>` type, referenced by a per-param structural `CRef` (like
    /// [`ResultScalar`](HostParam::ResultScalar)). Its core form flattens (canonical variant flatten) to
    /// `(disc:i32, join(record-fields, err-disc))` — the discriminant then the record's fields POSITIONALLY (in
    /// host WIT declaration order), with the `i32` err discriminant riding the FIRST field's slot on the Err arm.
    /// Because every Ok field is a SCALAR, joining it with the `i32` err disc never widens beyond the field's own
    /// width (an `i32` field stays `i32`; an `i64` field stays `i64` and the err disc widens into it), so the slot
    /// widths are exactly the record's field widths — no `mem` (a record-of-scalars flattens to registers, unlike
    /// a Bytes/list field). Carries `(field-name, field-ABI)` per Ok field in WIT declaration order (so the core
    /// flatten + component type derive from it) and the err enum's case names. The guest marshals it via
    /// `select::emit_result_record_arg_reg_flatten`: Ok recurses `emit_record_arg_marshal` on the payload record
    /// (its N pushes captured into the join slots), Err puts the err enum's disc in the first slot + zero-fills
    /// the rest. A record with a COMPOUND field (Bytes/list/nested) is a later increment (needs the in-mem
    /// marshal); the classifier only pushes this for `result_record_enum` (all-scalar Ok fields).
    ResultRecord(Vec<(String, RecordFieldAbi)>, Vec<String>),
    /// A bare `result<tuple-of-scalars, enum>` param (the top-level position, not nested) — crosses as the
    /// built-in WIT `result<tuple<T…>, err-enum>` type, referenced by a per-param structural `CRef` (like
    /// [`ResultRecord`](HostParam::ResultRecord)). The tuple analogue of the record-Ok result: its core form
    /// flattens (canonical variant flatten) to `(disc:i32, elem0, elem1, …)` — the discriminant then the Ok
    /// tuple's elements POSITIONALLY (element = declaration = component order, NO name-lex/WIT reorder — a tuple
    /// is positional, unlike a record), with the `i32` err discriminant riding the FIRST element's slot on the
    /// Err arm. Only slot 0 joins the `i32` err disc (the payloadless-enum Err flattens to a single `i32`); an
    /// integer/ptr first slot absorbs it by widening to `i64`, so slot 0 must NOT be a float — a float FIRST
    /// element would need the canonical reinterpret join, which `result_tuple_enum` declines (a float in a LATER
    /// element is fine — it rides its own `f64` slot, zero-filled on Err). The guest marshals it via
    /// `select::emit_result_tuple_arg_reg_flatten`: Ok recurses `emit_tuple_reg_flatten` on the payload tuple (its
    /// N pushes captured into the join slots), Err puts the err enum's disc in the first slot + zero-fills the
    /// rest. Carries each Ok element's boundary ABI (positional, no field names — a tuple is positional) so the
    /// core flatten + component type derive from it; an element may be a scalar OR any compound
    /// `emit_tuple_reg_flatten` handles (Bytes/list/record/nested-tuple/option), a mem-writing element forcing
    /// `set_needs_memory` + the cursor like the record result.
    ResultTuple(Vec<RecordFieldAbi>, Vec<String>),
    /// A bare `result<list<scalar>, enum>` param (the top-level position, not nested) — crosses as the built-in
    /// WIT `result<list<T>, err-enum>` type, referenced by a per-param structural `CRef`. The list-Ok sibling of
    /// the Bytes-Ok result ([`Result`](HostParam::Result)): its core form flattens to the SAME 3 slots
    /// `(disc:i32, ptr/errdisc:i32, count/0:i32)` — on Ok the guest marshals the value-heap `List` into shared
    /// `mem` (an outer `count`-slot array at the running cursor, each element inline) via `emit_list_arg_marshal`
    /// and passes `(outer-ptr, count)`; on Err it passes `(err-enum-disc, 0)`. Unlike the register-only
    /// scalar/record/tuple results, this DOES need `mem` + the scratch cursor (it copies the list into linear
    /// memory, like the Bytes result copies a rope). Carries the err enum's case names; the element `Ty` +
    /// component `result<list<T>, err>` type come from the arg type / declared WIT. The guest marshals it via
    /// `select::emit_result_list_arg_reg_flatten`. A non-scalar list element (`list<record/tuple/bytes/list>`) is
    /// a later increment; the classifier only pushes this for `result_list_enum` (scalar element).
    ResultList(Vec<String>),
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
        // A tuple-payload `variant` reaches `(list u8)` iff any tuple element does (all-scalar → false).
        RecordFieldAbi::VariantTuple { elem_abis, .. } => {
            elem_abis.iter().any(record_field_abi_reaches_bytes)
        }
        // A heterogeneous `variant` reaches `(list u8)` iff a payload case is `Bytes`/`List` (a `Scalar`/`Tuple`
        // -of-scalars case never does; this increment's scope is Scalar/Tuple, so this is `false` in practice).
        RecordFieldAbi::VariantMemMixed(cases) => cases.iter().any(|(_, k)| {
            matches!(
                k,
                Some(VariantPayloadKind::Bytes | VariantPayloadKind::List(_))
            )
        }),
        // A payload-less `enum` is a bare disc — never reaches `(list u8)`.
        RecordFieldAbi::Enum(_) => false,
        // A `flags` field packs into i32 bitset word(s) — never reaches `(list u8)`.
        RecordFieldAbi::Flags { .. } => false,
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
        // A tuple-payload `variant` FIELD flattens (canonical variant flatten) POSITIONALLY to `(disc, e0, e1,
        // …)` core slots — no memory, like the scalar `Variant`. When it is written to MEMORY instead (as a
        // `list<…variant-tuple…>` element / nested under a list), the enclosing `HostParam::List(_) => true` arm
        // forces the shared memory; a variant-tuple never reaches mem OUTSIDE a list (a top-level record/tuple
        // arg is register-flattened), so this stays `false` and avoids a spurious memory on a pure-register op.
        RecordFieldAbi::VariantTuple { .. } => false,
        // A heterogeneous `variant` FIELD needs memory iff it has a `Bytes`/`List` payload case — that case
        // rope-copies / marshals its payload into shared `mem` at the cursor when the field is register-flattened
        // (`emit_variant_mixed_arg_reg_flatten`'s Bytes/List arms), so the enclosing `HostParam::Record` must force
        // the shared-memory core module + the canon `Lower`'s `Memory` option. A scalar/tuple/record-only mixed
        // variant field flattens to pure core slots (no memory).
        RecordFieldAbi::VariantMemMixed(cases) => cases.iter().any(|(_, k)| {
            matches!(
                k,
                Some(VariantPayloadKind::Bytes) | Some(VariantPayloadKind::List(_))
            )
        }),
        // A payload-less `enum` flattens to a single `i32` disc — no memory.
        RecordFieldAbi::Enum(_) => false,
        // A `flags` field packs into i32 bitset word(s) inline — no memory.
        RecordFieldAbi::Flags { .. } => false,
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
    /// A general `variant { c0, c1(tuple<…>), … }` field whose ONE payload-bearing case carries a TUPLE of
    /// scalars (the rest nullary) — the compound-payload sibling of [`Variant`](RecordFieldAbi::Variant). Carries
    /// the case NAMES (kebab, DECLARATION = discriminant order), the tuple case's DISCRIMINANT, and the tuple's
    /// element ABIs (all scalar this increment). As a LIST element or a mem PRODUCT field it is written in place at
    /// its canonical variant layout (disc + the payload tuple at the payload offset) by
    /// `select::emit_variant_to_mem`'s tuple arm — so it MARSHALS INTO MEMORY (needs mem). As a REGISTER-flattened
    /// RECORD field it flattens (canonical variant flatten) POSITIONALLY to `(disc:i32, e0, e1, …)` via
    /// `select::emit_variant_tuple_arg_reg_flatten`; its component `variant` type (with the tuple case's `(tuple
    /// <elem>…)` payload) is built by [`record_field_cref`]. The element ABIs let [`record_field_abi_reaches_bytes`]
    /// recurse (all-scalar → no `(list u8)`).
    VariantTuple {
        case_names: Vec<String>,
        tuple_disc: u32,
        elem_abis: Vec<RecordFieldAbi>,
    },
    /// A HETEROGENEOUS `variant` FIELD whose payload cases MIX scalar and tuple-of-scalars kinds (e.g.
    /// `variant{a, b(s64), c(tuple<s32,s64>)}`) — the general per-case sibling of [`Variant`] (uniform scalar) and
    /// [`VariantTuple`] (single tuple). Carries the case NAMES (declaration order) paired with each case's payload
    /// KIND (`None` = nullary). As a LIST element / mem product field it is written in place at its canonical
    /// variant layout (disc + the SELECTED case's payload at the payload offset) by
    /// `select::emit_variant_mixed_to_mem`. This increment scopes the payload kinds to `Scalar`/`Tuple` — a
    /// `Bytes`/`List`/record payload case declines. At a REGISTER-flattened field position it is NOT emitted and
    /// declines honestly (the bare-ARG mix rides `HostParam::VariantMixed`, a distinct path).
    VariantMemMixed(Vec<(String, Option<VariantPayloadKind>)>),
    /// A payload-less `enum` field (a `Sum` whose every variant is nullary) — crosses as a component `enum`
    /// DEFINED type, ONE `i32` core slot (the discriminant, in declaration = discriminant order). The guest
    /// reads the value-heap sum's `sum-disc` (a payloadless enum's in-guest rep is a bare disc) and writes it
    /// inline — no payload, no `mem`. Carries the case names (kebab, declaration order). The nested (record
    /// FIELD) analogue of the top-level [`HostParam::Enum`] arg.
    Enum(Vec<String>),
    /// A WIT `flags{…}` FIELD — a bitset the guest models as a nested RECORD-of-bools. Crosses as a component
    /// `flags` DEFINED type (laid structurally from the record's WIT by `add_wit_type_deduped`), flattening to
    /// `ceil(labels/32)` `i32` bitset word(s) — ONE word (≤32 labels, the Component Model cap). The guest PACKS
    /// the nested bool-record's fields into the word via `select::emit_flags_arg_pack` (`arr-get`+`get-bool` per
    /// field, shifted into its label bit). Carries `field_bits` — the `(nested name-lex slot, flags bit)` per
    /// field, matched by NAME to the WIT labels — and the kebab `labels`. The record-FIELD analogue of the
    /// top-level [`HostParam::Flags`] arg; constructed by `reorder_record_fields_to_wit` (the only place with
    /// both the field abi + its WIT), which converts a bool-record field whose WIT field is `flags`.
    Flags {
        field_bits: Vec<(u32, u32)>,
        labels: Vec<String>,
    },
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

/// Whether `ty` is `result<scalar, enum>` — an Ok arm carrying an INTEGER-width scalar (not `Bytes`, whose own
/// arm is [`result_bytes_enum`]) and an Err arm that is a PAYLOAD-LESS enum. Returns `(ok-scalar-abi, err-enum
/// case names)` if so, else `None`. A `Sum` whose decl has exactly `Ok`/`Err` variants, instantiated at
/// `[ok-scalar, err-enum]`. The Ok scalar joins the `i32` err discriminant in the single value slot: an integer
/// widens (to `i64` if 64-bit), and a FLOAT bit-reinterprets into the join int (`join(f64,i32)=i64` /
/// `join(f32,i32)=i32`). The `(disc, join)` register flatten is `emit_result_scalar_arg_reg_
/// flatten`. Reads through erased nominal wrappers, mirroring [`result_bytes_enum`].
pub fn result_scalar_enum(db: &mut Db, ty: &Ty) -> Option<(AbiValType, Vec<String>)> {
    use crate::backend::common::export_name::kebab_extern_name;
    let stripped = ty.strip_nominal();
    let Ty::Sum { decl, args } = stripped else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    // The Ok arm (args[0]) must be a scalar. A float Ok is admitted: the canonical result flatten joins the
    // Ok's single slot with the `i32` err disc, and for a float that join is the reinterpret lattice —
    // `join(f64,i32)=i64` / `join(f32,i32)=i32` — which the marshal emits by bit-reinterpreting the float
    // into the (integer) join slot on the Ok arm (`emit_result_scalar_arg_reg_flatten`).
    let ok = abi_val_type(&args[0])?;
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
    let err_cases = ed
        .variants
        .iter()
        .map(|v| kebab_extern_name(&v.name))
        .collect();
    Some((ok, err_cases))
}

/// Whether `ty` is `result<record-of-scalars, enum>` — an Ok arm that is a RECORD every one of whose fields is a
/// SCALAR (so it flattens to registers, no `mem`; a compound field is a later increment) and an Err arm that is a
/// PAYLOAD-LESS enum. Returns `(the Ok record Ty, err-enum case names)` if so, else `None`. A `Sum` whose decl
/// has exactly `Ok`/`Err` variants, instantiated at `[record, enum]`. The register flatten is
/// `emit_result_record_arg_reg_flatten`; the classifier reorders the Ok fields to WIT order. Reads through erased
/// nominal wrappers, mirroring [`result_scalar_enum`].
pub fn result_record_enum(db: &mut Db, ty: &Ty) -> Option<(Ty, Vec<String>)> {
    use crate::backend::common::export_name::kebab_extern_name;
    let stripped = ty.strip_nominal();
    let Ty::Sum { decl, args } = stripped else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    // The Ok arm (args[0]) must be a boundary RECORD — every field crosses (`is_boundary_record`, the SAME admit
    // set the direct record ARG uses: scalar / Bytes / list / nested record / tuple / option / variant). Each
    // field is marshalled by `emit_record_arg_marshal` (a scalar inline, a Bytes/list field into `mem` at the
    // cursor, a nested product recursively) — the guest side is identical to a bare record arg, just wrapped in
    // the result's Ok arm. A `mem`-writing field forces `set_needs_memory` + the scratch cursor (like the direct
    // record arg + the Bytes/list result).
    let ok_record = args[0].clone();
    if !is_boundary_record(db, ok_record.strip_nominal()) {
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
    let err_cases = ed
        .variants
        .iter()
        .map(|v| kebab_extern_name(&v.name))
        .collect();
    Some((args[0].clone(), err_cases))
}

/// Whether `ty` is `result<list<T>, enum>` where the list ELEMENT is a SCALAR or an all-scalar product (a
/// record every field of which is a scalar, or a tuple every element of which is a scalar) — so the list marshals
/// into `mem` with `emit_list_arg_marshal` and NEVER reaches `list<u8>` (keeping `has_list_param` = false). An
/// element with a `Bytes`/`list`/`option`/nested compound is a later increment (it reaches `list<u8>` / needs the
/// shared list type). The Err arm must be a PAYLOAD-LESS enum. Returns `(the element Ty, err-enum case names)` if
/// so, else `None`. A `Sum` whose decl has exactly `Ok`/`Err` variants, instantiated at `[list, enum]`. The
/// 3-slot `(disc, ptr/errdisc, count/0)` flatten is `emit_result_list_arg_reg_flatten` (the list-Ok twin of the
/// Bytes-Ok `emit_result_arg_reg_flatten`). Reads through erased nominal wrappers, mirroring [`result_tuple_enum`].
/// NB: a `list<u8>` Ok is `Bytes` → that is [`result_bytes_enum`]'s job (a `list<u8>` arg type is `Ty::Bytes`).
pub fn result_list_enum(db: &mut Db, ty: &Ty) -> Option<(Ty, Vec<String>)> {
    use crate::backend::common::export_name::kebab_extern_name;
    let stripped = ty.strip_nominal();
    let Ty::Sum { decl, args } = stripped else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    // The Ok arm (args[0]) must be a `list<T>` whose element the shared list marshal handles — the SAME element
    // capability a `list<T>` ARG uses (`list_elem_marshalable`: a scalar, `Bytes`, a record/tuple product, a
    // nested `list`, an `option<…>`, or a `result<list<u8>,enum>`). `emit_list_arg_marshal` writes each element
    // inline (scalar) / at its canonical layout (`emit_record_to_mem` / `emit_tuple_to_mem` / a nested-list
    // header / `emit_option_to_mem`) — so the whole list marshals identically whether it is a bare arg or the Ok
    // arm of a result. The per-param component `result<list<T>, enum>` type is built structurally from the declared
    // WIT (so a `list<u8>`-reaching element needs no `has_list_param` shared-type change — `ResultList` rides the
    // structural-CRef path, not the fallback `(list u8)` index). NB: a `list<u8>` Ok is `Bytes` → `result_bytes_
    // enum`'s job (a `list<u8>` arg type is `Ty::Bytes`, not `Ty::List`, so this never sees it).
    let Ty::List(elem) = args[0].strip_nominal() else {
        return None;
    };
    let elem = (**elem).clone();
    if !(abi_val_type(&elem).is_some() || list_elem_marshalable(db, &elem)) {
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
    let err_cases = ed
        .variants
        .iter()
        .map(|v| kebab_extern_name(&v.name))
        .collect();
    Some((elem, err_cases))
}

/// Whether `ty` is `result<tuple, enum>` — an Ok arm that is a TUPLE every element of which is a boundary field
/// (`field_boundary_abi` — scalar/bytes/list/nested-compound) and an Err arm that is a PAYLOAD-LESS enum. The
/// `i32` err disc joins slot 0; an integer/ptr first slot absorbs it, and a FLOAT first element bit-reinterprets
/// into the int join (`join(f64,i32)=i64`, `join(f32,i32)=i32`) in the marshal (a float in a LATER element rides
/// its own float slot). Returns `(the Ok tuple Ty, err-enum case names)` if so, else `None`. A
/// `Sum` whose decl has exactly `Ok`/`Err` variants, instantiated at `[tuple, enum]`. The register flatten is
/// `emit_result_tuple_arg_reg_flatten` (positional — no field reorder). Reads through erased nominal wrappers,
/// mirroring [`result_record_enum`].
pub fn result_tuple_enum(db: &mut Db, ty: &Ty) -> Option<(Ty, Vec<String>)> {
    use crate::backend::common::export_name::kebab_extern_name;
    let stripped = ty.strip_nominal();
    let Ty::Sum { decl, args } = stripped else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    // The Ok arm (args[0]) must be a TUPLE with ≥1 element, every element a boundary-marshalable
    // field (any shape `field_boundary_abi` admits — scalar, bytes, list, or a nested compound —
    // symmetric with the record Ok arm; a compound element rides the in-mem marshal via a cursor).
    let Ty::Tuple(elems) = args[0].strip_nominal() else {
        return None;
    };
    if elems.is_empty() {
        return None;
    }
    let elems = elems.clone(); // release the borrow of `args`/`ty` before the `&mut db` calls
    // Every element must cross (`field_boundary_abi`). A FLOAT first element is admitted: the payloadless-enum
    // Err arm flattens to a single `i32`, so the result flatten joins that `i32` with ONLY the Ok payload's
    // FIRST slot; for a float that join is the reinterpret lattice (`join(f64,i32)=i64`, `join(f32,i32)=i32`),
    // which `emit_result_tuple_arg_reg_flatten` emits by bit-reinterpreting the first element into the (integer)
    // slot-0 join. A float in a LATER element never joins the disc — it rides its own `f64`/`f32` slot.
    for ety in elems.iter() {
        field_boundary_abi(db, ety)?;
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
    let err_cases = ed
        .variants
        .iter()
        .map(|v| kebab_extern_name(&v.name))
        .collect();
    Some((args[0].clone(), err_cases))
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
    let cases = variant_all_scalar_cases(db, ty)?;
    // REJECT a payload set that mixes int with float, or f32 with f64 — those need the canonical reinterpret
    // join lattice, handled by the sibling [`variant_mixed_scalar_payload_cases`] / `HostParam::VariantScalarsMixed`.
    // All-integer widths (incl bool/char, which share the i32 slot) join cleanly to the widest int slot, and a
    // uniform float is fine — those stay on this uniform-join scalar-variant path.
    if scalar_cases_have_reinterpret_mix(&cases) {
        return None;
    }
    Some(cases)
}

/// Every payload case of a scalar-payload `variant` shares one core-join family (all-int or uniform-float), so
/// `true` iff the payload abis MIX an integer with a float, or an `f32` with an `f64` — the case the uniform
/// [`variant_scalar_payload_cases`] declines and the reinterpret-join [`variant_mixed_scalar_payload_cases`]
/// claims. Nullary cases (payload `None`) contribute nothing.
fn scalar_cases_have_reinterpret_mix(cases: &[(String, Option<AbiValType>)]) -> bool {
    let (mut has_int, mut has_f32, mut has_f64) = (false, false, false);
    for pv in cases.iter().filter_map(|(_, p)| *p) {
        match pv {
            AbiValType::F32 => has_f32 = true,
            AbiValType::F64 => has_f64 = true,
            _ => has_int = true,
        }
    }
    (has_int && (has_f32 || has_f64)) || (has_f32 && has_f64)
}

/// The case list of a scalar-payload `variant` boundary type WITHOUT the uniform-join restriction — a `Sum`
/// that is NOT option-shaped ([`option_payload_ty`]) nor `result<Bytes,enum>` ([`result_bytes_enum`]), with AT
/// LEAST ONE payload case, every case nullary or a SINGLE scalar payload (any int/float width). Returns
/// `(kebab-case-name, Option<payload-scalar>)` per case in DECLARATION (= discriminant) order, else `None` (a
/// payloadless enum, a `Bytes`/compound/multi-payload case). Shared by [`variant_scalar_payload_cases`] (which
/// then rejects an int↔float / f32↔f64 mix) and [`variant_mixed_scalar_payload_cases`] (which requires it).
pub fn variant_all_scalar_cases(db: &mut Db, ty: &Ty) -> Option<Vec<(String, Option<AbiValType>)>> {
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
    let mut any_payload = false;
    for (disc, (name, n)) in variants.into_iter().enumerate() {
        match n {
            0 => cases.push((name, None)),
            1 => {
                let pty = crate::backend::wasm::select::variant_payload_ty_at(db, ty, disc as u32)?;
                let pv = abi_val_type(&pty)?; // a scalar payload only
                any_payload = true;
                cases.push((name, Some(pv)));
            }
            _ => return None, // a multi-payload case → a later increment
        }
    }
    any_payload.then_some(cases)
}

/// The case list of a scalar-payload `variant` whose payloads MIX an integer with a float (or `f32` with `f64`)
/// — the canonical reinterpret-join tagged union (`HostParam::VariantScalarsMixed`). Same admission as
/// [`variant_all_scalar_cases`] (every case nullary or one SCALAR payload, ≥1 payload, not option/result-shaped),
/// but ONLY when the mix the uniform [`variant_scalar_payload_cases`] declines is present — so the two detectors
/// are DISJOINT (a uniform-int / uniform-float variant stays on the scalar path). Returns `(kebab-case-name,
/// Option<payload-scalar>)` per case in DECLARATION (= discriminant) order. The register-flatten join slot is the
/// canonical `wit_ctype::flatten_variant` reinterpret join (a same-width int/float → that int width; anything
/// else → `i64`); a float payload bit-reinterprets into the integer slot at marshal time.
pub fn variant_mixed_scalar_payload_cases(
    db: &mut Db,
    ty: &Ty,
) -> Option<Vec<(String, Option<AbiValType>)>> {
    let cases = variant_all_scalar_cases(db, ty)?;
    scalar_cases_have_reinterpret_mix(&cases).then_some(cases)
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

/// ARG-SIDE: whether `ty` is a variant each of whose cases is nullary or carries ONE `Bytes`/`String` payload,
/// with AT LEAST ONE such Bytes-payload case. Returns the DISCRIMINANTS (declaration = component order) of the
/// Bytes-payload cases. This is the ARG marshal's admission for a `variant{nullary…, bytes-case(s)}` — the
/// register-flatten twin of [`result_bytes_enum`], flattening to `(disc:i32, ptr:i32, len:i32)`: the guest
/// `select::emit_variant_bytes_arg_reg_flatten` reads the disc and, on a Bytes case, copies the payload rope
/// into `mem` at the running cursor + pushes `(disc, ptr, len)`; on a nullary case pushes `(disc, 0, 0)`. The
/// component boundary type is the declared `variant` DEFINED type, laid STRUCTURALLY from the op's WIT
/// (`add_wit_type_deduped` → `CDef::Variant`), so a `(list u8)` payload case is expressed there. DISTINCT from
/// [`variant_scalar_payload_cases`] (scalar payloads, no `mem`) — a variant that MIXES a scalar payload with a
/// Bytes payload is a later increment (this requires every payload case to be `Bytes`/`String`, so the single
/// rope-copy marshal covers them all). Excludes option/result-shaped sums (their own arms).
pub fn variant_bytes_payload_cases(db: &mut Db, ty: &Ty) -> Option<Vec<i32>> {
    let Ty::Sum { decl, .. } = ty.strip_nominal() else {
        return None;
    };
    if option_payload_ty(db, ty).is_some() || result_bytes_enum(db, ty).is_some() {
        return None;
    }
    let decl = *decl;
    let payload_counts: Vec<usize> = {
        let d = db.type_decl_by_occ(decl)?;
        d.variants.iter().map(|v| v.payloads.len()).collect()
    };
    let mut bytes_discs = Vec::new();
    for (disc, n) in payload_counts.into_iter().enumerate() {
        match n {
            0 => {} // a nullary case → no payload slot
            1 => {
                let pty = crate::backend::wasm::select::variant_payload_ty_at(db, ty, disc as u32)?;
                // Every payload case MUST be Bytes/String (the single rope-copy marshal covers only Bytes); a
                // scalar/compound payload case → not this increment (returns None → the arg declines cleanly).
                if !matches!(pty.strip_nominal(), Ty::Bytes | Ty::String) {
                    return None;
                }
                bytes_discs.push(disc as i32);
            }
            _ => return None, // a multi-payload case → a later increment
        }
    }
    (!bytes_discs.is_empty()).then_some(bytes_discs)
}

/// ARG-SIDE: whether `ty` is a variant each of whose cases is nullary or carries ONE `list<scalar>` payload, all
/// list cases sharing the SAME scalar element type, with AT LEAST ONE such list case. Returns the list-payload
/// cases' DISCRIMINANTS (declaration = component order) paired with the shared element `Ty`. The `list` sibling
/// of [`variant_bytes_payload_cases`]: the same `(disc:i32, ptr:i32, count:i32)` 3-slot register-flatten, but on a
/// list case the guest MARSHALS the payload list into `mem` at the running cursor via `emit_list_arg_marshal`
/// (`vec-len`/`vec-get` + the scalar element) rather than a Bytes rope-copy. The component boundary type is the
/// declared `variant` DEFINED type laid STRUCTURALLY from the WIT (`add_wit_type_deduped` → `CDef::Variant` with a
/// `(list <elem>)` payload case). Requiring a SHARED element type lets the single `emit_list_arg_marshal` call
/// handle whichever list case fired (the payload handle is a list regardless of disc). A `list<compound>` element,
/// mixed element types, or a mixed scalar/bytes/list payload set is a later increment. Excludes option/result-
/// shaped sums (their own arms).
pub fn variant_list_payload_cases(db: &mut Db, ty: &Ty) -> Option<(Vec<i32>, Ty)> {
    let Ty::Sum { decl, .. } = ty.strip_nominal() else {
        return None;
    };
    if option_payload_ty(db, ty).is_some() || result_bytes_enum(db, ty).is_some() {
        return None;
    }
    let decl = *decl;
    let payload_counts: Vec<usize> = {
        let d = db.type_decl_by_occ(decl)?;
        d.variants.iter().map(|v| v.payloads.len()).collect()
    };
    let mut list_discs = Vec::new();
    let mut elem_ty: Option<Ty> = None;
    for (disc, n) in payload_counts.into_iter().enumerate() {
        match n {
            0 => {} // a nullary case → no payload slot
            1 => {
                let pty = crate::backend::wasm::select::variant_payload_ty_at(db, ty, disc as u32)?;
                // Every payload case MUST be a `list<scalar>` with the SAME element type (the single
                // `emit_list_arg_marshal(elem)` covers whichever list case fired); anything else declines cleanly.
                let Ty::List(inner) = pty.strip_nominal() else {
                    return None;
                };
                let inner = (**inner).clone();
                abi_val_type(&inner)?; // a non-scalar list element → a later increment (declines cleanly)
                match &elem_ty {
                    None => elem_ty = Some(inner),
                    Some(e) if *e == inner => {}
                    Some(_) => return None, // mixed element types → a later increment
                }
                list_discs.push(disc as i32);
            }
            _ => return None, // a multi-payload case → a later increment
        }
    }
    match elem_ty {
        Some(e) if !list_discs.is_empty() => Some((list_discs, e)),
        _ => None,
    }
}

/// ARG-SIDE: whether `ty` is a variant with EXACTLY ONE PAYLOAD-BEARING case whose payload TYPE is a `tuple` of
/// scalars, the rest nullary — EITHER a single `tuple`-typed payload (`b(tuple<s64,s64>)`) OR a MULTI-payload case
/// (`b(s64, s64)`), which `variant_payload_ty_at` synthesizes into the SAME tuple type (its value-heap rep is a
/// tuple handle either way). Returns the case's DISCRIMINANT (declaration = component order) paired with the
/// element ABIs (one `RecordFieldAbi` per tuple element, all scalar). The PRODUCT-payload sibling of
/// [`variant_bytes_payload_cases`] / [`variant_list_payload_cases`], but with a VARIABLE positional flatten
/// `(disc:i32, e0, e1, …)` (the tuple's elements inline, in element = component order) rather than a fixed 3 slots
/// — the register twin of [`result_tuple_enum`]'s Ok arm, minus the err-disc-in-slot-0 (a nullary variant case
/// zero-fills ALL payload slots). All-scalar so it flattens to registers with NO `mem`. The component boundary
/// type is the declared `variant` DEFINED type laid STRUCTURALLY from the WIT (`add_wit_type_deduped` →
/// `CDef::Variant` with a `(tuple <e>…)` payload case). Scoped to ONE tuple/multi-payload case + all-scalar
/// elements (a compound/bytes element, a second product case, or a nested tuple is a later increment). Excludes
/// option/result-shaped sums.
pub fn variant_tuple_payload_case(db: &mut Db, ty: &Ty) -> Option<(i32, Vec<RecordFieldAbi>)> {
    let Ty::Sum { decl, .. } = ty.strip_nominal() else {
        return None;
    };
    if option_payload_ty(db, ty).is_some() || result_bytes_enum(db, ty).is_some() {
        return None;
    }
    let decl = *decl;
    let payload_counts: Vec<usize> = {
        let d = db.type_decl_by_occ(decl)?;
        d.variants.iter().map(|v| v.payloads.len()).collect()
    };
    let mut tuple_case: Option<(i32, Vec<RecordFieldAbi>)> = None;
    for (disc, n) in payload_counts.into_iter().enumerate() {
        if n == 0 {
            continue; // a nullary case → no payload slot
        }
        if tuple_case.is_some() {
            return None; // a SECOND payload case → a later increment (multi-payload-case join)
        }
        // A payload-bearing case is a tuple-payload variant case iff its payload TYPE is a `tuple`: EITHER a
        // single `tuple`-typed payload (`b(tuple<…>)`, n==1), OR a MULTI-payload case (`b(s64, s64)`, n>=2) whose
        // payloads `variant_payload_ty_at` synthesizes into a tuple (its runtime rep IS a tuple handle —
        // `sum-payload` yields the payload array, `arr-get i` indexes it; core.rs). Both flatten IDENTICALLY via
        // `emit_variant_tuple_arg_reg_flatten` (sum-payload → tuple handle → `emit_tuple_reg_flatten`). A single
        // NON-tuple payload (scalar/bytes/list) yields a non-`Tuple` here → falls to its own detector arm.
        let pty = crate::backend::wasm::select::variant_payload_ty_at(db, ty, disc as u32)?;
        let Ty::Tuple(elems) = pty.strip_nominal() else {
            return None;
        };
        let elems: Vec<Ty> = elems.iter().cloned().collect();
        if elems.is_empty() {
            return None;
        }
        let mut abis = Vec::with_capacity(elems.len());
        for ety in &elems {
            // Every element MUST be a NO-mem scalar (`abi_val_type`) this increment — a bytes/list/nested
            // compound element would need the cursor + a richer flatten (a later increment).
            abi_val_type(ety)?;
            abis.push(field_boundary_abi(db, ety)?);
        }
        tuple_case = Some((disc as i32, abis));
    }
    tuple_case
}

/// ARG-SIDE: whether `ty` is a variant with EXACTLY ONE case carrying a `record` of scalars, the rest nullary.
/// Returns the record case's DISCRIMINANT (declaration = component order) paired with the record payload `Ty`.
/// The RECORD sibling of [`variant_tuple_payload_case`] — the same VARIABLE positional flatten `(disc:i32, f0,
/// f1, …)` but the fields are WIT-REORDERED (a record's guest field order is name-lex; the component/marshal use
/// the host WIT's declaration order), exactly as [`result_record_enum`]'s Ok arm. All-scalar → NO `mem`. The
/// component boundary type is the declared `variant` DEFINED type laid STRUCTURALLY from the WIT
/// (`add_wit_type_deduped` → `CDef::Variant` with a `(record …)` payload case). Scoped to a SINGLE record case +
/// all-scalar fields (a compound field, a second product case is a later increment). Excludes option/result sums.
pub fn variant_record_payload_case(db: &mut Db, ty: &Ty) -> Option<(i32, Ty)> {
    let Ty::Sum { decl, .. } = ty.strip_nominal() else {
        return None;
    };
    if option_payload_ty(db, ty).is_some() || result_bytes_enum(db, ty).is_some() {
        return None;
    }
    let decl = *decl;
    let payload_counts: Vec<usize> = {
        let d = db.type_decl_by_occ(decl)?;
        d.variants.iter().map(|v| v.payloads.len()).collect()
    };
    let mut record_case: Option<(i32, Ty)> = None;
    for (disc, n) in payload_counts.into_iter().enumerate() {
        match n {
            0 => {} // a nullary case → no payload slot
            1 => {
                if record_case.is_some() {
                    return None; // a SECOND payload case → a later increment (multi-payload-case join)
                }
                let pty = crate::backend::wasm::select::variant_payload_ty_at(db, ty, disc as u32)?;
                let Ty::Record(fields) = pty.strip_nominal() else {
                    return None; // a non-record payload → not this detector (bytes/list/tuple took their arms)
                };
                let fields = fields.clone();
                if fields.is_empty() {
                    return None;
                }
                // Every field MUST be a NO-mem scalar this increment (a Bytes/list/nested-compound field would
                // need the cursor + a richer flatten — a later increment).
                for fty in fields.values() {
                    abi_val_type(fty)?;
                }
                record_case = Some((disc as i32, pty.clone()));
            }
            _ => return None, // a multi-payload case → a later increment
        }
    }
    record_case
}

/// One payload case's kind for a MIXED-payload variant ([`variant_mixed_payload_cases`]): a `Scalar` (unboxed
/// into its join slot), a `Bytes`/`String` (rope-copied into `mem`, contributing a `(ptr, len)` pair), a
/// `List<scalar>` (marshaled into `mem` as an inline element array, contributing a `(ptr, count)` pair), or a
/// `Tuple` (a multi-payload / `tuple`-typed case flattened POSITIONALLY inline — one core slot per scalar
/// element, joined slot-wise with the other cases). The `Bytes` and `List` kinds share the two-i32-slot
/// `(ptr, len|count)` mem flatten; a `List`'s element `Ty` is carried so the marshal can lay the element array
/// and `used_ops` can declare the element's ops; a `Tuple` carries its elements' scalar ABI types (int OR
/// float) so the join + marshal know each inline slot's width and reinterpret a float element into its joined
/// slot. A record payload case in a mixed variant is still a later increment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariantPayloadKind {
    Scalar(AbiValType),
    Bytes,
    List(Ty),
    Tuple(Vec<AbiValType>),
    // A RECORD payload case whose fields are ALL scalar: flattens POSITIONALLY inline like a `Tuple` — one core
    // slot per field — but the slot ORDER follows the WIT record's field DECLARATION order (not the guest's
    // name-lex order). Carries the field `(kebab-name, ABI)` pairs (in WIT order once ordered by
    // `variant_mixed_payload_cases_wit` at the bare ARG, or by `wit_order_mem_mixed_record_cases` at a nested
    // record-FIELD / tuple-ELEMENT; in guest name-lex order from the bare `variant_mixed_payload_cases`, which is
    // used only for presence checks that do not care about slot order) and the guest record `Ty` (so the emit
    // reads its named fields via `emit_record_arg_marshal`, which itself orders field VALUES to WIT). The
    // `(name, abi)` pairs are the single WIT-orderable source: `record_field_cref` builds the `(record …)`
    // component type from them (name AND order), and `variant_mixed_join_slots` reads their ABIs for the flatten.
    Record(Vec<(String, AbiValType)>, Ty),
}

/// Whether a heterogeneous mem variant's payload-case KIND is one `select::emit_variant_mixed_to_mem` writes
/// in place at the canonical payload offset THIS increment: a `Scalar` (its width), a `Tuple` of scalars (the
/// product), `Bytes` (a `(ptr,len)` header + rope copy at the cursor), a `List` of SCALAR elements (a
/// `(ptr,count)` header + backing array at the cursor), or a `Record` of SCALAR fields (the product written at
/// the payload offset via `emit_record_to_mem`, WIT-ordered — the mem twin of the register mixed Record arm).
/// A `List` case's element may be a SCALAR or any COMPOUND `emit_list_arg_marshal` handles (record/tuple/nested
/// list/option) — the emit's List arm threads the element's declared WIT for those. This is unconditionally
/// `true` because [`variant_mixed_payload_cases`] ALREADY validated the element is marshalable (its List arm
/// admits only `abi_val_type(inner).is_some() || list_elem_marshalable(inner)`), and the three callers all pass
/// that detector's output — so a `List` case reaching here is known-marshalable. NB the `Record` case's WIT
/// field order MUST match the guest name-lex order (else the emit declines cleanly): the list marshal reserves
/// the per-element stride from `canonical_layout(record)` in guest order, so a divergent WIT order could write
/// past the reserved slot (record padding is field-order-dependent).
pub fn variant_mem_mixed_kind_supported(k: &VariantPayloadKind) -> bool {
    match k {
        VariantPayloadKind::Scalar(_)
        | VariantPayloadKind::Tuple(_)
        | VariantPayloadKind::Record(..)
        | VariantPayloadKind::List(_)
        | VariantPayloadKind::Bytes => true,
    }
}

/// Whether `ty` is an `option<mixed-variant>` whose payload variant's cases are all kinds
/// `select::emit_option_to_mem` can write as a LIST ELEMENT in place (disc byte + the variant's mem layout at
/// the payload offset via `emit_variant_mixed_to_mem`) — the same `variant_mem_mixed_kind_supported` set the
/// bare / record-FIELD / tuple-ELEMENT positions admit. A `Record` payload case is now included: the
/// option-element STRIDE is sized WIT-order (`canonical_layout_wit` threads the WIT through the option-shaped
/// `Sum` into its payload), so a divergent WIT field order no longer under-reserves the element slot.
pub fn option_mixed_variant_list_elem_ok(db: &mut Db, ty: &Ty) -> bool {
    option_payload_ty(db, ty).is_some_and(|p| {
        variant_mixed_payload_cases(db, &p).is_some_and(|cases| {
            cases
                .iter()
                .all(|(_, k)| variant_mem_mixed_kind_supported(k))
        })
    })
}

/// ARG-SIDE: whether `ty` is a variant whose payload cases MIX at least one `Scalar` case with at least one
/// `Bytes`/`String` case (the rest nullary) — the canonical HETEROGENEOUS tagged-union. Returns each payload
/// case's `(disc, kind)` in declaration order. Distinct from the uniform detectors: `variant_scalar_payload_cases`
/// (all-scalar), `variant_bytes_payload_cases` (all-Bytes). The register flatten is the canonical variant JOIN
/// (`wit_ctype::flatten_variant`): `[disc] ++ position-wise join over each payload case's flatten` — a `Scalar`
/// case flattens to one core slot, a `Bytes` case to `(i32 ptr, i32 len)`, joined slot-wise (mixed int widths →
/// `i64`). The guest `select::emit_variant_mixed_arg_reg_flatten` branches per case (scalar → unbox into slot 0
/// coerced to the joined width; bytes → rope-copy at the cursor → `(ptr,len)`; nullary → zero the slots). The
/// component boundary type is the declared `variant` DEFINED type (structural WIT). Excludes option/result sums.
/// Scoped to Scalar (int OR float) + Bytes + List-of-scalar + Tuple-of-scalars (int OR float elements) payload
/// kinds (a record payload case, a list of a non-scalar element, or a tuple with a non-scalar element, is a
/// later increment); a float scalar/tuple-element case is joined with the other cases' integer slots via the
/// canonical reinterpret lattice (`variant_mixed_join_slots` + `emit_scalar_coerce_into_slot`).
pub fn variant_mixed_payload_cases(db: &mut Db, ty: &Ty) -> Option<Vec<(i32, VariantPayloadKind)>> {
    let Ty::Sum { decl, .. } = ty.strip_nominal() else {
        return None;
    };
    if option_payload_ty(db, ty).is_some() || result_bytes_enum(db, ty).is_some() {
        return None;
    }
    let decl = *decl;
    let payload_counts: Vec<usize> = {
        let d = db.type_decl_by_occ(decl)?;
        d.variants.iter().map(|v| v.payloads.len()).collect()
    };
    let mut cases: Vec<(i32, VariantPayloadKind)> = Vec::new();
    let mut any_mem = false; // a Bytes OR List case — both take the two-i32-slot `(ptr, len|count)` mem flatten
    let mut any_tuple = false; // a multi-payload / `tuple`-typed case — inline positional flatten (N slots)
    let mut any_record = false; // a record-payload case — inline positional flatten (N slots), WIT field order
    for (disc, n) in payload_counts.into_iter().enumerate() {
        if n == 0 {
            continue; // a nullary case → no payload slot
        }
        // `variant_payload_ty_at` yields the case's payload type: the actual type for a single payload (n==1),
        // or a SYNTHESIZED `tuple` of the payload types for a multi-payload case (n>=2, whose runtime rep is a
        // tuple handle). Classify it once — this unifies the n==1 and n>=2 arms.
        let pty = crate::backend::wasm::select::variant_payload_ty_at(db, ty, disc as u32)?;
        let stripped = pty.strip_nominal();
        if matches!(stripped, Ty::Bytes | Ty::String) {
            any_mem = true;
            cases.push((disc as i32, VariantPayloadKind::Bytes));
        } else if let Ty::List(inner) = stripped {
            // A `list<T>` payload case: it marshals into `mem` as an inline element array → `(ptr, count)`, the
            // SAME two-i32-slot mem flatten as a Bytes case, REGARDLESS of the element type (the element WIDTH /
            // layout does not change the outer header). The element may be a SCALAR (offset-agnostic, `elem_wit =
            // None`) OR any COMPOUND `emit_list_arg_marshal` handles (`list_elem_marshalable`: a record/tuple
            // product, a nested list, an option, a `result<list<u8>, enum>`, a scalar variant) — the emit's List
            // arm threads the element WIT for those. A non-marshalable element declines the whole detector.
            let inner = (**inner).clone();
            if abi_val_type(&inner).is_none() && !list_elem_marshalable(db, &inner) {
                return None;
            }
            any_mem = true;
            cases.push((disc as i32, VariantPayloadKind::List(inner)));
        } else if let Ty::Tuple(elems) = stripped {
            // A TUPLE payload case (a `tuple`-typed single payload OR a multi-payload case): it flattens
            // POSITIONALLY inline to one core slot per element, joined slot-wise with the other cases. Each
            // element is a SCALAR (int OR float — the emit's Tuple arm reinterprets a float element into its
            // joined slot via `emit_scalar_coerce_into_slot`, exactly as the scalar case does); a Bytes/list/
            // nested-compound element would need the cursor + a richer flatten — a later increment.
            let mut abis = Vec::with_capacity(elems.len());
            for ety in elems.iter() {
                abis.push(abi_val_type(ety)?);
            }
            if abis.is_empty() {
                return None;
            }
            any_tuple = true;
            cases.push((disc as i32, VariantPayloadKind::Tuple(abis)));
        } else if let Some(v) = abi_val_type(&pty) {
            // A scalar payload — INT or FLOAT. The canonical variant JOIN reinterprets across cases
            // (`variant_mixed_join_slots`'s lattice: a float joined with the integer ptr/len/tuple slots of the
            // other cases → an integer slot), and `emit_variant_mixed_arg_reg_flatten`'s Scalar arm reinterprets
            // the unboxed value into that join slot (`emit_scalar_coerce_into_slot`). So no int-only guard — a
            // float scalar case mixed with a mem/tuple case is representable (an all-scalar int↔float mix, with
            // no mem/tuple case, is not "mixed" here and routes to `VariantScalarsMixed` instead).
            cases.push((disc as i32, VariantPayloadKind::Scalar(v)));
        } else if let Ty::Record(fields) = stripped {
            // A RECORD payload case whose fields are all SCALAR: like a tuple, it flattens POSITIONALLY inline to
            // one core slot per field — but the slot ORDER follows the WIT record's field DECLARATION order. Here
            // (no WIT) the ABIs are collected in guest name-lex order; `variant_mixed_payload_cases_wit` reorders
            // them to WIT order at the two sites that CONSUME the slot order (the classifier → serialize, and the
            // emit). A Bytes/list/nested-compound field would need the cursor + a richer flatten — a later increment.
            let mut abis = Vec::with_capacity(fields.len());
            for (sym, fty) in fields.iter() {
                let name =
                    crate::backend::common::export_name::kebab_extern_name(sym.name.as_ref());
                abis.push((name, abi_val_type(fty)?));
            }
            if abis.is_empty() {
                return None;
            }
            any_record = true;
            cases.push((disc as i32, VariantPayloadKind::Record(abis, pty.clone())));
        } else {
            return None; // an option/result-shaped or otherwise non-representable payload → a later increment
        }
    }
    // MIXED fires when AT LEAST ONE multi-slot payload case is present — a mem case (Bytes/List, two `(ptr,
    // len|count)` slots) OR a tuple case (N inline slots). This is the RESIDUAL variant-arg classifier: it is
    // dispatched LAST (after the `Variant`/`VariantScalarsMixed` all-scalar detectors, the all-`Bytes`
    // `variant_bytes_payload_cases`, the all-`list<scalar>`-same-element `variant_list_payload_cases`, and the
    // single-tuple `variant_tuple_payload_case`), so those narrower detectors claim their clean single-kind
    // case sets first and MIXED only sees what they decline. That residue is any representable combination the
    // per-case emit arms (Scalar/Bytes/List/Tuple/nullary) cover: a scalar mixed with a mem/tuple case, OR — with
    // NO scalar case — multiple or differing multi-slot cases (two tuples, a tuple beside a list, a Bytes beside a
    // list), or a record case, which no narrower detector handles. No `any_scalar` requirement: a scalar-less set
    // flattens the same way (the join is computed over whatever cases exist; the emit simply never takes a Scalar
    // arm). A non-scalar-element / non-scalar-field case still returns `None` above (a later increment), so this
    // never over-claims.
    (any_mem || any_tuple || any_record).then_some(cases)
}

/// [`variant_mixed_payload_cases`] with the variant's WIT type applied so any `Record` payload case's field ABIs
/// are ordered by the WIT record's field DECLARATION order (the canonical component-ABI slot order), not the
/// guest's name-lex order. The bare detector cannot do this (it has no WIT), so it collects a `Record` case's
/// ABIs name-lex; this wrapper — called ONLY where the slot order is CONSUMED (the classifier that builds the
/// `HostParam` fed to `serialize`, and the emit) — fixes them, keeping `serialize` (the param core type), the
/// emit (the pushed values, via `emit_record_arg_marshal` which also orders to WIT), and `host_imports` (the WIT
/// `variant` DEFINED type) all in the SAME order. Returns `None` (a clean decline) if a `Record` case has no
/// resolvable WIT record type or a WIT field is absent from the guest record. `variant_wit` is the arg's WIT type
/// (a `WitType::Variant`); `None` leaves the cases name-lex (a non-record set is unaffected either way).
pub fn variant_mixed_payload_cases_wit(
    db: &mut Db,
    ty: &Ty,
    variant_wit: Option<&crate::wit_world::WitType>,
) -> Option<Vec<(i32, VariantPayloadKind)>> {
    let mut cases = variant_mixed_payload_cases(db, ty)?;
    let has_record = cases
        .iter()
        .any(|(_, k)| matches!(k, VariantPayloadKind::Record(..)));
    if !has_record {
        return Some(cases); // no record case → the name-lex order is already the flatten order
    }
    let wit_cases = match variant_wit {
        Some(crate::wit_world::WitType::Variant(c)) => c,
        // A record case needs the WIT to order its fields — no WIT means we cannot lay a well-defined flatten.
        _ => return None,
    };
    for (disc, kind) in cases.iter_mut() {
        let VariantPayloadKind::Record(abis, _) = kind else {
            continue;
        };
        let rec_wit = wit_cases
            .get(*disc as usize)
            .and_then(|(_, p)| p.as_ref())?;
        let crate::wit_world::WitType::Record(wit_fields) = rec_wit else {
            return None;
        };
        // Reorder `abis` (guest name-lex) into WIT field-declaration order: for each WIT field, take the
        // (kebab-named) pair whose name matches. A WIT field absent from the guest record → decline.
        let mut ordered = Vec::with_capacity(wit_fields.len());
        for (fname, _) in wit_fields {
            let idx = abis.iter().position(|(n, _)| n == fname)?;
            ordered.push(abis[idx].clone());
        }
        *abis = ordered;
    }
    Some(cases)
}

/// Reorder each `Record` payload case's `(kebab-name, ABI)` field pairs in a NESTED [`RecordFieldAbi::VariantMemMixed`]
/// abi (a mixed-variant record-FIELD or tuple-ELEMENT) to the case's WIT record DECLARATION order, using
/// `variant_wit` (the field/element's WIT — a [`WitType::Variant`]). This is the register-flatten twin of
/// [`variant_mixed_payload_cases_wit`] (which orders the BARE-ARG cases): it keeps `serialize`'s
/// [`variant_mixed_join_slots`] flatten, [`host_imports::record_field_cref`]'s `(record …)` component type (which
/// reads the pairs' names AND order), and the emit's WIT-ordered push all in the SAME order for a mixed-variant
/// field/element whose guest name-lex record order DIVERGES from the WIT. A `Record` case with no resolvable WIT
/// record, or one where a WIT field is absent from the pairs (no clean bijection), is left name-lex: the emit
/// re-derives the WIT order itself ([`variant_mixed_payload_cases_wit`]) and declines cleanly when it cannot, so a
/// mis-order never miscompiles. `variant_wit` `None` / non-variant leaves the cases untouched.
pub fn wit_order_mem_mixed_record_cases(
    cases: &mut [(String, Option<VariantPayloadKind>)],
    variant_wit: Option<&crate::wit_world::WitType>,
) {
    let Some(crate::wit_world::WitType::Variant(wit_cases)) = variant_wit else {
        return;
    };
    for (disc, (_, kind)) in cases.iter_mut().enumerate() {
        let Some(VariantPayloadKind::Record(pairs, _)) = kind else {
            continue;
        };
        let Some((_, Some(crate::wit_world::WitType::Record(wit_fields)))) = wit_cases.get(disc)
        else {
            continue; // no WIT record for this case → leave name-lex (the emit declines cleanly)
        };
        // Reorder `pairs` (guest name-lex) into WIT field-declaration order: for each WIT field, take the
        // (kebab-named) pair whose name matches. Only replace on a clean bijection (every pair mapped exactly
        // once) — else leave name-lex and let the emit's own WIT-ordering decline.
        let mut ordered = Vec::with_capacity(wit_fields.len());
        for (fname, _) in wit_fields {
            let Some(idx) = pairs.iter().position(|(n, _)| n == fname) else {
                break;
            };
            ordered.push(pairs[idx].clone());
        }
        if ordered.len() == pairs.len() {
            *pairs = ordered;
        }
    }
}

/// The canonical variant-flatten PAYLOAD slots (core valtype bytes, EXCLUDING the leading disc) for a mixed
/// variant's payload cases — replicating [`wit_ctype::flatten_variant`]'s position-wise join so `serialize` +
/// `select::emit_variant_mixed_arg_reg_flatten` agree with the declared `variant` DEFINED type's flatten. Each
/// payload case flattens (a `Scalar` → its one core slot; a `Bytes` → `(i32 ptr, i32 len)`) and is joined
/// slot-wise into the accumulator (`join`: equal → same; int↔same-width-float → the int; else → `i64`).
pub fn variant_mixed_join_slots(cases: &[(i32, VariantPayloadKind)]) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::{CORE_F32, CORE_F64, CORE_I32, CORE_I64};
    let join = |a: u8, b: u8| -> u8 {
        if a == b {
            return a;
        }
        match (a, b) {
            (CORE_I32, CORE_F32) | (CORE_F32, CORE_I32) => CORE_I32,
            (CORE_I64, CORE_F64) | (CORE_F64, CORE_I64) => CORE_I64,
            _ => CORE_I64,
        }
    };
    let mut flat: Vec<u8> = Vec::new();
    for (_, kind) in cases {
        let case_flat: Vec<u8> = match kind {
            VariantPayloadKind::Scalar(v) => vec![v.core_byte()],
            // A Bytes case → `(ptr, len)`; a List case → `(ptr, count)` — both two i32 slots.
            VariantPayloadKind::Bytes | VariantPayloadKind::List(_) => vec![CORE_I32, CORE_I32],
            // A Tuple case → one core slot per element, positionally.
            VariantPayloadKind::Tuple(abis) => abis.iter().map(|a| a.core_byte()).collect(),
            // A Record case → one core slot per field, in the field ABIs' order (WIT declaration order once
            // `variant_mixed_payload_cases_wit` has ordered them). Same positional inline flatten as a Tuple.
            VariantPayloadKind::Record(abis, _) => {
                abis.iter().map(|(_, a)| a.core_byte()).collect()
            }
        };
        for (i, cb) in case_flat.into_iter().enumerate() {
            if i < flat.len() {
                flat[i] = join(flat[i], cb);
            } else {
                flat.push(cb);
            }
        }
    }
    flat
}

/// The `(guest name-lex slot, flags bit)` mapping for a record-of-bools arg crossing as a WIT `flags{labels}`
/// — each record field must be `Bool`, its kebab name must match a WIT label, and there must be exactly
/// `labels.len()` fields (≤32 — the Component Model caps a `flags` type at 32 labels / one i32; a >32-label
/// flags has no component boundary form, so it declines here). The PACK inverse of `param_field`'s flags-UNPACK
/// `field_bits` (same by-NAME matching). `None` if any condition fails (a non-bool field, a count/name
/// mismatch, or >32 labels) — the classifier then pushes nothing and the boundary guard declines.
pub(crate) fn flags_field_bits(
    fields: &std::collections::BTreeMap<crate::resolved::Symbol, Ty>,
    labels: &[String],
) -> Option<Vec<(u32, u32)>> {
    use crate::backend::common::export_name::kebab_extern_name;
    // The WASM Component Model caps `flags` at 32 labels (a single i32 — the validator rejects a component
    // with a >32-label flags type: "cannot have more than 32 flags"). So a >32-label flags has NO component
    // boundary form at all; decline it here (decline-don't-miscompile) rather than emit an invalid component.
    if labels.len() > 32 || fields.len() != labels.len() {
        return None;
    }
    let label_kebab: Vec<String> = labels.iter().map(|l| kebab_extern_name(l)).collect();
    let mut field_bits: Vec<(u32, u32)> = Vec::with_capacity(labels.len());
    for (slot, (fname, fty)) in fields.iter().enumerate() {
        if !matches!(fty.strip_nominal(), Ty::Bool) {
            return None;
        }
        let fk = kebab_extern_name(fname.name.as_ref());
        let bit = label_kebab.iter().position(|l| *l == fk)?;
        field_bits.push((slot as u32, bit as u32));
    }
    Some(field_bits)
}

/// The `(nested name-lex slot, flags bit)` mapping for a record FIELD that is a nested record-of-bools crossing
/// as a WIT `flags{labels}` — the abi-level twin of [`flags_field_bits`] (which works from the guest `Ty`),
/// used by [`reorder_record_fields_to_wit`] where only the field's already-built [`RecordFieldAbi`] is on hand.
/// Each nested sub-field must be a `Scalar(Bool)` (a distinct [`AbiValType::Bool`], so an int field can't
/// masquerade as flags), its kebab name must match a WIT label, and there must be exactly `labels.len()`
/// sub-fields (≤32 — the Component Model flags cap). `None` if any condition fails (then the field stays a
/// record and the marshal declines on the WIT mismatch — decline-don't-miscompile).
fn flags_field_bits_from_abi(
    sub: &[(String, RecordFieldAbi)],
    labels: &[String],
) -> Option<Vec<(u32, u32)>> {
    use crate::backend::common::export_name::kebab_extern_name;
    if labels.len() > 32 || sub.len() != labels.len() {
        return None;
    }
    let label_kebab: Vec<String> = labels.iter().map(|l| kebab_extern_name(l)).collect();
    let mut field_bits: Vec<(u32, u32)> = Vec::with_capacity(labels.len());
    for (slot, (name, abi)) in sub.iter().enumerate() {
        if !matches!(abi, RecordFieldAbi::Scalar(AbiValType::Bool)) {
            return None;
        }
        let fk = kebab_extern_name(name);
        let bit = label_kebab.iter().position(|l| *l == fk)?;
        field_bits.push((slot as u32, bit as u32));
    }
    Some(field_bits)
}

/// The boundary ABI of a shape-d record FIELD, or `None` if the field has no boundary form yet. Supports a
/// NO-WRAP scalar (`Int64`/`UInt64`/`Bool`/`Float64`/`Float32` — the read needs no i64→i32 narrow), a
/// `Bytes` (`list<u8>`) field, a NESTED record (recurse), and a `result<list<u8>, enum>` field
/// ([`result_bytes_enum`], the answer-back envelope). A narrow-int/`Char`/`Qty`/`String`, or a not-yet-mapped
/// compound, is a LATER slice → `None`. The guard's admit set + the classifier's `HostParam::Record`
/// production stay in lockstep (no arity skew between the boundary sig and the args).
pub(crate) fn field_boundary_abi(db: &mut Db, ty: &Ty) -> Option<RecordFieldAbi> {
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
                // An `option<tuple-of-scalars>` payload crosses as `option<tuple<…>>` — a POSITIONAL tuple has
                // no name-lex/WIT field-order ambiguity, so the payload's flattened slots (disc + one core slot
                // per element) line up with the marshal's positional push. Recurse the payload's abi (a
                // `RecordFieldAbi::Tuple` of scalars); its marshal is `emit_record_arg_marshal`'s
                // option<tuple-of-scalars> arm (scratch-flatten). Restricted to a FLAT all-scalar tuple this
                // increment — an `option<record>` (name-lex vs WIT ordering) or a nested/byte-leaf payload is a
                // later slice, so this MUST agree with that marshal arm's guard (decline-don't-miscompile).
                if let Ty::Tuple(elems) = payload.strip_nominal()
                    && !elems.is_empty()
                    && elems.iter().all(|e| abi_val_type(e).is_some())
                {
                    let inner = field_boundary_abi(db, &payload)?;
                    return Some(RecordFieldAbi::Option(Box::new(inner)));
                }
                // An `option<list<T>>` payload crosses as `option<list<elem>>` — `(disc, ptr, count)`; on Some
                // the payload list is marshalled into `mem` (`emit_list_arg_marshal`), on None `(0,0,0)`.
                // Admitted iff the list ELEMENT crosses at the boundary (`field_boundary_abi`, recursed via the
                // `List` arm above). Marshalled by `emit_record_arg_marshal`'s option<list> field arm (a record
                // FIELD) / `emit_option_reg_flatten`'s list branch (a top-level arg / tuple element) — MUST agree
                // with those marshal arms (decline-don't-miscompile).
                if let Ty::List(_) = payload.strip_nominal() {
                    let inner = field_boundary_abi(db, &payload)?; // `List(elem)`, or `None` if the elem declines
                    return Some(RecordFieldAbi::Option(Box::new(inner)));
                }
                // An `option<record>` payload (each field a scalar OR `Bytes`) crosses as `option<record<…>>`.
                // Unlike the tuple case, a record's fields are name-lex in the value-heap cell but DECLARATION-
                // ordered in the host WIT, so `reorder_record_fields_to_wit` recurses into this `Option(Record)`
                // payload (reordering the inner record's abi to the option payload's WIT record order) and the
                // marshal recursively marshals the payload record (each scalar → one slot, each `Bytes` →
                // `(ptr,len)` copied to `mem`). A nested-compound payload field is a later slice — MUST agree
                // with the marshal arm's guard (`scalar OR Bytes`).
                if let Ty::Record(sub) = payload.strip_nominal()
                    && !sub.is_empty()
                    && sub.values().all(|f| {
                        abi_val_type(f).is_some() || matches!(f.strip_nominal(), Ty::Bytes)
                    })
                {
                    let inner = field_boundary_abi(db, &payload)?;
                    return Some(RecordFieldAbi::Option(Box::new(inner)));
                }
                // An `option<variant>` payload (scalar-payload variant) crosses as `option<variant<…>>` —
                // `(disc, var-disc, join)`; recurse the payload's `Variant` abi. Marshalled by
                // `emit_record_arg_marshal`'s option<variant> field arm (a record FIELD) /
                // `emit_option_reg_flatten`'s variant branch (a top-level arg) — MUST agree with those marshal
                // arms (decline-don't-miscompile).
                if variant_scalar_payload_cases(db, &payload).is_some() {
                    let inner = field_boundary_abi(db, &payload)?; // `Variant(cases)`
                    return Some(RecordFieldAbi::Option(Box::new(inner)));
                }
                // An `option<variant>` payload that is a HETEROGENEOUS MIXED variant (a scalar case beside a
                // tuple / record / Bytes / List case) crosses as `option<variant<…>>` — recurse the payload's
                // `VariantMemMixed` abi. Marshalled by `emit_option_reg_flatten`'s mixed-variant branch: on Some
                // it flattens the payload variant via `emit_variant_mixed_arg_reg_flatten` (the SAME helper the
                // bare-ARG / record-FIELD / tuple-ELEMENT mixed variant uses, which WIT-orders a record case and
                // spills a Bytes/List case into `mem` at the option's reserved cursor), on None it zero-fills.
                // Checked AFTER the scalar-variant arm (disjoint: that one excludes a mem/tuple/record case).
                if variant_mixed_payload_cases(db, &payload).is_some_and(|cases| {
                    cases
                        .iter()
                        .all(|(_, k)| variant_mem_mixed_kind_supported(k))
                }) {
                    let inner = field_boundary_abi(db, &payload)?; // `VariantMemMixed(cases)`
                    return Some(RecordFieldAbi::Option(Box::new(inner)));
                }
                // A nested `option<option<T>>` payload crosses iff the INNER option itself crosses — recurse its
                // abi (`Option(T-abi)`) for ANY inner `T` (scalar → `(disc, scalar)`, bytes/list → `(disc, ptr,
                // len/count)`, tuple/record → `(disc, <fields…>)`). Marshalled by `emit_option_reg_flatten`'s
                // nested-option branch (a top-level arg / tuple element) / `emit_record_arg_marshal`'s
                // nested-option field arm (which DELEGATES to `emit_option_reg_flatten`) — kept in lockstep by
                // deriving the flatten from THIS abi. Checked last (an option is a Sum, distinct from all above).
                if option_payload_ty(db, &payload).is_some()
                    && field_boundary_abi(db, &payload).is_some()
                {
                    let inner = field_boundary_abi(db, &payload)?; // `Option(T-abi)`
                    return Some(RecordFieldAbi::Option(Box::new(inner)));
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
            // A payload-less `enum` field (a `Sum` all of whose variants are nullary) — the `enum` DEFINED type
            // + a single `i32` disc slot. Checked before the `variant` arm (an enum has no payload case, so
            // `variant_scalar_payload_cases` returns None for it anyway, but naming it explicitly documents the
            // shape). The nested analogue of the top-level enum arg (`HostParam::Enum`).
            if let Some(cases) = enum_cases(db, ty) {
                return Some(RecordFieldAbi::Enum(cases));
            }
            // A general `variant { c0, c1(scalar), … }` field (not option/result/enum-shaped) with uniform
            // scalar payloads — the `variant` DEFINED type + the `(disc, payload)` canonical flatten.
            if let Some(cases) = variant_scalar_payload_cases(db, ty) {
                return Some(RecordFieldAbi::Variant(cases));
            }
            // A `variant { c0, c1(tuple<…>), … }` field whose ONE payload case carries a TUPLE of scalars — the
            // compound-payload sibling. As a LIST element / mem PRODUCT field it is written in place by
            // `emit_variant_to_mem`'s tuple arm; as a REGISTER-flattened RECORD field it flattens to `(disc, e0,
            // …)` via `emit_variant_tuple_arg_reg_flatten`. Carries the case names (declaration order), the tuple
            // case's disc, and the tuple element ABIs (all scalar this increment).
            if let Some((tuple_disc, _)) = variant_tuple_payload_case(db, ty) {
                let tuple_ty =
                    crate::backend::wasm::select::variant_payload_ty_at(db, ty, tuple_disc as u32)?;
                let Ty::Tuple(elems) = tuple_ty.strip_nominal() else {
                    return None;
                };
                let elems: Vec<Ty> = elems.iter().cloned().collect();
                let mut elem_abis = Vec::with_capacity(elems.len());
                for e in &elems {
                    elem_abis.push(field_boundary_abi(db, e)?);
                }
                use crate::backend::common::export_name::kebab_extern_name;
                let Ty::Sum { decl, .. } = ty.strip_nominal() else {
                    return None;
                };
                let case_names: Vec<String> = {
                    let d = db.type_decl_by_occ(*decl)?;
                    d.variants
                        .iter()
                        .map(|v| kebab_extern_name(&v.name))
                        .collect()
                };
                return Some(RecordFieldAbi::VariantTuple {
                    case_names,
                    tuple_disc: tuple_disc as u32,
                    elem_abis,
                });
            }
            // A HETEROGENEOUS `variant` whose payload cases MIX scalar and tuple-of-scalars kinds — written in
            // place by `emit_variant_mixed_to_mem` (a LIST element / mem product field). Scoped to Scalar/Tuple
            // payload kinds (a Bytes/List/record case declines here so the marshal never sees an unsupported kind).
            if let Some(mixed) = variant_mixed_payload_cases(db, ty)
                && mixed
                    .iter()
                    .all(|(_, k)| variant_mem_mixed_kind_supported(k))
            {
                use crate::backend::common::export_name::kebab_extern_name;
                let Ty::Sum { decl, .. } = ty.strip_nominal() else {
                    return None;
                };
                let names: Vec<String> = {
                    let d = db.type_decl_by_occ(*decl)?;
                    d.variants
                        .iter()
                        .map(|v| kebab_extern_name(&v.name))
                        .collect()
                };
                // Pair each case name (declaration order) with its payload kind (`None` = nullary).
                let cases: Vec<(String, Option<VariantPayloadKind>)> = names
                    .into_iter()
                    .enumerate()
                    .map(|(d, name)| {
                        let kind = mixed
                            .iter()
                            .find(|(pd, _)| *pd as usize == d)
                            .map(|(_, k)| k.clone());
                        (name, kind)
                    })
                    .collect();
                return Some(RecordFieldAbi::VariantMemMixed(cases));
            }
            None
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
            // A bool-record field whose WIT field is `flags{…}` PACKS into a flags word (the record-FIELD twin
            // of `HostParam::Flags`). This is the ONLY site with both the field abi and its WIT, so the flags
            // conversion happens here. If the field is not a valid record-of-bools matching the labels,
            // `flags_field_bits_from_abi` returns None and the field stays `Record` → the marshal then hits the
            // WIT mismatch (WIT is flags, abi is record) and declines cleanly (decline-don't-miscompile).
            RecordFieldAbi::Record(sub) if matches!(fwit, WitType::Flags(_)) => {
                let WitType::Flags(labels) = fwit else {
                    unreachable!("guarded")
                };
                match flags_field_bits_from_abi(&sub, labels) {
                    Some(field_bits) => RecordFieldAbi::Flags {
                        field_bits,
                        labels: labels.clone(),
                    },
                    None => RecordFieldAbi::Record(sub),
                }
            }
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
            // Recurse into an `option<record>` payload: reorder the inner record's sub-fields to the option
            // PAYLOAD WIT record's declaration order (the field's `fwit` is `option<payload>`; unwrap it). A
            // non-record payload (option<scalar/tuple/bytes>) has no name-lex/WIT ambiguity, so it passes
            // through unchanged. This keeps the emitted `(option (record …))` component type + the flatten in
            // WIT order, matching the marshal's WIT-order push.
            RecordFieldAbi::Option(inner) => {
                let payload_wit = match fwit {
                    WitType::Option(p) => p.as_ref(),
                    _ => fwit,
                };
                let inner = match *inner {
                    RecordFieldAbi::Record(sub) => {
                        RecordFieldAbi::Record(reorder_record_fields_to_wit(sub, payload_wit))
                    }
                    other => other,
                };
                RecordFieldAbi::Option(Box::new(inner))
            }
            // A mixed-variant field: reorder each Record payload case's `(kebab-name, ABI)` pairs to the case's
            // WIT record DECLARATION order (`fwit` is the field's WIT variant), so `serialize`'s flatten,
            // `record_field_cref`'s `(record …)` component type, and the emit's WIT-ordered push all agree even
            // when the guest name-lex record order DIVERGES from the WIT. A case the WIT cannot order is left
            // name-lex — the emit re-derives the order and declines cleanly (decline-don't-miscompile).
            RecordFieldAbi::VariantMemMixed(mut cases) => {
                wit_order_mem_mixed_record_cases(&mut cases, Some(fwit));
                RecordFieldAbi::VariantMemMixed(cases)
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

/// Whether a top-level `tuple<…>` host-op ARGUMENT crosses natively as the built-in WIT `tuple<T…>`. Keyed to
/// [`emit_tuple_reg_flatten`]'s ELEMENT capability (the marshal), so the gate + classifier stay in lockstep: an
/// element crosses iff it is a SCALAR (`abi_val_type`), a `Bytes` leaf, a nested `tuple<…>` whose inner leaves
/// are all scalar/`Bytes` (recursed inline), a `list<T>` whose ELEMENT crosses at the boundary
/// ([`field_boundary_abi`], marshalled into `mem` by `emit_list_arg_marshal`), an `option<T>` whose payload
/// crosses ([`option_arg_crosses`], flattened by `emit_option_reg_flatten`), a scalar-payload `variant`
/// (flattened by `emit_variant_reg_flatten`), OR a `record` whose EVERY field crosses at the boundary
/// ([`is_boundary_record`], recursed by `emit_record_arg_marshal`).
/// Whether a value-heap `option<T>` crosses natively as the built-in WIT `option<T>` — its PAYLOAD is one
/// [`emit_option_reg_flatten`] handles: a SCALAR (`abi_val_type`), a `Bytes` leaf, a `tuple` of scalars/`Bytes`,
/// or a `record` of scalars/`Bytes`. A non-option `ty` yields `false` (no payload). Shared by the top-level
/// option-ARG gate + the option-ELEMENT-of-a-tuple gate, so they stay in lockstep with the marshal.
/// Whether a top-level `option<variant>` ARG's payload is a HETEROGENEOUS MIXED variant with a `Bytes`/`List`
/// payload case — which `emit_option_reg_flatten`'s mixed-variant branch spills into `mem` at the scratch
/// cursor, so the emit.rs pre-scan MUST reserve it (else the branch's `cursor.unwrap_or(pay_slot)` copies the
/// rope to a bogus slot — a latent corruption `wasm-tools validate` does NOT catch, only a runtime read would).
/// True iff the payload's boundary abi needs `mem` (its `VariantMemMixed` arm is true for a Bytes/List case); a
/// no-mem mixed variant (scalar/tuple/record cases) needs no cursor.
pub(crate) fn option_mixed_variant_needs_cursor(db: &mut Db, ty: &Ty) -> bool {
    option_payload_ty(db, ty).is_some_and(|p| {
        variant_mixed_payload_cases(db, &p).is_some()
            && field_boundary_abi(db, &p).is_some_and(|abi| record_field_abi_needs_memory(&abi))
    })
}

pub(crate) fn option_arg_crosses(db: &mut Db, ty: &Ty) -> bool {
    let Some(p) = option_payload_ty(db, ty) else {
        return false;
    };
    abi_val_type(&p).is_some()
        || matches!(p, Ty::Bytes)
        // a `tuple<…>` payload crosses iff EVERY element is one of the shapes the `emit_option_reg_flatten` tuple
        // branch marshals cleanly under an option: a SCALAR, a `Bytes`, a `list<T>` (element crossing), or a
        // HETEROGENEOUS MIXED `variant` (scalar/tuple/record/Bytes/List cases). `emit_option_reg_flatten`'s tuple
        // branch derives its capture widths from each element's `field_boundary_abi` and recurses
        // `emit_tuple_reg_flatten`. This is NARROWER than the direct-tuple `tuple_arg_crosses` gate on purpose: a
        // RECORD element with a sub-i64 (s32/s16/s8) field hits a PRE-EXISTING tuple<record> flatten mismatch
        // (a component-functype CDZ0910, reproducible on the DIRECT tuple arg too — a found gap routed to be
        // fixed), so an option<tuple<record>> DECLINES cleanly here rather than miscompile (decline-don't-
        // miscompile). Once the tuple<record{sub-i64}> flatten is fixed this can widen to full `tuple_arg_crosses`.
        || matches!(p.strip_nominal(), Ty::Tuple(es)
        if !es.is_empty()
            && es.iter().all(|e| {
                abi_val_type(e).is_some()
                    || matches!(e.strip_nominal(), Ty::Bytes)
                    || matches!(e.strip_nominal(), Ty::List(_))
                    || variant_mixed_payload_cases(db, e).is_some_and(|cases| {
                        cases.iter().all(|(_, k)| variant_mem_mixed_kind_supported(k))
                    })
            }))
        // a `list<T>` payload crosses iff its ELEMENT crosses at the boundary (`field_boundary_abi`) — the same
        // admit set a `list<T>` ARG / a record list FIELD use. `emit_option_reg_flatten`'s list branch marshals
        // the payload list into `mem` via `emit_list_arg_marshal` and pushes `(disc, ptr, count)`, the register
        // analogue of the option<bytes> `(disc, ptr, len)` branch.
        || matches!(p.strip_nominal(), Ty::List(inner)
            if field_boundary_abi(db, &(**inner).clone()).is_some())
        // a `record` payload crosses iff EVERY field crosses at the boundary ([`is_boundary_record`] /
        // `field_boundary_abi`) — the same admit set the direct record ARG uses, so an `option<record>`
        // accepts a `list`/nested-record/tuple field exactly where a bare record ARG does.
        || is_boundary_record(db, p.strip_nominal())
        // a scalar-payload `variant` payload crosses — `emit_option_reg_flatten`'s variant branch flattens the
        // payload variant via `emit_variant_reg_flatten` (the SAME helper the bare-variant ARG / a record
        // variant FIELD uses) and pushes `(opt-disc, var-disc, payload-join)`, the register analogue of the
        // `option<scalar>` branch with the variant's own `(disc, join)` flatten in the payload position.
        || variant_scalar_payload_cases(db, &p).is_some()
        // a HETEROGENEOUS MIXED `variant` payload (a scalar case beside a tuple / record / Bytes / List case)
        // crosses — `emit_option_reg_flatten`'s mixed-variant branch flattens it via
        // `emit_variant_mixed_arg_reg_flatten` (WIT-ordering a record case, spilling a Bytes/List case into `mem`
        // at the option's reserved cursor). MUST agree with that marshal arm's admit
        // (`variant_mem_mixed_kind_supported`) + the `field_boundary_abi` option arm. Checked after the
        // scalar-variant admit (disjoint: that returns None once a mem/tuple/record case is present).
        || variant_mixed_payload_cases(db, &p)
            .is_some_and(|cases| cases.iter().all(|(_, k)| variant_mem_mixed_kind_supported(k)))
        // a payload-less `enum` payload crosses — its disc reads inline as one i32 (the scalar-unbox path), so
        // `option<enum>` flattens to `(opt-disc, enum-disc)` exactly like `option<scalar>`; the abi is
        // `Option(Enum)` (so the component type is `(option (enum …))`, matching the world). Checked after the
        // variant admit (both are Sums; `enum_cases` requires ALL-nullary variants).
        || enum_cases(db, &p).is_some()
        // a nested `option<option<T>>` payload crosses iff the INNER option `p` itself crosses at the boundary —
        // `field_boundary_abi(option<T>)` is `Some` iff `T` (scalar / bytes / list / tuple / record / …) crosses.
        // `emit_option_reg_flatten`'s nested-option branch derives the inner option's flatten GENERICALLY from that
        // abi (`(outer-disc, inner-disc, <inner payload slots>)`); the classifier builds the SAME `Option(inner-abi)`
        // so serialize's flatten recursion agrees; the emit.rs pre-scan reserves the cursor iff the inner abi
        // needs `mem` (`record_field_abi_needs_memory`). `p` is the OUTER option's payload = the inner option.
        || (option_payload_ty(db, &p).is_some() && field_boundary_abi(db, &p).is_some())
}

pub(crate) fn tuple_arg_crosses(db: &mut Db, ty: &Ty) -> bool {
    let Ty::Tuple(elems) = ty.strip_nominal() else {
        return false;
    };
    if elems.is_empty() {
        return false;
    }
    let elems = elems.to_vec(); // release the borrow of `ty` before the `&mut db` calls
    elems.iter().all(|e| {
        abi_val_type(e).is_some()
            || matches!(e.strip_nominal(), Ty::Bytes)
            || matches!(e.strip_nominal(), Ty::Tuple(inner)
            if !inner.is_empty()
                && inner.iter().all(|x| {
                    abi_val_type(x).is_some() || matches!(x.strip_nominal(), Ty::Bytes)
                }))
            || match e.strip_nominal() {
                // a `list<T>` element crosses iff its ELEMENT crosses as a field boundary abi (the same
                // recursion the direct `list<T>` ARG + a record list FIELD use).
                Ty::List(inner) => {
                    let inner = (**inner).clone();
                    field_boundary_abi(db, &inner).is_some()
                }
                _ => false,
            }
            // an `option<T>` element crosses iff its payload crosses (`emit_option_reg_flatten`, the same twin a
            // top-level option ARG uses) — pushes `(disc, payload…)` inline into the tuple's positional flatten.
            || option_arg_crosses(db, e.strip_nominal())
            // a scalar-payload `variant` element crosses via `emit_variant_reg_flatten` (the twin a bare-variant
            // ARG / a variant record FIELD uses) — pushes `(disc, payload-join)` inline. Checked AFTER option
            // (an option is a Sum but `variant_scalar_payload_cases` excludes the 2-case option shape).
            || variant_scalar_payload_cases(db, e.strip_nominal()).is_some()
            // a tuple-payload `variant` element crosses via `emit_variant_tuple_arg_reg_flatten` (the twin the
            // top-level bare variant-tuple ARG / a variant-tuple record FIELD, SHAPE 256, use) — pushes
            // `(disc, e0, e1, …)` inline. Checked after the scalar-variant arm (it declines a tuple payload).
            || variant_tuple_payload_case(db, &e.strip_nominal().clone()).is_some()
            // a HETEROGENEOUS MIXED `variant` element (scalar + tuple + record + Bytes/List payload cases) crosses
            // via `emit_variant_mixed_arg_reg_flatten` (the twin the bare-ARG mixed variant / a mixed-variant
            // record FIELD, SHAPE 264/266/268/269, use) — pushes `(disc, joined-slots…)` inline. Checked after the
            // scalar-/single-tuple variant arms (they claim their clean shapes). This predicate has no per-element
            // WIT / cursor knowledge, so it admits any supported case set; the emit arm (`emit_tuple_reg_flatten`,
            // which HAS `elem_wits` + the reserved cursor) runs the name-lex==WIT record-order guard + the
            // `cursor.is_some()` guard and declines cleanly when they fail (admit-then-decline). A Bytes/List case
            // reserves the tuple's cursor via `tuple_arg_needs_cursor`'s variant leaf.
            || variant_mixed_payload_cases(db, &e.strip_nominal().clone())
                .is_some_and(|cases| cases.iter().all(|(_, k)| variant_mem_mixed_kind_supported(k)))
            // a payload-less `enum` element crosses as one i32 disc (the guest reads the value-heap sum's disc
            // inline via the scalar-unbox path, the SAME as a record enum FIELD). Checked AFTER variant (both
            // are Sums; `enum_cases` requires ALL-nullary, `variant_scalar_payload_cases` requires ≥1 payload).
            || enum_cases(db, &e.strip_nominal().clone()).is_some()
            || is_boundary_record(db, e.strip_nominal())
    })
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

/// Whether a record ARG has a HETEROGENEOUS MIXED `variant` FIELD (anywhere in its tree, recursing nested
/// records) with a `Bytes`/`List` payload case — such a case rope-copies / marshals its payload into shared
/// `mem` at the running cursor (`emit_variant_mixed_arg_reg_flatten`'s Bytes/List arms), so the arg must reserve
/// the scratch cursor. Complements [`record_has_bytes_field`]/[`record_has_list_field`] for the cursor gate — a
/// variant is a `Sum`, invisible to those. A scalar/tuple/record-only mixed variant field needs no cursor (it
/// declines to reserve here), and the register field emit's `cursor.is_some()` guard keeps a Bytes/List mixed
/// variant field DECLINING cleanly wherever the arg's pre-scan does not reserve one (decline-don't-miscompile).
pub fn record_has_mem_mixed_variant_field(db: &mut Db, ty: &Ty) -> bool {
    let Ty::Record(fields) = ty.strip_nominal() else {
        return false;
    };
    let ftys: Vec<Ty> = fields.values().cloned().collect();
    ftys.iter().any(|f| {
        variant_mixed_payload_cases(db, f).is_some_and(|cases| {
            cases
                .iter()
                .any(|(_, k)| matches!(k, VariantPayloadKind::Bytes | VariantPayloadKind::List(_)))
        }) || record_has_mem_mixed_variant_field(db, f)
    })
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

/// Whether a top-level `tuple<…>` ARG has a `Bytes` element — its rope is copied into shared `mem` (pushed as
/// `(ptr,len)`), so the arg needs the running scratch cursor reserved just like a Bytes arg / a Bytes record
/// field. An all-scalar tuple does NOT (it flattens positionally to inline core slots). Complements
/// [`record_has_bytes_field`] for the cursor-reservation gate (the top-level tuple twin of a Bytes record field).
pub fn tuple_has_bytes_element(ty: &Ty) -> bool {
    // Recurse through NESTED tuple elements + record fields: a `Bytes` ANYWHERE in the tuple's tree copies its
    // rope into shared `mem` (the nested marshal threads the cursor down), so the arg reserves the running
    // scratch cursor. Used only by the cursor-reservation gate, where a broader match is a harmless
    // over-reservation (an unused cursor slot) — never wrong.
    fn has_bytes(t: &Ty) -> bool {
        match t.strip_nominal() {
            Ty::Bytes => true,
            Ty::Tuple(es) => es.iter().any(has_bytes),
            Ty::Record(fs) => fs.values().any(has_bytes),
            _ => false,
        }
    }
    match ty.strip_nominal() {
        Ty::Tuple(elems) => elems.iter().any(has_bytes),
        _ => false,
    }
}

/// Whether a top-level `tuple<…>` host-op ARG needs the running scratch cursor — i.e. SOME leaf, recursing
/// nested tuples + record elements, copies runtime bytes into shared `mem`: a `Bytes`/`String` leaf, a
/// `list<T>`, a `result<list<u8>, enum>`, or an `option<bytes>`. An all-scalar tuple does not. This MUST cover
/// every cursor-consuming leaf `emit_tuple_reg_flatten`/`emit_record_arg_marshal` can reach for an admitted
/// tuple arg (via [`tuple_arg_crosses`]) — UNDER-detection panics the marshal's `cursor.expect(...)`;
/// OVER-detection is a harmless unused cursor slot. Broader than [`tuple_has_bytes_element`] (which is
/// `Bytes`-only), so the tuple-arg cursor pre-scan reserves for a record element with a list / result /
/// option<bytes> field, not just a `Bytes` field.
pub fn tuple_arg_needs_cursor(db: &mut Db, ty: &Ty) -> bool {
    fn leaf_needs(db: &mut Db, t: &Ty) -> bool {
        match t.strip_nominal() {
            Ty::Bytes | Ty::String | Ty::List(_) => true,
            Ty::Tuple(es) => {
                let es = es.to_vec(); // release the borrow before the recursive `&mut db` calls
                es.iter().any(|e| leaf_needs(db, e))
            }
            Ty::Record(fs) => {
                let fs = fs.clone(); // release the borrow before the recursive `&mut db` calls
                fs.values().any(|f| leaf_needs(db, f))
            }
            other => {
                // an `option<T>` leaf copies into `mem` iff its PAYLOAD does — recurse into the payload (an
                // `option<bytes>`/`option<record-with-a-bytes-field>`/`option<tuple-with-bytes>` needs the
                // cursor; an `option<scalar>`/`option<tuple-of-scalars>` does not). A `result<list<u8>, enum>`
                // leaf copies its Ok rope.
                let other = other.clone();
                if let Some(p) = option_payload_ty(db, &other) {
                    return leaf_needs(db, &p);
                }
                if result_bytes_enum(db, &other).is_some() {
                    return true;
                }
                // a HETEROGENEOUS MIXED `variant` leaf with a `Bytes`/`List` payload case copies / marshals that
                // case's payload into `mem` at the cursor (`emit_variant_mixed_arg_reg_flatten`'s Bytes/List arms),
                // so the tuple arg must reserve one — the tuple-element twin of `record_has_mem_mixed_variant_field`.
                variant_mixed_payload_cases(db, &other).is_some_and(|cases| {
                    cases.iter().any(|(_, k)| {
                        matches!(k, VariantPayloadKind::Bytes | VariantPayloadKind::List(_))
                    })
                })
            }
        }
    }
    let Ty::Tuple(elems) = ty.strip_nominal() else {
        return false;
    };
    let elems = elems.to_vec();
    elems.iter().any(|e| leaf_needs(db, e))
}

/// Whether a record ARG has an `option<bytes>` FIELD anywhere in its tree (recursing into nested records) —
/// its Some arm copies the payload rope into shared `mem`, so the arg needs the running scratch cursor. An
/// `option<scalar>` does NOT (it flattens to core slots). Complements [`record_has_bytes_field`]/
/// [`record_has_list_field`] for the cursor-reservation gate (an option is a `Sum`, invisible to those).
pub fn record_has_option_field_needing_mem(db: &mut Db, ty: &Ty) -> bool {
    let Ty::Record(fields) = ty.strip_nominal() else {
        return false;
    };
    let fields = (**fields).clone(); // release the borrow of `ty` before the recursive `&mut db` calls
    fields.values().any(|f| {
        option_payload_ty(db, f).is_some_and(|p| {
            // An option FIELD reserves the running scratch cursor iff its payload copies bytes into `mem` on
            // Some: a direct `option<bytes>`/`option<string>`, an `option<list>` (the payload list marshals its
            // backing array into `mem`), or an `option<record/tuple>` whose payload carries a `Bytes`/`list`
            // field (the recursive marshal spills those). Over-reservation is a harmless unused cursor slot.
            matches!(p.strip_nominal(), Ty::Bytes | Ty::String | Ty::List(_))
                || record_has_bytes_field(&p)
                || record_has_list_field(&p)
                // A NESTED option field (`option<option<X>>`): reserve iff the inner option's abi needs `mem`
                // (a Bytes/list leaf anywhere in `X`) — in lockstep with the delegating nested-option field marshal.
                || (option_payload_ty(db, &p).is_some()
                    && field_boundary_abi(db, &p).is_some_and(|abi| record_field_abi_needs_memory(&abi)))
                // An option<MIXED-VARIANT> field (`option<variant{…, c(list<u8>)/c(list<T>)}>`): reserve iff the
                // payload variant has a Bytes/List case, which `emit_option_reg_flatten`'s mixed-variant branch
                // spills into `mem` at the cursor on Some (via `emit_variant_mixed_arg_reg_flatten`). Without this
                // the record's cursor stays unreserved and that branch's `cursor.unwrap_or(pay_slot)` fallback
                // writes the rope to a SCRATCH-LOCAL index misused as a mem offset → an out-of-bounds trap that
                // `wasm-tools validate` does NOT catch (SHAPE 290). The option guard above skips this (a mixed
                // variant is not option-shaped, so `option_payload_ty(&p)` is None).
                || (variant_mixed_payload_cases(db, &p).is_some()
                    && field_boundary_abi(db, &p).is_some_and(|abi| record_field_abi_needs_memory(&abi)))
        }) || record_has_option_field_needing_mem(db, f)
    })
}

/// Whether a record ARG has a `result<list<u8>, enum>` FIELD anywhere in its tree (recursing into nested
/// records) — its Ok arm copies the payload rope into shared `mem` (`emit_record_arg_marshal`'s Result-field
/// arm), so the arg needs the running scratch cursor reserved. Complements [`record_has_bytes_field`]/
/// [`record_has_list_field`]/[`record_has_option_field_needing_mem`] for the cursor-reservation gate — a
/// `result` field is a `Sum`, invisible to the `Ty::Bytes`/`Ty::List`/`Ty::Tuple` matches those use. Without
/// it the marshal's `cursor.expect(...)` panics on a record-with-a-result-field ARG (SHAPE 215).
pub fn record_has_result_field(db: &mut Db, ty: &Ty) -> bool {
    let Ty::Record(fields) = ty.strip_nominal() else {
        return false;
    };
    let fields = (**fields).clone(); // release the borrow of `ty` before the recursive `&mut db` calls
    fields
        .values()
        .any(|f| result_bytes_enum(db, f).is_some() || record_has_result_field(db, f))
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
/// every field is `product_field_marshalable` (written in place at its canonical layout by `emit_record_to_mem`,
/// each field's WIT threaded so a nested record field is WIT-ordered). Kept in lockstep with the marshal's
/// element arms so the representability gate admits exactly what the marshal emits.
/// Whether a RECORD/TUPLE field of a `list<record|tuple>` ELEMENT is marshalable in place by
/// `select::emit_product_to_mem`: a scalar, a `Bytes`, an `option<scalar>`, a scalar/tuple `variant`, an `enum`,
/// a `list<T>` of a NO-WIT element (scalar/`Bytes`/nested list of those — [`list_field_no_wit`], written via
/// `emit_list_arg_marshal` at the cursor + a `(ptr,count)` header), a nested `tuple<…>` whose every element is
/// itself marshalable ([`tuple_field_marshalable`] — POSITIONAL, written via `emit_tuple_to_mem` with no field
/// WIT), or a nested RECORD field ([`record_field_marshalable`], written WIT-ordered via `emit_record_to_mem`) —
/// the last ONLY on the `wit = true` path (the enclosing writer supplies the field's WIT). An `option<compound>`
/// field, or a `list`/`tuple` whose element is a RECORD (needing the field WIT threaded through
/// `emit_tuple_to_mem` / `emit_list_arg_marshal`), is a later slice.
/// Whether `f` is a `list<T>` field whose element needs NO WIT to marshal — a scalar, a `Bytes`/`String`, or a
/// nested `list` of those (recursed). `emit_product_to_mem`'s list-field arm passes `elem_wit = None`, so it can
/// only lay a list whose element is offset-agnostic; a `record`/`tuple`/`option`/`variant` element would need
/// the field WIT threaded through (a later slice) and returns `false` here (the field then declines cleanly).
fn list_field_no_wit(f: &Ty) -> bool {
    let Ty::List(elem) = f.strip_nominal() else {
        return false;
    };
    match elem.strip_nominal() {
        Ty::Bytes | Ty::String => true,
        Ty::List(_) => list_field_no_wit(elem),
        other => abi_val_type(other).is_some(),
    }
}

/// Whether `f` is a `list<T>` field whose element is a FULL marshalable list element ([`list_elem_marshalable`] —
/// a `record`/`tuple`/`option`/… element that orders its fields by the element's WIT). Reachable ONLY when the
/// enclosing writer threads THIS field's WIT (`wit = true`): `emit_product_to_mem`'s list-field arm then passes
/// `Some(WitType::List(inner))` to `emit_list_arg_marshal`, whose element writers order the element WIT-order.
/// The WIT-threaded superset of [`list_field_no_wit`] (which stays the `wit = false` fallback).
fn list_field_with_wit(db: &mut Db, f: &Ty) -> bool {
    let Ty::List(elem) = f.strip_nominal() else {
        return false;
    };
    let elem = (*elem).clone(); // release the borrow before the `&mut db` recursion
    list_elem_marshalable(db, &elem)
}

/// Whether `f` is a nested `tuple<…>` field a product LIST-ELEMENT can carry — a non-empty tuple whose every
/// element is itself [`product_field_marshalable`]. Positional (no field-name reorder), but `emit_tuple_to_mem`
/// threads each element's WIT from the tuple's `WitType::Tuple(…)` WHEN the enclosing writer has the tuple's WIT
/// (`wit`) — so a nested RECORD element crosses iff `wit` is true (the element WITs are available to order its
/// fields); with `wit = false` (the tuple's WIT is not available) a record element declines. Mutually recursive
/// with [`product_field_marshalable`] (terminates on scalar leaves).
fn tuple_field_marshalable(db: &mut Db, f: &Ty, wit: bool) -> bool {
    let Ty::Tuple(elems) = f.strip_nominal() else {
        return false;
    };
    let elems = elems.to_vec(); // release the borrow of `f` before the recursive `&mut db` calls
    // A tuple's elements have their WIT available iff the tuple itself does (`emit_tuple_to_mem` threads
    // `elem_wits[i]` from the tuple's `WitType::Tuple`), so propagate `wit` to each element.
    !elems.is_empty() && elems.iter().all(|e| product_field_marshalable(db, e, wit))
}

/// Whether a RECORD/TUPLE `p` is marshalable as an `option<…>` PAYLOAD of a list element — its fields/elements
/// are each [`product_field_marshalable`] in the NO-WIT sense (`wit = false`). `emit_option_to_mem` writes the
/// payload product POSITIONALLY (it threads no field/element WIT to the payload record/tuple), and the scratch-
/// memory pre-scan does not yet recognize a record nested under an `option` list element — so a nested-record /
/// record-in-tuple payload is EXCLUDED here (it declines cleanly), matching the emit. The pre-existing
/// `option<record{scalar/bytes/list/tuple-of-scalars}>` / `option<tuple<scalar/bytes>>` shapes still admit.
fn option_payload_product_no_wit(db: &mut Db, p: &Ty) -> bool {
    match p.strip_nominal() {
        Ty::Record(fields) => {
            let fs: Vec<Ty> = fields.values().cloned().collect();
            !fs.is_empty() && fs.iter().all(|f| product_field_marshalable(db, f, false))
        }
        Ty::Tuple(elems) => {
            let es = elems.to_vec();
            !es.is_empty() && es.iter().all(|e| product_field_marshalable(db, e, false))
        }
        _ => false,
    }
}

/// Whether `f` is a nested RECORD field a product LIST-ELEMENT can carry — a non-empty record whose every field
/// is itself [`product_field_marshalable`] WITH its WIT (the nested record is written by `emit_record_to_mem`,
/// which threads each field's declared WIT). Only reachable when the enclosing writer supplies this field's WIT
/// (`wit = true` — the `emit_record_to_mem` path); a positional tuple element (`wit = false`) declines a record.
fn record_field_marshalable(db: &mut Db, f: &Ty) -> bool {
    let Ty::Record(fields) = f.strip_nominal() else {
        return false;
    };
    let ftys: Vec<Ty> = fields.values().cloned().collect(); // release the borrow before `&mut db` recursion
    !ftys.is_empty()
        && ftys
            .iter()
            .all(|ft| product_field_marshalable(db, ft, true))
}

/// `wit` = whether the enclosing writer can supply THIS field's declared WIT (true on the `emit_record_to_mem`
/// path, false for a positional `emit_tuple_to_mem` element) — gates the nested-RECORD arm, which needs the WIT
/// to order its name-lex fields to the host declaration order.
fn product_field_marshalable(db: &mut Db, f: &Ty, wit: bool) -> bool {
    matches!(f.strip_nominal(), Ty::Bytes | Ty::String)
        || abi_val_type(f).is_some()
        // A `list<T>` field of a product LIST-ELEMENT (`list<record{xs: list<s64>, …}>`): written by
        // `emit_product_to_mem`'s list-field arm — the list backing spilled into `mem` at the cursor + a
        // `(ptr,count)` header at the field offset (the list analogue of a `Bytes` field). A NO-WIT element
        // (scalar / `Bytes` / nested list of those, [`list_field_no_wit`]) crosses in ANY position; a `record`/
        // compound ELEMENT crosses ONLY when the enclosing writer threads THIS field's WIT (`wit` — the
        // `emit_record_to_mem` path passes `Some(WitType::List(inner))` to `emit_list_arg_marshal`, ordering the
        // element's fields), gated by [`list_field_with_wit`]; a positional tuple element (`wit = false`) admits
        // only the no-WIT form.
        || list_field_no_wit(f)
        || (wit && list_field_with_wit(db, f))
        // A nested `tuple<…>` field of a product LIST-ELEMENT (`list<record{t: tuple<s32,s64>, …}>`): written in
        // place by `emit_product_to_mem`'s tuple-field arm via `emit_tuple_to_mem` (POSITIONAL — no field WIT).
        // Each element must itself be `product_field_marshalable` (recursively) — a tuple with a record element
        // needs the field WIT threaded (a later slice), so it declines. A nested RECORD field also declines
        // (name-lex vs WIT order needs the WIT). Empty tuple excluded (no meaningful boundary form).
        || tuple_field_marshalable(db, f, wit)
        || option_payload_ty(db, f).is_some_and(|p| abi_val_type(&p).is_some())
        // A general `variant<scalar>` field of a product element (`list<record{v: variant{…}, …}>` /
        // `list<tuple<variant, …>>`): written in place by `select::emit_variant_to_mem`. Detected after
        // option (option takes its own arm); this is the residual general scalar-payload variant.
        || variant_scalar_payload_cases(db, f).is_some()
        // A `variant{nullary…, one tuple case}` field of a product element (`list<record{v: variant{…,
        // b(tuple<…>)}, …}>` / `list<tuple<variant{…, b(tuple<…>)}, …>>`): the SAME `emit_variant_to_mem`
        // writer lays the disc + the payload tuple at the canonical payload offset (its tuple-payload arm).
        // Detected after the scalar-variant arm (that arm declines a tuple payload).
        || variant_tuple_payload_case(db, f).is_some()
        // A payload-less `enum` field of a product element (`list<record{e: enum, …}>`): written in place as
        // its disc at the enum's canonical width. Detected after variant (both are Sums; `enum_cases` requires
        // ALL-nullary variants). The product-element analogue of the top-level record enum FIELD (SHAPE 174).
        || enum_cases(db, f).is_some()
        // A nested RECORD field of a product element (`list<record{r: record{…}, …}>`): written in place by
        // `emit_product_to_mem`'s record-field arm via `emit_record_to_mem`, WIT-ordered + WIT-SIZED (so a
        // DIVERGENT nested record reserves the correct extent). ONLY admitted when the enclosing writer supplies
        // this field's WIT (`wit` — the `emit_record_to_mem` path); a positional tuple element (`wit = false`)
        // declines it (a tuple threads no field WIT this slice). Detected last (a record is neither a Sum nor a
        // list/tuple, so the earlier arms already declined it).
        || (wit && record_field_marshalable(db, f))
}

pub fn list_elem_marshalable(db: &mut Db, ty: &Ty) -> bool {
    match ty.strip_nominal().clone() {
        Ty::Bytes | Ty::String => true,
        Ty::List(inner) => list_elem_marshalable(db, &inner),
        Ty::Record(fields) => {
            // A record ELEMENT is written by `emit_record_to_mem` with its WIT → its fields can supply WIT
            // (`wit = true`), so a nested record field crosses.
            let ftys: Vec<Ty> = fields.values().cloned().collect();
            !ftys.is_empty() && ftys.iter().all(|f| product_field_marshalable(db, f, true))
        }
        Ty::Tuple(elems) => {
            // A tuple ELEMENT of a list is written by `emit_tuple_to_mem`, which threads the tuple's element WITs
            // (from the list element's `WitType::Tuple`) → its elements have WIT (`wit = true`), so a nested
            // RECORD element crosses.
            let elems = elems.to_vec();
            !elems.is_empty() && elems.iter().all(|e| product_field_marshalable(db, e, true))
        }
        // A `result<list<u8>, enum>` element (`list<result<list<u8>, enum>>`): written in place at its canonical
        // result layout (disc byte + payload join) by `select::emit_result_to_mem` — Ok copies the Bytes rope at
        // the cursor + writes `(ptr,len)`, Err writes the err enum's disc. Detected before the option/variant arms
        // (a result is a 2-variant Sum but its Ok payload is `Bytes`, excluded from both). A non-Bytes ok / a
        // variant (non-enum) err is a later slice (`result_bytes_enum` declines it).
        ref other if result_bytes_enum(db, other).is_some() => true,
        // An `option<scalar|bytes|list|record|tuple>` element (`list<option<s64>>`, `list<option<bytes>>`,
        // `list<option<list>>`, `list<option<record>>`, `list<option<tuple>>`): written in place at its canonical
        // option layout (disc byte + payload) by `select::emit_option_to_mem`. A SCALAR payload writes its width
        // inline; a `Bytes`/`list` payload writes a `(ptr,len)`/`(ptr,count)` header at the payload offset with the
        // bytes/backing spilled at the cursor; a RECORD/TUPLE payload is written at the payload offset via the
        // product writer (each field `product_field_marshalable`, a Bytes field spilling at the cursor). A nested
        // `option<option<X>>` payload recurses `emit_option_to_mem` on the inner option, so it is admitted iff the
        // inner option is itself a marshalable element (`list_elem_marshalable` on the payload option).
        ref other
            if option_payload_ty(db, other).is_some_and(|p| {
                abi_val_type(&p).is_some()
                    || matches!(p.strip_nominal(), Ty::Bytes | Ty::String)
                    || matches!(p.strip_nominal(), Ty::List(inner)
                        if list_elem_marshalable(db, &(**inner).clone()))
                    || (matches!(p.strip_nominal(), Ty::Record(_) | Ty::Tuple(_))
                        && option_payload_product_no_wit(db, &p))
                    || (option_payload_ty(db, &p).is_some() && list_elem_marshalable(db, &p))
            }) =>
        {
            true
        }
        // An `option<MIXED-VARIANT>` element (`list<option<variant{a, b(s64), c(list<u8>)}>>`): written in place
        // at its canonical option layout (disc byte + the payload variant's mem layout at the payload offset) by
        // `select::emit_option_to_mem`'s mixed-variant arm, which recurses `emit_variant_mixed_to_mem`. The
        // preceding option arm declines it (a mixed variant is not scalar/Bytes/List/Record/Tuple and is not
        // option-shaped). SCOPED to order-agnostic payload cases (Scalar/Bytes/List/Tuple) — a Record case is a
        // later WIT-order-sizing slice (see [`option_mixed_variant_list_elem_ok`]).
        ref other if option_mixed_variant_list_elem_ok(db, other) => true,
        // A `variant<scalar>` element (`list<variant{a, b(s64), …}>`): written in place at its canonical
        // variant layout (disc + uniform scalar payload) by `select::emit_variant_to_mem`. Detected AFTER
        // option (option takes its own arm); this is the residual general scalar-payload variant. A mixed-
        // width / Bytes variant payload is a later slice (the flatten join widens).
        ref other if variant_scalar_payload_cases(db, other).is_some() => true,
        // A `variant{nullary…, one tuple case}` element (`list<variant{a, b(tuple<s32,s64>)}>`): written at its
        // canonical variant layout (disc + the tuple product at the payload offset) by `select::emit_variant_to_mem`
        // — the tuple case writes its all-scalar elements via `emit_product_to_mem`, a nullary case zero-fills the
        // payload region. Detected after the uniform scalar-variant arm (that arm declines a tuple payload). A
        // heterogeneous scalar+tuple mix is the NEXT arm; a bytes/nested-compound tuple element is a later slice.
        ref other if variant_tuple_payload_case(db, other).is_some() => true,
        // A HETEROGENEOUS `variant` element MIXING scalar + tuple-of-scalars payload cases (`list<variant{a,
        // b(s64), c(tuple<s32,s64>)}>`): written per-case in place at its canonical variant layout by
        // `select::emit_variant_mixed_to_mem`. Detected after the uniform scalar-variant + single-tuple arms
        // (they claim their clean shapes). SCOPED to Scalar/Tuple payload kinds — a Bytes/List/record case is a
        // later slice (so this gate matches exactly what the marshal emits).
        ref other
            if variant_mixed_payload_cases(db, other).is_some_and(|cases| {
                cases
                    .iter()
                    .all(|(_, k)| variant_mem_mixed_kind_supported(k))
            }) =>
        {
            true
        }
        // A payload-less `enum` element (`list<enum{a, b, …}>`): written in place as its discriminant at the
        // enum's canonical width (`disc_size(n_cases)`) by `select::emit_enum_to_mem`. Detected AFTER variant
        // (both are Sums; `enum_cases` requires ALL-nullary variants). The list-ELEMENT analogue of the record
        // enum FIELD (SHAPE 174) / the tuple enum ELEMENT (SHAPE 175).
        ref other if enum_cases(db, other).is_some() => true,
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
                    // A record-of-bools arg whose imposed WIT param is `flags{…}` crosses as the WIT `flags`
                    // bitset (the guest models flags as a PRODUCT record-of-bools). Checked BEFORE the generic
                    // record arm below (whose `WitType::Record` reorder would find no matching record + the emit
                    // would decline). The guest PACKS its bools into `ceil(n/32)` i32 word(s) by label→bit.
                    Ty::Record(fields)
                        if !peer_bound
                            && matches!(
                                wit_params.as_ref().and_then(|ps| ps.get(arg_i)),
                                Some(crate::wit_world::WitType::Flags(_))
                            ) =>
                    {
                        let Some(crate::wit_world::WitType::Flags(labels)) =
                            wit_params.as_ref().and_then(|ps| ps.get(arg_i))
                        else {
                            unreachable!("guarded by the arm")
                        };
                        if let Some(field_bits) = flags_field_bits(fields, labels) {
                            let labels_kebab: Vec<String> = {
                                use crate::backend::common::export_name::kebab_extern_name;
                                labels.iter().map(|l| kebab_extern_name(l)).collect()
                            };
                            params.push(HostParam::Flags {
                                field_bits,
                                labels: labels_kebab,
                            });
                        }
                        // else: not a matching record-of-bools → push nothing → the boundary guard declines.
                    }
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
                    // A `variant{nullary…, scalar-case(s)}` arg whose payloads MIX int with float (or f32 with
                    // f64) — the reinterpret-join tagged union the uniform `Variant` arm above declines (disjoint:
                    // `variant_scalar_payload_cases` returns None for the mix, `variant_mixed_scalar_payload_cases`
                    // returns None without it). Crosses as the declared `variant` DEFINED type via the SAME nominal
                    // builder as `Variant` (`comp_byte` expresses f32/f64), flattening to `(disc, join)` with the
                    // canonical reinterpret join. Marshalled by `emit_variant_mixed_scalar_arg_reg_flatten`.
                    _ if !peer_bound && variant_mixed_scalar_payload_cases(db, &at).is_some() => {
                        params.push(HostParam::VariantScalarsMixed(
                            variant_mixed_scalar_payload_cases(db, &at).unwrap(),
                        ));
                    }
                    // A `variant{nullary…, bytes-case(s)}` arg — a variant whose payload cases each carry a
                    // `Bytes`/`String` (`list<u8>`). Crosses as the declared `variant` DEFINED type (laid
                    // structurally from the WIT), flattening to `(disc, ptr, len)` — the 3-slot Bytes shape a
                    // `result<list<u8>, enum>` uses, but at a variant with arbitrary case discs. Marshalled by
                    // `emit_variant_bytes_arg_reg_flatten`. Checked BEFORE the scalar `_` arm (a Sum has no
                    // `abi_val_type`) and disjoint from the scalar `Variant` arm above (that returns None for a
                    // Bytes payload). A mixed scalar+Bytes / compound-payload variant is a later increment
                    // (`variant_bytes_payload_cases` requires every payload case to be `Bytes`/`String`).
                    _ if !peer_bound && variant_bytes_payload_cases(db, &at).is_some() => {
                        params.push(HostParam::VariantBytes(
                            variant_bytes_payload_cases(db, &at).unwrap(),
                        ));
                    }
                    // A `variant{nullary…, list<scalar>-case(s)}` arg — the `list` sibling of the bytes-case
                    // variant. Crosses as the declared `variant` DEFINED type (structural WIT), flattening to
                    // `(disc, ptr, count)` — on a list case the guest marshals the payload list into `mem`
                    // (`emit_variant_list_arg_reg_flatten` → `emit_list_arg_marshal`). Checked BEFORE the scalar
                    // `_` arm and disjoint from the scalar/bytes variant arms above (a `list` payload is neither
                    // `abi_val_type` nor `Bytes`). A `list<compound>` element / mixed element types is a later
                    // increment (`variant_list_payload_cases` requires a shared scalar element).
                    _ if !peer_bound && variant_list_payload_cases(db, &at).is_some() => {
                        params.push(HostParam::VariantList(
                            variant_list_payload_cases(db, &at).unwrap().0,
                        ));
                    }
                    // A `variant{nullary…, one tuple-of-scalars case}` arg — the PRODUCT-payload sibling. Crosses
                    // as the declared `variant` DEFINED type (structural WIT), flattening POSITIONALLY to
                    // `(disc, e0, e1, …)` via `emit_variant_tuple_arg_reg_flatten` (a nullary case zero-fills the
                    // payload slots). Checked BEFORE the scalar `_` arm and disjoint from the scalar/bytes/list
                    // variant arms above (a tuple payload is none of those). A compound tuple element / second
                    // product case is a later increment (`variant_tuple_payload_case` requires all-scalar, single).
                    _ if !peer_bound && variant_tuple_payload_case(db, &at).is_some() => {
                        let (tuple_disc, elem_abis) =
                            variant_tuple_payload_case(db, &at).unwrap();
                        params.push(HostParam::VariantTuple(tuple_disc, elem_abis));
                    }
                    // A `variant{nullary…, one record-of-scalars case}` arg — the RECORD sibling of the tuple
                    // payload. Builds the record's field ABIs then REORDERS them to the host WIT record's
                    // declaration order (extracted from the variant case's WIT), so the component type + core
                    // flatten agree with `emit_record_arg_marshal`'s WIT-order push. Checked BEFORE the scalar
                    // `_` arm and disjoint from the scalar/bytes/list/tuple variant arms above.
                    _ if !peer_bound && variant_record_payload_case(db, &at).is_some() => {
                        let (record_disc, record_ty) =
                            variant_record_payload_case(db, &at).unwrap();
                        if let Ty::Record(fields) = record_ty.strip_nominal() {
                            let fields = fields.clone();
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
                                // Reorder the name-lex fields to the variant case's WIT record declaration order.
                                let field_abis = match wit_params
                                    .as_ref()
                                    .and_then(|ps| ps.get(arg_i))
                                {
                                    Some(crate::wit_world::WitType::Variant(cases)) => {
                                        match cases
                                            .get(record_disc as usize)
                                            .and_then(|(_, p)| p.as_ref())
                                        {
                                            Some(rec_wit) => {
                                                reorder_record_fields_to_wit(field_abis, rec_wit)
                                            }
                                            None => field_abis,
                                        }
                                    }
                                    _ => field_abis,
                                };
                                params
                                    .push(HostParam::VariantRecord(record_disc, field_abis));
                            }
                        }
                    }
                    // A `variant{nullary…, scalar-case(s), bytes-case(s)}` arg — the canonical MIXED tagged-union.
                    // Crosses as the declared `variant` DEFINED type (structural WIT), flattening to the canonical
                    // join `(disc, joined-slots…)` via `emit_variant_mixed_arg_reg_flatten`. Checked AFTER the
                    // uniform scalar/bytes/list/tuple/record variant arms (this fires only when BOTH a scalar and
                    // a bytes payload case are present) and BEFORE the scalar `_` arm.
                    _ if !peer_bound && variant_mixed_payload_cases(db, &at).is_some() => {
                        // Build the WIT-AWARE cases so a RECORD payload case's field slots follow the WIT record's
                        // declaration order (the bare detector orders them name-lex) — keeping `serialize` (the
                        // flatten of these cases) aligned with `host_imports` (the WIT `variant` type) + the emit.
                        // A record case with no resolvable WIT field order → push nothing (the arg count falls
                        // short → a clean decline), mirroring the `VariantRecord` arm's `all_ok` guard.
                        if let Some(cases) = variant_mixed_payload_cases_wit(
                            db,
                            &at,
                            wit_params.as_ref().and_then(|ps| ps.get(arg_i)),
                        ) {
                            params.push(HostParam::VariantMixed(cases));
                        }
                    }
                    // A top-level `result<list<u8>, enum>` arg crosses as the built-in WIT
                    // `result<list<u8>, <enum>>` — the answer-back envelope shape. It flattens to
                    // `(disc:i32, ptr/errdisc:i32, len/0:i32)`: on Ok the guest writes the `list<u8>` payload
                    // into `mem` and passes `(0, ptr, len)`; on Err it passes `(component-disc, err-enum-disc,
                    // 0)`. Marshalled by `emit_result_arg_reg_flatten` (the register twin of the record `result`
                    // FIELD arm, minus the array-get). Checked BEFORE the scalar `_` arm (a Sum has no
                    // `abi_val_type`, so `_` would leave `params` short and decline); a `result<record,enum>` /
                    // `result<_, variant>` is a later increment (`result_bytes_enum` declines it → `_` declines).
                    _ if !peer_bound && result_bytes_enum(db, &at).is_some() => {
                        params.push(HostParam::Result(result_bytes_enum(db, &at).unwrap()));
                    }
                    // A top-level `result<scalar, enum>` arg crosses as the built-in WIT `result<ok, err-enum>`.
                    // It flattens to `(disc:i32, join)` — 2 slots, NOT the 3-slot Bytes shape: on Ok the guest
                    // unboxes the scalar payload; on Err it reads the err enum's disc; both into the shared join
                    // slot (`i64` iff the Ok scalar is 64-bit, else `i32`). Marshalled by
                    // `emit_result_scalar_arg_reg_flatten` (no rope, so NO `mem`). Checked BEFORE the scalar `_`
                    // arm (a Sum has no `abi_val_type`); a float Ok / a `variant` err arm is a later increment
                    // (`result_scalar_enum` declines it → `_` declines).
                    _ if !peer_bound && result_scalar_enum(db, &at).is_some() => {
                        let (ok, err_cases) = result_scalar_enum(db, &at).unwrap();
                        params.push(HostParam::ResultScalar(ok, err_cases));
                    }
                    // A top-level `result<record-of-scalars, enum>` arg crosses as the built-in WIT
                    // `result<record, err-enum>`. It flattens to `(disc:i32, record-fields…)` — the discriminant
                    // then the Ok record's fields POSITIONALLY (WIT order), the `i32` err disc riding the first
                    // field's slot on Err. Marshalled by `emit_result_record_arg_reg_flatten` (Ok recurses
                    // `emit_record_arg_marshal`; no rope → NO `mem`). Checked BEFORE the scalar `_` arm; a compound
                    // Ok field is a later increment (`result_record_enum` declines it → `_` declines).
                    _ if !peer_bound && result_record_enum(db, &at).is_some() => {
                        let (ok_record, err_cases) = result_record_enum(db, &at).unwrap();
                        if let Ty::Record(fields) = ok_record.strip_nominal() {
                            let fields = fields.clone();
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
                                // Reorder the name-lex Ok fields to the host WIT record's declaration order (the
                                // arg's WIT is `result<record, enum>`; the Ok arm carries the record type).
                                let field_abis = match wit_params
                                    .as_ref()
                                    .and_then(|ps| ps.get(arg_i))
                                {
                                    Some(crate::wit_world::WitType::Result {
                                        ok: Some(ok_wit),
                                        ..
                                    }) => reorder_record_fields_to_wit(field_abis, ok_wit),
                                    _ => field_abis,
                                };
                                params.push(HostParam::ResultRecord(field_abis, err_cases));
                            }
                        }
                    }
                    // A top-level `result<tuple-of-scalars, enum>` arg crosses as the built-in WIT
                    // `result<tuple<T…>, err-enum>`. It flattens to `(disc:i32, flatten(elem0), flatten(elem1), …)`
                    // — the discriminant then the Ok tuple's elements POSITIONALLY (no reorder — a tuple is
                    // positional), each element flattened by its `RecordFieldAbi` (scalar → one slot; a compound
                    // element → its in-mem `(ptr,len)` slots via the scratch cursor), the `i32` err disc riding
                    // the first element's slot on Err. Marshalled by `emit_result_tuple_arg_reg_flatten` (Ok
                    // recurses `emit_tuple_reg_flatten`). Checked BEFORE the scalar `_` arm; a FLOAT FIRST element
                    // (its slot 0 would need the canonical reinterpret join with the `i32` err disc) is a later
                    // increment (`result_tuple_enum` declines it → `_` declines) — a float in a LATER element
                    // rides its own `f64` slot and crosses fine.
                    _ if !peer_bound && result_tuple_enum(db, &at).is_some() => {
                        let (ok_tuple, err_cases) = result_tuple_enum(db, &at).unwrap();
                        if let Ty::Tuple(elems) = ok_tuple.strip_nominal() {
                            let elems = elems.to_vec();
                            let mut elem_abis = Vec::with_capacity(elems.len());
                            let mut all_ok = !elems.is_empty();
                            for ety in &elems {
                                match field_boundary_abi(db, ety) {
                                    Some(v) => elem_abis.push(v),
                                    None => {
                                        all_ok = false;
                                        break;
                                    }
                                }
                            }
                            if all_ok {
                                params.push(HostParam::ResultTuple(elem_abis, err_cases));
                            }
                        }
                    }
                    // A top-level `result<list<scalar>, enum>` arg crosses as the built-in WIT
                    // `result<list<T>, err-enum>`. It flattens to the SAME 3 slots as the Bytes-Ok result —
                    // `(disc, ptr/errdisc, count/0)` — but the Ok arm marshals the value-heap list into `mem`
                    // (`emit_result_list_arg_reg_flatten` → `emit_list_arg_marshal`) instead of copying a rope.
                    // Checked BEFORE the scalar `_` arm; a compound list element is a later increment
                    // (`result_list_enum` declines it → `_` declines). NB: `list<u8>` Ok = Bytes → `result_bytes_
                    // enum` above (a `list<u8>` arg type is `Ty::Bytes`, not `Ty::List`, so this never sees it).
                    _ if !peer_bound && result_list_enum(db, &at).is_some() => {
                        let (_elem, err_cases) = result_list_enum(db, &at).unwrap();
                        params.push(HostParam::ResultList(err_cases));
                    }
                    // A top-level `option<scalar>` / `option<bytes>` / `option<tuple-of-scalars>` arg crosses as
                    // the built-in WIT `option<T>` (its own arm — `variant_scalar_payload_cases` above EXCLUDES
                    // option-shaped sums, since option needs the distinct built-in type, not a `variant` DEFINED
                    // type). A SCALAR payload flattens to `(disc, scalar)`; a `Bytes` payload flattens to
                    // `(disc, ptr, len)`; an `option<tuple-of-scalars>` flattens to `(disc, flatten(tuple))` =
                    // disc + one core slot per POSITIONAL element — the register twin of the `RecordFieldAbi::
                    // Option` field, marshalled by `emit_option_reg_flatten` (its scalar/bytes/tuple branches).
                    // The component type + serialize flatten are already general over the payload abi (built from
                    // the declared WIT type / `flatten_record_field_abi`), so only the guest marshal is scoped:
                    // an `option<record>` / a nested/byte-leaf tuple payload is a later increment (leaves
                    // `params` short → declined). Checked BEFORE the scalar `_` arm (a Sum has no `abi_val_type`,
                    // so `_` would decline).
                    _ if !peer_bound && option_arg_crosses(db, &at) => {
                        let payload = option_payload_ty(db, &at).unwrap();
                        let abi = if let Some(pv) = abi_val_type(&payload) {
                            RecordFieldAbi::Scalar(pv)
                        } else if matches!(payload, Ty::Bytes) {
                            RecordFieldAbi::Bytes
                        } else if let Ty::List(elem) = payload.strip_nominal() {
                            // option<list<T>> → `RecordFieldAbi::Option(List(<elem abi>))`; the element abi is the
                            // shared `field_boundary_abi` (crosses by the arm guard). Marshalled by
                            // `emit_option_reg_flatten`'s list branch: `(disc, ptr, count)`, the list written into
                            // `mem` on Some. The `(option (list <elem>))` component type builds from this abi.
                            let elem = (**elem).clone();
                            let einner = field_boundary_abi(db, &elem)
                                .expect("option<list> element crosses by the arm guard");
                            RecordFieldAbi::List(Box::new(einner))
                        } else if let Ty::Tuple(elems) = payload.strip_nominal() {
                            // option<tuple> → the payload's `RecordFieldAbi::Tuple(…)`, each element's abi from the
                            // shared `field_boundary_abi` (scalar / Bytes / list / mixed variant / nested record /
                            // tuple / …) — the SAME builder the direct `tuple<…>` ARG + `emit_option_reg_flatten`'s
                            // tuple-branch `slot_vts` use, so the emitted `(option (tuple …))` component type + its
                            // core flatten agree with the guest marshal for ANY element shape. A prior narrow
                            // `Scalar`/`Bytes` map mis-typed a list element as `Bytes` (same 2×i32 flatten, wrong
                            // WIT) and gave a mixed-variant/record element a bogus `Bytes` abi → a component
                            // functype mismatch (CDZ0910). Each element crosses by the arm guard (`tuple_arg_crosses`).
                            let elems = elems.to_vec();
                            let mut abis = Vec::with_capacity(elems.len());
                            for e in &elems {
                                abis.push(
                                    field_boundary_abi(db, e)
                                        .expect("option<tuple> element crosses by the arm guard"),
                                );
                            }
                            RecordFieldAbi::Tuple(abis)
                        } else if variant_scalar_payload_cases(db, &payload).is_some() {
                            // option<variant> → `RecordFieldAbi::Variant(cases)` via the shared
                            // `field_boundary_abi` Variant arm (the SAME abi the bare-variant ARG / a record
                            // variant FIELD builds). Marshalled by `emit_option_reg_flatten`'s variant branch:
                            // `(opt-disc, var-disc, payload-join)`. The `(option (variant …))` component type
                            // builds from this abi. Checked before the record `else` (a variant is a Sum but
                            // NOT a `Ty::Record`, so the `else`'s `unreachable!` would fire).
                            field_boundary_abi(db, &payload)
                                .expect("option<variant> payload crosses by the arm guard")
                        } else if enum_cases(db, &payload).is_some() {
                            // option<enum> → `RecordFieldAbi::Enum(cases)` via the shared `field_boundary_abi`
                            // enum arm. Its disc reads inline as one i32 (the scalar-unbox path), so the option
                            // flattens to `(opt-disc, enum-disc)` via `emit_option_reg_flatten`'s scalar branch —
                            // no dedicated marshal arm. The `(option (enum …))` component type builds from this
                            // abi. Checked before the record `else` (an enum is a Sum, NOT a `Ty::Record`).
                            field_boundary_abi(db, &payload)
                                .expect("option<enum> payload crosses by the arm guard")
                        } else if option_payload_ty(db, &payload).is_some()
                            && field_boundary_abi(db, &payload).is_some()
                        {
                            // option<option<T>> → `RecordFieldAbi::Option(field_boundary_abi(option<T>))` =
                            // `Option(Option(T-abi))` for ANY inner `T` that crosses (scalar / bytes / list / tuple /
                            // record). Flattens to `(outer-disc, inner-disc, <inner payload slots>)` via
                            // `emit_option_reg_flatten`'s nested-option branch (which derives the width from this abi
                            // and recurses on the inner option handle; a Bytes/list leaf writes its backing into `mem`
                            // at the cursor). The `(option (option <T>))` component type builds from this abi. Checked
                            // before the record `else` (an option is a Sum, NOT a record).
                            field_boundary_abi(db, &payload)
                                .expect("option<option<T>> inner crosses by the arm guard")
                        } else if variant_mixed_payload_cases(db, &payload)
                            .is_some_and(|cases| {
                                cases.iter().all(|(_, k)| variant_mem_mixed_kind_supported(k))
                            })
                        {
                            // option<mixed-variant> → `RecordFieldAbi::Option(VariantMemMixed(cases))` via the
                            // shared `field_boundary_abi`. WIT-order each Record payload case's `(name, abi)`
                            // pairs to the payload variant's WIT declaration order
                            // (`wit_order_mem_mixed_record_cases`), so serialize's flatten + the `(option
                            // (variant …))` component type agree with `emit_option_reg_flatten`'s mixed-variant
                            // branch (which WIT-orders the emit via `variant_mixed_payload_cases_wit`). A Bytes/
                            // List payload case spills into `mem` at the option's reserved cursor. Checked before
                            // the record `else` (a variant is a Sum, NOT a `Ty::Record`, so the `else`'s
                            // `unreachable!` would fire).
                            let mut abi = field_boundary_abi(db, &payload)
                                .expect("option<mixed-variant> payload crosses by the arm guard");
                            let var_wit = match wit_params.as_ref().and_then(|ps| ps.get(arg_i)) {
                                Some(crate::wit_world::WitType::Option(pw)) => Some(pw.as_ref()),
                                _ => None,
                            };
                            if let RecordFieldAbi::VariantMemMixed(cases) = &mut abi {
                                wit_order_mem_mixed_record_cases(cases, var_wit);
                            }
                            abi
                        } else {
                            // option<record> → the payload's `RecordFieldAbi::Record(…)`, each field's abi from
                            // the shared recursive `field_boundary_abi` (scalar / Bytes / nested record / list /
                            // tuple / option / …) — the SAME builder the direct record ARG uses, so an
                            // `option<record>` accepts a `list`/nested field exactly where a bare record ARG does.
                            // Built name-lex, then REORDERED to the option payload WIT record's DECLARATION order
                            // (`reorder_record_fields_to_wit`) — the emitted `(option (record …))` component type +
                            // its core flatten must be WIT order to match `emit_option_reg_flatten`'s WIT-order
                            // marshal (a name-lex order silently fails the component-linker structural match).
                            let Ty::Record(sub) = payload.strip_nominal() else {
                                unreachable!("option payload is scalar/bytes/tuple/record by the guard")
                            };
                            let sub = sub.clone();
                            let mut fields: Vec<(String, RecordFieldAbi)> = Vec::with_capacity(sub.len());
                            for (sym, fty) in sub.iter() {
                                // Each field crosses by `option_arg_crosses` → `is_boundary_record`, so
                                // `field_boundary_abi` returns `Some`; treat a `None` as unreachable rather than
                                // silently dropping a field (which would desync the marshal from the abi).
                                let abi = field_boundary_abi(db, fty)
                                    .expect("option<record> field crosses by the arm guard");
                                fields.push((sym.name.to_string(), abi));
                            }
                            let fields = match wit_params.as_ref().and_then(|ps| ps.get(arg_i)) {
                                Some(crate::wit_world::WitType::Option(pw)) => {
                                    reorder_record_fields_to_wit(fields, pw.as_ref())
                                }
                                _ => fields,
                            };
                            RecordFieldAbi::Record(fields)
                        };
                        params.push(HostParam::Option(Box::new(abi)));
                    }
                    // A top-level `tuple<…>` arg crosses as the built-in WIT `tuple<T…>` — the guest flattens
                    // the value-heap tuple POSITIONALLY (a SCALAR element as one core slot, a `Bytes` element as
                    // `(ptr,len)` copied into `mem`, a NESTED tuple element recursed inline, a RECORD element
                    // recursed via `emit_record_arg_marshal`, no disc). `tuple_arg_crosses` (keyed to the
                    // marshal's element capability) is the gate; a record element's fields cross via the shared
                    // recursive `field_boundary_abi`. Checked BEFORE the scalar `_` arm (a tuple has no
                    // `abi_val_type`, so `_` would decline).
                    Ty::Tuple(elems) if !peer_bound && tuple_arg_crosses(db, &at) => {
                        let elems = elems.to_vec();
                        // Element `i`'s WIT (for a record element's field reorder), from the tuple's declared WIT.
                        let elem_wits: Option<Vec<crate::wit_world::WitType>> =
                            match wit_params.as_ref().and_then(|ps| ps.get(arg_i)) {
                                Some(crate::wit_world::WitType::Tuple(ws)) => Some(ws.clone()),
                                _ => None,
                            };
                        let mut abis = Vec::with_capacity(elems.len());
                        for (i, e) in elems.iter().enumerate() {
                            let abi = if let Some(pv) = abi_val_type(e) {
                                RecordFieldAbi::Scalar(pv)
                            } else if matches!(e.strip_nominal(), Ty::Bytes) {
                                RecordFieldAbi::Bytes // tuple<…, list<u8>, …> element → (ptr, len)
                            } else if let Ty::Tuple(inner) = e.strip_nominal() {
                                // a nested tuple element (positional, no name-lex ambiguity); a scalar inner
                                // → one slot, a Bytes inner → (ptr,len) copied to `mem` at the cursor.
                                let inner_abis = inner
                                    .iter()
                                    .map(|x| match abi_val_type(x) {
                                        Some(pv) => RecordFieldAbi::Scalar(pv),
                                        None => RecordFieldAbi::Bytes, // Bytes inner by the guard
                                    })
                                    .collect();
                                RecordFieldAbi::Tuple(inner_abis)
                            } else if matches!(e.strip_nominal(), Ty::List(_)) {
                                // a `list<T>` element crosses as a component `(list <elem>)` DEFINED type, core
                                // `(ptr, count)` — `emit_list_arg_marshal` writes the backing array + elements
                                // into `mem` at the cursor. Its abi is the shared recursive `field_boundary_abi`
                                // (`RecordFieldAbi::List(<elem abi>)`), the SAME as a direct list ARG / a record
                                // list FIELD. `tuple_arg_crosses` guarantees the element crosses.
                                field_boundary_abi(db, e)
                                    .expect("list element crosses by `tuple_arg_crosses`")
                            } else if let Some(opt_payload) = option_payload_ty(db, e) {
                                // an `option<T>` element flattens to `(disc, payload…)` via
                                // `emit_option_reg_flatten` (the register twin of the top-level option ARG). Its
                                // abi is `RecordFieldAbi::Option(<payload>)`; an `option<record>` payload is
                                // REORDERED to the element's option WIT record order (the marshal reads WIT order),
                                // exactly as the top-level option arg does. An `option<list>` payload is built
                                // INLINE here (`field_boundary_abi` deliberately does NOT admit `option<list>`, to
                                // keep an `option<list>` RECORD FIELD — whose inline marshal has no list arm —
                                // declining; the top-level option ARG + this tuple element build it inline, both
                                // marshalled by `emit_option_reg_flatten`'s list branch).
                                let abi = if let Ty::List(lelem) = opt_payload.strip_nominal() {
                                    let lelem = (**lelem).clone();
                                    let einner = field_boundary_abi(db, &lelem).expect(
                                        "option<list> element crosses by `tuple_arg_crosses`",
                                    );
                                    RecordFieldAbi::Option(Box::new(RecordFieldAbi::List(Box::new(
                                        einner,
                                    ))))
                                } else {
                                    field_boundary_abi(db, e)
                                        .expect("option element crosses by `tuple_arg_crosses`")
                                };
                                match (abi, elem_wits.as_ref().and_then(|ws| ws.get(i))) {
                                    (
                                        RecordFieldAbi::Option(inner),
                                        Some(crate::wit_world::WitType::Option(pw)),
                                    ) => {
                                        let inner = match *inner {
                                            RecordFieldAbi::Record(fields) => RecordFieldAbi::Record(
                                                reorder_record_fields_to_wit(fields, pw.as_ref()),
                                            ),
                                            other => other,
                                        };
                                        RecordFieldAbi::Option(Box::new(inner))
                                    }
                                    (abi, _) => abi,
                                }
                            } else if variant_scalar_payload_cases(db, e).is_some() {
                                // a scalar-payload `variant` element flattens to `(disc, payload-join)` via
                                // `emit_variant_reg_flatten` (the twin a bare-variant ARG / a variant record
                                // FIELD uses). Its abi is the shared `field_boundary_abi`
                                // (`RecordFieldAbi::Variant(cases)`). Checked AFTER the option branch (an option
                                // is a Sum but `variant_scalar_payload_cases` excludes the 2-case option shape).
                                field_boundary_abi(db, e)
                                    .expect("variant element crosses by `tuple_arg_crosses`")
                            } else if variant_tuple_payload_case(db, e).is_some() {
                                // a tuple-payload `variant` element flattens to `(disc, e0, e1, …)` via
                                // `emit_variant_tuple_arg_reg_flatten` (the twin the top-level bare variant-tuple
                                // ARG / a variant-tuple record FIELD, SHAPE 256, use). Its abi is the shared
                                // `field_boundary_abi` (`RecordFieldAbi::VariantTuple{…}`). Checked after the
                                // scalar-variant branch (it declines a tuple payload) and before the record else
                                // (a variant is a Sum, NOT a `Ty::Record`, so the else would panic).
                                field_boundary_abi(db, e)
                                    .expect("variant-tuple element crosses by `tuple_arg_crosses`")
                            } else if variant_mixed_payload_cases(db, e).is_some() {
                                // a HETEROGENEOUS mixed `variant` element flattens to `(disc, joined-slots…)` via
                                // `emit_variant_mixed_arg_reg_flatten` (the twin a bare-ARG mixed variant / a
                                // mixed-variant record FIELD, SHAPE 264, use). Its abi is the shared
                                // `field_boundary_abi` (`RecordFieldAbi::VariantMemMixed`); serialize flattens it
                                // with the canonical `variant_mixed_join_slots`. Reorder each Record payload case's
                                // `(name, abi)` pairs to the element's WIT record order (`elem_wits.get(i)`), so
                                // serialize's flatten agrees with the element's WIT-built component type + the
                                // emit's WIT-ordered push even when the guest name-lex record order diverges.
                                // Checked after the scalar-/single-tuple variant branches (they claim their clean
                                // shapes) and BEFORE the record else (a variant is a Sum, NOT a `Ty::Record`).
                                let mut abi = field_boundary_abi(db, e)
                                    .expect("mixed variant element crosses by `tuple_arg_crosses`");
                                if let RecordFieldAbi::VariantMemMixed(cases) = &mut abi {
                                    wit_order_mem_mixed_record_cases(
                                        cases,
                                        elem_wits.as_ref().and_then(|ws| ws.get(i)),
                                    );
                                }
                                abi
                            } else if enum_cases(db, &e.strip_nominal().clone()).is_some() {
                                // a payload-less `enum` element flattens to one i32 disc (the guest reads the
                                // value-heap sum's disc inline via the scalar-unbox path). Its abi is the shared
                                // `field_boundary_abi` (`RecordFieldAbi::Enum(cases)`). Checked before the record
                                // else (an enum is a Sum, NOT a `Ty::Record`, so the else would panic).
                                field_boundary_abi(db, e)
                                    .expect("enum element crosses by `tuple_arg_crosses`")
                            } else {
                                // a RECORD element: build each field's boundary abi via the shared recursive
                                // builder (`field_boundary_abi` — scalar/`Bytes`/nested record/list/tuple/option/
                                // result, the SAME set the direct record ARG + `emit_record_arg_marshal` handle),
                                // then REORDER to the element's WIT record order (the marshal pushes in WIT order,
                                // so the component type + core flatten must match — a name-lex order mis-links).
                                // `tuple_arg_crosses` guarantees every field crosses.
                                let Ty::Record(sub) = e.strip_nominal() else {
                                    unreachable!("tuple element is scalar/bytes/list/tuple/record by the guard")
                                };
                                let sub = sub.clone();
                                let mut fields: Vec<(String, RecordFieldAbi)> =
                                    Vec::with_capacity(sub.len());
                                for (sym, fty) in sub.iter() {
                                    let fabi = field_boundary_abi(db, fty)
                                        .expect("record-element field crosses by `tuple_arg_crosses`");
                                    fields.push((sym.name.to_string(), fabi));
                                }
                                let fields = match elem_wits.as_ref().and_then(|ws| ws.get(i)) {
                                    Some(ew) => reorder_record_fields_to_wit(fields, ew),
                                    None => fields,
                                };
                                RecordFieldAbi::Record(fields)
                            };
                            abis.push(abi);
                        }
                        params.push(HostParam::Tuple(abis));
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
            // A top-level `option<T>` / `tuple<T…>` arg needs mem iff a payload/element does — an
            // `option<bytes>` / a bytes-carrying tuple copies a rope into `mem` (an option<scalar> / all-scalar
            // tuple flattens with no mem → stays byte-identical).
            HostParam::Option(payload) => record_field_abi_needs_memory(payload),
            HostParam::Tuple(elems) => elems.iter().any(record_field_abi_needs_memory),
            // A top-level `result<list<u8>, enum>` arg copies the Ok `list<u8>` payload's rope into `mem`
            // (`emit_result_arg_reg_flatten`'s Ok arm), so it needs the shared-memory core module + the host
            // op lower's `Memory(0)` option — else the lower is emitted memoryless and the component fails
            // validation ("canonical option `memory` is required").
            // A `result<list<u8>, enum>` (rope copy) / a `result<list<scalar>, enum>` (the list marshalled into
            // `mem` by `emit_list_arg_marshal`) both write into linear memory on the Ok arm → need the shared-
            // memory core module + the host op lower's `Memory(0)`. The register-only scalar/record/tuple results
            // do NOT (they flatten to slots), so they fall to `_ => false`.
            HostParam::Result(_) | HostParam::ResultList(_) => true,
            // A `variant{…, bytes-case(s)}` copies a Bytes case's payload rope into `mem`
            // (`emit_variant_bytes_arg_reg_flatten`), so it needs the shared-memory core module + the host op
            // lower's `Memory(0)` — like the Bytes `Result`.
            HostParam::VariantBytes(_) => true,
            // A `variant{…, list<scalar>-case(s)}` marshals a list case's payload backing into `mem`
            // (`emit_variant_list_arg_reg_flatten` → `emit_list_arg_marshal`), so it needs `mem` too.
            HostParam::VariantList(_) => true,
            // A MIXED `variant` needs `mem` IFF it has a mem payload case — a Bytes case (rope copy) or a List
            // case (element backing) — that `emit_variant_mixed_arg_reg_flatten` writes into linear memory. A
            // scalar + TUPLE mix is register-only (positional inline flatten), so it needs no `mem`.
            HostParam::VariantMixed(cases) => cases
                .iter()
                .any(|(_, k)| matches!(k, VariantPayloadKind::Bytes | VariantPayloadKind::List(_))),
            // A `result<record, enum>` needs `mem` iff its Ok record has a field that marshals into memory (a
            // Bytes/list field — a record of only scalars flattens to registers, no mem). Mirrors the direct
            // `HostParam::Record` arm's per-field check.
            HostParam::ResultRecord(fields, _) => {
                fields.iter().any(|(_, f)| record_field_abi_needs_memory(f))
            }
            // A `result<tuple, enum>` needs `mem` iff its Ok tuple has an element that marshals into memory (a
            // bytes/list/compound element — a tuple of only scalars flattens to registers). Mirrors the direct
            // `HostParam::Tuple` arm's per-element check.
            HostParam::ResultTuple(elems, _) => elems.iter().any(record_field_abi_needs_memory),
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
            // A top-level scalar-payload variant whose payloads MIX int with float (or f32 with f64) crosses
            // NATIVELY as the declared `variant` DEFINED type — the canonical REINTERPRET join (a float payload
            // bit-reinterprets into the integer join slot; `select::emit_variant_mixed_scalar_arg_reg_flatten`;
            // all-scalar → no `mem`). Disjoint from `arg_is_boundary_variant` (the uniform detector declines the
            // mix). A `Bytes`/compound payload case is a different flavor, in lockstep with the classifier + marshal.
            let arg_is_boundary_variant_scalars_mixed = allow_option_bytes
                && !peer_bound
                && variant_mixed_scalar_payload_cases(db, &at).is_some();
            // A top-level `variant{nullary…, bytes-case(s)}` arg crosses NATIVELY as the declared `variant`
            // DEFINED type — the guest flattens the value-heap variant to `(disc, ptr/0, len/0)` core slots
            // (`select::emit_variant_bytes_arg_reg_flatten`, the arbitrary-disc twin of the `result<list<u8>,
            // enum>` flatten; a Bytes case copies its rope into `mem`). Same reducer/host-fused gating; a mixed
            // scalar+Bytes / compound-payload variant is a later increment (`variant_bytes_payload_cases`
            // declines it), matching the classifier + the marshal, in lockstep.
            let arg_is_boundary_variant_bytes =
                allow_option_bytes && !peer_bound && variant_bytes_payload_cases(db, &at).is_some();
            // A top-level `variant{nullary…, list<scalar>-case(s)}` arg crosses NATIVELY as the declared
            // `variant` DEFINED type — the guest marshals a list case's payload into `mem` and flattens to
            // `(disc, ptr/0, count/0)` (`select::emit_variant_list_arg_reg_flatten` → `emit_list_arg_marshal`),
            // the `list` sibling of the bytes-case variant. Same gating; a `list<compound>` element / mixed
            // element types is a later increment (`variant_list_payload_cases` declines it), matching the
            // classifier + the marshal, in lockstep.
            let arg_is_boundary_variant_list =
                allow_option_bytes && !peer_bound && variant_list_payload_cases(db, &at).is_some();
            // A top-level `variant{nullary…, one tuple-of-scalars case}` arg crosses NATIVELY as the declared
            // `variant` DEFINED type — the guest flattens the payload tuple positionally into
            // `(disc, e0, e1, …)` (`select::emit_variant_tuple_arg_reg_flatten`, the register twin of the
            // `result<tuple,enum>` Ok flatten; all-scalar → no `mem`). Same gating; a compound element / second
            // product case is a later increment (`variant_tuple_payload_case` declines it), in lockstep.
            let arg_is_boundary_variant_tuple =
                allow_option_bytes && !peer_bound && variant_tuple_payload_case(db, &at).is_some();
            // A top-level `variant{nullary…, one record-of-scalars case}` arg crosses NATIVELY as the declared
            // `variant` DEFINED type — the guest flattens the payload record's fields positionally (WIT order)
            // into `(disc, f0, f1, …)` (`select::emit_variant_record_arg_reg_flatten`, the record sibling of the
            // tuple-payload variant; all-scalar → no `mem`). Same gating; a compound field / second product case
            // is a later increment (`variant_record_payload_case` declines it), in lockstep.
            let arg_is_boundary_variant_record =
                allow_option_bytes && !peer_bound && variant_record_payload_case(db, &at).is_some();
            // A top-level MIXED `variant{nullary…, scalar-case(s), bytes-case(s)}` arg crosses NATIVELY as the
            // declared `variant` DEFINED type — the guest flattens per case into the canonical join
            // (`select::emit_variant_mixed_arg_reg_flatten`; a bytes case copies its rope into `mem`). A
            // list/tuple/record payload case or int↔float scalar mix is a later increment (declined), in lockstep.
            let arg_is_boundary_variant_mixed =
                allow_option_bytes && !peer_bound && variant_mixed_payload_cases(db, &at).is_some();
            // A top-level `option<scalar>` / `option<bytes>` / `option<tuple-of-scalars>` arg crosses NATIVELY
            // as the built-in WIT `option<T>` — the guest flattens the value-heap Option to `(disc, payload)`
            // core slots (`select::emit_option_reg_flatten`, the register twin of the `option<scalar>`/`::bytes`/
            // `::tuple` record-FIELD flatten; a Bytes payload copies its rope into `mem` on Some, a tuple payload
            // flattens each POSITIONAL element via `emit_tuple_reg_flatten`). Same reducer/host-fused gating; an
            // `option<record>` / a nested/byte-leaf tuple payload is a later increment so it is admitted here for
            // a SCALAR, `Bytes`, or all-scalar tuple payload — matching the classifier + the marshal, in lockstep.
            let arg_is_boundary_option =
                allow_option_bytes && !peer_bound && option_arg_crosses(db, &at);
            // A top-level `tuple<…>` arg crosses NATIVELY as the built-in WIT `tuple<T…>` — the guest flattens
            // the value-heap tuple positionally (`select::emit_tuple_reg_flatten`; a Bytes element copies its
            // rope into `mem`, a nested tuple element recurses inline, a record element recurses
            // `emit_record_arg_marshal` — each field crossing via `field_boundary_abi`). `tuple_arg_crosses` is
            // keyed to the marshal's element capability, so the gate + the classifier stay in lockstep.
            let arg_is_boundary_tuple =
                allow_option_bytes && !peer_bound && tuple_arg_crosses(db, &at);
            // A top-level `result<list<u8>, enum>` arg crosses NATIVELY as the built-in WIT
            // `result<list<u8>, <enum>>` — the guest flattens the value-heap result to `(disc, ptr/errdisc,
            // len/0)` core slots (`select::emit_result_arg_reg_flatten`, the register twin of the record
            // `result` FIELD flatten; the Ok `list<u8>` copies its rope into `mem`). Same reducer/host-fused
            // gating; a `result<record,enum>` / `result<_, variant>` is a later increment (`result_bytes_enum`
            // declines it), matching the classifier + the marshal, in lockstep.
            let arg_is_boundary_result =
                allow_option_bytes && !peer_bound && result_bytes_enum(db, &at).is_some();
            // A top-level `result<scalar, enum>` arg crosses NATIVELY as the built-in WIT `result<ok, err-enum>`
            // — the guest flattens the value-heap result to `(disc, join)` core slots
            // (`select::emit_result_scalar_arg_reg_flatten`; no rope, so no `mem`). Same gating as the Bytes
            // result; a float Ok / `variant` err arm is a later increment (`result_scalar_enum` declines it),
            // matching the classifier + the marshal, in lockstep.
            let arg_is_boundary_result_scalar =
                allow_option_bytes && !peer_bound && result_scalar_enum(db, &at).is_some();
            // A top-level `result<record-of-scalars, enum>` arg crosses NATIVELY as the built-in WIT
            // `result<record, err-enum>` — the guest flattens it to `(disc, record-fields…)`
            // (`select::emit_result_record_arg_reg_flatten`; no rope → no `mem`). A compound Ok field is a later
            // increment (`result_record_enum` declines it), matching the classifier + the marshal, in lockstep.
            let arg_is_boundary_result_record =
                allow_option_bytes && !peer_bound && result_record_enum(db, &at).is_some();
            // A top-level `result<tuple-of-scalars, enum>` arg crosses NATIVELY as the built-in WIT
            // `result<tuple<T…>, err-enum>` — the guest flattens it to `(disc, elem0, elem1, …)`
            // (`select::emit_result_tuple_arg_reg_flatten`; no rope → no `mem`). A compound/float element is a
            // later increment (`result_tuple_enum` declines it), matching the classifier + marshal, in lockstep.
            let arg_is_boundary_result_tuple =
                allow_option_bytes && !peer_bound && result_tuple_enum(db, &at).is_some();
            // A top-level `result<list<scalar>, enum>` arg crosses NATIVELY as the built-in WIT
            // `result<list<T>, err-enum>` — the guest marshals the Ok list into `mem` and flattens to
            // `(disc, ptr/errdisc, count/0)` (`select::emit_result_list_arg_reg_flatten`). A compound list element
            // is a later increment (`result_list_enum` declines it), matching the classifier + marshal, in lockstep.
            let arg_is_boundary_result_list =
                allow_option_bytes && !peer_bound && result_list_enum(db, &at).is_some();
            if !matches!(at, Ty::Unit | Ty::String | Ty::Bytes)
                && !ty_undetermined(&at)
                && !abi_ok(&at)
                && !arg_is_boundary_record
                && !arg_is_boundary_enum
                && !arg_is_boundary_list
                && !arg_is_boundary_variant
                && !arg_is_boundary_variant_scalars_mixed
                && !arg_is_boundary_variant_bytes
                && !arg_is_boundary_variant_list
                && !arg_is_boundary_variant_tuple
                && !arg_is_boundary_variant_record
                && !arg_is_boundary_variant_mixed
                && !arg_is_boundary_option
                && !arg_is_boundary_tuple
                && !arg_is_boundary_result
                && !arg_is_boundary_result_scalar
                && !arg_is_boundary_result_record
                && !arg_is_boundary_result_tuple
                && !arg_is_boundary_result_list
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

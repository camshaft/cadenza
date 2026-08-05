//! heap_unmarshal — the reducer-boundary READ direction (operator ruling C, §19e), dual of
//! [`crate::heap_marshal`].
//!
//! In the option-C handle-lowered MODE, a Cadenza reducer's `fold.apply` returns its `list<effect-request>`
//! as a single `u32` handle into the shared `cadenza:runtime/heap` value-heap (the spec's handle exchange).
//! WHEN a reducer is invoked that way, the host projects that handle back into
//! a `Vec` of [`wasm_host::EffectRequest`](crate::wasm_host::EffectRequest) — the WIT-generated
//! component-boundary type, NOT the kernel's public [`crate::effect::EffectRequest`] (the two are distinct;
//! the fold path converts the boundary type to the kernel struct, as `ComponentReducer` already does for
//! the WIT-structural path). This module is that read-direction bridge, driving
//! [`HeapHandle`]'s public read ops (vec-len / vec-get / arr-get / get-int / str-get / sum-disc /
//! sum-payload / read-bytes). Like its build-direction dual [`crate::heap_marshal`], it is a tested helper
//! staged AHEAD of its wiring: the current live boundary is still [`crate::wasm_host`]'s WIT-structural
//! `bindgen!` `fold.apply` (which lifts the structural `list<effect-request>` directly), and the
//! fold-boundary handle-ABI rebind is the slice that puts this on the call path.
//!
//! ## The value-heap layout this DECODES (verified against rcdzc's wasm backend)
//! - The returned value is a `list<effect-request>` — a value-heap VEC read by `vec-len` + `vec-get(i)`.
//! - Each element is an `effect-request` RECORD: a SORTED-field-NAME array (`Core::Proj`). The four fields
//!   `{correlation, kind, payload, target}` sort alphabetically to **arr[0]=correlation, arr[1]=kind,
//!   arr[2]=payload, arr[3]=target**.
//! - `correlation` / `payload` are `option<list<u8>>`: a `sum-new(disc, …)` read by `sum-disc`
//!   (**`Some=0`, `None=1`** — the prelude `Option`'s variant-declaration order, sums.rs:80) then, for
//!   `Some`, `sum-payload` → a bytes handle → `read-bytes`.
//! - `kind` is an `effect-kind` ENUM (all-nullary `shell=0`…`emit=5`). A standalone enum-disc is a bare
//!   i32, but as a RECORD FIELD it BOXES exactly like an int (v-rust-backend, authoritative — select.rs
//!   `box_op_ty`:2305: "an enum-discriminant sum … as a nested element boxes exactly like an integer").
//!   So arr[1] is read via `get-int` → the discriminant as an `i64`, NOT `sum-disc`.
//! - `target` is a `string`, read by `str-get`.
//!
//! An unknown `kind` discriminant is a hard [`ComponentError`] (a runtime-ABI drift the host must not
//! silently coerce), not a defaulted kind — fail loud, the same discipline as the marshalling's guards.

use crate::wasm_host::{ComponentError, EffectKind, EffectRequest, HeapHandle};

/// The prelude `Option` discriminants (variant-declaration order, `Some` first) — the SAME convention the
/// build side ([`crate::heap_marshal`]) writes, kept local so this read module carries no dependency on the
/// (separately-queued) build module.
const OPTION_SOME: i64 = 0;
const OPTION_NONE: i64 = 1;

/// Cap on the effect-list length we PRE-RESERVE for (reviewer, #2166 read-side DoS). `vec-len` returns a
/// raw unbounded `u32` reported BY the (untrusted) reducer's returned handle — a bogus large value (up to
/// `u32::MAX`) fed straight into `Vec::with_capacity` reserves hundreds of GB UP FRONT and aborts the host
/// via the alloc-error handler, BEFORE the per-element loop can fail-loud on the missing elements. Reserving
/// at most this many keeps the fast path for realistic effect counts while a bogus length just grows the
/// Vec with amortized reallocations and then Traps CHEAPLY on the first missing `vec-get`. A real fold emits a handful of
/// effects; 4096 is far above any legitimate turn. (Read-side analog of the #2151 write-side length guard.)
const MAX_PREALLOC_EFFECTS: u32 = 4096;

/// The effect-request record's SORTED-field-name indices: `{correlation, kind, payload, target}` sorted
/// alphabetically. Naming them keeps the field↔index mapping in one auditable place (a silent off-by-one
/// here would mis-decode every effect).
const FIELD_CORRELATION: u32 = 0;
const FIELD_KIND: u32 = 1;
const FIELD_PAYLOAD: u32 = 2;
const FIELD_TARGET: u32 = 3;

/// Map an `effect-kind` discriminant (0..=5, the WIT enum's declaration order) to the generated
/// [`EffectKind`]. An out-of-range discriminant is a runtime-ABI drift (the guest/runtime emitted a
/// discriminant this host build doesn't know) — a hard error, never a silent default.
fn disc_to_effect_kind(disc: i64) -> Result<EffectKind, ComponentError> {
    match disc {
        0 => Ok(EffectKind::Shell),
        1 => Ok(EffectKind::Http),
        2 => Ok(EffectKind::Model),
        3 => Ok(EffectKind::Now),
        4 => Ok(EffectKind::Timer),
        5 => Ok(EffectKind::Emit),
        other => Err(ComponentError::Trap(format!(
            "effect-request kind discriminant {other} is out of range 0..=5 (unknown effect-kind — \
             runtime-ABI drift)"
        ))),
    }
}

/// Read an `option<list<u8>>` value-heap sum handle back to `Option<Vec<u8>>`: `sum-disc` selects the arm
/// (`0=Some`/`1=None`), and `Some` reads its `sum-payload` bytes handle via `read-bytes`. An unknown
/// discriminant is a hard error (ABI drift). `Some([])` (an empty-but-present payload) round-trips as
/// `Some(vec![])`, distinct from `None`.
fn read_option_bytes<T>(
    heap: &mut HeapHandle<T>,
    handle: u32,
) -> Result<Option<Vec<u8>>, ComponentError> {
    let disc = i64::from(heap.sum_disc(handle)?);
    match disc {
        OPTION_SOME => {
            let payload = heap.sum_payload(handle)?;
            Ok(Some(heap.read_bytes(payload)?))
        }
        OPTION_NONE => Ok(None),
        other => Err(ComponentError::Trap(format!(
            "option<list<u8>> discriminant {other} is neither Some(0) nor None(1) — runtime-ABI drift"
        ))),
    }
}

/// Read ONE `effect-request` record handle back to an [`EffectRequest`]. The record is a sorted-field-name
/// array; each field is projected by its sorted index (see the `FIELD_*` constants) and decoded by its
/// type: `kind` via `get-int` (a boxed enum-disc), `target` via `str-get`, `correlation`/`payload` via
/// [`read_option_bytes`].
fn read_effect_request<T>(
    heap: &mut HeapHandle<T>,
    record: u32,
) -> Result<EffectRequest, ComponentError> {
    let correlation_h = heap.arr_get(record, FIELD_CORRELATION)?;
    let kind_h = heap.arr_get(record, FIELD_KIND)?;
    let payload_h = heap.arr_get(record, FIELD_PAYLOAD)?;
    let target_h = heap.arr_get(record, FIELD_TARGET)?;

    // `kind` is a BOXED enum-disc (v-rust-backend authoritative): read the discriminant as an i64.
    let kind = disc_to_effect_kind(heap.get_int(kind_h)?)?;
    let target = heap.str_get(target_h)?;
    let payload = read_option_bytes(heap, payload_h)?;
    let correlation = read_option_bytes(heap, correlation_h)?;

    Ok(EffectRequest {
        kind,
        target,
        payload,
        correlation,
    })
}

/// Project a reducer's returned `list<effect-request>` value-heap handle into
/// a `Vec` of [`wasm_host::EffectRequest`](crate::wasm_host::EffectRequest) — the WIT-generated
/// component-boundary type (NOT the kernel's [`crate::effect::EffectRequest`]; the fold path converts).
/// Walks the vec (`vec-len` + `vec-get(i)`) and reads each element record — the whole read-direction output
/// of the fold-boundary rebind, ready for the drive loop to authorize + dispatch.
pub fn read_effect_requests<T>(
    heap: &mut HeapHandle<T>,
    list: u32,
) -> Result<Vec<EffectRequest>, ComponentError> {
    let len = heap.vec_len(list)?;
    // Reserve for at most MAX_PREALLOC_EFFECTS — `len` is an unbounded guest-reported u32, so a bogus large
    // value must NOT drive an eager multi-GB reservation (alloc-abort DoS, #2166 read-side). A truthful
    // large len just grows the Vec with amortized reallocations; a bogus one Traps on the first missing `vec-get` below.
    let mut out = Vec::with_capacity(len.min(MAX_PREALLOC_EFFECTS) as usize);
    for i in 0..len {
        let record = heap.vec_get(list, i)?;
        out.push(read_effect_request(heap, record)?);
    }
    Ok(out)
}

// ── ASYNC twins (#2256 / v-ah-host ask 27000): the read direction for a handle-lowered fold on the ASYNC
// engine. Identical decode to the sync forms above — same sorted-field indices, Some/None convention,
// DoS-prealloc cap, and ABI-drift hard-Traps (the sync forms' docs are the single source of truth) — but
// each heap read op is driven via its `*_async` twin (the sync `Func::call` panics per-store on an
// async_support engine). `T: Send` (an async call may poll across an await point).

/// Async twin of [`read_option_bytes`].
async fn read_option_bytes_async<T: Send + 'static>(
    heap: &mut HeapHandle<T>,
    handle: u32,
) -> Result<Option<Vec<u8>>, ComponentError> {
    let disc = i64::from(heap.sum_disc_async(handle).await?);
    match disc {
        OPTION_SOME => {
            let payload = heap.sum_payload_async(handle).await?;
            Ok(Some(heap.read_bytes_async(payload).await?))
        }
        OPTION_NONE => Ok(None),
        other => Err(ComponentError::Trap(format!(
            "option<list<u8>> discriminant {other} is neither Some(0) nor None(1) — runtime-ABI drift"
        ))),
    }
}

/// Async twin of [`read_effect_request`].
async fn read_effect_request_async<T: Send + 'static>(
    heap: &mut HeapHandle<T>,
    record: u32,
) -> Result<EffectRequest, ComponentError> {
    let correlation_h = heap.arr_get_async(record, FIELD_CORRELATION).await?;
    let kind_h = heap.arr_get_async(record, FIELD_KIND).await?;
    let payload_h = heap.arr_get_async(record, FIELD_PAYLOAD).await?;
    let target_h = heap.arr_get_async(record, FIELD_TARGET).await?;

    let kind = disc_to_effect_kind(heap.get_int_async(kind_h).await?)?;
    let target = heap.str_get_async(target_h).await?;
    let payload = read_option_bytes_async(heap, payload_h).await?;
    let correlation = read_option_bytes_async(heap, correlation_h).await?;

    Ok(EffectRequest {
        kind,
        target,
        payload,
        correlation,
    })
}

/// Async twin of [`read_effect_requests`].
pub async fn read_effect_requests_async<T: Send + 'static>(
    heap: &mut HeapHandle<T>,
    list: u32,
) -> Result<Vec<EffectRequest>, ComponentError> {
    let len = heap.vec_len_async(list).await?;
    let mut out = Vec::with_capacity(len.min(MAX_PREALLOC_EFFECTS) as usize);
    for i in 0..len {
        let record = heap.vec_get_async(list, i).await?;
        out.push(read_effect_request_async(heap, record).await?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A synthetic `cadenza:runtime/heap` stub whose READ ops decode a FUNCTIONAL, test-authored value-heap
    // (not sentinels), so a test can lay out a real effect-request list in guest memory and assert the host
    // reads it back field-for-field. Layout in linear memory (all i32 cells, little-endian):
    //  - a VEC handle points at [len:i32][elem0-handle:i32][elem1-handle:i32]…
    //  - a RECORD (arr) handle points at [len:i32][field0:i32][field1:i32]… (sorted-field order)
    //  - a SUM handle points at [disc:i32][payload-handle:i32]
    //  - a BYTES handle points at [len:i32][bytes…]
    //  - a boxed INT handle points at [value:i64]  (get-int reads the i64)
    //  - a STRING handle points at [len:i32][utf8…]; str-get returns (ptr+4, len) via the retptr ABI
    // The test writes these structures into a `(data …)` image at fixed offsets and hands the top-level vec
    // handle to `read_effect_requests`. Ops are thin readers over that image.
    fn read_heap_stub() -> Vec<u8> {
        wat::parse_str(
            r#"(component
                 (core module $m
                   (memory (export "mem") 1)
                   (func (export "realloc") (param i32 i32 i32 i32) (result i32) (local.get 0))
                   ;; vec-len / arr-len share the [len:i32] header at the handle.
                   (func (export "vec-len") (param $h i32) (result i32) (i32.load (local.get $h)))
                   ;; vec-get(h, i) → i32 element at [h + 4 + 4*i].
                   (func (export "vec-get") (param $h i32) (param $i i32) (result i32)
                     (i32.load (i32.add (local.get $h) (i32.add (i32.const 4) (i32.mul (local.get $i) (i32.const 4))))))
                   ;; arr-get(h, i) → same layout as vec-get (a record is a sorted-field array).
                   (func (export "arr-get") (param $h i32) (param $i i32) (result i32)
                     (i32.load (i32.add (local.get $h) (i32.add (i32.const 4) (i32.mul (local.get $i) (i32.const 4))))))
                   ;; sum-disc → [h+0]; sum-payload → [h+4].
                   (func (export "sum-disc") (param $h i32) (result i32) (i32.load (local.get $h)))
                   (func (export "sum-payload") (param $h i32) (result i32) (i32.load (i32.add (local.get $h) (i32.const 4))))
                   ;; get-int → the i64 at the boxed-int handle.
                   (func (export "get-int") (param $h i32) (result i64) (i64.load (local.get $h)))
                   ;; bytes-len → [h+0]; bytes-get(h,i) → byte at [h+4+i].
                   (func (export "bytes-len") (param $h i32) (result i32) (i32.load (local.get $h)))
                   (func (export "bytes-get") (param $h i32) (param $i i32) (result i32)
                     (i32.load8_u (i32.add (i32.add (local.get $h) (i32.const 4)) (local.get $i))))
                   ;; str-get(h) → (ptr,len) via the retptr the canon ABI passes: write ptr=h+4, len=[h] to
                   ;; the two i32s at retptr, and lift reads the string from there.
                   (func (export "str-get") (param $h i32) (result i32)
                     (i32.store (i32.const 60000) (i32.add (local.get $h) (i32.const 4)))
                     (i32.store (i32.const 60004) (i32.load (local.get $h)))
                     (i32.const 60000))
                   ;; BUILD ops unused by the read path — bare sentinels (bind needs all 16 present).
                   (func (export "box-int") (param i64) (result i32) (i32.const 0))
                   (func (export "arr-alloc") (param i32) (result i32) (i32.const 0))
                   (func (export "arr-set") (param i32 i32 i32) (result i32) (local.get 0))
                   (func (export "sum-new") (param i32 i32) (result i32) (i32.const 0))
                   (func (export "str-new") (param i32 i32) (result i32) (i32.const 0))
                   (func (export "bytes-alloc") (param i32) (result i32) (i32.const 0))
                   (func (export "bytes-set") (param i32 i32 i32) (result i32) (local.get 0))
                   ;; ── The test image: ONE effect-request in a length-1 list. ────────────────────────────
                   ;; Offsets chosen disjoint. All multi-byte values little-endian.
                   ;; @100 vec: [len=1][elem0 = record@120]
                   (data (i32.const 100) "\01\00\00\00\78\00\00\00")
                   ;; @120 record (4 sorted fields): [len=4][correlation@200][kind@160][payload@240][target@180]
                   (data (i32.const 120) "\04\00\00\00\c8\00\00\00\a0\00\00\00\f0\00\00\00\b4\00\00\00")
                   ;; @160 boxed kind disc = 1 (Http), as an i64.
                   (data (i32.const 160) "\01\00\00\00\00\00\00\00")
                   ;; @180 target string: [len=8]"https://" — header then bytes.
                   (data (i32.const 180) "\08\00\00\00https://")
                   ;; @200 correlation = Some(@220): [disc=0][payload=@220]
                   (data (i32.const 200) "\00\00\00\00\dc\00\00\00")
                   ;; @220 bytes "id7": [len=3]"id7"
                   (data (i32.const 220) "\03\00\00\00id7")
                   ;; @240 payload = None: [disc=1][payload ignored=0]
                   (data (i32.const 240) "\01\00\00\00\00\00\00\00")
                   ;; @300 a BOGUS vec: header reports len=0xFFFFFFFF but there are NO element slots. The
                   ;; DoS-guard test hands this handle to read_effect_requests: with the MAX_PREALLOC cap the
                   ;; host does NOT eager-reserve ~4G*sizeof — it reserves the cap, then vec-get walks past
                   ;; the single-page memory and TRAPS (clean Err), never an alloc-abort.
                   (data (i32.const 300) "\ff\ff\ff\ff")
                   ;; @400 an EMPTY vec: [len=0], no element slots. read_effect_requests must return an
                   ;; empty Vec (the B1 empty-effects fold shape) without ever calling vec-get.
                   (data (i32.const 400) "\00\00\00\00")
                   ;; @420 a record whose correlation = Some(EMPTY bytes) and payload = None — pins that a
                   ;; present-but-empty payload round-trips as Some(vec![]), DISTINCT from None. Reuses the
                   ;; @160 kind (Http) + @180 target. [len=4][correlation@452][kind@160][payload@240][target@180]
                   (data (i32.const 420) "\04\00\00\00\c4\01\00\00\a0\00\00\00\f0\00\00\00\b4\00\00\00")
                   ;; @452 correlation = Some(@464): [disc=0][payload=@464]. (Kept clear of @440's field span.)
                   (data (i32.const 452) "\00\00\00\00\d0\01\00\00")
                   ;; @464 EMPTY bytes: [len=0], no bytes — read_bytes returns vec![].
                   (data (i32.const 464) "\00\00\00\00")
                   ;; @500 a length-1 vec whose element record@532 carries a BOGUS option discriminant in its
                   ;; correlation sum (disc=7, neither Some(0) nor None(1)) — read_option_bytes must hard-Trap
                   ;; (ABI drift), not silently default. [len=1][elem0=record@532] (0x0214 = 532)
                   (data (i32.const 500) "\01\00\00\00\14\02\00\00")
                   ;; @532 record: [len=4][correlation@560][kind@160][payload@240][target@180]
                   (data (i32.const 532) "\04\00\00\00\30\02\00\00\a0\00\00\00\f0\00\00\00\b4\00\00\00")
                   ;; @560 correlation sum with a BOGUS disc=7: [disc=7][payload ignored=0]
                   (data (i32.const 560) "\07\00\00\00\00\00\00\00"))
                 (core instance $i (instantiate $m))
                 (func $box-int (param "v" s64) (result u32) (canon lift (core func $i "box-int")))
                 (func $arr-alloc (param "len" u32) (result u32) (canon lift (core func $i "arr-alloc")))
                 (func $arr-set (param "arr" u32) (param "index" u32) (param "elem" u32) (result u32) (canon lift (core func $i "arr-set")))
                 (func $sum-new (param "disc" u32) (param "payload" u32) (result u32) (canon lift (core func $i "sum-new")))
                 (func $vec-len (param "v" u32) (result u32) (canon lift (core func $i "vec-len")))
                 (func $str-new (param "s" string) (result u32) (canon lift (core func $i "str-new") (memory $i "mem") (realloc (func $i "realloc"))))
                 (func $vec-get (param "v" u32) (param "index" u32) (result u32) (canon lift (core func $i "vec-get")))
                 (func $arr-get (param "arr" u32) (param "index" u32) (result u32) (canon lift (core func $i "arr-get")))
                 (func $sum-disc (param "handle" u32) (result u32) (canon lift (core func $i "sum-disc")))
                 (func $sum-payload (param "handle" u32) (result u32) (canon lift (core func $i "sum-payload")))
                 (func $get-int (param "handle" u32) (result s64) (canon lift (core func $i "get-int")))
                 (func $str-get (param "handle" u32) (result string) (canon lift (core func $i "str-get") (memory $i "mem") (realloc (func $i "realloc"))))
                 (func $bytes-alloc (param "len" u32) (result u32) (canon lift (core func $i "bytes-alloc")))
                 (func $bytes-set (param "buf" u32) (param "index" u32) (param "value" u32) (result u32) (canon lift (core func $i "bytes-set")))
                 (func $bytes-len (param "buf" u32) (result u32) (canon lift (core func $i "bytes-len")))
                 (func $bytes-get (param "buf" u32) (param "index" u32) (result u32) (canon lift (core func $i "bytes-get")))
                 (instance $heap
                   (export "box-int" (func $box-int))
                   (export "arr-alloc" (func $arr-alloc))
                   (export "arr-set" (func $arr-set))
                   (export "sum-new" (func $sum-new))
                   (export "vec-len" (func $vec-len))
                   (export "str-new" (func $str-new))
                   (export "vec-get" (func $vec-get))
                   (export "arr-get" (func $arr-get))
                   (export "sum-disc" (func $sum-disc))
                   (export "sum-payload" (func $sum-payload))
                   (export "get-int" (func $get-int))
                   (export "str-get" (func $str-get))
                   (export "bytes-alloc" (func $bytes-alloc))
                   (export "bytes-set" (func $bytes-set))
                   (export "bytes-len" (func $bytes-len))
                   (export "bytes-get" (func $bytes-get)))
                 (export "cadenza:runtime/heap" (instance $heap)))"#,
        )
        .expect("assemble read-ops heap stub")
    }

    fn bind_read_stub() -> HeapHandle<()> {
        let bytes = read_heap_stub();
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let component =
            wasmtime::component::Component::new(&engine, &bytes).expect("valid read stub");
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("read stub instantiates");
        match HeapHandle::bind(store, &instance) {
            Ok(h) => h,
            Err(e) => panic!("bind HeapHandle to the read stub: {e:?}"),
        }
    }

    // The end-to-end read: a length-1 effect-request list at @100 decodes to one EffectRequest with kind
    // Http (boxed disc 1 via get-int, NOT sum-disc), target "https://", correlation Some(b"id7"), payload
    // None. Pins the sorted-field ORDER, the boxed-enum-disc read op, and the option Some/None decode.
    #[test]
    fn reads_a_one_element_effect_request_list() {
        let mut heap = bind_read_stub();
        let effects = read_effect_requests(&mut heap, 100).expect("read effect list");
        assert_eq!(effects.len(), 1);
        let e = &effects[0];
        assert!(
            matches!(e.kind, EffectKind::Http),
            "kind = boxed disc 1 = Http"
        );
        assert_eq!(e.target, "https://");
        assert_eq!(
            e.correlation,
            Some(b"id7".to_vec()),
            "correlation = Some(b\"id7\")"
        );
        assert_eq!(e.payload, None, "payload = None (disc 1)");
    }

    // Every effect-kind discriminant 0..=5 maps to the right variant; 6 (out of range) is a hard error, not
    // a silent default — a runtime-ABI drift must surface.
    #[test]
    fn effect_kind_discriminants_map_and_reject_out_of_range() {
        assert!(matches!(disc_to_effect_kind(0), Ok(EffectKind::Shell)));
        assert!(matches!(disc_to_effect_kind(1), Ok(EffectKind::Http)));
        assert!(matches!(disc_to_effect_kind(2), Ok(EffectKind::Model)));
        assert!(matches!(disc_to_effect_kind(3), Ok(EffectKind::Now)));
        assert!(matches!(disc_to_effect_kind(4), Ok(EffectKind::Timer)));
        assert!(matches!(disc_to_effect_kind(5), Ok(EffectKind::Emit)));
        match disc_to_effect_kind(6) {
            Err(ComponentError::Trap(msg)) => assert!(msg.contains("out of range")),
            other => panic!("disc 6 must be a hard Trap, got {other:?}"),
        }
    }

    // DoS guard (reviewer, #2166 read-side): a reducer-returned list handle whose vec-len reports a bogus
    // huge length (0xFFFFFFFF) with no real element slots must NOT abort the host via an eager multi-GB
    // Vec::with_capacity — the cap bounds the reservation, then vec-get walks past memory and Traps cleanly.
    // The test PASSING (returns, doesn't abort/OOM the process) is itself the proof the cap works.
    #[test]
    fn a_bogus_huge_vec_len_traps_cleanly_not_an_alloc_abort() {
        let mut heap = bind_read_stub();
        match read_effect_requests(&mut heap, 300) {
            Err(_) => {} // clean fail-loud (a vec-get memory Trap), NOT a process abort
            Ok(v) => panic!(
                "a bogus u32::MAX vec-len must not read as a real {}-element list",
                v.len()
            ),
        }
    }

    // An EMPTY effect-request list (vec-len == 0) reads back as an empty Vec — the B1 empty-effects fold
    // shape. Pins that the zero-length path returns Ok(vec![]) (never calls vec-get, never errors), so a
    // reducer that emits no effects is decoded as "no effects", not a spurious read failure.
    #[test]
    fn reads_an_empty_effect_request_list_as_empty_vec() {
        let mut heap = bind_read_stub();
        let effects = read_effect_requests(&mut heap, 400).expect("read empty effect list");
        assert!(
            effects.is_empty(),
            "an empty (len-0) list must decode to an empty Vec, got {} effects",
            effects.len()
        );
    }

    // `Some([])` (a present-but-empty payload) round-trips DISTINCT from `None` — read_option_bytes must
    // return Some(vec![]) for disc 0 with empty bytes, not collapse it to None. Guards the documented
    // "empty-but-present" distinction the fold boundary depends on (an emitted effect with an explicit
    // empty correlation is not the same as no correlation).
    #[test]
    fn reads_some_empty_bytes_distinct_from_none() {
        let mut heap = bind_read_stub();
        let e =
            read_effect_request(&mut heap, 420).expect("read record with Some(empty) correlation");
        assert_eq!(
            e.correlation,
            Some(Vec::new()),
            "Some(empty bytes) must decode to Some(vec![]), distinct from None"
        );
        assert_eq!(e.payload, None, "payload is still None (disc 1)");
    }

    // A bogus option<list<u8>> discriminant (7 — neither Some(0) nor None(1)) is a hard Trap, NOT a silent
    // default. This is read_option_bytes's ABI-drift guard, the option-side analog of the effect-kind
    // discriminant reject; walked end-to-end via read_effect_requests so the whole read path surfaces it.
    #[test]
    fn a_bogus_option_discriminant_hard_traps_not_defaults() {
        let mut heap = bind_read_stub();
        match read_effect_requests(&mut heap, 500) {
            // Assert on the VARIANT (a hard Trap) + a STABLE, SPECIFIC anchor: the substring "discriminant 7"
            // (production format is "... discriminant {other} ..."). NOT the full human-readable phrase
            // (brittle to rewording, #2216) NOR a bare contains('7') (too loose — matches "17"/any stray 7,
            // #2222 overshoot). The middle ground pins the bogus disc is surfaced without coupling to the message.
            Err(ComponentError::Trap(msg)) => {
                assert!(
                    msg.contains("discriminant 7"),
                    "expected an option ABI-drift trap naming the bogus discriminant 7, got {msg:?}"
                )
            }
            other => panic!("a bogus option discriminant must hard-Trap, got {other:?}"),
        }
    }
}

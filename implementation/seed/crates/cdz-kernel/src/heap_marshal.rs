//! heap_marshal — the reducer-boundary INPUT marshalling (operator ruling C, §19e).
//!
//! The option-C reducer boundary keeps `wit/reducer.wit` STRUCTURALLY typed (`content-type`,
//! `option<list<u8>>`, `list<effect-request>`), but a Cadenza reducer compiled by `rcdzc` lowers its
//! `fold.apply` to `apply(u32, u32, u32) -> u32` over the SHARED `cadenza:runtime/heap` value-heap: every
//! structural argument crosses as an OPAQUE `u32` handle into that heap (the spec's "components composed
//! against a shared runtime exchange values as handles").
//!
//! ## When this module is on the call path (NOT the current live boundary yet)
//! This module is the marshalling helper for that **option-C handle-lowered MODE**: WHEN a reducer is
//! invoked via the `apply(u32, u32, u32)` handle-ABI, the host marshals the kernel's `(content_type,
//! payload, resumes)` fold inputs INTO value-heap handles first, using this module. It is NOT yet the live
//! boundary — [`crate::wasm_host`]'s [`ComponentReducer`](crate::wasm_host::ComponentReducer) still calls
//! the WIT-STRUCTURAL `fold.apply` via wasmtime's `bindgen!` (auto canon lift/lower of the structural WIT
//! types), and that stays the path until the handle-ABI fold-boundary rebind lands. So this is a correct,
//! tested helper staged AHEAD of its wiring — the fold-boundary rebind is the slice that puts it on the
//! call path. It drives [`HeapHandle`]'s public build ops (str-new / box-int / arr-alloc / arr-set /
//! sum-new / bytes-from / unit); the READ-direction dual (projecting the returned `list<effect-request>`
//! handle back to `Vec<EffectRequest>`) is [`crate::heap_unmarshal`].
//!
//! ## The value-heap layout this encodes (verified against rcdzc's wasm backend)
//! - A **record** is a SORTED-field-NAME array (`Core::Proj` layout — the fields laid out in field-name
//!   order, NOT declaration order), each field a BOXED handle. `content-type { family, version }`:
//!   `"family" < "version"` alphabetically, so `arr[0] = str-new(family)`, `arr[1] = box-int(version)`.
//! - An **`option<T>`** is a `sum-new(disc, payload)`. The discriminant is the VARIANT-DECLARATION order
//!   (`rcdzc` db.rs: "variants in declaration order — each position's index is its discriminant"), and for
//!   the prelude `Option` that is **`Some = 0`, `None = 1`** (sums.rs:80). `None`'s payload is the
//!   INLINE-UNIT (`arr-alloc(0)`, via [`HeapHandle::unit`]) — NOT handle `0`, which is NULL and would make
//!   a malformed sum (github-liaison #2122).
//! - **`list<u8>`** (a `Bytes` payload) is a value-heap bytes buffer built by [`HeapHandle::bytes_from`].
//!
//! Keeping this in its own module (vs. inside `wasm_host`) keeps the marshalling POLICY — which builds the
//! host↔guest value-heap agreement — separate from the `HeapHandle` mechanism it drives, and unit-testable
//! against a synthetic heap stub without a full reducer.

use crate::wasm_host::{ComponentError, HeapHandle};

/// The prelude `Option` discriminants, in variant-declaration order (`Some` first). `sum-new(disc, …)`
/// takes these — a Cadenza reducer reading its `option<list<u8>>` arg matches on the SAME discriminants.
const OPTION_SOME: u32 = 0;
const OPTION_NONE: u32 = 1;

/// Marshal a `content-type { family: string, version: u32 }` record into a value-heap record handle.
///
/// The record is a SORTED-field-name array (`Core::Proj`): `"family"` sorts before `"version"`, so
/// `arr[0]` is the boxed `family` string and `arr[1]` is the boxed `version` int. `version` is a `u32` in
/// the WIT but boxes through the value-heap's `box-int(s64)` — a `u32` widens losslessly to a non-negative
/// `i64`, so the reducer reads back exactly the version it was sent.
pub fn marshal_content_type<T>(
    heap: &mut HeapHandle<T>,
    family: &str,
    version: u32,
) -> Result<u32, ComponentError> {
    let family_h = heap.str_new(family)?;
    let version_h = heap.box_int(i64::from(version))?;
    let rec = heap.arr_alloc(2)?;
    let rec = heap.arr_set(rec, 0, family_h)?; // sorted idx 0 = "family"
    let rec = heap.arr_set(rec, 1, version_h)?; // sorted idx 1 = "version"
    Ok(rec)
}

/// Marshal an `option<list<u8>>` (a reducer `payload` / `resumes` arg) into a value-heap sum handle:
/// `Some(bytes)` → `sum-new(0, bytes-from(bytes))`, `None` → `sum-new(1, unit)`.
///
/// `Some(&[])` (an intentionally-EMPTY payload) is DISTINCT from `None` (absent): the former is
/// `sum-new(0, bytes-from(&[]))` — an empty-but-present bytes buffer under the `Some` discriminant — the
/// latter is the `None` discriminant over the inline-unit. This mirrors the WIT `option<list<u8>>` /
/// Rust `Option<Vec<u8>>` distinction the boundary preserves (reducer.wit: `none` ≠ `some([])`).
pub fn marshal_option_bytes<T>(
    heap: &mut HeapHandle<T>,
    data: Option<&[u8]>,
) -> Result<u32, ComponentError> {
    match data {
        Some(bytes) => {
            let payload = heap.bytes_from(bytes)?;
            heap.sum_new(OPTION_SOME, payload)
        }
        None => {
            // `None`'s payload is the inline-unit (`arr-alloc(0)`), never NULL (#2122).
            let unit = heap.unit()?;
            heap.sum_new(OPTION_NONE, unit)
        }
    }
}

/// Marshal the reducer `fold.apply` inputs — `(content-type, payload, resumes)` — into the three
/// value-heap handles the option-C lowered `apply(u32, u32, u32) -> u32` consumes, in argument order:
/// `(content_type_handle, payload_handle, resumes_handle)`. This is the whole build-direction input the
/// fold-boundary rebind hands to the guest's `apply`.
pub fn marshal_fold_inputs<T>(
    heap: &mut HeapHandle<T>,
    content_type: (&str, u32),
    payload: Option<&[u8]>,
    resumes: Option<&[u8]>,
) -> Result<(u32, u32, u32), ComponentError> {
    let ct = marshal_content_type(heap, content_type.0, content_type.1)?;
    let payload_h = marshal_option_bytes(heap, payload)?;
    let resumes_h = marshal_option_bytes(heap, resumes)?;
    Ok((ct, payload_h, resumes_h))
}

// ── ASYNC twins (#2256 / v-ah-host ask 27000): the marshalling for a handle-lowered fold on the ASYNC
// engine ([`AsyncComponentReducer::apply_handle_lowered_async`]). Identical build sequence to the sync
// forms above — the value-heap agreement (sorted-field record, Some=0/None=1 sum, empty-vs-absent bytes)
// has ONE source of truth in the sync forms' docs — but each heap op is driven via its `*_async` twin, so
// the sync `Func::call` per-store async-panic is avoided. `T: Send` (an async call may poll across await).

/// Async twin of [`marshal_content_type`].
pub async fn marshal_content_type_async<T: Send + 'static>(
    heap: &mut HeapHandle<T>,
    family: &str,
    version: u32,
) -> Result<u32, ComponentError> {
    let family_h = heap.str_new_async(family).await?;
    let version_h = heap.box_int_async(i64::from(version)).await?;
    let rec = heap.arr_alloc_async(2).await?;
    let rec = heap.arr_set_async(rec, 0, family_h).await?; // sorted idx 0 = "family"
    let rec = heap.arr_set_async(rec, 1, version_h).await?; // sorted idx 1 = "version"
    Ok(rec)
}

/// Async twin of [`marshal_option_bytes`].
pub async fn marshal_option_bytes_async<T: Send + 'static>(
    heap: &mut HeapHandle<T>,
    data: Option<&[u8]>,
) -> Result<u32, ComponentError> {
    match data {
        Some(bytes) => {
            let payload = heap.bytes_from_async(bytes).await?;
            heap.sum_new_async(OPTION_SOME, payload).await
        }
        None => {
            let unit = heap.unit_async().await?;
            heap.sum_new_async(OPTION_NONE, unit).await
        }
    }
}

/// Async twin of [`marshal_fold_inputs`].
pub async fn marshal_fold_inputs_async<T: Send + 'static>(
    heap: &mut HeapHandle<T>,
    content_type: (&str, u32),
    payload: Option<&[u8]>,
    resumes: Option<&[u8]>,
) -> Result<(u32, u32, u32), ComponentError> {
    let ct = marshal_content_type_async(heap, content_type.0, content_type.1).await?;
    let payload_h = marshal_option_bytes_async(heap, payload).await?;
    let resumes_h = marshal_option_bytes_async(heap, resumes).await?;
    Ok((ct, payload_h, resumes_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A synthetic `cadenza:runtime/heap` stub exporting all 16 ops (bind extracts every op, so all must be
    // present). BUILD ops that this module drives are made OBSERVABLE: str-new echoes the string LENGTH
    // (so a test sees which string was built), box-int echoes its argument (so version round-trips), sum-new
    // echoes its DISCRIMINANT (so a test sees Some=0 vs None=1), and arr-alloc/arr-set are FUNCTIONAL over a
    // real memory-backed record so a test can read a field back. The bytes ops are functional (a real
    // buffer). The unused READ ops (vec-*, arr-get, sum-disc/payload, get-int, str-get) are bare sentinels —
    // this module is the BUILD direction; the read dual has its own stub/tests.
    fn build_heap_stub() -> Vec<u8> {
        wat::parse_str(
            r#"(component
                 (core module $m
                   (memory (export "mem") 1)
                   (func (export "realloc") (param i32 i32 i32 i32) (result i32) (local.get 0))
                   ;; box-int ECHOES its arg (truncated to i32) so a boxed version round-trips through a test.
                   (func (export "box-int") (param $v i64) (result i32) (i32.wrap_i64 (local.get $v)))
                   ;; str-new ECHOES the string LENGTH (its 2nd canon-lowered arg) so a test sees the family.
                   (func (export "str-new") (param $ptr i32) (param $len i32) (result i32) (local.get $len))
                   ;; sum-new ECHOES its discriminant so a test can assert Some=0 / None=1 was built.
                   (func (export "sum-new") (param $disc i32) (param $payload i32) (result i32) (local.get $disc))
                   ;; arr-alloc/arr-set: a FUNCTIONAL record over memory — a handle is an offset to
                   ;; [len:i32][elem0:i32][elem1:i32]…, bump-allocated from 1024, so a test reads fields back.
                   (global $anext (mut i32) (i32.const 1024))
                   (func (export "arr-alloc") (param $len i32) (result i32)
                     (local $h i32)
                     (local.set $h (global.get $anext))
                     (i32.store (local.get $h) (local.get $len))
                     (global.set $anext
                       (i32.add (global.get $anext)
                         (i32.add (i32.const 4) (i32.mul (local.get $len) (i32.const 4)))))
                     (local.get $h))
                   (func (export "arr-set") (param $arr i32) (param $i i32) (param $elem i32) (result i32)
                     (i32.store
                       (i32.add (local.get $arr) (i32.add (i32.const 4) (i32.mul (local.get $i) (i32.const 4))))
                       (local.get $elem))
                     (local.get $arr))
                   ;; a test-only reader of the functional record (NOT a runtime op; used via arr-get below):
                   (func $aread (param $arr i32) (param $i i32) (result i32)
                     (i32.load (i32.add (local.get $arr) (i32.add (i32.const 4) (i32.mul (local.get $i) (i32.const 4))))))
                   ;; FUNCTIONAL bytes ops (real round-trip): handle = offset to [len:i32][bytes…] from 8192.
                   (global $bnext (mut i32) (i32.const 8192))
                   (func (export "bytes-alloc") (param $len i32) (result i32)
                     (local $h i32)
                     (local.set $h (global.get $bnext))
                     (i32.store (local.get $h) (local.get $len))
                     (global.set $bnext (i32.add (global.get $bnext) (i32.add (local.get $len) (i32.const 4))))
                     (local.get $h))
                   (func (export "bytes-set") (param $buf i32) (param $i i32) (param $v i32) (result i32)
                     (i32.store8 (i32.add (i32.add (local.get $buf) (i32.const 4)) (local.get $i)) (local.get $v))
                     (local.get $buf))
                   (func (export "bytes-len") (param $buf i32) (result i32) (i32.load (local.get $buf)))
                   (func (export "bytes-get") (param $buf i32) (param $i i32) (result i32)
                     (i32.load8_u (i32.add (i32.add (local.get $buf) (i32.const 4)) (local.get $i))))
                   ;; READ ops this module doesn't drive — bare sentinels (arr-get reads the functional record
                   ;; so the field-layout assertions can inspect it).
                   (func (export "vec-len") (param i32) (result i32) (i32.const 0))
                   (func (export "vec-get") (param i32 i32) (result i32) (i32.const 0))
                   (func (export "arr-get") (param $arr i32) (param $i i32) (result i32)
                     (call $aread (local.get $arr) (local.get $i)))
                   (func (export "sum-disc") (param i32) (result i32) (i32.const 0))
                   (func (export "sum-payload") (param i32) (result i32) (i32.const 0))
                   (func (export "get-int") (param i32) (result i64) (i64.const 0))
                   (func (export "str-get") (param i32) (result i32) (i32.const 0)))
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
        .expect("assemble build-ops heap stub")
    }

    fn bind_stub() -> HeapHandle<()> {
        let bytes = build_heap_stub();
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let component =
            wasmtime::component::Component::new(&engine, &bytes).expect("valid heap stub");
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("heap stub instantiates");
        // HeapHandle holds non-Debug Func/Store, so match rather than .expect().
        match HeapHandle::bind(store, &instance) {
            Ok(h) => h,
            Err(e) => panic!("bind HeapHandle to the build stub: {e:?}"),
        }
    }

    // content-type marshals to a 2-field record in SORTED field-name order: arr[0]=family (str-new echoes
    // its length → 4 for "http"), arr[1]=version (box-int echoes its value → 7). Proves the field ORDER
    // and that version widens through box-int losslessly.
    #[test]
    fn content_type_marshals_family_then_version_in_sorted_order() {
        let mut heap = bind_stub();
        let rec = marshal_content_type(&mut heap, "http", 7).expect("marshal ct");
        // The functional stub record: arr[0] = str-new("http") = len 4; arr[1] = box-int(7) = 7.
        assert_eq!(heap.arr_get(rec, 0).expect("field 0 = family"), 4);
        assert_eq!(heap.arr_get(rec, 1).expect("field 1 = version"), 7);
    }

    // A `Some(bytes)` payload builds `sum-new(0=Some, bytes-from(bytes))`: the stub's sum-new echoes the
    // discriminant, so we assert Some=0 was used; the round-trip of the bytes buffer is covered by
    // HeapHandle's own bytes test — here we pin the DISCRIMINANT choice (the marshalling policy).
    #[test]
    fn some_payload_uses_the_some_discriminant() {
        let mut heap = bind_stub();
        let h = marshal_option_bytes(&mut heap, Some(&[0xDE, 0xAD])).expect("marshal Some");
        assert_eq!(
            h, OPTION_SOME,
            "Some(bytes) must build sum-new with disc 0 (Some)"
        );
    }

    // A `Some(&[])` (intentionally-empty payload) is STILL the Some discriminant — distinct from None.
    #[test]
    fn empty_some_payload_is_still_some_not_none() {
        let mut heap = bind_stub();
        let h = marshal_option_bytes(&mut heap, Some(&[])).expect("marshal Some([])");
        assert_eq!(
            h, OPTION_SOME,
            "Some([]) is an empty-but-PRESENT payload — disc 0 (Some), never None"
        );
    }

    // A `None` payload builds `sum-new(1=None, unit)` — the None discriminant.
    #[test]
    fn none_payload_uses_the_none_discriminant() {
        let mut heap = bind_stub();
        let h = marshal_option_bytes(&mut heap, None).expect("marshal None");
        assert_eq!(h, OPTION_NONE, "None must build sum-new with disc 1 (None)");
    }

    // The full fold-input triple: content-type record + two option-bytes args, in apply's argument order.
    // Proves the three handles are built and returned positionally (ct, payload, resumes).
    #[test]
    fn fold_inputs_marshal_the_three_apply_args() {
        let mut heap = bind_stub();
        let (ct, payload, resumes) =
            marshal_fold_inputs(&mut heap, ("model", 2), Some(&[1, 2, 3]), None)
                .expect("marshal fold inputs");
        // ct is a record handle: field 0 = str-new("model") len 5, field 1 = box-int(2).
        assert_eq!(heap.arr_get(ct, 0).expect("ct family"), 5);
        assert_eq!(heap.arr_get(ct, 1).expect("ct version"), 2);
        // payload = Some(3 bytes) → Some disc; resumes = None → None disc.
        assert_eq!(payload, OPTION_SOME, "payload Some → disc 0");
        assert_eq!(resumes, OPTION_NONE, "resumes None → disc 1");
    }
}

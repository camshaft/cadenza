//! Bytes and string operations
//!
//! Bytes buffer, rope concat/slice, and UTF-8 string operations.

use super::*;

// ─── Bytes: a packed immutable byte buffer (in `raw`) ───────────────────────────────────
// OOB into a valid buffer traps; null is benign.

// The shared IMMORTAL empty-BYTES singleton (lazily minted, census-excluded) — see op_bytes_alloc.
runtime_local! {
    static EMPTY_BYTES: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

pub(crate) fn op_bytes_alloc(len: u32) -> Handle {
    // len==0 → the shared IMMORTAL empty-BYTES singleton (the IMM_UNIT analog for bytes): an empty bytes
    // value is CONSTANT, so allocate it ONCE, immortal (census-excluded), reuse. SOUND: an empty bytes is
    // never mutated in place — bytes-set on it is OOB (traps, 0 slots), concat builds a fresh Rope leaf,
    // and bytes_flatten is a no-op on an already-flat empty leaf. So the singleton is read-only.
    if len == 0 {
        return EMPTY_BYTES.with(|slot| {
            let mut e = slot.get();
            if e.0.is_null() {
                e = alloc_raw(
                    Vec::new(),
                    Raw::Inline {
                        len: 0,
                        buf: [0u8; INLINE_RAW_CAP],
                    },
                );
                op_mark_immortal(e);
                slot.set(e);
            }
            e
        });
    }
    // A ≤INLINE_RAW_CAP-byte buffer (a short string/section leaf — the common case when assembling a
    // rope from many small pieces) builds its zero-filled raw INLINE, skipping the transient `vec![0u8;
    // len]` that `alloc` would otherwise copy into the inline `Raw` and immediately free. That transient
    // Vec was pure malloc/free churn on the hot leaf-build path (dominant in a rope-assembly profile).
    if (len as usize) <= INLINE_RAW_CAP {
        return alloc_raw(
            Vec::new(),
            Raw::Inline {
                len: len as u8,
                buf: [0u8; INLINE_RAW_CAP],
            },
        );
    }
    alloc(Vec::new(), vec![0u8; len as usize])
}
/// Store a byte (the compiler guarantees `value` is 0–255) and return the buffer handle. OOB into a
/// valid buffer traps; null is a no-op.
pub(crate) fn op_bytes_set(buf: Handle, index: u32, value: u32) -> Handle {
    if is_immediate(buf) {
        return buf; // defensive (mirrors op_bytes_get/len): a bytes buffer is never an immediate;
        // return the handle unchanged (no-op write), never deref the tagged bits
    }
    match unsafe { buf.node_mut() } {
        None => {}
        // A `Rope` leaf (an already-refcounted leaf — a `bytes-concat`/`bytes-slice` result kept as
        // `Raw::Rope` by `bytes_leaf_from_bytevec`, or a `promote_leaf_to_rope` leaf; NOT the alloc+set
        // build path, whose >cap buffer is a `Raw::Heap` — `Raw::from` no longer Rope-ifies a fresh
        // >cap `Vec`, see `From<Vec<u8>> for Raw`) writes in place via `ByteVec::set_byte`, which
        // COPIES-ON-WRITE the backing `Bytes` chunk if it is shared (so a sibling `ByteVec` slicing the
        // same chunk is not corrupted), keeping the leaf a `Rope` rather than letting `as_mut_slice`
        // materialize it to a plain `Vec` — the shared storage stays refcounted.
        Some(n) => match &mut n.raw {
            Raw::Rope(bv) => {
                if bv.set_byte(index as usize, value as u8).is_err() {
                    trap_oob();
                }
            }
            raw => match raw.as_mut_slice().get_mut(index as usize) {
                Some(slot) => *slot = value as u8,
                None => trap_oob(),
            },
        },
    }
    buf
}
/// `bytes-get` — the logical byte at `index`. A single-chunk/inline leaf reads `raw` directly (O(1)); a
/// MULTI-CHUNK `Raw::Rope` leaf (a `bytes-concat`/`bytes-slice` result) is COMPACTED to one chunk in
/// place on this first full-read, then read (see `bytes_flatten` — this keeps the compiler's `0..len`
/// emit loop O(n) not O(n²) on a deep concat chain). OOB into a valid buffer traps; null is benign.
pub(crate) fn op_bytes_get(buf: Handle, index: u32) -> u32 {
    if is_immediate(buf) {
        return 0; // cross-kind totality: a bytes buffer is never itself an immediate
    }
    // Null-benign; then: a MULTI-CHUNK Rope leaf needs one compaction so `raw.get` sees contiguous
    // bytes (bounds-check first so a stray OOB doesn't force it). A single-chunk/inline leaf skips
    // straight to the O(1) direct read — the hot per-char path, unchanged.
    let multichunk = match unsafe { buf.node_ref() } {
        None => return 0,
        Some(n) => matches!(&n.raw, Raw::Rope(bv) if bv.as_contiguous().is_none()),
    };
    if multichunk {
        if index >= op_bytes_len(buf) {
            trap_oob();
        }
        bytes_flatten(buf); // compacts the multi-chunk Rope leaf in place → single contiguous chunk
    }
    match unsafe { buf.node_ref() } {
        None => 0,
        Some(n) => match n.raw.get(index as usize) {
            Some(&b) => b as u32,
            None => trap_oob(),
        },
    }
}

/// `bytes-len` — the logical byte length. O(1): every Bytes value is a LEAF (`bytes-concat`/`bytes-slice`
/// build `Raw::Rope` ByteVec leaves, never child-node ropes), so the length is its `raw`'s logical length
/// (`ByteVec::len` sums the chunks of a multi-chunk `Rope`; `Inline`/`Heap` return their byte count).
pub(crate) fn op_bytes_len(buf: Handle) -> u32 {
    if is_immediate(buf) {
        return 0; // cross-kind totality: a bytes buffer is never itself an immediate
    }
    with_node(buf, 0, |n| n.raw.len() as u32)
}

/// `bytes-new` (heap op, appended seq 916) — build a fresh Bytes leaf from a whole `list<u8>` in ONE call
/// (alloc + bulk-copy), the bulk twin of `bytes-alloc` + N× `bytes-set`. The LIFT-side marshaling-boundary op:
/// the compiler emits it to lift a host-provided byte slice (a reducer event field, already contiguous in
/// guest linear memory) into a heap Bytes in ONE cross-component call instead of one call/byte. Mirrors
/// `op_str_new` minus UTF-8 validation: an EMPTY slice → the shared IMMORTAL empty-BYTES singleton (reuse the
/// `op_bytes_alloc` len==0 mint-once path); else a fresh owned leaf holding `data` VERBATIM (`alloc` stores it
/// inline when ≤INLINE_RAW_CAP, in a refcounted ByteVec (Raw::Rope) when larger). A CONSTRUCTOR — produces a NEW owned Bytes and
/// CONSUMES nothing (`data` arrives by value, not as a handle). Called by `Guest::bytes_new`.
pub(crate) fn op_bytes_new(data: Vec<u8>) -> Handle {
    if data.is_empty() {
        return op_bytes_alloc(0); // the shared IMMORTAL empty-BYTES singleton (mint-once, census-excluded)
    }
    alloc(Vec::new(), data)
}

/// `bytes-read` (heap op, appended seq 916) — return the WHOLE logical byte content of `buf` as a `Vec<u8>` in
/// ONE call, the bulk twin of N× `bytes-get` and the LOWER-side companion of `bytes-new`. The compiler emits it
/// to lower a heap Bytes back to a host byte slice (a reducer result) in ONE cross-component call instead of one
/// call/byte. Mirrors `op_str_get`: COMPACT first (`buf` may be a multi-chunk `Raw::Rope` leaf — a
/// `bytes-concat`/`bytes-slice` result — and `as_slice` needs one contiguous chunk), unobservable +
/// content-preserving, then copy the leaf's `raw` out. An INSPECTOR — BORROWS `buf` (rc unchanged; the caller owns the drop, like
/// `str-get`/`bytes-get`). A null/immediate handle reads as the empty list (cross-kind totality). Called by
/// `Guest::bytes_read`.
pub(crate) fn op_bytes_read(buf: Handle) -> Vec<u8> {
    if is_immediate(buf) {
        return Vec::new(); // cross-kind totality: a bytes buffer is never itself an immediate
    }
    bytes_flatten(buf);
    with_node(buf, Vec::new(), |n| n.raw.as_slice().to_vec())
}

// ─── Bytes rope: O(1) concat/slice over a chunk-sharing ByteVec leaf, compact-on-read ─────────
// A Bytes value is ALWAYS a single LEAF (empty `handles`) whose `raw` holds the content; a
// `bytes-concat`/`bytes-slice` result is a `Raw::Rope(ByteVec)` leaf whose ByteVec is a rope OF ITS
// OWN refcounted `Bytes` chunks (the "rope" lives INSIDE the ByteVec, NOT as a tree of child nodes).
// So concat/slice copy no bytes until observed — killing the O(n²) copy cascade a compiler would
// otherwise hit assembling a module by concatenating sections (deferred materialization behind the
// observable bytes, value-heap-runtime.md §Deferred Materialization Is Permitted Behind The Observable
// Bytes). Ownership follows the `arr-set` convention: concat/slice/compact CONSUME their Bytes operands
// (a UNIQUE operand's ByteVec is MOVED, a shared one refcount-cloned — see `content_bytevec_consuming`).
// A multi-chunk `Rope` is compacted to one contiguous chunk on read (`bytes_flatten`).

/// Compact a bytes/string LEAF's content to a single contiguous chunk IN PLACE. Every Bytes value is a
/// leaf — a `bytes-concat`/`bytes-slice` result is a possibly-MULTI-CHUNK `Raw::Rope(ByteVec)` leaf (the
/// rope is the ByteVec's own chunk deque, not a tree of child nodes) — so this just `ByteVec::compact`s a
/// multi-chunk `Rope`, making every content reader (`as_slice`, `bytes-get`/`-read`, the champ key
/// hash/eq/cmp) see one `&[u8]`. No-op on an inline/heap or already-single-chunk leaf. Content-preserving,
/// so UNOBSERVABLE and safe even when shared (`rc > 1`) — every reader sees identical bytes before and
/// after: a value derived by combining/narrowing may defer materialization until observed.
//= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
//# A value the compiler derives by combining or narrowing existing values MAY defer the work of materializing its contents until an operation observes them, provided the deferral is not observable and is a deterministic function of the source, so that combining and narrowing values need not eagerly copy their contents.
pub(crate) fn bytes_flatten(h: Handle) {
    if let Some(n) = unsafe { h.node_mut() } {
        if let Raw::Rope(bv) = &mut n.raw {
            if bv.as_contiguous().is_none() {
                bv.compact();
            }
        }
    }
}

/// A bytes/string value's content as an owned `ByteVec`, sharing rather than copying where possible:
/// a `Rope` leaf's `ByteVec` clones by REFCOUNT (a bump on its shared `Bytes` chunks, no byte copy);
/// an inline/heap leaf copies its short bytes into a fresh `ByteVec`. BORROWS `h` (rc unchanged) — the
/// caller consumes `h` (via `op_drop`) after. A null/immediate reads as empty.
fn content_bytevec(h: Handle) -> etude_bytevec::ByteVec {
    if is_immediate(h) {
        return etude_bytevec::ByteVec::default();
    }
    with_node(h, etude_bytevec::ByteVec::default(), |n| match &n.raw {
        Raw::Rope(bv) => (**bv).clone(), // refcount bump — shares the Bytes chunks, no byte copy
        other => etude_bytevec::ByteVec::from(other.as_slice().to_vec()),
    })
}

/// Like [`content_bytevec`] but for a CONSUMED operand (the caller `op_drop`s `h` right after): when `h`'s
/// node is UNIQUELY owned (`rc == 1`), MOVE its `ByteVec` out instead of cloning it. This is the FBIP take
/// (mirrors the vector's rc==1 spine reuse) that keeps a growing concat cascade `rope = concat(rope, piece)`
/// — module assembly, the exact O(n²) the bytes rope exists to KILL — O(n): a clone re-refcounts the
/// accumulator's whole growing chunk container every step (O(n) work × n steps = O(n²)); a move is O(1).
/// SAFE because `rc == 1` means no other owner can observe `h`: emptying `h`'s raw is unobservable, and the
/// caller's `op_drop(h)` then frees just the shell — the moved chunks are no longer reachable through `h`,
/// so no double-free. A SHARED (`rc > 1`) or IMMORTAL (`rc == u32::MAX`, so `!= 1`) node CLONES (refcount-
/// shares the chunks) and does NOT mutate `h`'s raw — a sibling still needs the value. Callers run
/// `promote_leaf_to_rope(h)` FIRST, so a `Heap` leaf is already `Rope` here (the `Heap` arm is a defensive
/// fallback that reuses the buffer). A null/immediate reads as empty.
fn content_bytevec_consuming(h: Handle) -> etude_bytevec::ByteVec {
    if is_immediate(h) {
        return etude_bytevec::ByteVec::default();
    }
    // SHARED / IMMORTAL: clone (refcount-share the chunks); MUST NOT mutate `h`'s raw (a sibling reads it).
    if node_rc(h) != 1 {
        return content_bytevec(h);
    }
    // UNIQUE (rc == 1): take `h`'s content by MOVE. Replace the raw with an empty inline so the subsequent
    // `op_drop(h)` reclaims only the shell (a leaf already has empty handles). `Rope` moves the boxed
    // `ByteVec` out (no clone — the win); `Heap` reuses the `Vec`'s buffer; `Inline` copies its ≤cap bytes.
    match unsafe { h.node_mut() } {
        None => etude_bytevec::ByteVec::default(),
        Some(n) => {
            // Every Bytes value is a LEAF (concat/slice build ByteVec-rope leaves, never child-node ropes),
            // so taking `raw` moves the WHOLE content out. Pin that invariant: a future node-rope
            // reintroduction would make this silently take header-bytes as a ByteVec + orphan the children.
            debug_assert!(
                n.handles.is_empty(),
                "content_bytevec_consuming take assumes a bytes LEAF (empty handles)"
            );
            match core::mem::replace(
                &mut n.raw,
                Raw::Inline {
                    len: 0,
                    buf: [0u8; INLINE_RAW_CAP],
                },
            ) {
                Raw::Rope(bv) => *bv, // MOVE the boxed ByteVec out — the O(1) take that restores O(n) cascade
                Raw::Heap(v) => etude_bytevec::ByteVec::from(v), // reuse the buffer (unreachable after promote)
                Raw::Inline { len, buf } => etude_bytevec::ByteVec::from(buf[..len as usize].to_vec()),
            }
        }
    }
}

/// Allocate a fresh bytes/string LEAF (empty `handles`) holding `bv`'s content: INLINE when it fits the
/// cap (a short contiguous copy — preserves the no-heap short-leaf rep and the inline-small invariant
/// the reuse/rep tests pin), else a `Raw::Rope` that KEEPS the `ByteVec` (its chunks stay shared by
/// refcount — no byte copy). Reads/champ compact a multi-chunk `Rope` to a single chunk on demand
/// (see `bytes_flatten`).
fn bytes_leaf_from_bytevec(bv: etude_bytevec::ByteVec) -> Handle {
    if bv.len() <= INLINE_RAW_CAP {
        let bytes = bv.copy_to_bytes(); // ≤cap: one small contiguous copy into the inline buffer
        alloc_raw(alloc::vec::Vec::new(), Raw::inline(&bytes))
    } else {
        alloc_raw(
            alloc::vec::Vec::new(),
            Raw::Rope(alloc::boxed::Box::new(bv)),
        )
    }
}

/// `bytes-concat(a, b)` — a new Bytes = the bytes of `a` then `b`. Builds one `ByteVec` = a's chunks
/// then b's, appended by REFCOUNT (`ByteVec::append` moves the chunks — O(chunks), no byte copy), so a
/// concat cascade stays linear. CONSUMES `a` and `b` via `content_bytevec_consuming`: a UNIQUELY-owned
/// (`rc == 1`) operand's `ByteVec` is MOVED out (no clone), a shared one is refcount-cloned; then
/// `op_drop` releases our operand reference (freeing just the emptied shell on the move path). The move
/// is load-bearing for the growing cascade `rope = concat(rope, piece)` (module assembly): `rope` is
/// `rc == 1` each step, so moving its chunk container is O(1) — cloning it would be O(n) work × n steps
/// = the O(n²) copy the bytes rope exists to KILL. Empty operand is the identity.
pub(crate) fn op_bytes_concat(a: Handle, b: Handle) -> Handle {
    let la = op_bytes_len(a);
    let lb = op_bytes_len(b);
    if la == 0 {
        op_drop(a);
        return b;
    }
    if lb == 0 {
        op_drop(b);
        return a;
    }
    // Logical length is u32 across the ABI (`bytes-len -> u32`); a > 4 GiB Bytes is unrepresentable
    // on wasm32, so an overflow here is a compiler-invariant violation → trap.
    if la.checked_add(lb).is_none() {
        trap_oob();
    }
    // Promote `Heap` operands to shareable `Rope` first so `content_bytevec` RC-shares their chunks
    // instead of copying them: the result is `a`'s chunk then `b`'s chunk with NO byte copy (the true
    // O(1) concat), and a retained operand re-concatenated later shares rather than re-copies.
    promote_leaf_to_rope(a);
    promote_leaf_to_rope(b);
    let mut r = content_bytevec_consuming(a);
    let mut rb = content_bytevec_consuming(b);
    r.append(&mut rb);
    op_drop(a);
    op_drop(b);
    bytes_leaf_from_bytevec(r)
}

/// PROMOTE a `Heap` bytes leaf to a `Rope` leaf IN PLACE, reusing the `Vec`'s buffer as the single
/// `Bytes` chunk (zero-copy — `Bytes::from(Vec)` reuses the allocation). Content-preserving, so
/// UNOBSERVABLE even when `h` is shared (rc>1) — every sharer sees identical bytes, exactly like
/// `bytes_flatten`'s compaction (memory model #Sharing Is Not Observable). A no-op on an inline/Rope
/// leaf or a null/immediate. Why: a `Heap` leaf is eagerly-materialized and NOT shareable, so slicing
/// it copies the WHOLE parent every time; promoting it once lets this slice AND every later slice
/// RC-SHARE the parent's chunk — turning a lexer's repeated `String.at` from O(parent)/char into O(1)
/// without taxing plain construction (which stays cheap `Heap`, so a transient value never pays a `Box`).
fn promote_leaf_to_rope(h: Handle) {
    if let Some(n) = unsafe { h.node_mut() } {
        if matches!(n.raw, Raw::Heap(_)) {
            // Move the `Vec` out (replace with a trivial inline) then install the `Rope` — no byte copy.
            if let Raw::Heap(v) = core::mem::replace(
                &mut n.raw,
                Raw::Inline {
                    len: 0,
                    buf: [0u8; INLINE_RAW_CAP],
                },
            ) {
                n.raw = Raw::Rope(alloc::boxed::Box::new(etude_bytevec::ByteVec::from(v)));
            }
        }
    }
}

/// `bytes-slice(buf, start, len)` — a new Bytes = `len` bytes of `buf` from `start`, a single leaf with
/// no byte copy: `ByteVec::slice` narrows the parent's chunk view structurally (O(log n)). Total-or-trap:
/// `start + len > bytes-len(buf)` traps (checked in `u64`); `len == 0` is the empty Bytes (never a trap,
/// even at `start == len`). CONSUMES `buf`. A ≤cap result is materialized inline; a wider one stays a
/// `Raw::Rope` leaf sharing `buf`'s chunks (so a slice-of-a-slice just narrows the same chunk view).
///
/// The slice SHARES the parent's storage — `ByteVec::slice` structurally reuses the parent's `Bytes`
/// chunks (a refcount bump, no byte copy), so those chunks are genuinely RETAINED in the slice value's
/// own representation (its `ByteVec` holds them live), not hidden: the storage the slice retains is
/// exactly the storage it holds live, and dropping the slice releases its chunk references.
//= spec/capabilities/memory-and-resource-model.md#retained-storage-is-what-a-value-s-representation-holds-live
//# The storage a value retains MUST be the storage its representation actually holds live, so that a value that shares another value's storage keeps the shared storage retained rather than hidden.
pub(crate) fn op_bytes_slice(buf: Handle, start: u32, len: u32) -> Handle {
    let blen = op_bytes_len(buf);
    if (start as u64) + (len as u64) > (blen as u64) {
        trap_oob();
    }
    if len == 0 {
        op_drop(buf); // consume the operand; the empty result is independent
        return op_bytes_alloc(0);
    }
    // Promote a `Heap` parent to a shareable `Rope` first, so THIS slice and every later slice of the
    // same parent RC-share its chunk instead of each copying the whole parent (the repeated-`String.at`
    // lexer path). Then `ByteVec::slice` structurally narrows the chunk view (O(log n), no byte copy).
    // CONSUME `buf` via `content_bytevec_consuming`: a UNIQUE (`rc == 1`) parent's `ByteVec` is MOVED out
    // (no clone) then sliced; a shared one is refcount-cloned; `op_drop(buf)` releases our operand ref.
    promote_leaf_to_rope(buf);
    let r = content_bytevec_consuming(buf).slice((start as usize)..((start + len) as usize));
    op_drop(buf);
    bytes_leaf_from_bytevec(r)
}

/// `bytes-compact(buf)` — a Bytes equal to `buf` by content whose storage is INDEPENDENT of any
/// larger buffer `buf` was sliced from (memory-and-resource-model.md #Retained Storage: derive a
/// value that releases the parent's storage without changing the value). Falls out of the rope for
/// free: flattening `buf` in place materializes its own bytes and drops the parent it pinned, and a
/// leaf is already independent. CONSUMES and returns `buf` (now a leaf).
pub(crate) fn op_bytes_compact(buf: Handle) -> Handle {
    bytes_flatten(buf);
    buf
}

/// `str-nfc-normalize` (heap op 89, FINDING#23) — normalize a runtime String value to Unicode Normalization
/// Form C. A String is a UTF-8 byte leaf (possibly a rope): flatten `s`, read its logical bytes, normalize
/// them to NFC via the imported `cadenza:nfc/normalize` component (the runtime's DEPENDENCY — the heavy
/// Unicode tables live THERE, not in this runtime), and return a FRESH OWNED flat String leaf of the NFC
/// bytes. CONSUMES `s` (drops it; returns the fresh leaf) — the same spend-the-input contract as
/// `bytes-compact`/`str-to-bytes`. Idempotent (the imported `nfc` is). The NFC import is only linked in the
/// wasm component build; a native `cargo test` has no NFC component, so the call is gated to wasm and the
/// native build normalizes to a no-op passthrough (the native suite exercises the flatten/leaf plumbing, not
/// NFC content — NFC correctness is covered by cdz-nfc's own unit tests + the corpus witness).
#[cfg(target_arch = "wasm32")]
pub(crate) fn op_str_nfc(s: Handle) -> Handle {
    bytes_flatten(s);
    let bytes = unsafe { s.node_ref() }.map_or(&[][..], |n| n.raw.as_slice());
    let normalized = bindings::cadenza::nfc::normalize::nfc(bytes);
    // FAST PATH (the common case — ASCII / already-NFC text): if normalization changed nothing, `s` is
    // ALREADY its own NFC form, so return the input handle unchanged rather than allocating a fresh leaf +
    // dropping `s`. This matches the op's stated contract ("same handle, near-free for already-NFC") and
    // avoids heap churn on the overwhelmingly common path (most runtime text is ASCII or pre-composed).
    // Only a genuinely decomposed input (rare) allocates the fresh normalized leaf.
    if normalized == bytes {
        return s;
    }
    let out = alloc(Vec::new(), normalized);
    op_drop(s);
    out
}

/// Native stand-in for `op_str_nfc` (no NFC component linked off-wasm): flatten + return `s` unchanged. The
/// native suite covers the flatten/leaf plumbing; NFC content correctness lives in cdz-nfc's unit tests.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn op_str_nfc(s: Handle) -> Handle {
    bytes_flatten(s);
    s
}

// ─── String: a stored UTF-8 leaf (bytes in `raw`) ───────────────────────────────────────

// The shared IMMORTAL empty-STRING singleton (lazily minted, census-excluded) — see op_str_new.
runtime_local! {
    static EMPTY_STR: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

pub(crate) fn op_str_new(s: String) -> Handle {
    // Empty string → the shared IMMORTAL empty-STRING singleton (the IMM_UNIT analog for strings): an
    // empty string is CONSTANT, so allocate it ONCE, immortal (census-excluded), reuse. SOUND: an empty
    // string is never mutated in place — String.concat builds a fresh Rope leaf, and bytes_flatten /
    // str-get are no-ops on an already-flat empty leaf. Read-only singleton.
    if s.is_empty() {
        return EMPTY_STR.with(|slot| {
            let mut e = slot.get();
            if e.0.is_null() {
                e = alloc(Vec::new(), Vec::new());
                op_mark_immortal(e);
                slot.set(e);
            }
            e
        });
    }
    alloc(Vec::new(), s.into_bytes())
}
pub(crate) fn op_str_get(h: Handle) -> String {
    if is_immediate(h) {
        return String::new(); // cross-kind totality: a string is never itself an immediate
    }
    // A runtime String IS a bytes value: `String.concat`/`.at`-slice reuse `op_bytes_concat`/
    // `op_bytes_slice` (a String shares the Bytes representation), which build `Raw::Rope(ByteVec)`
    // LEAVES that may be MULTI-CHUNK, so `h`'s `raw` may be a chunk rope rather than one contiguous
    // buffer. COMPACT it to a single chunk first (`bytes_flatten` compacts a multi-chunk `Rope` in place;
    // content-preserving, so unobservable even on a shared value) — exactly as `op_bytes_get` and
    // value-encode's `Shape::Str` arm do. Without the compaction `as_slice` on a multi-chunk `Rope`
    // panics. A single-chunk/inline leaf is left untouched (flatten is a no-op there), so a plain
    // `str-new` string is unaffected.
    bytes_flatten(h);
    with_node(h, String::new(), |n| {
        String::from_utf8_lossy(&n.raw).into_owned()
    })
}

/// `String.from-bytes` — the TOTAL UTF-8 decode `Bytes → (Option String)`: validate a RUNTIME byte
/// buffer as well-formed UTF-8 (strict: rejects invalid bytes, overlong encodings, AND surrogate code
/// points — the three spec failure modes), returning the buffer AS a String (Some) or NULL (None). A
/// String IS a byte leaf (`op_str_new` = `alloc(bytes)`, byte-identical to a Bytes leaf), so a VALID
/// buffer needs no conversion — it is already a valid String; the op is UTF-8 VALIDATION + a re-tag.
/// CONSUMES `buf`: on success `buf` flows out as the String (its ownership transfers to the result); on
/// failure the caller drops it. COMPACT first (`buf` may be a multi-chunk `Raw::Rope` leaf — a
/// `Bytes.concat`/`.slice` result — and strict `from_utf8` must see one contiguous buffer), exactly as
/// `op_str_get`/`op_bytes_get`/value-encode's `Shape::Str` arm do. Returns `Handle::NULL` for invalid
/// UTF-8 so the compiler can build the `(Option String)` sum (`Some buf` / `None`), or wrap directly.
///
/// WIT-EXPORTED at index 85 (`str-from-bytes`) — the runtime half of the coordinated `String.from-bytes`
/// op, called by the `Guest::str_from_bytes` method. The compiler emits it (`Core::StrFromBytes`) when
/// `String.from-bytes` is applied to a RUNTIME byte sequence (a constant `Bytes.of` still folds in
/// lower.rs). The load-bearing logic (flatten + strict validate + consume/re-tag) lives here.
pub(crate) fn op_str_from_bytes(buf: Handle) -> Handle {
    if is_immediate(buf) {
        // The empty/inline-unit Bytes: no bytes → the empty string is valid UTF-8. `buf` (an immediate)
        // is itself a valid empty leaf-equivalent; return it (an immediate is a fine empty String).
        return buf;
    }
    bytes_flatten(buf);
    let valid = with_node(buf, false, |n| core::str::from_utf8(&n.raw).is_ok());
    if valid {
        buf // already a flat, valid-UTF-8 leaf — a String IS a byte leaf, no conversion
    } else {
        op_drop(buf); // ill-formed → None; release the consumed operand
        Handle::NULL
    }
}

/// `String.scalar-at` — the codepoint of the `scalar_index`-th UNICODE SCALAR of a String, or the
/// sentinel `NO_SCALAR` (`u32::MAX`) when the index is out of range. The SCALAR index is NOT the byte
/// index: a String is a UTF-8 byte-rope, so the Nth scalar can start at any byte offset (`"café"` has
/// byte-len 5 but scalar-len 4 — its scalar 3 `'é'` is a 2-byte encoding at byte offset 3). Returns the
/// scalar's Unicode codepoint as a `u32` (a `Char` at the language level is that codepoint immediate) —
/// UNLIKE `String.at`, whose `(Option String)` payload is a one-scalar SLICE ROPE that the physical
/// `champ_eq` mis-compares (the rope-eq bug the compiler-in-Cadenza lexer WORKS AROUND by lexing
/// `List Int64` char-codes). A `Char` codepoint is an ordinary integer, so comparing two of them is a
/// plain `i32.eq` — no rope, no content-eq hazard: this is the op a real text lexer wants.
///
/// COMPACT first (`buf` may be a multi-chunk `Raw::Rope` leaf — a `Bytes.concat`/`.slice` result — and
/// the UTF-8 scan must see one contiguous buffer); content-preserving, so UNOBSERVABLE on a shared
/// value — exactly as `op_str_get`/`op_bytes_get`/`str-from-bytes` do. BORROWS `buf` (an indexed read,
/// no consume). Decodes the flat leaf as UTF-8 and takes the Nth `char`; a well-formed String always
/// decodes, but an ill-formed buffer (defensive) reads as `NO_SCALAR`, never a trap.
///
/// WIT-EXPORTED as `bytes-scalar-at` (the runtime half of the `str-scalar-at` op). Returns the codepoint
/// or `NO_SCALAR`=u32::MAX (out-of-range / ill-formed), so the compiler maps that sentinel to `None` when
/// building the `(Option Char)` sum. PENDING the compiler side: `String.scalar-at` on a RUNTIME string
/// still declines at lower.rs `lower_str_scalar_at` ("constant strings only"; the constant case folds to a
/// `Leaf::Char`) until a `Core::StrScalarAt` variant + backend emit (i32 codepoint → Char box, sentinel →
/// None) is wired — that is a compiler-variant addition (v-compiler-primitives/v-rust-backend), tracked to
/// flip corpus 13-strings:3218. The flatten + UTF-8 scalar walk is done and proven here. The SCALAR-indexed
/// String family (`scalar-len`/`scalar-at`/`slice`) all rest on this same UTF-8 walk.
///
/// COST: COST — O(scalar_index): reaching the i-th scalar walks the UTF-8 from the START (a String is not
/// scalar-indexable in O(1) — variable-width encoding). This is INHERENT to random access by scalar
/// index, NOT a defect. WARNING: CONSEQUENCE for the compiler agent: a LEXER that scans a string left-to-right
/// via repeated `scalar-at(s, 0)`, `scalar-at(s, 1)`, … is O(N²) (measured: ~67 ns/scalar at N=64 rising
/// to ~3300 ns/scalar at N=4096). A sequential scan wants a CURSOR (`scalar-next(buf, byte_off) ->
/// (codepoint, next_byte_off)`, advancing by the scalar's width) — that would be a SEPARATE coordinated
/// op (a different ABI: a pair return). `scalar-at` is the right primitive for RANDOM access; do NOT
/// build a left-to-right lexer on it. (The current compiler-in-Cadenza lexer sidesteps the whole area by
/// lexing `List Int64` char-codes — which is O(N) via `List` iteration, so the cursor gap is not yet
/// blocking; raise the cursor only when a real-String sequential scan is written.)
pub(crate) fn op_bytes_scalar_at(buf: Handle, scalar_index: u32) -> u32 {
    const NO_SCALAR: u32 = u32::MAX; // out-of-range / ill-formed sentinel (not a valid Unicode scalar)
    if is_immediate(buf) {
        return NO_SCALAR; // the empty/inline-unit Bytes has no scalars — any index is out of range
    }
    bytes_flatten(buf);
    with_node(buf, NO_SCALAR, |n| match core::str::from_utf8(&n.raw) {
        Ok(s) => s
            .chars()
            .nth(scalar_index as usize)
            .map(|c| c as u32)
            .unwrap_or(NO_SCALAR),
        Err(_) => NO_SCALAR, // ill-formed (defensive — a well-formed String always decodes)
    })
}

// NOTE: a prepared-but-unexported `op_bytes_eq_content` (a borrowing flatten-both + `champ_eq` content
// equality) lived here to unblock the `String.at`-content-equality miscompile. RETIRED `spec@<this>`:
// the compiler fixed that bug the OTHER way — COMPACT-AT-PRODUCER (compact the `bytes-slice` to a flat
// leaf in the `Core::StrAt` emit + compact rope operands before `value-eq`/CHAMP-key, backend/wasm/
// select.rs), which the existing consuming `bytes-compact` op already serves. So the borrowing content-eq
// had no remaining coordination path and was dead maintenance surface (unexported → DCE'd → hash-neutral
// either way); removed it + its test. The underlying primitives it composed — `bytes_flatten` +
// `champ_eq` — stay thoroughly covered by the collection fuzzers + the `compact_makes_a_*_canonical`
// contract tests. (`op_str_from_bytes` is now WIT-EXPORTED at index 85 — the string round-trip blocker is
// wired; `op_bytes_scalar_at` remains prepared-but-unexported: scalar-at the lexer's random-access read.)

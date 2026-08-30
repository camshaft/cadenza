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
    // never mutated in place — bytes-set on it is OOB (traps, 0 slots), concat builds a fresh rope node,
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
        Some(n) => match n.raw.as_mut_slice().get_mut(index as usize) {
            Some(slot) => *slot = value as u8,
            None => trap_oob(),
        },
    }
    buf
}
/// `bytes-get` — the logical byte at `index`. A leaf reads `raw` directly (O(1)); a rope node
/// (slice/concat) is FLATTENED to a leaf in place on this first full-read, then read (see
/// `bytes_flatten` — this is what keeps the compiler's `0..len` emit loop O(n) instead of O(n²) on a
/// deep concat chain). OOB into a valid buffer traps; null is benign.
pub(crate) fn op_bytes_get(buf: Handle, index: u32) -> u32 {
    if is_immediate(buf) {
        return 0; // cross-kind totality: a bytes buffer is never itself an immediate
    }
    // Leaf fast path (and null-benign): today's behavior, unchanged.
    let is_leaf = match unsafe { buf.node_ref() } {
        None => return 0,
        Some(n) => n.handles.is_empty(),
    };
    if is_leaf {
        return match unsafe { buf.node_ref() } {
            None => 0,
            Some(n) => match n.raw.get(index as usize) {
                Some(&b) => b as u32,
                None => trap_oob(),
            },
        };
    }
    // Rope node: bounds-check against the logical length (so a stray OOB doesn't force a flatten),
    // then materialize once. After flatten, `buf` is a leaf; re-read it.
    if index >= op_bytes_len(buf) {
        trap_oob();
    }
    bytes_flatten(buf);
    with_node(buf, 0, |n| {
        n.raw.get(index as usize).map(|&b| b as u32).unwrap_or(0)
    })
}

/// `bytes-len` — the logical byte length. O(1): a leaf's is its physical `raw` length; a rope node
/// stores it (concat as `raw=[len]`, slice as `raw=[off, len]`) so length never walks the tree.
/// Reached only on a value the compiler statically types as Bytes (tagless dispatch), so a `handles`
/// length of 1 here always means a Bytes slice, never a vector header of the same physical shape.
pub(crate) fn op_bytes_len(buf: Handle) -> u32 {
    if is_immediate(buf) {
        return 0; // cross-kind totality: a bytes buffer is never itself an immediate
    }
    with_node(buf, 0, |n| match n.handles.len() {
        0 => n.raw.len() as u32,     // leaf: physical bytes
        1 => read_u32_at(&n.raw, 4), // slice [parent]: raw = [off, len] → len at offset 4
        _ => read_u32_at(&n.raw, 0), // concat [left, right]: raw = [len]
    })
}

// ─── Bytes rope: O(1) concat/slice over shared leaves, flatten-on-read ────────────────────
// A Bytes value is a rope of shared slices/concats bottoming out in leaves, so `bytes-concat` and
// `bytes-slice` are O(1) and copy no bytes until observed — killing the O(n²) copy cascade a compiler
// would otherwise hit assembling a module by concatenating sections (deferred materialization behind
// the observable bytes, value-heap-runtime.md §Deferred Materialization Is Permitted Behind The
// Observable Bytes). Same tagless trick as the persistent vector:
// no new `Node` field, children in `handles`, so the existing iterative `op_drop` reclaims a rope
// and a shared leaf survives until its last owner drops — zero new RC machinery. Ownership follows
// the `arr-set` convention: concat/slice/compact CONSUME their Bytes operands (stored in `handles`
// without dup), so the free cascade Just Works.

/// Materialize a rope node (slice/concat) into a leaf IN PLACE: fill `raw` with its logical bytes,
/// release the children it owned, clear `handles`. Content-preserving, so UNOBSERVABLE and safe even
/// when the node is shared (`rc > 1`) — every sharer sees identical bytes before and after (the
/// memory model's #Sharing Is Not Observable deferral clause). A leaf is left untouched (idempotent).
/// The fill is ITERATIVE (an explicit `(node, dst_off, src_start, count)` worklist) so a deep rope
/// cannot overflow the wasm call stack — same discipline as the free cascade.
///
/// A slice/concat rope IS a value combined or narrowed from existing Bytes whose materialization is
/// DEFERRED to this flatten: the deferral is unobservable (every reader sees identical logical bytes) and
/// a deterministic function of the source rope, so combining/narrowing need not eagerly copy.
//= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
//# A value the compiler derives by combining or narrowing existing values MAY defer the work of materializing its contents until an operation observes them, provided the deferral is not observable and is a deterministic function of the source, so that combining and narrowing values need not eagerly copy their contents.
pub(crate) fn bytes_flatten(h: Handle) {
    let arity = with_node(h, 0usize, |n| n.handles.len());
    if arity == 0 {
        return; // already a leaf
    }
    let len = op_bytes_len(h) as usize;
    // SMALL fast path (the dominant per-char case: `String.at` → a 1-byte slice → compact): the flattened
    // bytes fit the inline `Raw` cap, so materialize them into a STACK buffer and install an inline `Raw`
    // — skipping BOTH transient heap Vecs the general path allocates (the `dst` output Vec AND the `work`
    // seed Vec), which for a ≤12-byte result are pure malloc/free churn (the Vec is copied into inline
    // storage then freed — the transient-small-Vec smell). Was 2 allocs/flatten; a small flatten now
    // allocates NOTHING. BYTE-IDENTICAL: `Raw::from` inlines a ≤cap Vec anyway, so the leaf is the same.
    if len <= INLINE_RAW_CAP {
        let mut buf = [0u8; INLINE_RAW_CAP];
        fill_rope_bytes(h, &mut buf[..len], len);
        let children = match unsafe { h.node_mut() } {
            Some(n) => {
                n.raw = Raw::Inline {
                    len: len as u8,
                    buf,
                };
                n.handles.take()
            }
            None => return,
        };
        for &c in children.iter() {
            op_drop(c);
        }
        return;
    }
    let mut dst = vec![0u8; len];
    fill_rope_bytes(h, &mut dst, len);
    // Convert `h` to a leaf: install the bytes and take its children out (so `h` no longer references
    // them), THEN release those references. Order matters — `h` is a leaf before the drops, so a
    // child freed here can never be reached through `h`.
    let children = match unsafe { h.node_mut() } {
        Some(n) => {
            n.raw = Raw::from(dst); // the flattened bytes (a wide rope leaf → Heap)
            n.handles.take() // the (now-orphaned) rope children (an owned `Handles`) to drop below
        }
        None => return,
    };
    for &c in children.iter() {
        op_drop(c);
    }
}

/// Copy the `len` logical bytes of rope `h` into `dst` (`dst.len() == len`). The iterative walk (an
/// explicit `(node, dst_off, src_start, count)` worklist) means a deep rope cannot overflow the wasm
/// call stack. READ-ONLY on every node (the caller mutates `h`'s node AFTER this returns), so briefly
/// holding a shared ref per node is sound even across a shared subgraph. Shared by both `bytes_flatten`
/// size paths so the copy logic lives once. The worklist is REUSED from `FLATTEN_SCRATCH` (thread-local:
/// clear + refill), so after the first flatten it never allocates again — the walk is allocation-FREE
/// steady-state, and combined with the caller's stack-buffer output for a ≤cap result, a small flatten
/// (the hot per-char `String.at`-compact) allocates NOTHING.
pub(crate) fn fill_rope_bytes(h: Handle, dst: &mut [u8], len: usize) {
    FLATTEN_SCRATCH.with(|cell| {
        let work = &mut *cell.borrow_mut();
        work.clear();
        work.push((h, 0, 0, len));
        while let Some((node, dst_off, src_start, count)) = work.pop() {
            if count == 0 {
                continue;
            }
            let n = match unsafe { node.node_ref() } {
                Some(n) => n,
                None => continue, // null child — benign
            };
            match n.handles.len() {
                0 => {
                    // leaf: copy the sub-range, trapping on any inconsistency (a compiler-bug rope).
                    match (
                        n.raw.get(src_start..src_start + count),
                        dst.get_mut(dst_off..dst_off + count),
                    ) {
                        (Some(src), Some(d)) => d.copy_from_slice(src),
                        _ => trap_oob(),
                    }
                }
                1 => {
                    // slice [parent], raw = [off, len]: logical byte j is parent's byte (off + j).
                    let parent = n.handles[0];
                    let off = read_u32_at(&n.raw, 0) as usize;
                    work.push((parent, dst_off, off + src_start, count));
                }
                _ => {
                    // concat [left, right], raw = [len]: [0, ll) from left, [ll, len) from right.
                    let left = n.handles[0];
                    let right = n.handles[1];
                    let ll = op_bytes_len(left) as usize;
                    if src_start < ll {
                        let lcount = count.min(ll - src_start);
                        work.push((left, dst_off, src_start, lcount));
                        let rcount = count - lcount;
                        if rcount > 0 {
                            work.push((right, dst_off + lcount, 0, rcount));
                        }
                    } else {
                        work.push((right, dst_off, src_start - ll, count));
                    }
                }
            }
        }
    });
}

/// `bytes-concat(a, b)` — a new Bytes = the bytes of `a` then `b`. O(1): allocates one concat node,
/// copies nothing. CONSUMES `a` and `b`. Empty operand is the identity (returns the other, dropping
/// the empty one to honor consume-semantics), matching the corpus identity law.
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
    let total = match la.checked_add(lb) {
        Some(t) => t,
        None => trap_oob(),
    };
    // Concat rope node: [left, right] handles + inline 4-byte `[len]` raw. Build the 2-element handles
    // INLINE (`inline_from`) rather than `vec![a, b]` — a concat is arity-2 = exactly INLINE_HANDLES_CAP,
    // so a heap Vec would be allocated then immediately re-inlined + freed by `From<Vec>` (the transient-
    // Vec smell). Direct inline construction = the node Box only, one fewer alloc per concat.
    alloc_raw(
        Handles::inline_from(&[a, b]),
        Raw::inline(&total.to_le_bytes()),
    )
}

/// `bytes-slice(buf, start, len)` — a new Bytes = `len` bytes of `buf` from `start`. O(1): one slice
/// node, no copy. Total-or-trap: `start + len > bytes-len(buf)` traps (checked in `u64`); `len == 0`
/// is the empty Bytes (never a trap, even at `start == len`). CONSUMES `buf`. A slice OF a slice is
/// collapsed into the grandparent (`slice(p, off1+start, len)`) to bound rope depth.
///
/// The slice SHARES the parent's storage — its representation holds the parent handle live (`op_dup`),
/// so the parent buffer is genuinely RETAINED (its rc reflects the slice), not hidden: the storage the
/// slice value retains is exactly the storage it holds live, and `op_drop` of the slice releases the
/// parent reference.
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
    // Collapse slice-of-slice: if `buf` is itself a slice into `parent` at `off1`, point the new
    // slice straight at `parent` (dup it — the new node owns a reference; dropping `buf` releases
    // buf's, net parent rc unchanged). Bounds slice-chain depth at 1.
    let collapse = with_node(buf, None, |n| {
        if n.handles.len() == 1 {
            Some((n.handles[0], read_u32_at(&n.raw, 0)))
        } else {
            None
        }
    });
    if let Some((parent, off1)) = collapse {
        op_dup(parent);
        op_drop(buf);
        // slice-of-slice: 1 handle + inline [off,len]. Build the handle INLINE (arity-1 ≤ cap) — a
        // `vec![parent]` would allocate a heap Vec then get re-inlined + freed by `From<Vec>`.
        return alloc_raw(
            Handles::inline_from(&[parent]),
            slice_raw(off1 + start, len),
        );
    }
    // slice node: 1 handle + inline [off,len]. Inline the single handle (no transient heap Vec).
    alloc_raw(Handles::inline_from(&[buf]), slice_raw(start, len))
}

/// The 8-byte `[off][len]` raw header of a bytes SLICE node, built INLINE (no transient heap Vec).
pub(crate) fn slice_raw(off: u32, len: u32) -> Raw {
    let mut buf = [0u8; INLINE_RAW_CAP];
    buf[0..4].copy_from_slice(&off.to_le_bytes());
    buf[4..8].copy_from_slice(&len.to_le_bytes());
    Raw::Inline { len: 8, buf }
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
    // string is never mutated in place — String.concat builds a fresh rope node, and bytes_flatten /
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
    // A runtime String IS a bytes rope: `String.concat`/`.at`-slice build concat/slice nodes (a String
    // shares the Bytes rope representation, `@b77b3ae0`), so `h` may be a rope whose `raw` holds the node's
    // HEADER bytes (a concat's `[len]`, a slice's `[off, len]`), NOT the content. MATERIALIZE it to a flat
    // leaf first (iterative `bytes_flatten`, so a deep rope can't overflow the stack; content-preserving,
    // so unobservable even on a shared value) — exactly as `op_bytes_get` and value-encode's `Shape::Str`
    // arm do. Without the flatten a rope String read back its raw handle/length bytes as UTF-8 (garbage).
    // A flat leaf is left untouched (flatten is a no-op there), so a plain `str-new` string is unaffected.
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
/// failure the caller drops it. FLATTEN first (`buf` may be a rope — a `Bytes.concat`/`.slice` tree —
/// whose `raw` holds header bytes, NOT content; strict `from_utf8` must see the actual bytes), exactly as
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
/// FLATTEN first (`buf` may be a `Bytes.concat`/`.slice` rope whose `raw` holds header bytes, not
/// content) — iterative, so a deep rope can't overflow; content-preserving, so UNOBSERVABLE on a shared
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

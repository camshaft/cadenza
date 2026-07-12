//! The Cadenza value-heap runtime (M2), compiled to a wasm **component** exporting the
//! `cadenza:runtime/heap` interface. The emitted program component imports this interface and the
//! host composes the two; the program threads OPAQUE `u32` handles between the constructors and
//! reads values back through the accessors, never dereferencing them itself.
//!
//! # Model — a genuinely tagless, per-node heap
//! A value is an immutable, reference-counted node in an acyclic heap. Each node is its OWN
//! allocation (`Box<Node>`) — there is no central value table; a handle is just the node's address.
//! A node stores exactly three things:
//!
//! ```text
//! Node { rc, handles: [Handle], raw: [u8] }
//! ```
//!
//! - `rc` — the Perceus refcount (RC needs it).
//! - `handles` — the child handles. Its LENGTH is the "scan count": how many words the free cascade
//!   must recurse into. This is genuine runtime layout data, NOT a type tag — a variable-length
//!   node (list/map) can only learn its own length by carrying it, because a runtime-driven `drop`
//!   reaches a nested node transitively with no caller in scope to hand the length in.
//! - `raw` — packed scalar bits / a sum discriminant / a byte buffer / UTF-8 string bytes.
//!
//! There is **no discriminant / kind field**. `Int`, `Bool`, `Float`, `Bytes`, and `Str` all share
//! the identical descriptor (`handles` empty); a 2-tuple, a 2-element list, and a 2-field record are
//! byte-identical (`handles` length 2). The runtime cannot tell those apart — and never needs to,
//! because Cadenza has no type erasure: the COMPILER holds the exact static type at every use site
//! and only ever emits `get-int` where the type says Int. `rc + handles.len() + raw.len()` is the
//! irreducible per-node state; storing type identity would be a tag, and we store none:
//!
//= spec/contracts/component-abi.md#the-runtime-does-not-name-or-render-values
//# The value-heap runtime MUST NOT hold a value's TYPE as a per-value tag, so that — because the language has no type erasure and the compiler therefore knows a value's static type at every use site — the runtime stores only structure and data (a product's elements, a sum's variant discriminant, a leaf's payload) and never a type identity a reader would dispatch on.
//!
//! Because the heap is acyclic (values immutable, recursion via code not heap back-edges) a
//! reference-count discipline is a COMPLETE reclamation strategy — no tracing/cycle collector:
//!
//= spec/capabilities/memory-and-resource-model.md#the-value-heap-is-acyclic
//# The heap of values a program forms at runtime MUST be acyclic, because a value's contents are fixed when it is created and no operation mutates an existing value to refer to a value created later.
//!
//! # Three decouplings (all load-bearing — keep them)
//! - **Tagless.** No per-object type tag (see above). The typed WIT functions (`box-int`,
//!   `arr-alloc`, …) are typed ENTRY POINTS, not stored metadata: `box-int` and `bytes-alloc`
//!   produce physically same-shaped nodes differing only in content.
//! - **Core is `Handle`-typed, not `u32`-typed.** Every operation — including RC and the persistent
//!   collections — is written and tested against the internal `Handle` (a node
//!   pointer). The `u32` public handle is a lossless narrowing that exists ONLY at the WIT `Guest`
//!   boundary (`Handle::to_u32`/`from_u32`, wasm32 only — pointers are 32-bit there). RC and
//!   CHAMP/RRB are thus developed and unit-tested natively without touching wasm or `u32`.
//! - **Core is allocator-agnostic.** The heap core only ever uses `Box`/`Vec`, i.e. the global
//!   allocator. The embedded wasm allocator (talc) lives behind `#[global_allocator]` in the
//!   isolated `allocator` module, so swapping allocators changes zero core code.
//!
//! # Totality and traps
//! The runtime is total in correct operation and never needs a fallible primitive: map key-lookup
//! and the total-or-trap `.at` bounds checks are compiler-emitted (lookup returns a compiler-built
//! `Option` sum; `.at` emits the sign-aware bounds check + spec-kinded trap). Two runtime behaviors:
//! - **Scalar reads and null-handle reads are TOTAL** — never trap. A `get-*` on a mismatched node
//!   reinterprets bytes (deterministic garbage, a compiler bug, but no trap); a null handle yields a
//!   benign default so a stray null never faults linear memory.
//! - **An out-of-bounds index into a VALID node TRAPS** (operator decision: fail-fast). This can
//!   only happen if the compiler violated an invariant it guarantees (static tuple/record arity;
//!   compiler-emitted list/bytes/string bounds checks; the renderer indexing only `0..len`). We trap
//!   rather than return a silent wrong value the differential gate might miss.
//!
//! # Name-free rendering
//! The runtime does NOT name or render values: a record is a positional product (field names are
//! compile-time indices) and a sum is a `(discriminant, payload)`. Rendering to canonical
//! s-expression text is type-directed code the COMPILER emits into the program, walking a value of
//! KNOWN shape through these accessors and baking every name/keyword as a constant
//! (component-abi.md §The Runtime Does Not Name Or Render Values). The `render` mirror in the tests
//! drives exactly that walk off a static `Shape`, proving the accessors suffice WITHOUT a tag.
//!
//! Authored in Rust for maintainability; to be re-authored in Cadenza at M8/M9 self-hosting.

// The runtime's embedded allocator (talc), isolated so it is swappable in one file. Only present in
// the wasm build; native `cargo test` uses the system allocator.
#[cfg(target_arch = "wasm32")]
mod allocator;

// The generated component bindings + the WIT `Guest` impl exist only in the wasm component build.
// The heap core below is plain Rust over `Handle`, so native `cargo test` exercises it directly.
#[cfg(target_arch = "wasm32")]
#[allow(warnings)]
mod bindings;

#[cfg(target_arch = "wasm32")]
use bindings::exports::cadenza::runtime::heap::Guest;

/// A heap node: a Perceus refcount header, the child handles, and the packed raw payload. Each node
/// is an independent allocation; a `Handle` is its address. There is NO kind/discriminant field —
/// the node's Cadenza type is compile-time knowledge the compiler holds, never stored here.
struct Node {
    /// Perceus refcount — NON-ATOMIC (a component instance is single-threaded, so we avoid the
    /// 1.5–2× atomic penalty). Born at 1 (the constructor's returned reference); `dup` increments,
    /// `drop` decrements, and at 0 the node is freed. The heap is acyclic (immutable values,
    /// recursion via code not heap back-edges) so this is a COMPLETE reclamation discipline.
    rc: u32,
    /// The child handles this node owns. Its length is the free cascade's scan count. Empty for
    /// scalars/bytes/strings; the elements for an array (tuple/record/list); `[payload]` for a sum;
    /// `[k0, v0, k1, v1, …]` for a map. This is the ONE positional shape backing every compound —
    /// the tuple-vs-list-vs-record distinction and a map's key ordering are compile-time / language
    /// knowledge the runtime does not hold.
    handles: Vec<Handle>,
    /// Packed raw payload: a scalar's little-endian bits, a sum's little-endian discriminant, a
    /// byte buffer, or a string's UTF-8 bytes. Empty for pure-compound nodes (array/map). Read back
    /// by reinterpretation — the compiler's static type says how to read it. Stored as `Raw`, which
    /// inlines the common ≤`INLINE_RAW_CAP`-byte payload (scalars, sum discs, CHAMP headers, vec
    /// headers — the overwhelming majority) with NO heap Vec, spilling to the heap only for longer
    /// bytes/strings. This is storage-transparent: `Raw` derefs to `&[u8]`, so the tagless byte-hash
    /// (`champ_hash`/`champ_eq`/`champ_key_cmp`) and every reader see the identical bytes regardless.
    raw: Raw,
}

/// The inline capacity of a `Raw`'s payload. Sized to `CHAMP_HEADER_SIZE` (12) — the largest raw a hot
/// node carries (a CHAMP node's `[datamap][nodemap][size]`); a scalar is ≤8, a sum disc 4, a vec
/// header 8, so all of those inline too. Bytes/strings longer than this spill to the heap.
const INLINE_RAW_CAP: usize = 12;

/// A node's raw payload: inline for the common ≤`INLINE_RAW_CAP`-byte case (no heap allocation),
/// heap-backed for longer bytes/strings. Reads go through `Deref<Target = [u8]>` so it is a drop-in
/// for `&[u8]` everywhere the old `Vec<u8>` was borrowed — the byte-hash, comparisons, and every
/// `read_*`/`champ_*` accessor are storage-transparent. Writes use the explicit methods below, which
/// mirror the `Vec` surface the runtime used (`clear`/`extend_from_slice`/`resize` + in-place patches
/// via `as_mut_slice`), transparently promoting inline→heap if a write would exceed the inline cap.
enum Raw {
    Inline { len: u8, buf: [u8; INLINE_RAW_CAP] },
    Heap(Vec<u8>),
}

impl Raw {
    /// Build an INLINE `Raw` directly from `bytes` (≤`INLINE_RAW_CAP`) — no heap Vec. For the small
    /// scalar/disc/header constructors that would otherwise allocate a transient `Vec` just for `alloc`
    /// to copy inline and drop. Caller guarantees `bytes.len() <= INLINE_RAW_CAP` (scalars are ≤8, a
    /// sum disc 4, a header 12); a longer slice would truncate, so it's only used where that holds.
    fn inline(bytes: &[u8]) -> Raw {
        let mut buf = [0u8; INLINE_RAW_CAP];
        buf[..bytes.len()].copy_from_slice(bytes);
        Raw::Inline { len: bytes.len() as u8, buf }
    }
    fn as_slice(&self) -> &[u8] {
        match self {
            Raw::Inline { len, buf } => &buf[..*len as usize],
            Raw::Heap(v) => v,
        }
    }
    /// Empty the payload. Mirrors `Vec::clear` — a HEAP buffer is emptied but KEEPS its capacity (so a
    /// clear-then-refill, e.g. the per-step cursor `raw` rebuild in `champ_become_cursor`, reuses the
    /// allocation instead of reallocating every time); an inline buffer just resets its length. (We do
    /// NOT re-inline a heap buffer on clear: a node that once spilled is refilled to a similar size, so
    /// keeping the heap capacity is the right call — re-inlining would reallocate on the next refill.)
    fn clear(&mut self) {
        match self {
            Raw::Inline { len, .. } => *len = 0,
            Raw::Heap(v) => v.clear(),
        }
    }
    /// Append `bytes`, promoting inline→heap if the total would exceed the inline cap. Mirrors
    /// `Vec::extend_from_slice`.
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        match self {
            Raw::Inline { len, buf } => {
                let cur = *len as usize;
                if cur + bytes.len() <= INLINE_RAW_CAP {
                    buf[cur..cur + bytes.len()].copy_from_slice(bytes);
                    *len = (cur + bytes.len()) as u8;
                } else {
                    // Spill: materialize the current inline bytes + the new ones into a heap Vec.
                    let mut v = Vec::with_capacity(cur + bytes.len());
                    v.extend_from_slice(&buf[..cur]);
                    v.extend_from_slice(bytes);
                    *self = Raw::Heap(v);
                }
            }
            Raw::Heap(v) => v.extend_from_slice(bytes),
        }
    }
    /// Resize to `new_len`, filling new bytes with `fill` (only ever grows a short/absent header to
    /// `CHAMP_HEADER_SIZE` in practice). Mirrors `Vec::resize`.
    fn resize(&mut self, new_len: usize, fill: u8) {
        if new_len <= INLINE_RAW_CAP {
            // Fits inline: rebuild an inline buffer of `new_len` from the current bytes (truncate or
            // pad with `fill`), releasing any heap spill.
            let cur = self.as_slice();
            let keep = cur.len().min(new_len);
            let mut buf = [fill; INLINE_RAW_CAP];
            buf[..keep].copy_from_slice(&cur[..keep]);
            *self = Raw::Inline { len: new_len as u8, buf };
        } else {
            let mut v = self.as_slice().to_vec();
            v.resize(new_len, fill);
            *self = Raw::Heap(v);
        }
    }
    /// A mutable slice of the payload, for the in-place `raw[a..b].copy_from_slice(...)` header patches.
    /// The length is unchanged (the patches only overwrite existing bytes), so no inline↔heap flip.
    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Raw::Inline { len, buf } => &mut buf[..*len as usize],
            Raw::Heap(v) => v,
        }
    }
    fn len(&self) -> usize {
        match self {
            Raw::Inline { len, .. } => *len as usize,
            Raw::Heap(v) => v.len(),
        }
    }
}

impl From<Vec<u8>> for Raw {
    /// Build a `Raw` from a freshly-constructed byte vector (the `alloc` boundary): inline it when it
    /// fits the cap (the common case — the Vec is then dropped, unallocated-away), else keep the heap
    /// buffer verbatim (no copy).
    fn from(v: Vec<u8>) -> Raw {
        if v.len() <= INLINE_RAW_CAP {
            let mut buf = [0u8; INLINE_RAW_CAP];
            buf[..v.len()].copy_from_slice(&v);
            Raw::Inline { len: v.len() as u8, buf }
        } else {
            Raw::Heap(v)
        }
    }
}

impl Clone for Raw {
    fn clone(&self) -> Raw {
        // `.raw.clone()` sites want an owned copy of the bytes — re-derive via `From` so a small heap
        // buffer clones back to inline (and a large one stays heap). Keeps clones inline when possible.
        Raw::from(self.as_slice().to_vec())
    }
}

impl core::ops::Deref for Raw {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// The internal handle: the address of a `Node`. The core is written entirely in terms of this, so
/// RC and persistent collections are testable natively. `NULL` is the benign-default sentinel a
/// total read yields for a missing node (a mismatch is a compiler bug; the runtime must never trap
/// on a null read).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Handle(*mut Node);

impl Handle {
    const NULL: Handle = Handle(std::ptr::null_mut());
}

// ─── Low-bit tagged immediates (inline handles) — DESIGN-inline-handle-tagging.md §4.1 ────
// A `Handle` whose low 2 bits are non-zero carries its VALUE inline (no heap Node). Real nodes
// come only from `alloc` (`Box::into_raw`), which is ≥4-aligned, so a real pointer and NULL both
// have low bits `00`. This single discriminant (`is_immediate`) is shared by BOTH the native and
// the wasm32 build (alignment ≥4 holds on a native pointer and a wasm32 `u32` alike), which is why
// the native suite exercises the shipped representation and `to_u32`/`from_u32` need no change.
//
// LIVE: `op_box_int` returns an immediate for an in-window fixnum, `op_box_bool` always inlines, and
// `op_arr_alloc(0)` returns the inline unit — so an `is_immediate` handle really flows through the ops,
// and every deref-site guard is exercised. The guards stay INERT only for cross-kind ops where an
// immediate can never legitimately appear (an immediate is never a bytes/map/sum value).
//
// Encoding (tag = low 2 bits of `Handle.0`):
//   00  pointer or NULL (unchanged)
//   01  fixnum: 30-bit signed int in bits[31:2]; value = (h.0 as i32) >> 2 (arith shift)
//   10  atom:   subkind in bits[3:2]; 00 = unit, 01 = bool (value in bit[4]); 10/11 reserved
//   11  reserved (future widening)

/// The single classifier: a handle is immediate iff its low 2 bits are non-zero. NULL is tag `00`,
/// so it is NOT immediate (it keeps its benign `with_node`/RC-noop defaults).
#[inline]
fn is_immediate(h: Handle) -> bool {
    (h.0 as usize) & 0b11 != 0
}

/// The inline `unit` (empty tuple): atom subkind `00`, tag `10` ⇒ bits `0b0010`.
#[inline]
#[allow(dead_code)]
fn imm_unit() -> Handle {
    Handle(0b0010usize as *mut Node)
}

/// An inline boolean: atom subkind `01`, tag `10`, value in bit[4] ⇒ false = `0b0110`, true = `0b10110`.
#[inline]
#[allow(dead_code)]
fn imm_bool(b: bool) -> Handle {
    Handle((0b0110usize | ((b as usize) << 4)) as *mut Node)
}

/// Fixnum window (canonical-form-critical): a value inlines iff it fits the 30-bit signed range.
const FIXNUM_MIN: i64 = -(1 << 29);
const FIXNUM_MAX: i64 = (1 << 29) - 1;

/// Whether `v` fits the inline fixnum window `[-2^29, 2^29 - 1]`. THE single source of truth for the
/// boundary (used in exactly one place, `op_box_int`, in Phase 2) so a value never sometimes-inlines.
#[inline]
fn fixnum_fits(v: i64) -> bool {
    (FIXNUM_MIN..=FIXNUM_MAX).contains(&v)
}

/// An inline fixnum. Caller MUST have checked `fixnum_fits(v)`; the `(v << 2)` shift would otherwise
/// drop high bits. Tag `01` in the low 2 bits, value sign-extended into bits[31:2].
#[inline]
fn imm_int(v: i64) -> Handle {
    Handle((((v as i32) << 2) | 0b01) as usize as *mut Node)
}

/// Decode an inline fixnum (only valid when `is_immediate(h)` and tag is `01`). Arithmetic shift
/// sign-extends the 30-bit payload back to a full `i64`.
#[inline]
fn imm_as_int(h: Handle) -> i64 {
    ((h.0 as usize as i32) >> 2) as i64
}

/// Decode an inline boolean (only valid when `is_immediate(h)` and it is a bool atom).
#[inline]
#[allow(dead_code)]
fn imm_as_bool(h: Handle) -> bool {
    (h.0 as usize >> 4) & 1 != 0
}

/// The kind of an immediate. Only valid when `is_immediate(h)`.
#[allow(dead_code)]
enum ImmKind {
    Unit,
    Bool,
    Int,
}

/// Classify an immediate. Only valid when `is_immediate(h)`.
#[inline]
#[allow(dead_code)]
fn imm_kind(h: Handle) -> ImmKind {
    match (h.0 as usize) & 0b11 {
        0b01 => ImmKind::Int,
        _ /* 0b10 atom */ => {
            if (h.0 as usize >> 2) & 0b11 == 0 {
                ImmKind::Unit
            } else {
                ImmKind::Bool
            }
        }
    }
}

/// The raw bytes a BOXED twin of this immediate would carry (the same bytes `op_box_int`/
/// `op_box_bool`/`op_arr_alloc(0)` write), so an inline value hashes / compares EQUAL to its boxed
/// twin (open-Q#8) and orders consistently. An immediate always has arity 0 (no child handles).
/// Only valid when `is_immediate(h)`.
#[allow(dead_code)]
fn imm_canonical_raw(h: Handle) -> Vec<u8> {
    match imm_kind(h) {
        ImmKind::Unit => Vec::new(),
        ImmKind::Bool => vec![imm_as_bool(h) as u8],
        ImmKind::Int => (imm_as_int(h) as u64).to_le_bytes().to_vec(),
    }
}

/// `(canonical raw bytes, child arity)` for either an immediate (canonical raw, arity 0) or a real
/// node (its `raw`, `handles.len()`); NULL folds as `(empty, 0)`. Used by the CHAMP guards to compare
/// an inline value structurally with its boxed twin.
#[allow(dead_code)]
fn node_raw_arity(h: Handle) -> (Vec<u8>, usize) {
    if is_immediate(h) {
        (imm_canonical_raw(h), 0)
    } else {
        with_node(h, (Vec::new(), 0usize), |n| (n.raw.to_vec(), n.handles.len()))
    }
}

/// Call `f` with `h`'s canonical raw bytes (BORROWED) and child arity, allocating NOTHING — the
/// alloc-free twin of `node_raw_arity` for the hot CHAMP eq/cmp immediate branches. A real node lends
/// its `raw` slice directly (no clone); an immediate materializes its ≤8 canonical little-endian
/// bytes — the SAME bytes a boxed twin carries (open-Q#8), so eq/cmp treat inline and boxed alike —
/// into a stack buffer and lends a subslice; NULL folds as `(&[], 0)`. Extracted so the two callers
/// stop cloning a `Vec` per comparison to inspect ≤8 bytes (mirrors the `champ_hash` scalar fast path).
#[inline]
fn with_raw_arity<T>(h: Handle, f: impl FnOnce(&[u8], usize) -> T) -> T {
    if is_immediate(h) {
        let mut buf = [0u8; 8];
        let len = match imm_kind(h) {
            ImmKind::Unit => 0usize,
            ImmKind::Bool => {
                buf[0] = imm_as_bool(h) as u8;
                1
            }
            ImmKind::Int => {
                buf = (imm_as_int(h) as u64).to_le_bytes();
                8
            }
        };
        f(&buf[..len], 0)
    } else {
        match unsafe { h.0.as_ref() } {
            Some(node) => f(&node.raw, node.handles.len()),
            None => f(&[], 0), // NULL folds as (empty, 0)
        }
    }
}

// Count of nodes currently allocated and not yet freed. Compiled under the native test suite AND the
// `debug-counters` wasm feature: the native suite asserts exact reclamation and BOUNDED PEAK HEAP
// across iterations (the leak / peak-heap acceptance probe), and a wasm leak-check harness reads it via the
// `live-objects` export after a run to prove the compiler's Perceus dup/drop discipline balances
// (assert 0). In the DEFAULT (shipped) build neither the counter nor its updates exist — the runtime
// is zero-cost and byte-stable, and `live-objects` returns 0.
#[cfg(any(test, feature = "debug-counters"))]
thread_local! {
    static LIVE_NODES: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

/// The live heap-object count, or 0 when the counter is not compiled in (the default build). The
/// `live-objects` export returns this; a leak-check harness asserts it is 0 after a run to verify the
/// compiler's dup/drop discipline leaves nothing behind.
#[allow(dead_code)]
fn live_object_count() -> u32 {
    #[cfg(any(test, feature = "debug-counters"))]
    {
        LIVE_NODES.with(|n| n.get()).max(0) as u32
    }
    #[cfg(not(any(test, feature = "debug-counters")))]
    {
        0
    }
}

/// Allocate a node (refcount 1) from `handles` + `raw` and return its handle. Uses the global
/// allocator (talc on wasm) — the core never names it, so a size-classed free-list could be swapped
/// in here alone.
fn alloc(handles: Vec<Handle>, raw: Vec<u8>) -> Handle {
    // Convert the byte vector to a `Raw` (inline ≤INLINE_RAW_CAP, the common case) then delegate. Note:
    // a caller that ALREADY holds the bytes as a `Vec` (byte buffers, string leaves) still allocated
    // that Vec; the alloc-saving win is for callers that build a small header directly as inline `Raw`
    // via `alloc_raw` (see `champ_header`), never materializing a transient Vec.
    alloc_raw(handles, Raw::from(raw))
}

/// `alloc` but taking a ready `Raw` — lets a caller that builds a small header INLINE (e.g.
/// `champ_header`) skip the transient `Vec` allocation entirely.
fn alloc_raw(handles: Vec<Handle>, raw: Raw) -> Handle {
    #[cfg(any(test, feature = "debug-counters"))]
    LIVE_NODES.with(|n| n.set(n.get() + 1));
    Handle(Box::into_raw(Box::new(Node { rc: 1, handles, raw })))
}

/// Borrow a node to read from it TOTALLY; a null handle yields `default`. Centralizes the one unsafe
/// deref and its null check for the reads that are total by construction (scalars, lengths, sum
/// disc/payload, string). The index accessors do NOT use this — they distinguish a benign null from
/// an out-of-bounds index into a valid node (which traps), so they inline their own check.
fn with_node<T>(h: Handle, default: T, f: impl FnOnce(&Node) -> T) -> T {
    match unsafe { h.0.as_ref() } {
        Some(node) => f(node),
        None => default,
    }
}

/// Fail-fast on an out-of-bounds index into a VALID node. In correct operation this never fires:
/// tuple/record access is by static index, list/bytes/string `.at` get their bounds check emitted by
/// the compiler, and the renderer only ever indexes `0..len`. Reaching it means a compiler-invariant
/// violation — we trap (a panic, which `panic = "abort"` lowers to a wasm trap) rather than return a
/// silent wrong value the differential gate might miss. (Operator decision: trap on OOB.)
#[cold]
#[inline(never)]
fn trap_oob() -> ! {
    panic!("cdz-runtime: index out of bounds into a valid node")
}

/// Read up to 8 little-endian bytes of `raw` as a `u64` (zero-padded if shorter). Keeps scalar reads
/// TOTAL even on a mismatched/short node — a compiler bug yields deterministic bits, never a trap.
fn read_word(raw: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    let n = raw.len().min(8);
    buf[..n].copy_from_slice(&raw[..n]);
    u64::from_le_bytes(buf)
}

/// Read up to 4 little-endian bytes of `raw` as a `u32` (zero-padded if shorter) — the sum
/// discriminant, total on a short/mismatched node.
fn read_disc(raw: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    let n = raw.len().min(4);
    buf[..n].copy_from_slice(&raw[..n]);
    u32::from_le_bytes(buf)
}

// ─── Scalar leaves: box a primitive, read it back (TOTAL — reinterprets bytes, never traps) ──────

fn op_box_int(v: i64) -> Handle {
    // Normalize-on-construct (P2), THE single source of truth for the fixnum boundary: a value that
    // fits the inline window is ALWAYS an immediate, never boxed, so inline-3 and boxed-3 cannot
    // coexist (canonical form). Only out-of-window ints keep a heap Node.
    if fixnum_fits(v) {
        return imm_int(v);
    }
    alloc_raw(Vec::new(), Raw::inline(&(v as u64).to_le_bytes())) // 8-byte scalar: inline, no heap raw
}
fn op_get_int(h: Handle) -> i64 {
    if is_immediate(h) {
        return imm_as_int(h); // rep-agnostic decode; equals a boxed twin's `read_word`
    }
    with_node(h, 0, |n| read_word(&n.raw) as i64)
}
fn op_box_bool(v: bool) -> Handle {
    // Normalize-on-construct (P1b): a bool ALWAYS inlines, never boxes, so inline is the one
    // canonical representation. `imm_bool` carries the value in the tag bits — no heap Node.
    imm_bool(v)
}
fn op_get_bool(h: Handle) -> bool {
    if is_immediate(h) {
        return imm_as_bool(h); // rep-agnostic decode of an inline bool
    }
    with_node(h, false, |n| n.raw.first().is_some_and(|&b| b != 0))
}
fn op_box_float(v: f64) -> Handle {
    alloc_raw(Vec::new(), Raw::inline(&v.to_bits().to_le_bytes())) // 8-byte scalar: inline, no heap raw
}
fn op_get_float(h: Handle) -> f64 {
    if is_immediate(h) {
        return 0.0; // cross-kind totality: a float is never itself an immediate
    }
    with_node(h, 0.0, |n| f64::from_bits(read_word(&n.raw)))
}

// ─── Positional array — the ONE runtime shape for TUPLE, RECORD, and LIST ───────────────
// Elements live in `handles`. Access by an out-of-bounds index into a valid array TRAPS; a null
// handle is benign (returns NULL / no-op).

fn op_arr_alloc(len: u32) -> Handle {
    // Normalize-on-construct (P1b): the empty array (arity 0) IS the unit value, and unit ALWAYS
    // inlines — no heap Node. A non-empty array still allocates its slots.
    if len == 0 {
        return imm_unit();
    }
    alloc(vec![Handle::NULL; len as usize], Vec::new())
}
/// Write an element handle and return the array handle (for convenient threading). OOB into a valid
/// array traps; null is a no-op.
fn op_arr_set(arr: Handle, index: u32, elem: Handle) -> Handle {
    if is_immediate(arr) {
        return arr; // an immediate array (inline unit) has no slots; elem is stored, not deref'd
    }
    match unsafe { arr.0.as_mut() } {
        None => {}
        Some(n) => match n.handles.get_mut(index as usize) {
            Some(slot) => *slot = elem,
            None => trap_oob(),
        },
    }
    arr
}
fn op_arr_get(arr: Handle, index: u32) -> Handle {
    if is_immediate(arr) {
        trap_oob(); // an immediate array (inline unit) has 0 slots — any index is OOB
    }
    match unsafe { arr.0.as_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get(index as usize) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
fn op_arr_len(arr: Handle) -> u32 {
    if is_immediate(arr) {
        return 0; // inline unit has 0 elements
    }
    with_node(arr, 0, |n| n.handles.len() as u32)
}

// ─── Sum: a discriminant (in `raw`) plus a payload handle (in `handles`) ────────────────
// `sum-payload` is TOTAL (no runtime index): a mismatched node with no handle yields NULL.

fn op_sum_new(disc: u32, payload: Handle) -> Handle {
    // Build the 4-byte disc INLINE (no transient heap Vec) — a sum node is then the node Box + its
    // 1-element handles Vec, 2 allocs instead of 3.
    alloc_raw(vec![payload], Raw::inline(&disc.to_le_bytes()))
}
fn op_sum_disc(h: Handle) -> u32 {
    if is_immediate(h) {
        return 0; // cross-kind totality: a sum is never itself an immediate
    }
    with_node(h, 0, |n| read_disc(&n.raw))
}
fn op_sum_payload(h: Handle) -> Handle {
    if is_immediate(h) {
        return Handle::NULL; // cross-kind totality: a sum is never itself an immediate
    }
    with_node(h, Handle::NULL, |n| {
        n.handles.first().copied().unwrap_or(Handle::NULL)
    })
}

// ─── Bytes: a packed immutable byte buffer (in `raw`) ───────────────────────────────────
// OOB into a valid buffer traps; null is benign.

fn op_bytes_alloc(len: u32) -> Handle {
    alloc(Vec::new(), vec![0u8; len as usize])
}
/// Store a byte (the compiler guarantees `value` is 0–255) and return the buffer handle. OOB into a
/// valid buffer traps; null is a no-op.
fn op_bytes_set(buf: Handle, index: u32, value: u32) -> Handle {
    if is_immediate(buf) {
        return buf; // defensive (mirrors op_bytes_get/len): a bytes buffer is never an immediate;
                    // return the handle unchanged (no-op write), never deref the tagged bits
    }
    match unsafe { buf.0.as_mut() } {
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
fn op_bytes_get(buf: Handle, index: u32) -> u32 {
    if is_immediate(buf) {
        return 0; // cross-kind totality: a bytes buffer is never itself an immediate
    }
    // Leaf fast path (and null-benign): today's behavior, unchanged.
    let is_leaf = match unsafe { buf.0.as_ref() } {
        None => return 0,
        Some(n) => n.handles.is_empty(),
    };
    if is_leaf {
        return match unsafe { buf.0.as_ref() } {
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
fn op_bytes_len(buf: Handle) -> u32 {
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
fn bytes_flatten(h: Handle) {
    let arity = with_node(h, 0usize, |n| n.handles.len());
    if arity == 0 {
        return; // already a leaf
    }
    let len = op_bytes_len(h) as usize;
    let mut dst = vec![0u8; len];
    // Copy `count` logical bytes of `node` beginning at logical `src_start` into `dst[dst_off..]`.
    // The walk is READ-ONLY on every node (only `h` is mutated, after the loop), so briefly holding
    // a shared ref per node is sound even across the shared subgraph.
    let mut work: Vec<(Handle, usize, usize, usize)> = vec![(h, 0, 0, len)];
    while let Some((node, dst_off, src_start, count)) = work.pop() {
        if count == 0 {
            continue;
        }
        let n = match unsafe { node.0.as_ref() } {
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
    // Convert `h` to a leaf: install the bytes and take its children out (so `h` no longer references
    // them), THEN release those references. Order matters — `h` is a leaf before the drops, so a
    // child freed here can never be reached through `h`.
    let children = match unsafe { h.0.as_mut() } {
        Some(n) => {
            n.raw = Raw::from(dst); // the flattened bytes (a rope leaf; usually > inline cap → Heap)
            std::mem::take(&mut n.handles)
        }
        None => return,
    };
    for c in children {
        op_drop(c);
    }
}

/// `bytes-concat(a, b)` — a new Bytes = the bytes of `a` then `b`. O(1): allocates one concat node,
/// copies nothing. CONSUMES `a` and `b`. Empty operand is the identity (returns the other, dropping
/// the empty one to honor consume-semantics), matching the corpus identity law.
fn op_bytes_concat(a: Handle, b: Handle) -> Handle {
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
    alloc_raw(vec![a, b], Raw::inline(&total.to_le_bytes())) // concat rope node: 4-byte len, inline
}

/// `bytes-slice(buf, start, len)` — a new Bytes = `len` bytes of `buf` from `start`. O(1): one slice
/// node, no copy. Total-or-trap: `start + len > bytes-len(buf)` traps (checked in `u64`); `len == 0`
/// is the empty Bytes (never a trap, even at `start == len`). CONSUMES `buf`. A slice OF a slice is
/// collapsed into the grandparent (`slice(p, off1+start, len)`) to bound rope depth.
fn op_bytes_slice(buf: Handle, start: u32, len: u32) -> Handle {
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
        return alloc_raw(vec![parent], slice_raw(off1 + start, len)); // slice-of-slice: inline [off,len]
    }
    alloc_raw(vec![buf], slice_raw(start, len)) // slice node: inline [off,len], no transient Vec
}

/// The 8-byte `[off][len]` raw header of a bytes SLICE node, built INLINE (no transient heap Vec).
fn slice_raw(off: u32, len: u32) -> Raw {
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
fn op_bytes_compact(buf: Handle) -> Handle {
    bytes_flatten(buf);
    buf
}

// ─── String: a stored UTF-8 leaf (bytes in `raw`) ───────────────────────────────────────

fn op_str_new(s: String) -> Handle {
    alloc(Vec::new(), s.into_bytes())
}
fn op_str_get(h: Handle) -> String {
    if is_immediate(h) {
        return String::new(); // cross-kind totality: a string is never itself an immediate
    }
    with_node(h, String::new(), |n| {
        String::from_utf8_lossy(&n.raw).into_owned()
    })
}

// ─── Map: dynamic-key collection of (key, value) handle pairs, stored verbatim ──────────
// Pairs are flattened into `handles` as [k0, v0, k1, v1, …]; pair count = handles.len() / 2. OOB
// pair index into a valid map traps; null is benign.

fn op_map_alloc(len: u32) -> Handle {
    alloc(vec![Handle::NULL; (len as usize) * 2], Vec::new())
}
/// Write the (key, value) pair at `index` and return the map handle (for convenient threading). OOB
/// pair index into a valid map traps; null is a no-op.
fn op_map_set(m: Handle, index: u32, key: Handle, value: Handle) -> Handle {
    if is_immediate(m) {
        return m; // defensive (mirrors the map readers): a map is never an immediate; return the
                  // handle unchanged (no-op write), never deref the tagged bits
    }
    match unsafe { m.0.as_mut() } {
        None => {}
        Some(n) => {
            let base = (index as usize) * 2;
            if base + 1 < n.handles.len() {
                n.handles[base] = key;
                n.handles[base + 1] = value;
            } else {
                trap_oob();
            }
        }
    }
    m
}
fn op_map_key(m: Handle, index: u32) -> Handle {
    if is_immediate(m) {
        return Handle::NULL; // defensive: a map is never an immediate; benign default like null-in
    }
    match unsafe { m.0.as_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get((index as usize) * 2) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
fn op_map_val(m: Handle, index: u32) -> Handle {
    if is_immediate(m) {
        return Handle::NULL; // defensive: a map is never an immediate; benign default like null-in
    }
    match unsafe { m.0.as_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get((index as usize) * 2 + 1) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
fn op_map_len(m: Handle) -> u32 {
    with_node(m, 0, |n| (n.handles.len() / 2) as u32)
}

// ─── Reference-count calling convention (Perceus) ───────────────────────────────────────
// Written as `Handle`-typed core ops so the whole RC discipline is developed and tested natively,
// against real node pointers, before the `u32` wasm boundary ever sees it. The compiler emits `drop`
// where a heap value is released (a dead heap binding, and the resource destructor), so a compound's
// storage is reclaimed; it does NOT yet emit `dup` (the current escape/return paths transfer ownership
// rather than share), so `dup`'s call sites arrive when a construct first shares a handle.

/// `dup` — a new reference to `h` is being retained: increment its refcount. Null is a no-op.
fn op_dup(h: Handle) {
    if is_immediate(h) {
        return; // an immediate owns no heap — nothing to retain
    }
    if let Some(node) = unsafe { h.0.as_mut() } {
        node.rc += 1;
    }
}

/// The refcount of `h` (0 for null). The FBIP fast paths read this to decide, PER NODE, whether the
/// node is uniquely owned (`rc == 1`, safe to mutate in place) or shared (`rc > 1`, must path-copy) —
/// the aliasing-safety rule: a shared node backs another persistent version and must stay byte-identical.
fn node_rc(h: Handle) -> u32 {
    if is_immediate(h) {
        // An immediate is not a Node. Return a non-1 sentinel (2) so every FBIP `rc == 1` in-place
        // path takes the conservative copy and NEVER tries to mutate the tagged bits as a Node.
        return 2;
    }
    with_node(h, 0, |n| n.rc)
}

/// `drop` — a reference to `h` is being released: decrement its refcount, and at zero free the node
/// and release the references it owned (which may cascade). The compiler emits the `drop` call at a
/// source-determined point (the value's last use), so reclamation is deterministic, not a background
/// collector's choice:
///
//= spec/capabilities/memory-and-resource-model.md#cleanup-is-source-determined
//# The point at which a value's storage is released MUST be a deterministic function of the source.
///
/// Fast paths, cheapest first: a **shared** node (`rc > 1`) is a bare decrement — no scan, no
/// reclamation. A **leaf** (empty `handles`) costs no worklist allocation: `mem::take` of an empty
/// `Vec` does not allocate, so the loop below simply doesn't run. Only a unique COMPOUND seeds a
/// worklist — and it reuses the freed node's OWN `handles` vector as that seed (it is already
/// allocated and the node is about to die), so no fresh allocation for the root level.
///
/// The cascade is ITERATIVE, over an explicit worklist — NOT recursive. A recursive free would grow
/// the wasm call stack proportionally to structure DEPTH and could overflow it on a deep unique
/// list/tree (the same host-stack limit that bounds deep recursion elsewhere). The worklist bounds
/// stack use to O(1) frames; total work is still O(n) in the freed subtree. `LIVE_NODES` (tests)
/// lets us assert the whole subtree is reclaimed and peak heap stays bounded across iterations.
fn op_drop(root: Handle) {
    if is_immediate(root) {
        return; // an immediate owns no heap — nothing to release
    }
    let node = match unsafe { root.0.as_mut() } {
        Some(n) => n,
        None => return, // null — benign
    };
    if node.rc > 1 {
        node.rc -= 1; // shared: cheapest path, no reclamation
        return;
    }
    // rc == 1: last reference. Reclaim the node and cascade into its children. Reuse the node's own
    // `handles` vector as the worklist seed (leaf → empty, no allocation).
    let mut worklist = std::mem::take(&mut node.handles);
    unsafe { drop(Box::from_raw(root.0)) };
    #[cfg(any(test, feature = "debug-counters"))]
    LIVE_NODES.with(|n| n.set(n.get() - 1));

    while let Some(cur) = worklist.pop() {
        if is_immediate(cur) {
            continue; // an inline child owns no heap — the hottest RC path (doc-named)
        }
        let n = match unsafe { cur.0.as_mut() } {
            Some(n) => n,
            None => continue, // null child slot — benign
        };
        if n.rc > 1 {
            n.rc -= 1; // shared child survives; freed only when its last owner drops it
            continue;
        }
        // Move this node's children onto the worklist (draining its vector), then free it.
        worklist.append(&mut n.handles);
        unsafe { drop(Box::from_raw(cur.0)) };
        #[cfg(any(test, feature = "debug-counters"))]
        LIVE_NODES.with(|n| n.set(n.get() - 1));
    }
}

// ─── Reuse / FBIP (Perceus reset + reuse-aware constructors) ──────────────────────────────
// The in-place-update win: when a unique value is consumed and a value is rebuilt in the same
// breath (List.map, a functional record/cons rebuild), reuse the dying node's shell for the new
// one instead of free→malloc. Frame-limited by construction (research P3/P4): reuse fires ONLY on a
// UNIQUE node (`rc == 1`), so a reused cell is memory that was already live and about to die — peak
// heap cannot grow, and because no other reference observes the difference the reuse is invisible:
//
//= spec/capabilities/memory-and-resource-model.md#reuse-is-not-observable
//# When the compiler reuses a value's storage in place because no other reference to that value can observe the difference, that reuse MUST NOT change the program's observable behavior, so that reusing storage is a transparent optimization rather than a mutation of a value.
//
// The three ops form a two-step protocol the compiler emits:
//
//   token = reset(old);                        // old unique → emptied shell as a token; else null
//   new   = arr-alloc-reuse(len, token);       // token non-null → refit that shell; else fresh
//   …or   = sum-new-reuse(disc, payload, token);
//
// A reuse TOKEN is just a childless `rc == 1` node. It obeys the ordinary ownership ABI: it is
// CONSUMED by exactly one reuse-constructor, OR — if a control path doesn't rebuild — `drop`ped
// (which, on a childless unique node, frees exactly the shell). No separate "free token" op needed.
//
// Ordering the compiler must honor (the §4 dup-before-drop invariant, applied to reset): any child
// of `old` reused in the rebuild (e.g. recursing into a tree's subtrees, or reading a field into
// the new value) must be `dup`'d BEFORE `reset(old)`, because reset drops `old`'s references to its
// children. This is exactly the calling convention's existing rule; reset is a drop-for-its-shell.

/// `reset` — drop `node` for reuse. If UNIQUE (`rc == 1`): release the children it owns (a normal
/// cascading `drop` of each child reference — shared grandchildren survive), then RETAIN the emptied
/// shell (rc still 1, no children, no raw) and return it as a non-null reuse token. If SHARED
/// (`rc > 1`): decrement and return `NULL` — the other owners keep the node intact, so there is
/// nothing to reuse. Null in → null out. The returned token feeds a `*-reuse` constructor, or is
/// `drop`ped if unused (freeing the bare shell). Reuses the node's own handle/raw Vec backings, so a
/// same-arity refit performs no reallocation at all.
fn op_reset(node: Handle) -> Handle {
    if is_immediate(node) {
        return Handle::NULL; // an immediate owns no shell to reuse (covers the borrows below)
    }
    // Read rc through a short-lived borrow that ends before we recurse into children.
    let rc = match unsafe { node.0.as_ref() } {
        Some(n) => n.rc,
        None => return Handle::NULL, // null: nothing to reuse
    };
    if rc > 1 {
        if let Some(n) = unsafe { node.0.as_mut() } {
            n.rc -= 1; // shared: another owner keeps it intact; no reuse token
        }
        return Handle::NULL;
    }
    // Unique. Take the children out (ending the borrow before the drops), release each, then put
    // the now-empty backing Vec back so the shell keeps its allocation for the coming refit.
    let mut children = match unsafe { node.0.as_mut() } {
        Some(n) => std::mem::take(&mut n.handles),
        None => return Handle::NULL,
    };
    for &child in children.iter() {
        op_drop(child); // cascades; a child dup'd by the compiler before reset survives
    }
    children.clear(); // 0 elements, capacity retained
    if let Some(n) = unsafe { node.0.as_mut() } {
        n.handles = children; // restore the (empty, capacity-bearing) backing
        n.raw.clear();
    }
    node // the retained shell, rc == 1, empty — a reuse token
}

/// `arr-alloc-reuse` — `arr-alloc(len)`, but reusing `token`'s shell when it is a non-null reuse
/// token from `reset`: refit it to `len` NULL slots (reusing its handle-Vec backing when capacity
/// allows — the common same-length case reallocates nothing) and return it, allocating NO new node.
/// A null token allocates fresh, so a `reset` that declined to yield a token is transparent.
fn op_arr_alloc_reuse(len: u32, token: Handle) -> Handle {
    if is_immediate(token) {
        return op_arr_alloc(len); // defensive: reset never yields an immediate token
    }
    // Normalize (P2 canonical form): an empty array IS unit, and unit ALWAYS inlines — no boxed
    // twin may exist. A `len == 0` refit would otherwise return a BOXED empty node, forking the rep.
    // The token came from `op_reset` (rc == 1, childless), so drop it to free the shell — not reused,
    // not leaked — and return the canonical inline unit. `op_arr_alloc(0)` === `imm_unit()`.
    if len == 0 {
        op_drop(token); // childless unique shell → frees exactly the token node
        return imm_unit();
    }
    match unsafe { token.0.as_mut() } {
        None => op_arr_alloc(len),
        Some(n) => {
            n.rc = 1;
            n.handles.clear();
            n.handles.resize(len as usize, Handle::NULL);
            // Reset to an EMPTY INLINE raw (an array node carries no raw). `raw.clear()` would keep a
            // heap buffer if the token came from a reset bytes/string leaf — an empty heap Vec retained
            // for the node's life, and a non-canonical rep vs the inline-empty raw a fresh `op_arr_alloc`
            // produces. Assigning the inline-empty raw drops that spill and matches the fresh node.
            n.raw = Raw::Inline { len: 0, buf: [0u8; INLINE_RAW_CAP] };
            token
        }
    }
}

/// `sum-new-reuse` — `sum-new(disc, payload)`, but reusing `token`'s shell when non-null: repurpose
/// it as the `(disc, payload)` node with no new allocation. Null token allocates fresh.
fn op_sum_new_reuse(disc: u32, payload: Handle, token: Handle) -> Handle {
    if is_immediate(token) {
        return op_sum_new(disc, payload); // defensive: reset never yields an immediate token
    }
    match unsafe { token.0.as_mut() } {
        None => op_sum_new(disc, payload),
        Some(n) => {
            n.rc = 1;
            n.handles.clear();
            n.handles.push(payload);
            // Assign a fresh INLINE raw rather than clear()+extend_from_slice: if the token came from a
            // reset bytes/string leaf its raw was `Heap`, and clear() keeps a heap buffer (Vec::clear
            // semantics) — so extending 4 disc bytes into it would leave the reused sum node carrying a
            // HEAP raw where a fresh `op_sum_new` gives an INLINE one. That both retains a stray heap
            // allocation and forks the canonical storage rep for one logical value. A direct inline
            // assignment drops any heap spill and matches `op_sum_new` byte-for-byte.
            n.raw = Raw::inline(&disc.to_le_bytes());
            token
        }
    }
}

// ─── Persistent vector — a 32-way radix trie ──────────────────────────────────────────────
// A persistent (immutable, structurally-shared) growable sequence, laid out as a Bagwell/Clojure
// 32-way radix trie over the SAME tagless `Node`. No new node field and no change to the free
// cascade — exactly the bytes rope's trick: a vector's nodes are ordinary
// `Node`s whose children live in `handles`, so structural sharing is just `rc > 1` on a shared
// subtree and the existing iterative `op_drop` reclaims a whole trie transitively. Tagless dispatch
// keeps this from colliding with tuples/lists: the compiler only ever calls `vec-*` on a value whose
// static type is a Vec and `arr-*` on a tuple/list, so `handles`/`raw` are interpreted as a vector
// only inside these ops (same argument as the rope, §3 of that doc).
//
// # Layout
// - **Header** (the `vec` handle): `raw = [count: u32, shift: u32]`, `handles = [root]` (empty for
//   `count == 0`). `shift` is the root-level radix shift, `BITS * (levels - 1)`; a leaf-only tree
//   (≤ 32 elements) has `shift == 0`.
// - **Trie node** (interior or leaf): `handles = [child/element handles]` (arity 1..=32), `raw = []`.
//   Interior vs leaf is NOT stored — it is determined by the descent depth (`shift`), never a tag.
//   Elements are dense (indices `0..count`), so the trie is "left-full": every branch is full except
//   the rightmost path, and index `i`'s path is its base-32 digits (`(i >> level) & MASK`).
//
// # Ownership (value-heap-runtime.md §Constructors Consume And Accessors Borrow)
// `vec-empty` produces a new owned vector. `vec-push`/`vec-update` are CONSTRUCTORS: they **consume**
// the input vector `v` and the element, and produce a new owned vector — the old version is untouched
// (persistence), so a caller keeping both versions `dup`s `v` before the call (§3.1). Path-copying
// `dup`s every shared subtree carried into the new version, then `op_drop`s the consumed `v`; this is
// correct whether `v` is unique (drop frees the old rightmost spine; shared subtrees survive via the
// dup) or shared (drop just decrements; both versions co-own the shared subtrees). `vec-get` BORROWS
// (rc unchanged; the vector still owns the element); `vec-len` returns a `u32` by value.
//
// # Complexity & bounds
// O(log₃₂ N) index / update / push, sharing all-but-one root→leaf path per update and all-but-the
// rightmost spine per push. Trie height is ≤ 7 for any `u32` count (32⁷ > 2³²), so the trie descent
// recurses at most 7 deep — bounded, unlike the free cascade (which stays iterative in `op_drop` for
// deep UNIQUE structures). FBIP reuse of a unique `v`'s spine (`vec_push_fbip`) and RRB relaxed nodes
// for O(log N) concat/split (`op_vec_concat`/`op_vec_split`, `vec_is_relaxed`) are implemented. One
// optimization remains DEFERRED, with an identical observable contract (so it needs no WIT change): a
// Clojure-style *tail* for amortized-O(1) push.

/// Radix branching bits: 32-way (2⁵) fan-out, the Bagwell/Clojure default.
const VEC_BITS: u32 = 5;
/// Radix digit mask: `(1 << VEC_BITS) - 1` — extracts one base-32 digit of an index.
const VEC_MASK: u32 = (1 << VEC_BITS) - 1;

/// Read a little-endian `u32` at byte `off` of `raw`, zero-padded past the end (total: a short raw
/// yields 0, never a panic — same discipline as `read_word`/`read_disc`).
fn read_u32_at(raw: &[u8], off: usize) -> u32 {
    let mut b = [0u8; 4];
    for k in 0..4 {
        if let Some(&byte) = raw.get(off + k) {
            b[k] = byte;
        }
    }
    u32::from_le_bytes(b)
}

/// The count of a trie node's children (its arity). A null node has none (benign).
fn vec_arity(node: Handle) -> usize {
    with_node(node, 0, |n| n.handles.len())
}
/// The `i`-th child handle of a trie node, or NULL if absent (benign — the descent stays within a
/// valid tree by construction, so this never returns NULL in correct operation).
fn vec_child(node: Handle, i: usize) -> Handle {
    with_node(node, Handle::NULL, |n| {
        n.handles.get(i).copied().unwrap_or(Handle::NULL)
    })
}

/// Build a vector header owning `root` (or childless when `root` is NULL, i.e. the empty vector).
fn vec_alloc_header(count: u32, shift: u32, root: Handle) -> Handle {
    // The 8-byte `[count][shift]` vector header, built INLINE (no transient heap Vec).
    let mut raw = [0u8; INLINE_RAW_CAP];
    raw[0..4].copy_from_slice(&count.to_le_bytes());
    raw[4..8].copy_from_slice(&shift.to_le_bytes());
    let handles = if root == Handle::NULL {
        Vec::new()
    } else {
        vec![root]
    };
    alloc_raw(handles, Raw::Inline { len: 8, buf: raw })
}

/// Decode a header into `(count, shift, root)`. Borrows — no ownership change. A null/short header
/// yields the empty-vector triple.
fn vec_read_header(v: Handle) -> (u32, u32, Handle) {
    with_node(v, (0, 0, Handle::NULL), |n| {
        (
            read_u32_at(&n.raw, 0),
            read_u32_at(&n.raw, 4),
            n.handles.first().copied().unwrap_or(Handle::NULL),
        )
    })
}

/// A one-element leaf node holding `e` (consumed into it).
fn vec_leaf_of(e: Handle) -> Handle {
    alloc(vec![e], Vec::new())
}

/// Append `child` (consumed) to a trie node, `dup`ing the existing children into the copy — the
/// container gains an owned reference to each carried-over subtree while the old node keeps its own
/// (the subtree is now shared). Used both for a leaf gaining an element and an interior gaining a
/// branch; the two are the same op over `handles`.
fn vec_node_append(node: Handle, child: Handle) -> Handle {
    let arity = vec_arity(node);
    let mut hs = Vec::with_capacity(arity + 1);
    for j in 0..arity {
        let c = vec_child(node, j);
        op_dup(c);
        hs.push(c);
    }
    hs.push(child);
    alloc(hs, Vec::new())
}

/// Copy `node`, replacing child `sub` with `new_child` (consumed) and `dup`ing every sibling into the
/// copy (shared). This is the path-copy step: one new node per level, all off-path subtrees shared.
/// Emits a STRICT copy (empty raw); use it only when `node` is strict, or when the replacement changes
/// the node's kind. For a relaxed node whose sizes are unchanged (e.g. an in-place element update),
/// use `vec_node_replace_keep_raw` so the size table survives the copy.
fn vec_node_replace(node: Handle, sub: usize, new_child: Handle) -> Handle {
    let arity = vec_arity(node);
    let mut hs = Vec::with_capacity(arity);
    for j in 0..arity {
        if j == sub {
            hs.push(new_child);
        } else {
            let c = vec_child(node, j);
            op_dup(c);
            hs.push(c);
        }
    }
    alloc(hs, Vec::new())
}

/// Like `vec_node_replace`, but carries `node`'s `raw` (its relaxed size table) into the copy verbatim.
/// Correct only when the replacement does NOT change any child's element count — the case for
/// `vec-update`, which swaps one leaf element for another. Preserves the strict-vs-relaxed kind: a
/// strict node (empty raw) stays strict, a relaxed node keeps its table.
fn vec_node_replace_keep_raw(node: Handle, sub: usize, new_child: Handle) -> Handle {
    let arity = vec_arity(node);
    let mut hs = Vec::with_capacity(arity);
    for j in 0..arity {
        if j == sub {
            hs.push(new_child);
        } else {
            let c = vec_child(node, j);
            op_dup(c);
            hs.push(c);
        }
    }
    let raw = with_node(node, Vec::new(), |n| n.raw.to_vec());
    alloc(hs, raw)
}

/// Push-append helper for a RELAXED node whose last child gained exactly one element: copy the node
/// replacing child `last` with `new_child` (consumed, siblings shared) and bump ONLY the final
/// cumulative-size entry by 1 (every preceding boundary is unchanged since only the last child grew).
fn vec_relaxed_grow_last(node: Handle, last: usize, new_child: Handle) -> Handle {
    let copy = vec_node_replace_keep_raw(node, last, new_child);
    // `copy` is a fresh sole owner (rc 1) carrying the old size table; add 1 to its final u32 entry.
    // Mutate in place via `as_mut` — the same discipline the reuse ops use for a just-allocated node.
    if let Some(n) = unsafe { copy.0.as_mut() } {
        let off = n.raw.len() - 4; // raw.len() == 4*arity ≥ 4 for a relaxed node
        let bumped = read_u32_at(&n.raw, off) + 1;
        n.raw.as_mut_slice()[off..off + 4].copy_from_slice(&bumped.to_le_bytes());
    }
    copy
}

/// Push-append helper for a RELAXED node whose last child is full: copy the node appending `branch`
/// (consumed) as a new rightmost child covering exactly one new element, extending the size table with
/// `old_total + 1`.
fn vec_relaxed_append_branch(node: Handle, branch: Handle) -> Handle {
    let arity = vec_arity(node);
    let mut hs = Vec::with_capacity(arity + 1);
    for j in 0..arity {
        let c = vec_child(node, j);
        op_dup(c);
        hs.push(c);
    }
    hs.push(branch);
    let old_total = if arity == 0 {
        0
    } else {
        vec_relaxed_size_at(node, arity - 1)
    };
    let mut raw = with_node(node, Vec::new(), |n| n.raw.to_vec());
    raw.extend_from_slice(&(old_total + 1).to_le_bytes());
    alloc(hs, raw)
}

/// Build a fresh single-child spine from `level` down to `node` at level 0 (the new-branch case:
/// when a push starts a subtree that does not yet exist). Consumes `node`.
fn vec_new_path(level: u32, node: Handle) -> Handle {
    if level == 0 {
        node
    } else {
        alloc(vec![vec_new_path(level - VEC_BITS, node)], Vec::new())
    }
}

/// Insert element `e` at dense index `i` into the subtree rooted at `node` (borrowed), path-copying.
/// At a leaf (`level == 0`) `e` is appended; at an interior node the rightmost existing child is
/// path-copied (`sub < arity`) or a brand-new branch is appended (`sub == arity`). Returns a new
/// owned subtree; `e` is consumed.
fn vec_push_into(node: Handle, level: u32, i: u32, e: Handle) -> Handle {
    if level == 0 {
        vec_node_append(node, e)
    } else if vec_is_relaxed(node) {
        // Relaxed interior: a push always lands in (or just after) the RIGHTMOST child — appending is
        // strictly a right-edge operation. The last child holds `1 << level` elements at most; if it is
        // not yet full, recurse into it (rebased index) and bump the final size entry; otherwise start a
        // fresh branch and extend the size table. All other boundaries are untouched.
        let arity = vec_arity(node);
        let last = arity - 1; // relaxed nodes always have arity ≥ 1 (no empty nodes)
        let last_size = vec_relaxed_child_size(node, last);
        // The last child is a subtree at `level - VEC_BITS`, so its full capacity is `1 << level`
        // (a leaf at level 0 holds `1 << VEC_BITS` = 32). Recurse while it still has room.
        if (last_size as u64) < (1u64 << level) {
            // Rightmost child has room. Rebase the index into it: subtract every preceding child's
            // size (its own base = the cumulative size up to `last-1`).
            let base = if last == 0 {
                0
            } else {
                vec_relaxed_size_at(node, last - 1)
            };
            let new_child = vec_push_into(vec_child(node, last), level - VEC_BITS, i - base, e);
            vec_relaxed_grow_last(node, last, new_child)
        } else {
            let branch = vec_new_path(level - VEC_BITS, vec_leaf_of(e));
            vec_relaxed_append_branch(node, branch)
        }
    } else {
        let sub = ((i >> level) & VEC_MASK) as usize;
        if sub < vec_arity(node) {
            let new_child = vec_push_into(vec_child(node, sub), level - VEC_BITS, i, e);
            vec_node_replace(node, sub, new_child)
        } else {
            let branch = vec_new_path(level - VEC_BITS, vec_leaf_of(e));
            vec_node_append(node, branch)
        }
    }
}

/// Replace the element at index `i` in the subtree rooted at `node` (borrowed) with `e` (consumed),
/// path-copying the one root→leaf path and sharing everything off it. Returns a new owned subtree.
fn vec_update_into(node: Handle, level: u32, i: u32, e: Handle) -> Handle {
    if level == 0 {
        // Leaf: `i` is the (already rebased, for a relaxed descent) index; its low digit is the slot.
        vec_node_replace(node, (i & VEC_MASK) as usize, e)
    } else if vec_is_relaxed(node) {
        // Relaxed interior: locate the child via the size table and rebase the index; the update
        // leaves every child's element count unchanged, so the size table is carried through verbatim.
        let (sub, local_i) = vec_find_child_relaxed(node, i);
        let new_child = vec_update_into(vec_child(node, sub), level - VEC_BITS, local_i, e);
        vec_node_replace_keep_raw(node, sub, new_child)
    } else {
        let sub = ((i >> level) & VEC_MASK) as usize;
        let new_child = vec_update_into(vec_child(node, sub), level - VEC_BITS, i, e);
        vec_node_replace(node, sub, new_child)
    }
}

// ─── RRB relaxed-radix support (U1 foundation) ────────────────────────────────────────────
// A STRICT trie node (today's default) has `raw.is_empty()`: every child covers exactly `1 << level`
// elements, so a radix digit `(index >> level) & MASK` locates the child. A RELAXED node instead
// carries a per-child cumulative size table in `raw` (u32 LE, one entry per child), so
// `raw.len() == 4 * handles.len()` and `raw.len() > 0`. That table is `[size_0, size_0+size_1, …,
// total]`: entry `i` is the number of elements in children `0..=i`, and the last entry is the whole
// subtree total. Relaxed nodes are only produced by concat/split (U2/U3) at merge boundaries where a
// node's children are irregularly sized; U1 adds the representation and makes the read path honor it,
// but never CREATES one (so every existing all-strict test is unchanged).
//
// Discriminator safety — `raw.len() == 4 * handles.len() && raw.len() > 0` cannot misfire on:
//   • a vector HEADER: built only by `vec_alloc_header`, which always has `handles.len() ∈ {0,1}` and
//     `raw.len() == 8`; 8 == 4*1 would collide only at handles.len()==2, which a header never has.
//   • a STRICT trie node: `raw.is_empty()`, so `raw.len() > 0` is false.
//   • a LEAF: leaves are strict trie nodes (empty raw) here — same as above.
//   • a CHAMP map/set node or a bytes ROPE node: those helpers never call the vec helpers and vec ops
//     never touch them (no cross-op contamination), and their raw sizes (12 for CHAMP, 0/4/8 for rope)
//     don't match `4 * handles.len()` for the arities they carry.

/// True iff `node` is a RELAXED radix node — it carries a per-child cumulative size table in `raw`.
/// Strict nodes have empty raw and return false; a header/leaf/CHAMP/rope node returns false too (see
/// the discriminator-safety note above).
fn vec_is_relaxed(node: Handle) -> bool {
    with_node(node, false, |n| {
        let rlen = n.raw.len();
        let hlen = n.handles.len();
        rlen > 0 && rlen == 4 * hlen
    })
}

/// The `i`-th cumulative size (u32 LE at offset `4*i`) of a relaxed node's size table — the element
/// count of children `0..=i`. Short/absent raw yields 0 (benign, matching `read_u32_at`).
fn vec_relaxed_size_at(node: Handle, i: usize) -> u32 {
    with_node(node, 0, |n| read_u32_at(&n.raw, 4 * i))
}

/// The element count of child `i` alone in a relaxed node: `sizes[i] - sizes[i-1]` (with
/// `sizes[-1] := 0`). Used by U2/U3 rebalancing; kept here beside its reader.
#[cfg_attr(not(test), allow(dead_code))]
fn vec_relaxed_child_size(node: Handle, i: usize) -> u32 {
    let s = vec_relaxed_size_at(node, i);
    if i == 0 {
        s
    } else {
        s - vec_relaxed_size_at(node, i - 1)
    }
}

/// Locate the child of a RELAXED `node` that contains dense index `idx`, returning `(sub, local_idx)`
/// where `sub` is the child slot and `local_idx = idx - sizes[sub-1]` is the index rebased into that
/// child's subtree. Binary-searches the cumulative size table for the least `sub` with `idx <
/// sizes[sub]`. `idx` must be in-bounds (`idx < total`) — the caller's `index < count` guard ensures it.
fn vec_find_child_relaxed(node: Handle, idx: u32) -> (usize, u32) {
    let arity = vec_arity(node);
    let mut lo = 0usize;
    let mut hi = arity;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if idx < vec_relaxed_size_at(node, mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let base = if lo == 0 {
        0
    } else {
        vec_relaxed_size_at(node, lo - 1)
    };
    (lo, idx - base)
}

/// `vec-empty` — a new owned empty vector (rc 1). No root node until the first push.
fn op_vec_empty() -> Handle {
    vec_alloc_header(0, 0, Handle::NULL)
}

/// `vec-len` — the element count. Borrows; returns a `u32` by value.
fn op_vec_len(v: Handle) -> u32 {
    vec_read_header(v).0
}

/// `vec-get` — the element at `index` (BORROWED; the vector keeps ownership). An out-of-bounds index
/// TRAPS (fail-fast, like `arr-get`); the compiler emits the sign-aware bounds check on its side, so
/// reaching the trap is a compiler-invariant violation. After the `index < count` guard the trie
/// descent is in-bounds by construction.
fn op_vec_get(v: Handle, index: u32) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    if index >= count {
        trap_oob();
    }
    let mut node = root;
    let mut idx = index;
    let mut s = shift;
    while s > 0 {
        if vec_is_relaxed(node) {
            // Relaxed interior: scan the cumulative size table for the child holding `idx`, then
            // rebase `idx` into that child's subtree (subtract the preceding cumulative size).
            let (sub, local_idx) = vec_find_child_relaxed(node, idx);
            node = vec_child(node, sub);
            idx = local_idx;
        } else {
            // Strict interior: the radix digit is the child slot; `idx` stays absolute (the leaf-level
            // `idx & VEC_MASK` extracts the low digit) because a strict subtree is aligned to its level.
            node = vec_child(node, ((idx >> s) & VEC_MASK) as usize);
        }
        s -= VEC_BITS;
    }
    // Leaf level. After descending through any relaxed node, `idx` has been rebased so that its low
    // `VEC_BITS` are the slot within this leaf; for an all-strict descent `idx == index`, unchanged.
    vec_child(node, (idx & VEC_MASK) as usize)
}

// ─── FBIP (Functional But In-Place) rc==1 spine reuse for vec-push / vec-update (U4) ──────────
// When the touched spine is UNIQUELY owned we refit nodes in place instead of path-copying — zero
// heap traffic for the single-threaded push/update chains that dominate. Observationally IDENTICAL to
// the path-copy version; the win is fewer allocations.
//
// 🚨 ALIASING SAFETY (a violation silently corrupts a shared persistent version). `mine` means "this
// node is on a fully-unique path and is safe to mutate in place". It propagates STRICTLY DOWNWARD:
//   mine(root)  = header.rc == 1 && root.rc == 1
//   mine(child) = mine(parent)  && child.rc == 1
// Two facts make the per-node check mandatory: (1) `op_dup` on a header bumps ONLY the header's rc, so
// a header with rc>1 can still reach an rc==1 root — hence the header gate. (2) A child with rc==1 is
// still SHARED if any ancestor is shared (the other version reaches it THROUGH that ancestor) — hence
// once `mine` is false it stays false and we fall back to the exact existing path-copy for the whole
// remaining subtree. `mine=false` ⇒ delegate to `vec_push_into`/`vec_update_into` verbatim.

/// In-place set child slot `sub` of a node to `child` (no dup, no refcount bookkeeping — the caller
/// owns the transferred `child` and separately releases whatever was there). SAFETY contract: the
/// caller has verified this node is uniquely owned (rc == 1) before mutating it.
fn vec_set_child_inplace(node: Handle, sub: usize, child: Handle) {
    if let Some(n) = unsafe { node.0.as_mut() } {
        if let Some(slot) = n.handles.get_mut(sub) {
            *slot = child;
        }
    }
}

/// In-place append `child` to an rc==1 node's handles (no dup). SAFETY: caller verified rc == 1.
fn vec_push_child_inplace(node: Handle, child: Handle) {
    if let Some(n) = unsafe { node.0.as_mut() } {
        n.handles.push(child);
    }
}

/// In-place set an rc==1 vector HEADER's `[count][shift]` raw. SAFETY: caller verified rc == 1.
fn vec_set_header_inplace(v: Handle, count: u32, shift: u32) {
    if let Some(n) = unsafe { v.0.as_mut() } {
        n.raw.clear();
        n.raw.extend_from_slice(&count.to_le_bytes());
        n.raw.extend_from_slice(&shift.to_le_bytes());
    }
}

/// In-place add 1 to the FINAL u32 entry of an rc==1 relaxed node's size table (a push into its last
/// child grew that child by one element). SAFETY: caller verified rc == 1 and the node is relaxed.
fn vec_bump_last_size_inplace(node: Handle) {
    if let Some(n) = unsafe { node.0.as_mut() } {
        let off = n.raw.len() - 4;
        let bumped = read_u32_at(&n.raw, off) + 1;
        n.raw.as_mut_slice()[off..off + 4].copy_from_slice(&bumped.to_le_bytes());
    }
}

/// In-place append a new rightmost child `branch` (covering one new element) to an rc==1 relaxed node,
/// extending its size table with `old_total + 1`. SAFETY: caller verified rc == 1 and relaxed.
fn vec_relaxed_append_branch_inplace(node: Handle, branch: Handle) {
    let old_total = {
        let arity = vec_arity(node);
        if arity == 0 {
            0
        } else {
            vec_relaxed_size_at(node, arity - 1)
        }
    };
    if let Some(n) = unsafe { node.0.as_mut() } {
        n.handles.push(branch);
        n.raw.extend_from_slice(&(old_total + 1).to_le_bytes());
    }
}

/// FBIP variant of `vec_update_into`. When `mine`, mutate `node` in place and return the SAME handle;
/// otherwise delegate to the path-copying `vec_update_into` (returns a fresh node). Element counts are
/// unchanged by an update, so a relaxed node's size table is left untouched. Bounded-depth (≤7).
fn vec_update_fbip(node: Handle, level: u32, i: u32, e: Handle, mine: bool) -> Handle {
    if !mine {
        return vec_update_into(node, level, i, e); // shared: exact existing path-copy
    }
    if level == 0 {
        // Leaf: swap the element slot in place, releasing the old element (matches path-copy's net RC).
        let slot = (i & VEC_MASK) as usize;
        let old = vec_child(node, slot);
        vec_set_child_inplace(node, slot, e);
        op_drop(old);
        return node;
    }
    let (sub, loc) = if vec_is_relaxed(node) {
        vec_find_child_relaxed(node, i)
    } else {
        (((i >> level) & VEC_MASK) as usize, i)
    };
    let child = vec_child(node, sub);
    let child_mine = node_rc(child) == 1;
    let new_child = vec_update_fbip(child, level - VEC_BITS, loc, e, child_mine);
    if !child_mine {
        // Child was shared → path-copied into `new_child`; swap it in, release the old shared child.
        vec_set_child_inplace(node, sub, new_child);
        op_drop(child);
    }
    // else: child mutated in place (new_child == child); node already points to it. Raw unchanged.
    node
}

/// FBIP variant of `vec_push_into`. When `mine`, mutate the rightmost spine in place and return the
/// SAME handle; otherwise delegate to the path-copying `vec_push_into`. Maintains relaxed size tables
/// in place (bump-last on descent, extend on a new branch). Bounded-depth (≤7).
fn vec_push_fbip(node: Handle, level: u32, i: u32, e: Handle, mine: bool) -> Handle {
    if !mine {
        return vec_push_into(node, level, i, e); // shared: exact existing path-copy
    }
    if level == 0 {
        vec_push_child_inplace(node, e); // leaf gains the element
        return node;
    }
    if vec_is_relaxed(node) {
        let arity = vec_arity(node);
        let last = arity - 1;
        let last_size = vec_relaxed_child_size(node, last);
        if (last_size as u64) < (1u64 << level) {
            let base = if last == 0 {
                0
            } else {
                vec_relaxed_size_at(node, last - 1)
            };
            let child = vec_child(node, last);
            let child_mine = node_rc(child) == 1;
            let new_child = vec_push_fbip(child, level - VEC_BITS, i - base, e, child_mine);
            if !child_mine {
                vec_set_child_inplace(node, last, new_child);
                op_drop(child);
            }
            vec_bump_last_size_inplace(node); // the last child gained one element
        } else {
            let branch = vec_new_path(level - VEC_BITS, vec_leaf_of(e));
            vec_relaxed_append_branch_inplace(node, branch);
        }
    } else {
        let sub = ((i >> level) & VEC_MASK) as usize;
        if sub < vec_arity(node) {
            let child = vec_child(node, sub);
            let child_mine = node_rc(child) == 1;
            let new_child = vec_push_fbip(child, level - VEC_BITS, i, e, child_mine);
            if !child_mine {
                vec_set_child_inplace(node, sub, new_child);
                op_drop(child);
            }
        } else {
            let branch = vec_new_path(level - VEC_BITS, vec_leaf_of(e));
            vec_push_child_inplace(node, branch);
        }
    }
    node
}

/// `vec-push` — a new owned vector = `v` with `elem` appended at the end. CONSUMES `v` and `elem`.
/// FBIP fast path: when `v`'s header is uniquely owned it is REUSED as the result and the touched
/// rightmost spine is refit in place wherever each node is uniquely owned (rc==1); anywhere the spine
/// is shared it path-copies exactly as before (persistence preserved). When the header itself is shared
/// (another version holds `v`), the whole op is the original allocate-new-header path.
fn op_vec_push(v: Handle, elem: Handle) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    let header_mine = node_rc(v) == 1;

    if !header_mine {
        // Header shared: original behavior — build a fresh version, leave `v` (and its version) intact.
        let (new_root, new_shift) = if count == 0 {
            (vec_leaf_of(elem), 0)
        } else if (count as u64) == (1u64 << (shift + VEC_BITS)) {
            op_dup(root);
            let branch = vec_new_path(shift, vec_leaf_of(elem));
            (alloc(vec![root, branch], Vec::new()), shift + VEC_BITS)
        } else {
            (vec_push_into(root, shift, count, elem), shift)
        };
        let hdr = vec_alloc_header(count + 1, new_shift, new_root);
        op_drop(v);
        return hdr;
    }

    // Header uniquely owned: reuse it as the result (no header alloc, no drop of `v`).
    if count == 0 {
        // First element: the empty header gains a one-element leaf; shift stays 0.
        vec_push_child_inplace(v, vec_leaf_of(elem));
        vec_set_header_inplace(v, 1, 0);
    } else if (count as u64) == (1u64 << (shift + VEC_BITS)) {
        // Root full → grow a level. Transfer the old root into the new root WITHOUT a dup: `v`'s single
        // owned reference to `root` relocates into `new_root` (whatever else references `root` is
        // untouched), so no dup/drop is needed regardless of `root`'s own rc.
        let branch = vec_new_path(shift, vec_leaf_of(elem));
        let new_root = alloc(vec![root, branch], Vec::new());
        vec_set_child_inplace(v, 0, new_root);
        vec_set_header_inplace(v, count + 1, shift + VEC_BITS);
    } else {
        let root_mine = node_rc(root) == 1;
        let new_root = vec_push_fbip(root, shift, count, elem, root_mine);
        if !root_mine {
            // Root was shared → path-copied; swap the copy into the reused header, release old root.
            vec_set_child_inplace(v, 0, new_root);
            op_drop(root);
        }
        vec_set_header_inplace(v, count + 1, shift);
    }
    v
}

/// `vec-update` — a new owned vector = `v` with `index` set to `elem`. CONSUMES `v` and `elem`. OOB
/// index traps (like `vec-get`). FBIP fast path: when `v`'s header is uniquely owned it is reused and
/// the affected root→leaf path is refit in place wherever each node is uniquely owned; a shared node
/// path-copies exactly as before. A shared header takes the original allocate-new-header path.
fn op_vec_update(v: Handle, index: u32, elem: Handle) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    if index >= count {
        trap_oob();
    }
    if node_rc(v) != 1 {
        // Header shared: original path-copy; `v` (the other version) stays byte-identical.
        let new_root = vec_update_into(root, shift, index, elem);
        let hdr = vec_alloc_header(count, shift, new_root);
        op_drop(v);
        return hdr;
    }
    // Header uniquely owned: reuse it. count/shift are unchanged by an update, so its raw is unchanged.
    let root_mine = node_rc(root) == 1;
    let new_root = vec_update_fbip(root, shift, index, elem, root_mine);
    if !root_mine {
        // Root was shared → path-copied; swap the copy into the reused header, release the old root.
        vec_set_child_inplace(v, 0, new_root);
        op_drop(root);
    }
    v
}

/// The element count of the subtree rooted at `node`, whose top level is `level`. A leaf (`level == 0`)
/// contributes its arity; a RELAXED node reads the last cumulative-size entry (O(1)); a STRICT interior
/// node has all-but-its-last child full (`1 << level` each) and recurses only into the last (partial)
/// child — so the recursion is bounded by the trie height (≤7), never fanning out. Used to build a
/// merged node's cumulative size table during concat.
fn vec_subtree_size(node: Handle, level: u32) -> u32 {
    if level == 0 {
        return vec_arity(node) as u32;
    }
    let arity = vec_arity(node);
    if arity == 0 {
        return 0;
    }
    if vec_is_relaxed(node) {
        return vec_relaxed_size_at(node, arity - 1);
    }
    // Strict interior: the first `arity-1` children are each full at this level; the last may be partial.
    let full = (arity as u32 - 1) * (1u32 << level);
    full + vec_subtree_size(vec_child(node, arity - 1), level - VEC_BITS)
}

/// Lift an OWNED subtree `node` (currently at `shift`) to `target` shift by wrapping it in STRICT
/// single-child interior nodes (one per level). Consumes `node` — its ownership transfers into the
/// innermost wrapper. The wrapper chain is a valid strict, left-aligned subtree (exactly what
/// `vec_new_path` builds), so radix descent through it always selects child 0 and reaches `node`.
/// `target >= shift`; the loop is bounded by their difference (≤ the 7-level trie depth per operand).
fn vec_grow_to_shift(mut node: Handle, mut shift: u32, target: u32) -> Handle {
    while shift < target {
        node = alloc(vec![node], Vec::new()); // strict single-child wrapper; ownership of `node` moves in
        shift += VEC_BITS;
    }
    node
}

/// Push each child of an OWNED node into `out`, `dup`ing it so it survives the node's later `op_drop`
/// (same discipline as `vec_node_append`). The node keeps its own references; `out` gains owned ones.
fn vec_collect_children_dup(node: Handle, out: &mut Vec<Handle>) {
    let arity = vec_arity(node);
    for j in 0..arity {
        let c = vec_child(node, j);
        op_dup(c);
        out.push(c);
    }
}

/// Build a RELAXED interior node at `level` (its children are subtrees at `level - VEC_BITS`) owning
/// `children` (consumed). Computes the cumulative size table `[Σsize(0..=i)]` from each child's actual
/// element count via `vec_subtree_size`, so the table is strictly increasing with last == subtree
/// total (the U1 invariants). Precondition: `children` non-empty, none zero-size (the callers only pass
/// real subtrees of non-empty operands).
fn vec_relaxed_node(children: Vec<Handle>, level: u32) -> Handle {
    let child_level = level - VEC_BITS;
    let mut raw = Vec::with_capacity(4 * children.len());
    let mut running = 0u32;
    for &c in &children {
        running += vec_subtree_size(c, child_level);
        raw.extend_from_slice(&running.to_le_bytes());
    }
    alloc(children, raw)
}

/// `vec-concat` — a new owned vector = the elements of `a` followed by the elements of `b`. CONSUMES
/// both `a` and `b` (a constructor, like `vec-push`); a caller keeping either version `dup`s it first.
///
/// Ownership: an empty operand is the identity — `concat(empty, b)` returns `b` unchanged (its rc
/// already covers the caller) and reclaims the consumed empty header; likewise `concat(a, empty)`.
///
/// Algorithm (CONSERVATIVE-CORRECT, not FULL RRB leaf-rebalance — see the U2 handoff): lift both roots
/// to a common level, then CONCATENATE their child lists at that level. If the combined child list fits
/// one node (≤ 32) the result is a single node at that level (NO height increase); otherwise the list
/// is split into two balanced groups → two nodes → a 2-child parent one level up. A leaf-level merge of
/// two small vectors yields one STRICT leaf (indistinguishable from a push-built one); every other
/// merged node is RELAXED with a correct cumulative size table (built from `vec_subtree_size`). This is
/// "conservative" because it does NOT rebalance the boundary LEAVES (two partial leaves can stay
/// adjacent — the result is not maximally dense), but height stays minimal-ish: it only grows a level
/// when a node would exceed 32 children, so repeated fold-concat does NOT build a degenerate deep tree,
/// and `vec-get`/`vec-len`/`vec-push`/`vec-update` all traverse the result correctly. Bounded-depth
/// only: `vec_grow_to_shift`/`vec_subtree_size` are bounded by the ≤7-level trie height; no fan-out
/// recursion.
fn op_vec_concat(a: Handle, b: Handle) -> Handle {
    let (count_a, shift_a, root_a) = vec_read_header(a);
    let (count_b, shift_b, root_b) = vec_read_header(b);
    if count_a == 0 {
        op_drop(a); // reclaim the empty header; `b` passes through unchanged
        return b;
    }
    if count_b == 0 {
        op_drop(b);
        return a;
    }
    let total = count_a + count_b;
    let cap = VEC_MASK as usize + 1; // 32 — max children per node

    // Lift both roots to a common level and gather their children (dup'd) into one list at that level.
    let max_shift = shift_a.max(shift_b);
    op_dup(root_a);
    op_dup(root_b);
    let grown_a = vec_grow_to_shift(root_a, shift_a, max_shift);
    let grown_b = vec_grow_to_shift(root_b, shift_b, max_shift);
    let mut children: Vec<Handle> = Vec::new();
    vec_collect_children_dup(grown_a, &mut children);
    vec_collect_children_dup(grown_b, &mut children);
    op_drop(grown_a); // releases the grown wrappers + our root dups; the dup'd children survive in `children`
    op_drop(grown_b);

    // Build the merged root from `children` (subtrees at level `max_shift`, i.e. children of a node at
    // `max_shift + VEC_BITS`; when `max_shift == 0` they are ELEMENTS of a leaf).
    // `children` are the child subtrees of nodes at level `max_shift`, i.e. they live at level
    // `max_shift` themselves when `max_shift == 0` (elements) — more precisely, a node built from them
    // has its OWN level equal to `max_shift` and its children at `max_shift - VEC_BITS`.
    let m = children.len(); // ≤ 64 (each root has ≤32 children after growth)
    let (new_root, new_shift) = if m <= cap {
        if max_shift == 0 {
            // Leaf merge: elements fit one leaf → keep it STRICT (uniform size-1, no table needed).
            (alloc(children, Vec::new()), 0)
        } else {
            // One relaxed node (level `max_shift`) holds every gathered child; no height increase.
            (vec_relaxed_node(children, max_shift), max_shift)
        }
    } else {
        // Overflow: split into two balanced groups, wrap each at level `max_shift`, then a 2-child
        // relaxed parent at level `max_shift + VEC_BITS` (one level up).
        let k = m.div_ceil(2); // each group ≤ 32 since m ≤ 64
        let right = children.split_off(k);
        let left = children;
        let (g_left, g_right) = if max_shift == 0 {
            (alloc(left, Vec::new()), alloc(right, Vec::new())) // two strict leaves (level 0)
        } else {
            (
                vec_relaxed_node(left, max_shift),
                vec_relaxed_node(right, max_shift),
            )
        };
        (
            vec_relaxed_node(vec![g_left, g_right], max_shift + VEC_BITS),
            max_shift + VEC_BITS,
        )
    };

    let hdr = vec_alloc_header(total, new_shift, new_root);
    op_drop(a);
    op_drop(b);
    hdr
}

/// Split the subtree rooted at `node` (BORROWED, top level `level`) at LOCAL element index `idx`
/// (`0 < idx < subtree_size` at the top; deeper calls may hit the `idx==0`/`idx==size` boundaries).
/// Returns `(left, right)` as freshly OWNED subtrees at the SAME level as `node` — `Handle::NULL` for a
/// side that ends up empty. Carried-over whole children are `dup`ed (they survive the caller's later
/// `op_drop(v)`); the boundary child is split recursively. EVERY rebuilt boundary node is RELAXED with a
/// correct cumulative size table (a trim can leave an interior node non-dense — U2's gotcha — so it must
/// NOT stay strict); LEAVES stay strict (their elements are uniformly size-1, valid after any trim).
/// Bounded-depth: `level` drops by `VEC_BITS` each call, so recursion is ≤ the 7-level trie height.
fn vec_split_subtree(node: Handle, level: u32, idx: u32) -> (Handle, Handle) {
    if level == 0 {
        // Leaf: partition its elements. Each retained element is `dup`ed (the leaf still owns its own).
        let arity = vec_arity(node) as u32;
        let left = if idx == 0 {
            Handle::NULL
        } else {
            let mut hs = Vec::with_capacity(idx as usize);
            for j in 0..idx {
                let e = vec_child(node, j as usize);
                op_dup(e);
                hs.push(e);
            }
            alloc(hs, Vec::new()) // strict leaf
        };
        let right = if idx >= arity {
            Handle::NULL
        } else {
            let mut hs = Vec::with_capacity((arity - idx) as usize);
            for j in idx..arity {
                let e = vec_child(node, j as usize);
                op_dup(e);
                hs.push(e);
            }
            alloc(hs, Vec::new()) // strict leaf
        };
        return (left, right);
    }
    // Interior: find the child holding `idx` and the index within it.
    let arity = vec_arity(node);
    let (sub, loc) = if vec_is_relaxed(node) {
        vec_find_child_relaxed(node, idx)
    } else {
        // Strict, left-aligned, dense: child `sub` starts at `sub << level`.
        let sub = ((idx >> level) & VEC_MASK) as usize;
        (sub, idx & ((1u32 << level) - 1))
    };
    let (cl, cr) = vec_split_subtree(vec_child(node, sub), level - VEC_BITS, loc);
    // LEFT: whole children [0, sub) (dup'ed) then the split child's left part (if any).
    let mut left_children: Vec<Handle> = Vec::with_capacity(sub + 1);
    for j in 0..sub {
        let c = vec_child(node, j);
        op_dup(c);
        left_children.push(c);
    }
    if cl != Handle::NULL {
        left_children.push(cl);
    }
    // RIGHT: the split child's right part (if any) then whole children (sub, arity) (dup'ed).
    let mut right_children: Vec<Handle> = Vec::with_capacity(arity - sub);
    if cr != Handle::NULL {
        right_children.push(cr);
    }
    for j in (sub + 1)..arity {
        let c = vec_child(node, j);
        op_dup(c);
        right_children.push(c);
    }
    let left = if left_children.is_empty() {
        Handle::NULL
    } else {
        vec_relaxed_node(left_children, level) // rebuilt boundary node → relaxed with correct table
    };
    let right = if right_children.is_empty() {
        Handle::NULL
    } else {
        vec_relaxed_node(right_children, level)
    };
    (left, right)
}

/// `vec-split` — split `v` at element `index` into `(left, right)` where `left` is elements
/// `[0, index)` and `right` is elements `[index, len)`. CONSUMES `v`; returns TWO new owned vectors.
/// Boundaries: `index == 0` → `(empty, v)` (v flows through as the right output, unchanged); `index >=
/// len` → `(v, empty)`. Both outputs honor every relaxed-node invariant and are valid for downstream
/// get/len/push/update/concat. Bounded-depth (via `vec_split_subtree`). `index > len` is clamped to
/// `len` (total split — a benign no-op split, mirroring how the boundary is the identity).
fn op_vec_split(v: Handle, index: u32) -> (Handle, Handle) {
    let (count, shift, root) = vec_read_header(v);
    if index == 0 {
        // Left empty; `v` becomes the right output unchanged (no drop — its rc already covers the caller).
        return (op_vec_empty(), v);
    }
    if index >= count {
        // Right empty; `v` becomes the left output unchanged.
        return (v, op_vec_empty());
    }
    let (lroot, rroot) = vec_split_subtree(root, shift, index);
    // Both roots are non-NULL here (0 < index < count) and live at level `shift` — the header shift is
    // unchanged (split never increases height; a half may be sparse but descent stays correct).
    let left = vec_alloc_header(index, shift, lroot);
    let right = vec_alloc_header(count - index, shift, rroot);
    op_drop(v); // consume the input; carried subtrees survive via the dups, the boundary spine frees
    (left, right)
}

// ─── The WIT `Guest` impl: the ONLY place `u32` handles exist ───────────────────────────
// Present only in the wasm component build. Handles narrow to `u32` losslessly (wasm32 pointers are
// 32-bit); genuine runtime data (`arr-len` count, `sum-disc`, byte values) is already `u32` and
// passes straight through — the conversion is applied to handles alone.

#[cfg(target_arch = "wasm32")]
impl Handle {
    /// Narrow to the opaque public handle. Lossless on wasm32 (a `Node` address is 32-bit).
    fn to_u32(self) -> u32 {
        self.0 as usize as u32
    }
    /// Widen a public handle back to a node pointer. Inverse of `to_u32` on wasm32.
    fn from_u32(x: u32) -> Handle {
        Handle(x as usize as *mut Node)
    }
}

#[cfg(target_arch = "wasm32")]
struct Component;

#[cfg(target_arch = "wasm32")]
impl Guest for Component {
    fn box_int(v: i64) -> u32 {
        op_box_int(v).to_u32()
    }
    fn get_int(handle: u32) -> i64 {
        op_get_int(Handle::from_u32(handle))
    }
    fn box_bool(v: bool) -> u32 {
        op_box_bool(v).to_u32()
    }
    fn get_bool(handle: u32) -> bool {
        op_get_bool(Handle::from_u32(handle))
    }
    fn box_float(v: f64) -> u32 {
        op_box_float(v).to_u32()
    }
    fn get_float(handle: u32) -> f64 {
        op_get_float(Handle::from_u32(handle))
    }
    fn arr_alloc(len: u32) -> u32 {
        op_arr_alloc(len).to_u32()
    }
    fn arr_set(arr: u32, index: u32, elem: u32) -> u32 {
        op_arr_set(Handle::from_u32(arr), index, Handle::from_u32(elem)).to_u32()
    }
    fn arr_get(arr: u32, index: u32) -> u32 {
        op_arr_get(Handle::from_u32(arr), index).to_u32()
    }
    fn arr_len(arr: u32) -> u32 {
        op_arr_len(Handle::from_u32(arr))
    }
    fn sum_new(disc: u32, payload: u32) -> u32 {
        op_sum_new(disc, Handle::from_u32(payload)).to_u32()
    }
    fn sum_disc(handle: u32) -> u32 {
        op_sum_disc(Handle::from_u32(handle))
    }
    fn sum_payload(handle: u32) -> u32 {
        op_sum_payload(Handle::from_u32(handle)).to_u32()
    }
    fn bytes_alloc(len: u32) -> u32 {
        op_bytes_alloc(len).to_u32()
    }
    fn bytes_set(buf: u32, index: u32, value: u32) -> u32 {
        op_bytes_set(Handle::from_u32(buf), index, value).to_u32()
    }
    fn bytes_get(buf: u32, index: u32) -> u32 {
        op_bytes_get(Handle::from_u32(buf), index)
    }
    fn bytes_len(buf: u32) -> u32 {
        op_bytes_len(Handle::from_u32(buf))
    }
    fn str_new(s: String) -> u32 {
        op_str_new(s).to_u32()
    }
    fn str_get(handle: u32) -> String {
        op_str_get(Handle::from_u32(handle))
    }
    fn map_alloc(len: u32) -> u32 {
        op_map_alloc(len).to_u32()
    }
    fn map_set(m: u32, index: u32, key: u32, value: u32) -> u32 {
        op_map_set(
            Handle::from_u32(m),
            index,
            Handle::from_u32(key),
            Handle::from_u32(value),
        )
        .to_u32()
    }
    fn map_key(m: u32, index: u32) -> u32 {
        op_map_key(Handle::from_u32(m), index).to_u32()
    }
    fn map_val(m: u32, index: u32) -> u32 {
        op_map_val(Handle::from_u32(m), index).to_u32()
    }
    fn map_len(m: u32) -> u32 {
        op_map_len(Handle::from_u32(m))
    }

    // ── CHAMP persistent map (§37–45) ────────────────────────────────────────────────────
    fn map_empty() -> u32 {
        op_map_empty().to_u32()
    }
    fn map_insert(m: u32, key: u32, val: u32) -> u32 {
        op_map_insert(Handle::from_u32(m), Handle::from_u32(key), Handle::from_u32(val)).to_u32()
    }
    fn map_lookup(m: u32, key: u32) -> u32 {
        op_map_lookup(Handle::from_u32(m), Handle::from_u32(key)).to_u32()
    }
    fn map_remove(m: u32, key: u32) -> u32 {
        op_map_remove(Handle::from_u32(m), Handle::from_u32(key)).to_u32()
    }
    fn map_size(m: u32) -> u32 {
        op_map_size(Handle::from_u32(m))
    }
    fn map_iter(m: u32) -> u32 {
        op_map_iter(Handle::from_u32(m)).to_u32()
    }
    fn map_iter_next(cur: u32) -> u32 {
        op_map_iter_next(Handle::from_u32(cur)).to_u32()
    }
    fn map_iter_key(cur: u32) -> u32 {
        op_map_iter_key(Handle::from_u32(cur)).to_u32()
    }
    fn map_iter_val(cur: u32) -> u32 {
        op_map_iter_val(Handle::from_u32(cur)).to_u32()
    }

    // ── CHAMP persistent set (§46–53) ────────────────────────────────────────────────────
    fn set_empty() -> u32 {
        op_set_empty().to_u32()
    }
    fn set_insert(s: u32, elem: u32) -> u32 {
        op_set_insert(Handle::from_u32(s), Handle::from_u32(elem)).to_u32()
    }
    fn set_contains(s: u32, elem: u32) -> bool {
        op_set_contains(Handle::from_u32(s), Handle::from_u32(elem))
    }
    fn set_remove(s: u32, elem: u32) -> u32 {
        op_set_remove(Handle::from_u32(s), Handle::from_u32(elem)).to_u32()
    }
    fn set_size(s: u32) -> u32 {
        op_set_size(Handle::from_u32(s))
    }
    fn set_iter(s: u32) -> u32 {
        op_set_iter(Handle::from_u32(s)).to_u32()
    }
    fn set_iter_next(cur: u32) -> u32 {
        op_set_iter_next(Handle::from_u32(cur)).to_u32()
    }
    fn set_iter_elem(cur: u32) -> u32 {
        op_set_iter_elem(Handle::from_u32(cur)).to_u32()
    }
    fn set_union(a: u32, b: u32) -> u32 {
        op_set_union(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn set_intersection(a: u32, b: u32) -> u32 {
        op_set_intersection(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn set_difference(a: u32, b: u32) -> u32 {
        op_set_difference(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }

    fn dup(handle: u32) {
        op_dup(Handle::from_u32(handle))
    }
    fn drop(handle: u32) {
        op_drop(Handle::from_u32(handle))
    }
    fn reset(node: u32) -> u32 {
        op_reset(Handle::from_u32(node)).to_u32()
    }
    fn arr_alloc_reuse(len: u32, token: u32) -> u32 {
        op_arr_alloc_reuse(len, Handle::from_u32(token)).to_u32()
    }
    fn sum_new_reuse(disc: u32, payload: u32, token: u32) -> u32 {
        op_sum_new_reuse(disc, Handle::from_u32(payload), Handle::from_u32(token)).to_u32()
    }
    fn vec_empty() -> u32 {
        op_vec_empty().to_u32()
    }
    fn vec_len(v: u32) -> u32 {
        op_vec_len(Handle::from_u32(v))
    }
    fn vec_get(v: u32, index: u32) -> u32 {
        op_vec_get(Handle::from_u32(v), index).to_u32()
    }
    fn vec_push(v: u32, elem: u32) -> u32 {
        op_vec_push(Handle::from_u32(v), Handle::from_u32(elem)).to_u32()
    }
    fn vec_update(v: u32, index: u32, elem: u32) -> u32 {
        op_vec_update(Handle::from_u32(v), index, Handle::from_u32(elem)).to_u32()
    }
    fn vec_concat(a: u32, b: u32) -> u32 {
        op_vec_concat(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn vec_split(v: u32, index: u32) -> (u32, u32) {
        let (l, r) = op_vec_split(Handle::from_u32(v), index);
        (l.to_u32(), r.to_u32())
    }
    fn bytes_concat(a: u32, b: u32) -> u32 {
        op_bytes_concat(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bytes_slice(buf: u32, start: u32, len: u32) -> u32 {
        op_bytes_slice(Handle::from_u32(buf), start, len).to_u32()
    }
    fn bytes_compact(buf: u32) -> u32 {
        op_bytes_compact(Handle::from_u32(buf)).to_u32()
    }
    // Debug leak oracle (index 54). The number of live heap objects, or 0 when the counter is not
    // compiled in (default build). Not imported by the compiler; a leak-check harness asserts it is 0
    // after a run to verify the Perceus dup/drop discipline balances.
    fn live_objects() -> u32 {
        live_object_count()
    }
}

#[cfg(target_arch = "wasm32")]
bindings::export!(Component with_types_in bindings);

// ─── CHAMP (Compressed Hash-Array Mapped Prefix trie) shared node core ───────────────────
// The tag-free foundation shared by the persistent map AND set: header/bitmap/slot helpers plus
// structural hash and eq. The map/set insert/lookup/remove/iter ops are built on top of these.
//
// Node layout (mirrors the vector radix trie's tag-free discipline, §DESIGN):
//   raw     = [datamap:u32 LE][nodemap:u32 LE][size:u32 LE]   (12 bytes)
//   handles = [entries in ascending datamap-bit order][subnodes in ascending nodemap-bit order]
// datamap bit i ⇒ slot i is an inline ENTRY; nodemap bit i ⇒ slot i is a SUBNODE. `size` is the
// total entry count in the subtree (O(1) size + an eq inequality fast-reject). There is NO runtime
// tag — the compiler's static type selects the map vs set op family and the entry stride (2 vs 1).
//
// The trie is 32-way, 5 bits per level (reusing VEC_BITS/VEC_MASK), 7 levels over a 32-bit hash.

/// CHAMP node header size in bytes: three little-endian u32s (datamap, nodemap, size).
const CHAMP_HEADER_SIZE: usize = 12;

/// 32-bit FNV-1a offset basis and prime, for the structural hash.
const FNV_OFFSET: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

/// One FNV-1a byte step: xor then multiply. `wrapping_mul` because FNV is defined mod 2^32.
#[inline]
fn fnv_step(h: u32, b: u8) -> u32 {
    (h ^ b as u32).wrapping_mul(FNV_PRIME)
}

// ── Header read/write ───────────────────────────────────────────────────────────────────

/// Read the datamap word (u32 LE at offset 0) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
fn champ_datamap(raw: &[u8]) -> u32 {
    read_u32_at(raw, 0)
}
/// Read the nodemap word (u32 LE at offset 4) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
fn champ_nodemap(raw: &[u8]) -> u32 {
    read_u32_at(raw, 4)
}
/// Read the subtree size (u32 LE at offset 8) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
fn champ_size(raw: &[u8]) -> u32 {
    read_u32_at(raw, 8)
}
/// Build a CHAMP raw header `[datamap][nodemap][size]` (12 bytes, all little-endian).
#[allow(dead_code)]
fn champ_header(datamap: u32, nodemap: u32, size: u32) -> Raw {
    // Build the 12-byte `[datamap][nodemap][size]` header directly as an INLINE `Raw` — no transient
    // heap Vec (CHAMP_HEADER_SIZE == INLINE_RAW_CAP, so it always inlines). This is the alloc-saving
    // win: every fresh CHAMP node (merge split, collision, path-copy rebuild, empty map/set) previously
    // allocated a 12-byte Vec here that `alloc` then copied inline and dropped.
    let mut buf = [0u8; INLINE_RAW_CAP];
    buf[0..4].copy_from_slice(&datamap.to_le_bytes());
    buf[4..8].copy_from_slice(&nodemap.to_le_bytes());
    buf[8..12].copy_from_slice(&size.to_le_bytes());
    Raw::Inline { len: CHAMP_HEADER_SIZE as u8, buf }
}

// ── Bitmap / slot arithmetic ──────────────────────────────────────────────────────────────

/// Number of inline entries in this node: popcount of the datamap.
#[allow(dead_code)]
fn data_count(datamap: u32) -> u32 {
    datamap.count_ones()
}
/// Number of child subnodes in this node: popcount of the nodemap.
#[allow(dead_code)]
fn subnode_count(nodemap: u32) -> u32 {
    nodemap.count_ones()
}
/// Position of slot `i`'s entry within the entry region: count of set datamap bits below `i`.
/// `i` is a 5-bit slot (0..=31), so `1 << i` never overflows.
#[allow(dead_code)]
fn entry_index_for_slot(datamap: u32, i: u32) -> u32 {
    (datamap & ((1u32 << i) - 1)).count_ones()
}
/// Position of slot `i`'s subnode within the subnode region: count of set nodemap bits below `i`.
#[allow(dead_code)]
fn subnode_index_for_slot(nodemap: u32, i: u32) -> u32 {
    (nodemap & ((1u32 << i) - 1)).count_ones()
}
/// The 5-bit trie index selected by `level` (0-based): hash bits [5*level, 5*level+5).
#[allow(dead_code)]
fn level_index(hash: u32, level: u32) -> u32 {
    (hash >> (VEC_BITS * level)) & VEC_MASK
}

// ── Node-kind discrimination (tag-free) ─────────────────────────────────────────────────

/// True iff `node` is the canonical EMPTY node: both bitmaps 0 AND no handles. This is the root of
/// an empty map/set. Kept disambiguated from a collision node (which also has both bitmaps 0).
#[allow(dead_code)]
fn is_empty_node(node: Handle) -> bool {
    with_node(node, true, |n| {
        champ_datamap(&n.raw) == 0 && champ_nodemap(&n.raw) == 0 && n.handles.is_empty()
    })
}
/// True iff `node` is a COLLISION node: both bitmaps 0 AND at least one handle. Holds entries that
/// share a full 32-bit hash; only occurs at maximum depth and is linear-scanned by structural eq.
#[allow(dead_code)]
fn is_collision_node(node: Handle) -> bool {
    with_node(node, false, |n| {
        champ_datamap(&n.raw) == 0 && champ_nodemap(&n.raw) == 0 && !n.handles.is_empty()
    })
}

// ── Structural hash (FNV-1a), ITERATIVE ─────────────────────────────────────────────────

/// FNV-1a over `h`'s OWN canonical raw bytes (no children), starting from the offset basis — the
/// per-node "leaf hash" both the arity-0 fast path and the general walk's Combine step fold. Folds
/// the SAME little-endian bytes a boxed twin carries, so an inline value hashes equal to its boxed
/// twin (open-Q#8): an immediate contributes unit → no bytes, bool → one byte, int → 8 LE bytes; a
/// real node contributes its `n.raw`; a null handle contributes nothing (bare offset basis). Unlike
/// the old `imm_canonical_raw`-based fold this allocates NO intermediate `Vec`.
#[inline]
fn champ_node_raw_hash(h: Handle) -> u32 {
    if is_immediate(h) {
        let mut acc = FNV_OFFSET;
        match imm_kind(h) {
            ImmKind::Unit => {}
            ImmKind::Bool => acc = fnv_step(acc, imm_as_bool(h) as u8),
            ImmKind::Int => {
                for b in (imm_as_int(h) as u64).to_le_bytes() {
                    acc = fnv_step(acc, b);
                }
            }
        }
        acc
    } else {
        with_node(h, FNV_OFFSET, |n| {
            let mut acc = FNV_OFFSET;
            for &b in n.raw.iter() {
                acc = fnv_step(acc, b);
            }
            acc
        })
    }
}

/// A deterministic structural hash of the whole subtree rooted at `root`: FNV-1a over each node's
/// raw bytes folded with its children's hashes. Because the rep is canonical, structurally-equal
/// subtrees hash equal; differing raw or structure (very likely) differs.
///
/// ITERATIVE, not recursive: a post-order walk over an explicit task worklist plus a results stack
/// (mirroring `op_drop`'s worklist discipline) keeps native/wasm stack use at O(1) frames regardless
/// of trie depth. Null handles fold as the empty (offset-basis) hash. Does NOT cache — v1 recomputes.
#[allow(dead_code)]
fn champ_hash(root: Handle) -> u32 {
    // Fast path — the hot map/set KEY case. An arity-0 node (an immediate, or a scalar/string/bytes
    // leaf) hashes to exactly FNV-1a over its own canonical raw bytes with NO child folds, which is
    // precisely what the general walk produces from a single Visit→Combine of a childless node. Take
    // it directly and allocate NEITHER the `work`/`results` worklists below NOR any canonical-raw Vec.
    if is_immediate(root) || with_node(root, 0usize, |n| n.handles.len()) == 0 {
        return champ_node_raw_hash(root);
    }
    // Shallow-compound fast path — a 1-level compound KEY (a small tuple/record whose children are all
    // arity-0 leaves/immediates; the common compound-key shape). Its hash is the node's own raw fold
    // followed by each child's `champ_node_raw_hash` (each child is arity-0, so its own hash needs no
    // recursion). The general walk folds children in the order they sit on `results`, which — because
    // children are pushed in index order then popped LIFO — is REVERSE index order; reproduce that here.
    // Allocates NOTHING (vs the two worklist Vecs below). Falls through to the iterative walk only for a
    // genuinely NESTED compound (a child that is itself a compound).
    if let Some(hash) = with_node(root, None, |n| {
        if n.handles.iter().all(|&c| is_immediate(c) || with_node(c, 0usize, |cn| cn.handles.len()) == 0) {
            let mut acc = FNV_OFFSET;
            for &b in n.raw.iter() {
                acc = fnv_step(acc, b);
            }
            for &c in n.handles.iter().rev() {
                for b in champ_node_raw_hash(c).to_le_bytes() {
                    acc = fnv_step(acc, b);
                }
            }
            Some(acc)
        } else {
            None // a nested child — use the general iterative walk
        }
    }) {
        return hash;
    }
    // Two-phase task: Visit expands a node's children; Combine folds this node's raw + the child
    // hashes now sitting on `results`. Children are pushed Visit-first so their Combine completes
    // before their parent's — a standard single-stack iterative post-order.
    enum Task {
        Visit(Handle),
        Combine(Handle, usize), // (node, arity — how many child hashes to consume)
    }
    let mut work: Vec<Task> = vec![Task::Visit(root)];
    let mut results: Vec<u32> = Vec::new();
    while let Some(task) = work.pop() {
        match task {
            Task::Visit(h) => {
                if is_immediate(h) {
                    // Inline value: arity 0, no children to expand. Combine folds its canonical raw.
                    work.push(Task::Combine(h, 0));
                    continue;
                }
                let arity = with_node(h, 0usize, |n| n.handles.len());
                work.push(Task::Combine(h, arity));
                with_node(h, (), |n| {
                    for &c in &n.handles {
                        work.push(Task::Visit(c));
                    }
                });
            }
            Task::Combine(h, arity) => {
                // Fold this node's own canonical raw bytes (the SAME LE bytes a boxed twin carries, so
                // an inline value hashes equal to its boxed twin, open-Q#8), then the child hashes.
                let mut s = champ_node_raw_hash(h);
                // Consume the `arity` child hashes on top of `results` (deterministic order).
                let start = results.len().saturating_sub(arity);
                for &child_hash in &results[start..] {
                    for b in child_hash.to_le_bytes() {
                        s = fnv_step(s, b);
                    }
                }
                results.truncate(start);
                results.push(s);
            }
        }
    }
    results.pop().unwrap_or(FNV_OFFSET)
}

// ── Structural eq, ITERATIVE ────────────────────────────────────────────────────────────

/// Structural equality of two subtrees: equal raw bytes AND equal child count AND recursively-equal
/// children. Only needed on a hash collision. Compares raw byte-for-byte, so floats (-0.0 ≠ 0.0),
/// bytes, and strings are handled with no special-casing. Null-safe: two nulls are equal, one null
/// and one non-null differ.
///
/// ITERATIVE via an explicit pair worklist; identical pointers (structural sharing) short-circuit.
/// The worklist is LAZILY allocated — the root pair is processed directly, and the `Vec` is created
/// only when a COMPOUND node actually needs its children pushed. The dominant map/set case (scalar or
/// immediate keys, arity 0) therefore allocates NOTHING (this is on the hot `op_map_lookup` /
/// `set-contains` / insert-overwrite-probe path, once per hash-collision key comparison).
#[allow(dead_code)]
fn champ_eq(a: Handle, b: Handle) -> bool {
    // Shallow-compound fast path — the hot compound-KEY compare (a small tuple/record key on a slot
    // hit): two equal-arity compounds whose children are ALL arity-0 are equal iff their raw bytes
    // match and every child pair is `with_raw_arity`-equal, WITHOUT allocating the worklist Vec below.
    // Only fires when NEITHER side is immediate and both are real nodes (the general path handles
    // immediates/nulls); a nested child (arity > 0) falls through to the lazy worklist.
    if !is_immediate(a) && !is_immediate(b) && a != b {
        if let Some(result) = unsafe {
            match (a.0.as_ref(), b.0.as_ref()) {
                (Some(na), Some(nb)) => {
                    if *na.raw != *nb.raw || na.handles.len() != nb.handles.len() {
                        Some(false) // roots differ ⇒ not equal, no descent
                    } else if na.handles.iter().chain(nb.handles.iter()).all(|&c| is_immediate(c) || c.0.as_ref().map(|cn| cn.handles.is_empty()).unwrap_or(true)) {
                        // Shallow: every child on both sides is arity-0 → compare pairwise inline.
                        let eq = na.handles.iter().zip(nb.handles.iter()).all(|(&cx, &cy)| {
                            cx == cy || with_raw_arity(cx, |rx, ax| with_raw_arity(cy, |ry, ay| rx == ry && ax == ay))
                        });
                        Some(eq)
                    } else {
                        None // a nested child — use the general worklist walk
                    }
                }
                _ => None, // a null side — general path
            }
        } {
            return result;
        }
    }
    let mut work: Option<Vec<(Handle, Handle)>> = None;
    let mut pair = Some((a, b));
    while let Some((x, y)) = pair {
        // Process (x, y); `descend` is set to the children to push when both are equal compounds.
        let mut descend: Option<(&Node, &Node)> = None;
        if x == y {
            // same pointer (incl. both NULL) ⇒ identical subtree, no descent needed
        } else if is_immediate(x) || is_immediate(y) {
            // An immediate's `.0` is NOT a Node pointer — compare by decoded value (arity 0, so
            // equality reduces to equal canonical raw bytes and equal arity), WITHOUT allocating.
            let equal = with_raw_arity(x, |rx, ax| with_raw_arity(y, |ry, ay| rx == ry && ax == ay));
            if !equal {
                return false;
            }
        } else {
            match (unsafe { x.0.as_ref() }, unsafe { y.0.as_ref() }) {
                (None, None) => {}
                (Some(nx), Some(ny)) => {
                    if *nx.raw != *ny.raw || nx.handles.len() != ny.handles.len() {
                        return false;
                    }
                    descend = Some((nx, ny)); // equal compound roots — push children below
                }
                _ => return false, // exactly one null ⇒ differ
            }
        }
        if let Some((nx, ny)) = descend {
            let w = work.get_or_insert_with(Vec::new);
            for i in 0..nx.handles.len() {
                w.push((nx.handles[i], ny.handles[i]));
            }
        }
        pair = work.as_mut().and_then(Vec::pop);
    }
    true
}

/// A deterministic, insertion-INDEPENDENT total order over key subtrees, used to canonicalize the
/// storage order of collision-node entries (keys that share a full 32-bit hash cannot be ordered by
/// hash, so we tiebreak purely on structure). Compares, in order: `raw` bytes lexicographically,
/// then `handles.len()`, then children index-by-index. A null handle orders BEFORE any non-null.
///
/// CONSISTENT with `champ_eq`: `champ_key_cmp(a, b) == Equal` IFF `champ_eq(a, b) == true` (both are
/// "no structural difference anywhere"). ITERATIVE via an explicit stack — a DFS pre-order with
/// children pushed reversed so index 0 is compared (and fully descended) before index 1, which makes
/// the first difference found the lexicographically-first one. No unbounded recursion (wasm-safe).
/// The worklist is LAZILY allocated (see `champ_eq`): the root pair is processed directly and the
/// `Vec` is created only when a compound needs children pushed, so ordering two scalar/immediate keys
/// (the common collision-canonicalization case) allocates nothing.
#[allow(dead_code)]
fn champ_key_cmp(a: Handle, b: Handle) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    // Shallow-compound fast path (mirrors `champ_eq`): order two compounds whose children are ALL
    // arity-0 by raw bytes, then arity, then children in INDEX order (the general walk descends index 0
    // first), each via `with_raw_arity` — WITHOUT the worklist Vec. Consistent with the shallow
    // `champ_eq` path (both reduce to the same per-child (raw, arity) compare). Nested ⇒ general walk.
    if !is_immediate(a) && !is_immediate(b) && a != b {
        if let Some(ord) = unsafe {
            match (a.0.as_ref(), b.0.as_ref()) {
                (Some(na), Some(nb)) => {
                    let shallow = na.handles.iter().chain(nb.handles.iter()).all(|&c| {
                        is_immediate(c) || c.0.as_ref().map(|cn| cn.handles.is_empty()).unwrap_or(true)
                    });
                    if !shallow {
                        None // a nested child — use the general worklist walk
                    } else {
                        let mut ord = na.raw.as_slice().cmp(nb.raw.as_slice()).then(na.handles.len().cmp(&nb.handles.len()));
                        let mut i = 0;
                        while ord == Ordering::Equal && i < na.handles.len() {
                            let (cx, cy) = (na.handles[i], nb.handles[i]);
                            if cx != cy {
                                ord = with_raw_arity(cx, |rx, ax| {
                                    with_raw_arity(cy, |ry, ay| rx.cmp(ry).then(ax.cmp(&ay)))
                                });
                            }
                            i += 1;
                        }
                        Some(ord)
                    }
                }
                _ => None, // a null side — general path
            }
        } {
            return ord;
        }
    }
    let mut work: Option<Vec<(Handle, Handle)>> = None;
    let mut pair = Some((a, b));
    while let Some((x, y)) = pair {
        let mut descend: Option<(&Node, &Node)> = None;
        if x == y {
            // same pointer (incl. both NULL) ⇒ identical so far
        } else if is_immediate(x) || is_immediate(y) {
            // Order by canonical (raw bytes, arity) WITHOUT allocating — the SAME (raw, arity) ordering
            // `champ_eq` compares (slice `cmp` is byte-lexicographic like `Vec` `cmp`), keeping the two
            // consistent (`cmp == Equal` iff `champ_eq`). An immediate has arity 0, no children.
            let ord = with_raw_arity(x, |rx, ax| with_raw_arity(y, |ry, ay| rx.cmp(ry).then(ax.cmp(&ay))));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            match (unsafe { x.0.as_ref() }, unsafe { y.0.as_ref() }) {
                (None, None) => {}                        // both null (unreachable given x==y, but total)
                (None, Some(_)) => return Ordering::Less, // null orders before non-null
                (Some(_), None) => return Ordering::Greater,
                (Some(nx), Some(ny)) => {
                    match nx.raw.as_slice().cmp(ny.raw.as_slice()) {
                        Ordering::Equal => {}
                        ord => return ord, // raw bytes lexicographically
                    }
                    match nx.handles.len().cmp(&ny.handles.len()) {
                        Ordering::Equal => {}
                        ord => return ord, // then arity
                    }
                    descend = Some((nx, ny)); // equal so far — descend children
                }
            }
        }
        if let Some((nx, ny)) = descend {
            // Descend children in index order: push reversed so index 0 pops (and fully descends) first.
            let w = work.get_or_insert_with(Vec::new);
            for i in (0..nx.handles.len()).rev() {
                w.push((nx.handles[i], ny.handles[i]));
            }
        }
        pair = work.as_mut().and_then(Vec::pop);
    }
    Ordering::Equal
}

// ─── CHAMP persistent MAP: empty / lookup / insert / size ───────────────────────────────
// Built on the U1 node core. Map stride = 2: an entry occupies two consecutive handles
// `[key, value]`. A node's handles are `[k0,v0,k1,v1,…]` (entries, ascending datamap-bit order)
// followed by `[sub0,sub1,…]` (subnodes, ascending nodemap-bit order). `size` (raw offset 8) is the
// total entry count of the subtree, giving O(1) `op_map_size` and a fast eq inequality gate.
//
// Descent is 5 bits/level over a 32-bit hash, so the trie is at most `CHAMP_LEVELS` deep. Keys that
// share their full 32-bit hash land in a COLLISION node (both bitmaps 0, handles nonempty) at the
// bottom and are linear-scanned by `champ_eq`. Because that depth is bounded by the hash WIDTH (not
// by data size), the insert split recursion is bounded to ≤7 frames — categorically unlike the
// free cascade, so recursion here is stack-safe. Lookup is an explicit iterative descent.
//
// A SHARED spine is path-copied (fully immutable/persistent): every node on the modified path is
// rebuilt and its retained children are `op_dup`'d, so structural sharing across versions is exact. A
// UNIQUELY-OWNED spine (`rc == 1`) instead refits in place via `champ_insert_fbip`'s `mine` gate — the
// FBIP fast path — since no other reference can observe the mutation.

/// Number of 5-bit trie levels over a 32-bit hash (levels 0..=6; level 6 consumes the top 2 bits).
/// At or beyond this depth the hash is exhausted and colliding keys share a collision node.
const CHAMP_LEVELS: u32 = 7;

/// Read a node's subtree size (raw offset 8). Borrows; a null/short node reads 0.
#[allow(dead_code)]
fn champ_size_of(node: Handle) -> u32 {
    with_node(node, 0, |n| champ_size(&n.raw))
}

/// The canonical empty map: both bitmaps 0, size 0, no handles (exactly `is_empty_node`). U3's
/// remove-to-empty MUST reproduce this representation so callers can recognise emptiness uniformly.
#[allow(dead_code)]
fn op_map_empty() -> Handle {
    alloc_raw(Vec::new(), champ_header(0, 0, 0))
}

/// O(1) entry count of the map. BORROWS `m` (no rc change).
#[allow(dead_code)]
fn op_map_size(m: Handle) -> u32 {
    champ_size_of(m)
}

/// Shared stride-aware membership descent: find `key`, returning `Some((deepest_node, base))` where
/// `base` is the entry's first-column index in that node's `handles`, or `None` on miss. BORROWS.
/// Iterative descent (a `while` loop, no recursion), mirroring `op_vec_get`. Map lookup reads the
/// value at `base+1`; set contains just checks presence.
#[allow(dead_code)]
fn champ_find_base(m: Handle, key: Handle, stride: usize) -> Option<(Handle, usize)> {
    champ_find_base_h(m, key, champ_hash(key), stride)
}

/// `champ_find_base` but with `key`'s hash PRECOMPUTED by the caller — so a caller that already needs
/// `champ_hash(key)` for a following insert (set-algebra ∩/∖: probe-then-insert) computes it ONCE
/// instead of paying a second full subtree hash walk (costly for string/compound keys). BORROWS.
#[allow(dead_code)]
fn champ_find_base_h(m: Handle, key: Handle, hash: u32, stride: usize) -> Option<(Handle, usize)> {
    enum Step {
        Hit(usize),
        Miss,
        Descend(Handle),
    }
    let mut node = m;
    let mut level = 0u32;
    loop {
        let step = with_node(node, Step::Miss, |n| {
            let datamap = champ_datamap(&n.raw);
            let nodemap = champ_nodemap(&n.raw);
            if datamap == 0 && nodemap == 0 {
                // Empty node ⇒ miss; collision node ⇒ linear scan of entries.
                let mut idx = 0;
                while idx < n.handles.len() {
                    if champ_eq(n.handles[idx], key) {
                        return Step::Hit(idx);
                    }
                    idx += stride;
                }
                return Step::Miss;
            }
            let i = level_index(hash, level);
            let bit = 1u32 << i;
            if datamap & bit != 0 {
                let base = stride * entry_index_for_slot(datamap, i) as usize;
                if champ_eq(n.handles[base], key) {
                    return Step::Hit(base);
                }
                return Step::Miss;
            }
            if nodemap & bit != 0 {
                let sidx = subnode_index_for_slot(nodemap, i) as usize;
                let sbase = stride * data_count(datamap) as usize;
                return Step::Descend(n.handles[sbase + sidx]);
            }
            Step::Miss
        });
        match step {
            Step::Hit(base) => return Some((node, base)),
            Step::Miss => return None,
            Step::Descend(child) => {
                node = child;
                level += 1;
            }
        }
    }
}

/// Look up `key`, returning its value handle or `Handle::NULL` on miss. BORROWS both `m` and `key`
/// (no rc change — the returned handle is borrowed too; the caller dups if it needs to retain it).
#[allow(dead_code)]
fn op_map_lookup(m: Handle, key: Handle) -> Handle {
    match champ_find_base(m, key, MAP_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base + 1),
        None => Handle::NULL,
    }
}

// ─── STRIDE-PARAMETERIZED insert core (shared by MAP stride 2 and SET stride 1) ─────────
// An "entry" is the incoming key/element columns for ONE insert; `entry.key()` (column 0) is the
// key/element compared by `champ_eq`. Map entries are `[key, value]` (len 2); set entries are `[elem]`
// (len 1). The generic OVERWRITE rule (key already present) keeps the STORED key and takes the incoming
// value columns `entry[1..]`, dropping the incoming duplicate `entry[0]`; the old stored value columns
// die with the consumed node. For a set (len 1) that degenerates to "keep stored, drop incoming
// element" — idempotent insert, size unchanged.
//
// `Entry` is an INLINE (stack) buffer of ≤2 handles, NOT a heap `Vec` — an insert is on the hot path
// (map/set insert + every set-algebra element), so allocating a Vec per insert was pure waste (the
// entry's handles are only ever MOVED into node storage, never shared). An entry holds Copy `Handle`s,
// so moving it touches no refcount — refcount-identical to the old `Vec<Handle>`. It is deliberately
// NOT `Copy`: the compiler's move-checking then still enforces that an entry (and each handle it owns)
// is consumed exactly once — the linearity discipline the old owned `Vec` gave us for free.

/// The incoming columns for one insert: `len` handles (1 for a set elem, 2 for a map `[k,v]`) in a
/// fixed inline `[Handle; 2]`. Move-only (no `Copy`/`Clone`) so single-consumption is compiler-checked.
struct Entry {
    cols: [Handle; 2],
    len: usize,
}
impl Entry {
    /// A set element (len 1).
    fn elem(e: Handle) -> Entry {
        Entry { cols: [e, Handle::NULL], len: 1 }
    }
    /// A map key/value pair (len 2).
    fn kv(k: Handle, v: Handle) -> Entry {
        Entry { cols: [k, v], len: 2 }
    }
    /// The key/element column (column 0), compared by `champ_eq`.
    fn key(&self) -> Handle {
        self.cols[0]
    }
    /// Column `i` (0 ≤ i < len).
    fn col(&self, i: usize) -> Handle {
        self.cols[i]
    }
    /// The number of columns (the insert stride: 1 for a set, 2 for a map).
    fn len(&self) -> usize {
        self.len
    }
    /// Consume the entry, appending its columns onto `out` — used where the columns become part of a
    /// freshly-built node handle vector (the rare fresh-single / split / collision-append paths).
    fn extend_into(self, out: &mut Vec<Handle>) {
        out.extend_from_slice(&self.cols[..self.len]);
    }
    /// Consume the entry into a fresh 1- or 2-element `Vec` (only where a node's `handles` IS exactly
    /// this entry — the fresh-single-entry node).
    fn into_vec(self) -> Vec<Handle> {
        self.cols[..self.len].to_vec()
    }
    /// The entry's columns as a slice — for a caller that copies them into node storage BY VALUE
    /// (handles are `Copy`, so this relocates them without dup/drop, exactly like moving the old Vec's
    /// elements). The caller is responsible for consuming the entry exactly once overall.
    fn cols(&self) -> &[Handle] {
        &self.cols[..self.len]
    }
}

/// Append `entry`'s columns onto `out` by value (handles are `Copy`). Used at the collision-node
/// splice site, which conditionally splices at one of two positions; the caller `drop`s the entry once
/// after the single splice actually runs, so this borrows rather than consumes.
fn entry_splice(out: &mut Vec<Handle>, entry: &Entry) {
    out.extend_from_slice(entry.cols());
}

/// Build an `Entry` from the `stride` columns of `handles` starting at `base` — the STORED entry a
/// SPLIT folds together with the newcomer. `dup` ⇒ retain a reference to each column (the copy path,
/// where the consumed node still owns its copy); `!dup` ⇒ relocate the columns (the FBIP path, where
/// the node's handle vector was already taken and these references move out).
fn stored_entry_from(handles: &[Handle], base: usize, stride: usize, dup: bool) -> Entry {
    let mut cols = [Handle::NULL, Handle::NULL];
    for t in 0..stride {
        let h = handles[base + t];
        if dup {
            op_dup(h);
        }
        cols[t] = h;
    }
    Entry { cols, len: stride }
}

/// The trie-slot indices of a cursor's descent path — a stack of `u32`, one per focused frame. The
/// depth is HARD-BOUNDED by the trie: at most `CHAMP_LEVELS` (7) normal levels plus one collision
/// frame at the hash floor, so the stack never exceeds `SLOTS_CAP` entries. Storing it inline in a
/// fixed `[u32; SLOTS_CAP]` (instead of a `Vec`) removes the per-advance-step slots allocation on the
/// hot iteration path — `slots.len() == frames.len()`, so the two grow/shrink in lockstep. Overflow is
/// a compiler-invariant violation (a rope deeper than the hash allows) and TRAPS, never silently wraps.
const SLOTS_CAP: usize = CHAMP_LEVELS as usize + 2; // 7 normal levels + collision frame + margin
struct Slots {
    buf: [u32; SLOTS_CAP],
    len: usize,
}
impl Slots {
    fn new() -> Slots {
        Slots { buf: [0; SLOTS_CAP], len: 0 }
    }
    fn len(&self) -> usize {
        self.len
    }
    fn push(&mut self, v: u32) {
        if self.len >= SLOTS_CAP {
            trap_oob(); // cursor deeper than the trie permits — a compiler-invariant violation
        }
        self.buf[self.len] = v;
        self.len += 1;
    }
    fn pop(&mut self) {
        // Mirrors `Vec::pop`'s use here (the return value is never read) — just shrink.
        if self.len > 0 {
            self.len -= 1;
        }
    }
    /// The slot values in push order — for encoding into a cursor's raw header.
    fn as_slice(&self) -> &[u32] {
        &self.buf[..self.len]
    }
}
impl core::ops::Index<usize> for Slots {
    type Output = u32;
    fn index(&self, i: usize) -> &u32 {
        &self.buf[..self.len][i]
    }
}
impl core::ops::IndexMut<usize> for Slots {
    fn index_mut(&mut self, i: usize) -> &mut u32 {
        &mut self.buf[..self.len][i]
    }
}

/// Build a node holding exactly two entries whose keys hash to `h1`/`h2`, splitting by the trie
/// index at `level` and recursing while they collide; at the hash floor they share a collision node.
/// CONSUMES both entries (their handles become the new node's). Bounded recursion (≤7).
#[allow(dead_code)]
fn merge_two_entries(e1: Entry, h1: u32, e2: Entry, h2: u32, level: u32) -> Handle {
    if level >= CHAMP_LEVELS {
        // Hash exhausted: a collision node holding both entries, stored in canonical KEY order
        // (insertion-independent) so equal collision sets are byte-identical regardless of order.
        let (first, second) = if champ_key_cmp(e1.key(), e2.key()) == core::cmp::Ordering::Greater {
            (e2, e1)
        } else {
            (e1, e2)
        };
        let mut hs = Vec::with_capacity(first.len() + second.len());
        first.extend_into(&mut hs);
        second.extend_into(&mut hs);
        return alloc_raw(hs, champ_header(0, 0, 2));
    }
    let i1 = level_index(h1, level);
    let i2 = level_index(h2, level);
    if i1 == i2 {
        // Same slot: nest one subnode a level deeper.
        let sub = merge_two_entries(e1, h1, e2, h2, level + 1);
        alloc_raw(vec![sub], champ_header(0, 1 << i1, 2))
    } else if i1 < i2 {
        let mut hs = Vec::with_capacity(e1.len() + e2.len());
        e1.extend_into(&mut hs);
        e2.extend_into(&mut hs);
        alloc_raw(hs, champ_header((1 << i1) | (1 << i2), 0, 2))
    } else {
        let mut hs = Vec::with_capacity(e1.len() + e2.len());
        e2.extend_into(&mut hs);
        e1.extend_into(&mut hs);
        alloc_raw(hs, champ_header((1 << i2) | (1 << i1), 0, 2))
    }
}

/// Insert `entry` into a collision node (both bitmaps 0, `handles` nonempty). CONSUMES `node` and
/// `entry`. Overwrite (key present) keeps the stored key + takes incoming value columns, dropping
/// the incoming duplicate key; otherwise the entry is appended. Path-copied.
#[allow(dead_code)]
fn collision_insert(node: Handle, handles: Vec<Handle>, entry: Entry, stride: usize) -> Handle {
    let key = entry.key();
    let mut found = None;
    let mut idx = 0;
    while idx < handles.len() {
        if champ_eq(handles[idx], key) {
            found = Some(idx);
            break;
        }
        idx += stride;
    }
    match found {
        Some(j) => {
            let mut new_handles = Vec::with_capacity(handles.len());
            for (i2, &h) in handles.iter().enumerate() {
                if i2 > j && i2 < j + stride {
                    new_handles.push(entry.col(i2 - j)); // incoming value column
                } else {
                    op_dup(h);
                    new_handles.push(h);
                }
            }
            let entries = (handles.len() / stride) as u32;
            let new = alloc_raw(new_handles, champ_header(0, 0, entries));
            op_drop(entry.key()); // incoming duplicate key unused
            op_drop(node);
            new
        }
        None => {
            // New key: splice the incoming entry at the position that keeps entries sorted by KEY
            // (insertion-independent canonical order). Collision nodes are tiny, so a linear scan is
            // fine. Pure reordering of handles we already own — no extra dup/drop.
            let mut pos = handles.len(); // default: after all existing entries
            let mut idx = 0;
            while idx < handles.len() {
                if champ_key_cmp(key, handles[idx]) == core::cmp::Ordering::Less {
                    pos = idx;
                    break;
                }
                idx += stride;
            }
            let mut new_handles = Vec::with_capacity(handles.len() + stride);
            for (i2, &h) in handles.iter().enumerate() {
                if i2 == pos {
                    entry_splice(&mut new_handles, &entry); // owned incoming entry, in place
                }
                op_dup(h);
                new_handles.push(h);
            }
            if pos == handles.len() {
                entry_splice(&mut new_handles, &entry); // sorts last
            }
            let _ = entry; // consumed: its columns were copied into new_handles above (once, by `pos`)
            let entries = (handles.len() / stride + 1) as u32;
            let new = alloc_raw(new_handles, champ_header(0, 0, entries));
            op_drop(node);
            new
        }
    }
}

/// The per-node recursive insert core. CONSUMES `node` and `entry` (`entry[0]` = key, len = stride);
/// returns the new node. Bounded recursion (≤ `CHAMP_LEVELS`). Always path-copies.
#[allow(dead_code)]
fn champ_insert_node(node: Handle, entry: Entry, hash: u32, level: u32, stride: usize) -> Handle {
    let key = entry.key();
    // Read only the HEADER + arity upfront — NOT a clone of `handles`. The old code cloned the whole
    // handle vector here even on the SPLIT/EMPTY/collision branches that only ever READ it by index and
    // build a fresh, differently-sized result — a wasted Vec alloc + O(arity) copy on every path-copied
    // node. Now the OVERWRITE/DESCEND branches (which REUSE a full-length copy as their result) clone at
    // their own branch, and the growth branches read via a borrow. `arity` gates the empty/collision test.
    let (datamap, nodemap, size, arity) = with_node(
        node,
        (0u32, 0u32, 0u32, 0usize),
        |n| {
            (
                champ_datamap(&n.raw),
                champ_nodemap(&n.raw),
                champ_size(&n.raw),
                n.handles.len(),
            )
        },
    );

    // Empty node (fresh single entry) or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            let i = level_index(hash, level);
            let new = alloc_raw(entry.into_vec(), champ_header(1 << i, 0, 1)); // entry owned
            op_drop(node);
            return new;
        }
        // Collision node — needs an owned copy of the entries (the helper appends + rebuilds).
        let handles = with_node(node, Vec::new(), |n| n.handles.clone());
        return collision_insert(node, handles, entry, stride);
    }

    let dcount = data_count(datamap) as usize;
    let scount = subnode_count(nodemap) as usize;
    let subbase = stride * dcount;
    let i = level_index(hash, level);
    let bit = 1u32 << i;

    if datamap & bit != 0 {
        let eidx = entry_index_for_slot(datamap, i) as usize;
        let base = stride * eidx;
        let stored_key = champ_handle_at(node, base);
        if champ_eq(stored_key, key) {
            // OVERWRITE: keep the stored key, swap in the incoming value columns; size/arity unchanged.
            // Clone the handle vector ONCE and use it AS the result (mutate in place) rather than reading
            // one vector and building a second — the new node needs a full-length handles Vec anyway, so
            // the clone IS that vector. Each KEPT handle is dup'd so `new` owns its own reference; the
            // OLD value columns (overwritten below) are released by op_drop(node).
            let mut new_handles = with_node(node, Vec::new(), |n| n.handles.clone());
            for (idx, slot) in new_handles.iter_mut().enumerate() {
                if idx > base && idx < base + stride {
                    *slot = entry.col(idx - base); // incoming value column (owned); old ptr still on node
                } else {
                    op_dup(*slot); // kept handle: new node needs its own reference
                }
            }
            let new = alloc_raw(new_handles, champ_header(datamap, nodemap, size));
            op_drop(entry.key()); // incoming duplicate key unused
            op_drop(node); // frees the old value columns (and balances the dups) if node was unique
            return new;
        }
        // SPLIT: turn the inline entry + newcomer into a subnode. Build the STORED entry (dup'd, since
        // the consumed node's copy also survives via `op_drop(node)` releasing only ITS references).
        // Read the node's handles by BORROW (`champ_handle_at`) — no upfront clone — since the result is
        // a fresh, differently-shaped Vec (one entry removed, one subnode added).
        let stored_entry = with_node(node, Entry::elem(Handle::NULL), |n| {
            stored_entry_from(&n.handles, base, stride, true)
        });
        let h1 = champ_hash(stored_key);
        let sub = merge_two_entries(stored_entry, h1, entry, hash, level + 1);
        let new_datamap = datamap & !bit;
        let new_nodemap = nodemap | bit;
        let new_sidx = subnode_index_for_slot(new_nodemap, i) as usize;
        // Build the fresh result vector directly from a borrow of the node's handles.
        let new_handles = with_node(node, Vec::new(), |n| {
            let mut nh = Vec::with_capacity(n.handles.len() + 1);
            for e in 0..dcount {
                if e == eidx {
                    continue;
                }
                for t in 0..stride {
                    let h = n.handles[stride * e + t];
                    op_dup(h);
                    nh.push(h);
                }
            }
            let mut subs: Vec<Handle> = Vec::with_capacity(scount + 1);
            for s in 0..scount {
                let c = n.handles[subbase + s];
                op_dup(c);
                subs.push(c);
            }
            subs.insert(new_sidx, sub);
            nh.extend(subs);
            nh
        });
        let new = alloc_raw(new_handles, champ_header(new_datamap, new_nodemap, size + 1));
        op_drop(node);
        return new;
    }

    if nodemap & bit != 0 {
        // DESCEND into the subnode. Arity is unchanged, so the result reuses a full-length copy of the
        // node's handles — clone ONCE here (at the branch that needs it) and mutate the one child slot.
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = champ_handle_at(node, subbase + sidx);
        let old_child_size = champ_size_of(child);
        op_dup(child);
        let new_child = champ_insert_node(child, entry, hash, level + 1, stride);
        let delta = champ_size_of(new_child) - old_child_size; // 0 (overwrite) or 1 (new key)
        // Swap the one child slot to `new_child` (the recursion already consumed the old `child` ref via
        // the op_dup above) and dup each KEPT handle so `new` owns its own references.
        let mut new_handles = with_node(node, Vec::new(), |n| n.handles.clone());
        for (idx, slot) in new_handles.iter_mut().enumerate() {
            if idx == subbase + sidx {
                *slot = new_child; // owned; old child ref was consumed by the recursion
            } else {
                op_dup(*slot); // kept handle: new node needs its own reference
            }
        }
        let new = alloc_raw(new_handles, champ_header(datamap, nodemap, size + delta));
        op_drop(node);
        return new;
    }

    // EMPTY slot: place a new inline entry in canonical (ascending-bit) order. Fresh, larger result —
    // read the node's handles by BORROW (no upfront clone).
    let new_datamap = datamap | bit;
    let new_eidx = entry_index_for_slot(new_datamap, i) as usize;
    let mut new_handles = with_node(node, Vec::new(), |n| {
        let mut nh: Vec<Handle> = Vec::with_capacity(n.handles.len() + stride);
        for e in 0..dcount {
            for t in 0..stride {
                let h = n.handles[stride * e + t];
                op_dup(h);
                nh.push(h);
            }
        }
        nh
    });
    for (off, h) in entry.cols().iter().enumerate() {
        new_handles.insert(stride * new_eidx + off, *h); // entry columns at the entry position
    }
    let _ = entry; // consumed: columns relocated into new_handles by value (handles are Copy)
    with_node(node, (), |n| {
        for s in 0..scount {
            let c = n.handles[subbase + s];
            op_dup(c);
            new_handles.push(c);
        }
    });
    let new = alloc_raw(new_handles, champ_header(new_datamap, nodemap, size + 1));
    op_drop(node);
    new
}

// ─── FBIP (Functional But In-Place) rc==1 shell reuse for CHAMP insert/remove (U5) ────────────
// When the touched CHAMP spine is UNIQUELY owned we REUSE each node's shell (mutate its handles/raw
// in place) instead of alloc-new + drop-old. Observationally IDENTICAL to the path-copy core, and
// canonical-shape-identical (the in-place builders mirror the copy path's ordering byte-for-byte).
//
// 🚨 ALIASING SAFETY (a violation silently corrupts a shared persistent map/set). `mine` = "this node
// is on a fully-unique path, rc==1, safe to reuse in place". It propagates STRICTLY monotone-false
// downward: the map/set handle IS the root node (no separate header like the vector), so the ops gate
// on `node_rc(m)==1`, then `child_mine = mine && node_rc(child)==1`. At the FIRST node with rc>1 we
// delegate to the proven copy path (`champ_insert_node`/`champ_remove_node`) for that node and its
// whole subtree; the consume-and-return contract means a delegated child consumes exactly the one
// reference passed and returns the handle to place — no pre-dup / post-drop bookkeeping at the boundary
// (the same pattern as `vec_push_fbip`). A subnode with rc==1 reached THROUGH a shared ancestor is
// still shared, which is why `mine` never turns back true once false.

/// In-place shell reuse for a uniquely-owned (`rc == 1`) CHAMP `node`: install `handles` and WRITE the
/// 3-u32 header (`[datamap][nodemap][size]`) into the node's EXISTING `raw` buffer in place, rather
/// than taking a freshly-allocated `champ_header` Vec — saving a 12-byte Vec allocation per rebuilt
/// spine node (the profiler's dominant FBIP-insert/remove cost). Carried children were moved into
/// `handles` WITHOUT a dup (each single owned reference relocates); the caller must have already
/// `op_drop`ed any REMOVED children. Dropping the old `handles` Vec changes no refcount (`Handle` is a
/// `Copy` pointer with no `Drop`), so a carried child in BOTH the old and new Vec keeps its rc intact.
/// A CHAMP node's `raw` is always exactly `CHAMP_HEADER_SIZE`, so the header write never grows/reallocs;
/// a defensively short/absent `raw` is resized to fit. SAFETY: caller verified `node_rc(node) == 1`.
fn champ_become_hdr(node: Handle, handles: Vec<Handle>, datamap: u32, nodemap: u32, size: u32) -> Handle {
    if let Some(n) = unsafe { node.0.as_mut() } {
        n.handles = handles;
        if n.raw.len() != CHAMP_HEADER_SIZE {
            n.raw.resize(CHAMP_HEADER_SIZE, 0); // defensive; a real CHAMP node is already 12 bytes
        }
        let r = n.raw.as_mut_slice();
        r[0..4].copy_from_slice(&datamap.to_le_bytes());
        r[4..8].copy_from_slice(&nodemap.to_le_bytes());
        r[8..12].copy_from_slice(&size.to_le_bytes());
    }
    node
}

/// `mem::take` a uniquely-owned (`rc == 1`) node's handle vector out (a pointer swap, no clone),
/// leaving it transiently empty. The caller reinstalls a handle vector via `champ_become_hdr` before
/// returning, so no other reference observes the empty state (single-threaded). Moving a `Handle` out
/// touches no refcount (`Copy`, no `Drop`). SAFETY: caller verified `node_rc(node) == 1`.
fn champ_take_handles(node: Handle) -> Vec<Handle> {
    match unsafe { node.0.as_mut() } {
        Some(n) => std::mem::take(&mut n.handles),
        None => Vec::new(),
    }
}

/// Write a single child slot AND patch the `size` header field of a uniquely-owned (`rc == 1`) CHAMP
/// node IN PLACE — the zero-allocation path for a remove whose subnode kept its arity (only one child
/// pointer changes and the subtree count drops by one; datamap/nodemap are unchanged). SAFETY: caller
/// verified `node_rc(node) == 1` and `slot < handles.len()`, `raw.len() == CHAMP_HEADER_SIZE`.
fn champ_set_child_and_size_inplace(node: Handle, slot: usize, child: Handle, size: u32) {
    if let Some(n) = unsafe { node.0.as_mut() } {
        if let Some(h) = n.handles.get_mut(slot) {
            *h = child;
        }
        if n.raw.len() >= CHAMP_HEADER_SIZE {
            n.raw.as_mut_slice()[8..12].copy_from_slice(&size.to_le_bytes());
        }
    }
}

/// FBIP variant of `champ_insert_node`. `mine` ⇒ reuse `node`'s shell in place; `!mine` ⇒ delegate to
/// the path-copying `champ_insert_node` verbatim. Bounded recursion (≤ `CHAMP_LEVELS`). CONSUMES
/// `node` and `entry` (`entry[0]` = key, len = `stride`). See the safety note above.
fn champ_insert_fbip(
    node: Handle,
    entry: Entry,
    hash: u32,
    level: u32,
    stride: usize,
    mine: bool,
) -> Handle {
    if !mine {
        return champ_insert_node(node, entry, hash, level, stride); // shared: proven copy path
    }
    let key = entry.key();
    // Read the header + arity WITHOUT cloning `handles` (see the take below).
    let (datamap, nodemap, size, arity) =
        with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
            (
                champ_datamap(&n.raw),
                champ_nodemap(&n.raw),
                champ_size(&n.raw),
                n.handles.len(),
            )
        });

    // Empty (fresh single entry) or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            let i = level_index(hash, level);
            return champ_become_hdr(node, entry.into_vec(), 1 << i, 0, 1); // entry owned, moved in
        }
        // Collision node (full 32-bit hash clash — rare): path-copy via the proven helper, which
        // `op_drop`s `node` and so needs its child references intact — clone rather than take here.
        let handles = with_node(node, Vec::new(), |n| n.handles.clone());
        return collision_insert(node, handles, entry, stride);
    }

    // Normal (bitmap) node on a UNIQUE spine: TAKE its handle vector instead of cloning it. `node` is
    // `rc == 1` (the `mine` gate + monotone-false descent), so no other reference exists; the take is
    // a pointer swap (zero alloc, vs the clone's O(arity) copy on every spine node, every level). Every
    // path below rebuilds a fresh `new_handles` and `champ_become_hdr(node, …)` REINSTALLS it before this
    // function returns, so `node` is never observed in the transient empty state (single-threaded).
    // `mut` because the arity-preserving branches (OVERWRITE, DESCEND) mutate a slot in place and
    // reinstall this same vector rather than allocating a fresh one.
    let mut handles = match unsafe { node.0.as_mut() } {
        Some(n) => std::mem::take(&mut n.handles),
        None => Vec::new(),
    };

    let dcount = data_count(datamap) as usize;
    let subbase = stride * dcount; // subnodes follow the `dcount` entries; the SPLIT/EMPTY paths now
    let i = level_index(hash, level); // rebuild in place, so `scount` is no longer needed here
    let bit = 1u32 << i;

    if datamap & bit != 0 {
        let eidx = entry_index_for_slot(datamap, i) as usize;
        let base = stride * eidx;
        let stored_key = handles[base];
        if champ_eq(stored_key, key) {
            // OVERWRITE: keep stored key, take the incoming value columns, drop the old ones. Size same,
            // arity unchanged — so mutate the taken `handles` IN PLACE (swap the value columns) and
            // reinstall it, rather than allocating a fresh `new_handles` (saves one Vec per spine node).
            for t in 1..stride {
                op_drop(handles[base + t]); // old value column, replaced
                handles[base + t] = entry.col(t); // incoming value column (owned)
            }
            op_drop(entry.key()); // incoming duplicate key unused
            return champ_become_hdr(node, handles, datamap, nodemap, size);
        }
        // SPLIT: fold the inline entry + newcomer into a subnode. MOVE the stored entry (no dup — the
        // node's handle vec was already taken, so these references relocate) into the merge; carry the
        // other entries/subnodes without dup.
        let stored_entry = stored_entry_from(&handles, base, stride, false);
        let h1 = champ_hash(stored_key);
        let sub = merge_two_entries(stored_entry, h1, entry, hash, level + 1);
        let new_datamap = datamap & !bit;
        let new_nodemap = nodemap | bit;
        let new_sidx = subnode_index_for_slot(new_nodemap, i) as usize;
        // Transform the taken `handles` IN PLACE rather than building a fresh `new_handles` + `subs`:
        // (1) DRAIN the split entry's `stride` columns — their handles have already relocated into
        //     `sub` (via `stored_entry_from` + `merge_two_entries`), so DO NOT op_drop them; the drain
        //     just removes the now-duplicate pointers and shifts the tail (remaining entries+subnodes)
        //     left by `stride`. (2) INSERT `sub` at its canonical subnode position: after the drain the
        //     entry region holds `dcount-1` entries, so subnodes start at `stride*(dcount-1)`, and `sub`
        //     goes at `+ new_sidx`. The only allocation left is `sub` itself (intrinsic — a new node).
        handles.drain(base..base + stride);
        handles.insert(stride * (dcount - 1) + new_sidx, sub);
        return champ_become_hdr(node, handles, new_datamap, new_nodemap, size + 1);
    }

    if nodemap & bit != 0 {
        // DESCEND. Read the child's size BEFORE recursing (it may be mutated in place), and its rc to
        // decide `child_mine`. The recursion consumes the one reference we pass and returns the handle.
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = handles[subbase + sidx];
        let old_child_size = champ_size_of(child);
        let child_mine = node_rc(child) == 1;
        let new_child = champ_insert_fbip(child, entry, hash, level + 1, stride, child_mine);
        let delta = champ_size_of(new_child) - old_child_size; // 0 (overwrite) or 1 (new key)
        // Arity unchanged — swap the one child slot in the taken `handles` IN PLACE and reinstall,
        // rather than rebuilding a fresh Vec (saves one alloc per descended level, the common path).
        // The recursion CONSUMED `child` (the reference at this slot); writing `new_child` here is a
        // no-op when it reused the shell (`new_child == child`) and installs the fresh node otherwise.
        handles[subbase + sidx] = new_child;
        return champ_become_hdr(node, handles, datamap, nodemap, size + delta);
    }

    // EMPTY slot: place a new inline entry in canonical (ascending-bit) order. The entry region sits
    // BEFORE the subnodes (`subbase = stride*dcount`) and `new_eidx ≤ dcount`, so splicing the entry's
    // `stride` columns into the taken `handles` at `stride*new_eidx` lands them among the entries and
    // shifts the subnodes right — exactly the canonical layout. Reuse the taken vector (rc==1) rather
    // than building a fresh one: `Vec::insert` may grow it once, but there is no separate full-copy pass.
    let new_datamap = datamap | bit;
    let new_eidx = entry_index_for_slot(new_datamap, i) as usize;
    for (off, h) in entry.cols().iter().enumerate() {
        handles.insert(stride * new_eidx + off, *h); // incoming entry column (owned), no dup
    }
    let _ = entry; // consumed: columns relocated into `handles` by value (handles are Copy)
    champ_become_hdr(node, handles, new_datamap, nodemap, size + 1)
}

/// Insert `key => val`, returning the new map. CONSUMES `m`, `key`, `val`. Inserting an existing key
/// overwrites its value (size unchanged); a new key increments size. Persistent: to keep the old
/// map, `op_dup` it before inserting. FBIP: when `m` is uniquely owned (`rc == 1`) the touched spine
/// is refit in place; a shared map (`rc > 1`) path-copies (the old version stays byte-identical).
#[allow(dead_code)]
fn op_map_insert(m: Handle, key: Handle, val: Handle) -> Handle {
    let hash = champ_hash(key);
    let mine = node_rc(m) == 1;
    champ_insert_fbip(m, Entry::kv(key, val), hash, 0, MAP_STRIDE, mine)
}

// ─── CHAMP persistent MAP: remove (the exact inverse of insert) ─────────────────────────
// Remove is the mirror of insert and MUST preserve canonicality so two maps with equal contents
// stay byte-identically shaped (equal `champ_eq`/`champ_hash`) regardless of insert/remove history.
// The load-bearing inverses of the split:
//   • Removing an inline entry clears its datamap bit and drops `[k,v]`.
//   • A subnode that reduces to a SINGLE inline entry (and no subnodes) is COLLAPSED back into the
//     parent as an inline entry (inverse of `merge_two_entries`). A single-entry collision node
//     collapses the same way. This cascades: after inlining a child, the parent may itself become a
//     single-entry collapse candidate for ITS parent — handled naturally by each level re-checking
//     its child result on the bounded post-order return path (≤ `CHAMP_LEVELS` frames, stack-safe).
//   • Removing the map's final entry yields EXACTLY the canonical empty (`op_map_empty()` shape).
// A subnode is only ever collapsed when it holds a single ENTRY — a subnode of ≥2 entries can't be
// inlined and stays put, matching what insert would have produced. Like insert, a uniquely-owned
// spine refits in place (`champ_remove_fbip`'s `mine` gate); a shared spine path-copies.
//
// NOTE: the ROOT is never collapsed to an inline entry — a one-entry map is a root node with one
// inline entry (datamap has one bit), which is exactly what insert produces. Collapse only lifts a
// child INTO its parent; the top-level `op_map_remove` never inlines the root into anything.

/// If `node` holds exactly one entry and no subnodes (a single inline entry, or a single-entry
/// collision node), return that entry's `stride` handles (BORROWED — no rc change) so the parent can
/// inline it. Otherwise `None`. Root nodes are never passed here.
#[allow(dead_code)]
fn collapse_candidate(node: Handle, stride: usize) -> Option<Entry> {
    with_node(node, None, |n| {
        let dm = champ_datamap(&n.raw);
        let nm = champ_nodemap(&n.raw);
        if nm != 0 {
            return None; // has subnodes — cannot inline
        }
        let entries = if dm != 0 {
            data_count(dm) as usize
        } else {
            n.handles.len() / stride // collision (or empty) node
        };
        if entries == 1 && n.handles.len() >= stride {
            // The single entry's `stride` columns as an inline `Entry` (≤2 handles) — no heap Vec.
            let mut cols = [Handle::NULL, Handle::NULL];
            cols[..stride].copy_from_slice(&n.handles[0..stride]);
            Some(Entry { cols, len: stride })
        } else {
            None
        }
    })
}

/// The per-node recursive remove core. CONSUMES `node` (the input ref moves through to the returned
/// handle), BORROWS `key`. Returns `(new_node, removed)`; when `removed` is false the returned handle
/// is the unchanged input. Bounded recursion (≤ `CHAMP_LEVELS`). Always path-copies.
#[allow(dead_code)]
fn champ_remove_node(node: Handle, key: Handle, hash: u32, level: u32, stride: usize) -> (Handle, bool) {
    // Read only the HEADER + arity upfront — NOT a clone of `handles`. The old code cloned the whole
    // handle vector here even on the common ABSENT-key early-returns (`(node, false)`) and on the
    // fresh-shorter-result branches (found-entry drop, collapse) that only READ it by index — a wasted
    // Vec alloc + O(arity) copy on every path-copied node. Branches that reuse a full-length copy as the
    // result (DESCEND non-collapse) clone at their own branch; the rest borrow-and-build / return early.
    let (datamap, nodemap, size, arity) = with_node(
        node,
        (0u32, 0u32, 0u32, 0usize),
        |n| {
            (
                champ_datamap(&n.raw),
                champ_nodemap(&n.raw),
                champ_size(&n.raw),
                n.handles.len(),
            )
        },
    );

    // Empty node or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            return (node, false); // empty — absent
        }
        // Collision node: linear scan of entries (BORROW — no upfront clone; a miss returns unchanged).
        let found = with_node(node, None, |n| {
            let mut idx = 0;
            while idx < n.handles.len() {
                if champ_eq(n.handles[idx], key) {
                    return Some(idx);
                }
                idx += stride;
            }
            None
        });
        let j = match found {
            Some(j) => j,
            None => return (node, false), // absent — unchanged
        };
        let entries_after = (arity / stride - 1) as u32;
        // Fresh shorter result — build directly from a borrow of the node's handles.
        let new_handles = with_node(node, Vec::new(), |n| {
            let mut nh = Vec::with_capacity(n.handles.len() - stride);
            for (i2, &h) in n.handles.iter().enumerate() {
                if i2 >= j && i2 < j + stride {
                    continue; // removed entry columns: NOT dup'd, freed by op_drop(node) below
                }
                op_dup(h);
                nh.push(h);
            }
            nh
        });
        let new = alloc_raw(new_handles, champ_header(0, 0, entries_after));
        op_drop(node);
        return (new, true);
    }

    let dcount = data_count(datamap) as usize;
    let scount = subnode_count(nodemap) as usize;
    let subbase = stride * dcount;
    let i = level_index(hash, level);
    let bit = 1u32 << i;

    if datamap & bit != 0 {
        // Inline entry at this slot: present only if the stored key equals `key`.
        let eidx = entry_index_for_slot(datamap, i) as usize;
        if !champ_eq(champ_handle_at(node, stride * eidx), key) {
            return (node, false); // different key occupies the slot — absent
        }
        let new_datamap = datamap & !bit;
        // Fresh shorter result (one entry dropped) — build directly from a borrow of the node's handles.
        let new_handles = with_node(node, Vec::new(), |n| {
            let mut nh = Vec::with_capacity(n.handles.len() - stride);
            for e in 0..dcount {
                if e == eidx {
                    continue; // removed entry columns: not dup'd, freed by op_drop(node)
                }
                for t in 0..stride {
                    let h = n.handles[stride * e + t];
                    op_dup(h);
                    nh.push(h);
                }
            }
            for s in 0..scount {
                let c = n.handles[subbase + s];
                op_dup(c);
                nh.push(c);
            }
            nh
        });
        let new = alloc_raw(new_handles, champ_header(new_datamap, nodemap, size - 1));
        op_drop(node);
        return (new, true);
    }

    if nodemap & bit != 0 {
        // Descend into the subnode.
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = champ_handle_at(node, subbase + sidx);
        op_dup(child);
        let (new_child, removed) = champ_remove_node(child, key, hash, level + 1, stride);
        if !removed {
            op_drop(new_child); // unchanged (== child): undo the dup
            return (node, false);
        }
        if let Some(centry) = collapse_candidate(new_child, stride) {
            // COLLAPSE: inline the child's single entry into this node at slot i. Fresh reshaped result
            // (one subnode → one entry) — build directly from a borrow of the node's handles.
            for &h in centry.cols() {
                op_dup(h);
            }
            op_drop(new_child); // frees the collapsed child wrapper; entry cols survive via our dups
            let new_datamap = datamap | bit;
            let new_nodemap = nodemap & !bit;
            let new_eidx = entry_index_for_slot(new_datamap, i) as usize;
            let mut new_handles = with_node(node, Vec::new(), |n| {
                let mut nh = Vec::with_capacity(n.handles.len());
                for e in 0..dcount {
                    for t in 0..stride {
                        let h = n.handles[stride * e + t];
                        op_dup(h);
                        nh.push(h);
                    }
                }
                nh
            });
            for (off, &h) in centry.cols().iter().enumerate() {
                new_handles.insert(stride * new_eidx + off, h); // entry cols at the entry position
            }
            with_node(node, (), |n| {
                for s in 0..scount {
                    if s == sidx {
                        continue; // this subnode was collapsed away
                    }
                    let c = n.handles[subbase + s];
                    op_dup(c);
                    new_handles.push(c);
                }
            });
            let new = alloc_raw(new_handles, champ_header(new_datamap, new_nodemap, size - 1));
            op_drop(node);
            return (new, true);
        }
        // Subnode still holds ≥2 entries: keep it, just swap in the rebuilt child. Arity unchanged, so
        // CLONE the handle vector ONCE and use it AS the result — mutate the one child slot, dup the
        // rest — rather than reading one vector and building a second.
        let mut new_handles = with_node(node, Vec::new(), |n| n.handles.clone());
        for (idx, slot) in new_handles.iter_mut().enumerate() {
            if idx == subbase + sidx {
                *slot = new_child; // owned; old child ref was consumed by the recursion
            } else {
                op_dup(*slot); // kept handle: new node needs its own reference
            }
        }
        let new = alloc_raw(new_handles, champ_header(datamap, nodemap, size - 1));
        op_drop(node);
        return (new, true);
    }

    (node, false) // empty slot — absent
}

/// FBIP variant of `champ_remove_node`. `mine` ⇒ reuse `node`'s shell in place; `!mine` ⇒ delegate to
/// the path-copying `champ_remove_node` verbatim. CONSUMES `node`, BORROWS `key`. Returns
/// `(new_node, removed)`; when `removed` is false the returned handle is the unchanged input. Preserves
/// canonical shape (collapse cascade, remove-to-canonical-empty) exactly as the copy path. Bounded
/// recursion (≤ `CHAMP_LEVELS`).
fn champ_remove_fbip(
    node: Handle,
    key: Handle,
    hash: u32,
    level: u32,
    stride: usize,
    mine: bool,
) -> (Handle, bool) {
    if !mine {
        return champ_remove_node(node, key, hash, level, stride); // shared: proven copy path
    }
    // ALLOCATION-LAZY: read the header + INDIVIDUAL slots (borrows, no Vec) to decide absent-vs-present,
    // and only `mem::take` the handle vector in the branches that actually rebuild a node. Every ABSENT
    // path and the common non-collapse DESCEND then allocate NOTHING (the old code cloned the whole
    // handle vector at every level up front). `node` is rc==1 (caller-gated), so an in-place slot write
    // and a deferred take are both safe — no other reference observes the node.
    let (datamap, nodemap, size, arity) = with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
        (champ_datamap(&n.raw), champ_nodemap(&n.raw), champ_size(&n.raw), n.handles.len())
    });

    // Empty node or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            return (node, false); // empty — absent
        }
        // Collision node: scan entry keys by BORROW (champ_handle_at), no clone, to find a match.
        let mut found = None;
        let mut idx = 0;
        while idx < arity {
            if champ_eq(champ_handle_at(node, idx), key) {
                found = Some(idx);
                break;
            }
            idx += stride;
        }
        let j = match found {
            Some(j) => j,
            None => return (node, false), // absent — unchanged, zero alloc
        };
        // Present: drain the removed entry's `stride` columns from the taken vector IN PLACE (reusing
        // its allocation), shifting the remaining collision entries left — collision entries stay in
        // canonical KEY order, so the drain preserves it.
        let mut handles = champ_take_handles(node);
        let entries_after = (arity / stride - 1) as u32;
        for h in handles.drain(j..j + stride) {
            op_drop(h); // removed entry columns: release the node's reference
        }
        return (champ_become_hdr(node, handles, 0, 0, entries_after), true);
    }

    let dcount = data_count(datamap) as usize;
    let subbase = stride * dcount; // subnodes follow the `dcount` entries; the datamap-found and
    let i = level_index(hash, level); // COLLAPSE paths now rebuild in place, so `scount` isn't needed
    let bit = 1u32 << i;

    if datamap & bit != 0 {
        let eidx = entry_index_for_slot(datamap, i) as usize;
        if !champ_eq(champ_handle_at(node, stride * eidx), key) {
            return (node, false); // different key occupies the slot — absent, zero alloc
        }
        // Present: remove the entry's `stride` columns from the taken vector IN PLACE (drain the range),
        // reusing its allocation rather than building a fresh `new_handles`. The removed columns sit at
        // `[base .. base+stride)` within the entry region (which precedes the subnodes), so draining them
        // shifts the remaining entries + all subnodes left by `stride` — exactly the canonical layout.
        let mut handles = champ_take_handles(node);
        let new_datamap = datamap & !bit;
        let base = stride * eidx;
        for h in handles.drain(base..base + stride) {
            op_drop(h); // removed entry columns: release the node's references
        }
        return (
            champ_become_hdr(node, handles, new_datamap, nodemap, size - 1),
            true,
        );
    }

    if nodemap & bit != 0 {
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = champ_handle_at(node, subbase + sidx); // borrow the child slot, no clone
        let child_mine = node_rc(child) == 1;
        let (new_child, removed) = champ_remove_fbip(child, key, hash, level + 1, stride, child_mine);
        if !removed {
            // Unchanged: `new_child == child` (in-place path) or an untouched shared handle (copy path
            // returns the input on absent). Either way the node's reference is intact; nothing to undo.
            return (node, false); // zero alloc
        }
        if let Some(centry) = collapse_candidate(new_child, stride) {
            // COLLAPSE: inline the child's single entry into this node at slot i. Dup the entry cols
            // (they must survive the child wrapper's drop), then free the now-empty child wrapper.
            for &h in centry.cols() {
                op_dup(h);
            }
            op_drop(new_child); // frees the collapsed child; entry cols survive via our dups
            // Transform the taken `handles` IN PLACE rather than rebuilding: the collapsed subnode's ONE
            // handle leaves (at `subbase + sidx`) and the entry's `stride` columns enter the entry region
            // (at `stride * new_eidx`). Both regions are contiguous and the entry region precedes the
            // subnodes, so: (1) remove the subnode handle, then (2) splice the entry columns at their
            // slot — the drain shifts the surviving subnodes, and the splice shifts them again to sit
            // after the now-larger entry region. Reuses the vec; only the `centry` dups above allocate
            // nothing (Entry is inline). Net arity change is `stride - 1`.
            let mut handles = champ_take_handles(node);
            let new_datamap = datamap | bit;
            let new_nodemap = nodemap & !bit;
            let new_eidx = entry_index_for_slot(new_datamap, i) as usize;
            handles.remove(subbase + sidx); // the collapsed subnode handle leaves
            for (off, &h) in centry.cols().iter().enumerate() {
                handles.insert(stride * new_eidx + off, h); // inlined entry columns (dup'd above)
            }
            return (
                champ_become_hdr(node, handles, new_datamap, new_nodemap, size - 1),
                true,
            );
        }
        // Subnode still holds ≥2 entries: keep it, swap in the rebuilt child (== child if in-place).
        // Arity unchanged, so DON'T take/rebuild — write the one child slot and patch the size field
        // directly into `node`'s live storage (rc==1). The recursion consumed the old child reference
        // at this slot; overwriting it with `new_child` is a no-op when the child was refit in place
        // (`new_child == child`) and installs the fresh node otherwise. ZERO allocation.
        champ_set_child_and_size_inplace(node, subbase + sidx, new_child, size - 1);
        return (node, true);
    }

    (node, false) // empty slot — absent
}

/// Remove `key`, returning the new map (the canonical empty if the last entry is removed).
/// CONSUMES `m` (moves through to the result), BORROWS `key`. Removing an absent key returns `m`
/// unchanged in value with no alloc, no leak, and no double-free. FBIP: a uniquely-owned map
/// (`rc == 1`) refits the touched spine in place; a shared map path-copies (old version byte-identical).
#[allow(dead_code)]
fn op_map_remove(m: Handle, key: Handle) -> Handle {
    let hash = champ_hash(key);
    let mine = node_rc(m) == 1;
    let (new, _removed) = champ_remove_fbip(m, key, hash, 0, MAP_STRIDE, mine);
    new
}

// ─── CHAMP cursor + in-order iteration (shared by map AND set) ───────────────────────────
// A CURSOR is itself a Node — so `op_drop` reclaims it with no special-casing — but it is an
// EPHEMERAL LINEAR value NEVER passed to hash/eq/lookup/insert/remove. Its layout is deliberately
// distinct from a map node:
//   handles = the descent-path frames, root→deepest: `[frame0=root, frame1, …, frameDeepest]`, each
//             an owned (dup'd) reference so the whole focused path stays live for the walk. Bounded
//             to ≤ CHAMP_LEVELS frames + at most one collision frame.
//   raw     = `[state:u32][slot0:u32][slot1:u32]…[slotN:u32]` (one slot per frame, so
//             `slots.len() == handles.len()`). `state` is CURSOR_LIVE or CURSOR_EXHAUSTED. For a
//             NORMAL frame, `slot_i` is the trie-SLOT index (0..31) the walk is at; for a COLLISION
//             frame (the deepest, when present) `slot_i` is the 0-based ENTRY INDEX within it.
//
// In-order walk (the canonical order proved in U3): at each node, visit trie slots 0..31 ASCENDING;
// a datamap bit ⇒ emit the inline entry at that slot; a nodemap bit ⇒ descend into that subnode and
// finish its whole 0..31 walk BEFORE continuing the parent's higher slots. Collision-node entries
// are emitted in stored order. Because a bit is in exactly one bitmap, the order is unambiguous and
// identical for equal maps. The walk is naturally bounded by depth (no unbounded recursion).
//
// STRIDE-PARAMETERIZED so the set (stride 1) reuses these verbatim: `champ_descend_leftmost`,
// `champ_advance`, and `champ_cursor_current` take `stride` (2 for map k/v, 1 for set elems).
// A uniquely-owned cursor (`node_rc == 1`) advances IN PLACE via `champ_cursor_next_fbip`; a shared
// cursor path-copies a fresh independent one (never writing through the consumed cursor's frames/raw),
// so forked cursors stay independent by construction.

const CURSOR_LIVE: u32 = 0;
const CURSOR_EXHAUSTED: u32 = 1;
/// Handles-per-entry: map stores `[k,v]` (2), set stores `[e]` (1).
const MAP_STRIDE: usize = 2;
/// A set is CHAMP minus the value column — a PRIMITIVE collection (not `Map<T,Unit>`), stride 1.
const SET_STRIDE: usize = 1;

/// The `i`-th subnode of a node under the given entry stride, or NULL (benign).
#[allow(dead_code)]
fn champ_subnode_at(node: Handle, slot: u32, stride: usize) -> Handle {
    with_node(node, Handle::NULL, |n| {
        let dm = champ_datamap(&n.raw);
        let nm = champ_nodemap(&n.raw);
        let base = stride * data_count(dm) as usize;
        let sidx = subnode_index_for_slot(nm, slot) as usize;
        n.handles.get(base + sidx).copied().unwrap_or(Handle::NULL)
    })
}

/// A single handle from a node's `handles`, or NULL (benign).
#[allow(dead_code)]
fn champ_handle_at(node: Handle, idx: usize) -> Handle {
    with_node(node, Handle::NULL, |n| n.handles.get(idx).copied().unwrap_or(Handle::NULL))
}

/// From `node`, descend to the LEFTMOST (in-order first) entry, appending a `(node, slot)` frame at
/// each level. `frames`/`slots` receive BORROWED node pointers (the caller dups them for ownership).
/// `node` MUST be non-empty (callers exclude the empty root); subnodes are ≥2 entries by invariant,
/// so this always terminates at an inline entry or a collision frame.
#[allow(dead_code)]
fn champ_descend_leftmost(node: Handle, frames: &mut Vec<Handle>, slots: &mut Slots, stride: usize) {
    let mut cur = node;
    loop {
        let (dm, nm, is_coll) = with_node(cur, (0u32, 0u32, false), |n| {
            let dm = champ_datamap(&n.raw);
            let nm = champ_nodemap(&n.raw);
            (dm, nm, dm == 0 && nm == 0 && !n.handles.is_empty())
        });
        if is_coll {
            frames.push(cur);
            slots.push(0); // entry index 0
            return;
        }
        let combined = dm | nm;
        if combined == 0 {
            // Defensive: an empty node (should not occur below the root). Record it as a leaf.
            frames.push(cur);
            slots.push(0);
            return;
        }
        let j = combined.trailing_zeros();
        frames.push(cur);
        slots.push(j);
        if dm & (1 << j) != 0 {
            return; // inline entry — deepest frame reached
        }
        cur = champ_subnode_at(cur, j, stride); // nodemap bit — descend
    }
}

/// Advance the cursor's OWN `(frames, slots)` in place to the in-order successor, applying the frame
/// refcount delta INLINE (`op_drop` each popped frame, `op_dup` each newly-descended frame; the kept
/// prefix is untouched) — the FBIP twin of `champ_advance` for a uniquely-owned cursor. This lets the
/// caller advance the taken frames directly, with NO `frames.clone()` and NO post-hoc common-prefix
/// diff (the whole point: kill the last per-step Vec allocation in `champ_cursor_next_fbip`). The net
/// delta is IDENTICAL to `champ_advance` + the external diff — a kept frame: 0; a popped frame: −1; a
/// descended frame: +1. Returns true on a new entry, false when exhausted (frames/slots emptied, every
/// remaining frame dropped). SAFETY: caller verified the cursor is `rc == 1` and owns these frames.
fn champ_advance_fbip(frames: &mut Vec<Handle>, slots: &mut Slots, stride: usize) -> bool {
    loop {
        let depth = frames.len();
        if depth == 0 {
            return false; // exhausted — all frames already dropped as they were popped
        }
        let node = frames[depth - 1];
        let cur = slots[depth - 1];
        let (dm, nm, ecount, is_coll) = with_node(node, (0u32, 0u32, 0usize, false), |n| {
            let dm = champ_datamap(&n.raw);
            let nm = champ_nodemap(&n.raw);
            let is_coll = dm == 0 && nm == 0 && !n.handles.is_empty();
            (dm, nm, n.handles.len() / stride, is_coll)
        });
        if is_coll {
            if (cur as usize) + 1 < ecount {
                slots[depth - 1] = cur + 1; // next collision entry — same frame, no ref change
                return true;
            }
            op_drop(frames.pop().unwrap()); // exhausted this collision node — release its ref, resume at parent
            slots.pop();
            continue;
        }
        let combined = dm | nm;
        let above = if cur >= 31 {
            0
        } else {
            combined & !((1u32 << (cur + 1)) - 1)
        };
        if above == 0 {
            op_drop(frames.pop().unwrap()); // nothing left here — release this frame, resume at parent
            slots.pop();
            continue;
        }
        let j = above.trailing_zeros();
        slots[depth - 1] = j; // move within the SAME (kept) frame — no ref change
        if dm & (1 << j) != 0 {
            return true; // inline entry here — deepest
        }
        // nodemap bit: descend into the subnode and take its leftmost, DUP'ing each new frame.
        let child = champ_subnode_at(node, j, stride);
        champ_descend_leftmost_dup(child, frames, slots, stride);
        return true;
    }
}

/// `champ_descend_leftmost` but `op_dup`s each frame it pushes — the cursor takes ownership of every
/// newly-focused node on the descent. Used only by `champ_advance_fbip` (the inline-refcount advance).
fn champ_descend_leftmost_dup(node: Handle, frames: &mut Vec<Handle>, slots: &mut Slots, stride: usize) {
    let start = frames.len();
    champ_descend_leftmost(node, frames, slots, stride);
    for &f in &frames[start..] {
        op_dup(f); // the cursor now owns a reference to each descended frame
    }
}

/// Advance a working `(frames, slots)` walk state to the in-order successor. Returns true when it
/// lands on a new entry (state stays live), false when the walk is exhausted (frames/slots emptied).
/// Operates on the caller's COPIES (pointer values), so it never mutates any node. Bounded by depth.
#[allow(dead_code)]
fn champ_advance(frames: &mut Vec<Handle>, slots: &mut Slots, stride: usize) -> bool {
    loop {
        let depth = frames.len();
        if depth == 0 {
            return false; // exhausted
        }
        let node = frames[depth - 1];
        let cur = slots[depth - 1];
        let (dm, nm, ecount, is_coll) = with_node(node, (0u32, 0u32, 0usize, false), |n| {
            let dm = champ_datamap(&n.raw);
            let nm = champ_nodemap(&n.raw);
            let is_coll = dm == 0 && nm == 0 && !n.handles.is_empty();
            (dm, nm, n.handles.len() / stride, is_coll)
        });
        if is_coll {
            if (cur as usize) + 1 < ecount {
                slots[depth - 1] = cur + 1; // next collision entry
                return true;
            }
            frames.pop();
            slots.pop();
            continue; // exhausted this collision node — resume at parent
        }
        // Normal node: lowest set bit of (datamap|nodemap) strictly above `cur`.
        let combined = dm | nm;
        let above = if cur >= 31 {
            0
        } else {
            combined & !((1u32 << (cur + 1)) - 1)
        };
        if above == 0 {
            frames.pop();
            slots.pop();
            continue; // nothing left here — resume at parent
        }
        let j = above.trailing_zeros();
        slots[depth - 1] = j;
        if dm & (1 << j) != 0 {
            return true; // inline entry here — deepest
        }
        // nodemap bit: descend into the subnode and take its leftmost.
        let child = champ_subnode_at(node, j, stride);
        champ_descend_leftmost(child, frames, slots, stride);
        return true;
    }
}

/// Build a cursor node owning the (already-dup'd) `frames` and encoding `slots` + `state` in raw.
#[allow(dead_code)]
fn champ_make_cursor(frames: Vec<Handle>, slots: Slots, state: u32) -> Handle {
    let mut raw = Vec::with_capacity(4 * (1 + slots.len()));
    raw.extend_from_slice(&state.to_le_bytes());
    for s in slots.as_slice() {
        raw.extend_from_slice(&s.to_le_bytes());
    }
    alloc(frames, raw)
}

/// Read a cursor into `(state, frames, slots)`. `frames` are BORROWED pointer copies (owned by the
/// cursor); `slots.len() == frames.len()`.
#[allow(dead_code)]
fn champ_cursor_read(cur: Handle) -> (u32, Vec<Handle>, Slots) {
    with_node(cur, (CURSOR_EXHAUSTED, Vec::new(), Slots::new()), |n| {
        let state = read_u32_at(&n.raw, 0);
        let frames = n.handles.clone();
        let mut slots = Slots::new();
        for i in 0..frames.len() {
            slots.push(read_u32_at(&n.raw, 4 + 4 * i));
        }
        (state, frames, slots)
    })
}

/// Like `champ_cursor_read` but MOVES the frame handles out of the cursor (`mem::take`) instead of
/// cloning them — the alloc-free read for the FBIP advance, whose caller has verified the cursor is
/// UNIQUELY owned (`rc == 1`), so no other reference observes the transient empty `handles`. The
/// returned `frames` carry the cursor's frame references verbatim (a `Vec<Handle>` move touches no
/// refcount — `Handle` is `Copy`); the caller MUST reinstall a frame vector via `champ_become_cursor`
/// before returning (every path does). SAFETY: caller verified `node_rc(cur) == 1`.
fn champ_cursor_take(cur: Handle) -> (u32, Vec<Handle>, Slots) {
    match unsafe { cur.0.as_mut() } {
        Some(n) => {
            let state = read_u32_at(&n.raw, 0);
            let frames = std::mem::take(&mut n.handles);
            let mut slots = Slots::new();
            for i in 0..frames.len() {
                slots.push(read_u32_at(&n.raw, 4 + 4 * i));
            }
            (state, frames, slots)
        }
        None => (CURSOR_EXHAUSTED, Vec::new(), Slots::new()),
    }
}

/// The current entry's `(deepest_node, base_index_in_handles)` under `stride`, or None if exhausted.
/// The projection primitive SHARED by map (key = base, val = base+1) and set (elem = base). BORROWS,
/// and allocates NOTHING: it needs only the DEEPEST frame + its slot, so it reads the last handle and
/// the last raw u32 straight out of the cursor node — not `champ_cursor_read`, which would clone the
/// whole frames Vec and build the whole slots Vec just to index `[depth-1]` (this is called on EVERY
/// key/val/elem projection, so that was ~2 wasted Vec allocs per iteration step).
#[allow(dead_code)]
fn champ_cursor_current(cur: Handle, stride: usize) -> Option<(Handle, usize)> {
    // Read state, the deepest frame (last handle), and the deepest slot (last raw u32) in one borrow.
    let (node, slot) = with_node(cur, (Handle::NULL, None::<u32>), |n| {
        let state = read_u32_at(&n.raw, 0);
        if state != CURSOR_LIVE || n.handles.is_empty() {
            return (Handle::NULL, None);
        }
        let depth = n.handles.len();
        let node = n.handles[depth - 1];
        // slots live at raw[4 + 4*i]; the deepest is at i = depth-1. `slots.len() == frames.len()`.
        let slot = read_u32_at(&n.raw, 4 + 4 * (depth - 1));
        (node, Some(slot))
    });
    let slot = slot?;
    let base = with_node(node, 0usize, |n| {
        let dm = champ_datamap(&n.raw);
        let nm = champ_nodemap(&n.raw);
        if dm == 0 && nm == 0 {
            stride * slot as usize // collision frame: slot IS the entry index
        } else {
            stride * entry_index_for_slot(dm, slot) as usize // normal frame
        }
    });
    Some((node, base))
}

/// A cursor over `m` positioned at the first entry in walk order (or exhausted if `m` is empty).
/// BORROWS `m` (dups the frames it captures; the cursor owns those refs).
#[allow(dead_code)]
fn op_map_iter(m: Handle) -> Handle {
    if is_empty_node(m) {
        return champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let mut frames = Vec::new();
    let mut slots = Slots::new();
    champ_descend_leftmost(m, &mut frames, &mut slots, MAP_STRIDE);
    for &f in &frames {
        op_dup(f); // the cursor owns a reference to every focused node
    }
    champ_make_cursor(frames, slots, CURSOR_LIVE)
}

// ─── FBIP (Functional But In-Place) rc==1 cursor advance for map-iter-next / set-iter-next (U6) ──
// A cursor is a Node: `handles` = the descent-path frames (root→deepest, each a dup'd owned ref),
// `raw` = `[state:u32][slot0][slot1]…`. The copy path (`op_*_iter_next` below, kept as the rc>1
// fallback) clones the walk state, advances on the COPY, dups every resulting frame for a fresh
// cursor, then drops the consumed cursor. FBIP reuses the cursor SHELL in place when it is uniquely
// owned — the WIT's zero-steady-state-alloc promise for a non-forked walk.
//
// 🚨 ALIASING SAFETY: gate on `node_rc(cur) == 1`. A forked/peeked/teed cursor (rc>1) MUST take the
// copy path so the other owner's walk is undisturbed. The cursor is a leaf handle (no descent into
// it), so the check is a single rc read — no downward propagation like the trie ops.
//
// FRAME RC DISCIPLINE (the leak/double-free trap): `champ_advance` pops exhausted frames and pushes
// newly-descended ones on the COPY, touching NO refcounts. Old and new frame lists share the longest
// common PREFIX (identical node pointers — a CHAMP walk is a root→leaf path, and advance only trims a
// suffix then may descend a new suffter). So the exact ref delta is: retain the shared prefix
// (untouched), `op_drop` each POPPED old-tail frame, `op_dup` each newly-DESCENDED new-tail frame.
// This nets IDENTICALLY to the copy path (which dups all new frames then drops all old frames): a
// common-prefix frame there is +1 (dup) −1 (drop) = 0, a new frame +1, a popped frame −1 — same.

/// Refit an rc==1 cursor's shell in place: `handles` become `frames`, `raw` becomes `[state]slots…`.
/// No new allocation, no self-drop. Frame REFCOUNTS are the caller's responsibility (it applies the
/// dup/drop delta before calling); overwriting the old `handles` Vec changes no refcounts (`Handle` is
/// `Copy`, no `Drop`). SAFETY: caller verified `node_rc(cur) == 1`.
fn champ_become_cursor(cur: Handle, frames: Vec<Handle>, slots: Slots, state: u32) -> Handle {
    if let Some(n) = unsafe { cur.0.as_mut() } {
        // Reuse the cursor's EXISTING `raw` allocation (clear keeps its capacity) instead of allocating
        // a fresh Vec — the cursor is rc==1, and its raw already held a `[state]slots…` of comparable
        // size, so the re-extend rarely reallocates. Saves one Vec allocation per advance step.
        n.raw.clear();
        n.raw.extend_from_slice(&state.to_le_bytes());
        for s in slots.as_slice() {
            n.raw.extend_from_slice(&s.to_le_bytes());
        }
        n.handles = frames;
    }
    cur
}

/// FBIP in-place advance of a UNIQUELY-OWNED (`rc == 1`) cursor. Reuses `champ_advance` verbatim for
/// the traversal (identical order + exhausted-signal to the copy path), then applies ONLY the frame-ref
/// delta and refits `cur`'s shell in place. Returns `cur`. Stride selects map (2) vs set (1).
fn champ_cursor_next_fbip(cur: Handle, stride: usize) -> Handle {
    // `cur` is rc==1 (caller-gated) ⇒ MOVE its frames/slots out (no clone) and advance them IN PLACE.
    // `champ_advance_fbip` applies the frame refcount delta inline (drop popped, dup descended, keep
    // the prefix), so there is NO `frames.clone()` and NO post-hoc common-prefix diff — the last
    // per-step Vec allocation is gone. We reinstall via `champ_become_cursor` on every return path
    // before any other reference could observe `cur` (single-threaded, rc==1).
    let (state, mut frames, mut slots) = champ_cursor_take(cur);
    if state != CURSOR_LIVE {
        // Already exhausted: release any frames it held (normally none) and stay exhausted in place.
        for &f in &frames {
            op_drop(f);
        }
        return champ_become_cursor(cur, Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let live = champ_advance_fbip(&mut frames, &mut slots, stride);
    // On exhaustion `champ_advance_fbip` has already dropped every frame it popped (frames now empty);
    // on a live step it has applied the exact drop/dup delta. Either way `(frames, slots)` is the new
    // cursor state with correct refcounts — just reinstall it.
    let new_state = if live { CURSOR_LIVE } else { CURSOR_EXHAUSTED };
    champ_become_cursor(cur, frames, slots, new_state)
}

/// Advance to the in-order successor. CONSUMES `cur`, returns the advanced cursor. FBIP: a uniquely
/// owned cursor (`rc == 1`) is refit in place (zero steady-state alloc); a forked/shared cursor
/// (`rc > 1`) path-copies into a fresh cursor so the other owner's walk is undisturbed. At the last
/// entry the cursor becomes exhausted. Order + exhausted-signal are identical on both paths.
#[allow(dead_code)]
fn op_map_iter_next(cur: Handle) -> Handle {
    if node_rc(cur) == 1 {
        return champ_cursor_next_fbip(cur, MAP_STRIDE);
    }
    // Shared (rc>1): copy path — build a fresh independent cursor, then release just THIS reference.
    // `champ_cursor_read` CLONED the frames (the shared cursor keeps its own), so `frames`/`slots`
    // here are throwaway locals — MOVE them into the working `wf`/`ws` rather than cloning again.
    let (state, frames, slots) = champ_cursor_read(cur);
    if state != CURSOR_LIVE {
        op_drop(cur);
        return champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let mut wf = frames;
    let mut ws = slots;
    let live = champ_advance(&mut wf, &mut ws, MAP_STRIDE);
    let new = if live {
        for &f in &wf {
            op_dup(f); // new cursor's OWN refs (independent of the consumed cursor's)
        }
        champ_make_cursor(wf, ws, CURSOR_LIVE)
    } else {
        champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED)
    };
    op_drop(cur); // release the consumed cursor's frame refs
    new
}

/// The current key, or NULL when exhausted (NULL is the done-signal; a real key is never NULL).
/// BORROWS.
#[allow(dead_code)]
fn op_map_iter_key(cur: Handle) -> Handle {
    match champ_cursor_current(cur, MAP_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base),
        None => Handle::NULL,
    }
}

/// The current value (paired with `op_map_iter_key`; no per-step pair allocation). BORROWS.
#[allow(dead_code)]
fn op_map_iter_val(cur: Handle) -> Handle {
    match champ_cursor_current(cur, MAP_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base + 1),
        None => Handle::NULL,
    }
}

// ─── CHAMP persistent SET (CHAMP minus the value column, stride 1) ───────────────────────
// A set is a PRIMITIVE collection, NOT `Map<T, Unit>`: entries are ONE handle. Every op is a thin
// `SET_STRIDE` wrapper over the SAME shared trie core the map uses (`champ_insert_node`,
// `champ_remove_node`, `champ_find_base`, and the cursor walkers), so there is a single code path to
// trust. The node shape is identical to a map's (bitmaps + size + handles); only the stride at the
// use-site differs — and the compiler picks the op family statically, so a set node is only ever
// touched with stride 1.

/// The canonical empty set — byte-identical to the empty map (`alloc_raw(vec![], champ_header(0,0,0))`);
/// the collection kind is compile-time knowledge, not a runtime tag.
#[allow(dead_code)]
fn op_set_empty() -> Handle {
    alloc_raw(Vec::new(), champ_header(0, 0, 0))
}

/// O(1) element count. BORROWS `s`.
#[allow(dead_code)]
fn op_set_size(s: Handle) -> u32 {
    champ_size_of(s)
}

/// Total membership predicate — NEVER traps. BORROWS both `s` and `elem`. The only bool-returning
/// CHAMP op: the shared descent, returning presence instead of a value handle.
#[allow(dead_code)]
fn op_set_contains(s: Handle, elem: Handle) -> bool {
    champ_find_base(s, elem, SET_STRIDE).is_some()
}

/// `op_set_contains` with `elem`'s hash PRECOMPUTED — for a caller (set ∩/∖) that will also insert the
/// same element and so hashes it once, using this for the membership probe and the same hash for the
/// insert instead of re-walking the element twice. BORROWS both.
fn set_contains_h(s: Handle, elem: Handle, hash: u32) -> bool {
    champ_find_base_h(s, elem, hash, SET_STRIDE).is_some()
}

/// Insert `elem`, returning the new set. CONSUMES `s`, `elem`. Idempotent: inserting an existing
/// element leaves size unchanged and drops the incoming duplicate (the shared OVERWRITE rule with
/// no value columns keeps the stored element and drops the newcomer). Persistent: `op_dup` `s` first
/// to keep it.
#[allow(dead_code)]
fn op_set_insert(s: Handle, elem: Handle) -> Handle {
    set_insert_h(s, elem, champ_hash(elem))
}

/// `op_set_insert` with `elem`'s hash PRECOMPUTED — lets the set-algebra ops reuse the one hash they
/// computed for the membership probe (or the walk) instead of re-hashing. CONSUMES `s` and `elem`.
fn set_insert_h(s: Handle, elem: Handle, hash: u32) -> Handle {
    let mine = node_rc(s) == 1;
    // SET_STRIDE (1) routes through the SAME FBIP core as the map — one careful change covers both.
    champ_insert_fbip(s, Entry::elem(elem), hash, 0, SET_STRIDE, mine)
}

/// Remove `elem`, returning the new set (canonical empty if the last element is removed). CONSUMES
/// `s`, BORROWS `elem`. Absent element ⇒ no-op returning `s` unchanged, no leak. FBIP: uniquely-owned
/// set refits in place; a shared set path-copies (old version byte-identical).
#[allow(dead_code)]
fn op_set_remove(s: Handle, elem: Handle) -> Handle {
    set_remove_h(s, elem, champ_hash(elem))
}

/// `op_set_remove` with `elem`'s hash PRECOMPUTED — lets set-difference (remove-from-a form) hash each
/// `b`-element once for its removal instead of re-hashing. CONSUMES `s`, BORROWS `elem`.
fn set_remove_h(s: Handle, elem: Handle, hash: u32) -> Handle {
    let mine = node_rc(s) == 1;
    let (new, _removed) = champ_remove_fbip(s, elem, hash, 0, SET_STRIDE, mine);
    new
}

/// A cursor over `s` at the first element in walk order (exhausted if `s` is empty). BORROWS `s`
/// (dups the frames it captures). Same cursor representation as the map's.
#[allow(dead_code)]
fn op_set_iter(s: Handle) -> Handle {
    if is_empty_node(s) {
        return champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let mut frames = Vec::new();
    let mut slots = Slots::new();
    champ_descend_leftmost(s, &mut frames, &mut slots, SET_STRIDE);
    for &f in &frames {
        op_dup(f);
    }
    champ_make_cursor(frames, slots, CURSOR_LIVE)
}

/// Advance to the in-order successor. CONSUMES `cur`, returns the advanced cursor. FBIP: a uniquely
/// owned cursor (`rc == 1`) is refit in place (zero steady-state alloc); a forked/shared cursor
/// (`rc > 1`) path-copies into a fresh cursor so the other owner's walk is undisturbed. Order +
/// exhausted-signal identical to the map path (shared `champ_cursor_next_fbip` / `champ_advance`).
#[allow(dead_code)]
fn op_set_iter_next(cur: Handle) -> Handle {
    if node_rc(cur) == 1 {
        return champ_cursor_next_fbip(cur, SET_STRIDE);
    }
    // Shared (rc>1): copy path. `champ_cursor_read` already cloned the frames, so move the throwaway
    // `frames`/`slots` locals into the working `wf`/`ws` instead of cloning them a second time.
    let (state, frames, slots) = champ_cursor_read(cur);
    if state != CURSOR_LIVE {
        op_drop(cur);
        return champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED);
    }
    let mut wf = frames;
    let mut ws = slots;
    let live = champ_advance(&mut wf, &mut ws, SET_STRIDE);
    let new = if live {
        for &f in &wf {
            op_dup(f);
        }
        champ_make_cursor(wf, ws, CURSOR_LIVE)
    } else {
        champ_make_cursor(Vec::new(), Slots::new(), CURSOR_EXHAUSTED)
    };
    op_drop(cur);
    new
}

/// The current element, or NULL when exhausted (NULL is the done-signal; a real element is never
/// NULL). BORROWS.
#[allow(dead_code)]
fn op_set_iter_elem(cur: Handle) -> Handle {
    match champ_cursor_current(cur, SET_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base),
        None => Handle::NULL,
    }
}

// ─── CHAMP SET ALGEBRA — union / intersection / difference (U7) ───────────────────────────
// CORRECTNESS-FIRST landing over the existing insert + contains + iterate machinery: walk one
// operand's elements with a cursor (BORROWS) and build the result with `op_set_insert` (a CONSTRUCTOR
// that CONSUMES its accumulator + element — and is FBIP-fast on a uniquely-owned accumulator, so a
// fresh empty or a consumed operand refits in place with no per-insert node churn). A recursive
// CHAMP node-merge (structural union of subtrees, sharing whole shared subnodes) would be the
// O(min) form — DEFERRED as an optimization; this is O(n·log) but correct and canonical.
//
// Canonicality: `op_set_insert` places every element in the canonical CHAMP position (sorted collision
// nodes, compacted layout), so a set built by folding inserts is byte-identical (`champ_eq`/
// `champ_hash`) regardless of insertion order — the result is canonical BY CONSTRUCTION.
//
// Element ownership: a cursor / `set-contains` BORROWS its element, so an element CARRIED into the
// result is `op_dup`ed before `op_set_insert` consumes it; an element NOT carried stays owned by its
// operand and dies when that operand is `op_drop`ed. Both operands are consumed (dropped) at the end.

/// `set-union` — a new owned set with every element of `a` OR `b`. CONSUMES both. Built by walking the
/// SMALLER operand's elements into the LARGER (the accumulator base): union is commutative and the CHAMP
/// result is canonical-by-construction (insertion-order-independent), so the base choice cannot change
/// the result — only how many inserts run. Walking the smaller therefore does `min(|a|,|b|)` inserts
/// instead of always `|b|`. The base's own elements are carried by reusing it as the accumulator (no
/// per-element dup). An empty operand is the identity (`union(empty,b)==b`, `union(a,empty)==a`).
#[allow(dead_code)]
fn op_set_union(a: Handle, b: Handle) -> Handle {
    if a == b {
        // Same node (structural sharing / self-union): a ∪ a = a. The caller passed two references to
        // the one node; keep one as the result and release the other. O(1) vs the O(n·log) fold below.
        op_drop(b);
        return a;
    }
    if is_empty_node(a) {
        op_drop(a);
        return b;
    }
    if is_empty_node(b) {
        op_drop(b);
        return a;
    }
    // Insert the SMALLER set into the LARGER: fewer inserts, identical (canonical) result.
    let (base, walk) = if op_set_size(a) >= op_set_size(b) {
        (a, b)
    } else {
        (b, a)
    };
    let mut acc = base;
    let mut cur = op_set_iter(walk);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break;
        }
        // Hash `e` ONCE and pass it to the insert (via `set_insert_h`) rather than `op_set_insert`,
        // which would re-`champ_hash(e)` internally — a full subtree re-walk per element for a
        // compound/string element (free for scalars via the arity-0 fast path). Mirrors the hash-once
        // discipline already used by ∩/∖ below.
        let h = champ_hash(e);
        op_dup(e); // carried into the result; cursor only BORROWS it
        acc = set_insert_h(acc, e, h);
        cur = op_set_iter_next(cur);
    }
    op_drop(cur);
    op_drop(walk); // walk's own references released; carried elements survive via the dups above
    acc
}

/// `set-intersection` — a new owned set with elements in `a` AND `b`. CONSUMES both. Builds a fresh
/// empty accumulator, walks the SMALLER operand, and inserts each element that `set-contains` in the
/// other. `intersection(x, empty) == empty` and `intersection(empty, x) == empty`.
#[allow(dead_code)]
fn op_set_intersection(a: Handle, b: Handle) -> Handle {
    if a == b {
        // Same node: a ∩ a = a. Keep one reference, release the other. O(1). (Correct even for the
        // empty set: ∅ ∩ ∅ = ∅.)
        op_drop(b);
        return a;
    }
    if is_empty_node(a) || is_empty_node(b) {
        op_drop(a);
        op_drop(b);
        return op_set_empty();
    }
    // Walk the smaller operand, probe the larger — fewer contains-probes, same result.
    let (walk, probe) = if op_set_size(a) <= op_set_size(b) {
        (a, b)
    } else {
        (b, a)
    };
    let mut acc = op_set_empty();
    let mut cur = op_set_iter(walk);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break;
        }
        // Hash `e` ONCE and reuse it for both the membership probe and the insert (the probe and the
        // insert would otherwise each re-walk `e`'s subtree — a redundant hash per kept element).
        let h = champ_hash(e);
        if set_contains_h(probe, e, h) {
            op_dup(e); // carried; cursor BORROWS
            acc = set_insert_h(acc, e, h);
        }
        cur = op_set_iter_next(cur);
    }
    op_drop(cur);
    op_drop(walk);
    op_drop(probe);
    acc
}

/// `set-difference` — a new owned set with elements in `a` but NOT in `b`. CONSUMES both. Builds a
/// fresh empty accumulator, walks `a`, and inserts each element NOT `set-contains` in `b`.
/// `difference(a, empty) == a` (all of a) and `difference(empty, b) == empty`.
#[allow(dead_code)]
fn op_set_difference(a: Handle, b: Handle) -> Handle {
    if a == b {
        // Same node: a ∖ a = ∅. Release both references and return a fresh empty set. O(1).
        op_drop(a);
        op_drop(b);
        return op_set_empty();
    }
    if is_empty_node(a) {
        op_drop(a);
        op_drop(b);
        return op_set_empty();
    }
    if is_empty_node(b) {
        op_drop(b);
        return a; // nothing excluded — a unchanged
    }
    // FAST PATH — remove `b`'s elements FROM `a` in place, when that is a clear win: `a` is UNIQUELY
    // owned (rc==1, so each remove refits it in place, allocation-free) AND `|b| < |a|` (so `|b|`
    // removes beat the `|a|` inserts-into-a-fresh-set the general path would do). The `b`-cursor
    // iterates `b` while the removes target `a` (a DISTINCT tree) — no aliasing. Removing a `b`-element
    // absent from `a` is a no-op, so the result is exactly `a ∖ b`, canonical (remove preserves shape).
    // NOTE the guards: on a SHARED `a` every remove would path-copy (WORSE than the insert-fold), and
    // when `|b| ≥ |a|` the insert-fold does fewer ops — so both fall through to the general path below.
    if node_rc(a) == 1 && op_set_size(b) < op_set_size(a) {
        let mut acc = a;
        let mut cur = op_set_iter(b);
        loop {
            let e = op_set_iter_elem(cur);
            if e == Handle::NULL {
                break;
            }
            acc = set_remove_h(acc, e, champ_hash(e)); // hash `e` once; cursor + remove both BORROW it
            cur = op_set_iter_next(cur);
        }
        op_drop(cur);
        op_drop(b); // b's references released; nothing carried into the result
        return acc;
    }
    let mut acc = op_set_empty();
    let mut cur = op_set_iter(a);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break;
        }
        // Hash `e` ONCE for both the membership probe and (on a kept element) the insert.
        let h = champ_hash(e);
        if !set_contains_h(b, e, h) {
            op_dup(e); // carried; cursor BORROWS
            acc = set_insert_h(acc, e, h);
        }
        cur = op_set_iter_next(cur);
    }
    op_drop(cur);
    op_drop(a);
    op_drop(b);
    acc
}

// ─── Native tests: exercise the Handle-typed heap+accessor core ─────────────────────────

// TEMP PROFILER (removed after measurement): a process-wide counting allocator for the native test
// build only, so a probe can measure GROSS heap allocations (including transient Vecs freed at once —
// the target of the clone-elimination work) per runtime operation. Counting only; correctness intact.
#[cfg(test)]
struct CountingAlloc;
#[cfg(test)]
static ALLOC_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(test)]
unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
#[cfg(test)]
#[global_allocator]
static PROFILER_ALLOC: CountingAlloc = CountingAlloc;

#[cfg(test)]
mod tests {
    use super::*;

    /// No shared table to clear — every value is its own allocation and every test holds the handles
    /// it builds. Kept as a documented no-op so each test reads as a self-contained scenario.
    fn reset() {}

    fn alloc_calls() -> u64 {
        ALLOC_CALLS.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Allocation-ceiling regression guard for the hot CHAMP/RRB ops, run SINGLE-THREADED (see below).
    /// Uses the process-wide `CountingAlloc` (test build only) to count GROSS heap allocations —
    /// including transient Vecs freed immediately, which the `live-objects` node counter does NOT see —
    /// so it catches a future change that reintroduces the per-spine-node `new_handles`/`champ_header`
    /// allocations this commit removed. Ceilings sit comfortably above the figures measured 2026-07-12
    /// after the champ_become_hdr + in-place-slot + allocation-lazy-remove + alloc-free-cursor + lazy
    /// champ_eq/cmp worklist + EMPTY-slot splice + inline-Entry + inline-refcount cursor advance +
    /// in-place remove-drain/collapse + inline-Slots cursor + in-place SPLIT + shallow-compound hash+eq work
    /// (insert 766, remove 0, iterate 3, push 197, get 0, lookup 0, tuplekey-lookup 2000=probe-only, sum_new ~2000;
    /// set union 431 / ∩ 356 / ∖ 362, ∖ unique-small-b 774≈build-only; they are UPPER BOUNDS so noise never trips them but a
    /// regression toward the old 6779/8397/5248/1000 does.
    ///
    /// ⚠ MUST run alone: the counter is PROCESS-WIDE, so a concurrent test thread's allocations pollute
    /// the reading (observed ~51k when run in the default multi-threaded suite). It is therefore
    /// `#[ignore]`d in the normal run (and in `cargo test`/`cargo xtask check`) and exercised on demand:
    ///   `cargo test -p cdz-runtime hot_op_allocation_ceilings -- --ignored --test-threads=1 --nocapture`
    /// Correctness of these ops is covered independently by the FBIP canonical-shape / reference tests,
    /// which DO run in the normal suite; this test guards only the allocation budget.
    #[test]
    #[ignore] // process-wide counter — run alone with --ignored --test-threads=1 (see doc)
    fn hot_op_allocation_ceilings() {
        reset();
        let measure = |f: &mut dyn FnMut()| -> u64 {
            let start = alloc_calls();
            f();
            alloc_calls() - start
        };
        const N: i64 = 1000;

        // (A) map insert (unique, FBIP) — N fresh keys.
        let mut m = op_map_empty();
        let insert = measure(&mut || {
            for k in 0..N {
                m = op_map_insert(m, op_box_int(k), op_box_int(k * 2));
            }
        });
        println!("ALLOC map_insert x{N}: {insert}");
        assert!(insert <= 900, "unique map_insert x{N} allocs {insert} exceeds ceiling 900 (… → 1084 in-place SPLIT → ~766 inline champ_header raw; residual = the intrinsic subnode Box + handles Vec on a split)");

        // (A2) PERSISTENT insert (OVERWRITE) into a SHARED map — the real-world functional pattern (keep
        // the old version, derive a new one). `mkeep` is kept (rc>1) across each insert, so every insert
        // path-copies the touched spine (root→leaf, ~log32(N) nodes) via `champ_insert_node` instead of
        // refitting in place. Each existing key is overwritten (arity-preserving). The copy path was
        // cutting ~2 Vec allocs per path-copied node (a throwaway upfront `handles.clone()` PLUS a
        // separate `new_handles`); now it clones ONCE and mutates that copy → 8715→6143 (−30%). Guards
        // that the copy path stays single-Vec-per-node; a regression to the double-alloc would ~1.4x it.
        let mut mkeep = op_map_empty();
        for k in 0..N {
            mkeep = op_map_insert(mkeep, op_box_int(k), op_box_int(k));
        }
        let pinsert = measure(&mut || {
            for k in 0..N {
                op_dup(mkeep); // keep the base shared → force the path-copy branch
                let m2 = op_map_insert(mkeep, op_box_int(k), op_box_int(k * 3));
                op_drop(m2);
            }
        });
        println!("ALLOC map_insert_shared x{N}: {pinsert}");
        assert!(pinsert <= 6400, "shared/persistent map_insert (overwrite) x{N} allocs {pinsert} exceeds ceiling 6400 (path-copy: 1 Vec + 1 node Box per copied spine node; was 8715 with a wasted upfront handles.clone())");
        op_drop(mkeep);

        // (A3) persistent insert of a NEW key into a shared map — exercises the EMPTY-slot / SPLIT copy
        // branches (build a persistent map while keeping the prior version), the growth half of the copy
        // path. Base is the same N-element shared map; each iteration inserts a fresh, absent key (N+k)
        // that lands in an empty slot or splits. The upfront `handles.clone()` was PURE WASTE on these
        // fresh-result branches (they only read by index) — removed → 7445→6445 (−13%). Guards it stays
        // borrow-and-build (no upfront clone) on the growth path.
        let mut mkeep2 = op_map_empty();
        for k in 0..N {
            mkeep2 = op_map_insert(mkeep2, op_box_int(k), op_box_int(k));
        }
        let pinsert_new = measure(&mut || {
            for k in 0..N {
                op_dup(mkeep2);
                let m2 = op_map_insert(mkeep2, op_box_int(N + k), op_box_int(k));
                op_drop(m2);
            }
        });
        println!("ALLOC map_insert_shared_newkey x{N}: {pinsert_new}");
        assert!(pinsert_new <= 6700, "shared/persistent map_insert (new key) x{N} allocs {pinsert_new} exceeds ceiling 6700 (path-copy growth: borrow-and-build, no upfront clone; was 7445)");
        op_drop(mkeep2);

        // (C2) PERSISTENT remove from a SHARED map — keep the base (rc>1) across each remove, so every
        // remove path-copies the touched spine via `champ_remove_node` instead of refitting in place.
        // Had the SAME double-alloc smell as the insert copy path: an upfront `handles.clone()` (wasted
        // on the absent-key early-returns AND the fresh-shorter-result branches, which only read by
        // index) PLUS a separate `new_handles` per node. Now: read header+arity only upfront; the
        // arity-preserving DESCEND-no-collapse branch clones ONCE and mutates that copy; the shorter/
        // reshaped branches (found-entry drop, collapse) borrow-and-build. 9277→6705 (−28%). Guards the
        // remove copy path stays single-Vec-per-node.
        let mut mkeep3 = op_map_empty();
        for k in 0..N {
            mkeep3 = op_map_insert(mkeep3, op_box_int(k), op_box_int(k));
        }
        let premove = measure(&mut || {
            for k in 0..N {
                op_dup(mkeep3); // keep the base shared → force the path-copy branch
                let p = op_box_int(k);
                let m2 = op_map_remove(mkeep3, p);
                op_drop(p);
                op_drop(m2);
            }
        });
        println!("ALLOC map_remove_shared x{N}: {premove}");
        assert!(premove <= 7000, "shared/persistent map_remove x{N} allocs {premove} exceeds ceiling 7000 (path-copy: 1 Vec + 1 node Box per copied spine node; was 9277 with a wasted upfront handles.clone())");
        op_drop(mkeep3);

        // (B) full iteration (unique cursor walk).
        let iterate = measure(&mut || {
            let mut c = op_map_iter(m);
            while op_map_iter_key(c) != Handle::NULL {
                c = op_map_iter_next(c);
            }
            op_drop(c);
        });
        println!("ALLOC map_iterate x{N}: {iterate}");
        assert!(iterate <= 50, "unique map_iterate x{N} allocs {iterate} exceeds ceiling 50 (5248 → 2248 → 1126 → ~3 inline Slots buffer, iteration is now essentially alloc-free — only the initial cursor's frames Vec)");
        op_drop(m);

        // (C) map remove (unique) — remove all N.
        let mut m2 = op_map_empty();
        for k in 0..N {
            m2 = op_map_insert(m2, op_box_int(k), op_box_int(k));
        }
        let remove = measure(&mut || {
            for k in 0..N {
                let p = op_box_int(k);
                m2 = op_map_remove(m2, p);
                op_drop(p);
            }
        });
        println!("ALLOC map_remove x{N}: {remove}");
        assert!(remove <= 50, "unique map_remove x{N} allocs {remove} exceeds ceiling 50 (8397 → 5207 → 2953 → 1953 → 954 in-place drain → ~0 in-place COLLAPSE + inline collapse_candidate; remove is now allocation-FREE)");
        op_drop(m2);

        // (D) vec push (unique, FBIP) — the in-place RRB reference: near-zero amortized.
        let mut v = op_vec_empty();
        let push = measure(&mut || {
            for k in 0..N {
                v = op_vec_push(v, op_box_int(k));
            }
        });
        println!("ALLOC vec_push x{N}: {push}");
        assert!(push <= 400, "unique vec_push x{N} allocs {push} exceeds ceiling 400");
        // (E) vec get — a pure read must allocate NOTHING.
        let get = measure(&mut || {
            for k in 0..N as u32 {
                let _ = op_vec_get(v, k % N as u32);
            }
        });
        println!("ALLOC vec_get x{N}: {get}");
        assert_eq!(get, 0, "vec_get is a pure read — zero allocations");
        // (E2) vec update on a UNIQUELY-owned vec — the FBIP path swaps the element slot in place down
        // the spine (`vec_update_fbip`, `mine` all the way), so a random-access update on an owned vector
        // must allocate NOTHING. Guards that persistent update stays in-place on the unique-owner path
        // (a regression to path-copy would allocate a node per spine level per update).
        let vupd = measure(&mut || {
            for k in 0..N as u32 {
                v = op_vec_update(v, k % N as u32, op_box_int(k as i64 + 1));
            }
        });
        println!("ALLOC vec_update x{N}: {vupd}");
        assert_eq!(vupd, 0, "vec_update on a uniquely-owned vec is FBIP in-place — zero allocations");
        // (E3) PERSISTENT vec_update on a SHARED vec — keep the base (rc>1) across each update, so every
        // update path-copies the touched spine (root→leaf) via `vec_update_into`/`vec_node_replace`
        // instead of refitting in place. UNLIKE the CHAMP copy cores (which cloned the whole handle vec
        // upfront then built a second), the RRB copy path is ALREADY borrow-and-build: `vec_node_replace`
        // reads each child via `vec_child()` and builds ONE result Vec — no double-alloc smell. So this
        // sits at the path-copy floor (~2 allocs per copied spine node + header). Tracked so the common
        // real-world persistent update pattern (which the unique-FBIP E2 row never exercises) is guarded.
        let vupd_shared = measure(&mut || {
            for k in 0..N as u32 {
                op_dup(v); // keep the base shared → force the path-copy branch
                let v2 = op_vec_update(v, k % N as u32, op_box_int(k as i64 + 7));
                op_drop(v2);
            }
        });
        println!("ALLOC vec_update_shared x{N}: {vupd_shared}");
        assert!(vupd_shared <= 8200, "shared/persistent vec_update x{N} allocs {vupd_shared} exceeds ceiling 8200 (RRB path-copy floor: borrow-and-build, ~2 allocs per copied spine node + header)");
        // (D2) PERSISTENT vec_push on a SHARED vec — keep the base (rc>1) so each push path-copies the
        // rightmost spine via `vec_push_into`/`vec_node_append` instead of FBIP in place. Same borrow-and-
        // build copy path; tracked so the persistent-push pattern is guarded (the unique D row is FBIP).
        let vpush_shared = measure(&mut || {
            for _ in 0..N {
                op_dup(v);
                let v2 = op_vec_push(v, op_box_int(42));
                op_drop(v2);
            }
        });
        println!("ALLOC vec_push_shared x{N}: {vpush_shared}");
        assert!(vpush_shared <= 8300, "shared/persistent vec_push x{N} allocs {vpush_shared} exceeds ceiling 8300 (RRB path-copy floor: borrow-and-build rightmost spine + header)");
        op_drop(v);

        // TEMP PROBE: vec_concat / vec_split — the RRB O(log N) rebalancing ops (List.concat/List.split),
        // never benched. concat lifts both roots to a common level, gathers ≤64 children, builds 1-2
        // relaxed nodes: a SMALL constant node count independent of N (the shared subtrees are dup'd, not
        // copied). split rebuilds one boundary spine (≤7 relaxed nodes) + dup'd whole children. Measure
        // per-op (not ×N) since they're logarithmic. Build two N-element vecs once, outside the timing.
        let mk_vec = |lo: i64, hi: i64| -> Handle {
            let mut vv = op_vec_empty();
            for k in lo..hi {
                vv = op_vec_push(vv, op_box_int(k));
            }
            vv
        };
        let ca = mk_vec(0, N);
        let cbv = mk_vec(N, 2 * N);
        let concat_allocs = measure(&mut || {
            for _ in 0..100 {
                op_dup(ca);
                op_dup(cbv);
                op_drop(op_vec_concat(ca, cbv)); // consumes both dups
            }
        });
        println!("ALLOC vec_concat x100: {concat_allocs}");
        assert!(concat_allocs <= 2000, "vec_concat x100 allocs {concat_allocs} exceeds ceiling 2000 (O(log N) rebalance: ≤a few nodes/op, N-independent — a regression to element-copy would scale with N)");
        let split_allocs = measure(&mut || {
            for _ in 0..100 {
                op_dup(ca);
                let (l, r) = op_vec_split(ca, N as u32 / 2); // consumes the dup
                op_drop(l);
                op_drop(r);
            }
        });
        println!("ALLOC vec_split x100: {split_allocs}");
        assert!(split_allocs <= 3000, "vec_split x100 allocs {split_allocs} exceeds ceiling 3000 (O(log N) boundary-spine rebuild, N-independent)");
        op_drop(ca);
        op_drop(cbv);

        // (F) map lookup — a pure read on scalar keys must allocate NOTHING. Guards the lazy champ_eq
        // worklist (it used to allocate a `vec![(a,b)]` per key comparison even when the scalar keys
        // resolved with no child descent — 1 alloc per lookup on the hot path).
        let mut mm = op_map_empty();
        for k in 0..N {
            mm = op_map_insert(mm, op_box_int(k), op_box_int(k));
        }
        let lookup = measure(&mut || {
            for k in 0..N {
                let p = op_box_int(k); // small int ⇒ immediate, no box alloc
                let _ = op_map_lookup(mm, p);
                op_drop(p);
            }
        });
        println!("ALLOC map_lookup x{N}: {lookup}");
        assert_eq!(lookup, 0, "map_lookup on scalar keys is a pure read — zero allocations (was 1/op via champ_eq's eager worklist)");
        op_drop(mm);

        // (G) set algebra — union / intersection / difference of two N-element sets with 50% overlap.
        // These are O(n·log) insert-folds (union threads onto `a`; ∩/∖ probe-and-insert into a fresh
        // accumulator), so they dominate the remaining allocation budget; tracked so a change to the
        // insert/cursor/contains machinery they lean on is visible, and so a future O(min) node-merge
        // can be measured against them.
        let build_set = |lo: i64, hi: i64| -> Handle {
            let mut s = op_set_empty();
            for k in lo..hi {
                s = op_set_insert(s, op_box_int(k));
            }
            s
        };
        let sa = build_set(0, N);
        let sb = build_set(N / 2, N + N / 2); // 50% overlap, same size — ∩/∖ probe cost
        // Union uses a SMALLER second operand so the smaller-into-larger optimization is exercised
        // (union walks min(|a|,|sc|) = |sc| elements into the larger `sa`, not always |b|).
        let sc = build_set(N, N + N / 4); // size N/4, disjoint from sa
        let union = measure(&mut || {
            op_dup(sa);
            op_dup(sc);
            op_drop(op_set_union(sa, sc));
        });
        println!("ALLOC set_union x{N}: {union}");
        assert!(union <= 500, "set_union (walk the smaller N/4 into the larger N) allocs {union} exceeds ceiling 500");
        let inter = measure(&mut || {
            op_dup(sa);
            op_dup(sb);
            op_drop(op_set_intersection(sa, sb));
        });
        println!("ALLOC set_intersection x{N}: {inter}");
        assert!(inter <= 450, "set_intersection x{N} allocs {inter} exceeds ceiling 450");
        let diff = measure(&mut || {
            op_dup(sa);
            op_dup(sb);
            op_drop(op_set_difference(sa, sb));
        });
        println!("ALLOC set_difference x{N}: {diff}");
        assert!(diff <= 450, "set_difference x{N} allocs {diff} exceeds ceiling 450");
        op_drop(sa);
        op_drop(sb);
        op_drop(sc);

        // (G2) set difference with a UNIQUELY-OWNED large `a` minus a SMALL `b` — the remove-from-a fast
        // path (a rc==1, |b| < |a|): each of |b|'s removes refits `a` in place, so it is allocation-free,
        // vs the general insert-fold which rebuilds a fresh |a|-element set. `a` is consumed (not dup'd)
        // so it stays unique; `b` (size N/8) is kept via a dup for reuse. This guards the fast branch —
        // the (G) rows above use a SHARED equal-size `a`, which correctly stays on the insert-fold path.
        let db = build_set(0, N / 8); // small exclusion set, kept
        let ddiff = measure(&mut || {
            let da = build_set(0, N); // fresh unique `a` each iteration (consumed by the op)
            op_dup(db);
            op_drop(op_set_difference(da, db));
        });
        println!("ALLOC set_difference_unique_small_b x{N}: {ddiff}");
        // The fast path adds only the removes (in-place on unique `a`, 0-alloc) + the b-cursor; the build
        // of the fresh `da` per iteration dominates and is NOT what we measure — subtract it: a bare
        // build_set(0,N) is the map_insert cost. So this asserts the DIFFERENCE ITSELF adds little beyond
        // building da. Ceiling = build cost (~1084) + small headroom; a regression to the insert-fold
        // (which rebuilds another full set) would roughly double it.
        assert!(ddiff <= 1200, "unique-a small-b difference x{N} allocs {ddiff} exceeds ceiling 1200 (fast path: build da + in-place removes; a regression to the insert-fold would ~2x)");
        op_drop(db);

        // (H) map lookup by a SHALLOW-COMPOUND key (a 2-tuple) — a pure read whose only allocation
        // would be the probe tuple + champ_hash's worklist. Guards the shallow-compound champ_hash fast
        // path: hashing a 1-level compound key must NOT allocate the two worklist Vecs (only the probe
        // tuple node itself remains). Build a map of 2-tuple keys, then look each up with a fresh tuple.
        let ctuple = |a: i64, b: i64| -> Handle {
            let t = op_arr_alloc(2);
            op_arr_set(t, 0, op_box_int(a));
            op_arr_set(t, 1, op_box_int(b));
            t
        };
        let mut cm = op_map_empty();
        for k in 0..N {
            cm = op_map_insert(cm, ctuple(k, k + 1), op_box_int(k));
        }
        let clookup = measure(&mut || {
            for k in 0..N {
                let probe = ctuple(k, k + 1);
                let _ = op_map_lookup(cm, probe);
                op_drop(probe);
            }
        });
        println!("ALLOC map_lookup_tuplekey x{N}: {clookup}");
        // Each iteration allocates ONLY the probe tuple (arr node Box + its 2-slot handles Vec = 2);
        // BOTH the shallow-compound champ_hash fast path AND the shallow-compound champ_eq fast path add
        // NO worklist — so a hit costs exactly the probe. A regression to the general walk (hash and/or
        // eq) would add ~1-2 more per lookup. ~2000 for N=1000 = 2/lookup (the probe tuple).
        assert!(clookup <= 2500, "shallow-compound-key lookup x{N} allocs {clookup} exceeds ceiling 2500 (probe tuple only; shallow hash+eq fast paths add no worklist)");
        op_drop(cm);

        // (I) sum construction (Option/Result-shaped: disc in raw + payload handle) x1000. With the
        // inline `Raw`, the 4-byte disc no longer allocates a heap Vec — a sum node is now the node Box
        // + its 1-element handles Vec = 2 allocs/op (was 3: + the disc Vec). Guards the inline-raw win
        // for the sum path (the growing Option/Result usage).
        let sum = measure(&mut || {
            for k in 0..N {
                op_drop(op_sum_new(1, op_box_int(k)));
            }
        });
        println!("ALLOC sum_new x{N}: {sum}");
        assert!(sum <= 2200, "sum_new x{N} allocs {sum} exceeds ceiling 2200 (node Box + handles Vec; the disc is inline — was 3/op with a heap disc Vec)");

        // (J) bytes SLICE x1000 — a rope slice node over a shared leaf: 1 handle (the parent buf) +
        // the 8-byte `[off,len]` raw. With the inline `Raw` the `[off,len]` header no longer allocates
        // a transient heap Vec (`slice_raw` builds it inline), so a slice node is the node Box + its
        // 1-element handles Vec = 2 allocs/op (was 3: + the [off,len] Vec). Guards the inline-raw win
        // for the O(1)-no-copy bytes rope. The leaf is built + retained OUTSIDE the loop so we measure
        // only the slice node, not the leaf's construction.
        let leaf = {
            let b = op_bytes_alloc(16);
            for i in 0..16u32 {
                op_bytes_set(b, i, i);
            }
            b
        };
        let slice = measure(&mut || {
            for _ in 0..N {
                op_dup(leaf); // slice consumes a ref to its parent; keep the leaf alive across the batch
                op_drop(op_bytes_slice(leaf, 2, 8));
            }
        });
        println!("ALLOC bytes_slice x{N}: {slice}");
        assert!(slice <= 2200, "bytes_slice x{N} allocs {slice} exceeds ceiling 2200 (node Box + 1-elem handles Vec; the [off,len] raw is inline — was 3/op with a heap raw Vec)");
        op_drop(leaf);

        // (K) build a 2-tuple x1000 (`op_arr_alloc(2)` + two slot sets) — the common positional-product
        // constructor shared by tuples, records, and CHAMP `[k,v]` pairs. With scalar (immediate)
        // elements a tuple node is the node Box + its 2-element handles Vec = 2 allocs/op (empty raw = no
        // raw alloc, immediate elements = no element boxes). This is the tuple/record/pair construction
        // FLOOR under the current `Node.handles: Vec` layout — tracked so the pending inline-`handles`
        // lever (which targets exactly this ≤2-handle node) can be measured against it, and so a
        // regression in the arr-alloc/set path is visible.
        let tbuild = measure(&mut || {
            for k in 0..N {
                let t = op_arr_alloc(2);
                op_arr_set(t, 0, op_box_int(k));
                op_arr_set(t, 1, op_box_int(k + 1));
                op_drop(t);
            }
        });
        println!("ALLOC tuple2_build x{N}: {tbuild}");
        assert!(tbuild <= 2200, "tuple2_build x{N} allocs {tbuild} exceeds ceiling 2200 (node Box + 2-elem handles Vec; scalar elements are immediate, raw is empty — this is the ≤2-handle construction floor until handles inline)");
    }

    /// CPU-scaling PROBE (diagnostic, not a regression gate): times set ∩/∖ at growing N to reveal
    /// whether they are linear-ish or super-linear (the alloc bench can't see the O(log) contains-probe
    /// factor — evidence for whether the O(min) node-merge redesign is worth a future tick). Also times
    /// UNION over COMPOUND (tuple) elements, where hashing an element walks its whole subtree — this is
    /// what the `set_insert_h` hash-once change in `op_set_union` sped up (a scalar union can't show it,
    /// its element hash is O(1)). `#[ignore]`d, prints ns/element, no assertion.
    #[test]
    #[ignore] // diagnostic timing — run with --ignored --nocapture
    fn set_algebra_cpu_scaling_probe() {
        let build = |lo: i64, hi: i64| -> Handle {
            let mut s = op_set_empty();
            for k in lo..hi {
                s = op_set_insert(s, op_box_int(k));
            }
            s
        };
        for &n in &[1000i64, 4000, 16000, 64000] {
            let sa = build(0, n);
            let sb = build(n / 2, n + n / 2); // 50% overlap
            let reps = (64000 / n).max(1);
            let t0 = std::time::Instant::now();
            for _ in 0..reps {
                op_dup(sa);
                op_dup(sb);
                op_drop(op_set_intersection(sa, sb));
            }
            let inter_ns = t0.elapsed().as_nanos() as f64 / (reps as f64 * n as f64);
            let t1 = std::time::Instant::now();
            for _ in 0..reps {
                op_dup(sa);
                op_dup(sb);
                op_drop(op_set_difference(sa, sb));
            }
            let diff_ns = t1.elapsed().as_nanos() as f64 / (reps as f64 * n as f64);
            println!("SETSCALE n={n:>6}  ∩ {inter_ns:6.1} ns/elem   ∖ {diff_ns:6.1} ns/elem");
            op_drop(sa);
            op_drop(sb);
        }
        // Compound-element UNION: each element is a 3-deep nested tuple, so `champ_hash(e)` walks a real
        // subtree. Times union of a SMALL set into a LARGER base (the walk-the-smaller fold) — the case
        // the hash-once change targets. `n_small` elements are hashed once each now (was twice: probe +
        // the re-hash inside op_set_insert).
        let deep = |seed: i64| -> Handle {
            let inner = op_arr_alloc(2);
            op_arr_set(inner, 0, op_box_int(seed));
            op_arr_set(inner, 1, op_box_int(seed * 2));
            let outer = op_arr_alloc(2);
            op_arr_set(outer, 0, inner);
            op_arr_set(outer, 1, op_box_int(seed * 3));
            outer
        };
        let n_big = 4000i64;
        let n_small = 500i64;
        let mut big = op_set_empty();
        for k in 0..n_big {
            big = op_set_insert(big, deep(k));
        }
        let mut small = op_set_empty();
        for k in (n_big - n_small / 2)..(n_big + n_small / 2) {
            small = op_set_insert(small, deep(k)); // 50% overlap with big's tail
        }
        let reps = 40;
        let t = std::time::Instant::now();
        for _ in 0..reps {
            op_dup(big);
            op_dup(small);
            op_drop(op_set_union(big, small));
        }
        let union_ns = t.elapsed().as_nanos() as f64 / (reps as f64 * n_small as f64);
        println!("SETSCALE compound-union (small={n_small} into big={n_big})  {union_ns:6.1} ns/elem-walked");
        op_drop(big);
        op_drop(small);
    }

    /// The STATIC shape descriptor the compiler holds at each use site. There is no runtime type
    /// tag, so the renderer is driven ENTIRELY by this compile-time knowledge: the SAME heap node
    /// renders differently under different shapes (an `Arr[3,1]` is `(tuple 3 1)` under `Tuple` and
    /// `(list 3 1)` under `List`). This mirrors, in plain Rust, the type-directed renderer the
    /// compiler bakes into the emitted program.
    enum Shape {
        Int,
        Bool,
        Float,
        /// A fixed-arity positional product; empty = unit.
        Tuple(Vec<Shape>),
        /// A homogeneous, runtime-length sequence over one element shape.
        List(Box<Shape>),
        /// Named fields in positional order; names are compile-time constants.
        Record(Vec<(&'static str, Shape)>),
        /// Variants in discriminant order; the disc selects the name + payload shape.
        Sum(Vec<(&'static str, Shape)>),
        Bytes,
        Str,
    }

    /// A native mirror of the compiler-emitted, type-directed renderer. It walks a value through the
    /// runtime accessors EXACTLY as the emitted program will — reading scalars, `arr-len`/`arr-get`
    /// for sequences, `sum-disc`/`sum-payload` for sums, `bytes-*` for buffers — with the canonical
    /// name/keyword supplied by the static `Shape`, never by the runtime. This pins that the
    /// accessors are sufficient to render WITHOUT a runtime tag.
    /// Append the `b"…"` display escape of one byte — the same rules as the compiler's
    /// `escape_byte` and the exact inverse of the `b"…"` reader. Escape order is load-bearing:
    /// `\` and `"` sit inside the printable range, so they match before the passthrough arm.
    fn escape_byte(b: u32, out: &mut String) {
        match b {
            b if b == b'\n' as u32 => out.push_str("\\n"),
            b if b == b'\r' as u32 => out.push_str("\\r"),
            b if b == b'\t' as u32 => out.push_str("\\t"),
            b if b == b'\\' as u32 => out.push_str("\\\\"),
            b if b == b'"' as u32 => out.push_str("\\\""),
            0 => out.push_str("\\0"),
            0x20..=0x7e => out.push(b as u8 as char),
            _ => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push_str("\\x");
                out.push(HEX[((b >> 4) & 0xf) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }

    fn render(handle: Handle, shape: &Shape) -> String {
        match shape {
            Shape::Int => op_get_int(handle).to_string(),
            Shape::Bool => {
                if op_get_bool(handle) {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            Shape::Float => {
                let f = op_get_float(handle);
                // Whole floats keep a `.0` so their canonical text stays float-shaped.
                if f.is_finite() && f.fract() == 0.0 {
                    format!("{f:.1}")
                } else {
                    format!("{f}")
                }
            }
            Shape::Tuple(elems) => {
                if elems.is_empty() {
                    return "unit".into();
                }
                let mut out = String::from("(tuple");
                for (i, s) in elems.iter().enumerate() {
                    out.push(' ');
                    out.push_str(&render(op_arr_get(handle, i as u32), s));
                }
                out.push(')');
                out
            }
            Shape::List(elem) => {
                let n = op_arr_len(handle);
                let mut out = String::from("(list");
                for i in 0..n {
                    out.push(' ');
                    out.push_str(&render(op_arr_get(handle, i), elem));
                }
                out.push(')');
                out
            }
            Shape::Record(fields) => {
                let mut out = String::from("(record");
                for (i, (k, s)) in fields.iter().enumerate() {
                    out.push_str(&format!(" ({k} {})", render(op_arr_get(handle, i as u32), s)));
                }
                out.push(')');
                out
            }
            Shape::Sum(variants) => {
                let disc = op_sum_disc(handle) as usize;
                let (name, payload_shape) = &variants[disc];
                format!("({name} {})", render(op_sum_payload(handle), payload_shape))
            }
            Shape::Bytes => {
                // `b"…"` — the byte-string display form (matching the `bytes` crate's `Debug`, and
                // the exact inverse of the `b"…"` reader). Must agree byte-for-byte with the const
                // fold (`bytes_literal_text`) and the emitted-wasm renderer (`emit_byte_escape`).
                let n = op_bytes_len(handle);
                let mut out = String::from("b\"");
                for i in 0..n {
                    escape_byte(op_bytes_get(handle, i), &mut out);
                }
                out.push('"');
                out
            }
            Shape::Str => format!("\"{}\"", op_str_get(handle)),
        }
    }

    /// Read a node's refcount header directly (test-only). Immediate-aware: an immediate is not a
    /// Node, so `*h.0` would be UB — report the same non-1 sentinel `node_rc` does.
    fn rc_of(h: Handle) -> u32 {
        if is_immediate(h) {
            return 2;
        }
        unsafe { (*h.0).rc }
    }

    /// Test-only: is the node's raw payload HEAP-backed (spilled) rather than inline? Used to assert the
    /// reuse constructors normalize a reused shell's raw back to inline (a fresh constructor's rep).
    fn raw_is_heap(h: Handle) -> bool {
        if is_immediate(h) {
            return false;
        }
        matches!(unsafe { &(*h.0).raw }, Raw::Heap(_))
    }

    /// A DEFINITELY-BOXED int leaf (test-only): bypasses `op_box_int`'s P2 normalize so the RC /
    /// reuse / cascade tests keep exercising a real heap Node with rc == 1 (a small `op_box_int(v)`
    /// now inlines and would make those node-count / drop-a-leaf scenarios vacuous). Byte-identical
    /// to the pre-P2 boxed representation, so `op_get_int` decodes the same value through `with_node`.
    fn boxed_int_leaf(v: i64) -> Handle {
        alloc(Vec::new(), (v as u64).to_le_bytes().to_vec())
    }

    // ── Inline tagged-immediate helpers (producers: op_box_int fixnum / op_box_bool / op_arr_alloc(0)) ─────

    #[test]
    fn imm_encoding_roundtrip() {
        reset();
        // unit
        let u = imm_unit();
        assert!(is_immediate(u));
        assert!(matches!(imm_kind(u), ImmKind::Unit));
        // bools
        for b in [true, false] {
            let h = imm_bool(b);
            assert!(is_immediate(h));
            assert!(matches!(imm_kind(h), ImmKind::Bool));
            assert_eq!(imm_as_bool(h), b);
        }
        // ints across the window incl. boundaries
        for v in [FIXNUM_MIN, FIXNUM_MAX, 0i64, -1, 1, 42, -42, 536_870_910] {
            assert!(fixnum_fits(v), "expected {v} to fit the fixnum window");
            let h = imm_int(v);
            assert!(is_immediate(h), "imm_int({v}) should be immediate");
            assert!(matches!(imm_kind(h), ImmKind::Int));
            assert_eq!(imm_as_int(h), v, "imm_int/imm_as_int round-trip for {v}");
        }
        // the discriminator must NOT misfire on a real pointer or NULL
        let real = alloc(Vec::new(), 7i64.to_le_bytes().to_vec());
        assert!(!is_immediate(real), "a real alloc'd Node must not read as immediate");
        assert!(!is_immediate(Handle::NULL), "NULL is tag 00 → not immediate");
        op_drop(real);
    }

    #[test]
    fn imm_int_out_of_window_not_fits() {
        reset();
        assert!(!fixnum_fits(FIXNUM_MAX + 1), "2^29 must not fit");
        assert!(!fixnum_fits(FIXNUM_MIN - 1), "-(2^29)-1 must not fit");
        // sanity: the edges themselves do fit
        assert!(fixnum_fits(FIXNUM_MAX));
        assert!(fixnum_fits(FIXNUM_MIN));
    }

    #[test]
    fn imm_rc_ops_are_noops() {
        reset();
        let before = live_object_count();
        for h in [imm_unit(), imm_bool(true), imm_bool(false), imm_int(5), imm_int(-7)] {
            // node_rc MUST be the non-1 sentinel (never 1 → no FBIP in-place mutation of a non-Node)
            assert_eq!(node_rc(h), 2, "node_rc(immediate) must be 2, not 1");
            // dup/drop must not crash and must not touch the allocator
            op_dup(h);
            op_dup(h);
            op_drop(h);
            op_drop(h);
            // reset yields NULL (nothing to reuse), and the reuse ctors fall back to fresh alloc
            assert_eq!(op_reset(h), Handle::NULL);
        }
        assert_eq!(
            live_object_count(),
            before,
            "dup/drop/reset of immediates must not change the live-node count"
        );
    }

    #[test]
    fn imm_to_u32_roundtrip() {
        reset();
        // The wasm32 ABI (`to_u32`/`from_u32`) is identity casts through a 32-bit `usize`, so it
        // preserves the low tag bits exactly. Reproduce that projection here (`.0 as u32` then back);
        // on wasm32 this IS `from_u32(to_u32(h))`. The round-trip must stay the SAME immediate:
        // is_immediate, same kind, same decoded value.
        let cases: &[Handle] = &[
            imm_unit(),
            imm_bool(true),
            imm_bool(false),
            imm_int(0),
            imm_int(-1),
            imm_int(1),
            imm_int(FIXNUM_MAX),
            imm_int(FIXNUM_MIN),
        ];
        for &h in cases {
            let round = Handle((h.0 as usize as u32) as usize as *mut Node);
            assert!(is_immediate(round), "round-tripped handle must still be immediate");
            assert_eq!(
                std::mem::discriminant(&imm_kind(round)),
                std::mem::discriminant(&imm_kind(h)),
                "u32 ABI round-trip must preserve the immediate kind"
            );
            match imm_kind(h) {
                ImmKind::Int => assert_eq!(imm_as_int(round), imm_as_int(h)),
                ImmKind::Bool => assert_eq!(imm_as_bool(round), imm_as_bool(h)),
                ImmKind::Unit => {}
            }
        }
        // For values whose encoding fits in the low 32 bits with no sign extension into the host
        // pointer's high half (unit, bool, non-negative fixnums), the raw handle bits are identical —
        // exactly the wasm32 case where `usize` is 32-bit.
        for &h in &[imm_unit(), imm_bool(true), imm_bool(false), imm_int(0), imm_int(1), imm_int(FIXNUM_MAX)] {
            let round = Handle((h.0 as usize as u32) as usize as *mut Node);
            assert_eq!(round, h, "u32 ABI round-trip must be bit-identical for low-32-bit immediates");
        }
    }

    // ── Inline unit + bool: SHARED-REPRESENTATION payoff (P1b flips the producers) ────────

    #[test]
    fn producers_normalize_to_immediates() {
        reset();
        // Normalize-on-construct: a bool / unit value is now ALWAYS inline, never a boxed Node.
        assert!(is_immediate(op_box_bool(true)));
        assert!(is_immediate(op_box_bool(false)));
        assert!(is_immediate(op_arr_alloc(0)), "empty array (unit) must inline");
        // Since P2 a small in-window int ALSO inlines (op_box_int normalizes); an out-of-window int
        // still boxes, and a non-empty array still allocates.
        assert!(is_immediate(op_box_int(5)), "an in-window int inlines since P2");
        assert!(!is_immediate(op_box_int((1 << 30) as i64)), "an out-of-window int still boxes");
        let a = op_arr_alloc(2);
        assert!(!is_immediate(a));
        op_drop(a);
    }

    #[test]
    fn inline_bool_in_tuple_roundtrips() {
        reset();
        let before = live_nodes();
        // A 2-tuple (bool, small-int). Since P2 BOTH the bool AND the in-window int ride inline in
        // their slots → the ONLY node is the array itself (the P2 allocation win over the boxed era,
        // which would have been 3 nodes: array + boxed bool + boxed int).
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, op_box_bool(true));
        op_arr_set(t, 1, op_box_int(9));
        assert_eq!(
            live_nodes(),
            before + 1,
            "tuple(bool,small-int) = just the array node; both scalars ride inline in their slots"
        );
        // Project both elements back — the inline bool/int decode correctly through op_arr_get.
        assert!(op_get_bool(op_arr_get(t, 0)));
        assert_eq!(op_get_int(op_arr_get(t, 1)), 9);
        // Render matches the boxed-era text exactly.
        assert_eq!(
            render(t, &Shape::Tuple(vec![Shape::Bool, Shape::Int])),
            "(tuple true 9)"
        );
        op_drop(t);
        assert_eq!(live_nodes(), before, "array reclaimed; both inline scalars leave nothing");
    }

    #[test]
    fn inline_bool_as_map_set_key() {
        reset();
        // A CHAMP MAP keyed by a bool immediate: insert, look up, and confirm canonical-form equality
        // and hashing flow through the inline path (champ_hash/eq fold imm_canonical_raw).
        let m0 = op_map_empty();
        let m1 = op_map_insert(m0, op_box_bool(true), op_box_int(1));
        let m2 = op_map_insert(m1, op_box_bool(false), op_box_int(2));
        assert_eq!(op_get_int(op_map_lookup(m2, op_box_bool(true))), 1);
        assert_eq!(op_get_int(op_map_lookup(m2, op_box_bool(false))), 2);
        // A bool key hashes/compares equal to itself through the immediate arms.
        assert_eq!(champ_hash(op_box_bool(true)), champ_hash(op_box_bool(true)));
        assert!(champ_eq(op_box_bool(true), op_box_bool(true)));
        // Distinct bool immediates are NOT equal and (correctly) differ.
        assert!(!champ_eq(op_box_bool(true), op_box_bool(false)));
        op_drop(m2);

        // A SET with bool elements: contains returns correctly for both, false for neither-present is n/a.
        let s0 = op_set_empty();
        let s1 = op_set_insert(s0, op_box_bool(true));
        let s2 = op_set_insert(s1, op_box_bool(false));
        assert!(op_set_contains(s2, op_box_bool(true)));
        assert!(op_set_contains(s2, op_box_bool(false)));
        // Idempotent: re-inserting an existing bool element leaves size unchanged.
        assert_eq!(op_set_size(s2), 2);
        let s3 = op_set_insert(s2, op_box_bool(true));
        assert_eq!(op_set_size(s3), 2);
        op_drop(s3);
    }

    #[test]
    fn inline_unit_in_container() {
        reset();
        // Unit as a tuple element, a list element, and a sum payload — each round-trips and renders
        // identically to the pre-P1b boxed-empty-array form.
        let t = op_arr_alloc(1);
        op_arr_set(t, 0, op_arr_alloc(0)); // unit element (inline)
        assert_eq!(render(t, &Shape::Tuple(vec![Shape::Tuple(vec![])])), "(tuple unit)");
        assert_eq!(op_arr_len(op_arr_get(t, 0)), 0, "the inline unit element has 0 slots");
        op_drop(t);

        // A nullary variant carrying unit renders "(None unit)" as before.
        let none = op_sum_new(0, op_arr_alloc(0));
        assert_eq!(op_sum_disc(none), 0);
        assert_eq!(op_arr_len(op_sum_payload(none)), 0);
        op_drop(none);
    }

    #[test]
    fn inline_bool_renders_identically() {
        reset();
        // Byte-identical to the strings the pre-P1b boxed producers rendered (see bool_round_trip,
        // empty_arr_is_unit).
        assert_eq!(render(op_box_bool(true), &Shape::Bool), "true");
        assert_eq!(render(op_box_bool(false), &Shape::Bool), "false");
        assert_eq!(render(op_arr_alloc(0), &Shape::Tuple(vec![])), "unit");
    }

    #[test]
    fn inline_bool_list_no_leak() {
        reset();
        let before = live_nodes();
        // A runtime list (32-way trie) of bools: the trie spine allocates, but every bool ELEMENT is
        // inline. Building and dropping it must leave LIVE_NODES balanced — immediates don't leak and
        // their dup/drop (performed by the trie's structural sharing) are no-ops.
        let mut v = op_vec_empty();
        for i in 0..64 {
            v = op_vec_push(v, op_box_bool(i % 2 == 0));
        }
        assert_eq!(op_vec_len(v), 64);
        // Read a few back through the inline decode path.
        assert!(op_get_bool(op_vec_get(v, 0)));
        assert!(!op_get_bool(op_vec_get(v, 1)));
        op_drop(v);
        assert_eq!(live_nodes(), before, "list-of-bools fully reclaimed; inline bools leave nothing");
    }

    // ── Inline small ints: the fixnum window (P2 flips op_box_int) ────────────────────────

    #[test]
    fn op_box_int_normalizes_at_boundary() {
        reset();
        // A value that FITS the window is ALWAYS inline; just outside, it boxes. THE single boundary.
        assert!(is_immediate(op_box_int(FIXNUM_MAX)), "FIXNUM_MAX inlines");
        assert!(!is_immediate(op_box_int(FIXNUM_MAX + 1)), "FIXNUM_MAX+1 boxes");
        assert!(is_immediate(op_box_int(FIXNUM_MIN)), "FIXNUM_MIN inlines");
        assert!(!is_immediate(op_box_int(FIXNUM_MIN - 1)), "FIXNUM_MIN-1 boxes");
        assert!(is_immediate(op_box_int(0)));
        assert!(is_immediate(op_box_int(-1)));
        assert!(is_immediate(op_box_int(1)));
        // Every value round-trips through op_get_int exactly — inline OR boxed.
        for v in [
            FIXNUM_MIN,
            FIXNUM_MAX,
            FIXNUM_MIN - 1,
            FIXNUM_MAX + 1,
            0,
            -1,
            1,
            42,
            -42,
            i64::MAX,
            i64::MIN,
        ] {
            let h = op_box_int(v);
            assert_eq!(op_get_int(h), v, "op_box_int/op_get_int round-trip for {v}");
            if !is_immediate(h) {
                op_drop(h); // reclaim the boxed ones
            }
        }
    }

    #[test]
    fn inline_int_negative_behavioral_roundtrip() {
        reset();
        // NOTE (P1a/P1b gotcha 1): on a 64-bit native host `imm_int(v<0)` sign-extends into the
        // pointer's high 32 bits, so a RAW-BIT u32 round-trip would differ on native. We therefore
        // assert BEHAVIORAL identity — op_get_int decodes the right value and imm_kind == Int — which
        // is what the wasm32 ABI (32-bit usize) preserves bit-for-bit anyway.
        for v in [-1i64, -2, -42, -1000, FIXNUM_MIN, FIXNUM_MIN + 1, -(1 << 20)] {
            let h = op_box_int(v);
            assert!(is_immediate(h), "in-window negative {v} must inline");
            assert!(matches!(imm_kind(h), ImmKind::Int), "negative fixnum classifies as Int");
            assert_eq!(op_get_int(h), v, "negative fixnum decodes to the right value");
            // to_u32/from_u32 BEHAVIORAL round-trip (reproduce the wasm32 projection: .0 as u32 back):
            let round = Handle((h.0 as usize as u32) as usize as *mut Node);
            assert!(is_immediate(round));
            assert_eq!(imm_as_int(round), v, "negative fixnum survives the u32 ABI projection by value");
        }
    }

    #[test]
    fn inline_int_as_map_set_key() {
        reset();
        // Small ints as CHAMP map KEYS: normalize means the key is ALWAYS inline (no boxed twin can
        // exist), and champ_hash/eq fold the SAME `(v as u64).to_le_bytes()` a boxed int would carry.
        assert!(is_immediate(op_box_int(7)), "an in-window key can never arrive boxed");
        let m0 = op_map_empty();
        let m1 = op_map_insert(m0, op_box_int(7), op_box_int(70));
        let m2 = op_map_insert(m1, op_box_int(-3), op_box_int(-30));
        assert_eq!(op_get_int(op_map_lookup(m2, op_box_int(7))), 70);
        assert_eq!(op_get_int(op_map_lookup(m2, op_box_int(-3))), -30);
        // Two identical small-int keys hash/compare equal through the immediate arms.
        assert_eq!(champ_hash(op_box_int(7)), champ_hash(op_box_int(7)));
        assert!(champ_eq(op_box_int(7), op_box_int(7)));
        assert!(!champ_eq(op_box_int(7), op_box_int(8)));
        op_drop(m2);

        // A SET of small ints: contains, idempotent size.
        let s0 = op_set_empty();
        let s1 = op_set_insert(s0, op_box_int(1));
        let s2 = op_set_insert(s1, op_box_int(2));
        assert!(op_set_contains(s2, op_box_int(1)));
        assert!(op_set_contains(s2, op_box_int(2)));
        assert!(!op_set_contains(s2, op_box_int(3)));
        assert_eq!(op_set_size(s2), 2);
        let s3 = op_set_insert(s2, op_box_int(1)); // re-insert existing
        assert_eq!(op_set_size(s3), 2);
        op_drop(s3);
    }

    #[test]
    fn inline_int_hashes_equal_to_boxed_twin() {
        reset();
        // Canonical-form belt-and-suspenders (open-Q#8): an inline int and a HAND-BOXED twin of the
        // same value hash and compare EQUAL and render identically, so an older boxed stable-binary
        // value stays interoperable with the inline rep.
        let inline = op_box_int(3);
        let boxed = boxed_int_leaf(3);
        assert!(is_immediate(inline) && !is_immediate(boxed));
        assert_eq!(champ_hash(inline), champ_hash(boxed), "inline and boxed int hash equal");
        assert!(champ_eq(inline, boxed), "inline and boxed int compare equal");
        assert_eq!(render(inline, &Shape::Int), render(boxed, &Shape::Int));
        assert_eq!(render(inline, &Shape::Int), "3");
        op_drop(boxed);
    }

    #[test]
    fn inline_int_totality_no_ub() {
        reset();
        // Feed an int immediate to every cross-kind reader: each returns its documented default with
        // no crash / UB (the P1a guards, now exercised by a real immediate). A float/str/sum/bytes is
        // never itself an immediate, so these are pure totality defaults.
        let i = op_box_int(9);
        assert!(is_immediate(i));
        assert_eq!(op_get_float(i), 0.0);
        assert!(!op_get_bool(i)); // decodes bit[4] of the int's tag; total, never traps
        assert_eq!(op_str_get(i), "");
        assert_eq!(op_arr_len(i), 0);
        assert_eq!(op_sum_disc(i), 0);
        assert_eq!(op_sum_payload(i), Handle::NULL);
        assert_eq!(op_bytes_len(i), 0);
        assert_eq!(op_bytes_get(i, 0), 0);
    }

    #[test]
    fn inline_int_in_container_node_win() {
        reset();
        let before = live_nodes();
        // A tuple of 3 small ints: since P2 every element rides inline → the ONLY node is the array.
        // (Boxed era: 1 array + 3 int leaves = 4 nodes.)
        let t = op_arr_alloc(3);
        op_arr_set(t, 0, op_box_int(10));
        op_arr_set(t, 1, op_box_int(-20));
        op_arr_set(t, 2, op_box_int(30));
        assert_eq!(live_nodes(), before + 1, "tuple of small ints = just the array node");
        assert_eq!(op_get_int(op_arr_get(t, 0)), 10);
        assert_eq!(op_get_int(op_arr_get(t, 1)), -20);
        assert_eq!(op_get_int(op_arr_get(t, 2)), 30);
        assert_eq!(render(t, &Shape::List(Box::new(Shape::Int))), "(list 10 -20 30)");
        op_drop(t);
        assert_eq!(live_nodes(), before, "array reclaimed; inline ints leave nothing");
    }

    #[test]
    fn mixed_window_list() {
        reset();
        let before = live_nodes();
        // A container holding BOTH in-window (inline) and out-of-window (boxed) ints. The two
        // representations must coexist correctly in one container: get/len/render all correct, and
        // only the out-of-window elements cost a node. (A flat positional array is the shape `render`
        // walks under `Shape::List`; the trie is exercised separately via op_vec_* elsewhere.)
        let big1 = (1i64 << 30) + 5; // out of window → boxed
        let big2 = -(1i64 << 31); // out of window → boxed
        let values = [0i64, big1, -7, big2, FIXNUM_MAX, 42];
        let a = op_arr_alloc(values.len() as u32);
        for (i, &x) in values.iter().enumerate() {
            op_arr_set(a, i as u32, op_box_int(x));
        }
        // len via the array accessor; get + decode per element (inline and boxed transparently).
        assert_eq!(op_arr_len(a), values.len() as u32);
        for (i, &x) in values.iter().enumerate() {
            assert_eq!(op_get_int(op_arr_get(a, i as u32)), x, "element {i} = {x} reads back exactly");
        }
        // Exactly TWO elements (big1, big2) are boxed nodes; the array shell is the third node. The
        // four in-window ints ride inline — the P2 win, mid-container, alongside boxed neighbors.
        assert_eq!(live_nodes(), before + 3, "array shell + the 2 out-of-window boxed ints only");
        // Render walks every element under Shape::Int, mixing inline and boxed transparently.
        assert_eq!(
            render(a, &Shape::List(Box::new(Shape::Int))),
            format!("(list 0 {big1} -7 {big2} {FIXNUM_MAX} 42)")
        );
        op_drop(a);
        assert_eq!(live_nodes(), before, "whole mixed container reclaimed; inline elems leak nothing");
    }

    // NOTE (serializer / value-interchange): the runtime crate has NO value-interchange / Ast
    // serialization path that reads `node.raw` — the only value-observing surfaces are `render`
    // (covered above: inline and boxed ints render identically) and the `to_u32`/`from_u32` ABI
    // (identity casts, covered by the ABI round-trip tests). Ast encode/decode lives in
    // `cdz-compiler/src/ast.rs` over the compiler's syntax `Node`, never a runtime `Handle`, so there
    // is nothing serialization-shaped to test from here. Flagged as a cross-boundary review item.

    // ── Latent-hardening (review follow-ups): reuse-to-0 normalize + defensive guard set ──

    #[test]
    fn arr_alloc_reuse_zero_yields_imm_unit() {
        reset();
        let before = live_nodes();
        // Build a unique tuple, reset it to a reuse token (rc==1 childless shell), then refit to
        // len 0. The result MUST be the canonical inline unit — never a boxed empty node (which would
        // fork the unit rep) — and the token shell must be FREED, not leaked.
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, boxed_int_leaf(1));
        op_arr_set(t, 1, boxed_int_leaf(2));
        let token = op_reset(t); // frees the 2 children, retains the shell (1 node live)
        assert_ne!(token, Handle::NULL, "unique reset yields a token");
        assert_eq!(live_nodes(), before + 1, "just the retained shell is live");

        let u = op_arr_alloc_reuse(0, token);
        assert!(is_immediate(u), "reuse-to-0 must return an inline unit, never a boxed empty node");
        assert!(matches!(imm_kind(u), ImmKind::Unit));
        // Byte-identical (structurally) to the normal unit producer.
        assert!(champ_eq(u, op_arr_alloc(0)), "reuse-to-0 unit == op_arr_alloc(0) unit");
        // The token node was reclaimed — no leak, no boxed twin left behind.
        assert_eq!(live_nodes(), before, "the token shell is freed, not leaked");
    }

    #[test]
    fn bytes_map_ops_immediate_safe() {
        reset();
        // Defensive proof (mirrors the P1a inert-guard proof): no real code passes an immediate to
        // these mutators/readers today, so this directly unit-tests the guards. Each must return its
        // benign default and NOT crash / deref the tagged bits.
        let imm = op_box_int(5); // an immediate (inline fixnum)
        assert!(is_immediate(imm));
        let before = live_nodes();
        // op_bytes_set on an immediate → returns the handle unchanged (no-op write).
        assert_eq!(op_bytes_set(imm, 0, 0xAB), imm);
        // op_map_set on an immediate → returns the map handle unchanged.
        assert_eq!(op_map_set(imm, 0, op_box_int(1), op_box_int(2)), imm);
        // op_map_key / op_map_val on an immediate → benign NULL (like a null-in read).
        assert_eq!(op_map_key(imm, 0), Handle::NULL);
        assert_eq!(op_map_val(imm, 0), Handle::NULL);
        // No allocation, no free, no crash — the guards are inert on an immediate.
        assert_eq!(live_nodes(), before, "immediate-safe ops touch no heap");
    }

    // ── Scalars ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn int_round_trip() {
        reset();
        for v in [0i64, 42, -42, i64::MAX, i64::MIN] {
            assert_eq!(op_get_int(op_box_int(v)), v);
        }
        assert_eq!(render(op_box_int(0), &Shape::Int), "0");
        assert_eq!(render(op_box_int(-42), &Shape::Int), "-42");
        assert_eq!(render(op_box_int(i64::MAX), &Shape::Int), "9223372036854775807");
        assert_eq!(render(op_box_int(i64::MIN), &Shape::Int), "-9223372036854775808");
    }

    #[test]
    fn bool_round_trip() {
        reset();
        assert!(op_get_bool(op_box_bool(true)));
        assert!(!op_get_bool(op_box_bool(false)));
        assert_eq!(render(op_box_bool(true), &Shape::Bool), "true");
        assert_eq!(render(op_box_bool(false), &Shape::Bool), "false");
    }

    #[test]
    fn float_round_trip() {
        reset();
        for v in [0.0f64, 3.14, -2.5, 2.0, -100.0] {
            assert_eq!(op_get_float(op_box_float(v)), v);
        }
        // Fractional keeps its digits; whole number keeps a trailing `.0`.
        assert_eq!(render(op_box_float(3.14), &Shape::Float), "3.14");
        assert_eq!(render(op_box_float(2.0), &Shape::Float), "2.0");
    }

    // ── Arr (tuple / record / list) ───────────────────────────────────────────────────────

    #[test]
    fn empty_arr_is_unit() {
        reset();
        let a = op_arr_alloc(0);
        assert_eq!(op_arr_len(a), 0);
        assert_eq!(render(a, &Shape::Tuple(vec![])), "unit");
    }

    #[test]
    fn arr_two_elements() {
        reset();
        let a = op_arr_alloc(2);
        assert_eq!(op_arr_set(a, 0, op_box_int(3)), a); // arr-set returns the array handle
        op_arr_set(a, 1, op_box_int(1));
        assert_eq!(op_arr_len(a), 2);
        assert_eq!(op_get_int(op_arr_get(a, 0)), 3);
        assert_eq!(op_get_int(op_arr_get(a, 1)), 1);
    }

    #[test]
    fn same_bytes_different_render() {
        reset();
        // The load-bearing demonstration: identical heap node, DIFFERENT canonical text, chosen
        // entirely by the compiler-held static shape — no runtime tag involved.
        let a = op_arr_alloc(2);
        op_arr_set(a, 0, op_box_int(3));
        op_arr_set(a, 1, op_box_int(1));
        assert_eq!(
            render(a, &Shape::Tuple(vec![Shape::Int, Shape::Int])),
            "(tuple 3 1)"
        );
        assert_eq!(render(a, &Shape::List(Box::new(Shape::Int))), "(list 3 1)");
        assert_eq!(
            render(a, &Shape::Record(vec![("x", Shape::Int), ("y", Shape::Int)])),
            "(record (x 3) (y 1))"
        );
    }

    #[test]
    fn arr_mixed_element_types() {
        reset();
        let a = op_arr_alloc(2);
        op_arr_set(a, 0, op_box_int(42));
        op_arr_set(a, 1, op_box_bool(true));
        assert_eq!(
            render(a, &Shape::Tuple(vec![Shape::Int, Shape::Bool])),
            "(tuple 42 true)"
        );
    }

    #[test]
    fn nested_arr() {
        reset();
        // (tuple 1 (tuple 2 3)) — an arr whose element is itself an arr handle.
        let inner = op_arr_alloc(2);
        op_arr_set(inner, 0, op_box_int(2));
        op_arr_set(inner, 1, op_box_int(3));
        let outer = op_arr_alloc(2);
        op_arr_set(outer, 0, op_box_int(1));
        op_arr_set(outer, 1, inner);
        let shape = Shape::Tuple(vec![Shape::Int, Shape::Tuple(vec![Shape::Int, Shape::Int])]);
        assert_eq!(render(outer, &shape), "(tuple 1 (tuple 2 3))");
    }

    #[test]
    fn empty_list_renders() {
        reset();
        assert_eq!(
            render(op_arr_alloc(0), &Shape::List(Box::new(Shape::Int))),
            "(list)"
        );
    }

    // ── Sum ───────────────────────────────────────────────────────────────────────────────

    #[test]
    fn sum_round_trip() {
        reset();
        // A two-variant option-like sum: variant 0 = None (nullary), variant 1 = Some(Int).
        let payload = op_box_int(7);
        let some = op_sum_new(1, payload);
        assert_eq!(op_sum_disc(some), 1);
        assert_eq!(op_sum_payload(some), payload);
        assert_eq!(op_get_int(op_sum_payload(some)), 7);

        let variants = || {
            Shape::Sum(vec![
                ("None", Shape::Tuple(vec![])),
                ("Some", Shape::Int),
            ])
        };
        assert_eq!(render(some, &variants()), "(Some 7)");

        // disc 0 with an empty-arr payload = a nullary variant carrying unit.
        let none = op_sum_new(0, op_arr_alloc(0));
        assert_eq!(op_sum_disc(none), 0);
        assert_eq!(render(none, &variants()), "(None unit)");
    }

    // ── Bytes ───────────────────────────────────────────────────────────────────────────────

    #[test]
    fn bytes_empty() {
        reset();
        let b = op_bytes_alloc(0);
        assert_eq!(op_bytes_len(b), 0);
        assert_eq!(render(b, &Shape::Bytes), "b\"\"");
    }

    #[test]
    fn bytes_round_trip() {
        reset();
        let b = op_bytes_alloc(3);
        assert_eq!(op_bytes_set(b, 0, 1), b); // bytes-set returns the buffer handle
        op_bytes_set(b, 1, 2);
        op_bytes_set(b, 2, 255);
        assert_eq!(op_bytes_len(b), 3);
        assert_eq!(op_bytes_get(b, 0), 1);
        assert_eq!(op_bytes_get(b, 1), 2);
        assert_eq!(op_bytes_get(b, 2), 255);
        // Non-printable bytes render as `\xNN` (lowercase, matching the `bytes` crate's `Debug`).
        assert_eq!(render(b, &Shape::Bytes), "b\"\\x01\\x02\\xff\"");
    }

    #[test]
    fn bytes_mixed_printable_and_escapes() {
        reset();
        // A mix of printable ASCII, a special escape, and a hex byte: "AB", newline, 0xff.
        let src: [u8; 4] = [b'A', b'B', b'\n', 0xff];
        let b = op_bytes_alloc(src.len() as u32);
        for (i, &v) in src.iter().enumerate() {
            op_bytes_set(b, i as u32, v as u32);
        }
        assert_eq!(render(b, &Shape::Bytes), "b\"AB\\n\\xff\"");
    }

    #[test]
    fn bytes_escapes_quote_backslash_null() {
        reset();
        // `"`, `\`, NUL must escape; the PNG magic 0x89 is a hex byte.
        let src: [u8; 4] = [b'"', b'\\', 0x00, 0x89];
        let b = op_bytes_alloc(src.len() as u32);
        for (i, &v) in src.iter().enumerate() {
            op_bytes_set(b, i as u32, v as u32);
        }
        assert_eq!(render(b, &Shape::Bytes), "b\"\\\"\\\\\\0\\x89\"");
    }

    // ── String ──────────────────────────────────────────────────────────────────────────────

    #[test]
    fn str_round_trip() {
        reset();
        for s in ["", "hello", "héllo☃"] {
            assert_eq!(op_str_get(op_str_new(s.to_string())), s);
        }
        assert_eq!(render(op_str_new("".to_string()), &Shape::Str), "\"\"");
        assert_eq!(render(op_str_new("hello".to_string()), &Shape::Str), "\"hello\"");
        assert_eq!(render(op_str_new("héllo☃".to_string()), &Shape::Str), "\"héllo☃\"");
    }

    // ── Map ─────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn map_empty() {
        reset();
        let m = op_map_alloc(0);
        assert_eq!(op_map_len(m), 0);
    }

    #[test]
    fn map_round_trip() {
        reset();
        // { "a" -> 1, "b" -> 2 } as positional pairs; stored verbatim, no sort/dedup.
        let ka = op_str_new("a".to_string());
        let va = op_box_int(1);
        let kb = op_str_new("b".to_string());
        let vb = op_box_int(2);
        let m = op_map_alloc(2);
        assert_eq!(op_map_set(m, 0, ka, va), m); // map-set returns the map handle
        op_map_set(m, 1, kb, vb);
        assert_eq!(op_map_len(m), 2);
        assert_eq!(op_str_get(op_map_key(m, 0)), "a");
        assert_eq!(op_get_int(op_map_val(m, 0)), 1);
        assert_eq!(op_str_get(op_map_key(m, 1)), "b");
        assert_eq!(op_get_int(op_map_val(m, 1)), 2);
    }

    // ── Compound-of-compound: a record containing a list, a sum, bytes, and a string ──────────

    #[test]
    fn deeply_nested_render() {
        reset();
        // (record (xs (list 1 2)) (tag (Some 9)) (raw b"\x07") (name "hi"))
        let xs = op_arr_alloc(2);
        op_arr_set(xs, 0, op_box_int(1));
        op_arr_set(xs, 1, op_box_int(2));
        let tag = op_sum_new(1, op_box_int(9));
        let raw = op_bytes_alloc(1);
        op_bytes_set(raw, 0, 7);
        let name = op_str_new("hi".to_string());
        let rec = op_arr_alloc(4);
        op_arr_set(rec, 0, xs);
        op_arr_set(rec, 1, tag);
        op_arr_set(rec, 2, raw);
        op_arr_set(rec, 3, name);
        let shape = Shape::Record(vec![
            ("xs", Shape::List(Box::new(Shape::Int))),
            ("tag", Shape::Sum(vec![("None", Shape::Tuple(vec![])), ("Some", Shape::Int)])),
            ("raw", Shape::Bytes),
            ("name", Shape::Str),
        ]);
        assert_eq!(
            render(rec, &shape),
            "(record (xs (list 1 2)) (tag (Some 9)) (raw b\"\\x07\") (name \"hi\"))"
        );
    }

    // ── Birth refcount: every node is born with refcount 1 ────────────────────────────────────

    #[test]
    fn node_born_with_refcount_one() {
        reset();
        // Definitely-boxed values are born with rc == 1. `op_box_int(5)` now INLINES (no Node), so
        // use a genuinely-boxed leaf / an out-of-window int to exercise the heap birth-refcount.
        assert_eq!(rc_of(boxed_int_leaf(5)), 1);
        assert_eq!(rc_of(op_box_int((1 << 30) as i64)), 1, "out-of-window int boxes");
        assert_eq!(rc_of(op_arr_alloc(2)), 1);
        assert_eq!(rc_of(op_sum_new(0, op_arr_alloc(0))), 1);
        assert_eq!(rc_of(op_str_new("x".to_string())), 1);
        // An immediate is NOT a Node and must never report rc == 1 (a 1 would let an FBIP in-place
        // path mutate a non-Node) — the P2 canonical-form invariant.
        assert_ne!(rc_of(op_box_int(5)), 1, "an inline int must not look uniquely-owned");
        assert_ne!(rc_of(op_box_bool(true)), 1);
        assert_ne!(rc_of(op_arr_alloc(0)), 1);
    }

    // ── Tagless totality: scalar/null reads are total; OOB into a valid node traps ────────────

    #[test]
    fn scalar_reads_are_total_and_never_trap() {
        reset();
        // No stored tag ⇒ a `get-*` on a node of another kind REINTERPRETS raw bytes: deterministic,
        // possibly garbage (a compiler bug), but crucially TOTAL — it must not panic/trap.
        let i = op_box_int(9);
        let _ = op_get_bool(i); // reinterprets the low byte
        let _ = op_get_float(i); // reinterprets the 8 bytes as an f64
        let _ = op_sum_disc(i); // reinterprets the low 4 bytes as a discriminant
        // Structural reads that genuinely have nothing to return yield a benign default, not a trap.
        assert_eq!(op_arr_len(i), 0); // an Int owns no handles
        assert_eq!(op_sum_payload(i), Handle::NULL); // no handle to hand back
    }

    #[test]
    fn null_handle_reads_are_benign() {
        reset();
        // The benign-default sentinel: reading a null handle never faults, even with an index.
        assert_eq!(op_get_int(Handle::NULL), 0);
        assert!(!op_get_bool(Handle::NULL));
        assert_eq!(op_get_float(Handle::NULL), 0.0);
        assert_eq!(op_arr_len(Handle::NULL), 0);
        assert_eq!(op_arr_get(Handle::NULL, 99), Handle::NULL); // null + OOB index is still benign
        assert_eq!(op_sum_disc(Handle::NULL), 0);
        assert_eq!(op_sum_payload(Handle::NULL), Handle::NULL);
        assert_eq!(op_bytes_len(Handle::NULL), 0);
        assert_eq!(op_bytes_get(Handle::NULL, 99), 0);
        assert_eq!(op_str_get(Handle::NULL), "");
        assert_eq!(op_map_len(Handle::NULL), 0);
        assert_eq!(op_map_key(Handle::NULL, 99), Handle::NULL);
        assert_eq!(op_map_val(Handle::NULL, 99), Handle::NULL);
    }

    #[test]
    #[should_panic]
    fn arr_get_oob_into_valid_node_traps() {
        reset();
        let a = op_arr_alloc(2);
        let _ = op_arr_get(a, 5); // fail-fast: OOB index into a valid array
    }

    #[test]
    #[should_panic]
    fn bytes_get_oob_into_valid_node_traps() {
        reset();
        let b = op_bytes_alloc(2);
        let _ = op_bytes_get(b, 9);
    }

    #[test]
    #[should_panic]
    fn map_key_oob_into_valid_node_traps() {
        reset();
        let m = op_map_alloc(1);
        let _ = op_map_key(m, 5);
    }

    // ── Perceus reference counting ────────────────────────────────────────────────────────────

    /// Current count of live (allocated, not-yet-freed) nodes on this test thread. Tests measure
    /// DELTAS against a baseline captured at their start.
    fn live_nodes() -> i64 {
        LIVE_NODES.with(|n| n.get())
    }

    #[test]
    fn dup_and_drop_move_the_refcount() {
        reset();
        let h = boxed_int_leaf(5); // a real heap leaf: refcount motion is only observable on a Node
        assert_eq!(rc_of(h), 1);
        op_dup(h);
        assert_eq!(rc_of(h), 2);
        op_dup(h);
        assert_eq!(rc_of(h), 3);
        op_drop(h); // 3 -> 2: still live, value intact
        assert_eq!(rc_of(h), 2);
        assert_eq!(op_get_int(h), 5);
        op_drop(h); // 2 -> 1: still live
        assert_eq!(rc_of(h), 1);
        // Final drop frees it; we don't read `h` after this (it dangles by design).
        let before = live_nodes();
        op_drop(h);
        assert_eq!(live_nodes(), before - 1, "the last drop must free the node");
    }

    #[test]
    fn drop_at_zero_reclaims_a_leaf() {
        reset();
        let before = live_nodes();
        let h = boxed_int_leaf(42); // a genuinely heap-allocated leaf to reclaim
        assert_eq!(live_nodes(), before + 1);
        op_drop(h);
        assert_eq!(live_nodes(), before, "a leaf with rc 1 is freed on drop");
    }

    #[test]
    fn drop_cascades_through_owned_children() {
        reset();
        let before = live_nodes();
        // (tuple 1 (tuple 2 3)) — the ints are real heap leaves here (boxed_int_leaf) so the CASCADE
        // has genuine children to reclaim: 1 int + 1 inner-arr + 2 ints + 1 outer-arr = 5 nodes.
        let inner = op_arr_alloc(2);
        op_arr_set(inner, 0, boxed_int_leaf(2));
        op_arr_set(inner, 1, boxed_int_leaf(3));
        let outer = op_arr_alloc(2);
        op_arr_set(outer, 0, boxed_int_leaf(1));
        op_arr_set(outer, 1, inner);
        assert_eq!(live_nodes(), before + 5);
        // Dropping the root reclaims the ENTIRE subtree — all 5 nodes, no leak.
        op_drop(outer);
        assert_eq!(live_nodes(), before, "the whole owned subtree is reclaimed");
    }

    #[test]
    fn shared_child_survives_until_its_last_owner_drops() {
        reset();
        let before = live_nodes();
        // A single shared child under two parents (structural sharing / path-copying's core case).
        let child = op_arr_alloc(1);
        op_arr_set(child, 0, boxed_int_leaf(9)); // child + its (real heap) int = 2 nodes
        op_dup(child); // parent A retains a reference
        op_dup(child); // parent B retains a reference — child rc is now 3 (birth + 2 dups)
        let pa = op_arr_alloc(1);
        op_arr_set(pa, 0, child);
        let pb = op_arr_alloc(1);
        op_arr_set(pb, 0, child); // 2 parents = 2 more nodes
        assert_eq!(live_nodes(), before + 4);

        // Drop parent A: it releases ITS reference to the child, but B (and the birth ref) remain,
        // so the child and its int MUST survive. Only parent A's own node is freed.
        op_drop(pa);
        assert_eq!(live_nodes(), before + 3, "shared child must not be freed while B holds it");
        assert_eq!(op_get_int(op_arr_get(child, 0)), 9, "shared child still intact");

        // Drop parent B: releases the second reference; the birth reference still pins the child.
        op_drop(pb);
        assert_eq!(live_nodes(), before + 2, "child still pinned by its birth reference");
        assert_eq!(op_get_int(op_arr_get(child, 0)), 9);

        // Release the birth reference: now the child's last owner is gone → child + int reclaimed.
        op_drop(child);
        assert_eq!(live_nodes(), before, "last owner gone: shared subtree reclaimed");
    }

    #[test]
    fn deep_unique_structure_frees_without_stack_overflow() {
        reset();
        let before = live_nodes();
        // Build a deeply-nested cons-like spine: (tuple v (tuple v (tuple v … unit))). At a depth
        // that would blow a RECURSIVE free's call stack, the iterative worklist cascade must not.
        const DEPTH: usize = 200_000;
        let mut acc = op_arr_alloc(0); // unit terminator — now an INLINE immediate, allocates no node
        for _ in 0..DEPTH {
            let node = op_arr_alloc(2);
            op_arr_set(node, 0, boxed_int_leaf(1)); // a real heap leaf per level (the cascade reclaims it)
            op_arr_set(node, 1, acc);
            acc = node;
        }
        // DEPTH spine nodes + DEPTH boxed int leaves. The unit terminator is inline (no node) since P1b.
        assert_eq!(live_nodes(), before + (DEPTH as i64) * 2);
        op_drop(acc); // single drop must reclaim the whole spine iteratively
        assert_eq!(live_nodes(), before, "deep structure fully reclaimed, no overflow");
    }

    #[test]
    fn peak_heap_is_bounded_across_iterations() {
        reset();
        let baseline = live_nodes();
        // The peak-heap acceptance probe: a loop that builds many compounds and drops
        // each before the next runs with BOUNDED peak heap — live nodes return to baseline every
        // iteration, so the high-water mark does not grow with the iteration count.
        let mut peak = baseline;
        for i in 0..1000i64 {
            let t = op_arr_alloc(3);
            op_arr_set(t, 0, op_box_int(i));
            op_arr_set(t, 1, op_sum_new(1, op_box_int(i * 2)));
            op_arr_set(t, 2, op_str_new("x".to_string()));
            peak = peak.max(live_nodes());
            op_drop(t);
            assert_eq!(live_nodes(), baseline, "each iteration returns to baseline");
        }
        // Peak is one iteration's worth of nodes (a small constant), NOT ~1000 iterations' worth.
        assert!(
            peak - baseline <= 8,
            "peak heap must be bounded by one iteration's working set, saw {}",
            peak - baseline
        );
    }

    // ── RC calling convention: the emitted-sequence mirror ────────────────────────────────────
    // Each test SIMULATES the exact dup/drop sequence the compiler must emit for a pattern and
    // asserts, via LIVE_NODES, both properties the convention
    // guarantees: NO LEAK (heap returns to baseline) and NO EARLY FREE (kept values stay intact
    // until their last owner). These are the reference behaviors the compiler's emission reproduces;
    // a failing test would mean the primitives cannot support the prescribed convention.

    /// §3.5 / §4 — projection kept past the parent: `(let t (tuple a b) (arr-get t 0))`. The
    /// element is RETURNED, so it must be dup'd BEFORE the parent is dropped; then dropping the
    /// tuple frees the tuple node + the not-kept sibling, leaving the kept element valid.
    #[test]
    fn rc_convention_projection_return_dups_before_parent_drop() {
        reset();
        let before = live_nodes();
        // t = (tuple 3 1) — owned. Elements are real heap leaves so the dup-before-drop discipline
        // for the kept child is genuinely exercised (an inline int's dup/drop would be a no-op).
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, boxed_int_leaf(3));
        op_arr_set(t, 1, boxed_int_leaf(1));
        assert_eq!(live_nodes(), before + 3, "tuple + 2 ints");

        // Emit: kept = arr-get(t, 0) [BORROW]; dup(kept) [make it an owner]; drop(t) [release parent]
        let kept = op_arr_get(t, 0);
        op_dup(kept); // §4: dup the kept child BEFORE dropping the parent
        op_drop(t); // frees the tuple node + element 1; element 0 survives (rc went 1->2->1)

        assert_eq!(op_get_int(kept), 3, "kept element must survive the parent drop");
        assert_eq!(live_nodes(), before + 1, "only the kept element remains");
        op_drop(kept); // the returned owner is eventually released
        assert_eq!(live_nodes(), before, "no leak once the kept owner drops");
    }

    /// §3.5 — `match Some(x) => x`: dup the borrowed payload, then drop the scrutinee. Payload
    /// survives; the sum node is reclaimed.
    #[test]
    fn rc_convention_match_extract_keeps_payload() {
        reset();
        let before = live_nodes();
        let s = op_sum_new(1, boxed_int_leaf(42)); // Some(42) — owned scrutinee. sum + heap int = 2 nodes.
        assert_eq!(live_nodes(), before + 2);

        // Emit for the `Some x => x` arm: x = sum-payload(s) [BORROW]; dup(x); drop(s).
        let x = op_sum_payload(s);
        op_dup(x); // §3.5: dup the kept field BEFORE dropping the scrutinee
        op_drop(s); // frees only the sum node; payload survives (rc 1->2->1)

        assert_eq!(op_get_int(x), 42, "extracted payload survives the scrutinee drop");
        assert_eq!(live_nodes(), before + 1, "sum node reclaimed, payload kept");
        op_drop(x);
        assert_eq!(live_nodes(), before);
    }

    /// §3.5 (no-keep arm) — `match Some(_) => 0`: the payload is NOT kept, so no dup; dropping the
    /// scrutinee reclaims the whole sum INCLUDING the payload.
    #[test]
    fn rc_convention_match_discard_reclaims_whole_sum() {
        reset();
        let before = live_nodes();
        let s = op_sum_new(1, boxed_int_leaf(42)); // sum + heap-int payload = 2 nodes
        assert_eq!(live_nodes(), before + 2);
        // Arm returns a constant; payload not kept ⇒ just drop the scrutinee.
        op_drop(s);
        assert_eq!(live_nodes(), before, "whole sum + payload reclaimed when nothing is kept");
    }

    /// §3.3 — the duplicate-binder question, answered: `(tuple x x)` is a `dup`, not an error. The
    /// tuple owns TWO references to the same child; dropping the tuple reclaims the child exactly
    /// once (rc 2->1->0 across the two owned slots).
    #[test]
    fn rc_convention_duplicate_binder_tuple_x_x() {
        reset();
        let before = live_nodes();
        let x = op_arr_alloc(1); // a shareable child (an owned heap value bound to `x`)
        op_arr_set(x, 0, boxed_int_leaf(9)); // x + its (real heap) int = 2 nodes
        assert_eq!(live_nodes(), before + 2);

        // Emit `(tuple x x)`: dup(x) for slot 0; the original is consumed by slot 1.
        let t = op_arr_alloc(2);
        op_dup(x); // §3.3: one dup for the second owner
        op_arr_set(t, 0, x); // slot 0 owns one reference
        op_arr_set(t, 1, x); // slot 1 consumes the original — tuple now owns x twice
        assert_eq!(rc_of(x), 2, "the tuple holds two owned references to x");
        assert_eq!(live_nodes(), before + 3, "tuple node added; x not duplicated in memory");

        // Dropping the tuple releases BOTH references; x is reclaimed exactly once, no double-free.
        op_drop(t);
        assert_eq!(live_nodes(), before, "duplicate-binder child reclaimed exactly once");
    }

    /// §3.4 — branch balancing. `(if c xs ys)` returns one of two owned lists, both live at the
    /// `if`; each arm drops the not-returned one. Correct for BOTH values of `c`: no leak, no
    /// double-free either way.
    #[test]
    fn rc_convention_if_branches_balance_ownership() {
        reset();
        // Run the emitted schedule for both branch directions.
        for take_then in [true, false] {
            let before = live_nodes();
            let xs = op_arr_alloc(1);
            op_arr_set(xs, 0, boxed_int_leaf(1)); // xs + (real heap) int
            let ys = op_arr_alloc(1);
            op_arr_set(ys, 0, boxed_int_leaf(2)); // ys + (real heap) int
            assert_eq!(live_nodes(), before + 4, "two owned lists live at the if");

            // Emitted: then-arm { result=xs; drop ys }  else-arm { result=ys; drop xs }.
            let result = if take_then {
                op_drop(ys); // §3.4: the not-taken value is released in this arm
                xs
            } else {
                op_drop(xs);
                ys
            };

            let expect = if take_then { 1 } else { 2 };
            assert_eq!(op_get_int(op_arr_get(result, 0)), expect, "the taken list survives intact");
            assert_eq!(live_nodes(), before + 2, "exactly one list (2 nodes) survives");
            op_drop(result); // the if's owned result is eventually released
            assert_eq!(live_nodes(), before, "no leak, no double-free on either path");
        }
    }

    /// §3.1 — a bound-but-unused heap value (`(let x (tuple …) 0)`) is dropped at scope end;
    /// baseline restored.
    #[test]
    fn rc_convention_dead_binding_is_dropped() {
        reset();
        let before = live_nodes();
        let x = op_arr_alloc(2);
        op_arr_set(x, 0, boxed_int_leaf(1));
        op_arr_set(x, 1, boxed_int_leaf(2)); // x + 2 (real heap) ints; the body never uses x
        assert_eq!(live_nodes(), before + 3);
        op_drop(x); // §3.1: dead binding released at end of scope
        assert_eq!(live_nodes(), before, "dead binding fully reclaimed");
    }

    // ── Reuse / FBIP ───────────────────────────────────────────────────────────────────────────
    // `reset` + the `*-reuse` constructors give in-place update on unique data. The tests assert
    // the two load-bearing properties: (1) reuse is IN PLACE — the rebuilt node is the SAME
    // allocation (address identity + zero net LIVE_NODES growth), the whole point over free→malloc;
    // (2) reuse is FRAME-LIMITED — it fires ONLY on a unique node, so a shared value (a persistent
    // structure's other version) is never clobbered and peak heap cannot grow.

    /// `reset` on a UNIQUE node yields its shell as a non-null token, drops its owned children, and
    /// keeps exactly one node live (the emptied shell) — ready to be refit.
    #[test]
    fn reset_unique_yields_emptied_shell_token() {
        reset();
        let before = live_nodes();
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, boxed_int_leaf(3));
        op_arr_set(t, 1, boxed_int_leaf(4)); // shell + 2 (real heap) ints = 3 nodes
        assert_eq!(live_nodes(), before + 3);

        let token = op_reset(t);
        assert_eq!(token, t, "the token IS the reset node's shell (same handle)");
        assert_ne!(token, Handle::NULL, "unique reset yields a non-null token");
        assert_eq!(op_arr_len(token), 0, "children released; shell is empty");
        assert_eq!(rc_of(token), 1, "the retained shell keeps rc == 1");
        assert_eq!(live_nodes(), before + 1, "the 2 children freed; only the shell remains");

        op_drop(token); // an unused token is just a childless unique node — drop frees the shell
        assert_eq!(live_nodes(), before, "dropping an unused token frees exactly the shell");
    }

    /// `reset` on a SHARED node declines: it returns NULL, decrements, and leaves the node (and its
    /// children) fully intact for the other owner. This is the frame-limiting guard — a persistent
    /// structure's shared version is never reused out from under it.
    #[test]
    fn reset_shared_declines_and_preserves_the_node() {
        reset();
        let before = live_nodes();
        let t = op_arr_alloc(1);
        op_arr_set(t, 0, boxed_int_leaf(9)); // shell + (real heap) int = 2 nodes
        op_dup(t); // a second owner (e.g. another version sharing this node) — rc = 2
        assert_eq!(live_nodes(), before + 2);

        let token = op_reset(t);
        assert_eq!(token, Handle::NULL, "shared reset declines: null token");
        assert_eq!(rc_of(t), 1, "reset decremented the shared count by one");
        assert_eq!(op_get_int(op_arr_get(t, 0)), 9, "the shared node is fully intact");
        assert_eq!(live_nodes(), before + 2, "nothing freed: the other owner still holds it");

        op_drop(t); // release the surviving owner
        assert_eq!(live_nodes(), before);
    }

    /// A null token makes the reuse constructors behave EXACTLY as their plain forms (fresh alloc),
    /// so a declined `reset` is transparent to the emitted rebuild code.
    #[test]
    fn reuse_ctors_with_null_token_allocate_fresh() {
        reset();
        let before = live_nodes();
        let a = op_arr_alloc_reuse(2, Handle::NULL);
        assert_eq!(op_arr_len(a), 2, "null token: fresh array of the requested length");
        let s = op_sum_new_reuse(1, boxed_int_leaf(7), Handle::NULL);
        assert_eq!(op_sum_disc(s), 1);
        assert_eq!(op_get_int(op_sum_payload(s)), 7);
        assert_eq!(live_nodes(), before + 3, "array + sum + its (heap) int, all freshly allocated");
        op_drop(a);
        op_drop(s);
        assert_eq!(live_nodes(), before);
    }

    /// `arr-alloc-reuse` with a real token refits the SAME shell — address identity, no new node.
    #[test]
    fn arr_alloc_reuse_refits_the_same_shell() {
        reset();
        let before = live_nodes();
        let old = op_arr_alloc(2);
        op_arr_set(old, 0, boxed_int_leaf(1));
        op_arr_set(old, 1, boxed_int_leaf(2)); // shell + 2 (real heap) ints = 3 nodes
        assert_eq!(live_nodes(), before + 3);
        let token = op_reset(old); // children freed, shell retained (1 node)
        assert_eq!(live_nodes(), before + 1);

        let fresh = op_arr_alloc_reuse(3, token); // refit to a DIFFERENT length
        assert_eq!(fresh, old, "reuse returns the very same node — in-place, no allocation");
        assert_eq!(op_arr_len(fresh), 3, "refit to the new length");
        assert_eq!(live_nodes(), before + 1, "still one node: no new allocation for the rebuild");
        op_arr_set(fresh, 0, op_box_int(10));
        op_arr_set(fresh, 1, op_box_int(20));
        op_arr_set(fresh, 2, op_box_int(30));
        assert_eq!(op_get_int(op_arr_get(fresh, 2)), 30);
        op_drop(fresh);
        assert_eq!(live_nodes(), before);
    }

    /// A reuse TOKEN whose shell came from a node with a HEAP-backed raw (a bytes/string leaf longer
    /// than the inline cap) must NOT leave the reused node carrying that heap raw: `op_sum_new_reuse`
    /// and `op_arr_alloc_reuse` normalize the raw back to INLINE, matching what the fresh constructors
    /// produce. (Regression guard: the old `raw.clear()` + `extend_from_slice` kept a heap buffer — a
    /// stray retained allocation AND a non-canonical storage rep for one logical value; the value stayed
    /// byte-equal via Deref so hash/eq tests could NOT have caught it, hence this explicit rep check.)
    #[test]
    fn reuse_ctor_normalizes_a_heap_raw_token_to_inline() {
        reset();
        let before = live_nodes();
        // A bytes leaf longer than INLINE_RAW_CAP → its raw spills to the heap.
        let big_leaf = |n: usize| -> Handle {
            let bytes: Vec<u8> = (0..n as u32).map(|k| (k & 0xff) as u8).collect();
            alloc(Vec::new(), bytes)
        };

        // (1) reuse a heap-raw shell as a SUM node → raw must be inline (the 4-byte disc).
        let leaf = big_leaf(INLINE_RAW_CAP + 8);
        assert!(raw_is_heap(leaf), "precondition: a >cap bytes leaf has a heap raw");
        let token = op_reset(leaf); // childless heap-raw shell, rc==1
        assert_eq!(token, leaf, "unique reset yields the shell");
        let s = op_sum_new_reuse(3, op_box_int(42), token);
        assert!(!raw_is_heap(s), "reused sum node's raw is INLINE, not the token's leftover heap buffer");
        assert_eq!(op_sum_disc(s), 3, "disc correct");
        assert_eq!(op_get_int(op_sum_payload(s)), 42, "payload correct");
        // Byte-identical to a FRESH sum (same disc/payload) — the whole point of normalizing the rep.
        let fresh_sum = op_sum_new(3, op_box_int(42));
        assert!(champ_eq(s, fresh_sum), "reused sum equals a fresh one built the same way");
        assert_eq!(champ_hash(s), champ_hash(fresh_sum), "…and hashes identically");
        op_drop(s);
        op_drop(fresh_sum);

        // (2) reuse a heap-raw shell as an ARRAY node → raw must be inline (empty).
        let leaf2 = big_leaf(INLINE_RAW_CAP + 20);
        assert!(raw_is_heap(leaf2));
        let token2 = op_reset(leaf2);
        let a = op_arr_alloc_reuse(2, token2);
        assert!(!raw_is_heap(a), "reused array node's raw is INLINE-empty, not a leftover heap buffer");
        op_arr_set(a, 0, op_box_int(1));
        op_arr_set(a, 1, op_box_int(2));
        assert_eq!(op_get_int(op_arr_get(a, 1)), 2);
        op_drop(a);

        assert_eq!(live_nodes(), before, "no leak: every reused/fresh node reclaimed");
    }

    /// `sum-new-reuse` with a token repurposes the SAME shell as the new `(disc, payload)` node.
    #[test]
    fn sum_new_reuse_refits_the_same_shell() {
        reset();
        let before = live_nodes();
        let old = op_sum_new(0, op_arr_alloc(0)); // None-like: sum shell + unit payload = 2 nodes
        let token = op_reset(old); // unit payload freed, shell retained (1 node)
        assert_eq!(live_nodes(), before + 1);

        let payload = boxed_int_leaf(42); // a real heap payload so "reused shell + payload" = 2 nodes
        let fresh = op_sum_new_reuse(1, payload, token); // rebuild as Some(42), reusing the shell
        assert_eq!(fresh, old, "the sum shell is reused in place");
        assert_eq!(op_sum_disc(fresh), 1);
        assert_eq!(op_get_int(op_sum_payload(fresh)), 42);
        assert_eq!(live_nodes(), before + 2, "reused shell + the new payload int; shell not re-alloc'd");
        op_drop(fresh);
        assert_eq!(live_nodes(), before);
    }

    /// The headline FBIP property: mapping a function over a UNIQUE list rebuilds it with ZERO net
    /// allocation. Emitted per element: dup the elements to keep → reset the old cons/array shell →
    /// arr-alloc-reuse it → refill. Peak heap never exceeds the input's node count + the transient
    /// working set; the rebuilt list occupies the SAME shells as the input.
    #[test]
    fn fbip_map_over_unique_list_reuses_in_place() {
        reset();
        let before = live_nodes();
        const N: u32 = 8;
        // A unique flat list [0,1,…,N-1] — one array shell + N int leaves.
        let xs = op_arr_alloc(N);
        for i in 0..N {
            op_arr_set(xs, i, boxed_int_leaf(i as i64)); // real heap leaves: reset must reclaim them
        }
        assert_eq!(live_nodes(), before + 1 + N as i64, "array shell + N ints");
        let shell_addr = xs; // remember the identity to prove in-place reuse

        // Emit `List.map (+100)`: read each element (borrow), compute the new leaf, reset the old
        // array to a token, refit it, and refill. The old int leaves are consumed by the map
        // function (get-int reads them by value; we drop each old leaf as the map "uses" it).
        let peak_probe;
        {
            // Collect the new leaves first (a real emitter would interleave; the invariant we test
            // is that the SHELL is reused, so this ordering is representative).
            let mut new_leaves = Vec::new();
            for i in 0..N {
                let old_leaf = op_arr_get(xs, i); // borrow
                let v = op_get_int(old_leaf); // the map body reads the element by value
                new_leaves.push(boxed_int_leaf(v + 100)); // real heap leaves to keep the footprint math
            }
            // The old leaves are no longer needed: reset will drop them when it empties the shell.
            peak_probe = live_nodes(); // shell + N old ints + N new ints
            let token = op_reset(xs); // unique → frees the N old ints, retains the shell
            let ys = op_arr_alloc_reuse(N, token); // SAME shell, refit
            assert_eq!(ys, shell_addr, "the mapped list reuses the input's array shell in place");
            for (i, leaf) in new_leaves.into_iter().enumerate() {
                op_arr_set(ys, i as u32, leaf);
            }
            // Verify the mapped result.
            for i in 0..N {
                assert_eq!(op_get_int(op_arr_get(ys, i)), i as i64 + 100);
            }
            // Net nodes now: the one reused shell + N new ints = same footprint as the input.
            assert_eq!(
                live_nodes(),
                before + 1 + N as i64,
                "mapped list has the SAME node footprint as the input — reuse allocated no net node"
            );
            op_drop(ys);
        }
        // Peak during the rebuild was bounded by input + transient new leaves (2N+1), NOT doubled
        // by a free→malloc that keeps both the old array shell AND a fresh one.
        assert_eq!(peak_probe, before + 1 + 2 * N as i64, "peak = shell + old ints + new ints");
        assert_eq!(live_nodes(), before, "no leak after the mapped list is dropped");
    }

    /// The ordering invariant for reset (the §4 dup-before-drop rule): a child of the old node that
    /// the rebuild KEEPS must be dup'd BEFORE `reset`, because reset drops the old node's child
    /// references. With the dup, the kept child survives into the reused shell.
    #[test]
    fn reset_keeps_dup_d_child_alive_for_the_rebuild() {
        reset();
        let before = live_nodes();
        // old = (tuple keep discard); we rebuild (tuple keep) reusing old's shell, keeping `keep`.
        let keep = boxed_int_leaf(77); // real heap leaves: the dup-before-reset survival is only
        let discard = boxed_int_leaf(-1); // observable on ref-counted Nodes
        let old = op_arr_alloc(2);
        op_arr_set(old, 0, keep);
        op_arr_set(old, 1, discard); // shell + 2 ints = 3 nodes
        assert_eq!(live_nodes(), before + 3);

        op_dup(keep); // §4: dup the child we intend to carry BEFORE resetting the parent
        let token = op_reset(old); // frees `discard`; `keep` survives (rc 1->2 via dup, ->1 via drop)
        assert_eq!(live_nodes(), before + 2, "shell + kept child; discard freed");
        let rebuilt = op_arr_alloc_reuse(1, token);
        op_arr_set(rebuilt, 0, keep); // carry the kept child into the reused shell
        assert_eq!(op_get_int(op_arr_get(rebuilt, 0)), 77, "kept child survived reset into the reuse");
        assert_eq!(live_nodes(), before + 2, "still shell + kept child — reuse allocated nothing");
        op_drop(rebuilt);
        assert_eq!(live_nodes(), before, "no leak");
    }

    // ── Persistent vector (32-way radix trie) ──────────────────────────────────────────────────
    // The two load-bearing properties, mirrored from the rope/RC suites: (1) the OBSERVABLE contract
    // — push/get/update/len denote a dense immutable sequence, and old versions are unchanged by an
    // operation on a new one (PERSISTENCE); (2) RESOURCE behavior — path-copying shares subtrees
    // (bounded per-op allocation, not O(N) copy), the whole trie reclaims to baseline on drop via the
    // existing iterative cascade, and peak heap stays bounded across a build/drop loop.

    /// Read a whole vector into a Rust Vec of ints, via the borrowing `vec-get` — the mirror the
    /// compiler's renderer will drive (`vec-len` then `vec-get` over `0..len`).
    fn vec_to_ints(v: Handle) -> Vec<i64> {
        (0..op_vec_len(v))
            .map(|i| op_get_int(op_vec_get(v, i)))
            .collect()
    }

    /// Build a vector [0,1,…,n-1] of boxed ints by repeated push. Each push consumes the running
    /// vector and returns the next, so the final handle is the sole owner of the whole sequence.
    fn vec_range(n: i64) -> Handle {
        let mut v = op_vec_empty();
        for i in 0..n {
            v = op_vec_push(v, op_box_int(i));
        }
        v
    }

    /// Build a RELAXED interior node from `child_sizes`: child `i` is a strict leaf holding a run of
    /// consecutive ints so that the whole vector reads back as `[0, 1, …, total-1]`. The leaves have
    /// IRREGULAR sizes (not all `1 << level`), which is exactly what makes the parent relaxed; the
    /// parent's `raw` is the cumulative size table `[s0, s0+s1, …, total]` (u32 LE), and the returned
    /// handle is a normal vector HEADER at shift `VEC_BITS` owning that relaxed root. This is the only
    /// way to exercise the relaxed read path in U1, since normal push/update never build a relaxed node.
    fn vec_relaxed_of(child_sizes: &[u32]) -> Handle {
        let mut handles = Vec::with_capacity(child_sizes.len());
        let mut raw = Vec::with_capacity(4 * child_sizes.len());
        let mut running = 0u32;
        for &sz in child_sizes {
            // A strict leaf holding `sz` consecutive ints starting at `running`.
            let mut leaf_handles = Vec::with_capacity(sz as usize);
            for k in 0..sz {
                leaf_handles.push(op_box_int((running + k) as i64));
            }
            handles.push(alloc(leaf_handles, Vec::new()));
            running += sz;
            raw.extend_from_slice(&running.to_le_bytes());
        }
        let root = alloc(handles, raw); // raw.len() == 4*arity ⇒ relaxed
        vec_alloc_header(running, VEC_BITS, root)
    }

    #[test]
    fn vec_relaxed_node_indexing() {
        reset();
        let before = live_nodes();
        // Irregular child sizes [3,2,4] ⇒ cumulative table [3,5,9]; whole vector is [0..9).
        let v = vec_relaxed_of(&[3, 2, 4]);
        // The root MUST be recognized as relaxed (this is the read path under test).
        let (_c, _s, root) = vec_read_header(v);
        assert!(vec_is_relaxed(root), "hand-built root is a relaxed node");
        assert_eq!(op_vec_len(v), 9);
        // First, last, and both child boundaries (2→3 crosses child0→child1; 4→5 crosses child1→child2).
        assert_eq!(op_get_int(op_vec_get(v, 0)), 0, "first element");
        assert_eq!(op_get_int(op_vec_get(v, 2)), 2, "last of child 0");
        assert_eq!(op_get_int(op_vec_get(v, 3)), 3, "first of child 1 (boundary)");
        assert_eq!(op_get_int(op_vec_get(v, 4)), 4, "last of child 1");
        assert_eq!(op_get_int(op_vec_get(v, 5)), 5, "first of child 2 (boundary)");
        assert_eq!(op_get_int(op_vec_get(v, 8)), 8, "last element");
        // And the full dense round-trip.
        assert_eq!(vec_to_ints(v), (0..9).collect::<Vec<_>>());
        op_drop(v);
        assert_eq!(live_nodes(), before, "relaxed hand-built vector reclaims to baseline");
    }

    #[test]
    fn vec_is_relaxed_disambiguates_every_other_node_kind() {
        reset();
        let before = live_nodes();

        // (1) A vector HEADER (raw.len()==8, handles.len()∈{0,1}) is NEVER relaxed.
        let empty = op_vec_empty();
        assert!(!vec_is_relaxed(empty), "empty header");
        let v = vec_range(40); // spans 2 levels: header owns a strict interior root
        assert!(!vec_is_relaxed(v), "non-empty header");

        // (2) A STRICT interior node (empty raw) is NEVER relaxed.
        let (_c, _s, root) = vec_read_header(v);
        assert!(!vec_is_relaxed(root), "strict interior root");
        assert!(vec_arity(root) >= 2, "root is a genuine interior node");

        // (3) A LEAF (strict, empty raw) is NEVER relaxed.
        let leaf = vec_child(root, 0);
        assert!(!vec_is_relaxed(leaf), "strict leaf");

        // (4) A CHAMP map node (raw.len()==12) is NEVER relaxed.
        let m = op_map_insert(op_map_empty(), op_box_int(1), op_box_int(2));
        assert!(!vec_is_relaxed(m), "CHAMP map node");

        // (5) A bytes ROPE node (concat raw==4, slice raw==8) is NEVER relaxed.
        let rope = op_bytes_concat(bytes_leaf(b"ab"), bytes_leaf(b"cd"));
        assert!(!vec_is_relaxed(rope), "bytes rope concat node");
        let slice = op_bytes_slice(bytes_leaf(b"abcdef"), 1, 3);
        assert!(!vec_is_relaxed(slice), "bytes rope slice node");

        // Positive control: the hand-built relaxed node IS relaxed.
        let relaxed = vec_relaxed_of(&[2, 3]);
        let (_c2, _s2, rroot) = vec_read_header(relaxed);
        assert!(vec_is_relaxed(rroot), "hand-built relaxed node (positive control)");

        op_drop(empty);
        op_drop(v);
        op_drop(m);
        op_drop(rope);
        op_drop(slice);
        op_drop(relaxed);
        assert_eq!(live_nodes(), before, "no leak across the disambiguation cases");
    }

    #[test]
    fn vec_relaxed_update_preserves_size_table_and_reads_back() {
        reset();
        let before = live_nodes();
        // Update through a relaxed root: an element swap must not disturb any size table.
        let v = vec_relaxed_of(&[3, 2, 4]); // [0..9)
        let v = op_vec_update(v, 4, op_box_int(400)); // index 4 is in child 1
        let (_c, _s, root) = vec_read_header(v);
        assert!(vec_is_relaxed(root), "root stays relaxed after update");
        assert_eq!(op_get_int(op_vec_get(v, 4)), 400, "updated element");
        assert_eq!(op_get_int(op_vec_get(v, 3)), 3, "neighbor in same child untouched");
        assert_eq!(op_get_int(op_vec_get(v, 5)), 5, "neighbor across boundary untouched");
        assert_eq!(op_vec_len(v), 9, "count unchanged");
        op_drop(v);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn vec_relaxed_push_appends_and_grows_size_table() {
        reset();
        let before = live_nodes();
        // Push through a relaxed root lands on the right edge; the final size-table entry grows by 1
        // per element and the read-back stays dense.
        let mut v = vec_relaxed_of(&[3, 2, 4]); // [0..9), last child holds [5..9)
        for i in 9..20i64 {
            v = op_vec_push(v, op_box_int(i));
        }
        let (_c, _s, root) = vec_read_header(v);
        assert!(vec_is_relaxed(root), "root stays relaxed after right-edge pushes");
        assert_eq!(op_vec_len(v), 20);
        assert_eq!(
            vec_to_ints(v),
            (0..20).collect::<Vec<_>>(),
            "dense round-trip after pushes"
        );
        op_drop(v);
        assert_eq!(live_nodes(), before, "no leak");
    }

    /// Recursively validate every RELAXED node in a subtree (rooted at `node`, whose top level is
    /// `level == shift`) and return the subtree's element count. Asserts each relaxed node's size table
    /// is strictly increasing, each entry equals the running sum of its children's counts, and the last
    /// entry equals the subtree total — the U1 invariants a broken concat would violate. Strict nodes
    /// carry no table (nothing to check); a leaf (`level == 0`) contributes `arity` elements.
    fn assert_relaxed_invariants_rec(node: Handle, level: u32) -> u32 {
        if level == 0 {
            return vec_arity(node) as u32; // leaf: elements are its handles, uniformly size-1
        }
        let arity = vec_arity(node);
        let mut child_counts = Vec::with_capacity(arity);
        let mut total = 0u32;
        for i in 0..arity {
            let c = assert_relaxed_invariants_rec(vec_child(node, i), level - VEC_BITS);
            child_counts.push(c);
            total += c;
        }
        if vec_is_relaxed(node) {
            let mut running = 0u32;
            let mut prev = 0u32;
            for (i, &cc) in child_counts.iter().enumerate() {
                assert!(cc > 0, "no zero-size child in a relaxed node (child {i})");
                running += cc;
                let s = vec_relaxed_size_at(node, i);
                assert!(s > prev, "relaxed size table strictly increasing at {i}: {s} <= {prev}");
                assert_eq!(s, running, "cumulative entry {i} == running child-count sum");
                prev = s;
            }
            assert_eq!(prev, total, "last size-table entry == subtree total");
        }
        total
    }

    /// Assert a vector's whole tree honors the relaxed-node invariants, and its header count matches
    /// the recomputed leaf total.
    fn assert_vec_invariants(v: Handle) {
        let (count, shift, root) = vec_read_header(v);
        if count == 0 {
            return;
        }
        let leaf_total = assert_relaxed_invariants_rec(root, shift);
        assert_eq!(leaf_total, count, "header count == recomputed leaf total");
    }

    /// Concat two runtime vectors of the given ranges and check the result against the concatenation
    /// of the two oracles, element by element, plus length and the relaxed invariants. Consumes the
    /// two built vectors (concat is a constructor); drops the result; asserts no leak.
    fn check_concat(la: i64, lb: i64) {
        let before = live_nodes();
        let a = vec_range(la);
        let b = vec_range(lb);
        // Oracle: a is [0..la), b is [0..lb); concat is those two runs back to back.
        let mut oracle: Vec<i64> = (0..la).collect();
        oracle.extend(0..lb);
        let c = op_vec_concat(a, b);
        assert_eq!(op_vec_len(c) as i64, la + lb, "concat len == la+lb for ({la},{lb})");
        assert_vec_invariants(c);
        assert_eq!(vec_to_ints(c), oracle, "concat elements match oracle for ({la},{lb})");
        op_drop(c);
        assert_eq!(live_nodes(), before, "no leak for concat({la},{lb})");
    }

    #[test]
    fn vec_concat_matches_oracle() {
        reset();
        // Sizes chosen to cross trie-level boundaries: 0, 1, one-under/at/over a leaf (31/32/33),
        // multi-level (1000). Every ordered pair exercises both leaf-merge and relaxed-join paths, and
        // unequal heights (the grow-to-shift path).
        let sizes = [0i64, 1, 5, 31, 32, 33, 100, 1000];
        for &la in &sizes {
            for &lb in &sizes {
                check_concat(la, lb);
            }
        }
    }

    #[test]
    fn vec_concat_empty_operand_identity() {
        reset();
        let before = live_nodes();
        // concat(empty, b) == b element-wise.
        let b = vec_range(50);
        let empty = op_vec_empty();
        let c = op_vec_concat(empty, b);
        assert_eq!(op_vec_len(c), 50);
        assert_eq!(vec_to_ints(c), (0..50).collect::<Vec<_>>());
        op_drop(c);
        // concat(a, empty) == a element-wise.
        let a = vec_range(50);
        let empty2 = op_vec_empty();
        let c2 = op_vec_concat(a, empty2);
        assert_eq!(op_vec_len(c2), 50);
        assert_eq!(vec_to_ints(c2), (0..50).collect::<Vec<_>>());
        op_drop(c2);
        // concat(empty, empty) == empty.
        let c3 = op_vec_concat(op_vec_empty(), op_vec_empty());
        assert_eq!(op_vec_len(c3), 0);
        op_drop(c3);
        assert_eq!(live_nodes(), before, "identity concat leaves no leak");
    }

    #[test]
    fn vec_concat_then_push_get_update() {
        reset();
        let before = live_nodes();
        // A concat that forces a relaxed root (unequal heights: 40 spans 2 levels, 5 is one leaf).
        let a = vec_range(40);
        let b = vec_range(5); // will read back as 0..5 appended after 0..40
        let mut v = op_vec_concat(a, b);
        assert_eq!(op_vec_len(v), 45);
        // get across the seam
        assert_eq!(op_get_int(op_vec_get(v, 39)), 39, "last of A");
        assert_eq!(op_get_int(op_vec_get(v, 40)), 0, "first of B");
        assert_eq!(op_get_int(op_vec_get(v, 44)), 4, "last of B");
        // push more elements onto the concatenated (relaxed) vector
        for i in 0..30i64 {
            v = op_vec_push(v, op_box_int(1000 + i));
        }
        assert_eq!(op_vec_len(v), 75);
        assert_eq!(op_get_int(op_vec_get(v, 45)), 1000, "first pushed element");
        assert_eq!(op_get_int(op_vec_get(v, 74)), 1029, "last pushed element");
        // update across the seam and in the pushed tail
        v = op_vec_update(v, 40, op_box_int(-1)); // B region
        v = op_vec_update(v, 74, op_box_int(-2)); // pushed tail
        assert_eq!(op_get_int(op_vec_get(v, 40)), -1);
        assert_eq!(op_get_int(op_vec_get(v, 74)), -2);
        assert_eq!(op_get_int(op_vec_get(v, 39)), 39, "neighbor untouched by update");
        assert_vec_invariants(v);
        op_drop(v);
        assert_eq!(live_nodes(), before, "no leak after concat+push+update");
    }

    #[test]
    fn vec_concat_preserves_relaxed_invariant() {
        reset();
        let before = live_nodes();
        // Several concats that all produce relaxed roots; validate the size tables recursively.
        for &(la, lb) in &[(33i64, 33i64), (100, 40), (1000, 1000), (32, 1000)] {
            let a = vec_range(la);
            let b = vec_range(lb);
            let c = op_vec_concat(a, b);
            let (_count, _shift, root) = vec_read_header(c);
            assert!(vec_is_relaxed(root), "unequal/large concat produced a relaxed root ({la},{lb})");
            assert_vec_invariants(c); // strictly increasing, cumulative, last == total
            op_drop(c);
        }
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn vec_deep_concat_stack_safe() {
        reset();
        let before = live_nodes();
        // Fold-concat 200 small vectors into one big vector; confirms the iterative/bounded-depth impl
        // does not overflow the stack and the final sequence is exactly the concatenation.
        let mut acc = op_vec_empty();
        let mut oracle: Vec<i64> = Vec::new();
        for k in 0..200i64 {
            let piece = vec_range(7); // each piece is [0..7)
            oracle.extend(0..7);
            acc = op_vec_concat(acc, piece);
            let _ = k;
        }
        assert_eq!(op_vec_len(acc) as usize, oracle.len(), "folded length == 200*7");
        assert_eq!(vec_to_ints(acc), oracle, "folded elements match oracle");
        assert_vec_invariants(acc);
        op_drop(acc);
        assert_eq!(live_nodes(), before, "no leak after deep fold-concat");
    }

    /// Split a runtime vector [0..len) at `index` and check both halves against the oracle: left is
    /// [0..index), right is [index..len). Validates lengths, element-wise contents, and the relaxed
    /// invariants on both outputs; drops both; asserts no leak.
    fn check_split(len: i64, index: u32) {
        let before = live_nodes();
        let v = vec_range(len);
        let (l, r) = op_vec_split(v, index);
        let idx = index.min(len as u32);
        assert_eq!(op_vec_len(l), idx, "left len == index for (len={len}, idx={index})");
        assert_eq!(
            op_vec_len(r),
            len as u32 - idx,
            "right len == len-index for (len={len}, idx={index})"
        );
        assert_vec_invariants(l);
        assert_vec_invariants(r);
        let left_want: Vec<i64> = (0..idx as i64).collect();
        let right_want: Vec<i64> = (idx as i64..len).collect();
        assert_eq!(vec_to_ints(l), left_want, "left elements (len={len}, idx={index})");
        assert_eq!(vec_to_ints(r), right_want, "right elements (len={len}, idx={index})");
        op_drop(l);
        op_drop(r);
        assert_eq!(live_nodes(), before, "no leak for split(len={len}, idx={index})");
    }

    #[test]
    fn vec_split_matches_oracle() {
        reset();
        // Sizes crossing trie-level boundaries; split points at 0, 1, mid, len-1, len.
        for &len in &[0i64, 1, 5, 31, 32, 33, 100, 1000] {
            let mut points = vec![0u32, len as u32]; // 0 and len (boundaries)
            if len >= 1 {
                points.push(1);
                points.push(len as u32 - 1);
                points.push(len as u32 / 2);
            }
            for &p in &points {
                check_split(len, p);
            }
        }
    }

    #[test]
    fn vec_split_reconcat_roundtrip() {
        reset();
        // concat(split(v, i)) == v ELEMENT-WISE (structure differs; concat/split both reshape).
        for &i in &[0u32, 1, 17, 32, 33, 500, 999, 1000] {
            let before = live_nodes();
            let v = vec_range(1000);
            let (l, r) = op_vec_split(v, i);
            let joined = op_vec_concat(l, r); // consumes l and r
            assert_eq!(op_vec_len(joined), 1000, "reconcat len for i={i}");
            assert_eq!(vec_to_ints(joined), (0..1000).collect::<Vec<_>>(), "reconcat elements for i={i}");
            assert_vec_invariants(joined);
            op_drop(joined);
            assert_eq!(live_nodes(), before, "no leak for reconcat i={i}");
        }
    }

    #[test]
    fn vec_split_boundary() {
        reset();
        let before = live_nodes();
        // index 0 → (empty, v'), where v' reads identically to v.
        let v = vec_range(50);
        let (l, r) = op_vec_split(v, 0);
        assert_eq!(op_vec_len(l), 0, "index 0: left empty");
        assert_eq!(op_vec_len(r), 50, "index 0: right is all of v");
        assert_eq!(vec_to_ints(r), (0..50).collect::<Vec<_>>());
        op_drop(l);
        op_drop(r);
        // index >= len → (v', empty).
        let v2 = vec_range(50);
        let (l2, r2) = op_vec_split(v2, 50);
        assert_eq!(op_vec_len(l2), 50, "index len: left is all of v");
        assert_eq!(op_vec_len(r2), 0, "index len: right empty");
        assert_eq!(vec_to_ints(l2), (0..50).collect::<Vec<_>>());
        op_drop(l2);
        op_drop(r2);
        // index > len is clamped to len.
        let v3 = vec_range(10);
        let (l3, r3) = op_vec_split(v3, 999);
        assert_eq!(op_vec_len(l3), 10);
        assert_eq!(op_vec_len(r3), 0);
        op_drop(l3);
        op_drop(r3);
        assert_eq!(live_nodes(), before, "no leak across boundary splits");
    }

    #[test]
    fn vec_split_outputs_valid_for_downstream() {
        reset();
        let before = live_nodes();
        // Split a multi-level vector; then push/update/get/concat on BOTH halves.
        let v = vec_range(300);
        let (mut l, mut r) = op_vec_split(v, 137); // left [0..137), right [137..300)
        assert_eq!(op_get_int(op_vec_get(l, 136)), 136, "left last");
        assert_eq!(op_get_int(op_vec_get(r, 0)), 137, "right first");
        // push onto both
        for i in 0..40i64 {
            l = op_vec_push(l, op_box_int(1000 + i));
            r = op_vec_push(r, op_box_int(2000 + i));
        }
        assert_eq!(op_vec_len(l), 177);
        assert_eq!(op_vec_len(r), 203);
        assert_eq!(op_get_int(op_vec_get(l, 176)), 1039, "left pushed tail");
        assert_eq!(op_get_int(op_vec_get(r, 202)), 2039, "right pushed tail");
        // update across a former seam
        l = op_vec_update(l, 100, op_box_int(-7));
        assert_eq!(op_get_int(op_vec_get(l, 100)), -7);
        assert_vec_invariants(l);
        assert_vec_invariants(r);
        // concat the two halves back together (consumes both)
        let joined = op_vec_concat(l, r);
        assert_eq!(op_vec_len(joined), 177 + 203);
        assert_vec_invariants(joined);
        op_drop(joined);
        assert_eq!(live_nodes(), before, "no leak after split+downstream ops");
    }

    #[test]
    fn vec_split_preserves_relaxed_invariant() {
        reset();
        let before = live_nodes();
        // Split at several points of a large vector and validate both outputs' size tables.
        for &i in &[1u32, 33, 512, 999] {
            let v = vec_range(1000);
            let (l, r) = op_vec_split(v, i);
            assert_vec_invariants(l);
            assert_vec_invariants(r);
            op_drop(l);
            op_drop(r);
        }
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn vec_split_deep_stack_safe() {
        reset();
        let before = live_nodes();
        // A size-1500 vector spans 3 levels; splitting near the middle exercises full-depth descent.
        let v = vec_range(1500);
        let (l, r) = op_vec_split(v, 733);
        assert_eq!(op_vec_len(l), 733);
        assert_eq!(op_vec_len(r), 767);
        assert_eq!(vec_to_ints(l), (0..733).collect::<Vec<_>>());
        assert_eq!(vec_to_ints(r), (733..1500).collect::<Vec<_>>());
        assert_vec_invariants(l);
        assert_vec_invariants(r);
        op_drop(l);
        op_drop(r);
        assert_eq!(live_nodes(), before, "no leak after deep split");
    }

    #[test]
    fn vec_empty_is_len_zero() {
        reset();
        let before = live_nodes();
        let v = op_vec_empty();
        assert_eq!(op_vec_len(v), 0);
        assert_eq!(vec_to_ints(v), Vec::<i64>::new());
        op_drop(v);
        assert_eq!(live_nodes(), before, "empty vector reclaims to baseline");
    }

    #[test]
    fn vec_push_get_round_trip_small() {
        reset();
        // Within one leaf (≤ 32 elements): shift stays 0, root is a single leaf node.
        let v = vec_range(5);
        assert_eq!(op_vec_len(v), 5);
        for i in 0..5 {
            assert_eq!(op_get_int(op_vec_get(v, i as u32)), i);
        }
        assert_eq!(vec_to_ints(v), vec![0, 1, 2, 3, 4]);
        op_drop(v);
    }

    #[test]
    fn vec_get_renders_as_list() {
        reset();
        // The type-directed renderer walks a vec exactly as it walks a list: len then get over the
        // range. A vec of [3,1] therefore renders identically to `(list 3 1)` — its element shape is
        // all the renderer needs, no runtime tag.
        let v = op_vec_push(op_vec_push(op_vec_empty(), op_box_int(3)), op_box_int(1));
        let n = op_vec_len(v);
        let mut out = String::from("(list");
        for i in 0..n {
            out.push(' ');
            out.push_str(&render(op_vec_get(v, i), &Shape::Int));
        }
        out.push(')');
        assert_eq!(out, "(list 3 1)");
        op_drop(v);
    }

    #[test]
    fn vec_crosses_leaf_boundary_and_grows_levels() {
        reset();
        // 32 elements exactly fill one leaf (shift 0); 33 forces a level (root becomes interior,
        // shift = VEC_BITS). 1100 spans several branches of the second level. Read every index back.
        for &n in &[32i64, 33, 100, 1100] {
            let v = vec_range(n);
            assert_eq!(op_vec_len(v), n as u32, "len after {n} pushes");
            let got = vec_to_ints(v);
            let want: Vec<i64> = (0..n).collect();
            assert_eq!(got, want, "dense round-trip at n={n}");
            op_drop(v);
        }
    }

    #[test]
    fn vec_deep_three_levels() {
        reset();
        // 1025 > 32² = 1024 forces a THIRD level (shift = 2*VEC_BITS). Exercises the grow-the-root
        // path (`count == capacity`) and a descent of depth 2 in push/get.
        let v = vec_range(1025);
        let (count, shift, _root) = vec_read_header(v);
        assert_eq!(count, 1025);
        assert_eq!(shift, 2 * VEC_BITS, "1025 elements need a 3-level trie");
        assert_eq!(op_get_int(op_vec_get(v, 0)), 0);
        assert_eq!(op_get_int(op_vec_get(v, 1024)), 1024);
        assert_eq!(vec_to_ints(v), (0..1025).collect::<Vec<_>>());
        op_drop(v);
    }

    #[test]
    fn vec_update_does_not_mutate_the_old_version() {
        reset();
        // PERSISTENCE: update returns a new version; the old one is byte-for-byte unchanged. This is
        // the whole point of a persistent vector — the two versions coexist, sharing all but one path.
        let v0 = vec_range(100);
        op_dup(v0); // keep a second owner of v0 across the consuming update (§3.1)
        let v1 = op_vec_update(v0, 42, op_box_int(999));
        // v1 has the change…
        assert_eq!(op_get_int(op_vec_get(v1, 42)), 999);
        // …v0 does NOT — the old version still reads its original element.
        assert_eq!(op_get_int(op_vec_get(v0, 42)), 42);
        assert_eq!(op_vec_len(v0), 100);
        assert_eq!(op_vec_len(v1), 100);
        // Every OTHER index agrees between the versions.
        for i in 0..100u32 {
            if i != 42 {
                assert_eq!(op_get_int(op_vec_get(v0, i)), op_get_int(op_vec_get(v1, i)));
            }
        }
        op_drop(v0);
        op_drop(v1);
    }

    #[test]
    fn vec_push_does_not_mutate_the_old_version() {
        reset();
        // Pushing onto v0 yields v1; v0's length and contents are unchanged.
        let v0 = vec_range(40);
        op_dup(v0); // second owner across the consuming push
        let v1 = op_vec_push(v0, op_box_int(4242));
        assert_eq!(op_vec_len(v0), 40, "old version keeps its length");
        assert_eq!(op_vec_len(v1), 41, "new version is one longer");
        assert_eq!(op_get_int(op_vec_get(v1, 40)), 4242);
        assert_eq!(vec_to_ints(v0), (0..40).collect::<Vec<_>>());
        op_drop(v0);
        op_drop(v1);
    }

    #[test]
    fn vec_update_shares_all_but_one_path() {
        reset();
        // RESOURCE behavior: an update on a 3-level trie allocates only the copied root→leaf path
        // (≤ 3 interior/leaf nodes) + 1 new element + 1 header — NOT O(N). The rest is shared (rc>1).
        let v0 = vec_range(1025); // 3 levels
        op_dup(v0);
        let before = live_nodes();
        let v1 = op_vec_update(v0, 500, op_box_int(-1));
        let allocated = live_nodes() - before;
        // header + one path of (root, level-1, leaf) copies + the new element leaf. Bounded by the
        // trie height (≤ 7), never the element count. Assert a generous constant, not O(N).
        assert!(
            (1..=8).contains(&allocated),
            "update allocated {allocated} nodes — must be path-bounded, not O(N)"
        );
        assert_eq!(op_get_int(op_vec_get(v1, 500)), -1);
        op_drop(v0);
        op_drop(v1);
    }

    #[test]
    fn vec_whole_trie_reclaims_on_drop() {
        reset();
        // The existing iterative op_drop cascade reclaims an entire multi-level trie — every interior
        // node, leaf, element, and header — with no leak and no new RC machinery.
        let before = live_nodes();
        let v = vec_range(200); // 2-level trie + 200 int leaves + interior/leaf nodes + header
        assert!(live_nodes() > before, "the trie occupies many nodes");
        op_drop(v);
        assert_eq!(live_nodes(), before, "the whole vector subtree is reclaimed");
    }

    #[test]
    fn vec_shared_versions_reclaim_when_last_owner_drops() {
        reset();
        // Two versions share subtrees; dropping one must NOT free the shared subtrees the other still
        // holds, and only when BOTH are dropped does everything return to baseline.
        let before = live_nodes();
        let v0 = vec_range(100);
        op_dup(v0);
        let v1 = op_vec_update(v0, 10, op_box_int(7)); // shares all-but-one path with v0
        // Drop v0: the shared subtrees survive under v1; v1 still reads correctly.
        op_drop(v0);
        assert_eq!(op_get_int(op_vec_get(v1, 10)), 7);
        assert_eq!(op_get_int(op_vec_get(v1, 99)), 99, "shared tail intact after v0 dropped");
        assert!(live_nodes() > before, "v1 (and its shared subtrees) still live");
        // Drop v1: last owner of everything → baseline.
        op_drop(v1);
        assert_eq!(live_nodes(), before, "both versions gone: full reclamation");
    }

    #[test]
    fn vec_get_oob_traps() {
        reset();
        // Belt-and-suspenders: OOB is fail-fast. (The dedicated should_panic tests below pin the trap;
        // here we confirm the in-bounds edges do NOT trap.)
        let v = vec_range(10);
        assert_eq!(op_get_int(op_vec_get(v, 0)), 0);
        assert_eq!(op_get_int(op_vec_get(v, 9)), 9);
        op_drop(v);
    }

    #[test]
    #[should_panic]
    fn vec_get_oob_into_valid_vector_traps() {
        reset();
        let v = vec_range(10);
        let _ = op_vec_get(v, 10); // index == count: out of bounds
    }

    #[test]
    #[should_panic]
    fn vec_update_oob_into_valid_vector_traps() {
        reset();
        let v = vec_range(10);
        let _ = op_vec_update(v, 25, op_box_int(0));
    }

    #[test]
    fn vec_empty_get_traps() {
        // A get into the empty vector is OOB (count 0) — must trap, not read a null root.
        reset();
        let v = op_vec_empty();
        let r = std::panic::catch_unwind(|| op_vec_get(v, 0));
        assert!(r.is_err(), "get into empty vector must trap");
    }

    #[test]
    fn vec_peak_heap_bounded_across_build_drop_iterations() {
        reset();
        // The peak-heap acceptance probe (mirrors peak_heap_is_bounded_across_iterations): a loop that
        // builds a whole vector and drops it each iteration returns to baseline every time, so peak
        // heap is one vector's working set — it does NOT grow with the iteration count.
        let baseline = live_nodes();
        let mut peak = baseline;
        for _ in 0..200 {
            let v = vec_range(64); // spans 2 levels
            peak = peak.max(live_nodes());
            op_drop(v);
            assert_eq!(live_nodes(), baseline, "each iteration returns to baseline");
        }
        // One iteration's vector is a small constant relative to 200 iterations' worth.
        let one_iter = peak - baseline;
        assert!(one_iter > 0);
        assert!(
            one_iter < 200,
            "peak heap must be one vector's footprint, not the loop count; saw {one_iter}"
        );
    }

    #[test]
    fn vec_update_every_index_then_reads_back() {
        reset();
        // Stress the path-copy across a 2-level trie: functionally update every index to i*10, keeping
        // only the newest version each step (each update consumes the prior), then verify.
        let mut v = vec_range(70);
        for i in 0..70u32 {
            v = op_vec_update(v, i, op_box_int(i as i64 * 10));
        }
        for i in 0..70u32 {
            assert_eq!(op_get_int(op_vec_get(v, i)), i as i64 * 10);
        }
        op_drop(v);
    }

    // ── U4: FBIP rc==1 in-place spine reuse for vec-push / vec-update ───────────────────────────
    // The load-bearing property is ALIASING SAFETY: a push/update on a SHARED version (rc>1) must
    // path-copy and leave the other version byte-identical; the FBIP win (in-place refit) fires ONLY
    // when the touched spine is uniquely owned. These tests pin both halves.

    /// Assert a shared version survives a push on the other owner (both node kinds via `make`).
    fn check_push_shared_safe(make: impl Fn() -> Handle, orig_len: i64) {
        let before = live_nodes();
        let v1 = make();
        let orig: Vec<i64> = vec_to_ints(v1);
        assert_eq!(orig.len() as i64, orig_len);
        op_dup(v1); // rc(header) == 2: v1 is now a SHARED version
        let v2 = op_vec_push(v1, op_box_int(77_000));
        // v1 (the shared version) is UNCHANGED — not mutated in place.
        assert_eq!(op_vec_len(v1) as i64, orig_len, "shared version keeps its length");
        assert_eq!(vec_to_ints(v1), orig, "shared version byte-identical after other owner's push");
        // v2 has the pushed element appended.
        assert_eq!(op_vec_len(v2) as i64, orig_len + 1);
        assert_eq!(op_get_int(op_vec_get(v2, orig_len as u32)), 77_000);
        for (i, &x) in orig.iter().enumerate() {
            assert_eq!(op_get_int(op_vec_get(v2, i as u32)), x, "v2 prefix matches v1");
        }
        assert_vec_invariants(v1);
        assert_vec_invariants(v2);
        op_drop(v1);
        op_drop(v2);
        assert_eq!(live_nodes(), before, "no leak / no double-free");
    }

    #[test]
    fn vec_push_fbip_shared_version_unaffected() {
        reset();
        check_push_shared_safe(|| vec_range(5), 5); // single leaf (strict)
        check_push_shared_safe(|| vec_range(100), 100); // multi-level (strict)
        // RELAXED-rooted vector (post-concat): exercises the relaxed in-place / path-copy branch.
        check_push_shared_safe(
            || {
                let c = op_vec_concat(vec_range(40), vec_range(40));
                let (_, _, root) = vec_read_header(c);
                assert!(vec_is_relaxed(root), "concat produced a relaxed root");
                c
            },
            80,
        );
    }

    /// Assert a shared version survives an update on the other owner (both node kinds via `make`).
    fn check_update_shared_safe(make: impl Fn() -> Handle, len: i64, idx: u32) {
        let before = live_nodes();
        let v1 = make();
        let orig: Vec<i64> = vec_to_ints(v1);
        assert_eq!(orig.len() as i64, len);
        op_dup(v1); // shared version
        let v2 = op_vec_update(v1, idx, op_box_int(-999));
        // v1 unchanged at idx (and everywhere).
        assert_eq!(op_get_int(op_vec_get(v1, idx)), orig[idx as usize], "shared version unchanged at idx");
        assert_eq!(vec_to_ints(v1), orig, "shared version byte-identical");
        // v2 changed at idx, equal elsewhere.
        assert_eq!(op_get_int(op_vec_get(v2, idx)), -999, "new version changed at idx");
        for i in 0..len as u32 {
            if i != idx {
                assert_eq!(op_get_int(op_vec_get(v2, i)), orig[i as usize], "v2 equals v1 off the path");
            }
        }
        assert_vec_invariants(v1);
        assert_vec_invariants(v2);
        op_drop(v1);
        op_drop(v2);
        assert_eq!(live_nodes(), before, "no leak / no double-free");
    }

    #[test]
    fn vec_update_fbip_shared_version_unaffected() {
        reset();
        check_update_shared_safe(|| vec_range(5), 5, 2); // single leaf (strict)
        check_update_shared_safe(|| vec_range(1025), 1025, 500); // 3-level (strict)
        // RELAXED-rooted: update across the concat seam.
        check_update_shared_safe(
            || {
                let c = op_vec_concat(vec_range(40), vec_range(40));
                let (_, _, root) = vec_read_header(c);
                assert!(vec_is_relaxed(root), "concat produced a relaxed root");
                c
            },
            80,
            60, // in the right half of the concat
        );
    }

    #[test]
    fn vec_push_fbip_unique_reuses_in_place() {
        reset();
        // The FBIP win: a push/update on a UNIQUE (rc==1) vector allocates strictly fewer nodes than the
        // same op on a SHARED (rc>1) one, because the unique spine is refit in place (no copy).
        // Measure PUSH alloc delta, unique vs shared, on a mid-leaf (no root-growth) 2-level vector.
        let unique_push_alloc = {
            let v = vec_range(50); // 2 levels; a push into a non-full leaf touches root+leaf
            let before = live_nodes();
            // A DEFINITELY-BOXED pushed element (a small int now inlines): the FBIP property under
            // test is "unique push adds ONLY the element leaf", which needs a real leaf to count.
            let v2 = op_vec_push(v, boxed_int_leaf(1)); // v is unique → in-place refit
            let d = live_nodes() - before;
            op_drop(v2);
            d
        };
        let shared_push_alloc = {
            let v = vec_range(50);
            op_dup(v); // shared → must path-copy the spine
            let before = live_nodes();
            let v2 = op_vec_push(v, boxed_int_leaf(1));
            let d = live_nodes() - before;
            op_drop(v); // release the shared owner
            op_drop(v2);
            d
        };
        assert!(
            unique_push_alloc < shared_push_alloc,
            "FBIP push must allocate fewer nodes when unique ({unique_push_alloc}) than shared ({shared_push_alloc})"
        );
        // The unique push adds ONLY the new element leaf (the header + spine are reused): 1 node.
        assert_eq!(unique_push_alloc, 1, "unique push allocates just the pushed element");

        // Same for UPDATE: unique refits in place (0 new nodes beyond the replacement element), shared
        // path-copies the whole root→leaf spine + a fresh header.
        let unique_update_alloc = {
            let v = vec_range(1025); // 3 levels
            let before = live_nodes();
            let v2 = op_vec_update(v, 500, op_box_int(-1));
            let d = live_nodes() - before;
            op_drop(v2);
            d
        };
        let shared_update_alloc = {
            let v = vec_range(1025);
            op_dup(v);
            let before = live_nodes();
            let v2 = op_vec_update(v, 500, op_box_int(-1));
            let d = live_nodes() - before;
            op_drop(v);
            op_drop(v2);
            d
        };
        assert!(
            unique_update_alloc < shared_update_alloc,
            "FBIP update must allocate fewer when unique ({unique_update_alloc}) than shared ({shared_update_alloc})"
        );
        // Unique update: the header + whole spine are reused in place; the replacement element is an
        // inline immediate (0 new nodes) and the replaced old inline element frees nothing, so the NET
        // delta is 0 — the sharpest possible FBIP win. (Were the element boxed, the +1 new leaf would
        // be offset by the -1 freed old leaf for the same net 0; both reps give 0 here.)
        assert_eq!(unique_update_alloc, 0, "unique update reuses the spine; inline elem adds no node");
    }

    #[test]
    fn vec_fbip_partial_share_copies_only_shared_portion() {
        reset();
        let before = live_nodes();
        // A vector whose HEADER is unique but whose ROOT is shared with another version: the header is
        // reused, but the root (and the shared spine below) must path-copy, never mutate in place.
        // Build v0 (unique), then v1 = update(v0) sharing v0's subtrees; keep v1, drop v0's header only
        // by NOT dup-ing — instead construct explicit sharing via update which shares off-path subtrees.
        let v0 = vec_range(200);
        op_dup(v0);
        let v1 = op_vec_update(v0, 0, op_box_int(1_000)); // v1 shares all-but-path-0 with v0
        // Now push onto v1 (header rc==1) — its rightmost spine is shared with v0, so it must copy there
        // and NOT corrupt v0.
        let v0_orig = vec_to_ints(v0);
        let v2 = op_vec_push(v1, op_box_int(2_000));
        assert_eq!(vec_to_ints(v0), v0_orig, "v0 intact after push on a partially-shared sibling");
        assert_eq!(op_get_int(op_vec_get(v2, 200)), 2_000);
        assert_vec_invariants(v0);
        assert_vec_invariants(v2);
        op_drop(v0);
        op_drop(v2);
        assert_eq!(live_nodes(), before, "no leak / no double-free");
    }

    #[test]
    fn vec_fbip_still_matches_oracle() {
        reset();
        let before = live_nodes();
        // Mixed unique + shared push/update sequence vs a Vec oracle. Deterministic LCG for indices.
        let mut v = op_vec_empty();
        let mut oracle: Vec<i64> = Vec::new();
        let mut lcg: u64 = 0x1234_5678;
        let next = |lcg: &mut u64| {
            *lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (*lcg >> 33) as u32
        };
        for step in 0..500i64 {
            // push
            v = op_vec_push(v, op_box_int(step));
            oracle.push(step);
            // occasionally fork (share) then keep working on the new version — exercises rc>1 paths
            if step % 7 == 0 {
                op_dup(v);
                let forked = v; // shared owner
                v = op_vec_push(v, op_box_int(-step));
                oracle.push(-step);
                op_drop(forked); // release the shared owner
            }
            // occasionally update a random in-bounds index
            if !oracle.is_empty() {
                let idx = next(&mut lcg) % oracle.len() as u32;
                let val = step * 1000;
                v = op_vec_update(v, idx, op_box_int(val));
                oracle[idx as usize] = val;
            }
        }
        assert_eq!(op_vec_len(v) as usize, oracle.len(), "length matches oracle");
        assert_eq!(vec_to_ints(v), oracle, "elements match oracle after mixed FBIP ops");
        assert_vec_invariants(v);
        op_drop(v);
        assert_eq!(live_nodes(), before, "no leak across the mixed sequence");
    }

    #[test]
    fn vec_fbip_unique_chain_bounded_peak() {
        reset();
        // A long push chain on a UNIQUE vector: peak heap is the vector's own footprint, and steady-state
        // per-push allocation is ~1 node (the element), NOT a fresh spine each time — the FBIP win.
        let baseline = live_nodes();
        let mut v = op_vec_empty();
        for i in 0..1000i64 {
            v = op_vec_push(v, op_box_int(i));
        }
        // A 1000-element trie's node count is bounded (leaves + a couple interior levels + header),
        // dominated by the 1000 element leaves. If FBIP had NOT fired, transient copies would still be
        // freed each step (op_drop), so this checks correctness of the final structure + no leak.
        assert_eq!(op_vec_len(v), 1000);
        assert_eq!(vec_to_ints(v), (0..1000).collect::<Vec<_>>());
        assert_vec_invariants(v);
        let live = live_nodes() - baseline;
        // 1000 element leaves + ~32 leaf nodes + ~2 interior + header ≈ well under 1100.
        assert!(live < 1100, "final structure is bounded ({live} nodes), not O(chain length) leaked");
        op_drop(v);
        assert_eq!(live_nodes(), baseline, "chain fully reclaims");
    }

    // ── Bytes rope (O(1) concat/slice over shared leaves) ─────────────────────────────────────
    // Two load-bearing property groups:
    // (1) the OBSERVABLE contract — concat/slice/compact denote the same Bytes a copy would, by
    // `bytes-len`/`bytes-get`/logical-equality, and are associative-by-content; (2) the RESOURCE win
    // — concat/slice allocate ONE node (no byte copy), a deep concat chain reads out in O(total) not
    // O(n²) via flatten-on-access, the whole rope reclaims on drop, and a shared leaf survives.

    /// Build a leaf Bytes from a slice, via the existing alloc/set path.
    fn bytes_leaf(data: &[u8]) -> Handle {
        let b = op_bytes_alloc(data.len() as u32);
        for (i, &v) in data.iter().enumerate() {
            op_bytes_set(b, i as u32, v as u32);
        }
        b
    }
    /// Read a whole Bytes into a Rust Vec via the borrowing `bytes-get` — the compiler's emit loop.
    fn bytes_to_vec(h: Handle) -> Vec<u8> {
        (0..op_bytes_len(h))
            .map(|i| op_bytes_get(h, i) as u8)
            .collect()
    }

    #[test]
    fn rope_concat_round_trip() {
        reset();
        let c = op_bytes_concat(bytes_leaf(&[1, 2]), bytes_leaf(&[3, 4]));
        assert_eq!(op_bytes_len(c), 4);
        assert_eq!(bytes_to_vec(c), vec![1, 2, 3, 4]);
        op_drop(c);
    }

    #[test]
    fn rope_concat_allocates_one_node_no_copy() {
        reset();
        // O(1): concatenation adds exactly one concat node, copies no bytes into new leaves.
        let x = bytes_leaf(&[0; 50]);
        let y = bytes_leaf(&[1; 50]);
        let before = live_nodes();
        let c = op_bytes_concat(x, y);
        assert_eq!(live_nodes(), before + 1, "concat = one node, not 100 byte copies");
        assert_eq!(op_bytes_len(c), 100);
        op_drop(c);
    }

    #[test]
    fn rope_concat_empty_is_identity() {
        reset();
        // Empty is the identity on both sides (corpus law), and consumes the empty operand.
        let a = bytes_leaf(&[7, 8, 9]);
        let e = op_bytes_alloc(0);
        let r = op_bytes_concat(a, e); // right-empty → returns `a`, drops `e`
        assert_eq!(bytes_to_vec(r), vec![7, 8, 9]);
        let e2 = op_bytes_alloc(0);
        let b = bytes_leaf(&[5, 6]);
        let r2 = op_bytes_concat(e2, b); // left-empty → returns `b`, drops `e2`
        assert_eq!(bytes_to_vec(r2), vec![5, 6]);
        op_drop(r);
        op_drop(r2);
    }

    #[test]
    fn rope_concat_associative_by_content() {
        reset();
        // (a·b)·c and a·(b·c) — different tree shapes, identical logical bytes (the corpus law).
        let l = op_bytes_concat(
            op_bytes_concat(bytes_leaf(&[1, 2]), bytes_leaf(&[3])),
            bytes_leaf(&[4, 5]),
        );
        let r = op_bytes_concat(
            bytes_leaf(&[1, 2]),
            op_bytes_concat(bytes_leaf(&[3]), bytes_leaf(&[4, 5])),
        );
        assert_eq!(op_bytes_len(l), op_bytes_len(r));
        assert_eq!(bytes_to_vec(l), bytes_to_vec(r));
        assert_eq!(bytes_to_vec(l), vec![1, 2, 3, 4, 5]);
        op_drop(l);
        op_drop(r);
    }

    #[test]
    fn rope_slice_basic_and_across_concat_seam() {
        reset();
        // A slice reads a sub-range, including one that straddles a concat boundary.
        let buf = op_bytes_concat(bytes_leaf(&[1, 2]), bytes_leaf(&[3, 4])); // [1,2,3,4]
        op_dup(buf); // keep buf across the consuming slice
        let s = op_bytes_slice(buf, 1, 2); // [2,3] — spans the seam
        assert_eq!(op_bytes_len(s), 2);
        assert_eq!(bytes_to_vec(s), vec![2, 3]);
        // The parent is unchanged (persistence of the shared leaves).
        assert_eq!(bytes_to_vec(buf), vec![1, 2, 3, 4]);
        op_drop(buf);
        op_drop(s);
    }

    #[test]
    fn rope_slice_empty_and_edge_are_not_traps() {
        reset();
        let buf = bytes_leaf(&[1, 2, 3, 4]);
        op_dup(buf);
        op_dup(buf);
        let s0 = op_bytes_slice(buf, 0, 0); // len 0 → empty
        let s_end = op_bytes_slice(buf, 4, 0); // start == len, len 0 → empty, not a trap
        let s_full = op_bytes_slice(buf, 0, 4); // whole
        assert_eq!(op_bytes_len(s0), 0);
        assert_eq!(op_bytes_len(s_end), 0);
        assert_eq!(bytes_to_vec(s_full), vec![1, 2, 3, 4]);
        op_drop(s0);
        op_drop(s_end);
        op_drop(s_full);
    }

    #[test]
    #[should_panic]
    fn rope_slice_out_of_range_traps() {
        reset();
        let buf = bytes_leaf(&[1, 2, 3, 4]);
        let _ = op_bytes_slice(buf, 2, 3); // 2 + 3 = 5 > 4 → trap
    }

    #[test]
    fn rope_slice_of_slice_collapses() {
        reset();
        // A slice of a slice collapses onto the grandparent — the inner slice node is not retained,
        // so the chain depth stays 1 (bounded). Verify by content and that only the parent is pinned.
        let parent = bytes_leaf(&[10, 11, 12, 13, 14, 15]);
        let s1 = op_bytes_slice(parent, 1, 4); // [11,12,13,14], consumes parent
        let s2 = op_bytes_slice(s1, 1, 2); // [12,13] — collapses to slice(parent, 2, 2)
        // Inspect structure BEFORE any full read (a read would flatten s2 to a leaf). s2 must be a
        // slice (arity 1) whose single child is the ORIGINAL parent leaf, not the intermediate s1 —
        // proving the slice-of-slice collapsed. Also check the recomputed offset (1 + 1 = 2).
        assert_eq!(vec_arity(s2), 1, "s2 is still a slice before reading");
        let child = with_node(s2, Handle::NULL, |n| n.handles[0]);
        assert_eq!(vec_arity(child), 0, "collapsed slice points straight at the leaf parent");
        assert_eq!(with_node(s2, 99, |n| read_u32_at(&n.raw, 0)), 2, "offset collapsed to 1+1");
        // Now read: content is correct.
        assert_eq!(bytes_to_vec(s2), vec![12, 13]);
        op_drop(s2);
    }

    #[test]
    fn rope_get_flattens_in_place_and_is_unobservable() {
        reset();
        // The O(n²) guard: a right-leaning concat chain of depth ~N must
        // read out correctly, and after the first full read the node is a LEAF (flattened), so a
        // second pass reads the same bytes. Flatten is content-preserving ⇒ unobservable.
        let mut rope = bytes_leaf(&[0]);
        for k in 1..300u32 {
            rope = op_bytes_concat(rope, bytes_leaf(&[(k & 0xff) as u8]));
        }
        assert_eq!(op_bytes_len(rope), 300);
        // Before the first full read this is a concat node (arity 2).
        assert_eq!(vec_arity(rope), 2, "still a rope before first full read");
        let first: Vec<u8> = bytes_to_vec(rope);
        // After a full read it has flattened to a leaf (arity 0).
        assert_eq!(vec_arity(rope), 0, "flattened to a leaf on first full read");
        let second: Vec<u8> = bytes_to_vec(rope); // now O(1)/byte
        assert_eq!(first, second, "flatten is unobservable — same bytes");
        assert_eq!(first.len(), 300);
        assert_eq!(first[0], 0);
        assert_eq!(first[299], (299u32 & 0xff) as u8);
        op_drop(rope);
    }

    #[test]
    fn rope_whole_reclaims_on_drop() {
        reset();
        // The existing iterative op_drop reclaims a concat/slice tree with no new RC code.
        let before = live_nodes();
        let rope = op_bytes_concat(
            op_bytes_concat(bytes_leaf(&[1, 2]), bytes_leaf(&[3])),
            op_bytes_slice(bytes_leaf(&[9, 8, 7]), 1, 2),
        );
        assert!(live_nodes() > before);
        op_drop(rope);
        assert_eq!(live_nodes(), before, "whole rope (concats, slice, leaves) reclaimed");
    }

    #[test]
    fn rope_shared_leaf_survives_until_last_owner() {
        reset();
        // A leaf shared between two concat ropes survives while either rope holds it.
        let before = live_nodes();
        let shared = bytes_leaf(&[42, 43]);
        op_dup(shared); // second owner
        let r1 = op_bytes_concat(shared, bytes_leaf(&[1]));
        let r2 = op_bytes_concat(shared, bytes_leaf(&[2]));
        op_drop(r1);
        // r2 still reads the shared leaf's bytes.
        assert_eq!(bytes_to_vec(r2), vec![42, 43, 2]);
        assert!(live_nodes() > before, "shared leaf + r2 still live");
        op_drop(r2);
        assert_eq!(live_nodes(), before, "both ropes gone: full reclamation");
    }

    #[test]
    fn rope_compact_materializes_and_releases_parent() {
        reset();
        // #Retained Storage: a small slice of a large parent pins the whole parent; compact
        // materializes the sub-range into an independent leaf and drops the parent, freeing it.
        let before = live_nodes();
        let parent = bytes_leaf(&[0u8; 1000]); // one large leaf
        let s = op_bytes_slice(parent, 10, 3); // pins the 1000-byte parent
        assert_eq!(live_nodes(), before + 2, "large parent + slice node both live");
        let c = op_bytes_compact(s); // flatten → independent 3-byte leaf, parent released
        assert_eq!(c, s, "compact returns the same handle, now a leaf");
        assert_eq!(vec_arity(c), 0, "compacted to a leaf");
        assert_eq!(op_bytes_len(c), 3);
        assert_eq!(live_nodes(), before + 1, "the 1000-byte parent was released by compact");
        op_drop(c);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn rope_slice_content_matches_copy_over_many_offsets() {
        reset();
        // Exhaustive-ish contract check: for a built-up rope, every slice(start,len) equals the
        // same sub-range of the logical bytes — a rope is indistinguishable from a flat copy.
        let logical: Vec<u8> = (0..40u8).collect();
        // Build the same bytes as a right-leaning rope of 4 leaves of 10.
        let base = op_bytes_concat(
            op_bytes_concat(bytes_leaf(&logical[0..10]), bytes_leaf(&logical[10..20])),
            op_bytes_concat(bytes_leaf(&logical[20..30]), bytes_leaf(&logical[30..40])),
        );
        for start in 0..40u32 {
            for len in 0..=(40 - start) {
                op_dup(base);
                let s = op_bytes_slice(base, start, len);
                let got = bytes_to_vec(s);
                let want = &logical[start as usize..(start + len) as usize];
                assert_eq!(got, want, "slice({start},{len}) must equal the copy");
                op_drop(s);
            }
        }
        op_drop(base);
    }

    #[test]
    fn rope_deep_concat_reclaims_without_stack_overflow() {
        reset();
        // A deep unflattened rope must both reclaim (iterative op_drop) and flatten (iterative walk)
        // without overflowing the wasm call stack — the same discipline the free cascade uses.
        let before = live_nodes();
        let mut rope = bytes_leaf(&[0]);
        for k in 1..5000u32 {
            rope = op_bytes_concat(rope, bytes_leaf(&[(k & 0xff) as u8]));
        }
        // Read one byte near the end: forces a full flatten of a depth-~5000 rope (iterative walk).
        assert_eq!(op_bytes_get(rope, 4999), 4999u32 & 0xff);
        assert_eq!(vec_arity(rope), 0, "deep rope flattened iteratively");
        op_drop(rope);
        assert_eq!(live_nodes(), before, "deep rope fully reclaimed");
    }

    // ── CHAMP node core: bitmaps, slots, discrimination, structural hash + eq ─────────────

    #[test]
    fn champ_popcount_and_slot_indices() {
        reset();
        // datamap = 0b1010 ⇒ two inline entries (bits 1 and 3).
        assert_eq!(data_count(0b1010), 2);
        assert_eq!(subnode_count(0b1010), 2);
        // entry_index_for_slot counts set bits strictly below the slot.
        assert_eq!(entry_index_for_slot(0b1010, 0), 0); // no bits below 0
        assert_eq!(entry_index_for_slot(0b1010, 1), 0); // bit1 is the first entry
        assert_eq!(entry_index_for_slot(0b1010, 2), 1); // one bit (bit1) below slot 2
        assert_eq!(entry_index_for_slot(0b1010, 3), 1); // bit3 is the second entry
        assert_eq!(entry_index_for_slot(0b1010, 4), 2);
        // subnode indices follow the same arithmetic on the nodemap.
        assert_eq!(subnode_index_for_slot(0b1010, 3), 1);
        // High slot (bit 31) must not overflow `1 << i`.
        assert_eq!(entry_index_for_slot(0xffff_ffff, 31), 31);
    }

    #[test]
    fn champ_level_index_extracts_the_right_5_bits() {
        reset();
        // Craft a hash whose 5-bit digits are distinct per level.
        // level 0 = bits [0,5), level 1 = bits [5,10), level 6 = bits [30,32) (only 2 bits left).
        let hash: u32 = (0b00011) | (0b01010 << 5) | (0b10 << 30);
        assert_eq!(level_index(hash, 0), 0b00011);
        assert_eq!(level_index(hash, 1), 0b01010);
        assert_eq!(level_index(hash, 6), 0b10); // top 2 bits
    }

    #[test]
    fn champ_header_round_trips() {
        reset();
        let raw = champ_header(0xdead, 0xbeef, 7);
        assert_eq!(raw.len(), CHAMP_HEADER_SIZE);
        assert_eq!(champ_datamap(&raw), 0xdead);
        assert_eq!(champ_nodemap(&raw), 0xbeef);
        assert_eq!(champ_size(&raw), 7);
    }

    #[test]
    fn champ_empty_vs_collision_vs_normal_discrimination() {
        reset();
        let before = live_nodes();
        // Empty node: both bitmaps 0, no handles.
        let empty = alloc_raw(Vec::new(), champ_header(0, 0, 0));
        assert!(is_empty_node(empty));
        assert!(!is_collision_node(empty));
        // Collision node: both bitmaps 0 but holds entries.
        let e0 = op_box_int(1);
        let collision = alloc_raw(vec![e0], champ_header(0, 0, 1));
        assert!(!is_empty_node(collision));
        assert!(is_collision_node(collision));
        // Normal node: a datamap bit is set.
        let k = op_box_int(2);
        let v = op_box_int(3);
        let normal = alloc_raw(vec![k, v], champ_header(0b1, 0, 1));
        assert!(!is_empty_node(normal));
        assert!(!is_collision_node(normal));
        // A NULL handle is treated as empty (benign), never a collision.
        assert!(is_empty_node(Handle::NULL));
        assert!(!is_collision_node(Handle::NULL));
        op_drop(empty);
        op_drop(collision);
        op_drop(normal);
        assert_eq!(live_nodes(), before, "discrimination test reclaimed all nodes");
    }

    // Build a small normal CHAMP node owning two int leaves as one k/v entry (datamap bit 0).
    fn champ_kv_node(k: i64, v: i64) -> Handle {
        alloc_raw(vec![op_box_int(k), op_box_int(v)], champ_header(0b1, 0, 1))
    }

    #[test]
    fn champ_hash_is_deterministic_and_structural() {
        reset();
        let before = live_nodes();
        let a = champ_kv_node(10, 20);
        let b = champ_kv_node(10, 20); // structurally identical, distinct allocation
        let c = champ_kv_node(10, 21); // differs in a child's raw
        // Deterministic: same handle hashes the same across calls.
        assert_eq!(champ_hash(a), champ_hash(a));
        // Structural: equal-structured distinct nodes hash equal.
        assert_eq!(champ_hash(a), champ_hash(b));
        // Different structure ⇒ (very likely) different hash.
        assert_ne!(champ_hash(a), champ_hash(c));
        // Null hashes to the offset basis, deterministically.
        assert_eq!(champ_hash(Handle::NULL), champ_hash(Handle::NULL));
        op_drop(a);
        op_drop(b);
        op_drop(c);
        assert_eq!(live_nodes(), before, "hash test reclaimed all nodes");
    }

    /// An INDEPENDENT, naive RECURSIVE reference for the structural hash: FNV-1a over a node's own
    /// canonical raw bytes, then over each child's reference hash (LE). Deliberately written
    /// differently from the production iterative walk — no worklist, no leaf fast path — so it is a
    /// genuine oracle for `champ_hash`, not a copy of it. Children are folded in REVERSE index order
    /// because the production walk pushes children onto its worklist in order and pops them LIFO, so
    /// `results` presents them last-child-first; reproducing that here makes this a faithful oracle
    /// (the exact byte discipline the fast path and refactor must not disturb, not a re-invented one).
    fn champ_hash_ref(h: Handle) -> u32 {
        let (raw, arity) = node_raw_arity(h);
        let mut acc = FNV_OFFSET;
        for &b in &raw {
            acc = fnv_step(acc, b);
        }
        if !is_immediate(h) {
            with_node(h, (), |n| {
                for i in (0..arity).rev() {
                    let ch = champ_hash_ref(n.handles[i]);
                    for b in ch.to_le_bytes() {
                        acc = fnv_step(acc, b);
                    }
                }
            });
        }
        acc
    }

    /// The allocation-free arity-0 fast path in `champ_hash` must be BYTE-IDENTICAL to the general
    /// worklist walk (a hash drift would silently corrupt map/set placement and cross-version stability).
    /// Assert equality against the independent recursive oracle across the leaf cases the fast path
    /// covers — immediates (inline unit/bool/int), boxed out-of-window ints, floats, strings, empty
    /// bytes — AND across compounds (arrays, sums, a real CHAMP node, deep nesting) that take the
    /// general walk, so the shared `champ_node_raw_hash` fold is pinned on both branches at once.
    #[test]
    fn champ_hash_matches_naive_reference_across_shapes() {
        reset();
        let before = live_nodes();

        // Leaf / immediate cases — these hit the arity-0 fast path.
        let leaves = [
            imm_unit(),
            imm_bool(false),
            imm_bool(true),
            op_box_int(0),
            op_box_int(7),
            op_box_int(-1),
            op_box_int(FIXNUM_MAX),           // largest inline fixnum
            op_box_int(FIXNUM_MAX + 1),       // first BOXED int (out of the inline window)
            op_box_int(FIXNUM_MIN - 1),       // first boxed negative
            op_box_float(3.5),
            op_box_float(-0.0),               // distinct bits from 0.0
            op_str_new(String::new()),
            op_str_new("cadenza".to_string()),
            op_bytes_alloc(0),
            Handle::NULL,                     // null folds to the bare offset basis on both paths
        ];
        for &h in &leaves {
            assert_eq!(
                champ_hash(h),
                champ_hash_ref(h),
                "leaf/immediate fast path must match the naive reference",
            );
        }

        // An inline int and its BOXED twin must hash equal (open-Q#8) — one takes the fast path, the
        // other would too (both arity 0), but the bytes folded must be the canonical LE bytes alike.
        assert_eq!(
            champ_hash(imm_int(5)),
            champ_hash_ref(op_box_int(5)),
            "inline and boxed twin of the same int hash equal",
        );

        // Compound cases — these take the general worklist walk (arity > 0), exercising the shared
        // `champ_node_raw_hash` fold plus child folding.
        let arr = op_arr_alloc(2);
        op_arr_set(arr, 0, op_box_int(FIXNUM_MAX + 100)); // boxed child so a real leaf node is walked
        op_arr_set(arr, 1, imm_bool(true));               // immediate child folded via the fast leaf
        let sum = op_sum_new(3, op_box_int(9));
        let kv = champ_kv_node(10, 20);                    // a real CHAMP node with a set datamap bit
        let nested = op_arr_alloc(2);
        op_arr_set(nested, 0, arr); // arr's ownership moves into `nested`
        op_arr_set(nested, 1, sum); // sum's ownership moves into `nested`
        for &h in &[nested, kv] {
            assert_eq!(
                champ_hash(h),
                champ_hash_ref(h),
                "compound general walk must match the naive reference",
            );
        }

        // Reclaim everything (the boxed leaves, the strings, and the compounds).
        for &h in &leaves {
            op_drop(h);
        }
        op_drop(nested); // frees arr + sum + their children transitively
        op_drop(kv);
        assert_eq!(live_nodes(), before, "reference-hash test reclaimed all nodes");
    }

    #[test]
    fn map_keyed_by_shallow_compound_roundtrips_and_dedups() {
        reset();
        let before = live_nodes();
        // Exercises the shallow-compound champ_hash fast path via its real use: a map keyed by small
        // 2-tuples `(a, b)`. Insert distinct tuple keys, look them up, then re-insert an EQUAL-BUT-
        // DISTINCT-POINTER tuple key and confirm it OVERWRITES (deduped by structural hash+eq, not by
        // pointer) — which only works if the fast path hashes structurally-equal tuples identically.
        let tuple = |a: i64, b: i64| -> Handle {
            let t = op_arr_alloc(2);
            op_arr_set(t, 0, op_box_int(a));
            op_arr_set(t, 1, op_box_int(b));
            t
        };
        let mut m = op_map_empty();
        for &(a, b, v) in &[(1i64, 2i64, 10i64), (3, 4, 20), (1, 9, 30), (5, 5, 40)] {
            m = op_map_insert(m, tuple(a, b), op_box_int(v));
        }
        assert_eq!(op_map_size(m), 4, "four distinct tuple keys");
        // Look up by a FRESH tuple with the same contents — must hit (structural key match).
        for &(a, b, v) in &[(1i64, 2i64, 10i64), (3, 4, 20), (1, 9, 30), (5, 5, 40)] {
            let probe = tuple(a, b);
            let got = op_map_lookup(m, probe);
            assert_ne!(got, Handle::NULL, "tuple key ({a},{b}) found via a fresh, equal probe");
            assert_eq!(op_get_int(got), v, "tuple key ({a},{b}) maps to {v}");
            op_drop(probe);
        }
        // Overwrite (1,2) via a fresh equal key — size stays 4, value updates (dedup by hash+eq).
        m = op_map_insert(m, tuple(1, 2), op_box_int(999));
        assert_eq!(op_map_size(m), 4, "equal tuple key overwrote, did not add");
        let probe = tuple(1, 2);
        assert_eq!(op_get_int(op_map_lookup(m, probe)), 999, "value overwritten");
        op_drop(probe);
        // A miss on an absent tuple.
        let miss = tuple(7, 7);
        assert_eq!(op_map_lookup(m, miss), Handle::NULL, "absent tuple key misses");
        op_drop(miss);
        op_drop(m);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn champ_hash_deep_is_stack_safe() {
        reset();
        let before = live_nodes();
        // Nest single-child nodes ~5000 deep: recursion would overflow; the worklist must not.
        let mut node = op_box_int(0);
        for _ in 0..5000u32 {
            node = alloc_raw(vec![node], champ_header(0, 1, 1));
        }
        let _ = champ_hash(node); // must not overflow the stack
        op_drop(node);
        assert_eq!(live_nodes(), before, "deep hash test reclaimed all nodes");
    }

    #[test]
    fn champ_eq_structural_and_null_safe() {
        reset();
        let before = live_nodes();
        let a = champ_kv_node(10, 20);
        let b = champ_kv_node(10, 20); // structurally equal, distinct pointers
        let c = champ_kv_node(10, 21); // differing child raw
        let d = alloc_raw(vec![op_box_int(10)], champ_header(0b1, 0, 1)); // differing arity/raw
        assert!(champ_eq(a, a)); // same pointer
        assert!(champ_eq(a, b)); // structurally equal
        assert!(!champ_eq(a, c)); // child differs
        assert!(!champ_eq(a, d)); // arity + raw differ
        // Null-safety.
        assert!(champ_eq(Handle::NULL, Handle::NULL));
        assert!(!champ_eq(a, Handle::NULL));
        assert!(!champ_eq(Handle::NULL, a));
        op_drop(a);
        op_drop(b);
        op_drop(c);
        op_drop(d);
        assert_eq!(live_nodes(), before, "eq test reclaimed all nodes");
    }

    #[test]
    fn champ_eq_and_cmp_descend_nested_compounds_via_lazy_worklist() {
        reset();
        let before = live_nodes();
        // Guards the LAZY worklist in champ_eq/champ_key_cmp (the root pair is handled with no Vec; the
        // worklist is allocated only when a compound pushes children). This test exercises the path the
        // scalar fast case does NOT: deep NESTED compounds that force the worklist to be created and to
        // drive multi-level descent. Build a 4-level nest [[[[leaf]]]] two ways, differing only at the
        // DEEPEST leaf, and confirm eq/cmp find the difference (proving descent reaches the bottom) and
        // that identical nests compare Equal / eq. Also check cmp is consistent with eq and antisymmetric.
        fn nest(leaf: i64) -> Handle {
            // arity-1 compound chain: node -> node -> node -> node -> boxed-leaf. Use out-of-window ints
            // so the leaves are real (boxed) nodes, making every level a genuine compound descent.
            let mut h = boxed_int_leaf(leaf);
            for _ in 0..4 {
                h = alloc(vec![h], Vec::new()); // arity-1 compound (empty raw)
            }
            h
        }
        let big = (1i64 << 40) + 7; // out-of-fixnum-window ⇒ boxed leaf
        let x = nest(big);
        let y = nest(big); // structurally identical, distinct pointers all the way down
        let z = nest(big + 1); // differs ONLY at the deepest leaf

        // Identical nests: eq true, cmp Equal — the worklist must fully descend all 4 levels to confirm.
        assert!(champ_eq(x, y), "identical 4-level nests are eq (full descent)");
        assert_eq!(champ_key_cmp(x, y), core::cmp::Ordering::Equal, "identical nests cmp Equal");
        // Differ only at the deepest leaf: eq false, cmp non-Equal, and antisymmetric.
        assert!(!champ_eq(x, z), "nests differing at the deepest leaf are not eq");
        let ord = champ_key_cmp(x, z);
        assert_ne!(ord, core::cmp::Ordering::Equal, "deep-leaf difference is found by cmp");
        assert_eq!(champ_key_cmp(z, x), ord.reverse(), "cmp is antisymmetric across the deep difference");
        // eq/cmp consistency at depth.
        assert_eq!(champ_eq(x, z), champ_key_cmp(x, z) == core::cmp::Ordering::Equal);

        op_drop(x);
        op_drop(y);
        op_drop(z);
        assert_eq!(live_nodes(), before, "nested-compound eq/cmp test reclaimed all nodes");
    }

    #[test]
    fn champ_eq_and_cmp_shallow_compound_fast_path_is_consistent() {
        reset();
        let before = live_nodes();
        // Guards the SHALLOW-compound fast path in champ_eq/champ_key_cmp (both children arity-0, no
        // worklist). It must agree with the general walk across every difference kind, and champ_key_cmp
        // must stay CONSISTENT with champ_eq (cmp==Equal iff eq) and ANTISYMMETRIC. Build 2-tuples that
        // differ at child 0, at child 1, in arity, in raw — plus a NESTED tuple (a tuple whose child is
        // itself a tuple) to confirm the fast path correctly DECLINES (falls to the general walk) there.
        let tup = |cols: &[Handle]| -> Handle {
            let t = op_arr_alloc(cols.len() as u32);
            for (i, &c) in cols.iter().enumerate() {
                op_arr_set(t, i as u32, c);
            }
            t
        };
        // Shallow tuples over immediates + boxed leaves (out-of-window so real nodes).
        let big = |v: i64| (1i64 << 40) + v;
        let a = tup(&[op_box_int(1), op_box_int(2)]);
        let a2 = tup(&[op_box_int(1), op_box_int(2)]); // structurally equal, distinct pointer
        let b = tup(&[op_box_int(1), op_box_int(3)]); // differs at child 1
        let c = tup(&[op_box_int(0), op_box_int(2)]); // differs at child 0
        let d = tup(&[op_box_int(1)]);                // differs in arity
        let e = tup(&[boxed_int_leaf(big(1)), boxed_int_leaf(big(2))]); // boxed-leaf children (shallow)
        let e2 = tup(&[boxed_int_leaf(big(1)), boxed_int_leaf(big(2))]); // equal to e
        let e3 = tup(&[boxed_int_leaf(big(1)), boxed_int_leaf(big(9))]); // differs at child 1 (boxed)
        // A NESTED tuple: child 0 is itself a tuple → the fast path must decline to the general walk.
        let nested = tup(&[tup(&[op_box_int(1), op_box_int(2)]), op_box_int(9)]);
        let nested2 = tup(&[tup(&[op_box_int(1), op_box_int(2)]), op_box_int(9)]);

        // Equalities.
        assert!(champ_eq(a, a2), "equal shallow tuples are eq");
        assert_eq!(champ_key_cmp(a, a2), core::cmp::Ordering::Equal, "equal shallow tuples cmp Equal");
        assert!(champ_eq(e, e2), "equal shallow tuples over boxed leaves are eq");
        assert_eq!(champ_key_cmp(e, e2), core::cmp::Ordering::Equal);
        assert!(champ_eq(nested, nested2), "equal NESTED tuples are eq (via the general walk)");
        assert_eq!(champ_key_cmp(nested, nested2), core::cmp::Ordering::Equal);
        // Inequalities + eq/cmp consistency + antisymmetry across each difference kind.
        for &(x, y) in &[(a, b), (a, c), (a, d), (e, e3), (nested, a)] {
            assert!(!champ_eq(x, y), "differing tuples are not eq");
            let ord = champ_key_cmp(x, y);
            assert_ne!(ord, core::cmp::Ordering::Equal, "cmp finds the difference");
            assert_eq!(champ_key_cmp(y, x), ord.reverse(), "cmp antisymmetric");
            assert_eq!(champ_eq(x, y), champ_key_cmp(x, y) == core::cmp::Ordering::Equal, "cmp==Equal iff eq");
        }
        for &h in &[a, a2, b, c, d, e, e2, e3, nested, nested2] {
            op_drop(h);
        }
        assert_eq!(live_nodes(), before, "shallow-compound eq/cmp test reclaimed all nodes");
    }

    /// Guard the alloc-free `with_raw_arity` fast path in `champ_eq`/`champ_key_cmp` against the naive
    /// `node_raw_arity` (Vec-cloning) reference it replaced, across every shape whose comparison can
    /// touch the immediate branch: inline unit/bool/int, a hand-BOXED int twin, an out-of-window boxed
    /// int, floats (incl -0.0), empty/nonempty strings, empty bytes, and NULL. For each pair where at
    /// least one side is IMMEDIATE — the only path my edit touched — the new `champ_eq` must equal
    /// `rx==ry && ax==ay` over the old `node_raw_arity`, and `champ_key_cmp` must equal
    /// `rx.cmp(&ry).then(ax.cmp(&ay))`. Since every operand here is arity-0, that single-node compare
    /// IS the whole verdict, so the reference is exact. (Pairs where NEITHER side is immediate — e.g.
    /// a real leaf vs NULL — go through `champ_eq`'s UNCHANGED non-immediate arm, which distinguishes
    /// NULL from a non-null leaf; the `node_raw_arity` model folds both to `([],0)` and so does NOT
    /// model that arm, hence they're excluded.) Catches any drift in the ≤8-byte materialization/borrow.
    #[test]
    fn with_raw_arity_matches_node_raw_arity_reference() {
        reset();
        let before = live_nodes();
        // Reference verdicts computed the OLD (allocating) way, so the fast path is graded, not trusted.
        fn ref_eq(x: Handle, y: Handle) -> bool {
            let (rx, ax) = node_raw_arity(x);
            let (ry, ay) = node_raw_arity(y);
            rx == ry && ax == ay
        }
        fn ref_cmp(x: Handle, y: Handle) -> core::cmp::Ordering {
            let (rx, ax) = node_raw_arity(x);
            let (ry, ay) = node_raw_arity(y);
            rx.cmp(&ry).then(ax.cmp(&ay))
        }
        // Every operand is arity-0 (immediate or leaf) so the immediate branch decides the verdict.
        let operands = [
            imm_unit(),
            imm_bool(false),
            imm_bool(true),
            op_box_int(0),          // inline fixnum
            op_box_int(-1),         // inline negative
            op_box_int(536_870_912),// FIXNUM_MAX+1 ⇒ boxed leaf
            boxed_int_leaf(0),      // hand-boxed twin of inline 0
            boxed_int_leaf(-1),
            op_box_float(0.0),
            op_box_float(-0.0),     // -0.0 ≠ 0.0 by raw bytes
            op_box_float(1.5),
            op_str_new(String::new()),
            op_str_new("hi".to_string()),
            op_bytes_alloc(0),
            Handle::NULL,
        ];
        for (i, &x) in operands.iter().enumerate() {
            for (j, &y) in operands.iter().enumerate() {
                // The fast path fires iff at least one side is immediate — the only code I changed.
                if !is_immediate(x) && !is_immediate(y) {
                    continue;
                }
                assert_eq!(
                    champ_eq(x, y),
                    ref_eq(x, y),
                    "champ_eq disagrees with node_raw_arity reference at ({i},{j})"
                );
                assert_eq!(
                    champ_key_cmp(x, y),
                    ref_cmp(x, y),
                    "champ_key_cmp disagrees with node_raw_arity reference at ({i},{j})"
                );
            }
        }
        // Immediates/NULL own no heap; free only the real leaves we allocated.
        for &h in &operands {
            if !is_immediate(h) && h != Handle::NULL {
                op_drop(h);
            }
        }
        assert_eq!(live_nodes(), before, "raw-arity reference test reclaimed all nodes");
    }

    // ── CHAMP persistent MAP: empty / lookup / insert / size ──────────────────────────────

    /// Look up integer `k` in `m` (borrows), returning its i64 value if present. Builds and drops a
    /// fresh probe key; never retains the borrowed value handle.
    fn mlookup_int(m: Handle, k: i64) -> Option<i64> {
        let probe = op_box_int(k);
        let v = op_map_lookup(m, probe);
        op_drop(probe);
        if v == Handle::NULL {
            None
        } else {
            Some(op_get_int(v))
        }
    }

    /// Insert `k => v` (both boxed ints) into `m`, consuming `m`.
    fn minsert_int(m: Handle, k: i64, v: i64) -> Handle {
        op_map_insert(m, op_box_int(k), op_box_int(v))
    }

    #[test]
    fn map_empty_is_size_zero_and_misses() {
        reset();
        let before = live_nodes();
        let m = op_map_empty();
        assert!(is_empty_node(m));
        assert_eq!(op_map_size(m), 0);
        assert_eq!(mlookup_int(m, 42), None);
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_single_insert_then_lookup() {
        reset();
        let before = live_nodes();
        let m = minsert_int(op_map_empty(), 7, 700);
        assert_eq!(op_map_size(m), 1);
        assert_eq!(mlookup_int(m, 7), Some(700));
        assert_eq!(mlookup_int(m, 8), None);
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_overwrite_dedups_and_does_not_leak() {
        reset();
        let before = live_nodes();
        let m = minsert_int(op_map_empty(), 5, 111);
        let m = minsert_int(m, 5, 222); // overwrite same key
        assert_eq!(op_map_size(m), 1, "overwrite keeps size");
        assert_eq!(mlookup_int(m, 5), Some(222), "value replaced");
        op_drop(m);
        assert_eq!(live_nodes(), before, "old value + duplicate key reclaimed");
    }

    #[test]
    fn map_many_distinct_keys_all_lookup() {
        reset();
        let before = live_nodes();
        let pairs = [(1i64, 10i64), (2, 20), (3, 30), (17, 170), (99, 990), (1000, 10000)];
        let mut m = op_map_empty();
        for &(k, v) in &pairs {
            m = minsert_int(m, k, v);
        }
        assert_eq!(op_map_size(m), pairs.len() as u32);
        for &(k, v) in &pairs {
            assert_eq!(mlookup_int(m, k), Some(v), "key {k}");
        }
        assert_eq!(mlookup_int(m, 12345), None);
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_forces_subnode_split() {
        reset();
        let before = live_nodes();
        // Find two ints whose hashes share low-5 bits but differ overall ⇒ a level-0 split.
        let mut by_low: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
        let mut split: Option<(i64, i64)> = None;
        let mut v = 0i64;
        while v < 100_000 {
            let k = op_box_int(v);
            let h = champ_hash(k);
            op_drop(k);
            let low = h & 0x1f;
            if let Some(&v0) = by_low.get(&low) {
                let k0 = op_box_int(v0);
                let h0 = champ_hash(k0);
                op_drop(k0);
                if h0 != h {
                    split = Some((v0, v));
                    break;
                }
            } else {
                by_low.insert(low, v);
            }
            v += 1;
        }
        let (a, b) = split.expect("two keys sharing low-5 hash bits");
        let m = minsert_int(minsert_int(op_map_empty(), a, 1), b, 2);
        assert_eq!(op_map_size(m), 2);
        assert_eq!(mlookup_int(m, a), Some(1));
        assert_eq!(mlookup_int(m, b), Some(2));
        // Root must now hold a subnode (the split), not two inline entries.
        let (dm, nm) = with_node(m, (0u32, 0u32), |n| (champ_datamap(&n.raw), champ_nodemap(&n.raw)));
        assert_eq!(data_count(dm), 0, "root has no inline entries after split");
        assert_eq!(subnode_count(nm), 1, "root created exactly one subnode");
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_forces_collision_node() {
        reset();
        let before = live_nodes();
        // Two DISTINCT keys with fully-equal 32-bit champ_hash ⇒ a collision node at the hash floor.
        // Boxed SMALL ints have 5 trailing zero bytes, which makes FNV-1a effectively injective over
        // them; so we search FULL-WIDTH payloads (a splitmix mix of a counter spreads all 8 bytes),
        // where the birthday bound over 2^32 yields a pair within a few hundred thousand samples.
        let mix = |c: u64| -> i64 {
            let mut z = c.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)) as i64
        };
        let mut seen: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
        let mut pair: Option<(i64, i64)> = None;
        let mut c = 0u64;
        while c < 3_000_000 {
            let payload = mix(c);
            let k = op_box_int(payload);
            let h = champ_hash(k);
            op_drop(k);
            match seen.get(&h) {
                Some(&p0) if p0 != payload => {
                    pair = Some((p0, payload));
                    break;
                }
                Some(_) => {} // same payload re-derived (mix is not injective) — ignore
                None => {
                    seen.insert(h, payload);
                }
            }
            c += 1;
        }
        let (a, b) = pair.expect("a full 32-bit FNV collision among full-width payloads");
        assert_ne!(a, b);
        let m = minsert_int(minsert_int(op_map_empty(), a, 1000), b, 2000);
        assert_eq!(op_map_size(m), 2, "both colliding keys counted");
        assert_eq!(mlookup_int(m, a), Some(1000));
        assert_eq!(mlookup_int(m, b), Some(2000));
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_persistence_and_structural_sharing() {
        reset();
        let before = live_nodes();
        // v1 has two entries; keep it while deriving v2 by dup'ing before the consuming insert.
        let v1 = minsert_int(minsert_int(op_map_empty(), 100, 1), 200, 2);
        op_dup(v1);
        let v2 = minsert_int(v1, 300, 3);
        // v1 unchanged.
        assert_eq!(op_map_size(v1), 2);
        assert_eq!(mlookup_int(v1, 100), Some(1));
        assert_eq!(mlookup_int(v1, 200), Some(2));
        assert_eq!(mlookup_int(v1, 300), None, "v1 never saw key 300");
        // v2 has the new entry plus the shared originals.
        assert_eq!(op_map_size(v2), 3);
        assert_eq!(mlookup_int(v2, 100), Some(1));
        assert_eq!(mlookup_int(v2, 200), Some(2));
        assert_eq!(mlookup_int(v2, 300), Some(3));
        op_drop(v1);
        op_drop(v2);
        assert_eq!(live_nodes(), before, "shared subtrees freed exactly once");
    }

    // ── CHAMP persistent MAP: remove (inverse of insert; canonicality) ────────────────────

    /// Remove integer `k` from `m`, consuming `m`. Builds and drops a fresh probe key (remove
    /// borrows the key).
    fn mremove_int(m: Handle, k: i64) -> Handle {
        let probe = op_box_int(k);
        let out = op_map_remove(m, probe);
        op_drop(probe);
        out
    }

    /// Two low-5-bit-colliding-but-distinct ints (forces a subnode split at level 0), reusing the
    /// U2 search. Returns `(a, b)`; both fresh probes are dropped.
    fn low5_split_pair() -> (i64, i64) {
        let mut by_low: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
        let mut v = 0i64;
        while v < 100_000 {
            let k = op_box_int(v);
            let h = champ_hash(k);
            op_drop(k);
            let low = h & 0x1f;
            if let Some(&v0) = by_low.get(&low) {
                let k0 = op_box_int(v0);
                let h0 = champ_hash(k0);
                op_drop(k0);
                if h0 != h {
                    return (v0, v);
                }
            } else {
                by_low.insert(low, v);
            }
            v += 1;
        }
        panic!("no low-5 split pair found");
    }

    /// Two DISTINCT full-width payloads whose 32-bit champ_hash is fully equal (forces a collision
    /// node), reusing the U2 birthday search over splitmix-spread payloads.
    fn full_hash_collision_pair() -> (i64, i64) {
        let mix = |c: u64| -> i64 {
            let mut z = c.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)) as i64
        };
        let mut seen: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
        let mut c = 0u64;
        while c < 3_000_000 {
            let payload = mix(c);
            let k = op_box_int(payload);
            let h = champ_hash(k);
            op_drop(k);
            match seen.get(&h) {
                Some(&p0) if p0 != payload => return (p0, payload),
                Some(_) => {}
                None => {
                    seen.insert(h, payload);
                }
            }
            c += 1;
        }
        panic!("no full 32-bit collision found");
    }

    #[test]
    fn map_remove_present_key() {
        reset();
        let before = live_nodes();
        let mut m = op_map_empty();
        for &(k, v) in &[(1i64, 10i64), (2, 20), (3, 30)] {
            m = minsert_int(m, k, v);
        }
        m = mremove_int(m, 2);
        assert_eq!(op_map_size(m), 2);
        assert_eq!(mlookup_int(m, 1), Some(10));
        assert_eq!(mlookup_int(m, 2), None);
        assert_eq!(mlookup_int(m, 3), Some(30));
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_remove_inplace_drain_shifts_entries_and_subnodes_canonically() {
        reset();
        let before = live_nodes();
        // Guards the in-place `Vec::drain(base..base+stride)` in the datamap-found remove branch: the
        // removed entry's columns sit in the entry region (BEFORE the subnodes), so draining them must
        // shift BOTH the remaining inline entries AND every subnode left by `stride`, preserving the
        // canonical layout. Build a node with several inline entries PLUS a subnode (from a low-5
        // split), remove an inline entry whose slot is BELOW the subnode's (so the drain shifts the
        // subnode), and assert byte-identical (champ_eq + champ_hash) to the copy-path build + a fresh
        // build of the surviving keys. Value maps (stride 2) so the drain removes two columns at once.
        let (sa, sb) = low5_split_pair(); // sa,sb share low-5 ⇒ a subnode at the root
        // Ordinary keys in distinct low-5 slots so they stay inline alongside the subnode.
        let inline_keys = [1i64, 2, 3, 4];
        let remove_key = 2i64; // an inline entry to remove (present, not the split pair)
        let build = |shared: bool| -> Handle {
            let mut m = op_map_empty();
            let mut all: Vec<(i64, i64)> = vec![(sa, 100), (sb, 200)];
            for &k in &inline_keys {
                all.push((k, k * 10));
            }
            for &(k, v) in &all {
                if shared {
                    op_dup(m);
                    let old = m;
                    m = minsert_int(m, k, v);
                    op_drop(old);
                } else {
                    m = minsert_int(m, k, v);
                }
            }
            if shared {
                op_dup(m);
                let old = m;
                m = mremove_int(m, remove_key);
                op_drop(old);
            } else {
                m = mremove_int(m, remove_key); // unique → the in-place drain path
            }
            m
        };
        let fbip = build(false);
        let copy = build(true);
        let fresh = {
            // A fresh map of exactly the survivors, in a different insert order.
            let mut m = op_map_empty();
            for &(k, v) in &[(4i64, 40), (sa, 100), (3, 30), (sb, 200), (1, 10)] {
                m = minsert_int(m, k, v);
            }
            m
        };
        assert!(champ_eq(fbip, copy), "in-place drain remove == copy-path remove (canonical)");
        assert_eq!(champ_hash(fbip), champ_hash(copy), "byte-identical canonical shape");
        assert!(champ_eq(fbip, fresh), "== a fresh map of the survivors (order-independent canonical)");
        assert_eq!(mlookup_int(fbip, remove_key), None, "removed key absent");
        for &(k, v) in &[(sa, 100i64), (sb, 200), (1, 10), (3, 30), (4, 40)] {
            assert_eq!(mlookup_int(fbip, k), Some(v), "survivor {k} intact after the drain shift");
        }
        op_drop(fbip);
        op_drop(copy);
        op_drop(fresh);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn map_remove_absent_key_is_noop() {
        reset();
        let before = live_nodes();
        let mut m = op_map_empty();
        for &(k, v) in &[(1i64, 10i64), (2, 20)] {
            m = minsert_int(m, k, v);
        }
        m = mremove_int(m, 999); // absent
        assert_eq!(op_map_size(m), 2);
        assert_eq!(mlookup_int(m, 1), Some(10));
        assert_eq!(mlookup_int(m, 2), Some(20));
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_remove_inplace_descend_no_collapse_is_canonical() {
        reset();
        let before = live_nodes();
        // Guards the allocation-lazy champ_remove_fbip: the NON-COLLAPSE descend path now writes the
        // rebuilt child slot + patches the size header IN PLACE (champ_set_child_and_size_inplace),
        // and every ABSENT check reads slots by borrow with NO handle-vector clone. Build a subnode
        // holding THREE entries that share low-5 hash bits (so removing one leaves ≥2 → the subnode is
        // kept, not collapsed), and verify: (1) the FBIP in-place result is byte-identical (champ_eq +
        // champ_hash) to the copy-path build of the same final contents; (2) all survivors present, the
        // removed key absent; (3) an absent-key remove in the same shape is a true no-op; (4) no leak.
        //
        // low5_split_pair gives two low-5 colliders; a third same-low-5 key makes the level-1 subnode
        // hold 3 entries. Search for it the same way low5_split_pair does. `hash_of` boxes, hashes, and
        // drops a probe so nothing leaks during the search.
        let hash_of = |x: i64| -> u32 {
            let k = op_box_int(x);
            let h = champ_hash(k);
            op_drop(k);
            h
        };
        let (a, b) = low5_split_pair();
        let (ha, hb) = (hash_of(a), hash_of(b));
        let low = ha & 0x1f;
        let mut c = None;
        let mut v = 0i64;
        while v < 200_000 {
            if v != a && v != b {
                let h = hash_of(v);
                if h & 0x1f == low && h != ha && h != hb {
                    c = Some(v);
                    break;
                }
            }
            v += 1;
        }
        let c = c.expect("a third key sharing the low-5 bits");

        // Build {a,b,c, plus two ordinary keys} the FBIP (unique) way, then remove `b` (a deep, non-
        // collapsing removal since {a,c} keep the subnode at ≥2 entries).
        let build_then_remove_b = |shared: bool| -> Handle {
            let mut m = op_map_empty();
            for &(k, val) in &[(a, 1i64), (b, 2), (c, 3), (7i64, 70), (8, 80)] {
                if shared {
                    op_dup(m);
                    let old = m;
                    m = minsert_int(m, k, val);
                    op_drop(old);
                } else {
                    m = minsert_int(m, k, val);
                }
            }
            if shared {
                op_dup(m);
                let old = m;
                m = mremove_int(m, b);
                op_drop(old);
            } else {
                m = mremove_int(m, b); // unique → the in-place descend path
            }
            m
        };
        let fbip = build_then_remove_b(false);
        let copy = build_then_remove_b(true);
        assert!(champ_eq(fbip, copy), "in-place-descend remove == copy-path remove (canonical)");
        assert_eq!(champ_hash(fbip), champ_hash(copy), "byte-identical canonical shape");
        assert_eq!(op_map_size(fbip), 4, "one of five entries removed");
        assert_eq!(mlookup_int(fbip, b), None, "removed key absent");
        for &(k, val) in &[(a, 1i64), (c, 3), (7i64, 70), (8, 80)] {
            assert_eq!(mlookup_int(fbip, k), Some(val), "survivor key {k} intact");
        }
        // Absent-key remove on this shape is a true no-op (zero alloc path), value preserved.
        let fbip = mremove_int(fbip, 999_999);
        assert_eq!(op_map_size(fbip), 4, "absent remove leaves size");
        assert_eq!(mlookup_int(fbip, a), Some(1));
        op_drop(fbip);
        op_drop(copy);
        assert_eq!(live_nodes(), before, "no leak across the in-place-descend removes");
    }

    #[test]
    fn map_remove_down_to_canonical_empty() {
        reset();
        let before = live_nodes();
        let keys = [1i64, 2, 3, 17, 99, 1000];
        let mut m = op_map_empty();
        for &k in &keys {
            m = minsert_int(m, k, k * 10);
        }
        for &k in &keys {
            m = mremove_int(m, k);
        }
        // Byte-identical to a fresh empty map.
        let empty = op_map_empty();
        assert!(is_empty_node(m));
        assert_eq!(op_map_size(m), 0);
        assert!(champ_eq(m, empty), "remove-to-empty is byte-identical to op_map_empty()");
        assert_eq!(champ_hash(m), champ_hash(empty));
        op_drop(empty);
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_remove_collapses_subnode_to_inline() {
        reset();
        let before = live_nodes();
        let (a, b) = low5_split_pair();
        let m = minsert_int(minsert_int(op_map_empty(), a, 1), b, 2);
        // Sanity: the split produced a subnode at the root.
        let (dm0, nm0) = with_node(m, (0u32, 0u32), |n| (champ_datamap(&n.raw), champ_nodemap(&n.raw)));
        assert_eq!((data_count(dm0), subnode_count(nm0)), (0, 1), "split created a subnode");
        // Remove one of the two: the subnode must collapse back into a single inline entry.
        let m = mremove_int(m, a);
        let (dm, nm) = with_node(m, (0u32, 0u32), |n| (champ_datamap(&n.raw), champ_nodemap(&n.raw)));
        assert_eq!(data_count(dm), 1, "root collapsed to one inline entry");
        assert_eq!(nm, 0, "root has no subnodes after collapse");
        assert_eq!(op_map_size(m), 1);
        assert_eq!(mlookup_int(m, a), None);
        assert_eq!(mlookup_int(m, b), Some(2));
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_remove_collapses_collision_to_inline() {
        reset();
        let before = live_nodes();
        let (a, b) = full_hash_collision_pair();
        let m = minsert_int(minsert_int(op_map_empty(), a, 1000), b, 2000);
        assert_eq!(op_map_size(m), 2);
        // Remove one colliding key: the collision node collapses to a single inline entry.
        let m = mremove_int(m, a);
        assert_eq!(op_map_size(m), 1);
        assert_eq!(mlookup_int(m, a), None);
        assert_eq!(mlookup_int(m, b), Some(2000));
        // The survivor must be reachable as a plain inline entry, byte-identical to inserting it alone.
        let solo = minsert_int(op_map_empty(), b, 2000);
        assert!(champ_eq(m, solo), "collision collapse is canonical (== fresh single-entry map)");
        assert_eq!(champ_hash(m), champ_hash(solo));
        op_drop(solo);
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_remove_inplace_collapse_repositions_amid_entries_and_subnodes_canonically() {
        reset();
        let before = live_nodes();
        // Guards the in-place COLLAPSE (remove the collapsed subnode's handle, then splice the inlined
        // entry's columns into the entry region on the taken vec). The load-bearing case is a node that
        // holds OTHER inline entries AND OTHER subnodes besides the collapsing one — the remove+insert
        // must reposition so the inlined entry lands canonically among the entries and the surviving
        // subnodes stay correct. Two low-5 split pairs create TWO subnodes at the root; ordinary keys
        // add inline entries; removing one key from one split pair collapses THAT subnode while the
        // other subnode + inline entries remain. Assert byte-identical to the copy-path + fresh build.
        let (a, b) = low5_split_pair();
        let (c, d) = full_hash_collision_pair(); // a second, distinct pair → a second subnode
        let build = |shared: bool| -> Handle {
            let mut m = op_map_empty();
            // a,b (subnode #1) + c,d (subnode #2) + ordinary inline entries.
            let mut seq: Vec<(i64, i64)> = vec![(a, 1), (b, 2), (c, 3), (d, 4), (5i64, 50), (6, 60), (7, 70)];
            // Remove `a`: subnode #1 (from the a,b split) collapses to inline `b`, while subnode #2
            // (c,d) and the inline entries stay — exercising the reposition amid entries + a subnode.
            seq.push((a, -1)); // marker handled below
            let mut m2 = m;
            let inserts = &seq[..seq.len() - 1];
            for &(k, v) in inserts {
                if shared {
                    op_dup(m2);
                    let old = m2;
                    m2 = minsert_int(m2, k, v);
                    op_drop(old);
                } else {
                    m2 = minsert_int(m2, k, v);
                }
            }
            if shared {
                op_dup(m2);
                let old = m2;
                m2 = mremove_int(m2, a);
                op_drop(old);
            } else {
                m2 = mremove_int(m2, a); // unique → the in-place collapse path
            }
            m = m2;
            m
        };
        let fbip = build(false);
        let copy = build(true);
        assert!(champ_eq(fbip, copy), "in-place collapse == copy-path collapse (canonical)");
        assert_eq!(champ_hash(fbip), champ_hash(copy), "byte-identical canonical shape");
        assert_eq!(mlookup_int(fbip, a), None, "removed key gone");
        for &(k, v) in &[(b, 2i64), (c, 3), (d, 4), (5, 50), (6, 60), (7, 70)] {
            assert_eq!(mlookup_int(fbip, k), Some(v), "survivor {k} intact after in-place collapse");
        }
        op_drop(fbip);
        op_drop(copy);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn map_canonical_shape_invariance() {
        reset();
        let before = live_nodes();
        let (a, b) = low5_split_pair(); // exercise a split so the shape is nontrivial
        let c = 424242i64;
        let d = 7777i64;
        // A: insert [a,b,c,d] then remove d.
        let mut ma = op_map_empty();
        for &(k, v) in &[(a, 1), (b, 2), (c, 3), (d, 4)] {
            ma = minsert_int(ma, k, v);
        }
        ma = mremove_int(ma, d);
        // B: insert [a,b,c].
        let mut mb = op_map_empty();
        for &(k, v) in &[(a, 1), (b, 2), (c, 3)] {
            mb = minsert_int(mb, k, v);
        }
        assert_eq!(op_map_size(ma), op_map_size(mb));
        assert!(champ_eq(ma, mb), "insert-then-remove == direct insert (canonical)");
        assert_eq!(champ_hash(ma), champ_hash(mb));
        // Insert-order independence: [a,b,c] vs [c,b,a].
        let mut mc = op_map_empty();
        for &(k, v) in &[(c, 3), (b, 2), (a, 1)] {
            mc = minsert_int(mc, k, v);
        }
        assert!(champ_eq(mb, mc), "insert order does not affect shape");
        assert_eq!(champ_hash(mb), champ_hash(mc));
        op_drop(ma);
        op_drop(mb);
        op_drop(mc);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_remove_persistence() {
        reset();
        let before = live_nodes();
        let orig = minsert_int(minsert_int(minsert_int(op_map_empty(), 10, 1), 20, 2), 30, 3);
        op_dup(orig);
        let derived = mremove_int(orig, 20);
        // Original unchanged.
        assert_eq!(op_map_size(orig), 3);
        assert_eq!(mlookup_int(orig, 20), Some(2));
        // Derived has the key removed.
        assert_eq!(op_map_size(derived), 2);
        assert_eq!(mlookup_int(derived, 20), None);
        assert_eq!(mlookup_int(derived, 10), Some(1));
        assert_eq!(mlookup_int(derived, 30), Some(3));
        op_drop(orig);
        op_drop(derived);
        assert_eq!(live_nodes(), before, "shared subtrees freed exactly once");
    }

    // ── CHAMP cursor + in-order map iteration ─────────────────────────────────────────────

    /// Walk `m` (borrows) collecting (key,val) as i64 pairs in visitation order. Consumes the
    /// cursors it builds (iter + iter_next chain), leaving `m`'s rc untouched.
    fn collect_map(m: Handle) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        let mut cur = op_map_iter(m);
        loop {
            let k = op_map_iter_key(cur);
            if k == Handle::NULL {
                break;
            }
            let v = op_map_iter_val(cur);
            out.push((op_get_int(k), op_get_int(v)));
            cur = op_map_iter_next(cur);
        }
        op_drop(cur);
        out
    }

    #[test]
    fn map_iter_empty_is_exhausted() {
        reset();
        let before = live_nodes();
        let m = op_map_empty();
        let cur = op_map_iter(m);
        assert_eq!(op_map_iter_key(cur), Handle::NULL, "empty map cursor is exhausted");
        assert_eq!(op_map_iter_val(cur), Handle::NULL);
        op_drop(cur);
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_iter_single_entry() {
        reset();
        let before = live_nodes();
        let m = minsert_int(op_map_empty(), 7, 700);
        let cur = op_map_iter(m);
        assert_eq!(op_get_int(op_map_iter_key(cur)), 7);
        assert_eq!(op_get_int(op_map_iter_val(cur)), 700);
        let cur = op_map_iter_next(cur);
        assert_eq!(op_map_iter_key(cur), Handle::NULL, "past the only entry ⇒ exhausted");
        op_drop(cur);
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_iter_full_traversal_visits_each_once() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair(); // force a subnode split into the traversal
        let mut pairs: Vec<(i64, i64)> = vec![(1, 10), (2, 20), (3, 30), (17, 170), (99, 990), (1000, 10000)];
        pairs.push((sa, 111));
        pairs.push((sb, 222));
        let mut m = op_map_empty();
        for &(k, v) in &pairs {
            m = minsert_int(m, k, v);
        }
        let visited = collect_map(m);
        assert_eq!(visited.len(), op_map_size(m) as usize, "visited exactly size entries");
        assert_eq!(visited.len(), pairs.len());
        // Every inserted key seen exactly once, mapped to its value.
        let mut got: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        for (k, v) in &visited {
            assert!(got.insert(*k, *v).is_none(), "key {k} visited twice");
        }
        for (k, v) in &pairs {
            assert_eq!(got.get(k), Some(v), "key {k} maps to {v}");
        }
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_iter_order_is_deterministic() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair();
        let keys = [(1i64, 10i64), (5, 50), (sa, 100), (sb, 200), (42, 420), (7, 70)];
        // Build the same logical map two different insert orders.
        let mut m1 = op_map_empty();
        for &(k, v) in keys.iter() {
            m1 = minsert_int(m1, k, v);
        }
        let mut m2 = op_map_empty();
        for &(k, v) in keys.iter().rev() {
            m2 = minsert_int(m2, k, v);
        }
        let order1: Vec<i64> = collect_map(m1).into_iter().map(|(k, _)| k).collect();
        let order2: Vec<i64> = collect_map(m2).into_iter().map(|(k, _)| k).collect();
        assert_eq!(order1, order2, "canonical order is insert-order-independent");
        op_drop(m1);
        op_drop(m2);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_iter_fork_independence() {
        reset();
        let before = live_nodes();
        let m = minsert_int(minsert_int(minsert_int(op_map_empty(), 1, 10), 2, 20), 3, 30);
        // A shared cursor with rc>1: advancing the RESULT of next must not disturb the other ref.
        let cur = op_map_iter(m);
        let first_key = op_get_int(op_map_iter_key(cur));
        op_dup(cur); // now rc==2: `cur` referenced twice
        let advanced = op_map_iter_next(cur); // consumes one ref, returns a fresh cursor
        // The still-held original reference (cur) must project its ORIGINAL key unchanged.
        assert_eq!(op_get_int(op_map_iter_key(cur)), first_key, "fork undisturbed by advance");
        // The advanced cursor is at a different (successor) key.
        let adv_key = op_map_iter_key(advanced);
        assert_ne!(op_get_int(adv_key), first_key, "advanced cursor moved on");
        op_drop(cur);
        op_drop(advanced);
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_iter_visits_collision_entries() {
        reset();
        let before = live_nodes();
        let (a, b) = full_hash_collision_pair();
        let m = minsert_int(minsert_int(op_map_empty(), a, 1000), b, 2000);
        let visited = collect_map(m);
        assert_eq!(visited.len(), 2, "both colliding entries visited");
        let keys: std::collections::HashSet<i64> = visited.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&a) && keys.contains(&b), "both colliding keys seen");
        op_drop(m);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn map_iter_full_walk_no_leak() {
        reset();
        let before = live_nodes();
        let mut m = op_map_empty();
        for k in 0..40i64 {
            m = minsert_int(m, k, k * 3);
        }
        let visited = collect_map(m);
        assert_eq!(visited.len(), 40);
        // Walk again to be sure iter borrows (does not consume) the map.
        assert_eq!(collect_map(m).len(), 40);
        op_drop(m);
        assert_eq!(live_nodes(), before, "no leak across full walks");
    }

    // ── CHAMP persistent SET (stride 1) ───────────────────────────────────────────────────

    /// Insert boxed int `e` into `s`, consuming `s`.
    fn sinsert_int(s: Handle, e: i64) -> Handle {
        op_set_insert(s, op_box_int(e))
    }
    /// Membership of boxed int `e` in `s` (borrows). Builds+drops a fresh probe.
    fn scontains_int(s: Handle, e: i64) -> bool {
        let probe = op_box_int(e);
        let r = op_set_contains(s, probe);
        op_drop(probe);
        r
    }
    /// Remove boxed int `e` from `s`, consuming `s`.
    fn sremove_int(s: Handle, e: i64) -> Handle {
        let probe = op_box_int(e);
        let out = op_set_remove(s, probe);
        op_drop(probe);
        out
    }
    /// Walk `s` (borrows) collecting elements as i64 in visitation order.
    fn collect_set(s: Handle) -> Vec<i64> {
        let mut out = Vec::new();
        let mut cur = op_set_iter(s);
        loop {
            let e = op_set_iter_elem(cur);
            if e == Handle::NULL {
                break;
            }
            out.push(op_get_int(e));
            cur = op_set_iter_next(cur);
        }
        op_drop(cur);
        out
    }

    #[test]
    fn set_empty_size_and_contains() {
        reset();
        let before = live_nodes();
        let s = op_set_empty();
        assert!(is_empty_node(s));
        assert_eq!(op_set_size(s), 0);
        assert!(!scontains_int(s, 42), "empty set contains nothing, no trap");
        op_drop(s);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_insert_then_contains() {
        reset();
        let before = live_nodes();
        let s = sinsert_int(op_set_empty(), 7);
        assert_eq!(op_set_size(s), 1);
        assert!(scontains_int(s, 7));
        assert!(!scontains_int(s, 8));
        op_drop(s);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_duplicate_insert_is_idempotent() {
        reset();
        let before = live_nodes();
        let s = sinsert_int(sinsert_int(op_set_empty(), 5), 5);
        assert_eq!(op_set_size(s), 1, "duplicate insert keeps size 1");
        assert!(scontains_int(s, 5));
        op_drop(s);
        assert_eq!(live_nodes(), before, "duplicate element reclaimed");
    }

    #[test]
    fn set_many_distinct_elems() {
        reset();
        let before = live_nodes();
        let elems = [1i64, 2, 3, 17, 99, 1000];
        let mut s = op_set_empty();
        for &e in &elems {
            s = sinsert_int(s, e);
        }
        assert_eq!(op_set_size(s), elems.len() as u32);
        for &e in &elems {
            assert!(scontains_int(s, e), "elem {e}");
        }
        assert!(!scontains_int(s, 12345));
        op_drop(s);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_subnode_split() {
        reset();
        let before = live_nodes();
        let (a, b) = low5_split_pair();
        let s = sinsert_int(sinsert_int(op_set_empty(), a), b);
        assert_eq!(op_set_size(s), 2);
        let (dm, nm) = with_node(s, (0u32, 0u32), |n| (champ_datamap(&n.raw), champ_nodemap(&n.raw)));
        assert_eq!((data_count(dm), subnode_count(nm)), (0, 1), "split created a subnode");
        assert!(scontains_int(s, a) && scontains_int(s, b));
        op_drop(s);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_collision_node() {
        reset();
        let before = live_nodes();
        let (a, b) = full_hash_collision_pair();
        let s = sinsert_int(sinsert_int(op_set_empty(), a), b);
        assert_eq!(op_set_size(s), 2, "both colliding elems counted");
        assert!(scontains_int(s, a) && scontains_int(s, b));
        op_drop(s);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_remove_present_absent_and_to_empty() {
        reset();
        let before = live_nodes();
        let elems = [1i64, 2, 3, 17, 99, 1000];
        let mut s = op_set_empty();
        for &e in &elems {
            s = sinsert_int(s, e);
        }
        // Remove present.
        s = sremove_int(s, 3);
        assert_eq!(op_set_size(s), 5);
        assert!(!scontains_int(s, 3));
        assert!(scontains_int(s, 1) && scontains_int(s, 1000));
        // Remove absent = no-op.
        s = sremove_int(s, 424242);
        assert_eq!(op_set_size(s), 5);
        // Remove the rest down to empty.
        for &e in &elems {
            s = sremove_int(s, e);
        }
        let empty = op_set_empty();
        assert!(is_empty_node(s));
        assert_eq!(op_set_size(s), 0);
        assert!(champ_eq(s, empty), "remove-to-empty is byte-identical to op_set_empty()");
        assert_eq!(champ_hash(s), champ_hash(empty));
        op_drop(empty);
        op_drop(s);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_remove_collapses_subnode_and_collision() {
        reset();
        let before = live_nodes();
        // Subnode collapse.
        let (a, b) = low5_split_pair();
        let s = sinsert_int(sinsert_int(op_set_empty(), a), b);
        let s = sremove_int(s, a);
        let (dm, nm) = with_node(s, (0u32, 0u32), |n| (champ_datamap(&n.raw), champ_nodemap(&n.raw)));
        assert_eq!(data_count(dm), 1, "root collapsed to one inline elem");
        assert_eq!(nm, 0, "no subnodes after collapse");
        assert!(scontains_int(s, b) && !scontains_int(s, a));
        op_drop(s);
        // Collision collapse.
        let (c, d) = full_hash_collision_pair();
        let sc = sinsert_int(sinsert_int(op_set_empty(), c), d);
        let sc = sremove_int(sc, c);
        assert_eq!(op_set_size(sc), 1);
        let solo = sinsert_int(op_set_empty(), d);
        assert!(champ_eq(sc, solo), "collision collapse is canonical");
        assert_eq!(champ_hash(sc), champ_hash(solo));
        op_drop(solo);
        op_drop(sc);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_canonical_shape_invariance() {
        reset();
        let before = live_nodes();
        let (a, b) = low5_split_pair();
        let c = 424242i64;
        let d = 7777i64;
        // A: insert [a,b,c,d] then remove d.
        let mut sa = op_set_empty();
        for &e in &[a, b, c, d] {
            sa = sinsert_int(sa, e);
        }
        sa = sremove_int(sa, d);
        // B: insert [a,b,c].
        let mut sb = op_set_empty();
        for &e in &[a, b, c] {
            sb = sinsert_int(sb, e);
        }
        assert_eq!(op_set_size(sa), op_set_size(sb));
        assert!(champ_eq(sa, sb), "insert-then-remove == direct insert");
        assert_eq!(champ_hash(sa), champ_hash(sb));
        // Insert-order independence.
        let mut sc = op_set_empty();
        for &e in &[c, b, a] {
            sc = sinsert_int(sc, e);
        }
        assert!(champ_eq(sb, sc), "insert order does not affect shape");
        assert_eq!(champ_hash(sb), champ_hash(sc));
        op_drop(sa);
        op_drop(sb);
        op_drop(sc);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_iter_full_traversal_and_determinism() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair();
        let mut elems: Vec<i64> = vec![1, 2, 3, 17, 99, 1000];
        elems.push(sa);
        elems.push(sb);
        // Build the same set two insert orders.
        let mut s1 = op_set_empty();
        for &e in &elems {
            s1 = sinsert_int(s1, e);
        }
        let mut s2 = op_set_empty();
        for &e in elems.iter().rev() {
            s2 = sinsert_int(s2, e);
        }
        let v1 = collect_set(s1);
        let v2 = collect_set(s2);
        assert_eq!(v1.len(), op_set_size(s1) as usize, "visited exactly size elements");
        // Every element seen exactly once.
        let seen: std::collections::HashSet<i64> = v1.iter().copied().collect();
        assert_eq!(seen.len(), elems.len());
        for &e in &elems {
            assert!(seen.contains(&e), "elem {e} visited");
        }
        assert_eq!(v1, v2, "canonical order is insert-order-independent");
        op_drop(s1);
        op_drop(s2);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_iter_fork_independence_and_collision() {
        reset();
        let before = live_nodes();
        // Fork independence.
        let s = sinsert_int(sinsert_int(sinsert_int(op_set_empty(), 1), 2), 3);
        let cur = op_set_iter(s);
        let first = op_get_int(op_set_iter_elem(cur));
        op_dup(cur);
        let advanced = op_set_iter_next(cur);
        assert_eq!(op_get_int(op_set_iter_elem(cur)), first, "fork undisturbed by advance");
        assert_ne!(op_get_int(op_set_iter_elem(advanced)), first, "advanced moved on");
        op_drop(cur);
        op_drop(advanced);
        op_drop(s);
        // Collision-pair both visited.
        let (a, b) = full_hash_collision_pair();
        let sc = sinsert_int(sinsert_int(op_set_empty(), a), b);
        let visited: std::collections::HashSet<i64> = collect_set(sc).into_iter().collect();
        assert!(visited.contains(&a) && visited.contains(&b), "both colliding elems visited");
        op_drop(sc);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_persistence() {
        reset();
        let before = live_nodes();
        let orig = sinsert_int(sinsert_int(op_set_empty(), 10), 20);
        op_dup(orig);
        let derived = sinsert_int(orig, 30);
        // Original unchanged.
        assert_eq!(op_set_size(orig), 2);
        assert!(scontains_int(orig, 10) && scontains_int(orig, 20));
        assert!(!scontains_int(orig, 30));
        // Derived extends it.
        assert_eq!(op_set_size(derived), 3);
        assert!(scontains_int(derived, 30));
        // Remove-persistence too.
        op_dup(derived);
        let removed = sremove_int(derived, 20);
        assert_eq!(op_set_size(derived), 3);
        assert!(scontains_int(derived, 20));
        assert!(!scontains_int(removed, 20));
        assert_eq!(op_set_size(removed), 2);
        op_drop(orig);
        op_drop(derived);
        op_drop(removed);
        assert_eq!(live_nodes(), before, "shared subtrees freed exactly once");
    }

    // ── Collision-node canonicality across insert order (regression) ──────────────────────

    #[test]
    fn map_collision_node_is_canonical_across_insert_order() {
        reset();
        let before = live_nodes();
        let (a, b) = full_hash_collision_pair(); // share full 32-bit champ_hash
        // Same contents, two insert orders — the collision node must be byte-identical.
        let m1 = minsert_int(minsert_int(op_map_empty(), a, 100), b, 200);
        let m2 = minsert_int(minsert_int(op_map_empty(), b, 200), a, 100);
        assert_eq!(op_map_size(m1), 2);
        assert_eq!(op_map_size(m2), 2);
        assert!(champ_eq(m1, m2), "collision node canonical regardless of insert order");
        assert_eq!(champ_hash(m1), champ_hash(m2), "equal collision maps hash equal");
        assert_eq!(collect_map(m1), collect_map(m2), "iteration order identical");
        // Both keys still lookup to correct values.
        assert_eq!(mlookup_int(m1, a), Some(100));
        assert_eq!(mlookup_int(m1, b), Some(200));
        assert_eq!(mlookup_int(m2, a), Some(100));
        assert_eq!(mlookup_int(m2, b), Some(200));
        op_drop(m1);
        op_drop(m2);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn set_collision_node_is_canonical_across_insert_order() {
        reset();
        let before = live_nodes();
        let (a, b) = full_hash_collision_pair();
        let s1 = sinsert_int(sinsert_int(op_set_empty(), a), b);
        let s2 = sinsert_int(sinsert_int(op_set_empty(), b), a);
        assert_eq!(op_set_size(s1), 2);
        assert_eq!(op_set_size(s2), 2);
        assert!(champ_eq(s1, s2), "collision set canonical regardless of insert order");
        assert_eq!(champ_hash(s1), champ_hash(s2), "equal collision sets hash equal");
        assert_eq!(collect_set(s1), collect_set(s2), "iteration order identical");
        assert!(scontains_int(s1, a) && scontains_int(s1, b));
        assert!(scontains_int(s2, a) && scontains_int(s2, b));
        op_drop(s1);
        op_drop(s2);
        assert_eq!(live_nodes(), before);
    }

    #[test]
    fn champ_key_cmp_is_consistent_with_eq() {
        reset();
        let before = live_nodes();
        // Equal IFF champ_eq true; and it's a genuine (antisymmetric) order otherwise.
        let x = op_box_int(10);
        let y = op_box_int(10); // structurally equal, distinct alloc
        let z = op_box_int(11);
        assert_eq!(champ_key_cmp(x, y), core::cmp::Ordering::Equal);
        assert!(champ_eq(x, y));
        assert_ne!(champ_key_cmp(x, z), core::cmp::Ordering::Equal);
        assert!(!champ_eq(x, z));
        // Antisymmetry: cmp(x,z) is the reverse of cmp(z,x).
        assert_eq!(champ_key_cmp(x, z).reverse(), champ_key_cmp(z, x));
        // Null orders before any non-null; two nulls equal.
        assert_eq!(champ_key_cmp(Handle::NULL, x), core::cmp::Ordering::Less);
        assert_eq!(champ_key_cmp(x, Handle::NULL), core::cmp::Ordering::Greater);
        assert_eq!(champ_key_cmp(Handle::NULL, Handle::NULL), core::cmp::Ordering::Equal);
        op_drop(x);
        op_drop(y);
        op_drop(z);
        assert_eq!(live_nodes(), before);
    }

    // ── U5: FBIP rc==1 in-place shell reuse for CHAMP map/set insert+remove ─────────────────────
    // The load-bearing property is ALIASING SAFETY: an insert/remove on a SHARED map/set (rc>1) must
    // path-copy and leave the other version byte-identical (champ_eq + champ_hash); the FBIP win fires
    // only when the touched spine is uniquely owned. Canonical shape (collision order, collapse,
    // remove-to-canonical-empty) must survive the in-place path.

    /// Build a multi-level map that includes a subnode SPLIT and a COLLISION pair — the richest shape,
    /// used to exercise every FBIP branch. Returns `(m, split_a, split_b, coll_a, coll_b)`.
    fn rich_map() -> (Handle, i64, i64, i64, i64) {
        let (sa, sb) = low5_split_pair(); // forces a subnode at the root
        let (ca, cb) = full_hash_collision_pair(); // forces a collision node at the hash floor
        let mut m = op_map_empty();
        for &(k, v) in &[(sa, 1), (sb, 2), (ca, 3), (cb, 4), (7i64, 70), (9, 90)] {
            m = minsert_int(m, k, v);
        }
        (m, sa, sb, ca, cb)
    }

    #[test]
    fn map_insert_fbip_shared_version_unaffected() {
        reset();
        let before = live_nodes();
        let (m1, sa, _sb, ca, _cb) = rich_map();
        let orig_size = op_map_size(m1);
        // Snapshot m1's identity for the aliasing check.
        op_dup(m1); // snapshot owner
        let snap = m1;
        op_dup(m1); // rc == 3 now: m1 is a SHARED version
        let m2 = minsert_int(m1, 12345, 999); // insert a NEW key on the shared owner
        // m1 (shared) is byte-identical to the pre-insert snapshot.
        assert!(champ_eq(m1, snap), "shared map unchanged after other owner's insert");
        assert_eq!(champ_hash(m1), champ_hash(snap), "shared map hash unchanged");
        assert_eq!(op_map_size(m1), orig_size, "shared map size unchanged");
        assert_eq!(mlookup_int(m1, sa), Some(1), "shared map key still resolves");
        assert_eq!(mlookup_int(m1, ca), Some(3), "shared map collision key still resolves");
        assert_eq!(mlookup_int(m1, 12345), None, "shared map never saw the new key");
        // m2 has the change.
        assert_eq!(op_map_size(m2), orig_size + 1);
        assert_eq!(mlookup_int(m2, 12345), Some(999));
        assert_eq!(mlookup_int(m2, ca), Some(3), "m2 preserves the shared collision entry");
        op_drop(snap);
        op_drop(m1);
        op_drop(m2);
        assert_eq!(live_nodes(), before, "no leak / no double-free");
    }

    #[test]
    fn map_remove_fbip_shared_version_unaffected() {
        reset();
        let before = live_nodes();
        let (m1, sa, sb, ca, _cb) = rich_map();
        let orig_size = op_map_size(m1);
        op_dup(m1);
        let snap = m1;
        op_dup(m1); // shared version
        let m2 = mremove_int(m1, sa); // remove a key that lives under the split subnode
        assert!(champ_eq(m1, snap), "shared map unchanged after other owner's remove");
        assert_eq!(champ_hash(m1), champ_hash(snap), "shared map hash unchanged");
        assert_eq!(op_map_size(m1), orig_size, "shared map size unchanged");
        assert_eq!(mlookup_int(m1, sa), Some(1), "shared map still has the removed key");
        // m2 has the removal.
        assert_eq!(op_map_size(m2), orig_size - 1);
        assert_eq!(mlookup_int(m2, sa), None, "m2 removed the key");
        assert_eq!(mlookup_int(m2, sb), Some(2), "m2 kept the split sibling");
        assert_eq!(mlookup_int(m2, ca), Some(3), "m2 kept the collision entry");
        op_drop(snap);
        op_drop(m1);
        op_drop(m2);
        assert_eq!(live_nodes(), before, "no leak / no double-free");
    }

    #[test]
    fn set_insert_fbip_shared_version_unaffected() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair();
        let (ca, cb) = full_hash_collision_pair();
        let mut s1 = op_set_empty();
        for &e in &[sa, sb, ca, cb, 4i64, 8] {
            s1 = sinsert_int(s1, e);
        }
        let orig_size = op_set_size(s1);
        op_dup(s1);
        let snap = s1;
        op_dup(s1); // shared
        let s2 = sinsert_int(s1, 54321);
        assert!(champ_eq(s1, snap), "shared set unchanged after other owner's insert");
        assert_eq!(champ_hash(s1), champ_hash(snap));
        assert_eq!(op_set_size(s1), orig_size);
        assert!(!scontains_int(s1, 54321), "shared set never saw the new elem");
        assert!(scontains_int(s2, 54321));
        assert!(scontains_int(s2, ca), "s2 preserves the collision elem");
        assert_eq!(op_set_size(s2), orig_size + 1);
        op_drop(snap);
        op_drop(s1);
        op_drop(s2);
        assert_eq!(live_nodes(), before, "no leak / no double-free");
    }

    #[test]
    fn set_remove_fbip_shared_version_unaffected() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair();
        let (ca, cb) = full_hash_collision_pair();
        let mut s1 = op_set_empty();
        for &e in &[sa, sb, ca, cb, 4i64, 8] {
            s1 = sinsert_int(s1, e);
        }
        let orig_size = op_set_size(s1);
        op_dup(s1);
        let snap = s1;
        op_dup(s1); // shared
        let s2 = sremove_int(s1, ca); // remove one of a collision pair
        assert!(champ_eq(s1, snap), "shared set unchanged after other owner's remove");
        assert_eq!(champ_hash(s1), champ_hash(snap));
        assert_eq!(op_set_size(s1), orig_size);
        assert!(scontains_int(s1, ca), "shared set still has the removed elem");
        assert!(!scontains_int(s2, ca), "s2 removed the elem");
        assert!(scontains_int(s2, cb), "s2 kept the collision sibling");
        assert_eq!(op_set_size(s2), orig_size - 1);
        op_drop(snap);
        op_drop(s1);
        op_drop(s2);
        assert_eq!(live_nodes(), before, "no leak / no double-free");
    }

    #[test]
    fn champ_fbip_unique_reuses_in_place() {
        reset();
        // A UNIQUE map's insert of a NEW key into a subnode allocates strictly fewer nodes than the
        // SHARED case, because the touched spine is refit in place instead of path-copied.
        let unique_alloc = {
            let (m, _sa, _sb, _ca, _cb) = rich_map();
            let before = live_nodes();
            let m2 = minsert_int(m, 4242, 1); // new key; some existing slot occupied → descend
            let d = live_nodes() - before;
            op_drop(m2);
            d
        };
        let shared_alloc = {
            let (m, _sa, _sb, _ca, _cb) = rich_map();
            op_dup(m); // shared → path-copy the touched spine
            let before = live_nodes();
            let m2 = minsert_int(m, 4242, 1);
            let d = live_nodes() - before;
            op_drop(m);
            op_drop(m2);
            d
        };
        assert!(
            unique_alloc < shared_alloc,
            "FBIP map insert must allocate fewer when unique ({unique_alloc}) than shared ({shared_alloc})"
        );

        // Same for a set REMOVE (a collapse case exercises the deepest in-place rebuild).
        let (sa, sb) = low5_split_pair();
        let unique_rm = {
            let mut s = op_set_empty();
            for &e in &[sa, sb, 3i64, 5] {
                s = sinsert_int(s, e);
            }
            let before = live_nodes();
            let s2 = sremove_int(s, sa); // removes under the split; may collapse
            let d = live_nodes() - before;
            op_drop(s2);
            d
        };
        let shared_rm = {
            let mut s = op_set_empty();
            for &e in &[sa, sb, 3i64, 5] {
                s = sinsert_int(s, e);
            }
            op_dup(s);
            let before = live_nodes();
            let s2 = sremove_int(s, sa);
            let d = live_nodes() - before;
            op_drop(s);
            op_drop(s2);
            d
        };
        assert!(
            unique_rm <= shared_rm,
            "FBIP set remove must not allocate more when unique ({unique_rm}) than shared ({shared_rm})"
        );
        assert!(unique_rm < shared_rm, "and strictly fewer in the collapse case");
    }

    #[test]
    fn champ_fbip_canonical_shape_preserved() {
        reset();
        let before = live_nodes();
        // (1) COLLISION case: a unique map built by FBIP inserts must be byte-identical (champ_eq +
        // champ_hash) to the same map built fresh by the copy path (via a SHARED insert chain).
        let (ca, cb) = full_hash_collision_pair();
        let build = |shared: bool| -> Handle {
            let mut m = op_map_empty();
            for &(k, v) in &[(ca, 1i64), (cb, 2), (5i64, 50), (6, 60)] {
                if shared {
                    // Force the copy path at every step: dup then drop the old owner.
                    op_dup(m);
                    let old = m;
                    m = minsert_int(m, k, v);
                    op_drop(old);
                } else {
                    m = minsert_int(m, k, v); // unique → FBIP in place
                }
            }
            m
        };
        let fbip = build(false);
        let copy = build(true);
        assert!(champ_eq(fbip, copy), "FBIP-built collision map == copy-built");
        assert_eq!(champ_hash(fbip), champ_hash(copy), "byte-identical canonical shape");
        op_drop(fbip);
        op_drop(copy);

        // (2) COLLAPSE case: remove down through a subnode so a child collapses back to inline; the
        // FBIP result must match the copy-path result byte-for-byte, and match a map built WITHOUT the
        // collapsed key at all (the canonical shape a fresh insert set would produce).
        let (sa, sb) = low5_split_pair();
        let make_full = |shared: bool| -> Handle {
            let mut m = op_map_empty();
            for &(k, v) in &[(sa, 1i64), (sb, 2), (3i64, 30)] {
                m = minsert_int(m, k, v);
            }
            // remove sb: the split subnode {sa,sb} reduces to {sa} and must collapse back inline.
            if shared {
                op_dup(m);
                let old = m;
                m = mremove_int(m, sb);
                op_drop(old);
            } else {
                m = mremove_int(m, sb);
            }
            m
        };
        let collapsed_fbip = make_full(false);
        let collapsed_copy = make_full(true);
        let fresh = {
            let mut m = op_map_empty();
            for &(k, v) in &[(sa, 1i64), (3i64, 30)] {
                m = minsert_int(m, k, v);
            }
            m
        };
        assert!(champ_eq(collapsed_fbip, collapsed_copy), "FBIP collapse == copy collapse");
        assert!(champ_eq(collapsed_fbip, fresh), "collapse yields the canonical fresh shape");
        assert_eq!(champ_hash(collapsed_fbip), champ_hash(fresh));
        op_drop(collapsed_fbip);
        op_drop(collapsed_copy);
        op_drop(fresh);

        // (3) remove-to-canonical-empty via FBIP.
        let mut m = minsert_int(op_map_empty(), 42, 1);
        m = mremove_int(m, 42);
        assert!(is_empty_node(m), "FBIP remove of the last entry yields the canonical empty");
        let fresh_empty = op_map_empty();
        assert!(champ_eq(m, fresh_empty), "byte-identical to op_map_empty()");
        assert_eq!(champ_hash(m), champ_hash(fresh_empty));
        op_drop(fresh_empty);
        op_drop(m);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn champ_insert_fbip_empty_slot_splice_past_subnode_is_canonical() {
        reset();
        let before = live_nodes();
        // Guards the EMPTY-slot in-place splice (Vec::insert the entry columns into the taken `handles`
        // instead of rebuilding a fresh Vec). The load-bearing invariant is that the entry region sits
        // BEFORE the subnodes, so splicing at `stride*new_eidx` must SHIFT the subnodes right and land
        // the entry in canonical order. Build a root node that has a subnode (from a low-5 split), then
        // insert a fresh key whose slot is an empty datamap bit — exercising the splice on a node that
        // already holds a subnode — and assert byte-identical (champ_eq + champ_hash) to the copy-path
        // build, plus every key present. Do it for keys landing both before AND after the subnode's slot.
        let (a, b) = low5_split_pair(); // share low-5 ⇒ a level-0 subnode
        // Pick fresh keys that occupy DISTINCT level-0 slots (so they land in empty datamap bits, not
        // the subnode's slot and not each other's). Just search a few small ints for distinct low-5.
        let slot_of = |x: i64| -> u32 {
            let k = op_box_int(x);
            let s = champ_hash(k) & 0x1f;
            op_drop(k);
            s
        };
        let subnode_slot = slot_of(a); // a and b share low-5, so this is the subnode's level-0 slot
        let mut extras: Vec<(i64, u32)> = Vec::new();
        let mut v = 0i64;
        while extras.len() < 4 && v < 100_000 {
            let slot = slot_of(v);
            if v != a && v != b && slot != subnode_slot && !extras.iter().any(|&(_, s)| s == slot) {
                extras.push((v, slot));
            }
            v += 1;
        }
        let extra_keys: Vec<i64> = extras.iter().map(|&(k, _)| k).collect();

        let build = |shared: bool| -> Handle {
            let mut m = op_map_empty();
            // First the split pair (creates the subnode), then the extras (each an empty-slot splice on
            // a node that already contains the subnode).
            let mut all: Vec<(i64, i64)> = vec![(a, 1), (b, 2)];
            for (i, &k) in extra_keys.iter().enumerate() {
                all.push((k, 100 + i as i64));
            }
            for &(k, val) in &all {
                if shared {
                    op_dup(m);
                    let old = m;
                    m = minsert_int(m, k, val);
                    op_drop(old);
                } else {
                    m = minsert_int(m, k, val);
                }
            }
            m
        };
        let fbip = build(false);
        let copy = build(true);
        assert!(champ_eq(fbip, copy), "empty-slot splice past a subnode == copy-path build (canonical)");
        assert_eq!(champ_hash(fbip), champ_hash(copy), "byte-identical canonical shape");
        assert_eq!(mlookup_int(fbip, a), Some(1));
        assert_eq!(mlookup_int(fbip, b), Some(2));
        for (i, &k) in extra_keys.iter().enumerate() {
            assert_eq!(mlookup_int(fbip, k), Some(100 + i as i64), "spliced key {k} present");
        }
        op_drop(fbip);
        op_drop(copy);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn champ_insert_fbip_split_in_place_places_subnode_canonically() {
        reset();
        let before = live_nodes();
        // Guards the in-place SPLIT (drain the split entry's columns from the taken vec, then insert the
        // new subnode at `stride*(dcount-1) + new_sidx`). The load-bearing invariant is that after the
        // entry region shrinks by one, the new subnode lands at its CANONICAL subnode slot among any
        // pre-existing subnodes. Build a root that already holds MULTIPLE inline entries AND ≥1 subnode,
        // then insert a key that COLLIDES (at level 0) with one of the inline entries — forcing that
        // entry to split into a new subnode while other entries + the existing subnode stay put — and
        // assert byte-identical (champ_eq + champ_hash) to the copy-path build, all keys correct, no leak.
        let (sa, sb) = low5_split_pair();     // sa,sb share low-5 ⇒ an existing subnode at the root
        let (ca, cb) = full_hash_collision_pair(); // a distinct pair that also splits (to a collision node)
        let build = |shared: bool| -> Handle {
            let mut m = op_map_empty();
            // First sa,sb (creates subnode #1) + ordinary inline entries + ca alone (inline).
            let mut seq: Vec<(i64, i64)> = vec![(sa, 1), (sb, 2), (ca, 3), (1i64, 10), (2, 20), (3, 30)];
            // Then insert cb: it collides with ca (full-hash), so ca's inline entry SPLITS into a new
            // subnode #2 that must slot canonically alongside the existing subnode #1.
            seq.push((cb, 4));
            for &(k, v) in &seq {
                if shared {
                    op_dup(m);
                    let old = m;
                    m = minsert_int(m, k, v);
                    op_drop(old);
                } else {
                    m = minsert_int(m, k, v);
                }
            }
            m
        };
        let fbip = build(false);
        let copy = build(true);
        assert!(champ_eq(fbip, copy), "in-place SPLIT == copy-path build (canonical subnode placement)");
        assert_eq!(champ_hash(fbip), champ_hash(copy), "byte-identical canonical shape");
        for &(k, v) in &[(sa, 1i64), (sb, 2), (ca, 3), (cb, 4), (1, 10), (2, 20), (3, 30)] {
            assert_eq!(mlookup_int(fbip, k), Some(v), "key {k} present after the in-place split");
        }
        op_drop(fbip);
        op_drop(copy);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn champ_insert_fbip_deep_unique_spine_take_is_sound() {
        reset();
        let before = live_nodes();
        // Guards the `mem::take(&mut n.handles)` that replaced the per-level `handles.clone()` in the
        // UNIQUE insert spine of `champ_insert_fbip`. Two properties the take must not break:
        //   (1) A deep multi-level unique spine built by FBIP inserts is byte-identical (champ_eq +
        //       champ_hash) to the SAME map built via the copy path — the take's transient empty state
        //       must never leak into the produced value.
        //   (2) A version SHARED (rc>1) at the moment of a further unique insert stays byte-unchanged
        //       — the descent must copy-path exactly the shared node it reaches, and the take on the
        //       nodes ABOVE it (which are also shared once forked, so mine is false and no take runs)
        //       must not disturb the snapshot.
        // Keys that force several levels of subnode splits: share the low 5, 10, 15 hash bits.
        let deep_keys: [i64; 6] = [
            0,           // …00000_00000_00000
            1 << 5,      // differs only at level 1
            1 << 10,     // differs only at level 2
            (1 << 5) | (1 << 10),
            1,           // differs at level 0
            (1 << 10) | 1,
        ];
        let build = |shared: bool| -> Handle {
            let mut m = op_map_empty();
            for (i, &k) in deep_keys.iter().enumerate() {
                if shared {
                    op_dup(m); // force rc>1 → copy path at every step
                    let old = m;
                    m = minsert_int(m, k, i as i64);
                    op_drop(old);
                } else {
                    m = minsert_int(m, k, i as i64); // unique → the mem::take spine
                }
            }
            m
        };
        let fbip = build(false);
        let copy = build(true);
        assert!(champ_eq(fbip, copy), "deep unique FBIP spine == copy-path build");
        assert_eq!(champ_hash(fbip), champ_hash(copy), "byte-identical canonical shape");
        // Every key present with the right value in the FBIP-built map.
        for (i, &k) in deep_keys.iter().enumerate() {
            assert_eq!(mlookup_int(fbip, k), Some(i as i64), "key {k} present");
        }
        op_drop(fbip);
        op_drop(copy);

        // (2) Snapshot invariance across a further unique insert descending the shared spine.
        let mut m = op_map_empty();
        for (i, &k) in deep_keys.iter().enumerate() {
            m = minsert_int(m, k, i as i64);
        }
        op_dup(m); // snapshot: m now rc==2 (shared)
        let snap = m;
        let snap_hash = champ_hash(snap);
        // Insert a NEW key that descends the deepest shared subnode; the snapshot must be untouched.
        m = minsert_int(m, (1 << 5) | (1 << 10) | 1, 999);
        assert_eq!(champ_hash(snap), snap_hash, "shared snapshot unchanged after sibling insert");
        for (i, &k) in deep_keys.iter().enumerate() {
            assert_eq!(mlookup_int(snap, k), Some(i as i64), "snapshot key {k} intact");
        }
        assert_eq!(mlookup_int(m, (1 << 5) | (1 << 10) | 1), Some(999), "new key in the new version");
        op_drop(snap);
        op_drop(m);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn entry_columns_consumed_exactly_once_across_all_insert_paths() {
        reset();
        let before = live_nodes();
        // Guards the move-only inline `Entry` that replaced the per-insert `Vec<Handle>`: each entry's
        // key/value columns must be consumed EXACTLY ONCE across every insert path (fresh-single,
        // EMPTY-slot splice, OVERWRITE which drops the incoming key + swaps the value, SPLIT which folds
        // via merge_two_entries, DESCEND, and the collision-node splice) — no leak (double-count) and no
        // double-free (crash). Value maps (stride 2) exercise the two-column key+value handling; use
        // BOXED (out-of-window) values so each column is a real heap node whose rc the leak counter sees.
        let (sa, sb) = low5_split_pair(); // force a split
        let (ca, cb) = full_hash_collision_pair(); // force a collision node + merge at the hash floor
        let boxed = |v: i64| boxed_int_leaf((1i64 << 40) + v); // out-of-window ⇒ real node value
        // Build a map hitting fresh/empty-slot/split/descend/collision, then OVERWRITE several keys
        // (drops the old boxed value + the incoming duplicate key), then verify every value and no leak.
        let keys = [sa, sb, ca, cb, 1i64, 2, 3, 100, 101];
        let mut m = op_map_empty();
        for (i, &k) in keys.iter().enumerate() {
            m = op_map_insert(m, op_box_int(k), boxed(i as i64));
        }
        // Overwrite half the keys with new boxed values — the OVERWRITE path must drop the old value
        // node and the incoming duplicate key, keeping the stored key.
        for (i, &k) in keys.iter().enumerate().filter(|(i, _)| i % 2 == 0) {
            m = op_map_insert(m, op_box_int(k), boxed(1000 + i as i64));
        }
        assert_eq!(op_map_size(m) as usize, keys.len(), "overwrites did not change size");
        for (i, &k) in keys.iter().enumerate() {
            let want = if i % 2 == 0 { 1000 + i as i64 } else { i as i64 };
            let probe = op_box_int(k);
            let got = op_map_lookup(m, probe); // borrows the value; do not retain
            assert_eq!(op_get_int(got), (1i64 << 40) + want, "key {k} has the right (boxed) value");
            op_drop(probe);
        }
        op_drop(m);
        assert_eq!(live_nodes(), before, "every entry column freed exactly once — no leak, no double-free");
    }

    #[test]
    fn champ_fbip_still_matches_reference() {
        reset();
        let before = live_nodes();
        // Mixed unique/shared insert/remove sequence on a map vs a std reference. Deterministic LCG.
        let mut m = op_map_empty();
        let mut reference: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
        let mut lcg: u64 = 0xDEAD_BEEF;
        let next = |lcg: &mut u64| {
            *lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (*lcg >> 33) as u32
        };
        for step in 0..800u32 {
            let key = (next(&mut lcg) % 64) as i64; // small keyspace ⇒ real overwrites + removes
            let op = next(&mut lcg) % 3;
            if op < 2 {
                let val = step as i64;
                m = minsert_int(m, key, val);
                reference.insert(key, val);
            } else {
                m = mremove_int(m, key);
                reference.remove(&key);
            }
            // Occasionally fork (share) then keep mutating the new version — exercises rc>1 paths.
            if step % 11 == 0 {
                op_dup(m);
                let forked = m;
                m = minsert_int(m, 1000 + (step as i64 % 5), step as i64);
                reference.insert(1000 + (step as i64 % 5), step as i64);
                op_drop(forked);
            }
        }
        assert_eq!(op_map_size(m) as usize, reference.len(), "size matches reference");
        for (&k, &v) in &reference {
            assert_eq!(mlookup_int(m, k), Some(v), "key {k} matches reference");
        }
        // And no phantom keys: probe the whole small keyspace.
        for k in 0..64i64 {
            assert_eq!(mlookup_int(m, k), reference.get(&k).copied(), "keyspace probe {k}");
        }
        op_drop(m);
        assert_eq!(live_nodes(), before, "no leak across the mixed sequence");
    }

    // ── U6: FBIP rc==1 in-place cursor advance for map-iter-next / set-iter-next ────────────────
    // Load-bearing: (1) a forked/peeked/teed cursor (rc>1) stays INDEPENDENT — advancing one owner
    // must not disturb the other (aliasing catcher); (2) a unique (rc==1) walk allocates ZERO new
    // cursor nodes steady-state (the WIT promise); (3) order + exhausted-signal identical to the copy
    // path. The pre-existing collect_map/collect_set (which advance an rc==1 cursor in a loop) already
    // exercise the FBIP path across the whole suite — these pin the properties explicitly.

    /// A rich map with a subnode split + a collision pair, so the cursor's frame stack goes deep.
    fn deep_walk_map() -> Handle {
        let (sa, sb) = low5_split_pair();
        let (ca, cb) = full_hash_collision_pair();
        let mut m = op_map_empty();
        for &(k, v) in &[(sa, 1i64), (sb, 2), (ca, 3), (cb, 4), (5i64, 50), (11, 110), (23, 230)] {
            m = minsert_int(m, k, v);
        }
        m
    }

    #[test]
    fn map_iter_next_fbip_fork_independent() {
        reset();
        let before = live_nodes();
        let m = deep_walk_map();
        // Reference sequence via the copy path (fresh cursor each element is naturally rc==1, but we
        // capture the order to compare the forked walk against it).
        let full = collect_map(m);
        assert!(full.len() >= 5, "deep enough to have a multi-frame stack");

        // Fork at the FIRST position: dup so rc==2, advance ONE copy; the other must be undisturbed and
        // able to walk the FULL remaining sequence independently.
        let cur = op_map_iter(m);
        let first_key = op_get_int(op_map_iter_key(cur));
        assert_eq!(first_key, full[0].0, "cursor starts at the first entry");
        op_dup(cur); // rc == 2: forked
        let advanced = op_map_iter_next(cur); // rc>1 ⇒ copy path; must NOT mutate `cur`
        // The retained fork still projects its ORIGINAL current entry.
        assert_eq!(op_get_int(op_map_iter_key(cur)), first_key, "fork undisturbed by advance");
        assert_eq!(op_get_int(op_map_iter_val(cur)), full[0].1);
        // The advanced copy is at the SECOND entry.
        assert_eq!(op_get_int(op_map_iter_key(advanced)), full[1].0, "advanced copy moved to successor");
        // Now walk the fork independently through the ENTIRE sequence — it must reproduce `full`.
        let mut seq: Vec<(i64, i64)> = Vec::new();
        let mut c = cur; // `cur` is rc==1 again (advanced consumed one ref) ⇒ walks via FBIP in place
        loop {
            let k = op_map_iter_key(c);
            if k == Handle::NULL {
                break;
            }
            seq.push((op_get_int(k), op_get_int(op_map_iter_val(c))));
            c = op_map_iter_next(c);
        }
        assert_eq!(seq, full, "independent fork walk reproduces the full sequence");
        // And the advanced copy walks the remaining tail correctly.
        let mut tail: Vec<(i64, i64)> = Vec::new();
        let mut a = advanced;
        loop {
            let k = op_map_iter_key(a);
            if k == Handle::NULL {
                break;
            }
            tail.push((op_get_int(k), op_get_int(op_map_iter_val(a))));
            a = op_map_iter_next(a);
        }
        assert_eq!(tail, full[1..].to_vec(), "advanced copy walks the tail from entry 1");
        op_drop(c);
        op_drop(a);
        op_drop(m);
        assert_eq!(live_nodes(), before, "no leak / no double-free across forked walks");
    }

    #[test]
    fn set_iter_next_fbip_fork_independent() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair();
        let (ca, cb) = full_hash_collision_pair();
        let mut s = op_set_empty();
        for &e in &[sa, sb, ca, cb, 6i64, 12, 24] {
            s = sinsert_int(s, e);
        }
        let full = collect_set(s);
        let cur = op_set_iter(s);
        let first = op_get_int(op_set_iter_elem(cur));
        assert_eq!(first, full[0]);
        op_dup(cur); // forked, rc==2
        let advanced = op_set_iter_next(cur); // copy path
        assert_eq!(op_get_int(op_set_iter_elem(cur)), first, "fork undisturbed");
        assert_eq!(op_get_int(op_set_iter_elem(advanced)), full[1], "advanced moved on");
        // Independent full walk of the fork.
        let mut seq: Vec<i64> = Vec::new();
        let mut c = cur;
        loop {
            let e = op_set_iter_elem(c);
            if e == Handle::NULL {
                break;
            }
            seq.push(op_get_int(e));
            c = op_set_iter_next(c);
        }
        assert_eq!(seq, full, "independent fork walk reproduces the full set sequence");
        op_drop(c);
        op_drop(advanced);
        op_drop(s);
        assert_eq!(live_nodes(), before, "no leak / no double-free");
    }

    #[test]
    fn map_iter_next_fbip_unique_zero_alloc() {
        reset();
        let m = deep_walk_map();
        let size = op_map_size(m) as usize;
        // A UNIQUE cursor walk: LIVE_NODES stays FLAT across every advance (the cursor shell is refit in
        // place — zero steady-state alloc). Contrast the shared case, which allocates a fresh cursor.
        let cur = op_map_iter(m);
        let after_iter = live_nodes();
        let mut steps = 0;
        let mut c = cur;
        loop {
            if op_map_iter_key(c) == Handle::NULL {
                break;
            }
            let pre = live_nodes();
            c = op_map_iter_next(c); // rc==1 ⇒ FBIP in place
            let delta = live_nodes() - pre;
            // In place: the advance nets ZERO node allocations (it may dup/drop frame refs, but frames
            // already exist; no new cursor node is built).
            assert_eq!(delta, 0, "unique cursor advance allocates zero nodes (step {steps})");
            steps += 1;
        }
        assert_eq!(steps, size, "walked exactly size entries");
        assert_eq!(live_nodes(), after_iter, "LIVE_NODES flat across the whole unique walk");
        op_drop(c);
        op_drop(m);

        // Prove the SHARED path DOES allocate (so the zero above is meaningful, not a no-op op).
        let m2 = deep_walk_map();
        let cur2 = op_map_iter(m2);
        op_dup(cur2); // rc==2 ⇒ copy path
        let pre = live_nodes();
        let adv = op_map_iter_next(cur2);
        assert!(live_nodes() - pre > 0, "shared cursor advance allocates a fresh cursor node");
        op_drop(cur2);
        op_drop(adv);
        op_drop(m2);
    }

    #[test]
    fn set_iter_next_fbip_unique_zero_alloc() {
        reset();
        let mut s = op_set_empty();
        for k in 0..50i64 {
            s = sinsert_int(s, k);
        }
        let size = op_set_size(s) as usize;
        let cur = op_set_iter(s);
        let after_iter = live_nodes();
        let mut steps = 0;
        let mut c = cur;
        loop {
            if op_set_iter_elem(c) == Handle::NULL {
                break;
            }
            let pre = live_nodes();
            c = op_set_iter_next(c);
            assert_eq!(live_nodes() - pre, 0, "unique set-cursor advance is zero-alloc (step {steps})");
            steps += 1;
        }
        assert_eq!(steps, size);
        assert_eq!(live_nodes(), after_iter, "LIVE_NODES flat across the unique set walk");
        op_drop(c);
        op_drop(s);
    }

    #[test]
    fn champ_cursor_next_fbip_take_past_exhaustion_is_sound() {
        reset();
        let before = live_nodes();
        // Guards `champ_cursor_take` — the mem::take that replaced the per-step frame clone in the
        // rc==1 FBIP advance. Two properties the take must preserve on the EXHAUSTED-return paths (where
        // it reinstalls an EMPTY frame vector via champ_become_cursor): (1) advancing a unique cursor
        // PAST the last entry, then re-reading and re-advancing the exhausted cursor, stays sound —
        // key/val read NULL, further advances are stable no-ops; (2) no frame is leaked or double-freed
        // across the whole over-walk (LIVE_NODES returns to baseline after the final drop).
        let m = deep_walk_map();
        let size = op_map_size(m) as usize;
        let mut c = op_map_iter(m);
        let mut steps = 0;
        while op_map_iter_key(c) != Handle::NULL {
            c = op_map_iter_next(c);
            steps += 1;
        }
        assert_eq!(steps, size, "walked exactly size entries before exhaustion");
        // Now exhausted. Re-read: both projections must be the NULL done-signal.
        assert_eq!(op_map_iter_key(c), Handle::NULL, "exhausted cursor key is NULL");
        assert_eq!(op_map_iter_val(c), Handle::NULL, "exhausted cursor val is NULL");
        // Advance PAST the end several more times (each takes the rc==1 take path, reinstalls empty):
        // must stay exhausted, allocate no node, and not corrupt the (empty) frame set.
        for _ in 0..3 {
            let pre = live_nodes();
            c = op_map_iter_next(c);
            assert_eq!(live_nodes() - pre, 0, "advancing an exhausted unique cursor allocates nothing");
            assert_eq!(op_map_iter_key(c), Handle::NULL, "still exhausted after over-advance");
        }
        op_drop(c);
        op_drop(m);
        assert_eq!(live_nodes(), before, "no frame leaked or double-freed across the over-walk");
    }

    #[test]
    fn iter_next_fbip_full_traversal_matches() {
        reset();
        let before = live_nodes();
        // The FBIP walk (collect_map/collect_set advance an rc==1 cursor in place) must visit exactly
        // `size` entries, each once, in a DETERMINISTIC order. Compare two independent walks of the
        // same map/set — identical order proves determinism; and the size/uniqueness proves coverage.
        let m = deep_walk_map();
        let walk_a = collect_map(m);
        let walk_b = collect_map(m);
        assert_eq!(walk_a, walk_b, "two FBIP map walks are identically ordered (deterministic)");
        assert_eq!(walk_a.len(), op_map_size(m) as usize, "map walk visited exactly size entries");
        let keys: std::collections::HashSet<i64> = walk_a.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys.len(), walk_a.len(), "each map key visited exactly once (incl. collision)");
        op_drop(m);

        let (ca, cb) = full_hash_collision_pair();
        let mut s = op_set_empty();
        for &e in &[ca, cb, 1i64, 2, 3, 40, 41] {
            s = sinsert_int(s, e);
        }
        let sa = collect_set(s);
        let sb = collect_set(s);
        assert_eq!(sa, sb, "two FBIP set walks are identically ordered");
        assert_eq!(sa.len(), op_set_size(s) as usize, "set walk visited exactly size entries");
        let selems: std::collections::HashSet<i64> = sa.iter().copied().collect();
        assert_eq!(selems.len(), sa.len(), "each set elem visited once (incl. collision)");
        assert!(selems.contains(&ca) && selems.contains(&cb), "collision elems both visited");
        op_drop(s);
        assert_eq!(live_nodes(), before, "no leak across the traversals");
    }

    #[test]
    fn champ_advance_fbip_frame_refcounts_balance_over_deep_walk() {
        reset();
        let before = live_nodes();
        // Guards champ_advance_fbip: the frame refcount delta is now applied INLINE during the walk
        // (op_drop each popped frame at the pop site, op_dup each descended frame) rather than by a
        // post-hoc diff against a cloned frame list. A miscount would leak (too few drops) or double-free
        // (too many). Build a map DEEP enough that a single advance both POPS several exhausted frames
        // AND DESCENDS a fresh multi-level tail (the case the inline delta must get exactly right), walk
        // it fully in place to exhaustion, then over-advance — and assert LIVE_NODES returns to baseline.
        // Keys sharing low 5/10/15 bits force ≥3 levels of subnodes; a collision pair adds a collision
        // frame at the floor, so the walk exercises pop-from-collision + pop-from-normal + deep descend.
        let (ca, cb) = full_hash_collision_pair();
        let deep = [
            0i64, 1 << 5, 1 << 10, (1 << 5) | (1 << 10), (1 << 10) | (1 << 15),
            1, 2, ca, cb, 7, 8, 40, 41,
        ];
        // Map each key to a small, collision-safe tag value (a running index, not k*const — the
        // collision pair carries full-width i64 payloads that would overflow a multiply).
        let reference: std::collections::BTreeMap<i64, i64> =
            deep.iter().enumerate().map(|(i, &k)| (k, 1000 + i as i64)).collect();
        let mut m = op_map_empty();
        for (&k, &v) in &reference {
            m = op_map_insert(m, op_box_int(k), op_box_int(v));
        }
        // Full in-place walk to exhaustion (unique cursor → champ_advance_fbip every step).
        let mut cur = op_map_iter(m);
        let mut seen: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
        loop {
            let k = op_map_iter_key(cur);
            if k == Handle::NULL {
                break;
            }
            let v = op_map_iter_val(cur);
            seen.insert(op_get_int(k), op_get_int(v)); // key→value pairing must survive the walk
            cur = op_map_iter_next(cur);
        }
        assert_eq!(seen, reference, "in-place walk visited exactly the reference key→value map");
        // Over-advance the exhausted cursor a few times (each an rc==1 advance on empty frames).
        for _ in 0..3 {
            cur = op_map_iter_next(cur);
            assert_eq!(op_map_iter_key(cur), Handle::NULL, "stays exhausted");
        }
        op_drop(cur);
        op_drop(m);
        assert_eq!(live_nodes(), before, "frame refcounts balanced — no leak, no double-free");
    }

    #[test]
    fn cursor_depth_never_exceeds_inline_slots_cap() {
        reset();
        let before = live_nodes();
        // Guards the inline `Slots` buffer's fixed capacity (SLOTS_CAP): a cursor's frame stack must
        // NEVER exceed it, or `Slots::push` traps. Build the DEEPEST possible cursor path — a
        // full-hash-collision pair forces descent through every trie level down to a collision node at
        // the hash floor (the maximum frame depth), plus split pairs and ordinary keys to populate
        // intermediate levels — then walk the whole map and assert at EVERY step that the cursor's frame
        // count stays within SLOTS_CAP. `handles.len()` on the cursor node IS its live frame depth.
        let (ca, cb) = full_hash_collision_pair(); // share all 32 hash bits ⇒ deepest descent
        let (sa, sb) = low5_split_pair();
        let mut m = op_map_empty();
        // Include the collision pair (max depth), split pairs (mid-depth subnodes), and spread keys.
        let mut ks: Vec<i64> = vec![ca, cb, sa, sb];
        for k in 0..40i64 {
            ks.push(k * 7 + 1);
        }
        for (i, &k) in ks.iter().enumerate() {
            m = op_map_insert(m, op_box_int(k), op_box_int(i as i64));
        }
        let mut cur = op_map_iter(m);
        let mut steps = 0;
        loop {
            // The cursor node's `handles` are its descent frames; `slots.len() == frames.len()`, so this
            // is exactly what the inline Slots buffer must hold.
            let depth = with_node(cur, 0usize, |n| n.handles.len());
            assert!(
                depth <= SLOTS_CAP,
                "cursor frame depth {depth} exceeds inline SLOTS_CAP {SLOTS_CAP} at step {steps}"
            );
            if op_map_iter_key(cur) == Handle::NULL {
                break;
            }
            cur = op_map_iter_next(cur);
            steps += 1;
        }
        assert_eq!(steps, ks.len(), "walked every entry (deepest paths included)");
        op_drop(cur);
        op_drop(m);
        assert_eq!(live_nodes(), before, "no leak");
    }

    // ── U7: CHAMP set algebra — union / intersection / difference ──────────────────────────────
    // Correctness vs a std BTreeSet reference; canonical shape; correct RC (consume both operands, no
    // leak / no double-free); empty-operand identities; shared-operand safety (a kept operand `dup`ed
    // first stays intact after the consuming op).

    /// Build a set from a slice of ints (each `sinsert_int` consumes the running set).
    fn set_of(elems: &[i64]) -> Handle {
        let mut s = op_set_empty();
        for &e in elems {
            s = sinsert_int(s, e);
        }
        s
    }

    /// Assert a runtime set's membership + size EXACTLY match a reference over `universe`.
    fn assert_set_eq_reference(s: Handle, reference: &std::collections::BTreeSet<i64>, universe: &[i64]) {
        assert_eq!(op_set_size(s) as usize, reference.len(), "size matches reference");
        for &e in universe {
            assert_eq!(scontains_int(s, e), reference.contains(&e), "membership of {e} matches reference");
        }
    }

    #[test]
    fn set_union_matches_reference() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair(); // force subnode splits into the operands
        let (ca, cb) = full_hash_collision_pair(); // a collision pair spanning both operands
        // Overlapping, disjoint, subset, identical — encoded as element-set pairs over a universe.
        let cases: Vec<(Vec<i64>, Vec<i64>)> = vec![
            (vec![1, 2, 3], vec![3, 4, 5]),                 // overlapping
            (vec![1, 2, 3], vec![10, 11, 12]),              // disjoint
            (vec![1, 2, 3, 4, 5], vec![2, 4]),              // b subset of a
            (vec![7, 8, 9], vec![7, 8, 9]),                 // identical
            (vec![sa, sb, 3, 17, 42], vec![sb, 42, 100]),   // subnode splits + overlap
            (vec![ca, 1, 2], vec![cb, 2, 3]),               // collision pair split across operands
        ];
        for (ea, eb) in &cases {
            let mut reference: std::collections::BTreeSet<i64> = ea.iter().copied().collect();
            reference.extend(eb.iter().copied());
            let universe: Vec<i64> = {
                let mut u: std::collections::BTreeSet<i64> = ea.iter().copied().collect();
                u.extend(eb.iter().copied());
                u.insert(999); // a non-member probe
                u.into_iter().collect()
            };
            let r = op_set_union(set_of(ea), set_of(eb));
            assert_set_eq_reference(r, &reference, &universe);
            op_drop(r);
        }
        assert_eq!(live_nodes(), before, "no leak across union cases");
    }

    #[test]
    fn set_union_base_choice_is_canonical_and_order_independent() {
        reset();
        let before = live_nodes();
        // Guards the "walk the SMALLER operand into the LARGER" base choice in op_set_union. Because the
        // CHAMP result is canonical-by-construction, union(a,b) must be BYTE-IDENTICAL (champ_eq +
        // champ_hash) to union(b,a) AND to a fresh set of all elements — regardless of which operand is
        // larger (hence which becomes the accumulator base). Use ASYMMETRIC sizes so the two directions
        // pick different bases, plus subnode-split and collision keys so the shape is non-trivial.
        let (sa, sb) = low5_split_pair();
        let (ca, cb) = full_hash_collision_pair();
        let big: Vec<i64> = vec![sa, sb, ca, cb, 1, 2, 3, 4, 5, 6, 7, 8]; // 12 elements
        let small: Vec<i64> = vec![sb, ca, 5, 100]; // 4 elements, partial overlap
        let mut all: std::collections::BTreeSet<i64> = big.iter().copied().collect();
        all.extend(small.iter().copied());
        let fresh = set_of(&all.iter().copied().collect::<Vec<_>>());

        let ab = op_set_union(set_of(&big), set_of(&small)); // base = big
        let ba = op_set_union(set_of(&small), set_of(&big)); // base = big too (larger), via the swap
        assert!(champ_eq(ab, ba), "union(big,small) == union(small,big) byte-identically");
        assert_eq!(champ_hash(ab), champ_hash(ba));
        assert!(champ_eq(ab, fresh), "union == a fresh set of all elements (canonical)");
        assert_eq!(champ_hash(ab), champ_hash(fresh));
        assert_eq!(op_set_size(ab) as usize, all.len(), "union has every distinct element once");
        op_drop(ab);
        op_drop(ba);
        op_drop(fresh);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn set_algebra_same_operand_short_circuits() {
        reset();
        let before = live_nodes();
        // Guards the O(1) pointer-identity (a==b) short-circuits: the idempotent set laws must hold when
        // the SAME handle is passed to both operands (structural sharing / self-op), with correct rc
        // (each op CONSUMES two references to the one node). Build a non-trivial set (a subnode split +
        // a collision pair, so the shape is real), then check a∪a=a, a∩a=a, a∖a=∅ — each by dup'ing the
        // set twice so both operand slots hold the same node, and asserting contents + no leak. Also
        // covers the EMPTY set (∅∪∅=∅ etc.) so the short-circuit is correct on the degenerate node too.
        let (sa, sb) = low5_split_pair();
        let (ca, cb) = full_hash_collision_pair();
        let elems = [sa, sb, ca, cb, 1i64, 2, 3, 42];
        let ref_set: std::collections::BTreeSet<i64> = elems.iter().copied().collect();
        let universe: Vec<i64> = elems.iter().copied().chain([999]).collect();

        // a ∪ a = a  (dup twice → both slots the same node → the a==b branch fires)
        let s = set_of(&elems);
        op_dup(s);
        op_dup(s); // s now has 3 refs: the two we pass + the one we keep to compare
        let u = op_set_union(s, s);
        assert!(champ_eq(u, s), "a ∪ a == a");
        assert_set_eq_reference(u, &ref_set, &universe);
        op_drop(u);

        // a ∩ a = a
        op_dup(s);
        op_dup(s);
        let x = op_set_intersection(s, s);
        assert!(champ_eq(x, s), "a ∩ a == a");
        assert_set_eq_reference(x, &ref_set, &universe);
        op_drop(x);

        // a ∖ a = ∅
        op_dup(s);
        op_dup(s);
        let d = op_set_difference(s, s);
        assert!(is_empty_node(d), "a ∖ a == ∅");
        assert_eq!(op_set_size(d), 0);
        op_drop(d);
        op_drop(s); // the reference we kept

        // The EMPTY set through each self-op (∅ is also a valid a==b node). Use a FRESH empty set per
        // op (each op consumes exactly the two references it is passed), so no cross-op aliasing.
        let e1 = op_set_empty();
        op_dup(e1);
        let eu = op_set_union(e1, e1); // consumes both refs, returns one (== e1)
        assert!(is_empty_node(eu), "∅ ∪ ∅ == ∅");
        op_drop(eu);
        let e2 = op_set_empty();
        op_dup(e2);
        let ex = op_set_intersection(e2, e2);
        assert!(is_empty_node(ex), "∅ ∩ ∅ == ∅");
        op_drop(ex);
        let e3 = op_set_empty();
        op_dup(e3);
        let ed = op_set_difference(e3, e3); // consumes both refs, returns a fresh empty
        assert!(is_empty_node(ed), "∅ ∖ ∅ == ∅");
        op_drop(ed);

        assert_eq!(live_nodes(), before, "self-op short-circuits balanced all refs — no leak/double-free");
    }

    #[test]
    fn set_intersection_matches_reference() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair();
        let (ca, cb) = full_hash_collision_pair();
        let cases: Vec<(Vec<i64>, Vec<i64>)> = vec![
            (vec![1, 2, 3], vec![3, 4, 5]),
            (vec![1, 2, 3], vec![10, 11, 12]),              // disjoint ⇒ empty
            (vec![1, 2, 3, 4, 5], vec![2, 4]),              // ⇒ {2,4}
            (vec![7, 8, 9], vec![7, 8, 9]),                 // identical ⇒ itself
            (vec![sa, sb, 3, 17, 42], vec![sb, 42, 100]),   // ⇒ {sb,42}
            (vec![ca, cb, 1], vec![ca, 2]),                 // one collision elem shared
        ];
        for (ea, eb) in &cases {
            let ra: std::collections::BTreeSet<i64> = ea.iter().copied().collect();
            let rb: std::collections::BTreeSet<i64> = eb.iter().copied().collect();
            let reference: std::collections::BTreeSet<i64> = ra.intersection(&rb).copied().collect();
            let universe: Vec<i64> = ra.union(&rb).copied().chain([999]).collect();
            let r = op_set_intersection(set_of(ea), set_of(eb));
            assert_set_eq_reference(r, &reference, &universe);
            op_drop(r);
        }
        assert_eq!(live_nodes(), before, "no leak across intersection cases");
    }

    #[test]
    fn set_hash_carrying_variants_match_plain() {
        reset();
        let before = live_nodes();
        // Guards the precomputed-hash variants (set_contains_h / set_insert_h / champ_find_base_h) the
        // set-algebra ops now use to hash each element ONCE instead of twice: passing `champ_hash(e)`
        // explicitly must be indistinguishable from letting the op recompute it. A wrong precomputed
        // hash would misplace or fail to find the element — so assert the `_h` forms agree with the
        // plain forms across present/absent, over BOTH scalar and (subtree-hashed) string elements.
        let s = set_of(&[1, 2, 3, 10, 20]);
        // Scalar probes: present and absent, plain vs _h must agree.
        for &k in &[1i64, 2, 3, 10, 20, 4, 99, -1] {
            let probe = op_box_int(k);
            let h = champ_hash(probe);
            assert_eq!(
                op_set_contains(s, probe),
                set_contains_h(s, probe, h),
                "contains vs contains_h disagree for {k}"
            );
            op_drop(probe);
        }
        // String elements exercise a real subtree hash (the case the once-hash win actually helps).
        let mut strs = op_set_empty();
        for w in ["alpha", "beta", "gamma"] {
            strs = op_set_insert(strs, op_str_new(w.to_string()));
        }
        for w in ["beta", "delta", "alpha", "zzz"] {
            let probe = op_str_new(w.to_string());
            let h = champ_hash(probe);
            assert_eq!(
                op_set_contains(strs, probe),
                set_contains_h(strs, probe, h),
                "string contains vs contains_h disagree for {w:?}"
            );
            op_drop(probe);
        }
        // set_insert_h with the right hash must equal a plain insert (same canonical set).
        let via_h = {
            let mut a = op_set_empty();
            for &k in &[5i64, 6, 7] {
                let e = op_box_int(k);
                a = set_insert_h(a, e, champ_hash(e));
            }
            a
        };
        let via_plain = set_of(&[7, 5, 6]); // different order — canonical result is order-independent
        assert!(champ_eq(via_h, via_plain), "set_insert_h builds the same canonical set as op_set_insert");
        assert_eq!(champ_hash(via_h), champ_hash(via_plain));
        op_drop(via_h);
        op_drop(via_plain);
        op_drop(s);
        op_drop(strs);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn set_difference_matches_reference() {
        reset();
        let before = live_nodes();
        let (sa, sb) = low5_split_pair();
        let (ca, cb) = full_hash_collision_pair();
        let cases: Vec<(Vec<i64>, Vec<i64>)> = vec![
            (vec![1, 2, 3], vec![3, 4, 5]),                 // ⇒ {1,2}
            (vec![1, 2, 3], vec![10, 11, 12]),              // disjoint ⇒ a
            (vec![1, 2, 3, 4, 5], vec![2, 4]),              // ⇒ {1,3,5}
            (vec![7, 8, 9], vec![7, 8, 9]),                 // identical ⇒ empty
            (vec![sa, sb, 3, 17, 42], vec![sb, 42, 100]),   // ⇒ {sa,3,17}
            (vec![ca, cb, 1], vec![ca, 2]),                 // ⇒ {cb,1}
        ];
        for (ea, eb) in &cases {
            let ra: std::collections::BTreeSet<i64> = ea.iter().copied().collect();
            let rb: std::collections::BTreeSet<i64> = eb.iter().copied().collect();
            let reference: std::collections::BTreeSet<i64> = ra.difference(&rb).copied().collect();
            let universe: Vec<i64> = ra.union(&rb).copied().chain([999]).collect();
            let r = op_set_difference(set_of(ea), set_of(eb));
            assert_set_eq_reference(r, &reference, &universe);
            op_drop(r);
        }
        assert_eq!(live_nodes(), before, "no leak across difference cases");
    }

    #[test]
    fn set_algebra_empty_operands() {
        reset();
        let before = live_nodes();
        // union(empty, b) == b
        let r = op_set_union(op_set_empty(), set_of(&[1, 2, 3]));
        assert_eq!(op_set_size(r), 3);
        assert!(scontains_int(r, 2));
        op_drop(r);
        // union(a, empty) == a
        let r = op_set_union(set_of(&[4, 5]), op_set_empty());
        assert_eq!(op_set_size(r), 2);
        assert!(scontains_int(r, 4));
        op_drop(r);
        // intersection(x, empty) == empty  AND  intersection(empty, x) == empty
        let r = op_set_intersection(set_of(&[1, 2, 3]), op_set_empty());
        assert_eq!(op_set_size(r), 0);
        assert!(is_empty_node(r));
        op_drop(r);
        let r = op_set_intersection(op_set_empty(), set_of(&[1, 2, 3]));
        assert_eq!(op_set_size(r), 0);
        assert!(is_empty_node(r));
        op_drop(r);
        // difference(a, empty) == a
        let r = op_set_difference(set_of(&[7, 8]), op_set_empty());
        assert_eq!(op_set_size(r), 2);
        assert!(scontains_int(r, 7) && scontains_int(r, 8));
        op_drop(r);
        // difference(empty, b) == empty
        let r = op_set_difference(op_set_empty(), set_of(&[1, 2]));
        assert_eq!(op_set_size(r), 0);
        assert!(is_empty_node(r));
        op_drop(r);
        // both empty, every op
        for r in [
            op_set_union(op_set_empty(), op_set_empty()),
            op_set_intersection(op_set_empty(), op_set_empty()),
            op_set_difference(op_set_empty(), op_set_empty()),
        ] {
            assert!(is_empty_node(r));
            op_drop(r);
        }
        assert_eq!(live_nodes(), before, "no leak across empty-operand identities");
    }

    #[test]
    fn set_algebra_result_is_canonical() {
        reset();
        let before = live_nodes();
        let (ca, cb) = full_hash_collision_pair(); // ensure a collision pair lands in the result
        // union result vs the SAME logical set folded in a DIFFERENT insertion order.
        let r = op_set_union(set_of(&[ca, 1, 5]), set_of(&[cb, 5, 9]));
        // Logical result = {ca, cb, 1, 5, 9}. Build it fresh in a scrambled order.
        let fresh = set_of(&[9, ca, 5, cb, 1]);
        assert!(champ_eq(r, fresh), "union result is canonical (== differently-ordered fold)");
        assert_eq!(champ_hash(r), champ_hash(fresh), "byte-identical canonical shape");
        op_drop(r);
        op_drop(fresh);
        // intersection result canonicality, also with the collision pair.
        let r = op_set_intersection(set_of(&[ca, cb, 1, 2, 3]), set_of(&[ca, cb, 3, 4]));
        let fresh = set_of(&[3, cb, ca]); // logical {ca,cb,3}, scrambled
        assert!(champ_eq(r, fresh), "intersection result is canonical");
        assert_eq!(champ_hash(r), champ_hash(fresh));
        op_drop(r);
        op_drop(fresh);
        assert_eq!(live_nodes(), before, "no leak");
    }

    #[test]
    fn set_algebra_no_leak_shared_operands() {
        reset();
        let before = live_nodes();
        // A caller keeping an operand `dup`s it first; the consuming op must not corrupt the retained
        // reference. Snapshot `a` for the champ_eq check.
        let a = set_of(&[1, 2, 3, 4]);
        let a_snapshot = collect_set(a); // order snapshot for later comparison
        op_dup(a); // keep a second owner across the consuming union
        let b = set_of(&[3, 4, 5, 6]);
        op_dup(b); // keep b too
        let r = op_set_union(a, b); // consumes ONE ref of each
        // The retained references are unchanged in value.
        assert_eq!(collect_set(a), a_snapshot, "retained operand a unchanged after consuming union");
        assert_eq!(op_set_size(a), 4);
        assert!(scontains_int(b, 5), "retained operand b unchanged");
        assert_eq!(op_set_size(b), 4);
        // The union is correct.
        assert_eq!(op_set_size(r), 6);
        for e in [1, 2, 3, 4, 5, 6] {
            assert!(scontains_int(r, e));
        }
        op_drop(a);
        op_drop(b);
        op_drop(r);
        assert_eq!(live_nodes(), before, "no leak / no double-free with shared operands");
    }

    #[test]
    fn set_algebra_fuzz_matches_reference() {
        reset();
        let before = live_nodes();
        // Fixed-seed LCG: random element sets over a small universe, all three ops vs BTreeSet.
        let mut lcg: u64 = 0xA5A5_1234;
        let next = |lcg: &mut u64| {
            *lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (*lcg >> 33) as u32
        };
        for _ in 0..60 {
            let mk = |lcg: &mut u64| -> (Handle, std::collections::BTreeSet<i64>) {
                let n = (next(lcg) % 20) as usize;
                let mut set = op_set_empty();
                let mut r = std::collections::BTreeSet::new();
                for _ in 0..n {
                    let e = (next(lcg) % 40) as i64;
                    set = sinsert_int(set, e);
                    r.insert(e);
                }
                (set, r)
            };
            let (sa, ra) = mk(&mut lcg);
            let (sb, rb) = mk(&mut lcg);
            // Each of the 3 ops CONSUMES one ref of sa and sb; plus the final op_drop needs one more.
            // So 4 refs each: start at rc 1, dup 3 times.
            for _ in 0..3 {
                op_dup(sa);
                op_dup(sb);
            }
            let universe: Vec<i64> = (0..40).collect();
            let u = op_set_union(sa, sb);
            assert_set_eq_reference(u, &ra.union(&rb).copied().collect(), &universe);
            op_drop(u);
            let i = op_set_intersection(sa, sb);
            assert_set_eq_reference(i, &ra.intersection(&rb).copied().collect(), &universe);
            op_drop(i);
            let d = op_set_difference(sa, sb);
            assert_set_eq_reference(d, &ra.difference(&rb).copied().collect(), &universe);
            op_drop(d);
            op_drop(sa);
            op_drop(sb);
        }
        assert_eq!(live_nodes(), before, "no leak across the fuzz");
    }

    // ── Review gap: split a RELAXED vector (produced by concat) ─────────────────────────────────
    // Every other split test uses vec_range(n) (STRICT, push-built), so `vec_split_subtree`'s relaxed
    // descent branch (vec_is_relaxed ⇒ vec_find_child_relaxed) was never exercised. Concat two lists
    // then split the result — a natural composition — to hit it.

    /// Build a relaxed 80-element vector = concat([0..40), [0..40)); assert its root is relaxed so we
    /// KNOW the relaxed-split branch is taken. Oracle = 0..40 followed by 0..40.
    fn relaxed_80() -> Handle {
        let c = op_vec_concat(vec_range(40), vec_range(40));
        let (_count, _shift, root) = vec_read_header(c);
        assert!(vec_is_relaxed(root), "concat(40,40) must produce a relaxed root");
        c
    }

    fn relaxed_80_oracle() -> Vec<i64> {
        let mut o: Vec<i64> = (0..40).collect();
        o.extend(0..40);
        o
    }

    #[test]
    fn vec_split_of_relaxed_matches_oracle() {
        reset();
        let oracle = relaxed_80_oracle();
        for &p in &[0u32, 1, 10, 32, 40, 41, 50, 72, 79, 80] {
            let before = live_nodes();
            let c = relaxed_80(); // fresh relaxed vector per split point
            let (l, r) = op_vec_split(c, p);
            assert_eq!(op_vec_len(l), p, "left len == p (p={p})");
            assert_eq!(op_vec_len(r), 80 - p, "right len == 80-p (p={p})");
            let left_want: Vec<i64> = oracle[..p as usize].to_vec();
            let right_want: Vec<i64> = oracle[p as usize..].to_vec();
            assert_eq!(vec_to_ints(l), left_want, "left elements (p={p})");
            assert_eq!(vec_to_ints(r), right_want, "right elements (p={p})");
            op_drop(l);
            op_drop(r);
            assert_eq!(live_nodes(), before, "no leak for relaxed split (p={p})");
        }
    }

    #[test]
    fn vec_split_of_relaxed_reconcat_roundtrip() {
        reset();
        let oracle = relaxed_80_oracle();
        for &p in &[1u32, 10, 40, 41, 79] {
            let before = live_nodes();
            let c = relaxed_80();
            let (l, r) = op_vec_split(c, p);
            let joined = op_vec_concat(l, r); // consumes both halves
            assert_eq!(op_vec_len(joined), 80, "reconcat len (p={p})");
            assert_eq!(vec_to_ints(joined), oracle, "reconcat elements (p={p})");
            assert_vec_invariants(joined);
            op_drop(joined);
            assert_eq!(live_nodes(), before, "no leak for relaxed reconcat (p={p})");
        }
    }

    #[test]
    fn vec_split_of_relaxed_outputs_valid_downstream() {
        reset();
        let before = live_nodes();
        let oracle = relaxed_80_oracle();
        let c = relaxed_80();
        let (mut l, mut r) = op_vec_split(c, 33); // split mid-first-run: left [0..33), right [33..80)
        assert_eq!(op_get_int(op_vec_get(l, 32)), oracle[32], "left last");
        assert_eq!(op_get_int(op_vec_get(r, 0)), oracle[33], "right first");
        // push onto both halves
        for i in 0..40i64 {
            l = op_vec_push(l, op_box_int(1000 + i));
            r = op_vec_push(r, op_box_int(2000 + i));
        }
        assert_eq!(op_vec_len(l), 73);
        assert_eq!(op_vec_len(r), 87);
        assert_eq!(op_get_int(op_vec_get(l, 72)), 1039, "left pushed tail");
        assert_eq!(op_get_int(op_vec_get(r, 86)), 2039, "right pushed tail");
        // update in the carried-over (relaxed-origin) region of each half
        l = op_vec_update(l, 10, op_box_int(-7));
        r = op_vec_update(r, 5, op_box_int(-8));
        assert_eq!(op_get_int(op_vec_get(l, 10)), -7);
        assert_eq!(op_get_int(op_vec_get(r, 5)), -8);
        assert_eq!(op_get_int(op_vec_get(l, 9)), oracle[9], "left neighbor untouched");
        assert_vec_invariants(l);
        assert_vec_invariants(r);
        // concat the two halves back together
        let joined = op_vec_concat(l, r);
        assert_eq!(op_vec_len(joined), 73 + 87);
        assert_vec_invariants(joined);
        op_drop(joined);
        assert_eq!(live_nodes(), before, "no leak after relaxed split + downstream ops");
    }

    #[test]
    fn vec_split_of_relaxed_preserves_invariant() {
        reset();
        for &p in &[1u32, 10, 32, 40, 41, 72, 79] {
            let before = live_nodes();
            let c = relaxed_80();
            let (l, r) = op_vec_split(c, p);
            assert_vec_invariants(l);
            assert_vec_invariants(r);
            op_drop(l);
            op_drop(r);
            assert_eq!(live_nodes(), before, "no leak (p={p})");
        }
    }

    #[test]
    fn vec_split_of_deep_relaxed_matches_oracle() {
        reset();
        // Fold-concat several vec_range chunks into a DEEPER relaxed vector (>1 interior level), then
        // split at interior points. Each chunk is [0..k); the oracle is their concatenation.
        let chunks = [30i64, 45, 60, 33, 50, 40]; // total 258 — forces multiple levels
        let before = live_nodes();
        let mut acc = op_vec_empty();
        let mut oracle: Vec<i64> = Vec::new();
        for &k in &chunks {
            acc = op_vec_concat(acc, vec_range(k));
            oracle.extend(0..k);
        }
        let total = oracle.len() as u32;
        let (_c, _s, root) = vec_read_header(acc);
        assert!(vec_is_relaxed(root), "deep fold-concat must produce a relaxed root");
        assert_eq!(op_vec_len(acc), total);
        // Split at several interior points; keep acc alive by dup-before-split.
        for &p in &[1u32, 29, 30, 75, 135, 168, 257] {
            op_dup(acc);
            let (l, r) = op_vec_split(acc, p);
            assert_eq!(op_vec_len(l), p, "deep left len (p={p})");
            assert_eq!(op_vec_len(r), total - p, "deep right len (p={p})");
            assert_eq!(vec_to_ints(l), oracle[..p as usize].to_vec(), "deep left elems (p={p})");
            assert_eq!(vec_to_ints(r), oracle[p as usize..].to_vec(), "deep right elems (p={p})");
            assert_vec_invariants(l);
            assert_vec_invariants(r);
            op_drop(l);
            op_drop(r);
        }
        op_drop(acc);
        assert_eq!(live_nodes(), before, "no leak across deep relaxed splits");
    }
}

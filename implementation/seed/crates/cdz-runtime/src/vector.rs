//! Persistent vector (RRB trie)
//!
//! 32-way radix trie + packed-bool leaves + RRB relaxed-radix + FBIP reuse.

use super::*;

// ─── Persistent vector — a 32-way radix trie ──────────────────────────────────────────────
// A persistent (immutable, structurally-shared) growable sequence, laid out as a Bagwell/Clojure
// 32-way radix trie over the SAME tagless `Node`. No new node field and no change to the free
// cascade — exactly the bytes rope's trick: a vector's nodes are ordinary
// `Node`s whose children live in `handles`, so structural sharing is just `rc > 1` on a shared
// subtree and the existing iterative `op_drop` reclaims a whole trie transitively. Sharing a subtree
// is transparent: a shared node is immutable and byte-identical to a copied one, so whether a version
// shares its predecessor's storage or copies it never changes what any op observes — the persistent
// update path-copies the spine and `op_dup`s the retained children, so the two are indistinguishable.
//= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
//# When the compiler represents a value by sharing another value's storage rather than by copying it, that sharing MUST NOT change the program's observable behavior, so that sharing storage is a transparent optimization rather than a distinction between two equal values.
// Tagless dispatch keeps this from colliding with tuples/lists: the compiler only ever calls `vec-*` on a value whose
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
pub(crate) const VEC_BITS: u32 = 5;
/// Radix digit mask: `(1 << VEC_BITS) - 1` — extracts one base-32 digit of an index.
pub(crate) const VEC_MASK: u32 = (1 << VEC_BITS) - 1;

/// Read a little-endian `u32` at byte `off` of `raw`, zero-padded past the end (total: a short raw
/// yields 0, never a panic — same discipline as `read_word`/`read_disc`).
///
/// FAST PATH: when the full 4-byte window is in bounds (the ALWAYS case for a real CHAMP header —
/// `champ_datamap`/`champ_nodemap`/`champ_size` on a 12-byte raw — and vec headers / relaxed size
/// tables), read the 4-byte subslice in ONE `from_le_bytes` with a single bounds check, instead of the
/// four per-byte `.get()` bounds checks the general loop does. This is on the hottest descent
/// (`champ_find_base_h` reads datamap+nodemap PER LEVEL) so the per-byte branches were measurable
/// (`Raw::as_slice`/`read_u32_at` ~3% of the profile). Byte-identical output. The zero-padded loop
/// stays for the defensive short/absent-raw tail (e.g. `champ_become_hdr`'s pre-resize probe).
#[inline]
pub(crate) fn read_u32_at(raw: &[u8], off: usize) -> u32 {
    if let Some(window) = raw.get(off..off + 4) {
        // SAFETY-FREE: `window` is exactly 4 bytes, so the array conversion cannot fail.
        return u32::from_le_bytes(window.try_into().unwrap());
    }
    let mut b = [0u8; 4];
    for k in 0..4 {
        if let Some(&byte) = raw.get(off + k) {
            b[k] = byte;
        }
    }
    u32::from_le_bytes(b)
}

// ─── Packed-bool vector leaves (memory-dense `List Bool`) ─────────────────────────────────────
// A `List Bool` LEAF stores its ≤32 boolean elements BIT-PACKED into a single `u32` instead of as up
// to 32 separate `imm_bool` handles in a heap `Vec` — ~6× denser (5 inline bytes vs a heap Vec of 32
// pointers) and one fewer allocation per leaf. This is a PURE-RUNTIME optimization: the compiler emits
// the identical `vec-*` ops for a `List Bool`, and the runtime auto-detects a bool element at leaf
// construction (`vec_leaf_of`/`op_vec_of_arr`) — no WIT op, no hint, no type channel.
//
// A leaf is PACKED iff `handles` is EMPTY and `raw` is exactly `[count: u8][bits: u32 LE]` = 5 bytes
// (`PACKED_BOOL_LEAF_RAW_LEN`), stored INLINE. Bit `i` of `bits` (LSB-first) is element `i`; bits at or
// above `count` are 0. The 5-byte length is the discriminant: within the vec subsystem a strict leaf
// carries its elements in `handles` (empty raw), a relaxed node's raw is `4*arity` bytes, a vec header's
// raw is 8, so no other trie node collides — and `vec_leaf_is_packed` is only ever asked of a genuine
// trie node (a leaf or interior), never of an element, so an unrelated 5-byte bytes/scalar VALUE that
// happens to be a list element is never misread as a packed leaf.
//
// WHY IT IS UNOBSERVABLE. A `List` is already non-byte-canonical (a concat-built vector has relaxed
// nodes where an `of-arr`-built one is strict, for the same logical value), so `ty_heap_walkable`
// returns `false` for `Ty::List` — a list is NEVER structurally `value-eq`'d nor used as a map/set key
// (never `champ_hash`/`champ_eq`'d). Packing adds a THIRD leaf shape beside strict/relaxed; a tree may
// freely MIX them (e.g. a packed leaf beside an unpacked one after a concat) and still read correctly,
// because every read funnels through `vec_arity`/`vec_child` (below) which decode a packed leaf on the
// fly into a count and `imm_bool` elements. A bool `imm_bool` is an `op_dup`/`op_drop`/`node_rc` no-op,
// so handing synthesized immediates back to callers that dup/drop them is ownership-trivial.

/// The `raw` length of a packed-bool leaf: `[count: u8]` + `[bits: u32 LE]`.
pub(crate) const PACKED_BOOL_LEAF_RAW_LEN: usize = 5;

/// Whether `h` is an inline boolean immediate (the element type a packed leaf holds). A non-immediate
/// (a heap node) or a non-bool immediate (unit/int) is not — so a non-`Bool` list never packs.
#[inline]
pub(crate) fn imm_is_bool(h: Handle) -> bool {
    is_immediate(h) && matches!(imm_kind(h), ImmKind::Bool)
}

/// Whether `node` is a packed-bool leaf (empty handles + a 5-byte `[count][bits]` raw). Total: a null
/// handle, an immediate, or any other node shape yields `false`.
#[inline]
pub(crate) fn vec_leaf_is_packed(node: Handle) -> bool {
    with_node(node, false, |n| {
        n.handles.is_empty() && n.raw.len() == PACKED_BOOL_LEAF_RAW_LEN
    })
}

/// The element count of a packed leaf (its `raw[0]`). Caller has verified `vec_leaf_is_packed`.
#[inline]
pub(crate) fn packed_leaf_count(node: Handle) -> usize {
    with_node(node, 0, |n| n.raw.first().copied().unwrap_or(0) as usize)
}

/// The `count` and `bits` of a packed leaf in one borrow. Caller has verified `vec_leaf_is_packed`.
#[inline]
pub(crate) fn packed_leaf_parts(node: Handle) -> (u8, u32) {
    with_node(node, (0, 0), |n| {
        (n.raw.first().copied().unwrap_or(0), read_u32_at(&n.raw, 1))
    })
}

/// Element `i` (`i < count ≤ 32`) of a packed leaf as an `imm_bool`. Caller has verified
/// `vec_leaf_is_packed`; `i` is a leaf slot (`idx & VEC_MASK`) so `i < 32` and the shift never overflows.
#[inline]
pub(crate) fn packed_leaf_get(node: Handle, i: usize) -> Handle {
    let (_, bits) = packed_leaf_parts(node);
    imm_bool((bits >> i) & 1 != 0)
}

/// Build the 5-byte `[count][bits]` raw of a packed leaf, inline (no heap).
#[inline]
pub(crate) fn packed_leaf_raw(count: u8, bits: u32) -> Raw {
    let mut buf = [0u8; PACKED_BOOL_LEAF_RAW_LEN];
    buf[0] = count;
    buf[1..5].copy_from_slice(&bits.to_le_bytes());
    Raw::inline(&buf)
}

/// A freshly-owned packed leaf (rc 1) of `count` bools whose values are `bits` (LSB-first).
#[inline]
pub(crate) fn packed_leaf_new(count: u8, bits: u32) -> Handle {
    alloc_raw(Handles::new(), packed_leaf_raw(count, bits))
}

/// Set (or clear) bit `i` of `bits` to `v`.
#[inline]
pub(crate) fn set_bit(bits: u32, i: usize, v: bool) -> u32 {
    (bits & !(1u32 << i)) | ((v as u32) << i)
}

/// Convert an rc==1 packed leaf IN PLACE back to a normal strict leaf (elements as `imm_bool` handles,
/// empty raw). The defensive escape hatch for the (well-typed-impossible) case of a NON-bool element
/// joining a `List Bool` leaf — a list is homogeneous, so a packed leaf only ever exists in a `List Bool`
/// whose every element is a bool immediate, and this never fires for well-typed code; it keeps the leaf
/// mutators TOTAL (deterministic, never a miscompile) if the compiler ever emitted a mixed list.
pub(crate) fn packed_leaf_unpack_inplace(node: Handle) {
    let (count, bits) = packed_leaf_parts(node);
    if let Some(n) = unsafe { node.node_mut() } {
        let mut hs = Handles::new();
        for i in 0..count as usize {
            hs.push(imm_bool((bits >> i) & 1 != 0));
        }
        n.handles = hs;
        n.raw = Raw::from(Vec::new());
    }
}

/// PATH-COPY append of element `e` to a packed leaf: a fresh packed leaf of `count + 1` bits with bit
/// `count` = `e`'s value. `e` (a bool immediate) is consumed with no drop (an immediate owns no heap).
/// The original leaf is untouched (the caller releases it), matching `vec_node_append`'s dup-siblings
/// contract (a packed leaf has no siblings to dup). Defensive strict-leaf fallback if `e` is not a bool
/// or the leaf is somehow full (well-typed-impossible for a `List Bool` — a leaf-level append has room).
pub(crate) fn packed_leaf_append(node: Handle, e: Handle) -> Handle {
    let (count, bits) = packed_leaf_parts(node);
    if imm_is_bool(e) && (count as usize) < 32 {
        return packed_leaf_new(count + 1, set_bit(bits, count as usize, imm_as_bool(e)));
    }
    let mut hs = Vec::with_capacity(count as usize + 1);
    for i in 0..count as usize {
        hs.push(imm_bool((bits >> i) & 1 != 0));
    }
    hs.push(e);
    alloc(hs, Vec::new())
}

/// PATH-COPY replace of element `sub` of a packed leaf with `e`: a fresh packed leaf with bit `sub` =
/// `e`'s value. `e` (bool imm) consumed with no drop; the replaced element (also a bool imm) needs none.
/// The original leaf is untouched. Defensive strict-leaf fallback if `e` is not a bool.
pub(crate) fn packed_leaf_replace(node: Handle, sub: usize, e: Handle) -> Handle {
    let (count, bits) = packed_leaf_parts(node);
    if imm_is_bool(e) {
        return packed_leaf_new(count, set_bit(bits, sub, imm_as_bool(e)));
    }
    let mut hs = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        hs.push(if i == sub {
            e
        } else {
            imm_bool((bits >> i) & 1 != 0)
        });
    }
    alloc(hs, Vec::new())
}

/// IN-PLACE append (FBIP, rc==1) of `e` to a packed leaf: bump `count` and set bit `count`, patching
/// the 5-byte raw. SAFETY: caller verified rc == 1. Defensive unpack-then-push if `e` is not a bool or
/// the leaf is full.
pub(crate) fn packed_leaf_push_inplace(node: Handle, e: Handle) {
    if imm_is_bool(e) {
        let (count, bits) = packed_leaf_parts(node);
        if (count as usize) < 32 {
            if let Some(n) = unsafe { node.node_mut() } {
                n.raw = packed_leaf_raw(count + 1, set_bit(bits, count as usize, imm_as_bool(e)));
            }
            return;
        }
    }
    packed_leaf_unpack_inplace(node);
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles.push(e);
    }
}

/// IN-PLACE set (FBIP, rc==1) of element `sub` of a packed leaf to `e`: patch bit `sub` in the raw.
/// SAFETY: caller verified rc == 1. The caller separately releases the OLD element (a bool imm — a
/// drop no-op). Defensive unpack-then-set if `e` is not a bool.
pub(crate) fn packed_leaf_set_inplace(node: Handle, sub: usize, e: Handle) {
    if imm_is_bool(e) {
        let (count, bits) = packed_leaf_parts(node);
        if let Some(n) = unsafe { node.node_mut() } {
            n.raw = packed_leaf_raw(count, set_bit(bits, sub, imm_as_bool(e)));
        }
        return;
    }
    packed_leaf_unpack_inplace(node);
    if let Some(n) = unsafe { node.node_mut() } {
        if let Some(slot) = n.handles.get_mut(sub) {
            *slot = e;
        }
    }
}

/// Build a leaf from a slice of ≤32 element handles: PACKED when they are all bool immediates (the
/// `List Bool` case), else a normal strict leaf. Consumes the handles (moved into the leaf, or read as
/// bits — bool immediates own no heap so no drop is needed). Used by the `op_vec_of_arr` >32 chunking
/// path so a large `List Bool` literal packs every leaf, matching a push-built one.
pub(crate) fn vec_leaf_from_handles(hs: Vec<Handle>) -> Handle {
    if !hs.is_empty() && hs.len() <= 32 && hs.iter().all(|&e| imm_is_bool(e)) {
        let mut bits = 0u32;
        for (i, &e) in hs.iter().enumerate() {
            if imm_as_bool(e) {
                bits |= 1 << i;
            }
        }
        return packed_leaf_new(hs.len() as u8, bits);
    }
    alloc(hs, Vec::new())
}

/// If `arr` (an array node whose `handles` ARE its elements) holds 1..=32 elements that are ALL bool
/// immediates, return their packed bits (LSB = element 0); else `None`. Used to pack a `List Bool`
/// literal at `op_vec_of_arr` instead of reusing the arr node as a strict leaf.
pub(crate) fn arr_all_bool_bits(arr: Handle) -> Option<u32> {
    with_node(arr, None, |n| {
        let els = n.handles.as_slice();
        if els.is_empty() || els.len() > 32 || !els.iter().all(|&e| imm_is_bool(e)) {
            return None;
        }
        let mut bits = 0u32;
        for (i, &e) in els.iter().enumerate() {
            if imm_as_bool(e) {
                bits |= 1 << i;
            }
        }
        Some(bits)
    })
}

/// The count of a trie node's children (its arity). A null node has none (benign). A PACKED leaf reports
/// its bit count so every element-count reader (get/len/subtree-size/split/concat/invariant walks) sees
/// a packed leaf as a leaf of `count` elements with no other change.
pub(crate) fn vec_arity(node: Handle) -> usize {
    if vec_leaf_is_packed(node) {
        return packed_leaf_count(node);
    }
    with_node(node, 0, |n| n.handles.len())
}
/// The `i`-th child handle of a trie node, or NULL if absent (benign — the descent stays within a
/// valid tree by construction, so this never returns NULL in correct operation). A PACKED leaf decodes
/// bit `i` into an `imm_bool` on the fly, so every reader (leaf reads, dup-collect, split partition) sees
/// the same `imm_bool` elements it would from an unpacked leaf.
pub(crate) fn vec_child(node: Handle, i: usize) -> Handle {
    if vec_leaf_is_packed(node) {
        return packed_leaf_get(node, i);
    }
    with_node(node, Handle::NULL, |n| {
        n.handles.get(i).copied().unwrap_or(Handle::NULL)
    })
}

/// Build a vector header owning `root` (or childless when `root` is NULL, i.e. the empty vector).
pub(crate) fn vec_alloc_header(count: u32, shift: u32, root: Handle) -> Handle {
    // The 8-byte `[count][shift]` vector header, built INLINE (no transient heap Vec).
    let mut raw = [0u8; INLINE_RAW_CAP];
    raw[0..4].copy_from_slice(&count.to_le_bytes());
    raw[4..8].copy_from_slice(&shift.to_le_bytes());
    // A header holds its single root handle (or none, for the empty vector) — always ≤ 1 child, and it
    // NEVER grows past one (a root swap replaces slot 0 via `vec_set_child_inplace`; a level-grow builds a
    // fresh 2-child root, not a 2nd header handle). So carry the handle INLINE (`inline_from`) rather than
    // `vec![root]` — a header on EVERY vector construction otherwise pays a heap `Vec` alloc for one handle,
    // where the inline arm (cap 2) holds it in the `Node` itself. Reads/mutations are storage-transparent.
    let handles = if root == Handle::NULL {
        Handles::new()
    } else {
        Handles::inline_from(&[root])
    };
    alloc_raw(handles, Raw::Inline { len: 8, buf: raw })
}

/// Decode a header into `(count, shift, root)`. Borrows — no ownership change. A null/short header
/// yields the empty-vector triple.
pub(crate) fn vec_read_header(v: Handle) -> (u32, u32, Handle) {
    with_node(v, (0, 0, Handle::NULL), |n| {
        (
            read_u32_at(&n.raw, 0),
            read_u32_at(&n.raw, 4),
            n.handles.first().copied().unwrap_or(Handle::NULL),
        )
    })
}

/// A one-element leaf node holding `e` (consumed into it).
pub(crate) fn vec_leaf_of(e: Handle) -> Handle {
    if imm_is_bool(e) {
        // A `List Bool` leaf packs: element 0's value goes in bit 0. `e` is an immediate (nothing to
        // consume — no heap), so no drop is needed. Subsequent pushes grow this packed leaf in place.
        return packed_leaf_new(1, imm_as_bool(e) as u32);
    }
    // Born on the HEAP arm: an RRB leaf grows toward 32 elements via in-place `vec_push_child_inplace`,
    // so inlining it (≤2) would only pay a spill on the 3rd push with no lasting benefit (it ends up
    // heap regardless). `from_vec_heap` keeps the single-element Vec as the backing to grow into.
    alloc_raw(Handles::from_vec_heap(vec![e]), Raw::from(Vec::new()))
}

/// Append `child` (consumed) to a trie node, `dup`ing the existing children into the copy — the
/// container gains an owned reference to each carried-over subtree while the old node keeps its own
/// (the subtree is now shared). Used both for a leaf gaining an element and an interior gaining a
/// branch; the two are the same op over `handles`.
pub(crate) fn vec_node_append(node: Handle, child: Handle) -> Handle {
    if vec_leaf_is_packed(node) {
        // A packed leaf (always level 0) gains one element — grow the packed bits, no per-sibling dup
        // (its elements live in the raw, not in `handles`).
        return packed_leaf_append(node, child);
    }
    // Deref `node` ONCE and copy its children from the borrowed slice (was one `vec_child` deref +
    // null-check per sibling — up to 32 for a strict node; the compiler can't hoist them because
    // `op_dup` writes child memory, defeating alias analysis on the parent). `op_dup(c)` mutates a
    // CHILD node's rc, disjoint from the parent `&Node` we hold — sound (a well-formed RRB tree has
    // no cycles, so no child aliases its parent).
    let mut hs = with_node(node, Vec::new(), |n| {
        let children = n.handles.as_slice();
        let mut hs = Vec::with_capacity(children.len() + 1);
        for &c in children {
            op_dup(c);
            hs.push(c);
        }
        hs
    });
    hs.push(child);
    alloc(hs, Vec::new())
}

/// Copy `node`, replacing child `sub` with `new_child` (consumed) and `dup`ing every sibling into the
/// copy (shared). This is the path-copy step: one new node per level, all off-path subtrees shared.
/// Emits a STRICT copy (empty raw); use it only when `node` is strict, or when the replacement changes
/// the node's kind. For a relaxed node whose sizes are unchanged (e.g. an in-place element update),
/// use `vec_node_replace_keep_raw` so the size table survives the copy.
pub(crate) fn vec_node_replace(node: Handle, sub: usize, new_child: Handle) -> Handle {
    if vec_leaf_is_packed(node) {
        // A packed leaf (always level 0): replace bit `sub`. Its "siblings" are bits, not handles, so
        // there is nothing to dup and the old element (a bool imm) needs no drop.
        return packed_leaf_replace(node, sub, new_child);
    }
    // One deref of `node` + copy from the borrowed slice (was a `vec_child` deref per sibling). See
    // `vec_node_append` for the aliasing argument (op_dup mutates a disjoint CHILD node).
    let hs = with_node(node, Vec::new(), |n| {
        let children = n.handles.as_slice();
        let mut hs = Vec::with_capacity(children.len());
        for (j, &c) in children.iter().enumerate() {
            if j == sub {
                hs.push(new_child);
            } else {
                op_dup(c);
                hs.push(c);
            }
        }
        hs
    });
    alloc(hs, Vec::new())
}

/// Like `vec_node_replace`, but carries `node`'s `raw` (its relaxed size table) into the copy verbatim.
/// Correct only when the replacement does NOT change any child's element count — the case for
/// `vec-update`, which swaps one leaf element for another. Preserves the strict-vs-relaxed kind: a
/// strict node (empty raw) stays strict, a relaxed node keeps its table.
pub(crate) fn vec_node_replace_keep_raw(node: Handle, sub: usize, new_child: Handle) -> Handle {
    // One deref of `node` — copy the children from the borrowed slice AND read the raw size table in
    // the same borrow (was a `vec_child` deref per sibling PLUS a separate `with_node` for the raw).
    let (hs, raw) = with_node(node, (Vec::new(), Vec::new()), |n| {
        let children = n.handles.as_slice();
        let mut hs = Vec::with_capacity(children.len());
        for (j, &c) in children.iter().enumerate() {
            if j == sub {
                hs.push(new_child);
            } else {
                op_dup(c);
                hs.push(c);
            }
        }
        (hs, n.raw.to_vec())
    });
    alloc(hs, raw)
}

/// Push-append helper for a RELAXED node whose last child gained exactly one element: copy the node
/// replacing child `last` with `new_child` (consumed, siblings shared) and bump ONLY the final
/// cumulative-size entry by 1 (every preceding boundary is unchanged since only the last child grew).
pub(crate) fn vec_relaxed_grow_last(node: Handle, last: usize, new_child: Handle) -> Handle {
    let copy = vec_node_replace_keep_raw(node, last, new_child);
    // `copy` is a fresh sole owner (rc 1) carrying the old size table; add 1 to its final u32 entry.
    // Mutate in place via `as_mut` — the same discipline the reuse ops use for a just-allocated node.
    if let Some(n) = unsafe { copy.node_mut() } {
        let off = n.raw.len() - 4; // raw.len() == 4*arity ≥ 4 for a relaxed node
        let bumped = read_u32_at(&n.raw, off) + 1;
        n.raw.as_mut_slice()[off..off + 4].copy_from_slice(&bumped.to_le_bytes());
    }
    copy
}

/// Push-append helper for a RELAXED node whose last child is full: copy the node appending `branch`
/// (consumed) as a new rightmost child covering exactly one new element, extending the size table with
/// `old_total + 1`.
pub(crate) fn vec_relaxed_append_branch(node: Handle, branch: Handle) -> Handle {
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
pub(crate) fn vec_new_path(level: u32, node: Handle) -> Handle {
    if level == 0 {
        node
    } else {
        // Arity-1 single-child spine node — build the handle INLINE (no transient `vec![child]` heap Vec
        // that `From<Vec>` would re-inline + free).
        alloc_raw(
            Handles::inline_from(&[vec_new_path(level - VEC_BITS, node)]),
            Raw::from(Vec::new()),
        )
    }
}

/// Insert element `e` at dense index `i` into the subtree rooted at `node` (borrowed), path-copying.
/// At a leaf (`level == 0`) `e` is appended; at an interior node the rightmost existing child is
/// path-copied (`sub < arity`) or a brand-new branch is appended (`sub == arity`). Returns a new
/// owned subtree; `e` is consumed.
pub(crate) fn vec_push_into(node: Handle, level: u32, i: u32, e: Handle) -> Handle {
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
pub(crate) fn vec_update_into(node: Handle, level: u32, i: u32, e: Handle) -> Handle {
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
pub(crate) fn vec_is_relaxed(node: Handle) -> bool {
    with_node(node, false, |n| {
        let rlen = n.raw.len();
        let hlen = n.handles.len();
        rlen > 0 && rlen == 4 * hlen
    })
}

/// The `i`-th cumulative size (u32 LE at offset `4*i`) of a relaxed node's size table — the element
/// count of children `0..=i`. Short/absent raw yields 0 (benign, matching `read_u32_at`).
pub(crate) fn vec_relaxed_size_at(node: Handle, i: usize) -> u32 {
    with_node(node, 0, |n| read_u32_at(&n.raw, 4 * i))
}

/// The element count of child `i` alone in a relaxed node: `sizes[i] - sizes[i-1]` (with
/// `sizes[-1] := 0`). Used by U2/U3 rebalancing; kept here beside its reader.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn vec_relaxed_child_size(node: Handle, i: usize) -> u32 {
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
pub(crate) fn vec_find_child_relaxed(node: Handle, idx: u32) -> (usize, u32) {
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

// `vec-empty` — a new owned empty vector (rc 1). No root node until the first push.
// The shared IMMORTAL empty-vec singleton (lazily minted on first use), the `IMM_UNIT` analog for lists.
// `Handle::NULL` marks "not yet minted" (a real empty-vec is a heap node, never null), so the first
// `op_vec_empty` allocates + immortalizes it and every later call returns the SAME node.
runtime_local! {
    static EMPTY_VEC: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

pub(crate) fn op_vec_empty() -> Handle {
    // Return the shared IMMORTAL empty-vec. An empty list is CONSTANT (all empties are structurally equal,
    // no elements), so ONE immortal instance is semantically correct: it is EXCLUDED from the live-objects
    // census (rc = IMMORTAL; `op_mark_immortal` decrements the counter), so it NEVER shows as a leak — this
    // is the fix for the mixed-recursive List-fold terminal leak (the base `(list)` and every runtime-
    // generated empty, e.g. value-encode's defensive totals, now share one node instead of minting a fresh
    // MORTAL empty per call). It is also NEVER FBIP-mutated: every vec mutator gates on `node_rc == 1`, and
    // IMMORTAL != 1, so `vec-push`/`vec-set` on it take the persistent COPY path (verified) — read-only,
    // no shared-singleton corruption (the same discipline that protects `IMM_UNIT` and the immortal sums).
    EMPTY_VEC.with(|slot| {
        let mut e = slot.get();
        if e.0.is_null() {
            e = vec_alloc_header(0, 0, Handle::NULL);
            op_mark_immortal(e);
            slot.set(e);
        }
        e
    })
}

/// `vec-len` — the element count. Borrows; returns a `u32` by value.
pub(crate) fn op_vec_len(v: Handle) -> u32 {
    vec_read_header(v).0
}

/// `vec-get` — the element at `index` (BORROWED; the vector keeps ownership). An out-of-bounds index
/// TRAPS (fail-fast, like `arr-get`); the compiler emits the sign-aware bounds check on its side, so
/// reaching the trap is a compiler-invariant violation. After the `index < count` guard the trie
/// descent is in-bounds by construction.
pub(crate) fn op_vec_get(v: Handle, index: u32) -> Handle {
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
// CRITICAL: ALIASING SAFETY (a violation silently corrupts a shared persistent version). `mine` means "this
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
pub(crate) fn vec_set_child_inplace(node: Handle, sub: usize, child: Handle) {
    if vec_leaf_is_packed(node) {
        // A packed leaf (always level 0): patch bit `sub` in place. The old element (a bool imm) is
        // released by the caller — a drop no-op.
        packed_leaf_set_inplace(node, sub, child);
        return;
    }
    if let Some(n) = unsafe { node.node_mut() } {
        if let Some(slot) = n.handles.get_mut(sub) {
            *slot = child;
        }
    }
}

/// In-place append `child` to an rc==1 node's handles (no dup). SAFETY: caller verified rc == 1.
pub(crate) fn vec_push_child_inplace(node: Handle, child: Handle) {
    if vec_leaf_is_packed(node) {
        // A packed leaf (always level 0) gains one element: grow the packed bits in place. (The header
        // and interior nodes carry ≥1 handle or an 8-byte raw, so they never match here.)
        packed_leaf_push_inplace(node, child);
        return;
    }
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles.push(child);
    }
}

/// In-place set an rc==1 vector HEADER's `[count][shift]` raw. SAFETY: caller verified rc == 1.
///
/// A header's raw is ALWAYS exactly 8 bytes (`vec_alloc_header` builds it inline), so PATCH the two
/// u32s in place via `as_mut_slice` rather than `clear()` + two capacity-checked `extend_from_slice`
/// calls. This runs on EVERY `op_vec_push`/`op_vec_update` on a unique vector (the hot FBIP path) and
/// showed up as `vec_set_header_inplace` + `Raw::extend_from_slice` in the vec profile. The defensive
/// branch (a header somehow not 8 bytes — never in correct operation) rebuilds via clear+extend.
pub(crate) fn vec_set_header_inplace(v: Handle, count: u32, shift: u32) {
    if let Some(n) = unsafe { v.node_mut() } {
        if n.raw.len() == 8 {
            let r = n.raw.as_mut_slice();
            r[0..4].copy_from_slice(&count.to_le_bytes());
            r[4..8].copy_from_slice(&shift.to_le_bytes());
        } else {
            n.raw.clear();
            n.raw.extend_from_slice(&count.to_le_bytes());
            n.raw.extend_from_slice(&shift.to_le_bytes());
        }
    }
}

/// In-place add 1 to the FINAL u32 entry of an rc==1 relaxed node's size table (a push into its last
/// child grew that child by one element). SAFETY: caller verified rc == 1 and the node is relaxed.
pub(crate) fn vec_bump_last_size_inplace(node: Handle) {
    if let Some(n) = unsafe { node.node_mut() } {
        let off = n.raw.len() - 4;
        let bumped = read_u32_at(&n.raw, off) + 1;
        n.raw.as_mut_slice()[off..off + 4].copy_from_slice(&bumped.to_le_bytes());
    }
}

/// In-place append a new rightmost child `branch` (covering one new element) to an rc==1 relaxed node,
/// extending its size table with `old_total + 1`. SAFETY: caller verified rc == 1 and relaxed.
pub(crate) fn vec_relaxed_append_branch_inplace(node: Handle, branch: Handle) {
    let old_total = {
        let arity = vec_arity(node);
        if arity == 0 {
            0
        } else {
            vec_relaxed_size_at(node, arity - 1)
        }
    };
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles.push(branch);
        n.raw.extend_from_slice(&(old_total + 1).to_le_bytes());
    }
}

/// FBIP variant of `vec_update_into`. When `mine`, mutate `node` in place and return the SAME handle;
/// otherwise delegate to the path-copying `vec_update_into` (returns a fresh node). Element counts are
/// unchanged by an update, so a relaxed node's size table is left untouched. Bounded-depth (≤7).
pub(crate) fn vec_update_fbip(node: Handle, level: u32, i: u32, e: Handle, mine: bool) -> Handle {
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
pub(crate) fn vec_push_fbip(node: Handle, level: u32, i: u32, e: Handle, mine: bool) -> Handle {
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
pub(crate) fn op_vec_push(v: Handle, elem: Handle) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    let header_mine = node_rc(v) == 1;

    if !header_mine {
        // Header shared: original behavior — build a fresh version, leave `v` (and its version) intact.
        let (new_root, new_shift) = if count == 0 {
            (vec_leaf_of(elem), 0)
        } else if (count as u64) == (1u64 << (shift + VEC_BITS)) {
            op_dup(root);
            let branch = vec_new_path(shift, vec_leaf_of(elem));
            // Arity-2 new root — build handles INLINE (exactly INLINE_HANDLES_CAP, no transient Vec).
            (
                alloc_raw(Handles::inline_from(&[root, branch]), Raw::from(Vec::new())),
                shift + VEC_BITS,
            )
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
        // Arity-2 new root — build handles INLINE (no transient `vec![root, branch]` heap Vec).
        let new_root = alloc_raw(Handles::inline_from(&[root, branch]), Raw::from(Vec::new()));
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

/// PATH-COPY front-prepend of `elem` into the subtree `node` at `level`, returning the NEW owned subtree,
/// or `None` if `node` is FULL at the front (its front spine is full to the leaf AND `node` itself has 32
/// children — the caller then grows a level). BALANCED (log-depth): `elem` PACKS into the leftmost leaf
/// while it has room (< 32), and a fresh front subtree is added only when a level is genuinely full — the
/// front-growth mirror of `vec-push`'s tail-append, NOT a new single-element child per prepend (which built
/// a degenerate O(n)-deep tree, `List.prepend`'s original miscompile: value-correct but O(n) reads + a
/// `1<<level` overflow in `vec_subtree_size` past ~7 levels).
///
/// RECLAIM: `node` is BORROWED (not consumed) — the caller frees the old tree via `op_drop(v)`, whose
/// cascade reclaims exactly the OLD front spine this copied. Off-path siblings CARRY FORWARD, so they are
/// `dup`'d (rc bumped) into the new node → the cascade takes them rc2→1 (survive). The front child that the
/// recursion descends into is NOT dup'd (a fresh front replaces it; the cascade frees the old one). This is
/// the standard persistent path-copy, rc-correct for a uniquely-owned AND a shared `node`.
pub(crate) fn vec_prepend_into(node: Handle, level: u32, elem: Handle) -> Option<Handle> {
    let arity = vec_arity(node);
    let cap = VEC_MASK as usize + 1; // 32
    if level == 0 {
        // Leaf: pack `elem` at the front if there is room; a full leaf declines (caller adds a sibling).
        if arity >= cap {
            return None;
        }
        let mut kids = Vec::with_capacity(arity + 1);
        kids.push(elem);
        vec_collect_children_dup(node, &mut kids); // dup the old elements (they carry forward)
        return Some(vec_leaf_from_handles(kids));
    }
    let front = vec_child(node, 0);
    if let Some(new_front) = vec_prepend_into(front, level - VEC_BITS, elem) {
        // The front subtree absorbed `elem`. Path-copy: new child 0 = new_front (the old front is freed by
        // the caller's cascade — NOT dup'd here); children 1.. carry forward (dup'd).
        let mut kids = Vec::with_capacity(arity);
        kids.push(new_front);
        for i in 1..arity {
            let c = vec_child(node, i);
            op_dup(c);
            kids.push(c);
        }
        return Some(vec_relaxed_node(kids, level));
    }
    // Front subtree is FULL. If `node` has room, add a NEW front child — a fresh spine to a 1-element leaf
    // at this level's child depth (amortized rare, exactly `vec-push`'s full-tail case). ALL old children
    // (including the full old front child 0) carry forward → dup them.
    if arity >= cap {
        return None; // `node` full too — the caller grows a level above it
    }
    let mut kids = Vec::with_capacity(arity + 1);
    kids.push(vec_new_path(level - VEC_BITS, vec_leaf_of(elem)));
    vec_collect_children_dup(node, &mut kids); // dup all old children (they follow the new front child)
    Some(vec_relaxed_node(kids, level))
}

/// `vec-prepend` — a new owned vector = `elem` followed by the elements of `v`. CONSUMES `v` and `elem`
/// (a constructor, like `vec-push`); the FRONT-growth twin of `vec-push`'s tail-growth. `List.prepend`
/// lowers to this, REPLACING the old `concat(singleton, v)` path — which invoked the full RRB merge
/// (lifting the singleton to `v`'s level) per prepend and leaked the superseded front-spine. Front growth
/// PACKS into the leftmost leaf (`vec_prepend_into`), staying LOG-DEPTH — not a new single-element child
/// per prepend (which built a degenerate O(n)-deep tree). RECLAIM discipline: the path-copy `dup`s the
/// carried-forward (off-path) children and `op_drop(v)` frees the old front spine — rc-correct for BOTH a
/// uniquely-owned `v` (shell frees, shared children go rc2→1) AND a shared `v` (rc > 1 → decremented,
/// children stay shared, RRB persistence intact). SOUND on the immortal empty-vec base (count 0 → a fresh
/// one-element vector; `op_drop` of the shared immortal is a no-op).
pub(crate) fn op_vec_prepend(v: Handle, elem: Handle) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    let cap = VEC_MASK as usize + 1; // 32 — max children per node
    if count == 0 {
        // Onto an empty vector (incl. the shared immortal empty-vec): a fresh one-element vector.
        let hdr = vec_alloc_header(1, 0, vec_leaf_of(elem));
        op_drop(v); // frees a mortal empty header; a no-op on the immortal empty singleton
        return hdr;
    }
    if shift == 0 {
        // Leaf root: the new leaf is `elem` then the old ≤32 elements (dup'd so they survive op_drop(v)).
        let mut kids = Vec::with_capacity(count as usize + 1);
        kids.push(elem);
        vec_collect_children_dup(root, &mut kids);
        let (new_root, new_shift) = if kids.len() <= cap {
            (vec_leaf_from_handles(kids), 0)
        } else {
            // 33 elements → two leaves under a 2-child relaxed parent (one level up).
            let k = kids.len().div_ceil(2);
            let right = kids.split_off(k);
            (
                vec_relaxed_node(
                    vec![vec_leaf_from_handles(kids), vec_leaf_from_handles(right)],
                    VEC_BITS,
                ),
                VEC_BITS,
            )
        };
        let hdr = vec_alloc_header(count + 1, new_shift, new_root);
        op_drop(v);
        return hdr;
    }
    // Interior root: PACK `elem` into the leftmost leaf via the balanced path-copy. On overflow (the whole
    // front spine full AND the root has 32 children) grow ONE level: a fresh single-element front subtree
    // beside the (dup'd) old root — the front-growth mirror of `vec-push`'s root-overflow, and the ONLY
    // place a single-element spine is minted (amortized, never per-prepend).
    let (new_root, new_shift) = match vec_prepend_into(root, shift, elem) {
        Some(new_root) => (new_root, shift),
        None => {
            let new_front = vec_new_path(shift, vec_leaf_of(elem)); // 1-element front subtree at the old root's level
            op_dup(root); // the old root carries forward as the new root's second child
            (
                vec_relaxed_node(vec![new_front, root], shift + VEC_BITS),
                shift + VEC_BITS,
            )
        }
    };
    let hdr = vec_alloc_header(count + 1, new_shift, new_root);
    op_drop(v);
    hdr
}

/// `vec-update` — a new owned vector = `v` with `index` set to `elem`. CONSUMES `v` and `elem`. OOB
/// index traps (like `vec-get`). FBIP fast path: when `v`'s header is uniquely owned it is reused and
/// the affected root→leaf path is refit in place wherever each node is uniquely owned; a shared node
/// path-copies exactly as before. A shared header takes the original allocate-new-header path.
pub(crate) fn op_vec_update(v: Handle, index: u32, elem: Handle) -> Handle {
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
pub(crate) fn vec_subtree_size(node: Handle, level: u32) -> u32 {
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
pub(crate) fn vec_grow_to_shift(mut node: Handle, mut shift: u32, target: u32) -> Handle {
    while shift < target {
        // Arity-1 strict single-child wrapper — inline the single handle (no transient `vec![node]`).
        node = alloc_raw(Handles::inline_from(&[node]), Raw::from(Vec::new()));
        shift += VEC_BITS;
    }
    node
}

/// Push each child of an OWNED node into `out`, `dup`ing it so it survives the node's later `op_drop`
/// (same discipline as `vec_node_append`). The node keeps its own references; `out` gains owned ones.
pub(crate) fn vec_collect_children_dup(node: Handle, out: &mut Vec<Handle>) {
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
pub(crate) fn vec_relaxed_node(children: Vec<Handle>, level: u32) -> Handle {
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
pub(crate) fn op_vec_concat(a: Handle, b: Handle) -> Handle {
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
    // Pre-size to the known maximum (each root has ≤`cap`=32 children after growth, so ≤64 total) — one
    // allocation, skipping the 1→2→…→64 realloc chain a growing `Vec` would do while gathering.
    let mut children: Vec<Handle> = Vec::with_capacity(2 * cap);
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
            // Leaf merge: elements fit one leaf. PACK it when all-bool (a `List Bool` concat stays
            // dense), else a STRICT leaf (uniform size-1, no table needed).
            (vec_leaf_from_handles(children), 0)
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
            // Two leaves (level 0): packed when all-bool, else strict.
            (vec_leaf_from_handles(left), vec_leaf_from_handles(right))
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

/// `vec-of-arr` — build a persistent vector from an already-built flat `arr` (the tuple/record array
/// primitive) in ONE call, the lowering target for a `(list …)` literal. CONSUMES `arr`. The `arr` node
/// carries the elements exactly as a vector LEAF does (`handles` = elements, empty `raw`), so:
///   - 0 elements (arr is inline unit)   → the empty vector.
///   - ≤32 elements (fits one leaf)      → REUSE the `arr` node itself as the leaf-root by move (zero
///     extra allocation beyond the 8-byte header) — the common small-list-literal case.
///   - >32 elements                      → drain the elements into ≤32-element strict leaves and build a
///     strict left-full radix trie bottom-up in one pass (no per-element persistent rebuild).
/// The result is byte-identical to (and interchangeable with) a push-built vector of the same elements.
pub(crate) fn op_vec_of_arr(arr: Handle) -> Handle {
    let count = op_arr_len(arr);
    if count == 0 {
        op_drop(arr); // consume the (inline unit) arr; the empty vector is independent
        return op_vec_empty();
    }
    let cap = VEC_MASK as usize + 1; // 32 elements per leaf
    if (count as usize) <= cap {
        // A `List Bool` literal (≤32 all-bool elements) packs into ONE dense leaf instead of reusing the
        // arr node — ~6× denser. `arr`'s elements are bool immediates (nothing to drop individually);
        // release the now-superseded arr shell.
        if let Some(bits) = arr_all_bool_bits(arr) {
            let leaf = packed_leaf_new(count as u8, bits);
            op_drop(arr);
            return vec_alloc_header(count, 0, leaf);
        }
        // The arr node IS a valid strict single leaf (handles = elements, empty raw). Move it in as the
        // root — no copy, no per-element push. `shift == 0` for a leaf-only tree.
        return vec_alloc_header(count, 0, arr);
    }
    // >32: take the arr's element handles out (a move — no dup, they relocate into the leaves) and pack
    // them into ≤32-element leaves — packed when all-bool (a large `List Bool` literal), else strict.
    let mut elems = champ_take_handles(arr).into_vec();
    op_drop(arr); // the now-empty arr shell
    let mut leaves: Vec<Handle> = Vec::with_capacity(elems.len().div_ceil(cap));
    let mut rest = &mut elems[..];
    while !rest.is_empty() {
        let take = rest.len().min(cap);
        let (chunk, tail) = rest.split_at_mut(take);
        leaves.push(vec_leaf_from_handles(chunk.to_vec()));
        rest = tail;
    }
    // Build a strict, left-full radix trie bottom-up: each interior level groups ≤32 children until one
    // root remains. `shift` rises by VEC_BITS per level. (A vec built this way is dense/left-full, so it
    // stays STRICT — no relaxed size tables needed, unlike concat/split boundary nodes.)
    let mut level_nodes = leaves;
    let mut shift = 0u32;
    while level_nodes.len() > 1 {
        shift += VEC_BITS;
        let mut parents: Vec<Handle> = Vec::with_capacity(level_nodes.len().div_ceil(cap));
        let mut i = 0;
        while i < level_nodes.len() {
            let end = (i + cap).min(level_nodes.len());
            parents.push(alloc(level_nodes[i..end].to_vec(), Vec::new())); // strict interior node
            i = end;
        }
        level_nodes = parents;
    }
    vec_alloc_header(count, shift, level_nodes.pop().unwrap())
}

/// Split the subtree rooted at `node` (BORROWED, top level `level`) at LOCAL element index `idx`
/// (`0 < idx < subtree_size` at the top; deeper calls may hit the `idx==0`/`idx==size` boundaries).
/// Returns `(left, right)` as freshly OWNED subtrees at the SAME level as `node` — `Handle::NULL` for a
/// side that ends up empty. Carried-over whole children are `dup`ed (they survive the caller's later
/// `op_drop(v)`); the boundary child is split recursively. EVERY rebuilt boundary node is RELAXED with a
/// correct cumulative size table (a trim can leave an interior node non-dense — U2's gotcha — so it must
/// NOT stay strict); LEAVES stay strict (their elements are uniformly size-1, valid after any trim).
/// Bounded-depth: `level` drops by `VEC_BITS` each call, so recursion is ≤ the 7-level trie height.
pub(crate) fn vec_split_subtree(node: Handle, level: u32, idx: u32) -> (Handle, Handle) {
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
            vec_leaf_from_handles(hs) // packed when all-bool, else strict
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
            vec_leaf_from_handles(hs) // packed when all-bool, else strict
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

/// The RIGHT-ONLY twin of `vec_split_subtree`: build ONLY the tail `[idx, …)` subtree, never the
/// discarded left prefix. `vec-drop` (the `(list p… .. rest)` REST binder, the HOT per-element fold step)
/// throws the left half away, so `vec_split_subtree`'s left leaf + `left_children` Vec + left relaxed node
/// + the dup-then-drop of every left element are PURE WASTE — roughly half the split's allocation. This
/// keeps only the kept side: whole children AFTER the boundary (dup'ed — they survive the caller's
/// `op_drop(v)`) plus the boundary child's own tail (recursively). The dropped children ([0,sub) and the
/// boundary child's head) are NOT dup'ed, so the caller's `op_drop(v)` reclaims them — identical ownership
/// to the split path, minus the transient left half. Returns the right subtree at `node`'s level (NULL if
/// the tail is empty). Same relaxed-rebuild + bounded-depth discipline as `vec_split_subtree`.
pub(crate) fn vec_take_tail(node: Handle, level: u32, idx: u32) -> Handle {
    if level == 0 {
        let arity = vec_arity(node) as u32;
        if idx >= arity {
            return Handle::NULL;
        }
        let mut hs = Vec::with_capacity((arity - idx) as usize);
        for j in idx..arity {
            let e = vec_child(node, j as usize);
            op_dup(e);
            hs.push(e);
        }
        return vec_leaf_from_handles(hs); // kept tail elements — packed when all-bool, else strict
    }
    let arity = vec_arity(node);
    let (sub, loc) = if vec_is_relaxed(node) {
        vec_find_child_relaxed(node, idx)
    } else {
        let sub = ((idx >> level) & VEC_MASK) as usize;
        (sub, idx & ((1u32 << level) - 1))
    };
    // Only the boundary child is split (its tail kept); its head + all children [0, sub) are left for the
    // caller's `op_drop(v)` to reclaim (never dup'ed here).
    let cr = vec_take_tail(vec_child(node, sub), level - VEC_BITS, loc);
    let mut right_children: Vec<Handle> = Vec::with_capacity(arity - sub);
    if cr != Handle::NULL {
        right_children.push(cr);
    }
    for j in (sub + 1)..arity {
        let c = vec_child(node, j);
        op_dup(c);
        right_children.push(c);
    }
    if right_children.is_empty() {
        Handle::NULL
    } else {
        vec_relaxed_node(right_children, level)
    }
}

/// `vec-drop(v, index)` — the tail `[index, len)`, CONSUMING `v`. The `(list p… .. rest)` REST-binder op
/// and the per-element step of a list fold, so it is HOT. Builds ONLY the kept tail spine (`vec_take_tail`)
/// — NOT the discarded left prefix a `split`+drop-left would materialize then free — then reclaims the
/// original tree (`op_drop(v)` frees every node the tail did not `dup`). Boundaries mirror `op_vec_split`:
/// `index == 0` returns `v` unchanged (whole tail); `index >= len` returns the canonical empty.
pub(crate) fn op_vec_drop_tail(v: Handle, index: u32) -> Handle {
    let (count, shift, root) = vec_read_header(v);
    if index == 0 {
        return v; // whole vector is the tail — flow through unchanged (its rc already covers the caller)
    }
    if index >= count {
        op_drop(v);
        return op_vec_empty();
    }
    let rroot = vec_take_tail(root, shift, index);
    // rroot is non-NULL (0 < index < count) at level `shift` — split never raises height.
    let tail = vec_alloc_header(count - index, shift, rroot);
    op_drop(v); // reclaim the original tree; kept subtrees survive via the dups in vec_take_tail
    tail
}

/// `vec-split` — split `v` at element `index` into `(left, right)` where `left` is elements
/// `[0, index)` and `right` is elements `[index, len)`. CONSUMES `v`; returns TWO new owned vectors.
/// Boundaries: `index == 0` → `(empty, v)` (v flows through as the right output, unchanged); `index >=
/// len` → `(v, empty)`. Both outputs honor every relaxed-node invariant and are valid for downstream
/// get/len/push/update/concat. Bounded-depth (via `vec_split_subtree`). `index > len` is clamped to
/// `len` (total split — a benign no-op split, mirroring how the boundary is the identity).
pub(crate) fn op_vec_split(v: Handle, index: u32) -> (Handle, Handle) {
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

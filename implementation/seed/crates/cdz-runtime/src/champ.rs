//! CHAMP persistent map and set
//!
//! Compressed Hash-Array Mapped Prefix trie for maps and sets.

use super::*;

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
pub(crate) const CHAMP_HEADER_SIZE: usize = 12;

/// 32-bit FNV-1a offset basis and prime, for the structural hash.
pub(crate) const FNV_OFFSET: u32 = 0x811c_9dc5;
pub(crate) const FNV_PRIME: u32 = 0x0100_0193;

/// One FNV-1a byte step: xor then multiply. `wrapping_mul` because FNV is defined mod 2^32.
#[inline]
pub(crate) fn fnv_step(h: u32, b: u8) -> u32 {
    (h ^ b as u32).wrapping_mul(FNV_PRIME)
}

// ── Header read/write ───────────────────────────────────────────────────────────────────

/// Read the datamap word (u32 LE at offset 0) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
pub(crate) fn champ_datamap(raw: &[u8]) -> u32 {
    read_u32_at(raw, 0)
}
/// Read the nodemap word (u32 LE at offset 4) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
pub(crate) fn champ_nodemap(raw: &[u8]) -> u32 {
    read_u32_at(raw, 4)
}
/// Read the subtree size (u32 LE at offset 8) from a node's raw header. Short/empty raw ⇒ 0.
#[allow(dead_code)]
pub(crate) fn champ_size(raw: &[u8]) -> u32 {
    read_u32_at(raw, 8)
}
/// Build a CHAMP raw header `[datamap][nodemap][size]` (12 bytes, all little-endian).
#[allow(dead_code)]
pub(crate) fn champ_header(datamap: u32, nodemap: u32, size: u32) -> Raw {
    // Build the 12-byte `[datamap][nodemap][size]` header directly as an INLINE `Raw` — no transient
    // heap Vec (CHAMP_HEADER_SIZE == INLINE_RAW_CAP, so it always inlines). This is the alloc-saving
    // win: every fresh CHAMP node (merge split, collision, path-copy rebuild, empty map/set) previously
    // allocated a 12-byte Vec here that `alloc` then copied inline and dropped.
    let mut buf = [0u8; INLINE_RAW_CAP];
    buf[0..4].copy_from_slice(&datamap.to_le_bytes());
    buf[4..8].copy_from_slice(&nodemap.to_le_bytes());
    buf[8..12].copy_from_slice(&size.to_le_bytes());
    Raw::Inline {
        len: CHAMP_HEADER_SIZE as u8,
        buf,
    }
}

// ── Bitmap / slot arithmetic ──────────────────────────────────────────────────────────────

/// Number of inline entries in this node: popcount of the datamap.
#[allow(dead_code)]
pub(crate) fn data_count(datamap: u32) -> u32 {
    datamap.count_ones()
}
/// Number of child subnodes in this node: popcount of the nodemap.
#[allow(dead_code)]
pub(crate) fn subnode_count(nodemap: u32) -> u32 {
    nodemap.count_ones()
}
/// Position of slot `i`'s entry within the entry region: count of set datamap bits below `i`.
/// `i` is a 5-bit slot (0..=31), so `1 << i` never overflows.
#[allow(dead_code)]
pub(crate) fn entry_index_for_slot(datamap: u32, i: u32) -> u32 {
    (datamap & ((1u32 << i) - 1)).count_ones()
}
/// Position of slot `i`'s subnode within the subnode region: count of set nodemap bits below `i`.
#[allow(dead_code)]
pub(crate) fn subnode_index_for_slot(nodemap: u32, i: u32) -> u32 {
    (nodemap & ((1u32 << i) - 1)).count_ones()
}
/// The 5-bit trie index selected by `level` (0-based): hash bits [5*level, 5*level+5).
#[allow(dead_code)]
pub(crate) fn level_index(hash: u32, level: u32) -> u32 {
    (hash >> (VEC_BITS * level)) & VEC_MASK
}

// ── Node-kind discrimination (tag-free) ─────────────────────────────────────────────────

/// True iff `node` is the canonical EMPTY node: both bitmaps 0 AND no handles. This is the root of
/// an empty map/set. Kept disambiguated from a collision node (which also has both bitmaps 0).
#[allow(dead_code)]
pub(crate) fn is_empty_node(node: Handle) -> bool {
    with_node(node, true, |n| {
        champ_datamap(&n.raw) == 0 && champ_nodemap(&n.raw) == 0 && n.handles.is_empty()
    })
}
/// True iff `node` is a COLLISION node: both bitmaps 0 AND at least one handle. Holds entries that
/// share a full 32-bit hash; only occurs at maximum depth and is linear-scanned by structural eq.
#[allow(dead_code)]
pub(crate) fn is_collision_node(node: Handle) -> bool {
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
pub(crate) fn champ_node_raw_hash(h: Handle) -> u32 {
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

/// One step of `champ_hash`'s iterative post-order walk (module-scoped so the reusable thread-local
/// worklist below can name it). `Visit` expands a node's children; `Combine` folds the node's own raw
/// with the `arity` child hashes now on the results stack.
pub(crate) enum HashTask {
    Visit(Handle),
    Combine(Handle, usize),
}

runtime_local! {
    /// REUSED scratch worklists for `champ_hash`'s general (nested-compound) walk. A nested KEY is
    /// hashed on every insert/lookup/remove, and each hash needs a `work` task stack + a `results`
    /// hash stack — freshly allocating them (even pre-sized) was 2 heap allocs PER HASH. Caching them
    /// thread-locally lets each hash `clear()` + reuse the buffers: they grow ONCE to the high-water
    /// mark, then every subsequent hash is allocation-FREE. Safe because the runtime is single-threaded
    /// and `champ_hash`'s walk is iterative + never re-enters `champ_hash` (so the borrow never nests).
    static HASH_SCRATCH: core::cell::RefCell<(Vec<HashTask>, Vec<u32>)> =
        core::cell::RefCell::new((Vec::new(), Vec::new()));
}

runtime_local! {
    /// REUSED scratch worklist for `champ_eq`'s general (NESTED-compound) equality walk — the same
    /// alloc-elision as `HASH_SCRATCH`. The shallow fast path (both compounds have only arity-0
    /// children) never touches this; a genuinely nested compound (e.g. a record-of-records or
    /// tuple-of-tuples compared by the language `=`, now that `value-eq` exposes `champ_eq`) fell
    /// through to a freshly-allocated `Vec<(Handle,Handle)>` per comparison. Reusing one buffer
    /// (clear + refill) makes a nested value-eq allocation-FREE steady-state. Safe: single-threaded,
    /// the walk is iterative and never re-enters `champ_eq` (`with_raw_arity` uses a stack buffer), so
    /// the borrow never nests.
    static EQ_SCRATCH: core::cell::RefCell<Vec<(Handle, Handle)>> =
        core::cell::RefCell::new(Vec::new());
}

runtime_local! {
    /// REUSED scratch worklist for `fill_rope_bytes` (the `bytes_flatten` materialize walk) — same
    /// alloc-elision as `HASH_SCRATCH`/`EQ_SCRATCH`. Every `String.at`/`Bytes` read + `bytes-compact`
    /// flattens a rope, and the walk needs a `(node, dst_off, src_start, count)` task stack; freshly
    /// allocating it was 1 heap alloc PER FLATTEN (on TOP of the output buffer — which the small path now
    /// keeps on the stack). Caching it thread-locally lets each flatten `clear()` + reuse: grow once, then
    /// every subsequent flatten is allocation-FREE steady-state — the lexer's per-char `String.at`-compact
    /// (the hot text-scan loop) now allocates NOTHING for a ≤12-byte result. Safe: single-threaded, the
    /// walk is iterative and never re-enters `bytes_flatten` (it only reads nodes + calls the O(1)
    /// `op_bytes_len`), so the borrow never nests.
    pub(crate) static FLATTEN_SCRATCH: core::cell::RefCell<Vec<(Handle, usize, usize, usize)>> =
        core::cell::RefCell::new(Vec::new());
}

/// A deterministic structural hash of the whole subtree rooted at `root`: FNV-1a over each node's
/// raw bytes folded with its children's hashes. Because the rep is canonical, structurally-equal
/// subtrees hash equal; differing raw or structure (very likely) differs.
///
/// ITERATIVE, not recursive: a post-order walk over an explicit task worklist plus a results stack
/// (mirroring `op_drop`'s worklist discipline) keeps native/wasm stack use at O(1) frames regardless
/// of trie depth. Null handles fold as the empty (offset-basis) hash. Does NOT cache — v1 recomputes.
#[allow(dead_code)]
pub(crate) fn champ_hash(root: Handle) -> u32 {
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
        if n.handles
            .iter()
            .all(|&c| is_immediate(c) || with_node(c, 0usize, |cn| cn.handles.len()) == 0)
        {
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
    // General nested-compound walk. Two-phase task: `Visit` expands a node's children; `Combine` folds
    // this node's raw + the child hashes now on `results`. Children are pushed Visit-first so their
    // Combine completes before their parent's — a standard single-stack iterative post-order. The two
    // scratch stacks are REUSED from a thread-local (see `HASH_SCRATCH`): each call clears them and
    // returns them empty-but-capacious, so after the first nested hash they never allocate again — the
    // walk is allocation-FREE steady-state (was 2 allocs/hash even when pre-sized).
    HASH_SCRATCH.with(|cell| {
        let (work, results) = &mut *cell.borrow_mut();
        work.clear();
        results.clear();
        work.push(HashTask::Visit(root));
        while let Some(task) = work.pop() {
            match task {
                HashTask::Visit(h) => {
                    if is_immediate(h) {
                        // Inline value: arity 0, no children to expand. Combine folds its canonical raw.
                        work.push(HashTask::Combine(h, 0));
                        continue;
                    }
                    let arity = with_node(h, 0usize, |n| n.handles.len());
                    work.push(HashTask::Combine(h, arity));
                    with_node(h, (), |n| {
                        for &c in n.handles.iter() {
                            work.push(HashTask::Visit(c));
                        }
                    });
                }
                HashTask::Combine(h, arity) => {
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
    })
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
pub(crate) fn champ_eq(a: Handle, b: Handle) -> bool {
    // Shallow-compound fast path — the hot compound-KEY compare (a small tuple/record key on a slot
    // hit): two equal-arity compounds whose children are ALL arity-0 are equal iff their raw bytes
    // match and every child pair is `with_raw_arity`-equal, WITHOUT allocating the worklist Vec below.
    // Only fires when NEITHER side is immediate and both are real nodes (the general path handles
    // immediates/nulls); a nested child (arity > 0) falls through to the lazy worklist.
    if !is_immediate(a) && !is_immediate(b) && a != b {
        if let Some(result) = unsafe {
            match (a.node_ref(), b.node_ref()) {
                (Some(na), Some(nb)) => {
                    if *na.raw != *nb.raw || na.handles.len() != nb.handles.len() {
                        Some(false) // roots differ ⇒ not equal, no descent
                    } else if na.handles.iter().chain(nb.handles.iter()).all(|&c| {
                        is_immediate(c)
                            || c.node_ref().map(|cn| cn.handles.is_empty()).unwrap_or(true)
                    }) {
                        // Shallow: every child on both sides is arity-0 → compare pairwise inline.
                        let eq = na.handles.iter().zip(nb.handles.iter()).all(|(&cx, &cy)| {
                            cx == cy
                                || with_raw_arity(cx, |rx, ax| {
                                    with_raw_arity(cy, |ry, ay| rx == ry && ax == ay)
                                })
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
    // General NESTED-compound walk. The worklist is REUSED from a thread-local (`EQ_SCRATCH`): each
    // call clears it and returns it empty-but-capacious, so after the first nested compare it never
    // allocates again (was a fresh `Vec` per nested comparison). Safe: single-threaded + the walk is
    // iterative and never re-enters `champ_eq`, so the borrow never nests.
    EQ_SCRATCH.with(|cell| {
        let work = &mut *cell.borrow_mut();
        work.clear();
        let mut pair = Some((a, b));
        while let Some((x, y)) = pair {
            // Process (x, y); `descend` is set to the children to push when both are equal compounds.
            let mut descend: Option<(&Node, &Node)> = None;
            if x == y {
                // same pointer (incl. both NULL) ⇒ identical subtree, no descent needed
            } else if is_immediate(x) || is_immediate(y) {
                // An immediate's `.0` is NOT a Node pointer — compare by decoded value (arity 0, so
                // equality reduces to equal canonical raw bytes and equal arity), WITHOUT allocating.
                let equal =
                    with_raw_arity(x, |rx, ax| with_raw_arity(y, |ry, ay| rx == ry && ax == ay));
                if !equal {
                    return false;
                }
            } else {
                match (unsafe { x.node_ref() }, unsafe { y.node_ref() }) {
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
                for i in 0..nx.handles.len() {
                    work.push((nx.handles[i], ny.handles[i]));
                }
            }
            pair = work.pop();
        }
        true
    })
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
pub(crate) fn champ_key_cmp(a: Handle, b: Handle) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    // Shallow-compound fast path (mirrors `champ_eq`): order two compounds whose children are ALL
    // arity-0 by raw bytes, then arity, then children in INDEX order (the general walk descends index 0
    // first), each via `with_raw_arity` — WITHOUT the worklist Vec. Consistent with the shallow
    // `champ_eq` path (both reduce to the same per-child (raw, arity) compare). Nested ⇒ general walk.
    if !is_immediate(a) && !is_immediate(b) && a != b {
        if let Some(ord) = unsafe {
            match (a.node_ref(), b.node_ref()) {
                (Some(na), Some(nb)) => {
                    let shallow = na.handles.iter().chain(nb.handles.iter()).all(|&c| {
                        is_immediate(c)
                            || c.node_ref().map(|cn| cn.handles.is_empty()).unwrap_or(true)
                    });
                    if !shallow {
                        None // a nested child — use the general worklist walk
                    } else {
                        let mut ord = na
                            .raw
                            .as_slice()
                            .cmp(nb.raw.as_slice())
                            .then(na.handles.len().cmp(&nb.handles.len()));
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
            let ord = with_raw_arity(x, |rx, ax| {
                with_raw_arity(y, |ry, ay| rx.cmp(ry).then(ax.cmp(&ay)))
            });
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            match (unsafe { x.node_ref() }, unsafe { y.node_ref() }) {
                (None, None) => {} // both null (unreachable given x==y, but total)
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

/// A work item for [`value_cmp_shaped`]'s iterative three-way walk: either a PAIR of handles to compare
/// under a shape index, or a LENGTH tiebreak (evaluated only after a list's common-prefix elements all
/// compared equal — the shorter list is Less).
pub(crate) enum CmpTask {
    Pair { a: Handle, b: Handle, shape_ix: u32 },
    LenTie { la: u32, lb: u32 },
}

/// The BLESSED total-order three-way comparison of two heap values `a`, `b` of the SAME type, guided by the
/// compiler-baked shape `desc` (the same descriptor `value_encode` reads). Returns `Some(Less/Equal/Greater)`
/// for an orderable pair, or `None` when the shape does not offer a total order (a Float/Float32 leaf — the
/// spec's #319 partial-order carve-out; a Bytes leaf — the spec blesses no Bytes order; a Set/Map — ordering
/// not yet offered) OR the descriptor is malformed. The compiler DECLINES ordering for those at lower time
/// (it never emits a `value-cmp` call for a non-orderable type), so `None` here is a defensive not-reached.
///
/// LEXICOGRAPHIC per `core-semantics.md` #Compound Ordering Is Lexicographic + #58/#64: leaves by their
/// blessed order (Int NUMERIC/signed, BigInt/Rational by value, Bool false<true, Str content-lexicographic
/// over the UTF-8 bytes = scalar order), tuples/records by field in canonical (descriptor) order, sums by
/// discriminant then payload, lists element-wise with a proper prefix LESS than its extension. Iterative
/// (an explicit `CmpTask` stack — wasm-safe, no unbounded native recursion over deep data, like
/// `champ_key_cmp`); the first non-`Equal` leaf/length decides and the walk returns immediately.
/// Compare two values as a SCALAR LEAF, allocation-free — the zero-Vec fast path `value_cmp_shaped` takes for
/// a scalar-rooted compare (and the shape a canonically-scalar Set/Map key/element always has). Returns:
///   - `None` — the shape (resolved through Named/Ref) is NOT a scalar leaf (a Tuple/List/Record/Sum): the
///     caller falls through to the general iterative work-stack walk.
///   - `Some(Some(ord))` — an orderable scalar leaf compared to `ord` (Int NUMERIC/signed, Bool false<true,
///     Unit always equal, Str content-lexicographic over the flattened UTF-8 bytes, BigInt/Rational by value).
///   - `Some(None)` — a scalar-position leaf that offers NO total order (Float/Float32/Bytes): the caller
///     returns this `None` (the op maps it to the unordered sentinel), matching the work-stack walk's verdict.
pub(crate) fn compare_scalar_leaf(
    desc: &Descriptor,
    a: Handle,
    b: Handle,
    shape_ix: u32,
) -> Option<Option<core::cmp::Ordering>> {
    use core::cmp::Ordering;
    let shape = resolve_shape(desc, shape_ix)?;
    Some(match shape {
        Shape::Int => Some(op_get_int(a).cmp(&op_get_int(b))),
        // A Char is SEMANTICS-int — order by the code-point integer (identical to `Int`), never a char-
        // specific collation; only its RENDER differs.
        Shape::Char => Some(op_get_int(a).cmp(&op_get_int(b))),
        Shape::Bool => Some(op_get_bool(a).cmp(&op_get_bool(b))),
        Shape::Unit => Some(Ordering::Equal),
        Shape::BigInt => Some(op_bigint_cmp(a, b).cmp(&0)),
        Shape::Rational => Some(op_rational_cmp(a, b).cmp(&0)),
        Shape::Str | Shape::Symbol => {
            // Content-lexicographic over the flattened UTF-8 bytes — a rope String is flattened first
            // (content-preserving/unobservable), then the borrowed leaf slices compare with NO `to_vec`
            // (the zero-alloc discipline `champ_eq` uses). This is the arm the string-keyed-map sort hits.
            bytes_flatten(a);
            bytes_flatten(b);
            let av = unsafe { a.node_ref() };
            let bv = unsafe { b.node_ref() };
            let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
            let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
            Some(as_.cmp(bs))
        }
        // A FLOAT orders by its CANONICAL BIT PATTERN as an UNSIGNED integer — NOT numeric order. This is the
        // element-derived deterministic order collections-and-text.md #Set Iteration Is Deterministic requires
        // (agreeing with the canonical byte form, which totally orders floats): every NaN is collapsed to the
        // one quiet NaN by `op_box_float` on construction, and +0.0/-0.0 keep their distinct bits, so the bit
        // pattern is a TOTAL order (no unordered pairs). Read the canonical bits via the width's getter and
        // compare as `u64`/`u32` — exactly the Rust backend's `__CdzF64`/`__CdzF32` wrapper order (`self.0`,
        // where `self.0 = to_bits()`), so a float Set.to-list / Map.to-list key enumerates identically on both
        // backends. By this order a NEGATIVE float (sign bit set = high bit) sorts AFTER every positive, and
        // -0.0 (0x8000…) after +0.0 (0x0). This is the ORDERING used ONLY where an order is offered (to-list
        // enumeration, a float Set/Map key sort); float `<` remains the IEEE partial order (NaN unordered),
        // decided at compile time — the compiler routes numeric `<` away from this walk.
        Shape::Float => Some(op_get_float(a).to_bits().cmp(&op_get_float(b).to_bits())),
        Shape::Float32 => Some(
            op_get_float32(a)
                .to_bits()
                .cmp(&op_get_float32(b).to_bits()),
        ),
        // Bytes has a blessed TOTAL order (§order): content-lexicographic over its UNSIGNED byte values —
        // the SAME machinery as `Shape::Str` above (both are byte leaves; a Bytes value may be a rope, so
        // flatten both to a leaf first — content-preserving/unobservable — then compare the borrowed `raw`
        // slices with NO `to_vec`, the zero-alloc discipline `champ_eq` uses). Unlike a float this order is
        // total AND composes inside a compound; the compiler's `orderable_leaf_or_compound` admits Bytes in
        // lockstep with this arm.
        Shape::Bytes => {
            bytes_flatten(a);
            bytes_flatten(b);
            let av = unsafe { a.node_ref() };
            let bv = unsafe { b.node_ref() };
            let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
            let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
            Some(as_.cmp(bs))
        }
        // Not a scalar leaf — a compound / set / map / framed root: the caller falls through to the walk.
        Shape::Tuple(_)
        | Shape::List(_)
        | Shape::Record(_)
        | Shape::Sum(_)
        | Shape::Spread(_)
        | Shape::Set(_)
        | Shape::Map(..)
        | Shape::Framed(..) => return None,
        // Named/Ref were already resolved by `resolve_shape`; a residual one is a malformed descriptor.
        Shape::Named(..) | Shape::Ref(_) => return None,
    })
}

pub(crate) fn value_cmp_shaped(
    desc: &Descriptor,
    a: Handle,
    b: Handle,
    root_shape: u32,
) -> Option<core::cmp::Ordering> {
    use core::cmp::Ordering;
    // ZERO-ALLOC SCALAR FAST PATH. A SCALAR-leaf root (Int/Bool/Unit/Str/BigInt/Rational, resolved through
    // Named/Ref) — the overwhelmingly common `value-cmp` shape AND the ONLY shape a canonically-scalar Set/Map
    // KEY or element takes — compares in place with NO work-stack `Vec`. This is what the old dedicated
    // `canonical_scalar_order` did; folding it in keeps a String-keyed `Map.to-list`/`Set.to-list` sort
    // (`sort_unstable_by` → O(N·log N) comparisons) allocation-FREE, which a `vec![…]`-per-compare would blow
    // past (~2·N·log N transient Vecs, the alloc-bench regression this restores). A COMPOUND root falls
    // through to the general iterative work-stack walk below (its per-call Vec is amortized over the whole
    // structure, not per leaf). `compare_scalar_leaf` returns `None` for a non-scalar shape (→ fall through)
    // or a genuinely non-orderable scalar-position leaf (Float/Float32/Bytes → the unordered `None` the op
    // maps to the sentinel).
    if let Some(ord) = compare_scalar_leaf(desc, a, b, root_shape) {
        return ord;
    }
    let mut work: Vec<CmpTask> = vec![CmpTask::Pair {
        a,
        b,
        shape_ix: root_shape,
    }];
    while let Some(task) = work.pop() {
        match task {
            CmpTask::LenTie { la, lb } => match la.cmp(&lb) {
                Ordering::Equal => {}
                ord => return Some(ord), // a proper prefix is Less than its extension
            },
            CmpTask::Pair { a, b, shape_ix } => {
                // Resolve `Ref`/`Named` indirections to the underlying shape (bounded by `resolve_shape`).
                let shape = resolve_shape(desc, shape_ix)?;
                match shape {
                    Shape::Int => match op_get_int(a).cmp(&op_get_int(b)) {
                        Ordering::Equal => {}
                        ord => return Some(ord),
                    },
                    // Char = semantics-int: order by the code-point integer, exactly as `Int`.
                    Shape::Char => match op_get_int(a).cmp(&op_get_int(b)) {
                        Ordering::Equal => {}
                        ord => return Some(ord),
                    },
                    Shape::Bool => match op_get_bool(a).cmp(&op_get_bool(b)) {
                        Ordering::Equal => {}
                        ord => return Some(ord),
                    },
                    Shape::Unit => {} // the unit value is a singleton — always equal
                    Shape::BigInt => match op_bigint_cmp(a, b) {
                        0 => {}
                        n if n < 0 => return Some(Ordering::Less),
                        _ => return Some(Ordering::Greater),
                    },
                    Shape::Rational => match op_rational_cmp(a, b) {
                        0 => {}
                        n if n < 0 => return Some(Ordering::Less),
                        _ => return Some(Ordering::Greater),
                    },
                    // A String is content-lexicographic over its UTF-8 bytes (== Unicode-scalar order for
                    // well-formed UTF-8, #58); a Bytes value is content-lexicographic over its UNSIGNED byte
                    // values (§order). BOTH are `raw`-byte leaves comparing identically, so they share this
                    // arm. Either may be a ROPE — flatten both to a leaf first (iterative, content-preserving/
                    // unobservable) so `raw` holds the logical bytes, then compare the borrowed slices without
                    // allocating (the same zero-alloc discipline `champ_eq` uses). Bytes composes soundly here
                    // inside a compound (its order is total), unlike a float — the compiler admits Bytes in
                    // lockstep (`orderable_leaf_or_compound`).
                    Shape::Str | Shape::Symbol | Shape::Bytes => {
                        bytes_flatten(a);
                        bytes_flatten(b);
                        let ord = {
                            let av = unsafe { a.node_ref() };
                            let bv = unsafe { b.node_ref() };
                            let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
                            let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
                            as_.cmp(bs)
                        };
                        match ord {
                            Ordering::Equal => {}
                            o => return Some(o),
                        }
                    }
                    // Non-orderable — the spec offers no total order (defensive; the compiler declines these
                    // before emitting a value-cmp call). Float/Float32 (#319 partial), Set/Map (not offered).
                    Shape::Float | Shape::Float32 | Shape::Set(_) | Shape::Map(..) => {
                        return None;
                    }
                    Shape::Tuple(elems) => {
                        let elems = elems.clone();
                        if (op_arr_len(a) as usize) < elems.len()
                            || (op_arr_len(b) as usize) < elems.len()
                        {
                            return None; // malformed vs the descriptor
                        }
                        for (i, &es) in elems.iter().enumerate().rev() {
                            work.push(CmpTask::Pair {
                                a: op_arr_get(a, i as u32),
                                b: op_arr_get(b, i as u32),
                                shape_ix: es,
                            });
                        }
                    }
                    Shape::Record(fields) => {
                        // A record's runtime rep is a `tuple` arr in the descriptor's field order (the same
                        // canonical order equality/encode use); compare field values in that order.
                        let fields: Vec<u32> = fields.iter().map(|(_, ix)| *ix).collect();
                        if (op_arr_len(a) as usize) < fields.len()
                            || (op_arr_len(b) as usize) < fields.len()
                        {
                            return None;
                        }
                        for (i, &fs) in fields.iter().enumerate().rev() {
                            work.push(CmpTask::Pair {
                                a: op_arr_get(a, i as u32),
                                b: op_arr_get(b, i as u32),
                                shape_ix: fs,
                            });
                        }
                    }
                    Shape::List(elem) => {
                        // Lexicographic: compare min(la,lb) elements; if all equal, the SHORTER list is Less
                        // (the LenTie task, pushed FIRST so it evaluates LAST — only after every prefix pair).
                        let elem = *elem;
                        let (la, lb) = (op_vec_len(a), op_vec_len(b));
                        let minl = la.min(lb);
                        work.push(CmpTask::LenTie { la, lb });
                        for i in (0..minl).rev() {
                            work.push(CmpTask::Pair {
                                a: op_vec_get(a, i),
                                b: op_vec_get(b, i),
                                shape_ix: elem,
                            });
                        }
                    }
                    Shape::Sum(variants) => {
                        // By discriminant first (the variant's index, as the canonical byte form encodes it),
                        // then by payload within the same variant. Different discriminants decide immediately.
                        //
                        // DISC of an ALL-NULLARY sum stored as an Int IMMEDIATE (SOUNDNESS #43 witness 4): a
                        // nullary variant boxes via `box-int` (enum-disc → OP_BOX_INT), and a small disc
                        // (0/1/2…) fixnum_fits, so `op_box_int` returns an IMMEDIATE int carrying the disc as
                        // its value — NOT a heap sum node. `op_sum_disc` returns 0 for ANY immediate ("a sum
                        // is never itself an immediate"), so WITHOUT this every nullary key read disc 0 → all
                        // Equal → the stable to-list sort kept insertion order → wrong enumeration ({Hi,Mid,Lo}
                        // heads Mid not Lo). So decode the disc from the immediate's int value here; a
                        // payload-carrying variant is a real heap node (`op_sum_disc` reads its stored disc,
                        // unchanged). Localized to the cmp/sort path — `op_sum_disc`'s immediate→0 contract
                        // (relied on by the render/decode callers) is untouched.
                        let variants = variants.clone();
                        let (da, db) = (sum_disc_shaped(a), sum_disc_shaped(b));
                        // DECLINE on an out-of-range disc BEFORE ordering (PR#891): `sum_disc_shaped` returns
                        // `u32::MAX` for a malformed non-int immediate under a Sum shape. The `Equal` arm below
                        // already declines via `variants.get(da)?`, but the DIFFERING-disc arm returns an
                        // Ordering directly — so a `u32::MAX` (or otherwise out-of-range) disc would order
                        // deterministically-but-wrong instead of declining. Validate BOTH discs are real
                        // variant indices first; either out of range ⇒ malformed ⇒ decline (None), matching
                        // the render path + the descriptor-walk reject-don't-miscompile contract.
                        if da as usize >= variants.len() || db as usize >= variants.len() {
                            return None;
                        }
                        match da.cmp(&db) {
                            Ordering::Equal => {
                                let (_, payload_shape) = variants.get(da as usize)?;
                                work.push(CmpTask::Pair {
                                    a: op_sum_payload(a),
                                    b: op_sum_payload(b),
                                    shape_ix: *payload_shape,
                                });
                            }
                            ord => return Some(ord),
                        }
                    }
                    Shape::Spread(elems) => {
                        // A multi-payload variant's payload is a `Spread` — a tuple arr of the boxed payloads;
                        // compare element-wise in order (reached only as a Sum payload's shape).
                        let elems = elems.clone();
                        if (op_arr_len(a) as usize) < elems.len()
                            || (op_arr_len(b) as usize) < elems.len()
                        {
                            return None;
                        }
                        for (i, &es) in elems.iter().enumerate().rev() {
                            work.push(CmpTask::Pair {
                                a: op_arr_get(a, i as u32),
                                b: op_arr_get(b, i as u32),
                                shape_ix: es,
                            });
                        }
                    }
                    // A `Framed` frame (`(: value type-node)`) wraps an INNER value shape (like `Named` but
                    // with a full type node) — transparent for ordering: compare the inner value. Not
                    // followed by `resolve_shape` (which only chases `Ref`/`Named`), so descend it here.
                    Shape::Framed(_type_node, inner) => {
                        let inner = *inner;
                        work.push(CmpTask::Pair {
                            a,
                            b,
                            shape_ix: inner,
                        });
                    }
                    // `Ref`/`Named` were resolved by `resolve_shape` above and never reach here.
                    Shape::Ref(_) | Shape::Named(..) => return None,
                }
            }
        }
    }
    Some(Ordering::Equal)
}

/// A task for the iterative `value_eq_shaped` walk — a pair of handles to compare under a shape. Simpler
/// than `CmpTask`: equality has no length-tiebreak (a length mismatch is decided immediately, in-line).
pub(crate) enum EqTask {
    Pair { a: Handle, b: Handle, shape_ix: u32 },
}

/// STRUCTURAL EQUALITY of two heap values of the same type, guided by the compiler-baked shape `desc` —
/// the equality companion of `value_cmp_shaped`. Returns `Some(true/false)`, or `None` on a malformed
/// descriptor (defensive; the compiler bakes only a well-formed one). BORROWS both operands (an inspector,
/// like `value-eq`/`value-cmp`).
///
/// Unlike `value_cmp_shaped`, EVERY leaf is compared by EQUALITY — including the ones value-cmp DECLINES for
/// ordering: a Float/Float32/Bytes leaf compares by its CANONICAL BYTE FORM (spec core-semantics §313: float
/// equality is TOTAL — NaN canonicalized to one form, ±0 distinct — even though float ORDERING is only the
/// §319 IEEE partial order; and a Bytes leaf is byte-canonical at construction). This is WHY a `List<Float>`
/// (or any list-containing-float compound) `=` needs THIS walk, not value-cmp (which declines the float leaf)
/// nor the tagless `champ_eq` (unsound for the non-shape-canonical RRB list SPINE): the walk descends the
/// list ELEMENT-WISE (shape-independent, like value-cmp's List arm) while comparing each leaf by its
/// byte-canonical form. Iterative (an explicit `EqTask` stack — wasm-safe on deep data, like
/// `champ_key_cmp`/`value_cmp_shaped`); the FIRST inequality (or length/discriminant mismatch) short-circuits.
#[allow(dead_code)] // wired in BRICK 2 (the value-eq-shaped op export + List<Float> `=` emit routing)
pub(crate) fn value_eq_shaped(
    desc: &Descriptor,
    a: Handle,
    b: Handle,
    root_shape: u32,
) -> Option<bool> {
    // Compare two byte-canonical LEAVES (Float/Float32/Bytes/String/Symbol) by their raw bytes. A rope
    // String/Bytes is flattened first (content-preserving, unobservable) so `raw` holds the logical bytes —
    // exactly the `Shape::Str` discipline in value_cmp_shaped, extended to the float/bytes leaves that
    // equality (unlike ordering) admits. A float leaf's `raw` is its canonical byte form (`op_box_float`
    // normalizes NaN + preserves ±0's sign bit), so byte-equality IS the spec's canonical-byte-form rule.
    pub(crate) fn leaf_bytes_eq(a: Handle, b: Handle) -> bool {
        bytes_flatten(a);
        bytes_flatten(b);
        let av = unsafe { a.node_ref() };
        let bv = unsafe { b.node_ref() };
        let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
        let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
        as_ == bs
    }
    let mut work: Vec<EqTask> = vec![EqTask::Pair {
        a,
        b,
        shape_ix: root_shape,
    }];
    while let Some(EqTask::Pair { a, b, shape_ix }) = work.pop() {
        let shape = resolve_shape(desc, shape_ix)?;
        match shape {
            Shape::Int => {
                if op_get_int(a) != op_get_int(b) {
                    return Some(false);
                }
            }
            // Char = semantics-int: equal iff the code-point integers are equal, exactly as `Int`.
            Shape::Char => {
                if op_get_int(a) != op_get_int(b) {
                    return Some(false);
                }
            }
            Shape::Bool => {
                if op_get_bool(a) != op_get_bool(b) {
                    return Some(false);
                }
            }
            Shape::Unit => {} // singleton — always equal
            Shape::BigInt => {
                if op_bigint_cmp(a, b) != 0 {
                    return Some(false);
                }
            }
            Shape::Rational => {
                if op_rational_cmp(a, b) != 0 {
                    return Some(false);
                }
            }
            // Byte-canonical leaves — equality by canonical raw bytes (float eq TOTAL per §313; Bytes/String
            // byte-canonical). This is the KEY difference from value_cmp_shaped, which DECLINES these.
            Shape::Float | Shape::Float32 | Shape::Bytes | Shape::Str | Shape::Symbol => {
                if !leaf_bytes_eq(a, b) {
                    return Some(false);
                }
            }
            // Set/Map are canonical-by-construction CHAMP handles — equal iff champ_eq (byte-identical), which
            // IS sound for them (order-independent canonical rep). No descriptor descent needed.
            Shape::Set(_) | Shape::Map(..) => {
                if !champ_eq(a, b) {
                    return Some(false);
                }
            }
            Shape::Tuple(elems) => {
                let elems = elems.clone();
                if (op_arr_len(a) as usize) < elems.len() || (op_arr_len(b) as usize) < elems.len()
                {
                    return None;
                }
                for (i, &es) in elems.iter().enumerate() {
                    work.push(EqTask::Pair {
                        a: op_arr_get(a, i as u32),
                        b: op_arr_get(b, i as u32),
                        shape_ix: es,
                    });
                }
            }
            Shape::Record(fields) => {
                let fields: Vec<u32> = fields.iter().map(|(_, ix)| *ix).collect();
                if (op_arr_len(a) as usize) < fields.len()
                    || (op_arr_len(b) as usize) < fields.len()
                {
                    return None;
                }
                for (i, &fs) in fields.iter().enumerate() {
                    work.push(EqTask::Pair {
                        a: op_arr_get(a, i as u32),
                        b: op_arr_get(b, i as u32),
                        shape_ix: fs,
                    });
                }
            }
            Shape::List(elem) => {
                // Element-wise, SHAPE-INDEPENDENT (the whole point — the RRB spine is not byte-canonical):
                // equal iff same length AND every element equal. A length mismatch is decided immediately.
                let elem = *elem;
                let (la, lb) = (op_vec_len(a), op_vec_len(b));
                if la != lb {
                    return Some(false);
                }
                for i in 0..la {
                    work.push(EqTask::Pair {
                        a: op_vec_get(a, i),
                        b: op_vec_get(b, i),
                        shape_ix: elem,
                    });
                }
            }
            Shape::Sum(variants) => {
                let variants = variants.clone();
                let (da, db) = (op_sum_disc(a), op_sum_disc(b));
                if da != db {
                    return Some(false); // different variants ⇒ unequal
                }
                let (_, payload_shape) = variants.get(da as usize)?;
                work.push(EqTask::Pair {
                    a: op_sum_payload(a),
                    b: op_sum_payload(b),
                    shape_ix: *payload_shape,
                });
            }
            Shape::Spread(elems) => {
                let elems = elems.clone();
                if (op_arr_len(a) as usize) < elems.len() || (op_arr_len(b) as usize) < elems.len()
                {
                    return None;
                }
                for (i, &es) in elems.iter().enumerate() {
                    work.push(EqTask::Pair {
                        a: op_arr_get(a, i as u32),
                        b: op_arr_get(b, i as u32),
                        shape_ix: es,
                    });
                }
            }
            Shape::Framed(_type_node, inner) => {
                let inner = *inner;
                work.push(EqTask::Pair {
                    a,
                    b,
                    shape_ix: inner,
                });
            }
            Shape::Ref(_) | Shape::Named(..) => return None,
        }
    }
    Some(true)
}

/// A task for the iterative post-order rebuild in `value_canonicalize_shaped`. `Visit` expands a node
/// (pushing a matching `Build*` then its children); each `Build*` pops its now-canonical children off the
/// results stack and assembles the canonical parent. A single explicit stack → no native recursion over
/// deep data (wasm-safe, like `champ_key_cmp`/`encode_value`).
pub(crate) enum CanonTask {
    Visit {
        h: Handle,
        shape_ix: u32,
        refs: u32,
    },
    /// Pop `n` canonical elements (in child order) → a fresh `arr`, then `op_vec_of_arr` → the canonical
    /// STRICT left-full RRB vec (the unique push-shape). This is what makes a concat-built list key
    /// byte-identical to a push-built one.
    BuildList {
        n: usize,
    },
    /// Pop `n` canonical elements → a fresh `arr` (the runtime rep of a tuple/record/spread).
    BuildArr {
        n: usize,
    },
    /// Pop ONE canonical payload → `op_sum_new(disc, payload)`.
    BuildSum {
        disc: u32,
    },
}

/// Build a fresh `arr` from the LAST `n` handles on `results` (in child order), which are MOVED in (each
/// is a fresh owned canonical child — no dup, no drop). Returns the arr (owned). `n == 0` → the inline unit.
pub(crate) fn canon_build_arr(results: &mut Vec<Handle>, n: usize) -> Handle {
    let start = results.len() - n;
    let arr = op_arr_alloc(n as u32);
    for (i, h) in results.drain(start..).enumerate() {
        op_arr_set(arr, i as u32, h);
    }
    arr
}

/// Drop every partially-built canonical handle on `results` and return `None` — the cleanup path when a
/// malformed descriptor / arity mismatch aborts the walk, so a decline never LEAKS the work done so far.
pub(crate) fn canon_decline(results: &mut Vec<Handle>) -> Option<Handle> {
    for h in results.drain(..) {
        op_drop(h);
    }
    None
}

/// Produce the BLESSED CANONICAL form of a heap value `a` of the type described by `desc`/`root_shape`: a
/// fresh OWNED value, byte-identical for any two values that are EQUAL as values regardless of how each was
/// constructed. The load-bearing case is a **List** (RRB vector): a concat-built and a push-built list with
/// the same elements have DIFFERENT internal shapes (relaxed interior nodes vs a strict trie), so the
/// tagless `champ_hash`/`champ_eq` byte-walk places them in different CHAMP slots — a Map/Set with such a
/// list KEY false-MISSES (collections-and-text.md §162: a key's identity is construction-INDEPENDENT). This
/// rebuilds every list to its unique strict left-full shape (via `op_vec_of_arr`), recursing through any
/// list-CONTAINING compound (a `(tuple (list…) Int)` key), so the byte-walk becomes exact.
///
/// BORROWS `a` (returns a fresh owned tree; the caller/key-site releases `a` and later drops the returned
/// temporary — the model `value-encode` uses). Scalar leaves (Int/BigInt/Rational/Bool/Unit/Float/Float32)
/// are canonical by construction → dup as-is. Str/Bytes → `bytes_flatten` (a rope's canonical flat leaf,
/// shared). Set/Map → dup as-is: a CHAMP is canonical at its OWN level; a Set-of-lists / Map-with-list-
/// values (a list buried inside a collection that is itself a key) is a rarer RESIDUAL — the same nested
/// edge the String/Bytes key story leaves — deferred to a follow-on. TOTAL: a malformed descriptor or
/// arity mismatch declines to `None` (the caller falls back to the input as-is), never traps, never leaks.
/// Iterative (an explicit `CanonTask` stack) so a deeply-nested list does not overflow the guest stack.
pub(crate) fn value_canonicalize_shaped(
    desc: &Descriptor,
    a: Handle,
    root_shape: u32,
) -> Option<Handle> {
    let mut work: Vec<CanonTask> = vec![CanonTask::Visit {
        h: a,
        shape_ix: root_shape,
        refs: 0,
    }];
    let mut results: Vec<Handle> = Vec::new();
    while let Some(task) = work.pop() {
        match task {
            CanonTask::BuildList { n } => {
                let arr = canon_build_arr(&mut results, n);
                results.push(op_vec_of_arr(arr));
            }
            CanonTask::BuildArr { n } => {
                let arr = canon_build_arr(&mut results, n);
                results.push(arr);
            }
            CanonTask::BuildSum { disc } => {
                let payload = results.pop()?;
                results.push(op_sum_new(disc, payload));
            }
            CanonTask::Visit { h, shape_ix, refs } => {
                if refs > ENCODE_REF_CYCLE_CAP {
                    return canon_decline(&mut results); // a Ref/Named chain that never reaches a node
                }
                match desc.table.get(shape_ix as usize) {
                    None => return canon_decline(&mut results),
                    // Indirections: same `h`, no node reached → count toward the cycle cap.
                    Some(Shape::Ref(target) | Shape::Named(_, target)) => {
                        work.push(CanonTask::Visit {
                            h,
                            shape_ix: *target,
                            refs: refs + 1,
                        });
                    }
                    // Scalar leaves are canonical BY CONSTRUCTION (`box_*` normalizes: BigInt sign-magnitude,
                    // Rational lowest-terms, Float NaN one byte form). Retain the borrowed handle as the fresh
                    // owned result — an immediate's `op_dup` is a no-op and owns no heap.
                    Some(
                        Shape::Int
                        | Shape::BigInt
                        | Shape::Rational
                        | Shape::Bool
                        | Shape::Char
                        | Shape::Unit
                        | Shape::Float
                        | Shape::Float32,
                    ) => {
                        op_dup(h);
                        results.push(h);
                    }
                    // A String/Bytes may be a ROPE → flatten to its canonical flat leaf (in place, content-
                    // preserving/unobservable even on a shared node), then retain. A flat leaf flattens no-op.
                    Some(Shape::Str | Shape::Symbol | Shape::Bytes) => {
                        bytes_flatten(h);
                        op_dup(h);
                        results.push(h);
                    }
                    // A Set/Map handle is canonical at its OWN level (order-independent CHAMP). Retain as-is;
                    // a list buried inside it is the documented residual (see the fn doc).
                    Some(Shape::Set(_) | Shape::Map(..)) => {
                        op_dup(h);
                        results.push(h);
                    }
                    // THE load-bearing arm: rebuild the list to its canonical strict shape. Canonicalize each
                    // element first (recurse), then `BuildList` reassembles via `op_vec_of_arr`.
                    Some(Shape::List(elem)) => {
                        let elem = *elem;
                        let len = op_vec_len(h);
                        work.push(CanonTask::BuildList { n: len as usize });
                        for i in (0..len).rev() {
                            work.push(CanonTask::Visit {
                                h: op_vec_get(h, i),
                                shape_ix: elem,
                                refs: 0,
                            });
                        }
                    }
                    // A tuple / record / multi-payload spread is an `arr` at run time; canonicalize each field
                    // in the descriptor's canonical order, then `BuildArr` reassembles the arr.
                    Some(Shape::Tuple(elems)) => {
                        let elems = elems.clone();
                        if (op_arr_len(h) as usize) < elems.len() {
                            return canon_decline(&mut results);
                        }
                        work.push(CanonTask::BuildArr { n: elems.len() });
                        for (i, &es) in elems.iter().enumerate().rev() {
                            work.push(CanonTask::Visit {
                                h: op_arr_get(h, i as u32),
                                shape_ix: es,
                                refs: 0,
                            });
                        }
                    }
                    Some(Shape::Spread(elems)) => {
                        let elems = elems.clone();
                        if (op_arr_len(h) as usize) < elems.len() {
                            return canon_decline(&mut results);
                        }
                        work.push(CanonTask::BuildArr { n: elems.len() });
                        for (i, &es) in elems.iter().enumerate().rev() {
                            work.push(CanonTask::Visit {
                                h: op_arr_get(h, i as u32),
                                shape_ix: es,
                                refs: 0,
                            });
                        }
                    }
                    Some(Shape::Record(fields)) => {
                        let field_ixs: Vec<u32> = fields.iter().map(|(_, ix)| *ix).collect();
                        if (op_arr_len(h) as usize) < field_ixs.len() {
                            return canon_decline(&mut results);
                        }
                        work.push(CanonTask::BuildArr { n: field_ixs.len() });
                        for (i, &fs) in field_ixs.iter().enumerate().rev() {
                            work.push(CanonTask::Visit {
                                h: op_arr_get(h, i as u32),
                                shape_ix: fs,
                                refs: 0,
                            });
                        }
                    }
                    // A sum: canonicalize the payload under the ACTIVE variant's shape, then rebuild the shell.
                    Some(Shape::Sum(variants)) => {
                        let variants = variants.clone();
                        let disc = op_sum_disc(h);
                        let Some((_, payload_shape)) = variants.get(disc as usize) else {
                            return canon_decline(&mut results);
                        };
                        work.push(CanonTask::BuildSum { disc });
                        work.push(CanonTask::Visit {
                            h: op_sum_payload(h),
                            shape_ix: *payload_shape,
                            refs: 0,
                        });
                    }
                    // A `(: value type-node)` frame — transparent for the VALUE: canonicalize the inner value.
                    Some(Shape::Framed(_type_node, inner)) => {
                        work.push(CanonTask::Visit {
                            h,
                            shape_ix: *inner,
                            refs: refs + 1,
                        });
                    }
                }
            }
        }
    }
    // Exactly one fully-assembled canonical root remains (a well-formed walk); anything else is malformed.
    if results.len() == 1 {
        results.pop()
    } else {
        canon_decline(&mut results)
    }
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
pub(crate) const CHAMP_LEVELS: u32 = 7;

/// Read a node's subtree size (raw offset 8). Borrows; a null/short node reads 0.
#[allow(dead_code)]
pub(crate) fn champ_size_of(node: Handle) -> u32 {
    with_node(node, 0, |n| champ_size(&n.raw))
}

// The canonical empty map: both bitmaps 0, size 0, no handles (exactly `is_empty_node`). U3's
// remove-to-empty MUST reproduce this representation so callers can recognise emptiness uniformly.
// The shared IMMORTAL empty-MAP singleton (the IMM_UNIT / empty-vec analog for maps) — lazily minted,
// rc=IMMORTAL (census-excluded), so an empty map allocates ONCE and is reused, never per-occurrence.
runtime_local! {
    static EMPTY_MAP: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

pub(crate) fn op_map_empty() -> Handle {
    // One shared immortal empty CHAMP node. An empty map is CONSTANT (no entries), so one immortal is
    // correct + census-EXCLUDED (never a leak) + free to share. SOUND: map insert gates on node_rc==1
    // (champ_insert_fbip's `mine`), and IMMORTAL != 1, so an insert onto the singleton takes the proven
    // COPY path — the shared empty is never mutated in place.
    EMPTY_MAP.with(|slot| {
        let mut e = slot.get();
        if e.0.is_null() {
            e = alloc_raw(Vec::new(), champ_header(0, 0, 0));
            op_mark_immortal(e);
            slot.set(e);
        }
        e
    })
}

/// O(1) entry count of the map. BORROWS `m` (no rc change).
#[allow(dead_code)]
pub(crate) fn op_map_size(m: Handle) -> u32 {
    champ_size_of(m)
}

/// Shared stride-aware membership descent: find `key`, returning `Some((deepest_node, base))` where
/// `base` is the entry's first-column index in that node's `handles`, or `None` on miss. BORROWS.
/// Iterative descent (a `while` loop, no recursion), mirroring `op_vec_get`. Map lookup reads the
/// value at `base+1`; set contains just checks presence.
#[allow(dead_code)]
pub(crate) fn champ_find_base(m: Handle, key: Handle, stride: usize) -> Option<(Handle, usize)> {
    champ_find_base_h(m, key, champ_hash(key), stride)
}

/// `champ_find_base` but with `key`'s hash PRECOMPUTED by the caller — so a caller that already needs
/// `champ_hash(key)` for a following insert (set-algebra ∩/∖: probe-then-insert) computes it ONCE
/// instead of paying a second full subtree hash walk (costly for string/compound keys). BORROWS.
#[allow(dead_code)]
pub(crate) fn champ_find_base_h(
    m: Handle,
    key: Handle,
    hash: u32,
    stride: usize,
) -> Option<(Handle, usize)> {
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
pub(crate) fn op_map_lookup(m: Handle, key: Handle) -> Handle {
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
pub(crate) struct Entry {
    cols: [Handle; 2],
    len: usize,
}
impl Entry {
    /// A set element (len 1).
    pub(crate) fn elem(e: Handle) -> Entry {
        Entry {
            cols: [e, Handle::NULL],
            len: 1,
        }
    }
    /// A map key/value pair (len 2).
    pub(crate) fn kv(k: Handle, v: Handle) -> Entry {
        Entry {
            cols: [k, v],
            len: 2,
        }
    }
    /// The key/element column (column 0), compared by `champ_eq`.
    pub(crate) fn key(&self) -> Handle {
        self.cols[0]
    }
    /// Column `i` (0 ≤ i < len).
    pub(crate) fn col(&self, i: usize) -> Handle {
        self.cols[i]
    }
    /// The number of columns (the insert stride: 1 for a set, 2 for a map).
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    /// Consume the entry, appending its columns onto `out` — used where the columns become part of a
    /// freshly-built node handle vector (the rare fresh-single / split / collision-append paths).
    pub(crate) fn extend_into(self, out: &mut Vec<Handle>) {
        out.extend_from_slice(&self.cols[..self.len]);
    }
    /// Consume the entry into a `Handles` built INLINE — for the fresh-single-entry node whose `handles`
    /// IS exactly this entry. An entry has ≤2 columns (= INLINE_HANDLES_CAP), so this always fits the
    /// inline arm with NO heap Vec, unlike a `Vec`-based build that `From<Vec>` would then re-inline and
    /// free (a transient alloc on every fresh CHAMP node — the common map/set build path).
    pub(crate) fn into_handles(self) -> Handles {
        Handles::inline_from(&self.cols[..self.len])
    }
    /// The entry's columns as a slice — for a caller that copies them into node storage BY VALUE
    /// (handles are `Copy`, so this relocates them without dup/drop, exactly like moving the old Vec's
    /// elements). The caller is responsible for consuming the entry exactly once overall.
    pub(crate) fn cols(&self) -> &[Handle] {
        &self.cols[..self.len]
    }
}

/// Append `entry`'s columns onto `out` by value (handles are `Copy`). Used at the collision-node
/// splice site, which conditionally splices at one of two positions; the caller `drop`s the entry once
/// after the single splice actually runs, so this borrows rather than consumes.
pub(crate) fn entry_splice(out: &mut Vec<Handle>, entry: &Entry) {
    out.extend_from_slice(entry.cols());
}

/// Build an `Entry` from the `stride` columns of `handles` starting at `base` — the STORED entry a
/// SPLIT folds together with the newcomer. `dup` ⇒ retain a reference to each column (the copy path,
/// where the consumed node still owns its copy); `!dup` ⇒ relocate the columns (the FBIP path, where
/// the node's handle vector was already taken and these references move out).
pub(crate) fn stored_entry_from(
    handles: &[Handle],
    base: usize,
    stride: usize,
    dup: bool,
) -> Entry {
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
pub(crate) const SLOTS_CAP: usize = CHAMP_LEVELS as usize + 2; // 7 normal levels + collision frame + margin
pub(crate) struct Slots {
    buf: [u32; SLOTS_CAP],
    len: usize,
}
impl Slots {
    pub(crate) fn new() -> Slots {
        Slots {
            buf: [0; SLOTS_CAP],
            len: 0,
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    pub(crate) fn push(&mut self, v: u32) {
        if self.len >= SLOTS_CAP {
            trap_oob(); // cursor deeper than the trie permits — a compiler-invariant violation
        }
        self.buf[self.len] = v;
        self.len += 1;
    }
    pub(crate) fn pop(&mut self) {
        // Mirrors `Vec::pop`'s use here (the return value is never read) — just shrink.
        if self.len > 0 {
            self.len -= 1;
        }
    }
    /// The slot values in push order — for encoding into a cursor's raw header.
    pub(crate) fn as_slice(&self) -> &[u32] {
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
pub(crate) fn merge_two_entries(e1: Entry, h1: u32, e2: Entry, h2: u32, level: u32) -> Handle {
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
        // Same slot: nest one subnode a level deeper. Arity-1 — inline the single subnode handle (no
        // transient `vec![sub]` heap Vec that `From<Vec>` would re-inline + free).
        let sub = merge_two_entries(e1, h1, e2, h2, level + 1);
        alloc_raw(Handles::inline_from(&[sub]), champ_header(0, 1 << i1, 2))
    } else if i1 < i2 {
        // Two entries in different slots of a fresh 2-entry node, columns in ascending-slot order.
        // For a SET (stride 1) that is 2 handles = inline-eligible: `merge_entry_pair` builds them
        // INLINE (no transient heap Vec that `From<Vec>` would re-inline + free — this SPLIT fires on
        // every set-insert into an occupied slot). A MAP (stride 2) is 4 handles → heap, as before.
        alloc_raw(
            merge_entry_pair(&e1, &e2),
            champ_header((1 << i1) | (1 << i2), 0, 2),
        )
    } else {
        alloc_raw(
            merge_entry_pair(&e2, &e1),
            champ_header((1 << i2) | (1 << i1), 0, 2),
        )
    }
}

/// Build the `handles` for a fresh 2-entry CHAMP node from two entries in the given order, INLINE when
/// the total column count fits (a SET's two 1-column entries = 2 ≤ INLINE_HANDLES_CAP), else on the
/// heap (a MAP's two 2-column entries = 4). Consumes neither (handles are `Copy`); the caller has
/// already relocated ownership of the entries' columns into the returned node.
pub(crate) fn merge_entry_pair(first: &Entry, second: &Entry) -> Handles {
    let total = first.len() + second.len();
    if total <= INLINE_HANDLES_CAP {
        let mut buf = [Handle::NULL; INLINE_HANDLES_CAP];
        buf[..first.len()].copy_from_slice(first.cols());
        buf[first.len()..total].copy_from_slice(second.cols());
        Handles::Inline {
            buf,
            len: total as u8,
        }
    } else {
        let mut hs = Vec::with_capacity(total);
        hs.extend_from_slice(first.cols());
        hs.extend_from_slice(second.cols());
        Handles::Heap(hs)
    }
}

/// Insert `entry` into a collision node (both bitmaps 0, `handles` nonempty). CONSUMES `node` and
/// `entry`. Overwrite (key present) keeps the stored key + takes incoming value columns, dropping
/// the incoming duplicate key; otherwise the entry is appended. Path-copied. Returns
/// `(new_node, size_delta)` where `size_delta` is 0 (overwrite) or 1 (new key) — so the caller
/// propagates the size change WITHOUT a `champ_size_of` re-read of the child subtree.
#[allow(dead_code)]
pub(crate) fn collision_insert(
    node: Handle,
    handles: Vec<Handle>,
    entry: Entry,
    stride: usize,
) -> (Handle, u32) {
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
            (new, 0) // overwrite: size unchanged
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
            (new, 1) // new key: size + 1
        }
    }
}

/// The per-node recursive insert core. CONSUMES `node` and `entry` (`entry[0]` = key, len = stride);
/// returns `(new_node, size_delta)` where `size_delta` is 0 (an existing key was overwritten) or 1 (a
/// new key was added). Bounded recursion (≤ `CHAMP_LEVELS`). Always path-copies. Returning the delta
/// lets the DESCEND branch set the parent's size header WITHOUT two `champ_size_of` subtree re-reads.
#[allow(dead_code)]
pub(crate) fn champ_insert_node(
    node: Handle,
    entry: Entry,
    hash: u32,
    level: u32,
    stride: usize,
) -> (Handle, u32) {
    let key = entry.key();
    // Read only the HEADER + arity upfront — NOT a clone of `handles`. The old code cloned the whole
    // handle vector here even on the SPLIT/EMPTY/collision branches that only ever READ it by index and
    // build a fresh, differently-sized result — a wasted Vec alloc + O(arity) copy on every path-copied
    // node. Now the OVERWRITE/DESCEND branches (which REUSE a full-length copy as their result) clone at
    // their own branch, and the growth branches read via a borrow. `arity` gates the empty/collision test.
    let (datamap, nodemap, size, arity) = with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
        (
            champ_datamap(&n.raw),
            champ_nodemap(&n.raw),
            champ_size(&n.raw),
            n.handles.len(),
        )
    });

    // Empty node (fresh single entry) or collision node.
    if datamap == 0 && nodemap == 0 {
        if arity == 0 {
            let i = level_index(hash, level);
            let new = alloc_raw(entry.into_handles(), champ_header(1 << i, 0, 1)); // entry owned (inline handles, no transient Vec)
            op_drop(node);
            return (new, 1); // fresh single entry: a new key
        }
        // Collision node — needs an owned copy of the entries (the helper appends + rebuilds).
        let handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
        return collision_insert(node, handles, entry, stride); // returns (node, delta)
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
            let mut new_handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
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
            return (new, 0); // overwrite: size unchanged
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
            // Splice the subnodes directly into `nh` with `sub` at `new_sidx` — no transient `subs` Vec
            // (dup each carried subnode; `sub` is already owned from `merge_two_entries`).
            for s in 0..new_sidx {
                let c = n.handles[subbase + s];
                op_dup(c);
                nh.push(c);
            }
            nh.push(sub);
            for s in new_sidx..scount {
                let c = n.handles[subbase + s];
                op_dup(c);
                nh.push(c);
            }
            nh
        });
        let new = alloc_raw(
            new_handles,
            champ_header(new_datamap, new_nodemap, size + 1),
        );
        op_drop(node);
        return (new, 1); // split adds the new key
    }

    if nodemap & bit != 0 {
        // DESCEND into the subnode. Arity is unchanged, so the result reuses a full-length copy of the
        // node's handles — clone ONCE here (at the branch that needs it) and mutate the one child slot.
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = champ_handle_at(node, subbase + sidx);
        op_dup(child);
        // The recursion RETURNS the size delta (0 overwrite / 1 new key) directly — no `champ_size_of`
        // re-read of the child subtree before and after (two 12-byte-header node reads per level, a
        // measurable ~5% of insert in profiling).
        let (new_child, delta) = champ_insert_node(child, entry, hash, level + 1, stride);
        // Swap the one child slot to `new_child` (the recursion already consumed the old `child` ref via
        // the op_dup above) and dup each KEPT handle so `new` owns its own references.
        let mut new_handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
        for (idx, slot) in new_handles.iter_mut().enumerate() {
            if idx == subbase + sidx {
                *slot = new_child; // owned; old child ref was consumed by the recursion
            } else {
                op_dup(*slot); // kept handle: new node needs its own reference
            }
        }
        let new = alloc_raw(new_handles, champ_header(datamap, nodemap, size + delta));
        op_drop(node);
        return (new, delta); // propagate the child's delta up
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
    (new, 1) // empty slot filled: a new key
}

// ─── FBIP (Functional But In-Place) rc==1 shell reuse for CHAMP insert/remove (U5) ────────────
// When the touched CHAMP spine is UNIQUELY owned we REUSE each node's shell (mutate its handles/raw
// in place) instead of alloc-new + drop-old. Observationally IDENTICAL to the path-copy core, and
// canonical-shape-identical (the in-place builders mirror the copy path's ordering byte-for-byte).
//
// CRITICAL: ALIASING SAFETY (a violation silently corrupts a shared persistent map/set). `mine` = "this node
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
pub(crate) fn champ_become_hdr(
    node: Handle,
    handles: impl Into<Handles>,
    datamap: u32,
    nodemap: u32,
    size: u32,
) -> Handle {
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles = handles.into();
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
pub(crate) fn champ_take_handles(node: Handle) -> Handles {
    match unsafe { node.node_mut() } {
        Some(n) => n.handles.take(),
        None => Handles::new(),
    }
}

/// Write a single child slot AND patch the `size` header field of a uniquely-owned (`rc == 1`) CHAMP
/// node IN PLACE — the zero-allocation path for a remove whose subnode kept its arity (only one child
/// pointer changes and the subtree count drops by one; datamap/nodemap are unchanged). SAFETY: caller
/// verified `node_rc(node) == 1` and `slot < handles.len()`, `raw.len() == CHAMP_HEADER_SIZE`.
pub(crate) fn champ_set_child_and_size_inplace(
    node: Handle,
    slot: usize,
    child: Handle,
    size: u32,
) {
    if let Some(n) = unsafe { node.node_mut() } {
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
/// `node` and `entry` (`entry[0]` = key, len = `stride`). See the safety note above. Returns
/// `(new_node, size_delta)` — 0 (overwrite) or 1 (new key) — so the DESCEND branch propagates the size
/// change WITHOUT two `champ_size_of` subtree re-reads per level (a measurable insert cost, profiled).
pub(crate) fn champ_insert_fbip(
    node: Handle,
    entry: Entry,
    hash: u32,
    level: u32,
    stride: usize,
    mine: bool,
) -> (Handle, u32) {
    if !mine {
        return champ_insert_node(node, entry, hash, level, stride); // shared: proven copy path
    }
    let key = entry.key();
    // Read the header + arity WITHOUT cloning `handles` (see the take below).
    let (datamap, nodemap, size, arity) = with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
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
            return (
                champ_become_hdr(node, entry.into_handles(), 1 << i, 0, 1),
                1,
            ); // fresh: new key (inline handles, no transient Vec)
        }
        // Collision node (full 32-bit hash clash — rare): path-copy via the proven helper, which
        // `op_drop`s `node` and so needs its child references intact — clone rather than take here.
        let handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
        return collision_insert(node, handles, entry, stride); // returns (node, delta)
    }

    // Normal (bitmap) node on a UNIQUE spine: TAKE its handle vector instead of cloning it. `node` is
    // `rc == 1` (the `mine` gate + monotone-false descent), so no other reference exists; the take is
    // a pointer swap (zero alloc, vs the clone's O(arity) copy on every spine node, every level). Every
    // path below rebuilds a fresh `new_handles` and `champ_become_hdr(node, …)` REINSTALLS it before this
    // function returns, so `node` is never observed in the transient empty state (single-threaded).
    // `mut` because the arity-preserving branches (OVERWRITE, DESCEND) mutate a slot in place and
    // reinstall this same vector rather than allocating a fresh one.
    let mut handles = match unsafe { node.node_mut() } {
        Some(n) => n.handles.take(),
        None => Handles::new(),
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
                handles.set(base + t, entry.col(t)); // incoming value column (owned)
            }
            op_drop(entry.key()); // incoming duplicate key unused
            return (champ_become_hdr(node, handles, datamap, nodemap, size), 0); // overwrite
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
        handles.drain_range(base, stride);
        handles.insert(stride * (dcount - 1) + new_sidx, sub);
        return (
            champ_become_hdr(node, handles, new_datamap, new_nodemap, size + 1),
            1,
        ); // split: new key
    }

    if nodemap & bit != 0 {
        // DESCEND. Read the child's rc to decide `child_mine`; the recursion consumes the one reference
        // we pass and RETURNS `(new_child, delta)`. Taking the delta from the return replaces the two
        // `champ_size_of(child)` subtree re-reads (before + after) the old code did per level — a
        // measurable ~5% of insert in profiling, for a value the recursion already knows.
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = handles[subbase + sidx];
        let child_mine = node_rc(child) == 1;
        let (new_child, delta) =
            champ_insert_fbip(child, entry, hash, level + 1, stride, child_mine);
        // Arity unchanged — swap the one child slot in the taken `handles` IN PLACE and reinstall,
        // rather than rebuilding a fresh Vec (saves one alloc per descended level, the common path).
        // The recursion CONSUMED `child` (the reference at this slot); writing `new_child` here is a
        // no-op when it reused the shell (`new_child == child`) and installs the fresh node otherwise.
        handles.set(subbase + sidx, new_child);
        return (
            champ_become_hdr(node, handles, datamap, nodemap, size + delta),
            delta,
        );
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
    (
        champ_become_hdr(node, handles, new_datamap, nodemap, size + 1),
        1,
    ) // empty slot filled: new key
}

/// Insert `key => val`, returning the new map. CONSUMES `m`, `key`, `val`. Inserting an existing key
/// overwrites its value (size unchanged); a new key increments size. Persistent: to keep the old
/// map, `op_dup` it before inserting. FBIP: when `m` is uniquely owned (`rc == 1`) the touched spine
/// is refit in place; a shared map (`rc > 1`) path-copies (the old version stays byte-identical).
#[allow(dead_code)]
pub(crate) fn op_map_insert(m: Handle, key: Handle, val: Handle) -> Handle {
    let hash = champ_hash(key);
    let mine = node_rc(m) == 1;
    champ_insert_fbip(m, Entry::kv(key, val), hash, 0, MAP_STRIDE, mine).0 // discard the size delta
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
pub(crate) fn collapse_candidate(node: Handle, stride: usize) -> Option<Entry> {
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
pub(crate) fn champ_remove_node(
    node: Handle,
    key: Handle,
    hash: u32,
    level: u32,
    stride: usize,
) -> (Handle, bool) {
    // Read only the HEADER + arity upfront — NOT a clone of `handles`. The old code cloned the whole
    // handle vector here even on the common ABSENT-key early-returns (`(node, false)`) and on the
    // fresh-shorter-result branches (found-entry drop, collapse) that only READ it by index — a wasted
    // Vec alloc + O(arity) copy on every path-copied node. Branches that reuse a full-length copy as the
    // result (DESCEND non-collapse) clone at their own branch; the rest borrow-and-build / return early.
    let (datamap, nodemap, size, arity) = with_node(node, (0u32, 0u32, 0u32, 0usize), |n| {
        (
            champ_datamap(&n.raw),
            champ_nodemap(&n.raw),
            champ_size(&n.raw),
            n.handles.len(),
        )
    });

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
            let new = alloc_raw(
                new_handles,
                champ_header(new_datamap, new_nodemap, size - 1),
            );
            op_drop(node);
            return (new, true);
        }
        // Subnode still holds ≥2 entries: keep it, just swap in the rebuilt child. Arity unchanged, so
        // CLONE the handle vector ONCE and use it AS the result — mutate the one child slot, dup the
        // rest — rather than reading one vector and building a second.
        let mut new_handles = with_node(node, Vec::new(), |n| n.handles.to_vec());
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
pub(crate) fn champ_remove_fbip(
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
        (
            champ_datamap(&n.raw),
            champ_nodemap(&n.raw),
            champ_size(&n.raw),
            n.handles.len(),
        )
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
        for t in 0..stride {
            op_drop(handles[j + t]); // removed entry columns: release the node's reference
        }
        handles.drain_range(j, stride); // shift the remaining collision entries left, in place
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
        for t in 0..stride {
            op_drop(handles[base + t]); // removed entry columns: release the node's references
        }
        handles.drain_range(base, stride); // shift the remaining entries + subnodes left, in place
        return (
            champ_become_hdr(node, handles, new_datamap, nodemap, size - 1),
            true,
        );
    }

    if nodemap & bit != 0 {
        let sidx = subnode_index_for_slot(nodemap, i) as usize;
        let child = champ_handle_at(node, subbase + sidx); // borrow the child slot, no clone
        let child_mine = node_rc(child) == 1;
        let (new_child, removed) =
            champ_remove_fbip(child, key, hash, level + 1, stride, child_mine);
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
            handles.drain_range(subbase + sidx, 1); // the collapsed subnode handle leaves (already dropped)
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
pub(crate) fn op_map_remove(m: Handle, key: Handle) -> Handle {
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

pub(crate) const CURSOR_LIVE: u32 = 0;
pub(crate) const CURSOR_EXHAUSTED: u32 = 1;
/// Handles-per-entry: map stores `[k,v]` (2), set stores `[e]` (1).
pub(crate) const MAP_STRIDE: usize = 2;
/// A set is CHAMP minus the value column — a PRIMITIVE collection (not `Map<T,Unit>`), stride 1.
pub(crate) const SET_STRIDE: usize = 1;

/// The `i`-th subnode of a node under the given entry stride, or NULL (benign).
#[allow(dead_code)]
pub(crate) fn champ_subnode_at(node: Handle, slot: u32, stride: usize) -> Handle {
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
pub(crate) fn champ_handle_at(node: Handle, idx: usize) -> Handle {
    with_node(node, Handle::NULL, |n| {
        n.handles.get(idx).copied().unwrap_or(Handle::NULL)
    })
}

/// From `node`, descend to the LEFTMOST (in-order first) entry, appending a `(node, slot)` frame at
/// each level. `frames`/`slots` receive BORROWED node pointers (the caller dups them for ownership).
/// `node` MUST be non-empty (callers exclude the empty root); subnodes are ≥2 entries by invariant,
/// so this always terminates at an inline entry or a collision frame.
#[allow(dead_code)]
pub(crate) fn champ_descend_leftmost(
    node: Handle,
    frames: &mut Vec<Handle>,
    slots: &mut Slots,
    stride: usize,
) {
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
pub(crate) fn champ_advance_fbip(
    frames: &mut Vec<Handle>,
    slots: &mut Slots,
    stride: usize,
) -> bool {
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
pub(crate) fn champ_descend_leftmost_dup(
    node: Handle,
    frames: &mut Vec<Handle>,
    slots: &mut Slots,
    stride: usize,
) {
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
pub(crate) fn champ_advance(frames: &mut Vec<Handle>, slots: &mut Slots, stride: usize) -> bool {
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
pub(crate) fn champ_make_cursor(frames: Vec<Handle>, slots: Slots, state: u32) -> Handle {
    let mut raw = Vec::with_capacity(4 * (1 + slots.len()));
    raw.extend_from_slice(&state.to_le_bytes());
    for s in slots.as_slice() {
        raw.extend_from_slice(&s.to_le_bytes());
    }
    // Keep the frame stack on the HEAP arm (see `from_vec_heap`): cursor frames are push/popped as a Vec
    // by `champ_advance_fbip` and moved out by `champ_cursor_take`; inlining a shallow cursor would force
    // a Vec re-materialize every advance step (regresses iterate).
    alloc_raw(Handles::from_vec_heap(frames), Raw::from(raw))
}

/// Read a cursor into `(state, frames, slots)`. `frames` are BORROWED pointer copies (owned by the
/// cursor); `slots.len() == frames.len()`.
#[allow(dead_code)]
pub(crate) fn champ_cursor_read(cur: Handle) -> (u32, Vec<Handle>, Slots) {
    with_node(cur, (CURSOR_EXHAUSTED, Vec::new(), Slots::new()), |n| {
        let state = read_u32_at(&n.raw, 0);
        let frames = n.handles.to_vec();
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
pub(crate) fn champ_cursor_take(cur: Handle) -> (u32, Vec<Handle>, Slots) {
    match unsafe { cur.node_mut() } {
        Some(n) => {
            let state = read_u32_at(&n.raw, 0);
            let frames = n.handles.take().into_vec(); // cursor frames need Vec semantics (heap arm)
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
pub(crate) fn champ_cursor_current(cur: Handle, stride: usize) -> Option<(Handle, usize)> {
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
pub(crate) fn op_map_iter(m: Handle) -> Handle {
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
// CRITICAL: ALIASING SAFETY: gate on `node_rc(cur) == 1`. A forked/peeked/teed cursor (rc>1) MUST take the
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
pub(crate) fn champ_become_cursor(
    cur: Handle,
    frames: Vec<Handle>,
    slots: Slots,
    state: u32,
) -> Handle {
    if let Some(n) = unsafe { cur.node_mut() } {
        // Reuse the cursor's EXISTING `raw` allocation (clear keeps its capacity) instead of allocating
        // a fresh Vec — the cursor is rc==1, and its raw already held a `[state]slots…` of comparable
        // size, so the re-extend rarely reallocates. Saves one Vec allocation per advance step.
        //
        // Pack `[state][slot0][slot1]…` into a single STACK buffer and write it with ONE
        // `extend_from_slice`, rather than a per-slot `extend_from_slice` loop (up to 9 calls, each
        // re-checking inline/heap capacity). This advance runs once PER WALKED ELEMENT and showed up as
        // `champ_become_cursor` + `Raw::extend_from_slice` ~5% of the set-algebra profile.
        let sl = slots.as_slice();
        let mut buf = [0u8; 4 * (SLOTS_CAP + 1)]; // state + up to SLOTS_CAP slots; never overflows
        buf[0..4].copy_from_slice(&state.to_le_bytes());
        for (i, &s) in sl.iter().enumerate() {
            buf[4 + 4 * i..8 + 4 * i].copy_from_slice(&s.to_le_bytes());
        }
        let total = 4 * (sl.len() + 1);
        n.raw.clear();
        n.raw.extend_from_slice(&buf[..total]);
        n.handles = Handles::from_vec_heap(frames); // keep the frame stack on the heap arm (see above)
    }
    cur
}

/// FBIP in-place advance of a UNIQUELY-OWNED (`rc == 1`) cursor. Reuses `champ_advance` verbatim for
/// the traversal (identical order + exhausted-signal to the copy path), then applies ONLY the frame-ref
/// delta and refits `cur`'s shell in place. Returns `cur`. Stride selects map (2) vs set (1).
pub(crate) fn champ_cursor_next_fbip(cur: Handle, stride: usize) -> Handle {
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
pub(crate) fn op_map_iter_next(cur: Handle) -> Handle {
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
pub(crate) fn op_map_iter_key(cur: Handle) -> Handle {
    match champ_cursor_current(cur, MAP_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base),
        None => Handle::NULL,
    }
}

/// The current value (paired with `op_map_iter_key`; no per-step pair allocation). BORROWS.
#[allow(dead_code)]
pub(crate) fn op_map_iter_val(cur: Handle) -> Handle {
    match champ_cursor_current(cur, MAP_STRIDE) {
        Some((node, base)) => champ_handle_at(node, base + 1),
        None => Handle::NULL,
    }
}

/// `map-merge` — merge two persistent CHAMP maps, LAST-WRITER-WINS: `b`'s entries OVERWRITE `a`'s on a
/// key conflict (`b` is the "last writer"). CONSUMES both `a` and `b`; returns the merged map. This is
/// the runtime primitive behind `Map.union` and the map arm of value-position spread `#map((= k v) (.. m))`
/// (the caller picks which operand is `b`/last so the spread's winner matches its surface order).
///
/// IMPLEMENTATION: iterate `b`'s entries with the shared CHAMP cursor and `op_map_insert` each into `a`.
/// `op_map_insert` OVERWRITES a duplicate key (canonical CHAMP insert), so `b` wins; it also CONSUMES its
/// key+val, so each cursor-BORROWED `(k, v)` is `dup`'d first. `a` flows through as the accumulator —
/// FBIP-refit in place while uniquely owned, else path-copied by insert (also the a==b self-merge safety:
/// the cursor holds dup'd frame refs into the map, so insert sees rc>1 and path-copies rather than mutating
/// a node the cursor still walks). O(|b| · log|a∪b|); a structural node-merge (share unchanged subtrees)
/// is a later perf optimization. Empty is the identity on both sides (empty `b` → cursor yields nothing,
/// `a` returned; empty `a` → `b`'s entries re-inserted into empty = `b`). rc-balanced: `b`'s surviving
/// entries are `dup`'d into `acc` before `b` is dropped, so no leak and no double-free.
///
/// Wired via the `map-merge` WIT export (op 98) + the `Guest::map_merge` impl; the `Core::MapMerge`
/// variant + backend arms + `Map.union` prelude ride the same coordinated hash-bump flag-day.
pub(crate) fn op_map_merge(a: Handle, b: Handle) -> Handle {
    let mut acc = a;
    let mut cur = op_map_iter(b); // BORROWS b (dups the focused descent frames into the cursor)
    loop {
        let k = op_map_iter_key(cur); // BORROW; NULL is the exhausted done-signal
        if k == Handle::NULL {
            break;
        }
        let v = op_map_iter_val(cur); // BORROW (paired with the key)
        // `op_map_insert` CONSUMES key + val; `k`/`v` are borrowed from `b`'s nodes → retain a fresh
        // reference for each so `b`'s originals stay live until `b` itself is dropped below.
        op_dup(k);
        op_dup(v);
        acc = op_map_insert(acc, k, v); // overwrites the dup'd key → `b` wins (last-writer)
        cur = op_map_iter_next(cur); // consumes `cur`, returns the advanced (or exhausted) cursor
    }
    op_drop(cur); // release the exhausted cursor (its frame refs into `b`)
    op_drop(b); // consume `b`: its kept entries were dup'd into `acc`, its spine frees
    acc
}

// ─── CHAMP persistent SET (CHAMP minus the value column, stride 1) ───────────────────────
// A set is a PRIMITIVE collection, NOT `Map<T, Unit>`: entries are ONE handle. Every op is a thin
// `SET_STRIDE` wrapper over the SAME shared trie core the map uses (`champ_insert_node`,
// `champ_remove_node`, `champ_find_base`, and the cursor walkers), so there is a single code path to
// trust. The node shape is identical to a map's (bitmaps + size + handles); only the stride at the
// use-site differs — and the compiler picks the op family statically, so a set node is only ever
// touched with stride 1.

// The canonical empty set — byte-identical to the empty map (`alloc_raw(vec![], champ_header(0,0,0))`);
// the collection kind is compile-time knowledge, not a runtime tag.
// The shared IMMORTAL empty-SET singleton (per-type, mirrors EMPTY_MAP). Separate from EMPTY_MAP for
// type-clarity + zero cross-type aliasing, though an empty set + empty map are structurally identical.
runtime_local! {
    static EMPTY_SET: core::cell::Cell<Handle> = core::cell::Cell::new(Handle::NULL);
}

pub(crate) fn op_set_empty() -> Handle {
    // One shared immortal empty CHAMP node (see op_map_empty). SOUND: set insert gates on node_rc==1
    // and IMMORTAL != 1, so an insert path-copies off the singleton — never mutated in place.
    EMPTY_SET.with(|slot| {
        let mut e = slot.get();
        if e.0.is_null() {
            e = alloc_raw(Vec::new(), champ_header(0, 0, 0));
            op_mark_immortal(e);
            slot.set(e);
        }
        e
    })
}

/// O(1) element count. BORROWS `s`.
#[allow(dead_code)]
pub(crate) fn op_set_size(s: Handle) -> u32 {
    champ_size_of(s)
}

/// Total membership predicate — NEVER traps. BORROWS both `s` and `elem`. The only bool-returning
/// CHAMP op: the shared descent, returning presence instead of a value handle.
#[allow(dead_code)]
pub(crate) fn op_set_contains(s: Handle, elem: Handle) -> bool {
    champ_find_base(s, elem, SET_STRIDE).is_some()
}

/// `op_set_contains` with `elem`'s hash PRECOMPUTED — for a caller (set ∩/∖) that will also insert the
/// same element and so hashes it once, using this for the membership probe and the same hash for the
/// insert instead of re-walking the element twice. BORROWS both.
pub(crate) fn set_contains_h(s: Handle, elem: Handle, hash: u32) -> bool {
    champ_find_base_h(s, elem, hash, SET_STRIDE).is_some()
}

/// Insert `elem`, returning the new set. CONSUMES `s`, `elem`. Idempotent: inserting an existing
/// element leaves size unchanged and drops the incoming duplicate (the shared OVERWRITE rule with
/// no value columns keeps the stored element and drops the newcomer). Persistent: `op_dup` `s` first
/// to keep it.
#[allow(dead_code)]
pub(crate) fn op_set_insert(s: Handle, elem: Handle) -> Handle {
    set_insert_h(s, elem, champ_hash(elem))
}

/// `op_set_insert` with `elem`'s hash PRECOMPUTED — lets the set-algebra ops reuse the one hash they
/// computed for the membership probe (or the walk) instead of re-hashing. CONSUMES `s` and `elem`.
pub(crate) fn set_insert_h(s: Handle, elem: Handle, hash: u32) -> Handle {
    let mine = node_rc(s) == 1;
    // SET_STRIDE (1) routes through the SAME FBIP core as the map — one careful change covers both.
    champ_insert_fbip(s, Entry::elem(elem), hash, 0, SET_STRIDE, mine).0 // discard the size delta
}

/// Remove `elem`, returning the new set (canonical empty if the last element is removed). CONSUMES
/// `s`, BORROWS `elem`. Absent element ⇒ no-op returning `s` unchanged, no leak. FBIP: uniquely-owned
/// set refits in place; a shared set path-copies (old version byte-identical).
#[allow(dead_code)]
pub(crate) fn op_set_remove(s: Handle, elem: Handle) -> Handle {
    set_remove_h(s, elem, champ_hash(elem))
}

/// `op_set_remove` with `elem`'s hash PRECOMPUTED — lets set-difference (remove-from-a form) hash each
/// `b`-element once for its removal instead of re-hashing. CONSUMES `s`, BORROWS `elem`.
pub(crate) fn set_remove_h(s: Handle, elem: Handle, hash: u32) -> Handle {
    let mine = node_rc(s) == 1;
    let (new, _removed) = champ_remove_fbip(s, elem, hash, 0, SET_STRIDE, mine);
    new
}

/// A cursor over `s` at the first element in walk order (exhausted if `s` is empty). BORROWS `s`
/// (dups the frames it captures). Same cursor representation as the map's.
#[allow(dead_code)]
pub(crate) fn op_set_iter(s: Handle) -> Handle {
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
pub(crate) fn op_set_iter_next(cur: Handle) -> Handle {
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
pub(crate) fn op_set_iter_elem(cur: Handle) -> Handle {
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
pub(crate) fn op_set_union(a: Handle, b: Handle) -> Handle {
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
pub(crate) fn op_set_intersection(a: Handle, b: Handle) -> Handle {
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
pub(crate) fn op_set_difference(a: Handle, b: Handle) -> Handle {
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

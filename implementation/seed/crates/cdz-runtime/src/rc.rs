//! Reference-count calling convention (Perceus) and reuse/FBIP
//!
//! Perceus RC discipline + reset/reuse-aware constructors.

use super::*;

// ─── Reference-count calling convention (Perceus) ───────────────────────────────────────
// Written as `Handle`-typed core ops so the whole RC discipline is developed and tested natively,
// against real node pointers, before the `u32` wasm boundary ever sees it. The compiler emits `drop`
// where a heap value is released (a dead heap binding, and the resource destructor), so a compound's
// storage is reclaimed; it does NOT yet emit `dup` (the current escape/return paths transfer ownership
// rather than share), so `dup`'s call sites arrive when a construct first shares a handle.

/// `dup` — a new reference to `h` is being retained: increment its refcount. Null is a no-op.
/// The reserved refcount sentinel of an IMMORTAL heap node (a build-once static held by a module global for
/// the whole instance). `dup`/`drop` are NO-OPS on it (it is never retained or freed), and it is excluded
/// from the live-objects census — an immortal is not a leak, exactly like the inline `IMM_UNIT`. `u32::MAX`
/// is safe as the sentinel: it is unreachable by real refcounting (it would require 4 billion live dups),
/// and being `!= 1` it makes every FBIP `rc == 1` in-place path conservatively path-copy, so a shared
/// immortal is never mutated. Set by `op_mark_immortal`; checked by `op_dup`/`op_drop`.
pub(crate) const IMMORTAL: u32 = u32::MAX;

/// `mark-immortal(handle)` (heap index 95) — convert a freshly-built heap node into an IMMORTAL one (see
/// [`IMMORTAL`]): its refcount becomes the sentinel so `dup`/`drop` no-op on it and it leaves the census.
/// The node was already counted at `alloc`, so converting it DECREMENTS the census (debug counter) to net it
/// to zero. Idempotent (a re-mark does not double-decrement). An immediate has no node and is returned
/// unchanged (already census-free + rc-noop). GENERAL over any heap node; returns the same handle.
pub(crate) fn op_mark_immortal(h: Handle) -> Handle {
    if is_immediate(h) {
        return h;
    }
    if let Some(node) = unsafe { h.node_mut() }
        && node.rc != IMMORTAL
    {
        #[cfg(any(test, feature = "debug-counters"))]
        let before = node.rc;
        node.rc = IMMORTAL;
        #[cfg(any(test, feature = "debug-counters"))]
        {
            LIVE_NODES.with(|n| n.set(n.get() - 1));
            // rc-trace: a node LEAVING the census as immortal — record it so the leak summary excludes it
            // (census-exit, not a leak; not a freed drop). `rc_after` = 0 marks the census-exit.
            rc_trace_push(
                RC_TRACE_MARK_IMMORTAL,
                node.node_id,
                rc_struct_tag(node),
                before,
                0,
                false,
                RC_TRACE_NO_PARENT,
            );
        }
    }
    h
}

/// `mark-immortal-deep(handle)` (heap index 96) — the TRANSITIVE [`op_mark_immortal`]: mark the root node
/// AND every node reachable through child handles IMMORTAL. For a build-once static whose value is a
/// MULTI-NODE heap structure with no compile-time per-node handle — a `>32` RRB list (interior + leaf
/// nodes) or a CHAMP map (interior nodes + `[k,v]` data entries). The walk is over `node.handles` — the
/// SAME child set `op_drop`'s free-cascade scans — so a map's key+value handles, a list's element handles,
/// and any nested compound payloads are ALL marked, not just the spine (else the payloads would stay
/// mortal and leak, or be freed under the immortal). ITERATIVE (an explicit worklist, no recursion) so a
/// deep RRB trie cannot overflow the wasm stack. IDEMPOTENT + DAG-safe: an already-IMMORTAL node is
/// skipped, so a shared node (persistent structures share) marks exactly once — no double census-decrement,
/// no cycle. An immediate (non-heap) handle owns no node and is skipped. Returns the same root handle.
pub(crate) fn op_mark_immortal_deep(root: Handle) -> Handle {
    // A LIFO worklist of handles yet to mark. Seeded with the root; a node's children are pushed as it is
    // marked. Handles are `Copy` (a pointer/immediate), so pushing a child READS it — the node stays live
    // and immortal (unlike `op_drop`, which takes the handles as it frees).
    let mut worklist: Vec<Handle> = Vec::new();
    worklist.push(root);
    while let Some(cur) = worklist.pop() {
        if is_immediate(cur) {
            continue; // an immediate owns no heap node — nothing to mark
        }
        if let Some(node) = unsafe { cur.node_mut() }
            && node.rc != IMMORTAL
        {
            #[cfg(any(test, feature = "debug-counters"))]
            let before = node.rc;
            node.rc = IMMORTAL;
            #[cfg(any(test, feature = "debug-counters"))]
            {
                LIVE_NODES.with(|n| n.set(n.get() - 1));
                // rc-trace: per marked node — census-exit-as-immortal (excluded from the leak summary).
                rc_trace_push(
                    RC_TRACE_MARK_IMMORTAL,
                    node.node_id,
                    rc_struct_tag(node),
                    before,
                    0,
                    false,
                    RC_TRACE_NO_PARENT,
                );
            }
            // Mark this node's children transitively. `handles` derefs to `[Handle]`, covering the inline
            // (≤2, e.g. a CHAMP `[k,v]` entry) and heap-spilled (a wide RRB/CHAMP node) cases uniformly.
            for &child in node.handles.iter() {
                worklist.push(child);
            }
        }
        // An already-IMMORTAL node: skip (its subtree was already marked on the path that first reached it).
    }
    root
}

pub(crate) fn op_dup(h: Handle) {
    if is_immediate(h) {
        return; // an immediate owns no heap — nothing to retain
    }
    if let Some(node) = unsafe { h.node_mut() } {
        // UAF/wild-handle guard (debug only): retaining a freed or fabricated cell is a bug.
        #[cfg(any(test, feature = "debug-counters"))]
        assert_node_live(h.0, node.guard, "dup");
        if node.rc != IMMORTAL {
            #[cfg(any(test, feature = "debug-counters"))]
            let before = node.rc;
            node.rc += 1; // an IMMORTAL node is never retained (dup is a no-op — the global owns it forever)
            #[cfg(any(test, feature = "debug-counters"))]
            rc_trace_push(
                RC_TRACE_DUP,
                node.node_id,
                rc_struct_tag(node),
                before,
                before + 1,
                false,
                RC_TRACE_NO_PARENT,
            );
        }
    }
}

/// The refcount of `h` (0 for null). The FBIP fast paths read this to decide, PER NODE, whether the
/// node is uniquely owned (`rc == 1`, safe to mutate in place) or shared (`rc > 1`, must path-copy) —
/// the aliasing-safety rule: a shared node backs another persistent version and must stay byte-identical.
pub(crate) fn node_rc(h: Handle) -> u32 {
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
pub(crate) fn op_drop(root: Handle) {
    if is_immediate(root) {
        return; // an immediate owns no heap — nothing to release
    }
    let node = match unsafe { root.node_mut() } {
        Some(n) => n,
        None => return, // null — benign
    };
    // UAF/wild-handle guard (debug only): dropping a freed cell is a double-free.
    #[cfg(any(test, feature = "debug-counters"))]
    assert_node_live(root.0, node.guard, "drop (double-free)");
    if node.rc == IMMORTAL {
        return; // an IMMORTAL node is never freed (a module global holds it) — drop is a no-op. MUST come
        // before the `rc > 1` decrement, else the sentinel would erode toward 1 and free the static.
    }
    if node.rc > 1 {
        #[cfg(any(test, feature = "debug-counters"))]
        let before = node.rc;
        node.rc -= 1; // shared: cheapest path, no reclamation
        #[cfg(any(test, feature = "debug-counters"))]
        rc_trace_push(
            RC_TRACE_DROP,
            node.node_id,
            rc_struct_tag(node),
            before,
            before - 1,
            false,
            RC_TRACE_NO_PARENT,
        );
        return;
    }
    // rc == 1: last reference. Reclaim the node and cascade into its children.
    // rc-trace: capture the root's id + structural tag BEFORE the worklist seeding empties its handles
    // (a Heap root donates its Vec via mem::take), so the tag reflects the live shape. The root id is
    // also the `cascade_parent` recorded on every child freed in this drop (v1: root-cascade linkage).
    #[cfg(any(test, feature = "debug-counters"))]
    let root_id = node.node_id;
    #[cfg(any(test, feature = "debug-counters"))]
    let root_tag = rc_struct_tag(node);
    //
    // The worklist is allocated LAZILY: an inline node's ≤2 children are pushed straight onto the
    // (initially-empty) worklist, and a `Vec` is materialized only if/when a HEAP child is expanded (a
    // node with >2 children — which necessarily already owns a heap Vec, so the cascade is heap-bound
    // there regardless). This keeps the dominant case — dropping a small (≤2-child, often all-immediate)
    // compound like a tuple/sum/`[k,v]` — ALLOCATION-FREE, matching the pre-inline behavior where the
    // freed node's own handle Vec was reused as the worklist seed. `SmallVec`-style: the seed lives in a
    // fixed `[Handle; INLINE_HANDLES_CAP]` until a spill is unavoidable.
    let mut seed_buf = [Handle::NULL; INLINE_HANDLES_CAP];
    let mut seed_len = 0usize;
    // The worklist REUSES a dying heap node's own `Vec` as its backing rather than allocating a fresh
    // one (the pre-inline behavior — the freed node's handle Vec was going to be freed anyway, so using
    // it as scratch is a zero-alloc cascade). It stays empty (no alloc) until the FIRST heap node is
    // reached, whose Vec it adopts by move; inline nodes' ≤2 children fill `seed_buf` with no heap at all.
    let mut worklist: Vec<Handle> = Vec::new();
    // Seed from the root: an inline root fills the buffer; a heap root donates its Vec as the worklist.
    match &mut node.handles {
        Handles::Inline { buf, len } => {
            seed_buf[..*len as usize].copy_from_slice(&buf[..*len as usize]);
            seed_len = *len as usize;
        }
        Handles::Heap(v) => worklist = core::mem::take(v),
    }
    // Release the root. DEBUG: bump the generation ODD (= freed) and RETAIN the cell (release its
    // raw/handle backings to bound debug memory, but leak the shell so the address stays a detectable
    // freed cell for the UAF guards above). SHIPPED: deallocate as before — this arm is byte-for-byte the
    // original free, so the release runtime is unchanged.
    #[cfg(any(test, feature = "debug-counters"))]
    {
        node.guard = freed_guard(root.0);
        node.raw.clear();
        node.handles = Handles::default();
    }
    #[cfg(not(any(test, feature = "debug-counters")))]
    unsafe {
        drop(Box::from_raw(root.0));
    }
    #[cfg(any(test, feature = "debug-counters"))]
    LIVE_NODES.with(|n| n.set(n.get() - 1));
    #[cfg(any(test, feature = "debug-counters"))]
    rc_trace_push(
        RC_TRACE_DROP,
        root_id,
        root_tag,
        1,
        0,
        true,
        RC_TRACE_NO_PARENT,
    );

    loop {
        // Pop from the worklist first (deeper heap subtrees), then drain the inline seed.
        let cur = match worklist.pop() {
            Some(c) => c,
            None if seed_len > 0 => {
                seed_len -= 1;
                seed_buf[seed_len]
            }
            None => break,
        };
        if is_immediate(cur) {
            continue; // an inline child owns no heap — the hottest RC path (doc-named)
        }
        let n = match unsafe { cur.node_mut() } {
            Some(n) => n,
            None => continue, // null child slot — benign
        };
        // UAF/wild-handle guard (debug only): a freed child still referenced by a dying compound is a
        // double-free / dangling child.
        #[cfg(any(test, feature = "debug-counters"))]
        assert_node_live(cur.0, n.guard, "drop-cascade (dangling child)");
        if n.rc == IMMORTAL {
            continue; // an IMMORTAL child (e.g. a shared build-once static nested in a dying compound) is
            // never freed and its count is untouched — skip it, do not decrement toward freeing.
        }
        if n.rc > 1 {
            #[cfg(any(test, feature = "debug-counters"))]
            let before = n.rc;
            n.rc -= 1; // shared child survives; freed only when its last owner drops it
            #[cfg(any(test, feature = "debug-counters"))]
            rc_trace_push(
                RC_TRACE_DROP,
                n.node_id,
                rc_struct_tag(n),
                before,
                before - 1,
                false,
                root_id,
            );
            continue;
        }
        // rc-trace: capture the child's id + tag BEFORE its handles are moved onto the worklist / it is
        // freed below; `cascade_parent = root_id` (v1 root-cascade linkage).
        #[cfg(any(test, feature = "debug-counters"))]
        let child_id = n.node_id;
        #[cfg(any(test, feature = "debug-counters"))]
        let child_tag = rc_struct_tag(n);
        // Move this node's children onto the pending set, then free it. An inline child-set with room
        // fills the seed buffer (no alloc). Otherwise: if the worklist is still empty, ADOPT this node's
        // own Vec as the worklist backing (a heap node owns one; reuse it — zero alloc, as the pre-inline
        // cascade did); if the worklist is already backed, append into it.
        match &mut n.handles {
            Handles::Inline { buf, len } if seed_len + *len as usize <= INLINE_HANDLES_CAP => {
                seed_buf[seed_len..seed_len + *len as usize].copy_from_slice(&buf[..*len as usize]);
                seed_len += *len as usize;
            }
            Handles::Heap(v) if worklist.is_empty() => {
                // Adopt the dying node's Vec (no alloc), then fold any pending inline-seed items in.
                worklist = core::mem::take(v);
                if seed_len > 0 {
                    worklist.extend_from_slice(&seed_buf[..seed_len]);
                    seed_len = 0;
                }
            }
            _ => {
                if seed_len > 0 {
                    worklist.extend_from_slice(&seed_buf[..seed_len]);
                    seed_len = 0;
                }
                n.handles.append_into(&mut worklist);
            }
        }
        // Release the child (see the root free above): DEBUG bumps the generation odd + retains for UAF
        // detection; SHIPPED deallocates byte-for-byte as before.
        #[cfg(any(test, feature = "debug-counters"))]
        {
            n.guard = freed_guard(cur.0);
            n.raw.clear();
            n.handles = Handles::default();
        }
        #[cfg(not(any(test, feature = "debug-counters")))]
        unsafe {
            drop(Box::from_raw(cur.0));
        }
        #[cfg(any(test, feature = "debug-counters"))]
        LIVE_NODES.with(|n| n.set(n.get() - 1));
        #[cfg(any(test, feature = "debug-counters"))]
        rc_trace_push(RC_TRACE_DROP, child_id, child_tag, 1, 0, true, root_id);
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
pub(crate) fn op_reset(node: Handle) -> Handle {
    if is_immediate(node) {
        return Handle::NULL; // an immediate owns no shell to reuse (covers the borrows below)
    }
    // Read rc through a short-lived borrow that ends before we recurse into children.
    let rc = match unsafe { node.node_ref() } {
        Some(n) => n.rc,
        None => return Handle::NULL, // null: nothing to reuse
    };
    if rc > 1 {
        if let Some(n) = unsafe { node.node_mut() } {
            n.rc -= 1; // shared: another owner keeps it intact; no reuse token
        }
        return Handle::NULL;
    }
    // Unique. Take the children out (ending the borrow before the drops), release each, then put
    // the now-empty backing Vec back so the shell keeps its allocation for the coming refit.
    let mut children = match unsafe { node.node_mut() } {
        Some(n) => core::mem::take(&mut n.handles),
        None => return Handle::NULL,
    };
    for &child in children.iter() {
        op_drop(child); // cascades; a child dup'd by the compiler before reset survives
    }
    children.clear(); // 0 elements, capacity retained
    if let Some(n) = unsafe { node.node_mut() } {
        n.handles = children; // restore the (empty, capacity-bearing) backing
        n.raw.clear();
    }
    node // the retained shell, rc == 1, empty — a reuse token
}

/// `arr-alloc-reuse` — `arr-alloc(len)`, but reusing `token`'s shell when it is a non-null reuse
/// token from `reset`: refit it to `len` NULL slots (reusing its handle-Vec backing when capacity
/// allows — the common same-length case reallocates nothing) and return it, allocating NO new node.
/// A null token allocates fresh, so a `reset` that declined to yield a token is transparent.
pub(crate) fn op_arr_alloc_reuse(len: u32, token: Handle) -> Handle {
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
    match unsafe { token.node_mut() } {
        None => op_arr_alloc(len),
        Some(n) => {
            n.rc = 1;
            // Refit the handles to `len` NULL slots, matching what a FRESH `op_arr_alloc(len)` produces:
            // ≤cap → INLINE, wider → a heap Vec. A wide reset token carries a `Handles::Heap` whose Vec
            // `clear()`/`resize` KEEP (clear retains capacity; resize only spills inline→heap, never
            // re-inlines heap→inline) — so refitting it SMALL would leave a stray heap Vec where the fresh
            // node is inline: a retained allocation for the node's life AND a forked storage rep for one
            // logical value, invisible to `champ_eq`/`champ_hash` (they read via `as_slice`). This is the
            // handles-arm twin of the raw-arm divergence normalized below. Assign a fresh inline `Handles`
            // for a ≤cap refit (dropping any leftover heap Vec); reuse the token's Vec backing in place
            // only for a WIDE refit (the FBIP win — the common same-length refit reallocates nothing).
            if (len as usize) <= INLINE_HANDLES_CAP {
                n.handles = Handles::inline_nulls(len as usize);
            } else {
                n.handles.clear();
                n.handles.resize(len as usize, Handle::NULL);
            }
            // Reset to an EMPTY INLINE raw (an array node carries no raw). `raw.clear()` would keep a
            // heap buffer if the token came from a reset bytes/string leaf — an empty heap Vec retained
            // for the node's life, and a non-canonical rep vs the inline-empty raw a fresh `op_arr_alloc`
            // produces. Assigning the inline-empty raw drops that spill and matches the fresh node.
            n.raw = Raw::Inline {
                len: 0,
                buf: [0u8; INLINE_RAW_CAP],
            };
            token
        }
    }
}

/// `sum-new-reuse` — `sum-new(disc, payload)`, but reusing `token`'s shell when non-null: repurpose
/// it as the `(disc, payload)` node with no new allocation. Null token allocates fresh.
pub(crate) fn op_sum_new_reuse(disc: u32, payload: Handle, token: Handle) -> Handle {
    if is_immediate(token) {
        return op_sum_new(disc, payload); // defensive: reset never yields an immediate token
    }
    match unsafe { token.node_mut() } {
        None => op_sum_new(disc, payload),
        Some(n) => {
            n.rc = 1;
            // A sum node is ALWAYS arity 1 (a single payload), so a fresh `op_sum_new` gives INLINE
            // handles. A wide reset token carries a `Handles::Heap` whose Vec `clear()` + `push` KEEP the
            // heap arm (clear retains capacity; push on a Heap stays Heap) — leaving the reused sum node
            // carrying a stray heap Vec where the fresh node is inline: a retained allocation AND a forked
            // storage rep, invisible to `champ_eq`/`champ_hash` (the handles-arm twin of the raw-arm
            // divergence normalized below). Assign a fresh inline single-payload `Handles` directly,
            // dropping any leftover heap Vec — matching `op_sum_new` byte-for-byte.
            n.handles = Handles::inline_from(&[payload]);
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

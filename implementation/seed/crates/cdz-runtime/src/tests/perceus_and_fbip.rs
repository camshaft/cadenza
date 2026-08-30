use super::*;

#[test]
fn node_born_with_refcount_one() {
    reset();
    // Definitely-boxed values are born with rc == 1. `op_box_int(5)` now INLINES (no Node), so
    // use a genuinely-boxed leaf / an out-of-window int to exercise the heap birth-refcount.
    assert_eq!(rc_of(boxed_int_leaf(5)), 1);
    assert_eq!(
        rc_of(op_box_int((1 << 30) as i64)),
        1,
        "out-of-window int boxes"
    );
    assert_eq!(rc_of(op_arr_alloc(2)), 1);
    assert_eq!(rc_of(op_sum_new(0, op_arr_alloc(0))), 1);
    assert_eq!(rc_of(op_str_new("x".to_string())), 1);
    // An immediate is NOT a Node and must never report rc == 1 (a 1 would let an FBIP in-place
    // path mutate a non-Node) — the P2 canonical-form invariant.
    assert_ne!(
        rc_of(op_box_int(5)),
        1,
        "an inline int must not look uniquely-owned"
    );
    assert_ne!(rc_of(op_box_bool(true)), 1);
    assert_ne!(rc_of(op_arr_alloc(0)), 1);
}

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
    assert_eq!(
        live_nodes(),
        before + 3,
        "shared child must not be freed while B holds it"
    );
    assert_eq!(
        op_get_int(op_arr_get(child, 0)),
        9,
        "shared child still intact"
    );

    // Drop parent B: releases the second reference; the birth reference still pins the child.
    op_drop(pb);
    assert_eq!(
        live_nodes(),
        before + 2,
        "child still pinned by its birth reference"
    );
    assert_eq!(op_get_int(op_arr_get(child, 0)), 9);

    // Release the birth reference: now the child's last owner is gone → child + int reclaimed.
    op_drop(child);
    assert_eq!(
        live_nodes(),
        before,
        "last owner gone: shared subtree reclaimed"
    );
}

#[test]
fn dag_single_root_reaches_a_shared_child_via_two_paths_drops_once() {
    reset();
    let before = live_nodes();
    // `inner` shared by both paths; op_box_int is an immediate (uncounted), so only the 3 arr nodes
    // (inner, sub, root) are live Nodes.
    let inner = op_arr_alloc(1);
    op_arr_set(inner, 0, op_box_int(7));
    op_dup(inner); // one reference per parent path → rc == 2
    let sub = op_arr_alloc(2);
    op_arr_set(sub, 0, inner); // path 2: root → sub → inner
    op_arr_set(sub, 1, op_box_int(9));
    let root = op_arr_alloc(2);
    op_arr_set(root, 0, inner); // path 1: root → inner (the SAME node)
    op_arr_set(root, 1, sub);
    assert_eq!(
        live_nodes(),
        before + 3,
        "3 arr nodes (inner, sub, root); ints are immediates"
    );
    assert_eq!(
        node_rc(inner),
        2,
        "inner is shared by exactly the two paths"
    );
    // Both paths read the shared child correctly (it is genuinely reachable two ways).
    assert_eq!(
        op_get_int(op_arr_get(op_arr_get(root, 0), 0)),
        7,
        "path 1 (root.0) reaches inner"
    );
    assert_eq!(
        op_get_int(op_arr_get(op_arr_get(op_arr_get(root, 1), 0), 0)),
        7,
        "path 2 (root.1.0) reaches the SAME inner"
    );
    // Drop the ONE root: the cascade must reclaim ALL three nodes exactly once — the first visit to
    // `inner` (via root.0) decrements rc 2→1 without recursing; the second (via sub→inner) decrements
    // 1→0 and frees. No leak, no double-free/UAF.
    op_drop(root);
    assert_eq!(
        live_nodes(),
        before,
        "single-root DAG fully reclaimed — shared child freed exactly once across its two paths"
    );
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
    assert_eq!(
        live_nodes(),
        before,
        "deep structure fully reclaimed, no overflow"
    );
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

    assert_eq!(
        op_get_int(kept),
        3,
        "kept element must survive the parent drop"
    );
    assert_eq!(live_nodes(), before + 1, "only the kept element remains");
    op_drop(kept); // the returned owner is eventually released
    assert_eq!(live_nodes(), before, "no leak once the kept owner drops");
}

#[test]
fn rc_convention_nested_compound_projection_survives_aggregate_drop() {
    reset();
    let before = live_nodes();
    // r = (tuple W.Atom(42) 7): a nested-compound child (sum + payload = 2 nodes) + a scalar sibling.
    let nested = op_sum_new(1, boxed_int_leaf(42));
    let r = op_arr_alloc(2);
    op_arr_set(r, 0, nested);
    op_arr_set(r, 1, boxed_int_leaf(7));
    assert_eq!(
        live_nodes(),
        before + 4,
        "aggregate + nested sum + its payload + scalar sibling"
    );
    // Project the NESTED child out and keep it (dup before dropping the aggregate) — the escape.
    let kept = op_arr_get(r, 0);
    op_dup(kept); // the escaped nested-compound is now rc=2
    op_drop(r); // frees r + the scalar sibling; MUST NOT free `kept` or its payload (rc>1 stops the cascade)
    // The escaped child's WHOLE subtree is intact — reading its payload is not a use-after-free.
    assert_eq!(
        op_get_int(op_sum_payload(kept)),
        42,
        "the kept nested-compound's payload survives the aggregate drop (no UAF)"
    );
    assert_eq!(
        live_nodes(),
        before + 2,
        "only the escaped child + its payload remain (aggregate + sibling freed, subtree NOT freed)"
    );
    op_drop(kept); // the escaped owner's last drop reclaims its subtree
    assert_eq!(live_nodes(), before, "no leak once the escaped child drops");
}

#[test]
fn rc_convention_shared_payload_consumed_while_parent_and_sibling_survive() {
    reset();
    let before = live_nodes();
    // node = (tuple head=Name('a'), elems=[Name('b'), Int(5)]) — a "List" AST node: a head + child list.
    let head = op_sum_new(0, boxed_int_leaf(97)); // Name('a')
    let e0 = op_sum_new(0, boxed_int_leaf(98)); // Name('b')
    let e1 = op_sum_new(1, boxed_int_leaf(5)); // Int(5)
    let elems = op_arr_alloc(2);
    op_arr_set(elems, 0, e0);
    op_arr_set(elems, 1, e1);
    let node = op_arr_alloc(2);
    op_arr_set(node, 0, head);
    op_arr_set(node, 1, elems);
    // The compiler's FIX shape: DUP the `elems` payload before passing it into the consuming walk
    // (rc 1→2 — one reference for `node`, one for the walk that will consume it).
    let elems_ref = op_arr_get(node, 1);
    op_dup(elems_ref);
    // Owner A — the consuming (mutual-)recursive walk fully releases ITS `elems` reference (the walk
    // drops each element as it descends; here modelled as a single op_drop of the dup'd ref).
    op_drop(elems_ref);
    // Owner B — the SIBLING read of the PARENT `node` (head-of-node) must be UNCORRUPTED. This is the
    // exact read the miscompile broke (`head reads None`); the dup keeps `elems` (and thus `node`) alive.
    assert_eq!(
        op_get_int(op_sum_payload(op_arr_get(node, 0))),
        97,
        "sibling head-of-node read survives the shared payload's consume (no corruption)"
    );
    // And `elems`, still held by `node`, is fully intact — both children readable.
    assert_eq!(
        op_get_int(op_sum_payload(op_arr_get(op_arr_get(node, 1), 0))),
        98,
        "the shared payload's element 0 (Name 'b') survives"
    );
    assert_eq!(
        op_get_int(op_sum_payload(op_arr_get(op_arr_get(node, 1), 1))),
        5,
        "…and element 1 (Int 5) survives"
    );
    // Dropping the parent reclaims everything exactly once (the walk already released its dup'd ref).
    op_drop(node);
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free: the dup'd payload ref was consumed, the parent's ref freed the rest"
    );
}

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

    assert_eq!(
        op_get_int(x),
        42,
        "extracted payload survives the scrutinee drop"
    );
    assert_eq!(live_nodes(), before + 1, "sum node reclaimed, payload kept");
    op_drop(x);
    assert_eq!(live_nodes(), before);
}

#[test]
fn rc_convention_match_discard_reclaims_whole_sum() {
    reset();
    let before = live_nodes();
    let s = op_sum_new(1, boxed_int_leaf(42)); // sum + heap-int payload = 2 nodes
    assert_eq!(live_nodes(), before + 2);
    // Arm returns a constant; payload not kept ⇒ just drop the scrutinee.
    op_drop(s);
    assert_eq!(
        live_nodes(),
        before,
        "whole sum + payload reclaimed when nothing is kept"
    );
}

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
    assert_eq!(
        live_nodes(),
        before + 3,
        "tuple node added; x not duplicated in memory"
    );

    // Dropping the tuple releases BOTH references; x is reclaimed exactly once, no double-free.
    op_drop(t);
    assert_eq!(
        live_nodes(),
        before,
        "duplicate-binder child reclaimed exactly once"
    );
}

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
        assert_eq!(
            op_get_int(op_arr_get(result, 0)),
            expect,
            "the taken list survives intact"
        );
        assert_eq!(
            live_nodes(),
            before + 2,
            "exactly one list (2 nodes) survives"
        );
        op_drop(result); // the if's owned result is eventually released
        assert_eq!(
            live_nodes(),
            before,
            "no leak, no double-free on either path"
        );
    }
}

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

#[test]
fn reset_unique_yields_emptied_shell_token() {
    reset();
    let before = live_nodes();
    let t = op_arr_alloc(2);
    op_arr_set(t, 0, boxed_int_leaf(3));
    op_arr_set(t, 1, boxed_int_leaf(4)); // shell + 2 (real heap) ints = 3 nodes
    assert_eq!(live_nodes(), before + 3);

    let token = op_reset(t);
    assert_eq!(
        token, t,
        "the token IS the reset node's shell (same handle)"
    );
    assert_ne!(token, Handle::NULL, "unique reset yields a non-null token");
    assert_eq!(op_arr_len(token), 0, "children released; shell is empty");
    assert_eq!(rc_of(token), 1, "the retained shell keeps rc == 1");
    assert_eq!(
        live_nodes(),
        before + 1,
        "the 2 children freed; only the shell remains"
    );

    op_drop(token); // an unused token is just a childless unique node — drop frees the shell
    assert_eq!(
        live_nodes(),
        before,
        "dropping an unused token frees exactly the shell"
    );
}

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
    assert_eq!(
        op_get_int(op_arr_get(t, 0)),
        9,
        "the shared node is fully intact"
    );
    assert_eq!(
        live_nodes(),
        before + 2,
        "nothing freed: the other owner still holds it"
    );

    op_drop(t); // release the surviving owner
    assert_eq!(live_nodes(), before);
}

#[test]
fn reuse_ctors_with_null_token_allocate_fresh() {
    reset();
    let before = live_nodes();
    let a = op_arr_alloc_reuse(2, Handle::NULL);
    assert_eq!(
        op_arr_len(a),
        2,
        "null token: fresh array of the requested length"
    );
    let s = op_sum_new_reuse(1, boxed_int_leaf(7), Handle::NULL);
    assert_eq!(op_sum_disc(s), 1);
    assert_eq!(op_get_int(op_sum_payload(s)), 7);
    assert_eq!(
        live_nodes(),
        before + 3,
        "array + sum + its (heap) int, all freshly allocated"
    );
    op_drop(a);
    op_drop(s);
    assert_eq!(live_nodes(), before);
}

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
    assert_eq!(
        fresh, old,
        "reuse returns the very same node — in-place, no allocation"
    );
    assert_eq!(op_arr_len(fresh), 3, "refit to the new length");
    assert_eq!(
        live_nodes(),
        before + 1,
        "still one node: no new allocation for the rebuild"
    );
    op_arr_set(fresh, 0, op_box_int(10));
    op_arr_set(fresh, 1, op_box_int(20));
    op_arr_set(fresh, 2, op_box_int(30));
    assert_eq!(op_get_int(op_arr_get(fresh, 2)), 30);
    op_drop(fresh);
    assert_eq!(live_nodes(), before);
}

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
    assert!(
        raw_is_heap(leaf),
        "precondition: a >cap bytes leaf has a heap raw"
    );
    let token = op_reset(leaf); // childless heap-raw shell, rc==1
    assert_eq!(token, leaf, "unique reset yields the shell");
    let s = op_sum_new_reuse(3, op_box_int(42), token);
    assert!(
        !raw_is_heap(s),
        "reused sum node's raw is INLINE, not the token's leftover heap buffer"
    );
    assert_eq!(op_sum_disc(s), 3, "disc correct");
    assert_eq!(op_get_int(op_sum_payload(s)), 42, "payload correct");
    // Byte-identical to a FRESH sum (same disc/payload) — the whole point of normalizing the rep.
    let fresh_sum = op_sum_new(3, op_box_int(42));
    assert!(
        champ_eq(s, fresh_sum),
        "reused sum equals a fresh one built the same way"
    );
    assert_eq!(
        champ_hash(s),
        champ_hash(fresh_sum),
        "…and hashes identically"
    );
    op_drop(s);
    op_drop(fresh_sum);

    // (2) reuse a heap-raw shell as an ARRAY node → raw must be inline (empty).
    let leaf2 = big_leaf(INLINE_RAW_CAP + 20);
    assert!(raw_is_heap(leaf2));
    let token2 = op_reset(leaf2);
    let a = op_arr_alloc_reuse(2, token2);
    assert!(
        !raw_is_heap(a),
        "reused array node's raw is INLINE-empty, not a leftover heap buffer"
    );
    op_arr_set(a, 0, op_box_int(1));
    op_arr_set(a, 1, op_box_int(2));
    assert_eq!(op_get_int(op_arr_get(a, 1)), 2);
    op_drop(a);

    assert_eq!(
        live_nodes(),
        before,
        "no leak: every reused/fresh node reclaimed"
    );
}

#[test]
fn reuse_ctor_normalizes_a_wide_heap_handles_token_to_inline() {
    reset();
    let before = live_nodes();

    // (1) reuse a WIDE array shell (arity 4 → heap handles) as a ≤cap ARRAY node.
    let wide = op_arr_alloc(4); // 4 > INLINE_HANDLES_CAP → Handles::Heap
    for i in 0..4 {
        op_arr_set(wide, i, op_box_int(i as i64));
    }
    assert!(
        handles_is_heap(wide),
        "precondition: an arity-4 array has a heap handle vector"
    );
    let token = op_reset(wide); // childless heap-handles shell, rc==1
    assert_eq!(token, wide, "unique reset yields the shell");
    let a = op_arr_alloc_reuse(2, token); // refit to 2 slots (≤ cap)
    assert!(
        !handles_is_heap(a),
        "reused array node's handles are INLINE for a ≤cap arity, not the token's leftover heap Vec"
    );
    op_arr_set(a, 0, op_box_int(10));
    op_arr_set(a, 1, op_box_int(20));
    // Byte-identical rep to a FRESH arity-2 array built the same way.
    let fresh_arr = op_arr_alloc(2);
    op_arr_set(fresh_arr, 0, op_box_int(10));
    op_arr_set(fresh_arr, 1, op_box_int(20));
    assert!(
        champ_eq(a, fresh_arr),
        "reused array equals a fresh one built the same way"
    );
    assert_eq!(
        champ_hash(a),
        champ_hash(fresh_arr),
        "…and hashes identically"
    );
    op_drop(a);
    op_drop(fresh_arr);

    // (2) reuse a WIDE array shell as a SUM node (arity 1 → inline handles when fresh).
    let wide2 = op_arr_alloc(5);
    for i in 0..5 {
        op_arr_set(wide2, i, op_box_int(i as i64));
    }
    assert!(handles_is_heap(wide2));
    let token2 = op_reset(wide2);
    let s = op_sum_new_reuse(1, op_box_int(42), token2);
    assert!(
        !handles_is_heap(s),
        "reused sum node's single-payload handles are INLINE, not the token's leftover heap Vec"
    );
    assert_eq!(op_sum_disc(s), 1, "disc correct");
    assert_eq!(op_get_int(op_sum_payload(s)), 42, "payload correct");
    let fresh_sum = op_sum_new(1, op_box_int(42));
    assert!(
        champ_eq(s, fresh_sum),
        "reused sum equals a fresh one built the same way"
    );
    assert_eq!(
        champ_hash(s),
        champ_hash(fresh_sum),
        "…and hashes identically"
    );
    op_drop(s);
    op_drop(fresh_sum);

    assert_eq!(
        live_nodes(),
        before,
        "no leak: every reused/fresh node reclaimed"
    );
}

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
    assert_eq!(
        live_nodes(),
        before + 2,
        "reused shell + the new payload int; shell not re-alloc'd"
    );
    op_drop(fresh);
    assert_eq!(live_nodes(), before);
}

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
        assert_eq!(
            ys, shell_addr,
            "the mapped list reuses the input's array shell in place"
        );
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
    assert_eq!(
        peak_probe,
        before + 1 + 2 * N as i64,
        "peak = shell + old ints + new ints"
    );
    assert_eq!(
        live_nodes(),
        before,
        "no leak after the mapped list is dropped"
    );
}

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
    assert_eq!(
        live_nodes(),
        before + 2,
        "shell + kept child; discard freed"
    );
    let rebuilt = op_arr_alloc_reuse(1, token);
    op_arr_set(rebuilt, 0, keep); // carry the kept child into the reused shell
    assert_eq!(
        op_get_int(op_arr_get(rebuilt, 0)),
        77,
        "kept child survived reset into the reuse"
    );
    assert_eq!(
        live_nodes(),
        before + 2,
        "still shell + kept child — reuse allocated nothing"
    );
    op_drop(rebuilt);
    assert_eq!(live_nodes(), before, "no leak");
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
    assert_eq!(
        op_get_int(op_vec_get(v, 3)),
        3,
        "first of child 1 (boundary)"
    );
    assert_eq!(op_get_int(op_vec_get(v, 4)), 4, "last of child 1");
    assert_eq!(
        op_get_int(op_vec_get(v, 5)),
        5,
        "first of child 2 (boundary)"
    );
    assert_eq!(op_get_int(op_vec_get(v, 8)), 8, "last element");
    // And the full dense round-trip.
    assert_eq!(vec_to_ints(v), (0..9).collect::<Vec<_>>());
    op_drop(v);
    assert_eq!(
        live_nodes(),
        before,
        "relaxed hand-built vector reclaims to baseline"
    );
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
    assert!(
        vec_is_relaxed(rroot),
        "hand-built relaxed node (positive control)"
    );

    op_drop(empty);
    op_drop(v);
    op_drop(m);
    op_drop(rope);
    op_drop(slice);
    op_drop(relaxed);
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the disambiguation cases"
    );
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
    assert_eq!(
        op_get_int(op_vec_get(v, 3)),
        3,
        "neighbor in same child untouched"
    );
    assert_eq!(
        op_get_int(op_vec_get(v, 5)),
        5,
        "neighbor across boundary untouched"
    );
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
    assert!(
        vec_is_relaxed(root),
        "root stays relaxed after right-edge pushes"
    );
    assert_eq!(op_vec_len(v), 20);
    assert_eq!(
        vec_to_ints(v),
        (0..20).collect::<Vec<_>>(),
        "dense round-trip after pushes"
    );
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak");
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
fn vec_concat_is_associative_observably() {
    reset();
    let before = live_nodes();
    // A vec of `n` elements starting at `lo` (distinct ranges per operand → order-sensitive).
    let mkv = |lo: i64, n: i64| -> Handle {
        let mut v = op_vec_empty();
        for i in 0..n {
            v = op_vec_push(v, op_box_int(lo + i));
        }
        v
    };
    let list_desc: &[u8] = &[0x02, 0x00, 0x07, 0x00, 0x01]; // [0]=Int [1]=List(elem→0); root=1
    let sizes = [1i64, 5, 31, 32, 33, 100, 500];
    for &na in &sizes {
        for &nb in &sizes {
            for &nc in &[1i64, 33, 100] {
                // Left association: (a ++ b) ++ c.
                let left = {
                    let ab = op_vec_concat(mkv(0, na), mkv(1000, nb));
                    op_vec_concat(ab, mkv(2000, nc))
                };
                // Right association: a ++ (b ++ c).
                let right = {
                    let bc = op_vec_concat(mkv(1000, nb), mkv(2000, nc));
                    op_vec_concat(mkv(0, na), bc)
                };
                // Both are well-formed RRB trees (relaxed size tables consistent, header count == leaves).
                assert_vec_invariants(left);
                assert_vec_invariants(right);
                // Observable equivalence 1: identical element sequence.
                assert_eq!(
                    vec_to_ints(left),
                    vec_to_ints(right),
                    "concat associativity: same element sequence for ({na},{nb},{nc})"
                );
                // Observable equivalence 2: byte-identical value-encode (renders by element order), so
                // the differing internal shapes are unobservable at the boundary.
                let el = op_value_encode_form(left, list_desc);
                let er = op_value_encode_form(right, list_desc);
                assert_eq!(
                    el, er,
                    "concat associativity: byte-identical value-encode for ({na},{nb},{nc})"
                );
                op_drop(left);
                op_drop(right);
            }
        }
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the associativity matrix"
    );
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
    assert_eq!(
        op_get_int(op_vec_get(v, 39)),
        39,
        "neighbor untouched by update"
    );
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
        assert!(
            vec_is_relaxed(root),
            "unequal/large concat produced a relaxed root ({la},{lb})"
        );
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
    assert_eq!(
        op_vec_len(acc) as usize,
        oracle.len(),
        "folded length == 200*7"
    );
    assert_eq!(vec_to_ints(acc), oracle, "folded elements match oracle");
    assert_vec_invariants(acc);
    op_drop(acc);
    assert_eq!(live_nodes(), before, "no leak after deep fold-concat");
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
        assert_eq!(
            vec_to_ints(joined),
            (0..1000).collect::<Vec<_>>(),
            "reconcat elements for i={i}"
        );
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
fn vec_drop_returns_the_tail_and_reclaims_the_prefix() {
    reset();
    let before = live_nodes();
    // A size-1500 vector (3 levels) — the drop point spans full-depth descent like the split test.
    for &idx in &[0u32, 1, 733, 1499, 1500, 2000] {
        let v = vec_range(1500);
        let tail = vec_drop_impl(v, idx);
        let clamped = idx.min(1500);
        assert_eq!(
            op_vec_len(tail),
            1500 - clamped,
            "vec-drop({idx}) tail length"
        );
        assert_eq!(
            vec_to_ints(tail),
            (clamped as i64..1500).collect::<Vec<_>>(),
            "vec-drop({idx}) tail content is [idx, len)"
        );
        assert_vec_invariants(tail);
        op_drop(tail);
        assert_eq!(
            live_nodes(),
            before,
            "vec-drop({idx}) reclaims the prefix + result fully — no leak (consuming op)"
        );
    }
}

#[test]
fn vec_drop_tail_matches_split_drop_left() {
    reset();
    let before = live_nodes();
    // `[lo, hi)` as a push-built vector.
    fn vrange(lo: i64, hi: i64) -> Handle {
        let mut v = op_vec_empty();
        for i in lo..hi {
            v = op_vec_push(v, op_box_int(i));
        }
        v
    }
    // Two builders: a STRICT push-built vector, and a RELAXED concat-built one (same 0..N contents).
    let strict = |n: u32| vrange(0, n as i64);
    let relaxed = |n: u32| {
        // concat two halves so the boundary interior nodes go relaxed.
        let mid = (n / 2) as i64;
        op_vec_concat(vrange(0, mid), vrange(mid, n as i64))
    };
    for build in [&strict as &dyn Fn(u32) -> Handle, &relaxed] {
        for &n in &[40u32, 1500] {
            for &idx in &[0u32, 1, 2, n / 2, n - 1, n, n + 5] {
                // Reference: split + drop-left (a fresh copy).
                let vr = build(n);
                let (l, r) = op_vec_split(vr, idx);
                op_drop(l);
                // Under test: drop-tail (another fresh copy).
                let vt = build(n);
                let t = op_vec_drop_tail(vt, idx);
                assert!(
                    champ_eq(r, t),
                    "drop_tail(n={n}, idx={idx}) is champ_eq to split+drop-left (byte-identical canonical shape)"
                );
                assert_eq!(op_vec_len(t), op_vec_len(r), "same length");
                assert_eq!(vec_to_ints(t), vec_to_ints(r), "same tail content");
                assert_vec_invariants(t);
                op_drop(r);
                op_drop(t);
            }
        }
    }
    assert_eq!(live_nodes(), before, "no leak across the differential");
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
fn vec_of_arr_matches_push_built_across_sizes() {
    reset();
    let before = live_nodes();
    // Cover: empty, single leaf (≤32), exactly one full leaf (32), just over (33 → 2 levels),
    // and a multi-leaf tree (100). For each, `vec-of-arr(arr)` must read back identically to a
    // push-built vector of the same elements, and be BYTE-INTERCHANGEABLE (further vec ops work).
    for n in [0i64, 1, 5, 31, 32, 33, 64, 100] {
        let v = op_vec_of_arr(arr_of_ints(0, n));
        assert_eq!(op_vec_len(v), n as u32, "vec-of-arr len for n={n}");
        let want: Vec<i64> = (0..n).collect();
        assert_eq!(vec_to_ints(v), want, "vec-of-arr elements for n={n}");
        // Interchangeable with a push-built vector: push one more, read back.
        let v = op_vec_push(v, op_box_int(n));
        let mut want2 = want.clone();
        want2.push(n);
        assert_eq!(vec_to_ints(v), want2, "vec-of-arr then push for n={n}");
        op_drop(v);
    }
    assert_eq!(live_nodes(), before, "no leak across the vec-of-arr sizes");
}

#[test]
fn vec_of_arr_small_reuses_arr_as_leaf_no_extra_node() {
    reset();
    let before = live_nodes();
    // A ≤32-element arr: `vec-of-arr` reuses the arr node as the single leaf-root, so the ONLY new
    // node is the 8-byte header — the arr shell is NOT freed-and-reallocated. Build a 3-elem arr
    // (arr node + 3 boxed... but small ints inline → arr node only): live = before + 1 (arr).
    let a = op_arr_alloc(3);
    for i in 0..3u32 {
        op_arr_set(a, i, op_box_int(i as i64)); // small ints are immediate — no boxed nodes
    }
    assert_eq!(
        live_nodes(),
        before + 1,
        "just the arr node (immediate elements)"
    );
    let v = op_vec_of_arr(a);
    // header (new) + the reused arr-as-leaf = before + 2; NOT before + 3 (no throwaway leaf).
    assert_eq!(
        live_nodes(),
        before + 2,
        "vec-of-arr adds ONLY the header — arr reused as the leaf"
    );
    assert_eq!(vec_to_ints(v), vec![0, 1, 2]);
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn vec_of_arr_result_is_indistinguishable_from_push_built() {
    reset();
    let before = live_nodes();
    // The memory/WIT contract: a vec-of-arr result is INTERCHANGEABLE with a push-built vector —
    // concat/split/update all work and agree with the oracle. Use n=70 (multi-leaf) to exercise the
    // trie-built path, not just a single leaf.
    let a = op_vec_of_arr(arr_of_ints(0, 70));
    let b = vec_range(70); // push-built twin
    // update through the vec-of-arr-built vector
    let a = op_vec_update(a, 65, op_box_int(999));
    assert_eq!(
        op_get_int(op_vec_get(a, 65)),
        999,
        "update on a vec-of-arr-built vector"
    );
    assert_eq!(op_get_int(op_vec_get(a, 0)), 0, "other elements intact");
    // split it and reconcat — round-trips
    op_dup(a);
    let (l, r) = op_vec_split(a, 40);
    let joined = op_vec_concat(l, r);
    assert_eq!(
        vec_to_ints(joined),
        vec_to_ints(a),
        "split+reconcat round-trips"
    );
    op_drop(joined);
    op_drop(a);
    op_drop(b);
    assert_eq!(
        live_nodes(),
        before,
        "no leak across update/split/concat on a vec-of-arr vector"
    );
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
    assert_eq!(
        live_nodes(),
        before,
        "the whole vector subtree is reclaimed"
    );
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
    assert_eq!(
        op_get_int(op_vec_get(v1, 99)),
        99,
        "shared tail intact after v0 dropped"
    );
    assert!(
        live_nodes() > before,
        "v1 (and its shared subtrees) still live"
    );
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
    assert_eq!(
        unique_push_alloc, 1,
        "unique push allocates just the pushed element"
    );

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
    assert_eq!(
        unique_update_alloc, 0,
        "unique update reuses the spine; inline elem adds no node"
    );
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
    assert_eq!(
        vec_to_ints(v0),
        v0_orig,
        "v0 intact after push on a partially-shared sibling"
    );
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
        *lcg = lcg
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
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
    assert_eq!(
        op_vec_len(v) as usize,
        oracle.len(),
        "length matches oracle"
    );
    assert_eq!(
        vec_to_ints(v),
        oracle,
        "elements match oracle after mixed FBIP ops"
    );
    assert_vec_invariants(v);
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak across the mixed sequence");
}

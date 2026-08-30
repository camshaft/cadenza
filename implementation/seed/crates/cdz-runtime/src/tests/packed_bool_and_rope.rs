use super::*;

#[test]
fn packed_bool_leaf_is_actually_packed_and_dense() {
    reset();
    let before = live_nodes();
    // A ≤32 bool vector's ROOT leaf is packed: empty handles, 5-byte `[count][bits]` raw, and its
    // whole footprint is header + ONE leaf node — no heap Vec of 32 handles.
    let bs: Vec<bool> = (0..17).map(|i| i % 3 == 0).collect();
    let v = op_vec_push(op_vec_empty(), op_box_bool(true)); // seed to allocate a header
    op_drop(v);
    let v = vec_of_bools(&bs);
    let (_, shift, root) = vec_read_header(v);
    assert_eq!(shift, 0, "≤32 bools live in a single leaf (shift 0)");
    assert!(vec_leaf_is_packed(root), "the leaf is packed");
    assert_eq!(
        packed_leaf_count(root),
        bs.len(),
        "packed count == element count"
    );
    with_node(root, (), |n| {
        assert!(n.handles.is_empty(), "packed leaf holds NO handles");
        assert_eq!(
            n.raw.len(),
            PACKED_BOOL_LEAF_RAW_LEN,
            "packed leaf raw is exactly [count][bits]"
        );
    });
    // Footprint: header + one packed leaf = 2 nodes, regardless of the 17 elements (they're bits,
    // and bool immediates are not nodes anyway).
    assert_eq!(
        live_nodes(),
        before + 2,
        "a 17-bool vector is just header + one packed leaf"
    );
    assert_eq!(vec_to_bools(v), bs, "reads back the exact bool sequence");
    op_drop(v);
    assert_eq!(live_nodes(), before, "packed leaf drops clean, no leak");
}

#[test]
fn packed_bool_get_len_push_update_match_oracle() {
    reset();
    let before = live_nodes();
    // Cover single-leaf and multi-leaf sizes; the >32 cases exercise packed leaves inside a trie.
    for &n in &[0usize, 1, 2, 31, 32, 33, 64, 100, 1000] {
        let bs = bool_pattern(n, 0xC0FFEE ^ n as u64);
        let v = vec_of_bools(&bs);
        assert_eq!(op_vec_len(v) as usize, n, "len for n={n}");
        assert_eq!(vec_to_bools(v), bs, "elements for n={n}");
        assert_vec_invariants(v);
        // push one more, then update every third index to its negation — packed-aware push/update.
        let extra = n % 2 == 0;
        let mut v = op_vec_push(v, op_box_bool(extra));
        let mut want = bs.clone();
        want.push(extra);
        for i in (0..want.len()).step_by(3) {
            v = op_vec_update(v, i as u32, op_box_bool(!want[i]));
            want[i] = !want[i];
        }
        assert_eq!(vec_to_bools(v), want, "after push+updates for n={n}");
        assert_vec_invariants(v);
        op_drop(v);
    }
    assert_eq!(live_nodes(), before, "no leak across packed-bool sizes");
}

#[test]
fn packed_bool_persistence_old_version_unchanged() {
    reset();
    let before = live_nodes();
    // Update on a SHARED packed leaf path-copies: the old version is byte-identical afterward.
    let bs: Vec<bool> = (0..20).map(|i| i % 2 == 0).collect();
    let v0 = vec_of_bools(&bs);
    op_dup(v0); // share
    let v1 = op_vec_update(v0, 7, op_box_bool(!bs[7]));
    // v0 is unchanged; v1 has index 7 flipped.
    assert_eq!(
        vec_to_bools(v0),
        bs,
        "shared old version unchanged by update"
    );
    let mut want1 = bs.clone();
    want1[7] = !bs[7];
    assert_eq!(vec_to_bools(v1), want1, "new version reflects the update");
    op_drop(v0);
    op_drop(v1);
    assert_eq!(live_nodes(), before, "no leak across the shared update");
}

#[test]
fn packed_bool_of_arr_packs_a_list_literal() {
    reset();
    let before = live_nodes();
    // `vec-of-arr` of a ≤32 all-bool arr packs into ONE dense leaf. Footprint: header + packed leaf.
    let bs: Vec<bool> = (0..25).map(|i| (i * 7) % 5 < 2).collect();
    let a = arr_of_bools(&bs);
    assert_eq!(
        live_nodes(),
        before + 1,
        "just the arr node (bool immediates)"
    );
    let v = op_vec_of_arr(a);
    let (_, shift, root) = vec_read_header(v);
    assert_eq!(shift, 0);
    assert!(vec_leaf_is_packed(root), "of-arr produced a packed leaf");
    // header + packed leaf (the arr shell was dropped and replaced by the packed leaf) = before + 2.
    assert_eq!(
        live_nodes(),
        before + 2,
        "packed of-arr = header + one packed leaf"
    );
    assert_eq!(vec_to_bools(v), bs, "of-arr packed elements match");
    // Interchangeable with a push-built twin.
    let twin = vec_of_bools(&bs);
    assert_eq!(vec_to_bools(v), vec_to_bools(twin));
    op_drop(twin);
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn packed_bool_large_of_arr_all_leaves_packed() {
    reset();
    let before = live_nodes();
    // A >32 all-bool arr builds a trie; EVERY leaf is packed (Inc 3 density), read-back matches.
    let bs = bool_pattern(200, 0xABCD);
    let v = op_vec_of_arr(arr_of_bools(&bs));
    assert_eq!(op_vec_len(v) as usize, bs.len());
    assert_eq!(vec_to_bools(v), bs, "large of-arr elements match");
    assert_vec_invariants(v);
    // Walk the leaves: every level-0 node is packed.
    fn assert_all_leaves_packed(node: Handle, level: u32) {
        if level == 0 {
            assert!(vec_leaf_is_packed(node), "every bool leaf is packed");
            return;
        }
        for i in 0..vec_arity(node) {
            assert_all_leaves_packed(vec_child(node, i), level - VEC_BITS);
        }
    }
    let (_, shift, root) = vec_read_header(v);
    assert_all_leaves_packed(root, shift);
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn packed_bool_fbip_in_place_grows_and_sets() {
    reset();
    let before = live_nodes();
    // A UNIQUE bool vector's packed leaf grows and updates IN PLACE (no path-copy alloc): pushing
    // 30 bools onto a unique single-leaf vector never allocates a second leaf, and the whole thing
    // stays header + one packed leaf.
    let mut v = op_vec_empty();
    let mut want = Vec::new();
    for i in 0..30 {
        let b = i % 4 == 0;
        v = op_vec_push(v, op_box_bool(b));
        want.push(b);
    }
    assert_eq!(
        live_nodes(),
        before + 2,
        "unique packed push stays header + one packed leaf (in-place bit growth)"
    );
    // In-place updates on the unique leaf.
    for i in (1..30).step_by(5) {
        v = op_vec_update(v, i as u32, op_box_bool(!want[i]));
        want[i] = !want[i];
    }
    assert_eq!(
        live_nodes(),
        before + 2,
        "unique packed update mutates in place (no new node)"
    );
    assert_eq!(vec_to_bools(v), want, "in-place FBIP result matches oracle");
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn packed_bool_crossing_a_leaf_boundary_starts_a_new_packed_leaf() {
    reset();
    let before = live_nodes();
    // Push 40 bools: the 33rd crosses the 32-element leaf boundary, growing a level and starting a
    // SECOND packed leaf. Both leaves are packed; read-back matches; grows to shift VEC_BITS.
    let bs = bool_pattern(40, 0x5EED);
    let v = vec_of_bools(&bs);
    let (_, shift, root) = vec_read_header(v);
    assert_eq!(shift, VEC_BITS, "40 > 32 → one interior level");
    assert_eq!(vec_arity(root), 2, "two leaves under the root");
    assert!(vec_leaf_is_packed(vec_child(root, 0)), "first leaf packed");
    assert!(vec_leaf_is_packed(vec_child(root, 1)), "second leaf packed");
    assert_eq!(vec_to_bools(v), bs, "crossing-boundary elements match");
    assert_vec_invariants(v);
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn packed_bool_value_encode_round_trips() {
    reset();
    // The host boundary reads a List via ONLY `op_vec_len` + `op_vec_get`, and `op_vec_get` returns
    // an `imm_bool` for a packed leaf, so value-encode renders a packed `List Bool` element-by-element
    // exactly as it would an unpacked one. Assert the get/len walk denotes the same booleans.
    for &n in &[1usize, 32, 33, 100] {
        let bs = bool_pattern(n, 0xF00D ^ n as u64);
        let v = vec_of_bools(&bs);
        let rendered: Vec<bool> = (0..op_vec_len(v))
            .map(|i| op_get_bool(op_vec_get(v, i)))
            .collect();
        assert_eq!(rendered, bs, "value-encode get/len walk for n={n}");
        op_drop(v);
    }
}

#[test]
fn packed_bool_defensive_unpack_on_non_bool_element() {
    reset();
    let before = live_nodes();
    // Well-typed code never mixes a non-bool into a `List Bool`, but the leaf mutators stay TOTAL:
    // pushing a NON-bool onto a unique packed leaf unpacks it to a normal strict leaf and keeps the
    // sequence readable (as ints here, since we deliberately mix — a deterministic, not corrupt,
    // fallback). Build a packed leaf, then push a boxed int via the low-level in-place path.
    let mut v = op_vec_push(op_vec_empty(), op_box_bool(true));
    v = op_vec_push(v, op_box_bool(false));
    let (_, _, root) = vec_read_header(v);
    assert!(vec_leaf_is_packed(root), "starts packed");
    // Push an out-of-window boxed int (a real heap node) — forces the defensive unpack.
    let big = 1i64 << 40; // out of the fixnum window → a heap leaf
    v = op_vec_push(v, op_box_int(big));
    let (_, _, root) = vec_read_header(v);
    assert!(!vec_leaf_is_packed(root), "unpacked after a non-bool push");
    assert_eq!(vec_arity(root), 3, "three elements now");
    // Elements read back: two bools (as 1/0 via get-int on the imm bools is not meaningful; read the
    // first two as bools, the third as the int).
    assert!(op_get_bool(op_vec_get(v, 0)));
    assert!(!op_get_bool(op_vec_get(v, 1)));
    assert_eq!(op_get_int(op_vec_get(v, 2)), big);
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak across the defensive unpack");
}

#[test]
fn packed_bool_concat_stays_packed_and_matches_oracle() {
    reset();
    let before = live_nodes();
    // Concat two `List Bool`s across leaf-boundary sizes; the result reads back as the concatenation
    // AND every leaf stays packed (Inc 2 density — the leaf-merge and overflow-split rebuild pack).
    for &(na, nb) in &[
        (0usize, 5usize),
        (5, 0),
        (1, 1),
        (16, 16), // fits one leaf → leaf-merge path
        (20, 20), // > 32 → overflow-split into two packed leaves
        (31, 33),
        (32, 32),
        (100, 50), // multi-level relaxed join, boundary leaves packed
        (33, 100),
    ] {
        let ba = bool_pattern(na, 0x11 ^ na as u64);
        let bb = bool_pattern(nb, 0x22 ^ nb as u64);
        let a = vec_of_bools(&ba);
        let b = vec_of_bools(&bb);
        let c = op_vec_concat(a, b); // consumes a, b
        let mut want = ba.clone();
        want.extend(bb.iter().copied());
        assert_eq!(op_vec_len(c) as usize, want.len(), "concat len ({na},{nb})");
        assert_eq!(vec_to_bools(c), want, "concat elements ({na},{nb})");
        assert_vec_invariants(c);
        assert_all_bool_leaves_packed(c);
        op_drop(c);
    }
    assert_eq!(live_nodes(), before, "no leak across bool concats");
}

#[test]
fn packed_bool_split_stays_packed_and_matches_oracle() {
    reset();
    let before = live_nodes();
    // Split a `List Bool` at various indices; BOTH halves read back correctly and keep packed leaves.
    for &n in &[1usize, 8, 32, 33, 64, 100] {
        let bs = bool_pattern(n, 0x33 ^ n as u64);
        for &idx in &[0usize, 1, n / 2, n.saturating_sub(1), n] {
            let idx = idx.min(n);
            let v = vec_of_bools(&bs);
            let (l, r) = op_vec_split(v, idx as u32); // consumes v
            assert_eq!(op_vec_len(l) as usize, idx, "left len n={n} idx={idx}");
            assert_eq!(op_vec_len(r) as usize, n - idx, "right len n={n} idx={idx}");
            assert_eq!(
                vec_to_bools(l),
                bs[..idx].to_vec(),
                "left elems n={n} idx={idx}"
            );
            assert_eq!(
                vec_to_bools(r),
                bs[idx..].to_vec(),
                "right elems n={n} idx={idx}"
            );
            assert_vec_invariants(l);
            assert_vec_invariants(r);
            assert_all_bool_leaves_packed(l);
            assert_all_bool_leaves_packed(r);
            op_drop(l);
            op_drop(r);
        }
    }
    assert_eq!(live_nodes(), before, "no leak across bool splits");
}

#[test]
fn packed_bool_split_reconcat_roundtrips_packed() {
    reset();
    let before = live_nodes();
    // split then concat the halves back = the original (persistent-collection identity), still packed.
    let bs = bool_pattern(120, 0xBEEF);
    for &idx in &[1usize, 40, 64, 90] {
        let v = vec_of_bools(&bs);
        let (l, r) = op_vec_split(v, idx as u32);
        let back = op_vec_concat(l, r); // consumes both halves
        assert_eq!(
            vec_to_bools(back),
            bs,
            "split/reconcat identity at idx={idx}"
        );
        assert_vec_invariants(back);
        assert_all_bool_leaves_packed(back);
        op_drop(back);
    }
    assert_eq!(live_nodes(), before, "no leak");
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
    assert!(
        live < 1100,
        "final structure is bounded ({live} nodes), not O(chain length) leaked"
    );
    op_drop(v);
    assert_eq!(live_nodes(), baseline, "chain fully reclaims");
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
fn mark_immortal_census_excluded_and_dup_drop_noop() {
    let base = LIVE_NODES.with(|n| n.get());
    let s = bytes_leaf(&[1, 2, 3]);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base + 1,
        "one live node after building the static"
    );
    let s = op_mark_immortal(s);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "an immortal is excluded from the census (nets to zero, like IMM_UNIT)"
    );
    assert_eq!(node_rc(s), IMMORTAL, "rc is the IMMORTAL sentinel");
    op_dup(s);
    assert_eq!(
        node_rc(s),
        IMMORTAL,
        "dup is a no-op on an immortal (never retained)"
    );
    op_drop(s);
    assert_eq!(
        node_rc(s),
        IMMORTAL,
        "drop is a no-op on an immortal (never freed)"
    );
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "census unchanged across dup/drop of an immortal"
    );
    assert_eq!(
        bytes_to_vec(s),
        vec![1, 2, 3],
        "the immortal Bytes is readable intact after dup/drop (a consumer's bare global.get)"
    );
    let _ = op_mark_immortal(s);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "re-marking an already-immortal node does not double-decrement the census"
    );
}

#[test]
fn immortal_embedded_in_dropped_runtime_compound_survives() {
    let base = LIVE_NODES.with(|n| n.get());
    let stat = op_mark_immortal(bytes_leaf(&[7, 8]));
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "the hoisted immortal is census-excluded"
    );
    let tup = op_arr_alloc(2);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base + 1,
        "the runtime tuple shell is one live node"
    );
    op_arr_set(tup, 0, stat); // embed the immortal child (moved into the slot, rc untouched)
    op_arr_set(tup, 1, op_box_int(42)); // a scalar sibling (immediate — no node)
    op_drop(tup); // recursively drop the runtime parent
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "the tuple shell is freed; the immortal child is NOT (census back to base)"
    );
    assert_eq!(
        node_rc(stat),
        IMMORTAL,
        "the embedded immortal's rc is untouched by the parent's drop cascade"
    );
    assert_eq!(
        bytes_to_vec(stat),
        vec![7, 8],
        "the embedded immortal Bytes is readable intact after the parent drop (no UAF)"
    );
}

#[test]
fn mark_immortal_deep_covers_list_elements_and_map_kv() {
    // A >32-element LIST of HEAP elements → a multi-level RRB (interior + leaf nodes) with 40 element
    // leaves. Deep-marking must reach the element leaves, not just the trie structure.
    let base = LIVE_NODES.with(|n| n.get());
    let mut xs = op_vec_empty(); // an RRB VECTOR seed (`vec-push`-able); an `arr-alloc(0)` is the empty TUPLE (an immediate), which `vec-push` cannot read a header from
    for i in 0..40u32 {
        xs = op_vec_push(xs, bytes_leaf(&[i as u8]));
    }
    assert!(
        LIVE_NODES.with(|n| n.get()) > base,
        "the built list holds live nodes"
    );
    let xs = op_mark_immortal_deep(xs);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "deep-mark excludes the WHOLE list (trie nodes + all 40 element leaves) from the census"
    );
    assert_eq!(node_rc(xs), IMMORTAL, "the list root is immortal");
    assert_eq!(op_vec_len(xs), 40, "the immortal list is readable (len)");
    assert_eq!(
        node_rc(op_vec_get(xs, 17)),
        IMMORTAL,
        "an ELEMENT leaf is immortal too (deep, not just the spine) — the census-crux"
    );
    op_dup(xs);
    op_drop(xs);
    assert_eq!(
        node_rc(xs),
        IMMORTAL,
        "dup/drop no-op on the immortal list root"
    );
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "census unchanged across dup/drop of the deep-immortal list"
    );

    // A MAP with HEAP keys + HEAP values → a CHAMP with `[k,v]` data entries. Deep-marking must reach
    // the key AND value handles inside each entry, not just the HAMT nodes.
    let base2 = LIVE_NODES.with(|n| n.get());
    let mut m = op_map_empty();
    for i in 0..8u32 {
        m = op_map_insert(
            m,
            bytes_leaf(&[i as u8, 0xAA]),
            bytes_leaf(&[i as u8, 0xBB]),
        );
    }
    assert!(
        LIVE_NODES.with(|n| n.get()) > base2,
        "the built map holds live nodes"
    );
    let m = op_mark_immortal_deep(m);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base2,
        "deep-mark excludes the WHOLE map (HAMT nodes + every key + every value) from the census"
    );
    assert_eq!(node_rc(m), IMMORTAL, "the map root is immortal");
    assert_eq!(op_map_size(m), 8, "the immortal map is readable (size)");
    op_dup(m);
    op_drop(m);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base2,
        "census unchanged across dup/drop of the deep-immortal map"
    );
}

#[test]
fn mark_immortal_deep_is_dag_safe_over_shared_nodes() {
    let base = LIVE_NODES.with(|n| n.get());
    // A >32 multi-level RRB `xs`, then a push-derived `ys` that SHARES xs's untouched subtrees (a push
    // onto a shared/rc>1 root path-copies only the touched spine and dups+shares the rest). Holding
    // BOTH in a tuple makes those shared leaves reachable via two paths — a genuine heap DAG.
    let mut xs = op_vec_empty();
    for i in 0..40u32 {
        xs = op_vec_push(xs, bytes_leaf(&[i as u8]));
    }
    op_dup(xs); // keep xs live past the push (vec-push consumes its arg)
    let ys = op_vec_push(xs, bytes_leaf(&[0xFF])); // ys shares xs's untouched leaves (now rc 2)
    let tup = op_arr_alloc(2);
    op_arr_set(tup, 0, xs); // both owned by the tuple (moved into slots, rc untouched)
    op_arr_set(tup, 1, ys);
    assert!(
        LIVE_NODES.with(|n| n.get()) > base,
        "the DAG holds live nodes"
    );

    let tup = op_mark_immortal_deep(tup);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "census nets to base — every DISTINCT node (incl. the SHARED leaves) marked EXACTLY once, no double-decrement"
    );
    assert_eq!(node_rc(tup), IMMORTAL, "the tuple root is immortal");
    assert_eq!(
        node_rc(op_arr_get(tup, 0)),
        IMMORTAL,
        "the xs child is immortal"
    );
    assert_eq!(
        node_rc(op_arr_get(tup, 1)),
        IMMORTAL,
        "the ys child is immortal"
    );
    // Both lists stay readable THROUGH the shared, now-immortal leaves; a shared element (index < 40,
    // present in both) is itself immortal.
    assert_eq!(
        op_vec_len(op_arr_get(tup, 0)),
        40,
        "xs readable via the shared immortal nodes"
    );
    assert_eq!(
        op_vec_len(op_arr_get(tup, 1)),
        41,
        "ys readable via the shared immortal nodes"
    );
    assert_eq!(
        node_rc(op_vec_get(op_arr_get(tup, 1), 17)),
        IMMORTAL,
        "a SHARED element leaf (in both xs and ys) is immortal"
    );
}

#[test]
fn drop_of_mortal_shell_over_deep_immortal_leaves_the_immortal_intact() {
    let base = LIVE_NODES.with(|n| n.get());
    let mut xs = op_vec_empty();
    for i in 0..40u32 {
        xs = op_vec_push(xs, bytes_leaf(&[i as u8]));
    }
    let xs = op_mark_immortal_deep(xs);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "the deep-immortal list is census-excluded"
    );
    // A MORTAL tuple wrapping the immortal list (+ a scalar sibling). The shell is one live node.
    let tup = op_arr_alloc(2);
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base + 1,
        "the mortal tuple shell is one live node"
    );
    op_arr_set(tup, 0, xs); // embed the immortal (moved in, rc untouched)
    op_arr_set(tup, 1, op_box_int(7)); // an immediate scalar sibling (no node)
    op_drop(tup); // cascade drops the shell; the immortal child is skipped, not recursed/freed
    assert_eq!(
        LIVE_NODES.with(|n| n.get()),
        base,
        "only the shell is freed (census back to base); the whole immortal list survives untouched"
    );
    assert_eq!(
        node_rc(xs),
        IMMORTAL,
        "the immortal list root survived the shell drop"
    );
    assert_eq!(
        op_vec_len(xs),
        40,
        "the immortal list is still fully readable after the shell drop"
    );
    assert_eq!(
        node_rc(op_vec_get(xs, 23)),
        IMMORTAL,
        "an element leaf survived (not freed under the shell)"
    );
}

#[test]
#[should_panic(expected = "use-after-free")]
fn double_drop_is_caught_as_use_after_free() {
    let h = bytes_leaf(&[1, 2, 3]);
    op_drop(h); // last ref → frees, poisoning the retained cell
    op_drop(h); // the double-free: drop of a freed node → UAF panic
}

#[test]
#[should_panic(expected = "use-after-free")]
fn dup_after_free_is_caught_as_use_after_free() {
    let h = bytes_leaf(&[4, 5]);
    op_drop(h);
    op_dup(h); // dup of a freed node → UAF panic
}

#[test]
#[should_panic(expected = "use-after-free")]
fn read_after_free_is_caught_as_use_after_free() {
    let h = bytes_leaf(&[6, 7]);
    op_drop(h);
    let _ = node_rc(h); // read of a freed node → UAF panic
}

#[test]
#[should_panic(expected = "use-after-free")]
fn read_after_free_through_a_direct_getter_is_caught_as_use_after_free() {
    let arr = op_arr_alloc(2);
    op_drop(arr); // last ref → frees the array node, poisoning the retained cell
    let _ = op_arr_get(arr, 0); // direct getter deref of a freed node → UAF panic via node_ref
}

#[test]
fn tuple_payload_spine_reclaim_single_cascade_vs_materialized_tuple_box() {
    // ---- (A) borrow-only reads → single cascading drop frees both boxes ----
    let base = live_nodes();
    let head_a = op_box_int(7);
    assert!(
        is_immediate(head_a),
        "small int is an immediate — no head box in this shape"
    );
    let tail_a = bytes_leaf(&[1, 2, 3]); // +1 node (the carried tail, standing in for IntList)
    let tup_a = op_arr_alloc(2); // +1 node (tuple box)
    op_arr_set(tup_a, 0, head_a);
    op_arr_set(tup_a, 1, tail_a);
    let cons_a = op_sum_new(0, tup_a); // +1 node (Cons owns the tuple box)
    assert_eq!(live_nodes() - base, 3, "Cons + tuple box + tail");
    // Reads mirror the emit for `(smt t (+ acc h))` — both BORROW, no rc bump on the tuple box.
    let t_read = op_arr_get(op_sum_payload(cons_a), 1);
    let _h_read = op_arr_get(op_sum_payload(cons_a), 0); // head path (second SumPayload nav)
    op_dup(t_read); // carry the tail forward before the reclaim drop
    op_drop(cons_a); // single cascading drop
    assert_eq!(
        live_nodes() - base,
        1,
        "(A) one cascading drop of Cons reclaims BOTH boxes; only the dup'd tail survives"
    );
    op_drop(t_read);
    assert_eq!(live_nodes() - base, 0, "(A) tail reclaimed → balanced");

    // ---- (B) tuple box materialized (rc 2) → single drop leaks it; explicit drop reclaims ----
    let base_b = live_nodes();
    let head_b = op_box_int(7);
    let tail_b = bytes_leaf(&[4, 5, 6]);
    let tup_b = op_arr_alloc(2);
    op_arr_set(tup_b, 0, head_b);
    op_arr_set(tup_b, 1, tail_b);
    let cons_b = op_sum_new(0, tup_b);
    // The arm materializes (owns) the tuple box: an extra dup → tuple box rc 1→2.
    let tup_owned = op_sum_payload(cons_b); // borrow → tuple box handle
    op_dup(tup_owned); // materialize
    let t_read_b = op_arr_get(tup_owned, 1);
    op_dup(t_read_b); // carry the tail
    op_drop(cons_b); // cascades: tuple box 2→1 (NOT freed) → LEAK
    assert_eq!(
        live_nodes() - base_b,
        2,
        "(B) tuple box materialized (rc 2) → one drop of Cons leaves it at rc 1: tuple box + tail leak"
    );
    // The emit must ALSO drop the materialized ref — SOUND (matches the dup), not a double-free.
    op_drop(tup_owned); // tuple box 1→0 → free, cascade drops tail 2→1
    assert_eq!(
        live_nodes() - base_b,
        1,
        "(B) explicit drop of the materialized tuple-box ref reclaims it; tail survives"
    );
    op_drop(t_read_b);
    assert_eq!(live_nodes() - base_b, 0, "(B) balanced");
}

#[test]
fn tuple_payload_boxed_head_cascade_free_is_sound_unique_and_escaped() {
    const BIG: i64 = 1 << 40; // out of the fixnum window → a boxed head node
    // (1) unique boxed head → cascade-freed with the tuple box.
    let base = live_nodes();
    let head = op_box_int(BIG);
    assert!(
        !is_immediate(head),
        "a large int must be a boxed node (a real head cell)"
    );
    let tail = bytes_leaf(&[1, 2, 3]);
    let tup = op_arr_alloc(2);
    op_arr_set(tup, 0, head);
    op_arr_set(tup, 1, tail);
    let cons = op_sum_new(0, tup);
    assert_eq!(
        live_nodes() - base,
        4,
        "Cons + tuple box + boxed head + tail"
    );
    op_dup(op_sum_payload(cons)); // materialize the tuple box (rc 2), mirroring the arm
    let t = op_arr_get(op_sum_payload(cons), 1);
    op_dup(t); // carry the tail
    op_drop(op_sum_payload(cons)); // drop the materialized tuple-box ref (rc 2→1)
    op_drop(cons); // free Cons → cascade frees tuple box (1→0) → boxed head + tail(dup-protected)
    assert_eq!(
        live_nodes() - base,
        1,
        "(1) unique boxed head cascade-freed with the tuple box; only the dup'd tail survives"
    );
    op_drop(t);
    assert_eq!(live_nodes() - base, 0, "(1) balanced");

    // (2) ESCAPED boxed head (a proper owned dup, rc 2) → survives the cascade.
    let base2 = live_nodes();
    let head2 = op_box_int(BIG);
    let tail2 = bytes_leaf(&[4, 5, 6]);
    let tup2 = op_arr_alloc(2);
    op_arr_set(tup2, 0, head2);
    op_arr_set(tup2, 1, tail2);
    let cons2 = op_sum_new(0, tup2);
    // Read the head via borrow-projection, then OWN it (a proper escape dups): head rc 1→2.
    let escaped_head = op_arr_get(op_sum_payload(cons2), 0);
    op_dup(escaped_head);
    let t2 = op_arr_get(op_sum_payload(cons2), 1);
    op_dup(t2); // carry the tail
    op_drop(cons2); // cascade: tuple box 1→0 free → head2 (2→1 SURVIVES) + tail2 (dup-protected)
    assert_eq!(
        live_nodes() - base2,
        2,
        "(2) escaped boxed head (rc 2) SURVIVES the cascade; head + tail remain"
    );
    op_drop(escaped_head); // release the escapee → head freed
    op_drop(t2);
    assert_eq!(
        live_nodes() - base2,
        0,
        "(2) balanced — no premature free, no leak"
    );
}

#[test]
fn empty_vec_is_a_shared_immortal_singleton_never_mutated_by_push() {
    let base = live_nodes();
    let e1 = op_vec_empty();
    let e2 = op_vec_empty();
    assert_eq!(e1.0, e2.0, "op_vec_empty returns the SAME shared singleton");
    assert_eq!(op_vec_len(e1), 0, "the singleton is empty");
    // Immortal → census-EXCLUDED: minting it did not raise the live count.
    assert_eq!(
        live_nodes() - base,
        0,
        "the immortal empty-vec is not counted as live"
    );
    // Push takes the COPY path (rc = IMMORTAL != 1): a FRESH vec, the singleton untouched.
    let pushed = op_vec_push(e1, op_box_int(7));
    assert_ne!(
        pushed.0, e1.0,
        "push on the immortal empty builds a FRESH vec, not in-place"
    );
    assert_eq!(
        op_vec_len(pushed),
        1,
        "the fresh vec carries the pushed element"
    );
    // The singleton is UNCHANGED — still the same node, still empty (no shared corruption).
    assert_eq!(
        op_vec_empty().0,
        e1.0,
        "the singleton is still the same node"
    );
    assert_eq!(
        op_vec_len(e1),
        0,
        "the singleton is still EMPTY (the push did not mutate it)"
    );
    // TWO INDEPENDENT pushes off the SAME shared empty must NOT alias — the flag-day soundness
    // witness: a mutated-shared-immortal would make the second push see the first's element (or the
    // two results alias). Each push path-copies off the immortal, so the results are distinct vecs.
    let pushed2 = op_vec_push(op_vec_empty(), op_box_int(9));
    assert_ne!(
        pushed2.0, pushed.0,
        "two independent pushes off the shared empty do NOT alias"
    );
    assert_eq!(
        op_vec_len(pushed2),
        1,
        "the second result is its own 1-element vec"
    );
    assert_eq!(
        op_get_int(op_vec_get(pushed, 0)),
        7,
        "first result still carries 7 (not clobbered)"
    );
    assert_eq!(
        op_get_int(op_vec_get(pushed2, 0)),
        9,
        "second result carries 9"
    );
    assert_eq!(
        op_vec_len(op_vec_empty()),
        0,
        "the shared empty is STILL empty after both pushes"
    );
    op_drop(pushed); // frees the fresh vecs; any drop of the immortal is a no-op
    op_drop(pushed2);
    assert_eq!(
        live_nodes() - base,
        0,
        "balanced — only the census-excluded immortal remains"
    );
}

#[test]
fn empty_collection_constructors_are_shared_immortal_singletons() {
    let base = live_nodes();
    assert_eq!(
        op_map_empty().0,
        op_map_empty().0,
        "empty map is a shared singleton"
    );
    assert_eq!(
        op_set_empty().0,
        op_set_empty().0,
        "empty set is a shared singleton"
    );
    assert_eq!(
        op_bytes_alloc(0).0,
        op_bytes_alloc(0).0,
        "empty bytes is a shared singleton"
    );
    assert_eq!(
        op_str_new(String::new()).0,
        op_str_new(String::new()).0,
        "empty string is a shared singleton"
    );
    assert_eq!(
        live_nodes() - base,
        0,
        "the four empty singletons are census-excluded (immortal — not counted live)"
    );
    // Insert onto the immortal empty map/set path-copies (rc=IMMORTAL != 1): a FRESH node, and the
    // shared empty singleton is left untouched.
    let m = op_map_insert(op_map_empty(), op_box_int(1), op_box_int(2));
    assert_ne!(
        m.0,
        op_map_empty().0,
        "map insert builds a FRESH map off the immortal empty"
    );
    assert_eq!(op_map_size(m), 1, "the fresh map carries the entry");
    assert_eq!(
        op_map_size(op_map_empty()),
        0,
        "the immortal empty map is STILL empty (not mutated)"
    );
    let s = op_set_insert(op_set_empty(), op_box_int(7));
    assert_ne!(
        s.0,
        op_set_empty().0,
        "set insert builds a FRESH set off the immortal empty"
    );
    assert_eq!(op_set_size(s), 1, "the fresh set carries the element");
    assert_eq!(
        op_set_size(op_set_empty()),
        0,
        "the immortal empty set is STILL empty"
    );
    assert_eq!(
        op_bytes_len(op_bytes_alloc(0)),
        0,
        "the empty bytes singleton stays empty"
    );
    op_drop(m);
    op_drop(s);
    assert_eq!(
        live_nodes() - base,
        0,
        "balanced — only the census-excluded immortal empties remain"
    );
}

#[test]
fn vec_prepend_builds_correct_multilevel_and_reclaims() {
    let base = live_nodes();
    let n: i64 = 100; // > 32 → multi-level (interior relaxed root exercised)
    let mut v = op_vec_empty();
    for i in 0..n {
        v = op_vec_prepend(v, op_box_int(i));
    }
    assert_eq!(op_vec_len(v) as i64, n, "prepend built an n-element list");
    // Prepend puts the last-added element at index 0, the first at index n-1.
    assert_eq!(
        op_get_int(op_vec_get(v, 0)),
        n - 1,
        "index 0 = the last-prepended element"
    );
    assert_eq!(
        op_get_int(op_vec_get(v, (n - 1) as u32)),
        0,
        "index n-1 = the first-prepended element"
    );
    let mut sum = 0i64;
    for idx in 0..n {
        sum += op_get_int(op_vec_get(v, idx as u32));
    }
    assert_eq!(
        sum,
        (0..n).sum(),
        "every element is present + readable (sum matches)"
    );
    op_drop(v);
    assert_eq!(
        live_nodes() - base,
        0,
        "NO LEAK — each intermediate prepend version was reclaimed, and the final list frees clean"
    );
}

#[test]
fn hash_blake3_matches_the_blake3_crate() {
    reset();
    let before = live_nodes();

    // (1) A flat leaf → the crate's digest, exactly 32 bytes.
    let input: &[u8] = b"cadenza contract declaration";
    let leaf = bytes_leaf(input);
    let digest = op_hash_blake3(leaf);
    assert_eq!(op_bytes_len(digest), 32, "a blake3 digest is 32 bytes");
    assert_eq!(
        bytes_to_vec(digest),
        blake3::hash(input).as_bytes().to_vec(),
        "hash-blake3 must equal blake3::hash of the same bytes"
    );
    op_drop(leaf); // safe BECAUSE the op only borrowed it (a consumed input would double-free here)
    op_drop(digest);

    // (2) A ROPE input hashes as its FLATTENED bytes (the op reads logically via the index accessors).
    let rope = op_bytes_concat(bytes_leaf(&[1u8, 2, 3]), bytes_leaf(&[4u8, 5]));
    let d_rope = op_hash_blake3(rope);
    assert_eq!(
        bytes_to_vec(d_rope),
        blake3::hash(&[1u8, 2, 3, 4, 5]).as_bytes().to_vec(),
        "a rope input hashes identically to its flattened byte sequence"
    );
    op_drop(rope);
    op_drop(d_rope);

    // (3) The empty input hashes to blake3's defined empty-input digest; never traps.
    let empty = op_bytes_alloc(0);
    let d_empty = op_hash_blake3(empty);
    assert_eq!(
        bytes_to_vec(d_empty),
        blake3::hash(b"").as_bytes().to_vec(),
        "empty input → blake3's empty-string digest"
    );
    op_drop(empty);
    op_drop(d_empty);

    assert_eq!(
        live_nodes(),
        before,
        "no leak — the op borrows its input and every handle is released"
    );
}

#[test]
fn ast_print_renders_canonical_sexpr_text() {
    reset();
    let discs = bytes_leaf(&[0, 1, 2, 3, 4, 5, 6]); // int,float,bool,str,name,bytes,list

    // (+ 1 2): Ast.List(6) [Ast.Name(4,"+"), Ast.Int(0,1), Ast.Int(0,2)] — the print_ast_value example.
    // The list payload is a persistent RRB VECTOR (built with `vec-*`, exactly as the compiler lowers a
    // `(list …)`), NOT an `arr-*` tuple — a tuple-built list would pass with the wrong `arr-*` reader and
    // mask the RRB-vs-tuple accessor bug (v-cp's nested-element repro below is the regression lock).
    let name = op_sum_new(4, op_str_new("+".to_string()));
    let i1 = op_sum_new(0, op_bigint_of_i64(1));
    let i2 = op_sum_new(0, op_bigint_of_i64(2));
    let mut vec = op_vec_empty();
    vec = op_vec_push(vec, name);
    vec = op_vec_push(vec, i1);
    vec = op_vec_push(vec, i2);
    let list = op_sum_new(6, vec);
    let out = op_ast_print(list, discs);
    assert_eq!(
        op_str_get(out),
        "(+ 1 2)",
        "List of Name + two Ints → (+ 1 2)"
    );
    op_drop(list);
    op_drop(out);

    // v-cp's nested-element repro (regression lock): a SINGLE-element list Ast.List[Ast.Int 5] → "(5)"
    // (was "(0)" when the List arm read the RRB vec with `arr-*`).
    let v1 = op_vec_push(op_vec_empty(), op_sum_new(0, op_bigint_of_i64(5)));
    let single = op_sum_new(6, v1);
    let o = op_ast_print(single, discs);
    assert_eq!(
        op_str_get(o),
        "(5)",
        "single-element list reads its element via vec-get, not arr-get"
    );
    op_drop(single);
    op_drop(o);

    // A NESTED list Ast.List[Ast.List[Ast.Name f, Ast.Int 2]] → "((f 2))" — the recursion + vec read.
    let mut inner = op_vec_empty();
    inner = op_vec_push(inner, op_sum_new(4, op_str_new("f".to_string())));
    inner = op_vec_push(inner, op_sum_new(0, op_bigint_of_i64(2)));
    let outer = op_vec_push(op_vec_empty(), op_sum_new(6, inner));
    let nested = op_sum_new(6, outer);
    let o = op_ast_print(nested, discs);
    assert_eq!(
        op_str_get(o),
        "((f 2))",
        "nested list recurses and reads each element via vec-get"
    );
    op_drop(nested);
    op_drop(o);

    // Ast.Name → the bare word.
    let nm = op_sum_new(4, op_str_new("foo".to_string()));
    let o = op_ast_print(nm, discs);
    assert_eq!(op_str_get(o), "foo");
    op_drop(nm);
    op_drop(o);

    // Ast.Bool(true) → "true".
    let bt = op_sum_new(2, op_box_bool(true));
    let o = op_ast_print(bt, discs);
    assert_eq!(op_str_get(o), "true");
    op_drop(bt);
    op_drop(o);

    // Ast.Str with a quote + newline → the escaped `"…"` literal (closed set \n \t \r \\ \").
    let st = op_sum_new(3, op_str_new("a\"b\nc".to_string()));
    let o = op_ast_print(st, discs);
    assert_eq!(
        op_str_get(o),
        "\"a\\\"b\\nc\"",
        "Str renders as an escaped double-quoted literal"
    );
    op_drop(st);
    op_drop(o);

    op_drop(discs);
}

#[test]
fn ast_encode_matches_builder_codec_bytes() {
    reset();
    // 9-disc descriptor [int,float,bool,str,name,list,bytes,char,symbol] = discs 0..=8.
    let discs = bytes_leaf(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]); // M2: 16 discs (+ ctor discs list_ctor..member)

    // Heap Ast: (List [Name "f", Int 300, Int -3, Bool true, Str "hi", Bytes b"\x00\xff", Char 'λ',
    // Sym "m", Float 1.5, List [Int 7]]) — covers every variant incl. multi-byte + negative Int + nesting.
    let mut v = op_vec_empty();
    v = op_vec_push(v, op_sum_new(4, op_str_new("f".to_string())));
    v = op_vec_push(v, op_sum_new(0, op_bigint_of_i64(300)));
    v = op_vec_push(v, op_sum_new(0, op_bigint_of_i64(-3)));
    v = op_vec_push(v, op_sum_new(2, op_box_bool(true)));
    v = op_vec_push(v, op_sum_new(3, op_str_new("hi".to_string())));
    v = op_vec_push(v, op_sum_new(6, bytes_leaf(&[0, 255])));
    v = op_vec_push(v, op_sum_new(7, op_box_int('λ' as i64)));
    v = op_vec_push(v, op_sum_new(8, op_str_new("m".to_string())));
    v = op_vec_push(v, op_sum_new(1, op_box_float(1.5)));
    let inner = op_vec_push(op_vec_empty(), op_sum_new(0, op_bigint_of_i64(7)));
    v = op_vec_push(v, op_sum_new(5, inner));
    let root = op_sum_new(5, v);
    let got = op_ast_encode(root, discs);
    let got_bytes = bytes_to_vec(got);

    // Same Ast built directly through the shared Builder + codec (the compile-fold path).
    let idec = |n: i64| crate::ast::Leaf::Int {
        value: crate::ast::IntValue::from_i64(n),
        radix: crate::ast::Radix::Dec,
    };
    let mut b = crate::ast::Builder::new();
    let e_name = b.atom_leaf(crate::ast::Leaf::Name("f".into()));
    let e_300 = b.atom_leaf(idec(300));
    let e_neg3 = b.atom_leaf(idec(-3));
    let e_bool = b.atom_leaf(crate::ast::Leaf::Bool(true));
    let e_str = b.atom_leaf(crate::ast::Leaf::Str("hi".into()));
    let e_bytes = b.atom_leaf(crate::ast::Leaf::Bytes(alloc::vec![0, 255].into()));
    let e_char = b.atom_leaf(crate::ast::Leaf::Char('λ'));
    let e_sym = b.atom_leaf(crate::ast::Leaf::Sym("m".into()));
    let e_float = b.atom_leaf(crate::ast::Leaf::Float(
        crate::ast::Decimal::from_f64(1.5).unwrap(),
    ));
    let e_inner_7 = b.atom_leaf(idec(7));
    let e_inner = b.list(alloc::vec![e_inner_7]);
    let root_b = b.list(alloc::vec![
        e_name, e_300, e_neg3, e_bool, e_str, e_bytes, e_char, e_sym, e_float, e_inner
    ]);
    let want_bytes = crate::codec::encode(&b.finish(root_b));

    assert_eq!(
        got_bytes, want_bytes,
        "runtime ast-encode bytes must equal the Builder+codec::encode of the same Ast"
    );
    op_drop(root);
    op_drop(got);
    op_drop(discs);
}

#[test]
fn ast_decode_round_trips_encode() {
    reset();
    let discs = bytes_leaf(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]); // M2: 16 discs (+ ctor discs list_ctor..member)

    // Same all-variant Ast as the encode test.
    let mut v = op_vec_empty();
    v = op_vec_push(v, op_sum_new(4, op_str_new("f".to_string())));
    v = op_vec_push(v, op_sum_new(0, op_bigint_of_i64(300)));
    v = op_vec_push(v, op_sum_new(0, op_bigint_of_i64(-3)));
    v = op_vec_push(v, op_sum_new(2, op_box_bool(true)));
    v = op_vec_push(v, op_sum_new(3, op_str_new("hi".to_string())));
    v = op_vec_push(v, op_sum_new(6, bytes_leaf(&[0, 255])));
    v = op_vec_push(v, op_sum_new(7, op_box_int('λ' as i64)));
    v = op_vec_push(v, op_sum_new(8, op_str_new("m".to_string())));
    v = op_vec_push(v, op_sum_new(1, op_box_float(1.5)));
    let inner = op_vec_push(op_vec_empty(), op_sum_new(0, op_bigint_of_i64(7)));
    v = op_vec_push(v, op_sum_new(5, inner));
    let root = op_sum_new(5, v);

    let enc1 = op_ast_encode(root, discs);
    let enc1_bytes = bytes_to_vec(enc1);
    let decoded = op_ast_decode(enc1, discs);
    assert_ne!(
        decoded,
        Handle::NULL,
        "a well-formed cdzast document decodes to an Ast, not NULL"
    );
    let enc2 = op_ast_encode(decoded, discs);
    assert_eq!(
        bytes_to_vec(enc2),
        enc1_bytes,
        "encode(decode(encode v)) must equal encode(v) — decode rebuilt the same Ast"
    );

    // A malformed byte sequence (wrong header) decodes to NULL — the Err path, no trap.
    let junk = bytes_leaf(&[1, 2, 3, 4]);
    assert_eq!(
        op_ast_decode(junk, discs),
        Handle::NULL,
        "a non-cdzast byte sequence decodes to NULL (Err), never a trap"
    );

    op_drop(root);
    op_drop(enc1);
    op_drop(decoded);
    op_drop(enc2);
    op_drop(junk);
    op_drop(discs);
}

#[test]
fn ast_encode_decode_round_trips_compound_ctor_forms() {
    reset();
    let discs = bytes_leaf(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
    // disc positions: int=0 bool=2 str=3 name=4; list_ctor=9 tuple_ctor=10 record_ctor=11 map_ctor=12
    // set_ctor=13 field_pair=14 member=15.
    let int = |n: i64| op_sum_new(0, op_bigint_of_i64(n));
    let name = |s: &str| op_sum_new(4, op_str_new(s.to_string()));
    let strv = |s: &str| op_sum_new(3, op_str_new(s.to_string()));
    // FieldPair/Member payload = a 2-elem `arr` (Tuple Ast Ast).
    let pair = |disc: u32, a: Handle, c: Handle| {
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, a);
        op_arr_set(t, 1, c);
        op_sum_new(disc, t)
    };
    // Collection payload = a `(List Ast)` RRB vector.
    let vecof = |elems: &[Handle]| {
        let mut v = op_vec_empty();
        for &e in elems {
            v = op_vec_push(v, e);
        }
        v
    };

    // root = TupleCtor[ ListCtor[1,2], RecordCtor[a=7, b=true], MapCtor[3="x"], SetCtor[5,9], Member(obj,key) ]
    let listc = op_sum_new(9, vecof(&[int(1), int(2)]));
    let rec = op_sum_new(
        11,
        vecof(&[
            pair(14, name("a"), int(7)),
            pair(14, name("b"), op_sum_new(2, op_box_bool(true))),
        ]),
    );
    let mp = op_sum_new(12, vecof(&[pair(14, int(3), strv("x"))]));
    let setc = op_sum_new(13, vecof(&[int(5), int(9)]));
    let mem = pair(15, name("obj"), name("key"));
    let root = op_sum_new(10, vecof(&[listc, rec, mp, setc, mem]));

    let enc1 = op_ast_encode(root, discs);
    let enc1_bytes = bytes_to_vec(enc1);
    assert!(
        !enc1_bytes.is_empty(),
        "op93 must encode a compound-ctor Ast (was EMPTY before the ctor encode arms)"
    );

    // (1) byte-identity with the shared Builder+codec path (post-order emit matches the heap walk).
    let idec = |n: i64| crate::ast::Leaf::Int {
        value: crate::ast::IntValue::from_i64(n),
        radix: crate::ast::Radix::Dec,
    };
    use crate::ast::CompoundCtor as C;
    let mut b = crate::ast::Builder::new();
    let l1 = b.atom_leaf(idec(1));
    let l2 = b.atom_leaf(idec(2));
    let listc_b = b.compound(C::List, &[l1, l2]);
    let na = b.atom_leaf(crate::ast::Leaf::Name("a".into()));
    let v7 = b.atom_leaf(idec(7));
    let fpa = b.field_pair(na, v7);
    let nb = b.atom_leaf(crate::ast::Leaf::Name("b".into()));
    let vt = b.atom_leaf(crate::ast::Leaf::Bool(true));
    let fpb = b.field_pair(nb, vt);
    let rec_b = b.compound(C::Record, &[fpa, fpb]);
    let k3 = b.atom_leaf(idec(3));
    let sx = b.atom_leaf(crate::ast::Leaf::Str("x".into()));
    let fpm = b.field_pair(k3, sx);
    let mp_b = b.compound(C::Map, &[fpm]);
    let s5 = b.atom_leaf(idec(5));
    let s9 = b.atom_leaf(idec(9));
    let setc_b = b.compound(C::Set, &[s5, s9]);
    let mobj = b.atom_leaf(crate::ast::Leaf::Name("obj".into()));
    let mkey = b.atom_leaf(crate::ast::Leaf::Name("key".into()));
    let mem_b = b.member(mobj, mkey);
    let root_b = b.compound(C::Tuple, &[listc_b, rec_b, mp_b, setc_b, mem_b]);
    let want_bytes = crate::codec::encode(&b.finish(root_b));
    assert_eq!(
        enc1_bytes, want_bytes,
        "runtime op93 encode of the compound-ctor Ast must equal the shared Builder+codec form \
         (the compile-time Ast.encode fold) — head-first ctor leaves, byte-for-byte"
    );

    // (2) round-trips through op94 decode.
    let decoded = op_ast_decode(enc1, discs);
    assert_ne!(
        decoded,
        Handle::NULL,
        "a compound-ctor cdzast document decodes to an Ast, not NULL"
    );
    let enc2 = op_ast_encode(decoded, discs);
    assert_eq!(
        bytes_to_vec(enc2),
        enc1_bytes,
        "encode(decode(encode v)) == encode(v) — decode rebuilt the same compound-ctor Ast"
    );

    op_drop(root);
    op_drop(enc1);
    op_drop(decoded);
    op_drop(enc2);
    op_drop(discs);
}

#[test]
fn ast_encode_decode_non_finite_floats_round_trip() {
    reset();
    let discs = bytes_leaf(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]); // M2: 16 discs (+ ctor discs list_ctor..member)
    let check = |f: f64, leaf: crate::ast::Leaf| {
        // heap Ast.Float(f) → op93 encode
        let node = op_sum_new(1, op_box_float(f)); // disc 1 = float, per the descriptor above
        let enc = op_ast_encode(node, discs);
        // byte-identity with the shared Builder+codec of `leaf` (the compile-fold form)
        let mut b = crate::ast::Builder::new();
        let root = b.atom_leaf(leaf);
        let want = crate::codec::encode(&b.finish(root));
        assert_eq!(
            bytes_to_vec(enc),
            want,
            "op93 non-finite encode == compile-fold codec bytes"
        );
        // decode back → Ast.Float with the SAME non-finite value
        let dec = op_ast_decode(enc, discs);
        assert_ne!(dec, Handle::NULL);
        let got = op_get_float(op_sum_payload(dec));
        if f.is_nan() {
            assert!(got.is_nan(), "NaN round-trips");
        } else {
            assert_eq!(got, f, "±inf round-trips");
        }
        op_drop(node);
        op_drop(enc);
        op_drop(dec);
    };
    check(f64::NAN, crate::ast::Leaf::FloatNan);
    check(
        f64::INFINITY,
        crate::ast::Leaf::FloatInf { negative: false },
    );
    check(
        f64::NEG_INFINITY,
        crate::ast::Leaf::FloatInf { negative: true },
    );
    op_drop(discs);
}

#[test]
fn rope_concat_allocates_one_node_no_copy() {
    reset();
    // O(1): concatenation adds exactly one concat node, copies no bytes into new leaves.
    let x = bytes_leaf(&[0; 50]);
    let y = bytes_leaf(&[1; 50]);
    let before = live_nodes();
    let c = op_bytes_concat(x, y);
    assert_eq!(
        live_nodes(),
        before + 1,
        "concat = one node, not 100 byte copies"
    );
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
    assert_eq!(
        vec_arity(child),
        0,
        "collapsed slice points straight at the leaf parent"
    );
    assert_eq!(
        with_node(s2, 99, |n| read_u32_at(&n.raw, 0)),
        2,
        "offset collapsed to 1+1"
    );
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
    assert_eq!(
        live_nodes(),
        before,
        "whole rope (concats, slice, leaves) reclaimed"
    );
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
fn rope_dup_retained_operand_survives_being_consumed_by_concat() {
    reset();
    let before = live_nodes();
    let e = op_str_new(String::from("hello")); // the shared string operand
    op_dup(e); // the compiler's dup-retain: rc=2 (one ref for the concat, one for the later use)
    let x = op_str_new(String::from("!"));
    let rope = op_bytes_concat(e, x); // consumes ONE `e` ref + `x` → "hello!"
    // The LATER live use reads the ORIGINAL `e` directly — it must be intact, NOT corrupted/freed.
    assert_eq!(
        op_str_get(e),
        "hello",
        "the dup-retained original reads its full content after a ref was consumed by concat"
    );
    assert_eq!(
        op_bytes_len(e),
        5,
        "…and its byte-len is unchanged (the concat consumed a separate ref, not this one)"
    );
    // The rope built from the other ref is correct too.
    assert_eq!(
        op_str_get(rope),
        "hello!",
        "the concat rope reads both operands"
    );
    op_drop(e);
    op_drop(rope);
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free: the consumed ref lives in the rope, the retained ref freed here"
    );
}

#[test]
fn rope_compact_materializes_and_releases_parent() {
    reset();
    // #Retained Storage: a small slice of a large parent pins the whole parent; compact
    // materializes the sub-range into an independent leaf and drops the parent, freeing it.
    let before = live_nodes();
    let parent = bytes_leaf(&[0u8; 1000]); // one large leaf
    let s = op_bytes_slice(parent, 10, 3); // pins the 1000-byte parent
    assert_eq!(
        live_nodes(),
        before + 2,
        "large parent + slice node both live"
    );
    let c = op_bytes_compact(s); // flatten → independent 3-byte leaf, parent released
    assert_eq!(c, s, "compact returns the same handle, now a leaf");
    assert_eq!(vec_arity(c), 0, "compacted to a leaf");
    assert_eq!(op_bytes_len(c), 3);
    assert_eq!(
        live_nodes(),
        before + 1,
        "the 1000-byte parent was released by compact"
    );
    op_drop(c);
    assert_eq!(live_nodes(), before);
}

#[test]
fn compact_makes_a_rope_key_canonical_champ_eq_and_hash_match_the_flat_twin() {
    reset();
    let before = live_nodes();
    let content = b"the-quick-brown-fox"; // > INLINE_RAW_CAP so the flat twin is a Heap leaf too
    let flat = bytes_leaf(content);
    // A rope of the SAME content, split across a seam mid-word.
    let rope = op_bytes_concat(bytes_leaf(&content[..7]), bytes_leaf(&content[7..]));
    // (1) TRIPWIRE: before compaction the rope is a DIFFERENT physical shape → NOT champ_eq to the flat
    // twin (this is WHY the compiler must compact a rope key; a raw rope key would mis-key).
    assert!(
        !champ_eq(rope, flat),
        "a rope is champ_eq-DISTINCT from its flat twin (physical-bytes compare — the reason keys are compacted)"
    );
    // (2) THE GUARANTEE: compact the rope → now champ_eq AND champ_hash-identical to the flat twin, so
    // the compiler's compact-before-insert genuinely canonicalizes a String/Bytes key.
    let compacted = op_bytes_compact(rope);
    assert!(
        champ_eq(compacted, flat),
        "a COMPACTED rope is champ_eq to its flat twin — the runtime half of the key-canonicalization contract"
    );
    assert_eq!(
        champ_hash(compacted),
        champ_hash(flat),
        "a compacted rope hashes IDENTICALLY to its flat twin (equal keys must hash equal or a map mis-buckets)"
    );
    // And the two are interchangeable as a SET element: inserting both dedups to size 1.
    let s = op_set_insert(op_set_insert(op_set_empty(), compacted), flat);
    assert_eq!(
        op_set_size(s),
        1,
        "a compacted rope and its flat twin are the SAME set element"
    );
    op_drop(s);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn compact_makes_a_slice_rope_canonical_the_string_at_shape() {
    reset();
    let before = live_nodes();
    // Two 1-byte slices of "a" at DIFFERENT offsets into DIFFERENT parents (the `String.at` shape).
    let p1 = bytes_leaf(b"banana");
    let sl1 = op_bytes_slice(p1, 1, 1); // "a" at index 1
    let p2 = bytes_leaf(b"banana");
    let sl2 = op_bytes_slice(p2, 3, 1); // "a" at index 3 — same content, different offset
    let flat_a = bytes_leaf(b"a");
    // (1) TRIPWIRE — raw slices are champ_eq-DISTINCT from each other AND the flat "a" (physical `[off,
    // len]` compare, NOT content). This IS the miscompile the compiler must compact away.
    assert!(
        !champ_eq(sl1, sl2),
        "two slices of the same char at different offsets are champ_eq-distinct (physical [off,len])"
    );
    assert!(
        !champ_eq(sl1, flat_a),
        "a raw slice is champ_eq-distinct from a flat leaf of the same content"
    );
    // (2) THE GUARANTEE — compact each slice → all champ_eq + hash-identical to the flat twin.
    let c1 = op_bytes_compact(sl1);
    let c2 = op_bytes_compact(sl2);
    assert!(
        champ_eq(c1, flat_a),
        "a compacted slice == the flat char (the String.at fix's runtime half)"
    );
    assert!(
        champ_eq(c1, c2),
        "two compacted slices of the same content are champ_eq (count-a now works)"
    );
    assert_eq!(
        champ_hash(c1),
        champ_hash(flat_a),
        "…and hashes identically"
    );
    op_drop(c1);
    op_drop(c2);
    op_drop(flat_a);
    assert_eq!(live_nodes(), before, "no leak across the slice-compact");
}

#[test]
fn compact_of_a_dual_used_shared_slice_view_is_balanced_with_the_dup() {
    reset();
    let before = live_nodes();
    let parent = bytes_leaf(&[10, 20, 30, 40, 50]);
    let sl = op_bytes_slice(parent, 1, 2); // slice-view [20,30]; the view now owns parent's ref
    op_dup(sl); // SIMULATE the compiler dup for the dual consuming use → sl rc 2 (shared)
    let t1 = op_bytes_compact(sl); // compact #1: flattens the now-SHARED view in place
    let t2 = op_bytes_compact(sl); // compact #2: sl is a flat leaf now → no-op, same handle
    let b = op_bytes_concat(t1, t2); // consumes BOTH refs of sl (rc 2 → 0, freed once)
    assert_eq!(
        op_bytes_len(b),
        4,
        "concat of [20,30] ++ [20,30] is 4 bytes"
    );
    let b_flat = op_bytes_compact(b);
    let expected = bytes_leaf(&[20, 30, 20, 30]);
    assert!(
        champ_eq(b_flat, expected),
        "content is [20,30,20,30] — value correct"
    );
    op_drop(b_flat);
    op_drop(expected);
    assert_eq!(
        live_nodes(),
        before,
        "census balances (no double-free, no leak) — compact-of-a-shared view is sound WITH the dup, so the adv54b fix is purely the compiler dup"
    );
}

#[test]
fn probe_relaxed_rrb_read_loop_balances_at_runtime() {
    reset();
    let before = live_nodes();
    let n: i64 = 1100;
    let mut v = op_vec_empty();
    for i in 0..n {
        v = op_vec_prepend(v, op_box_int(i)); // relaxed (front-growth) RRB
    }
    // Read EVERY index, borrowing v (NO threading dup) — the List.at read dance: vec-get borrows the
    // element, the emit dups it for the `Some` payload, the match extract + Some-shell drop cascades -1.
    for idx in 0..(n as u32) {
        let e = op_vec_get(v, idx); // BORROW (rc unchanged)
        op_dup(e); // emit dups the borrowed vec-get result for the Some payload
        let some = op_sum_new(0, e); // Some(e)
        let _payload = op_sum_payload(some); // match extract (borrow) — read, don't consume
        op_drop(some); // drop the Some shell → cascades -1 to the dup'd e (balances the dup)
    }
    op_drop(v); // drop the list at loop end
    assert_eq!(
        live_nodes(),
        before,
        "relaxed-RRB read loop (vec-get + Some, borrowed list) balances at runtime (→ the corpus L2501 read-loop leak is COMPILER threading reclaim, not the relaxed read path)"
    );
}

#[test]
fn probe_single_use_compacted_slice_view_balances_at_runtime() {
    reset();
    let before = live_nodes();
    let parent = bytes_leaf(&[9, 20, 30, 8]); // P (rc1)
    let sl = op_bytes_slice(parent, 1, 2); // view [20,30]; view now owns P's ref (P rc1, held by view)
    let k = op_bytes_compact(sl); // flatten in place → drops P; k == sl, now a flat [20,30] leaf
    let flat = bytes_leaf(&[20, 30]);
    let _eq = champ_eq(k, flat); // a BORROWING compare (map-lookup/value-eq shape) — consumes neither
    op_drop(flat); // drop the flat RHS/probe (owned temp)
    op_drop(k); // drop the (arm-owned) compacted key
    assert_eq!(
        live_nodes(),
        before,
        "single-use compacted slice-view balances at the runtime layer (→ any corpus leak-2 is COMPILER emit reclaim, not runtime)"
    );
}

#[test]
fn value_eq_is_physical_bytes_a_rope_operand_needs_compiler_compaction() {
    reset();
    let before = live_nodes();
    let content = b"map-insert"; // > INLINE_RAW_CAP so the flat twin is a Heap leaf too
    let flat = op_str_new(String::from_utf8(content.to_vec()).unwrap());
    let rope = op_bytes_concat(bytes_leaf(&content[..4]), bytes_leaf(&content[4..])); // "map-" + "insert"
    // value-eq (op 61) = champ_eq: a rope vs its flat twin is DISTINCT (physical-byte compare) — this is
    // exactly the compiler contract boundary. A COMPACTED rope, however, IS value-eq to the flat twin.
    assert!(
        !champ_eq(rope, flat),
        "value-eq is physical-byte (champ_eq): an UNcompacted rope ≠ its flat twin — the compiler must compact before value-eq"
    );
    let compacted = op_bytes_compact(rope);
    assert!(
        champ_eq(compacted, flat),
        "a COMPACTED rope IS value-eq to its flat twin — canonicalization makes the physical-byte compare correct"
    );
    op_drop(compacted);
    op_drop(flat);
    assert_eq!(live_nodes(), before, "no leak");
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

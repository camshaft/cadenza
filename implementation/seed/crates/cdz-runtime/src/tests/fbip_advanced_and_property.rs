use super::*;

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
    assert!(
        champ_eq(m1, snap),
        "shared map unchanged after other owner's insert"
    );
    assert_eq!(
        champ_hash(m1),
        champ_hash(snap),
        "shared map hash unchanged"
    );
    assert_eq!(op_map_size(m1), orig_size, "shared map size unchanged");
    assert_eq!(
        mlookup_int(m1, sa),
        Some(1),
        "shared map key still resolves"
    );
    assert_eq!(
        mlookup_int(m1, ca),
        Some(3),
        "shared map collision key still resolves"
    );
    assert_eq!(
        mlookup_int(m1, 12345),
        None,
        "shared map never saw the new key"
    );
    // m2 has the change.
    assert_eq!(op_map_size(m2), orig_size + 1);
    assert_eq!(mlookup_int(m2, 12345), Some(999));
    assert_eq!(
        mlookup_int(m2, ca),
        Some(3),
        "m2 preserves the shared collision entry"
    );
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
    assert!(
        champ_eq(m1, snap),
        "shared map unchanged after other owner's remove"
    );
    assert_eq!(
        champ_hash(m1),
        champ_hash(snap),
        "shared map hash unchanged"
    );
    assert_eq!(op_map_size(m1), orig_size, "shared map size unchanged");
    assert_eq!(
        mlookup_int(m1, sa),
        Some(1),
        "shared map still has the removed key"
    );
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
    assert!(
        champ_eq(s1, snap),
        "shared set unchanged after other owner's insert"
    );
    assert_eq!(champ_hash(s1), champ_hash(snap));
    assert_eq!(op_set_size(s1), orig_size);
    assert!(
        !scontains_int(s1, 54321),
        "shared set never saw the new elem"
    );
    assert!(scontains_int(s2, 54321));
    assert!(scontains_int(s2, ca), "s2 preserves the collision elem");
    assert_eq!(op_set_size(s2), orig_size + 1);
    op_drop(snap);
    op_drop(s1);
    op_drop(s2);
    assert_eq!(live_nodes(), before, "no leak / no double-free");
}

#[test]
fn set_contains_borrow_does_not_block_insert_inplace_reuse_no_scaling_leak() {
    reset();
    let before = live_nodes();
    // A uniquely-owned (rc==1) accumulator, like a fresh loop-carried `seen`.
    let mut seen = sinsert_int(op_set_empty(), 0);
    assert_eq!(node_rc(seen), 1, "fresh accumulator is exclusively owned");
    // Model TWO-SUM's per-iteration shape: BORROW (contains) an already-present element, then INSERT a
    // new one — repeatedly. If a completed borrow blocked reuse, each insert would path-copy and orphan
    // the old `seen`, so the live count would scale and the final drop would not return to baseline.
    for i in 1..24i64 {
        let _present = scontains_int(seen, i - 1); // BORROW that completes before the insert
        assert_eq!(
            node_rc(seen),
            1,
            "a completed Set.contains borrow leaves the accumulator at rc==1 (does not block reuse)"
        );
        seen = sinsert_int(seen, i); // rc==1 ⇒ in-place FBIP refit, no orphaned prior version
        assert_eq!(
            node_rc(seen),
            1,
            "the reused accumulator stays uniquely owned"
        );
    }
    assert_eq!(
        op_set_size(seen) as i64,
        24,
        "all distinct elements inserted"
    );
    op_drop(seen);
    assert_eq!(
        live_nodes(),
        before,
        "NO scaling leak — in-place reuse orphaned nothing; the final drop reclaims everything"
    );
}

#[test]
fn if_join_shared_rc1_base_subsumed_by_builder_balances_on_single_drop() {
    // VEC (List family) — 40 elems crosses the 32-leaf boundary into a multi-level spine.
    {
        reset();
        let before = live_nodes();
        let mut base = op_vec_empty();
        for i in 0..40 {
            base = op_vec_push(base, op_box_int(i));
        }
        assert_eq!(node_rc(base), 1, "vec: base uniquely owned after dup-skip");
        let result = op_vec_push(base, op_box_int(999)); // else-arm consumes the rc1 base in place
        assert_eq!(
            node_rc(result),
            1,
            "vec: result is the subsumed rc1 base, uniquely owned"
        );
        op_drop(result); // the single post-if drop
        assert_eq!(
            live_nodes(),
            before,
            "vec: subsumed base reclaimed by the one drop — no leak, no double-free"
        );
    }
    // SET (CHAMP)
    {
        reset();
        let before = live_nodes();
        let mut base = op_set_empty();
        for i in 0..40 {
            base = sinsert_int(base, i);
        }
        assert_eq!(node_rc(base), 1, "set: base uniquely owned after dup-skip");
        let result = sinsert_int(base, 999);
        assert_eq!(node_rc(result), 1, "set: result is the subsumed rc1 base");
        op_drop(result);
        assert_eq!(
            live_nodes(),
            before,
            "set: subsumed base reclaimed by the one drop — no leak, no double-free"
        );
    }
    // MAP (CHAMP)
    {
        reset();
        let before = live_nodes();
        let mut base = op_map_empty();
        for i in 0..40 {
            base = minsert_int(base, i, i * 2);
        }
        assert_eq!(node_rc(base), 1, "map: base uniquely owned after dup-skip");
        let result = minsert_int(base, 999, 1);
        assert_eq!(node_rc(result), 1, "map: result is the subsumed rc1 base");
        op_drop(result);
        assert_eq!(
            live_nodes(),
            before,
            "map: subsumed base reclaimed by the one drop — no leak, no double-free"
        );
    }
}

#[test]
fn rope_concat_of_rc1_base_balances_on_single_drop() {
    reset();
    let before = live_nodes();
    // A uniquely-owned (rc1) multi-node rope base.
    let base = op_bytes_concat(
        op_str_new(String::from("caf")),
        op_str_new(String::from("é")),
    );
    assert_eq!(node_rc(base), 1, "rope: base uniquely owned after dup-skip");
    let x = op_str_new(String::from("XY"));
    let result = op_bytes_concat(base, x); // else-arm: consumes the rc1 base into a new concat node
    assert_eq!(node_rc(result), 1, "rope: result uniquely owned");
    assert_eq!(
        op_str_get(result),
        "caféXY",
        "rope: correct concatenated content"
    );
    op_drop(result); // the single post-if drop reclaims the base (now the node's child) + x
    assert_eq!(
        live_nodes(),
        before,
        "rope: base reclaimed by the one drop — no leak, no double-free"
    );
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
    assert!(
        champ_eq(s1, snap),
        "shared set unchanged after other owner's remove"
    );
    assert_eq!(champ_hash(s1), champ_hash(snap));
    assert_eq!(op_set_size(s1), orig_size);
    assert!(
        scontains_int(s1, ca),
        "shared set still has the removed elem"
    );
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
    assert!(
        unique_rm < shared_rm,
        "and strictly fewer in the collapse case"
    );
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
    assert!(
        champ_eq(fbip, copy),
        "FBIP-built collision map == copy-built"
    );
    assert_eq!(
        champ_hash(fbip),
        champ_hash(copy),
        "byte-identical canonical shape"
    );
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
    assert!(
        champ_eq(collapsed_fbip, collapsed_copy),
        "FBIP collapse == copy collapse"
    );
    assert!(
        champ_eq(collapsed_fbip, fresh),
        "collapse yields the canonical fresh shape"
    );
    assert_eq!(champ_hash(collapsed_fbip), champ_hash(fresh));
    op_drop(collapsed_fbip);
    op_drop(collapsed_copy);
    op_drop(fresh);

    // (3) remove-to-canonical-empty via FBIP.
    let mut m = minsert_int(op_map_empty(), 42, 1);
    m = mremove_int(m, 42);
    assert!(
        is_empty_node(m),
        "FBIP remove of the last entry yields the canonical empty"
    );
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
    assert!(
        champ_eq(fbip, copy),
        "empty-slot splice past a subnode == copy-path build (canonical)"
    );
    assert_eq!(
        champ_hash(fbip),
        champ_hash(copy),
        "byte-identical canonical shape"
    );
    assert_eq!(mlookup_int(fbip, a), Some(1));
    assert_eq!(mlookup_int(fbip, b), Some(2));
    for (i, &k) in extra_keys.iter().enumerate() {
        assert_eq!(
            mlookup_int(fbip, k),
            Some(100 + i as i64),
            "spliced key {k} present"
        );
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
    let (sa, sb) = low5_split_pair(); // sa,sb share low-5 ⇒ an existing subnode at the root
    let (ca, cb) = full_hash_collision_pair(); // a distinct pair that also splits (to a collision node)
    let build = |shared: bool| -> Handle {
        let mut m = op_map_empty();
        // First sa,sb (creates subnode #1) + ordinary inline entries + ca alone (inline).
        let mut seq: Vec<(i64, i64)> =
            vec![(sa, 1), (sb, 2), (ca, 3), (1i64, 10), (2, 20), (3, 30)];
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
    assert!(
        champ_eq(fbip, copy),
        "in-place SPLIT == copy-path build (canonical subnode placement)"
    );
    assert_eq!(
        champ_hash(fbip),
        champ_hash(copy),
        "byte-identical canonical shape"
    );
    for &(k, v) in &[
        (sa, 1i64),
        (sb, 2),
        (ca, 3),
        (cb, 4),
        (1, 10),
        (2, 20),
        (3, 30),
    ] {
        assert_eq!(
            mlookup_int(fbip, k),
            Some(v),
            "key {k} present after the in-place split"
        );
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
        0,       // …00000_00000_00000
        1 << 5,  // differs only at level 1
        1 << 10, // differs only at level 2
        (1 << 5) | (1 << 10),
        1, // differs at level 0
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
    assert!(
        champ_eq(fbip, copy),
        "deep unique FBIP spine == copy-path build"
    );
    assert_eq!(
        champ_hash(fbip),
        champ_hash(copy),
        "byte-identical canonical shape"
    );
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
    assert_eq!(
        champ_hash(snap),
        snap_hash,
        "shared snapshot unchanged after sibling insert"
    );
    for (i, &k) in deep_keys.iter().enumerate() {
        assert_eq!(
            mlookup_int(snap, k),
            Some(i as i64),
            "snapshot key {k} intact"
        );
    }
    assert_eq!(
        mlookup_int(m, (1 << 5) | (1 << 10) | 1),
        Some(999),
        "new key in the new version"
    );
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
    assert_eq!(
        op_map_size(m) as usize,
        keys.len(),
        "overwrites did not change size"
    );
    for (i, &k) in keys.iter().enumerate() {
        let want = if i % 2 == 0 {
            1000 + i as i64
        } else {
            i as i64
        };
        let probe = op_box_int(k);
        let got = op_map_lookup(m, probe); // borrows the value; do not retain
        assert_eq!(
            op_get_int(got),
            (1i64 << 40) + want,
            "key {k} has the right (boxed) value"
        );
        op_drop(probe);
    }
    op_drop(m);
    assert_eq!(
        live_nodes(),
        before,
        "every entry column freed exactly once — no leak, no double-free"
    );
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
        *lcg = lcg
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
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
    assert_eq!(
        op_map_size(m) as usize,
        reference.len(),
        "size matches reference"
    );
    for (&k, &v) in &reference {
        assert_eq!(mlookup_int(m, k), Some(v), "key {k} matches reference");
    }
    // And no phantom keys: probe the whole small keyspace.
    for k in 0..64i64 {
        assert_eq!(
            mlookup_int(m, k),
            reference.get(&k).copied(),
            "keyspace probe {k}"
        );
    }
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak across the mixed sequence");
}

#[test]
fn prop_map_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<MapOp>>()
        .for_each(|ops| run_map_op_sequence(ops));
}

#[test]
fn prop_set_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<SetOp>>()
        .for_each(|ops| run_set_op_sequence(ops));
}

#[test]
fn prop_strset_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<StrSetOp>>()
        .for_each(|ops| run_strset_op_sequence(ops));
}

#[test]
fn prop_tuplekey_map_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<TupleKeyOp>>()
        .for_each(|ops| run_tuplekey_op_sequence(ops));
}

#[test]
fn prop_strkey_map_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<StrKeyOp>>()
        .for_each(|ops| run_strkey_op_sequence(ops));
}

#[test]
fn prop_map_str_val_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<MapStrValOp>>()
        .for_each(|ops| run_map_str_val_op_sequence(ops));
}

#[test]
fn prop_reset_reuse_roundtrip_is_canonical_and_leak_free() {
    bolero::check!()
        .with_type::<Vec<ResetReuseCase>>()
        .for_each(|cases| {
            for c in cases {
                run_reset_reuse_case(c);
            }
        });
}

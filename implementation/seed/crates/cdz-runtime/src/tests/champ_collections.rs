use super::*;

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
    assert_eq!(
        live_nodes(),
        before,
        "discrimination test reclaimed all nodes"
    );
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
        op_box_int(FIXNUM_MAX),     // largest inline fixnum
        op_box_int(FIXNUM_MAX + 1), // first BOXED int (out of the inline window)
        op_box_int(FIXNUM_MIN - 1), // first boxed negative
        op_box_float(3.5),
        op_box_float(-0.0), // distinct bits from 0.0
        op_str_new(String::new()),
        op_str_new("cadenza".to_string()),
        op_bytes_alloc(0),
        Handle::NULL, // null folds to the bare offset basis on both paths
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
    op_arr_set(arr, 1, imm_bool(true)); // immediate child folded via the fast leaf
    let sum = op_sum_new(3, op_box_int(9));
    let kv = champ_kv_node(10, 20); // a real CHAMP node with a set datamap bit
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
    assert_eq!(
        live_nodes(),
        before,
        "reference-hash test reclaimed all nodes"
    );
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
        assert_ne!(
            got,
            Handle::NULL,
            "tuple key ({a},{b}) found via a fresh, equal probe"
        );
        assert_eq!(op_get_int(got), v, "tuple key ({a},{b}) maps to {v}");
        op_drop(probe);
    }
    // Overwrite (1,2) via a fresh equal key — size stays 4, value updates (dedup by hash+eq).
    m = op_map_insert(m, tuple(1, 2), op_box_int(999));
    assert_eq!(op_map_size(m), 4, "equal tuple key overwrote, did not add");
    let probe = tuple(1, 2);
    assert_eq!(
        op_get_int(op_map_lookup(m, probe)),
        999,
        "value overwritten"
    );
    op_drop(probe);
    // A miss on an absent tuple.
    let miss = tuple(7, 7);
    assert_eq!(
        op_map_lookup(m, miss),
        Handle::NULL,
        "absent tuple key misses"
    );
    op_drop(miss);
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn map_merge_last_writer_wins_and_no_leak() {
    reset();
    let before = live_nodes();
    // Heap-boxed VALUES (FIXNUM_MAX + n) so the merge's per-entry val dup/drop path is exercised
    // (small-int keys are immediates → no probe-drop needed, and their dup/drop is a no-op).
    // a = {1:10, 2:20, 3:30}
    let mut a = op_map_empty();
    a = op_map_insert(a, op_box_int(1), op_box_int(FIXNUM_MAX + 10));
    a = op_map_insert(a, op_box_int(2), op_box_int(FIXNUM_MAX + 20));
    a = op_map_insert(a, op_box_int(3), op_box_int(FIXNUM_MAX + 30));
    // b = {2:200, 4:40} — key 2 conflicts with a.
    let mut b = op_map_empty();
    b = op_map_insert(b, op_box_int(2), op_box_int(FIXNUM_MAX + 200));
    b = op_map_insert(b, op_box_int(4), op_box_int(FIXNUM_MAX + 40));
    // LAST-WRITER-WINS: b overwrites a on the conflicting key 2. Consumes both a and b.
    let m = op_map_merge(a, b);
    assert_eq!(op_map_len(m), 4, "union = {{1,2,3,4}}");
    assert_eq!(
        op_get_int(op_map_lookup(m, op_box_int(1))),
        FIXNUM_MAX + 10,
        "a-only key kept"
    );
    assert_eq!(
        op_get_int(op_map_lookup(m, op_box_int(2))),
        FIXNUM_MAX + 200,
        "conflicting key 2: b wins (last-writer)"
    );
    assert_eq!(
        op_get_int(op_map_lookup(m, op_box_int(3))),
        FIXNUM_MAX + 30,
        "a-only key kept"
    );
    assert_eq!(
        op_get_int(op_map_lookup(m, op_box_int(4))),
        FIXNUM_MAX + 40,
        "b-only key added"
    );
    op_drop(m);
    assert_eq!(live_nodes(), before, "map-merge: no leak / no double-free");
}

#[test]
fn map_merge_empty_operand_is_identity_no_leak() {
    reset();
    let before = live_nodes();
    // merge(a, empty) == a (contents preserved), and no leak.
    let mut a = op_map_empty();
    a = op_map_insert(a, op_box_int(1), op_box_int(FIXNUM_MAX + 1));
    a = op_map_insert(a, op_box_int(2), op_box_int(FIXNUM_MAX + 2));
    let m1 = op_map_merge(a, op_map_empty());
    assert_eq!(op_map_len(m1), 2, "merge(a, empty) keeps a's 2 entries");
    assert_eq!(op_get_int(op_map_lookup(m1, op_box_int(1))), FIXNUM_MAX + 1);
    assert_eq!(op_get_int(op_map_lookup(m1, op_box_int(2))), FIXNUM_MAX + 2);
    op_drop(m1);
    // merge(empty, b) == b.
    let mut b = op_map_empty();
    b = op_map_insert(b, op_box_int(7), op_box_int(FIXNUM_MAX + 7));
    let m2 = op_map_merge(op_map_empty(), b);
    assert_eq!(op_map_len(m2), 1, "merge(empty, b) keeps b's entry");
    assert_eq!(op_get_int(op_map_lookup(m2, op_box_int(7))), FIXNUM_MAX + 7);
    op_drop(m2);
    // merge(empty, empty) == empty.
    let m3 = op_map_merge(op_map_empty(), op_map_empty());
    assert_eq!(op_map_len(m3), 0, "merge(empty, empty) is empty");
    op_drop(m3);
    assert_eq!(
        live_nodes(),
        before,
        "map-merge identity: no leak / no double-free"
    );
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
fn champ_eq_is_tagless_same_raw_different_kind_is_equal() {
    reset();
    let before = live_nodes();
    // A boxed Int (outside the fixnum window → a real heap leaf with 8 LE raw bytes, zero handles).
    let n: i64 = 0x0102_0304_0506_0708;
    let int_leaf = op_box_int(n);
    assert!(
        !is_immediate(int_leaf),
        "the value is boxed (heap leaf), not an inline fixnum"
    );
    // A Bytes leaf holding those exact 8 little-endian bytes — same raw, same (zero) arity, DIFFERENT
    // type. The runtime has no tag, so it is indistinguishable from the Int leaf.
    let bytes_leaf = op_bytes_alloc(8);
    for k in 0..8u32 {
        op_bytes_set(bytes_leaf, k, ((n >> (8 * k)) & 0xff) as u32);
    }
    assert!(
        champ_eq(int_leaf, bytes_leaf),
        "tagless: an Int and a same-raw Bytes leaf are champ_eq (no type tag to tell them apart)"
    );
    assert_eq!(
        champ_hash(int_leaf),
        champ_hash(bytes_leaf),
        "…and hash identically (byte-hash is storage/type-transparent)"
    );
    assert_eq!(
        champ_key_cmp(int_leaf, bytes_leaf),
        core::cmp::Ordering::Equal,
        "…and compare Equal — hence homogeneous keys are the COMPILER's responsibility"
    );
    op_drop(int_leaf);
    op_drop(bytes_leaf);
    assert_eq!(live_nodes(), before, "no leak");
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
    assert!(
        champ_eq(x, y),
        "identical 4-level nests are eq (full descent)"
    );
    assert_eq!(
        champ_key_cmp(x, y),
        core::cmp::Ordering::Equal,
        "identical nests cmp Equal"
    );
    // Differ only at the deepest leaf: eq false, cmp non-Equal, and antisymmetric.
    assert!(
        !champ_eq(x, z),
        "nests differing at the deepest leaf are not eq"
    );
    let ord = champ_key_cmp(x, z);
    assert_ne!(
        ord,
        core::cmp::Ordering::Equal,
        "deep-leaf difference is found by cmp"
    );
    assert_eq!(
        champ_key_cmp(z, x),
        ord.reverse(),
        "cmp is antisymmetric across the deep difference"
    );
    // eq/cmp consistency at depth.
    assert_eq!(
        champ_eq(x, z),
        champ_key_cmp(x, z) == core::cmp::Ordering::Equal
    );

    op_drop(x);
    op_drop(y);
    op_drop(z);
    assert_eq!(
        live_nodes(),
        before,
        "nested-compound eq/cmp test reclaimed all nodes"
    );
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
    let d = tup(&[op_box_int(1)]); // differs in arity
    let e = tup(&[boxed_int_leaf(big(1)), boxed_int_leaf(big(2))]); // boxed-leaf children (shallow)
    let e2 = tup(&[boxed_int_leaf(big(1)), boxed_int_leaf(big(2))]); // equal to e
    let e3 = tup(&[boxed_int_leaf(big(1)), boxed_int_leaf(big(9))]); // differs at child 1 (boxed)
    // A NESTED tuple: child 0 is itself a tuple → the fast path must decline to the general walk.
    let nested = tup(&[tup(&[op_box_int(1), op_box_int(2)]), op_box_int(9)]);
    let nested2 = tup(&[tup(&[op_box_int(1), op_box_int(2)]), op_box_int(9)]);

    // Equalities.
    assert!(champ_eq(a, a2), "equal shallow tuples are eq");
    assert_eq!(
        champ_key_cmp(a, a2),
        core::cmp::Ordering::Equal,
        "equal shallow tuples cmp Equal"
    );
    assert!(
        champ_eq(e, e2),
        "equal shallow tuples over boxed leaves are eq"
    );
    assert_eq!(champ_key_cmp(e, e2), core::cmp::Ordering::Equal);
    assert!(
        champ_eq(nested, nested2),
        "equal NESTED tuples are eq (via the general walk)"
    );
    assert_eq!(champ_key_cmp(nested, nested2), core::cmp::Ordering::Equal);
    // Inequalities + eq/cmp consistency + antisymmetry across each difference kind.
    for &(x, y) in &[(a, b), (a, c), (a, d), (e, e3), (nested, a)] {
        assert!(!champ_eq(x, y), "differing tuples are not eq");
        let ord = champ_key_cmp(x, y);
        assert_ne!(ord, core::cmp::Ordering::Equal, "cmp finds the difference");
        assert_eq!(champ_key_cmp(y, x), ord.reverse(), "cmp antisymmetric");
        assert_eq!(
            champ_eq(x, y),
            champ_key_cmp(x, y) == core::cmp::Ordering::Equal,
            "cmp==Equal iff eq"
        );
    }
    for &h in &[a, a2, b, c, d, e, e2, e3, nested, nested2] {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "shallow-compound eq/cmp test reclaimed all nodes"
    );
}

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
        op_box_int(0),           // inline fixnum
        op_box_int(-1),          // inline negative
        op_box_int(536_870_912), // FIXNUM_MAX+1 ⇒ boxed leaf
        boxed_int_leaf(0),       // hand-boxed twin of inline 0
        boxed_int_leaf(-1),
        op_box_float(0.0),
        op_box_float(-0.0), // -0.0 ≠ 0.0 by raw bytes
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
    assert_eq!(
        live_nodes(),
        before,
        "raw-arity reference test reclaimed all nodes"
    );
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
fn insert_size_delta_stays_exact_across_deep_spines_and_overwrites() {
    reset();
    let before = live_nodes();
    // 400 distinct keys spanning many hash prefixes → deep spines + splits at multiple levels.
    let keys: Vec<i64> = (0..400).map(|k| k * 7 + 1).collect();
    let mut m = op_map_empty();
    let mut expected = 0u32;
    for (i, &k) in keys.iter().enumerate() {
        m = minsert_int(m, k, k * 2);
        expected += 1; // a fresh key: delta 1 at every ancestor
        assert_eq!(
            op_map_size(m),
            expected,
            "size after inserting fresh key #{i} ({k})"
        );
    }
    // Overwrite every key (delta 0 everywhere) — size must NOT change at any step.
    for &k in &keys {
        m = minsert_int(m, k, k * 3);
        assert_eq!(
            op_map_size(m),
            expected,
            "overwrite of {k} must not change size"
        );
    }
    // Now the SHARED (path-copy) insert path: keep the base, derive versions, check their sizes.
    for &k in &[keys[0], 999_999, keys[200], 888_888] {
        op_dup(m); // base stays shared → the derived insert path-copies
        let m2 = minsert_int(m, k, 0);
        let was_present = keys.contains(&k);
        let want = if was_present { expected } else { expected + 1 };
        assert_eq!(
            op_map_size(m2),
            want,
            "shared-insert size for key {k} (present={was_present})"
        );
        assert_eq!(
            op_map_size(m),
            expected,
            "the shared base's size is untouched"
        );
        op_drop(m2);
    }
    // Full membership + value sweep on the (overwritten) map.
    for &k in &keys {
        assert_eq!(
            mlookup_int(m, k),
            Some(k * 3),
            "key {k} resolves to its overwritten value"
        );
    }
    assert_eq!(mlookup_int(m, 999_999), None, "an absent key still misses");
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak across the whole sequence");
}

#[test]
fn map_many_distinct_keys_all_lookup() {
    reset();
    let before = live_nodes();
    let pairs = [
        (1i64, 10i64),
        (2, 20),
        (3, 30),
        (17, 170),
        (99, 990),
        (1000, 10000),
    ];
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
    let (dm, nm) = with_node(m, (0u32, 0u32), |n| {
        (champ_datamap(&n.raw), champ_nodemap(&n.raw))
    });
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
fn deep_champ_trie_insert_lookup_remove_at_depth_3() {
    reset();
    let before = live_nodes();
    // Keys found by a birthday search: all six share the low 15 bits (`& 0x7FFF == 0x1a4f`) of their
    // champ_hash → they collide at levels 0,1,2 and split only at level 3.
    let keys: [i64; 6] = [21471, 52398, 90452, 123525, 195537, 212302];
    // PRECONDITION: the shared low-15-bit hash prefix still holds (else the frozen hash changed —
    // re-find a group; this test is only meaningful when the keys force a deep descent).
    for &k in &keys {
        let kh = op_box_int(k);
        assert_eq!(
            champ_hash(kh) & 0x7FFF,
            0x1a4f,
            "PRECONDITION: key {k} must share the low-15-bit hash prefix 0x1a4f (else re-find keys)"
        );
        op_drop(kh);
    }
    // Measure the trie depth of a key's descent (walk subnodes until the data slot).
    let depth_of = |m: Handle, k: i64| -> u32 {
        let kh = op_box_int(k);
        let hash = champ_hash(kh);
        let mut node = m;
        let mut level = 0u32;
        loop {
            let child = with_node(node, None, |n| {
                let dm = champ_datamap(&n.raw);
                let nm = champ_nodemap(&n.raw);
                if dm == 0 && nm == 0 {
                    return None; // collision/empty — descent ends
                }
                let i = level_index(hash, level);
                let bit = 1u32 << i;
                if nm & bit != 0 {
                    let sidx = subnode_index_for_slot(nm, i) as usize;
                    let sbase = 2 * data_count(dm) as usize;
                    Some(n.handles[sbase + sidx])
                } else {
                    None // data slot (or absent) here — descent ends
                }
            });
            match child {
                Some(c) => {
                    node = c;
                    level += 1;
                }
                None => break,
            }
        }
        op_drop(kh);
        level
    };

    // Build the deep map: key i → value 1000+i.
    let mut m = op_map_empty();
    for (i, &k) in keys.iter().enumerate() {
        m = minsert_int(m, k, 1000 + i as i64);
    }
    assert_eq!(op_map_size(m), 6, "all six deep keys present");
    // The shared 15-bit prefix forces descent through levels 0,1,2 → depth ≥ 3.
    let d = depth_of(m, keys[0]);
    assert!(
        d >= 3,
        "the shared low-15-bit prefix forces a DEEP trie (depth {d}, expected ≥3) — deeper than the \
         fuzzer's depth-1 u8 keyspace"
    );
    // Deep LOOKUP: every key resolves to its own value (descent through 3 subnode levels).
    for (i, &k) in keys.iter().enumerate() {
        assert_eq!(
            mlookup_int(m, k),
            Some(1000 + i as i64),
            "deep key {k} resolves through the multi-level descent"
        );
    }
    // Deep REMOVE: remove one key; the deep spine collapses, the rest stay intact.
    m = mremove_int(m, keys[2]);
    assert_eq!(op_map_size(m), 5, "one deep key removed");
    assert_eq!(
        mlookup_int(m, keys[2]),
        None,
        "the removed deep key is gone"
    );
    for (i, &k) in keys.iter().enumerate() {
        if i == 2 {
            continue;
        }
        assert_eq!(
            mlookup_int(m, k),
            Some(1000 + i as i64),
            "surviving deep key {k} still resolves after the deep remove"
        );
    }
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak across the deep-trie ops");
}

#[test]
fn map_string_key_collision_node_distinguishes_by_content() {
    reset();
    let before = live_nodes();
    // Found via a champ_hash birthday search over "k{n}": both hash to 2462319294 (searched ~261k).
    let (a, b) = ("k32728", "k261234");
    // PRECONDITION: they genuinely collide (full 32-bit champ_hash equal). If this fails, the hash
    // changed — find a new colliding pair; the test below is only meaningful on a real collision node.
    let (ka, kb) = (op_str_new(String::from(a)), op_str_new(String::from(b)));
    assert_eq!(
        champ_hash(ka),
        champ_hash(kb),
        "PRECONDITION: {a:?} and {b:?} must share a full 32-bit champ_hash (else re-find a pair)"
    );
    assert!(
        !champ_eq(ka, kb),
        "…but they are DISTINCT strings (differ by content)"
    );
    op_drop(ka);
    op_drop(kb);

    // Insert both colliding string keys → a collision node at the hash floor.
    let mut m = op_map_empty();
    m = op_map_insert(m, op_str_new(String::from(a)), op_box_int(111));
    m = op_map_insert(m, op_str_new(String::from(b)), op_box_int(222));
    assert_eq!(
        op_map_size(m),
        2,
        "both colliding string keys are kept (not merged by the shared hash)"
    );
    // Each resolves to its OWN value BY CONTENT (the collision-node raw-byte champ_eq scan).
    let look = |m: Handle, k: &str| -> i64 {
        let kh = op_str_new(String::from(k));
        let v = op_map_lookup(m, kh);
        op_drop(kh);
        if v == Handle::NULL { -1 } else { op_get_int(v) }
    };
    assert_eq!(
        look(m, a),
        111,
        "collision key {a:?} resolves to its own value"
    );
    assert_eq!(
        look(m, b),
        222,
        "collision key {b:?} resolves to its own value"
    );
    // A THIRD, non-present string (also just a probe) must MISS even if it shares the hash prefix.
    assert_eq!(look(m, "k0"), -1, "an absent key misses");

    // Remove ONE collision entry: the collision node drains to the other entry, which stays intact.
    // `op_map_remove` BORROWS the key (it hashes + compares it, doesn't consume), so drop the probe.
    let rk = op_str_new(String::from(a));
    m = op_map_remove(m, rk);
    op_drop(rk);
    assert_eq!(op_map_size(m), 1, "one collision entry removed");
    assert_eq!(look(m, a), -1, "the removed collision key is gone");
    assert_eq!(
        look(m, b),
        222,
        "the surviving collision key still resolves to its own value"
    );
    op_drop(m);
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the string-collision-node ops"
    );
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
    assert!(
        champ_eq(fbip, copy),
        "in-place drain remove == copy-path remove (canonical)"
    );
    assert_eq!(
        champ_hash(fbip),
        champ_hash(copy),
        "byte-identical canonical shape"
    );
    assert!(
        champ_eq(fbip, fresh),
        "== a fresh map of the survivors (order-independent canonical)"
    );
    assert_eq!(mlookup_int(fbip, remove_key), None, "removed key absent");
    for &(k, v) in &[(sa, 100i64), (sb, 200), (1, 10), (3, 30), (4, 40)] {
        assert_eq!(
            mlookup_int(fbip, k),
            Some(v),
            "survivor {k} intact after the drain shift"
        );
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
    assert!(
        champ_eq(fbip, copy),
        "in-place-descend remove == copy-path remove (canonical)"
    );
    assert_eq!(
        champ_hash(fbip),
        champ_hash(copy),
        "byte-identical canonical shape"
    );
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
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the in-place-descend removes"
    );
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
    assert!(
        champ_eq(m, empty),
        "remove-to-empty is byte-identical to op_map_empty()"
    );
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
    let (dm0, nm0) = with_node(m, (0u32, 0u32), |n| {
        (champ_datamap(&n.raw), champ_nodemap(&n.raw))
    });
    assert_eq!(
        (data_count(dm0), subnode_count(nm0)),
        (0, 1),
        "split created a subnode"
    );
    // Remove one of the two: the subnode must collapse back into a single inline entry.
    let m = mremove_int(m, a);
    let (dm, nm) = with_node(m, (0u32, 0u32), |n| {
        (champ_datamap(&n.raw), champ_nodemap(&n.raw))
    });
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
    assert!(
        champ_eq(m, solo),
        "collision collapse is canonical (== fresh single-entry map)"
    );
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
        let mut seq: Vec<(i64, i64)> =
            vec![(a, 1), (b, 2), (c, 3), (d, 4), (5i64, 50), (6, 60), (7, 70)];
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
    assert!(
        champ_eq(fbip, copy),
        "in-place collapse == copy-path collapse (canonical)"
    );
    assert_eq!(
        champ_hash(fbip),
        champ_hash(copy),
        "byte-identical canonical shape"
    );
    assert_eq!(mlookup_int(fbip, a), None, "removed key gone");
    for &(k, v) in &[(b, 2i64), (c, 3), (d, 4), (5, 50), (6, 60), (7, 70)] {
        assert_eq!(
            mlookup_int(fbip, k),
            Some(v),
            "survivor {k} intact after in-place collapse"
        );
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
    assert!(
        champ_eq(ma, mb),
        "insert-then-remove == direct insert (canonical)"
    );
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
    let orig = minsert_int(
        minsert_int(minsert_int(op_map_empty(), 10, 1), 20, 2),
        30,
        3,
    );
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

#[test]
fn map_iter_empty_is_exhausted() {
    reset();
    let before = live_nodes();
    let m = op_map_empty();
    let cur = op_map_iter(m);
    assert_eq!(
        op_map_iter_key(cur),
        Handle::NULL,
        "empty map cursor is exhausted"
    );
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
    assert_eq!(
        op_map_iter_key(cur),
        Handle::NULL,
        "past the only entry ⇒ exhausted"
    );
    op_drop(cur);
    op_drop(m);
    assert_eq!(live_nodes(), before);
}

#[test]
fn map_iter_full_traversal_visits_each_once() {
    reset();
    let before = live_nodes();
    let (sa, sb) = low5_split_pair(); // force a subnode split into the traversal
    let mut pairs: Vec<(i64, i64)> = vec![
        (1, 10),
        (2, 20),
        (3, 30),
        (17, 170),
        (99, 990),
        (1000, 10000),
    ];
    pairs.push((sa, 111));
    pairs.push((sb, 222));
    let mut m = op_map_empty();
    for &(k, v) in &pairs {
        m = minsert_int(m, k, v);
    }
    let visited = collect_map(m);
    assert_eq!(
        visited.len(),
        op_map_size(m) as usize,
        "visited exactly size entries"
    );
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
    let keys = [
        (1i64, 10i64),
        (5, 50),
        (sa, 100),
        (sb, 200),
        (42, 420),
        (7, 70),
    ];
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
    assert_eq!(
        order1, order2,
        "canonical order is insert-order-independent"
    );
    op_drop(m1);
    op_drop(m2);
    assert_eq!(live_nodes(), before);
}

#[test]
fn map_iter_order_is_deterministic_for_string_keys() {
    reset();
    let before = live_nodes();
    // Varied lengths + a shared "key"/"keyword" prefix (distinct hashes, adjacent-ish slots) + the
    // empty string, to spread keys across the trie rather than one bucket.
    let names = [
        "key",
        "keyword",
        "a",
        "",
        "bb",
        "ccc",
        "z",
        "a-longer-identifier",
    ];
    let build = |order: &dyn Fn(usize) -> usize| -> Handle {
        let mut m = op_map_empty();
        for i in 0..names.len() {
            let j = order(i);
            m = op_map_insert(m, op_str_new(names[j].to_string()), op_box_int(j as i64));
        }
        m
    };
    let m1 = build(&|i| i); // forward insert order
    let m2 = build(&|i| names.len() - 1 - i); // reverse insert order
    let collect = |m: Handle| -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = op_map_iter(m);
        loop {
            let k = op_map_iter_key(cur);
            if k == Handle::NULL {
                break;
            }
            out.push(op_str_get(k));
            cur = op_map_iter_next(cur);
        }
        op_drop(cur);
        out
    };
    let order1 = collect(m1);
    let order2 = collect(m2);
    assert_eq!(
        order1.len(),
        names.len(),
        "the cursor visits every distinct string key exactly once"
    );
    assert_eq!(
        order1, order2,
        "a string-keyed map iterates in the SAME (CHAMP hash) order regardless of insert order"
    );
    op_drop(m1);
    op_drop(m2);
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the string-key iteration"
    );
}

#[test]
fn map_cursor_is_hash_order_which_differs_from_canonical_render_order() {
    reset();
    let before = live_nodes();
    // Keys whose CHAMP hash order differs from numeric order (256's LE bytes, spread magnitudes).
    let keyvals = [
        (256i64, 2560i64),
        (1, 10),
        (3, 30),
        (2, 20),
        (100, 1000),
        (7, 70),
    ];
    let mut m = op_map_empty();
    for &(k, v) in &keyvals {
        m = op_map_insert(m, op_box_int(k), op_box_int(v));
    }
    // (A) the runtime cursor's visiting order.
    let mut cursor_keys: Vec<i64> = Vec::new();
    let mut cur = op_map_iter(m);
    loop {
        let k = op_map_iter_key(cur);
        if k == Handle::NULL {
            break;
        }
        cursor_keys.push(op_get_int(k));
        cur = op_map_iter_next(cur);
    }
    op_drop(cur);
    // (B) the canonical byte-form key order = value-encode's sort = ascending numeric.
    let mut canonical_keys: Vec<i64> = keyvals.iter().map(|&(k, _)| k).collect();
    canonical_keys.sort_unstable();
    // The cursor visits EVERY key exactly once (a correct, complete traversal)…
    let mut cursor_sorted = cursor_keys.clone();
    cursor_sorted.sort_unstable();
    assert_eq!(
        cursor_sorted, canonical_keys,
        "the cursor visits exactly the map's keys (complete traversal)"
    );
    // …but NOT in canonical order — this is the contract gap the doc-comment describes.
    assert_ne!(
        cursor_keys, canonical_keys,
        "the cursor walks HASH order, which for these keys differs from the canonical (numeric) byte-\
         form order — a `Map.fold`/`keys` over the raw cursor would violate *Map Iteration Is \
         Deterministic*'s agree-with-canonical clause; the compiler must re-sort when exposing iteration"
    );
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn map_string_key_cursor_is_hash_order_not_lexicographic() {
    reset();
    let before = live_nodes();
    // Identifier-like keys (a symbol-table / keyword-set shape) whose FNV-hash order ≠ lexicographic.
    let names = ["let", "in", "lambda", "app", "var", "if", "match", "case"];
    let mut m = op_map_empty();
    for (i, nm) in names.iter().enumerate() {
        m = op_map_insert(m, op_str_new(String::from(*nm)), op_box_int(i as i64));
    }
    // (A) the raw cursor's visiting order.
    let mut cursor_keys: Vec<String> = Vec::new();
    let mut cur = op_map_iter(m);
    loop {
        let k = op_map_iter_key(cur);
        if k == Handle::NULL {
            break;
        }
        cursor_keys.push(op_str_get(k));
        cur = op_map_iter_next(cur);
    }
    op_drop(cur);
    // (B) the canonical byte-form key order for strings = lexicographic (what value-encode's
    //     `value_cmp_shaped` Str arm produces + what a spec-conformant `Map.fold` must match).
    let mut lex: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    lex.sort();
    // The cursor visits EVERY key exactly once (a correct, complete traversal)…
    let mut cursor_sorted = cursor_keys.clone();
    cursor_sorted.sort();
    assert_eq!(
        cursor_sorted, lex,
        "the string-key cursor visits exactly the map's keys (complete traversal)"
    );
    // …but NOT in lexicographic (canonical) order — the contract gap the port must not build on blindly.
    assert_ne!(
        cursor_keys, lex,
        "the STRING-key cursor walks FNV-hash order, which differs from the canonical (lexicographic) \
         byte-form order — a `Map.to-list`/`keys`/`fold` emitted over the raw cursor would violate \
         *Map Iteration Is Deterministic*'s agree-with-canonical clause AND emit the self-hosting \
         compiler's string-keyed bindings in hash order; the compiler must re-sort (or use a \
         canonical-order runtime op) when exposing iteration"
    );
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn map_iter_fork_independence() {
    reset();
    let before = live_nodes();
    let m = minsert_int(
        minsert_int(minsert_int(op_map_empty(), 1, 10), 2, 20),
        3,
        30,
    );
    // A shared cursor with rc>1: advancing the RESULT of next must not disturb the other ref.
    let cur = op_map_iter(m);
    let first_key = op_get_int(op_map_iter_key(cur));
    op_dup(cur); // now rc==2: `cur` referenced twice
    let advanced = op_map_iter_next(cur); // consumes one ref, returns a fresh cursor
    // The still-held original reference (cur) must project its ORIGINAL key unchanged.
    assert_eq!(
        op_get_int(op_map_iter_key(cur)),
        first_key,
        "fork undisturbed by advance"
    );
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
    assert!(
        keys.contains(&a) && keys.contains(&b),
        "both colliding keys seen"
    );
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
    let (dm, nm) = with_node(s, (0u32, 0u32), |n| {
        (champ_datamap(&n.raw), champ_nodemap(&n.raw))
    });
    assert_eq!(
        (data_count(dm), subnode_count(nm)),
        (0, 1),
        "split created a subnode"
    );
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
    assert!(
        champ_eq(s, empty),
        "remove-to-empty is byte-identical to op_set_empty()"
    );
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
    let (dm, nm) = with_node(s, (0u32, 0u32), |n| {
        (champ_datamap(&n.raw), champ_nodemap(&n.raw))
    });
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
    assert_eq!(
        v1.len(),
        op_set_size(s1) as usize,
        "visited exactly size elements"
    );
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
    assert_eq!(
        op_get_int(op_set_iter_elem(cur)),
        first,
        "fork undisturbed by advance"
    );
    assert_ne!(
        op_get_int(op_set_iter_elem(advanced)),
        first,
        "advanced moved on"
    );
    op_drop(cur);
    op_drop(advanced);
    op_drop(s);
    // Collision-pair both visited.
    let (a, b) = full_hash_collision_pair();
    let sc = sinsert_int(sinsert_int(op_set_empty(), a), b);
    let visited: std::collections::HashSet<i64> = collect_set(sc).into_iter().collect();
    assert!(
        visited.contains(&a) && visited.contains(&b),
        "both colliding elems visited"
    );
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
    assert!(
        champ_eq(m1, m2),
        "collision node canonical regardless of insert order"
    );
    assert_eq!(
        champ_hash(m1),
        champ_hash(m2),
        "equal collision maps hash equal"
    );
    assert_eq!(
        collect_map(m1),
        collect_map(m2),
        "iteration order identical"
    );
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
    assert!(
        champ_eq(s1, s2),
        "collision set canonical regardless of insert order"
    );
    assert_eq!(
        champ_hash(s1),
        champ_hash(s2),
        "equal collision sets hash equal"
    );
    assert_eq!(
        collect_set(s1),
        collect_set(s2),
        "iteration order identical"
    );
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
    assert_eq!(
        champ_key_cmp(Handle::NULL, Handle::NULL),
        core::cmp::Ordering::Equal
    );
    op_drop(x);
    op_drop(y);
    op_drop(z);
    assert_eq!(live_nodes(), before);
}

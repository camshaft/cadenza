use super::*;

#[test]
fn prop_vec_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<VecOp>>()
        .for_each(|ops| run_vec_op_sequence(ops));
}

#[test]
fn prop_packed_bool_vector_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<VecOp>>()
        .for_each(|ops| run_bool_vec_op_sequence(ops));
}

#[test]
fn prop_bytes_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<BytesOp>>()
        .for_each(|ops| run_bytes_op_sequence(ops));
}

#[test]
fn prop_value_encode_is_total_under_arbitrary_descriptor() {
    bolero::check!()
        .with_type::<Vec<u8>>()
        .for_each(|desc| assert_encode_is_total(desc));
}

#[test]
fn prop_eq_hash_cmp_contract_over_random_compound_trees() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
        reset();
        let before = live_nodes();
        // Twin A and B: same bytes → structurally identical, DISTINCT nodes.
        let (mut ca, mut cb) = (0usize, 0usize);
        let (mut ba, mut bb) = (64u32, 64u32);
        let a = build_rand_value(bytes, &mut ca, &mut ba, 0);
        let b = build_rand_value(bytes, &mut cb, &mut bb, 0);
        // (1) structurally-equal distinct-node twins: eq, hash-equal, cmp Equal.
        assert!(champ_eq(a, b), "structurally-identical twins are champ_eq");
        assert_eq!(
            champ_hash(a),
            champ_hash(b),
            "…and hash identically (the map-key contract)"
        );
        assert_eq!(
            champ_key_cmp(a, b),
            core::cmp::Ordering::Equal,
            "…and champ_key_cmp Equal"
        );
        // self-consistency: a value equals itself, hashes stably, cmp Equal to itself.
        assert!(champ_eq(a, a));
        assert_eq!(champ_hash(a), champ_hash(a));
        assert_eq!(champ_key_cmp(a, a), core::cmp::Ordering::Equal);
        // (2) a tree from DIFFERENT bytes: whenever champ_key_cmp says Equal, champ_eq must agree, and
        // when not-Equal, champ_eq must be false — cmp and eq never disagree (order/eq consistency).
        let mut flipped = bytes.clone();
        if let Some(first) = flipped.first_mut() {
            *first = first.wrapping_add(1); // perturb the shape/scalar
        } else {
            flipped.push(1);
        }
        let (mut cc, mut bc) = (0usize, 64u32);
        let c = build_rand_value(&flipped, &mut cc, &mut bc, 0);
        let cmp_ac = champ_key_cmp(a, c);
        let eq_ac = champ_eq(a, c);
        assert_eq!(
            cmp_ac == core::cmp::Ordering::Equal,
            eq_ac,
            "champ_key_cmp Equal IFF champ_eq — order and equality must never disagree"
        );
        if eq_ac {
            // if they did come out equal, the hash contract still holds.
            assert_eq!(
                champ_hash(a),
                champ_hash(c),
                "eq ⟹ hash-equal (perturbed tree)"
            );
        }
        // antisymmetry: cmp(a,c) is the reverse of cmp(c,a).
        assert_eq!(
            cmp_ac.reverse(),
            champ_key_cmp(c, a),
            "champ_key_cmp is antisymmetric"
        );
        op_drop(a);
        op_drop(b);
        op_drop(c);
        assert_eq!(
            live_nodes(),
            before,
            "no leak building/comparing random trees"
        );
    });
}

#[test]
fn prop_value_encode_iterative_matches_recursive_over_random_shapes() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
        reset();
        let before = live_nodes();
        let mut table: Vec<S> = Vec::new();
        let (mut cur, mut budget) = (0usize, 40u32);
        let (v, root) = build_rand_value_and_shape(bytes, &mut cur, &mut budget, 0, &mut table);
        let descriptor = Descriptor { table, root };
        // Iterative production walk (what the guest runs).
        let iter_doc = {
            let mut b = DocBuilder::default();
            encode_value(
                &descriptor,
                &mut b,
                &mut Vec::new(),
                &mut Vec::new(),
                v,
                descriptor.root,
            )
            .map(|r| b.finish(r))
        };
        // Recursive reference over the SAME borrowed value + descriptor.
        let rec_doc = {
            let mut b = DocBuilder::default();
            encode_value_recursive(&descriptor, &mut b, v, descriptor.root, 0).map(|r| b.finish(r))
        };
        assert_eq!(
            iter_doc, rec_doc,
            "iterative and recursive value-encode disagree on a random mixed value"
        );
        op_drop(v);
        assert_eq!(
            live_nodes(),
            before,
            "value-encode borrows — no leak building/encoding a random value"
        );
    });
}

#[test]
fn prop_value_encode_leaf_order_is_canon_over_random_shapes() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
        reset();
        let before = live_nodes();
        let mut table: Vec<S> = Vec::new();
        let (mut cur, mut budget) = (0usize, 40u32);
        let (v, root) = build_rand_value_and_shape(bytes, &mut cur, &mut budget, 0, &mut table);
        let descriptor = Descriptor { table, root };
        // Encode via the production iterative walk, then parse the document back.
        let mut b = DocBuilder::default();
        if let Some(r) = encode_value(
            &descriptor,
            &mut b,
            &mut Vec::new(),
            &mut Vec::new(),
            v,
            descriptor.root,
        ) {
            let doc_bytes = b.finish(r);
            let doc = parse_doc(&doc_bytes).expect("a document value-encode produced must parse");
            // Re-walk the struct tree PRE-order LTR; each leaf's FIRST reference must have the next
            // id under canon first-encounter numbering (0, 1, 2, …). Any jump means the leaf pool
            // diverges from codec::encode(canon(tree)) — a post-order-emission regression.
            let mut seen: alloc::collections::BTreeSet<u32> = alloc::collections::BTreeSet::new();
            let mut expected_next: u32 = 0;
            let mut stack: Vec<u32> = vec![doc.root];
            while let Some(struct_ix) = stack.pop() {
                match doc.structs.get(struct_ix as usize) {
                    Some(ParsedStruct::Atom(leaf_id)) => {
                        if !seen.contains(leaf_id) {
                            assert_eq!(
                                *leaf_id, expected_next,
                                "leaf {leaf_id} first-encountered out of canon pre-order \
                                 (expected {expected_next}) on a random value — value-encode's \
                                 leaf pool diverges from codec::encode(canon(tree))"
                            );
                            seen.insert(*leaf_id);
                            expected_next += 1;
                        }
                    }
                    Some(ParsedStruct::List(kids)) => {
                        for &k in kids.iter().rev() {
                            stack.push(k);
                        }
                    }
                    None => panic!("dangling struct index {struct_ix} in a produced document"),
                }
            }
            assert_eq!(
                seen.len() as u32,
                expected_next,
                "every distinct leaf first-encountered exactly once in pre-order"
            );
        }
        op_drop(v);
        assert_eq!(
            live_nodes(),
            before,
            "value-encode borrows — no leak building/encoding a random value"
        );
    });
}

#[test]
fn prop_value_decode_round_trips_over_random_shapes() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
        reset();
        let before = live_nodes();
        let mut table: Vec<S> = Vec::new();
        let (mut cur, mut budget) = (0usize, 40u32);
        let (v, root) = build_rand_value_and_shape(bytes, &mut cur, &mut budget, 0, &mut table);
        let descriptor = Descriptor { table, root };
        // Encode via the production walk; if it declines (a malformed random descriptor), skip — the
        // encode-totality property is covered by prop_value_encode_is_total; here we test the round-trip.
        let mut b = DocBuilder::default();
        if let Some(r) = encode_value(
            &descriptor,
            &mut b,
            &mut Vec::new(),
            &mut Vec::new(),
            v,
            descriptor.root,
        ) {
            let doc_bytes = b.finish(r);
            let parsed = parse_doc(&doc_bytes).expect("a produced document must parse");
            let decoded = decode_value(&descriptor, &parsed, parsed.root, descriptor.root, 0);
            assert_ne!(
                decoded,
                Handle::NULL,
                "value-decode returned NULL on a value value-encode just produced (round-trip must succeed)"
            );
            let eq = value_eq_shaped(&descriptor, decoded, v, descriptor.root);
            assert_eq!(
                eq,
                Some(true),
                "decode ∘ encode must reconstruct a structurally-equal value"
            );
            op_drop(decoded);
        }
        op_drop(v);
        assert_eq!(
            live_nodes(),
            before,
            "no leak across a random encode→decode round-trip"
        );
    });
}

#[test]
fn prop_value_decode_is_total_on_arbitrary_bytes() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
        reset();
        let before = live_nodes();
        // Split the stream into (descriptor, document) at a stream-derived point so both halves are
        // arbitrary and independently sized. An empty half is a valid degenerate input to exercise.
        let split = if bytes.is_empty() {
            0
        } else {
            (bytes[0] as usize) % (bytes.len() + 1)
        };
        let (desc_bytes, doc_bytes) = bytes.split_at(split.min(bytes.len()));
        // The property: NO panic / trap / overflow. Result (Handle or NULL) is not inspected.
        let out = op_value_decode(doc_bytes, desc_bytes);
        core::hint::black_box(out);
        // A decode that DID build a partial value before declining must not leak it; a returned handle
        // is dropped so the leak assertion is exact.
        if out != Handle::NULL {
            op_drop(out);
        }
        assert_eq!(
            live_nodes(),
            before,
            "value-decode leaks nothing on arbitrary bytes (declines cleanly, dropping any partial)"
        );
    });
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
    assert_eq!(
        op_get_int(op_map_iter_key(cur)),
        first_key,
        "fork undisturbed by advance"
    );
    assert_eq!(op_get_int(op_map_iter_val(cur)), full[0].1);
    // The advanced copy is at the SECOND entry.
    assert_eq!(
        op_get_int(op_map_iter_key(advanced)),
        full[1].0,
        "advanced copy moved to successor"
    );
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
    assert_eq!(
        seq, full,
        "independent fork walk reproduces the full sequence"
    );
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
    assert_eq!(
        tail,
        full[1..].to_vec(),
        "advanced copy walks the tail from entry 1"
    );
    op_drop(c);
    op_drop(a);
    op_drop(m);
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across forked walks"
    );
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
    assert_eq!(
        op_get_int(op_set_iter_elem(advanced)),
        full[1],
        "advanced moved on"
    );
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
    assert_eq!(
        seq, full,
        "independent fork walk reproduces the full set sequence"
    );
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
        assert_eq!(
            delta, 0,
            "unique cursor advance allocates zero nodes (step {steps})"
        );
        steps += 1;
    }
    assert_eq!(steps, size, "walked exactly size entries");
    assert_eq!(
        live_nodes(),
        after_iter,
        "LIVE_NODES flat across the whole unique walk"
    );
    op_drop(c);
    op_drop(m);

    // Prove the SHARED path DOES allocate (so the zero above is meaningful, not a no-op op).
    let m2 = deep_walk_map();
    let cur2 = op_map_iter(m2);
    op_dup(cur2); // rc==2 ⇒ copy path
    let pre = live_nodes();
    let adv = op_map_iter_next(cur2);
    assert!(
        live_nodes() - pre > 0,
        "shared cursor advance allocates a fresh cursor node"
    );
    op_drop(cur2);
    op_drop(adv);
    op_drop(m2);
}

#[test]
fn bigint_of_bytes_round_trips_a_beyond_i64_value() {
    // `op_bigint_of_bytes` builds a BigInt leaf from the canonical sign-magnitude bytes of a Bytes leaf
    // — the compiler's beyond-i64 constant materialization. Bake the sign-magnitude bytes of a value
    // LARGER than i64 (1e20 = 10000000000² > i64::MAX ~9.2e18) into a Bytes leaf, build the BigInt, and
    // confirm it equals the SAME value computed by runtime arithmetic. Also confirms it CONSUMES buf.
    reset();
    let expected = op_bigint_mul(
        op_bigint_of_i64(10_000_000_000),
        op_bigint_of_i64(10_000_000_000),
    );
    // The canonical sign-magnitude bytes of a beyond-i64 value (what the compiler would bake).
    let sm_bytes = |v: i128| -> alloc::vec::Vec<u8> {
        let mut buf = [0u8; 32];
        let n = bigint::Big::i128_to_sign_magnitude_bytes_into(v, &mut buf).expect("fits 32");
        buf[..n].to_vec()
    };
    let sm = sm_bytes(100_000_000_000_000_000_000i128);
    let buf = op_bytes_alloc(sm.len() as u32);
    for (i, &b) in sm.iter().enumerate() {
        op_bytes_set(buf, i as u32, b as u32);
    }
    let got = op_bigint_of_bytes(buf); // consumes buf
    assert_eq!(
        op_bigint_cmp(got, expected),
        0,
        "bigint-of-bytes(sign-magnitude bytes of 1e20) equals 1e10 * 1e10"
    );
    // A negative beyond-i64 value round-trips with its sign.
    let neg_sm = sm_bytes(-100_000_000_000_000_000_000i128);
    let nbuf = op_bytes_alloc(neg_sm.len() as u32);
    for (i, &b) in neg_sm.iter().enumerate() {
        op_bytes_set(nbuf, i as u32, b as u32);
    }
    let ngot = op_bigint_of_bytes(nbuf);
    let neg_expected = op_bigint_sub(op_bigint_of_i64(0), expected); // -expected
    assert_eq!(
        op_bigint_cmp(ngot, neg_expected),
        0,
        "bigint-of-bytes of a negative value keeps its sign"
    );
    op_drop(got);
    op_drop(ngot);
    op_drop(expected);
    op_drop(neg_expected);
}

#[test]
fn bigint_of_bytes_flattens_a_rope_input() {
    reset();
    let before = live_object_count();
    // Sign-magnitude bytes of a beyond-i64 value (1e20). `[sign][LE magnitude]` — several bytes.
    let mut buf = [0u8; 32];
    let n =
        bigint::Big::i128_to_sign_magnitude_bytes_into(100_000_000_000_000_000_000i128, &mut buf)
            .expect("fits 32");
    let sm = &buf[..n];
    assert!(
        sm.len() >= 4,
        "the value needs a multi-byte magnitude to span a rope seam"
    );
    // A leaf carrying a byte slice.
    let leaf = |bytes: &[u8]| -> Handle {
        let h = op_bytes_alloc(bytes.len() as u32);
        for (i, &b) in bytes.iter().enumerate() {
            op_bytes_set(h, i as u32, b as u32);
        }
        h
    };
    // FLAT reference.
    let flat_big = op_bigint_of_bytes(leaf(sm)); // consumes
    // ROPE of the same bytes: concat [.. mid] + [mid ..], the seam mid-magnitude.
    let mid = sm.len() / 2;
    let rope = op_bytes_concat(leaf(&sm[..mid]), leaf(&sm[mid..])); // consumes both leaves
    let rope_big = op_bigint_of_bytes(rope); // flattens the rope, then decodes; consumes
    assert_eq!(
        op_bigint_cmp(flat_big, rope_big),
        0,
        "bigint-of-bytes of a ROPE equals the flat leaf — the bytes_flatten materialized it before decoding"
    );
    // And byte-identical leaves (the champ-key / canonical property).
    assert!(
        champ_eq(flat_big, rope_big),
        "the two BigInt leaves are byte-identical (canonical)"
    );
    op_drop(flat_big);
    op_drop(rope_big);
    assert_eq!(
        live_object_count(),
        before,
        "no leak (both byte leaves consumed, both BigInts dropped)"
    );
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
        assert_eq!(
            live_nodes() - pre,
            0,
            "unique set-cursor advance is zero-alloc (step {steps})"
        );
        steps += 1;
    }
    assert_eq!(steps, size);
    assert_eq!(
        live_nodes(),
        after_iter,
        "LIVE_NODES flat across the unique set walk"
    );
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
    assert_eq!(
        op_map_iter_key(c),
        Handle::NULL,
        "exhausted cursor key is NULL"
    );
    assert_eq!(
        op_map_iter_val(c),
        Handle::NULL,
        "exhausted cursor val is NULL"
    );
    // Advance PAST the end several more times (each takes the rc==1 take path, reinstalls empty):
    // must stay exhausted, allocate no node, and not corrupt the (empty) frame set.
    for _ in 0..3 {
        let pre = live_nodes();
        c = op_map_iter_next(c);
        assert_eq!(
            live_nodes() - pre,
            0,
            "advancing an exhausted unique cursor allocates nothing"
        );
        assert_eq!(
            op_map_iter_key(c),
            Handle::NULL,
            "still exhausted after over-advance"
        );
    }
    op_drop(c);
    op_drop(m);
    assert_eq!(
        live_nodes(),
        before,
        "no frame leaked or double-freed across the over-walk"
    );
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
    assert_eq!(
        walk_a, walk_b,
        "two FBIP map walks are identically ordered (deterministic)"
    );
    assert_eq!(
        walk_a.len(),
        op_map_size(m) as usize,
        "map walk visited exactly size entries"
    );
    let keys: std::collections::HashSet<i64> = walk_a.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        keys.len(),
        walk_a.len(),
        "each map key visited exactly once (incl. collision)"
    );
    op_drop(m);

    let (ca, cb) = full_hash_collision_pair();
    let mut s = op_set_empty();
    for &e in &[ca, cb, 1i64, 2, 3, 40, 41] {
        s = sinsert_int(s, e);
    }
    let sa = collect_set(s);
    let sb = collect_set(s);
    assert_eq!(sa, sb, "two FBIP set walks are identically ordered");
    assert_eq!(
        sa.len(),
        op_set_size(s) as usize,
        "set walk visited exactly size entries"
    );
    let selems: std::collections::HashSet<i64> = sa.iter().copied().collect();
    assert_eq!(
        selems.len(),
        sa.len(),
        "each set elem visited once (incl. collision)"
    );
    assert!(
        selems.contains(&ca) && selems.contains(&cb),
        "collision elems both visited"
    );
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
        0i64,
        1 << 5,
        1 << 10,
        (1 << 5) | (1 << 10),
        (1 << 10) | (1 << 15),
        1,
        2,
        ca,
        cb,
        7,
        8,
        40,
        41,
    ];
    // Map each key to a small, collision-safe tag value (a running index, not k*const — the
    // collision pair carries full-width i64 payloads that would overflow a multiply).
    let reference: std::collections::BTreeMap<i64, i64> = deep
        .iter()
        .enumerate()
        .map(|(i, &k)| (k, 1000 + i as i64))
        .collect();
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
    assert_eq!(
        seen, reference,
        "in-place walk visited exactly the reference key→value map"
    );
    // Over-advance the exhausted cursor a few times (each an rc==1 advance on empty frames).
    for _ in 0..3 {
        cur = op_map_iter_next(cur);
        assert_eq!(op_map_iter_key(cur), Handle::NULL, "stays exhausted");
    }
    op_drop(cur);
    op_drop(m);
    assert_eq!(
        live_nodes(),
        before,
        "frame refcounts balanced — no leak, no double-free"
    );
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
    assert_eq!(
        steps,
        ks.len(),
        "walked every entry (deepest paths included)"
    );
    op_drop(cur);
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn set_union_matches_reference() {
    reset();
    let before = live_nodes();
    let (sa, sb) = low5_split_pair(); // force subnode splits into the operands
    let (ca, cb) = full_hash_collision_pair(); // a collision pair spanning both operands
    // Overlapping, disjoint, subset, identical — encoded as element-set pairs over a universe.
    let cases: Vec<(Vec<i64>, Vec<i64>)> = vec![
        (vec![1, 2, 3], vec![3, 4, 5]),               // overlapping
        (vec![1, 2, 3], vec![10, 11, 12]),            // disjoint
        (vec![1, 2, 3, 4, 5], vec![2, 4]),            // b subset of a
        (vec![7, 8, 9], vec![7, 8, 9]),               // identical
        (vec![sa, sb, 3, 17, 42], vec![sb, 42, 100]), // subnode splits + overlap
        (vec![ca, 1, 2], vec![cb, 2, 3]),             // collision pair split across operands
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
    assert!(
        champ_eq(ab, ba),
        "union(big,small) == union(small,big) byte-identically"
    );
    assert_eq!(champ_hash(ab), champ_hash(ba));
    assert!(
        champ_eq(ab, fresh),
        "union == a fresh set of all elements (canonical)"
    );
    assert_eq!(champ_hash(ab), champ_hash(fresh));
    assert_eq!(
        op_set_size(ab) as usize,
        all.len(),
        "union has every distinct element once"
    );
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

    assert_eq!(
        live_nodes(),
        before,
        "self-op short-circuits balanced all refs — no leak/double-free"
    );
}

#[test]
fn set_intersection_matches_reference() {
    reset();
    let before = live_nodes();
    let (sa, sb) = low5_split_pair();
    let (ca, cb) = full_hash_collision_pair();
    let cases: Vec<(Vec<i64>, Vec<i64>)> = vec![
        (vec![1, 2, 3], vec![3, 4, 5]),
        (vec![1, 2, 3], vec![10, 11, 12]), // disjoint ⇒ empty
        (vec![1, 2, 3, 4, 5], vec![2, 4]), // ⇒ {2,4}
        (vec![7, 8, 9], vec![7, 8, 9]),    // identical ⇒ itself
        (vec![sa, sb, 3, 17, 42], vec![sb, 42, 100]), // ⇒ {sb,42}
        (vec![ca, cb, 1], vec![ca, 2]),    // one collision elem shared
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
    assert!(
        champ_eq(via_h, via_plain),
        "set_insert_h builds the same canonical set as op_set_insert"
    );
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
        (vec![1, 2, 3], vec![3, 4, 5]),               // ⇒ {1,2}
        (vec![1, 2, 3], vec![10, 11, 12]),            // disjoint ⇒ a
        (vec![1, 2, 3, 4, 5], vec![2, 4]),            // ⇒ {1,3,5}
        (vec![7, 8, 9], vec![7, 8, 9]),               // identical ⇒ empty
        (vec![sa, sb, 3, 17, 42], vec![sb, 42, 100]), // ⇒ {sa,3,17}
        (vec![ca, cb, 1], vec![ca, 2]),               // ⇒ {cb,1}
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
    assert_eq!(
        live_nodes(),
        before,
        "no leak across empty-operand identities"
    );
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
    assert!(
        champ_eq(r, fresh),
        "union result is canonical (== differently-ordered fold)"
    );
    assert_eq!(
        champ_hash(r),
        champ_hash(fresh),
        "byte-identical canonical shape"
    );
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
    assert_eq!(
        collect_set(a),
        a_snapshot,
        "retained operand a unchanged after consuming union"
    );
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
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free with shared operands"
    );
}

#[test]
fn set_algebra_fuzz_matches_reference() {
    reset();
    let before = live_nodes();
    // Fixed-seed LCG: random element sets over a small universe, all three ops vs BTreeSet.
    let mut lcg: u64 = 0xA5A5_1234;
    let next = |lcg: &mut u64| {
        *lcg = lcg
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
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
    assert_eq!(
        op_get_int(op_vec_get(l, 9)),
        oracle[9],
        "left neighbor untouched"
    );
    assert_vec_invariants(l);
    assert_vec_invariants(r);
    // concat the two halves back together
    let joined = op_vec_concat(l, r);
    assert_eq!(op_vec_len(joined), 73 + 87);
    assert_vec_invariants(joined);
    op_drop(joined);
    assert_eq!(
        live_nodes(),
        before,
        "no leak after relaxed split + downstream ops"
    );
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
    assert!(
        vec_is_relaxed(root),
        "deep fold-concat must produce a relaxed root"
    );
    assert_eq!(op_vec_len(acc), total);
    // Split at several interior points; keep acc alive by dup-before-split.
    for &p in &[1u32, 29, 30, 75, 135, 168, 257] {
        op_dup(acc);
        let (l, r) = op_vec_split(acc, p);
        assert_eq!(op_vec_len(l), p, "deep left len (p={p})");
        assert_eq!(op_vec_len(r), total - p, "deep right len (p={p})");
        assert_eq!(
            vec_to_ints(l),
            oracle[..p as usize].to_vec(),
            "deep left elems (p={p})"
        );
        assert_eq!(
            vec_to_ints(r),
            oracle[p as usize..].to_vec(),
            "deep right elems (p={p})"
        );
        assert_vec_invariants(l);
        assert_vec_invariants(r);
        op_drop(l);
        op_drop(r);
    }
    op_drop(acc);
    assert_eq!(live_nodes(), before, "no leak across deep relaxed splits");
}

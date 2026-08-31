use super::*;

#[test]
fn cdz_abi_imm_unit_constant_matches_imm_unit_bits() {
    let unit_bits = imm_unit().0 as usize as u32;
    assert_eq!(
        unit_bits.to_le_bytes(),
        CDZ_ABI_IMM_UNIT,
        "the `cdz-abi` IMM_UNIT constant the compiler emits ({CDZ_ABI_IMM_UNIT:?}) MUST equal \
         imm_unit()'s LE bit pattern ({:?}) — a divergence miscompiles the None arm of \
         List.at/Map.lookup/String.at/Bytes.at (spec@5d9a1dc1 emits IMM_UNIT there directly)",
        unit_bits.to_le_bytes()
    );
    // The whole contract the emit relies on: op_arr_alloc(0) IS the inline unit, byte-identical to the
    // constant. A 0-length array, an empty tuple/record — all the inline unit immediate.
    assert_eq!(
        op_arr_alloc(0).0 as usize as u32,
        unit_bits,
        "op_arr_alloc(0) must be the same inline unit the IMM_UNIT constant denotes"
    );
}

#[test]
fn rc_trace_records_events_and_attributes_a_leak() {
    // The rc-trace diagnostic: enable recording, exercise a balanced node + a leaked one, and confirm
    // the event log gives the attribution static op-counts cannot — WHICH handle leaked (alloc'd, never
    // reached a freed drop) vs a fully-reclaimed one.
    rc_trace_enable(true);
    // A leaf (raw-bearing, 0 handles): alloc (rc 0→1), dup (1→2), drop (2→1), drop (1→0 freed).
    let balanced_h = alloc(alloc::vec::Vec::new(), alloc::vec::Vec::from([1u8, 2, 3]));
    op_dup(balanced_h);
    op_drop(balanced_h);
    op_drop(balanced_h);
    // A second leaf, alloc'd and NEVER dropped — the leak.
    let leaked_h = alloc(alloc::vec::Vec::new(), alloc::vec::Vec::from([9u8]));
    let (events, truncated) = rc_trace_snapshot();
    rc_trace_enable(false);

    assert!(
        !truncated,
        "a handful of events must not overflow RC_TRACE_CAP"
    );
    let allocs: alloc::vec::Vec<_> = events.iter().filter(|e| e.op == RC_TRACE_ALLOC).collect();
    assert_eq!(
        allocs.len(),
        2,
        "exactly two ALLOC events recorded in the enabled window"
    );
    let (balanced, leaked) = (allocs[0].node, allocs[1].node);
    assert_ne!(balanced, leaked, "node ids are unique per alloc");

    // The balanced node: a DUP 1→2 and a final freed DROP reaching rc0; tagged Leaf (0 handles + raw).
    let bal: alloc::vec::Vec<_> = events.iter().filter(|e| e.node == balanced).collect();
    assert_eq!(bal[0].op, RC_TRACE_ALLOC);
    assert_eq!(
        bal[0].tag, RC_TAG_LEAF,
        "a raw-bearing 0-handle node is structurally a Leaf"
    );
    assert!(
        bal.iter()
            .any(|e| e.op == RC_TRACE_DUP && e.rc_before == 1 && e.rc_after == 2),
        "the dup is recorded with rc 1→2"
    );
    assert!(
        bal.iter()
            .any(|e| e.op == RC_TRACE_DROP && e.freed && e.rc_after == 0),
        "the balanced node reaches a freed drop (rc0)"
    );

    // The leaked node: an ALLOC but NO freed DROP — the definitive leak-attribution signal.
    let lk: alloc::vec::Vec<_> = events.iter().filter(|e| e.node == leaked).collect();
    assert_eq!(lk.len(), 1, "only the ALLOC event — nothing released it");
    assert!(
        !lk.iter().any(|e| e.op == RC_TRACE_DROP && e.freed),
        "the leaked handle never reaches a freed drop — attributable as node#{leaked}"
    );

    // Reclaim the intentional leak so LIVE_NODES nets to zero for sibling tests.
    op_drop(leaked_h);
}

#[test]
fn node_layout_sizes_are_pinned_native() {
    use core::mem::size_of;
    assert_eq!(
        size_of::<Node>(),
        72,
        "Node size changed — a bloat is paid by every heap value. NOTE: this is the NATIVE/debug \
         layout, which carries the debug-ONLY `guard` (UAF) + `node_id` (rc-trace attribution) \
         fields; both are `#[cfg(any(test, feature=\"debug-counters\"))]`, ABSENT from the shipped \
         Node, so the RELEASE layout + REQUIRED_RUNTIME_HASH are byte-unchanged. Was 64 before the \
         rc-trace `node_id` field (+8 w/ alignment) — debug-only diagnostic cost, no release bloat."
    );
    assert_eq!(
        size_of::<Handles>(),
        32,
        "Handles size changed (inline [Handle;2] arm + tag, or Vec)"
    );
    assert_eq!(
        size_of::<Raw>(),
        24,
        "Raw size changed (inline [u8;12]+len arm, or Vec)"
    );
    assert_eq!(size_of::<Handle>(), 8, "Handle is a single native pointer");
    // The inline-raw cap is the CHAMP header size — the largest hot raw that must stay inline.
    assert_eq!(
        INLINE_RAW_CAP, 12,
        "INLINE_RAW_CAP must fit a CHAMP header inline"
    );
}

#[test]
fn read_u32_at_fast_and_padded_paths_agree() {
    // Independent reference: exactly the zero-padded byte-by-byte definition.
    fn reference(raw: &[u8], off: usize) -> u32 {
        let mut b = [0u8; 4];
        for k in 0..4 {
            if let Some(&byte) = raw.get(off + k) {
                b[k] = byte;
            }
        }
        u32::from_le_bytes(b)
    }
    let raws: &[&[u8]] = &[
        &[],                                      // absent: fallback only
        &[0xAB],                                  // 1 byte: fallback (3 zero-padded)
        &[0x01, 0x02],                            // 2 bytes
        &[0xFF, 0xEE, 0xDD],                      // 3 bytes: fallback (1 zero-padded)
        &[0x78, 0x56, 0x34, 0x12],                // exactly 4: fast path, no pad
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], // 12-byte CHAMP-header-sized raw
    ];
    for raw in raws {
        // Probe every offset from 0 to just past the end — covers in-bounds (fast) and past-end
        // (fallback, including a partial window straddling the end).
        for off in 0..=raw.len() + 2 {
            assert_eq!(
                read_u32_at(raw, off),
                reference(raw, off),
                "read_u32_at(raw len {}, off {off}) disagrees with the zero-padded reference",
                raw.len()
            );
        }
    }
}

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
    assert!(
        !is_immediate(real),
        "a real alloc'd Node must not read as immediate"
    );
    assert!(
        !is_immediate(Handle::NULL),
        "NULL is tag 00 → not immediate"
    );
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
    for h in [
        imm_unit(),
        imm_bool(true),
        imm_bool(false),
        imm_int(5),
        imm_int(-7),
    ] {
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
        assert!(
            is_immediate(round),
            "round-tripped handle must still be immediate"
        );
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
    for &h in &[
        imm_unit(),
        imm_bool(true),
        imm_bool(false),
        imm_int(0),
        imm_int(1),
        imm_int(FIXNUM_MAX),
    ] {
        let round = Handle((h.0 as usize as u32) as usize as *mut Node);
        assert_eq!(
            round, h,
            "u32 ABI round-trip must be bit-identical for low-32-bit immediates"
        );
    }
}

#[test]
fn producers_normalize_to_immediates() {
    reset();
    // Normalize-on-construct: a bool / unit value is now ALWAYS inline, never a boxed Node.
    assert!(is_immediate(op_box_bool(true)));
    assert!(is_immediate(op_box_bool(false)));
    assert!(
        is_immediate(op_arr_alloc(0)),
        "empty array (unit) must inline"
    );
    // Since P2 a small in-window int ALSO inlines (op_box_int normalizes); an out-of-window int
    // still boxes, and a non-empty array still allocates.
    assert!(
        is_immediate(op_box_int(5)),
        "an in-window int inlines since P2"
    );
    assert!(
        !is_immediate(op_box_int((1 << 30) as i64)),
        "an out-of-window int still boxes"
    );
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
        render(t, &Shape::Tuple(vec![Shape::Bool, Shape::Int].into())),
        "(tuple true 9)"
    );
    op_drop(t);
    assert_eq!(
        live_nodes(),
        before,
        "array reclaimed; both inline scalars leave nothing"
    );
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
    assert_eq!(
        render(t, &Shape::Tuple(vec![Shape::Tuple(vec![].into())])),
        "(tuple unit)"
    );
    assert_eq!(
        op_arr_len(op_arr_get(t, 0)),
        0,
        "the inline unit element has 0 slots"
    );
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
    assert_eq!(
        render(op_arr_alloc(0), &Shape::Tuple(vec![].into())),
        "unit"
    );
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
    assert_eq!(
        live_nodes(),
        before,
        "list-of-bools fully reclaimed; inline bools leave nothing"
    );
}

#[test]
fn op_box_int_normalizes_at_boundary() {
    reset();
    // A value that FITS the window is ALWAYS inline; just outside, it boxes. THE single boundary.
    assert!(is_immediate(op_box_int(FIXNUM_MAX)), "FIXNUM_MAX inlines");
    assert!(
        !is_immediate(op_box_int(FIXNUM_MAX + 1)),
        "FIXNUM_MAX+1 boxes"
    );
    assert!(is_immediate(op_box_int(FIXNUM_MIN)), "FIXNUM_MIN inlines");
    assert!(
        !is_immediate(op_box_int(FIXNUM_MIN - 1)),
        "FIXNUM_MIN-1 boxes"
    );
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
fn bigint_ops_wire_the_limb_library_through_heap_leaves() {
    reset();
    // B3a: the `op_bigint_*` WIT-op glue boxes a `Big` into a sign-magnitude heap LEAF and reads it
    // back through the arithmetic. A BigInt leaf is ALWAYS heap (never a fixnum immediate) — distinct
    // type. The `bigint::Big` arithmetic itself is differential-tested vs num-bigint in `bigint.rs`;
    // this pins the WIRING (box/unbox round-trip + each op threading handles), the B3a deliverable.
    // Every value here fits i64 for a readable round-trip, but the rep is the unbounded limb leaf.
    for v in [0i64, 1, -1, 42, -42, 1_000_000, i64::MAX, i64::MIN] {
        let h = op_bigint_of_i64(v);
        assert!(
            !is_immediate(h),
            "a BigInt leaf is always heap, never an immediate ({v})"
        );
        assert_eq!(
            op_bigint_to_i64_checked(h),
            v,
            "bigint-of-i64 / to-i64-checked round-trip for {v}"
        );
        op_drop(h);
    }
    // Arithmetic threads handles: (6 op 2) for each op, checked back to i64.
    let check = |op: fn(Handle, Handle) -> Handle, a: i64, b: i64, want: i64, name: &str| {
        let (ha, hb) = (op_bigint_of_i64(a), op_bigint_of_i64(b));
        let hr = op(ha, hb);
        assert_eq!(op_bigint_to_i64_checked(hr), want, "bigint {name}");
        op_drop(ha);
        op_drop(hb);
        op_drop(hr);
    };
    check(op_bigint_add, 6, 2, 8, "add");
    check(op_bigint_sub, 6, 2, 4, "sub");
    check(op_bigint_mul, 6, 2, 12, "mul");
    check(op_bigint_div, 7, 2, 3, "div (truncating)");
    check(op_bigint_div, -7, 2, -3, "div toward zero");
    // cmp: -1 / 0 / 1.
    let three_way = |a: i64, b: i64| {
        let (ha, hb) = (op_bigint_of_i64(a), op_bigint_of_i64(b));
        let c = op_bigint_cmp(ha, hb);
        op_drop(ha);
        op_drop(hb);
        c
    };
    assert_eq!(three_way(1, 2), -1);
    assert_eq!(three_way(2, 2), 0);
    assert_eq!(three_way(3, 2), 1);
    // A product that OVERFLOWS i64 is a valid BigInt (never traps) but does NOT narrow back.
    let (ha, hb) = (op_bigint_of_i64(i64::MAX), op_bigint_of_i64(2));
    let big = op_bigint_mul(ha, hb);
    assert!(
        !is_immediate(big),
        "the overflowing product is a heap BigInt"
    );
    // (No trap on the mul itself — magnitude grows. Narrowing THAT back would trap; not exercised
    // here to keep the test panic-free — the trap path is a compiler/gate concern.)
    op_drop(ha);
    op_drop(hb);
    op_drop(big);
    assert_eq!(live_object_count(), 0, "no BigInt leak");
}

#[test]
fn bigint_div_truncates_toward_zero_all_signs_and_cmp_orders_negatives() {
    reset();
    let before = live_object_count();
    let div = |a: i64, b: i64| -> i64 {
        let (ha, hb) = (op_bigint_of_i64(a), op_bigint_of_i64(b));
        let hr = op_bigint_div(ha, hb);
        let r = op_bigint_to_i64_checked(hr);
        op_drop(ha);
        op_drop(hb);
        op_drop(hr);
        r
    };
    // Truncation toward zero (Rust `/` on i64 is the reference): all four sign combos.
    for &(a, b) in &[
        (7, 2),
        (-7, 2),
        (7, -2),
        (-7, -2),
        (1, 3),
        (-1, 3),
        (1, -3),
        (-1, -3),
        (100, 7),
        (-100, 7),
    ] {
        assert_eq!(
            div(a, b),
            a / b,
            "bigint-div {a}/{b} truncates toward zero (matches i64 /)"
        );
    }
    // Exact division has no rounding either way; a dividend smaller than the divisor → 0 (toward zero).
    assert_eq!(div(6, 3), 2);
    assert_eq!(div(-6, 3), -2);
    assert_eq!(div(2, 5), 0, "|dividend| < |divisor| → 0");
    assert_eq!(
        div(-2, 5),
        0,
        "…and stays 0, not -1 (toward zero, not floor)"
    );
    // bigint-cmp orders negatives correctly: -5 < -3 < 0 < 3 < 5.
    let cmp = |a: i64, b: i64| -> i64 {
        let (ha, hb) = (op_bigint_of_i64(a), op_bigint_of_i64(b));
        let c = op_bigint_cmp(ha, hb);
        op_drop(ha);
        op_drop(hb);
        c
    };
    assert_eq!(cmp(-5, -3), -1, "-5 < -3");
    assert_eq!(cmp(-3, -5), 1, "-3 > -5");
    assert_eq!(cmp(-3, 3), -1, "-3 < 3");
    assert_eq!(cmp(-3, -3), 0, "-3 == -3");
    assert_eq!(cmp(0, -1), 1, "0 > -1");
    assert_eq!(cmp(-1, 0), -1, "-1 < 0");
    assert_eq!(live_object_count(), before, "no BigInt leak");
}

#[test]
fn bigint_of_i64_direct_i128_box_is_byte_identical_to_the_big_route() {
    reset();
    let before = live_object_count();
    for v in [
        0i64,
        1,
        -1,
        255,
        256,
        -256,
        i32::MAX as i64,
        i32::MIN as i64,
        4_294_967_296, // 2^32 — crosses the single→double limb boundary in Big::from_i64
        -4_294_967_296,
        i64::MAX,
        i64::MIN, // unsigned_abs limb-boundary endpoint
        1_000_003,
        -1_000_003,
    ] {
        let via_direct = op_bigint_of_i64(v); // the new direct-i128 path
        let via_big = box_bigint(&bigint::Big::from_i64(v)); // the old Big route (oracle)
        assert!(
            champ_eq(via_direct, via_big),
            "bigint-of-i64({v}): direct-i128 leaf must be byte-identical (champ_eq) to the Big route"
        );
        assert_eq!(
            op_bigint_to_i64_checked(via_direct),
            v,
            "bigint-of-i64({v}) round-trips through to-i64-checked"
        );
        op_drop(via_direct);
        op_drop(via_big);
    }
    assert_eq!(live_object_count(), before, "no BigInt leak");
}

#[test]
fn bigint_rem_takes_dividend_sign_all_combos() {
    reset();
    let before = live_object_count();
    let rem = |a: i64, b: i64| -> i64 {
        let (ha, hb) = (op_bigint_of_i64(a), op_bigint_of_i64(b));
        let hr = op_bigint_rem(ha, hb);
        let r = op_bigint_to_i64_checked(hr);
        op_drop(ha);
        op_drop(hb);
        op_drop(hr);
        r
    };
    // Remainder matches Rust i64 `%` (dividend's sign) across all four sign combos + exact/zero cases.
    for &(a, b) in &[
        (17, 5),
        (-17, 5),
        (17, -5),
        (-17, -5),
        (6, 3),
        (-6, 3),
        (2, 5),
        (-2, 5),
        (0, 7),
    ] {
        assert_eq!(rem(a, b), a % b, "bigint-rem {a} % {b} == i64 %");
        // And the division identity: a == (a/b)*b + (a%b). The bigint ops BORROW their operands, so
        // each intermediate handle must be dropped explicitly (no consuming chain).
        let (ha, hb) = (op_bigint_of_i64(a), op_bigint_of_i64(b));
        let hq = op_bigint_div(ha, hb);
        let hr = op_bigint_rem(ha, hb);
        let qb = op_bigint_mul(hq, hb);
        let sum = op_bigint_add(qb, hr);
        assert_eq!(
            op_bigint_to_i64_checked(sum),
            a,
            "a == (a/b)*b + (a%b) for {a},{b}"
        );
        op_drop(ha);
        op_drop(hb);
        op_drop(hq);
        op_drop(hr);
        op_drop(qb);
        op_drop(sum);
    }
    assert_eq!(live_object_count(), before, "no BigInt leak");
}

#[test]
#[should_panic]
fn bigint_div_by_zero_traps() {
    reset();
    let (a, b) = (op_bigint_of_i64(10), op_bigint_of_i64(0));
    let _ = op_bigint_div(a, b); // fail-fast: division by zero
}

#[test]
#[should_panic]
fn bigint_rem_by_zero_traps() {
    reset();
    let (a, b) = (op_bigint_of_i64(10), op_bigint_of_i64(0));
    let _ = op_bigint_rem(a, b); // fail-fast: remainder by zero
}

#[test]
#[should_panic]
fn rational_of_zero_denominator_traps() {
    reset();
    let (n, d) = (op_bigint_of_i64(1), op_bigint_of_i64(0));
    let _ = op_rational_of(n, d); // fail-fast: zero denominator (consumes n, d)
}

#[test]
fn rational_ops_normalize_and_compute_over_bigint_children() {
    reset();
    // R3a: a Rational is a normalized 2-handle node `[num, den]` of BigInt leaves. `rational-of`
    // normalizes (gcd-reduce, sign on num, denom > 0); the arithmetic is exact; `rational-cmp` orders
    // by value. Build a rational from two BigInt handles (which `rational-of` CONSUMES) and read the
    // normalized components back via `rational-num`/`rational-den`.
    let rat = |n: i64, d: i64| op_rational_of(op_bigint_of_i64(n), op_bigint_of_i64(d));
    let read = |r: Handle| -> (i64, i64) {
        let nh = op_rational_num(r);
        let dh = op_rational_den(r);
        let v = (op_bigint_to_i64_checked(nh), op_bigint_to_i64_checked(dh));
        op_drop(nh);
        op_drop(dh);
        v
    };
    // NORMALIZATION: 2/4 → 1/2; sign onto numerator; both-negative cancels; 0/d → 0/1; whole → n/1.
    for &((n, d), want) in &[
        ((1i64, 2i64), (1i64, 2i64)),
        ((2, 4), (1, 2)),
        ((6, 8), (3, 4)),
        ((1, -2), (-1, 2)),
        ((-1, -2), (1, 2)),
        ((0, 5), (0, 1)),
        ((10, 5), (2, 1)),
        ((7, 1), (7, 1)),
    ] {
        let r = rat(n, d);
        assert_eq!(read(r), want, "normalize {n}/{d}");
        op_drop(r);
    }
    // ARITHMETIC (exact): 1/3 + 1/6 = 1/2; 1/2 - 3/4 = -1/4; (2/3)*(3/4) = 1/2; (1/2)/(3/4) = 2/3.
    let check = |op: fn(Handle, Handle) -> Handle,
                 a: (i64, i64),
                 b: (i64, i64),
                 want: (i64, i64),
                 name: &str| {
        let (ra, rb) = (rat(a.0, a.1), rat(b.0, b.1));
        let r = op(ra, rb);
        assert_eq!(read(r), want, "rational {name}");
        op_drop(ra);
        op_drop(rb);
        op_drop(r);
    };
    check(op_rational_add, (1, 3), (1, 6), (1, 2), "add");
    check(op_rational_sub, (1, 2), (3, 4), (-1, 4), "sub");
    check(op_rational_mul, (2, 3), (3, 4), (1, 2), "mul");
    check(op_rational_div, (1, 2), (3, 4), (2, 3), "div");
    // CMP: 1/3 < 1/2 (-1), 1/2 = 2/4 (0), 3/4 > 1/2 (1).
    let cmp = |a: (i64, i64), b: (i64, i64)| {
        let (ra, rb) = (rat(a.0, a.1), rat(b.0, b.1));
        let c = op_rational_cmp(ra, rb);
        op_drop(ra);
        op_drop(rb);
        c
    };
    assert_eq!(cmp((1, 3), (1, 2)), -1);
    assert_eq!(cmp((1, 2), (2, 4)), 0);
    assert_eq!(cmp((3, 4), (1, 2)), 1);
    // Two EQUAL rationals reached differently are byte-identical (champ_eq basis): 2/4 and 1/2.
    let (r1, r2) = (rat(2, 4), rat(1, 2));
    assert!(
        champ_eq(r1, r2),
        "2/4 and 1/2 are the same normalized rational"
    );
    op_drop(r1);
    op_drop(r2);
    assert_eq!(live_object_count(), 0, "no Rational leak");
}

#[test]
fn rational_ops_match_an_i128_fraction_reference_under_random_pairs() {
    // i128 gcd (Euclid, non-negative) + normalize: lowest terms, den > 0, sign on num. Matches
    // `normalize_rational`'s contract. Inputs are bounded so cross-multiplication stays within i128.
    fn gcd_i128(mut a: i128, mut b: i128) -> i128 {
        a = a.abs();
        b = b.abs();
        while b != 0 {
            let t = a % b;
            a = b;
            b = t;
        }
        a
    }
    fn norm(mut n: i128, mut d: i128) -> (i128, i128) {
        if d < 0 {
            n = -n;
            d = -d;
        }
        let g = gcd_i128(n, d);
        if g == 0 { (0, 1) } else { (n / g, d / g) }
    }
    // Read a runtime rational's normalized (num, den) as i128 (fits — operands are small).
    fn read(r: Handle) -> (i128, i128) {
        let (nh, dh) = (op_rational_num(r), op_rational_den(r));
        let v = (
            op_bigint_to_i64_checked(nh) as i128,
            op_bigint_to_i64_checked(dh) as i128,
        );
        op_drop(nh);
        op_drop(dh);
        v
    }
    let rat = |n: i64, d: i64| op_rational_of(op_bigint_of_i64(n), op_bigint_of_i64(d));
    // Two bytes → a (num in −128..127, den in 1..64) pair; den forced nonzero.
    bolero::check!()
        .with_type::<(i8, u8, i8, u8)>()
        .for_each(|&(an, ad, bn, bd)| {
            let before = live_object_count();
            let (a_n, a_d) = (an as i64, (ad % 63 + 1) as i64); // den ∈ 1..=63, never 0
            let (b_n, b_d) = (bn as i64, (bd % 63 + 1) as i64);
            let (ra, rb) = (rat(a_n, a_d), rat(b_n, b_d));
            // Construction normalizes — must match the reference.
            assert_eq!(read(ra), norm(a_n as i128, a_d as i128), "of {a_n}/{a_d}");
            assert_eq!(read(rb), norm(b_n as i128, b_d as i128), "of {b_n}/{b_d}");
            let (ran, rad) = norm(a_n as i128, a_d as i128);
            let (rbn, rbd) = norm(b_n as i128, b_d as i128);
            // add/sub/mul: cross-multiply over the NORMALIZED reference components, then renormalize.
            let add = op_rational_add(ra, rb);
            assert_eq!(
                read(add),
                norm(ran * rbd + rbn * rad, rad * rbd),
                "add {a_n}/{a_d} {b_n}/{b_d}"
            );
            let sub = op_rational_sub(ra, rb);
            assert_eq!(
                read(sub),
                norm(ran * rbd - rbn * rad, rad * rbd),
                "sub {a_n}/{a_d} {b_n}/{b_d}"
            );
            let mul = op_rational_mul(ra, rb);
            assert_eq!(
                read(mul),
                norm(ran * rbn, rad * rbd),
                "mul {a_n}/{a_d} {b_n}/{b_d}"
            );
            // cmp: cross-multiply (both dens > 0). Sign of (ran*rbd − rbn*rad).
            let want_cmp = (ran * rbd - rbn * rad).signum() as i64;
            assert_eq!(
                op_rational_cmp(ra, rb),
                want_cmp,
                "cmp {a_n}/{a_d} {b_n}/{b_d}"
            );
            let mut live = alloc::vec![add, sub, mul];
            // div: only when b ≠ 0 (else it TRAPS — the fixed test covers that path).
            if rbn != 0 {
                let div = op_rational_div(ra, rb);
                assert_eq!(
                    read(div),
                    norm(ran * rbd, rad * rbn),
                    "div {a_n}/{a_d} {b_n}/{b_d}"
                );
                live.push(div);
            }
            for h in live {
                op_drop(h);
            }
            op_drop(ra);
            op_drop(rb);
            assert_eq!(
                live_object_count(),
                before,
                "no leak across the random rational ops"
            );
        });
}

#[test]
fn bigint_value_equal_leaves_are_byte_identical_champ_eq_and_hash() {
    reset();
    let before = live_object_count();
    // Same value, three different computations: 8 = of(8) = 6+2 = 10-2 = 2*4.
    let direct = op_bigint_of_i64(8);
    let via_add = {
        let (a, b) = (op_bigint_of_i64(6), op_bigint_of_i64(2));
        let r = op_bigint_add(a, b);
        op_drop(a);
        op_drop(b);
        r
    };
    let via_sub = {
        let (a, b) = (op_bigint_of_i64(10), op_bigint_of_i64(2));
        let r = op_bigint_sub(a, b);
        op_drop(a);
        op_drop(b);
        r
    };
    let via_mul = {
        let (a, b) = (op_bigint_of_i64(2), op_bigint_of_i64(4));
        let r = op_bigint_mul(a, b);
        op_drop(a);
        op_drop(b);
        r
    };
    for (name, h) in [("add", via_add), ("sub", via_sub), ("mul", via_mul)] {
        assert!(
            champ_eq(direct, h),
            "8-via-{name} is champ_eq to of(8) (same map key)"
        );
        assert_eq!(
            champ_hash(direct),
            champ_hash(h),
            "8-via-{name} hashes identically"
        );
        assert_eq!(op_bigint_cmp(direct, h), 0, "…and cmp == 0");
        op_drop(h);
    }
    op_drop(direct);

    // The ZERO canonicality trap: `x - x`, `0 * x`, and `of(0)` must ALL be the SAME canonical zero
    // leaf (no `-0`, empty magnitude) — else a computed zero key wouldn't match a literal zero key.
    let zero_of = op_bigint_of_i64(0);
    let x = op_bigint_of_i64(-12345);
    let zero_sub = {
        let x2 = op_bigint_of_i64(-12345);
        let r = op_bigint_sub(x, x2);
        op_drop(x2);
        r
    };
    let zero_mul = {
        let z = op_bigint_of_i64(0);
        let r = op_bigint_mul(x, z);
        op_drop(z);
        r
    };
    assert!(
        champ_eq(zero_of, zero_sub),
        "x - x is the canonical zero (no -0)"
    );
    assert!(champ_eq(zero_of, zero_mul), "0 * x is the canonical zero");
    assert_eq!(
        champ_hash(zero_of),
        champ_hash(zero_sub),
        "zero hashes identically (sub)"
    );
    assert_eq!(
        champ_hash(zero_of),
        champ_hash(zero_mul),
        "zero hashes identically (mul)"
    );
    // A negative and its positive counterpart must DIFFER (sign is part of the canonical form).
    let neg = op_bigint_of_i64(-7);
    let pos = op_bigint_of_i64(7);
    assert!(
        !champ_eq(neg, pos),
        "-7 and 7 are distinct leaves (sign in the canonical bytes)"
    );
    op_drop(zero_of);
    op_drop(x);
    op_drop(zero_sub);
    op_drop(zero_mul);
    op_drop(neg);
    op_drop(pos);
    assert_eq!(live_object_count(), before, "no leak");
}

#[test]
fn bigint_to_i64_checked_traps_just_past_the_boundary() {
    reset();
    // In range: the extremes narrow back exactly (also covered above, re-pinned here beside the trap).
    for v in [i64::MAX, i64::MIN, 0, -1] {
        let h = op_bigint_of_i64(v);
        assert_eq!(op_bigint_to_i64_checked(h), v, "in-range {v} narrows back");
        op_drop(h);
    }
    // i64::MAX + 1 is a valid BigInt but does NOT fit i64 → to-i64-checked traps.
    let over = {
        let (m, one) = (op_bigint_of_i64(i64::MAX), op_bigint_of_i64(1));
        let r = op_bigint_add(m, one);
        op_drop(m);
        op_drop(one);
        r
    };
    let result = std::panic::catch_unwind(|| op_bigint_to_i64_checked(over));
    assert!(
        result.is_err(),
        "i64::MAX + 1 must TRAP the checked narrow, not wrap"
    );
    op_drop(over);
    // i64::MIN - 1 (the other side).
    let under = {
        let (m, one) = (op_bigint_of_i64(i64::MIN), op_bigint_of_i64(1));
        let r = op_bigint_sub(m, one);
        op_drop(m);
        op_drop(one);
        r
    };
    let result = std::panic::catch_unwind(|| op_bigint_to_i64_checked(under));
    assert!(result.is_err(), "i64::MIN - 1 must TRAP the checked narrow");
    op_drop(under);
}

#[test]
fn bigint_ops_on_large_multi_limb_values_through_the_heap() {
    reset();
    let before = live_object_count();
    // Big multi-limb operands: p ≈ 2^126 (i64::MAX squared) and q ≈ 2^190 — both well beyond i64,
    // spanning several base-2³² limbs so the box/unbox exercises multi-limb magnitude bytes.
    let max = bigint::Big::from_i64(i64::MAX);
    let p = max.mul(&max); // ~2^126, positive
    let neg_p = bigint::Big::zero().sub(&p); // -p, exercises the sign byte on a multi-limb magnitude
    let q = p.mul(&max); // ~2^189
    // Round-trip each through the heap leaf: unbox(box(x)) == x by value (cmp == Equal).
    for b in [&p, &neg_p, &q, &bigint::Big::from_i64(0)] {
        let h = box_bigint(b);
        assert!(!is_immediate(h), "a large BigInt is a heap leaf");
        assert_eq!(
            unbox_bigint(h).cmp(b),
            core::cmp::Ordering::Equal,
            "box/unbox round-trip"
        );
        op_drop(h);
    }
    // Each WIT op on boxed large operands must unbox to the library's direct result (canonical bytes).
    let check = |op: fn(Handle, Handle) -> Handle,
                 a: &bigint::Big,
                 b: &bigint::Big,
                 want: &bigint::Big,
                 name: &str| {
        let (ha, hb) = (box_bigint(a), box_bigint(b));
        let hr = op(ha, hb);
        assert_eq!(
            unbox_bigint(hr).cmp(want),
            core::cmp::Ordering::Equal,
            "large bigint {name}"
        );
        // canonical: the op result's leaf bytes equal a freshly-boxed `want`'s (champ_eq / same key).
        let hw = box_bigint(want);
        assert!(
            champ_eq(hr, hw),
            "large bigint {name} leaf is canonical (champ_eq to a fresh box)"
        );
        op_drop(ha);
        op_drop(hb);
        op_drop(hr);
        op_drop(hw);
    };
    check(op_bigint_add, &p, &q, &p.add(&q), "add");
    check(op_bigint_sub, &p, &q, &p.sub(&q), "sub (goes negative)");
    check(op_bigint_mul, &p, &q, &p.mul(&q), "mul (~2^315)");
    let (dq, dr) = q.divmod(&p).unwrap();
    check(op_bigint_div, &q, &p, &dq, "div (multi-limb quotient)");
    // rem at large multi-limb (op 73, added after this test's other arms) — the remainder half of the
    // same divmod, its magnitude spanning limbs; and the identity `q == (q/p)*p + (q%p)` over the ops.
    check(op_bigint_rem, &q, &p, &dr, "rem (multi-limb remainder)");
    {
        let (hq2, hp2) = (box_bigint(&q), box_bigint(&p));
        let hquot = op_bigint_div(hq2, hp2);
        let hrem = op_bigint_rem(hq2, hp2);
        let qp = op_bigint_mul(hquot, hp2);
        let recon = op_bigint_add(qp, hrem);
        assert!(champ_eq(recon, hq2), "q == (q/p)*p + (q%p) at multi-limb");
        for h in [hq2, hp2, hquot, hrem, qp, recon] {
            op_drop(h);
        }
    }
    // cmp on large operands: p < q, q > p, p == p.
    let (hp, hq) = (box_bigint(&p), box_bigint(&q));
    assert_eq!(op_bigint_cmp(hp, hq), -1, "p < q");
    assert_eq!(op_bigint_cmp(hq, hp), 1, "q > p");
    let hp2 = box_bigint(&p);
    assert_eq!(op_bigint_cmp(hp, hp2), 0, "p == p");
    // A large value does NOT narrow to i64 (traps) — the checked narrow's out-of-range path on a
    // genuinely-multi-limb value (the i64-boundary test only reached exactly ±1 past the edge).
    let hp3 = box_bigint(&p);
    assert!(
        std::panic::catch_unwind(|| op_bigint_to_i64_checked(hp3)).is_err(),
        "a ~2^126 BigInt traps the i64 checked narrow"
    );
    op_drop(hp);
    op_drop(hq);
    op_drop(hp2);
    op_drop(hp3);
    assert_eq!(
        live_object_count(),
        before,
        "no leak across the large-value ops"
    );
}

#[test]
fn bigint_i128_fast_path_matches_the_big_slow_path_at_the_boundary() {
    reset();
    let before = live_object_count();
    // The oracle: the op result's leaf must be canonical == a fresh box of the `Big`-computed answer.
    let check = |op: fn(Handle, Handle) -> Handle,
                 big: fn(&bigint::Big, &bigint::Big) -> bigint::Big,
                 a: &bigint::Big,
                 b: &bigint::Big,
                 name: &str| {
        let (ha, hb) = (box_bigint(a), box_bigint(b));
        let hr = op(ha, hb);
        let want = big(a, b);
        let hw = box_bigint(&want);
        assert!(
            champ_eq(hr, hw),
            "{name}: fast/slow path leaf must be byte-identical (canonical champ_eq to the Big result)"
        );
        assert_eq!(
            unbox_bigint(hr).cmp(&want),
            core::cmp::Ordering::Equal,
            "{name}: value equal"
        );
        for h in [ha, hb, hr, hw] {
            op_drop(h);
        }
    };
    let big = |v: i128| {
        bigint::Big::from_sign_magnitude_bytes(&{
            let mut buf = [0u8; 17];
            let n = bigint::Big::i128_to_sign_magnitude_bytes_into(v, &mut buf).unwrap();
            buf[..n].to_vec()
        })
    };
    // (a) IN-RANGE operands + in-range results — the fast path.
    for &(x, y) in &[
        (0i128, 0i128),
        (12345, 67890),
        (-12345, 67890),
        (i64::MAX as i128, i64::MAX as i128), // sum/product still < i128::MAX
        (-(i64::MAX as i128), i64::MAX as i128),
        (i128::MAX, 0), // add 0 — identity, top of range
        (i128::MIN, 0),
        (i128::MAX, -1), // sub path near the top
    ] {
        let (bx, by) = (big(x), big(y));
        check(op_bigint_add, bigint::Big::add, &bx, &by, "add in-range");
        check(op_bigint_sub, bigint::Big::sub, &bx, &by, "sub in-range");
    }
    // (b) result OVERFLOWS i128 → the checked op returns None → the `Big` slow path runs. Must be
    // canonical. `i128::MAX + 1`, `i128::MIN - 1`, and `i128::MAX * i128::MAX` (~2^254).
    {
        let (one, negone) = (big(1), big(-1));
        let (bmax, bmin) = (big(i128::MAX), big(i128::MIN));
        let add = bigint::Big::add;
        let (sub, mul) = (bigint::Big::sub, bigint::Big::mul);
        check(op_bigint_add, add, &bmax, &one, "add ovf +1");
        check(op_bigint_sub, sub, &bmin, &one, "sub ovf -1");
        check(op_bigint_add, add, &bmin, &negone, "add ovf neg");
        check(op_bigint_mul, mul, &bmax, &bmax, "mul ovf ^2");
    }
    // (c) an OPERAND itself exceeds i128 (a multi-limb `Big` ~2^130) → fast path declined for that op.
    {
        let huge = big(i128::MAX).mul(&bigint::Big::from_i64(8)); // ~2^130, out of i128 range
        let small = big(3);
        let (add, mul) = (bigint::Big::add, bigint::Big::mul);
        check(op_bigint_add, add, &huge, &small, "add op>i128");
        check(op_bigint_mul, mul, &huge, &small, "mul op>i128");
    }
    assert_eq!(
        live_object_count(),
        before,
        "no leak across the boundary ops"
    );
}

#[test]
fn bigint_div_rem_i128_fast_path_matches_big_at_the_boundary() {
    reset();
    let before = live_object_count();
    // Oracle: box the `Big`-divmod quotient / remainder and assert the op's leaf is canonical-equal.
    let check = |a: &bigint::Big, b: &bigint::Big, name: &str| {
        let (want_q, want_r) = a.divmod(b).expect("divisor non-zero in this test");
        let (ha, hb) = (box_bigint(a), box_bigint(b));
        let hq = op_bigint_div(ha, hb);
        let hr = op_bigint_rem(ha, hb);
        let (hwq, hwr) = (box_bigint(&want_q), box_bigint(&want_r));
        assert!(
            champ_eq(hq, hwq),
            "{name}: div fast/slow leaf must be byte-identical (champ_eq to the Big quotient)"
        );
        assert!(
            champ_eq(hr, hwr),
            "{name}: rem fast/slow leaf must be byte-identical (champ_eq to the Big remainder)"
        );
        for h in [ha, hb, hq, hr, hwq, hwr] {
            op_drop(h);
        }
    };
    let big = |v: i128| {
        bigint::Big::from_sign_magnitude_bytes(&{
            let mut buf = [0u8; 17];
            let n = bigint::Big::i128_to_sign_magnitude_bytes_into(v, &mut buf).unwrap();
            buf[..n].to_vec()
        })
    };
    // (a) IN-RANGE — all four sign combos + exact + remainder-larger-than-dividend + zero dividend.
    for &(x, y) in &[
        (17i128, 5i128),
        (-17, 5),
        (17, -5),
        (-17, -5),
        (100, 10),
        (5, 7),
        (-5, 7),
        (0, 5),
        (i64::MAX as i128, 3),
        (i64::MIN as i128, 3),
        (i128::MAX, 7),
        (i128::MIN, 7),
    ] {
        check(&big(x), &big(y), "div/rem in-range");
    }
    // (b) an OPERAND exceeds i128 (~2^130) → the fast path declines → `Big` runs. Must be canonical.
    {
        let huge = big(i128::MAX).mul(&bigint::Big::from_i64(8)); // ~2^130
        check(&huge, &big(7), "div/rem op>i128");
        check(&huge, &big(-3), "div/rem op>i128 neg divisor");
    }
    // (c) the `i128::MIN / -1` OVERFLOW: `checked_div`/`checked_rem` return None → MUST fall to `Big`
    //     (quotient 2^127, remainder 0). A dropped `checked_*` guard would PANIC on native overflow.
    check(
        &big(i128::MIN),
        &big(-1),
        "div/rem i128::MIN / -1 overflow → Big",
    );
    assert_eq!(
        live_object_count(),
        before,
        "no leak across the div/rem boundary ops"
    );
}

#[test]
fn bigint_escape_reuses_kind_int_leaf_arbitrary_width() {
    reset();
    // A large multi-limb BigInt: i64::MAX² ≈ 2^126 (well beyond any fixed-width int).
    let max = bigint::Big::from_i64(i64::MAX);
    let big = max.mul(&max);
    // Extract (sign, BIG-ENDIAN magnitude) the way a `Shape::BigInt` encode arm would — from the
    // canonical sign-magnitude bytes `[sign][LE mag…]`: drop the sign, reverse to BE, strip leading 0s.
    let sm = big.to_sign_magnitude_bytes();
    let neg = sm[0] != 0;
    let mut be_mag: Vec<u8> = sm[1..].iter().rev().copied().collect();
    while be_mag.first() == Some(&0) {
        be_mag.remove(0);
    }
    // Build a single-int doc via the existing DocLeaf::Int (exactly what int_leaf produces, but with a
    // >i64 magnitude — proving the leaf is not i64-bounded).
    let mut b = DocBuilder::default();
    let leaf = {
        b.leaves.push(DocLeaf::Int(neg, be_mag.clone()));
        (b.leaves.len() - 1) as u32
    };
    let root = b.atom(leaf);
    let doc = b.finish(root);
    // Decode the leaf back: header(8) · leaf_count(1) · [KIND · LEB(len) · mag]. KIND 0 = pos, 3 = neg.
    assert_eq!(doc[8], 1, "one leaf");
    let kind = doc[9];
    assert_eq!(
        kind,
        if neg { 3 } else { 0 },
        "KIND_INT sign matches (0 pos / 3 neg)"
    );
    let len = doc[10] as usize;
    let decoded_mag = &doc[11..11 + len];
    assert_eq!(
        decoded_mag,
        &be_mag[..],
        "the >i64 magnitude round-trips through KIND_INT verbatim"
    );
    // Reconstruct the value from the decoded (sign, BE magnitude) and confirm it equals `big`.
    let mut recon = bigint::Big::zero();
    let base = bigint::Big::from_i64(256);
    for &byte in decoded_mag {
        recon = recon.mul(&base).add(&bigint::Big::from_i64(byte as i64));
    }
    if neg {
        recon = bigint::Big::zero().sub(&recon);
    }
    assert_eq!(
        recon.cmp(&big),
        core::cmp::Ordering::Equal,
        "KIND_INT leaf reconstructs the exact BigInt"
    );
}

#[test]
fn value_encode_bigint_leaf_via_shape_tag_17() {
    reset();
    let before = live_object_count();
    let desc: &[u8] = &[0x01, 0x11, 0x00]; // table_len=1, [0]=BigInt(tag 17=0x11), root=0
    let check = |big: &bigint::Big, note: &str| {
        let h = box_bigint(big);
        let doc = op_value_encode_form(h, desc).unwrap_or_else(|| panic!("encode {note}"));
        // Differential vs the recursive oracle (its S::BigInt arm).
        let descriptor = decode_descriptor(desc).expect("descriptor");
        let mut b = DocBuilder::default();
        let root =
            encode_value_recursive(&descriptor, &mut b, h, descriptor.root, 0).expect("recursive");
        assert_eq!(
            doc,
            b.finish(root),
            "iterative and recursive BigInt encode agree ({note})"
        );
        op_drop(h);
        doc
    };
    check(&bigint::Big::from_i64(42), "small positive");
    check(&bigint::Big::from_i64(-7), "negative");
    check(&bigint::Big::zero(), "zero");
    check(&bigint::Big::from_i64(i64::MAX), "i64::MAX boundary");
    // A >i64 value: i64::MAX² ≈ 2^126. Assert the exact KIND_INT bytes (positive, big-endian magnitude).
    let max = bigint::Big::from_i64(i64::MAX);
    let big = max.mul(&max);
    let doc = check(&big, "i64::MAX² (>i64, multi-limb)");
    // doc: header(8) · leaf_count(1) · KIND_INT(0=pos) · LEB(len) · BE-magnitude · struct(1) · atom · root.
    assert_eq!(doc[8], 1, "one leaf");
    assert_eq!(doc[9], 0, "KIND_INT positive (i64::MAX² > 0)");
    let len = doc[10] as usize;
    let be_mag = &doc[11..11 + len];
    // Reconstruct from the emitted BE magnitude and confirm it equals `big` — the leaf carried the full
    // >i64 value, not a truncated i64.
    let mut recon = bigint::Big::zero();
    let base = bigint::Big::from_i64(256);
    for &byte in be_mag {
        recon = recon.mul(&base).add(&bigint::Big::from_i64(byte as i64));
    }
    assert_eq!(
        recon.cmp(&big),
        core::cmp::Ordering::Equal,
        "the escaped KIND_INT leaf is the exact 2^126 value"
    );
    assert!(
        len > 8,
        "the magnitude exceeds 8 bytes — a genuinely >i64 BigInt crossed the boundary"
    );
    assert_eq!(live_object_count(), before, "no leak");
}

#[test]
fn inline_int_negative_behavioral_roundtrip() {
    reset();
    // NOTE (P1a/P1b gotcha 1): on a 64-bit native host `imm_int(v<0)` sign-extends into the
    // pointer's high 32 bits, so a RAW-BIT u32 round-trip would differ on native. We therefore
    // assert BEHAVIORAL identity — op_get_int decodes the right value and imm_kind == Int — which
    // is what the wasm32 ABI (32-bit usize) preserves bit-for-bit anyway.
    for v in [
        -1i64,
        -2,
        -42,
        -1000,
        FIXNUM_MIN,
        FIXNUM_MIN + 1,
        -(1 << 20),
    ] {
        let h = op_box_int(v);
        assert!(is_immediate(h), "in-window negative {v} must inline");
        assert!(
            matches!(imm_kind(h), ImmKind::Int),
            "negative fixnum classifies as Int"
        );
        assert_eq!(
            op_get_int(h),
            v,
            "negative fixnum decodes to the right value"
        );
        // to_u32/from_u32 BEHAVIORAL round-trip (reproduce the wasm32 projection: .0 as u32 back):
        let round = Handle((h.0 as usize as u32) as usize as *mut Node);
        assert!(is_immediate(round));
        assert_eq!(
            imm_as_int(round),
            v,
            "negative fixnum survives the u32 ABI projection by value"
        );
    }
}

#[test]
fn inline_int_as_map_set_key() {
    reset();
    // Small ints as CHAMP map KEYS: normalize means the key is ALWAYS inline (no boxed twin can
    // exist), and champ_hash/eq fold the SAME `(v as u64).to_le_bytes()` a boxed int would carry.
    assert!(
        is_immediate(op_box_int(7)),
        "an in-window key can never arrive boxed"
    );
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
    assert_eq!(
        champ_hash(inline),
        champ_hash(boxed),
        "inline and boxed int hash equal"
    );
    assert!(
        champ_eq(inline, boxed),
        "inline and boxed int compare equal"
    );
    assert_eq!(render(inline, &Shape::Int), render(boxed, &Shape::Int));
    assert_eq!(render(inline, &Shape::Int), "3");
    op_drop(boxed);
}

#[test]
fn compound_key_with_an_immediate_child_equals_its_boxed_child_twin() {
    reset();
    let before = live_nodes();
    // t_inline = (imm 3, imm 7); t_boxed = (BOXED 3, imm 7) — same value, mixed child reps.
    let t_inline = op_arr_alloc(2);
    op_arr_set(t_inline, 0, op_box_int(3)); // immediate child (small int normalizes to inline)
    op_arr_set(t_inline, 1, op_box_int(7));
    let t_boxed = op_arr_alloc(2);
    op_arr_set(t_boxed, 0, boxed_int_leaf(3)); // a genuinely-boxed twin of value 3
    op_arr_set(t_boxed, 1, op_box_int(7));
    // The eq/hash/cmp trinity holds across the child rep boundary.
    assert!(
        champ_eq(t_inline, t_boxed),
        "a tuple with an immediate int-child == a tuple with a boxed int-child (same value)"
    );
    assert_eq!(
        champ_hash(t_inline),
        champ_hash(t_boxed),
        "…and hashes identically (shallow-compound hash decodes both child reps the same)"
    );
    assert_eq!(
        champ_key_cmp(t_inline, t_boxed),
        core::cmp::Ordering::Equal,
        "…and orders Equal (cmp consistent with eq across the rep boundary)"
    );
    // As MAP KEYS: insert keyed by the boxed-child tuple, look up with the immediate-child tuple → HIT.
    let mut m = op_map_empty();
    op_dup(t_boxed);
    m = op_map_insert(m, t_boxed, op_box_int(100));
    let v = op_map_lookup(m, t_inline);
    assert_ne!(
        v,
        Handle::NULL,
        "the immediate-child key finds the boxed-child entry"
    );
    assert_eq!(op_get_int(v), 100, "…and reads its value");
    // Re-insert with the immediate-child tuple: SAME key by value → overwrite, size stays 1.
    op_dup(t_inline);
    m = op_map_insert(m, t_inline, op_box_int(200));
    assert_eq!(
        op_map_size(m),
        1,
        "immediate-child and boxed-child tuples are ONE key (overwrite)"
    );
    assert_eq!(
        op_get_int(op_map_lookup(m, t_boxed)),
        200,
        "the overwrite is visible through the boxed-child key too"
    );
    op_drop(m);
    op_drop(t_inline);
    op_drop(t_boxed);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_cmp_shaped_orders_by_blessed_per_leaf_and_lexicographic_rules() {
    use super::Descriptor;
    use super::S;
    use core::cmp::Ordering;
    reset();
    // Int: NUMERIC order, incl. negatives (raw little-endian bytes would sort -1 as huge).
    let desc_int = Descriptor {
        table: vec![S::Int],
        root: 0,
    };
    assert_eq!(
        value_cmp_shaped(&desc_int, op_box_int(-5), op_box_int(3), 0),
        Some(Ordering::Less),
        "-5 < 3 by numeric value"
    );
    assert_eq!(
        value_cmp_shaped(&desc_int, op_box_int(3), op_box_int(3), 0),
        Some(Ordering::Equal)
    );
    assert_eq!(
        value_cmp_shaped(&desc_int, op_box_int(10), op_box_int(-10), 0),
        Some(Ordering::Greater),
        "10 > -10 (signed; a raw-byte compare would sort -10 as larger)"
    );
    // Tuple(Int,Int): lexicographic by field — first decides, then second.
    let desc_tup = Descriptor {
        table: vec![S::Int, S::Tuple(vec![0, 0].into())],
        root: 1,
    };
    let mk_pair = |x: i64, y: i64| {
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, op_box_int(x));
        op_arr_set(t, 1, op_box_int(y));
        t
    };
    assert_eq!(
        value_cmp_shaped(&desc_tup, mk_pair(1, 9), mk_pair(2, 0), 1),
        Some(Ordering::Less),
        "(1,9) < (2,0): first field 1<2 decides"
    );
    assert_eq!(
        value_cmp_shaped(&desc_tup, mk_pair(2, 3), mk_pair(2, 7), 1),
        Some(Ordering::Less),
        "(2,3) < (2,7): first equal, second 3<7 decides"
    );
    assert_eq!(
        value_cmp_shaped(&desc_tup, mk_pair(2, 3), mk_pair(2, 3), 1),
        Some(Ordering::Equal)
    );
    // List(Int): lexicographic; a proper prefix is LESS than its extension.
    let desc_list = Descriptor {
        table: vec![S::Int, S::List(0)],
        root: 1,
    };
    let mk_list = |xs: &[i64]| {
        let mut v = op_vec_empty();
        for &x in xs {
            v = op_vec_push(v, op_box_int(x));
        }
        v
    };
    assert_eq!(
        value_cmp_shaped(&desc_list, mk_list(&[1, 2]), mk_list(&[1, 2, 3]), 1),
        Some(Ordering::Less),
        "[1,2] < [1,2,3]: a proper prefix is less than its extension"
    );
    assert_eq!(
        value_cmp_shaped(&desc_list, mk_list(&[1, 3]), mk_list(&[1, 2, 9]), 1),
        Some(Ordering::Greater),
        "[1,3] > [1,2,9]: first differing element 3>2 decides"
    );
    assert_eq!(
        value_cmp_shaped(&desc_list, mk_list(&[]), mk_list(&[1]), 1),
        Some(Ordering::Less),
        "[] < [1]: empty is less than non-empty"
    );
    // A Float leaf orders by its CANONICAL BIT PATTERN as an UNSIGNED integer (NOT numeric order) — the
    // element-derived deterministic order to-list enumeration uses, matching the Rust `__CdzF64` wrapper.
    // (Numeric float `<` stays the IEEE partial order, declined at compile time — a different path.)
    let desc_float = Descriptor {
        table: vec![S::Float],
        root: 0,
    };
    assert_eq!(
        value_cmp_shaped(&desc_float, op_box_float(1.0), op_box_float(2.0), 0),
        Some(Ordering::Less),
        "1.0 < 2.0 by unsigned bit pattern (both positive)"
    );
    // A NEGATIVE float sorts AFTER every positive: the sign bit is the high bit, so -1.0 (0xBFF0…) as a
    // u64 exceeds 2.5 (0x4004…). This is the ruling's blessed order ({-1.0,0.5,2.5} → [0.5,2.5,-1.0]).
    assert_eq!(
        value_cmp_shaped(&desc_float, op_box_float(-1.0), op_box_float(2.5), 0),
        Some(Ordering::Greater),
        "-1.0 sorts AFTER 2.5 (sign bit = high bit; negatives last in unsigned-bits order)"
    );
    // +0.0 and -0.0 are DISTINCT and ordered (+0.0 = 0x0 before -0.0 = 0x8000…), consistent with their
    // distinct canonical byte forms (op_box_float keeps their bits).
    assert_eq!(
        value_cmp_shaped(&desc_float, op_box_float(0.0), op_box_float(-0.0), 0),
        Some(Ordering::Less),
        "+0.0 < -0.0 by bit pattern (distinct canonical forms)"
    );
    // Every NaN collapses to one canonical quiet NaN on construction → two NaNs compare Equal (a total
    // order has no unordered pair; the to-list sort treats them as one position).
    assert_eq!(
        value_cmp_shaped(
            &desc_float,
            op_box_float(f64::NAN),
            op_box_float(f64::NAN),
            0
        ),
        Some(Ordering::Equal),
        "canonical NaN == canonical NaN (NaN collapsed on box; a total order)"
    );
    // A Bytes leaf has a BLESSED TOTAL order (§order): content-lexicographic over its UNSIGNED byte
    // values — the same `raw`-slice compare as Str, unlike Float (bare-bits-only). Build Bytes values and
    // pin the ordering rules, INCLUDING a byte >= 128 (the case a SIGNED-byte compare gets wrong).
    let desc_bytes = Descriptor {
        table: vec![S::Bytes],
        root: 0,
    };
    let mk_bytes = |bs: &[u8]| {
        let b = op_bytes_alloc(bs.len() as u32);
        let mut h = b;
        for (i, &x) in bs.iter().enumerate() {
            h = op_bytes_set(h, i as u32, x as u32);
        }
        h
    };
    assert_eq!(
        value_cmp_shaped(&desc_bytes, mk_bytes(&[1, 2]), mk_bytes(&[1, 3]), 0),
        Some(Ordering::Less),
        "[1,2] < [1,3]: first differing byte 2<3 decides (lexicographic)"
    );
    assert_eq!(
        value_cmp_shaped(&desc_bytes, mk_bytes(&[1, 2]), mk_bytes(&[1, 2, 0]), 0),
        Some(Ordering::Less),
        "[1,2] < [1,2,0]: a proper prefix is less than its extension"
    );
    assert_eq!(
        value_cmp_shaped(&desc_bytes, mk_bytes(&[0x80]), mk_bytes(&[0x7f]), 0),
        Some(Ordering::Greater),
        "[0x80] > [0x7f]: UNSIGNED byte order (128 > 127; a signed-byte compare would sort 0x80 as -128 < 127)"
    );
    assert_eq!(
        value_cmp_shaped(&desc_bytes, mk_bytes(&[9, 9]), mk_bytes(&[9, 9]), 0),
        Some(Ordering::Equal),
        "equal byte sequences compare Equal"
    );
    assert_eq!(
        value_cmp_shaped(&desc_bytes, mk_bytes(&[]), mk_bytes(&[0]), 0),
        Some(Ordering::Less),
        "empty Bytes < non-empty (empty is a proper prefix)"
    );
    // A Bytes leaf INSIDE a compound composes soundly (unlike a float): Tuple(Bytes,Int) is lexicographic.
    let desc_tb = Descriptor {
        table: vec![S::Bytes, S::Int, S::Tuple(vec![0, 1].into())],
        root: 2,
    };
    let mk_tb = |bs: &[u8], y: i64| {
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, mk_bytes(bs));
        op_arr_set(t, 1, op_box_int(y));
        t
    };
    assert_eq!(
        value_cmp_shaped(&desc_tb, mk_tb(&[1], 9), mk_tb(&[2], 0), 2),
        Some(Ordering::Less),
        "(b\"\\x01\",9) < (b\"\\x02\",0): the Bytes field decides first (composes inside a compound)"
    );
    // Consistency with equality: cmp == Equal exactly when champ_eq.
    let p = mk_pair(5, 6);
    let q = mk_pair(5, 6);
    assert_eq!(value_cmp_shaped(&desc_tup, p, q, 1), Some(Ordering::Equal));
    assert!(champ_eq(p, q), "cmp Equal agrees with champ_eq");
    let bx = mk_bytes(&[7, 8]);
    let by = mk_bytes(&[7, 8]);
    assert_eq!(
        value_cmp_shaped(&desc_bytes, bx, by, 0),
        Some(Ordering::Equal)
    );
    assert!(champ_eq(bx, by), "Bytes cmp Equal agrees with champ_eq");
}

#[test]
fn value_cmp_shaped_sum_record_and_deep_nesting() {
    use super::Descriptor;
    use super::S;
    use core::cmp::Ordering;
    reset();
    // Sum with two variants: 0 = A(Int), 1 = B(Int). Discriminant decides first; same variant → payload.
    let desc_sum = Descriptor {
        table: vec![
            S::Int,
            S::Sum(vec![("A".into(), 0), ("B".into(), 0)].into()),
        ],
        root: 1,
    };
    let a5 = op_sum_new(0, op_box_int(5));
    let a9 = op_sum_new(0, op_box_int(9));
    let b0 = op_sum_new(1, op_box_int(0));
    assert_eq!(
        value_cmp_shaped(&desc_sum, a5, b0, 1),
        Some(Ordering::Less),
        "A(5) < B(0): lower discriminant 0<1 decides, payload ignored"
    );
    assert_eq!(
        value_cmp_shaped(&desc_sum, a5, a9, 1),
        Some(Ordering::Less),
        "A(5) < A(9): same discriminant → payload 5<9 decides"
    );
    assert_eq!(
        value_cmp_shaped(&desc_sum, a5, a5, 1),
        Some(Ordering::Equal)
    );
    // Record {x:Int, y:Int} — a tuple arr in field order; compare by field.
    let desc_rec = Descriptor {
        table: vec![
            S::Int,
            S::Record(vec![("x".into(), 0), ("y".into(), 0)].into()),
        ],
        root: 1,
    };
    let rec = |x: i64, y: i64| {
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, op_box_int(x));
        op_arr_set(t, 1, op_box_int(y));
        t
    };
    assert_eq!(
        value_cmp_shaped(&desc_rec, rec(1, 2), rec(1, 5), 1),
        Some(Ordering::Less),
        "record (x:1,y:2) < (x:1,y:5): x equal, y 2<5 decides"
    );
    // DEEP nesting: a list of lists … 200 deep (well past any shallow native-recursion limit). The
    // iterative CmpTask stack must handle it without overflowing. Build `[[…[[1]]…]]` vs `[[…[[2]]…]]`.
    let mut table = vec![S::Int]; // 0 = Int
    let mut cur = 0u32;
    for _ in 0..200 {
        table.push(S::List(cur));
        cur = (table.len() - 1) as u32;
    }
    let desc_deep = Descriptor { table, root: cur };
    let build_deep = |leaf: i64| {
        let mut v = op_box_int(leaf);
        for _ in 0..200 {
            let outer = op_vec_empty();
            v = op_vec_push(outer, v);
        }
        v
    };
    assert_eq!(
        value_cmp_shaped(&desc_deep, build_deep(1), build_deep(2), cur),
        Some(Ordering::Less),
        "a 200-deep nested list compares by its innermost leaf without a stack overflow (iterative walk)"
    );
}

#[test]
fn value_eq_shaped_handles_float_leaves_and_list_spine() {
    use super::Descriptor;
    use super::S;
    reset();
    // desc: [0]=Float, [1]=List(0).
    let desc = Descriptor {
        table: vec![S::Float, S::List(0)],
        root: 1,
    };
    let push_flist = |xs: &[f64]| {
        let mut v = op_vec_empty();
        for &x in xs {
            v = op_vec_push(v, op_box_float(x));
        }
        v
    };
    let concat_flist = |lo: &[f64], hi: &[f64]| op_vec_concat(push_flist(lo), push_flist(hi));
    // (1) concat-built vs push-built List<Float> with the same elements → EQUAL (element-wise, spine-indep).
    // Use n=40 elements so the concat leaves a RELAXED (non-shape-canonical) node — champ_eq would MISS.
    let xs: Vec<f64> = (0..40).map(|i| i as f64 * 0.5).collect();
    let a = concat_flist(&xs[..20], &xs[20..]);
    let b = push_flist(&xs);
    assert_eq!(
        value_eq_shaped(&desc, a, b, 1),
        Some(true),
        "concat-vs-push List<Float> equal (element-wise)"
    );
    op_drop(a);
    op_drop(b);
    // (2) a differing float element → NOT equal.
    let c = push_flist(&[1.0, 2.0, 3.0]);
    let d = push_flist(&[1.0, 2.5, 3.0]);
    assert_eq!(
        value_eq_shaped(&desc, c, d, 1),
        Some(false),
        "a differing float element → not equal"
    );
    // (3) different length → not equal.
    let e = push_flist(&[1.0, 2.0]);
    assert_eq!(
        value_eq_shaped(&desc, c, e, 1),
        Some(false),
        "different length → not equal"
    );
    op_drop(c);
    op_drop(d);
    op_drop(e);
    // (4) NaN == NaN (canonical byte form), and -0.0 ≠ +0.0, at the LEAF (bare Float shape).
    let desc_f = Descriptor {
        table: vec![S::Float],
        root: 0,
    };
    assert_eq!(
        value_eq_shaped(&desc_f, op_box_float(f64::NAN), op_box_float(f64::NAN), 0),
        Some(true),
        "NaN == NaN by canonical byte form (§313)"
    );
    assert_eq!(
        value_eq_shaped(&desc_f, op_box_float(-0.0), op_box_float(0.0), 0),
        Some(false),
        "-0.0 ≠ +0.0 (canonical byte form distinguishes sign)"
    );
    // (5) deep nesting: a 200-deep list-of-...-of-List<Float> is stack-safe (iterative EqTask walk).
    let mut table = vec![S::Float];
    let mut cur = 0u32;
    for _ in 0..200 {
        table.push(S::List(cur));
        cur = (table.len() - 1) as u32;
    }
    let desc_deep = Descriptor { table, root: cur };
    let build_deep = |leaf: f64| {
        let mut v = op_box_float(leaf);
        for _ in 0..200 {
            let outer = op_vec_empty();
            v = op_vec_push(outer, v);
        }
        v
    };
    let da = build_deep(3.5);
    let db = build_deep(3.5);
    assert_eq!(
        value_eq_shaped(&desc_deep, da, db, cur),
        Some(true),
        "200-deep list<float> equal, no overflow"
    );
    op_drop(da);
    op_drop(db);
}

#[test]
fn value_cmp_shaped_flattens_a_bytes_slice_view_list_element() {
    use super::Descriptor;
    use super::S;
    use core::cmp::Ordering;
    reset();
    // desc: [0]=Bytes, [1]=List(0).
    let desc = Descriptor {
        table: vec![S::Bytes, S::List(0)],
        root: 1,
    };
    let mk_bytes = |bs: &[u8]| {
        let b = op_bytes_alloc(bs.len() as u32);
        let mut h = b;
        for (i, &x) in bs.iter().enumerate() {
            h = op_bytes_set(h, i as u32, x as u32);
        }
        h
    };
    // A slice VIEW of [9,20,30,8] at offset 1 len 2 → window [20,30] (arity>0, NOT a flat leaf).
    let parent = mk_bytes(&[9, 20, 30, 8]);
    let view = op_bytes_slice(parent, 1, 2);
    assert!(
        with_node(view, 0usize, |n| n.handles.len()) > 0,
        "precondition: the slice is a VIEW node (arity>0), not already flat"
    );
    let list_view = op_vec_push(op_vec_empty(), view);
    let list_flat = op_vec_push(op_vec_empty(), mk_bytes(&[20, 30]));
    assert_eq!(
        value_cmp_shaped(&desc, list_view, list_flat, 1),
        Some(Ordering::Equal),
        "[<slice-view 20,30>] == [<flat 20,30>]: the Bytes list element flattens the view before comparing"
    );
}

#[test]
fn value_canonicalize_makes_concat_and_push_list_keys_collide() {
    use super::Descriptor;
    use super::S;
    reset();
    // desc: [0]=Int, [1]=List(0). A concat-built and a push-built [0..n) canonicalize byte-identical.
    let desc_list = Descriptor {
        table: vec![S::Int, S::List(0)],
        root: 1,
    };
    let push_list = |n: i64| {
        let mut v = op_vec_empty();
        for i in 0..n {
            v = op_vec_push(v, op_box_int(i));
        }
        v
    };
    let concat_list = |n: i64| {
        let split = (n / 2).max(1);
        let lo = {
            let mut v = op_vec_empty();
            for i in 0..split {
                v = op_vec_push(v, op_box_int(i));
            }
            v
        };
        let hi = {
            let mut v = op_vec_empty();
            for i in split..n {
                v = op_vec_push(v, op_box_int(i));
            }
            v
        };
        op_vec_concat(lo, hi)
    };
    for &n in &[3i64, 32, 33, 40, 100] {
        // Canonicalize the concat-built key BEFORE it goes in as a map key.
        let raw_key = concat_list(n);
        let key = value_canonicalize_shaped(&desc_list, raw_key, 1).expect("canon key");
        op_drop(raw_key);
        let m = op_map_insert(op_map_empty(), key, op_box_int(999));
        // Query with a canonicalized PUSH-built equal key — must HIT.
        let raw_q = push_list(n);
        let q = value_canonicalize_shaped(&desc_list, raw_q, 1).expect("canon query");
        op_drop(raw_q);
        let hit = op_map_lookup(m, q);
        assert_ne!(
            hit,
            Handle::NULL,
            "n={n}: canonicalized concat-key must be found by push-key"
        );
        assert_eq!(op_get_int(hit), 999, "n={n}: and yield the stored value");
        op_drop(q);
        // A genuinely different list ([0..n) with the last element bumped) must still MISS.
        let raw_diff = {
            let mut v = op_vec_empty();
            for i in 0..n {
                v = op_vec_push(v, op_box_int(if i == n - 1 { i + 1000 } else { i }));
            }
            v
        };
        let diff = value_canonicalize_shaped(&desc_list, raw_diff, 1).expect("canon diff");
        op_drop(raw_diff);
        assert_eq!(
            op_map_lookup(m, diff),
            Handle::NULL,
            "n={n}: a different list must still miss"
        );
        op_drop(diff);
        op_drop(m);
    }
    // Nested key: (tuple (list Int) Int) — a list buried in a compound key must ALSO canonicalize.
    let desc_nested = Descriptor {
        table: vec![S::Int, S::List(0), S::Tuple(vec![1, 0].into())],
        root: 2,
    };
    let mk_pair = |list_h: Handle, tag: i64| {
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, list_h);
        op_arr_set(t, 1, op_box_int(tag));
        t
    };
    let raw_k = mk_pair(concat_list(40), 7);
    let k = value_canonicalize_shaped(&desc_nested, raw_k, 2).expect("canon nested key");
    op_drop(raw_k);
    let m = op_map_insert(op_map_empty(), k, op_box_int(555));
    let raw_qn = mk_pair(push_list(40), 7);
    let qn = value_canonicalize_shaped(&desc_nested, raw_qn, 2).expect("canon nested query");
    op_drop(raw_qn);
    let hit = op_map_lookup(m, qn);
    assert_ne!(
        hit,
        Handle::NULL,
        "nested (concat-list, tag) key found by (push-list, tag)"
    );
    assert_eq!(op_get_int(hit), 555);
    op_drop(qn);
    op_drop(m);
    #[cfg(any(test, feature = "debug-counters"))]
    assert_eq!(
        live_object_count(),
        0,
        "canonicalize is borrow-and-copy: no leaked cells"
    );
}

#[test]
fn value_canonicalize_deep_nested_list_is_stack_safe() {
    use super::Descriptor;
    use super::S;
    reset();
    let mut table = vec![S::Int];
    let mut cur = 0u32;
    for _ in 0..200 {
        table.push(S::List(cur));
        cur = (table.len() - 1) as u32;
    }
    let desc_deep = Descriptor { table, root: cur };
    let mut v = op_box_int(42);
    for _ in 0..200 {
        let outer = op_vec_empty();
        v = op_vec_push(outer, v);
    }
    let canon = value_canonicalize_shaped(&desc_deep, v, cur).expect("deep canon");
    op_drop(v);
    // Peel 200 levels of single-element lists back down to the leaf.
    let mut cursor = canon;
    for _ in 0..200 {
        assert_eq!(op_vec_len(cursor), 1);
        cursor = op_vec_get(cursor, 0);
    }
    assert_eq!(
        op_get_int(cursor),
        42,
        "deep canonicalized list preserves its innermost leaf"
    );
    op_drop(canon);
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
    assert_eq!(
        live_nodes(),
        before + 1,
        "tuple of small ints = just the array node"
    );
    assert_eq!(op_get_int(op_arr_get(t, 0)), 10);
    assert_eq!(op_get_int(op_arr_get(t, 1)), -20);
    assert_eq!(op_get_int(op_arr_get(t, 2)), 30);
    assert_eq!(
        render(t, &Shape::List(Box::new(Shape::Int))),
        "(list 10 -20 30)"
    );
    op_drop(t);
    assert_eq!(
        live_nodes(),
        before,
        "array reclaimed; inline ints leave nothing"
    );
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
        assert_eq!(
            op_get_int(op_arr_get(a, i as u32)),
            x,
            "element {i} = {x} reads back exactly"
        );
    }
    // Exactly TWO elements (big1, big2) are boxed nodes; the array shell is the third node. The
    // four in-window ints ride inline — the P2 win, mid-container, alongside boxed neighbors.
    assert_eq!(
        live_nodes(),
        before + 3,
        "array shell + the 2 out-of-window boxed ints only"
    );
    // Render walks every element under Shape::Int, mixing inline and boxed transparently.
    assert_eq!(
        render(a, &Shape::List(Box::new(Shape::Int))),
        format!("(list 0 {big1} -7 {big2} {FIXNUM_MAX} 42)")
    );
    op_drop(a);
    assert_eq!(
        live_nodes(),
        before,
        "whole mixed container reclaimed; inline elems leak nothing"
    );
}

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
    assert!(
        is_immediate(u),
        "reuse-to-0 must return an inline unit, never a boxed empty node"
    );
    assert!(matches!(imm_kind(u), ImmKind::Unit));
    // Byte-identical (structurally) to the normal unit producer.
    assert!(
        champ_eq(u, op_arr_alloc(0)),
        "reuse-to-0 unit == op_arr_alloc(0) unit"
    );
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

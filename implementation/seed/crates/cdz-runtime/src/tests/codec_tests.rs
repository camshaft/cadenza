use super::*;

#[test]
fn set_to_list_yields_canonical_order() {
    reset();
    let desc = set_int_descriptor();
    // Insert 10, 2, 1, 3 (hash order ≠ value order); canonical order is 1, 2, 3, 10.
    let mut s = op_set_empty();
    for &e in &[10i64, 2, 1, 3] {
        s = op_set_insert(s, op_box_int(e));
    }
    let list = op_set_to_list(s, &desc);
    let got: Vec<i64> = (0..op_vec_len(list))
        .map(|i| op_get_int(op_vec_get(list, i)))
        .collect();
    assert_eq!(
        got,
        vec![1, 2, 3, 10],
        "set-to-list must yield elements in canonical value order (sorted), not hash/insertion order"
    );
    op_drop(s);
    op_drop(list);
    #[cfg(any(test, feature = "debug-counters"))]
    assert_eq!(
        live_object_count(),
        0,
        "set-to-list leak: the set + the result list (each element dup'd in) must net to 0 live cells"
    );
}

#[test]
fn set_to_list_orders_compound_tuple_elements_lexicographically() {
    reset();
    let desc = set_tuple_int_int_descriptor();
    let mk = |a: i64, b: i64| {
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, op_box_int(a));
        op_arr_set(t, 1, op_box_int(b));
        t
    };
    let mut s = op_set_empty();
    for &(a, b) in &[(3i64, 1i64), (1, 2), (2, 0)] {
        s = op_set_insert(s, mk(a, b));
    }
    let list = op_set_to_list(s, &desc);
    let got: Vec<(i64, i64)> = (0..op_vec_len(list))
        .map(|i| {
            let t = op_vec_get(list, i);
            (op_get_int(op_arr_get(t, 0)), op_get_int(op_arr_get(t, 1)))
        })
        .collect();
    assert_eq!(
        got,
        vec![(1, 2), (2, 0), (3, 1)],
        "set-to-list over compound elements must yield lexicographic order, not decline (breaker 10761)"
    );
    op_drop(s);
    op_drop(list);
    #[cfg(any(test, feature = "debug-counters"))]
    assert_eq!(
        live_object_count(),
        0,
        "compound set-to-list leak: the set + the result list of tuples (each dup'd in) must net to 0"
    );
}

#[test]
fn map_to_list_yields_canonical_key_order() {
    reset();
    let desc = map_int_int_descriptor();
    // Insert (30→300),(10→100),(20→200); canonical KEY order is 10,20,30.
    let mut m = op_map_empty();
    for &(k, v) in &[(30i64, 300i64), (10, 100), (20, 200)] {
        m = op_map_insert(m, op_box_int(k), op_box_int(v));
    }
    let list = op_map_to_list(m, &desc);
    let got: Vec<(i64, i64)> = (0..op_vec_len(list))
        .map(|i| {
            let entry = op_vec_get(list, i);
            (
                op_get_int(op_arr_get(entry, 0)),
                op_get_int(op_arr_get(entry, 1)),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![(10, 100), (20, 200), (30, 300)],
        "map-to-list must yield entries as (key,value) tuples in canonical key order"
    );
    op_drop(m);
    op_drop(list);
    #[cfg(any(test, feature = "debug-counters"))]
    assert_eq!(
        live_object_count(),
        0,
        "map-to-list leak: the map + the result list of entry tuples (each k,v dup'd in) must net to 0"
    );
}

#[test]
fn set_to_list_declines_a_non_set_descriptor_to_empty() {
    reset();
    // A descriptor whose root is a bare Int (table [0]=Int, root=0) — not a Set.
    let desc = vec![1u8, 0u8, 0u8]; // table_len=1, [0]=Int, root=0
    let mut s = op_set_empty();
    s = op_set_insert(s, op_box_int(7));
    let list = op_set_to_list(s, &desc);
    assert_eq!(
        op_vec_len(list),
        0,
        "a non-Set root descriptor must decline to the empty list (never-trap total)"
    );
    op_drop(s);
    op_drop(list);
}

#[test]
fn value_encode_of_a_framed_int_tuple_is_the_colon_framed_golden() {
    reset();
    let desc = framed_int_pair_descriptor();
    let pair = op_arr_alloc(2);
    op_arr_set(pair, 0, op_box_int(5));
    op_arr_set(pair, 1, op_box_int(105));
    let got = op_value_encode_form(pair, &desc).expect("encode framed int pair");

    // (1) iterative production walk == recursive oracle (the byte-equality guard the sibling
    // differential relies on, here on a root-Framed Tuple rather than a Named sum).
    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, pair, descriptor.root, 0)
        .expect("recursive encode");
    let rec_doc = b.finish(root);
    assert_eq!(
        got, rec_doc,
        "iterative and recursive framed-tuple encode disagree"
    );

    // (2) decode ∘ encode == id (the doc round-trips back to the same value form).
    let back = op_value_decode(&got, &desc);
    assert_ne!(
        back,
        Handle::NULL,
        "framed-tuple doc must decode (round-trip)"
    );
    let reencoded = op_value_encode_form(back, &desc).expect("re-encode decoded framed tuple");
    assert_eq!(
        got, reencoded,
        "decode∘encode is not the identity on the framed tuple"
    );
    op_drop(back);

    // (3) exact golden bytes — the FULL colon-framed typed document (leaf pool + struct spine).
    // Leaves in canon pre-order first-encounter: ':' , 'tuple', 5, 105, 'Tuple', 'Int64'; the
    // '(: value type)' frame is the outer form (head ':'), value = (tuple 5 105), type = (Tuple Int64
    // Int64). Asserting the WHOLE document (not just a prefix) makes this a symmetric full-byte
    // wasm==rust guard against the same golden the cadenza-ast mirror asserts (reviewer refinement).
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // cdzast\x00\x01
        0x06, // 6 leaves
        0x0a, 0x01, 0x3a, // NAME ':'
        0x15, // M2 Ctor(Tuple) value head (kind 21, payloadless) — was NAME 'tuple'
        0x00, 0x01, 0x05, // INT 5
        0x00, 0x01, 0x69, // INT 105
        0x0a, 0x05, 0x54, 0x75, 0x70, 0x6c, 0x65, // NAME 'Tuple'
        0x0a, 0x05, 0x49, 0x6e, 0x74, 0x36, 0x34, // NAME 'Int64'
        // struct spine (post-order structs; TAG_ATOM=0/TAG_LIST=1 + child refs):
        0x0a, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x01, 0x03, 0x01, 0x02, 0x03, 0x00,
        0x04, 0x00, 0x05, 0x00, 0x05, 0x01, 0x03, 0x05, 0x06, 0x07, 0x01, 0x03, 0x00, 0x04, 0x08,
        0x09,
    ];
    assert_eq!(
        got, expect,
        "framed int-tuple must be the full colon-framed golden document"
    );
    assert_eq!(
        got[8], 0x06,
        "leaf count 6 (framed) not 3 (bare) — the divergence guard"
    );

    op_drop(pair);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_of_a_framed_int_record_is_the_colon_framed_golden() {
    reset();
    let desc = framed_int_record_descriptor();
    // A record VALUE is a heap array of its field values in declared order (the descriptor names them).
    let rec = op_arr_alloc(2);
    op_arr_set(rec, 0, op_box_int(5));
    op_arr_set(rec, 1, op_box_int(105));
    let got = op_value_encode_form(rec, &desc).expect("encode framed int record");

    // (1) iterative production walk == recursive oracle.
    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, rec, descriptor.root, 0)
        .expect("recursive encode");
    assert_eq!(
        got,
        b.finish(root),
        "iterative and recursive framed-record encode disagree"
    );

    // (2) decode ∘ encode == id.
    let back = op_value_decode(&got, &desc);
    assert_ne!(
        back,
        Handle::NULL,
        "framed-record doc must decode (round-trip)"
    );
    let reencoded = op_value_encode_form(back, &desc).expect("re-encode decoded framed record");
    assert_eq!(
        got, reencoded,
        "decode∘encode is not the identity on the framed record"
    );
    op_drop(back);

    // (3) exact leaf pool — the deduped leaves in canon pre-order first-encounter: ':' , 'record',
    // '=' , 'a', 5, 'b', 105, 'Int64'. 8 deduped leaves. KEY DETAIL: the frame's type node is
    // `(record (a Int64) (b Int64))` — its head atom is the LOWERCASE `record`, the SAME atom as the
    // value form's `(record …)` head, so it INTERNS ONCE (no distinct `Record` type leaf); likewise the
    // field-name atoms `a`/`b` are shared between the value's `(= a …)` and the type's `(a Int64)`, and
    // `Int64` interns once across both fields. So the pool is 8, not 9 (a bare record would omit ':' and
    // 'Int64' entirely — this leaf-count is the framed-vs-bare divergence guard).
    // M2 head-first golden: the value head is Ctor(Record)=0x16 + fields are (FieldPair name value) with
    // FieldPair head=0x19; the value+type 'record' head is NO LONGER shared (value is a ctor leaf, the
    // type node keeps NAME 'record'), so the pool is 9 leaves (was 8 deduped) and the spine refs shift.
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, 0x09, 0x0a, 0x01, 0x3a, 0x16, 0x19, 0x0a,
        0x01, 0x61, 0x00, 0x01, 0x05, 0x0a, 0x01, 0x62, 0x00, 0x01, 0x69, 0x0a, 0x06, 0x72, 0x65,
        0x63, 0x6f, 0x72, 0x64, 0x0a, 0x05, 0x49, 0x6e, 0x74, 0x36, 0x34, 0x14, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x01, 0x03, 0x02, 0x03, 0x04, 0x00, 0x02, 0x00,
        0x05, 0x00, 0x06, 0x01, 0x03, 0x06, 0x07, 0x08, 0x01, 0x03, 0x01, 0x05, 0x09, 0x00, 0x07,
        0x00, 0x03, 0x00, 0x08, 0x01, 0x02, 0x0c, 0x0d, 0x00, 0x05, 0x00, 0x08, 0x01, 0x02, 0x0f,
        0x10, 0x01, 0x03, 0x0b, 0x0e, 0x11, 0x01, 0x03, 0x00, 0x0a, 0x12, 0x13,
    ];
    assert_eq!(
        got, expect,
        "framed int-record must be the full colon-framed golden document"
    );
    assert_eq!(
        got[8], 0x09,
        "leaf count 9 (M2 framed record — value head un-shared from the type head)"
    );

    op_drop(rec);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_of_a_framed_generic_sum_is_the_colon_framed_golden() {
    reset();
    let desc = framed_option_int_descriptor();

    // (Some 5): disc 0, single Int payload.
    let some = op_sum_new(0, op_box_int(5));
    let got_some = op_value_encode_form(some, &desc).expect("encode Some 5");
    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, some, descriptor.root, 0)
        .expect("recursive encode Some");
    assert_eq!(
        got_some,
        b.finish(root),
        "iterative/recursive disagree on Some"
    );
    let back = op_value_decode(&got_some, &desc);
    assert_ne!(back, Handle::NULL, "Some doc must decode");
    assert_eq!(
        got_some,
        op_value_encode_form(back, &desc).expect("re-encode Some"),
        "decode∘encode ≠ id on Some"
    );
    op_drop(back);
    // FULL document: ':' , 'Some', 5, 'Option', 'Int64' (5 leaves) + spine — the parametric Option node.
    let expect_some: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // cdzast\x00\x01
        0x05, // 5 leaves
        0x0a, 0x01, 0x3a, // ':'
        0x0a, 0x04, 0x53, 0x6f, 0x6d, 0x65, // 'Some'
        0x00, 0x01, 0x05, // INT 5
        0x0a, 0x06, 0x4f, 0x70, 0x74, 0x69, 0x6f, 0x6e, // 'Option'
        0x0a, 0x05, 0x49, 0x6e, 0x74, 0x36, 0x34, // 'Int64'
        // struct spine:
        0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x02, 0x01, 0x02, 0x00, 0x03, 0x00, 0x04,
        0x01, 0x02, 0x04, 0x05, 0x01, 0x03, 0x00, 0x03, 0x06, 0x07,
    ];
    assert_eq!(
        got_some, expect_some,
        "Some must be the full colon-framed golden document"
    );
    assert_eq!(
        got_some[8], 0x05,
        "Some leaf count = 5 (framed generic sum)"
    );
    op_drop(some);

    // None: disc 1, nullary (unit) payload → renders (None unit).
    let none = op_sum_new(1, op_arr_alloc(0));
    let got_none = op_value_encode_form(none, &desc).expect("encode None");
    let mut b2 = DocBuilder::default();
    let root2 = encode_value_recursive(&descriptor, &mut b2, none, descriptor.root, 0)
        .expect("recursive encode None");
    assert_eq!(
        got_none,
        b2.finish(root2),
        "iterative/recursive disagree on None"
    );
    let back2 = op_value_decode(&got_none, &desc);
    assert_ne!(back2, Handle::NULL, "None doc must decode");
    assert_eq!(
        got_none,
        op_value_encode_form(back2, &desc).expect("re-encode None"),
        "decode∘encode ≠ id on None"
    );
    op_drop(back2);
    // FULL document: ':' , 'None', 'unit', 'Option', 'Int64' (5 leaves) + spine.
    let expect_none: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // cdzast\x00\x01
        0x05, // 5 leaves
        0x0a, 0x01, 0x3a, // ':'
        0x0a, 0x04, 0x4e, 0x6f, 0x6e, 0x65, // 'None'
        0x0a, 0x04, 0x75, 0x6e, 0x69, 0x74, // 'unit'
        0x0a, 0x06, 0x4f, 0x70, 0x74, 0x69, 0x6f, 0x6e, // 'Option'
        0x0a, 0x05, 0x49, 0x6e, 0x74, 0x36, 0x34, // 'Int64'
        // struct spine:
        0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x02, 0x01, 0x02, 0x00, 0x03, 0x00, 0x04,
        0x01, 0x02, 0x04, 0x05, 0x01, 0x03, 0x00, 0x03, 0x06, 0x07,
    ];
    assert_eq!(
        got_none, expect_none,
        "None must be the full colon-framed golden document"
    );
    assert_eq!(
        got_none[8], 0x05,
        "None leaf count = 5 (framed generic sum)"
    );
    op_drop(none);

    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_of_a_named_monomorphic_sum_is_the_colon_framed_golden() {
    reset();
    let desc = named_shape_descriptor();
    // (Rect 5 6): disc 1, payload a 2-element arr (the Spread splices its two Int64s flat).
    let pair = op_arr_alloc(2);
    op_arr_set(pair, 0, op_box_int(5));
    op_arr_set(pair, 1, op_box_int(6));
    let rect = op_sum_new(1, pair);
    let got = op_value_encode_form(rect, &desc).expect("encode Rect 5 6");

    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, rect, descriptor.root, 0)
        .expect("recursive encode Rect");
    assert_eq!(got, b.finish(root), "iterative/recursive disagree on Rect");

    let back = op_value_decode(&got, &desc);
    assert_ne!(back, Handle::NULL, "Rect doc must decode");
    assert_eq!(
        got,
        op_value_encode_form(back, &desc).expect("re-encode Rect"),
        "decode∘encode ≠ id on Rect"
    );
    op_drop(back);

    // FULL document: ':' , 'Rect', 5, 6, 'Shape' (5 leaves) + spine — a bare-name 'Shape' frame
    // (Named), the two payloads flat. NO parametric type node (contrast the generic Option's node).
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // cdzast\x00\x01
        0x05, // 5 leaves
        0x0a, 0x01, 0x3a, // ':'
        0x0a, 0x04, 0x52, 0x65, 0x63, 0x74, // 'Rect'
        0x00, 0x01, 0x05, // INT 5
        0x00, 0x01, 0x06, // INT 6
        0x0a, 0x05, 0x53, 0x68, 0x61, 0x70, 0x65, // 'Shape'
        // struct spine:
        0x07, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x01, 0x03, 0x01, 0x02, 0x03, 0x00,
        0x04, 0x01, 0x03, 0x00, 0x04, 0x05, 0x06,
    ];
    assert_eq!(
        got, expect,
        "Rect must be the full colon-framed golden document (Named root)"
    );
    assert_eq!(
        got[8], 0x05,
        "Rect leaf count = 5 (Named mono sum, flat payloads)"
    );

    op_drop(rect);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_of_a_framed_int_float_tuple_is_the_colon_framed_golden() {
    reset();
    let desc = framed_int_float_pair_descriptor();
    let pair = op_arr_alloc(2);
    op_arr_set(pair, 0, op_box_int(5));
    op_arr_set(pair, 1, op_box_float(2.5));
    let got = op_value_encode_form(pair, &desc).expect("encode framed int/float pair");

    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, pair, descriptor.root, 0)
        .expect("recursive encode");
    assert_eq!(
        got,
        b.finish(root),
        "iterative/recursive disagree on int/float tuple"
    );

    let back = op_value_decode(&got, &desc);
    assert_ne!(back, Handle::NULL, "int/float tuple doc must decode");
    assert_eq!(
        got,
        op_value_encode_form(back, &desc).expect("re-encode"),
        "decode∘encode ≠ id on int/float tuple"
    );
    op_drop(back);

    // FULL document: ':' , 'tuple', 5, 2.5 (KIND_FLOAT), 'Tuple', 'Int64', 'Float64' (7 leaves) + spine.
    // The FLOAT leaf is KIND_FLOAT(6) + neg(0) + exponent as a FIXED 8-byte BIG-ENDIAN i64 (-1 =
    // 0xFF×8) + siglen(1) + significand([25]) — i.e. 25×10⁻¹ = 2.5 (exact decimal, not lossy bits).
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // cdzast\x00\x01
        0x07, // 7 leaves
        0x0a, 0x01, 0x3a, // ':'
        0x15, // M2 Ctor(Tuple) value head (kind 21) — was NAME 'tuple'
        0x00, 0x01, 0x05, // INT 5
        0x06, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
        0x19, // FLOAT 2.5 (exp -1, sig 25)
        0x0a, 0x05, 0x54, 0x75, 0x70, 0x6c, 0x65, // 'Tuple'
        0x0a, 0x05, 0x49, 0x6e, 0x74, 0x36, 0x34, // 'Int64'
        0x0a, 0x07, 0x46, 0x6c, 0x6f, 0x61, 0x74, 0x36, 0x34, // 'Float64'
        // struct spine:
        0x0a, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x01, 0x03, 0x01, 0x02, 0x03, 0x00,
        0x04, 0x00, 0x05, 0x00, 0x06, 0x01, 0x03, 0x05, 0x06, 0x07, 0x01, 0x03, 0x00, 0x04, 0x08,
        0x09,
    ];
    assert_eq!(
        got, expect,
        "framed int/float tuple must be the full colon-framed golden document"
    );
    assert_eq!(got[8], 0x07, "leaf count = 7 (framed int/float tuple)");

    op_drop(pair);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_of_a_framed_int_map_is_the_colon_framed_golden() {
    reset();
    let desc = framed_int_map_descriptor();
    // Insert 8 THEN observe canonical order puts 7 first — build 7 then 8 (canonical is key-sorted
    // regardless, but this documents the value the fixture names).
    let m = op_map_insert(
        op_map_insert(op_map_empty(), op_box_int(7), op_box_int(70)),
        op_box_int(8),
        op_box_int(99),
    );
    let got = op_value_encode_form(m, &desc).expect("encode framed int map");

    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, m, descriptor.root, 0)
        .expect("recursive encode");
    assert_eq!(got, b.finish(root), "iterative/recursive disagree on map");

    let back = op_value_decode(&got, &desc);
    assert_ne!(back, Handle::NULL, "map doc must decode");
    assert_eq!(
        got,
        op_value_encode_form(back, &desc).expect("re-encode map"),
        "decode∘encode ≠ id on map"
    );
    op_drop(back);

    // FULL document: ':' , 'map', 7, 70, 8, 99, 'Map', 'Int64' (8 leaves) + spine. Entries in
    // canonical key order (7 before 8).
    // M2 head-first golden: value head Ctor(Map)=0x17; each entry is (FieldPair k v) with FieldPair
    // head=0x19 (was a bare `(k v)` pair). 9 leaves (was 8 — the FieldPair ctor leaf is added).
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, 0x09, 0x0a, 0x01, 0x3a, 0x17, 0x19, 0x00,
        0x01, 0x07, 0x00, 0x01, 0x46, 0x00, 0x01, 0x08, 0x00, 0x01, 0x63, 0x0a, 0x03, 0x4d, 0x61,
        0x70, 0x0a, 0x05, 0x49, 0x6e, 0x74, 0x36, 0x34, 0x10, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02,
        0x00, 0x03, 0x00, 0x04, 0x01, 0x03, 0x02, 0x03, 0x04, 0x00, 0x02, 0x00, 0x05, 0x00, 0x06,
        0x01, 0x03, 0x06, 0x07, 0x08, 0x01, 0x03, 0x01, 0x05, 0x09, 0x00, 0x07, 0x00, 0x08, 0x00,
        0x08, 0x01, 0x03, 0x0b, 0x0c, 0x0d, 0x01, 0x03, 0x00, 0x0a, 0x0e, 0x0f,
    ];
    assert_eq!(
        got, expect,
        "framed int map must be the full colon-framed golden document"
    );
    assert_eq!(
        got[8], 0x09,
        "map leaf count = 9 (M2: + the FieldPair ctor leaf)"
    );

    op_drop(m);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_of_a_framed_int_set_is_the_colon_framed_golden() {
    reset();
    let desc = framed_int_set_descriptor();
    let mut s = op_set_empty();
    for e in [7i64, 12, 17] {
        s = op_set_insert(s, op_box_int(e));
    }
    let got = op_value_encode_form(s, &desc).expect("encode framed int set");

    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, s, descriptor.root, 0)
        .expect("recursive encode");
    assert_eq!(got, b.finish(root), "iterative/recursive disagree on set");

    let back = op_value_decode(&got, &desc);
    assert_ne!(back, Handle::NULL, "set doc must decode");
    assert_eq!(
        got,
        op_value_encode_form(back, &desc).expect("re-encode set"),
        "decode∘encode ≠ id on set"
    );
    op_drop(back);

    // FULL document: ':' , '.', 'Set', 'of', 'list', 7, 12, 17, 'Int64' (9 leaves) + spine. The
    // ((. Set of) (list …)) member-access form, elements in canonical order 7 < 12 < 17.
    // M2 head-first golden: a Set is flat `(Ctor(Set) e…)` — the Set ctor head=0x18 + the sorted
    // elements directly (was `((. Set of) (list e…))`). 7 leaves (was 9 — dropped `.`/`of`/`list`
    // names, added the Set ctor leaf).
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, 0x07, 0x0a, 0x01, 0x3a, 0x18, 0x00, 0x01,
        0x07, 0x00, 0x01, 0x0c, 0x00, 0x01, 0x11, 0x0a, 0x03, 0x53, 0x65, 0x74, 0x0a, 0x05, 0x49,
        0x6e, 0x74, 0x36, 0x34, 0x0a, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04,
        0x01, 0x04, 0x01, 0x02, 0x03, 0x04, 0x00, 0x05, 0x00, 0x06, 0x01, 0x02, 0x06, 0x07, 0x01,
        0x03, 0x00, 0x05, 0x08, 0x09,
    ];
    assert_eq!(
        got, expect,
        "framed int set must be the full colon-framed golden document"
    );
    assert_eq!(
        got[8], 0x07,
        "set leaf count = 7 (M2 flat Ctor(Set) — dropped the (.Set of)/list wrapper)"
    );

    op_drop(s);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_of_a_framed_bigint_is_the_colon_framed_golden() {
    reset();
    let desc = framed_bigint_descriptor();
    let n = op_bigint_of_i64(5);
    let got = op_value_encode_form(n, &desc).expect("encode framed bigint");

    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, n, descriptor.root, 0)
        .expect("recursive encode");
    assert_eq!(
        got,
        b.finish(root),
        "iterative/recursive disagree on bigint"
    );

    let back = op_value_decode(&got, &desc);
    assert_ne!(back, Handle::NULL, "bigint doc must decode");
    assert_eq!(
        got,
        op_value_encode_form(back, &desc).expect("re-encode bigint"),
        "decode∘encode ≠ id on bigint"
    );
    op_drop(back);

    // FULL document: ':' , 5 (KIND_INT), 'BigInt' (3 leaves) + spine.
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // cdzast\x00\x01
        0x03, // 3 leaves
        0x0a, 0x01, 0x3a, // ':'
        0x00, 0x01, 0x05, // INT 5 (a BigInt renders as a plain KIND_INT leaf)
        0x0a, 0x06, 0x42, 0x69, 0x67, 0x49, 0x6e, 0x74, // 'BigInt'
        // struct spine:
        0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x01, 0x02, 0x03,
    ];
    assert_eq!(
        got, expect,
        "framed bigint must be the full colon-framed golden document"
    );
    assert_eq!(got[8], 0x03, "bigint leaf count = 3");

    op_drop(n);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_of_a_framed_rational_is_the_colon_framed_golden() {
    reset();
    let desc = framed_rational_descriptor();
    let r = op_rational_of(op_bigint_of_i64(3), op_bigint_of_i64(4));
    let got = op_value_encode_form(r, &desc).expect("encode framed rational");

    let descriptor = decode_descriptor(&desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, r, descriptor.root, 0)
        .expect("recursive encode");
    assert_eq!(
        got,
        b.finish(root),
        "iterative/recursive disagree on rational"
    );

    let back = op_value_decode(&got, &desc);
    assert_ne!(back, Handle::NULL, "rational doc must decode");
    assert_eq!(
        got,
        op_value_encode_form(back, &desc).expect("re-encode rational"),
        "decode∘encode ≠ id on rational"
    );
    op_drop(back);

    // seq-204: the value is now the native head+children list `(KIND_RATIONAL 3 4)` — NOT the old
    // num/den NAME. iterative==recursive + decode∘encode==id above pin the bytes; the exact 3-way
    // cross-renderer byte golden (op62 == rust emit == cadenza-ast Builder::rational) re-pins together
    // in the coordinated flag-day land. Assert the form is native here:
    assert!(
        got.contains(&doc::KIND_RATIONAL),
        "the value must carry the native KIND_RATIONAL(27) tag head, not a num/den name"
    );
    assert!(
        !got.windows(3).any(|w| w == b"3/4"),
        "no legacy num/den NAME string — 3 and 4 are ordinary Int child leaves"
    );

    op_drop(r);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_decode_of_char_scalar_emits_kind_char_and_round_trips() {
    reset();
    let desc = char_scalar_descriptor();
    let descriptor = decode_descriptor(&desc).expect("char descriptor");
    for c in ['A', 'λ', '🦀', '\u{0}'] {
        let cp = c as i64;
        let v = op_box_int(cp);
        let got = op_value_encode_form(v, &desc).expect("encode char");

        // (1) op62 took the Char arm — the document carries a KIND_CHAR leaf, not a KIND_INT.
        let leaf = kind_char_leaf_bytes(c);
        assert!(
            got.windows(leaf.len()).any(|w| w == leaf.as_slice()),
            "char {c:?} (U+{cp:04X}) must encode as a KIND_CHAR leaf {leaf:02x?}, not KIND_INT — doc {got:02x?}"
        );

        // (2) iterative production walk == recursive oracle (exercises the mirror's S::Char arm).
        let mut b = DocBuilder::default();
        let root = encode_value_recursive(&descriptor, &mut b, v, descriptor.root, 0)
            .expect("recursive encode char");
        assert_eq!(
            got,
            b.finish(root),
            "iterative/recursive disagree on char {c:?}"
        );

        // (3) decode ∘ encode == id, back to the SAME codepoint (tag-19 decode_shape + op90 KIND_CHAR).
        let back = op_value_decode(&got, &desc);
        assert_ne!(back, Handle::NULL, "char {c:?} doc must decode");
        assert_eq!(
            op_get_int(back),
            cp,
            "char {c:?} must round-trip back to its codepoint"
        );
        op_drop(back);
        op_drop(v);
    }
    assert_eq!(
        live_nodes(),
        0,
        "char values are immediates — no leak across the codepoint sweep"
    );
}

#[test]
fn value_encode_of_char_in_a_tuple_round_trips() {
    reset();
    let desc = char_int_tuple_descriptor();
    let descriptor = decode_descriptor(&desc).expect("tuple descriptor");
    let t = op_arr_alloc(2);
    op_arr_set(t, 0, op_box_int('λ' as i64));
    op_arr_set(t, 1, op_box_int(42));
    let got = op_value_encode_form(t, &desc).expect("encode (tuple Char Int)");

    let leaf = kind_char_leaf_bytes('λ');
    assert!(
        got.windows(leaf.len()).any(|w| w == leaf.as_slice()),
        "the tuple's Char field must emit a KIND_CHAR leaf {leaf:02x?} — doc {got:02x?}"
    );

    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, t, descriptor.root, 0)
        .expect("recursive encode tuple");
    assert_eq!(
        got,
        b.finish(root),
        "iterative/recursive disagree on (tuple Char Int)"
    );

    let back = op_value_decode(&got, &desc);
    assert_ne!(back, Handle::NULL, "(tuple Char Int) doc must decode");
    assert_eq!(
        op_get_int(op_arr_get(back, 0)),
        'λ' as i64,
        "char field round-trips"
    );
    assert_eq!(op_get_int(op_arr_get(back, 1)), 42, "int field round-trips");
    op_drop(back);

    op_drop(t);
    assert_eq!(live_nodes(), 0, "tuple freed clean — no leak");
}

#[test]
fn value_encode_of_a_surrogate_char_declines_cleanly() {
    reset();
    let desc = char_scalar_descriptor();
    let bad = op_box_int(0xD800); // high-surrogate — not a scalar value
    assert!(
        op_value_encode_form(bad, &desc).is_none(),
        "a surrogate codepoint must make value-encode decline, not trap or mis-emit"
    );
    op_drop(bad);
    assert_eq!(live_nodes(), 0, "declined encode leaks nothing");
}

#[test]
fn value_encode_form_matches_the_codec_for_a_recursive_sum() {
    reset();
    let desc = intlist_descriptor();
    // Nil (disc 1, unit payload) → the LEN-46 oracle dump (see the const IntList value form).
    let nil = op_sum_new(1, op_arr_alloc(0));
    let got = op_value_encode_form(nil, &desc).expect("encode Nil");
    let expect_nil: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, 0x04, 0x0a, 0x01, 0x3a, 0x0a, 0x03, 0x4e,
        0x69, 0x6c, 0x0a, 0x04, 0x75, 0x6e, 0x69, 0x74, 0x0a, 0x02, 0x49, 0x4c, 0x06, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x02, 0x01, 0x02, 0x01, 0x02, 0x00, 0x03, 0x01, 0x03, 0x00, 0x03, 0x04,
        0x05,
    ];
    assert_eq!(
        got, expect_nil,
        "Nil value form must be byte-identical to the codec"
    );
    op_drop(nil);

    // Cons(tuple 1 Nil) → the LEN-77 oracle dump.
    let inner_nil = op_sum_new(1, op_arr_alloc(0));
    let pair = op_arr_alloc(2);
    op_arr_set(pair, 0, op_box_int(1));
    op_arr_set(pair, 1, inner_nil);
    let cons = op_sum_new(0, pair);
    let got = op_value_encode_form(cons, &desc).expect("encode Cons");
    let expect_cons: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, 0x07, 0x0a, 0x01, 0x3a, 0x0a, 0x04, 0x43,
        0x6f, 0x6e, 0x73, 0x15, 0x00, 0x01, 0x01, // Cons, M2 Ctor(Tuple)=0x15, INT 1
        0x0a, 0x03, 0x4e, 0x69, 0x6c, 0x0a, 0x04, 0x75, 0x6e, 0x69, 0x74, 0x0a, 0x02, 0x49, 0x4c,
        0x0b, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00, 0x05, 0x01, 0x02,
        0x04, 0x05, 0x01, 0x03, 0x02, 0x03, 0x06, 0x01, 0x02, 0x01, 0x07, 0x00, 0x06, 0x01, 0x03,
        0x00, 0x08, 0x09, 0x0a,
    ];
    assert_eq!(
        got, expect_cons,
        "Cons value form must be byte-identical to the codec"
    );
    op_drop(cons);
}

#[test]
fn value_encode_sum_arm_carrying_a_record_is_not_empty() {
    reset();
    let before = live_nodes();
    // v-cml's exact descriptor bytes.
    let desc: &[u8] = &[
        0x05, 0x09, 0x02, 0x01, 0x41, 0x01, 0x01, 0x42, 0x03, 0x04, 0x03, 0x08, 0x01, 0x01, 0x78,
        0x02, 0x0a, 0x01, 0x50, 0x00, 0x04,
    ];
    // Build B(record x="hi"): arm B has disc 1; its payload is a 1-field record arr [str "hi"].
    let rec = op_arr_alloc(1);
    op_arr_set(rec, 0, op_str_new(String::from("hi")));
    let v = op_sum_new(1, rec); // B(record …)
    let doc_bytes = op_value_encode_form(v, desc).expect("encode Sum-arm-Record must not decline");
    let doc = parse_doc(&doc_bytes).expect("parse the value-form document");
    // The record must actually be rendered: the "B" head, "record"/"x" names, and the "hi" str leaf
    // must all be present. The reported bug drops the record → these leaves are absent.
    let has_name = |want: &str| {
        doc.leaves
            .iter()
            .any(|l| matches!(l, ParsedLeaf::Name(n) if n == want.as_bytes()))
    };
    let has_str = |want: &str| {
        doc.leaves
            .iter()
            .any(|l| matches!(l, ParsedLeaf::Str(b) if b == want.as_bytes()))
    };
    let has_ctor = |k: u8| {
        doc.leaves
            .iter()
            .any(|l| matches!(l, ParsedLeaf::Ctor(c) if *c == k))
    };
    assert!(has_name("B"), "the B variant head must be rendered");
    assert!(
        has_ctor(doc::KIND_RECORD_CTOR),
        "the M2 Record ctor head must be rendered (bug: record dropped)"
    );
    assert!(
        has_name("x"),
        "the record field name x must be rendered (bug: record dropped)"
    );
    assert!(
        has_str("hi"),
        "the record field value \"hi\" must be rendered (bug: empty leaf)"
    );
    op_drop(v);
    assert_eq!(
        live_nodes(),
        before,
        "no leak encoding the Sum-arm-Record value"
    );
}

#[test]
fn value_encode_leaf_order_is_canon_pre_order_first_encounter() {
    reset();
    let desc = record_with_set_descriptor();
    // record { members: {10, 2, 1, 3} (hash order ≠ value order), tag: 42 }.
    let mut set = op_set_empty();
    for &e in &[10i64, 2, 1, 3] {
        set = op_set_insert(set, op_box_int(e));
    }
    let rec = op_arr_alloc(2);
    op_arr_set(rec, 0, set); // field 0 = members (Set)
    op_arr_set(rec, 1, op_box_int(42)); // field 1 = tag (Int)
    let doc_bytes = op_value_encode_form(rec, &desc).expect("encode record-with-set");
    let doc = parse_doc(&doc_bytes).expect("parse the value-form document");

    // Re-walk the struct tree PRE-ORDER (root, then children left-to-right, an atom on first visit),
    // recording the order leaves are first referenced. `expected_next` is the id the NEXT
    // first-encountered leaf must have under canon's first-encounter numbering: 0, then 1, then 2, …
    // A leaf id that jumps ahead (or a re-encountered leaf that isn't already assigned) means the
    // emission order does NOT match canon — the exact post-order-emission regression this gate exists
    // to catch.
    let mut seen: alloc::collections::BTreeSet<u32> = alloc::collections::BTreeSet::new();
    let mut expected_next: u32 = 0;
    let mut stack: Vec<u32> = vec![doc.root];
    while let Some(struct_ix) = stack.pop() {
        match doc.structs.get(struct_ix as usize) {
            Some(ParsedStruct::Atom(leaf_id)) => {
                if !seen.contains(leaf_id) {
                    assert_eq!(
                        *leaf_id, expected_next,
                        "leaf {leaf_id} first-encountered out of canon pre-order (expected \
                         {expected_next}) — value-encode's leaf pool diverges from \
                         codec::encode(canon(tree)); a `=`/Set-head atom is emitted post-order again"
                    );
                    seen.insert(*leaf_id);
                    expected_next += 1;
                }
            }
            Some(ParsedStruct::List(kids)) => {
                // Push children in REVERSE so they pop left-to-right (source order) — matching canon's
                // `visit`, which pushes children reversed onto its job stack for the same reason.
                for &k in kids.iter().rev() {
                    stack.push(k);
                }
            }
            None => panic!("dangling struct index {struct_ix} in parsed document"),
        }
    }
    assert_eq!(
        seen.len() as u32,
        expected_next,
        "every distinct leaf must be first-encountered exactly once in pre-order"
    );
    // Sanity: the fixture actually exercised both convergence sites (M2: a FieldPair ctor head for the
    // record field + a Set ctor head), so a future refactor can't accidentally make this gate vacuous.
    let has_ctor = |k: u8| {
        doc.leaves
            .iter()
            .any(|l| matches!(l, ParsedLeaf::Ctor(c) if *c == k))
    };
    assert!(
        has_ctor(doc::KIND_FIELD_PAIR),
        "fixture must contain a record-field FieldPair ctor leaf"
    );
    assert!(
        has_ctor(doc::KIND_SET_CTOR),
        "fixture must contain a Set ctor head leaf"
    );

    op_drop(rec);
    assert_eq!(live_nodes(), 0, "no leak: the record (and its set) dropped");
}

#[test]
fn value_encode_iterative_matches_recursive_reference() {
    // The N=500 differential drives the RECURSIVE oracle 500 levels deep. That oracle exists only as
    // the simple recursive mirror of the iterative production walk, so it must STAY recursive — but as
    // Set/Map/Spread/Framed arms were added to it its debug-build frame grew (Rust sizes a frame to its
    // largest arm), and 500 frames now exceed the 2 MB default test-thread stack. Run the body on a
    // thread with a generous stack so the byte-identity coverage at N=500 is preserved. The heap and
    // its counters are thread-local, so `reset()`/`live_nodes()` all belong INSIDE the spawned thread.
    // (The production walk is iterative and proven safe to N=50 000 by the sibling deep test.)
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(differential_body)
        .expect("spawn oracle-differential thread")
        .join()
        .expect("oracle-differential thread panicked");
}

#[test]
fn value_encode_deep_recursive_value_does_not_overflow_the_stack() {
    reset();
    let desc = intlist_descriptor();
    const DEEP: usize = 50_000;
    let v = build_intlist(DEEP);
    let doc =
        op_value_encode_form(v, &desc).expect("deep encode must succeed, not crash or decline");
    // A well-formed non-empty document (header + pools + root); the exact length is not the point.
    assert!(
        doc.len() > DEEP,
        "a {DEEP}-element list yields a document with at least one struct per node"
    );
    op_drop(v);
    assert_eq!(live_nodes(), 0, "no leak after the deep list is dropped");
}

#[test]
fn value_encode_renders_a_string_leaf() {
    reset();
    let before = live_nodes();
    // Descriptor: table [0] = Str, root = 0. Tag 3 = Str.
    let desc: &[u8] = &[0x01, 0x03, 0x00]; // table_len=1, [0]=Str(tag 3), root=0
    let s = op_str_new(String::from("hi"));
    let got = op_value_encode_form(s, desc).expect("encode a String value");
    // header(8) · leaf_count=1 · leaf0 = KIND_STR(7) len=2 'h' 'i' · struct_count=1 ·
    // struct0 = TAG_ATOM(0) leaf 0 · root=0
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // cdzast\0\1
        0x01, // leaf_count = 1
        0x07, 0x02, 0x68, 0x69, // KIND_STR, len 2, "hi"
        0x01, // struct_count = 1
        0x00, 0x00, // TAG_ATOM, leaf id 0
        0x00, // root = 0
    ];
    assert_eq!(
        got, expect,
        "String value form must be a KIND_STR leaf, byte-identical to the codec"
    );
    op_drop(s);

    // An EMPTY string round-trips (zero-length body).
    let e = op_str_new(String::new());
    let got_e = op_value_encode_form(e, desc).expect("encode empty String");
    let expect_e: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, 0x01, 0x07, 0x00, 0x01, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        got_e, expect_e,
        "empty String → KIND_STR with a zero-length body"
    );
    op_drop(e);

    // A multi-byte (UTF-8) string keeps its bytes verbatim (length is BYTES, not scalars).
    let u = op_str_new(String::from("é")); // 2 UTF-8 bytes: 0xC3 0xA9
    let got_u = op_value_encode_form(u, desc).expect("encode UTF-8 String");
    assert_eq!(
        &got_u[8..13],
        &[0x01, 0x07, 0x02, 0xC3, 0xA9],
        "UTF-8 body verbatim, len = byte count"
    );
    op_drop(u);

    assert_eq!(live_nodes(), before, "no leak: every string value dropped");
}

#[test]
fn value_encode_str_leaf_inline_and_heap_boundary_round_trips() {
    reset();
    let before = live_nodes();
    let desc: &[u8] = &[0x01, 0x03, 0x00]; // [0]=Str(tag 3), root=0
    for len in [INLINE_RAW_CAP, INLINE_RAW_CAP + 1] {
        let body: Vec<u8> = (0..len as u8).map(|i| b'a' + (i % 26)).collect();
        let s = op_str_new(String::from_utf8(body.clone()).unwrap());
        let got = op_value_encode_form(s, desc).expect("encode a String");
        // header(8) · leaf_count=1 · KIND_STR(7) · LEB(len) · body · struct_count=1 · TAG_ATOM 0 · root 0
        let mut expect = vec![0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, 0x01, 0x07];
        let mut leb = len as u64;
        loop {
            let mut byte = (leb & 0x7f) as u8;
            leb >>= 7;
            if leb != 0 {
                byte |= 0x80;
            }
            expect.push(byte);
            if leb == 0 {
                break;
            }
        }
        expect.extend_from_slice(&body);
        expect.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // struct_count, TAG_ATOM, leaf 0, root 0
        assert_eq!(
            got, expect,
            "len={len} String encodes byte-exact (inline vs heap Raw arm invisible)"
        );
        op_drop(s);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the inline/heap boundary"
    );
}

#[test]
fn value_encode_descriptor_cache_does_not_cross_contaminate() {
    reset();
    let before = live_nodes();
    let desc_int: &[u8] = &[0x01, 0x00, 0x00]; // [0]=Int, root=0
    let desc_str: &[u8] = &[0x01, 0x03, 0x00]; // [0]=Str, root=0
    let iv = op_box_int(7);
    let sv = op_str_new(String::from("hi"));
    // The canonical single-value docs, captured by a FIRST decode of each (before any interleaving).
    let want_int = op_value_encode_form(iv, desc_int).expect("int");
    let want_str = op_value_encode_form(sv, desc_str).expect("str");
    // ALTERNATING A,B,A,B — every call is a cache MISS (bytes differ from the prior entry). Each must
    // still equal its canonical doc: a stale-entry bug would return the other value's shape.
    for _ in 0..4 {
        assert_eq!(
            op_value_encode_form(iv, desc_int).expect("int alt"),
            want_int,
            "Int under alternation"
        );
        assert_eq!(
            op_value_encode_form(sv, desc_str).expect("str alt"),
            want_str,
            "Str under alternation"
        );
    }
    // REPEATED A,A,A then B,B,B — cache HITS after the first; must still be correct.
    for _ in 0..3 {
        assert_eq!(
            op_value_encode_form(iv, desc_int).expect("int rep"),
            want_int,
            "Int under repetition"
        );
    }
    for _ in 0..3 {
        assert_eq!(
            op_value_encode_form(sv, desc_str).expect("str rep"),
            want_str,
            "Str under repetition"
        );
    }
    op_drop(iv);
    op_drop(sv);
    assert_eq!(live_nodes(), before, "no leak across the cache thrash");
}

#[test]
fn value_encode_rope_string_flattens_like_a_flat_leaf() {
    reset();
    let before = live_nodes();
    let desc: &[u8] = &[0x01, 0x03, 0x00]; // [0]=Str(tag 3), root=0

    // Helper: a str leaf carrying `s`'s UTF-8 bytes (same node shape a str/bytes leaf uses).
    let str_leaf_bytes = |s: &str| -> Handle {
        let b = op_bytes_alloc(s.len() as u32);
        for (i, &by) in s.as_bytes().iter().enumerate() {
            op_bytes_set(b, i as u32, by as u32);
        }
        b
    };

    // A ROPE: concat "caf" + "é" (é = 0xC3 0xA9, spanning the seam), then a further concat — exactly the
    // `String.concat` shape. The logical content is "caféXY".
    let rope = op_bytes_concat(str_leaf_bytes("caf"), str_leaf_bytes("é"));
    let rope = op_bytes_concat(rope, str_leaf_bytes("XY"));
    let got_rope = op_value_encode_form(rope, desc).expect("encode a rope String");

    // The equivalent FLAT string must produce byte-identical output (the flatten makes them agree).
    let flat = op_str_new(String::from("caféXY"));
    let got_flat = op_value_encode_form(flat, desc).expect("encode the flat String");
    assert_eq!(
        got_rope, got_flat,
        "a rope String value-encodes identically to its flattened content (the Shape::Str flatten)"
    );
    // And byte-exact: KIND_STR(7), len 7 (café=5 bytes + XY=2), body "caféXY".
    let expect_body: &[u8] = &[0x07, 0x07, b'c', b'a', b'f', 0xC3, 0xA9, b'X', b'Y'];
    assert_eq!(
        &got_rope[9..9 + expect_body.len()],
        expect_body,
        "KIND_STR with the flattened UTF-8 body"
    );

    op_drop(rope);
    op_drop(flat);
    assert_eq!(live_nodes(), before, "no leak: rope + flat strings dropped");
}

#[test]
fn value_encode_boxed_int_extremes_byte_exact() {
    reset();
    let before = live_nodes();
    let desc: &[u8] = &[0x01, 0x00, 0x00]; // table_len=1, [0]=Int(tag 0), root=0
    let hdr = [0x63u8, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01]; // cdzast\0\1
    // Assemble the expected single-int document from (kind, big-endian magnitude bytes).
    let doc_of = |kind: u8, mag: &[u8]| -> Vec<u8> {
        let mut d = hdr.to_vec();
        d.push(0x01); // leaf_count = 1
        d.push(kind);
        d.push(mag.len() as u8); // LEB len (all these lengths are < 128 → one byte)
        d.extend_from_slice(mag);
        d.push(0x01); // struct_count = 1
        d.push(0x00); // TAG_ATOM
        d.push(0x00); // leaf id 0
        d.push(0x00); // root = 0
        d
    };
    // (value, expected kind, expected big-endian magnitude with leading zeros stripped)
    let cases: &[(i64, u8, &[u8])] = &[
        (0, 0, &[]), // zero → empty magnitude, POSITIVE
        (1, 0, &[0x01]),
        (-1, 3, &[0x01]), // negative one
        (255, 0, &[0xff]),
        (256, 0, &[0x01, 0x00]), // two-byte magnitude, no stray leading zero
        (
            i64::MAX,
            0,
            &[0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        ),
        (
            i64::MIN,
            3,
            &[0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ), // unsigned_abs = 2^63
    ];
    for &(v, kind, mag) in cases {
        let h = op_box_int(v);
        let got = op_value_encode_form(h, desc).expect("encode a boxed int");
        assert_eq!(
            got,
            doc_of(kind, mag),
            "value-encode of {v} must be the codec's KIND_INT sign+magnitude form"
        );
        op_drop(h);
    }
    assert_eq!(live_nodes(), before, "no leak: every boxed int dropped");
}

#[test]
fn value_encode_reused_builder_is_byte_identical_and_state_free() {
    reset();
    let before = live_nodes();
    let desc = intlist_descriptor();

    // (1) The SAME value encoded repeatedly on the reused builder is byte-identical every time.
    let small = build_intlist(3);
    let d1 = op_value_encode_form(small, &desc).expect("encode #1");
    let d2 = op_value_encode_form(small, &desc).expect("encode #2");
    let d3 = op_value_encode_form(small, &desc).expect("encode #3");
    assert_eq!(
        d1, d2,
        "repeated encode #2 identical — reused builder carries no state"
    );
    assert_eq!(d1, d3, "repeated encode #3 identical");

    // (2) A LARGE encode between two SMALL ones must not bleed the large value's leaves/structs into
    // the small document — `reset` clears the pools, so the small doc equals its standalone encoding.
    let big = build_intlist(200);
    let dbig = op_value_encode_form(big, &desc).expect("encode a large list");
    assert!(
        dbig.len() > d1.len(),
        "the large value produces a larger document"
    );
    let d_after = op_value_encode_form(small, &desc).expect("encode small again after large");
    assert_eq!(
        d1, d_after,
        "a small value after a large one encodes IDENTICALLY — the reused builder was fully reset (no leftover leaves/structs from the large encode)"
    );

    op_drop(small);
    op_drop(big);
    assert_eq!(
        live_nodes(),
        before,
        "no leak: the reused builder retains capacity, not owned nodes"
    );
}

#[test]
fn value_decode_round_trips_scalar_leaves() {
    reset();
    let before = live_nodes();
    // Each original is dropped after its round-trip so the leak assertion is exact (an immediate like a
    // small int/bool is not a heap node; a boxed float / string leaf is, so drop them all uniformly).
    let cases: &[(Handle, &[u8])] = &[
        (op_box_int(42), &[0x01, 0x00, 0x00]), // Int (tag 0)
        (op_box_int(-7), &[0x01, 0x00, 0x00]),
        (op_box_int(0), &[0x01, 0x00, 0x00]),
        (op_box_bool(true), &[0x01, 0x01, 0x00]), // Bool (tag 1)
        (op_box_bool(false), &[0x01, 0x01, 0x00]),
        (op_box_float(1.5), &[0x01, 0x02, 0x00]), // Float (tag 2)
        (op_box_float(-2.0), &[0x01, 0x02, 0x00]),
        (op_box_float(3.14159), &[0x01, 0x02, 0x00]),
        (op_str_new(String::from("hello")), &[0x01, 0x03, 0x00]), // Str (tag 3)
        (op_str_new(String::new()), &[0x01, 0x03, 0x00]),
        (op_box_float32(0.1f32), &[0x01, 0x0e, 0x00]), // Float32 (tag 14)
    ];
    for &(v, desc) in cases {
        assert_value_roundtrips(v, desc);
        op_drop(v);
    }
    assert_eq!(live_nodes(), before, "no leak across scalar round-trips");
}

#[test]
fn value_decode_round_trips_bytes_leaf() {
    reset();
    let before = live_nodes();
    let buf = op_bytes_alloc(3);
    op_bytes_set(buf, 0, 7);
    op_bytes_set(buf, 1, 0);
    op_bytes_set(buf, 2, 255);
    assert_value_roundtrips(buf, &[0x01, 0x04, 0x00]); // Bytes (tag 4)
    op_drop(buf);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_decode_round_trips_flat_tuple() {
    reset();
    let before = live_nodes();
    // desc: table [0]=Int, [1]=Tuple[0,0], root=1.
    // [table_len=2][0:Int][6:Tuple][n=2][0][0][root=1]
    let desc: &[u8] = &[0x02, 0x00, 0x06, 0x02, 0x00, 0x00, 0x01];
    let t = op_arr_alloc(2);
    op_arr_set(t, 0, op_box_int(3));
    op_arr_set(t, 1, op_box_int(-5));
    assert_value_roundtrips(t, desc);
    op_drop(t);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_decode_round_trips_record_eq_form() {
    reset();
    let before = live_nodes();
    // desc: table [0]=Int, [1]=Record{a:0,b:0}, root=1. Record tag 8: [8][n=2][len 'a'][0][len 'b'][0].
    let desc: &[u8] = &[
        0x02, // table_len
        0x00, // [0] Int
        0x08, 0x02, 0x01, b'a', 0x00, 0x01, b'b', 0x00, // [1] Record{a→0,b→0}
        0x01, // root = 1
    ];
    // Fields in canonical (sorted) order a,b → positional [a,b]. (Value renders `(record (= a 1) (= b 9))`.)
    let r = op_arr_alloc(2);
    op_arr_set(r, 0, op_box_int(1));
    op_arr_set(r, 1, op_box_int(9));
    assert_value_roundtrips(r, desc);
    op_drop(r);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_decode_round_trips_recursive_list() {
    reset();
    let before = live_nodes();
    // desc: table [0]=Int, [1]=List(0), root=1. List tag 7: [7][elem=0].
    let desc: &[u8] = &[0x02, 0x00, 0x07, 0x00, 0x01];
    let mut v = op_vec_empty();
    for i in 0..5 {
        v = op_vec_push(v, op_box_int(i * 10 - 20));
    }
    assert_value_roundtrips(v, desc);
    op_drop(v);
    // An EMPTY list too.
    let e = op_vec_empty();
    assert_value_roundtrips(e, desc);
    op_drop(e);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_decode_round_trips_sum_variants() {
    reset();
    let before = live_nodes();
    // desc: table [0]=Int, [1]=Unit, [2]=Sum{None→1, Some→0}, root=2.
    // Sum tag 9: [9][n=2][len 'None'][1][len 'Some'][0].
    let desc: &[u8] = &[
        0x03, // table_len
        0x00, // [0] Int
        0x05, // [1] Unit
        0x09, 0x02, 0x04, b'N', b'o', b'n', b'e', 0x01, 0x04, b'S', b'o', b'm', b'e',
        0x00, // [2] Sum
        0x02, // root = 2
    ];
    // Some(9): disc 1, payload Int.
    let some = op_sum_new(1, op_box_int(9));
    assert_value_roundtrips(some, desc);
    op_drop(some);
    // None: disc 0, payload unit.
    let none = op_sum_new(0, imm_unit());
    assert_value_roundtrips(none, desc);
    op_drop(none);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_decode_returns_null_on_shape_mismatch_never_traps() {
    reset();
    let before = live_nodes();
    // Encode an Int, decode it against a Str descriptor → NULL (mismatch), no trap, no leak.
    let v = op_box_int(5);
    let int_desc: &[u8] = &[0x01, 0x00, 0x00];
    let str_desc: &[u8] = &[0x01, 0x03, 0x00];
    let doc = op_value_encode_form(v, int_desc).expect("encode");
    assert_eq!(
        op_value_decode(&doc, str_desc),
        Handle::NULL,
        "shape mismatch → NULL"
    );
    // A garbage document → NULL (bad header).
    assert_eq!(
        op_value_decode(&[0, 1, 2, 3], int_desc),
        Handle::NULL,
        "bad header → NULL"
    );
    // A malformed descriptor → NULL.
    assert_eq!(
        op_value_decode(&doc, &[0xff]),
        Handle::NULL,
        "bad descriptor → NULL"
    );
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak (NULL is not a heap node)");
}

#[test]
fn value_encode_deeply_nested_type_node_declines_no_overflow() {
    reset();
    let before = live_nodes();
    fn leb(o: &mut Vec<u8>, mut v: u64) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            o.push(b);
            if v == 0 {
                break;
            }
        }
    }
    // table_len=2, [0]=Int, [1]=Framed<DEPTH-nested TypeNode>[inner=0]; root=1.
    let mut d = Vec::new();
    leb(&mut d, 2);
    d.push(0); // [0] Int
    d.push(15); // [1] Framed
    const DEPTH: usize = 200_000; // vastly exceeds TYPE_NODE_DEPTH_CAP
    for _ in 0..DEPTH {
        leb(&mut d, 0); // empty head
        leb(&mut d, 1); // 1 child → recurse
    }
    leb(&mut d, 0); // innermost: empty head
    leb(&mut d, 0); // 0 children
    leb(&mut d, 0); // Framed inner idx → 0
    leb(&mut d, 1); // root = 1
    let v = op_box_int(7);
    // MUST return (as None), NOT overflow the stack.
    assert!(
        op_value_encode_form(v, &d).is_none(),
        "a runaway-nested type node declines, it does not abort the guest"
    );
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak on the declined encode");
}

#[test]
fn value_encode_wide_record_matches_recursive_reference() {
    reset();
    let before = live_nodes();
    const N: usize = 300;
    // Descriptor: table [0]=Int, [1]=Record with N fields "f0".."f{N-1}", each field → 0 (Int). root=1.
    fn leb(o: &mut Vec<u8>, mut v: u64) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            o.push(b);
            if v == 0 {
                break;
            }
        }
    }
    let mut d = Vec::new();
    leb(&mut d, 2);
    d.push(0); // [0] Int
    d.push(8); // [1] Record
    leb(&mut d, N as u64);
    for i in 0..N {
        let name = alloc::format!("f{i}");
        leb(&mut d, name.len() as u64);
        d.extend_from_slice(name.as_bytes());
        leb(&mut d, 0); // field type → Int
    }
    leb(&mut d, 1); // root = the Record

    let rec = op_arr_alloc(N as u32);
    for i in 0..N {
        op_arr_set(rec, i as u32, op_box_int(i as i64));
    }
    let iter_doc = op_value_encode_form(rec, &d).expect("wide record encodes");
    // Differential: the recursive oracle (shares `name_leaf`, so this also confirms the map-dedup and a
    // hypothetical scan-dedup agree) must produce byte-identical output.
    let descriptor = decode_descriptor(&d).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, rec, descriptor.root, 0).expect("recursive");
    assert_eq!(
        iter_doc,
        b.finish(root),
        "wide-record iterative and recursive encode must agree"
    );
    op_drop(rec);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_encode_string_list_matches_recursive_reference() {
    reset();
    // Descriptor built programmatically (nested shapes are error-prone as a hand array):
    // table [0]=Str, [1]=Sum[(Cons→2),(Nil→3)], [2]=Tuple[→0,→1], [3]=Unit, [4]=Named("SL"→1); root=4.
    let mut d: Vec<u8> = Vec::new();
    let leb = |out: &mut Vec<u8>, v: u64| {
        let mut v = v;
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    };
    let name = |out: &mut Vec<u8>, s: &str| {
        let mut tmp = Vec::new();
        let mut v = s.len() as u64;
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            tmp.push(b);
            if v == 0 {
                break;
            }
        }
        out.extend_from_slice(&tmp);
        out.extend_from_slice(s.as_bytes());
    };
    leb(&mut d, 5); // table_len
    d.push(3); // [0] Str
    d.push(9); // [1] Sum
    leb(&mut d, 2);
    name(&mut d, "Cons");
    leb(&mut d, 2);
    name(&mut d, "Nil");
    leb(&mut d, 3);
    d.push(6); // [2] Tuple [→0, →1]
    leb(&mut d, 2);
    leb(&mut d, 0);
    leb(&mut d, 1);
    d.push(5); // [3] Unit
    d.push(10); // [4] Named("SL" → 1)
    name(&mut d, "SL");
    leb(&mut d, 1);
    leb(&mut d, 4); // root

    // Build ["a", "bb", "ccc"] as Cons(tuple s rest)…Nil.
    let strs = ["a", "bb", "ccc"];
    let mut acc = op_sum_new(1, op_arr_alloc(0)); // Nil
    for s in strs.iter().rev() {
        let pair = op_arr_alloc(2);
        op_arr_set(pair, 0, op_str_new(String::from(*s)));
        op_arr_set(pair, 1, acc);
        acc = op_sum_new(0, pair);
    }
    let iter_doc = op_value_encode_form(acc, &d).expect("iterative encode of a Str list");
    // Recursive oracle over the same value.
    let descriptor = decode_descriptor(&d).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, acc, descriptor.root, 0).expect("recursive");
    let rec_doc = b.finish(root);
    assert_eq!(
        iter_doc, rec_doc,
        "iterative and recursive String-list encode must agree"
    );
    // The three string bodies appear in the leaf pool.
    assert!(iter_doc.windows(1).any(|w| w == b"a"), "string 'a' present");
    assert!(
        iter_doc.windows(3).any(|w| w == b"ccc"),
        "string 'ccc' present"
    );
    op_drop(acc);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_renders_a_bytes_leaf() {
    reset();
    let before = live_nodes();
    // Descriptor: table [0] = Bytes, root = 0. Tag 4 = Bytes.
    let desc: &[u8] = &[0x01, 0x04, 0x00]; // table_len=1, [0]=Bytes(tag 4), root=0

    // (1) A flat leaf [0x01, 0x02, 0xff].
    let flat = bytes_leaf(&[0x01, 0x02, 0xff]);
    let got = op_value_encode_form(flat, desc).expect("encode a Bytes leaf");
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // cdzast\0\1
        0x01, // leaf_count = 1
        0x0b, 0x03, 0x01, 0x02, 0xff, // KIND_BYTES(11), len 3, bytes
        0x01, // struct_count = 1
        0x00, 0x00, // TAG_ATOM, leaf id 0
        0x00, // root = 0
    ];
    assert_eq!(
        got, expect,
        "Bytes value form must be a KIND_BYTES leaf, byte-identical to the codec"
    );
    op_drop(flat);

    // (2) An EMPTY bytes value.
    let empty = op_bytes_alloc(0);
    let got_e = op_value_encode_form(empty, desc).expect("encode empty Bytes");
    let expect_e: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, 0x01, 0x0b, 0x00, 0x01, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        got_e, expect_e,
        "empty Bytes → KIND_BYTES with a zero-length body"
    );
    op_drop(empty);

    // (3) A ROPE (concat of two leaves) must FLATTEN to its logical bytes before encoding — the
    // KIND_BYTES body is the concatenation, identical to a flat leaf of the same content.
    let rope = op_bytes_concat(bytes_leaf(&[0xaa, 0xbb]), bytes_leaf(&[0xcc]));
    let got_r = op_value_encode_form(rope, desc).expect("encode a Bytes rope");
    assert_eq!(
        &got_r[8..14],
        &[0x01, 0x0b, 0x03, 0xaa, 0xbb, 0xcc],
        "a rope flattens to its logical bytes (0xaa 0xbb 0xcc) in one KIND_BYTES leaf"
    );
    op_drop(rope);

    assert_eq!(live_nodes(), before, "no leak: every bytes value dropped");
}

#[test]
fn value_encode_bytes_list_matches_recursive_reference() {
    reset();
    // table [0]=Bytes, [1]=Sum[(Cons→2),(Nil→3)], [2]=Tuple[→0,→1], [3]=Unit, [4]=Named("BL"→1); root=4.
    let mut d: Vec<u8> = Vec::new();
    let leb = |out: &mut Vec<u8>, mut v: u64| loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    };
    let name = |out: &mut Vec<u8>, s: &str| {
        let mut v = s.len() as u64;
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
        out.extend_from_slice(s.as_bytes());
    };
    leb(&mut d, 5);
    d.push(4); // [0] Bytes
    d.push(9); // [1] Sum
    leb(&mut d, 2);
    name(&mut d, "Cons");
    leb(&mut d, 2);
    name(&mut d, "Nil");
    leb(&mut d, 3);
    d.push(6); // [2] Tuple [→0, →1]
    leb(&mut d, 2);
    leb(&mut d, 0);
    leb(&mut d, 1);
    d.push(5); // [3] Unit
    d.push(10); // [4] Named("BL" → 1)
    name(&mut d, "BL");
    leb(&mut d, 1);
    leb(&mut d, 4);

    // Build a list of two Bytes elements, one of them a ROPE (to exercise flatten under the walk).
    let e0 = bytes_leaf(&[0x10, 0x20]);
    let e1 = op_bytes_concat(bytes_leaf(&[0x30]), bytes_leaf(&[0x40, 0x50])); // rope → 0x30 0x40 0x50
    let nil = op_sum_new(1, op_arr_alloc(0));
    let pair1 = op_arr_alloc(2);
    op_arr_set(pair1, 0, e1);
    op_arr_set(pair1, 1, nil);
    let cons1 = op_sum_new(0, pair1);
    let pair0 = op_arr_alloc(2);
    op_arr_set(pair0, 0, e0);
    op_arr_set(pair0, 1, cons1);
    let acc = op_sum_new(0, pair0);

    let iter_doc = op_value_encode_form(acc, &d).expect("iterative encode of a Bytes list");
    let descriptor = decode_descriptor(&d).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, acc, descriptor.root, 0).expect("recursive");
    let rec_doc = b.finish(root);
    assert_eq!(
        iter_doc, rec_doc,
        "iterative and recursive Bytes-list encode must agree"
    );
    // The flattened rope body appears verbatim.
    assert!(
        iter_doc.windows(3).any(|w| w == [0x30, 0x40, 0x50]),
        "flattened rope body present"
    );
    op_drop(acc);
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn value_encode_renders_a_float_leaf() {
    reset();
    let before = live_nodes();
    let desc: &[u8] = &[0x01, 0x02, 0x00]; // table [0] = Float (tag 2), root = 0

    // 1.5 → decimal 15 × 10^-1: negative=0, exponent=-1 (i64 BE), siglen=1, mag=[0x0f].
    let f = op_box_float(1.5);
    let got = op_value_encode_form(f, desc).expect("encode 1.5");
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // header
        0x01, // leaf_count = 1
        0x06, // KIND_FLOAT
        0x00, // negative = false
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // exponent = -1 (i64 big-endian)
        0x01, 0x0f, // siglen = 1, magnitude [15]
        0x01, // struct_count = 1
        0x00, 0x00, // TAG_ATOM, leaf 0
        0x00, // root
    ];
    assert_eq!(
        got, expect,
        "1.5 → KIND_FLOAT decimal 15×10^-1, byte-identical to the codec"
    );
    op_drop(f);

    // 0.0 → zero: exponent 0 (8 zero bytes), siglen 0 (empty magnitude).
    let z = op_box_float(0.0);
    let got_z = op_value_encode_form(z, desc).expect("encode 0.0");
    assert_eq!(
        &got_z[9..20],
        &[
            0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
        ],
        "0.0 → KIND_FLOAT, negative=0, exponent=0, siglen=0 (empty magnitude)"
    );
    op_drop(z);

    // -2.0 → negative flag set; {:e} of -2.0 = -2e0 → digits "2", exp 0, mag [2].
    let n = op_box_float(-2.0);
    let got_n = op_value_encode_form(n, desc).expect("encode -2.0");
    assert_eq!(got_n[10], 0x01, "negative flag set for -2.0");
    assert_eq!(&got_n[19..21], &[0x01, 0x02], "siglen 1, magnitude [2]");
    op_drop(n);

    // A NON-FINITE float ENCODES as its dedicated PAYLOADLESS word-form leaf (#7479): nan/inf have no
    // exact-decimal (KIND_FLOAT) form, so they CROSS the value-encode boundary as KIND_FLOAT_NAN=17 /
    // KIND_FLOAT_POS_INF=18 / KIND_FLOAT_NEG_INF=19 rather than declining the whole encode. The kind byte
    // sits at index 9 (8-byte header + 1-byte leaf_count), and there is NO payload after it (unlike
    // KIND_FLOAT's negative+exponent+siglen+magnitude). (Was `is_none()`/"declines" pre-#7479; render-ty's
    // #7479 non-finite-encode changed the contract but this ungated unit test kept the old assertion.)
    let nan = op_box_float(f64::NAN);
    let got_nan = op_value_encode_form(nan, desc).expect("nan encodes (KIND_FLOAT_NAN)");
    assert_eq!(
        got_nan[9],
        doc::KIND_FLOAT_NAN,
        "nan → KIND_FLOAT_NAN (17), payloadless"
    );
    op_drop(nan);
    let pinf = op_box_float(f64::INFINITY);
    let got_pinf = op_value_encode_form(pinf, desc).expect("+inf encodes (KIND_FLOAT_POS_INF)");
    assert_eq!(
        got_pinf[9],
        doc::KIND_FLOAT_POS_INF,
        "+inf → KIND_FLOAT_POS_INF (18), payloadless"
    );
    op_drop(pinf);
    let ninf = op_box_float(f64::NEG_INFINITY);
    let got_ninf = op_value_encode_form(ninf, desc).expect("-inf encodes (KIND_FLOAT_NEG_INF)");
    assert_eq!(
        got_ninf[9],
        doc::KIND_FLOAT_NEG_INF,
        "-inf → KIND_FLOAT_NEG_INF (19), payloadless"
    );
    op_drop(ninf);

    assert_eq!(live_nodes(), before, "no leak: every float value dropped");
}

#[test]
fn value_encode_renders_a_float32_as_the_f32_shortest_decimal() {
    reset();
    let before = live_nodes();
    // Descriptor: table [0] = Float32 (tag 14), root = 0.
    let desc: &[u8] = &[0x01, 0x0e, 0x00];

    // 1.5f32 → decimal 15 × 10^-1: byte-exact (same decimal as 1.5f64, since 1.5 is exact in both).
    let f = op_box_float32(1.5f32);
    let got = op_value_encode_form(f, desc).expect("encode 1.5f32");
    let expect: &[u8] = &[
        0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01, // header
        0x01, // leaf_count
        0x06, 0x00, // KIND_FLOAT, negative=false
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // exponent = -1
        0x01, 0x0f, // siglen 1, magnitude [15]
        0x01, 0x00, 0x00, 0x00, // struct_count, TAG_ATOM leaf 0, root
    ];
    assert_eq!(got, expect, "1.5f32 → KIND_FLOAT decimal 15×10^-1");
    op_drop(f);

    // 0.1f32 → the f32's shortest decimal is "0.1" = 1 × 10^-1, NOT the f64-promoted precision. Extract
    // the leaf's (exponent, magnitude): magnitude [1], exponent -1.
    let g = op_box_float32(0.1f32);
    let got_g = op_value_encode_form(g, desc).expect("encode 0.1f32");
    // bytes [10]=neg, [11..19]=exp BE, [19]=siglen, [20..]=mag.
    assert_eq!(got_g[10], 0, "0.1f32 is positive");
    let mut eb = [0u8; 8];
    eb.copy_from_slice(&got_g[11..19]);
    assert_eq!(
        i64::from_be_bytes(eb),
        -1,
        "0.1f32 exponent is -1 (→ 0.1), NOT the f64-promotion's -17"
    );
    assert_eq!(got_g[19], 1, "siglen 1");
    assert_eq!(
        &got_g[20..21],
        &[0x01],
        "magnitude [1] → 1×10^-1 = 0.1, NOT 10000000149011612×10^-17"
    );
    op_drop(g);

    // A non-finite f32 ENCODES its dedicated payloadless word-form leaf (#7479), not declines: nan →
    // KIND_FLOAT_NAN=17 (kind byte at index 9, no payload). (Was `is_none()`/"declines" pre-#7479.)
    let nan = op_box_float32(f32::NAN);
    let got_nan = op_value_encode_form(nan, desc).expect("f32 nan encodes (KIND_FLOAT_NAN)");
    assert_eq!(
        got_nan[9],
        doc::KIND_FLOAT_NAN,
        "f32 nan → KIND_FLOAT_NAN (17), payloadless"
    );
    op_drop(nan);

    // Differential: the iterative walk matches the recursive oracle byte-for-byte for a Float32.
    let h = op_box_float32(0.1f32);
    let iter_doc = op_value_encode_form(h, desc).expect("iterative");
    let descriptor = decode_descriptor(desc).expect("descriptor");
    let mut bld = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut bld, h, descriptor.root, 0).expect("recursive");
    assert_eq!(
        iter_doc,
        bld.finish(root),
        "iterative and recursive Float32 encode must agree"
    );
    op_drop(h);

    assert_eq!(live_nodes(), before, "no leak: every float32 value dropped");
}

#[test]
fn value_encode_float_decimal_round_trips_to_the_same_f64() {
    reset();
    let desc: &[u8] = &[0x01, 0x02, 0x00];
    for &v in &[
        1.5f64,
        0.0,
        -2.0,
        3.14159,
        1e10,
        -1e-10,
        123456.789,
        0.1,
        -0.0,
        f64::MAX,
        f64::MIN,
        5e-324,
    ] {
        let h = op_box_float(v);
        let doc = op_value_encode_form(h, desc).expect("finite float encodes");
        // Use the limb-based, LEB-length-aware reader — a WHOLE float's FULL exact expansion (e.g.
        // f64::MAX) has a 128-byte significand whose length is a multi-byte LEB, which a fixed-offset /
        // u128 read would garble/overflow.
        let decimal = float_doc_to_decimal(&doc);
        let reconstructed: f64 = decimal.parse().expect("decimal parses");
        assert_eq!(
            reconstructed.to_bits(),
            v.to_bits(),
            "float {v} round-trips through its KIND_FLOAT decimal ({decimal})"
        );
        op_drop(h);
    }
    assert_eq!(live_nodes(), 0, "no leak");
}

#[test]
fn prop_float_leaf_round_trips_bit_exact_under_random_f64() {
    bolero::check!().with_type::<u64>().for_each(|&bits| {
        reset();
        let v = f64::from_bits(bits);
        let desc: &[u8] = &[0x01, 0x02, 0x00]; // [0]=Float(tag 2), root=0
        let h = op_box_float(v);
        let doc = op_value_encode_form(h, desc);
        if v.is_finite() {
            let doc = doc.expect("a finite float must encode");
            let decimal = float_doc_to_decimal(&doc);
            let reconstructed: f64 = decimal.parse().expect("the KIND_FLOAT decimal must parse");
            assert_eq!(
                reconstructed.to_bits(),
                v.to_bits(),
                "f64 {v} (bits {bits:#018x}) must round-trip through its decimal {decimal}"
            );
        } else {
            // nan/inf have no exact-decimal form → they ENCODE via their dedicated PAYLOADLESS word-form
            // leaf (#7479): KIND_FLOAT_NAN=17 / KIND_FLOAT_POS_INF=18 / KIND_FLOAT_NEG_INF=19 (kind byte
            // at index 9, no payload). `op_box_float` canonicalizes NaN, so a non-finite is not bit-round-
            // tripped here — only its dedicated kind byte is checked. (Was `is_none()`/"declines" pre-#7479.)
            let doc = doc.expect("a non-finite float encodes its dedicated word-form leaf");
            let expected_kind = if v.is_nan() {
                doc::KIND_FLOAT_NAN
            } else if v.is_sign_positive() {
                doc::KIND_FLOAT_POS_INF
            } else {
                doc::KIND_FLOAT_NEG_INF
            };
            assert_eq!(
                doc[9], expected_kind,
                "non-finite f64 {v} (bits {bits:#018x}) → its dedicated payloadless kind byte"
            );
        }
        op_drop(h);
        assert_eq!(live_nodes(), 0, "no leak for bits {bits:#018x}");
    });
}

#[test]
fn prop_float32_leaf_round_trips_bit_exact_under_random_f32() {
    bolero::check!().with_type::<u32>().for_each(|&bits| {
        reset();
        let v = f32::from_bits(bits);
        let desc: &[u8] = &[0x01, 0x0e, 0x00]; // [0]=Float32(tag 14), root=0
        let h = op_box_float32(v);
        let doc = op_value_encode_form(h, desc);
        if v.is_finite() {
            let doc = doc.expect("a finite f32 must encode");
            let decimal = float_doc_to_decimal(&doc);
            // Parse the decimal back AS AN f32 — the value form is the f32's own shortest decimal, so
            // it must reconstruct the exact f32 bits (a promoted-f64 decimal would NOT).
            let reconstructed: f32 = decimal
                .parse()
                .expect("the KIND_FLOAT decimal must parse as f32");
            assert_eq!(
                reconstructed.to_bits(),
                v.to_bits(),
                "f32 {v} (bits {bits:#010x}) must round-trip through its decimal {decimal}"
            );
        } else {
            // A non-finite f32 ENCODES via its dedicated payloadless word-form leaf (#7479), same as f64:
            // KIND_FLOAT_NAN=17 / KIND_FLOAT_POS_INF=18 / KIND_FLOAT_NEG_INF=19 (kind byte at index 9).
            // (Was `is_none()`/"declines" pre-#7479.)
            let doc = doc.expect("a non-finite f32 encodes its dedicated word-form leaf");
            let expected_kind = if v.is_nan() {
                doc::KIND_FLOAT_NAN
            } else if v.is_sign_positive() {
                doc::KIND_FLOAT_POS_INF
            } else {
                doc::KIND_FLOAT_NEG_INF
            };
            assert_eq!(
                doc[9], expected_kind,
                "non-finite f32 {v} (bits {bits:#010x}) → its dedicated payloadless kind byte"
            );
        }
        op_drop(h);
        assert_eq!(live_nodes(), 0, "no leak for f32 bits {bits:#010x}");
    });
}

#[test]
fn box_float_canonicalizes_nan_to_one_byte_form() {
    reset();
    let before = live_nodes();

    // Two DIFFERENT NaN bit patterns (a signaling-ish NaN with payload, and a sign-bit-set NaN).
    let nan_a = f64::from_bits(0x7ff8_0000_0000_0001); // quiet NaN, payload 1
    let nan_b = f64::from_bits(0xfff8_0000_dead_beef); // sign bit + different payload
    assert!(nan_a.is_nan() && nan_b.is_nan());
    assert_ne!(
        nan_a.to_bits(),
        nan_b.to_bits(),
        "the two source NaNs differ in raw bits"
    );

    let a = op_box_float(nan_a);
    let b = op_box_float(nan_b);
    // Both stored as the canonical NaN → byte-identical leaves → champ_eq true, champ_hash equal.
    assert_eq!(
        op_get_float(a).to_bits(),
        f64::NAN.to_bits(),
        "a boxed NaN reads back as the canonical quiet NaN"
    );
    assert_eq!(op_get_float(b).to_bits(), f64::NAN.to_bits());
    assert!(
        champ_eq(a, b),
        "two NaN values are structurally EQUAL (one canonical form)"
    );
    assert_eq!(
        champ_hash(a),
        champ_hash(b),
        "…and hash identically (so they are the SAME map key)"
    );
    // A NaN also equals the canonical `f64::NAN` produced the ordinary way.
    let c = op_box_float(f64::NAN);
    assert!(champ_eq(a, c) && champ_hash(a) == champ_hash(c));
    op_drop(a);
    op_drop(b);
    op_drop(c);

    // -0.0 and 0.0 keep their DISTINCT byte forms (only NaN is collapsed): NOT equal, NOT same key.
    let zpos = op_box_float(0.0);
    let zneg = op_box_float(-0.0);
    assert_eq!(
        op_get_float(zneg).to_bits(),
        (-0.0f64).to_bits(),
        "-0.0 keeps its sign bit"
    );
    assert!(
        !champ_eq(zpos, zneg),
        "-0.0 ≠ 0.0 (distinct canonical byte forms)"
    );
    op_drop(zpos);
    op_drop(zneg);

    // ±inf keep their bits too (finite-check is is_nan, not is_finite).
    let inf = op_box_float(f64::INFINITY);
    let ninf = op_box_float(f64::NEG_INFINITY);
    assert_eq!(
        op_get_float(inf).to_bits(),
        f64::INFINITY.to_bits(),
        "inf unchanged"
    );
    assert!(!champ_eq(inf, ninf), "+inf ≠ -inf");
    op_drop(inf);
    op_drop(ninf);

    assert_eq!(live_nodes(), before, "no leak: every float value dropped");
}

#[test]
fn nan_in_a_compound_is_value_eq_across_bit_patterns() {
    reset();
    let before = live_nodes();
    let nan_a = f64::from_bits(0x7ff8_0000_0000_0001);
    let nan_b = f64::from_bits(0xfff8_0000_dead_beef);
    // (nan_a, 5) and (nan_b, 5) — different NaN bits, same int → structurally EQUAL after canon.
    let mk = |nan: f64| -> Handle {
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, op_box_float(nan));
        op_arr_set(t, 1, op_box_int(5));
        t
    };
    let ta = mk(nan_a);
    let tb = mk(nan_b);
    assert!(
        champ_eq(ta, tb),
        "two tuples carrying a NaN are value-eq (canonical NaN byte form)"
    );
    assert_eq!(
        champ_hash(ta),
        champ_hash(tb),
        "…and hash identically (same compound map key)"
    );
    op_drop(ta);
    op_drop(tb);

    // A tuple with -0.0 differs from one with 0.0 (the -0.0/0.0 forms genuinely differ).
    let mkz = |z: f64| -> Handle {
        let t = op_arr_alloc(2);
        op_arr_set(t, 0, op_box_float(z));
        op_arr_set(t, 1, op_box_int(5));
        t
    };
    let tzp = mkz(0.0);
    let tzn = mkz(-0.0);
    assert!(
        !champ_eq(tzp, tzn),
        "(−0.0, 5) ≠ (0.0, 5) — distinct canonical byte forms"
    );
    op_drop(tzp);
    op_drop(tzn);

    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_encode_renders_a_set_in_canonical_order() {
    reset();
    let before = live_nodes();
    // Descriptor: table [0] = Int, [1] = Set(→0), root = 1. Tag 12 = Set.
    let desc: &[u8] = &[0x02, 0x00, 0x0c, 0x00, 0x01]; // len 2, [0]=Int(0), [1]=Set(tag12, elem 0), root 1

    // Insert ints in NON-sorted order incl. values whose LITTLE-ENDIAN bytes disagree with numeric
    // order (1 vs 256): 256, 1, 3, 2 → canonical numeric order is 1, 2, 3, 256.
    let mut s = op_set_empty();
    for &v in &[256i64, 1, 3, 2] {
        s = op_set_insert(s, op_box_int(v));
    }
    let doc = op_value_encode_form(s, desc).expect("encode a Set");

    // Differential: the recursive oracle must produce byte-identical output.
    let descriptor = decode_descriptor(desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, s, descriptor.root, 0).expect("recursive");
    let rec_doc = b.finish(root);
    assert_eq!(
        doc, rec_doc,
        "iterative and recursive Set encode must agree"
    );

    // The document must decode to `(Set.of (list 1 2 3 256))` — the ints in NUMERIC order. Rather than
    // hand-derive every byte, assert the int magnitudes appear in ascending order in the leaf pool: the
    // KIND_INT leaves carry big-endian magnitudes [1],[2],[3],[1,0](=256) in that sequence.
    // Find the four int-leaf magnitudes in emission order (KIND_INT_POS_DEC = 0).
    let mut mags: Vec<Vec<u8>> = Vec::new();
    let mut i = 9; // after 8-byte header + 1-byte leaf_count
    // leaf_count is at [8]; parse that many leaves.
    let leaf_count = doc[8] as usize;
    for _ in 0..leaf_count {
        let kind = doc[i];
        i += 1;
        if kind == 0 {
            // KIND_INT_POS_DEC: LEB len + magnitude
            let len = doc[i] as usize;
            i += 1;
            mags.push(doc[i..i + len].to_vec());
            i += len;
        } else if kind == 10 {
            // KIND_NAME: LEB len + bytes
            let len = doc[i] as usize;
            i += 1 + len;
        } else if (20..=26).contains(&kind) {
            // M2 ctor-head leaf (LIST/TUPLE/RECORD/MAP/SET_CTOR, FIELD_PAIR, MEMBER) — payloadless.
        } else {
            panic!("unexpected leaf kind {kind} in a Set-of-Int document");
        }
    }
    assert_eq!(
        mags,
        vec![vec![1u8], vec![2u8], vec![3u8], vec![1u8, 0u8]],
        "the int leaves appear in NUMERIC order 1,2,3,256 (256 = big-endian magnitude [1,0]), NOT the \
         CHAMP hash order or little-endian byte order the elements were inserted in"
    );
    op_drop(s);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_encode_renders_a_string_set_in_lexicographic_order() {
    reset();
    let before = live_nodes();
    // desc: [0]=Str(tag 3), [1]=Set(tag 12, elem→0), root=1.
    let desc: &[u8] = &[0x02, 0x03, 0x0c, 0x00, 0x01];
    // Insert out of lexicographic order, incl. the empty string + a shared "foo"/"foobar" prefix.
    let mut s = op_set_empty();
    for e in &["foo", "bar", "baz", "", "foobar"] {
        s = op_set_insert(s, op_str_new((*e).to_string()));
    }
    let doc = op_value_encode_form(s, desc).expect("encode a Set String");
    // Differential: the recursive oracle must produce byte-identical output.
    let descriptor = decode_descriptor(desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, s, descriptor.root, 0).expect("recursive");
    assert_eq!(
        doc,
        b.finish(root),
        "iterative and recursive Set String encode must agree"
    );
    // Decode the KIND_STR (kind 7) leaves in emission order — the elements in canonical order.
    let mut strs: Vec<String> = Vec::new();
    let leaf_count = doc[8] as usize;
    let mut i = 9;
    for _ in 0..leaf_count {
        let kind = doc[i];
        i += 1;
        match kind {
            7 => {
                // KIND_STR: LEB len + UTF-8 bytes.
                let len = doc[i] as usize;
                i += 1;
                strs.push(String::from_utf8(doc[i..i + len].to_vec()).expect("utf8"));
                i += len;
            }
            10 => {
                // KIND_NAME (`list`/`.`/`Set`/`of` heads): skip.
                let len = doc[i] as usize;
                i += 1 + len;
            }
            20..=26 => {} // M2 payloadless ctor-head leaf (20-26)
            other => panic!("unexpected leaf kind {other} in a Set-of-String document"),
        }
    }
    assert_eq!(
        strs,
        vec![
            "".to_string(),
            "bar".to_string(),
            "baz".to_string(),
            "foo".to_string(),
            "foobar".to_string(),
        ],
        "a Set String renders its elements in LEXICOGRAPHIC byte order (empty first, \"foo\" before \
         its extension \"foobar\"), NOT the CHAMP hash order they were inserted/stored in"
    );
    op_drop(s);
    assert_eq!(live_nodes(), before, "no leak across the Set String encode");
}

#[test]
fn value_encode_renders_a_map_in_canonical_key_order() {
    reset();
    let before = live_nodes();
    // Descriptor: [0]=Int (key), [1]=Int (val), [2]=Map(→0,→1), root = 2. Tag 13 = Map.
    let desc: &[u8] = &[0x03, 0x00, 0x00, 0x0d, 0x00, 0x01, 0x02]; // len 3; [0]Int [1]Int [2]Map(k0,v1); root 2

    // Insert keys out of numeric order incl. 256 (whose LE bytes disagree with numeric order vs 1).
    // Value = key * 10 so the pairing is checkable: (1 10)(2 20)(3 30)(256 2560).
    let mut m = op_map_empty();
    for &k in &[256i64, 1, 3, 2] {
        m = op_map_insert(m, op_box_int(k), op_box_int(k * 10));
    }
    let doc = op_value_encode_form(m, desc).expect("encode a Map");

    // Differential: the recursive oracle must produce byte-identical output.
    let descriptor = decode_descriptor(desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, m, descriptor.root, 0).expect("recursive");
    let rec_doc = b.finish(root);
    assert_eq!(
        doc, rec_doc,
        "iterative and recursive Map encode must agree"
    );

    // Collect the int-leaf magnitudes in emission order — they are the keys+values interleaved in
    // canonical KEY order: k1 v1 k2 v2 … = 1 10 2 20 3 30 256 2560.
    // 10=[0x0a], 20=[0x14], 30=[0x1e], 256=[1,0], 2560=[0x0a,0x00].
    let mut mags: Vec<Vec<u8>> = Vec::new();
    let leaf_count = doc[8] as usize;
    let mut i = 9;
    for _ in 0..leaf_count {
        let kind = doc[i];
        i += 1;
        if kind == 0 {
            let len = doc[i] as usize;
            i += 1;
            mags.push(doc[i..i + len].to_vec());
            i += len;
        } else if kind == 10 {
            let len = doc[i] as usize;
            i += 1 + len;
        } else if (20..=26).contains(&kind) {
            // M2 payloadless ctor-head leaf (20-26)
        } else {
            panic!("unexpected leaf kind {kind} in a Map-of-Int document");
        }
    }
    assert_eq!(
        mags,
        vec![
            vec![1u8],
            vec![0x0au8], // (1 10)
            vec![2u8],
            vec![0x14u8], // (2 20)
            vec![3u8],
            vec![0x1eu8], // (3 30)
            vec![1u8, 0u8],
            vec![0x0au8, 0u8], // (256 2560)
        ],
        "entries appear in NUMERIC KEY order 1,2,3,256 (256=big-endian [1,0]); each key paired with \
         its value (key*10) — NOT the CHAMP hash order the keys were inserted in"
    );
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_encode_renders_negative_int_keys_in_numeric_not_byte_order() {
    reset();
    let before = live_nodes();

    // Decode the int leaves of a document (KIND_INT: 0 positive / 3 negative, LEB len + BE magnitude;
    // KIND_NAME=10 skipped) into their SIGNED values, in emission order.
    fn int_leaves_signed(doc: &[u8]) -> Vec<i64> {
        let mut vals = Vec::new();
        let leaf_count = doc[8] as usize;
        let mut i = 9usize;
        for _ in 0..leaf_count {
            let kind = doc[i];
            i += 1;
            match kind {
                0 | 3 => {
                    let len = doc[i] as usize;
                    i += 1;
                    let mut m: i64 = 0;
                    for &b in &doc[i..i + len] {
                        m = (m << 8) | (b as i64);
                    }
                    i += len;
                    vals.push(if kind == 3 { -m } else { m });
                }
                10 => {
                    let len = doc[i] as usize;
                    i += 1 + len;
                }
                20..=26 => {} // M2 payloadless ctor-head leaf (20-26)
                other => panic!("unexpected leaf kind {other} in an int document"),
            }
        }
        vals
    }

    // (A) a SET of mixed negative/positive ints, inserted in scrambled order.
    let set_desc: &[u8] = &[0x02, 0x00, 0x0c, 0x00, 0x01]; // [0]=Int [1]=Set(elem0) root1
    let mut s = op_set_empty();
    for &v in &[3i64, -5, 0, -1, 2, -128, 127] {
        s = op_set_insert(s, op_box_int(v));
    }
    let sdoc = op_value_encode_form(s, set_desc).expect("encode a Set of signed ints");
    // Differential: the recursive oracle agrees byte-for-byte.
    let sdescr = decode_descriptor(set_desc).expect("set descriptor");
    let mut sb = DocBuilder::default();
    let sroot = encode_value_recursive(&sdescr, &mut sb, s, sdescr.root, 0).expect("recursive set");
    assert_eq!(
        sdoc,
        sb.finish(sroot),
        "iterative and recursive Set encode agree (signed)"
    );
    assert_eq!(
        int_leaves_signed(&sdoc),
        vec![-128, -5, -1, 0, 2, 3, 127],
        "a Set of signed ints renders in NUMERIC order (negatives BEFORE positives), NOT the raw \
         little-endian byte order champ_key_cmp uses (which would sort negatives last)"
    );
    op_drop(s);

    // (B) a MAP with signed keys, value = key so the pairing is checkable in the interleaved leaves.
    let map_desc: &[u8] = &[0x03, 0x00, 0x00, 0x0d, 0x00, 0x01, 0x02]; // [0]Int [1]Int [2]Map(k0,v1) root2
    let mut m = op_map_empty();
    for &k in &[3i64, -5, 0, -1, 2] {
        m = op_map_insert(m, op_box_int(k), op_box_int(k));
    }
    let mdoc = op_value_encode_form(m, map_desc).expect("encode a Map of signed keys");
    let mdescr = decode_descriptor(map_desc).expect("map descriptor");
    let mut mb = DocBuilder::default();
    let mroot = encode_value_recursive(&mdescr, &mut mb, m, mdescr.root, 0).expect("recursive map");
    assert_eq!(
        mdoc,
        mb.finish(mroot),
        "iterative and recursive Map encode agree (signed)"
    );
    // key,value interleaved in canonical KEY order; value == key here, so each key appears twice.
    assert_eq!(
        int_leaves_signed(&mdoc),
        vec![-5, -5, -1, -1, 0, 0, 2, 2, 3, 3],
        "a Map with signed keys renders entries in NUMERIC KEY order (negatives first)"
    );
    op_drop(m);

    assert_eq!(
        live_nodes(),
        before,
        "no leak across the signed-key set/map encodes"
    );
}

#[test]
fn value_encode_map_of_lists_walks_the_nested_value() {
    reset();
    let before = live_nodes();
    // Descriptor: [0]=Int (key), [1]=Int (list elem), [2]=List(elem→1), [3]=Map(key→0, val→2); root=3.
    // Tags: 0=Int, 7=List, 13=Map.
    let desc: &[u8] = &[
        0x04, // table_len = 4
        0x00, // [0] Int
        0x00, // [1] Int
        0x07, 0x01, // [2] List(elem → 1)
        0x0d, 0x00, 0x02, // [3] Map(key → 0, val → 2)
        0x03, // root = 3
    ];
    // Build { 2: [20,21], 1: [10], 3: [] } — inserted OUT of key order (so the canonical sort reorders),
    // an EMPTY value list (the zero-element assembler under a map value), and multi-element lists.
    let mk_list = |elems: &[i64]| -> Handle {
        let mut v = op_vec_empty();
        for &e in elems {
            v = op_vec_push(v, op_box_int(e));
        }
        v
    };
    let mut m = op_map_empty();
    m = op_map_insert(m, op_box_int(2), mk_list(&[20, 21]));
    m = op_map_insert(m, op_box_int(1), mk_list(&[10]));
    m = op_map_insert(m, op_box_int(3), mk_list(&[]));
    let doc = op_value_encode_form(m, desc).expect("encode a Map of Lists");

    // Differential: the recursive oracle (its S::Map recurses the value shape) must agree byte-for-byte.
    let descriptor = decode_descriptor(desc).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, m, descriptor.root, 0).expect("recursive");
    assert_eq!(
        doc,
        b.finish(root),
        "iterative and recursive Map-of-Lists encode must agree"
    );

    // The int leaves, in emission order, are the keys+values interleaved in canonical KEY order:
    // (1 [10]) (2 [20 21]) (3 []) → 1, 10, 2, 20, 21, 3. (Empty list contributes no int leaf.)
    let mut ints: Vec<i64> = Vec::new();
    let leaf_count = doc[8] as usize;
    let mut i = 9;
    for _ in 0..leaf_count {
        let kind = doc[i];
        i += 1;
        if kind == 0 {
            let len = doc[i] as usize;
            i += 1;
            let mut v = 0i64;
            for &byte in &doc[i..i + len] {
                v = (v << 8) | byte as i64;
            }
            ints.push(v);
            i += len;
        } else if kind == 10 {
            let len = doc[i] as usize;
            i += 1 + len;
        } else if (20..=26).contains(&kind) {
            // M2 payloadless ctor-head leaf (20-26)
        } else {
            panic!("unexpected leaf kind {kind} in a Map-of-Lists document");
        }
    }
    assert_eq!(
        ints,
        vec![1, 10, 2, 20, 21, 3],
        "keys in numeric order, each followed by its value list's elements (empty list adds none)"
    );
    op_drop(m);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_encode_empty_collections() {
    reset();
    let before = live_nodes();

    // Helper: encode `v` under descriptor `d`, assert iterative==recursive oracle, return the doc.
    let check = |v: Handle, d: &[u8]| -> Vec<u8> {
        let doc = op_value_encode_form(v, d).expect("encode empty collection");
        let descriptor = decode_descriptor(d).expect("descriptor");
        let mut b = DocBuilder::default();
        let root =
            encode_value_recursive(&descriptor, &mut b, v, descriptor.root, 0).expect("recursive");
        assert_eq!(
            doc,
            b.finish(root),
            "iterative and recursive empty-collection encode must agree"
        );
        doc
    };

    // Empty SET → M2 `(Ctor(Set))`. desc: [0]=Int, [1]=Set(→0), root=1.
    let es = op_set_empty();
    let sd: &[u8] = &[0x02, 0x00, 0x0c, 0x00, 0x01];
    let sdoc = check(es, sd);
    // ONE leaf — the payloadless Set ctor head (kind 24); no `list`/`Set` name leaves anymore.
    assert_eq!(
        sdoc[8], 1,
        "empty set document has ONE leaf (the Set ctor head)"
    );
    assert_eq!(
        sdoc[9],
        doc::KIND_SET_CTOR,
        "empty set renders the bare Ctor(Set) head"
    );
    op_drop(es);

    // Empty MAP → M2 `(Ctor(Map))`. desc: [0]=Int, [1]=Int, [2]=Map(→0,→1), root=2.
    let em = op_map_empty();
    let md: &[u8] = &[0x03, 0x00, 0x00, 0x0d, 0x00, 0x01, 0x02];
    let mdoc = check(em, md);
    assert_eq!(
        mdoc[8], 1,
        "empty map document has ONE leaf (the Map ctor head)"
    );
    assert_eq!(
        mdoc[9],
        doc::KIND_MAP_CTOR,
        "empty map renders the bare Ctor(Map) head"
    );
    op_drop(em);

    // Empty LIST → M2 `(Ctor(List))`. desc: [0]=Int, [1]=List(→0), root=1.
    let el = op_vec_empty();
    let ld: &[u8] = &[0x02, 0x00, 0x07, 0x00, 0x01];
    let ldoc = check(el, ld);
    assert_eq!(
        ldoc[8], 1,
        "empty list document has ONE leaf (the List ctor head)"
    );
    assert_eq!(
        ldoc[9],
        doc::KIND_LIST_CTOR,
        "empty list renders the bare Ctor(List) head"
    );
    op_drop(el);

    assert_eq!(
        live_nodes(),
        before,
        "no leak: every empty collection dropped"
    );
}

#[test]
fn value_encode_multi_payload_variant_escapes_flat_via_spread() {
    reset();
    let before = live_nodes();
    // Tree = (Node Int64 Tree Tree) | Leaf. Descriptor:
    //   [0] Int; [1] Unit (Leaf payload); [2] Sum[(Node→3),(Leaf→1)]; [3] Spread[0,2,2] (Node's
    //   payload: Int, Tree, Tree — the two Tree elements Ref back to [2]); root=2.
    let mut d: Vec<u8> = Vec::new();
    let leb = |out: &mut Vec<u8>, v: u64| {
        let mut v = v;
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    };
    let name = |out: &mut Vec<u8>, s: &str| {
        let mut n = s.len() as u64;
        loop {
            let mut b = (n & 0x7f) as u8;
            n >>= 7;
            if n != 0 {
                b |= 0x80;
            }
            out.push(b);
            if n == 0 {
                break;
            }
        }
        out.extend_from_slice(s.as_bytes());
    };
    leb(&mut d, 4); // table_len
    d.push(0); // [0] Int
    d.push(5); // [1] Unit
    d.push(9); // [2] Sum
    leb(&mut d, 2); // 2 variants
    name(&mut d, "Node");
    leb(&mut d, 3); // Node → [3]
    name(&mut d, "Leaf");
    leb(&mut d, 1); // Leaf → [1] (Unit)
    d.push(16); // [3] Spread
    leb(&mut d, 3); // 3 elements
    leb(&mut d, 0); // Int
    leb(&mut d, 2); // Tree (Ref to the Sum)
    leb(&mut d, 2); // Tree
    leb(&mut d, 2); // root = [2]

    // Build `Node(1, Leaf, Leaf)`. A Leaf = sum disc 1, unit payload; a Node = sum disc 0, payload =
    // a 3-tuple arr [box_int(1), leaf, leaf].
    let leaf = || op_sum_new(1, op_arr_alloc(0));
    let payload = op_arr_alloc(3);
    op_arr_set(payload, 0, op_box_int(1));
    op_arr_set(payload, 1, leaf());
    op_arr_set(payload, 2, leaf());
    let node = op_sum_new(0, payload);

    let doc = op_value_encode_form(node, &d).expect("encode a multi-payload variant");
    // Differential: iterative == recursive oracle byte-for-byte.
    let descriptor = decode_descriptor(&d).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, node, descriptor.root, 0).expect("recursive");
    assert_eq!(
        doc,
        b.finish(root),
        "iterative and recursive Spread encode must agree"
    );

    // FLAT structure: the name leaves in walk order are `Node`, then `Leaf`, `Leaf` (the two children;
    // `unit` for each Leaf's payload). Crucially NO `tuple` name — the Int + two Trees are spliced
    // directly under `Node`, not wrapped. Collect names + assert `tuple` is absent, `Node`/`Leaf`/`unit` present.
    let mut names: Vec<String> = Vec::new();
    let leaf_count = doc[8] as usize;
    let mut i = 9;
    for _ in 0..leaf_count {
        let kind = doc[i];
        i += 1;
        match kind {
            0 => {
                let len = doc[i] as usize;
                i += 1 + len;
            }
            10 => {
                let len = doc[i] as usize;
                i += 1;
                names.push(String::from_utf8(doc[i..i + len].to_vec()).unwrap());
                i += len;
            }
            20..=26 => {} // M2 payloadless ctor-head leaf (20-26)
            k => panic!("unexpected leaf kind {k} in a Spread document"),
        }
    }
    assert!(
        !names.iter().any(|n| n == "tuple"),
        "multi-payload variant is FLAT — no `tuple` wrapper, got {names:?}"
    );
    assert!(names.iter().any(|n| n == "Node"), "`Node` head present");
    assert!(names.iter().any(|n| n == "Leaf"), "`Leaf` children present");
    assert!(
        names.iter().any(|n| n == "unit"),
        "each Leaf's unit payload present"
    );
    op_drop(node);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_encode_renders_a_framed_parametric_type() {
    reset();
    let before = live_nodes();
    // Descriptor: [0]=Int, [1]=List(→0), [2]=Framed("List", ["Int64"], inner→1). root=2.
    // Bytes: table_len=3; [0]=Int(tag0); [1]=List(tag7, elem 0); [2]=Framed(tag15, "List", 1 arg
    // "Int64", inner 1); root=2.
    let mut d: Vec<u8> = Vec::new();
    let leb = |out: &mut Vec<u8>, v: u64| {
        let mut v = v;
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    };
    let name = |out: &mut Vec<u8>, s: &str| {
        let mut n = s.len() as u64;
        loop {
            let mut b = (n & 0x7f) as u8;
            n >>= 7;
            if n != 0 {
                b |= 0x80;
            }
            out.push(b);
            if n == 0 {
                break;
            }
        }
        out.extend_from_slice(s.as_bytes());
    };
    leb(&mut d, 3); // table_len
    d.push(0); // [0] Int
    d.push(7); // [1] List
    leb(&mut d, 0); // elem → 0
    d.push(15); // [2] Framed
    // The type node is now a RECURSIVE `TypeNode` (`08d4a99a`): every node — INCLUDING a leaf — declares
    // its own child count, so a nested type like `(List Int64)` is `List{ Int64{} }`. `Int64` therefore
    // needs an explicit `n_children = 0` before the `inner` index (the old flat `[head][n_args](arg)*n`
    // wire had no per-arg child count — that stale layout desynced the recursive decoder).
    name(&mut d, "List"); // head
    leb(&mut d, 1); // List n_children = 1
    name(&mut d, "Int64"); // child[0] head
    leb(&mut d, 0); // Int64 n_children = 0 (a leaf type node)
    leb(&mut d, 1); // inner → 1
    leb(&mut d, 2); // root

    // A real RUNTIME list value (RRB vec, NOT an arr — the Shape::List arm reads via vec-len/vec-get).
    let mut v = op_vec_empty();
    for i in 1..=3i64 {
        v = op_vec_push(v, op_box_int(i));
    }
    let doc = op_value_encode_form(v, &d).expect("encode a Framed(list)");

    // Differential: the recursive oracle must produce byte-identical output.
    let descriptor = decode_descriptor(&d).expect("descriptor");
    let mut b = DocBuilder::default();
    let root =
        encode_value_recursive(&descriptor, &mut b, v, descriptor.root, 0).expect("recursive");
    assert_eq!(
        doc,
        b.finish(root),
        "iterative and recursive Framed encode must agree"
    );

    // The document's NAME leaves — the `(: (list 1 2 3) (List Int64))` frame emits, in walk order,
    // the names `:`, `list`, then the type head/args `List`, `Int64` (ints are KIND_INT leaves, not
    // names). Collect the name-leaf strings in emission order and check the frame's names are present.
    let mut names: Vec<String> = Vec::new();
    let leaf_count = doc[8] as usize;
    let mut i = 9;
    for _ in 0..leaf_count {
        let kind = doc[i];
        i += 1;
        match kind {
            0 => {
                // KIND_INT_POS_DEC: LEB len + magnitude
                let len = doc[i] as usize;
                i += 1 + len;
            }
            10 => {
                // KIND_NAME: LEB len + utf8
                let len = doc[i] as usize;
                i += 1;
                names.push(String::from_utf8(doc[i..i + len].to_vec()).unwrap());
                i += len;
            }
            20..=26 => {} // M2 payloadless ctor-head leaf (20-26)
            k => panic!("unexpected leaf kind {k} in a Framed(list) document"),
        }
    }
    // The frame's NAME leaves, in walk order: `:` (the outer frame head), then `List` + `Int64` (the
    // type node, emitted AFTER the value). The value's list head is now the M2 Ctor(List) LEAF (not the
    // `list` name), so it no longer appears among the name leaves.
    let names_ref: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        names_ref,
        alloc::vec![":", "List", "Int64"],
        "Framed frame emits `:`, then the type head `List` + arg `Int64` (value list head is a ctor leaf)"
    );
    op_drop(v);
    assert_eq!(live_nodes(), before, "no leak");
}

#[test]
fn value_encode_large_multilevel_vec_list_matches_and_has_correct_elements() {
    reset();
    let before = live_nodes();
    // Descriptor: table [Int(0), List(→0)], root=1 — a bare `(List Int)`.
    let mut d: Vec<u8> = Vec::new();
    let leb = |out: &mut Vec<u8>, mut v: u64| loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    };
    leb(&mut d, 2); // table_len
    d.push(0); // [0] Int
    d.push(7); // [1] List
    leb(&mut d, 0); // elem → 0
    leb(&mut d, 1); // root = 1
    let descriptor = decode_descriptor(&d).expect("descriptor decodes");

    for n in [31i64, 32, 33, 64, 100] {
        // Build a REAL RRB vec of `1..=n` (values chosen so each KIND_INT magnitude is a single byte
        // == the value, for a clean content check; all < 128 so one big-endian byte).
        let mut v = op_vec_empty();
        for i in 1..=n {
            v = op_vec_push(v, op_box_int(i));
        }
        assert_eq!(op_vec_len(v) as i64, n, "built a real vec of {n} elements");

        let doc = op_value_encode_form(v, &d).expect("encode the large list");

        // (1) DIFFERENTIAL: the recursive oracle must produce byte-identical output.
        let mut b = DocBuilder::default();
        let root = encode_value_recursive(&descriptor, &mut b, v, descriptor.root, 0)
            .expect("recursive oracle encodes");
        assert_eq!(
            doc,
            b.finish(root),
            "n={n}: iterative and recursive Shape::List encode must agree over a multi-level vec"
        );

        // (2) CONTENT: the KIND_INT leaves, in emission order, must be exactly [1,2,…,n] (single-byte
        // magnitudes). This is INDEPENDENT of `vec-get` (it reads the serialized bytes), so a shared
        // trie-descent bug that fooled the differential is caught here.
        let mut ints: Vec<u8> = Vec::new();
        let leaf_count = doc[8] as usize;
        let mut i = 9;
        for _ in 0..leaf_count {
            let kind = doc[i];
            i += 1;
            match kind {
                0 => {
                    // KIND_INT_POS_DEC: LEB len + big-endian magnitude (single byte for 1..=100).
                    let len = doc[i] as usize;
                    i += 1;
                    assert_eq!(
                        len, 1,
                        "n={n}: each element 1..=100 is a single magnitude byte"
                    );
                    ints.push(doc[i]);
                    i += len;
                }
                10 => {
                    // KIND_NAME (the `list` head): LEB len + bytes.
                    let len = doc[i] as usize;
                    i += 1 + len;
                }
                20..=26 => {} // M2 payloadless ctor-head leaf (20-26)
                k => panic!("n={n}: unexpected leaf kind {k} in a (List Int) document"),
            }
        }
        let want: Vec<u8> = (1..=n as u8).collect();
        assert_eq!(
            ints, want,
            "n={n}: the encoded ints are exactly 1..={n} IN ORDER — op_vec_get descends the multi-level \
             trie correctly at every index (a boundary-crossing descent bug would reorder/drop elements)"
        );

        op_drop(v);
    }
    assert_eq!(live_nodes(), before, "no leak across the large-vec encodes");
}

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
    assert!(
        insert <= 900,
        "unique map_insert x{N} allocs {insert} exceeds ceiling 900 (… → 1084 in-place SPLIT → ~766 inline champ_header raw; residual = the intrinsic subnode Box + handles Vec on a split)"
    );

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
    assert!(
        pinsert <= 6400,
        "shared/persistent map_insert (overwrite) x{N} allocs {pinsert} exceeds ceiling 6400 (path-copy: 1 Vec + 1 node Box per copied spine node; was 8715 with a wasted upfront handles.clone())"
    );
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
    assert!(
        pinsert_new <= 6100,
        "shared/persistent map_insert (new key) x{N} allocs {pinsert_new} exceeds ceiling 6100 (path-copy growth: borrow-and-build, no upfront clone; measured ~6016 after the SPLIT branch splices subnodes directly into `nh` — no transient `subs` Vec (was ~6418), 7445 before the no-upfront-clone. Overwrite-only shared insert (no splits) is unaffected)"
    );
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
    assert!(
        premove <= 7000,
        "shared/persistent map_remove x{N} allocs {premove} exceeds ceiling 7000 (path-copy: 1 Vec + 1 node Box per copied spine node; was 9277 with a wasted upfront handles.clone())"
    );
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
    assert!(
        iterate <= 50,
        "unique map_iterate x{N} allocs {iterate} exceeds ceiling 50 (5248 → 2248 → 1126 → ~3 inline Slots buffer, iteration is now essentially alloc-free — only the initial cursor's frames Vec)"
    );
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
    assert!(
        remove <= 50,
        "unique map_remove x{N} allocs {remove} exceeds ceiling 50 (8397 → 5207 → 2953 → 1953 → 954 in-place drain → ~0 in-place COLLAPSE + inline collapse_candidate; remove is now allocation-FREE)"
    );
    op_drop(m2);

    // (C2) vec-drop FOLD pattern — a `(match xs ((list x .. rest) …))` list fold binds `rest` =
    // `vec-drop(v, 1)` each iteration (the natural `sum`, now a constant-stack loop). `vec-drop` builds
    // ONLY the kept tail spine (`op_vec_drop_tail`), NOT the discarded left prefix a `split`+drop-left
    // would materialize-then-free — halving the per-step allocation (13925 → ~7000 over the walk, ~7/
    // elem = the kept-tail spine rebuild; the residual O(log N)/step is inherent to RRB drop-front, which
    // only a `vec-iter` CURSOR eliminates — a compiler-coordinated follow-up). Walk a fresh N-elem vec.
    {
        let mut fv = op_vec_empty();
        for k in 0..N as i64 {
            fv = op_vec_push(fv, op_box_int(k));
        }
        let fold = measure(&mut || {
            op_dup(fv); // keep fv; the loop consumes the dup'd chain
            let mut cur = fv;
            while op_vec_len(cur) > 0 {
                cur = op_vec_drop_tail(cur, 1);
            }
        });
        println!("ALLOC vec_drop_fold x{N}: {fold}");
        assert!(
            fold <= 6200,
            "vec_drop_fold x{N} allocs {fold} exceeds ceiling 6200 (~6/elem: the kept-tail spine only, ONE header per step; was 6994 before the vector header carried its root handle INLINE (no per-header Vec), 13925 before that via split+drop-left building the discarded prefix. A regression to the split path would ~2x; re-adding the header Vec adds ~1/elem; a vec-iter cursor would cut to ~O(1)/elem)"
        );
        op_drop(fv);
    }

    // (D) vec push (unique, FBIP) — the in-place RRB reference: near-zero amortized.
    let mut v = op_vec_empty();
    let push = measure(&mut || {
        for k in 0..N {
            v = op_vec_push(v, op_box_int(k));
        }
    });
    println!("ALLOC vec_push x{N}: {push}");
    assert!(
        push <= 400,
        "unique vec_push x{N} allocs {push} exceeds ceiling 400"
    );
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
    assert_eq!(
        vupd, 0,
        "vec_update on a uniquely-owned vec is FBIP in-place — zero allocations"
    );
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
    assert!(
        vupd_shared <= 6100,
        "shared/persistent vec_update x{N} allocs {vupd_shared} exceeds ceiling 6100 (RRB path-copy floor: borrow-and-build, ~2 allocs per copied spine node + ONE inline-handle header; was 6968 before the header carried its root inline — re-adding the header Vec would climb back ~1000)"
    );
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
    assert!(
        vpush_shared <= 6100,
        "shared/persistent vec_push x{N} allocs {vpush_shared} exceeds ceiling 6100 (RRB path-copy floor: borrow-and-build rightmost spine + ONE inline-handle header; was 7000 before the header carried its root inline — re-adding the header Vec would climb back ~1000)"
    );
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
    assert!(
        concat_allocs <= 1300,
        "vec_concat x100 allocs {concat_allocs} exceeds ceiling 1300 (O(log N) rebalance: ≤a few nodes/op + ONE inline-handle result header, N-independent; was 1200 before the header carried its root inline — a regression to element-copy would scale with N)"
    );
    let split_allocs = measure(&mut || {
        for _ in 0..100 {
            op_dup(ca);
            let (l, r) = op_vec_split(ca, N as u32 / 2); // consumes the dup
            op_drop(l);
            op_drop(r);
        }
    });
    println!("ALLOC vec_split x100: {split_allocs}");
    assert!(
        split_allocs <= 1400,
        "vec_split x100 allocs {split_allocs} exceeds ceiling 1400 (O(log N) boundary-spine rebuild + TWO inline-handle output headers, N-independent; was 1500 before the header carried its root inline — the two headers each save one Vec, so ~-2/op, and re-adding them would climb back to 1500)"
    );
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
    assert_eq!(
        lookup, 0,
        "map_lookup on scalar keys is a pure read — zero allocations (was 1/op via champ_eq's eager worklist)"
    );
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
    // NOTE (RESOLVED): inline-handles step 2 briefly raised the set ops (union 373→376, ∩ 356→389,
    // ∖ 362→395) — a set data-node growing 2→3 entries spilled inline→heap once. `merge_entry_pair`
    // then built the 2-entry SET split node + fresh CHAMP entry INLINE, recovering set-algebra to
    // 270/263/266 (BELOW the pre-inline figures). The once-considered "born-heap for growing CHAMP set
    // nodes" follow-up was investigated and DECLINED: cap=3 only partially recovers and costs +8 bytes
    // on every Node; born-heap needs a will-grow flag threaded through the FBIP insert core (the
    // crown-jewel path). The ~few-alloc residual is inherent to inline storage and not worth that risk.
    assert!(
        union <= 320,
        "set_union (walk the smaller N/4 into the larger N) allocs {union} exceeds ceiling 320 (was 376 → 270 after merge_entry_pair inlines the 2-handle SET split node)"
    );
    let inter = measure(&mut || {
        op_dup(sa);
        op_dup(sb);
        op_drop(op_set_intersection(sa, sb));
    });
    println!("ALLOC set_intersection x{N}: {inter}");
    assert!(
        inter <= 320,
        "set_intersection x{N} allocs {inter} exceeds ceiling 320 (was 385 → 263 after merge_entry_pair)"
    );
    let diff = measure(&mut || {
        op_dup(sa);
        op_dup(sb);
        op_drop(op_set_difference(sa, sb));
    });
    println!("ALLOC set_difference x{N}: {diff}");
    assert!(
        diff <= 320,
        "set_difference x{N} allocs {diff} exceeds ceiling 320 (was 391 → 266 after merge_entry_pair)"
    );
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
    assert!(
        ddiff <= 600,
        "unique-a small-b difference x{N} allocs {ddiff} exceeds ceiling 600 (fast path: build da + in-place removes; was 807 → 488 after merge_entry_pair; a regression to the insert-fold would ~2x)"
    );
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
    // Each iteration allocates ONLY the probe tuple = 1 node Box (`op_arr_alloc(2)` carries its 2
    // handles INLINE, empty raw, immediate elements — the inline-handles win; NOT the 2/op an
    // out-of-line handles Vec cost before it). BOTH the shallow-compound champ_hash fast path AND the
    // shallow-compound champ_eq fast path add NO worklist — so a hit costs exactly the probe node.
    // ~1000 for N=1000 = 1/lookup. A regression to the general hash/eq walk would add ~1-2 more per
    // lookup; a regression to an out-of-line probe-tuple handles Vec would be ~2/op.
    assert!(
        clookup <= 1000,
        "shallow-compound-key lookup x{N} allocs {clookup} exceeds ceiling 1000 (1/op = JUST the probe tuple's node Box; shallow hash+eq fast paths add no worklist, probe handles inline)"
    );
    op_drop(cm);

    // (H2) map lookup by a NESTED-COMPOUND key (a 4-deep nested tuple) — the key falls THROUGH the
    // shallow-compound fast path into champ_hash's general iterative walk, so this guards that walk's
    // pre-sized worklists (`Vec::with_capacity` — a growing Vec would realloc 1→2→4→8 per hash, ~29%
    // slower; @c467820). Each iteration builds a fresh nested probe (4 arr nodes) + looks it up. The
    // allocation is the probe's nodes; the pre-sized hash worklists must NOT realloc-churn on top.
    let nested = |seed: i64| -> Handle {
        let mut t = op_box_int(seed);
        for d in 1..4i64 {
            let outer = op_arr_alloc(2);
            op_arr_set(outer, 0, op_box_int(seed + d));
            op_arr_set(outer, 1, t);
            t = outer;
        }
        t
    };
    let mut nm = op_map_empty();
    for k in 0..N {
        nm = op_map_insert(nm, nested(k), op_box_int(k));
    }
    let nlookup = measure(&mut || {
        for k in 0..N {
            let probe = nested(k);
            let _ = op_map_lookup(nm, probe);
            op_drop(probe);
        }
    });
    println!("ALLOC map_lookup_nestedkey x{N}: {nlookup}");
    // ~4/op for N=1000: ONLY the 4-deep probe's arr nodes. BOTH general-walk worklists that a nested
    // key touches are now REUSED from a thread-local — champ_hash's (`HASH_SCRATCH`, @b3ac802) AND the
    // slot-hit champ_eq's (`EQ_SCRATCH`, this tick): each clears + reuses its buffer, so a nested-key
    // hash AND its equality compare are both allocation-FREE (was 5/op with the eq worklist Vec, 7/op
    // before the hash worklists were reused). Guards that neither thread-local reuse regressed.
    assert!(
        nlookup <= 4400,
        "nested-compound-key lookup x{N} allocs {nlookup} exceeds ceiling 4400 (probe arr nodes only; both the champ_hash AND champ_eq general-walk worklists are thread-local-reused, allocation-free — was 5000 with the eq Vec)"
    );
    op_drop(nm);

    // (H2b) DIRECT structural value-equality (`champ_eq`, the language `=` on two runtime compounds
    // via `value-eq`) over a NESTED compound — the general worklist walk, NOT the shallow fast path.
    // Two equal 4-deep nested tuples are compared N times; `champ_eq` BORROWS both operands and the
    // walk is iterative, so a comparison allocates NOTHING now that the worklist is reused from the
    // `EQ_SCRATCH` thread-local (was one `Vec<(Handle,Handle)>` per compare). Build both operands
    // ONCE outside the measured loop so only the compares are timed. Guards the eq-worklist reuse.
    let nested = |seed: i64| -> Handle {
        let mut t = op_box_int(seed);
        for d in 1..4i64 {
            let outer = op_arr_alloc(2);
            op_arr_set(outer, 0, op_box_int(seed + d));
            op_arr_set(outer, 1, t);
            t = outer;
        }
        t
    };
    let ea = nested(42);
    let eb = nested(42); // structurally equal to `ea`, distinct nodes → forces the full walk
    let veq = measure(&mut || {
        for _ in 0..N {
            assert!(champ_eq(ea, eb)); // equal nested compounds — full general worklist walk
        }
    });
    println!("ALLOC value_eq_nested x{N}: {veq}");
    // ZERO allocations: `champ_eq` borrows, and the general-walk worklist is reused from `EQ_SCRATCH`
    // (clear + refill, grows once then never allocates). A regression to a per-compare Vec would push
    // this to ~N. Both operands are pre-built + dropped outside the measured closure.
    assert!(
        veq <= 50,
        "nested value-eq x{N} allocs {veq} exceeds ceiling 50 (champ_eq borrows + the general-walk worklist is thread-local-reused → allocation-free; a per-compare Vec would be ~N)"
    );
    op_drop(ea);
    op_drop(eb);

    // (H3) map lookup by a heap-backed STRING key (the JSON-object / dictionary shape: keys are
    // multi-byte strings whose raw is a Heap `Vec<u8>`, not an inline immediate). A string key is
    // arity-0, so champ_hash takes the arity-0 fast path (FNV over the leaf's raw, NO worklist) and
    // a slot-hit champ_eq compares raw bytes with NO worklist — so the LOOKUP itself allocates
    // nothing; the only per-iteration allocation is the probe string leaf. The probe key bytes are
    // pre-built ONCE outside the measured closure (a `Vec<Vec<u8>>`) so the counting allocator does
    // NOT charge `format!`'s buffer-growth to the lookup — inside the loop we build exactly one
    // string leaf per probe (`op_str_new` on a cloned byte string) and look it up.
    let skeys: Vec<Vec<u8>> = (0..N)
        .map(|k| format!("key-{k:0>11}").into_bytes())
        .collect();
    let mut sm = op_map_empty();
    for bytes in &skeys {
        let key = op_str_new(String::from_utf8(bytes.clone()).unwrap());
        sm = op_map_insert(sm, key, op_box_int(0));
    }
    let slookup = measure(&mut || {
        for bytes in &skeys {
            // `bytes.clone()` = the probe's key bytes (1 alloc, unavoidable — a probe needs bytes);
            // `String::from_utf8` reuses that buffer; `op_str_new`'s `into_bytes` reuses it again, so
            // the runtime adds only the leaf's node Box. = 2 allocs/probe, NONE from the lookup.
            let probe = op_str_new(String::from_utf8(bytes.clone()).unwrap());
            let _ = op_map_lookup(sm, probe);
            op_drop(probe);
        }
    });
    println!("ALLOC map_lookup_stringkey x{N}: {slookup}");
    // Each iteration allocates ONLY the probe string leaf: the cloned key-bytes Vec + the node Box (the
    // 16-byte string exceeds the 12-byte inline cap, so the raw stays Heap = the cloned Vec, reused via
    // `into_bytes` — no extra raw alloc). = 2 allocs/op. The arity-0 champ_hash fast path and the
    // no-worklist champ_eq byte compare add NOTHING to the lookup. ~2000 for N=1000 = 2/lookup (probe
    // bytes + node). A regression to an allocating hash/eq walk for string keys would exceed this.
    assert!(
        slookup <= 2200,
        "string-key lookup x{N} allocs {slookup} exceeds ceiling 2200 (probe string leaf only: cloned key bytes + node Box; arity-0 hash + no-worklist eq add nothing to the lookup)"
    );
    op_drop(sm);

    // (I) sum construction (Option/Result-shaped: disc in raw + payload handle) x1000. A sum node is
    // JUST the node Box = 1 alloc/op: `op_sum_new` builds its 4-byte disc raw INLINE and its single
    // payload handle INLINE (`Handles::inline_from(&[payload])`), and an immediate payload boxes to no
    // node. Was 3/op (heap disc Vec + heap handles Vec + node), then 2/op after the inline-raw, now
    // 1/op with inline handles. Guards that the Option/Result-heavy sum path stays at the node-Box floor.
    let sum = measure(&mut || {
        for k in 0..N {
            op_drop(op_sum_new(1, op_box_int(k)));
        }
    });
    println!("ALLOC sum_new x{N}: {sum}");
    assert!(
        sum <= 1000,
        "sum_new x{N} allocs {sum} exceeds ceiling 1000 (JUST the node Box = 1/op; disc raw inline, payload handle inline, immediate payload — a regression to an out-of-line disc/handles Vec would be 2-3/op)"
    );

    // (J) bytes SLICE x1000 — a rope slice node over a shared leaf: 1 handle (the parent buf) +
    // the 8-byte `[off,len]` raw. Both the `[off,len]` raw (`slice_raw`) AND the single handle
    // (`Handles::inline_from`) are now built INLINE, so a slice node is JUST the node Box = 1
    // alloc/op (was 3: + a heap [off,len] Vec + a heap `vec![buf]` handles Vec; the raw inlined in an
    // earlier tick, the handles Vec here). Guards the inline win for the O(1)-no-copy bytes rope. The
    // leaf is built + retained OUTSIDE the loop so we measure only the slice node.
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
    assert!(
        slice <= 1100,
        "bytes_slice x{N} allocs {slice} exceeds ceiling 1100 (JUST the node Box = 1/op; both the [off,len] raw and the single handle are inline — was 2/op with a heap vec![buf] handles Vec)"
    );
    op_drop(leaf);

    // (J2) bytes CONCAT x1000 — a rope concat node over two shared leaves: 2 handles (left, right) +
    // the inline 4-byte `[len]` raw = ONE allocation of the node Box + its 2-elem handles (INLINE,
    // no heap Vec — a concat is arity-2, exactly INLINE_HANDLES_CAP). O(1), copies NOTHING (the two
    // operands are shared subtrees, not copied). Guards the O(1)-no-copy concat: a regression to
    // eager materialization (copying the operands' bytes into a fresh leaf) would scale with the
    // operand lengths, not stay constant. Both operands built + retained OUTSIDE the loop.
    let la = {
        let b = op_bytes_alloc(16);
        for i in 0..16u32 {
            op_bytes_set(b, i, i);
        }
        b
    };
    let lb = {
        let b = op_bytes_alloc(16);
        for i in 0..16u32 {
            op_bytes_set(b, i, 100 + i);
        }
        b
    };
    let concat = measure(&mut || {
        for _ in 0..N {
            op_dup(la); // concat consumes a ref to each operand; keep both alive across the batch
            op_dup(lb);
            op_drop(op_bytes_concat(la, lb));
        }
    });
    println!("ALLOC bytes_concat x{N}: {concat}");
    assert!(
        concat <= 1100,
        "bytes_concat x{N} allocs {concat} exceeds ceiling 1100 (ONE node Box + inline 2-elem handles + inline [len] raw = 1 alloc/op; a regression to eager byte-copy would scale with operand length, or to a heap handles Vec would ~2x)"
    );
    op_drop(la);
    op_drop(lb);

    // (J3) bytes FLATTEN — materialize a DEEP concat rope into one leaf (triggered by `bytes-get` on a
    // rope). Build a rope of `DEPTH` concat nodes over small leaves, then a single `bytes-get` walks +
    // copies the logical bytes into a fresh leaf ONCE (an O(total-bytes) iterative walk, NOT per-node
    // re-flatten). Allocates the one flattened leaf's Heap raw (the bytes exceed the inline cap). A
    // regression to O(depth²) (e.g. an accidental per-node re-flatten, or op_bytes_len re-walking)
    // would blow the count up. Measured per-flatten (not ×N) since it's a one-time O(n) materialize.
    const DEPTH: u32 = 64;
    let flatten = measure(&mut || {
        let mut rope = {
            let b = op_bytes_alloc(4);
            for i in 0..4u32 {
                op_bytes_set(b, i, i);
            }
            b
        };
        for _ in 0..DEPTH {
            let piece = {
                let b = op_bytes_alloc(4);
                for i in 0..4u32 {
                    op_bytes_set(b, i, i + 1);
                }
                b
            };
            rope = op_bytes_concat(rope, piece); // grows a right-leaning concat spine
        }
        let _ = op_bytes_get(rope, 0); // forces bytes_flatten: one O(total) walk into a fresh leaf
        op_drop(rope);
    });
    println!("ALLOC bytes_flatten x{DEPTH}: {flatten}");
    // Build allocs: 1 base leaf + DEPTH×(piece leaf + concat node) then flatten adds the leaf's Heap
    // raw. All bounded by O(DEPTH), NOT O(DEPTH²). Ceiling = generous headroom over the linear count.
    assert!(
        flatten <= 400,
        "bytes_flatten DEPTH={DEPTH} allocs {flatten} exceeds ceiling 400 (linear in DEPTH: base + DEPTH×(piece+concat) + one flattened leaf; a regression to O(DEPTH²) re-flatten/re-walk would blow up)"
    );

    // (J3b) SMALL flatten — the hot per-char shape a real STRING LEXER hits: `String.at(s,i)` returns a
    // 1-byte SLICE which the compiler compacts (= `bytes_flatten`) before comparing to a char literal.
    // A ≤INLINE_RAW_CAP result is materialized into a STACK buffer + inline `Raw` (no output Vec) and the
    // walk worklist is REUSED from `FLATTEN_SCRATCH` (thread-local) — so a small flatten is ALLOCATION-
    // FREE steady-state. Measure ONLY the compact (build the slice outside the timed op is impossible
    // since compact consumes it, so build+compact and subtract the build baseline). Was 2/flatten (a
    // transient `dst` Vec + the `work` seed Vec, both freed); now 0. Guards the lexer's per-char cost.
    let src8 = op_str_new(String::from("abcdefgh"));
    let build_base = measure(&mut || {
        for _ in 0..N {
            op_dup(src8); // op_bytes_slice CONSUMES its operand → dup the shared source first
            op_drop(op_bytes_slice(src8, 3, 1)); // build a 1-byte slice, drop it (baseline)
        }
    });
    let build_plus_compact = measure(&mut || {
        for _ in 0..N {
            op_dup(src8);
            op_drop(op_bytes_compact(op_bytes_slice(src8, 3, 1))); // build + flatten
        }
    });
    op_drop(src8);
    let per_small_flatten = build_plus_compact.saturating_sub(build_base);
    println!(
        "ALLOC small_flatten x{N}: {per_small_flatten} (build+compact {build_plus_compact} − build {build_base})"
    );
    assert!(
        per_small_flatten <= 100,
        "small (≤cap) flatten allocs {per_small_flatten} for x{N} exceeds ceiling 100 (≈0/flatten: stack-buffer output + reused FLATTEN_SCRATCH worklist; a regression to the transient dst Vec + work Vec would be ~2/flatten = ~2000)"
    );

    // (K) build a 2-tuple x1000 (`op_arr_alloc(2)` + two slot sets) — the common positional-product
    // constructor shared by tuples, records, and CHAMP `[k,v]` pairs. With scalar (immediate) elements
    // a tuple node is JUST the node Box = 1 alloc/op: `op_arr_alloc(2)` carries its ≤2 handles INLINE
    // (`Handles::inline_nulls`, no heap Vec), its raw is empty (inline), and immediate elements box to
    // no node. This is the ≤2-handle product-construction FLOOR — the inline-`handles` lever is already
    // taken here (measured 1/op, NOT the 2/op an out-of-line handles Vec would cost). Tracked so a
    // regression (e.g. handles spilling to heap on this path) is immediately visible.
    let tbuild = measure(&mut || {
        for k in 0..N {
            let t = op_arr_alloc(2);
            op_arr_set(t, 0, op_box_int(k));
            op_arr_set(t, 1, op_box_int(k + 1));
            op_drop(t);
        }
    });
    println!("ALLOC tuple2_build x{N}: {tbuild}");
    assert!(
        tbuild <= 1000,
        "tuple2_build x{N} allocs {tbuild} exceeds ceiling 1000 (JUST the node Box = 1/op; handles inline, raw empty, immediate elements — a regression to an out-of-line handles Vec would be ~2/op)"
    );

    // (K3d) `bigint-of-i64` — the WIDENING entry (`BigInt.of` on a runtime int; also the on-ramp for
    // fixed-width int arithmetic that promotes to BigInt). Boxes the i64 DIRECTLY through the i128 path
    // (`box_bigint_i128` → inline sign-magnitude bytes, NO `Big`) = ONLY the result node = 1/op. Was
    // 2/op — the `box_bigint(&Big::from_i64(v))` route allocated a transient `Big` limb `Vec` (freed
    // once serialized to the inline leaf) on top of the node. A regression back to the `Big` route
    // would climb to ~2/op.
    let bofi = measure(&mut || {
        for _ in 0..N {
            op_drop(op_bigint_of_i64(1_000_003));
        }
    });
    println!("ALLOC bigint_of_i64 x{N}: {bofi}");
    assert!(
        bofi <= 1500,
        "bigint_of_i64 x{N} allocs {bofi} exceeds ceiling 1500 (direct i128 box: only the result node, 1/op; was 2/op via the transient Big::from_i64 limb Vec — a regression to the Big route would climb to ~2000)"
    );

    // (K4) `bigint-add` — a runtime BigInt op (B3b/B3c emit these for runtime-valued BigInt arithmetic),
    // now on the hot path of any bignum loop. The SMALL-operand FAST PATH reads both operands as `i128`
    // DIRECTLY from their raw sign-magnitude bytes (no limb `Vec`), computes with `checked_add`, and
    // boxes the `i128` result — so a small add allocates ONLY the result node (was: 2 unbox Vecs + a
    // result magnitude Vec + the node = 4/op; now 1/op). A value out of i128 range or an overflowing
    // result falls back to the full `Big` path (byte-identical). Build the two operands ONCE outside
    // the loop; measure only the add + result drop. Guards that the fast path stays alloc-lean.
    let (bi_a, bi_b) = (op_bigint_of_i64(12345), op_bigint_of_i64(67890));
    let bigadd = measure(&mut || {
        for _ in 0..N {
            let r = op_bigint_add(bi_a, bi_b); // borrows both operands
            op_drop(r);
        }
    });
    op_drop(bi_a);
    op_drop(bi_b);
    println!("ALLOC bigint_add x{N}: {bigadd}");
    // Per op on SMALL operands: ONLY the result node (both operands read as i128 from their raw bytes,
    // the i128 result boxed directly) = 1/op. Was 4/op (2 unbox Vecs + a result magnitude Vec + node)
    // before the i128 fast path. The ceiling catches a regression that loses the fast path (climbs back
    // to ~4/op = 4000) or adds per-op churn; the exact figure is measured + baselined.
    assert!(
        bigadd <= 2000,
        "bigint_add x{N} allocs {bigadd} exceeds ceiling 2000 (i128 fast path: only the result node, ~1/op; was 4/op with the full Big unbox/box — a lost fast path would climb back to ~4000)"
    );

    // (K4a) `bigint-div`/`-rem` — the RESULT-BUILDING division ops (B3b/B3c emit them for runtime BigInt
    // `/`/`%`, and they back fixed-width int `/`/`%` once widened). They now share the SAME i128 fast
    // path as add/sub/mul: read both operands as `i128` from raw bytes, `checked_div`/`checked_rem`
    // (truncate-toward-zero / dividend-sign — byte-identical to `Big::divmod`), box the i128 result — so
    // a small div/rem allocates ONLY the result node = 1/op (was ~4/op: 2 unbox limb Vecs + the `divmod`
    // quotient+remainder Vecs + the node). A zero divisor or the `i128::MIN / -1` overflow falls through
    // to the `Big` path (which traps on zero, or produces the wide result). Measure div + rem together.
    let (bd_a, bd_b) = (op_bigint_of_i64(1_000_003), op_bigint_of_i64(97));
    let bigdiv = measure(&mut || {
        for _ in 0..N {
            op_drop(op_bigint_div(bd_a, bd_b)); // borrows both operands
            op_drop(op_bigint_rem(bd_a, bd_b));
        }
    });
    op_drop(bd_a);
    op_drop(bd_b);
    println!("ALLOC bigint_div_rem x{N}: {bigdiv}");
    // Per iteration: one div + one rem, each the result node only = 2/op → ~2·N. Was ~8/op (each op:
    // 2 unbox Vecs + divmod's 2 result Vecs + node ≈ 4) before the fast path. The ceiling catches a
    // regression that loses the fast path (climbs back to ~8·N = 8000) or adds per-op churn.
    assert!(
        bigdiv <= 4000,
        "bigint_div_rem x{N} allocs {bigdiv} exceeds ceiling 4000 (i128 fast path: div + rem each the result node only, ~2/op combined; was ~8/op with the full Big unbox/divmod/box — a lost fast path would climb back to ~8000)"
    );

    // (K4b) `bigint-cmp` — a READ-ONLY comparison (the primitive `<`/`>`/`<=`/`>=`/`=` on BigInt lower
    // to, and the BigInt map/set-key comparator). It compares the operands' `raw` sign-magnitude slices
    // DIRECTLY (`Big::cmp_sign_magnitude_bytes`) with NO `Big` decode → ZERO allocations (was 2/op, both
    // unbox limb Vecs, when it went through `unbox_bigint`). Build the operands once; measure the cmp.
    let (bc_a, bc_b) = (op_bigint_of_i64(111), op_bigint_of_i64(222));
    let bigcmp = measure(&mut || {
        for _ in 0..N {
            core::hint::black_box(op_bigint_cmp(bc_a, bc_b));
        }
    });
    op_drop(bc_a);
    op_drop(bc_b);
    println!("ALLOC bigint_cmp x{N}: {bigcmp}");
    assert_eq!(
        bigcmp, 0,
        "bigint_cmp x{N} allocs {bigcmp} — a comparison reads the raw slices directly, allocating NOTHING (a regression to unbox-both would be ~2/op)"
    );

    // (K4c) `bigint-to-i64-checked` — the READ-ONLY checked narrowing (`Int64.of` on a runtime BigInt).
    // It reads the leaf's `raw` sign-magnitude slice DIRECTLY (`Big::i64_checked_from_sign_magnitude_
    // bytes`) with NO `Big` decode → ZERO allocations (was 1/op, the unbox limb Vec).
    let bt = op_bigint_of_i64(9999);
    let bigto = measure(&mut || {
        for _ in 0..N {
            core::hint::black_box(op_bigint_to_i64_checked(bt));
        }
    });
    op_drop(bt);
    println!("ALLOC bigint_to_i64 x{N}: {bigto}");
    assert_eq!(
        bigto, 0,
        "bigint_to_i64 x{N} allocs {bigto} — the narrowing reads the raw slice directly, allocating NOTHING (a regression to unbox would be ~1/op)"
    );

    // (K4d) `rational-cmp` — the READ-ONLY Rational comparison (`<`/`>`/`=`/… on Rationals, R3b). Both
    // operands normalized (den > 0), so `a/b <=> c/d` ⇔ `a·d <=> c·b`. FAST PATH: when all four
    // components fit i64 (the common case), the cross-products fit i128 without overflow and the compare
    // is exact NATIVE arithmetic — ZERO allocation (was 6/op: 4 unbox limb Vecs + 2 mul Vecs, via the
    // full `Big` cross-multiply). The bigint-cmp read-the-raw-slice lesson, extended to a rational.
    let (rc_a, rc_b) = (
        op_rational_of(op_bigint_of_i64(1), op_bigint_of_i64(3)),
        op_rational_of(op_bigint_of_i64(1), op_bigint_of_i64(6)),
    );
    let rcmp = measure(&mut || {
        for _ in 0..N {
            core::hint::black_box(op_rational_cmp(rc_a, rc_b));
        }
    });
    println!("ALLOC rational_cmp x{N}: {rcmp}");
    assert_eq!(
        rcmp, 0,
        "rational_cmp x{N} allocs {rcmp} — the i64-components fast path cross-multiplies in i128 with NO Big (a regression to the full unbox+Big-mul would be ~6/op)"
    );

    // (K4e) `rational-add` — a RESULT-BUILDING Rational op (R3b emits it for runtime rational `+`). The
    // i64-COMPONENTS FAST PATH (all four `(num,den)` fit i64 — the common case) cross-multiplies + adds
    // in i128 (`checked_add`; overflow → the `Big` path), gcd-reduces in i128, and boxes — so the ONLY
    // allocation is the result Rational node + its 2 BigInt-leaf children = 3/op (was ~23/op: 4 unbox
    // limb Vecs + cross-multiply + gcd-normalize Vecs on the full `Big` path). Same fast-path shape as
    // `rational-cmp` above and bigint's i128 add. A component out of i64 range falls back to `Big`.
    let (ra_a, ra_b) = (
        op_rational_of(op_bigint_of_i64(1), op_bigint_of_i64(3)),
        op_rational_of(op_bigint_of_i64(1), op_bigint_of_i64(6)),
    );
    let radd = measure(&mut || {
        for _ in 0..N {
            op_drop(op_rational_add(ra_a, ra_b));
        }
    });
    println!("ALLOC rational_add x{N}: {radd}");
    assert!(
        radd <= 3500,
        "rational_add x{N} allocs {radd} exceeds ceiling 3500 (~3/op = result node + 2 BigInt children; the i64-components fast path cross-multiplies + gcd-reduces in i128 with no `Big`/limb Vec. Was 23/op on the full-Big path — a lost fast path would climb back to ~23000)"
    );
    op_drop(rc_a);
    op_drop(rc_b);
    op_drop(ra_a);
    op_drop(ra_b);

    // (K2) `vec-of-arr` — the bulk list-literal constructor. EVERY `(list e0…e{n-1})` literal lowers to
    // `arr-alloc(n)` + n×`arr-set` then ONE `vec-of-arr` (NOT `vec-empty` + n×`vec-push`), so this op is
    // on the hot path of every list construction yet was previously un-benched. Two shapes:
    //   • SMALL (≤32): the arr node IS a valid single strict leaf — `vec-of-arr` MOVES it in as the root
    //     (no per-element copy), so the only allocation beyond the caller's arr is the vec HEADER node.
    //   • LARGE (>32): the elements are repacked into ≤32-element strict leaves + a bottom-up radix trie.
    // Build the arr INSIDE the loop (its construction is the caller's cost, same as a real literal) and
    // drop the resulting vec each iteration. A regression to a `vec-push` chain would be ~n allocs/list
    // (trie churn per element) instead of the near-constant bulk build.
    let voa_small = measure(&mut || {
        for _ in 0..N {
            let a = arr_of_ints(0, 8); // an 8-element list literal — fits one leaf
            op_drop(op_vec_of_arr(a));
        }
    });
    println!("ALLOC vec_of_arr_small x{N}: {voa_small}");
    // MEASURED ~3/list: the arr (node Box + its 8-element heap handles Vec = 2) is the CALLER's cost (the
    // 8 elements are immediate ints → no boxes); `vec-of-arr` adds ONLY the vec-header node (1) — its raw
    // is INLINE and its single root handle is now carried INLINE too (no per-header Vec) — since the ≤32
    // arr is MOVED in as the leaf root (no per-element copy). Was ~4/list when the header allocated a Vec
    // for its one handle. A `vec-push`-chain regression would add ~8 trie-churn allocs/list.
    assert!(
        voa_small <= 3500,
        "vec_of_arr_small x{N} allocs {voa_small} exceeds ceiling 3500 (~3/list: caller's arr node+handles (2) + vec-of-arr's zero-copy leaf move + ONE inline-handle header (1); was ~4/list before the header carried its root inline — a vec-push-chain regression would be ~8/list, re-adding the header Vec ~1/list)"
    );
    const VOA_BIG: i64 = 100; // >32 → the bottom-up strict-trie repack path
    let voa_big = measure(&mut || {
        let a = arr_of_ints(0, VOA_BIG);
        op_drop(op_vec_of_arr(a));
    });
    println!("ALLOC vec_of_arr_big len={VOA_BIG}: {voa_big}");
    // MEASURED 18: 100 elements → ⌈100/32⌉=4 strict leaves (Box+Vec each) + 1 interior root (Box+Vec) +
    // header (node+raw) + the moved `elems` buffer — all N-INDEPENDENT beyond ⌈n/32⌉. The element handles
    // are MOVED out of the arr (`into_vec` on the Heap arm), not re-copied. Guards the bottom-up trie
    // build against an O(n)-alloc regression (a push-chain or per-element node would be ~{VOA_BIG}).
    assert!(
        voa_big <= 40,
        "vec_of_arr_big len={VOA_BIG} allocs {voa_big} exceeds ceiling 40 (⌈n/32⌉ strict leaves + interior spine + header, element handles MOVED not copied; a per-element regression would be ~{VOA_BIG})"
    );

    // (K3) FBIP IN-PLACE REUSE — the `reset` → `arr-alloc-reuse` → refill protocol the compiler emits
    // for a `List.map` / functional rebuild over a UNIQUE value: instead of free+malloc a new shell,
    // the dying node's shell (its node Box + handle-Vec backing) is RETAINED as a token and refit in
    // place. This is the whole point of the FBIP ops (runtime-complete + correctness-tested via
    // `fbip_map_over_unique_list_reuses_in_place`) yet the reuse WIN was un-benched. Measure a 3-element
    // "map" done BOTH ways over the loop and prove the reuse path shell-node cost is ZERO. NOTE the ops
    // are not yet compiler-EMITTED (that's compiler-side), so this guards the runtime's readiness.
    const REUSE_LEN: u32 = 3;
    // Baseline: a fresh-alloc rebuild — drop the old shell, arr-alloc a NEW one (what a non-FBIP
    // emitter does). Per iteration: REUSE_LEN new leaves (immediate ints → 0) + a NEW node Box + its
    // handle Vec = the shell cost we want reuse to eliminate.
    let reuse_fresh = measure(&mut || {
        for k in 0..N {
            let xs = op_arr_alloc(REUSE_LEN);
            for i in 0..REUSE_LEN {
                op_arr_set(xs, i, op_box_int(k + i as i64));
            }
            op_drop(xs); // free the shell — the fresh path will malloc a new one next iteration
            let ys = op_arr_alloc(REUSE_LEN); // FRESH shell (node Box + handle Vec)
            for i in 0..REUSE_LEN {
                op_arr_set(ys, i, op_box_int(k + i as i64 + 100));
            }
            op_drop(ys);
        }
    });
    println!("ALLOC fbip_map_fresh x{N}: {reuse_fresh}");
    // The FBIP path: reset the unique shell to a token, refit it (SAME node, no new Box/Vec), refill.
    let reuse_fbip = measure(&mut || {
        for k in 0..N {
            let xs = op_arr_alloc(REUSE_LEN);
            for i in 0..REUSE_LEN {
                op_arr_set(xs, i, op_box_int(k + i as i64));
            }
            let token = op_reset(xs); // unique → frees old leaves, RETAINS the shell as a token
            let ys = op_arr_alloc_reuse(REUSE_LEN, token); // SAME shell refit — no new node/Vec
            for i in 0..REUSE_LEN {
                op_arr_set(ys, i, op_box_int(k + i as i64 + 100));
            }
            op_drop(ys);
        }
    });
    println!("ALLOC fbip_map_reuse x{N}: {reuse_fbip}");
    // Reuse must allocate STRICTLY FEWER than fresh: it eliminates the second shell's node Box + handle
    // Vec every iteration (same-length refit reuses the retained backing, capacity intact → no realloc).
    // Guard the invariant that reuse never allocates MORE than fresh, and that it saves ≥ the per-iter
    // shell (≥ N allocs saved over the batch). A regression that lost the shell-reuse would erase the gap.
    assert!(
        reuse_fbip < reuse_fresh,
        "FBIP reuse x{N} ({reuse_fbip}) must allocate fewer than the fresh-alloc rebuild ({reuse_fresh}) — reuse refits the retained shell, saving a node Box + handle Vec per map"
    );
    assert!(
        reuse_fresh - reuse_fbip >= N as u64,
        "FBIP reuse should save ≥ the per-iteration shell node ({N} over the batch); saved {} (fresh {reuse_fresh} − reuse {reuse_fbip})",
        reuse_fresh - reuse_fbip
    );

    // (L) value-encode a recursive value (the op-62 escape walker): encode a FIXED 50-element IntList
    // repeatedly. Each encode builds a fresh value-form document — a leaf pool Vec + struct table Vec
    // + the output byte Vec + the returned Bytes leaf — so its allocation is INHERENTLY linear in the
    // value's node count (each Cons/tuple/int emits leaves+structs) and constant per encode. The value
    // is built ONCE outside the measured loop (only the encode is timed). This row guards against a
    // regression to per-NODE transient churn or an O(N²) re-walk in the iterative walker (the
    // `EncodeWork` stack + `out`/`work` Vecs must stay grow-once, not realloc-per-node).
    let ve_desc = intlist_descriptor();
    let ve_list = build_intlist(50);
    const VE_REPS: usize = 100;
    let venc = measure(&mut || {
        for _ in 0..VE_REPS {
            let doc = op_value_encode_form(ve_list, &ve_desc).expect("encode");
            core::hint::black_box(&doc);
        }
    });
    println!("ALLOC value_encode x{VE_REPS}: {venc}");

    // (L2) value-encode a STRING-KEYED MAP — exercises `map_entries_canonical`'s `sort_by`, whose
    // comparator (`value_cmp_shaped`'s `Shape::Str` arm) compares the keys' stored UTF-8 bytes.
    // That comparator BORROWS both nodes' raw slices and compares in place; a regression to
    // `to_vec`-per-compare would allocate ~2·N·log N transient Vecs PURELY to sort (here ~2·32·5 ≈ 320
    // per encode). The map is built ONCE outside the loop, so only the encode+sort is measured. A
    // scalar-Int-keyed map sorts by `op_get_int` (no alloc) — this row specifically guards the STRING
    // comparator, the only ordering path that ever touched the heap.
    // Descriptor: [0]=Str (key), [1]=Int (val), [2]=Map(→0,→1), root=2. Tags: 3=Str, 0=Int, 13=Map.
    let smap_desc: &[u8] = &[0x03, 0x03, 0x00, 0x0d, 0x00, 0x01, 0x02];
    const SMAP_N: i64 = 32;
    let mut smap = op_map_empty();
    for k in 0..SMAP_N {
        // Keys "k00".."k31" — distinct, and inserted in-order so the CHAMP holds them in HASH order,
        // forcing the canonical `sort_by` to actually reorder (its comparator is the measured work).
        smap = op_map_insert(smap, op_str_new(alloc::format!("k{k:02}")), op_box_int(k));
    }
    const SMAP_REPS: usize = 100;
    let smap_enc = measure(&mut || {
        for _ in 0..SMAP_REPS {
            let doc = op_value_encode_form(smap, smap_desc).expect("encode string-keyed map");
            core::hint::black_box(&doc);
        }
    });
    println!("ALLOC value_encode_stringkeymap x{SMAP_REPS}: {smap_enc}");
    // The per-encode allocation is the document's grow-once pools + the entries Vec + the output/Bytes
    // leaf — all LINEAR in entry count, NONE from the key comparator (borrowed-slice compare). A
    // `to_vec`-per-compare regression would add ~2·N·log N (~320/encode = ~32000 over 100 reps).
    assert!(
        smap_enc <= 1200,
        "value_encode_stringkeymap x{SMAP_REPS} allocs {smap_enc} exceeds ceiling 1200 (10200 → 7100 (`DocLeaf::IntScalar` int values) → ~4800 (`ENCODE_BUILDER`+`ENCODE_OUT` reuse) → ~1600 (`DocLeaf::Str` stores `Raw` — a SHORT key like \"k00\" inlines, no per-leaf `Vec` clone) → ~903 (`ENCODE_WORK` reuse — the work stack no longer grows-from-zero per encode). The residual is the entries Vec + the output byte Vec growth. A LONG (>12-byte) key still heaps. The Str key comparator compares BORROWED slices — a to_vec-per-compare regression would add ~2·N·log N sort allocs, ~32000, firmly past this ceiling)"
    );
    op_drop(smap);

    // (L3) value-encode a LARGE int-keyed MAP — the shape the compiler now escapes (variable-length
    // collection results). Guards that a big map encode's ALLOCATION stays LINEAR in entry count: the
    // walk collects entries into ONE `entries` Vec (grow-once), sorts by `op_get_int` (no heap — an int
    // key sort allocates nothing), and emits grow-once leaf/struct/child pools + the output. A
    // regression to a per-entry transient Vec, or an O(N²) re-walk of the CHAMP, would blow the ceiling
    // (CPU scaling was separately verified ~flat 4.3-4.9 µs/entry to N=4096 — this row is the
    // machine-agnostic alloc guard for the same property). Built ONCE; only the encode is measured.
    let lmap_desc: &[u8] = &[0x03, 0x00, 0x00, 0x0d, 0x00, 0x01, 0x02]; // [0]=Int [1]=Int [2]=Map(k0,v1); root=2
    const LMAP_N: i64 = 1000;
    let mut lmap = op_map_empty();
    for k in 0..LMAP_N {
        lmap = op_map_insert(lmap, op_box_int(k), op_box_int(k * 10));
    }
    let lmap_enc = measure(&mut || {
        let doc = op_value_encode_form(lmap, lmap_desc).expect("encode a large int map");
        core::hint::black_box(&doc);
    });
    println!("ALLOC value_encode_largemap N={LMAP_N}: {lmap_enc}");
    // MEASURED ~42 for a SINGLE N=1000 encode: with `DocLeaf::IntScalar` (no per-int magnitude Vec)
    // AND the reused `ENCODE_BUILDER`/`ENCODE_OUT` pools (grown warm by the earlier value_encode reps in
    // this same test), a 1000-entry encode reallocates almost nothing — just the entries Vec + the
    // output byte Vec's own growth + the returned Bytes leaf. Was 2065 (per-int Vec) → 67 (IntScalar) →
    // ~42 (pool reuse). WARNING: this row runs AFTER the value_encode/stringkeymap rows, so the thread-local
    // pools are already at a high-water mark — the figure measures steady-state reuse, not cold growth.
    // A per-entry transient Vec or an O(N²) CHAMP re-walk would be orders of magnitude over (O(N²) ≈ 10⁶).
    assert!(
        lmap_enc <= 300,
        "value_encode_largemap N={LMAP_N} allocs {lmap_enc} exceeds ceiling 300 (2065 → 67 (`DocLeaf::IntScalar`) → ~42 (`ENCODE_BUILDER`/`ENCODE_OUT` reuse, warm from earlier rows): only the entries Vec + output byte Vec growth remain. A per-int-Vec or lost-pool-reuse regression would climb back to hundreds/thousands)"
    );
    op_drop(lmap);

    // (M) FREE CASCADE — `op_drop` of a DEEP unique structure. This is the single hottest RC path (the
    // compiler emits `drop` at every dead heap binding and the resource destructor), yet every OTHER
    // row above bundles the drop with O(N) CONSTRUCTION, so a regression in the cascade's own
    // allocation behavior is masked. Here construction is OUTSIDE the measured closure — ONLY the
    // teardown is timed. The invariant under guard: reclaiming an N-deep spine costs O(1) allocations,
    // NOT O(N). The cascade seeds an inline root's ≤2 children into a fixed `[Handle; 2]` buffer and,
    // on reaching the first HEAP child, ADOPTS that dying node's own handle Vec (`core::mem::take`) as
    // the worklist backing — so it never allocates a fresh worklist per level. A regression that
    // materializes a fresh Vec per node (O(N) allocs) or reverts to a recursive free (stack overflow at
    // this depth) trips this. Build a `(arr2 leaf spine)` cons-spine identical to the overflow test.
    const DROP_DEPTH: i64 = 4000;
    let build_spine = || {
        let mut acc = op_arr_alloc(0); // inline unit terminator — no node
        for _ in 0..DROP_DEPTH {
            let node = op_arr_alloc(2);
            op_arr_set(node, 0, op_box_int(1)); // immediate leaf — no element box
            op_arr_set(node, 1, acc);
            acc = node;
        }
        acc
    };
    let spine = build_spine();
    let drop_allocs = measure(&mut || op_drop(spine));
    println!("ALLOC free_cascade_deep DEPTH={DROP_DEPTH}: {drop_allocs}");
    // MEASURED near-ZERO: the root donates seed_buf, the first heap child's Vec is adopted as the sole
    // worklist, and every deeper level's ≤2 children refill the fixed buffer or the adopted Vec (which
    // may realloc a small O(log DEPTH) number of times as it grows — NOT O(DEPTH)). The ceiling is a
    // tiny constant, DEPTH-INDEPENDENT: a per-node-Vec regression at DEPTH=4000 would blow past it by
    // three orders of magnitude.
    assert!(
        drop_allocs <= 40,
        "free_cascade_deep DEPTH={DROP_DEPTH} allocs {drop_allocs} exceeds ceiling 40 (O(1) teardown: fixed seed buffer + adopt-by-move worklist; a fresh-Vec-per-node regression would be ~O(DEPTH), a recursive-free regression would stack-overflow)"
    );
    // MEASURED ~13 allocs/encode of a 50-element IntList (~150 value nodes) = ~0.09 allocs/node — after
    // the flat `child_pool` arena (was ~195/encode = ~1.3/node, `@80bf18d9`), the output-Vec pre-size
    // that killed the serialization realloc churn (~100→92, `@84ebc883`), the `DocLeaf::IntScalar`
    // raw-i64 leaf that stopped each int malloc'ing a magnitude Vec (92→43, `@6decb84a`), the reused
    // thread-local `ENCODE_BUILDER`/`ENCODE_OUT` pools (43→19: no more grow-from-ZERO per call), the
    // `DESCRIPTOR_CACHE` (19→13: the descriptor's table Vec + nested shape Vecs + name Strings — ~6/encode,
    // decoded ONCE and cached by bytes), AND the reused thread-local `ENCODE_WORK` stack (13→~7: the
    // iterative walk's task stack grows O(depth) — O(N) for a Cons-list — so a fresh Vec per call paid
    // an O(log N) grow-chain EVERY encode; now it grows once + refills allocation-free, after
    // `EncodeWork` became `'static`). The remaining allocs are the output byte Vec's own growth. Ceiling
    // TIGHTENED 1800→1000 to track the reduced ~7/encode floor; catches an O(N²) re-walk, a lost
    // pool/descriptor/work reuse, or a return of per-node Vec / output-realloc churn. `xtask bench`'s
    // baseline (~737) is the tight guard; this is the coarse in-suite backstop.
    assert!(
        venc <= 1000,
        "value_encode x{VE_REPS} allocs {venc} exceeds ceiling 1000 (~7/encode of a 50-node list after the ENCODE_WORK reuse; was ~13 before. Residual = the output byte Vec's growth. A lost cache/pool/work reuse, a per-int-Vec, or an output-realloc regression would climb)"
    );
    op_drop(ve_list);
}

#[test]
#[ignore] // diagnostic timing — run with --release --ignored --nocapture (DEBUG ns numbers are ~10-50x inflated + ratio-distorted — worthless)
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
    println!(
        "SETSCALE compound-union (small={n_small} into big={n_big})  {union_ns:6.1} ns/elem-walked"
    );
    op_drop(big);
    op_drop(small);
}

#[test]
#[ignore] // diagnostic timing — run with --release --ignored --nocapture (DEBUG ns numbers are ~10-50x inflated + ratio-distorted — worthless)
fn string_key_map_cpu_scaling_probe() {
    // Pre-build all key byte strings ONCE so the timed loops measure the runtime, not `format!`.
    let make_keys = |n: usize, len: usize| -> Vec<Vec<u8>> {
        (0..n)
            .map(|k| {
                let mut s = format!("key-{k}");
                while s.len() < len {
                    s.push('x');
                }
                s.into_bytes()
            })
            .collect()
    };
    let n = 8000usize;
    for &len in &[8usize, 24, 64, 256] {
        let keys = make_keys(n, len);
        // Build the map.
        let mut m = op_map_empty();
        for bytes in &keys {
            let key = op_str_new(String::from_utf8(bytes.clone()).unwrap());
            m = op_map_insert(m, key, op_box_int(0));
        }
        // Time repeated full-map lookup (every key hit). reps scaled so total work is comparable.
        let reps = (2_000_000 / n).max(1);
        let t = std::time::Instant::now();
        for _ in 0..reps {
            for bytes in &keys {
                let probe = op_str_new(String::from_utf8(bytes.clone()).unwrap());
                let _ = op_map_lookup(m, probe);
                op_drop(probe);
            }
        }
        let ns = t.elapsed().as_nanos() as f64 / (reps as f64 * n as f64);
        println!("STRKEY len={len:>4}  lookup {ns:6.1} ns/op  (n={n})");
        op_drop(m);
    }
}

#[test]
#[ignore] // diagnostic timing — run with --release --ignored --nocapture (DEBUG ns numbers are ~10-50x inflated + ratio-distorted — worthless)
fn shared_vec_copy_path_cpu_scaling_probe() {
    for &n in &[1000i64, 4000, 16000, 64000] {
        // Build an N-element base vector, kept shared (rc>1) across the timed ops.
        let mut base = op_vec_empty();
        for k in 0..n {
            base = op_vec_push(base, op_box_int(k));
        }
        // A FIXED large op count (~1M push + 1M update) so timing is stable across N (the copy path
        // is O(log N) per op — total ops must NOT shrink with N or large-N tiers get too few samples).
        let reps = 1_000_000i64;
        // Shared PUSH: dup the base (force path-copy), push, drop the result — base survives.
        let t0 = std::time::Instant::now();
        for _ in 0..reps {
            op_dup(base);
            op_drop(op_vec_push(base, op_box_int(0)));
        }
        let push_ns = t0.elapsed().as_nanos() as f64 / reps as f64;
        // Shared UPDATE at a middle index (a full-depth spine copy, not the right edge).
        let mid = (n / 2) as u32;
        let t1 = std::time::Instant::now();
        for _ in 0..reps {
            op_dup(base);
            op_drop(op_vec_update(base, mid, op_box_int(7)));
        }
        let upd_ns = t1.elapsed().as_nanos() as f64 / reps as f64;
        println!("VECSHARED n={n:>6}  push {push_ns:7.1} ns/op   update {upd_ns:7.1} ns/op");
        op_drop(base);
    }
}

#[test]
#[ignore] // diagnostic timing — run with --release --ignored --nocapture (DEBUG ns numbers are ~10-50x inflated + ratio-distorted — worthless)
fn shared_map_copy_path_cpu_scaling_probe() {
    for &n in &[1000i64, 4000, 16000, 64000] {
        // Build an N-entry base map, kept shared (rc>1) so each op path-copies instead of FBIP.
        let mut base = op_map_empty();
        for k in 0..n {
            base = op_map_insert(base, op_box_int(k), op_box_int(k));
        }
        let reps = 1_000_000i64;
        // Shared INSERT (overwrite an existing key → OVERWRITE/DESCEND path-copy).
        let t0 = std::time::Instant::now();
        for _ in 0..reps {
            op_dup(base);
            op_drop(op_map_insert(base, op_box_int(0), op_box_int(1)));
        }
        let ins_ns = t0.elapsed().as_nanos() as f64 / reps as f64;
        // Shared REMOVE of a present key (found-entry drop → path-copy up the spine).
        let t1 = std::time::Instant::now();
        for _ in 0..reps {
            op_dup(base);
            op_drop(op_map_remove(base, op_box_int(0)));
        }
        let rem_ns = t1.elapsed().as_nanos() as f64 / reps as f64;
        println!("MAPSHARED n={n:>6}  insert {ins_ns:7.1} ns/op   remove {rem_ns:7.1} ns/op");
        op_drop(base);
    }
}

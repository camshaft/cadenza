use super::*;

#[test]
fn int_round_trip() {
    reset();
    for v in [0i64, 42, -42, i64::MAX, i64::MIN] {
        assert_eq!(op_get_int(op_box_int(v)), v);
    }
    assert_eq!(render(op_box_int(0), &Shape::Int), "0");
    assert_eq!(render(op_box_int(-42), &Shape::Int), "-42");
    assert_eq!(
        render(op_box_int(i64::MAX), &Shape::Int),
        "9223372036854775807"
    );
    assert_eq!(
        render(op_box_int(i64::MIN), &Shape::Int),
        "-9223372036854775808"
    );
}

#[test]
fn bool_round_trip() {
    reset();
    assert!(op_get_bool(op_box_bool(true)));
    assert!(!op_get_bool(op_box_bool(false)));
    assert_eq!(render(op_box_bool(true), &Shape::Bool), "true");
    assert_eq!(render(op_box_bool(false), &Shape::Bool), "false");
}

#[test]
fn float_round_trip() {
    reset();
    for v in [0.0f64, 3.14, -2.5, 2.0, -100.0] {
        assert_eq!(op_get_float(op_box_float(v)), v);
    }
    // Fractional keeps its digits; whole number keeps a trailing `.0`.
    assert_eq!(render(op_box_float(3.14), &Shape::Float), "3.14");
    assert_eq!(render(op_box_float(2.0), &Shape::Float), "2.0");
}

#[test]
fn empty_arr_is_unit() {
    reset();
    let a = op_arr_alloc(0);
    assert_eq!(op_arr_len(a), 0);
    assert_eq!(render(a, &Shape::Tuple(vec![].into())), "unit");
}

#[test]
fn arr_two_elements() {
    reset();
    let a = op_arr_alloc(2);
    assert_eq!(op_arr_set(a, 0, op_box_int(3)), a); // arr-set returns the array handle
    op_arr_set(a, 1, op_box_int(1));
    assert_eq!(op_arr_len(a), 2);
    assert_eq!(op_get_int(op_arr_get(a, 0)), 3);
    assert_eq!(op_get_int(op_arr_get(a, 1)), 1);
}

#[test]
fn same_bytes_different_render() {
    reset();
    // The load-bearing demonstration: identical heap node, DIFFERENT canonical text, chosen
    // entirely by the compiler-held static shape — no runtime tag involved.
    let a = op_arr_alloc(2);
    op_arr_set(a, 0, op_box_int(3));
    op_arr_set(a, 1, op_box_int(1));
    assert_eq!(
        render(a, &Shape::Tuple(vec![Shape::Int, Shape::Int].into())),
        "(tuple 3 1)"
    );
    assert_eq!(render(a, &Shape::List(Box::new(Shape::Int))), "(list 3 1)");
    assert_eq!(
        render(
            a,
            &Shape::Record(vec![("x", Shape::Int), ("y", Shape::Int)].into())
        ),
        "(record (= x 3) (= y 1))"
    );
}

#[test]
fn arr_mixed_element_types() {
    reset();
    let a = op_arr_alloc(2);
    op_arr_set(a, 0, op_box_int(42));
    op_arr_set(a, 1, op_box_bool(true));
    assert_eq!(
        render(a, &Shape::Tuple(vec![Shape::Int, Shape::Bool].into())),
        "(tuple 42 true)"
    );
}

#[test]
fn nested_arr() {
    reset();
    // (tuple 1 (tuple 2 3)) — an arr whose element is itself an arr handle.
    let inner = op_arr_alloc(2);
    op_arr_set(inner, 0, op_box_int(2));
    op_arr_set(inner, 1, op_box_int(3));
    let outer = op_arr_alloc(2);
    op_arr_set(outer, 0, op_box_int(1));
    op_arr_set(outer, 1, inner);
    let shape = Shape::Tuple(vec![
        Shape::Int,
        Shape::Tuple(vec![Shape::Int, Shape::Int].into()),
    ]);
    assert_eq!(render(outer, &shape), "(tuple 1 (tuple 2 3))");
}

#[test]
fn empty_list_renders() {
    reset();
    assert_eq!(
        render(op_arr_alloc(0), &Shape::List(Box::new(Shape::Int))),
        "(list)"
    );
}

#[test]
fn sum_round_trip() {
    reset();
    // A two-variant option-like sum: variant 0 = None (nullary), variant 1 = Some(Int).
    let payload = op_box_int(7);
    let some = op_sum_new(1, payload);
    assert_eq!(op_sum_disc(some), 1);
    assert_eq!(op_sum_payload(some), payload);
    assert_eq!(op_get_int(op_sum_payload(some)), 7);

    let variants = || {
        Shape::Sum(vec![
            ("None", Shape::Tuple(vec![].into())),
            ("Some", Shape::Int),
        ])
    };
    assert_eq!(render(some, &variants()), "(Some 7)");

    // disc 0 with an empty-arr payload = a nullary variant carrying unit.
    let none = op_sum_new(0, op_arr_alloc(0));
    assert_eq!(op_sum_disc(none), 0);
    assert_eq!(render(none, &variants()), "(None unit)");
}

#[test]
fn bytes_empty() {
    reset();
    let b = op_bytes_alloc(0);
    assert_eq!(op_bytes_len(b), 0);
    assert_eq!(render(b, &Shape::Bytes), "b\"\"");
}

#[test]
fn bytes_round_trip() {
    reset();
    let b = op_bytes_alloc(3);
    assert_eq!(op_bytes_set(b, 0, 1), b); // bytes-set returns the buffer handle
    op_bytes_set(b, 1, 2);
    op_bytes_set(b, 2, 255);
    assert_eq!(op_bytes_len(b), 3);
    assert_eq!(op_bytes_get(b, 0), 1);
    assert_eq!(op_bytes_get(b, 1), 2);
    assert_eq!(op_bytes_get(b, 2), 255);
    // Non-printable bytes render as `\xNN` (lowercase, matching the `bytes` crate's `Debug`).
    assert_eq!(render(b, &Shape::Bytes), "b\"\\x01\\x02\\xff\"");
}

#[test]
fn bytes_alloc_small_is_inline_large_is_heap_and_both_round_trip() {
    reset();
    let before = live_nodes();
    // Fill a buffer of `len` with 0..len and verify set/get/len round-trip.
    let make = |len: u32| -> Handle {
        let b = op_bytes_alloc(len);
        for i in 0..len {
            op_bytes_set(b, i, (i & 0xff) as u32);
        }
        b
    };
    // Small (<= cap): inline raw.
    let small = make(INLINE_RAW_CAP as u32); // exactly at the boundary — still inline
    assert!(
        !raw_is_heap(small),
        "a <=cap bytes leaf has an INLINE raw (no transient Vec allocated)"
    );
    assert_eq!(op_bytes_len(small), INLINE_RAW_CAP as u32);
    for i in 0..INLINE_RAW_CAP as u32 {
        assert_eq!(op_bytes_get(small, i), i, "small leaf byte {i} round-trips");
    }
    // Large (> cap): heap raw.
    let large = make(INLINE_RAW_CAP as u32 + 5);
    assert!(raw_is_heap(large), "a >cap bytes leaf spills to a heap raw");
    for i in 0..(INLINE_RAW_CAP as u32 + 5) {
        assert_eq!(
            op_bytes_get(large, i),
            i & 0xff,
            "large leaf byte {i} round-trips"
        );
    }
    // A small leaf must compare/hash EQUAL to a fresh twin (canonical rep, one value).
    let small2 = make(INLINE_RAW_CAP as u32);
    assert!(
        champ_eq(small, small2),
        "two identically-built small leaves are champ_eq"
    );
    assert_eq!(
        champ_hash(small),
        champ_hash(small2),
        "…and hash identically"
    );
    op_drop(small);
    op_drop(small2);
    op_drop(large);
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the small/large leaf builds"
    );
}

#[test]
fn bytes_mixed_printable_and_escapes() {
    reset();
    // A mix of printable ASCII, a special escape, and a hex byte: "AB", newline, 0xff.
    let src: [u8; 4] = [b'A', b'B', b'\n', 0xff];
    let b = op_bytes_alloc(src.len() as u32);
    for (i, &v) in src.iter().enumerate() {
        op_bytes_set(b, i as u32, v as u32);
    }
    assert_eq!(render(b, &Shape::Bytes), "b\"AB\\n\\xff\"");
}

#[test]
fn bytes_escapes_quote_backslash_null() {
    reset();
    // `"`, `\`, NUL must escape; the PNG magic 0x89 is a hex byte.
    let src: [u8; 4] = [b'"', b'\\', 0x00, 0x89];
    let b = op_bytes_alloc(src.len() as u32);
    for (i, &v) in src.iter().enumerate() {
        op_bytes_set(b, i as u32, v as u32);
    }
    assert_eq!(render(b, &Shape::Bytes), "b\"\\\"\\\\\\0\\x89\"");
}

#[test]
fn str_round_trip() {
    reset();
    for s in ["", "hello", "héllo☃"] {
        assert_eq!(op_str_get(op_str_new(s.to_string())), s);
    }
    assert_eq!(render(op_str_new("".to_string()), &Shape::Str), "\"\"");
    assert_eq!(
        render(op_str_new("hello".to_string()), &Shape::Str),
        "\"hello\""
    );
    assert_eq!(
        render(op_str_new("héllo☃".to_string()), &Shape::Str),
        "\"héllo☃\""
    );
}

#[test]
fn str_get_on_a_rope_returns_the_flattened_content() {
    reset();
    let before = live_nodes();
    let leaf = |s: &str| {
        let b = op_bytes_alloc(s.len() as u32);
        for (i, &by) in s.as_bytes().iter().enumerate() {
            op_bytes_set(b, i as u32, by as u32);
        }
        b
    };
    // "caf" + "é" (é = 0xC3 0xA9, spanning the seam) + "XY" — the `String.concat` shape → "caféXY".
    let rope = op_bytes_concat(leaf("caf"), leaf("é"));
    let rope = op_bytes_concat(rope, leaf("XY"));
    assert_eq!(
        op_str_get(rope),
        "caféXY",
        "a rope String reads back its logical content, not header bytes"
    );
    // Identical to the flat twin (the whole point of the flatten).
    let flat = op_str_new(String::from("caféXY"));
    assert_eq!(op_str_get(rope), op_str_get(flat));
    op_drop(flat);
    op_drop(rope);
    // An empty rope (concat of two empties) reads as "" — the degenerate case.
    let empty_rope = op_bytes_concat(op_bytes_alloc(0), op_bytes_alloc(0));
    assert_eq!(op_str_get(empty_rope), "");
    op_drop(empty_rope);
    assert_eq!(live_nodes(), before, "no leak: ropes + twin dropped");
}

#[test]
fn str_from_bytes_validates_utf8_and_is_a_byte_leaf() {
    reset();
    let before = live_nodes();
    // (1) a valid multi-byte string → a String leaf byte-identical to op_str_new's.
    let ok = op_str_from_bytes(bytes_leaf("café".as_bytes()));
    assert!(ok != Handle::NULL, "valid UTF-8 decodes to Some");
    let twin = op_str_new(String::from("café"));
    assert!(
        champ_eq(ok, twin),
        "a decoded String == op_str_new's leaf (a String IS a byte leaf)"
    );
    assert_eq!(op_str_get(ok), "café", "content round-trips");
    op_drop(ok);
    op_drop(twin);
    // (2) a ROPE input (the runtime-built `Bytes.concat` shape) flattens + validates (é on the seam).
    let rope = op_bytes_concat(bytes_leaf(b"caf"), bytes_leaf("é".as_bytes()));
    let from_rope = op_str_from_bytes(rope);
    assert!(from_rope != Handle::NULL, "a valid rope decodes to Some");
    assert_eq!(
        op_str_get(from_rope),
        "café",
        "the rope flattens to the right content"
    );
    op_drop(from_rope);
    // (3) strict rejection of the three ill-formed classes → NULL (None): invalid lead byte, an
    //     OVERLONG "/" (0xC0 0xAF, should be 1-byte 0x2F), a surrogate D800, a bad continuation.
    for bad in [
        &[0xFFu8][..],           // invalid byte
        &[0xC0, 0xAF][..],       // overlong encoding of '/'
        &[0xED, 0xA0, 0x80][..], // UTF-8-encoded surrogate U+D800
        &[0xE2, 0x28, 0xA1][..], // bad continuation
    ] {
        let n = op_str_from_bytes(bytes_leaf(bad));
        assert_eq!(n, Handle::NULL, "ill-formed UTF-8 {bad:?} → None");
    }
    // (4) the empty buffer (a HEAP leaf, how a 0-element `Bytes.of` builds) is valid "" .
    let empty = op_str_from_bytes(bytes_leaf(b""));
    assert_eq!(op_str_get(empty), "", "empty bytes → empty string (valid)");
    assert!(
        !is_immediate(empty),
        "a heap empty-bytes leaf stays a heap leaf"
    );
    // (4b) the IMMEDIATE-input branch: `op_str_from_bytes` short-circuits an immediate `buf` (the
    // empty-compound constant `imm_unit`, which a 0-length Bytes can also be) by returning it AS the
    // String. The result MUST be canonically interchangeable with a real `""` literal — `champ_eq`
    // decodes an immediate as arity-0/empty-raw, the same as a heap empty leaf, so the two empty-String
    // representations compare EQUAL (a `str-from-bytes` empty used as a map key / in `=` must match a
    // `""` literal, whichever representation each took). Pins that the immediate short-circuit doesn't
    // introduce a second, unequal empty-String form. (The immediate is `op_drop`-safe — a no-op.)
    let from_imm = op_str_from_bytes(imm_unit());
    assert!(
        is_immediate(from_imm),
        "an immediate input passes through as an immediate"
    );
    assert_eq!(op_str_get(from_imm), "", "immediate empty reads as \"\"");
    assert!(
        champ_eq(from_imm, empty),
        "the immediate empty String == the heap empty String (canonical, interchangeable as a key)"
    );
    let real_empty = op_str_new(String::new());
    assert!(
        champ_eq(from_imm, real_empty),
        "the immediate empty String == a `\"\"` literal (op_str_new) — one canonical empty String"
    );
    assert_eq!(
        champ_hash(from_imm),
        champ_hash(real_empty),
        "…and hashes identically, so they dedup as the same map key"
    );
    op_drop(real_empty);
    op_drop(empty);
    // (6) a SHARED (rc==2) ROPE input: `op_str_from_bytes` flattens the rope IN PLACE (content-
    // preserving, so unobservable) and, on valid UTF-8, returns the SAME handle as the String —
    // touching NO refcount. The scenario the compiler can hit: `String.from-bytes` on a rope still
    // referenced by another owner. Verify that (a) the result IS that handle (a valid rope-turned-leaf
    // String), (b) the refcount is CONSERVED (still 2 — one logical ref consumed by the return, the
    // other owner's intact), (c) the OTHER owner still reads the correct content (the in-place flatten
    // didn't corrupt its view), (d) both refs drop cleanly with no leak/double-free. A rope String and
    // a flat String of the same content are champ_eq (a String IS a byte leaf).
    let shared = op_bytes_concat(bytes_leaf(b"caf"), bytes_leaf("é".as_bytes())); // valid UTF-8 rope
    op_dup(shared); // rc == 2: two owners
    let as_str = op_str_from_bytes(shared); // owner A's call
    assert_eq!(
        as_str, shared,
        "valid rope → returned AS the same handle (a String IS the byte leaf)"
    );
    assert_eq!(
        node_rc(shared),
        2,
        "refcount conserved — the other owner's reference is intact"
    );
    assert_eq!(
        op_str_get(shared),
        "café",
        "the OTHER owner still reads the correct content (in-place flatten is unobservable)"
    );
    let flat_twin = op_str_new(String::from("café"));
    assert!(
        champ_eq(as_str, flat_twin),
        "the from-bytes rope String == a flat String of the same content"
    );
    op_drop(flat_twin);
    op_drop(as_str); // owner A's ref (== shared)
    op_drop(shared); // owner B's ref
    // (5) balance: every buffer consumed (valid ones dropped, invalid ones released internally). The
    // immediate `from_imm` needs no drop (an immediate holds no node — dropping it is a no-op).
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across str-from-bytes"
    );
}

#[test]
fn bytes_scalar_at_reads_the_nth_unicode_scalar_by_codepoint() {
    reset();
    let before = live_nodes();
    const NO_SCALAR: u32 = u32::MAX;
    // (1) ASCII "abc": scalars 'a','b','c' = 97,98,99; index 3 is out of range.
    let a = op_str_new(String::from("abc"));
    assert_eq!(op_bytes_scalar_at(a, 0), 97, "abc scalar 0 = 'a'");
    assert_eq!(op_bytes_scalar_at(a, 2), 99, "abc scalar 2 = 'c'");
    assert_eq!(
        op_bytes_scalar_at(a, 3),
        NO_SCALAR,
        "abc scalar 3 out of range"
    );
    // (6) BORROW: `a` survives the reads (no consume) and still reads.
    assert_eq!(op_str_get(a), "abc", "buf survives scalar-at (borrowed)");
    op_drop(a);
    // (2) MULTI-BYTE "café": byte-len 5 but scalar-len 4 — scalar 3 = 'é' (233), a 2-byte encoding at
    //     BYTE offset 3. Proves the SCALAR index is not the byte index.
    let cafe = op_str_new(String::from("café"));
    assert_eq!(op_bytes_len(cafe), 5, "café is 5 BYTES");
    assert_eq!(op_bytes_scalar_at(cafe, 0), 99, "café scalar 0 = 'c'");
    assert_eq!(
        op_bytes_scalar_at(cafe, 3),
        233,
        "café scalar 3 = 'é' (233), NOT byte 3"
    );
    assert_eq!(
        op_bytes_scalar_at(cafe, 4),
        NO_SCALAR,
        "café has only 4 scalars"
    );
    op_drop(cafe);
    // (3) a 4-byte scalar: "a😀b" — scalar 1 = U+1F600 (128512), scalar 2 = 'b' (98).
    let emoji = op_str_new(String::from("a😀b"));
    assert_eq!(op_bytes_scalar_at(emoji, 0), 97, "a😀b scalar 0 = 'a'");
    assert_eq!(
        op_bytes_scalar_at(emoji, 1),
        128512,
        "a😀b scalar 1 = U+1F600"
    );
    assert_eq!(
        op_bytes_scalar_at(emoji, 2),
        98,
        "a😀b scalar 2 = 'b' (past the 4-byte scalar)"
    );
    op_drop(emoji);
    // (4) a ROPE input (Bytes.concat with é split onto the second leaf) must flatten then decode.
    let rope = op_bytes_concat(bytes_leaf(b"caf"), bytes_leaf("é".as_bytes()));
    assert_eq!(
        op_bytes_scalar_at(rope, 3),
        233,
        "rope scalar 3 = 'é' after flatten across the seam"
    );
    op_drop(rope);
    // (5) empty leaf + immediate → the sentinel (no scalars).
    let empty = op_bytes_alloc(0);
    assert_eq!(
        op_bytes_scalar_at(empty, 0),
        NO_SCALAR,
        "empty has no scalar 0"
    );
    op_drop(empty);
    assert_eq!(
        op_bytes_scalar_at(imm_unit(), 0),
        NO_SCALAR,
        "an immediate has no scalars"
    );
    assert_eq!(
        live_nodes(),
        before,
        "no leak (scalar-at borrows, never consumes)"
    );
}

#[test]
fn map_empty() {
    reset();
    let m = op_map_alloc(0);
    assert_eq!(op_map_len(m), 0);
}

#[test]
fn map_round_trip() {
    reset();
    // { "a" -> 1, "b" -> 2 } as positional pairs; stored verbatim, no sort/dedup.
    let ka = op_str_new("a".to_string());
    let va = op_box_int(1);
    let kb = op_str_new("b".to_string());
    let vb = op_box_int(2);
    let m = op_map_alloc(2);
    assert_eq!(op_map_set(m, 0, ka, va), m); // map-set returns the map handle
    op_map_set(m, 1, kb, vb);
    assert_eq!(op_map_len(m), 2);
    assert_eq!(op_str_get(op_map_key(m, 0)), "a");
    assert_eq!(op_get_int(op_map_val(m, 0)), 1);
    assert_eq!(op_str_get(op_map_key(m, 1)), "b");
    assert_eq!(op_get_int(op_map_val(m, 1)), 2);
}

#[test]
fn deeply_nested_render() {
    reset();
    // (record (= xs (list 1 2)) (= tag (Some 9)) (= raw b"\x07") (= name "hi"))
    let xs = op_arr_alloc(2);
    op_arr_set(xs, 0, op_box_int(1));
    op_arr_set(xs, 1, op_box_int(2));
    let tag = op_sum_new(1, op_box_int(9));
    let raw = op_bytes_alloc(1);
    op_bytes_set(raw, 0, 7);
    let name = op_str_new("hi".to_string());
    let rec = op_arr_alloc(4);
    op_arr_set(rec, 0, xs);
    op_arr_set(rec, 1, tag);
    op_arr_set(rec, 2, raw);
    op_arr_set(rec, 3, name);
    let shape = Shape::Record(vec![
        ("xs", Shape::List(Box::new(Shape::Int))),
        (
            "tag",
            Shape::Sum(vec![
                ("None", Shape::Tuple(vec![].into())),
                ("Some", Shape::Int),
            ]),
        ),
        ("raw", Shape::Bytes),
        ("name", Shape::Str),
    ]);
    assert_eq!(
        render(rec, &shape),
        "(record (= xs (list 1 2)) (= tag (Some 9)) (= raw b\"\\x07\") (= name \"hi\"))"
    );
}

#[test]
fn value_encode_record_with_heap_valued_fields_matches_recursive_reference() {
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
    fn nm(o: &mut Vec<u8>, s: &str) {
        leb(o, s.len() as u64);
        o.extend_from_slice(s.as_bytes());
    }
    // Descriptor table: [0]=Int, [1]=List(→0), [2]=Sum[None→3, Some→0], [3]=Unit, [4]=Bytes, [5]=Str,
    // [6]=Record{xs→1, tag→2, raw→4, name→5}. root=6.
    let mut d = Vec::new();
    leb(&mut d, 7);
    d.push(0); // [0] Int
    d.push(7);
    leb(&mut d, 0); // [1] List(→0)
    d.push(9);
    leb(&mut d, 2);
    nm(&mut d, "None");
    leb(&mut d, 3);
    nm(&mut d, "Some");
    leb(&mut d, 0); // [2] Sum
    d.push(5); // [3] Unit
    d.push(4); // [4] Bytes
    d.push(3); // [5] Str
    d.push(8);
    leb(&mut d, 4);
    nm(&mut d, "xs");
    leb(&mut d, 1);
    nm(&mut d, "tag");
    leb(&mut d, 2);
    nm(&mut d, "raw");
    leb(&mut d, 4);
    nm(&mut d, "name");
    leb(&mut d, 5); // [6] Record
    leb(&mut d, 6); // root
    // The value: (record (= xs (list 1 2)) (= tag (Some 9)) (= raw b"\x07") (= name "hi")). `xs` is a REAL
    // RRB vec (Shape::List reads via vec-get), the others heap sum/bytes/str leaves.
    let xs = {
        let mut v = op_vec_empty();
        v = op_vec_push(v, op_box_int(1));
        v = op_vec_push(v, op_box_int(2));
        v
    };
    let tag = op_sum_new(1, op_box_int(9));
    let raw = {
        let b = op_bytes_alloc(1);
        op_bytes_set(b, 0, 7);
        b
    };
    let name = op_str_new("hi".to_string());
    let rec = op_arr_alloc(4);
    op_arr_set(rec, 0, xs);
    op_arr_set(rec, 1, tag);
    op_arr_set(rec, 2, raw);
    op_arr_set(rec, 3, name);

    let doc =
        op_value_encode_form(rec, &d).expect("record-with-heap-fields encodes via the escape");

    // (1) DIFFERENTIAL: the recursive oracle must produce byte-identical output.
    let descriptor = decode_descriptor(&d).expect("descriptor decodes");
    let mut b = DocBuilder::default();
    let root = encode_value_recursive(&descriptor, &mut b, rec, descriptor.root, 0)
        .expect("recursive oracle encodes");
    assert_eq!(
        doc,
        b.finish(root),
        "iterative and recursive Record-with-heap-fields encode must agree"
    );

    // (2) INDEPENDENT ANCHOR: parse the leaf pool (each leaf = KIND byte + payload). Collect the NAME
    // leaves (KIND_NAME=10 — the `record`/`list` heads, field keys, Sum variant head) in emission order
    // AND the STR values (KIND_STR=7 — a String VALUE, a DISTINCT kind from a NAME). Field keys must
    // appear in FIELD order (a wrong field interleaving on the work stack would reorder them), and the
    // String field content "hi" must appear as a STR leaf. Uses the codec's actual KIND constants.
    let mut names: Vec<String> = Vec::new();
    let mut strs: Vec<String> = Vec::new();
    let leaf_count = doc[8] as usize;
    let mut i = 9;
    for _ in 0..leaf_count {
        let kind = doc[i];
        i += 1;
        match kind {
            doc::KIND_INT_POS_DEC | 3 => {
                // KIND_INT (pos=0 / neg=3): LEB len + big-endian magnitude
                let len = doc[i] as usize;
                i += 1 + len;
            }
            doc::KIND_NAME => {
                let len = doc[i] as usize;
                i += 1;
                names.push(String::from_utf8(doc[i..i + len].to_vec()).unwrap());
                i += len;
            }
            doc::KIND_STR => {
                let len = doc[i] as usize;
                i += 1;
                strs.push(String::from_utf8(doc[i..i + len].to_vec()).unwrap());
                i += len;
            }
            doc::KIND_BYTES => {
                let len = doc[i] as usize;
                i += 1 + len;
            }
            20..=26 => {} // M2 payloadless ctor-head leaf (20-26)
            k => panic!("unexpected leaf kind {k} in the record document"),
        }
    }
    // Field keys appear in FIELD ORDER (xs < tag < raw < name).
    let field_key_positions: Vec<usize> = ["xs", "tag", "raw", "name"]
        .iter()
        .map(|k| {
            names
                .iter()
                .position(|n| n == k)
                .unwrap_or_else(|| panic!("field key {k} missing from names {names:?}"))
        })
        .collect();
    assert!(
        field_key_positions.windows(2).all(|w| w[0] < w[1]),
        "record field keys appear in FIELD order (xs<tag<raw<name) in {names:?}"
    );
    // The Sum variant head (a NAME) is present. The record/list heads are now M2 ctor LEAVES (kinds
    // 22/20), not names — their presence + structure is verified by part-1's byte-differential
    // (production == recursive oracle) above, so this name check only pins the surviving variant head.
    assert!(
        names.iter().any(|n| n == "Some"),
        "the Sum variant head `Some` is present in {names:?}"
    );
    // The String field VALUE "hi" is emitted as a KIND_STR leaf (distinct from the NAME leaves).
    assert!(
        strs.iter().any(|s| s == "hi"),
        "the String field value \"hi\" is emitted as a STR leaf in {strs:?}"
    );

    op_drop(rec);
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the record-with-heap-fields encode"
    );
}

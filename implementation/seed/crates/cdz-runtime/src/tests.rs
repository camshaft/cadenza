use super::*;

/// No shared table to clear — every value is its own allocation and every test holds the handles
/// it builds. Kept as a documented no-op so each test reads as a self-contained scenario.
fn reset() {}

/// The `IntList` shape descriptor `(type IL (Cons (Tuple Int64 IL)) Nil)`, wrapped in the outer
/// `(: <value> IL)` frame — a TABLE with a self-`Ref` closing the recursion (as the compiler bakes
/// it). Table: [0]=Int, [1]=Sum[(Cons→2),(Nil→3)], [2]=Tuple[→0,→1], [3]=Unit, [4]=Named("IL"→1);
/// root=4. The `Cons` payload tuple's second element (→1) points back at the Sum — a finite 1-entry
/// cycle the value walk unfolds to the value's depth.
fn intlist_descriptor() -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut v: u64) {
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
    }
    fn name(out: &mut Vec<u8>, s: &str) {
        leb(out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }
    let mut d = Vec::new();
    leb(&mut d, 5); // table_len = 5
    // [0] Int
    d.push(0);
    // [1] Sum [(Cons → 2), (Nil → 3)]
    d.push(9);
    leb(&mut d, 2);
    name(&mut d, "Cons");
    leb(&mut d, 2);
    name(&mut d, "Nil");
    leb(&mut d, 3);
    // [2] Tuple [→0, →1]
    d.push(6);
    leb(&mut d, 2);
    leb(&mut d, 0);
    leb(&mut d, 1);
    // [3] Unit
    d.push(5);
    // [4] Named("IL" → 1)
    d.push(10);
    name(&mut d, "IL");
    leb(&mut d, 1);
    leb(&mut d, 4); // root = 4
    d
}

/// A minimal `(Set Int64)` descriptor: table [0]=Int, [1]=Set(→0); root=1. Set tag = 12.
fn set_int_descriptor() -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut v: u64) {
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
    }
    let mut d = Vec::new();
    leb(&mut d, 2); // table_len = 2
    d.push(0); // [0] Int
    d.push(12); // [1] Set(→0)
    leb(&mut d, 0);
    leb(&mut d, 1); // root = 1
    d
}

/// A minimal `(Map Int64 Int64)` descriptor: table [0]=Int, [1]=Map(key→0, val→0); root=1. Map tag = 13.
fn map_int_int_descriptor() -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut v: u64) {
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
    }
    let mut d = Vec::new();
    leb(&mut d, 2); // table_len = 2
    d.push(0); // [0] Int
    d.push(13); // [1] Map(key→0, val→0)
    leb(&mut d, 0);
    leb(&mut d, 0);
    leb(&mut d, 1); // root = 1
    d
}

/// `set-to-list` enumerates a set's elements as a `List` in CANONICAL element-value order — NOT the
/// CHAMP hash/insertion order — reusing the SAME sorted walk value-encode renders `(Set.of …)` from.
/// Insert ints in a deliberately non-sorted order; the result list must be `[1, 2, 3, 10]`, and the
/// heap must balance (each element is `dup`'d into the owned result vec; dropping the set + the list
/// nets to zero live objects).
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

/// A `(Set (Tuple Int64 Int64))` descriptor: table [0]=Int, [1]=Tuple[→0,→0], [2]=Set(→1); root=2.
/// Set tag = 12, Tuple tag = 6. Elements are ORDERABLE COMPOUNDS — `set-to-list` must sort them by the
/// SAME lexicographic total order `value_cmp_shaped` (== the runtime `<`) supplies, not decline.
fn set_tuple_int_int_descriptor() -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut v: u64) {
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
    }
    let mut d = Vec::new();
    leb(&mut d, 3); // table_len = 3
    d.push(0); // [0] Int
    d.push(6); // [1] Tuple [→0, →0]
    leb(&mut d, 2);
    leb(&mut d, 0);
    leb(&mut d, 0);
    d.push(12); // [2] Set(→1)
    leb(&mut d, 1);
    leb(&mut d, 2); // root = 2
    d
}

/// `set-to-list` over a set whose elements are ORDERABLE COMPOUNDS (`(Tuple Int64 Int64)`) enumerates them
/// in canonical LEXICOGRAPHIC element order — the SAME total order the runtime `<`/`Core::ValueCmp` walk
/// (`value_cmp_shaped`) supplies — NOT a decline. This is breaker's differential repro 10761: wasm used to
/// false-decline a compound-element set (the scalar-only guard), while the value form + rust computed the
/// order. Insert `(3,1),(1,2),(2,0)` in hash order; the result must be the lexicographic order
/// `(1,2),(2,0),(3,1)` (first component decisive, second breaking a tie), and the heap must balance.
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

/// `map-to-list` enumerates a map's entries as a `List (Tuple k v)` in CANONICAL KEY order, each entry
/// a 2-element tuple `[key, value]`. Insert keys out of order; the result must be the entries sorted by
/// key, with values intact, and the heap must balance.
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

/// A non-scalar (unorderable) element/key shape, or a descriptor whose root is not a Set/Map, DECLINES
/// to the EMPTY list — the never-trap totality contract (the compiler bakes only a well-formed
/// descriptor, but the op must be total on any input). Here a `(Set Int64)` value handed a MISMATCHED
/// descriptor whose root is a bare `Int` (not a Set) yields the empty list, not a trap.
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

/// The root-`Framed` plain-Tuple descriptor `(: <value> (Tuple Int64 Int64))` — a tag-15 `Framed`
/// whose TypeNode is `Tuple` with two `Int64` children, inner → a `Tuple[→Int, →Int]` table entry.
/// This is the descriptor `sum_shape_descriptor` bakes for a `Value.encode` of a two-int tuple (the
/// PUBLIC value-encode path frames the compound; the fold/reducer boundary's `bare_shape_descriptor`
/// does NOT — see rcdzc `sum_shape_descriptor` vs `bare_shape_descriptor`). Kept as a test constant.
fn framed_int_pair_descriptor() -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut v: u64) {
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
    }
    fn name(out: &mut Vec<u8>, s: &str) {
        leb(out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }
    let mut d = Vec::new();
    leb(&mut d, 3); // table_len = 3
    // [0] Int
    d.push(0);
    // [1] Tuple [→0, →0]
    d.push(6);
    leb(&mut d, 2);
    leb(&mut d, 0);
    leb(&mut d, 0);
    // [2] Framed( TypeNode Tuple[Int64, Int64], inner → 1 )
    d.push(15);
    // TypeNode: head "Tuple", 2 children each head "Int64" with 0 children
    name(&mut d, "Tuple");
    leb(&mut d, 2);
    name(&mut d, "Int64");
    leb(&mut d, 0);
    name(&mut d, "Int64");
    leb(&mut d, 0);
    leb(&mut d, 1); // inner → 1 (the Tuple shape)
    leb(&mut d, 2); // root = 2 (the Framed)
    d
}

/// GOLDEN + cross-backend divergence pin: `Value.encode` of a two-int tuple must render the
/// `(: (tuple 5 105) (Tuple Int64 Int64))` COLON-FRAMED typed document — NOT the bare `(tuple 5 105)`.
/// This is the exact shape a native-backend divergence surfaced on (v-rust-backend, 2026-08-16: the
/// native codec emitted the bare 35-byte form vs this framed form; reviewer flagged that no standing
/// pin caught a cross-backend `Value.encode` byte divergence, an invariant v-runtime owns). Guards the
/// framed-root walk three ways: iterative == recursive oracle, decode ∘ encode == id, and the exact
/// golden bytes. A native backend (rcdzc rust / cadenza-ast) mirrors this same golden constant so the
/// two codecs are pinned to one byte string. `#[cfg(test)]` → does not touch the runtime hash.
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

/// The root-`Framed` two-field-record descriptor `(: <value> (Record (a Int64) (b Int64)))` —
/// tag-15 `Framed` whose TypeNode is `record` with two field children (`a`→`Int64`, `b`→`Int64`,
/// each a `(name <type>)` node), inner → a `Record[a→0, b→0]` table entry. This is what
/// `sum_shape_descriptor` bakes for a `Value.encode` of a two-`Int64`-field record.
fn framed_int_record_descriptor() -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut v: u64) {
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
    }
    fn name(out: &mut Vec<u8>, s: &str) {
        leb(out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }
    let mut d = Vec::new();
    leb(&mut d, 3); // table_len = 3
    // [0] Int
    d.push(0);
    // [1] Record [ a→0, b→0 ]
    d.push(8);
    leb(&mut d, 2);
    name(&mut d, "a");
    leb(&mut d, 0);
    name(&mut d, "b");
    leb(&mut d, 0);
    // [2] Framed( TypeNode record[ a[Int64], b[Int64] ], inner → 1 )
    d.push(15);
    // TypeNode: head "record", 2 children, each a field node (head = field name, 1 child = "Int64").
    name(&mut d, "record");
    leb(&mut d, 2);
    name(&mut d, "a");
    leb(&mut d, 1);
    name(&mut d, "Int64");
    leb(&mut d, 0);
    name(&mut d, "b");
    leb(&mut d, 1);
    name(&mut d, "Int64");
    leb(&mut d, 0);
    leb(&mut d, 1); // inner → 1 (the Record shape)
    leb(&mut d, 2); // root = 2 (the Framed)
    d
}

/// GOLDEN + cross-backend divergence pin, RECORD shape (v-rust-backend fixture 1, 2026-08-16): a
/// `Value.encode` of `(record (= a 5) (= b 105))` at `(Record (a Int64) (b Int64))` must render the
/// colon-framed `(: (record (= a 5) (= b 105)) (Record (a Int64) (b Int64)))` doc. The cadenza-ast
/// mirror (`codec::tests`) asserts the SAME byte string, so the two codecs are pinned together. Guarded
/// three ways (iterative == recursive oracle, decode∘encode == id, exact leaf pool). `#[cfg(test)]`.
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

/// A small LEB + length-prefixed-name descriptor builder, shared by the framed-Sum goldens below.
fn desc_leb(out: &mut Vec<u8>, mut v: u64) {
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
}
fn desc_name(out: &mut Vec<u8>, s: &str) {
    desc_leb(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// The generic-Sum descriptor `(: <value> (Option Int64))` — a boxed GENERIC sum (`args` non-empty)
/// roots at a PARAMETRIC `Framed(TypeNode Option[Int64], inner)`. Table: [0] Int, [1] Unit (the None
/// payload), [2] Sum[(Some→0),(None→1)], [3] Framed(Option[Int64] → 2); root = 3.
fn framed_option_int_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 4); // table_len = 4
    d.push(0); // [0] Int
    d.push(5); // [1] Unit
    // [2] Sum [ (Some → 0), (None → 1) ]
    d.push(9);
    desc_leb(&mut d, 2);
    desc_name(&mut d, "Some");
    desc_leb(&mut d, 0);
    desc_name(&mut d, "None");
    desc_leb(&mut d, 1);
    // [3] Framed( TypeNode Option[ Int64 ], inner → 2 )
    d.push(15);
    desc_name(&mut d, "Option");
    desc_leb(&mut d, 1);
    desc_name(&mut d, "Int64");
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 2); // inner → 2 (the Sum)
    desc_leb(&mut d, 3); // root = 3 (the Framed)
    d
}

/// GOLDEN pin, GENERIC-SUM shape (v-rust-backend fixtures 2+3, 2026-08-16): `Value.encode` of
/// `(Some 5)` / `None` at `(Option Int64)` must render the colon-framed doc with a PARAMETRIC `Option`
/// type node — `(: (Some 5) (Option Int64))` and `(: (None unit) (Option Int64))`. Guarded three ways
/// each (iterative == recursive oracle, decode∘encode == id, exact leaf pool). The cadenza-ast mirror
/// asserts the same byte strings. `#[cfg(test)]` → no runtime-hash change.
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

/// The monomorphic-Sum descriptor `(: <value> Shape)` where `Shape = (Circle Int64) | (Rect Int64
/// Int64)` — a MONOMORPHIC sum (`args: []`) roots at a bare-name `Named("Shape", inner)`, NOT a
/// parametric `Framed`. A multi-payload variant's payload is a `Spread` (its elements splice flat).
/// Table: [0] Int, [1] Spread[→0,→0] (Rect's two Int64s), [2] Sum[(Circle→0),(Rect→1)],
/// [3] Named("Shape" → 2); root = 3.
fn named_shape_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 4); // table_len = 4
    d.push(0); // [0] Int
    // [1] Spread [→0, →0]
    d.push(16);
    desc_leb(&mut d, 2);
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 0);
    // [2] Sum [ (Circle → 0), (Rect → 1) ]
    d.push(9);
    desc_leb(&mut d, 2);
    desc_name(&mut d, "Circle");
    desc_leb(&mut d, 0);
    desc_name(&mut d, "Rect");
    desc_leb(&mut d, 1);
    // [3] Named( "Shape", inner → 2 )
    d.push(10);
    desc_name(&mut d, "Shape");
    desc_leb(&mut d, 2);
    desc_leb(&mut d, 3); // root = 3 (the Named)
    d
}

/// GOLDEN pin, MONOMORPHIC-multi-payload-SUM shape (v-rust-backend fixture 4, 2026-08-16):
/// `Value.encode` of `(Rect 5 6)` at `Shape` must render `(: (Rect 5 6) Shape)` — the frame is a
/// bare-name `Named` (NOT a parametric type node, because `Shape` is monomorphic), and `Rect`'s two
/// payloads splice FLAT (a `Spread`). This exercises the Named-vs-Framed root distinction the earlier
/// goldens don't. Guarded three ways; the cadenza-ast mirror asserts the same bytes. `#[cfg(test)]`.
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

/// The framed Int×Float tuple descriptor `(: <value> (Tuple Int64 Float64))` — tag-15 `Framed` whose
/// TypeNode is `Tuple[Int64, Float64]`, inner → a `Tuple[→Int, →Float]`. Exercises the FLOAT leaf
/// (KIND_FLOAT exact-decimal) inside the framed cross-backend golden.
fn framed_int_float_pair_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 4); // table_len = 4 (Int, Float, Tuple, Framed)
    d.push(0); // [0] Int
    d.push(2); // [1] Float
    // [2] Tuple [→0, →1]
    d.push(6);
    desc_leb(&mut d, 2);
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 1);
    // [3] Framed( TypeNode Tuple[ Int64, Float64 ], inner → 2 )
    d.push(15);
    desc_name(&mut d, "Tuple");
    desc_leb(&mut d, 2);
    desc_name(&mut d, "Int64");
    desc_leb(&mut d, 0);
    desc_name(&mut d, "Float64");
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 2); // inner → 2
    desc_leb(&mut d, 3); // root = 3
    d
}

/// GOLDEN pin, FLOAT shape (v-rust-backend fixture, 2026-08-16): `Value.encode` of `(tuple 5 2.5)` at
/// `(Tuple Int64 Float64)` must render `(: (tuple 5 2.5) (Tuple Int64 Float64))` — the 2.5 is a
/// KIND_FLOAT exact-decimal leaf (Decimal false/25/-1, i.e. 25×10⁻¹), NOT a lossy f64 bit pattern. This
/// pins the exact-decimal Float leaf identity that the 3 codecs (runtime/rcdzc/cadenza-ast) share; the
/// cadenza-ast mirror (`1712ab8d7`) asserts the same bytes. Guarded three ways. `#[cfg(test)]`.
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

/// The framed Map descriptor `(: <value> (Map Int64 Int64))` — tag-15 `Framed` whose TypeNode is
/// `Map[Int64, Int64]`, inner → a `Map(key→0, val→0)` (tag 13).
fn framed_int_map_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 3); // table_len = 3
    d.push(0); // [0] Int
    // [1] Map [ key→0, val→0 ]
    d.push(13);
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 0);
    // [2] Framed( TypeNode Map[ Int64, Int64 ], inner → 1 )
    d.push(15);
    desc_name(&mut d, "Map");
    desc_leb(&mut d, 2);
    desc_name(&mut d, "Int64");
    desc_leb(&mut d, 0);
    desc_name(&mut d, "Int64");
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 1); // inner → 1
    desc_leb(&mut d, 2); // root = 2
    d
}

/// The framed Set descriptor `(: <value> (Set Int64))` — tag-15 `Framed` whose TypeNode is
/// `Set[Int64]`, inner → a `Set(elem→0)` (tag 12).
fn framed_int_set_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 3); // table_len = 3
    d.push(0); // [0] Int
    // [1] Set [ elem→0 ]
    d.push(12);
    desc_leb(&mut d, 0);
    // [2] Framed( TypeNode Set[ Int64 ], inner → 1 )
    d.push(15);
    desc_name(&mut d, "Set");
    desc_leb(&mut d, 1);
    desc_name(&mut d, "Int64");
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 1); // inner → 1
    desc_leb(&mut d, 2); // root = 2
    d
}

/// GOLDEN pin, MAP shape (v-rust-backend fixture, 2026-08-16): `Value.encode` of
/// `(Map.insert (Map.insert Map.empty 7 70) 8 99)` at `(Map Int64 Int64)` must render
/// `(: (map (7 70) (8 99)) (Map Int64 Int64))` — the entries in CANONICAL KEY ORDER (7 before 8,
/// regardless of insert order), the value head `map` distinct from the type node's `Map`. Pins the
/// member-order-at-build contract (the runtime's canonical map iteration). Guarded three ways; the
/// cadenza-ast mirror asserts the same bytes. `#[cfg(test)]`.
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

/// GOLDEN pin, SET shape (v-rust-backend fixture, 2026-08-16): `Value.encode` of
/// `(Set.of (list 7 12 17))` at `(Set Int64)` must render `(: ((. Set of) (list 7 12 17)) (Set
/// Int64))` — the `((. Set of) (list …))` member-access form, elements in CANONICAL order. Pins the
/// Set member-order-at-build contract. Guarded three ways; the cadenza-ast mirror asserts the same
/// bytes. `#[cfg(test)]`.
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

/// The framed BigInt descriptor `(: <value> BigInt)` — tag-15 `Framed` whose TypeNode is a bare-leaf
/// `BigInt`, inner → a `BigInt` (tag 17). A BigInt renders as a plain KIND_INT leaf.
fn framed_bigint_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 2); // table_len = 2
    d.push(17); // [0] BigInt
    // [1] Framed( TypeNode 'BigInt' (0 children), inner → 0 )
    d.push(15);
    desc_name(&mut d, "BigInt");
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 0); // inner → 0
    desc_leb(&mut d, 1); // root → 1
    d
}

/// GOLDEN pin, BIGINT shape (v-rust-backend fixture, 2026-08-16; the last leaf gap, native codec
/// `df50352da`): `Value.encode` of `(BigInt.of 5)` at `BigInt` must render `(: 5 BigInt)` — the BigInt
/// is a plain KIND_INT leaf (byte-identical to a boxed int 5). Guarded three ways; the cadenza-ast
/// mirror asserts the same bytes. `#[cfg(test)]`.
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

/// The framed Rational descriptor `(: <num>/<den> Rational)` — tag-18 `Rational` leaf, wrapped in a
/// tag-15 `Framed` whose TypeNode is the childless name `Rational` (mirrors the BigInt descriptor:
/// one scalar-ish leaf under one frame).
fn framed_rational_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 2); // table_len = 2
    d.push(18); // [0] Rational
    // [1] Framed( TypeNode 'Rational' (0 children), inner → 0 )
    d.push(15);
    desc_name(&mut d, "Rational");
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 0); // inner → 0
    desc_leb(&mut d, 1); // root → 1
    d
}

/// GOLDEN pin, RATIONAL shape (closes the 8+1 cross-backend `Value.encode` byte-identity guard;
/// v-rust-backend landed the native-rust Rational R2 arm on trunk `f62a6dc18`, `backend/rust/expr.rs`
/// `emit_value_form`: `Ty::Rational => __b.name(&val.to_display_string())` — a SINGLE NAME leaf, the
/// exact form this pins). `Value.encode` of `(Rational.of 3 4)` at `Rational` must render
/// `(: 3/4 Rational)` — the value is ONE `num/den` NAME leaf (lowest-terms, sign-on-numerator, den>0),
/// NOT a `(record num den)` and NOT the 2-BigInt-handle heap node. Byte-identical to v-rb's
/// `to_display_string()` NAME leaf by construction. Guarded three ways (iterative==recursive oracle,
/// decode∘encode==id, exact full-document bytes). `#[cfg(test)]`.
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

// A bare `Char` value-codec descriptor: one shape-table entry (tag 19 = Char) at the root. A char
// value is an immediate int codepoint at runtime; the descriptor's tag 19 is the ONLY thing that
// distinguishes it from an `Int` at the encode/decode boundary (it selects the `KIND_CHAR` leaf).
fn char_scalar_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 1); // table_len = 1
    d.push(19); // [0] Char
    desc_leb(&mut d, 0); // root → 0
    d
}

// `(tuple Char Int)` — exercises Char as a non-root child (the walk reaches it through `arr-get`).
fn char_int_tuple_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 3); // table_len = 3
    d.push(19); // [0] Char
    d.push(0); //  [1] Int
    d.push(6); // [2] Tuple(2) → [0, 1]
    desc_leb(&mut d, 2);
    desc_leb(&mut d, 0);
    desc_leb(&mut d, 1);
    desc_leb(&mut d, 2); // root → 2
    d
}

// The `KIND_CHAR` doc leaf op62 must emit for a codepoint `c`: tag then the scalar UTF-8-encoded
// (LEB length + 1..4 bytes), matching `DocBuilder::char_leaf` / cadenza-ast's `KIND_CHAR` framing.
fn kind_char_leaf_bytes(c: char) -> Vec<u8> {
    let mut buf = [0u8; 4];
    let s = c.encode_utf8(&mut buf);
    let mut leaf = Vec::new();
    leaf.push(0x0d); // doc::KIND_CHAR
    desc_leb(&mut leaf, s.len() as u64);
    leaf.extend_from_slice(s.as_bytes());
    leaf
}

/// Char VALUE codec (the tag-19 `Shape::Char` path — distinct from the AST `Leaf::Char` reflection
/// path, and the ONLY witness for it while the corpus producer is still draft): for a spread of
/// codepoints (ASCII, 2-byte, 4-byte, NUL), op62 `value-encode` must (1) emit a `KIND_CHAR` leaf —
/// NOT a `KIND_INT` — which is what makes a char RENDER as a char and not its integer (the round-trip
/// alone can't catch a Char→Int render regression, since a char value IS an int); (2) agree with the
/// recursive oracle byte-for-byte; and (3) round-trip through op90 `value-decode` back to the same
/// codepoint. Char is an immediate int at runtime, so nothing heap-allocates and the census stays 0.
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

/// Char as a compound CHILD: `(tuple #\λ 42)`. The tuple heap node is built, encoded (the char field
/// still emits a KIND_CHAR leaf), decoded back, and the char field recovers its codepoint; dropping
/// the decoded tuple frees clean. Guards that the Char arm fires through `arr-get`, not just at root.
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

/// A lone surrogate (U+D800) is not a Unicode scalar, so `char::from_u32` rejects it: op62's Char arm
/// returns `None` and `value-encode` DECLINES (returns None) rather than trapping or emitting garbage —
/// a bad codepoint is DATA, handled totally. The immediate int leaves the census untouched.
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

/// The ORIGINAL recursive `encode_value`, kept as the differential oracle for the iterative
/// production walk. Byte-for-byte identical logic; the ONLY difference is native recursion vs the
/// production explicit heap stack. A deep value overflows THIS (that is the bug the iterative walk
/// fixes), so the differential test drives it only to modest depth.
fn encode_value_recursive(
    desc: &super::Descriptor,
    b: &mut super::DocBuilder,
    h: Handle,
    shape_ix: u32,
    depth: u32,
) -> Option<u32> {
    use super::Shape as S;
    // Oracle uses TOTAL-depth as its cap (the original recursive semantics); the production walk uses
    // the sharper non-consuming `refs` cap. The differential test drives only shallow values (deep
    // ones overflow THIS recursive oracle — the bug the iterative walk fixes), so the two caps never
    // disagree on the tested inputs, and byte-identity is what the test asserts.
    if depth > ENCODE_REF_CYCLE_CAP {
        return None;
    }
    let shape = desc.table.get(shape_ix as usize)?;
    Some(match shape {
        S::Ref(target) => return encode_value_recursive(desc, b, h, *target, depth + 1),
        S::Int => {
            let l = b.int_leaf(op_get_int(h));
            b.atom(l)
        }
        S::BigInt => {
            let l = b.bigint_leaf(&unbox_bigint(h));
            b.atom(l)
        }
        S::Rational => {
            // seq-204 native head+children (mirrors encode_value): `(KIND_RATIONAL <num> <den>)`.
            let (num, den) = unbox_rational(h);
            let tag_leaf = b.ctor_leaf(doc::KIND_RATIONAL);
            let tag = b.atom(tag_leaf);
            let num_leaf = b.bigint_leaf(&num);
            let num_atom = b.atom(num_leaf);
            let den_leaf = b.bigint_leaf(&den);
            let den_atom = b.atom(den_leaf);
            b.list_head_tail(tag, &[num_atom, den_atom])
        }
        S::Bool => {
            let l = b.bool_leaf(op_get_bool(h));
            b.atom(l)
        }
        S::Char => {
            // Mirror op62's Char arm: a char value is an immediate int (code-point) → KIND_CHAR leaf.
            let c = char::from_u32(op_get_int(h) as u32)?;
            let l = b.char_leaf(c);
            b.atom(l)
        }
        S::Unit => {
            let l = b.name_leaf("unit");
            b.atom(l)
        }
        S::Str => {
            // MATERIALIZE a rope string (concat/slice nodes) to a flat leaf before reading `raw` —
            // exactly as `S::Bytes` does; without it a runtime string rendered its raw handle bytes.
            bytes_flatten(h);
            let bytes = with_node(h, Vec::new(), |n| n.raw.as_slice().to_vec());
            let l = b.str_leaf(&bytes);
            b.atom(l)
        }
        S::Bytes => {
            bytes_flatten(h);
            let bytes = with_node(h, Vec::new(), |n| n.raw.as_slice().to_vec());
            let l = b.bytes_leaf(&bytes);
            b.atom(l)
        }
        S::Float => {
            let l = b.float_leaf(op_get_float(h))?;
            b.atom(l)
        }
        S::Float32 => {
            let l = b.float32_leaf(op_get_float32(h))?;
            b.atom(l)
        }
        S::Tuple(elems) => {
            if elems.is_empty() {
                let l = b.name_leaf("unit");
                return Some(b.atom(l));
            }
            let head = b.ctor_leaf(doc::KIND_TUPLE_CTOR);
            let head_s = b.atom(head);
            let mut children = vec![head_s];
            for (i, &es) in elems.iter().enumerate() {
                children.push(encode_value_recursive(
                    desc,
                    b,
                    op_arr_get(h, i as u32),
                    es,
                    depth + 1,
                )?);
            }
            b.list(&children)
        }
        S::List(elem) => {
            // A Cadenza `List` is an RRB `vec` — read with `vec-len`/`vec-get` (NOT `arr-len`/
            // `arr-get`, which read a vec's root-node arity, not the logical element count). Mirrors
            // the production `Shape::List` arm; the earlier list tests build `Cons(tuple …)` recursive
            // SUMS (arr-based), so this arm was previously unexercised — the Framed(list) test reaches
            // it on a real `vec`.
            let (elem, n) = (*elem, op_vec_len(h));
            let head = b.ctor_leaf(doc::KIND_LIST_CTOR);
            let head_s = b.atom(head);
            let mut children = vec![head_s];
            for i in 0..n {
                children.push(encode_value_recursive(
                    desc,
                    b,
                    op_vec_get(h, i),
                    elem,
                    depth + 1,
                )?);
            }
            b.list(&children)
        }
        S::Record(fields) => {
            let head = b.ctor_leaf(doc::KIND_RECORD_CTOR);
            let head_s = b.atom(head);
            let mut children = vec![head_s];
            for (i, (k, fs)) in fields.iter().enumerate() {
                // CANON CONVERGENCE (mirrors the iterative `VisitField`): build the FieldPair ctor head
                // then the key atom BEFORE recursing into the value, so leaves intern in canon pre-order
                // first-encounter. M2 field form `(FieldPair name value)` (was `(= name value)`).
                let eq_leaf = b.ctor_leaf(doc::KIND_FIELD_PAIR);
                let eq = b.atom(eq_leaf);
                let kname = b.name_leaf(k);
                let katom = b.atom(kname);
                let fval =
                    encode_value_recursive(desc, b, op_arr_get(h, i as u32), *fs, depth + 1)?;
                children.push(b.list(&[eq, katom, fval]));
            }
            b.list(&children)
        }
        S::Sum(variants) => {
            let disc = op_sum_disc(h) as usize;
            let (head, payload_shape) = variants.get(disc)?;
            let (head, payload_shape) = (head.clone(), *payload_shape);
            let head_leaf = b.name_leaf(&head);
            let head_s = b.atom(head_leaf);
            let payload_h = op_sum_payload(h);
            // A MULTI-payload variant's payload is a `Spread` — splice its tuple elements FLAT under the
            // variant head (`(Cons h t)`), mirroring the iterative walk. A single/nullary payload
            // recurses into the one payload shape (`(Cons (tuple h t))` / `(None unit)`).
            if let Some(S::Spread(elems)) = desc.table.get(payload_shape as usize) {
                let elems = elems.clone();
                let mut children = vec![head_s];
                for (i, &es) in elems.iter().enumerate() {
                    children.push(encode_value_recursive(
                        desc,
                        b,
                        op_arr_get(payload_h, i as u32),
                        es,
                        depth + 1,
                    )?);
                }
                b.list(&children)
            } else {
                let payload = encode_value_recursive(desc, b, payload_h, payload_shape, depth + 1)?;
                b.list(&[head_s, payload])
            }
        }
        S::Named(name, inner) => {
            let (name, inner) = (name.clone(), *inner);
            let colon = b.name_leaf(":");
            let colon_s = b.atom(colon);
            let value = encode_value_recursive(desc, b, h, inner, depth + 1)?;
            let tname = b.name_leaf(&name);
            let tname_s = b.atom(tname);
            b.list(&[colon_s, value, tname_s])
        }
        S::Framed(type_node, inner) => {
            // The `(: value <type-node>)` frame — mirrors the iterative walk: colon, the value, then
            // the (possibly nested) type node, then the outer list. The type node is rendered from the
            // baked `TypeNode` (compile-time-known), so it handles arbitrary nesting.
            let inner = *inner;
            let colon = b.name_leaf(":");
            let colon_s = b.atom(colon);
            let value = encode_value_recursive(desc, b, h, inner, depth + 1)?;
            let type_s = b.render_type_node(type_node);
            b.list(&[colon_s, value, type_s])
        }
        S::Set(elem) => {
            let elem = *elem;
            let sorted = set_elements_canonical(desc, h, elem)?;
            // M2 head-first (mirrors the iterative `Set` arm): flat `(Ctor(Set) e1 … en)` — the Set
            // ctor head atom + sorted elements as direct children (was `((. Set of) (list e…))`).
            let head = b.ctor_leaf(doc::KIND_SET_CTOR);
            let head_s = b.atom(head);
            let mut children = vec![head_s];
            for e in sorted {
                children.push(encode_value_recursive(desc, b, e, elem, depth + 1)?);
            }
            b.list(&children)
        }
        S::Map(key, val) => {
            let (key, val) = (*key, *val);
            let entries = map_entries_canonical(desc, h, key)?;
            // M2 head-first (mirrors the iterative `Map` arm): `(Ctor(Map) (FieldPair k v)…)`. Each entry
            // interns its FieldPair ctor head PRE-order (before the k/v subtrees, canon first-encounter;
            // the FieldPair leaf dedups) — was `(map (k v)…)`.
            let head = b.ctor_leaf(doc::KIND_MAP_CTOR);
            let head_s = b.atom(head);
            let mut children = vec![head_s];
            for (k, v) in entries {
                let fp = b.ctor_leaf(doc::KIND_FIELD_PAIR);
                let fp_s = b.atom(fp);
                let ks = encode_value_recursive(desc, b, k, key, depth + 1)?;
                let vs = encode_value_recursive(desc, b, v, val, depth + 1)?;
                children.push(b.list(&[fp_s, ks, vs]));
            }
            b.list(&children)
        }
        S::Spread(elems) => {
            // Reached DIRECTLY (not via a Sum variant) only by a malformed descriptor — render as an
            // ordinary `tuple`, the safe fallback the iterative walk uses.
            if elems.is_empty() {
                let l = b.name_leaf("unit");
                return Some(b.atom(l));
            }
            let elems = elems.clone();
            let head = b.name_leaf("tuple");
            let head_s = b.atom(head);
            let mut children = vec![head_s];
            for (i, &es) in elems.iter().enumerate() {
                children.push(encode_value_recursive(
                    desc,
                    b,
                    op_arr_get(h, i as u32),
                    es,
                    depth + 1,
                )?);
            }
            b.list(&children)
        }
    })
}

fn build_intlist(n: usize) -> Handle {
    let mut acc = op_sum_new(1, op_arr_alloc(0)); // Nil
    for i in (0..n).rev() {
        let pair = op_arr_alloc(2);
        op_arr_set(pair, 0, op_box_int(i as i64));
        op_arr_set(pair, 1, acc);
        acc = op_sum_new(0, pair); // Cons(tuple i rest)
    }
    acc
}

/// A `record { members: (Set Int64), tag: Int64 }` descriptor — table [0]=Int, [1]=Set(→0),
/// [2]=Record[(members→1),(tag→0)]; root=2. Exercises BOTH canon-convergence sites in one value: the
/// record-field `=` head AND the Set `(. Set of)` head.
fn record_with_set_descriptor() -> Vec<u8> {
    fn leb(out: &mut Vec<u8>, mut v: u64) {
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
    }
    fn name(out: &mut Vec<u8>, s: &str) {
        leb(out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }
    let mut d = Vec::new();
    leb(&mut d, 3); // table_len = 3
    d.push(0); // [0] Int
    d.push(12); // [1] Set(→0)
    leb(&mut d, 0);
    d.push(8); // [2] Record with 2 fields, in descriptor (sorted) order: members, tag
    leb(&mut d, 2);
    name(&mut d, "members");
    leb(&mut d, 1); // members → Set
    name(&mut d, "tag");
    leb(&mut d, 0); // tag → Int
    leb(&mut d, 2); // root = 2
    d
}

/// CANON-STABILITY GATE (protects the value-encode→canon convergence, trunk 51a4a7a8d): value-encode's
/// document must have its LEAVES interned in canon's order — strictly PRE-ORDER, first-encounter,
/// left-to-right over the struct tree from the root (see cadenza-ast/canon.rs `visit`). This is what
/// makes `value_encode(v)` == `codec::encode(canon(tree))` byte-for-byte, i.e. a STABLE content-address.
///
/// REPRO (v-compiler-ml issue, b1 blocker): a Sum arm carrying a RECORD must value-encode the record,
/// not an empty leaf. Descriptor (v-cml-pinned, 21 bytes): P = A of Bytes | B of Record(x: Str), rooted
/// at Named("P"). Value B(record x="hi") must encode to `(: (B (record (= x "hi"))) P)` — the doc MUST
/// contain the "B", "record", "x" name leaves and the "hi" str leaf. An empty render is the reported bug.
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

/// A rendered-text gate CANNOT catch a regression here: emitting the record-field `=` or the Set
/// `(. Set of)` head POST-order (the pre-convergence bug) produces IDENTICAL rendered s-expr text but a
/// DIFFERENT leaf pool — so it would slip silently past the corpus. This walks the parsed document and
/// asserts each leaf id is first-referenced in non-decreasing pre-order, exactly canon's numbering.
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

/// The iterative production `encode_value` must produce BYTE-IDENTICAL documents to the recursive
/// oracle, across the interesting shapes (nested sums, lists, tuples). Drives only modest depth — a
/// deep value would overflow the recursive oracle (the exact bug the iterative walk fixes).
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

fn differential_body() {
    reset();
    let desc = intlist_descriptor();
    for &n in &[0usize, 1, 2, 5, 50, 500] {
        let v = build_intlist(n);
        let iter_doc = op_value_encode_form(v, &desc).expect("iterative encode");
        // Recursive oracle over the same borrowed value.
        let descriptor = decode_descriptor(&desc).expect("descriptor");
        let mut b = DocBuilder::default();
        let root = encode_value_recursive(&descriptor, &mut b, v, descriptor.root, 0)
            .expect("recursive encode");
        let rec_doc = b.finish(root);
        assert_eq!(
            iter_doc, rec_doc,
            "iterative and recursive encode disagree at N={n}"
        );
        op_drop(v);
    }
    assert_eq!(live_nodes(), 0, "no leak: every built list dropped");
}

/// The headline robustness property: a DEEP recursive value (a long linked list — the shape this op
/// exists to encode) encodes WITHOUT overflowing the stack. The recursive walker aborted the guest
/// between ~4.5 k levels (native 2 MB stack, worse in wasm); the iterative walk handles it in heap.
/// 50 000 ≫ that native crash depth, proving the fix is real (not just a raised cap).
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

/// `value-encode` renders a String payload (`Shape::Str`) as a `KIND_STR` leaf — the codec's string
/// leaf (kind 7, `write_bytes` = LEB len + UTF-8 body). Previously `encode_value` DECLINED on
/// `Shape::Str` (returned `None`), so a recursive value carrying a string (an AST node, a JSON tree)
/// could not cross the host boundary at all, even though the wire format has the kind. Byte-exact.
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

/// A `Str`/`Bytes` leaf stores its bytes as `Raw` (inline ≤INLINE_RAW_CAP=12, else heap) so a SHORT
/// string allocates NO per-leaf `Vec` — but a LONGER string must still round-trip byte-exact through the
/// heap arm. The inline↔heap boundary (12 bytes) is invisible in the output (both write the same KIND_STR
/// len+body), so pin it: a 12-byte (inline max) and a 13-byte (first heap) string each encode to their
/// exact KIND_STR bytes. Guards `Raw::from_slice`'s boundary in the leaf path — a short-string regression
/// (dropping the inline arm) or an off-by-one at the cap would still pass the existing "hi" test.
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

/// The single-entry `DESCRIPTOR_CACHE` must never cross-contaminate: two DIFFERENT descriptors, whether
/// alternated (thrashing the 1-entry cache — every call a miss) or repeated (hitting), must each yield
/// the SAME output as a fresh decode would. The cache key is the descriptor BYTES, so a byte-different
/// descriptor must always re-decode; this pins that the key comparison + refresh is correct (a bug that
/// returned the STALE cached descriptor for new bytes would render the wrong value). Encodes an Int
/// (desc A) and a Str (desc B) in an ALTERNATING sequence, then each REPEATED, asserting every result.
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

/// value-encode of a ROPE String (concat/slice nodes) via `Shape::Str` must MATERIALIZE it first.
/// Since a runtime `String.concat`/`String.at`-slice lowers to the SAME `bytes-concat`/`bytes-slice`
/// rope nodes as Bytes (a String IS a bytes rope), a rope-String reaching `Shape::Str` is NOT a flat
/// leaf — the encoder must `bytes_flatten` before reading `raw` (fixed `@b77b3ae0`; without it a rope
/// String rendered its raw HANDLE bytes = garbage). Every OTHER `Shape::Str` test uses `op_str_new` (a
/// FLAT leaf), so the flatten line was runtime-untested (only an e2e wasmtime spot check). Build a rope
/// String the way the compiler does and assert it encodes byte-identically to the equivalent flat one.
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

/// value-encode of a BOXED i64 at the extremes, byte-exact against the codec's KIND_INT sign+magnitude
/// form. `int_round_trip` only checks `op_get_int` (round-trip) and a test-side `render` reimpl — NOT
/// the real codec bytes through `op_value_encode_form`. The riskiest value is `i64::MIN`, whose `-v`
/// overflows: the scalar path stores the raw `i64` (`DocLeaf::IntScalar`) and `i64_be_magnitude`
/// derives the wire bytes at finish via `v.unsigned_abs()` (= 2^63, magnitude `80 00…00`) so the
/// magnitude is right and the sign flag negative. A big-endian / leading-zero-strip / sign bug in the
/// finish-time derivation slips past the small-value `intlist` differential; this pins the boundary
/// (and guards that the raw-i64 `IntScalar` form is byte-identical to the old heap-magnitude form).
/// Descriptor:
/// table [0]=Int (tag 0), root=0. Doc = header(8)·leaf_count(1)·[KIND·LEB(mlen)·mag]·struct(1)·
/// [TAG_ATOM·0]·root(0). KIND 0 = pos, 3 = neg.
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

/// The reused thread-local `DocBuilder`/`out` (`ENCODE_BUILDER`/`ENCODE_OUT`) must be a PURE allocation
/// optimisation: (1) BYTE-IDENTICAL output across repeated encodes (a stale-state bug from an
/// incomplete `reset` would corrupt the 2nd+ encode), (2) encoding a SMALL value right after a LARGE
/// one must not leak the large value's data into the small one's document (the `reset` clears the
/// pools; retained capacity must not surface as content), (3) no node leak. Guards the reset+reuse
/// contract that the alloc-ceiling win rests on. Encodes the SAME value 3× (must be identical), then a
/// large value, then a small value again (must equal its first encoding — reused-but-cleared pools).
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

// ─── value-decode (idx 90) round-trip: value-decode ∘ value-encode ≅ id ────────────────────
// The acceptance bar (DESIGN-binary-ast-abi B0): for a value `v` of shape `desc`, decoding the
// canonical value-form document `value-encode` produces must reconstruct a value structurally equal to
// `v` (`value_eq_shaped`). Covers the shape spectrum the encode corpus exercises, run BACKWARDS.

/// Round-trip `v` (shape `desc`): `value-decode(value-encode(v)) ≅ v` via `value_eq_shaped`, and assert
/// no leak once both are dropped. `desc` is the descriptor byte-slice (`[table_len][shapes…][root]`).
fn assert_value_roundtrips(v: Handle, desc: &[u8]) {
    let doc = op_value_encode_form(v, desc).expect("encode");
    let decoded = op_value_decode(&doc, desc);
    assert_ne!(
        decoded,
        Handle::NULL,
        "value-decode returned NULL (mismatch)"
    );
    let descriptor = decode_descriptor(desc).expect("descriptor");
    let eq = value_eq_shaped(&descriptor, decoded, v, descriptor.root);
    assert_eq!(
        eq,
        Some(true),
        "decoded value must be structurally equal to the original"
    );
    op_drop(decoded);
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

/// A malformed descriptor whose Framed TYPE NODE nests absurdly deep DECLINES (`None`), it does not
/// overflow the stack. `decode_type_node` recurses per nesting level, and a level is only 2 bytes
/// (`[name_len=0][n_children=1]`), so before the `TYPE_NODE_DEPTH_CAP` a ~200 KB descriptor recursed
/// ~200 k deep and SIGABRT'd the guest — violating value-encode's "never a trap" totality (a
/// compiler-baked type node is always shallow, but the escape op must decline any input). The cap
/// makes it decline. A genuine type (`(Map Int (List Bool))`, depth 2) is far under the cap, unaffected.
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

/// A WIDE record (many DISTINCT field names) encodes byte-identically to the recursive oracle. This is
/// the shape whose `name_leaf` dedup was O(N²) (each distinct field name missed the linear scan and
/// walked all prior leaves — a 3200-field record took ~183 ms; after the `name_index` map it is O(N),
/// ~14 ms). Byte-identity here proves the map-based dedup produces the SAME leaf pool + indices as the
/// scan did (a repeated name still resolves to its first index). A moderate N keeps the test fast while
/// exercising the many-distinct-name path the small fixed-shape tests never reach.
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

/// A String nested inside a recursive sum encodes (the real use — a value form like an AST node with
/// an identifier, or a `List Str`). Descriptor: a Cons/Nil list whose element is a Str. Drives the
/// iterative walk through Sum → Tuple → Str and back via Ref, and checks byte-identity vs the oracle.
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

/// `value-encode` renders a Bytes payload (`Shape::Bytes`) as a `KIND_BYTES` leaf (kind 11, same
/// `write_bytes` framing as Str/Name). Previously `encode_value` DECLINED on `Shape::Bytes`, so a
/// recursive value carrying a Bytes field could not cross the host boundary. A Bytes value may be a
/// ROPE (concat/slice); the walk flattens it first (iterative, unobservable). Byte-exact + rope case.
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

/// A Bytes field nested inside a recursive sum encodes (a parse tree, a binary structure). Descriptor:
/// a Cons/Nil list whose element is Bytes; drives the iterative walk through Sum → Tuple → Bytes and
/// back via Ref, and checks byte-identity vs the recursive oracle (which flattens ropes identically).
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

/// `value-encode` renders a Float payload (`Shape::Float`) as a `KIND_FLOAT` leaf — the codec's exact
/// decimal (kind 6: negative(u8) + exponent(fixed 8-byte BE i64) + LEB siglen + big-endian magnitude).
/// The runtime f64 is converted to the decimal by a port of the compiler's `Decimal::from_f64`. A
/// NON-FINITE float declines (no exact-decimal form), matching `from_f64`.
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

    // A NON-FINITE float declines (nan/inf have no exact-decimal form → whole encode is None).
    let nan = op_box_float(f64::NAN);
    assert!(
        op_value_encode_form(nan, desc).is_none(),
        "nan declines (no exact decimal)"
    );
    op_drop(nan);
    let inf = op_box_float(f64::INFINITY);
    assert!(op_value_encode_form(inf, desc).is_none(), "inf declines");
    op_drop(inf);

    assert_eq!(live_nodes(), before, "no leak: every float value dropped");
}

/// `value-encode` renders a Float32 (`Shape::Float32`) as a `KIND_FLOAT` leaf carrying the f32's OWN
/// shortest decimal — the whole reason Float32 gets a 4-byte leaf instead of a promoted f64. The
/// headline case: `0.1f32` encodes as `1 × 10^-1` (decimal "0.1"), NOT the f64-promotion's
/// `10000000149011612 × 10^-17`. Also: `1.5f32` byte-exact; a non-finite f32 declines.
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

    // A non-finite f32 declines.
    let nan = op_box_float32(f32::NAN);
    assert!(
        op_value_encode_form(nan, desc).is_none(),
        "f32 nan declines"
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

/// The decimal `float_leaf` produces must ROUND-TRIP back to the original f64 (it is the shortest
/// round-tripping form). Reconstruct the decimal STRING `[-]<digits>e<exp>` from the emitted
/// `(neg, exponent, magnitude)` and parse it with Rust's CORRECTLY-ROUNDED `str::parse::<f64>` (exact,
/// unlike lossy `sig * 10f64.powi(exp)`). Compare bit-for-bit across finite values incl. ±0.0/subnormal.
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

/// Decode a single-Float value-encode document back to the decimal string it denotes:
/// `[-]<significand>e<exponent>`, where the significand is the big-endian base-256 magnitude read as a
/// base-10 integer. Robust to a magnitude of ANY length (repeated ÷10 on a base-256 limb vector — no
/// u128 width assumption), so it works for a fuzzed value's full shortest decimal. Doc layout:
/// header(8)·leaf_count(1)·[KIND_FLOAT · neg(1) · exp(8 BE) · siglen(LEB) · mag] · struct… — the float
/// leaf is first, at offset 9: [9]=KIND, [10]=neg, [11..19]=exp, [19..]=siglen(LEB), then mag. The
/// siglen is a VARIABLE-length LEB (`doc_leb`), NOT a fixed byte — a full-expansion significand (a
/// whole float's exact decimal, e.g. f64::MAX = a 128-byte magnitude) has a multi-byte length, so read
/// the LEB and advance past it before the magnitude.
fn float_doc_to_decimal(doc: &[u8]) -> String {
    let neg = doc[10] == 1;
    let mut eb = [0u8; 8];
    eb.copy_from_slice(&doc[11..19]);
    let exp = i64::from_be_bytes(eb);
    // Read the LEB128 significand length starting at offset 19.
    let mut siglen = 0usize;
    let mut shift = 0u32;
    let mut off = 19usize;
    loop {
        let byte = doc[off];
        off += 1;
        siglen |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let mag = &doc[off..off + siglen]; // big-endian base-256
    // base-256 (big-endian) → decimal string via repeated division by 10.
    let mut limbs: Vec<u32> = mag.iter().map(|&b| b as u32).collect(); // most-significant first
    let mut digits_rev: Vec<u8> = Vec::new();
    while limbs.iter().any(|&l| l != 0) {
        let mut rem = 0u32;
        for l in limbs.iter_mut() {
            let cur = rem * 256 + *l;
            *l = cur / 10;
            rem = cur % 10;
        }
        digits_rev.push(b'0' + rem as u8);
        // trim leading (most-significant) zero limbs so the loop terminates promptly
        while limbs.first() == Some(&0) && limbs.len() > 1 {
            limbs.remove(0);
        }
    }
    let sig: String = if digits_rev.is_empty() {
        "0".into()
    } else {
        digits_rev.iter().rev().map(|&b| b as char).collect()
    };
    format!("{}{}e{}", if neg { "-" } else { "" }, sig, exp)
}

/// FUZZ: the `float_leaf` conversion (f64 → codec KIND_FLOAT decimal via `{:e}` + base-10→base-256
/// Horner) over RANDOM f64 bit patterns — far stronger than the ~12 hand-picked values in
/// `value_encode_float_decimal_round_trips_to_the_same_f64`. Every FINITE f64 encoded via the real
/// `op_value_encode_form` must, when its KIND_FLOAT decimal is parsed back, reconstruct the EXACT bits
/// — a digit-count, exponent-fold, or Horner-carry bug in `float_leaf_from_sci` would surface on some
/// bit pattern the fixed list misses. A non-finite float declines (`None`) — asserted too. No leak.
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
            // nan/inf have no exact-decimal form → the walker declines (they cross by dedicated forms).
            // (`op_box_float` canonicalizes NaN, but the encode of a non-finite still declines.)
            assert!(
                doc.is_none(),
                "a non-finite float must DECLINE the value-encode, not emit garbage"
            );
        }
        op_drop(h);
        assert_eq!(live_nodes(), 0, "no leak for bits {bits:#018x}");
    });
}

/// FUZZ the Float32 encode (`float32_leaf`) over RANDOM f32 bit patterns — the companion to the f64
/// fuzz. `float32_leaf` shares `float_leaf_from_sci` but feeds it the f32's OWN shortest decimal
/// (`{f32:e}`, NOT a promoted f64 whose decimal differs), so the digit strings it converts are a
/// distinct population. Every finite f32, encoded via the real `op_value_encode_form`, must round-trip
/// bit-exactly through its KIND_FLOAT decimal parsed back AS AN f32; a non-finite f32 declines. No leak.
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
            assert!(
                doc.is_none(),
                "a non-finite f32 must DECLINE the value-encode"
            );
        }
        op_drop(h);
        assert_eq!(live_nodes(), 0, "no leak for f32 bits {bits:#010x}");
    });
}

/// `op_box_float` normalizes every NaN — of ANY bit pattern — to the ONE canonical quiet NaN
/// (`f64::NAN.to_bits()`), so a float leaf has a single canonical byte form (deterministic-value-
/// form.md). Two NaN values that differ ONLY in their (unobservable) payload/sign bits must therefore
/// box to byte-IDENTICAL leaves and be equal under `champ_eq` / hash-identical under `champ_hash` —
/// otherwise they would be distinct map/set keys, violating the spec (every NaN equals every NaN). A
/// finite value keeps its bits, so `-0.0` stays DISTINCT from `0.0`.
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

/// NaN canonicalization composes through a COMPOUND: two tuples `(nan, x)` built from DIFFERENT NaN
/// bit patterns are `value-eq` equal and hash-identical (so they are the same map key), because each
/// NaN element canonicalizes to one byte form on `box-float`. This is the reachable path Float64-in-
/// compound (@ea74c89f) + NaN-canonicalization (@f25d7075) enable together — `value-eq` (op 61) IS the
/// language `=` on runtime compounds, so a struct/tuple carrying a NaN must compare structurally equal
/// regardless of the NaN's origin. A tuple with -0.0 vs one with 0.0 stays UNEQUAL (distinct forms).
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

/// `value-encode` renders a Set (`Shape::Set`) as `((. Set of) (list e1 … en))` with elements in
/// CANONICAL key-VALUE order — NOT the CHAMP hash order. The walk collects the elements + sorts by the
/// element's canonical scalar value (matching the compiler's `const_key_order`). Verifies the
/// structure + canonical INT order (numeric, not raw-byte) + differential vs the recursive oracle.
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

/// `value-encode` of a `Set String` — the EXACT shape the compiler-in-Cadenza port returns across the
/// host boundary (e.g. `free-vars.cdz`'s `Set String` of an AST's identifier Names). The int set-render
/// test above covers `value_cmp_shaped`'s numeric-Int arm; this covers its `Shape::Str` arm
/// (lexicographic BYTE order over the flattened leaf) driving `set_elements_canonical`'s sort. A String
/// element takes the arity-0 heap-byte-leaf champ path (distinct from an immediate int), and the
/// render must be lexicographic — NOT the CHAMP hash order the set stores/iterates in. Verifies the
/// canonical order (incl. the empty string sorting first + a shared "foo"/"foobar" prefix) + the
/// iterative-vs-recursive-oracle byte-identity + no leak.
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

/// `value-encode` renders a Map (`Shape::Map`) as `(map (k1 v1) … (kn vn))` with entries in CANONICAL
/// KEY order — NOT the CHAMP hash order. The walk collects (key,value) pairs + sorts by the key's
/// canonical scalar value (matching `const_key_order`). Verifies the structure + canonical INT-key
/// order (numeric, not raw-byte) + differential vs the recursive oracle.
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

/// The Set/Map canonical-render-order tests above use only POSITIVE keys (256 vs 1), where
/// little-endian byte order already disagrees with numeric order. But NEGATIVE ints are the SEVERE
/// divergence: a negative's little-endian `raw` bytes are `0xFF…` (large unsigned), so a raw-byte
/// comparison — exactly what `champ_key_cmp` (the CHAMP KEY comparator) uses — sorts every negative
/// AFTER every positive, the OPPOSITE of numeric order. The value-encode render order comes from a
/// SEPARATE walk, `value_cmp_shaped`'s `Shape::Int` arm, which reads `op_get_int` (SIGNED) — so negatives
/// render correctly BEFORE positives. This pins that: a signed-key map/set (symbol tables with
/// sentinels, coordinate/offset maps) renders in true numeric order, and guards against a future
/// "optimization" that makes `value_cmp_shaped` reuse the raw-byte `champ_key_cmp` rule (which
/// would silently mis-order every negative — a bug the positive-only 256-vs-1 tests can't catch).
///
/// Reconstructs each int leaf's SIGNED value from the wire (kind 0 = KIND_INT_POS_DEC positive, kind
/// 3 = its negative variant; magnitude is big-endian) and asserts strict ascending numeric order.
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

/// value-encode of a NESTED-COLLECTION value: a `Map Int (List Int)` — the map VALUE is itself a
/// collection walked recursively (`Shape::Map`'s `val` shape can be any encodable shape, not just a
/// scalar; the arm's comment says so but every other map/set test uses a scalar value). This is the
/// shape the compiler's "sum + nested-collection compound results" work now escapes via value-encode,
/// so the recursive value-walk (map → each entry's value → List → vec elements) must be exercised. Assert
/// byte-identity to the recursive oracle (which mirrors the nested walk) + entries in canonical KEY
/// order with each value list intact.
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

/// value-encode of EMPTY collections — the zero-element assembler edge (`SetOf`/`MapOf`/`List` with
/// 0 children, `list_head_tail` with an empty tail, the `checked_sub(0)` in the assemblers). An empty
/// collection returned to the host is common; a zero-element bug (underflow, dropped head, wrong form)
/// would be a silent miscompile. Empty set → `((. Set of) (list))`, empty map → `(map)`, empty list
/// → `(list)`. Verified byte-identical to the recursive oracle + the concrete forms.
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

/// `value-encode` renders a MULTI-payload recursive variant via `Shape::Spread` (descriptor tag 16):
/// the payload elements are spliced FLAT under the variant head — `(Node 1 l r)`, NOT the
/// tuple-wrapped `(Node (tuple 1 l r))` (landed @75fe7e80). That production Sum→Spread walk arm had NO
/// dedicated value-encode test (only the differential oracle arm — the same gap as Framed). A splice
/// bug (tuple-wrapping, wrong element order, wrong arity) would be a silent miscompile on a common
/// recursive shape (a tree). Verifies iterative==recursive byte-identity + the FLAT rendering.
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

/// `value-encode` renders a `Shape::Framed` (descriptor tag 15) as the `(: value (head arg…))`
/// parametric-type frame — the shape a RUNTIME `List` result escapes as `(: (list …) (List <elem>))`
/// (landed @72d5d80a). That production walk arm had NO dedicated value-encode test (only the
/// differential oracle arm); an encoding bug (wrong tag, arg order, or frame nesting) would be a
/// silent miscompile on a real escape path. Verifies iterative==recursive byte-identity AND the
/// concrete rendered structure.
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

/// `value-encode` of a `Shape::List` over a LARGE, MULTI-LEVEL RRB vec — the shape the compiler-in-
/// Cadenza port produces for any list literal / `List.map` result with >32 elements (a list literal
/// lowers to `vec-of-arr`, and `String.concat`/fold builds grow real `vec`s). Every OTHER `Shape::List`
/// encode test builds a ≤3-element vec — a SINGLE RRB leaf (`shift=0`), so `op_vec_get`'s multi-level
/// TRIE DESCENT (interior nodes at index ≥32) was never exercised on the escape path, exactly the arm
/// whose doc-comment records the past "arr-len/arr-get read the root arity → rendered only the first
/// element" bug. Sizes 31/32/33/64/100 straddle the `VEC_BITS=5` (branch 32) level boundary (31 = full
/// single leaf, 32 = still one leaf, 33 = first 2-level, 64/100 = deeper 2-level).
///
/// TWO independent checks so a bug can't hide: (1) DIFFERENTIAL — the iterative production walk must
/// byte-match the recursive oracle (catches a walk-order/index bug in one walker); (2) CONTENT — decode
/// the document's KIND_INT leaves and assert they equal the pushed sequence `1..=n` IN ORDER (catches a
/// SHARED `op_vec_get` trie-descent bug that would fool the differential — both walkers call `vec-get`,
/// so a wrong element at a trie boundary would make them AGREE on the wrong answer).
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

fn alloc_calls() -> u64 {
    ALLOC_CALLS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Allocation-ceiling regression guard for the hot CHAMP/RRB ops, run SINGLE-THREADED (see below).
/// Uses the process-wide `CountingAlloc` (test build only) to count GROSS heap allocations —
/// including transient Vecs freed immediately, which the `live-objects` node counter does NOT see —
/// so it catches a future change that reintroduces the per-spine-node `new_handles`/`champ_header`
/// allocations this commit removed. Ceilings sit comfortably above the figures measured 2026-07-12
/// after the champ_become_hdr + in-place-slot + allocation-lazy-remove + alloc-free-cursor + lazy
/// champ_eq/cmp worklist + EMPTY-slot splice + inline-Entry + inline-refcount cursor advance +
/// in-place remove-drain/collapse + inline-Slots cursor + in-place SPLIT + shallow-compound hash+eq work
/// (insert 766, remove 0, iterate 3, push 197, get 0, lookup 0, tuplekey-lookup 2000=probe-only, sum_new ~2000;
/// set union 431 / ∩ 356 / ∖ 362, ∖ unique-small-b 774≈build-only; they are UPPER BOUNDS so noise never trips them but a
/// regression toward the old 6779/8397/5248/1000 does.
///
/// WARNING: MUST run alone: the counter is PROCESS-WIDE, so a concurrent test thread's allocations pollute
/// the reading (observed ~51k when run in the default multi-threaded suite). It is therefore
/// `#[ignore]`d in the normal run (and in `cargo test`/`cargo xtask check`) and exercised on demand:
///   `cargo test -p cdz-runtime hot_op_allocation_ceilings -- --ignored --test-threads=1 --nocapture`
/// Correctness of these ops is covered independently by the FBIP canonical-shape / reference tests,
/// which DO run in the normal suite; this test guards only the allocation budget.
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

/// CPU-scaling PROBE (diagnostic, not a regression gate): times set ∩/∖ at growing N to reveal
/// whether they are linear-ish or super-linear (the alloc bench can't see the O(log) contains-probe
/// factor — evidence for whether the O(min) node-merge redesign is worth a future tick). Also times
/// UNION over COMPOUND (tuple) elements, where hashing an element walks its whole subtree — this is
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

/// CPU-scaling PROBE (diagnostic, not a gate) for the STRING-KEYED map shape (JSON-object /
/// dictionary): keys are multi-byte heap strings, so every insert/lookup pays a byte-serial FNV
/// over the whole key plus a byte compare on a slot hit. Times build + lookup at growing key
/// LENGTH to reveal whether the cost is dominated by the FNV walk (scales with key bytes) or the
/// trie descent (scales with map size). Run under `perf` to attribute the hot region. Never
/// profiled before — the existing probes all use int/tuple/nested keys.
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

/// CPU-scaling PROBE (diagnostic, not a gate) for the SHARED/PERSISTENT vec copy path — the
/// functional-update pattern (keep the base version, derive a new one), the largest realistic
/// allocator (vec_push_shared/vec_update_shared ~7000 allocs/1000). Each op path-copies the touched
/// RRB spine (root→leaf) via `vec_node_replace`/`vec_node_append`, `op_dup`ing every off-path
/// sibling. Times shared push + shared update at growing N to reveal the copy-path hot region under
/// `perf` (the alloc bench sees the count but not where the CPU goes). Never profiled before.
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

/// CPU-scaling PROBE (diagnostic, not a gate) for the SHARED/PERSISTENT CHAMP map copy path — the
/// functional-update pattern on a map (keep the base version, derive a new one). This is the second-
/// largest realistic allocator (map_insert_shared 6143, map_remove_shared 6685 allocs/1000); the
/// alloc bench tracks the COUNT, this times where the CPU goes under `perf`. Each op path-copies the
/// touched spine root→leaf via `champ_insert_node`/`champ_remove_node` (clone-once-and-mutate,
/// dup every off-path sibling). Complements `shared_vec_copy_path_cpu_scaling_probe`; the map copy
/// path was never dedicated-CPU-profiled.
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

/// The STATIC shape descriptor the compiler holds at each use site. There is no runtime type
/// tag, so the renderer is driven ENTIRELY by this compile-time knowledge: the SAME heap node
/// renders differently under different shapes (an `Arr[3,1]` is `(tuple 3 1)` under `Tuple` and
/// `(list 3 1)` under `List`). This mirrors, in plain Rust, the type-directed renderer the
/// compiler bakes into the emitted program.
enum Shape {
    Int,
    Bool,
    Float,
    /// A fixed-arity positional product; empty = unit.
    Tuple(Vec<Shape>),
    /// A homogeneous, runtime-length sequence over one element shape.
    List(Box<Shape>),
    /// Named fields in positional order; names are compile-time constants.
    Record(Vec<(&'static str, Shape)>),
    /// Variants in discriminant order; the disc selects the name + payload shape.
    Sum(Vec<(&'static str, Shape)>),
    Bytes,
    Str,
}

/// A native mirror of the compiler-emitted, type-directed renderer. It walks a value through the
/// runtime accessors EXACTLY as the emitted program will — reading scalars, `arr-len`/`arr-get`
/// for sequences, `sum-disc`/`sum-payload` for sums, `bytes-*` for buffers — with the canonical
/// name/keyword supplied by the static `Shape`, never by the runtime. This pins that the
/// accessors are sufficient to render WITHOUT a runtime tag.
/// Append the `b"…"` display escape of one byte — the same rules as the compiler's
/// `escape_byte` and the exact inverse of the `b"…"` reader. Escape order is load-bearing:
/// `\` and `"` sit inside the printable range, so they match before the passthrough arm.
fn escape_byte(b: u32, out: &mut String) {
    match b {
        b if b == b'\n' as u32 => out.push_str("\\n"),
        b if b == b'\r' as u32 => out.push_str("\\r"),
        b if b == b'\t' as u32 => out.push_str("\\t"),
        b if b == b'\\' as u32 => out.push_str("\\\\"),
        b if b == b'"' as u32 => out.push_str("\\\""),
        0 => out.push_str("\\0"),
        0x20..=0x7e => out.push(b as u8 as char),
        _ => {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            out.push_str("\\x");
            out.push(HEX[((b >> 4) & 0xf) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
    }
}

fn render(handle: Handle, shape: &Shape) -> String {
    match shape {
        Shape::Int => op_get_int(handle).to_string(),
        Shape::Bool => {
            if op_get_bool(handle) {
                "true".into()
            } else {
                "false".into()
            }
        }
        Shape::Float => {
            let f = op_get_float(handle);
            // Whole floats keep a `.0` so their canonical text stays float-shaped.
            if f.is_finite() && f.fract() == 0.0 {
                format!("{f:.1}")
            } else {
                format!("{f}")
            }
        }
        Shape::Tuple(elems) => {
            if elems.is_empty() {
                return "unit".into();
            }
            let mut out = String::from("(tuple");
            for (i, s) in elems.iter().enumerate() {
                out.push(' ');
                out.push_str(&render(op_arr_get(handle, i as u32), s));
            }
            out.push(')');
            out
        }
        Shape::List(elem) => {
            let n = op_arr_len(handle);
            let mut out = String::from("(list");
            for i in 0..n {
                out.push(' ');
                out.push_str(&render(op_arr_get(handle, i), elem));
            }
            out.push(')');
            out
        }
        Shape::Record(fields) => {
            let mut out = String::from("(record");
            for (i, (k, s)) in fields.iter().enumerate() {
                // `(= name value)` ascription form (record-type Phase B full-symmetry migration).
                out.push_str(&format!(
                    " (= {k} {})",
                    render(op_arr_get(handle, i as u32), s)
                ));
            }
            out.push(')');
            out
        }
        Shape::Sum(variants) => {
            let disc = op_sum_disc(handle) as usize;
            let (name, payload_shape) = &variants[disc];
            format!("({name} {})", render(op_sum_payload(handle), payload_shape))
        }
        Shape::Bytes => {
            // `b"…"` — the byte-string display form (matching the `bytes` crate's `Debug`, and
            // the exact inverse of the `b"…"` reader). Must agree byte-for-byte with the const
            // fold (`bytes_literal_text`) and the emitted-wasm renderer (`emit_byte_escape`).
            let n = op_bytes_len(handle);
            let mut out = String::from("b\"");
            for i in 0..n {
                escape_byte(op_bytes_get(handle, i), &mut out);
            }
            out.push('"');
            out
        }
        Shape::Str => format!("\"{}\"", op_str_get(handle)),
    }
}

/// Read a node's refcount header directly (test-only). Immediate-aware: an immediate is not a
/// Node, so `*h.0` would be UB — report the same non-1 sentinel `node_rc` does.
fn rc_of(h: Handle) -> u32 {
    if is_immediate(h) {
        return 2;
    }
    unsafe { (*h.0).rc }
}

/// Test-only: is the node's raw payload HEAP-backed (spilled) rather than inline? Used to assert the
/// reuse constructors normalize a reused shell's raw back to inline (a fresh constructor's rep).
fn raw_is_heap(h: Handle) -> bool {
    if is_immediate(h) {
        return false;
    }
    matches!(unsafe { &(*h.0).raw }, Raw::Heap(_))
}

/// Test-only: is the node's handle vector HEAP-backed (spilled past the inline cap) rather than
/// inline? The handles-arm twin of `raw_is_heap`: used to assert the reuse constructors normalize a
/// reused shell's HANDLES back to inline for a ≤`INLINE_HANDLES_CAP`-child node, matching a fresh
/// constructor's rep (a wide reset token keeps a `Handles::Heap` unless the refit re-inlines it).
fn handles_is_heap(h: Handle) -> bool {
    if is_immediate(h) {
        return false;
    }
    matches!(unsafe { &(*h.0).handles }, Handles::Heap(_))
}

/// A DEFINITELY-BOXED int leaf (test-only): bypasses `op_box_int`'s P2 normalize so the RC /
/// reuse / cascade tests keep exercising a real heap Node with rc == 1 (a small `op_box_int(v)`
/// now inlines and would make those node-count / drop-a-leaf scenarios vacuous). Byte-identical
/// to the pre-P2 boxed representation, so `op_get_int` decodes the same value through `with_node`.
fn boxed_int_leaf(v: i64) -> Handle {
    alloc(Vec::new(), (v as u64).to_le_bytes().to_vec())
}

// ── Inline tagged-immediate helpers (producers: op_box_int fixnum / op_box_bool / op_arr_alloc(0)) ─────

/// LAYOUT GUARD: `Node` is paid by EVERY heap value, so its size is load-bearing for allocation +
/// cache behavior. There is otherwise no signal if a change bloats it (a new field, a widened
/// `Handles`/`Raw` variant). Pin the NATIVE-host sizes so a structural regression is caught in the
/// std test suite. WARNING: These are the 64-bit NATIVE sizes (`Handle` = `*mut Node` = 8, `Vec` = 24); the
/// SHIPPED wasm32 layout is smaller (`Handle` = 4, `Vec` = 12) — this test can't run on wasm, but the
/// native size moves TOGETHER with wasm for STRUCTURAL changes (an added field / a wider enum variant
/// bloats both), which is the regression worth catching. Node=64 is already minimal for its fields:
/// `rc:u32`(4) + `Handles`(32) + `Raw`(24) = 60, padded to 64 for the 8-alignment `Handles`/`Raw`
/// require — the 4-byte pad after `rc` is unavoidable. If you INTEND to change the layout, update
/// these + re-measure the wasm hash impact.
/// The `cdz-abi` section constant `CDZ_ABI_IMM_UNIT` — which `xtask codegen` extracts and the compiler
/// emits as `IMM_UNIT` — MUST be the exact little-endian `u32` bit pattern of `imm_unit()`. The two are
/// INDEPENDENT hard-coded `0b0010` literals (one in `imm_unit()`, one in the `cdz-abi` static) linked by
/// nothing but this test. The stakes rose with `spec@5d9a1dc1`: the compiler now emits `IMM_UNIT`
/// DIRECTLY as the `None`-arm result of `List.at`/`Map.lookup`/`String.at`/`Bytes.at` (replacing a
/// runtime `arr-alloc(0)` CALL that computed the value from `imm_unit()`). So if a future immediate-tag
/// rework changed `imm_unit()`'s encoding WITHOUT updating `CDZ_ABI_IMM_UNIT`, every miss of those four
/// common ops would return a MALFORMED "unit" the runtime misreads (a silent miscompile) — where before
/// the runtime recomputed the correct value at the call. `arr-alloc(0)` also returns `imm_unit()`, so
/// this pins the whole "empty compound / nullary payload == the inline unit constant" ABI contract the
/// compiler leans on. (Native test: read the bits via `.0 as usize as u32`; `Handle::to_u32` is
/// wasm32-only.)
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
fn node_layout_sizes_are_pinned_native() {
    use core::mem::size_of;
    assert_eq!(
        size_of::<Node>(),
        64,
        "Node size changed — a bloat is paid by every heap value"
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

/// `read_u32_at` has two paths — a fast in-bounds 4-byte window read and a zero-padded fallback for
/// a short/absent raw. This locks their EQUIVALENCE at the boundary: both must yield the same
/// little-endian value where 4 bytes exist, and zero-pad missing high bytes. It is the hottest
/// header read (`champ_datamap`/`nodemap`/`size`, every descent level), so a boundary mistake would
/// silently corrupt bitmaps/sizes; a reference recompute over every offset of several raw lengths
/// (including 0/1/2/3-byte short raws that exercise ONLY the fallback) pins the contract.
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

// ── Inline unit + bool: SHARED-REPRESENTATION payoff (P1b flips the producers) ────────

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

// ── Inline small ints: the fixnum window (P2 flips op_box_int) ────────────────────────

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

/// `bigint-div` TRUNCATES toward zero across the FULL sign matrix, and `bigint-cmp` orders negatives
/// correctly — the ops the compiler now emits for a runtime `/` and comparison (B3b `@acb1768f`). The
/// op-level glue test only checked `7/2` and `-7/2`; the other two sign combos + the truncation
/// DIRECTION (toward zero, NOT floor: `-7/2` is `-3` not `-4`) + negative cmp ordering were unpinned
/// at the op level (the library `divmod` is differential-tested, but a box/unbox sign-flag bug would
/// slip past that). A wrong truncation direction is a silent wrong answer for runtime negative BigInt
/// division.
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

/// `bigint-of-i64` boxes DIRECTLY via the i128 path (`box_bigint_i128`, no transient `Big`) — the leaf
/// MUST stay byte-identical to the old `box_bigint(&Big::from_i64(v))` route (both emit the canonical
/// `[sign][LE magnitude, trailing-zeros-stripped]` form). Pins that equivalence across the full i64
/// range — the endpoints (`i64::MIN`, whose `unsigned_abs` is a limb-boundary case), the i32 boundaries,
/// exactly 2^32 (single→double limb in `Big::from_i64`), and zero — so a future refactor of EITHER path
/// can't silently diverge (a divergent leaf would break BigInt map-key equality + narrowing). Also
/// checks the value round-trips through `bigint-to-i64-checked`.
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

/// `bigint-rem` (op 73, the `%` the compiler now emits for a runtime BigInt) — the remainder of
/// TRUNCATING division, so its sign is the DIVIDEND's, matching Rust `%` on i64 across the full sign
/// matrix. Backed by the same `divmod` as `bigint-div` (the `r` half), so `a == (a/b)*b + (a%b)`.
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

/// `bigint-div` by ZERO TRAPS (numeric-model.md — an unbounded range gives `n/0` no value). The
/// zero-divisor trap was covered only implicitly (via the `Big::divmod` differential returning `None`);
/// no test asserted the OP itself traps. This matters especially since `op_bigint_div` gained an i128
/// FAST PATH (`spec@9bcfb04e`): a zero divisor makes `checked_div` return `None`, so it falls THROUGH
/// to the `Big` path — which traps. This pins that the fast path does NOT swallow the trap (return a
/// bogus value); a regression that mis-handled `y==0` in the fast path would return instead of panic.
#[test]
#[should_panic]
fn bigint_div_by_zero_traps() {
    reset();
    let (a, b) = (op_bigint_of_i64(10), op_bigint_of_i64(0));
    let _ = op_bigint_div(a, b); // fail-fast: division by zero
}

/// `bigint-rem` by ZERO TRAPS (the `%` companion of div — same `divmod`-`None` origin, same i128
/// fast-path fall-through). Pins the op-level trap for the remainder path independently of div.
#[test]
#[should_panic]
fn bigint_rem_by_zero_traps() {
    reset();
    let (a, b) = (op_bigint_of_i64(10), op_bigint_of_i64(0));
    let _ = op_bigint_rem(a, b); // fail-fast: remainder by zero
}

/// `rational-of` with a ZERO DENOMINATOR TRAPS (a rational `n/0` has no value — the rational analogue
/// of ÷0). The trap fires BEFORE normalization (`op_rational_of` checks `den.is_zero()` after reading
/// both operands). Pins the construction-time trap the port hits building a Rational from computed
/// components.
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

/// DIFFERENTIAL FUZZ for the runtime Rational ops (R3a, ops 74-81) — the safety net the bigint ops have
/// (`differential_arithmetic_vs_num_bigint`, 5000 pairs) but Rational did NOT: the sibling's landing test
/// is FIXED inputs only, and the arithmetic's subtle logic (sign placement, gcd reduction, cross-multiply
/// add/sub/mul/div, cmp direction) is exactly where random inputs catch a bug a spot-check misses. Cross-
/// check every op against a self-contained `i128`-fraction reference (exact at fuzzer scale — small
/// operands; no new dep). The reference reduces + normalizes the SAME way the runtime must (gcd to lowest
/// terms, sign on the numerator, denominator strictly positive), so the runtime's `(num, den)` output
/// must equal it byte-for-byte — which also pins the map-key canonicalization (equal value → identical
/// normalized form). Denominators are forced nonzero (the zero-denom TRAP is covered by the fixed test).
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

/// A BigInt is a RAW-ONLY leaf compared by its `raw` bytes (`champ_eq`) and hashed over them
/// (`champ_hash`) — exactly like Bytes/String. So two BigInts that are EQUAL BY VALUE but reached by
/// DIFFERENT arithmetic MUST produce byte-IDENTICAL leaves, else they'd be distinct map/set keys and
/// `=` would wrongly return false. This holds only if every op returns a NORMALIZED `Big` (no trailing
/// zero limbs, no `-0`) and `to_sign_magnitude_bytes` is canonical. `bigint.rs` differential-tests
/// VALUES vs num-bigint but NOT this heap-leaf byte form — the property BigInt-as-map-key depends on.
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

/// `bigint-to-i64-checked` traps EXACTLY at the i64 range boundary: `i64::MAX`/`MIN` fit, one beyond
/// each traps. The op-glue test skipped the trap path ("a compiler/gate concern"), but the boundary is
/// the whole point of the CHECKED narrow (`Int64.of` an out-of-range BigInt must trap, not wrap). Build
/// `i64::MAX + 1` = `bigint-add(of(MAX), of(1))` and assert it panics (→ a wasm trap under abort).
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

/// The `op_bigint_*` glue on genuinely LARGE, MULTI-LIMB (>i64) values. Every other bigint test enters
/// via `op_bigint_of_i64` (≤64-bit), so the box/unbox of a multi-limb magnitude — several 4-byte limbs,
/// trailing-zero stripping ACROSS limb boundaries, the sign byte — and arithmetic PRODUCING/CONSUMING
/// >i64 values were untested through the heap. `bigint.rs` differential-tests the limb arithmetic vs
/// num-bigint, but NOT the heap round-trip. Build large `Big`s directly, box them, run the WIT ops, and
/// check the result unboxes to the value the library computes — pinning that the leaf byte codec and
/// each op thread multi-limb operands correctly. (B3b will emit exactly these >i64 BigInts.)
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

/// The `i128` arithmetic FAST PATH (add/sub/mul when both operands fit i128 and the result doesn't
/// overflow) must produce a leaf BYTE-IDENTICAL to the full `Big` SLOW path — the fast path is a pure
/// allocation optimisation, not a semantics change. Drives values that straddle the i128 boundary in
/// both directions: (a) both fit + result fits → fast path, result must `champ_eq` a freshly-boxed
/// `Big` result; (b) result OVERFLOWS i128 (e.g. `i128::MAX + 1`, `i128::MIN - 1`, `i128::MAX *
/// i128::MAX`) → falls back to the `Big` path, must still be canonical; (c) an OPERAND exceeds i128 →
/// fast path declined, `Big` path used. Also pins the `i128`↔bytes helpers via the op results. Guards
/// the fast/slow agreement the `num-bigint` differential (which goes through the ops) also protects,
/// but SPECIFICALLY at the overflow endpoints a random differential rarely hits exactly.
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

/// `bigint-div`/`-rem` i128 FAST PATH ↔ `Big` slow path at the boundary — the div/rem analogue of the
/// add/sub/mul boundary test above (they got the i128 fast path in a separate increment). The fast path
/// uses Rust's `checked_div`/`checked_rem` (truncate-toward-zero quotient, dividend-sign remainder —
/// EXACTLY `divmod`'s semantics), so the result leaf MUST be byte-identical (`champ_eq`) to boxing the
/// `Big`-`divmod` answer, across: (a) in-range operands + all four sign combos (fast path); (b) an
/// operand exceeding i128 → the fast path declines → `Big` runs; (c) the `i128::MIN / -1` OVERFLOW —
/// `checked_div`/`checked_rem` return `None`, so this MUST fall through to `Big` (the one non-zero case
/// where the native op can't represent the answer). A regression that dropped the `checked_*` guard
/// would panic here (overflow) instead of falling back.
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

/// DESIGN VALIDATION for the pending BigInt ESCAPE (B3c): a runtime BigInt crossing the host boundary
/// can REUSE the existing codec `KIND_INT` leaf (`DocLeaf::Int`, sign + big-endian magnitude) — it is
/// ALREADY arbitrary-precision (the magnitude is just bytes, NOT i64-bounded), matching the compiler's
/// `IntValue { negative, magnitude: Vec<u8> }`. So the escape needs NO new wire tag and NO
/// two's-complement form (the spec's "awaits the two's-complement encoding" note is over-conservative
/// for the value-encode path) — only a `Shape::BigInt` walk arm that reads the `Big`'s sign + BE
/// magnitude into a `DocLeaf::Int`. This test proves the reuse: build a >i64 `Big`, emit it as a
/// KIND_INT leaf the way that arm would, and confirm the serialized doc's leaf bytes are the exact
/// sign + big-endian magnitude (round-tripping to the same value). Guards the finding so B3c is a
/// runtime one-liner, not a wire-format change.
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

/// The `Shape::BigInt` value-encode arm (B3c, descriptor tag 17): a boxed runtime BigInt escapes via
/// `op_value_encode_form`, reading the value with `unbox_bigint` (arbitrary width, NOT i64-capped) and
/// rendering the SAME `KIND_INT` leaf as a fixed-width Int. Cover an i64-fitting value, a >i64 value
/// (i64::MAX² ≈ 2^126, the whole point), a negative, and zero — byte-identical to the recursive oracle
/// each time (the oracle's S::BigInt arm mirrors production), plus the exact KIND_INT sign+magnitude
/// for the >i64 case (proving the leaf is not i64-bounded).
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

/// The inline-vs-boxed equivalence above, but for a scalar child INSIDE A COMPOUND — the case that
/// exercises `champ_eq`/`champ_hash`/`champ_key_cmp`'s SHALLOW-COMPOUND fast path (children compared/
/// hashed via `with_raw_arity`, which must decode an immediate child and a boxed child IDENTICALLY).
/// The scalar test covers a bare int; this covers a `tuple(int, …)` where one tuple's int-child is an
/// IMMEDIATE and the twin's is a HAND-BOXED int of the same value. They must be eq + hash-equal +
/// cmp-Equal, AND behave as ONE map key: a key built with a boxed child and a key built with an
/// immediate child are the SAME key (lookup hits across the rep boundary; re-insert overwrites, size
/// unchanged). This is the canonical-form property that keeps a COMPOUND map key correct regardless of
/// how its scalar children were constructed (different build paths can yield either rep). A shallow-
/// path bug that only handled immediate-vs-immediate (or boxed-vs-boxed) children would mis-dedup here.
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

/// `value_cmp_shaped` — the descriptor-guided three-way BLESSED order (heap-ordering slice 2's runtime
/// core, still UNEXPORTED so hash-neutral). Covers the ordering rules v-inference blessed: Int by
/// NUMERIC value (incl. the negative case raw-byte order gets wrong), tuple lexicographic by field,
/// list lexicographic with a proper prefix LESS than its extension, sum by discriminant, consistency
/// with equality (Equal iff champ_eq), and the non-orderable declines (Float leaf → None).
#[test]
fn value_cmp_shaped_orders_by_blessed_per_leaf_and_lexicographic_rules() {
    use super::{Descriptor, Shape};
    use core::cmp::Ordering;
    reset();
    // Int: NUMERIC order, incl. negatives (raw little-endian bytes would sort -1 as huge).
    let desc_int = Descriptor {
        table: vec![Shape::Int],
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
        table: vec![Shape::Int, Shape::Tuple(vec![0, 0].into())],
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
        table: vec![Shape::Int, Shape::List(0)],
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
        table: vec![Shape::Float],
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
        table: vec![Shape::Bytes],
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
        table: vec![Shape::Bytes, Shape::Int, Shape::Tuple(vec![0, 1].into())],
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

/// `value_cmp_shaped` hardening: SUM by discriminant-then-payload, RECORD by field order, and a DEEPLY
/// nested list (the iterative-walk / wasm-safety claim — must not overflow the native stack).
#[test]
fn value_cmp_shaped_sum_record_and_deep_nesting() {
    use super::{Descriptor, Shape};
    use core::cmp::Ordering;
    reset();
    // Sum with two variants: 0 = A(Int), 1 = B(Int). Discriminant decides first; same variant → payload.
    let desc_sum = Descriptor {
        table: vec![
            Shape::Int,
            Shape::Sum(vec![("A".into(), 0), ("B".into(), 0)].into()),
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
            Shape::Int,
            Shape::Record(vec![("x".into(), 0), ("y".into(), 0)].into()),
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
    let mut table = vec![Shape::Int]; // 0 = Int
    let mut cur = 0u32;
    for _ in 0..200 {
        table.push(Shape::List(cur));
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

/// `value_eq_shaped` (the equality companion of `value_cmp_shaped`): handles the leaves value-cmp
/// DECLINES for ordering — a `List<Float>` compares element-wise by CANONICAL BYTE FORM (§313 float eq
/// total), which value-cmp can't (it declines the float leaf) and champ_eq can't (unsound for the
/// non-shape-canonical RRB spine). Pins: concat-vs-push List<Float> equal; a differing float → not equal;
/// NaN == NaN (canonicalized); -0.0 ≠ +0.0; a deep list-of-float nesting is stack-safe.
#[test]
fn value_eq_shaped_handles_float_leaves_and_list_spine() {
    use super::{Descriptor, Shape};
    reset();
    // desc: [0]=Float, [1]=List(0).
    let desc = Descriptor {
        table: vec![Shape::Float, Shape::List(0)],
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
        table: vec![Shape::Float],
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
    let mut table = vec![Shape::Float];
    let mut cur = 0u32;
    for _ in 0..200 {
        table.push(Shape::List(cur));
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

/// REGRESSION ISOLATION for the Bytes-total-order slice: a `List<Bytes>` whose element is a runtime SLICE
/// VIEW must compare equal to its flat twin under `value_cmp_shaped` (op=Eq path), just as `value_eq_shaped`
/// already did — the per-leaf Bytes arm must flatten the view (`bytes_flatten`) before comparing `raw`.
#[test]
fn value_cmp_shaped_flattens_a_bytes_slice_view_list_element() {
    use super::{Descriptor, Shape};
    use core::cmp::Ordering;
    reset();
    // desc: [0]=Bytes, [1]=List(0).
    let desc = Descriptor {
        table: vec![Shape::Bytes, Shape::List(0)],
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

/// THE list-key miscompile fix (`value_canonicalize_shaped`): a Map with a CONCAT-built list KEY must be
/// found by a PUSH-built equal key AFTER canonicalizing both keys, at sizes straddling the leaf/multi-
/// level boundary (n≤32 already collapsed; n≥33 was the false-miss). Also nested (`(tuple (list) Int)`),
/// and a genuinely-different list must still MISS. Leak-clean: canonicalize BORROWS its input and returns
/// a fresh owned key; dropping the map + the two fresh canonical keys per size must net to 0 live cells.
#[test]
fn value_canonicalize_makes_concat_and_push_list_keys_collide() {
    use super::{Descriptor, Shape};
    reset();
    // desc: [0]=Int, [1]=List(0). A concat-built and a push-built [0..n) canonicalize byte-identical.
    let desc_list = Descriptor {
        table: vec![Shape::Int, Shape::List(0)],
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
        table: vec![Shape::Int, Shape::List(0), Shape::Tuple(vec![1, 0].into())],
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

/// `value_canonicalize_shaped` is ITERATIVE (wasm-safe): a 200-deep nested list canonicalizes without
/// overflowing the native stack, and the result reads back to the same innermost leaf.
#[test]
fn value_canonicalize_deep_nested_list_is_stack_safe() {
    use super::{Descriptor, Shape};
    reset();
    let mut table = vec![Shape::Int];
    let mut cur = 0u32;
    for _ in 0..200 {
        table.push(Shape::List(cur));
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

// NOTE (serializer / value-interchange): the runtime crate has NO value-interchange / Ast
// serialization path that reads `node.raw` — the only value-observing surfaces are `render`
// (covered above: inline and boxed ints render identically) and the `to_u32`/`from_u32` ABI
// (identity casts, covered by the ABI round-trip tests). Ast encode/decode lives in
// `cdz-compiler/src/ast.rs` over the compiler's syntax `Node`, never a runtime `Handle`, so there
// is nothing serialization-shaped to test from here. Flagged as a cross-boundary review item.

// ── Latent-hardening (review follow-ups): reuse-to-0 normalize + defensive guard set ──

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

// ── Scalars ─────────────────────────────────────────────────────────────────────────

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

// ── Arr (tuple / record / list) ───────────────────────────────────────────────────────

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

// ── Sum ───────────────────────────────────────────────────────────────────────────────

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

// ── Bytes ───────────────────────────────────────────────────────────────────────────────

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

/// `op_bytes_alloc` builds a ≤INLINE_RAW_CAP-byte buffer with an INLINE raw (no transient `vec![0;
/// len]`) and a longer one on the heap. Guards the two paths agree on value AND representation: a
/// small leaf's raw must be inline (the perf win) while still set/get/len-ing identically to a large
/// heap leaf, and both must render + compare (champ_eq) the same as the other rep would. (Rep
/// divergence behind Raw's Deref is invisible to a value-only check — iter-29's lens — hence the
/// explicit raw_is_heap assertions.)
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

// ── String ──────────────────────────────────────────────────────────────────────────────

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

/// `str-get` (op 18) on a ROPE String must return the logical CONTENT, not the rope node's header
/// bytes. A runtime String IS a bytes rope (`String.concat`/`.at`-slice build concat/slice nodes,
/// sharing the Bytes representation, `@b77b3ae0`), so a concat/slice String reaching `str-get` is NOT
/// a flat leaf — before the `bytes_flatten` fix, `op_str_get` read the concat node's `raw=[len]` (4
/// bytes) as UTF-8 and returned garbage ("\u{7}\0\0\0" for a 7-byte rope). This is the SAME latent bug
/// the value-encode `Shape::Str` arm was hardened against; `str-get` had no emit site yet (the compiler
/// returns a String via the value-encode escape, not `str-get`), so it was unreached — but wiring a
/// direct String return would have silently corrupted every rope. Build a rope with a multi-byte scalar
/// spanning a seam and assert it reads back byte-for-byte equal to the flat twin.
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

/// `op_str_from_bytes` — the READY-BUT-UNEXPORTED load-bearing half of the coordinated `str-from-bytes`
/// op (a total UTF-8 decode `Bytes → (Option String)`; the compiler-in-Cadenza port's decode/encode
/// string content is blocked on it — `String.from-bytes` on a runtime Bytes declines at lower.rs). Pins
/// the contract so the compiler's eventual `Core::StrFromBytes` emit calls a PROVEN fn: (1) valid UTF-8
/// → the buffer AS a String, byte-identical to `op_str_new` (a String IS a byte leaf); (2) a ROPE input
/// flattens first (the runtime-built-Bytes shape — `Bytes.concat`); (3) strict rejection of invalid
/// bytes, an overlong encoding, AND a surrogate (the three spec failure modes) → NULL; (4) empty → valid
/// ""; (5) no leak (consumes `buf`; a valid result is dropped, an invalid one already released).
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

/// `op_bytes_scalar_at(buf, i)` — the codepoint of the i-th UNICODE SCALAR, or `u32::MAX` out of range.
/// The op a real text lexer wants: a `Char` codepoint (an immediate integer, compared by a plain
/// `i32.eq`), sidestepping the `String.at` slice-rope content-eq hazard the compiler-in-Cadenza lexer
/// works around. Covers: (1) ASCII by-scalar read; (2) MULTI-BYTE where the SCALAR index ≠ the BYTE
/// index (`"café"` byte-len 5, scalar 3 = 'é' = 233); (3) a 4-byte scalar (emoji U+1F600); (4) a ROPE
/// input (flatten across the concat seam); (5) out-of-range + empty/immediate → the `u32::MAX` sentinel;
/// (6) it BORROWS (no consume — the buffer survives + reads again, node count balances).

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

// ── Map ─────────────────────────────────────────────────────────────────────────────────

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

// ── Compound-of-compound: a record containing a list, a sum, bytes, and a string ──────────

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

/// The SAME record-with-heap-fields shape as `deeply_nested_render`, but through the REAL value-encode
/// ESCAPE (`op_value_encode_form` — the iterative work-stack walk + `Descriptor` decode that crosses the
/// host boundary), not the simpler `render` helper. This is the compiler-in-Cadenza port's AST-NODE
/// shape: a Record whose FIELDS are themselves heap compounds (a List, a Sum, Bytes, a String) — the
/// exact thing it serializes. Every OTHER record-encode test (`value_encode_wide_record`) uses all-SCALAR
/// fields, so the escape's `Shape::Record`-over-HEAP-FIELDS assembly (push each field's subtree onto the
/// work stack, pop them in field order into the record assembler) was unexercised by the real escape.
/// Checks: (1) DIFFERENTIAL — the iterative production walk byte-matches the recursive oracle (two
/// independent implementations); (2) INDEPENDENT ANCHOR — the field NAME leaves appear in field order
/// and the String field content "hi" is present (a wrong field interleaving on the work stack would
/// reorder them); (3) no leak.
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

// ── Birth refcount: every node is born with refcount 1 ────────────────────────────────────

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

// ── Tagless totality: scalar/null reads are total; OOB into a valid node traps ────────────

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

// ── Perceus reference counting ────────────────────────────────────────────────────────────

/// Current count of live (allocated, not-yet-freed) nodes on this test thread. Tests measure
/// DELTAS against a baseline captured at their start.
fn live_nodes() -> i64 {
    LIVE_NODES.with(|n| n.get())
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

/// A DAG within ONE value — a single root that reaches the SAME shared child via TWO distinct PATHS
/// (the hash-consing / structural-sharing shape a CSE pass produces: `9a35fbac`). The prior test shares
/// a child under two SEPARATE root handles dropped one at a time; this shares it inside ONE value and
/// drops that ONE root in a single `op_drop`, so the free cascade VISITS the shared child TWICE —
/// exercising the "shared (rc>1) → decrement, DON'T recurse" arm on the first visit and the "unique
/// (rc==1) → recurse + free" arm on the second (lib.rs `n.rc > 1` at ~3260). A cascade that freed on
/// the first visit would UAF the second path; one that never decremented would leak. Shape:
/// `root = tuple(inner, tuple(inner, 9))` with `inner = tuple(7)` shared (rc==2, one ref per path).
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

// ── RC calling convention: the emitted-sequence mirror ────────────────────────────────────
// Each test SIMULATES the exact dup/drop sequence the compiler must emit for a pattern and
// asserts, via LIVE_NODES, both properties the convention
// guarantees: NO LEAK (heap returns to baseline) and NO EARLY FREE (kept values stay intact
// until their last owner). These are the reference behaviors the compiler's emission reproduces;
// a failing test would mean the primitives cannot support the prescribed convention.

/// §3.5 / §4 — projection kept past the parent: `(let t (tuple a b) (arr-get t 0))`. The
/// element is RETURNED, so it must be dup'd BEFORE the parent is dropped; then dropping the
/// tuple frees the tuple node + the not-kept sibling, leaving the kept element valid.
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

/// The NESTED-COMPOUND variant of the projection-escape — the `spec@76aa1bdc` UAF shape. The test
/// above keeps a FLAT-leaf child; there the free-cascade's "stop at rc>1" has nothing below to wrongly
/// free. `76aa1bdc` was a Perceus UAF where a projection extracted a nested-compound child (a boxed sum
/// `W.Atom(payload)` — a child WITH its own subtree) out of an aggregate and kept it, but the compiler
/// dropped the aggregate anyway → the free-cascade descended into the escaped child and freed its
/// subtree (a use-after-free). That was a COMPILER emit bug (fixed compiler-side); the RUNTIME property
/// it relies on — `op_drop` of an aggregate whose nested-compound child was dup'd-out (rc≥2) must
/// decrement that child and NOT recurse into its subtree — is exercised here: a flat-leaf test can't
/// (no subtree to wrongly free). Pins that the cascade stops at a shared child WITH children, leaving
/// the whole subtree intact + reclaiming cleanly on the child's own last drop.
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

/// The MIRROR of the projection-escape above, and the runtime contract the compiler's fix for the
/// mutual-recursion still-live-binding miscompile RELIES ON (`spec@6db817a3`: an `fc↔fl` walk consumes
/// a node's shared child payload while a sibling operand still reads the parent — the idiomatic
/// homoiconic-Ast resolver shape; fix = "dup the aggregate/collection operand of a (mutual-)recursive
/// call whose callee consumes a payload a sibling still reads"). Here we KEEP the parent and DUP-then-
/// fully-CONSUME a payload reference (owner A = the consuming recursive walk), asserting owner B (the
/// parent + a SIBLING read of it) is UNCORRUPTED. The existing projection test drops the PARENT and
/// keeps the child; this keeps the parent and consumes a dup'd payload ref — the shape the resolver hits.
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

/// §3.5 — `match Some(x) => x`: dup the borrowed payload, then drop the scrutinee. Payload
/// survives; the sum node is reclaimed.
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

/// §3.5 (no-keep arm) — `match Some(_) => 0`: the payload is NOT kept, so no dup; dropping the
/// scrutinee reclaims the whole sum INCLUDING the payload.
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

/// §3.3 — the duplicate-binder question, answered: `(tuple x x)` is a `dup`, not an error. The
/// tuple owns TWO references to the same child; dropping the tuple reclaims the child exactly
/// once (rc 2->1->0 across the two owned slots).
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

/// §3.4 — branch balancing. `(if c xs ys)` returns one of two owned lists, both live at the
/// `if`; each arm drops the not-returned one. Correct for BOTH values of `c`: no leak, no
/// double-free either way.
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

/// §3.1 — a bound-but-unused heap value (`(let x (tuple …) 0)`) is dropped at scope end;
/// baseline restored.
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

// ── Reuse / FBIP ───────────────────────────────────────────────────────────────────────────
// `reset` + the `*-reuse` constructors give in-place update on unique data. The tests assert
// the two load-bearing properties: (1) reuse is IN PLACE — the rebuilt node is the SAME
// allocation (address identity + zero net LIVE_NODES growth), the whole point over free→malloc;
// (2) reuse is FRAME-LIMITED — it fires ONLY on a unique node, so a shared value (a persistent
// structure's other version) is never clobbered and peak heap cannot grow.

/// `reset` on a UNIQUE node yields its shell as a non-null token, drops its owned children, and
/// keeps exactly one node live (the emptied shell) — ready to be refit.
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

/// `reset` on a SHARED node declines: it returns NULL, decrements, and leaves the node (and its
/// children) fully intact for the other owner. This is the frame-limiting guard — a persistent
/// structure's shared version is never reused out from under it.
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

/// A null token makes the reuse constructors behave EXACTLY as their plain forms (fresh alloc),
/// so a declined `reset` is transparent to the emitted rebuild code.
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

/// `arr-alloc-reuse` with a real token refits the SAME shell — address identity, no new node.
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

/// A reuse TOKEN whose shell came from a node with a HEAP-backed raw (a bytes/string leaf longer
/// than the inline cap) must NOT leave the reused node carrying that heap raw: `op_sum_new_reuse`
/// and `op_arr_alloc_reuse` normalize the raw back to INLINE, matching what the fresh constructors
/// produce. (Regression guard: the old `raw.clear()` + `extend_from_slice` kept a heap buffer — a
/// stray retained allocation AND a non-canonical storage rep for one logical value; the value stayed
/// byte-equal via Deref so hash/eq tests could NOT have caught it, hence this explicit rep check.)
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

/// The HANDLES-arm twin of `reuse_ctor_normalizes_a_heap_raw_token_to_inline`. A reuse TOKEN whose
/// shell came from a WIDE node (arity > `INLINE_HANDLES_CAP`, so its handle vector spilled to the
/// heap) must NOT leave the reused node carrying that heap Vec when it is refit to a ≤cap child
/// count: `op_arr_alloc_reuse`/`op_sum_new_reuse` normalize the HANDLES back to INLINE, matching what
/// the fresh constructors (`op_arr_alloc`/`op_sum_new`, which build ≤cap nodes inline) produce.
///
/// (Regression guard: `Handles::clear()`/`resize`/`push` all KEEP the `Heap` arm — `clear` retains
/// the Vec's capacity, and `push`/`resize` only SPILL inline→heap, never re-inline heap→inline. So a
/// wide token refit small kept a stray heap Vec for the node's life AND a non-canonical storage rep
/// for one logical value. Byte-equal via `as_slice` → `champ_eq`/`champ_hash` could NOT catch it,
/// hence this explicit rep check — the same class the raw-arm guard covers.)
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

/// `sum-new-reuse` with a token repurposes the SAME shell as the new `(disc, payload)` node.
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

/// The headline FBIP property: mapping a function over a UNIQUE list rebuilds it with ZERO net
/// allocation. Emitted per element: dup the elements to keep → reset the old cons/array shell →
/// arr-alloc-reuse it → refill. Peak heap never exceeds the input's node count + the transient
/// working set; the rebuilt list occupies the SAME shells as the input.
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

/// The ordering invariant for reset (the §4 dup-before-drop rule): a child of the old node that
/// the rebuild KEEPS must be dup'd BEFORE `reset`, because reset drops the old node's child
/// references. With the dup, the kept child survives into the reused shell.
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

// ── Persistent vector (32-way radix trie) ──────────────────────────────────────────────────
// The two load-bearing properties, mirrored from the rope/RC suites: (1) the OBSERVABLE contract
// — push/get/update/len denote a dense immutable sequence, and old versions are unchanged by an
// operation on a new one (PERSISTENCE); (2) RESOURCE behavior — path-copying shares subtrees
// (bounded per-op allocation, not O(N) copy), the whole trie reclaims to baseline on drop via the
// existing iterative cascade, and peak heap stays bounded across a build/drop loop.

/// Read a whole vector into a Rust Vec of ints, via the borrowing `vec-get` — the mirror the
/// compiler's renderer will drive (`vec-len` then `vec-get` over `0..len`).
fn vec_to_ints(v: Handle) -> Vec<i64> {
    (0..op_vec_len(v))
        .map(|i| op_get_int(op_vec_get(v, i)))
        .collect()
}

/// Build a vector [0,1,…,n-1] of boxed ints by repeated push. Each push consumes the running
/// vector and returns the next, so the final handle is the sole owner of the whole sequence.
fn vec_range(n: i64) -> Handle {
    let mut v = op_vec_empty();
    for i in 0..n {
        v = op_vec_push(v, op_box_int(i));
    }
    v
}

/// Build a RELAXED interior node from `child_sizes`: child `i` is a strict leaf holding a run of
/// consecutive ints so that the whole vector reads back as `[0, 1, …, total-1]`. The leaves have
/// IRREGULAR sizes (not all `1 << level`), which is exactly what makes the parent relaxed; the
/// parent's `raw` is the cumulative size table `[s0, s0+s1, …, total]` (u32 LE), and the returned
/// handle is a normal vector HEADER at shift `VEC_BITS` owning that relaxed root. This is the only
/// way to exercise the relaxed read path in U1, since normal push/update never build a relaxed node.
fn vec_relaxed_of(child_sizes: &[u32]) -> Handle {
    let mut handles = Vec::with_capacity(child_sizes.len());
    let mut raw = Vec::with_capacity(4 * child_sizes.len());
    let mut running = 0u32;
    for &sz in child_sizes {
        // A strict leaf holding `sz` consecutive ints starting at `running`.
        let mut leaf_handles = Vec::with_capacity(sz as usize);
        for k in 0..sz {
            leaf_handles.push(op_box_int((running + k) as i64));
        }
        handles.push(alloc(leaf_handles, Vec::new()));
        running += sz;
        raw.extend_from_slice(&running.to_le_bytes());
    }
    let root = alloc(handles, raw); // raw.len() == 4*arity ⇒ relaxed
    vec_alloc_header(running, VEC_BITS, root)
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

/// Recursively validate every RELAXED node in a subtree (rooted at `node`, whose top level is
/// `level == shift`) and return the subtree's element count. Asserts each relaxed node's size table
/// is strictly increasing, each entry equals the running sum of its children's counts, and the last
/// entry equals the subtree total — the U1 invariants a broken concat would violate. Strict nodes
/// carry no table (nothing to check); a leaf (`level == 0`) contributes `arity` elements.
fn assert_relaxed_invariants_rec(node: Handle, level: u32) -> u32 {
    if level == 0 {
        return vec_arity(node) as u32; // leaf: elements are its handles, uniformly size-1
    }
    let arity = vec_arity(node);
    let mut child_counts = Vec::with_capacity(arity);
    let mut total = 0u32;
    for i in 0..arity {
        let c = assert_relaxed_invariants_rec(vec_child(node, i), level - VEC_BITS);
        child_counts.push(c);
        total += c;
    }
    if vec_is_relaxed(node) {
        let mut running = 0u32;
        let mut prev = 0u32;
        for (i, &cc) in child_counts.iter().enumerate() {
            assert!(cc > 0, "no zero-size child in a relaxed node (child {i})");
            running += cc;
            let s = vec_relaxed_size_at(node, i);
            assert!(
                s > prev,
                "relaxed size table strictly increasing at {i}: {s} <= {prev}"
            );
            assert_eq!(
                s, running,
                "cumulative entry {i} == running child-count sum"
            );
            prev = s;
        }
        assert_eq!(prev, total, "last size-table entry == subtree total");
    }
    total
}

/// Assert a vector's whole tree honors the relaxed-node invariants, and its header count matches
/// the recomputed leaf total.
fn assert_vec_invariants(v: Handle) {
    let (count, shift, root) = vec_read_header(v);
    if count == 0 {
        return;
    }
    let leaf_total = assert_relaxed_invariants_rec(root, shift);
    assert_eq!(leaf_total, count, "header count == recomputed leaf total");
}

/// Concat two runtime vectors of the given ranges and check the result against the concatenation
/// of the two oracles, element by element, plus length and the relaxed invariants. Consumes the
/// two built vectors (concat is a constructor); drops the result; asserts no leak.
fn check_concat(la: i64, lb: i64) {
    let before = live_nodes();
    let a = vec_range(la);
    let b = vec_range(lb);
    // Oracle: a is [0..la), b is [0..lb); concat is those two runs back to back.
    let mut oracle: Vec<i64> = (0..la).collect();
    oracle.extend(0..lb);
    let c = op_vec_concat(a, b);
    assert_eq!(
        op_vec_len(c) as i64,
        la + lb,
        "concat len == la+lb for ({la},{lb})"
    );
    assert_vec_invariants(c);
    assert_eq!(
        vec_to_ints(c),
        oracle,
        "concat elements match oracle for ({la},{lb})"
    );
    op_drop(c);
    assert_eq!(live_nodes(), before, "no leak for concat({la},{lb})");
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

/// `vec-concat` is ASSOCIATIVE: `(a ++ b) ++ c` and `a ++ (b ++ c)` denote the same list. This is the
/// runtime foundation for the compiler's `List.concat` associativity law (pinned in corpus
/// `spec@5783cabb`). It is the SUBTLE RRB property: the two associations run the relaxed-node rebalance
/// with DIFFERENT boundaries, so they build DIFFERENT internal tree SHAPES — but an RRB vector is
/// element-canonical, NOT shape-canonical (concat leaves relaxed interior nodes), so the invariant is
/// OBSERVABLE equivalence, NOT `champ_eq`: the element sequence (`vec_to_ints`) and the value-encode
/// (renders by `op_vec_get` in order) must agree, and each result's RRB structural invariants must hold.
/// Covers a systematic size matrix straddling the `VEC_BITS=5` (32) + multi-level boundaries so the
/// rebalance genuinely runs on both sides — a range the corpus (fixed sizes) cannot reach. Distinct
/// value ranges per operand make any element-order divergence detectable (not masked by equal values).
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

/// Split a runtime vector [0..len) at `index` and check both halves against the oracle: left is
/// [0..index), right is [index..len). Validates lengths, element-wise contents, and the relaxed
/// invariants on both outputs; drops both; asserts no leak.
fn check_split(len: i64, index: u32) {
    let before = live_nodes();
    let v = vec_range(len);
    let (l, r) = op_vec_split(v, index);
    let idx = index.min(len as u32);
    assert_eq!(
        op_vec_len(l),
        idx,
        "left len == index for (len={len}, idx={index})"
    );
    assert_eq!(
        op_vec_len(r),
        len as u32 - idx,
        "right len == len-index for (len={len}, idx={index})"
    );
    assert_vec_invariants(l);
    assert_vec_invariants(r);
    let left_want: Vec<i64> = (0..idx as i64).collect();
    let right_want: Vec<i64> = (idx as i64..len).collect();
    assert_eq!(
        vec_to_ints(l),
        left_want,
        "left elements (len={len}, idx={index})"
    );
    assert_eq!(
        vec_to_ints(r),
        right_want,
        "right elements (len={len}, idx={index})"
    );
    op_drop(l);
    op_drop(r);
    assert_eq!(
        live_nodes(),
        before,
        "no leak for split(len={len}, idx={index})"
    );
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

/// `vec-drop(v, index)` (op 72, the `(list p… .. rest)` REST-pattern binder) returns the TAIL
/// `[index, len)` as one vector and RECLAIMS the dropped prefix `[0, index)`. It's `vec-split` keeping
/// only the right half — a single-u32 result, CONSUMING `v`. Landed `@494d2e44` with no runtime test.
/// This mirrors the wit wrapper's EXACT body (`op_vec_drop_tail`, the build-only-the-tail path) and
/// guards: correct tail content across offsets, the `index==0` (whole) + `index>=len` (empty) edges,
/// the result's RRB invariants, and — since it's consuming and reclaims the prefix — NO LEAK.
fn vec_drop_impl(v: Handle, index: u32) -> Handle {
    op_vec_drop_tail(v, index)
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

/// `op_vec_drop_tail` (build-only-the-tail) must be BYTE-IDENTICAL to the old `split`+drop-left it
/// replaced — same tail content AND same canonical RRB shape (`champ_eq`), just ~half the allocation
/// (no discarded left prefix). Differential across offsets on BOTH a strict (push-built) and a RELAXED
/// (concat-built) vector — the relaxed case is where the boundary-node rebuild + size-table recompute
/// must match. Also covers a 3-level vector (full-depth descent) and the whole/empty edges.
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

/// Build a flat `arr` of boxed ints [lo, lo+1, …, hi-1] — the compiler's `(list …)` pre-step that
/// `op_vec_of_arr` then ingests. (arr-alloc + arr-set, exactly the tuple/record primitive.)
fn arr_of_ints(lo: i64, hi: i64) -> Handle {
    let n = (hi - lo) as u32;
    let a = op_arr_alloc(n);
    for i in 0..n {
        op_arr_set(a, i, op_box_int(lo + i as i64));
    }
    a
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

// ── U4: FBIP rc==1 in-place spine reuse for vec-push / vec-update ───────────────────────────
// The load-bearing property is ALIASING SAFETY: a push/update on a SHARED version (rc>1) must
// path-copy and leave the other version byte-identical; the FBIP win (in-place refit) fires ONLY
// when the touched spine is uniquely owned. These tests pin both halves.

/// Assert a shared version survives a push on the other owner (both node kinds via `make`).
fn check_push_shared_safe(make: impl Fn() -> Handle, orig_len: i64) {
    let before = live_nodes();
    let v1 = make();
    let orig: Vec<i64> = vec_to_ints(v1);
    assert_eq!(orig.len() as i64, orig_len);
    op_dup(v1); // rc(header) == 2: v1 is now a SHARED version
    let v2 = op_vec_push(v1, op_box_int(77_000));
    // v1 (the shared version) is UNCHANGED — not mutated in place.
    assert_eq!(
        op_vec_len(v1) as i64,
        orig_len,
        "shared version keeps its length"
    );
    assert_eq!(
        vec_to_ints(v1),
        orig,
        "shared version byte-identical after other owner's push"
    );
    // v2 has the pushed element appended.
    assert_eq!(op_vec_len(v2) as i64, orig_len + 1);
    assert_eq!(op_get_int(op_vec_get(v2, orig_len as u32)), 77_000);
    for (i, &x) in orig.iter().enumerate() {
        assert_eq!(
            op_get_int(op_vec_get(v2, i as u32)),
            x,
            "v2 prefix matches v1"
        );
    }
    assert_vec_invariants(v1);
    assert_vec_invariants(v2);
    op_drop(v1);
    op_drop(v2);
    assert_eq!(live_nodes(), before, "no leak / no double-free");
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

/// Assert a shared version survives an update on the other owner (both node kinds via `make`).
fn check_update_shared_safe(make: impl Fn() -> Handle, len: i64, idx: u32) {
    let before = live_nodes();
    let v1 = make();
    let orig: Vec<i64> = vec_to_ints(v1);
    assert_eq!(orig.len() as i64, len);
    op_dup(v1); // shared version
    let v2 = op_vec_update(v1, idx, op_box_int(-999));
    // v1 unchanged at idx (and everywhere).
    assert_eq!(
        op_get_int(op_vec_get(v1, idx)),
        orig[idx as usize],
        "shared version unchanged at idx"
    );
    assert_eq!(vec_to_ints(v1), orig, "shared version byte-identical");
    // v2 changed at idx, equal elsewhere.
    assert_eq!(
        op_get_int(op_vec_get(v2, idx)),
        -999,
        "new version changed at idx"
    );
    for i in 0..len as u32 {
        if i != idx {
            assert_eq!(
                op_get_int(op_vec_get(v2, i)),
                orig[i as usize],
                "v2 equals v1 off the path"
            );
        }
    }
    assert_vec_invariants(v1);
    assert_vec_invariants(v2);
    op_drop(v1);
    op_drop(v2);
    assert_eq!(live_nodes(), before, "no leak / no double-free");
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

// ── Packed-bool vector leaves (memory-dense `List Bool`) ────────────────────────────────────
// A `List Bool` stores each leaf's ≤32 bools BIT-PACKED (`[count][bits]`, 5 inline bytes) instead
// of as up to 32 handles in a heap Vec. The properties below mirror the int-vector suite: the
// OBSERVABLE contract (get/len/push/update/of-arr denote the same bool sequence, byte-interchangeable
// with an unpacked build), and RESOURCE behavior (packed = one node with NO heap Vec, drops clean).

/// Read a whole bool vector into a Rust `Vec<bool>` via the borrowing `vec-get` (mirrors the
/// renderer: `vec-len` then `vec-get` over `0..len`, decoding each element with `op_get_bool`).
fn vec_to_bools(v: Handle) -> Vec<bool> {
    (0..op_vec_len(v))
        .map(|i| op_get_bool(op_vec_get(v, i)))
        .collect()
}

/// Build a bool vector by repeated push (each push consumes the running vector). The final handle
/// solely owns the whole sequence — and each leaf is packed by construction.
fn vec_of_bools(bs: &[bool]) -> Handle {
    let mut v = op_vec_empty();
    for &b in bs {
        v = op_vec_push(v, op_box_bool(b));
    }
    v
}

/// Build an array node whose elements are `bs` as bool immediates (the `(list …)` literal shape a
/// `vec-of-arr` lowers from).
fn arr_of_bools(bs: &[bool]) -> Handle {
    let a = op_arr_alloc(bs.len() as u32);
    for (i, &b) in bs.iter().enumerate() {
        op_arr_set(a, i as u32, op_box_bool(b));
    }
    a
}

/// A deterministic LCG-driven bool pattern of length `n` (bit 40 of the state), for stress tests.
fn bool_pattern(n: usize, seed: u64) -> Vec<bool> {
    let mut lcg = seed;
    (0..n)
        .map(|_| {
            lcg = lcg
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (lcg >> 40) & 1 != 0
        })
        .collect()
}

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

/// Assert every level-0 leaf under `v`'s root is a PACKED-bool leaf (the density invariant a
/// `List Bool` should hold after any op). A bool vector built ANY way — push, of-arr, concat,
/// split — must satisfy this: packing is never lost.
fn assert_all_bool_leaves_packed(v: Handle) {
    fn rec(node: Handle, level: u32) {
        if level == 0 {
            assert!(
                vec_leaf_is_packed(node),
                "every bool leaf stays packed after the op"
            );
            return;
        }
        for i in 0..vec_arity(node) {
            rec(vec_child(node, i), level - VEC_BITS);
        }
    }
    let (count, shift, root) = vec_read_header(v);
    if count == 0 {
        return; // empty vector has no leaf
    }
    rec(root, shift);
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

// ── Bytes rope (O(1) concat/slice over shared leaves) ─────────────────────────────────────
// Two load-bearing property groups:
// (1) the OBSERVABLE contract — concat/slice/compact denote the same Bytes a copy would, by
// `bytes-len`/`bytes-get`/logical-equality, and are associative-by-content; (2) the RESOURCE win
// — concat/slice allocate ONE node (no byte copy), a deep concat chain reads out in O(total) not
// O(n²) via flatten-on-access, the whole rope reclaims on drop, and a shared leaf survives.

/// Build a leaf Bytes from a slice, via the existing alloc/set path.
fn bytes_leaf(data: &[u8]) -> Handle {
    let b = op_bytes_alloc(data.len() as u32);
    for (i, &v) in data.iter().enumerate() {
        op_bytes_set(b, i as u32, v as u32);
    }
    b
}
/// Read a whole Bytes into a Rust Vec via the borrowing `bytes-get` — the compiler's emit loop.
fn bytes_to_vec(h: Handle) -> Vec<u8> {
    (0..op_bytes_len(h))
        .map(|i| op_bytes_get(h, i) as u8)
        .collect()
}

#[test]
fn rope_concat_round_trip() {
    reset();
    let c = op_bytes_concat(bytes_leaf(&[1, 2]), bytes_leaf(&[3, 4]));
    assert_eq!(op_bytes_len(c), 4);
    assert_eq!(bytes_to_vec(c), vec![1, 2, 3, 4]);
    op_drop(c);
}

/// `mark-immortal` (index 95): converting a build-once static heap node makes it CENSUS-EXCLUDED (the
/// live-objects count nets to zero — an immortal held by a module global is not a leak) and makes
/// `dup`/`drop` NO-OPS on it (the global owns it for the whole instance; a consumer's `global.get` +
/// harmless no-op drop reads it intact). Deltas from a captured baseline (`reset` is a no-op here).
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

/// The EMBEDDING sharp-edge (v-static-data increment 6): a hoisted IMMORTAL constant embedded as a CHILD
/// of a RUNTIME compound survives when that runtime parent is recursively dropped — `op_drop`'s cascade
/// NO-OPS on the immortal child (never decrementing/freeing it), so a `(tuple <static> 42)` built at
/// runtime and dropped leaves the static intact + readable + census-neutral (no UAF).
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

/// DEEP mark-immortal (op 96, v-static-data large-list/map build-once hoist): `op_mark_immortal_deep`
/// marks a MULTI-NODE structure AND its payloads immortal transitively — the RRB list's interior/leaf
/// nodes AND its element handles, the CHAMP map's interior nodes AND its `[k,v]` payload handles — so a
/// build-once constant list(>32)/map nets to ZERO census (no leak) and every node dup/drop-no-ops (no
/// UAF under a runtime consumer). The crux (v-static-data): the ELEMENTS/KEYS/VALUES, not just the
/// spine, must be marked — asserted via `node_rc` on a read-back element/value.
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

/// DEEP mark-immortal (op 96) is DAG-SAFE: persistent structures SHARE nodes, so a node reachable via
/// two paths must be marked EXACTLY ONCE — a re-visit is skipped (rc already IMMORTAL), decrementing
/// the census once per DISTINCT node, never twice. This pins the no-double-census-decrement invariant
/// v-core-opt's large-list/map build-once hoist relies on: a deep-mark that re-decremented a shared
/// node would push the census BELOW `base` (caught here), and a shared-live node marked/freed on the
/// wrong path would corrupt the other owner. The `== base` assert IS the DAG check.
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

/// A MULTI-NODE deep-immortal value nested under a MORTAL shell survives the shell's drop: `op_drop`'s
/// free cascade SKIPS an IMMORTAL child WITHOUT recursing into or decrementing its subtree, so dropping
/// the mortal parent frees ONLY the parent — the whole immortal list (spine + leaves + elements) is
/// untouched. This pins the double-reclaim-safety v-core-opt relies on when a hoisted deep-immortal
/// static is embedded in an ordinary refcounted value it reclaims.
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

/// The DEBUG-build USE-AFTER-FREE detector (operator safety net for the leak-reclaim work: "UAF is
/// much worse than leaks"). The free path bumps a dedicated `generation` field ODD (= freed) and
/// retains the cell — kept SEPARATE from `rc` so the refcount stays pure — so a DOUBLE-DROP is caught
/// as a loud panic instead of corrupting the heap. This is exactly the failure an UNSOUND reclaim drop
/// produces (a value dropped while another owner is live → that owner's later drop is a double-free) —
/// now a red run, not a shipped silent bug.
#[test]
#[should_panic(expected = "use-after-free")]
fn double_drop_is_caught_as_use_after_free() {
    let h = bytes_leaf(&[1, 2, 3]);
    op_drop(h); // last ref → frees, poisoning the retained cell
    op_drop(h); // the double-free: drop of a freed node → UAF panic
}

/// Dup-after-free (retaining a freed cell — the other half of an unsound reclaim) is caught too; the
/// guard precedes the rc bump so the poison is never silently incremented.
#[test]
#[should_panic(expected = "use-after-free")]
fn dup_after_free_is_caught_as_use_after_free() {
    let h = bytes_leaf(&[4, 5]);
    op_drop(h);
    op_dup(h); // dup of a freed node → UAF panic
}

/// Read-after-free through the central reader (`node_rc` → `with_node`) is caught — so a freed value
/// consumed by any accessor surfaces the UAF rather than reading poisoned/garbage bytes.
#[test]
#[should_panic(expected = "use-after-free")]
fn read_after_free_is_caught_as_use_after_free() {
    let h = bytes_leaf(&[6, 7]);
    op_drop(h);
    let _ = node_rc(h); // read of a freed node → UAF panic
}

/// Read-after-free through a DIRECT INDEX GETTER (`op_arr_get` → `Handle::node_ref`, which BYPASSES
/// the `with_node` / `with_raw_arity` chokepoints) is now caught too. This is the access-site-coverage
/// win: the guard moved from the two chokepoints onto every direct node deref, so a freed container
/// consumed by a getter traps at the getter instead of reading poisoned/garbage bytes. (Before the
/// `node_ref` refactor this read would have dereffed the freed cell unguarded.)
#[test]
#[should_panic(expected = "use-after-free")]
fn read_after_free_through_a_direct_getter_is_caught_as_use_after_free() {
    let arr = op_arr_alloc(2);
    op_drop(arr); // last ref → frees the array node, poisoning the retained cell
    let _ = op_arr_get(arr, 0); // direct getter deref of a freed node → UAF panic via node_ref
}

/// Empirically settles the §5 TUPLE-PAYLOAD two-shell reclaim rc-model (v-core-opt's ss1 shape: a
/// `Cons` sum whose payload is a `(head, tail)` tuple box). Two scenarios pin exactly WHEN a single
/// cascading drop reclaims both boxes vs when an explicit second drop is required:
///  (A) BORROW-ONLY reads (`op_sum_payload` + `op_arr_get` both borrow): the tuple box stays
///      UNIQUELY Cons-owned, so one cascading `op_drop(cons)` frees BOTH the Cons shell AND the
///      tuple box, and a prior `op_dup(tail)` carries the tail forward — a net reclaim of 2/iter.
///  (B) the arm MATERIALIZES the tuple box (an extra `op_dup`, rc 2 — what binding the tuple to an
///      owned local does): now `op_drop(cons)` only decrements it to rc 1 (LEAK), so the emit MUST
///      also drop that extra ref. That explicit tuple-box drop is NOT a double-free — it matches
///      the materialization dup. This is the CORRECTED co-design answer for a net-no-op emit.
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

/// Extends the tuple-payload reclaim rc-model to a BOXED head (a bigint — a real node) to settle the
/// UAF gate question: is the head cascade-free unconditionally sound, or does the reclaim need a
/// head-escape gate? Proves it is UNCONDITIONALLY SOUND given correct rc placement:
///  (1) a boxed head UNIQUELY owned by the tuple box (rc 1) is cascade-freed by `drop(Cons)` — it is
///      dead (nothing else holds it), so no dangling ref.
///  (2) a boxed head that ESCAPED with a proper OWNED dup (rc 2) SURVIVES the cascade (2→1) — the
///      escapee's ref is intact, no premature free.
/// `op_drop` is rc-aware, so the head drop is safe in BOTH cases — no head-escape gate is needed,
/// PROVIDED every owned escape dups. A borrow-projection read (`op_sum_payload`/`op_arr_get`) never
/// transfers ownership, so it cannot silently leave an un-dup'd escapee at rc 1.
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

/// The empty-vec is a SHARED IMMORTAL SINGLETON (the `IMM_UNIT` analog for lists): `op_vec_empty`
/// returns the SAME node every call, it is census-EXCLUDED (never a leak — the mixed-recursive
/// List-fold terminal fix), and a `vec-push` on it takes the persistent COPY path (rc = IMMORTAL != 1)
/// so the singleton is NEVER mutated in place. This is the soundness control for the immortal-empty-vec
/// fix (no shared-singleton corruption).
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

/// The empty MAP / SET / BYTES / STRING constructors return shared IMMORTAL singletons — the empty-vec
/// / IMM_UNIT generalization (operator directive: an empty value should allocate once, immortal,
/// reused). Same handle every call, census-EXCLUDED (never a leak), and an insert/build path-copies
/// off them (rc=IMMORTAL != 1) leaving the singleton empty (no in-place mutation of the shared empty).
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

/// `op_vec_prepend` builds a correct multi-level RRB AND reclaims each intermediate version — the
/// dedicated front-growth op that replaces `concat(singleton, v)` (which leaked ~17 cells/prepend).
/// Mirrors the 05:2521 build loop (out = prepend(out, i)): the result is [n-1, …, 1, 0]. The
/// post-drop census == base is the leak witness — if intermediate versions leaked, dropping the final
/// list would leave them live (unreachable from `v`), so census > base.
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

/// `hash-blake3` (heap index 91) is BYTE-IDENTICAL to `blake3::hash` of the same input — for a flat
/// leaf, a ROPE (which must flatten first), and the empty input. This pins the RUNTIME half of the
/// design's §9 byte-identity invariant (DESIGN-compiler-primitives.md): the compile-time `Blake3.of`
/// fold calls the SAME `blake3::hash`, so op==crate here means both halves agree bit-for-bit. Also
/// verifies the op BORROWS its input (the caller can still drop the input afterwards, and every handle
/// returns to the live-node baseline — the op consumes nothing).
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

/// `ast-print` (heap op 92) renders a runtime Ast heap value to canonical re-readable s-expr text,
/// byte-identical to the compiler's `print_ast_value`. Builds Asts directly (`sum-new` at chosen discs
/// + payloads) and asserts the text; the disc→variant map is read from the baked `discs` Bytes (here
/// `[int,float,bool,str,name,bytes,list] = [0..=6]`). Covers the List recursion + Int/Name/Bool/Str.
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

/// `ast-encode` self-consistency: a heap `Ast` walked by `op_ast_encode` produces the SAME canonical
/// `cdzast` bytes as building the equivalent `Arenas` directly through the shared `Builder` +
/// `codec::encode`. This validates the heap-walk's disc dispatch + type bridges (Big→IntValue, char,
/// float→Decimal, RRB vec elements) — the byte-identity contract with the compile-time `Ast.encode` fold,
/// which runs that same `Builder`/`codec` path. Every input is built with the constructors the compiler
/// emits (bigint leaf, RRB `vec-push` for lists, boxed scalar for char) per the #3621 test-fidelity rule.
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

/// `ast-decode` round-trips `ast-encode`: `encode(decode(encode(v))) == encode(v)` over an Ast spanning
/// every variant. Encode is byte-canonical, so equal re-encoded bytes prove `decode` rebuilt the SAME
/// Ast (structure + every leaf value, through both type bridges). Also checks a malformed byte sequence
/// decodes to NULL (the `Err` path), never a trap.
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

/// M2 (OPTION B) `ast-encode`/`ast-decode` over the 7 first-class compound-ctor reflected forms
/// (ListCtor/TupleCtor/RecordCtor/MapCtor/SetCtor + FieldPair/Member, discs 9–15). Two contracts:
/// (1) the runtime op93 encode of a reflected ctor value is byte-identical to the shared cadenza-ast
/// `Builder`+`codec::encode` path — the compile-time `Ast.encode` fold's form, via the SAME
/// `compound`/`field_pair`/`member` emit primitives (`b.compound` heads with the ctor LEAF KIND, not a
/// Name/Str head); (2) it round-trips through op94 decode (`encode(decode(encode v)) == encode(v)`,
/// decoded ≠ NULL), proving `decode_arenas_to_ast`'s ctor arm is the exact inverse. BEFORE the encode
/// ctor arms landed, op93 returned EMPTY on any ctor disc (silent mis-encode + a decode/encode asymmetry
/// vs the compile-time fold) — this pins the symmetry closed.
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

/// Non-finite `Ast.Float` (NaN / ±inf) round-trips through op93 encode + op94 decode via the codec's
/// payload-less leaf tags (17/18/19), and the ENCODED bytes are byte-identical to the shared
/// Builder+codec of the matching `Leaf::FloatNan`/`FloatInf` (the compile-time `Ast.encode` fold's
/// form). Guards the compile/runtime agreement v-cp's encode-flip relies on.
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

/// The runtime contract behind the compiler's Perceus DUP-RETAIN fix (`spec@6c1120b2`): a heap value
/// used as a CONSUMING operand (here `String.concat`, which consumes its operand into the rope node)
/// AND with a LATER live use is emitted with a `dup` first, so the later use reads the intact original.
/// The prior test reads the shared leaf THROUGH another concat rope; this reads the ORIGINAL reference
/// DIRECTLY — the `(let ((e S)) (+ (len (String.concat e x)) (len e)))` shape the fix repairs (which
/// returned a wrong value when the dup was missing). After `dup(e)` + `concat(e, x)`, the original `e`
/// must read its full content + correct byte-len (the concat consumed a SEPARATE reference, not this
/// one), and everything reclaims exactly once.
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

/// The KEY-CANONICALIZATION contract that map/set keys rest on: `champ_eq`/`champ_hash` compare a
/// node's PHYSICAL bytes (fast — NO flatten on the hot key path), so a rope Bytes/String value and its
/// flat twin are NOT `champ_eq`-equal (different physical shape). The design keeps this correct by the
/// COMPILER `bytes-compact`ing a rope key BEFORE insert (champ-map-set-design §canonical-except-rope).
/// So the runtime's half of the contract is: **`bytes-compact` of a rope MUST yield a leaf that is
/// `champ_eq` AND `champ_hash`-identical to the flat twin of the same content** — else the compiler's
/// compact-before-insert wouldn't actually canonicalize the key and a rope-vs-flat key would still
/// mis-dedup. Pin BOTH halves: (1) a rope is champ_eq-DISTINCT from its flat twin (the physical-bytes
/// design — a rope key WITHOUT compaction would be a wrong key, the tripwire); (2) compact makes it
/// champ_eq + champ_hash-IDENTICAL to the flat twin (the runtime guarantee the compiler relies on).
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

/// The runtime half of the `String.at` content-equality fix (`spec@a2c75cc0` root-caused the
/// compiler-side miscompile). The contract test above uses a CONCAT rope; a `String.at` result is a
/// distinct rope shape — a `bytes-SLICE` (`raw = [off, len]`, arity 1 — the parent). `champ_eq`
/// physical-byte-compares that `[off,len]` header, so two slices of the same char at DIFFERENT offsets
/// (or into different parents) are champ_eq-DISTINCT despite equal content — EXACTLY the miscompile
/// (`String.at "banana" 1` ≠ `String.at "banana" 3` though both are "a", so `count-a` returns 0). The
/// compiler's fix compacts the `String.at` result before `=`; that fix RELIES on the runtime's
/// `bytes-compact` flattening a SLICE (not just a concat) to a champ_eq-canonical flat leaf. Pin that
/// runtime half here — the SLICE arm of `bytes_flatten` (read parent's `off+j`), not the concat arm.
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

/// EMPIRICAL classification of the adv54b root-cause split — does the fix need a RUNTIME half? adv54b
/// is `Bytes.concat (String.to-bytes tail) (String.to-bytes tail)` where `tail` is a slice-VIEW used
/// TWICE; the compiler under-dups the dual consuming use, so concat double-frees. This replays the op
/// sequence WITH the correct compiler dup simulated (`op_dup` of the dual-used view) and asserts the
/// census BALANCES + the value is correct. It PASSES, which proves `op_bytes_compact`/`bytes_flatten`
/// of a SHARED node is SOUND: unlike a value-CHANGING FBIP mutator (`vec-push`), flatten is a
/// value-PRESERVING canonicalization (rope→flat leaf, same logical bytes; it releases only the node's
/// OWN single child-ref), so it needs no `rc == 1` guard. => the adv54b fix is PURELY the compiler dup
/// (v-core-opt's half); there is NO runtime half. This test pins that the runtime stays sound once the
/// dup lands.
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

/// LANE-SPLIT PROBE (v-memory-safety/breaker List.at-over-relaxed-RRB read leak, corpus L2501): does
/// reading every index of a PREPEND-built (relaxed size-table) RRB via `op_vec_get` + the Some-shell
/// dance, WITHOUT threading the list (v borrowed, not dup'd), balance at the RUNTIME layer? The corpus
/// case (build+readsum n=1100) leaks 18972 — but build-only is 0 (op sound) and a SINGLE read is 0, so
/// the leak is the READ LOOP. This isolates the vec-get+Some READ path (runtime) from the compiled
/// recursive THREADING of the borrowed list (compiler). If this balances, the leak is the compiled
/// loop/threading reclaim (COMPILER); if it leaks, the relaxed-node read path itself leaks (RUNTIME).
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

/// LANE-SPLIT PROBE (v-memory-safety/breaker slice-view-as-key leak-2): does a SINGLE-use borrowed
/// slice-view, compacted then borrowed-compared then dropped, balance at the RUNTIME layer? The corpus
/// cases (19-sets view-as-CHAMP-key, value-eq-of-view) leak 2. If this exact runtime op sequence
/// balances, the leak is COMPILER emit reclaim (a missing/extra drop around the compacted operand); if
/// it leaks, the leak is RUNTIME (bytes_flatten of a single-owned view). Distinct from the dual-use
/// `compact_of_a_dual_used_shared_slice_view_is_balanced_with_the_dup` above (that needs the compiler dup).
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

/// CONTRACT-BOUNDARY TRIPWIRE for `value-eq` (op 61, the language `=`): it is `champ_eq` — a PHYSICAL-
/// byte compare, BY CONTRACT (fast, shared with the map-key path). So a ROPE Bytes/String value is
/// value-eq-DISTINCT from its flat twin even with equal CONTENT — the COMPILER must canonicalize
/// (`bytes-compact`) an operand before `value-eq`, exactly as for a map key. WARNING:WARNING: This documents a KNOWN
/// LATENT COMPILER MISCOMPILE (`spec@b4700bb9`): `ty_heap_walkable::Ty::String => true` admits a runtime
/// String to `value-eq` on a stale premise ("String.concat never makes a rope"), but runtime
/// `String.concat` now emits `bytes-concat` (a rope), so `(= (String.concat rt_a rt_b) other)` compares
/// rope-vs-flat and returns the WRONG answer. NOT a runtime bug — the runtime is physical-bytes by
/// design; the compiler owes the compact. When the compiler fix lands (compact-before-value-eq, OR
/// re-decline `Ty::String`), a `(String.concat rt rt)` `=` becomes correct and a rope-operand value-eq
/// no longer reaches the runtime uncompacted — this test pins the runtime side stays physical-bytes
/// (a flatten inside `champ_eq` would slow the hot key path); flip/extend it only with that design call.
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

// ── CHAMP node core: bitmaps, slots, discrimination, structural hash + eq ─────────────

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

// Build a small normal CHAMP node owning two int leaves as one k/v entry (datamap bit 0).
fn champ_kv_node(k: i64, v: i64) -> Handle {
    alloc_raw(vec![op_box_int(k), op_box_int(v)], champ_header(0b1, 0, 1))
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

/// An INDEPENDENT, naive RECURSIVE reference for the structural hash: FNV-1a over a node's own
/// canonical raw bytes, then over each child's reference hash (LE). Deliberately written
/// differently from the production iterative walk — no worklist, no leaf fast path — so it is a
/// genuine oracle for `champ_hash`, not a copy of it. Children are folded in REVERSE index order
/// because the production walk pushes children onto its worklist in order and pops them LIFO, so
/// `results` presents them last-child-first; reproducing that here makes this a faithful oracle
/// (the exact byte discipline the fast path and refactor must not disturb, not a re-invented one).
fn champ_hash_ref(h: Handle) -> u32 {
    let (raw, arity) = node_raw_arity(h);
    let mut acc = FNV_OFFSET;
    for &b in &raw {
        acc = fnv_step(acc, b);
    }
    if !is_immediate(h) {
        with_node(h, (), |n| {
            for i in (0..arity).rev() {
                let ch = champ_hash_ref(n.handles[i]);
                for b in ch.to_le_bytes() {
                    acc = fnv_step(acc, b);
                }
            }
        });
    }
    acc
}

/// The allocation-free arity-0 fast path in `champ_hash` must be BYTE-IDENTICAL to the general
/// worklist walk (a hash drift would silently corrupt map/set placement and cross-version stability).
/// Assert equality against the independent recursive oracle across the leaf cases the fast path
/// covers — immediates (inline unit/bool/int), boxed out-of-window ints, floats, strings, empty
/// bytes — AND across compounds (arrays, sums, a real CHAMP node, deep nesting) that take the
/// general walk, so the shared `champ_node_raw_hash` fold is pinned on both branches at once.
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

/// TAGLESS invariant (the spec's determinism "no-type-tag" principle, duvet-annotated `@b470dd82`):
/// the runtime stores only STRUCTURE + DATA, never a value's TYPE, so `champ_eq`/`champ_hash`/
/// `champ_key_cmp` compare RAW BYTES + arity — they physically CANNOT distinguish two values of
/// DIFFERENT types that happen to share the same raw bytes and (zero) arity. A boxed Int and a Bytes
/// leaf holding the Int's little-endian bytes are therefore champ_eq + hash-equal + cmp-Equal. This is
/// not a bug — it is WHY keeping a map/set's keys HOMOGENEOUS is the COMPILER's obligation (the runtime
/// can't enforce it), and WHY the byte-hash is storage-transparent. Pinning it guards against anyone
/// accidentally adding a type discriminator to the comparison path (which would break byte-hash
/// transparency + the map/set key contract for a compiler that relies on this).
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

/// Guard the alloc-free `with_raw_arity` fast path in `champ_eq`/`champ_key_cmp` against the naive
/// `node_raw_arity` (Vec-cloning) reference it replaced, across every shape whose comparison can
/// touch the immediate branch: inline unit/bool/int, a hand-BOXED int twin, an out-of-window boxed
/// int, floats (incl -0.0), empty/nonempty strings, empty bytes, and NULL. For each pair where at
/// least one side is IMMEDIATE — the only path my edit touched — the new `champ_eq` must equal
/// `rx==ry && ax==ay` over the old `node_raw_arity`, and `champ_key_cmp` must equal
/// `rx.cmp(&ry).then(ax.cmp(&ay))`. Since every operand here is arity-0, that single-node compare
/// IS the whole verdict, so the reference is exact. (Pairs where NEITHER side is immediate — e.g.
/// a real leaf vs NULL — go through `champ_eq`'s UNCHANGED non-immediate arm, which distinguishes
/// NULL from a non-null leaf; the `node_raw_arity` model folds both to `([],0)` and so does NOT
/// model that arm, hence they're excluded.) Catches any drift in the ≤8-byte materialization/borrow.
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

// ── CHAMP persistent MAP: empty / lookup / insert / size ──────────────────────────────

/// Look up integer `k` in `m` (borrows), returning its i64 value if present. Builds and drops a
/// fresh probe key; never retains the borrowed value handle.
fn mlookup_int(m: Handle, k: i64) -> Option<i64> {
    let probe = op_box_int(k);
    let v = op_map_lookup(m, probe);
    op_drop(probe);
    if v == Handle::NULL {
        None
    } else {
        Some(op_get_int(v))
    }
}

/// Insert `k => v` (both boxed ints) into `m`, consuming `m`.
fn minsert_int(m: Handle, k: i64, v: i64) -> Handle {
    op_map_insert(m, op_box_int(k), op_box_int(v))
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

/// The size header must stay EXACTLY correct as inserts descend deep spines and split — this is the
/// job of the `(handle, delta)` the insert core now RETURNS (0 overwrite / 1 new key) instead of
/// recomputing via two `champ_size_of` subtree reads. Interleaves new-key inserts (delta 1, must
/// bump size at EVERY ancestor level) with overwrites (delta 0, must bump NOTHING), on BOTH the
/// unique-FBIP path and the shared path-copy path, then verifies size + a full membership sweep. A
/// wrong propagated delta would desync the size header from the true count at some interior node.
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

/// A DEEP CHAMP trie. The map/set FUZZERS use a u8 keyspace (≤256 keys) which only builds a trie of
/// DEPTH 1 (measured); even 4096 sequential int keys reach only depth 2. But a 32-bit hash supports up
/// to 7 levels (`level_index` shifts 5 bits/level), and the multi-level insert/lookup/remove DESCENT
/// through interior subnodes at levels ≥3 — the `champ_insert_node`/`champ_remove_node` recursion +
/// the subnode-index bookkeeping (`subnode_index_for_slot`, `data_count`) at deep levels — was
/// UNEXERCISED. This forces a DEPTH-3 trie with 6 keys that all share the low 15 bits of their
/// `champ_hash` (prefix `0x1a4f`), so levels 0/1/2 each descend a subnode. A PRECONDITION guard
/// asserts the shared prefix still holds — if the frozen FNV hash ever changes, the test fails LOUDLY
/// (re-find keys) rather than silently degrading to a shallow trie. Verifies deep lookup (each key its
/// own value), size, remove-at-depth (one key gone, the rest intact, the deep spine collapses
/// correctly), and no leak.
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

/// A STRING-KEY collision node — the string sibling of `map_forces_collision_node` (which uses INT
/// keys). The compiler-in-Cadenza port's maps are STRING-keyed, and a string key takes the arity-0
/// HEAP-BYTE-LEAF champ path: the collision node's linear scan compares keys by `champ_eq` = RAW-BYTE
/// content (not the int-immediate compare the int-collision test exercises). Two identifier-like
/// strings that happen to share a full 32-bit FNV hash must still be kept DISTINCT and each resolve to
/// its OWN value BY CONTENT — and removing one collision entry must leave the other intact (the
/// collision-node drain path). Uses a hardcoded pair found by a birthday search over `k{n}` strings;
/// a PRECONDITION guard asserts they still collide, so if the frozen FNV hash ever changes this test
/// fails LOUDLY (re-find a colliding pair) rather than silently degrading to a non-collision case.
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

// ── CHAMP persistent MAP: remove (inverse of insert; canonicality) ────────────────────

/// Remove integer `k` from `m`, consuming `m`. Builds and drops a fresh probe key (remove
/// borrows the key).
fn mremove_int(m: Handle, k: i64) -> Handle {
    let probe = op_box_int(k);
    let out = op_map_remove(m, probe);
    op_drop(probe);
    out
}

/// Two low-5-bit-colliding-but-distinct ints (forces a subnode split at level 0), reusing the
/// U2 search. Returns `(a, b)`; both fresh probes are dropped.
fn low5_split_pair() -> (i64, i64) {
    let mut by_low: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
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
                return (v0, v);
            }
        } else {
            by_low.insert(low, v);
        }
        v += 1;
    }
    panic!("no low-5 split pair found");
}

/// Two DISTINCT full-width payloads whose 32-bit champ_hash is fully equal (forces a collision
/// node), reusing the U2 birthday search over splitmix-spread payloads.
fn full_hash_collision_pair() -> (i64, i64) {
    let mix = |c: u64| -> i64 {
        let mut z = c.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) as i64
    };
    let mut seen: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
    let mut c = 0u64;
    while c < 3_000_000 {
        let payload = mix(c);
        let k = op_box_int(payload);
        let h = champ_hash(k);
        op_drop(k);
        match seen.get(&h) {
            Some(&p0) if p0 != payload => return (p0, payload),
            Some(_) => {}
            None => {
                seen.insert(h, payload);
            }
        }
        c += 1;
    }
    panic!("no full 32-bit collision found");
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

// ── CHAMP cursor + in-order map iteration ─────────────────────────────────────────────

/// Walk `m` (borrows) collecting (key,val) as i64 pairs in visitation order. Consumes the
/// cursors it builds (iter + iter_next chain), leaving `m`'s rc untouched.
fn collect_map(m: Handle) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut cur = op_map_iter(m);
    loop {
        let k = op_map_iter_key(cur);
        if k == Handle::NULL {
            break;
        }
        let v = op_map_iter_val(cur);
        out.push((op_get_int(k), op_get_int(v)));
        cur = op_map_iter_next(cur);
    }
    op_drop(cur);
    out
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

/// `map_iter_order_is_deterministic` (above) proves insert-order-independent cursor iteration for INT
/// keys (immediate). STRING keys take a DIFFERENT champ path — an arity-0 heap-byte leaf whose slot is
/// chosen by `champ_hash`'s raw-byte FNV, not an int's little-endian bytes — so their CHAMP placement,
/// and thus the cursor's descent order, is a distinct code path. The self-hosting compiler's
/// symbol-table maps are STRING-keyed and it will iterate them (e.g. to emit definitions in a stable
/// order once `Map.fold`/`keys` are exposed — the runtime cursor is already shipped), so a string-key
/// cursor-order bug would make a compiler built on top produce non-deterministic output. Pin that a
/// string-keyed map iterates in the SAME order regardless of insert order (the order is CHAMP hash
/// order — NOT lexicographic; value-encode separately re-sorts to canonical render order).
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

/// TRIPWIRE + COORDINATION CONTRACT for the compiler agent wiring `Map.fold`/`Map.keys`/`to-list`.
/// The runtime cursor (`map-iter`/`-next`/`-key`) walks the CHAMP in HASH order — deterministic and
/// insert-order-independent (pinned above), which satisfies HALF of the spec's *Map Iteration Is
/// Deterministic*: "a deterministic order derived from the keys, not from insertion order." BUT the
/// spec has a SECOND clause: "The order in which a map's entries are visited MUST AGREE with the order
/// its canonical byte form places them in" (collections-and-text.md §Map Iteration Is Deterministic,
/// cited at rcdzc lower.rs ~8207). The canonical byte form orders keys by `value_cmp_shaped`
/// (NUMERIC for ints, lexicographic for strings, lexicographic for orderable compounds — what
/// value-encode renders + what `Map.fold` output must match). HASH order ≠ canonical order, so a
/// `Map.fold`/`keys` emitted directly over the raw
/// cursor would VIOLATE the spec AND disagree with the same map's own `print`/value-encode output.
/// This test PINS that the two orders differ (so the discrepancy can't be forgotten) and documents the
/// contract: when iteration is exposed to the language, the compiler MUST re-sort the cursor output
/// through the canonical order (the runtime already does this internally in `map_entries_canonical`;
/// the alternative is a future canonical-order runtime cursor op — a coordination decision, NOT a
/// silent hash-order fold). If a future change makes the cursor ITSELF canonical-ordered, this test
/// will flip — update it (and the compiler emit) together; do not just delete the assertion.
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

/// TRIPWIRE (STRING-KEY sibling of the int-key contract above) — the case the compiler-in-Cadenza port
/// ACTUALLY hits. Its symbol tables / free-var environments are STRING-keyed (`Map String Ast`,
/// `Set String`), and the compiler-ml agent's enumeration-gap finding (`spec@d1724597`) reasoned "the
/// runtime walks a Map/Set in canonical order already" — which is FALSE: a STRING key takes the arity-0
/// heap-byte-leaf champ path (slot from a raw-byte FNV hash), so the cursor's order is HASH order, NOT
/// the LEXICOGRAPHIC (canonical byte-form) order the spec's *Map Iteration Is Deterministic* second
/// clause requires. The int-key tripwire above proves the divergence for numeric-vs-LE-byte order; this
/// proves it for the string keyspace the port will feed a future `Map.to-list`/`keys`/`fold` — so a
/// front-end built directly on the raw cursor would emit the self-hosting compiler's bindings in HASH
/// order (spec-violating, and disagreeing with the map's own `print`). The FIX when iteration is exposed
/// is the same as the int case: re-sort the cursor output through the canonical (here lexicographic)
/// order — the runtime already does this internally in `map_entries_canonical`'s `Shape::Str` arm, or a
/// coordinated canonical-order cursor op could expose it. Realistic identifier-like keys (a lexer/parser
/// token table) make the two orders concretely differ.
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

// ── CHAMP persistent SET (stride 1) ───────────────────────────────────────────────────

/// Insert boxed int `e` into `s`, consuming `s`.
fn sinsert_int(s: Handle, e: i64) -> Handle {
    op_set_insert(s, op_box_int(e))
}
/// Membership of boxed int `e` in `s` (borrows). Builds+drops a fresh probe.
fn scontains_int(s: Handle, e: i64) -> bool {
    let probe = op_box_int(e);
    let r = op_set_contains(s, probe);
    op_drop(probe);
    r
}
/// Remove boxed int `e` from `s`, consuming `s`.
fn sremove_int(s: Handle, e: i64) -> Handle {
    let probe = op_box_int(e);
    let out = op_set_remove(s, probe);
    op_drop(probe);
    out
}
/// Walk `s` (borrows) collecting elements as i64 in visitation order.
fn collect_set(s: Handle) -> Vec<i64> {
    let mut out = Vec::new();
    let mut cur = op_set_iter(s);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break;
        }
        out.push(op_get_int(e));
        cur = op_set_iter_next(cur);
    }
    op_drop(cur);
    out
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

// ── Collision-node canonicality across insert order (regression) ──────────────────────

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

// ── U5: FBIP rc==1 in-place shell reuse for CHAMP map/set insert+remove ─────────────────────
// The load-bearing property is ALIASING SAFETY: an insert/remove on a SHARED map/set (rc>1) must
// path-copy and leave the other version byte-identical (champ_eq + champ_hash); the FBIP win fires
// only when the touched spine is uniquely owned. Canonical shape (collision order, collapse,
// remove-to-canonical-empty) must survive the in-place path.

/// Build a multi-level map that includes a subnode SPLIT and a COLLISION pair — the richest shape,
/// used to exercise every FBIP branch. Returns `(m, split_a, split_b, coll_a, coll_b)`.
fn rich_map() -> (Handle, i64, i64, i64, i64) {
    let (sa, sb) = low5_split_pair(); // forces a subnode at the root
    let (ca, cb) = full_hash_collision_pair(); // forces a collision node at the hash floor
    let mut m = op_map_empty();
    for &(k, v) in &[(sa, 1), (sb, 2), (ca, 3), (cb, 4), (7i64, 70), (9, 90)] {
        m = minsert_int(m, k, v);
    }
    (m, sa, sb, ca, cb)
}

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

/// CO-VERIFY (v-core-opt TWO-SUM reclaim arc, 19-sets:546): the RUNTIME does NOT scale-leak a Set
/// accumulator across a `contains`-then-`insert` loop, and a completed `Set.contains` BORROW does NOT
/// block the next `set-insert`'s in-place FBIP reuse. `set_insert_h` gates reuse purely on
/// `node_rc(s) == 1` at the call, and `op_set_contains` BORROWS (no dup/retain) — so a contains that
/// COMPLETES before the insert leaves the accumulator at rc==1, and the insert refits in place with no
/// orphaned prior version. This isolates the observed TWO-SUM scaling leak to the COMPILER emit (a live
/// dup of `seen` held ACROSS the insert — the contains-borrow dup or the loop-param preservation dup —
/// which raises rc to ≥2 at the insert and forces the copy path). The runtime is liveness-precise; no
/// runtime change is warranted. If this ever regresses (final drop doesn't reclaim), the runtime WOULD
/// be the culprit.
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

/// CO-VERIFY (v-core-opt #5352, if-join-shared-child family fix: 712 SET / MAP / LIST-05-compound /
/// ROPE-980). v-mem ruled the fix = SKIP the spurious cross-arm dup of a base collection shared across
/// an `if`'s two arms. The LOAD-BEARING runtime fact (which makes dup-skip-ALONE safe — no double-free —
/// rather than needing a drop-elide) is: with the dup removed the base is rc==1, so the `else`-arm
/// derivative builder (vec-push / set-insert / map-insert) FBIP-CONSUMES it IN PLACE — the base's node
/// is SUBSUMED into the result, so the result IS the base and its ONE post-if drop reclaims everything.
/// There is no separate live base to double-free. This models that `else`-arm at the primitive level per
/// collection: rc1 base → builder consumes-in-place (result stays rc==1) → single drop → balanced. A
/// borrow-and-share builder would instead leave the base live after the drop (leak); a non-subsuming
/// consume would underflow (double-free). Balance-to-baseline after ONE drop pins neither happens.
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

/// A rope `bytes-concat` that CONSUMES a uniquely-owned (rc==1) base into the result, with the result
/// dropped once, balances (base survives as the new concat node's child; the single drop reclaims it).
/// A valid consume + single-drop reclaim.
///
/// WARNING: SCOPE CORRECTION (was mis-framed as an if-join-shared co-verify): this does NOT model the
/// if-join-shared "concat-child-of-keep" shape, which is the UNSAFE one. `bytes-concat` does NOT refit
/// the base in place — it ALLOCATES a NEW node with base as a CHILD — so unlike vec-push/set-insert/
/// map-insert (which SUBSUME the base in place, making the base IS-the-result identity that lets the
/// #5352 dup-skip-alone predicate hold), a rope concat leaves base as a distinct consumed child. On a
/// fresh cdz, the rope if-join (980) mode2 DOUBLE-FREES under dup-skip-alone, so v-core-opt's predicate
/// narrows `escapes_into_if_result` to the IN-PLACE-REUSE builders and EXCLUDES concat (the dup must be
/// RETAINED for concat). This test only pins the plain consume-balance, NOT that if-join concat is safe.
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

// ── Property tests (bolero) — a generated-input generalization of the hand-rolled fuzz oracles ──
// These drive RANDOM operation sequences (bolero shrinks failures to a minimal counterexample) at
// the crown-jewel refcount/FBIP paths, checking three invariants against a std reference on EVERY
// generated sequence: (1) value equivalence (lookup/contains match the oracle over the whole
// keyspace); (2) canonical-shape — two maps with equal contents are champ_eq + champ_hash-equal
// regardless of build order (byte-canonicality, the property the whole tagless design rests on);
// (3) no leak / no double-free (live_nodes() returns to baseline). They run in the normal suite as
// property tests AND under `cargo xtask miri` — so a memory bug the inline-`handles` change (task
// #22) might introduce is caught by Miri on a RANDOM adversarial sequence, not just fixed cases.
//
// A `Model` mirrors the runtime map with a BTreeMap while maintaining a STACK of live forked
// versions (each a (Handle, BTreeMap) pair) so the sequence exercises rc>1 shared-version paths —
// the exact aliasing surface where a path-copy vs in-place-reuse bug would corrupt a sibling.

#[derive(Debug, bolero::TypeGenerator)]
enum MapOp {
    Insert { key: u8, val: u8 },
    Remove { key: u8 },
    Fork,       // dup the current version + push it onto the live stack
    DropForked, // drop + pop the most recent forked version (no-op if none)
}

/// Build a reference-equal fresh map from a BTreeMap oracle (for the canonical-shape cross-check).
fn map_of_reference(reference: &std::collections::BTreeMap<i64, i64>) -> Handle {
    let mut m = op_map_empty();
    for (&k, &v) in reference {
        m = minsert_int(m, k, v);
    }
    m
}

fn run_map_op_sequence(ops: &[MapOp]) {
    let before = live_nodes();
    let mut m = op_map_empty();
    let mut reference: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    // Live forked versions: each keeps its own reference snapshot to verify it stays UNDISTURBED.
    let mut forks: Vec<(Handle, std::collections::BTreeMap<i64, i64>)> = Vec::new();
    for op in ops {
        match *op {
            MapOp::Insert { key, val } => {
                let (k, v) = (key as i64, val as i64);
                m = minsert_int(m, k, v);
                reference.insert(k, v);
            }
            MapOp::Remove { key } => {
                let k = key as i64;
                m = mremove_int(m, k);
                reference.remove(&k);
            }
            MapOp::Fork => {
                op_dup(m); // now rc>1: the next mutation of `m` must path-copy, leaving this snapshot intact
                forks.push((m, reference.clone()));
            }
            MapOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
    }
    // (1) value equivalence over the whole u8 keyspace (probes present + absent keys).
    assert_eq!(
        op_map_size(m) as usize,
        reference.len(),
        "size matches reference"
    );
    for k in 0..=255i64 {
        assert_eq!(
            mlookup_int(m, k),
            reference.get(&k).copied(),
            "key {k} matches reference"
        );
    }
    // (1b) CURSOR completeness: walk `op_map_iter` to exhaustion and collect every (key,value) it
    // visits. The cursor walks CHAMP HASH order (NOT the reference's sorted order), so compare as a
    // MAP/set — it must visit EXACTLY the reference's entries, each once. This is the property a future
    // `Map.fold`/`keys` rests on, over a CHAMP shaped by the RANDOM insert/remove/fork churn above
    // (collapsed collision nodes, sparse bitmaps, in-place-drained-then-refilled subtrees) — states the
    // fixed-shape cursor tests don't reach. The cursor BORROWS each key/value (no consume), so the
    // collected handles are only read (`op_get_int`), never dropped; the cursor itself is dropped when
    // exhausted. A duplicate/missing/extra key or a wrong value would diverge from the reference here.
    let mut visited: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    let mut cur = op_map_iter(m);
    loop {
        let k = op_map_iter_key(cur);
        if k == Handle::NULL {
            break; // exhausted
        }
        let v = op_map_iter_val(cur);
        let prev = visited.insert(op_get_int(k), op_get_int(v));
        assert!(
            prev.is_none(),
            "the cursor visits key {} at most once (no duplicate emission)",
            op_get_int(k)
        );
        cur = op_map_iter_next(cur);
    }
    op_drop(cur);
    assert_eq!(
        visited, reference,
        "the cursor visits EXACTLY the reference's (key,value) entries — complete + correct enumeration \
         over a churned CHAMP (the Map.fold/keys property; hash order, compared as a map)"
    );
    // (2) canonical shape: same contents ⇒ byte-identical to a freshly-built twin, regardless of the
    // insert/remove/fork history that produced `m`.
    let twin = map_of_reference(&reference);
    assert!(
        champ_eq(m, twin),
        "map equals a fresh twin of the same contents (canonical)"
    );
    assert_eq!(champ_hash(m), champ_hash(twin), "…and hashes identically");
    op_drop(twin);
    // Every forked snapshot must be UNDISTURBED by the later mutations of `m` (aliasing safety).
    for (h, snap) in &forks {
        assert_eq!(
            op_map_size(*h) as usize,
            snap.len(),
            "forked snapshot size intact"
        );
        for (&k, &v) in snap {
            assert_eq!(
                mlookup_int(*h, k),
                Some(v),
                "forked snapshot key {k} intact"
            );
        }
    }
    // (3) no leak / no double-free: release everything, live count returns to baseline.
    op_drop(m);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the whole sequence"
    );
}

#[test]
fn prop_map_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<MapOp>>()
        .for_each(|ops| run_map_op_sequence(ops));
}

#[derive(Debug, bolero::TypeGenerator)]
enum SetOp {
    Insert { elem: u8 },
    Remove { elem: u8 },
    Fork,
    DropForked,
}

fn set_of_reference(reference: &std::collections::BTreeSet<i64>) -> Handle {
    let mut s = op_set_empty();
    for &e in reference {
        s = sinsert_int(s, e);
    }
    s
}

fn run_set_op_sequence(ops: &[SetOp]) {
    let before = live_nodes();
    let mut s = op_set_empty();
    let mut reference: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    let mut forks: Vec<(Handle, std::collections::BTreeSet<i64>)> = Vec::new();
    for op in ops {
        match *op {
            SetOp::Insert { elem } => {
                let e = elem as i64;
                s = sinsert_int(s, e);
                reference.insert(e);
            }
            SetOp::Remove { elem } => {
                let e = elem as i64;
                s = sremove_int(s, e);
                reference.remove(&e);
            }
            SetOp::Fork => {
                op_dup(s);
                forks.push((s, reference.clone()));
            }
            SetOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
    }
    assert_eq!(
        op_set_size(s) as usize,
        reference.len(),
        "set size matches reference"
    );
    for e in 0..=255i64 {
        assert_eq!(
            scontains_int(s, e),
            reference.contains(&e),
            "membership of {e} matches"
        );
    }
    // CURSOR completeness: walk `op_set_iter` to exhaustion and collect every element it visits. The
    // cursor walks CHAMP HASH order (NOT sorted), so compare as a SET — it must visit EXACTLY the
    // reference's elements, each once. This is the property a future `Set.to-list`/`fold` rests on,
    // over a CHAMP shaped by the random insert/remove/fork churn above (collapsed collision nodes,
    // sparse bitmaps, in-place-drained subtrees) — states fixed-shape cursor tests don't reach. The
    // set fuzzer previously verified membership only via `scontains` point probes, never the cursor
    // (the same gap the map fuzzer had before `spec@1cdf6fb7`). The cursor BORROWS each element (read
    // via `op_get_int`, never dropped); it is dropped when exhausted; the `live_nodes()==before`
    // balance below still holds. A missing/extra/duplicate element diverges from the reference here.
    let mut visited: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    let mut cur = op_set_iter(s);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break; // exhausted
        }
        let fresh = visited.insert(op_get_int(e));
        assert!(
            fresh,
            "the set cursor visits element {} at most once (no duplicate emission)",
            op_get_int(e)
        );
        cur = op_set_iter_next(cur);
    }
    op_drop(cur);
    assert_eq!(
        visited, reference,
        "the set cursor visits EXACTLY the reference's elements — complete + correct enumeration over \
         a churned CHAMP (the Set.to-list/fold property; hash order, compared as a set)"
    );
    let twin = set_of_reference(&reference);
    assert!(
        champ_eq(s, twin),
        "set equals a fresh twin of the same contents (canonical)"
    );
    assert_eq!(champ_hash(s), champ_hash(twin), "…and hashes identically");
    op_drop(twin);
    for (h, snap) in &forks {
        assert_eq!(
            op_set_size(*h) as usize,
            snap.len(),
            "forked set snapshot size intact"
        );
        for &e in snap {
            assert!(scontains_int(*h, e), "forked set snapshot elem {e} intact");
        }
    }
    op_drop(s);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the set sequence"
    );
}

#[test]
fn prop_set_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<SetOp>>()
        .for_each(|ops| run_set_op_sequence(ops));
}

// STRING-ELEMENT variant — a `Set String` (a deduplicated string collection: keyword sets, a
// visited-name set, a compiler's interned-symbol set). `run_set_op_sequence` uses only INT elements
// (immediate), so the arity-0 HEAP-BYTE-LEAF champ path — arity-0 raw-byte FNV `champ_hash` + a
// slot-hit raw-byte `champ_eq` — is unexercised for a SET. This is a DISTINCT path from both the
// int-set fuzzer (immediate elements) AND the string-MAP-key fuzzer (heap-byte leaf but STRIDE 2, a
// key paired with a value): a set is STRIDE 1, so its data-node layout, collision handling, and
// canonical dedup differ. Reuses `strkey_name`'s 8 flat names; keys built FLAT (`op_str_new`) — a
// rope element is the compiler's to `bytes-compact` before insert (champ_eq is physical-bytes by
// contract), out of scope. Same four properties the int-set fuzz checks.
#[derive(Debug, bolero::TypeGenerator)]
enum StrSetOp {
    Insert { elem: u8 },
    Remove { elem: u8 },
    Fork,
    DropForked,
}

fn run_strset_op_sequence(ops: &[StrSetOp]) {
    let before = live_nodes();
    let mut s = op_set_empty();
    let mut reference: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut forks: Vec<(Handle, std::collections::BTreeSet<String>)> = Vec::new();
    for op in ops {
        match *op {
            StrSetOp::Insert { elem } => {
                let name = strkey_name(elem);
                s = op_set_insert(s, op_str_new(name.clone())); // consumes the element
                reference.insert(name);
            }
            StrSetOp::Remove { elem } => {
                let name = strkey_name(elem);
                let probe = op_str_new(name.clone());
                s = op_set_remove(s, probe); // BORROWS the element
                op_drop(probe); // we own the probe
                reference.remove(&name);
            }
            StrSetOp::Fork => {
                op_dup(s); // rc>1: the next mutation path-copies, leaving this snapshot intact
                forks.push((s, reference.clone()));
            }
            StrSetOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
    }
    // (1) size + membership over the whole small keyspace (present + absent).
    assert_eq!(
        op_set_size(s) as usize,
        reference.len(),
        "string-set size matches reference"
    );
    for k in 0..8u8 {
        let name = strkey_name(k);
        let probe = op_str_new(name.clone());
        let got = op_set_contains(s, probe); // borrows
        op_drop(probe);
        assert_eq!(
            got,
            reference.contains(&name),
            "string-set membership of {name:?} matches reference"
        );
    }
    // (2) canonical shape: same contents ⇒ byte-identical to a fresh twin (what set dedup rests on).
    let twin = {
        let mut t = op_set_empty();
        for name in &reference {
            t = op_set_insert(t, op_str_new(name.clone()));
        }
        t
    };
    assert!(
        champ_eq(s, twin),
        "string-set equals a fresh twin of the same contents (canonical)"
    );
    assert_eq!(champ_hash(s), champ_hash(twin), "…and hashes identically");
    op_drop(twin);
    // (3) forked snapshots undisturbed by later mutation of `s` (aliasing safety on the string path).
    for (h, snap) in &forks {
        assert_eq!(
            op_set_size(*h) as usize,
            snap.len(),
            "forked string-set snapshot size intact"
        );
        for name in snap {
            let probe = op_str_new(name.clone());
            let got = op_set_contains(*h, probe);
            op_drop(probe);
            assert!(got, "forked string-set snapshot elem {name:?} intact");
        }
    }
    // (4) no leak / no double-free across the whole sequence.
    op_drop(s);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the whole string-set sequence"
    );
}

#[test]
fn prop_strset_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<StrSetOp>>()
        .for_each(|ops| run_strset_op_sequence(ops));
}

// COMPOUND-KEY variant — 2-tuple keys are the ≤2-handle nodes the inline-`handles` change (#22)
// targets, so this is the MOST load-bearing shape for de-risking it: every insert/lookup builds a
// tuple node (node + 2-elem handles), hashes it via the shallow-compound path, and champ_eq-compares
// it on a slot hit. A refcount bug in the tuple key's handles would surface here under Miri.
#[derive(Debug, bolero::TypeGenerator)]
enum TupleKeyOp {
    Insert { a: u8, b: u8, val: u8 },
    Remove { a: u8, b: u8 },
    Fork,
    DropForked,
}

fn ctuple_key(a: i64, b: i64) -> Handle {
    let t = op_arr_alloc(2);
    op_arr_set(t, 0, op_box_int(a));
    op_arr_set(t, 1, op_box_int(b));
    t
}

fn run_tuplekey_op_sequence(ops: &[TupleKeyOp]) {
    let before = live_nodes();
    let mut m = op_map_empty();
    let mut reference: std::collections::BTreeMap<(i64, i64), i64> =
        std::collections::BTreeMap::new();
    let mut forks: Vec<(Handle, std::collections::BTreeMap<(i64, i64), i64>)> = Vec::new();
    // Small tuple keyspace (a,b ∈ 0..8) → real overwrites, splits, and shared-prefix hashing.
    for op in ops {
        match *op {
            TupleKeyOp::Insert { a, b, val } => {
                let (a, b, v) = ((a % 8) as i64, (b % 8) as i64, val as i64);
                m = op_map_insert(m, ctuple_key(a, b), op_box_int(v));
                reference.insert((a, b), v);
            }
            TupleKeyOp::Remove { a, b } => {
                let (a, b) = ((a % 8) as i64, (b % 8) as i64);
                let probe = ctuple_key(a, b);
                m = op_map_remove(m, probe);
                op_drop(probe); // remove BORROWS the key — we own the probe, drop it
                reference.remove(&(a, b));
            }
            TupleKeyOp::Fork => {
                op_dup(m);
                forks.push((m, reference.clone()));
            }
            TupleKeyOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
    }
    assert_eq!(
        op_map_size(m) as usize,
        reference.len(),
        "tuple-key map size matches reference"
    );
    for a in 0..8i64 {
        for b in 0..8i64 {
            let probe = ctuple_key(a, b);
            let got = op_map_lookup(m, probe);
            op_drop(probe);
            let want = reference.get(&(a, b)).copied();
            assert_eq!(
                if got == Handle::NULL {
                    None
                } else {
                    Some(op_get_int(got))
                },
                want,
                "tuple key ({a},{b}) matches reference"
            );
        }
    }
    op_drop(m);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak across the tuple-key sequence"
    );
}

#[test]
fn prop_tuplekey_map_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<TupleKeyOp>>()
        .for_each(|ops| run_tuplekey_op_sequence(ops));
}

// STRING-KEY variant — the JSON-object / symbol-table shape (a self-hosting compiler's environment
// maps are string-keyed). A string key is a DISTINCT champ path from int (immediate) and tuple
// (shallow-compound) keys: an arity-0 HEAP-BYTE leaf, so `champ_hash` takes the arity-0 raw-byte FNV
// fast path and a slot-hit `champ_eq` compares raw bytes — neither exercised through insert/remove/
// fork/overwrite/canonical-twin by the int or tuple fuzzers. Keys are built FLAT (`op_str_new`), the
// same leaf the compiler emits for a String key (a rope key is the compiler's to `bytes-compact`
// before insert — champ_eq is physical-bytes by contract, so a raw rope would mis-dedup; not this
// test's concern). A small keyspace (8 short names) forces real overwrites, removes, and node splits.
#[derive(Debug, bolero::TypeGenerator)]
enum StrKeyOp {
    Insert { key: u8, val: u8 },
    Remove { key: u8 },
    Fork,
    DropForked,
}

// The 8 fixed string keys the small keyspace draws from (varied lengths, some sharing a leading byte
// to drive shared-prefix hash slots; all flat leaves).
fn strkey_name(k: u8) -> String {
    match k % 8 {
        0 => "a".to_string(),
        1 => "bb".to_string(),
        2 => "ccc".to_string(),
        3 => "key".to_string(),
        4 => "keyword".to_string(), // shares "key" prefix with #3 (distinct hashes, exercises slots)
        5 => "".to_string(),        // the empty string is a valid key (zero-length byte leaf)
        6 => "a-longer-identifier".to_string(),
        _ => "z".to_string(),
    }
}

fn run_strkey_op_sequence(ops: &[StrKeyOp]) {
    let before = live_nodes();
    let mut m = op_map_empty();
    let mut reference: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut forks: Vec<(Handle, std::collections::BTreeMap<String, i64>)> = Vec::new();
    for op in ops {
        match *op {
            StrKeyOp::Insert { key, val } => {
                let name = strkey_name(key);
                let v = val as i64;
                m = op_map_insert(m, op_str_new(name.clone()), op_box_int(v)); // consumes key+val
                reference.insert(name, v);
            }
            StrKeyOp::Remove { key } => {
                let name = strkey_name(key);
                let probe = op_str_new(name.clone());
                m = op_map_remove(m, probe); // BORROWS the key
                op_drop(probe); // we own the probe
                reference.remove(&name);
            }
            StrKeyOp::Fork => {
                op_dup(m); // rc>1: the next mutation path-copies, leaving this snapshot intact
                forks.push((m, reference.clone()));
            }
            StrKeyOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
    }
    // (1) size + per-key lookup vs the reference (probe every key in the small keyspace, present + absent).
    assert_eq!(
        op_map_size(m) as usize,
        reference.len(),
        "string-key map size matches reference"
    );
    for k in 0..8u8 {
        let name = strkey_name(k);
        let probe = op_str_new(name.clone());
        let got = op_map_lookup(m, probe); // borrows
        op_drop(probe);
        let want = reference.get(&name).copied();
        assert_eq!(
            if got == Handle::NULL {
                None
            } else {
                Some(op_get_int(got))
            },
            want,
            "string key {name:?} matches reference"
        );
    }
    // (2) canonical shape: same contents ⇒ byte-identical to a fresh twin (what string-key dedup rests on).
    let twin = {
        let mut t = op_map_empty();
        for (name, &v) in &reference {
            t = op_map_insert(t, op_str_new(name.clone()), op_box_int(v));
        }
        t
    };
    assert!(
        champ_eq(m, twin),
        "string-key map equals a fresh twin of the same contents (canonical)"
    );
    assert_eq!(champ_hash(m), champ_hash(twin), "…and hashes identically");
    op_drop(twin);
    // (3) forked snapshots undisturbed by later mutation of `m` (aliasing safety on the string-key path).
    for (h, snap) in &forks {
        assert_eq!(
            op_map_size(*h) as usize,
            snap.len(),
            "forked string-key snapshot size intact"
        );
        for (name, &v) in snap {
            let probe = op_str_new(name.clone());
            let got = op_map_lookup(*h, probe);
            op_drop(probe);
            assert_eq!(
                if got == Handle::NULL {
                    None
                } else {
                    Some(op_get_int(got))
                },
                Some(v),
                "forked snapshot string key {name:?} intact"
            );
        }
    }
    // (4) no leak / no double-free across the whole sequence.
    op_drop(m);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the whole string-key sequence"
    );
}

#[test]
fn prop_strkey_map_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<StrKeyOp>>()
        .for_each(|ops| run_strkey_op_sequence(ops));
}

// COMPOUND-VALUE variant — a `Map String String` (a compiler ENVIRONMENT: `Map String Ast`, the
// shape `subst.cdz`/`free-vars.cdz` use). Every OTHER map fuzzer uses an immediate INT value
// (`op_box_int`), which is refcount-FREE — so the compound-VALUE ownership discipline is untested:
// inserting a heap value transfers a reference; OVERWRITING a key must DROP the old value (else leak);
// REMOVING/dropping the map must free every value; FORK shares the values across versions. A String
// value is a heap `Node` (rc-tracked), so a double-free (overwrite frees twice), a leak (overwrite
// forgets the old), or a fork-aliasing bug (shared value freed early) would surface here as a
// `live_nodes()` imbalance — invisible to the int-valued fuzzers. `op_map_lookup` returns the value
// BORROWED (the map keeps ownership); the reference oracle holds owned `String`s for comparison.
#[derive(Debug, bolero::TypeGenerator)]
enum MapStrValOp {
    Insert { key: u8, val: u8 },
    Remove { key: u8 },
    Fork,
    DropForked,
}

// A small pool of distinct string VALUES (heap nodes), varied so overwrites really change content.
fn strval(v: u8) -> String {
    match v % 6 {
        0 => "apple".to_string(),
        1 => "banana".to_string(),
        2 => "".to_string(),
        3 => "a-longer-value-string".to_string(),
        4 => "x".to_string(),
        _ => "banana".to_string(), // deliberate dup with #1 (distinct nodes, equal content)
    }
}

fn run_map_str_val_op_sequence(ops: &[MapStrValOp]) {
    let before = live_nodes();
    let mut m = op_map_empty();
    let mut reference: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut forks: Vec<(Handle, std::collections::BTreeMap<String, String>)> = Vec::new();
    for op in ops {
        match *op {
            MapStrValOp::Insert { key, val } => {
                let k = strkey_name(key);
                let v = strval(val);
                // Consumes both the key and the (heap) value; overwriting `k` must drop the old value.
                m = op_map_insert(m, op_str_new(k.clone()), op_str_new(v.clone()));
                reference.insert(k, v);
            }
            MapStrValOp::Remove { key } => {
                let k = strkey_name(key);
                let probe = op_str_new(k.clone());
                m = op_map_remove(m, probe); // BORROWS the key; frees the removed value
                op_drop(probe);
                reference.remove(&k);
            }
            MapStrValOp::Fork => {
                op_dup(m); // rc>1: shares the compound values across versions
                forks.push((m, reference.clone()));
            }
            MapStrValOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
    }
    // (1) size + per-key VALUE lookup vs the reference (a borrowed String value, read via op_str_get).
    assert_eq!(
        op_map_size(m) as usize,
        reference.len(),
        "compound-value map size matches reference"
    );
    for k in 0..8u8 {
        let name = strkey_name(k);
        let probe = op_str_new(name.clone());
        let got = op_map_lookup(m, probe); // borrows both; returns the value BORROWED
        op_drop(probe);
        let want = reference.get(&name);
        match (got == Handle::NULL, want) {
            (true, None) => {}
            (false, Some(w)) => assert_eq!(
                &op_str_get(got),
                w,
                "value for key {name:?} matches reference (borrowed, not dropped)"
            ),
            _ => panic!("presence mismatch for key {name:?}"),
        }
    }
    // (2) forked snapshots undisturbed — the shared compound values survive via the fork.
    for (h, snap) in &forks {
        assert_eq!(
            op_map_size(*h) as usize,
            snap.len(),
            "forked compound-value snapshot size intact"
        );
        for (name, wv) in snap {
            let probe = op_str_new(name.clone());
            let got = op_map_lookup(*h, probe);
            op_drop(probe);
            assert!(got != Handle::NULL, "forked snapshot key {name:?} present");
            assert_eq!(
                &op_str_get(got),
                wv,
                "forked snapshot value for {name:?} intact"
            );
        }
    }
    // (3) no leak / no double-free — the balance check catches an overwrite that forgets the old
    // value (leak), an overwrite/remove that frees twice, or a fork that shares a value freed early.
    op_drop(m);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the compound-value map sequence"
    );
}

#[test]
fn prop_map_str_val_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<MapStrValOp>>()
        .for_each(|ops| run_map_str_val_op_sequence(ops));
}

// ── Perceus RESET / REUSE round-trip: a reused shell is byte-canonical + leak-free ──────────
// `op_reset` (Perceus: a unique dying node → an empty reuse token) + `op_arr_alloc_reuse` /
// `op_sum_new_reuse` (refit the token in place, no fresh Node) are SHIPPED but the compiler doesn't
// emit them yet (full Perceus deferred). They have a HISTORY of rep-divergence bugs (`@b90ab6b`
// heap-RAW→inline, `@0fb3362c` heap-HANDLES→inline) — a reused shell that keeps a leftover heap
// raw/handles Vec where a FRESH ctor gives inline is a canonical-form violation INVISIBLE to
// champ_eq/hash (they read via `Deref`), so only a `raw_is_heap`/`handles_is_heap` rep-assert catches
// it. Those are FIXED-shape tests; this fuzzes the COMBINATORIAL space (a source of random arity +
// inline/heap raw origin, reset, reused as an arr OR a sum of random arity). The invariant: a reused
// node is byte-IDENTICAL (`champ_eq` + `champ_hash`) to a from-scratch build of the same value, and
// the whole round-trip is leak/double-free-free (`live_nodes()==before`). A latent reuse bug would
// activate the instant the compiler emits reset/reuse — this pins the contract before then.
#[derive(Debug, bolero::TypeGenerator)]
struct ResetReuseCase {
    src_shape: u8, // which source shell to reset
    reuse_as_sum: bool,
    target_n: u8, // target arr arity (0..=4) or sum disc (0..=3)
    payload: u8,
}

fn run_reset_reuse_case(c: &ResetReuseCase) {
    let before = live_nodes();
    // Build a UNIQUE (rc==1) source shell of a chosen shape — the node `reset` will recycle.
    let src = match c.src_shape % 6 {
        0 => op_arr_alloc(2), // small tuple (inline handles, empty raw)
        1 => op_arr_alloc(5), // wide arr (heap handles)
        2 => op_sum_new(1, op_box_int(c.payload as i64)), // sum (1 inline handle, 4-byte inline raw)
        3 => {
            // a heap-RAW leaf (>INLINE_RAW_CAP bytes) — the `@b90ab6b` bug shape.
            let bytes: Vec<u8> = (0..(INLINE_RAW_CAP as u32 + 8))
                .map(|k| (k & 0xff) as u8)
                .collect();
            alloc(Vec::new(), bytes)
        }
        4 => {
            // a shell with a HEAP CHILD (a nested arr as a sum payload) — so `op_reset`'s child-drop
            // performs REAL reclamation; if reset forgot to drop the child, `live_nodes` catches the leak
            // (the immediate-child shapes above can't — an immediate isn't a counted Node).
            let inner = op_arr_alloc(2);
            op_arr_set(inner, 0, op_box_int(c.payload as i64));
            op_arr_set(inner, 1, op_box_int(c.payload as i64 + 1));
            op_sum_new(0, inner)
        }
        _ => op_arr_alloc(0), // an inline unit — reset must decline (returns NULL), reuse allocs fresh
    };
    let token = op_reset(src); // unique → the shell (or NULL for the inline-unit case)
    if c.reuse_as_sum {
        let disc = (c.target_n % 3) as u32;
        let reused = op_sum_new_reuse(disc, op_box_int(c.payload as i64), token);
        let fresh = op_sum_new(disc, op_box_int(c.payload as i64));
        assert!(
            champ_eq(reused, fresh),
            "reused sum == fresh sum (disc {disc}, src_shape {})",
            c.src_shape % 6
        );
        assert_eq!(
            champ_hash(reused),
            champ_hash(fresh),
            "…and hashes identically"
        );
        op_drop(reused);
        op_drop(fresh);
    } else {
        let n = (c.target_n % 5) as u32; // 0..=4 slots
        let build = |tok: Handle| -> Handle {
            let a = op_arr_alloc_reuse(n, tok);
            for i in 0..n {
                op_arr_set(a, i, op_box_int((c.payload as i64) + i as i64));
            }
            a
        };
        let reused = build(token);
        let fresh = {
            let a = op_arr_alloc(n);
            for i in 0..n {
                op_arr_set(a, i, op_box_int((c.payload as i64) + i as i64));
            }
            a
        };
        assert!(
            champ_eq(reused, fresh),
            "reused arr == fresh arr (n {n}, src_shape {})",
            c.src_shape % 6
        );
        assert_eq!(
            champ_hash(reused),
            champ_hash(fresh),
            "…and hashes identically"
        );
        op_drop(reused);
        op_drop(fresh);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the reset→reuse round-trip"
    );
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

// ── RRB persistent VECTOR randomized differential vs `Vec<i64>` ─────────────────────────────
// The map/set already have `prop_*_matches_reference` fuzz tests, but the RRB vector — the most
// structurally intricate collection (relaxed-radix rebalancing on concat/split, path-copy on a
// shared update, strict-vs-relaxed node invariants) — had only FIXED oracle spot-checks
// (`vec_concat_matches_oracle`, `vec_split_matches_oracle`). This drives random push/update/
// concat/split/fork sequences and checks the SAME four properties the map fuzz does: (1) element
// equivalence vs a `Vec<i64>` reference, (2) the RRB structural invariants hold, (3) same contents
// ⇒ byte-canonical (equals a fresh push-built twin) regardless of history, (4) forked snapshots
// stay UNDISTURBED by later mutation (aliasing safety), + no leak / no double-free at the end.
#[derive(Debug, bolero::TypeGenerator)]
enum VecOp {
    Push { elem: u8 },
    // FRONT growth — `op_vec_prepend`, the dedicated front-growth twin of `Push`. Its own path
    // (`vec_prepend_into`, packs into the leftmost leaf, mints a fresh front subtree only on a full
    // level) once built a degenerate O(n)-deep tree (#4982); interleaving it with split/concat/update/
    // fork here pins that the front-growth spine composes with relaxed interior nodes — element-equal,
    // shape-unobservable, and leak-free — under an arbitrary history, not just the fixed spot-checks.
    Prepend { elem: u8 },
    // `index`/`at` are taken MODULO the current length at apply time, so a generated value always
    // lands in-range (the ops trap OOB — the reference is what defines "in range"). A no-op on empty.
    Update { index: u8, elem: u8 },
    SplitKeepLeft { at: u8 },
    SplitKeepRight { at: u8 },
    // GROW ops — without these, short random sequences keep the vector single-leaf (≤32), never
    // reaching the MULTI-LEVEL tree or the RELAXED interior nodes that split/concat rebalancing
    // builds (the RRB's hardest correctness surface). `PushRange` bulk-appends a run (fast to a
    // multi-level STRICT tree); `ConcatRange` concats a fresh range on, whose boundary nodes go
    // RELAXED. `n` is scaled so a handful of these crosses 32 (1 leaf) and 1024 (2 levels).
    PushRange { n: u8 },
    ConcatRange { n: u8 },
    Fork,
    DropForked,
}

/// Build a vector from a `Vec<i64>` reference by repeated push — the canonical construction, so two
/// vectors with equal contents built THIS way are byte-identical (the twin oracle for property 3).
fn vec_of_reference(reference: &[i64]) -> Handle {
    let mut v = op_vec_empty();
    for &e in reference {
        v = op_vec_push(v, op_box_int(e));
    }
    v
}

fn run_vec_op_sequence(ops: &[VecOp]) {
    let before = live_nodes();
    let mut v = op_vec_empty();
    let mut reference: Vec<i64> = Vec::new();
    // Live forks: each keeps its own snapshot to verify it stays undisturbed by later mutation of `v`.
    let mut forks: Vec<(Handle, Vec<i64>)> = Vec::new();
    for op in ops {
        match *op {
            VecOp::Push { elem } => {
                let e = elem as i64;
                v = op_vec_push(v, op_box_int(e));
                reference.push(e);
            }
            VecOp::Prepend { elem } => {
                let e = elem as i64;
                v = op_vec_prepend(v, op_box_int(e));
                reference.insert(0, e);
            }
            VecOp::Update { index, elem } => {
                if !reference.is_empty() {
                    let i = (index as usize) % reference.len();
                    let e = elem as i64;
                    v = op_vec_update(v, i as u32, op_box_int(e));
                    reference[i] = e;
                }
            }
            VecOp::SplitKeepLeft { at } => {
                // Split at a valid boundary `0..=len`, keep the LEFT half, drop the right.
                let n = reference.len();
                let idx = if n == 0 { 0 } else { (at as usize) % (n + 1) };
                let (l, r) = op_vec_split(v, idx as u32);
                op_drop(r);
                v = l;
                reference.truncate(idx);
            }
            VecOp::SplitKeepRight { at } => {
                let n = reference.len();
                let idx = if n == 0 { 0 } else { (at as usize) % (n + 1) };
                let (l, r) = op_vec_split(v, idx as u32);
                op_drop(l);
                v = r;
                reference = reference.split_off(idx);
            }
            VecOp::PushRange { n } => {
                // Append `n % 40 + 1` consecutive ints (1..=40 — a few of these cross the 32-elem leaf
                // and stack toward the 1024-elem 2-level boundary), each via a real `vec-push`.
                let count = (n as i64) % 40 + 1;
                let base = reference.len() as i64;
                for j in 0..count {
                    let e = base + j;
                    v = op_vec_push(v, op_box_int(e));
                    reference.push(e);
                }
            }
            VecOp::ConcatRange { n } => {
                // Concat a fresh `[0..n%40+1)` vector onto `v`. Concat of two non-aligned vectors is what
                // builds RELAXED interior nodes (irregular child sizes + cumulative size tables) — the
                // path fixed oracle spot-checks under-cover. Consumes both; `v` becomes the result.
                let count = (n as i64) % 40 + 1;
                let tail: Vec<i64> = (0..count).collect();
                let tv = vec_of_reference(&tail);
                v = op_vec_concat(v, tv);
                reference.extend(tail);
            }
            VecOp::Fork => {
                op_dup(v); // rc>1: the next mutation of `v` must path-copy, leaving this snapshot intact
                forks.push((v, reference.clone()));
            }
            VecOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
    }
    // (1) element equivalence + length vs the reference.
    assert_eq!(
        op_vec_len(v) as usize,
        reference.len(),
        "vector length matches reference"
    );
    assert_eq!(vec_to_ints(v), reference, "vector elements match reference");
    // (2) the RRB structural invariants (relaxed size tables consistent, header count == leaf total).
    assert_vec_invariants(v);
    // (3) element-canonical but NOT shape-canonical. Unlike a CHAMP map/set (whose storage IS
    // canonical — same contents ⇒ byte-identical, so the map fuzz asserts `champ_eq` to a twin), an
    // RRB vector legitimately keeps DIFFERENT internal shapes for the same element sequence: concat
    // builds RELAXED interior nodes and split can leave a non-minimal-height spine (e.g. a 21-element
    // vector at shift=5 with a single child, vs a push-built one at shift=0). So `champ_eq(v, twin)`
    // is NOT an invariant here and must not be asserted — the property that HOLDS is that the shape
    // difference is UNOBSERVABLE: reading elements in order (`vec_to_ints`, checked in (1)) and the
    // value-encode of `v` (which renders by element, `op_vec_get` in order) both agree with a fresh
    // push-built twin. Assert the observable equivalence via a list-shape value-encode.
    let twin = vec_of_reference(&reference);
    let list_desc: &[u8] = &[0x02, 0x00, 0x07, 0x00, 0x01]; // [0]=Int [1]=List(elem→0); root=1
    let enc_v = op_value_encode_form(v, list_desc);
    let enc_twin = op_value_encode_form(twin, list_desc);
    assert_eq!(
        enc_v, enc_twin,
        "the vector's value-encode equals a fresh push-built twin's — the internal RRB shape \
         difference (relaxed / non-minimal height from concat/split) is UNOBSERVABLE at the boundary"
    );
    op_drop(twin);
    // (4) forked snapshots undisturbed by later mutation of `v` (aliasing safety).
    for (h, snap) in &forks {
        assert_eq!(
            op_vec_len(*h) as usize,
            snap.len(),
            "forked vector snapshot length intact"
        );
        assert_eq!(
            vec_to_ints(*h),
            *snap,
            "forked vector snapshot elements intact"
        );
        assert_vec_invariants(*h);
    }
    // no leak / no double-free: release everything, live count returns to baseline.
    op_drop(v);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the whole vector sequence"
    );
}

#[test]
fn prop_vec_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<VecOp>>()
        .for_each(|ops| run_vec_op_sequence(ops));
}

// ── PACKED-BOOL vector randomized differential vs `Vec<bool>` ───────────────────────────────
// The same random push/update/split/concat/fork harness as the int vector, but over a `List Bool`
// whose leaves are bit-packed. It pins the FULL contract for the packed representation under an
// arbitrary op history: (1) elements match a `Vec<bool>` reference through every op, (2) RRB
// invariants hold, (3) the value-encode equals a fresh push-built twin (packing is unobservable at
// the boundary), (4) forks stay undisturbed, no leak — PLUS the density invariant unique to bools:
// after EVERY op, every leaf is still a packed-bool leaf (packing is never lost — the operator's
// "one representation" requirement, enforced structurally, not just observationally).
fn run_bool_vec_op_sequence(ops: &[VecOp]) {
    let before = live_nodes();
    let mut v = op_vec_empty();
    let mut reference: Vec<bool> = Vec::new();
    let mut forks: Vec<(Handle, Vec<bool>)> = Vec::new();
    // Map the shared `VecOp` byte payloads onto bools: an element byte's low bit is the value; a
    // range appends a deterministic bool pattern. Every leaf must stay packed after each op.
    for op in ops {
        match *op {
            VecOp::Push { elem } => {
                let b = elem & 1 != 0;
                v = op_vec_push(v, op_box_bool(b));
                reference.push(b);
            }
            VecOp::Prepend { elem } => {
                let b = elem & 1 != 0;
                v = op_vec_prepend(v, op_box_bool(b));
                reference.insert(0, b);
            }
            VecOp::Update { index, elem } => {
                if !reference.is_empty() {
                    let i = (index as usize) % reference.len();
                    let b = elem & 1 != 0;
                    v = op_vec_update(v, i as u32, op_box_bool(b));
                    reference[i] = b;
                }
            }
            VecOp::SplitKeepLeft { at } => {
                let n = reference.len();
                let idx = if n == 0 { 0 } else { (at as usize) % (n + 1) };
                let (l, r) = op_vec_split(v, idx as u32);
                op_drop(r);
                v = l;
                reference.truncate(idx);
            }
            VecOp::SplitKeepRight { at } => {
                let n = reference.len();
                let idx = if n == 0 { 0 } else { (at as usize) % (n + 1) };
                let (l, r) = op_vec_split(v, idx as u32);
                op_drop(l);
                v = r;
                reference = reference.split_off(idx);
            }
            VecOp::PushRange { n } => {
                let count = (n as usize) % 40 + 1;
                for j in 0..count {
                    let b = (reference.len() + j) % 3 == 0;
                    v = op_vec_push(v, op_box_bool(b));
                    reference.push(b);
                }
            }
            VecOp::ConcatRange { n } => {
                let count = (n as usize) % 40 + 1;
                let tail: Vec<bool> = (0..count).map(|j| j % 2 == 0).collect();
                let tv = vec_of_bools(&tail);
                v = op_vec_concat(v, tv);
                reference.extend(tail);
            }
            VecOp::Fork => {
                op_dup(v);
                forks.push((v, reference.clone()));
            }
            VecOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
        // DENSITY INVARIANT: every leaf is packed after each individual op — packing is never lost.
        assert_all_bool_leaves_packed(v);
    }
    // (1) element equivalence + length.
    assert_eq!(
        op_vec_len(v) as usize,
        reference.len(),
        "bool vector length matches reference"
    );
    assert_eq!(
        vec_to_bools(v),
        reference,
        "bool vector elements match reference"
    );
    // (2) RRB structural invariants.
    assert_vec_invariants(v);
    // (3) value-encode equals a fresh push-built twin — packing is unobservable at the boundary.
    let twin = vec_of_bools(&reference);
    let list_desc: &[u8] = &[0x02, 0x01, 0x07, 0x00, 0x01]; // count=2 [0]=Bool [1]=List(elem→0); root=1
    let enc_v = op_value_encode_form(v, list_desc);
    let enc_twin = op_value_encode_form(twin, list_desc);
    assert_eq!(
        enc_v, enc_twin,
        "the packed bool vector's value-encode equals a fresh push-built twin's — packing and any \
         RRB shape difference are UNOBSERVABLE at the boundary"
    );
    op_drop(twin);
    // (4) forks undisturbed + no leak.
    for (h, snap) in &forks {
        assert_eq!(
            op_vec_len(*h) as usize,
            snap.len(),
            "forked bool snapshot length intact"
        );
        assert_eq!(
            vec_to_bools(*h),
            *snap,
            "forked bool snapshot elements intact"
        );
        assert_vec_invariants(*h);
    }
    op_drop(v);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the whole bool vector sequence"
    );
}

#[test]
fn prop_packed_bool_vector_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<VecOp>>()
        .for_each(|ops| run_bool_vec_op_sequence(ops));
}

// ── BYTES ROPE randomized differential vs `Vec<u8>` ─────────────────────────────────────────
// The bytes rope (O(1) concat/slice over shared leaves, flatten-on-read, slice-of-slice collapse)
// is as structurally intricate as the RRB vector but had only FIXED spot-checks. Mirror the vector
// differential: random concat / slice / compact / read / fork sequences vs a `Vec<u8>` reference.
// Like the RRB vector, the rope is ELEMENT-canonical but NOT shape-canonical (a concat/slice tree
// is a different rep from a flat leaf of the same bytes), so the invariant is CONTENT equivalence
// (`bytes_to_vec` + `bytes-len`), NOT `champ_eq`. WARNING: `op_bytes_get`/`bytes_to_vec` FLATTEN a rope in
// place — content-preserving + unobservable, but it mutates shape, so read each handle's content
// ONCE and compare to its reference; a forked snapshot is verified by its OWN content read.
#[derive(Debug, bolero::TypeGenerator)]
enum BytesOp {
    // Append a fresh `n % 40 + 1`-byte leaf via `bytes-concat` (grows the rope; concat is O(1) so a
    // deep right-leaning spine builds — the shape flatten-on-read must handle without O(n²)/overflow).
    ConcatRange { n: u8 },
    // Slice `[start, start+len)` of the current bytes (both taken modulo the live length so always
    // in range; a 0-len or full slice is a valid edge, never a trap). Exercises slice + seam-cross +
    // the slice-of-slice collapse when applied to an already-sliced rope.
    Slice { start: u8, len: u8 },
    Compact, // materialize to an independent leaf (releases any pinned parent) — content-preserving
    ReadOne, // read a single byte (flattens a rope in place) — must not change observable content
    Fork,
    DropForked,
}

fn run_bytes_op_sequence(ops: &[BytesOp]) {
    let before = live_nodes();
    let mut v = op_bytes_alloc(0); // the empty Bytes
    let mut reference: Vec<u8> = Vec::new();
    let mut forks: Vec<(Handle, Vec<u8>)> = Vec::new();
    for op in ops {
        match *op {
            BytesOp::ConcatRange { n } => {
                let count = (n as usize) % 40 + 1;
                let tail: Vec<u8> = (0..count)
                    .map(|j| (j as u8).wrapping_mul(7).wrapping_add(1))
                    .collect();
                let tv = bytes_leaf(&tail);
                v = op_bytes_concat(v, tv);
                reference.extend_from_slice(&tail);
            }
            BytesOp::Slice { start, len } => {
                let blen = reference.len();
                // start ∈ [0, blen]; len ∈ [0, blen-start] — always a valid (possibly empty) range.
                let s = if blen == 0 {
                    0
                } else {
                    (start as usize) % (blen + 1)
                };
                let max_len = blen - s;
                let l = if max_len == 0 {
                    0
                } else {
                    (len as usize) % (max_len + 1)
                };
                v = op_bytes_slice(v, s as u32, l as u32);
                reference = reference[s..s + l].to_vec();
            }
            BytesOp::Compact => {
                v = op_bytes_compact(v); // content unchanged; storage becomes independent
                // CANONICITY: compact must yield a leaf `champ_eq` + `champ_hash`-IDENTICAL to a FRESH
                // flat leaf of the same content — the property the compiler's rope-key/value-eq
                // canonicalization relies on (physical-byte `champ_eq` is correct only if compact makes
                // a rope byte-identical to a flat key/operand). The content check below covers bytes;
                // this pins the CANONICAL byte FORM (incl. the inline/heap `Raw` boundary) over the
                // arbitrary fuzzed rope shape `v` currently holds — not just the one hand-built shape.
                let twin = bytes_leaf(&reference);
                assert!(
                    champ_eq(v, twin),
                    "compacted rope is champ_eq to a fresh flat leaf of the same content (canonical form)"
                );
                assert_eq!(
                    champ_hash(v),
                    champ_hash(twin),
                    "compacted rope hashes identically to its flat twin (equal keys must hash equal)"
                );
                op_drop(twin);
            }
            BytesOp::ReadOne => {
                if !reference.is_empty() {
                    // Reading byte 0 flattens `v` in place; the value read must match the reference.
                    let got = op_bytes_get(v, 0) as u8;
                    assert_eq!(
                        got, reference[0],
                        "bytes-get(0) matches reference (flatten-safe)"
                    );
                }
            }
            BytesOp::Fork => {
                op_dup(v); // rc>1: a following consuming op path-copies, leaving this snapshot intact
                forks.push((v, reference.clone()));
            }
            BytesOp::DropForked => {
                if let Some((h, _)) = forks.pop() {
                    op_drop(h);
                }
            }
        }
    }
    // (1) length + full content vs the reference. `bytes_to_vec` flattens `v` in place — fine, it is
    // content-preserving and this is the last read of `v`'s shape we depend on.
    assert_eq!(
        op_bytes_len(v) as usize,
        reference.len(),
        "bytes length matches reference"
    );
    assert_eq!(
        bytes_to_vec(v),
        reference,
        "bytes content matches reference"
    );
    // (2) forked snapshots undisturbed by later mutation of `v` (aliasing safety over shared leaves).
    // Each fork reads its OWN content (flattening that snapshot, independent of `v`).
    for (h, snap) in &forks {
        assert_eq!(
            op_bytes_len(*h) as usize,
            snap.len(),
            "forked bytes snapshot length intact"
        );
        assert_eq!(
            bytes_to_vec(*h),
            *snap,
            "forked bytes snapshot content intact"
        );
    }
    // (3) no leak / no double-free across the whole rope of shared slices/concats.
    op_drop(v);
    for (h, _) in forks {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "no leak / no double-free across the whole bytes sequence"
    );
}

#[test]
fn prop_bytes_matches_reference_under_random_op_sequences() {
    bolero::check!()
        .with_type::<Vec<BytesOp>>()
        .for_each(|ops| run_bytes_op_sequence(ops));
}

// ── value-encode TOTALITY under an ARBITRARY (possibly malformed) descriptor ────────────────
// `op_value_encode_form` is the value-heap ESCAPE — the runtime's most complex descriptor-driven
// walk, run at the host boundary. Its contract (docstring): "A malformed descriptor / unrenderable
// shape yields the empty Bytes … never a trap." The compiler only bakes well-formed descriptors
// today, but a future descriptor-gen bug must DECLINE, not crash/hang the guest with no diagnostic.
// Fuzz RANDOM descriptor bytes against representative values and assert the op always RETURNS (Some
// or None — never a panic/trap/hang) and leaks nothing. This caught the arity-mismatch trap the
// Tuple/Record/Spread arms now guard (a descriptor claiming N fields for a node with fewer used to
// `op_arr_get`-trap instead of declining) — the same class as `decode_descriptor`'s `with_capacity`
// on an untrusted length, which this also exercises.
fn assert_encode_is_total(desc: &[u8]) {
    let before = live_nodes();
    // A menagerie of representative values: a scalar leaf, a 2-tuple, a nested sum, a small vec, a
    // string, a bytes leaf, a 1-entry map. A mismatched descriptor against ANY must decline, not trap.
    let values: Vec<Handle> = alloc::vec![
        op_box_int(42),
        {
            let t = op_arr_alloc(2);
            op_arr_set(t, 0, op_box_int(1));
            op_arr_set(t, 1, op_box_int(2));
            t
        },
        op_sum_new(0, op_box_int(7)),
        {
            let mut v = op_vec_empty();
            for i in 0..3 {
                v = op_vec_push(v, op_box_int(i));
            }
            v
        },
        op_str_new(String::from("hi")),
        bytes_leaf(&[1, 2, 3]),
        op_map_insert(op_map_empty(), op_box_int(1), op_box_int(10)),
        // A runtime BigInt (wire tag 17, `Shape::BigInt` → `unbox_bigint`/`bigint_leaf`). MULTI-LIMB
        // (i64::MAX² ≈ 2^126) so the arbitrary-width `bigint_leaf` arm — sign + BE-magnitude → KIND_INT
        // — is exercised under the fuzz, not just the one fixed `bigint_escape…` shape. A BigInt under a
        // MISMATCHED random descriptor must decline (not trap); under a real tag-17 descriptor it renders.
        box_bigint(&bigint::Big::from_i64(i64::MAX).mul(&bigint::Big::from_i64(i64::MAX))),
        // Float64 / Float32 (tags 2 / 14 → `float_leaf`/`float32_leaf`). These arms convert an f64/f32
        // to an exact decimal and DECLINE (`None`, so the whole encode declines) on a non-finite value —
        // a totality path (a return, not a trap) the fuzz must also cover under arbitrary descriptors.
        op_box_float(1.5),
        op_box_float32(-2.5f32),
    ];
    for &h in &values {
        // The property: NO panic. The result (Some doc / None decline) is not checked for content —
        // only that the op RETURNS. `black_box` so the walk isn't optimized away.
        let out = op_value_encode_form(h, desc);
        core::hint::black_box(&out);
    }
    for h in values {
        op_drop(h);
    }
    assert_eq!(
        live_nodes(),
        before,
        "value-encode leaks nothing regardless of the descriptor (declines cleanly, borrows the value)"
    );
}

#[test]
fn prop_value_encode_is_total_under_arbitrary_descriptor() {
    bolero::check!()
        .with_type::<Vec<u8>>()
        .for_each(|desc| assert_encode_is_total(desc));
}

// ── The eq ⟹ hash ⟹ cmp contract over ARBITRARY heterogeneous compound trees ────────────────
// EVERY map/set key rests on: structurally-equal values (distinct nodes) are `champ_eq`, hash
// IDENTICALLY (`champ_hash`), and compare Equal (`champ_key_cmp`) — a violation silently corrupts keys
// (a lookup misses its own entry, or two distinct keys collapse). The fixed-shape tests
// (`champ_hash_matches_naive_reference`, `champ_key_cmp_is_consistent_with_eq`) cover hand-built
// shapes; the map/set fuzzes cover int/string/tuple keys. NONE fuzzes the contract over RANDOM nested
// MIXED structures (tuple-of-sum-of-bytes-of-float …). Build a tree from random bytes TWICE (distinct
// nodes) and assert the three agree; also that a byte-DIFFERENT tree is not-eq with consistent cmp.

/// Build a compound value tree from `bytes` (a cursor into them), bounded by `budget` nodes and
/// `depth`. Deterministic: the same byte prefix builds the same structure, so two calls give
/// structurally-identical, distinct-node twins. Returns (handle, bytes_consumed_advances via the
/// shared cursor). Leaves when out of budget/bytes/depth.
fn build_rand_value(bytes: &[u8], cur: &mut usize, budget: &mut u32, depth: u32) -> Handle {
    let tag = if *cur < bytes.len() { bytes[*cur] } else { 0 };
    *cur += 1;
    // A small scalar payload byte (deterministic from the stream).
    let p = if *cur < bytes.len() { bytes[*cur] } else { 0 };
    *cur += 1;
    // Out of budget/depth → force a scalar leaf so the tree is finite.
    let allow_compound = *budget > 2 && depth < 5;
    *budget = budget.saturating_sub(1);
    match tag % if allow_compound { 9 } else { 6 } {
        0 => op_box_int(p as i64 - 128), // small signed int (incl. negatives)
        1 => op_box_bool(p & 1 == 0),
        2 => op_arr_alloc(0),                          // unit (inline)
        3 => op_str_new(alloc::format!("s{}", p % 7)), // one of a few strings (dedup/collision)
        4 => {
            // a small bytes leaf
            let b = op_bytes_alloc((p % 4) as u32);
            for i in 0..(p % 4) as u32 {
                op_bytes_set(b, i, (p.wrapping_add(i as u8)) as u32);
            }
            b
        }
        5 => op_box_float(((p % 5) as f64) - 2.0), // a few finite floats incl. negative
        6 => {
            // a 2-tuple of sub-values
            let a = build_rand_value(bytes, cur, budget, depth + 1);
            let b = build_rand_value(bytes, cur, budget, depth + 1);
            let t = op_arr_alloc(2);
            op_arr_set(t, 0, a);
            op_arr_set(t, 1, b);
            t
        }
        7 => {
            // a sum with a single sub-value payload, disc in 0..3
            let payload = build_rand_value(bytes, cur, budget, depth + 1);
            op_sum_new((p % 3) as u32, payload)
        }
        _ => {
            // a 3-tuple (records/wider products)
            let a = build_rand_value(bytes, cur, budget, depth + 1);
            let b = build_rand_value(bytes, cur, budget, depth + 1);
            let c = build_rand_value(bytes, cur, budget, depth + 1);
            let t = op_arr_alloc(3);
            op_arr_set(t, 0, a);
            op_arr_set(t, 1, b);
            op_arr_set(t, 2, c);
            t
        }
    }
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

// ── value-encode ITERATIVE vs RECURSIVE equivalence over RANDOM MIXED shapes ────────────────
// The escape's load-bearing correctness property is: the iterative production walk (`encode_value`,
// an explicit worklist — the walk the guest actually runs) produces byte-IDENTICAL output to the
// simple recursive reference (`encode_value_recursive`). Today that equivalence is differential-tested
// ONLY on int LISTS (`value_encode_iterative_matches_recursive_reference`, varied depth); every other
// shape (nested tuple/sum/record, mixed leaves) is guarded only by FIXED hand-built encode tests, not
// by the iterative-vs-recursive equivalence over VARIED shapes — exactly the nested AST shapes a
// self-hosting compiler's value-encode will hit. A worklist-management bug in the iterative walk
// (wrong child order, a mishandled Sum/Tuple frame, a pool-reuse aliasing error) that the recursive
// mirror would NOT have would slip through. This fuzzes that equivalence: build a random mixed value
// AND its matching descriptor together, then assert both walks agree byte-for-byte.

/// Build a random value AND its matching shape descriptor from one byte stream. Appends each node's
/// `Shape` to `table` and returns its index, so the value's node structure and the descriptor stay
/// aligned by construction. Shapes mirror `build_rand_value`'s producers — int/bool/unit/str/bytes/
/// float leaves + the compounds with a settled canonical value form: 2-tuple, sum, 2-field record,
/// set-of-int, list-of-int, map-int→int, 3-tuple (the List/Map/Set/Record/Tuple heads all encode
/// head-first via their M2 `KIND_*_CTOR` ctor-leaf; map/record entries via `FieldPair`). Depth
/// is capped low (the recursive oracle overflows on deep values — that is the ITERATIVE walk's reason
/// to exist, tested separately by `value_encode_deep_recursive_value_does_not_overflow_the_stack`);
/// this targets shape VARIETY, not depth.
fn build_rand_value_and_shape(
    bytes: &[u8],
    cur: &mut usize,
    budget: &mut u32,
    depth: u32,
    table: &mut Vec<super::Shape>,
) -> (Handle, u32) {
    use super::Shape as S;
    let tag = if *cur < bytes.len() { bytes[*cur] } else { 0 };
    *cur += 1;
    let p = if *cur < bytes.len() { bytes[*cur] } else { 0 };
    *cur += 1;
    let allow_compound = *budget > 3 && depth < 4;
    *budget = budget.saturating_sub(1);
    // Push a shape and return its table index.
    fn emit(t: &mut Vec<super::Shape>, s: super::Shape) -> u32 {
        t.push(s);
        (t.len() - 1) as u32
    }
    match tag % if allow_compound { 13 } else { 6 } {
        0 => {
            let h = op_box_int(p as i64 - 128);
            (h, emit(table, S::Int))
        }
        1 => {
            let h = op_box_bool(p & 1 == 0);
            (h, emit(table, S::Bool))
        }
        2 => {
            let h = op_arr_alloc(0); // unit
            (h, emit(table, S::Unit))
        }
        3 => {
            let h = op_str_new(alloc::format!("s{}", p % 7));
            (h, emit(table, S::Str))
        }
        4 => {
            let n = (p % 4) as u32;
            let b = op_bytes_alloc(n);
            for i in 0..n {
                op_bytes_set(b, i, (p.wrapping_add(i as u8)) as u32);
            }
            (b, emit(table, S::Bytes))
        }
        5 => {
            let h = op_box_float(((p % 5) as f64) - 2.0);
            (h, emit(table, S::Float))
        }
        6 => {
            // 2-tuple. Reserve this node's table slot BEFORE recursing so children get later indices.
            let ix = emit(table, S::Tuple(vec![0, 0].into()));
            let (a, sa) = build_rand_value_and_shape(bytes, cur, budget, depth + 1, table);
            let (bch, sb) = build_rand_value_and_shape(bytes, cur, budget, depth + 1, table);
            table[ix as usize] = S::Tuple(vec![sa, sb].into());
            let t = op_arr_alloc(2);
            op_arr_set(t, 0, a);
            op_arr_set(t, 1, bch);
            (t, ix)
        }
        7 => {
            // Sum with a single payload, disc in 0..3. The descriptor's variant table must have an
            // entry for the CHOSEN disc (the walk indexes `variants[disc]`); give it disc+1 variants,
            // all pointing at the same payload shape (only the chosen one is read).
            let disc = (p % 3) as usize;
            let ix = emit(table, S::Sum(vec![].into()));
            let (payload, sp) = build_rand_value_and_shape(bytes, cur, budget, depth + 1, table);
            let variants: Vec<(Rc<str>, u32)> = (0..=disc)
                .map(|d| (alloc::format!("V{d}").into(), sp))
                .collect();
            table[ix as usize] = S::Sum(variants.into());
            (op_sum_new(disc as u32, payload), ix)
        }
        8 => {
            // 2-field Record, fields in descriptor (sorted) name order `f0` < `f1` — exercises the
            // record-field `=` convergence site. Reserve the slot before recursing (children later).
            let ix = emit(table, S::Record(vec![].into()));
            let (a, sa) = build_rand_value_and_shape(bytes, cur, budget, depth + 1, table);
            let (bch, sb) = build_rand_value_and_shape(bytes, cur, budget, depth + 1, table);
            table[ix as usize] = S::Record(vec![("f0".into(), sa), ("f1".into(), sb)].into());
            let r = op_arr_alloc(2);
            op_arr_set(r, 0, a);
            op_arr_set(r, 1, bch);
            (r, ix)
        }
        9 => {
            // Set of Int (scalar elements are canonically orderable — `set_elements_canonical` sorts
            // them) — exercises the Set `(. Set of)` head convergence site. Insert a few ints in
            // non-sorted order; the encode re-sorts to canonical value order.
            let ix = emit(table, S::Set(0));
            // Element shape [0] = Int must exist BEFORE the Set entry references it. The Set entry is
            // at `ix`; point it at a freshly-emitted Int shape so the index is valid regardless of
            // where `ix` landed.
            let elem_ix = emit(table, S::Int);
            table[ix as usize] = S::Set(elem_ix);
            let mut s = op_set_empty();
            for k in 0..((p % 4) as i64) {
                s = op_set_insert(s, op_box_int((3 - k) * 7 + (p as i64 & 3)));
            }
            (s, ix)
        }
        10 => {
            // List of Int — exercises the head-first `KIND_LIST_CTOR` ctor-leaf head + the RRB spine
            // encode (0..4 elements; an EMPTY list is the single ctor-head leaf, the empty-collection
            // case). Element shape `[elem_ix] = Int` is emitted BEFORE the List entry references it, so
            // the index is valid regardless of where `ix` landed.
            let ix = emit(table, S::List(0));
            let elem_ix = emit(table, S::Int);
            table[ix as usize] = S::List(elem_ix);
            let mut l = op_vec_empty();
            for k in 0..((p % 4) as i64) {
                l = op_vec_push(l, op_box_int(k * 5 - 3 + (p as i64 & 1)));
            }
            (l, ix)
        }
        11 => {
            // Map Int→Int — exercises the head-first `KIND_MAP_CTOR` head + the `FieldPair`-headed
            // entries (the M2 map-entry form). Scalar Int keys are canonically orderable, so the encode
            // re-sorts to canonical key order; insert a few DISTINCT keys in non-sorted order. Key then
            // value shape emitted before the Map entry references them.
            let ix = emit(table, S::Map(0, 0));
            let k_ix = emit(table, S::Int);
            let v_ix = emit(table, S::Int);
            table[ix as usize] = S::Map(k_ix, v_ix);
            let mut m = op_map_empty();
            for k in 0..((p % 4) as i64) {
                m = op_map_insert(m, op_box_int((3 - k) * 11), op_box_int(k + 1));
            }
            (m, ix)
        }
        _ => {
            // 3-tuple.
            let ix = emit(table, S::Tuple(vec![0, 0, 0].into()));
            let (a, sa) = build_rand_value_and_shape(bytes, cur, budget, depth + 1, table);
            let (bch, sb) = build_rand_value_and_shape(bytes, cur, budget, depth + 1, table);
            let (cch, sc) = build_rand_value_and_shape(bytes, cur, budget, depth + 1, table);
            table[ix as usize] = S::Tuple(vec![sa, sb, sc].into());
            let t = op_arr_alloc(3);
            op_arr_set(t, 0, a);
            op_arr_set(t, 1, bch);
            op_arr_set(t, 2, cch);
            (t, ix)
        }
    }
}

#[test]
fn prop_value_encode_iterative_matches_recursive_over_random_shapes() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
        reset();
        let before = live_nodes();
        let mut table: Vec<super::Shape> = Vec::new();
        let (mut cur, mut budget) = (0usize, 40u32);
        let (v, root) = build_rand_value_and_shape(bytes, &mut cur, &mut budget, 0, &mut table);
        let descriptor = super::Descriptor { table, root };
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

/// CANON-STABILITY across the FULL shape space (the property companion of the hand-written
/// `value_encode_leaf_order_is_canon_pre_order_first_encounter`): for ANY random value/descriptor,
/// value-encode's document must have its LEAVES interned in canon's order — strictly PRE-ORDER,
/// first-encounter, left-to-right over the struct tree from the root (cadenza-ast/canon.rs `visit`).
/// That is exactly what makes `value_encode(v)` == `codec::encode(canon(tree))` a stable content-
/// address. The two-shape unit test only reaches the record-`=` and Set-head arms; this exercises
/// Tuple/List/Sum/Map/Named/Framed/nested arms too, so a post-order regression in ANY arm is caught.
#[test]
fn prop_value_encode_leaf_order_is_canon_over_random_shapes() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
        reset();
        let before = live_nodes();
        let mut table: Vec<super::Shape> = Vec::new();
        let (mut cur, mut budget) = (0usize, 40u32);
        let (v, root) = build_rand_value_and_shape(bytes, &mut cur, &mut budget, 0, &mut table);
        let descriptor = super::Descriptor { table, root };
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

/// value-DECODE (heap idx 90) is the inverse of value-encode: for ANY random value, encoding then
/// decoding under the same descriptor must reconstruct a STRUCTURALLY-EQUAL value — the B0 round-trip
/// property (`decode ∘ encode == id`), across the full shape space (Tuple/List/Sum/Record/Set/nested),
/// not just the hand-picked `value_decode_round_trips_*` cases. Also asserts decode never leaks and
/// never traps (returns a handle or declines to NULL). Drives `decode_value` on the in-memory
/// `Descriptor`+`ParsedDoc` directly (op_value_decode's guts) so no descriptor byte-serializer is
/// needed. NOTE: a Set re-canonicalizes on encode (elements sorted by value), so the decoded Set is
/// value-equal though not necessarily node-identical — `value_eq_shaped` compares by canonical value,
/// which is the correct equality here.
#[test]
fn prop_value_decode_round_trips_over_random_shapes() {
    bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
        reset();
        let before = live_nodes();
        let mut table: Vec<super::Shape> = Vec::new();
        let (mut cur, mut budget) = (0usize, 40u32);
        let (v, root) = build_rand_value_and_shape(bytes, &mut cur, &mut budget, 0, &mut table);
        let descriptor = super::Descriptor { table, root };
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

/// value-DECODE totality on ARBITRARY bytes — the decode-side sibling of
/// `prop_value_encode_is_total_under_arbitrary_descriptor`. Since B2/B3 (`apply(list<u8>)->list<u8>`),
/// `op_value_decode` is on the critical path of every reducer call, fed guest-produced (and thus
/// potentially malformed or adversarial) doc + descriptor bytes by the kernel. It MUST be TOTAL: for
/// ANY two byte strings it returns a Handle or declines to NULL — never traps (which would abort the
/// kernel), never leaks, never overflows the stack. The hand test only checks 3 fixed malformed inputs;
/// this fuzzes BOTH the document AND the descriptor (the split point derived from the stream, so both
/// halves range over arbitrary bytes independently). Content is NOT asserted — only that the op returns.
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

// ── U6: FBIP rc==1 in-place cursor advance for map-iter-next / set-iter-next ────────────────
// Load-bearing: (1) a forked/peeked/teed cursor (rc>1) stays INDEPENDENT — advancing one owner
// must not disturb the other (aliasing catcher); (2) a unique (rc==1) walk allocates ZERO new
// cursor nodes steady-state (the WIT promise); (3) order + exhausted-signal identical to the copy
// path. The pre-existing collect_map/collect_set (which advance an rc==1 cursor in a loop) already
// exercise the FBIP path across the whole suite — these pin the properties explicitly.

/// A rich map with a subnode split + a collision pair, so the cursor's frame stack goes deep.
fn deep_walk_map() -> Handle {
    let (sa, sb) = low5_split_pair();
    let (ca, cb) = full_hash_collision_pair();
    let mut m = op_map_empty();
    for &(k, v) in &[
        (sa, 1i64),
        (sb, 2),
        (ca, 3),
        (cb, 4),
        (5i64, 50),
        (11, 110),
        (23, 230),
    ] {
        m = minsert_int(m, k, v);
    }
    m
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

/// `op_bigint_of_bytes` calls `bytes_flatten(buf)` before reading `raw` — because a Bytes leaf may be a
/// ROPE (concat/slice nodes), whose `raw` holds the node's HEADER bytes, NOT the content (the same
/// rope-read landmine fixed in `str-get`, `@9b24aeb2`). The compiler bakes a FLAT leaf, so that flatten
/// is defensive — and the sibling's round-trip test builds via `op_bytes_alloc` (flat), leaving the
/// flatten path UNEXERCISED. Pin it: build the SAME sign-magnitude bytes as a ROPE (concat across a
/// seam) and confirm `bigint-of-bytes` yields the identical BigInt as the flat leaf — proving the
/// flatten materializes the rope before decoding, not reading concat-header garbage as a magnitude.
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

// ── U7: CHAMP set algebra — union / intersection / difference ──────────────────────────────
// Correctness vs a std BTreeSet reference; canonical shape; correct RC (consume both operands, no
// leak / no double-free); empty-operand identities; shared-operand safety (a kept operand `dup`ed
// first stays intact after the consuming op).

/// Build a set from a slice of ints (each `sinsert_int` consumes the running set).
fn set_of(elems: &[i64]) -> Handle {
    let mut s = op_set_empty();
    for &e in elems {
        s = sinsert_int(s, e);
    }
    s
}

/// Assert a runtime set's membership + size EXACTLY match a reference over `universe`.
fn assert_set_eq_reference(
    s: Handle,
    reference: &std::collections::BTreeSet<i64>,
    universe: &[i64],
) {
    assert_eq!(
        op_set_size(s) as usize,
        reference.len(),
        "size matches reference"
    );
    for &e in universe {
        assert_eq!(
            scontains_int(s, e),
            reference.contains(&e),
            "membership of {e} matches reference"
        );
    }
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

// ── Review gap: split a RELAXED vector (produced by concat) ─────────────────────────────────
// Every other split test uses vec_range(n) (STRICT, push-built), so `vec_split_subtree`'s relaxed
// descent branch (vec_is_relaxed ⇒ vec_find_child_relaxed) was never exercised. Concat two lists
// then split the result — a natural composition — to hit it.

/// Build a relaxed 80-element vector = concat([0..40), [0..40)); assert its root is relaxed so we
/// KNOW the relaxed-split branch is taken. Oracle = 0..40 followed by 0..40.
fn relaxed_80() -> Handle {
    let c = op_vec_concat(vec_range(40), vec_range(40));
    let (_count, _shift, root) = vec_read_header(c);
    assert!(
        vec_is_relaxed(root),
        "concat(40,40) must produce a relaxed root"
    );
    c
}

fn relaxed_80_oracle() -> Vec<i64> {
    let mut o: Vec<i64> = (0..40).collect();
    o.extend(0..40);
    o
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

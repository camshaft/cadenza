pub(crate) use super::Shape as S;
pub(crate) use super::*;

/// No shared table to clear — every value is its own allocation and every test holds the handles
/// it builds. Kept as a documented no-op so each test reads as a self-contained scenario.
pub(crate) fn reset() {}

/// The `IntList` shape descriptor `(type IL (Cons (Tuple Int64 IL)) Nil)`, wrapped in the outer
/// `(: <value> IL)` frame — a TABLE with a self-`Ref` closing the recursion (as the compiler bakes
/// it). Table: [0]=Int, [1]=Sum[(Cons→2),(Nil→3)], [2]=Tuple[→0,→1], [3]=Unit, [4]=Named("IL"→1);
/// root=4. The `Cons` payload tuple's second element (→1) points back at the Sum — a finite 1-entry
/// cycle the value walk unfolds to the value's depth.
pub(crate) fn intlist_descriptor() -> Vec<u8> {
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
pub(crate) fn set_int_descriptor() -> Vec<u8> {
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
pub(crate) fn map_int_int_descriptor() -> Vec<u8> {
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

/// A `(Set (Tuple Int64 Int64))` descriptor: table [0]=Int, [1]=Tuple[→0,→0], [2]=Set(→1); root=2.
/// Set tag = 12, Tuple tag = 6. Elements are ORDERABLE COMPOUNDS — `set-to-list` must sort them by the
/// SAME lexicographic total order `value_cmp_shaped` (== the runtime `<`) supplies, not decline.
pub(crate) fn set_tuple_int_int_descriptor() -> Vec<u8> {
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

/// `map-to-list` enumerates a map's entries as a `List (Tuple k v)` in CANONICAL KEY order, each entry
/// a 2-element tuple `[key, value]`. Insert keys out of order; the result must be the entries sorted by
/// key, with values intact, and the heap must balance.

/// A non-scalar (unorderable) element/key shape, or a descriptor whose root is not a Set/Map, DECLINES
/// to the EMPTY list — the never-trap totality contract (the compiler bakes only a well-formed
/// descriptor, but the op must be total on any input). Here a `(Set Int64)` value handed a MISMATCHED
/// descriptor whose root is a bare `Int` (not a Set) yields the empty list, not a trap.

/// The root-`Framed` plain-Tuple descriptor `(: <value> (Tuple Int64 Int64))` — a tag-15 `Framed`
/// whose TypeNode is `Tuple` with two `Int64` children, inner → a `Tuple[→Int, →Int]` table entry.
/// This is the descriptor `sum_shape_descriptor` bakes for a `Value.encode` of a two-int tuple (the
/// PUBLIC value-encode path frames the compound; the fold/reducer boundary's `bare_shape_descriptor`
/// does NOT — see rcdzc `sum_shape_descriptor` vs `bare_shape_descriptor`). Kept as a test constant.
pub(crate) fn framed_int_pair_descriptor() -> Vec<u8> {
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

/// The root-`Framed` two-field-record descriptor `(: <value> (Record (a Int64) (b Int64)))` —
/// tag-15 `Framed` whose TypeNode is `record` with two field children (`a`→`Int64`, `b`→`Int64`,
/// each a `(name <type>)` node), inner → a `Record[a→0, b→0]` table entry. This is what
/// `sum_shape_descriptor` bakes for a `Value.encode` of a two-`Int64`-field record.
pub(crate) fn framed_int_record_descriptor() -> Vec<u8> {
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

/// A small LEB + length-prefixed-name descriptor builder, shared by the framed-Sum goldens below.
pub(crate) fn desc_leb(out: &mut Vec<u8>, mut v: u64) {
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
pub(crate) fn desc_name(out: &mut Vec<u8>, s: &str) {
    desc_leb(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// The generic-Sum descriptor `(: <value> (Option Int64))` — a boxed GENERIC sum (`args` non-empty)
/// roots at a PARAMETRIC `Framed(TypeNode Option[Int64], inner)`. Table: [0] Int, [1] Unit (the None
/// payload), [2] Sum[(Some→0),(None→1)], [3] Framed(Option[Int64] → 2); root = 3.
pub(crate) fn framed_option_int_descriptor() -> Vec<u8> {
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

/// The monomorphic-Sum descriptor `(: <value> Shape)` where `Shape = (Circle Int64) | (Rect Int64
/// Int64)` — a MONOMORPHIC sum (`args: []`) roots at a bare-name `Named("Shape", inner)`, NOT a
/// parametric `Framed`. A multi-payload variant's payload is a `Spread` (its elements splice flat).
/// Table: [0] Int, [1] Spread[→0,→0] (Rect's two Int64s), [2] Sum[(Circle→0),(Rect→1)],
/// [3] Named("Shape" → 2); root = 3.
pub(crate) fn named_shape_descriptor() -> Vec<u8> {
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

/// The framed Int×Float tuple descriptor `(: <value> (Tuple Int64 Float64))` — tag-15 `Framed` whose
/// TypeNode is `Tuple[Int64, Float64]`, inner → a `Tuple[→Int, →Float]`. Exercises the FLOAT leaf
/// (KIND_FLOAT exact-decimal) inside the framed cross-backend golden.
pub(crate) fn framed_int_float_pair_descriptor() -> Vec<u8> {
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

/// The framed Map descriptor `(: <value> (Map Int64 Int64))` — tag-15 `Framed` whose TypeNode is
/// `Map[Int64, Int64]`, inner → a `Map(key→0, val→0)` (tag 13).
pub(crate) fn framed_int_map_descriptor() -> Vec<u8> {
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
pub(crate) fn framed_int_set_descriptor() -> Vec<u8> {
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

/// GOLDEN pin, SET shape (v-rust-backend fixture, 2026-08-16): `Value.encode` of
/// `(Set.of (list 7 12 17))` at `(Set Int64)` must render `(: ((. Set of) (list 7 12 17)) (Set
/// Int64))` — the `((. Set of) (list …))` member-access form, elements in CANONICAL order. Pins the
/// Set member-order-at-build contract. Guarded three ways; the cadenza-ast mirror asserts the same
/// bytes. `#[cfg(test)]`.

/// The framed BigInt descriptor `(: <value> BigInt)` — tag-15 `Framed` whose TypeNode is a bare-leaf
/// `BigInt`, inner → a `BigInt` (tag 17). A BigInt renders as a plain KIND_INT leaf.
pub(crate) fn framed_bigint_descriptor() -> Vec<u8> {
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

/// The framed Rational descriptor `(: <num>/<den> Rational)` — tag-18 `Rational` leaf, wrapped in a
/// tag-15 `Framed` whose TypeNode is the childless name `Rational` (mirrors the BigInt descriptor:
/// one scalar-ish leaf under one frame).
pub(crate) fn framed_rational_descriptor() -> Vec<u8> {
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

// A bare `Char` value-codec descriptor: one shape-table entry (tag 19 = Char) at the root. A char
// value is an immediate int codepoint at runtime; the descriptor's tag 19 is the ONLY thing that
// distinguishes it from an `Int` at the encode/decode boundary (it selects the `KIND_CHAR` leaf).
pub(crate) fn char_scalar_descriptor() -> Vec<u8> {
    let mut d = Vec::new();
    desc_leb(&mut d, 1); // table_len = 1
    d.push(19); // [0] Char
    desc_leb(&mut d, 0); // root → 0
    d
}

// `(tuple Char Int)` — exercises Char as a non-root child (the walk reaches it through `arr-get`).
pub(crate) fn char_int_tuple_descriptor() -> Vec<u8> {
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
pub(crate) fn kind_char_leaf_bytes(c: char) -> Vec<u8> {
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

/// Char as a compound CHILD: `(tuple #\λ 42)`. The tuple heap node is built, encoded (the char field
/// still emits a KIND_CHAR leaf), decoded back, and the char field recovers its codepoint; dropping
/// the decoded tuple frees clean. Guards that the Char arm fires through `arr-get`, not just at root.

/// A lone surrogate (U+D800) is not a Unicode scalar, so `char::from_u32` rejects it: op62's Char arm
/// returns `None` and `value-encode` DECLINES (returns None) rather than trapping or emitting garbage —
/// a bad codepoint is DATA, handled totally. The immediate int leaves the census untouched.

/// The ORIGINAL recursive `encode_value`, kept as the differential oracle for the iterative
/// production walk. Byte-for-byte identical logic; the ONLY difference is native recursion vs the
/// production explicit heap stack. A deep value overflows THIS (that is the bug the iterative walk
/// fixes), so the differential test drives it only to modest depth.
pub(crate) fn encode_value_recursive(
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
        S::Symbol => {
            // Mirror encode_value's Symbol member-compound `((. Symbol of) "text")` (byte-identity oracle):
            // list([ list([KIND_MEMBER, name "Symbol", name "of"]), str ]). Read the raw like S::Str.
            bytes_flatten(h);
            let bytes = with_node(h, Vec::new(), |n| n.raw.as_slice().to_vec());
            let str_leaf = b.str_leaf(&bytes);
            let str_atom = b.atom(str_leaf);
            let member_kind = b.ctor_leaf(doc::KIND_MEMBER);
            let member_atom = b.atom(member_kind);
            let sym_leaf = b.name_leaf("Symbol");
            let sym_atom = b.atom(sym_leaf);
            let of_leaf = b.name_leaf("of");
            let of_atom = b.atom(of_leaf);
            let member = b.list(&[member_atom, sym_atom, of_atom]);
            b.list(&[member, str_atom])
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

pub(crate) fn build_intlist(n: usize) -> Handle {
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
pub(crate) fn record_with_set_descriptor() -> Vec<u8> {
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

/// A rendered-text gate CANNOT catch a regression here: emitting the record-field `=` or the Set
/// `(. Set of)` head POST-order (the pre-convergence bug) produces IDENTICAL rendered s-expr text but a
/// DIFFERENT leaf pool — so it would slip silently past the corpus. This walks the parsed document and
/// asserts each leaf id is first-referenced in non-decreasing pre-order, exactly canon's numbering.

/// The iterative production `encode_value` must produce BYTE-IDENTICAL documents to the recursive
/// oracle, across the interesting shapes (nested sums, lists, tuples). Drives only modest depth — a
/// deep value would overflow the recursive oracle (the exact bug the iterative walk fixes).

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

/// `value-encode` renders a String payload (`Shape::Str`) as a `KIND_STR` leaf — the codec's string
/// leaf (kind 7, `write_bytes` = LEB len + UTF-8 body). Previously `encode_value` DECLINED on
/// `Shape::Str` (returned `None`), so a recursive value carrying a string (an AST node, a JSON tree)
/// could not cross the host boundary at all, even though the wire format has the kind. Byte-exact.

/// A `Str`/`Bytes` leaf stores its bytes as `Raw` (inline ≤INLINE_RAW_CAP=12, else heap) so a SHORT
/// string allocates NO per-leaf `Vec` — but a LONGER string must still round-trip byte-exact through the
/// heap arm. The inline↔heap boundary (12 bytes) is invisible in the output (both write the same KIND_STR
/// len+body), so pin it: a 12-byte (inline max) and a 13-byte (first heap) string each encode to their
/// exact KIND_STR bytes. Guards `Raw::from_slice`'s boundary in the leaf path — a short-string regression
/// (dropping the inline arm) or an off-by-one at the cap would still pass the existing "hi" test.

/// The single-entry `DESCRIPTOR_CACHE` must never cross-contaminate: two DIFFERENT descriptors, whether
/// alternated (thrashing the 1-entry cache — every call a miss) or repeated (hitting), must each yield
/// the SAME output as a fresh decode would. The cache key is the descriptor BYTES, so a byte-different
/// descriptor must always re-decode; this pins that the key comparison + refresh is correct (a bug that
/// returned the STALE cached descriptor for new bytes would render the wrong value). Encodes an Int
/// (desc A) and a Str (desc B) in an ALTERNATING sequence, then each REPEATED, asserting every result.

/// value-encode of a ROPE String (concat/slice nodes) via `Shape::Str` must MATERIALIZE it first.
/// Since a runtime `String.concat`/`String.at`-slice lowers to the SAME `bytes-concat`/`bytes-slice`
/// rope nodes as Bytes (a String IS a bytes rope), a rope-String reaching `Shape::Str` is NOT a flat
/// leaf — the encoder must `bytes_flatten` before reading `raw` (fixed `@b77b3ae0`; without it a rope
/// String rendered its raw HANDLE bytes = garbage). Every OTHER `Shape::Str` test uses `op_str_new` (a
/// FLAT leaf), so the flatten line was runtime-untested (only an e2e wasmtime spot check). Build a rope
/// String the way the compiler does and assert it encodes byte-identically to the equivalent flat one.

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

/// The reused thread-local `DocBuilder`/`out` (`ENCODE_BUILDER`/`ENCODE_OUT`) must be a PURE allocation
/// optimisation: (1) BYTE-IDENTICAL output across repeated encodes (a stale-state bug from an
/// incomplete `reset` would corrupt the 2nd+ encode), (2) encoding a SMALL value right after a LARGE
/// one must not leak the large value's data into the small one's document (the `reset` clears the
/// pools; retained capacity must not surface as content), (3) no node leak. Guards the reset+reuse
/// contract that the alloc-ceiling win rests on. Encodes the SAME value 3× (must be identical), then a
/// large value, then a small value again (must equal its first encoding — reused-but-cleared pools).

// ─── value-decode (idx 90) round-trip: value-decode ∘ value-encode ≅ id ────────────────────
// The acceptance bar (DESIGN-binary-ast-abi B0): for a value `v` of shape `desc`, decoding the
// canonical value-form document `value-encode` produces must reconstruct a value structurally equal to
// `v` (`value_eq_shaped`). Covers the shape spectrum the encode corpus exercises, run BACKWARDS.

/// Round-trip `v` (shape `desc`): `value-decode(value-encode(v)) ≅ v` via `value_eq_shaped`, and assert
/// no leak once both are dropped. `desc` is the descriptor byte-slice (`[table_len][shapes…][root]`).
pub(crate) fn assert_value_roundtrips(v: Handle, desc: &[u8]) {
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

/// A malformed descriptor whose Framed TYPE NODE nests absurdly deep DECLINES (`None`), it does not
/// overflow the stack. `decode_type_node` recurses per nesting level, and a level is only 2 bytes
/// (`[name_len=0][n_children=1]`), so before the `TYPE_NODE_DEPTH_CAP` a ~200 KB descriptor recursed
/// ~200 k deep and SIGABRT'd the guest — violating value-encode's "never a trap" totality (a
/// compiler-baked type node is always shallow, but the escape op must decline any input). The cap
/// makes it decline. A genuine type (`(Map Int (List Bool))`, depth 2) is far under the cap, unaffected.

/// A WIDE record (many DISTINCT field names) encodes byte-identically to the recursive oracle. This is
/// the shape whose `name_leaf` dedup was O(N²) (each distinct field name missed the linear scan and
/// walked all prior leaves — a 3200-field record took ~183 ms; after the `name_index` map it is O(N),
/// ~14 ms). Byte-identity here proves the map-based dedup produces the SAME leaf pool + indices as the
/// scan did (a repeated name still resolves to its first index). A moderate N keeps the test fast while
/// exercising the many-distinct-name path the small fixed-shape tests never reach.

/// A String nested inside a recursive sum encodes (the real use — a value form like an AST node with
/// an identifier, or a `List Str`). Descriptor: a Cons/Nil list whose element is a Str. Drives the
/// iterative walk through Sum → Tuple → Str and back via Ref, and checks byte-identity vs the oracle.

/// `value-encode` renders a Bytes payload (`Shape::Bytes`) as a `KIND_BYTES` leaf (kind 11, same
/// `write_bytes` framing as Str/Name). Previously `encode_value` DECLINED on `Shape::Bytes`, so a
/// recursive value carrying a Bytes field could not cross the host boundary. A Bytes value may be a
/// ROPE (concat/slice); the walk flattens it first (iterative, unobservable). Byte-exact + rope case.

/// A Bytes field nested inside a recursive sum encodes (a parse tree, a binary structure). Descriptor:
/// a Cons/Nil list whose element is Bytes; drives the iterative walk through Sum → Tuple → Bytes and
/// back via Ref, and checks byte-identity vs the recursive oracle (which flattens ropes identically).

/// `value-encode` renders a Float payload (`Shape::Float`) as a `KIND_FLOAT` leaf — the codec's exact
/// decimal (kind 6: negative(u8) + exponent(fixed 8-byte BE i64) + LEB siglen + big-endian magnitude).
/// The runtime f64 is converted to the decimal by a port of the compiler's `Decimal::from_f64`. A
/// NON-FINITE float declines (no exact-decimal form), matching `from_f64`.

/// `value-encode` renders a Float32 (`Shape::Float32`) as a `KIND_FLOAT` leaf carrying the f32's OWN
/// shortest decimal — the whole reason Float32 gets a 4-byte leaf instead of a promoted f64. The
/// headline case: `0.1f32` encodes as `1 × 10^-1` (decimal "0.1"), NOT the f64-promotion's
/// `10000000149011612 × 10^-17`. Also: `1.5f32` byte-exact; a non-finite f32 declines.

/// The decimal `float_leaf` produces must ROUND-TRIP back to the original f64 (it is the shortest
/// round-tripping form). Reconstruct the decimal STRING `[-]<digits>e<exp>` from the emitted
/// `(neg, exponent, magnitude)` and parse it with Rust's CORRECTLY-ROUNDED `str::parse::<f64>` (exact,
/// unlike lossy `sig * 10f64.powi(exp)`). Compare bit-for-bit across finite values incl. ±0.0/subnormal.

/// Decode a single-Float value-encode document back to the decimal string it denotes:
/// `[-]<significand>e<exponent>`, where the significand is the big-endian base-256 magnitude read as a
/// base-10 integer. Robust to a magnitude of ANY length (repeated ÷10 on a base-256 limb vector — no
/// u128 width assumption), so it works for a fuzzed value's full shortest decimal. Doc layout:
/// header(8)·leaf_count(1)·[KIND_FLOAT · neg(1) · exp(8 BE) · siglen(LEB) · mag] · struct… — the float
/// leaf is first, at offset 9: [9]=KIND, [10]=neg, [11..19]=exp, [19..]=siglen(LEB), then mag. The
/// siglen is a VARIABLE-length LEB (`doc_leb`), NOT a fixed byte — a full-expansion significand (a
/// whole float's exact decimal, e.g. f64::MAX = a 128-byte magnitude) has a multi-byte length, so read
/// the LEB and advance past it before the magnitude.
pub(crate) fn float_doc_to_decimal(doc: &[u8]) -> String {
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

/// FUZZ the Float32 encode (`float32_leaf`) over RANDOM f32 bit patterns — the companion to the f64
/// fuzz. `float32_leaf` shares `float_leaf_from_sci` but feeds it the f32's OWN shortest decimal
/// (`{f32:e}`, NOT a promoted f64 whose decimal differs), so the digit strings it converts are a
/// distinct population. Every finite f32, encoded via the real `op_value_encode_form`, must round-trip
/// bit-exactly through its KIND_FLOAT decimal parsed back AS AN f32; a non-finite f32 declines. No leak.

/// `op_box_float` normalizes every NaN — of ANY bit pattern — to the ONE canonical quiet NaN
/// (`f64::NAN.to_bits()`), so a float leaf has a single canonical byte form (deterministic-value-
/// form.md). Two NaN values that differ ONLY in their (unobservable) payload/sign bits must therefore
/// box to byte-IDENTICAL leaves and be equal under `champ_eq` / hash-identical under `champ_hash` —
/// otherwise they would be distinct map/set keys, violating the spec (every NaN equals every NaN). A
/// finite value keeps its bits, so `-0.0` stays DISTINCT from `0.0`.

/// NaN canonicalization composes through a COMPOUND: two tuples `(nan, x)` built from DIFFERENT NaN
/// bit patterns are `value-eq` equal and hash-identical (so they are the same map key), because each
/// NaN element canonicalizes to one byte form on `box-float`. This is the reachable path Float64-in-
/// compound (@ea74c89f) + NaN-canonicalization (@f25d7075) enable together — `value-eq` (op 61) IS the
/// language `=` on runtime compounds, so a struct/tuple carrying a NaN must compare structurally equal
/// regardless of the NaN's origin. A tuple with -0.0 vs one with 0.0 stays UNEQUAL (distinct forms).

/// `value-encode` renders a Set (`Shape::Set`) as `((. Set of) (list e1 … en))` with elements in
/// CANONICAL key-VALUE order — NOT the CHAMP hash order. The walk collects the elements + sorts by the
/// element's canonical scalar value (matching the compiler's `const_key_order`). Verifies the
/// structure + canonical INT order (numeric, not raw-byte) + differential vs the recursive oracle.

/// `value-encode` of a `Set String` — the EXACT shape the compiler-in-Cadenza port returns across the
/// host boundary (e.g. `free-vars.cdz`'s `Set String` of an AST's identifier Names). The int set-render
/// test above covers `value_cmp_shaped`'s numeric-Int arm; this covers its `Shape::Str` arm
/// (lexicographic BYTE order over the flattened leaf) driving `set_elements_canonical`'s sort. A String
/// element takes the arity-0 heap-byte-leaf champ path (distinct from an immediate int), and the
/// render must be lexicographic — NOT the CHAMP hash order the set stores/iterates in. Verifies the
/// canonical order (incl. the empty string sorting first + a shared "foo"/"foobar" prefix) + the
/// iterative-vs-recursive-oracle byte-identity + no leak.

/// `value-encode` renders a Map (`Shape::Map`) as `(map (k1 v1) … (kn vn))` with entries in CANONICAL
/// KEY order — NOT the CHAMP hash order. The walk collects (key,value) pairs + sorts by the key's
/// canonical scalar value (matching `const_key_order`). Verifies the structure + canonical INT-key
/// order (numeric, not raw-byte) + differential vs the recursive oracle.

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

/// value-encode of a NESTED-COLLECTION value: a `Map Int (List Int)` — the map VALUE is itself a
/// collection walked recursively (`Shape::Map`'s `val` shape can be any encodable shape, not just a
/// scalar; the arm's comment says so but every other map/set test uses a scalar value). This is the
/// shape the compiler's "sum + nested-collection compound results" work now escapes via value-encode,
/// so the recursive value-walk (map → each entry's value → List → vec elements) must be exercised. Assert
/// byte-identity to the recursive oracle (which mirrors the nested walk) + entries in canonical KEY
/// order with each value list intact.

/// value-encode of EMPTY collections — the zero-element assembler edge (`SetOf`/`MapOf`/`List` with
/// 0 children, `list_head_tail` with an empty tail, the `checked_sub(0)` in the assemblers). An empty
/// collection returned to the host is common; a zero-element bug (underflow, dropped head, wrong form)
/// would be a silent miscompile. Empty set → `((. Set of) (list))`, empty map → `(map)`, empty list
/// → `(list)`. Verified byte-identical to the recursive oracle + the concrete forms.

/// `value-encode` renders a MULTI-payload recursive variant via `Shape::Spread` (descriptor tag 16):
/// the payload elements are spliced FLAT under the variant head — `(Node 1 l r)`, NOT the
/// tuple-wrapped `(Node (tuple 1 l r))` (landed @75fe7e80). That production Sum→Spread walk arm had NO
/// dedicated value-encode test (only the differential oracle arm — the same gap as Framed). A splice
/// bug (tuple-wrapping, wrong element order, wrong arity) would be a silent miscompile on a common
/// recursive shape (a tree). Verifies iterative==recursive byte-identity + the FLAT rendering.

/// `value-encode` renders a `Shape::Framed` (descriptor tag 15) as the `(: value (head arg…))`
/// parametric-type frame — the shape a RUNTIME `List` result escapes as `(: (list …) (List <elem>))`
/// (landed @72d5d80a). That production walk arm had NO dedicated value-encode test (only the
/// differential oracle arm); an encoding bug (wrong tag, arg order, or frame nesting) would be a
/// silent miscompile on a real escape path. Verifies iterative==recursive byte-identity AND the
/// concrete rendered structure.

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

pub(crate) fn alloc_calls() -> u64 {
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

/// CPU-scaling PROBE (diagnostic, not a regression gate): times set ∩/∖ at growing N to reveal
/// whether they are linear-ish or super-linear (the alloc bench can't see the O(log) contains-probe
/// factor — evidence for whether the O(min) node-merge redesign is worth a future tick). Also times
/// UNION over COMPOUND (tuple) elements, where hashing an element walks its whole subtree — this is

/// CPU-scaling PROBE (diagnostic, not a gate) for the STRING-KEYED map shape (JSON-object /
/// dictionary): keys are multi-byte heap strings, so every insert/lookup pays a byte-serial FNV
/// over the whole key plus a byte compare on a slot hit. Times build + lookup at growing key
/// LENGTH to reveal whether the cost is dominated by the FNV walk (scales with key bytes) or the
/// trie descent (scales with map size). Run under `perf` to attribute the hot region. Never
/// profiled before — the existing probes all use int/tuple/nested keys.

/// CPU-scaling PROBE (diagnostic, not a gate) for the SHARED/PERSISTENT vec copy path — the
/// functional-update pattern (keep the base version, derive a new one), the largest realistic
/// allocator (vec_push_shared/vec_update_shared ~7000 allocs/1000). Each op path-copies the touched
/// RRB spine (root→leaf) via `vec_node_replace`/`vec_node_append`, `op_dup`ing every off-path
/// sibling. Times shared push + shared update at growing N to reveal the copy-path hot region under
/// `perf` (the alloc bench sees the count but not where the CPU goes). Never profiled before.

/// CPU-scaling PROBE (diagnostic, not a gate) for the SHARED/PERSISTENT CHAMP map copy path — the
/// functional-update pattern on a map (keep the base version, derive a new one). This is the second-
/// largest realistic allocator (map_insert_shared 6143, map_remove_shared 6685 allocs/1000); the
/// alloc bench tracks the COUNT, this times where the CPU goes under `perf`. Each op path-copies the
/// touched spine root→leaf via `champ_insert_node`/`champ_remove_node` (clone-once-and-mutate,
/// dup every off-path sibling). Complements `shared_vec_copy_path_cpu_scaling_probe`; the map copy
/// path was never dedicated-CPU-profiled.

/// The STATIC shape descriptor the compiler holds at each use site. There is no runtime type
/// tag, so the renderer is driven ENTIRELY by this compile-time knowledge: the SAME heap node
/// renders differently under different shapes (an `Arr[3,1]` is `(tuple 3 1)` under `Tuple` and
/// `(list 3 1)` under `List`). This mirrors, in plain Rust, the type-directed renderer the
/// compiler bakes into the emitted program.
pub(crate) enum Shape {
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
pub(crate) fn escape_byte(b: u32, out: &mut String) {
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

pub(crate) fn render(handle: Handle, shape: &Shape) -> String {
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
pub(crate) fn rc_of(h: Handle) -> u32 {
    if is_immediate(h) {
        return 2;
    }
    unsafe { (*h.0).rc }
}

/// Test-only: is the node's raw payload HEAP-backed (spilled) rather than inline? Used to assert the
/// reuse constructors normalize a reused shell's raw back to inline (a fresh constructor's rep).
pub(crate) fn raw_is_heap(h: Handle) -> bool {
    if is_immediate(h) {
        return false;
    }
    matches!(unsafe { &(*h.0).raw }, Raw::Heap(_))
}

/// Test-only: is the node's handle vector HEAP-backed (spilled past the inline cap) rather than
/// inline? The handles-arm twin of `raw_is_heap`: used to assert the reuse constructors normalize a
/// reused shell's HANDLES back to inline for a ≤`INLINE_HANDLES_CAP`-child node, matching a fresh
/// constructor's rep (a wide reset token keeps a `Handles::Heap` unless the refit re-inlines it).
pub(crate) fn handles_is_heap(h: Handle) -> bool {
    if is_immediate(h) {
        return false;
    }
    matches!(unsafe { &(*h.0).handles }, Handles::Heap(_))
}

/// A DEFINITELY-BOXED int leaf (test-only): bypasses `op_box_int`'s P2 normalize so the RC /
/// reuse / cascade tests keep exercising a real heap Node with rc == 1 (a small `op_box_int(v)`
/// now inlines and would make those node-count / drop-a-leaf scenarios vacuous). Byte-identical
/// to the pre-P2 boxed representation, so `op_get_int` decodes the same value through `with_node`.
pub(crate) fn boxed_int_leaf(v: i64) -> Handle {
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

/// `read_u32_at` has two paths — a fast in-bounds 4-byte window read and a zero-padded fallback for
/// a short/absent raw. This locks their EQUIVALENCE at the boundary: both must yield the same
/// little-endian value where 4 bytes exist, and zero-pad missing high bytes. It is the hottest
/// header read (`champ_datamap`/`nodemap`/`size`, every descent level), so a boundary mistake would
/// silently corrupt bitmaps/sizes; a reference recompute over every offset of several raw lengths
/// (including 0/1/2/3-byte short raws that exercise ONLY the fallback) pins the contract.

// ── Inline unit + bool: SHARED-REPRESENTATION payoff (P1b flips the producers) ────────

// ── Inline small ints: the fixnum window (P2 flips op_box_int) ────────────────────────

/// `bigint-div` TRUNCATES toward zero across the FULL sign matrix, and `bigint-cmp` orders negatives
/// correctly — the ops the compiler now emits for a runtime `/` and comparison (B3b `@acb1768f`). The
/// op-level glue test only checked `7/2` and `-7/2`; the other two sign combos + the truncation
/// DIRECTION (toward zero, NOT floor: `-7/2` is `-3` not `-4`) + negative cmp ordering were unpinned
/// at the op level (the library `divmod` is differential-tested, but a box/unbox sign-flag bug would
/// slip past that). A wrong truncation direction is a silent wrong answer for runtime negative BigInt
/// division.

/// `bigint-of-i64` boxes DIRECTLY via the i128 path (`box_bigint_i128`, no transient `Big`) — the leaf
/// MUST stay byte-identical to the old `box_bigint(&Big::from_i64(v))` route (both emit the canonical
/// `[sign][LE magnitude, trailing-zeros-stripped]` form). Pins that equivalence across the full i64
/// range — the endpoints (`i64::MIN`, whose `unsigned_abs` is a limb-boundary case), the i32 boundaries,
/// exactly 2^32 (single→double limb in `Big::from_i64`), and zero — so a future refactor of EITHER path
/// can't silently diverge (a divergent leaf would break BigInt map-key equality + narrowing). Also
/// checks the value round-trips through `bigint-to-i64-checked`.

/// `bigint-rem` (op 73, the `%` the compiler now emits for a runtime BigInt) — the remainder of
/// TRUNCATING division, so its sign is the DIVIDEND's, matching Rust `%` on i64 across the full sign
/// matrix. Backed by the same `divmod` as `bigint-div` (the `r` half), so `a == (a/b)*b + (a%b)`.

/// `bigint-div` by ZERO TRAPS (numeric-model.md — an unbounded range gives `n/0` no value). The
/// zero-divisor trap was covered only implicitly (via the `Big::divmod` differential returning `None`);
/// no test asserted the OP itself traps. This matters especially since `op_bigint_div` gained an i128
/// FAST PATH (`spec@9bcfb04e`): a zero divisor makes `checked_div` return `None`, so it falls THROUGH
/// to the `Big` path — which traps. This pins that the fast path does NOT swallow the trap (return a
/// bogus value); a regression that mis-handled `y==0` in the fast path would return instead of panic.

/// `bigint-rem` by ZERO TRAPS (the `%` companion of div — same `divmod`-`None` origin, same i128
/// fast-path fall-through). Pins the op-level trap for the remainder path independently of div.

/// `rational-of` with a ZERO DENOMINATOR TRAPS (a rational `n/0` has no value — the rational analogue
/// of ÷0). The trap fires BEFORE normalization (`op_rational_of` checks `den.is_zero()` after reading
/// both operands). Pins the construction-time trap the port hits building a Rational from computed
/// components.

/// DIFFERENTIAL FUZZ for the runtime Rational ops (R3a, ops 74-81) — the safety net the bigint ops have
/// (`differential_arithmetic_vs_num_bigint`, 5000 pairs) but Rational did NOT: the sibling's landing test
/// is FIXED inputs only, and the arithmetic's subtle logic (sign placement, gcd reduction, cross-multiply
/// add/sub/mul/div, cmp direction) is exactly where random inputs catch a bug a spot-check misses. Cross-
/// check every op against a self-contained `i128`-fraction reference (exact at fuzzer scale — small
/// operands; no new dep). The reference reduces + normalizes the SAME way the runtime must (gcd to lowest
/// terms, sign on the numerator, denominator strictly positive), so the runtime's `(num, den)` output
/// must equal it byte-for-byte — which also pins the map-key canonicalization (equal value → identical
/// normalized form). Denominators are forced nonzero (the zero-denom TRAP is covered by the fixed test).

/// A BigInt is a RAW-ONLY leaf compared by its `raw` bytes (`champ_eq`) and hashed over them
/// (`champ_hash`) — exactly like Bytes/String. So two BigInts that are EQUAL BY VALUE but reached by
/// DIFFERENT arithmetic MUST produce byte-IDENTICAL leaves, else they'd be distinct map/set keys and
/// `=` would wrongly return false. This holds only if every op returns a NORMALIZED `Big` (no trailing
/// zero limbs, no `-0`) and `to_sign_magnitude_bytes` is canonical. `bigint.rs` differential-tests
/// VALUES vs num-bigint but NOT this heap-leaf byte form — the property BigInt-as-map-key depends on.

/// `bigint-to-i64-checked` traps EXACTLY at the i64 range boundary: `i64::MAX`/`MIN` fit, one beyond
/// each traps. The op-glue test skipped the trap path ("a compiler/gate concern"), but the boundary is
/// the whole point of the CHECKED narrow (`Int64.of` an out-of-range BigInt must trap, not wrap). Build
/// `i64::MAX + 1` = `bigint-add(of(MAX), of(1))` and assert it panics (→ a wasm trap under abort).

/// The `op_bigint_*` glue on genuinely LARGE, MULTI-LIMB (>i64) values. Every other bigint test enters
/// via `op_bigint_of_i64` (≤64-bit), so the box/unbox of a multi-limb magnitude — several 4-byte limbs,
/// trailing-zero stripping ACROSS limb boundaries, the sign byte — and arithmetic PRODUCING/CONSUMING
/// >i64 values were untested through the heap. `bigint.rs` differential-tests the limb arithmetic vs
/// num-bigint, but NOT the heap round-trip. Build large `Big`s directly, box them, run the WIT ops, and
/// check the result unboxes to the value the library computes — pinning that the leaf byte codec and
/// each op thread multi-limb operands correctly. (B3b will emit exactly these >i64 BigInts.)

/// The `i128` arithmetic FAST PATH (add/sub/mul when both operands fit i128 and the result doesn't
/// overflow) must produce a leaf BYTE-IDENTICAL to the full `Big` SLOW path — the fast path is a pure
/// allocation optimisation, not a semantics change. Drives values that straddle the i128 boundary in
/// both directions: (a) both fit + result fits → fast path, result must `champ_eq` a freshly-boxed
/// `Big` result; (b) result OVERFLOWS i128 (e.g. `i128::MAX + 1`, `i128::MIN - 1`, `i128::MAX *
/// i128::MAX`) → falls back to the `Big` path, must still be canonical; (c) an OPERAND exceeds i128 →
/// fast path declined, `Big` path used. Also pins the `i128`↔bytes helpers via the op results. Guards
/// the fast/slow agreement the `num-bigint` differential (which goes through the ops) also protects,
/// but SPECIFICALLY at the overflow endpoints a random differential rarely hits exactly.

/// `bigint-div`/`-rem` i128 FAST PATH ↔ `Big` slow path at the boundary — the div/rem analogue of the
/// add/sub/mul boundary test above (they got the i128 fast path in a separate increment). The fast path
/// uses Rust's `checked_div`/`checked_rem` (truncate-toward-zero quotient, dividend-sign remainder —
/// EXACTLY `divmod`'s semantics), so the result leaf MUST be byte-identical (`champ_eq`) to boxing the
/// `Big`-`divmod` answer, across: (a) in-range operands + all four sign combos (fast path); (b) an
/// operand exceeding i128 → the fast path declines → `Big` runs; (c) the `i128::MIN / -1` OVERFLOW —
/// `checked_div`/`checked_rem` return `None`, so this MUST fall through to `Big` (the one non-zero case
/// where the native op can't represent the answer). A regression that dropped the `checked_*` guard
/// would panic here (overflow) instead of falling back.

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

/// The `Shape::BigInt` value-encode arm (B3c, descriptor tag 17): a boxed runtime BigInt escapes via
/// `op_value_encode_form`, reading the value with `unbox_bigint` (arbitrary width, NOT i64-capped) and
/// rendering the SAME `KIND_INT` leaf as a fixed-width Int. Cover an i64-fitting value, a >i64 value
/// (i64::MAX² ≈ 2^126, the whole point), a negative, and zero — byte-identical to the recursive oracle
/// each time (the oracle's S::BigInt arm mirrors production), plus the exact KIND_INT sign+magnitude
/// for the >i64 case (proving the leaf is not i64-bounded).

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

/// `value_cmp_shaped` — the descriptor-guided three-way BLESSED order (heap-ordering slice 2's runtime
/// core, still UNEXPORTED so hash-neutral). Covers the ordering rules v-inference blessed: Int by
/// NUMERIC value (incl. the negative case raw-byte order gets wrong), tuple lexicographic by field,
/// list lexicographic with a proper prefix LESS than its extension, sum by discriminant, consistency
/// with equality (Equal iff champ_eq), and the non-orderable declines (Float leaf → None).

/// `value_cmp_shaped` hardening: SUM by discriminant-then-payload, RECORD by field order, and a DEEPLY
/// nested list (the iterative-walk / wasm-safety claim — must not overflow the native stack).

/// `value_eq_shaped` (the equality companion of `value_cmp_shaped`): handles the leaves value-cmp
/// DECLINES for ordering — a `List<Float>` compares element-wise by CANONICAL BYTE FORM (§313 float eq
/// total), which value-cmp can't (it declines the float leaf) and champ_eq can't (unsound for the
/// non-shape-canonical RRB spine). Pins: concat-vs-push List<Float> equal; a differing float → not equal;
/// NaN == NaN (canonicalized); -0.0 ≠ +0.0; a deep list-of-float nesting is stack-safe.

/// REGRESSION ISOLATION for the Bytes-total-order slice: a `List<Bytes>` whose element is a runtime SLICE
/// VIEW must compare equal to its flat twin under `value_cmp_shaped` (op=Eq path), just as `value_eq_shaped`
/// already did — the per-leaf Bytes arm must flatten the view (`bytes_flatten`) before comparing `raw`.

/// THE list-key miscompile fix (`value_canonicalize_shaped`): a Map with a CONCAT-built list KEY must be
/// found by a PUSH-built equal key AFTER canonicalizing both keys, at sizes straddling the leaf/multi-
/// level boundary (n≤32 already collapsed; n≥33 was the false-miss). Also nested (`(tuple (list) Int)`),
/// and a genuinely-different list must still MISS. Leak-clean: canonicalize BORROWS its input and returns
/// a fresh owned key; dropping the map + the two fresh canonical keys per size must net to 0 live cells.

/// `value_canonicalize_shaped` is ITERATIVE (wasm-safe): a 200-deep nested list canonicalizes without
/// overflowing the native stack, and the result reads back to the same innermost leaf.

// NOTE (serializer / value-interchange): the runtime crate has NO value-interchange / Ast
// serialization path that reads `node.raw` — the only value-observing surfaces are `render`
// (covered above: inline and boxed ints render identically) and the `to_u32`/`from_u32` ABI
// (identity casts, covered by the ABI round-trip tests). Ast encode/decode lives in
// `cdz-compiler/src/ast.rs` over the compiler's syntax `Node`, never a runtime `Handle`, so there
// is nothing serialization-shaped to test from here. Flagged as a cross-boundary review item.

// ── Latent-hardening (review follow-ups): reuse-to-0 normalize + defensive guard set ──

// ── Scalars ─────────────────────────────────────────────────────────────────────────

// ── Arr (tuple / record / list) ───────────────────────────────────────────────────────

// ── Sum ───────────────────────────────────────────────────────────────────────────────

// ── Bytes ───────────────────────────────────────────────────────────────────────────────

/// `op_bytes_alloc` builds a ≤INLINE_RAW_CAP-byte buffer with an INLINE raw (no transient `vec![0;
/// len]`) and a longer one on the heap. Guards the two paths agree on value AND representation: a
/// small leaf's raw must be inline (the perf win) while still set/get/len-ing identically to a large
/// heap leaf, and both must render + compare (champ_eq) the same as the other rep would. (Rep
/// divergence behind Raw's Deref is invisible to a value-only check — iter-29's lens — hence the
/// explicit raw_is_heap assertions.)

// ── String ──────────────────────────────────────────────────────────────────────────────

/// `str-get` (op 18) on a ROPE String must return the logical CONTENT, not the rope node's header
/// bytes. A runtime String IS a bytes rope (`String.concat`/`.at`-slice build concat/slice nodes,
/// sharing the Bytes representation, `@b77b3ae0`), so a concat/slice String reaching `str-get` is NOT
/// a flat leaf — before the `bytes_flatten` fix, `op_str_get` read the concat node's `raw=[len]` (4
/// bytes) as UTF-8 and returned garbage ("\u{7}\0\0\0" for a 7-byte rope). This is the SAME latent bug
/// the value-encode `Shape::Str` arm was hardened against; `str-get` had no emit site yet (the compiler
/// returns a String via the value-encode escape, not `str-get`), so it was unreached — but wiring a
/// direct String return would have silently corrupted every rope. Build a rope with a multi-byte scalar
/// spanning a seam and assert it reads back byte-for-byte equal to the flat twin.

/// `op_str_from_bytes` — the READY-BUT-UNEXPORTED load-bearing half of the coordinated `str-from-bytes`
/// op (a total UTF-8 decode `Bytes → (Option String)`; the compiler-in-Cadenza port's decode/encode
/// string content is blocked on it — `String.from-bytes` on a runtime Bytes declines at lower.rs). Pins
/// the contract so the compiler's eventual `Core::StrFromBytes` emit calls a PROVEN fn: (1) valid UTF-8
/// → the buffer AS a String, byte-identical to `op_str_new` (a String IS a byte leaf); (2) a ROPE input
/// flattens first (the runtime-built-Bytes shape — `Bytes.concat`); (3) strict rejection of invalid
/// bytes, an overlong encoding, AND a surrogate (the three spec failure modes) → NULL; (4) empty → valid
/// ""; (5) no leak (consumes `buf`; a valid result is dropped, an invalid one already released).

/// `op_bytes_scalar_at(buf, i)` — the codepoint of the i-th UNICODE SCALAR, or `u32::MAX` out of range.
/// The op a real text lexer wants: a `Char` codepoint (an immediate integer, compared by a plain
/// `i32.eq`), sidestepping the `String.at` slice-rope content-eq hazard the compiler-in-Cadenza lexer
/// works around. Covers: (1) ASCII by-scalar read; (2) MULTI-BYTE where the SCALAR index ≠ the BYTE
/// index (`"café"` byte-len 5, scalar 3 = 'é' = 233); (3) a 4-byte scalar (emoji U+1F600); (4) a ROPE
/// input (flatten across the concat seam); (5) out-of-range + empty/immediate → the `u32::MAX` sentinel;
/// (6) it BORROWS (no consume — the buffer survives + reads again, node count balances).

// ── Map ─────────────────────────────────────────────────────────────────────────────────

// ── Compound-of-compound: a record containing a list, a sum, bytes, and a string ──────────

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

// ── Birth refcount: every node is born with refcount 1 ────────────────────────────────────

// ── Tagless totality: scalar/null reads are total; OOB into a valid node traps ────────────

// ── Perceus reference counting ────────────────────────────────────────────────────────────

/// Current count of live (allocated, not-yet-freed) nodes on this test thread. Tests measure
/// DELTAS against a baseline captured at their start.
pub(crate) fn live_nodes() -> i64 {
    LIVE_NODES.with(|n| n.get())
}

/// A DAG within ONE value — a single root that reaches the SAME shared child via TWO distinct PATHS
/// (the hash-consing / structural-sharing shape a CSE pass produces: `9a35fbac`). The prior test shares
/// a child under two SEPARATE root handles dropped one at a time; this shares it inside ONE value and
/// drops that ONE root in a single `op_drop`, so the free cascade VISITS the shared child TWICE —
/// exercising the "shared (rc>1) → decrement, DON'T recurse" arm on the first visit and the "unique
/// (rc==1) → recurse + free" arm on the second (lib.rs `n.rc > 1` at ~3260). A cascade that freed on
/// the first visit would UAF the second path; one that never decremented would leak. Shape:
/// `root = tuple(inner, tuple(inner, 9))` with `inner = tuple(7)` shared (rc==2, one ref per path).

// ── RC calling convention: the emitted-sequence mirror ────────────────────────────────────
// Each test SIMULATES the exact dup/drop sequence the compiler must emit for a pattern and
// asserts, via LIVE_NODES, both properties the convention
// guarantees: NO LEAK (heap returns to baseline) and NO EARLY FREE (kept values stay intact
// until their last owner). These are the reference behaviors the compiler's emission reproduces;
// a failing test would mean the primitives cannot support the prescribed convention.

/// §3.5 / §4 — projection kept past the parent: `(let t (tuple a b) (arr-get t 0))`. The
/// element is RETURNED, so it must be dup'd BEFORE the parent is dropped; then dropping the
/// tuple frees the tuple node + the not-kept sibling, leaving the kept element valid.

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

/// The MIRROR of the projection-escape above, and the runtime contract the compiler's fix for the
/// mutual-recursion still-live-binding miscompile RELIES ON (`spec@6db817a3`: an `fc↔fl` walk consumes
/// a node's shared child payload while a sibling operand still reads the parent — the idiomatic
/// homoiconic-Ast resolver shape; fix = "dup the aggregate/collection operand of a (mutual-)recursive
/// call whose callee consumes a payload a sibling still reads"). Here we KEEP the parent and DUP-then-
/// fully-CONSUME a payload reference (owner A = the consuming recursive walk), asserting owner B (the
/// parent + a SIBLING read of it) is UNCORRUPTED. The existing projection test drops the PARENT and
/// keeps the child; this keeps the parent and consumes a dup'd payload ref — the shape the resolver hits.

/// §3.5 — `match Some(x) => x`: dup the borrowed payload, then drop the scrutinee. Payload
/// survives; the sum node is reclaimed.

/// §3.5 (no-keep arm) — `match Some(_) => 0`: the payload is NOT kept, so no dup; dropping the
/// scrutinee reclaims the whole sum INCLUDING the payload.

/// §3.3 — the duplicate-binder question, answered: `(tuple x x)` is a `dup`, not an error. The
/// tuple owns TWO references to the same child; dropping the tuple reclaims the child exactly
/// once (rc 2->1->0 across the two owned slots).

/// §3.4 — branch balancing. `(if c xs ys)` returns one of two owned lists, both live at the
/// `if`; each arm drops the not-returned one. Correct for BOTH values of `c`: no leak, no
/// double-free either way.

/// §3.1 — a bound-but-unused heap value (`(let x (tuple …) 0)`) is dropped at scope end;
/// baseline restored.

// ── Reuse / FBIP ───────────────────────────────────────────────────────────────────────────
// `reset` + the `*-reuse` constructors give in-place update on unique data. The tests assert
// the two load-bearing properties: (1) reuse is IN PLACE — the rebuilt node is the SAME
// allocation (address identity + zero net LIVE_NODES growth), the whole point over free→malloc;
// (2) reuse is FRAME-LIMITED — it fires ONLY on a unique node, so a shared value (a persistent
// structure's other version) is never clobbered and peak heap cannot grow.

/// `reset` on a UNIQUE node yields its shell as a non-null token, drops its owned children, and
/// keeps exactly one node live (the emptied shell) — ready to be refit.

/// `reset` on a SHARED node declines: it returns NULL, decrements, and leaves the node (and its
/// children) fully intact for the other owner. This is the frame-limiting guard — a persistent
/// structure's shared version is never reused out from under it.

/// A null token makes the reuse constructors behave EXACTLY as their plain forms (fresh alloc),
/// so a declined `reset` is transparent to the emitted rebuild code.

/// `arr-alloc-reuse` with a real token refits the SAME shell — address identity, no new node.

/// A reuse TOKEN whose shell came from a node with a HEAP-backed raw (a bytes/string leaf longer
/// than the inline cap) must NOT leave the reused node carrying that heap raw: `op_sum_new_reuse`
/// and `op_arr_alloc_reuse` normalize the raw back to INLINE, matching what the fresh constructors
/// produce. (Regression guard: the old `raw.clear()` + `extend_from_slice` kept a heap buffer — a
/// stray retained allocation AND a non-canonical storage rep for one logical value; the value stayed
/// byte-equal via Deref so hash/eq tests could NOT have caught it, hence this explicit rep check.)

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

/// `sum-new-reuse` with a token repurposes the SAME shell as the new `(disc, payload)` node.

/// The headline FBIP property: mapping a function over a UNIQUE list rebuilds it with ZERO net
/// allocation. Emitted per element: dup the elements to keep → reset the old cons/array shell →
/// arr-alloc-reuse it → refill. Peak heap never exceeds the input's node count + the transient
/// working set; the rebuilt list occupies the SAME shells as the input.

/// The ordering invariant for reset (the §4 dup-before-drop rule): a child of the old node that
/// the rebuild KEEPS must be dup'd BEFORE `reset`, because reset drops the old node's child
/// references. With the dup, the kept child survives into the reused shell.

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

/// `vec-drop(v, index)` (op 72, the `(list p… .. rest)` REST-pattern binder) returns the TAIL
/// `[index, len)` as one vector and RECLAIMS the dropped prefix `[0, index)`. It's `vec-split` keeping
/// only the right half — a single-u32 result, CONSUMING `v`. Landed `@494d2e44` with no runtime test.
/// This mirrors the wit wrapper's EXACT body (`op_vec_drop_tail`, the build-only-the-tail path) and
/// guards: correct tail content across offsets, the `index==0` (whole) + `index>=len` (empty) edges,
/// the result's RRB invariants, and — since it's consuming and reclaims the prefix — NO LEAK.
fn vec_drop_impl(v: Handle, index: u32) -> Handle {
    op_vec_drop_tail(v, index)
}

/// `op_vec_drop_tail` (build-only-the-tail) must be BYTE-IDENTICAL to the old `split`+drop-left it
/// replaced — same tail content AND same canonical RRB shape (`champ_eq`), just ~half the allocation
/// (no discarded left prefix). Differential across offsets on BOTH a strict (push-built) and a RELAXED
/// (concat-built) vector — the relaxed case is where the boundary-node rebuild + size-table recompute
/// must match. Also covers a 3-level vector (full-depth descent) and the whole/empty edges.

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
pub(crate) fn vec_of_bools(bs: &[bool]) -> Handle {
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

/// `mark-immortal` (index 95): converting a build-once static heap node makes it CENSUS-EXCLUDED (the
/// live-objects count nets to zero — an immortal held by a module global is not a leak) and makes
/// `dup`/`drop` NO-OPS on it (the global owns it for the whole instance; a consumer's `global.get` +
/// harmless no-op drop reads it intact). Deltas from a captured baseline (`reset` is a no-op here).

/// The EMBEDDING sharp-edge (v-static-data increment 6): a hoisted IMMORTAL constant embedded as a CHILD
/// of a RUNTIME compound survives when that runtime parent is recursively dropped — `op_drop`'s cascade
/// NO-OPS on the immortal child (never decrementing/freeing it), so a `(tuple <static> 42)` built at
/// runtime and dropped leaves the static intact + readable + census-neutral (no UAF).

/// DEEP mark-immortal (op 96, v-static-data large-list/map build-once hoist): `op_mark_immortal_deep`
/// marks a MULTI-NODE structure AND its payloads immortal transitively — the RRB list's interior/leaf
/// nodes AND its element handles, the CHAMP map's interior nodes AND its `[k,v]` payload handles — so a
/// build-once constant list(>32)/map nets to ZERO census (no leak) and every node dup/drop-no-ops (no
/// UAF under a runtime consumer). The crux (v-static-data): the ELEMENTS/KEYS/VALUES, not just the
/// spine, must be marked — asserted via `node_rc` on a read-back element/value.

/// DEEP mark-immortal (op 96) is DAG-SAFE: persistent structures SHARE nodes, so a node reachable via
/// two paths must be marked EXACTLY ONCE — a re-visit is skipped (rc already IMMORTAL), decrementing
/// the census once per DISTINCT node, never twice. This pins the no-double-census-decrement invariant
/// v-core-opt's large-list/map build-once hoist relies on: a deep-mark that re-decremented a shared
/// node would push the census BELOW `base` (caught here), and a shared-live node marked/freed on the
/// wrong path would corrupt the other owner. The `== base` assert IS the DAG check.

/// A MULTI-NODE deep-immortal value nested under a MORTAL shell survives the shell's drop: `op_drop`'s
/// free cascade SKIPS an IMMORTAL child WITHOUT recursing into or decrementing its subtree, so dropping
/// the mortal parent frees ONLY the parent — the whole immortal list (spine + leaves + elements) is
/// untouched. This pins the double-reclaim-safety v-core-opt relies on when a hoisted deep-immortal
/// static is embedded in an ordinary refcounted value it reclaims.

/// The DEBUG-build USE-AFTER-FREE detector (operator safety net for the leak-reclaim work: "UAF is
/// much worse than leaks"). The free path bumps a dedicated `generation` field ODD (= freed) and
/// retains the cell — kept SEPARATE from `rc` so the refcount stays pure — so a DOUBLE-DROP is caught
/// as a loud panic instead of corrupting the heap. This is exactly the failure an UNSOUND reclaim drop
/// produces (a value dropped while another owner is live → that owner's later drop is a double-free) —
/// now a red run, not a shipped silent bug.

/// Dup-after-free (retaining a freed cell — the other half of an unsound reclaim) is caught too; the
/// guard precedes the rc bump so the poison is never silently incremented.

/// Read-after-free through the central reader (`node_rc` → `with_node`) is caught — so a freed value
/// consumed by any accessor surfaces the UAF rather than reading poisoned/garbage bytes.

/// Read-after-free through a DIRECT INDEX GETTER (`op_arr_get` → `Handle::node_ref`, which BYPASSES
/// the `with_node` / `with_raw_arity` chokepoints) is now caught too. This is the access-site-coverage
/// win: the guard moved from the two chokepoints onto every direct node deref, so a freed container
/// consumed by a getter traps at the getter instead of reading poisoned/garbage bytes. (Before the
/// `node_ref` refactor this read would have dereffed the freed cell unguarded.)

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

/// The empty-vec is a SHARED IMMORTAL SINGLETON (the `IMM_UNIT` analog for lists): `op_vec_empty`
/// returns the SAME node every call, it is census-EXCLUDED (never a leak — the mixed-recursive
/// List-fold terminal fix), and a `vec-push` on it takes the persistent COPY path (rc = IMMORTAL != 1)
/// so the singleton is NEVER mutated in place. This is the soundness control for the immortal-empty-vec
/// fix (no shared-singleton corruption).

/// The empty MAP / SET / BYTES / STRING constructors return shared IMMORTAL singletons — the empty-vec
/// / IMM_UNIT generalization (operator directive: an empty value should allocate once, immortal,
/// reused). Same handle every call, census-EXCLUDED (never a leak), and an insert/build path-copies
/// off them (rc=IMMORTAL != 1) leaving the singleton empty (no in-place mutation of the shared empty).

/// `op_vec_prepend` builds a correct multi-level RRB AND reclaims each intermediate version — the
/// dedicated front-growth op that replaces `concat(singleton, v)` (which leaked ~17 cells/prepend).
/// Mirrors the 05:2521 build loop (out = prepend(out, i)): the result is [n-1, …, 1, 0]. The
/// post-drop census == base is the leak witness — if intermediate versions leaked, dropping the final
/// list would leave them live (unreachable from `v`), so census > base.

/// `hash-blake3` (heap index 91) is BYTE-IDENTICAL to `blake3::hash` of the same input — for a flat
/// leaf, a ROPE (which must flatten first), and the empty input. This pins the RUNTIME half of the
/// design's §9 byte-identity invariant (DESIGN-compiler-primitives.md): the compile-time `Blake3.of`
/// fold calls the SAME `blake3::hash`, so op==crate here means both halves agree bit-for-bit. Also
/// verifies the op BORROWS its input (the caller can still drop the input afterwards, and every handle
/// returns to the live-node baseline — the op consumes nothing).

/// `ast-print` (heap op 92) renders a runtime Ast heap value to canonical re-readable s-expr text,
/// byte-identical to the compiler's `print_ast_value`. Builds Asts directly (`sum-new` at chosen discs
/// + payloads) and asserts the text; the disc→variant map is read from the baked `discs` Bytes (here
/// `[int,float,bool,str,name,bytes,list] = [0..=6]`). Covers the List recursion + Int/Name/Bool/Str.

/// `ast-encode` self-consistency: a heap `Ast` walked by `op_ast_encode` produces the SAME canonical
/// `cdzast` bytes as building the equivalent `Arenas` directly through the shared `Builder` +
/// `codec::encode`. This validates the heap-walk's disc dispatch + type bridges (Big→IntValue, char,
/// float→Decimal, RRB vec elements) — the byte-identity contract with the compile-time `Ast.encode` fold,
/// which runs that same `Builder`/`codec` path. Every input is built with the constructors the compiler
/// emits (bigint leaf, RRB `vec-push` for lists, boxed scalar for char) per the #3621 test-fidelity rule.

/// `ast-decode` round-trips `ast-encode`: `encode(decode(encode(v))) == encode(v)` over an Ast spanning
/// every variant. Encode is byte-canonical, so equal re-encoded bytes prove `decode` rebuilt the SAME
/// Ast (structure + every leaf value, through both type bridges). Also checks a malformed byte sequence
/// decodes to NULL (the `Err` path), never a trap.

/// M2 (OPTION B) `ast-encode`/`ast-decode` over the 7 first-class compound-ctor reflected forms
/// (ListCtor/TupleCtor/RecordCtor/MapCtor/SetCtor + FieldPair/Member, discs 9–15). Two contracts:
/// (1) the runtime op93 encode of a reflected ctor value is byte-identical to the shared cadenza-ast
/// `Builder`+`codec::encode` path — the compile-time `Ast.encode` fold's form, via the SAME
/// `compound`/`field_pair`/`member` emit primitives (`b.compound` heads with the ctor LEAF KIND, not a
/// Name/Str head); (2) it round-trips through op94 decode (`encode(decode(encode v)) == encode(v)`,
/// decoded ≠ NULL), proving `decode_arenas_to_ast`'s ctor arm is the exact inverse. BEFORE the encode
/// ctor arms landed, op93 returned EMPTY on any ctor disc (silent mis-encode + a decode/encode asymmetry
/// vs the compile-time fold) — this pins the symmetry closed.

/// Non-finite `Ast.Float` (NaN / ±inf) round-trips through op93 encode + op94 decode via the codec's
/// payload-less leaf tags (17/18/19), and the ENCODED bytes are byte-identical to the shared
/// Builder+codec of the matching `Leaf::FloatNan`/`FloatInf` (the compile-time `Ast.encode` fold's
/// form). Guards the compile/runtime agreement v-cp's encode-flip relies on.

/// The runtime contract behind the compiler's Perceus DUP-RETAIN fix (`spec@6c1120b2`): a heap value
/// used as a CONSUMING operand (here `String.concat`, which consumes its operand into the rope node)
/// AND with a LATER live use is emitted with a `dup` first, so the later use reads the intact original.
/// The prior test reads the shared leaf THROUGH another concat rope; this reads the ORIGINAL reference
/// DIRECTLY — the `(let ((e S)) (+ (len (String.concat e x)) (len e)))` shape the fix repairs (which
/// returned a wrong value when the dup was missing). After `dup(e)` + `concat(e, x)`, the original `e`
/// must read its full content + correct byte-len (the concat consumed a SEPARATE reference, not this
/// one), and everything reclaims exactly once.

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

/// The runtime half of the `String.at` content-equality fix (`spec@a2c75cc0` root-caused the
/// compiler-side miscompile). The contract test above uses a CONCAT rope; a `String.at` result is a
/// distinct rope shape — a `bytes-SLICE` (`raw = [off, len]`, arity 1 — the parent). `champ_eq`
/// physical-byte-compares that `[off,len]` header, so two slices of the same char at DIFFERENT offsets
/// (or into different parents) are champ_eq-DISTINCT despite equal content — EXACTLY the miscompile
/// (`String.at "banana" 1` ≠ `String.at "banana" 3` though both are "a", so `count-a` returns 0). The
/// compiler's fix compacts the `String.at` result before `=`; that fix RELIES on the runtime's
/// `bytes-compact` flattening a SLICE (not just a concat) to a champ_eq-canonical flat leaf. Pin that
/// runtime half here — the SLICE arm of `bytes_flatten` (read parent's `off+j`), not the concat arm.

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

/// LANE-SPLIT PROBE (v-memory-safety/breaker List.at-over-relaxed-RRB read leak, corpus L2501): does
/// reading every index of a PREPEND-built (relaxed size-table) RRB via `op_vec_get` + the Some-shell
/// dance, WITHOUT threading the list (v borrowed, not dup'd), balance at the RUNTIME layer? The corpus
/// case (build+readsum n=1100) leaks 18972 — but build-only is 0 (op sound) and a SINGLE read is 0, so
/// the leak is the READ LOOP. This isolates the vec-get+Some READ path (runtime) from the compiled
/// recursive THREADING of the borrowed list (compiler). If this balances, the leak is the compiled
/// loop/threading reclaim (COMPILER); if it leaks, the relaxed-node read path itself leaks (RUNTIME).

/// LANE-SPLIT PROBE (v-memory-safety/breaker slice-view-as-key leak-2): does a SINGLE-use borrowed
/// slice-view, compacted then borrowed-compared then dropped, balance at the RUNTIME layer? The corpus
/// cases (19-sets view-as-CHAMP-key, value-eq-of-view) leak 2. If this exact runtime op sequence
/// balances, the leak is COMPILER emit reclaim (a missing/extra drop around the compacted operand); if
/// it leaks, the leak is RUNTIME (bytes_flatten of a single-owned view). Distinct from the dual-use
/// `compact_of_a_dual_used_shared_slice_view_is_balanced_with_the_dup` above (that needs the compiler dup).

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

// ── CHAMP node core: bitmaps, slots, discrimination, structural hash + eq ─────────────

// Build a small normal CHAMP node owning two int leaves as one k/v entry (datamap bit 0).
fn champ_kv_node(k: i64, v: i64) -> Handle {
    alloc_raw(vec![op_box_int(k), op_box_int(v)], champ_header(0b1, 0, 1))
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

/// TAGLESS invariant (the spec's determinism "no-type-tag" principle, duvet-annotated `@b470dd82`):
/// the runtime stores only STRUCTURE + DATA, never a value's TYPE, so `champ_eq`/`champ_hash`/
/// `champ_key_cmp` compare RAW BYTES + arity — they physically CANNOT distinguish two values of
/// DIFFERENT types that happen to share the same raw bytes and (zero) arity. A boxed Int and a Bytes
/// leaf holding the Int's little-endian bytes are therefore champ_eq + hash-equal + cmp-Equal. This is
/// not a bug — it is WHY keeping a map/set's keys HOMOGENEOUS is the COMPILER's obligation (the runtime
/// can't enforce it), and WHY the byte-hash is storage-transparent. Pinning it guards against anyone
/// accidentally adding a type discriminator to the comparison path (which would break byte-hash
/// transparency + the map/set key contract for a compiler that relies on this).

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

/// The size header must stay EXACTLY correct as inserts descend deep spines and split — this is the
/// job of the `(handle, delta)` the insert core now RETURNS (0 overwrite / 1 new key) instead of
/// recomputing via two `champ_size_of` subtree reads. Interleaves new-key inserts (delta 1, must
/// bump size at EVERY ancestor level) with overwrites (delta 0, must bump NOTHING), on BOTH the
/// unique-FBIP path and the shared path-copy path, then verifies size + a full membership sweep. A
/// wrong propagated delta would desync the size header from the true count at some interior node.

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

/// A STRING-KEY collision node — the string sibling of `map_forces_collision_node` (which uses INT
/// keys). The compiler-in-Cadenza port's maps are STRING-keyed, and a string key takes the arity-0
/// HEAP-BYTE-LEAF champ path: the collision node's linear scan compares keys by `champ_eq` = RAW-BYTE
/// content (not the int-immediate compare the int-collision test exercises). Two identifier-like
/// strings that happen to share a full 32-bit FNV hash must still be kept DISTINCT and each resolve to
/// its OWN value BY CONTENT — and removing one collision entry must leave the other intact (the
/// collision-node drain path). Uses a hardcoded pair found by a birthday search over `k{n}` strings;
/// a PRECONDITION guard asserts they still collide, so if the frozen FNV hash ever changes this test
/// fails LOUDLY (re-find a colliding pair) rather than silently degrading to a non-collision case.

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

/// `map_iter_order_is_deterministic` (above) proves insert-order-independent cursor iteration for INT
/// keys (immediate). STRING keys take a DIFFERENT champ path — an arity-0 heap-byte leaf whose slot is
/// chosen by `champ_hash`'s raw-byte FNV, not an int's little-endian bytes — so their CHAMP placement,
/// and thus the cursor's descent order, is a distinct code path. The self-hosting compiler's
/// symbol-table maps are STRING-keyed and it will iterate them (e.g. to emit definitions in a stable
/// order once `Map.fold`/`keys` are exposed — the runtime cursor is already shipped), so a string-key
/// cursor-order bug would make a compiler built on top produce non-deterministic output. Pin that a
/// string-keyed map iterates in the SAME order regardless of insert order (the order is CHAMP hash
/// order — NOT lexicographic; value-encode separately re-sorts to canonical render order).

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

// ── Collision-node canonicality across insert order (regression) ──────────────────────

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
pub(crate) enum VecOp {
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
pub(crate) fn vec_of_reference(reference: &[i64]) -> Handle {
    let mut v = op_vec_empty();
    for &e in reference {
        v = op_vec_push(v, op_box_int(e));
    }
    v
}

pub(crate) fn run_vec_op_sequence(ops: &[VecOp]) {
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

// ── PACKED-BOOL vector randomized differential vs `Vec<bool>` ───────────────────────────────
// The same random push/update/split/concat/fork harness as the int vector, but over a `List Bool`
// whose leaves are bit-packed. It pins the FULL contract for the packed representation under an
// arbitrary op history: (1) elements match a `Vec<bool>` reference through every op, (2) RRB
// invariants hold, (3) the value-encode equals a fresh push-built twin (packing is unobservable at
// the boundary), (4) forks stay undisturbed, no leak — PLUS the density invariant unique to bools:
// after EVERY op, every leaf is still a packed-bool leaf (packing is never lost — the operator's
// "one representation" requirement, enforced structurally, not just observationally).
pub(crate) fn run_bool_vec_op_sequence(ops: &[VecOp]) {
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
pub(crate) fn build_rand_value(
    bytes: &[u8],
    cur: &mut usize,
    budget: &mut u32,
    depth: u32,
) -> Handle {
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
pub(crate) fn build_rand_value_and_shape(
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

/// CANON-STABILITY across the FULL shape space (the property companion of the hand-written
/// `value_encode_leaf_order_is_canon_pre_order_first_encounter`): for ANY random value/descriptor,
/// value-encode's document must have its LEAVES interned in canon's order — strictly PRE-ORDER,
/// first-encounter, left-to-right over the struct tree from the root (cadenza-ast/canon.rs `visit`).
/// That is exactly what makes `value_encode(v)` == `codec::encode(canon(tree))` a stable content-
/// address. The two-shape unit test only reaches the record-`=` and Set-head arms; this exercises
/// Tuple/List/Sum/Map/Named/Framed/nested arms too, so a post-order regression in ANY arm is caught.

/// value-DECODE (heap idx 90) is the inverse of value-encode: for ANY random value, encoding then
/// decoding under the same descriptor must reconstruct a STRUCTURALLY-EQUAL value — the B0 round-trip
/// property (`decode ∘ encode == id`), across the full shape space (Tuple/List/Sum/Record/Set/nested),
/// not just the hand-picked `value_decode_round_trips_*` cases. Also asserts decode never leaks and
/// never traps (returns a handle or declines to NULL). Drives `decode_value` on the in-memory
/// `Descriptor`+`ParsedDoc` directly (op_value_decode's guts) so no descriptor byte-serializer is
/// needed. NOTE: a Set re-canonicalizes on encode (elements sorted by value), so the decoded Set is
/// value-equal though not necessarily node-identical — `value_eq_shaped` compares by canonical value,
/// which is the correct equality here.

/// value-DECODE totality on ARBITRARY bytes — the decode-side sibling of
/// `prop_value_encode_is_total_under_arbitrary_descriptor`. Since B2/B3 (`apply(list<u8>)->list<u8>`),
/// `op_value_decode` is on the critical path of every reducer call, fed guest-produced (and thus
/// potentially malformed or adversarial) doc + descriptor bytes by the kernel. It MUST be TOTAL: for
/// ANY two byte strings it returns a Handle or declines to NULL — never traps (which would abort the
/// kernel), never leaks, never overflows the stack. The hand test only checks 3 fixed malformed inputs;
/// this fuzzes BOTH the document AND the descriptor (the split point derived from the stream, so both
/// halves range over arbitrary bytes independently). Content is NOT asserted — only that the op returns.

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

/// `op_bigint_of_bytes` calls `bytes_flatten(buf)` before reading `raw` — because a Bytes leaf may be a
/// ROPE (concat/slice nodes), whose `raw` holds the node's HEADER bytes, NOT the content (the same
/// rope-read landmine fixed in `str-get`, `@9b24aeb2`). The compiler bakes a FLAT leaf, so that flatten
/// is defensive — and the sibling's round-trip test builds via `op_bytes_alloc` (flat), leaving the
/// flatten path UNEXERCISED. Pin it: build the SAME sign-magnitude bytes as a ROPE (concat across a
/// seam) and confirm `bigint-of-bytes` yields the identical BigInt as the flat leaf — proving the
/// flatten materializes the rope before decoding, not reading concat-header garbage as a magnitude.

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

mod champ_collections;
mod codec_tests;
mod differentials_and_contracts;
mod fbip_advanced_and_property;
mod immediates_and_scalars;
mod packed_bool_and_rope;
mod perceus_and_fbip;
mod structures;

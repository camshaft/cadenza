//! Value-form encode and decode
//!
//! Render runtime values to canonical binary AST documents and decode them back.

use super::*;

// ─── Value-form encode (index 62): render a runtime value to its canonical binary AST document ──
//
// The type-directed renderer the compiler bakes into a program (`sum_form_template` / the fixed
// hole-templates) can render a value of FIXED shape, but a RUNTIME RECURSIVE sum (a linked list, a
// tree — `(type IL (Cons (Tuple Int64 IL)) Nil)`) has unbounded depth, so no fixed template exists and
// the escape declined. This op walks such a value to its canonical value form — the binary AST codec
// document (`codec.rs`: header · leaf pool · struct table · root) — guided by a SHAPE DESCRIPTOR the
// compiler bakes as bytes. The runtime stays NOMINAL-AGNOSTIC: every NAME (the `:` frame, a variant
// head, `tuple`, `unit`, the type name) comes from the descriptor, never invented here; the runtime
// owns only the document ASSEMBLY (leaf dedup, struct indices, byte layout) — the error-prone part that
// hand-emitted wasm would get wrong. See `DESIGN-recursive-sum-escape-walker.md` (approach C).
//
// Shape descriptor wire format (a compiler-baked constant, read by `decode_descriptor`):
//   [ table_len:LEB ]( Shape )*table_len   [ root:LEB ]
// A descriptor is a TABLE of shapes + a root index; a shape references another by INDEX (tag 11 Ref),
// so a self-referential type is FINITE — the recursive payload position is a `Ref` back to the sum's
// table entry, and the value walk follows it only as deep as the runtime value actually nests. Each
// shape is a tag byte + per-tag operands (all counts/lengths unsigned LEB128):
//     0 Int | 1 Bool | 2 Float | 3 Str | 4 Bytes | 5 Unit
//     6 Tuple  [ n ][ elem: idx ]*n                       — each element is a table INDEX
//     7 List   [ elem: idx ]
//     8 Record [ n ]( [ name_len ][ name_utf8 ] [ field: idx ] )*n
//     9 Sum    [ n ]( [ head_len ][ head_utf8 ] [ payload: idx ] )*n       (nullary payload → a Unit idx)
//    10 Named  [ name_len ][ name_utf8 ] [ inner: idx ]   — the `(: <value> <name>)` frame (root only)
//    11 Ref    [ idx ]                                    — an alias to another table entry (recursion)
//    12 Set    [ elem: idx ]                              — 13 Map [ key: idx ][ val: idx ] — 14 Float32
//    15 Framed <TypeNode> [ inner: idx ]   where TypeNode = [ head_len ][ head_utf8 ] [ n ]( TypeNode )*n
//              — the `(: <value> <type-node>)` frame: an arbitrary (possibly NESTED) type node written
//                RECURSIVELY, so a nested element type shows (e.g. `(List (List Int64))`, `(Map Int64
//                (Set Int64))`). A leaf node has n=0 (a bare name). Used for a runtime collection result.
// (Every child position is an INDEX into the table, not an inline shape — that is what lets a cycle
// close: entry k's Sum names entry k as a payload, a finite 1-entry loop the value walk unfolds.)

/// The canonical binary-AST codec tags — kept in lock-step with `rcdzc::codec` (the native encoder this
/// reproduces byte-for-byte). A drift is caught by the `encode_matches_codec` cross-check in the native
/// suite (a runtime document is decoded by `rcdzc::codec::decode` and compared to the source tree).
pub(crate) mod doc {
    pub const SCHEMA_HEADER: [u8; 8] = *b"cdzast\x00\x01";
    pub const KIND_INT_POS_DEC: u8 = 0;
    pub const KIND_FLOAT: u8 = 6;
    pub const KIND_STR: u8 = 7;
    // Non-finite float VALUES — payloadless single kind bytes (like KIND_BOOL_*), matching cadenza-ast
    // codec's KIND_FLOAT_NAN / KIND_FLOAT_POS_INF / KIND_FLOAT_NEG_INF (17/18/19). A non-finite float has
    // no exact decimal (KIND_FLOAT's form), so it crosses the value-encode boundary as one of these
    // dedicated word-form leaves instead of declining the encode (which collapsed any compound holding a
    // non-finite float). NaN is a single canonical sign-less leaf; infinity carries its sign.
    pub const KIND_FLOAT_NAN: u8 = 17;
    pub const KIND_FLOAT_POS_INF: u8 = 18;
    pub const KIND_FLOAT_NEG_INF: u8 = 19;
    pub const KIND_BOOL_FALSE: u8 = 8;
    pub const KIND_BOOL_TRUE: u8 = 9;
    pub const KIND_NAME: u8 = 10;
    pub const KIND_BYTES: u8 = 11;
    // A Unicode-scalar CHAR leaf — the scalar UTF-8-encoded (LEB len + those 1-4 bytes, `write_bytes`
    // framing like a string body), matching cadenza-ast codec's `KIND_CHAR`. Char = bool-analog: int at
    // runtime, no distinct rep; this wire kind is only the RENDER form (a `#\c` char literal on decode).
    pub const KIND_CHAR: u8 = 13;
    // M2 native-compound-data CTOR-HEAD kinds — payloadless single kind-bytes (like KIND_BOOL_*), matching
    // cadenza-ast codec's KIND_*_CTOR / KIND_FIELD_PAIR / KIND_MEMBER. The head-first ctor leaf is the LIST
    // HEAD atom (children follow); the codec has NO canon pass, so build-order IS the content-address form.
    pub const KIND_LIST_CTOR: u8 = 20;
    pub const KIND_TUPLE_CTOR: u8 = 21;
    pub const KIND_RECORD_CTOR: u8 = 22;
    pub const KIND_MAP_CTOR: u8 = 23;
    pub const KIND_SET_CTOR: u8 = 24;
    pub const KIND_FIELD_PAIR: u8 = 25;
    pub const KIND_MEMBER: u8 = 26;
    // Native exact-rational (seq-204): PAYLOADLESS tag leaf (single kind byte, no body — like FIELD_PAIR/
    // MEMBER). A rational VALUE is the LIST `(KIND_RATIONAL <num> <den>)`: this tag as the head atom, then
    // two ordinary Int leaves (numerator, denominator, normalized). Self-typing (no colon frame). This is
    // the DocBuilder emitter's mirror of cadenza-ast's codec kind 27 (byte-identical by construction).
    pub const KIND_RATIONAL: u8 = 27;
    pub const TAG_ATOM: u8 = 0;
    pub const TAG_LIST: u8 = 1;
}

/// Append `value` as unsigned LEB128 — the codec's `write_u64`, byte-identical.
pub(crate) fn doc_leb(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// A shape descriptor node — a value position's shape. Child positions are TABLE INDICES (`u32`), so a
/// recursive type closes as a finite cycle. `Named` carries the outer type name for the
/// `(: <value> <Type>)` frame; `Ref` is an alias to another table entry.
pub(crate) enum Shape {
    Int,
    Bool,
    /// A Unicode-scalar Char leaf — an i32 code-point, SEMANTICS-identical to `Int` (compare / eq / hash by
    /// the code-point integer, exactly as `Bool` is by its 0/1 int); only the RENDER differs (a `KIND_CHAR`
    /// char literal, not a decimal `KIND_INT`). Char = bool-analog: int at runtime, no distinct rep, a
    /// render tag only (descriptor tag 19, mirroring `Bool`'s tag 1). A char value is stored as an immediate
    /// int (the code-point), so it is read with `op_get_int` and boxed with `op_box_int`.
    Char,
    /// A Symbol leaf — shares the String runtime rep (a Symbol's identity IS its UTF-8 text, see
    /// `Symbol.of`), so compare / eq / hash / ORDER treat a Symbol IDENTICALLY to `Str` (a `(Set Symbol)`/
    /// `(Map Symbol _)` key orders by its text via `value_cmp_shaped`). Only the RENDER differs: it emits the
    /// CONSTRUCTION form `((. Symbol of) "text")` — the `#7694` Member-access member-compound, byte-matching
    /// `const_value_ast` + the rust backend — NOT the bare `Str` leaf `"text"` (which is ambiguous with a real
    /// String). Symbol = the Str-analog of how `Char` is the Int-analog: same rep, render tag only (tag 20).
    Symbol,
    Float,
    Str,
    Bytes,
    Unit,
    /// An arbitrary-precision integer leaf (a runtime `BigInt`, `box_bigint`'s sign-magnitude Raw leaf).
    /// Rendered via the SAME `KIND_INT` codec leaf as `Int` — the leaf is already arbitrary-width (sign +
    /// big-endian magnitude bytes, NOT i64-bounded), so a BigInt needs NO new wire kind, only its own
    /// SHAPE tag (so the walk reads the value via `unbox_bigint`, not `op_get_int` which caps at i64).
    BigInt,
    /// An exact-rational leaf (a runtime `Rational`, `box_rational_normalized`'s normalized 2-BigInt-handle
    /// node). Rendered as a single `num/den` NAME leaf — the walk reads both components via `unbox_rational`
    /// and formats each `Big` decimal in the runtime (the codec's Int leaf formats decimal on the HOST, but
    /// a rational is ONE name leaf, so the runtime does it), matching the constant form `(: 1/2 Rational)`.
    Rational,
    /// A Float32 leaf — read with `get-float32` (an `f32`) and rendered as the f32's SHORTEST decimal,
    /// distinct from `Float` (Float64). A Float32 is stored 4-byte (`box-float32`), so its canonical value
    /// form is the f32's, not a promoted f64's (`0.1f32` renders `0.1`, not `0.10000000149011612`).
    Float32,
    // Child-index/field lists are `Arc<[…]>` (not `Vec`) so the descriptor-guided walks (value_cmp_shaped
    // in the Set/Map render `sort_unstable_by` hot path, value_encode, value_eq_shaped) can CHEAPLY clone
    // them (a refcount bump, not an O(n) copy) to drop the `&desc.table` borrow before pushing to the work
    // stack. Shape is in-memory-only (built by decode_shape from the wire descriptor, never serialized), so
    // this retype is hash-neutral. Field names are `Rc<str>` (deduped-friendly, cheap-clone) for the same
    // reason. (operator-commissioned cheap-clone audit, v-core-opt 2026-08-10.)
    Tuple(Rc<[u32]>),
    List(u32),
    Record(Rc<[(Rc<str>, u32)]>),
    Sum(Rc<[(Rc<str>, u32)]>),
    Named(Rc<str>, u32),
    Ref(u32),
    /// A SET over one element shape — rendered `(Set.of (list e1 … en))` with the elements in CANONICAL
    /// key-VALUE order (collections-and-text.md §A Set's canonical form). The runtime iterates the CHAMP
    /// in hash order, so the walk SORTS by the element's canonical scalar value (matching the compiler's
    /// `const_key_order`), NOT by hash or raw bytes. Only a SCALAR element shape is orderable-and-encodable.
    Set(u32),
    /// A MAP from a key shape to a value shape — rendered `(map (k1 v1) … (kn vn))` with entries in
    /// CANONICAL KEY order (collections-and-text.md §A Map Renders As Its Entries In Canonical Key Order),
    /// NOT hash order. Only a SCALAR KEY shape is orderable-and-encodable; the VALUE may be any encodable
    /// shape (the walk recurses on it). `(key_shape, value_shape)` table indices.
    Map(u32, u32),
    /// A `(: <value> <type-node>)` frame — like `Named` but the TYPE is an arbitrary (possibly NESTED)
    /// type node, not a single name. Carries a recursive [`TypeNode`] so a nested collection renders its
    /// full parametric type — e.g. `(List (List Int64))`, `(Map Int64 (List Bool))` — matching the
    /// constant-value form. The `u32` is the inner value shape index.
    Framed(TypeNode, u32),
    /// A MULTI-payload sum variant's payload — a tuple handle at run time (`arr` of the boxed payloads)
    /// whose elements render FLATTENED as the variant's children: `(Cons h t)`, NOT `(Cons (tuple h t))`.
    /// Read exactly like a `Tuple` (each element via `arr-get`) but the enclosing `Sum` walk splices the
    /// elements directly under the variant head instead of emitting a `tuple` form. Only a `Sum` variant's
    /// payload references a `Spread`; a genuine tuple VALUE stays a `Tuple`.
    Spread(Rc<[u32]>),
}

/// A compile-time-baked TYPE node for a `Framed` frame: `head` + child type nodes. A LEAF type
/// (`Int64`/`Bool`/`String`/`Unit`/a nominal name) has no children and renders as the bare name atom; a
/// PARAMETRIC type (`(List e)`, `(Map k v)`, `(Tuple …)`, `(Set e)`) renders `list([head, child…])`, each
/// child rendered recursively. The whole thing is compile-time-known (the result type), so the runtime
/// only re-emits it — it never inspects the runtime value to build the type.
pub(crate) struct TypeNode {
    head: String,
    children: Vec<TypeNode>,
}

/// Decode a [`TypeNode`]: `[ head_len ][ head_utf8 ] [ n_children:LEB ]( TypeNode )*n`.
/// Max nesting of a Framed type node. A genuine type is shallow — `(Map Int64 (List Bool))` is depth 2,
/// and the compiler bakes only such well-formed nodes — so a cap far above any real type still declines a
/// MALFORMED descriptor whose TypeNode nests thousands deep before it overflows the native/wasm call
/// stack. WITHOUT this, `decode_type_node`'s recursion is bounded only by the byte length (each level is
/// just `[name_len=0][n_children=1]` = 2 bytes), so a ~200 KB descriptor crashes the guest — violating
/// value-encode's "never a trap" totality contract (a compiler-baked descriptor is always shallow, but
/// the escape op must DECLINE any input, not abort).
pub(crate) const TYPE_NODE_DEPTH_CAP: u32 = 256;

pub(crate) fn decode_type_node(d: &[u8], pos: &mut usize, depth: u32) -> Option<TypeNode> {
    if depth > TYPE_NODE_DEPTH_CAP {
        return None; // a malformed descriptor's runaway TypeNode nesting — decline, don't overflow
    }
    let head = desc_name(d, pos)?;
    let n = desc_leb(d, pos)?;
    // `reserve_cap`: clamp an untrusted child count to remaining bytes so a malformed TypeNode can't
    // `with_capacity`-abort (each child is ≥1 byte).
    let mut children = Vec::with_capacity(reserve_cap(n, d, *pos));
    for _ in 0..n {
        children.push(decode_type_node(d, pos, depth + 1)?);
    }
    Some(TypeNode { head, children })
}

/// The decoded descriptor: the shape table + the root index. A child index into `table` is followed by
/// the value walk (with a depth cap as a malformed-descriptor backstop).
pub(crate) struct Descriptor {
    pub(crate) table: Vec<Shape>,
    pub(crate) root: u32,
}

pub(crate) fn desc_leb(d: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *d.get(*pos)?;
        *pos += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

pub(crate) fn desc_name(d: &[u8], pos: &mut usize) -> Option<String> {
    let len = desc_leb(d, pos)? as usize;
    let bytes = d.get(*pos..*pos + len)?;
    *pos += len;
    core::str::from_utf8(bytes).ok().map(String::from)
}

/// A pre-reservation capacity for a count `n` decoded from UNTRUSTED descriptor bytes, CLAMPED to the
/// bytes remaining after `pos`. Every element decoded from a count consumes ≥1 byte, so a legitimate `n`
/// never exceeds `d.len() - pos`; clamping turns a bogus huge LEB (e.g. from a random/malformed
/// descriptor) into a small reservation the `?`-guarded loop then fails out of, instead of
/// `Vec::with_capacity(n)` trying to reserve gigabytes and ABORTING the guest (the value-encode escape's
/// "never a trap" totality contract). Costs nothing on well-formed input (the clamp never binds there).
#[inline]
pub(crate) fn reserve_cap(n: u64, d: &[u8], pos: usize) -> usize {
    (n as usize).min(d.len().saturating_sub(pos))
}

pub(crate) fn decode_shape(d: &[u8], pos: &mut usize) -> Option<Shape> {
    let tag = *d.get(*pos)?;
    *pos += 1;
    Some(match tag {
        0 => Shape::Int,
        1 => Shape::Bool,
        2 => Shape::Float,
        3 => Shape::Str,
        4 => Shape::Bytes,
        5 => Shape::Unit,
        6 => {
            let n = desc_leb(d, pos)?;
            let mut elems = Vec::with_capacity(reserve_cap(n, d, *pos));
            for _ in 0..n {
                elems.push(desc_leb(d, pos)? as u32);
            }
            Shape::Tuple(elems.into())
        }
        7 => Shape::List(desc_leb(d, pos)? as u32),
        8 => {
            let n = desc_leb(d, pos)?;
            let mut fields = Vec::with_capacity(reserve_cap(n, d, *pos));
            for _ in 0..n {
                let name: Rc<str> = desc_name(d, pos)?.into();
                fields.push((name, desc_leb(d, pos)? as u32));
            }
            Shape::Record(fields.into())
        }
        9 => {
            let n = desc_leb(d, pos)?;
            let mut variants = Vec::with_capacity(reserve_cap(n, d, *pos));
            for _ in 0..n {
                let head: Rc<str> = desc_name(d, pos)?.into();
                variants.push((head, desc_leb(d, pos)? as u32));
            }
            Shape::Sum(variants.into())
        }
        10 => {
            let name: Rc<str> = desc_name(d, pos)?.into();
            Shape::Named(name, desc_leb(d, pos)? as u32)
        }
        11 => Shape::Ref(desc_leb(d, pos)? as u32),
        12 => Shape::Set(desc_leb(d, pos)? as u32),
        13 => {
            let key = desc_leb(d, pos)? as u32;
            let val = desc_leb(d, pos)? as u32;
            Shape::Map(key, val)
        }
        14 => Shape::Float32,
        15 => {
            // Framed: <TypeNode> [ inner: idx ]  where TypeNode = [ head ][ n ]( TypeNode )*n (recursive).
            let type_node = decode_type_node(d, pos, 0)?;
            Shape::Framed(type_node, desc_leb(d, pos)? as u32)
        }
        16 => {
            // Spread: [ n ]( idx )*n — same wire shape as Tuple (tag 6), a distinct tag so the Sum walk
            // knows to splice the elements FLAT under the variant head rather than wrap them in `tuple`.
            let n = desc_leb(d, pos)?;
            let mut elems = Vec::with_capacity(reserve_cap(n, d, *pos));
            for _ in 0..n {
                elems.push(desc_leb(d, pos)? as u32);
            }
            Shape::Spread(elems.into())
        }
        17 => Shape::BigInt, // arbitrary-precision integer leaf (a runtime BigInt), rendered as KIND_INT
        18 => Shape::Rational, // exact-rational leaf (a 2-BigInt-handle node), rendered as a num/den name
        19 => Shape::Char, // Unicode-scalar Char leaf — int at runtime, rendered as a KIND_CHAR char literal
        20 => Shape::Symbol, // Symbol leaf — str at runtime (order/eq/hash as Str), rendered `((. Symbol of) "…")`
        _ => return None,
    })
}

pub(crate) fn decode_descriptor(d: &[u8]) -> Option<Descriptor> {
    let mut pos = 0usize;
    let n = desc_leb(d, &mut pos)?;
    // `reserve_cap`: a bogus huge table count from a malformed descriptor must not `with_capacity`-abort;
    // each shape is ≥1 byte so a real `n` ≤ remaining bytes, and the `?`-loop fails out of an overlong one.
    let mut table = Vec::with_capacity(reserve_cap(n, d, pos));
    for _ in 0..n {
        table.push(decode_shape(d, &mut pos)?);
    }
    let root = desc_leb(d, &mut pos)? as u32;
    if root as usize >= table.len() {
        return None;
    }
    Some(Descriptor { table, root })
}

runtime_local! {
    /// REUSED completed-struct stack for `encode_value`'s walk (the `out` results Vec) — the companion of
    /// `ENCODE_BUILDER`. `encode_value`'s `out` grew from zero every call (a fresh `Vec<u32>` per encode);
    /// caching it here + `clear()`ing per call retains capacity, so after the first walk it never
    /// reallocates. Safe: single-threaded, the walk is iterative + never re-enters `encode_value`, so the
    /// borrow never nests.
    static ENCODE_OUT: core::cell::RefCell<Vec<u32>> = core::cell::RefCell::new(Vec::new());
}

runtime_local! {
    /// REUSED WORK stack for `encode_value`'s iterative walk — the companion of `ENCODE_OUT`. The `work`
    /// stack grows O(depth) (each container's assembler stays on the stack while its children are visited,
    /// so a Cons-list's depth is O(N)), so a fresh `Vec<EncodeWork>` per encode paid an O(log depth)
    /// grow-chain of reallocs EVERY call. Now that `EncodeWork` is `'static` (its formerly-borrowed key/
    /// name/type-node fields are re-derived from `desc` at process time), the stack caches here +
    /// `clear()`s per call, retaining capacity → grows ONCE to the high-water mark then refills allocation-
    /// FREE. Measured: value_encode of a 50-node list dropped from ~13/encode toward the output-Vec floor.
    /// Safe: single-threaded, iterative, never re-enters `encode_value` — the borrow never nests.
    static ENCODE_WORK: core::cell::RefCell<Vec<EncodeWork>> = core::cell::RefCell::new(Vec::new());
}

runtime_local! {
    /// REUSED `DocBuilder` for `op_value_encode_form` — the value-form escape (op 62) is the hot
    /// host-boundary path (every collection/compound result crossing to the host runs one encode), and a
    /// fresh `DocBuilder::default()` per call grew its `leaves`/`structs`/`child_pool`/`name_index` pools
    /// FROM ZERO every time (~7 realloc doublings each for a modest value = the bulk of the residual
    /// ~43-alloc floor). Caching one builder thread-locally + `reset()`ing it (clear, capacity retained)
    /// makes the pool growth pay ONCE to the high-water mark, then every later encode refills allocation-
    /// FREE — the same alloc-elision as `HASH_SCRATCH`/`EQ_SCRATCH`. Safe: the runtime is single-threaded
    /// and `op_value_encode_form` never re-enters itself (the walk is iterative), so the borrow never
    /// nests. The document bytes are UNCHANGED — reuse only affects allocation, not the emitted output.
    static ENCODE_BUILDER: core::cell::RefCell<DocBuilder> =
        core::cell::RefCell::new(DocBuilder::new_const());
}

runtime_local! {
    /// SINGLE-ENTRY cache of the LAST decoded descriptor: `(descriptor bytes, decoded Descriptor)`.
    /// `decode_descriptor` allocates a `Vec<Shape>` table + a nested Vec per Tuple/Record/Sum/Spread shape
    /// + a `String` per Named/field/variant — a fixed per-call cost that was paid FRESH on every encode
    /// (measured 6 of the ~19 residual allocs for the IntList descriptor, ~31%). But an escape SITE always
    /// crosses the boundary with the SAME compiler-baked descriptor bytes (an escape in a loop re-encodes
    /// under one descriptor), so a 1-entry cache keyed by the byte slice hits ~every call after the first:
    /// on a hit the decode is skipped entirely (0 allocs); on a miss (first call, or a different escape
    /// site interleaved) it decodes + replaces the entry (1 alloc for the cloned key + the decode). The
    /// bytes are the cache key (a `Descriptor` decoded from identical bytes IS identical — the decode is a
    /// pure function of the bytes), so a hit is always correct. Safe: single-threaded; `op_value_encode_
    /// form` clones out / uses the cached `Descriptor` under one borrow and never re-enters itself.
    static DESCRIPTOR_CACHE: core::cell::RefCell<Option<(Vec<u8>, Descriptor)>> =
        core::cell::RefCell::new(None);
}

/// The document builder — a growing leaf pool + struct table, with leaf DEDUP (a repeated name/int
/// collapses to one pool entry, matching the canonical arenas the native encoder is handed). Each
/// `push_*` returns the entry's absolute index; `finish(root)` serializes to the codec document.
#[derive(Default)]
pub(crate) struct DocBuilder {
    pub(crate) leaves: Vec<DocLeaf>,
    pub(crate) structs: Vec<DocStruct>,
    /// Flat arena for every `List` struct's children: a `List` records a `(start, len)` RANGE into this
    /// one pool instead of owning a per-node `Vec<u32>`. Turns N per-compound-node small-Vec allocations
    /// into amortized growth of a single shared Vec (value-encode of a deep value was ~1.3 allocs/node,
    /// dominated by these per-node child Vecs). Children of DIFFERENT lists never interleave: each `list`
    /// call appends its children contiguously and the walk completes one struct before the next.
    pub(crate) child_pool: Vec<u32>,
    /// Name → leaf-index, so `name_leaf`'s dedup is O(log N) not a linear scan of ALL leaves. Without it,
    /// a value with K DISTINCT names (a WIDE record's fields, a many-variant sum's heads) makes the K-th
    /// `name_leaf` scan ~K prior leaves → O(K²) encode (measured: a 3200-field record took ~183 ms vs the
    /// linear ~9 ms). Repeated names (the `Cons`/`tuple` heads in a long list) were already O(1) — the
    /// scan short-circuits on the first match near the front — but DISTINCT names were the quadratic case.
    pub(crate) name_index: alloc::collections::BTreeMap<String, u32>,
}
pub(crate) enum DocLeaf {
    Name(String),
    /// A SCALAR (i64-bounded) integer leaf — stores the raw `i64`, NOT a heap magnitude `Vec`. The
    /// canonical `[sign][big-endian magnitude, leading-zeros-stripped]` wire form is derived directly
    /// into the pre-sized output at `finish` time (the magnitude is ≤8 stack bytes), so a scalar int
    /// leaf allocates NOTHING — this is the dominant leaf in every escaped value (each list/tuple/record
    /// int emitted one `Vec<u8>` before, ~50 of ~92 allocs for a 50-int list). Byte-identical to the old
    /// `Int(neg, be_mag)` form. An arbitrary-width BigInt (>i64) still uses `Int` (a real heap magnitude).
    IntScalar(i64),
    Int(bool, Vec<u8>), // (negative, big-endian magnitude) — BigInt / arbitrary width only
    Bool(bool),
    /// A Unicode-scalar Char leaf. Wire form is `KIND_CHAR` + the scalar UTF-8-encoded (LEB len + 1-4
    /// bytes), byte-identical to cadenza-ast codec's `Leaf::Char` framing.
    Char(char),
    // UTF-8 body / raw byte payload stored as `Raw` (inline for ≤INLINE_RAW_CAP bytes — the common short
    // string/key case — else heap), so a SHORT string/bytes leaf allocates NOTHING (a JSON-dictionary key
    // "k00", a small tag) instead of a per-leaf `Vec<u8>`. `Raw` owns its bytes (no lifetime coupling to the
    // source node), so the pooled leaf stays `'static`. `finish` reads `.as_slice()` — storage-transparent.
    Str(Raw),   // UTF-8 body verbatim (the runtime String's raw bytes)
    Bytes(Raw), // raw byte payload verbatim (the runtime Bytes value, rope flattened)
    Float {
        negative: bool,
        exponent: i64,
        significand: Vec<u8>,
    }, // exact decimal (from f64), big-endian mag
    /// The non-finite float value NaN — a payloadless leaf (`KIND_FLOAT_NAN`), matching cadenza-ast codec's
    /// `Leaf::FloatNan`. A non-finite float has no exact decimal, so it crosses as this dedicated word-form
    /// leaf instead of declining the encode (which collapsed any compound containing it).
    FloatNan,
    /// A non-finite float infinity (`+∞`/`−∞`) — payloadless (`KIND_FLOAT_POS_INF`/`KIND_FLOAT_NEG_INF`),
    /// matching cadenza-ast codec's `Leaf::FloatInf { negative }`.
    FloatInf {
        negative: bool,
    },
    /// A payloadless M2 ctor-head leaf — stores its `doc::KIND_*_CTOR`/`KIND_FIELD_PAIR`/`KIND_MEMBER` byte
    /// (20-26). Wire form is that single kind byte (no body), like `Bool`. The head-first list-head atom for
    /// a native compound value; DEDUPED by `ctor_leaf` (matching cadenza-ast `Builder::leaf`'s general dedup).
    Ctor(u8),
}
pub(crate) enum DocStruct {
    Atom(u32),
    /// A list struct: its children are `child_pool[start .. start + len]` (a RANGE into the builder's
    /// shared arena, not an owned Vec).
    List {
        start: u32,
        len: u32,
    },
}

/// The canonical big-endian magnitude of a scalar `i64`, leading zeros stripped (empty for zero), into a
/// STACK buffer — the codec's `KIND_INT` magnitude for an i64-bounded value. Returns `(negative, &mag)`
/// borrowing `buf`. `unsigned_abs` handles `i64::MIN` without overflow (magnitude `80 00…00`). The
/// `DocLeaf::IntScalar` write path uses this to emit the same bytes the old heap-`Vec` form did, with NO
/// allocation — the write is `out.extend_from_slice(mag)` straight from the stack.
#[inline]
pub(crate) fn i64_be_magnitude(v: i64, buf: &mut [u8; 8]) -> (bool, &[u8]) {
    *buf = v.unsigned_abs().to_be_bytes();
    let start = buf.iter().position(|&b| b != 0).unwrap_or(buf.len());
    // Zero carries an EMPTY magnitude and is never negative on the wire (matches the old `int_leaf` +
    // `DocLeaf::Int`'s finish rule, and `bigint_leaf`'s canonical zero).
    let mag = &buf[start..];
    (v < 0 && !mag.is_empty(), mag)
}

impl DocBuilder {
    /// A const-constructible EMPTY builder — the initializer for the reused `ENCODE_BUILDER` thread-local
    /// (a `const {}` thread-local body needs a const init; `#[derive(Default)]`'s `default()` is not const).
    /// Every field's empty form is a const fn (`Vec::new`/`BTreeMap::new`).
    const fn new_const() -> DocBuilder {
        DocBuilder {
            leaves: Vec::new(),
            structs: Vec::new(),
            child_pool: Vec::new(),
            name_index: alloc::collections::BTreeMap::new(),
        }
    }
    /// Clear every pool for REUSE across encodes — retains each buffer's CAPACITY (`Vec::clear` /
    /// `BTreeMap::clear` free no backing store), so after the first encode grows them to the high-water
    /// mark, subsequent encodes refill without reallocating. Called by `op_value_encode_form` on the
    /// reused `ENCODE_BUILDER` before each walk. Dropping the old `DocLeaf` entries frees their owned
    /// Strings/byte Vecs (a name/str/bytes/float leaf) — only the SPINE Vecs' capacity is retained.
    pub(crate) fn reset(&mut self) {
        self.leaves.clear();
        self.structs.clear();
        self.child_pool.clear();
        self.name_index.clear();
    }
    pub(crate) fn name_leaf(&mut self, name: &str) -> u32 {
        // Dedup names to a single leaf. HYBRID, so the common encode pays ZERO extra allocation:
        //  • SMALL regime (few distinct names — the norm: `Cons`/`Nil`/`tuple`/`record`/`map`/`:`/keys):
        //    scan the existing `DocLeaf::Name` entries directly. Allocation-FREE (the name String lives
        //    only in the leaf, no duplicate map key) and fast — the scan short-circuits on the first match
        //    near the front, so a repeated head is O(1).
        //  • LARGE regime (many DISTINCT names — a wide record's fields, a many-variant sum): once the
        //    NAME leaf count crosses `NAME_INDEX_THRESHOLD` the linear scan would go O(N²) (a 3200-field
        //    record took 183 ms), so build `name_index` ONCE from the leaves seen so far and use the
        //    BTreeMap (O(log N)) thereafter (~15 ms). Byte-identical either way — a repeated name resolves
        //    to its FIRST-inserted index in both.
        const NAME_INDEX_THRESHOLD: u32 = 16;
        if self.name_index.is_empty() {
            let mut name_count = 0u32;
            for (i, l) in self.leaves.iter().enumerate() {
                if let DocLeaf::Name(n) = l {
                    if n == name {
                        return i as u32;
                    }
                    name_count += 1;
                }
            }
            let i = self.leaves.len() as u32;
            self.leaves.push(DocLeaf::Name(String::from(name)));
            if name_count + 1 > NAME_INDEX_THRESHOLD {
                // Crossed the threshold — index every name leaf ONCE; the map owns dedup from here.
                for (idx, l) in self.leaves.iter().enumerate() {
                    if let DocLeaf::Name(n) = l {
                        self.name_index.insert(n.clone(), idx as u32);
                    }
                }
            }
            return i;
        }
        // Large regime: the map owns the dedup (O(log N)).
        if let Some(&i) = self.name_index.get(name) {
            return i;
        }
        let i = self.leaves.len() as u32;
        self.leaves.push(DocLeaf::Name(String::from(name)));
        self.name_index.insert(String::from(name), i);
        i
    }
    pub(crate) fn int_leaf(&mut self, v: i64) -> u32 {
        // Store the raw `i64` — the canonical `[sign][big-endian magnitude, leading-zeros-stripped]` wire
        // form is derived directly into the output at `finish` (a ≤8-byte stack magnitude), so a scalar int
        // leaf allocates NO heap Vec. Byte-IDENTICAL to the old `Int(v<0, be_mag_stripped)` form.
        self.leaves.push(DocLeaf::IntScalar(v));
        (self.leaves.len() - 1) as u32
    }
    /// A BigInt leaf — the SAME `KIND_INT` codec leaf as `int_leaf`, but for an arbitrary-precision value.
    /// `Big::to_sign_magnitude_bytes` yields `[sign][LE magnitude…]` (trailing zeros stripped); the codec's
    /// `DocLeaf::Int` wants (negative, BIG-endian magnitude, leading zeros stripped), so drop the sign
    /// byte, reverse to big-endian, and trim leading zeros. Zero → empty magnitude (positive), matching
    /// `int_leaf`'s canonical zero. No i64 bound — the magnitude is however many bytes the value needs.
    pub(crate) fn bigint_leaf(&mut self, b: &bigint::Big) -> u32 {
        let sm = b.to_sign_magnitude_bytes(); // [sign][LE mag…]
        let neg = sm.first().copied().unwrap_or(0) != 0;
        let mut magnitude: Vec<u8> = sm.get(1..).unwrap_or(&[]).iter().rev().copied().collect();
        while magnitude.first() == Some(&0) {
            magnitude.remove(0);
        }
        // A zero magnitude is never negative on the wire (matches `int_leaf` + `DocLeaf::Int`'s finish rule).
        let neg = neg && !magnitude.is_empty();
        self.leaves.push(DocLeaf::Int(neg, magnitude));
        (self.leaves.len() - 1) as u32
    }
    pub(crate) fn bool_leaf(&mut self, b: bool) -> u32 {
        self.leaves.push(DocLeaf::Bool(b));
        (self.leaves.len() - 1) as u32
    }
    /// A Unicode-scalar Char leaf (`doc::KIND_CHAR`) — the render form of an int-repped char value. Not
    /// deduped (like `bool_leaf`/`int_leaf`): the decoder re-interns on read.
    pub(crate) fn char_leaf(&mut self, c: char) -> u32 {
        self.leaves.push(DocLeaf::Char(c));
        (self.leaves.len() - 1) as u32
    }
    /// An M2 ctor-head leaf (`doc::KIND_*_CTOR`/`KIND_FIELD_PAIR`/`KIND_MEMBER`, 20-26) — the payloadless
    /// head atom of a native compound value. DEDUPED to its FIRST-inserted id (matching cadenza-ast
    /// `Builder::leaf`'s general `leaf_index` dedup, which `const_value_ast` uses via `atom_leaf`), so a
    /// value with repeated ctors (a list-of-tuples) byte-matches `const_value_ast`. Only ≤7 distinct ctor
    /// kinds exist, so the linear scan is effectively O(1).
    pub(crate) fn ctor_leaf(&mut self, kind: u8) -> u32 {
        for (i, l) in self.leaves.iter().enumerate() {
            if let DocLeaf::Ctor(k) = l
                && *k == kind
            {
                return i as u32;
            }
        }
        self.leaves.push(DocLeaf::Ctor(kind));
        (self.leaves.len() - 1) as u32
    }
    /// A string leaf — the UTF-8 body verbatim (the codec's `KIND_STR`, `write_bytes` = LEB len + bytes,
    /// identical framing to a `Name` leaf but a distinct kind). Not deduped (like `int_leaf`/`bool_leaf`):
    /// the codec DECODER re-interns leaves on read, so a repeated string in the pool is harmless. Takes a
    /// BORROWED slice + stores it as `Raw` (inline for a short string — no heap alloc; the leaf owns its
    /// bytes so no lifetime coupling to the source node).
    pub(crate) fn str_leaf(&mut self, bytes: &[u8]) -> u32 {
        self.leaves.push(DocLeaf::Str(Raw::from_slice(bytes)));
        (self.leaves.len() - 1) as u32
    }
    /// A bytes leaf — the raw byte payload verbatim (the codec's `KIND_BYTES`, `write_bytes` = LEB len +
    /// bytes, same framing as a `Str`/`Name` leaf, distinct kind). Not deduped (like `str_leaf`). Borrowed
    /// slice → `Raw` (inline for a short payload — no heap alloc).
    pub(crate) fn bytes_leaf(&mut self, bytes: &[u8]) -> u32 {
        self.leaves.push(DocLeaf::Bytes(Raw::from_slice(bytes)));
        (self.leaves.len() - 1) as u32
    }
    /// A float leaf — the EXACT decimal `(-1)^neg · significand · 10^exponent` the codec's `KIND_FLOAT`
    /// stores. Converts the runtime `f64` to that decimal by a byte-for-byte PORT of the compiler's
    /// `Decimal::from_f64` (rcdzc `ast.rs`): `{:e}` shortest round-tripping text → sign, digit string,
    /// base-10 exponent, then a base-10→base-256 Horner conversion of the digits (no BigInt — plain
    /// `Vec<u8>`, so `no_std`-portable). A NON-FINITE float (`nan`/`inf`) has no exact decimal / no written
    /// form (like `from_f64`'s `None`), so it emits its dedicated payloadless word-form leaf
    /// ([`DocLeaf::FloatNan`]/[`DocLeaf::FloatInf`]) instead of declining — a non-finite float in a compound
    /// must cross the boundary (previously the decline collapsed the whole compound).
    pub(crate) fn float_leaf(&mut self, f: f64) -> Option<u32> {
        if !f.is_finite() {
            return Some(self.non_finite_float_leaf(f.is_nan(), f.is_sign_negative()));
        }
        // A WHOLE float renders its FULL exact expansion (`{f:.0}`, matches scalar display_float + rust);
        // a non-whole keeps `{:e}` (shortest == written form; `{f:.0}` would round the fraction away).
        let text = if is_whole_f64(f) {
            format!("{f:.0}")
        } else {
            format!("{f:e}")
        };
        self.float_leaf_from_sci(&text)
    }
    /// A Float32 leaf — the f32's SHORTEST round-tripping decimal (via `{:e}` on the `f32`, NOT a
    /// promoted f64 whose shortest decimal differs — `0.1f32` → `"1e-1"` not `"1.0000000149…e-1"`). Same
    /// `KIND_FLOAT` encoding as `float_leaf`; a non-finite f32 emits the dedicated word-form leaf (the same
    /// payloadless `FloatNan`/`FloatInf` — non-finites carry no width), matching `float_leaf`.
    pub(crate) fn float32_leaf(&mut self, f: f32) -> Option<u32> {
        if !f.is_finite() {
            return Some(self.non_finite_float_leaf(f.is_nan(), f.is_sign_negative()));
        }
        self.float_leaf_from_sci(&format!("{f:e}"))
    }
    /// Push a non-finite float's dedicated payloadless word-form leaf and return its index — `FloatNan`
    /// (`is_nan`, sign-less canonical, matching cadenza-ast's single canonical NaN) else `FloatInf` with the
    /// sign. Shared by `float_leaf`/`float32_leaf` so a non-finite float in a compound crosses the
    /// value-encode boundary instead of collapsing the compound to empty.
    fn non_finite_float_leaf(&mut self, is_nan: bool, negative: bool) -> u32 {
        self.leaves.push(if is_nan {
            DocLeaf::FloatNan
        } else {
            DocLeaf::FloatInf { negative }
        });
        (self.leaves.len() - 1) as u32
    }
    /// Build a `KIND_FLOAT` `DocLeaf::Float` from a `[-]D[.DDDD]eEXP` scientific-notation string (the
    /// `{:e}` form of an f32 or f64): parse sign / digit string / base-10 exponent, then a base-10→
    /// base-256 Horner conversion of the digits (no BigInt — `no_std`-portable). Shared by both float
    /// widths so the exact decimal is the value's OWN shortest form. `None` on a malformed string.
    pub(crate) fn float_leaf_from_sci(&mut self, sci: &str) -> Option<u32> {
        let (negative, rest) = match sci.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, sci),
        };
        let (mantissa, exp10): (&str, i64) = match rest.split_once('e') {
            Some((m, e)) => (m, e.parse().ok()?),
            None => (rest, 0),
        };
        let (int_part, frac_part) = match mantissa.split_once('.') {
            Some((i, fr)) => (i, fr),
            None => (mantissa, ""),
        };
        let mut digits = String::from(int_part);
        digits.push_str(frac_part);
        let exponent = exp10 - frac_part.len() as i64; // fold fractional digits into the exponent
        // base-10 digits → little-endian base-256 magnitude (Horner: acc = acc*10 + d).
        let mut mag: Vec<u8> = Vec::new();
        for ch in digits.bytes() {
            if !ch.is_ascii_digit() {
                return None;
            }
            let mut carry = (ch - b'0') as u32;
            for byte in mag.iter_mut() {
                let v = (*byte as u32) * 10 + carry;
                *byte = (v & 0xff) as u8;
                carry = v >> 8;
            }
            while carry > 0 {
                mag.push((carry & 0xff) as u8);
                carry >>= 8;
            }
        }
        // strip most-significant zeros → big-endian minimal magnitude, empty iff zero.
        while mag.last() == Some(&0) {
            mag.pop();
        }
        mag.reverse();
        self.leaves.push(DocLeaf::Float {
            negative,
            exponent,
            significand: mag,
        });
        Some((self.leaves.len() - 1) as u32)
    }
    pub(crate) fn atom(&mut self, leaf: u32) -> u32 {
        self.structs.push(DocStruct::Atom(leaf));
        (self.structs.len() - 1) as u32
    }
    /// Record a `List` struct whose children are `children` — appended CONTIGUOUSLY to the shared
    /// `child_pool` (no per-node Vec). Takes a slice so callers pass a stack array (`&[a, b]`) or an
    /// existing slice (`&out[base..]`) without allocating a temporary Vec.
    pub(crate) fn list(&mut self, children: &[u32]) -> u32 {
        let start = self.child_pool.len() as u32;
        self.child_pool.extend_from_slice(children);
        self.structs.push(DocStruct::List {
            start,
            len: children.len() as u32,
        });
        (self.structs.len() - 1) as u32
    }
    /// Record a `List` whose first child is `head` and remaining children are `tail` — the assembler
    /// shape (`head` + the completed sub-results in `out[base..]`), building the range directly in the
    /// pool with NO temporary Vec.
    pub(crate) fn list_head_tail(&mut self, head: u32, tail: &[u32]) -> u32 {
        let start = self.child_pool.len() as u32;
        self.child_pool.push(head);
        self.child_pool.extend_from_slice(tail);
        self.structs.push(DocStruct::List {
            start,
            len: 1 + tail.len() as u32,
        });
        (self.structs.len() - 1) as u32
    }
    /// Render a [`TypeNode`] to a struct index (recursive): a LEAF type (no children) → the bare name
    /// atom; a PARAMETRIC type → `list([head-atom, child…])`, each child rendered recursively. Builds the
    /// `(: value <type>)` frame's type position for a `Framed` — handles arbitrary nesting like
    /// `(List (List Int64))` / `(Map Int64 (List Bool))`.
    pub(crate) fn render_type_node(&mut self, tn: &TypeNode) -> u32 {
        let head_leaf = self.name_leaf(&tn.head);
        let head_atom = self.atom(head_leaf);
        if tn.children.is_empty() {
            head_atom
        } else {
            let child_structs: Vec<u32> = tn
                .children
                .iter()
                .map(|c| self.render_type_node(c))
                .collect();
            self.list_head_tail(head_atom, &child_structs)
        }
    }
    pub(crate) fn finish(&self, root: u32) -> Vec<u8> {
        // Pre-size the output so serializing a large document doesn't realloc-churn (grow-once, the same
        // discipline as the leaf/struct/child pools). Cheap UPPER-BOUND estimate in one pass: header +
        // counts/root LEBs, per leaf a kind byte + a ≤10-byte length/exponent field + its payload bytes,
        // per struct a tag + ≤5-byte LEB, and ≤5 bytes per pooled child index. An over-estimate only wastes
        // a little transient capacity; it never truncates (the writers still push). MEASURED −8 reallocs/
        // encode on a 50-node list, −12 on a 1000-entry map.
        let leaf_bytes: usize = self
            .leaves
            .iter()
            .map(|l| match l {
                DocLeaf::IntScalar(_) => 11 + 8, // kind + ≤10-byte len LEB + ≤8 magnitude bytes
                DocLeaf::Int(_, mag) => 11 + mag.len(),
                DocLeaf::Bool(_) => 1,
                DocLeaf::Char(_) => 1 + 1 + 4, // kind + LEB len + ≤4 UTF-8 scalar bytes
                DocLeaf::Ctor(_) => 1,         // payloadless single kind byte
                DocLeaf::Name(n) => 11 + n.len(),
                DocLeaf::Str(b) | DocLeaf::Bytes(b) => 11 + b.len(),
                DocLeaf::Float { significand, .. } => 20 + significand.len(),
                DocLeaf::FloatNan | DocLeaf::FloatInf { .. } => 1, // payloadless single kind byte
            })
            .sum();
        let est = doc::SCHEMA_HEADER.len()
            + 20 // the two count LEBs + root LEB, generous
            + leaf_bytes
            + self.structs.len() * 6 // tag + LEB per struct
            + self.child_pool.len() * 5; // ≤5-byte LEB per pooled List child
        let mut out = Vec::with_capacity(est);
        out.extend_from_slice(&doc::SCHEMA_HEADER);
        doc_leb(&mut out, self.leaves.len() as u64);
        for leaf in &self.leaves {
            match leaf {
                DocLeaf::IntScalar(v) => {
                    // Derive the canonical `[sign][BE magnitude]` on the STACK (no heap Vec) and write the
                    // SAME bytes the `Int` arm below writes for the equivalent value. Kinds (sign<<0 offset):
                    // pos-dec = 0, neg-dec = 3 (codec KIND_INT_*); zero → empty magnitude, positive kind.
                    let mut buf = [0u8; 8];
                    let (is_neg, mag) = i64_be_magnitude(*v, &mut buf);
                    out.push(if is_neg {
                        doc::KIND_INT_POS_DEC + 3
                    } else {
                        doc::KIND_INT_POS_DEC
                    });
                    doc_leb(&mut out, mag.len() as u64);
                    out.extend_from_slice(mag);
                }
                DocLeaf::Int(neg, mag) => {
                    // Zero carries an empty magnitude and the POSITIVE kind (never negative-zero).
                    let is_neg = *neg && !mag.is_empty();
                    // Kinds are (sign<<0 offset): pos-dec = 0, neg-dec = 3 (see codec KIND_INT_*).
                    out.push(if is_neg {
                        doc::KIND_INT_POS_DEC + 3
                    } else {
                        doc::KIND_INT_POS_DEC
                    });
                    doc_leb(&mut out, mag.len() as u64);
                    out.extend_from_slice(mag);
                }
                DocLeaf::Bool(b) => out.push(if *b {
                    doc::KIND_BOOL_TRUE
                } else {
                    doc::KIND_BOOL_FALSE
                }),
                DocLeaf::Char(c) => {
                    // KIND_CHAR + the scalar UTF-8-encoded (LEB len + 1-4 bytes) — the `write_bytes` framing
                    // (like a Str body), byte-identical to cadenza-ast codec's `Leaf::Char` encode.
                    out.push(doc::KIND_CHAR);
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    doc_leb(&mut out, s.len() as u64);
                    out.extend_from_slice(s.as_bytes());
                }
                DocLeaf::Ctor(k) => out.push(*k), // payloadless M2 ctor-head kind byte (20-26)
                DocLeaf::Name(n) => {
                    out.push(doc::KIND_NAME);
                    doc_leb(&mut out, n.len() as u64);
                    out.extend_from_slice(n.as_bytes());
                }
                DocLeaf::Str(bytes) => {
                    // KIND_STR + write_bytes (LEB len + UTF-8 body) — same framing as a Name, distinct kind.
                    out.push(doc::KIND_STR);
                    doc_leb(&mut out, bytes.len() as u64);
                    out.extend_from_slice(bytes.as_slice());
                }
                DocLeaf::Bytes(bytes) => {
                    // KIND_BYTES + write_bytes (LEB len + raw bytes) — same framing as Str/Name, distinct kind.
                    out.push(doc::KIND_BYTES);
                    doc_leb(&mut out, bytes.len() as u64);
                    out.extend_from_slice(bytes.as_slice());
                }
                DocLeaf::Float {
                    negative,
                    exponent,
                    significand,
                } => {
                    // KIND_FLOAT + negative(u8) + exponent(FIXED 8-byte big-endian i64, NOT LEB) +
                    // LEB significand length + big-endian magnitude bytes. Matches the codec's Float write.
                    out.push(doc::KIND_FLOAT);
                    out.push(*negative as u8);
                    out.extend_from_slice(&exponent.to_be_bytes());
                    doc_leb(&mut out, significand.len() as u64);
                    out.extend_from_slice(significand);
                }
                // Non-finite floats — a single payloadless kind byte (like Bool/Ctor), byte-identical to
                // cadenza-ast codec's KIND_FLOAT_NAN / KIND_FLOAT_POS_INF / KIND_FLOAT_NEG_INF.
                DocLeaf::FloatNan => out.push(doc::KIND_FLOAT_NAN),
                DocLeaf::FloatInf { negative } => out.push(if *negative {
                    doc::KIND_FLOAT_NEG_INF
                } else {
                    doc::KIND_FLOAT_POS_INF
                }),
            }
        }
        doc_leb(&mut out, self.structs.len() as u64);
        for s in &self.structs {
            match s {
                DocStruct::Atom(id) => {
                    out.push(doc::TAG_ATOM);
                    doc_leb(&mut out, *id as u64);
                }
                DocStruct::List { start, len } => {
                    out.push(doc::TAG_LIST);
                    doc_leb(&mut out, *len as u64);
                    let (s, l) = (*start as usize, *len as usize);
                    for &c in &self.child_pool[s..s + l] {
                        doc_leb(&mut out, c as u64);
                    }
                }
            }
        }
        doc_leb(&mut out, root as u64);
        out
    }
}

/// Follow a shape index through `Named`/`Ref` wrappers to the underlying shape (an erased newtype / a
/// table alias adds no runtime representation). Bounded by a small hop budget as a malformed-cycle
/// backstop. Returns the resolved `&Shape` (borrowed from the table), or `None` on a broken/cyclic index.
pub(crate) fn resolve_shape(desc: &Descriptor, mut shape_ix: u32) -> Option<&Shape> {
    for _ in 0..64 {
        match desc.table.get(shape_ix as usize)? {
            Shape::Ref(target) | Shape::Named(_, target) => shape_ix = *target,
            other => return Some(other),
        }
    }
    None
}

/// Collect a SET's elements into a Vec of (borrowed) element handles, SORTED into canonical key-VALUE
/// order under the element shape `elem_ix` (resolved through `Named`/`Ref`). The CHAMP iterates hash
/// order, so this re-sorts to the canonical render order. `None` (the encode declines) when the element
/// shape is not a canonically-orderable SCALAR — matching the compiler's `const_key_order`, which
/// declines a nested-compound element. The returned handles are BORROWED (the set still owns them); the
/// caller only reads them to encode, so no dup/drop is needed.
pub(crate) fn set_elements_canonical(
    desc: &Descriptor,
    set: Handle,
    elem_ix: u32,
) -> Option<Vec<Handle>> {
    // The element must offer a total order — a blessed scalar leaf OR an orderable COMPOUND (a tuple/list/
    // record/sum all of whose leaves are orderable). `value_cmp_shaped` supplies that order for BOTH cases
    // (it's the same descriptor-guided total order the runtime `<`/`Core::ValueCmp` walk and value-encode
    // use), so we probe orderability once and reuse the walk for the sort. A non-orderable element (a float/
    // bytes/set/map leaf) makes `value_cmp_shaped` return `None` → the encode declines (empty list), matching
    // the compiler, which only bakes a set descriptor over an orderable element.
    let mut elems: Vec<Handle> = Vec::new();
    let mut cur = op_set_iter(set);
    loop {
        let e = op_set_iter_elem(cur);
        if e == Handle::NULL {
            break; // exhausted
        }
        elems.push(e);
        cur = op_set_iter_next(cur);
    }
    op_drop(cur); // release the final (exhausted) cursor
    // Probe orderability on a representative element (all elements share `elem_ix`'s shape). An empty set is
    // trivially orderable — nothing to sort. A `None` on a non-empty set means a non-orderable element shape.
    if let Some(&probe) = elems.first()
        && value_cmp_shaped(desc, probe, probe, elem_ix).is_none()
    {
        return None; // a non-orderable element shape — unrenderable, decline
    }
    // Sort into canonical VALUE order via the descriptor-guided total order. Set members are DISTINCT, so
    // stability is irrelevant → the in-place `sort_unstable_by` (no merge scratch-buffer allocation, better
    // constants) gives the same canonical order as a stable sort with one fewer heap allocation. A `None`
    // from `value_cmp_shaped` mid-sort (defensive — the orderability probe above already ruled it out) reads
    // as Equal, keeping the sort total (never a panic).
    elems.sort_unstable_by(|&x, &y| {
        value_cmp_shaped(desc, x, y, elem_ix).unwrap_or(core::cmp::Ordering::Equal)
    });
    Some(elems)
}

/// Collect a MAP's entries into a Vec of (borrowed) `(key, value)` handle pairs, SORTED into canonical
/// KEY-value order under the KEY shape `key_ix` (resolved through `Named`/`Ref`). The CHAMP iterates
/// hash order, so this re-sorts to the canonical render order (`collections-and-text.md §A Map Renders
/// As Its Entries In Canonical Key Order`). `None` (the encode declines) when the KEY shape is not a
/// canonically-orderable SCALAR — matching the compiler's `const_key_order`. The VALUE may be any
/// encodable shape (the walk recurses on it). Handles are BORROWED (the map owns them); no dup/drop.
pub(crate) fn map_entries_canonical(
    desc: &Descriptor,
    map: Handle,
    key_ix: u32,
) -> Option<Vec<(Handle, Handle)>> {
    // The KEY must offer a total order — a blessed scalar leaf OR an orderable COMPOUND (tuple/list/record/
    // sum of orderable leaves). `value_cmp_shaped` supplies that order for BOTH (the same total order the
    // runtime `<`/value-encode use), so we probe orderability once and reuse the walk for the sort. A
    // non-orderable key (a float/bytes/set/map leaf) makes it return `None` → the encode declines, matching
    // the compiler, which only bakes a map descriptor over an orderable key.
    let mut entries: Vec<(Handle, Handle)> = Vec::new();
    let mut cur = op_map_iter(map);
    loop {
        let k = op_map_iter_key(cur);
        if k == Handle::NULL {
            break; // exhausted
        }
        let v = op_map_iter_val(cur);
        entries.push((k, v));
        cur = op_map_iter_next(cur);
    }
    op_drop(cur); // release the final (exhausted) cursor
    // Probe orderability on a representative KEY (all keys share `key_ix`'s shape); an empty map is trivially
    // orderable. A `None` on a non-empty map means a non-orderable key shape.
    if let Some(&(probe, _)) = entries.first()
        && value_cmp_shaped(desc, probe, probe, key_ix).is_none()
    {
        return None; // a non-orderable key shape — unrenderable, decline
    }
    // Sort by canonical KEY order via the descriptor-guided total order. Map keys are DISTINCT → stability is
    // irrelevant, so `sort_unstable_by` (in-place, no merge scratch-buffer allocation) gives the same
    // canonical order with one fewer heap allocation than the stable `sort_by`. A defensive mid-sort `None`
    // (ruled out by the probe) reads as Equal, keeping the sort total.
    entries.sort_unstable_by(|&(ka, _), &(kb, _)| {
        value_cmp_shaped(desc, ka, kb, key_ix).unwrap_or(core::cmp::Ordering::Equal)
    });
    Some(entries)
}

/// `set-to-list(s, desc)` — enumerate a SET's elements as a runtime `List` (a persistent vec) in CANONICAL
/// element-value order (collections-and-text.md §A Set's canonical form: program iteration order == the
/// canonical byte-form order, NOT the CHAMP hash order the raw cursor walks). Reuses `set_elements_canonical`
/// (the same sorted collection value-encode uses to render `(Set.of (list …))`), so the observable order is
/// IDENTICAL to the value form — one source of truth for canonical order. BORROWS `s` and `desc` (an
/// inspector — the caller owns `s`'s release; `desc` is a compiler-baked constant): each element handle the
/// sorted walk returns is BORROWED (the set still owns it), so it is `dup`'d before being stored in the fresh
/// OWNED result vec (the vec now co-owns a reference; the set keeps its own). A malformed descriptor or a
/// non-scalar (unorderable) element shape yields the EMPTY vec — the defensive total matching value-encode's
/// never-trap contract (the compiler only bakes a well-formed `Set` descriptor here). The result is a normal
/// `List a` handle the front-end consumes exactly like any list.
pub(crate) fn op_set_to_list(set: Handle, desc: &[u8]) -> Handle {
    let Some(descriptor) = decode_descriptor(desc) else {
        return op_vec_empty();
    };
    // The root shape must resolve to a `Set(elem_ix)`; anything else is a malformed/mismatched descriptor.
    let elem_ix = match resolve_shape(&descriptor, descriptor.root) {
        Some(Shape::Set(e)) => *e,
        _ => return op_vec_empty(),
    };
    let Some(elems) = set_elements_canonical(&descriptor, set, elem_ix) else {
        return op_vec_empty(); // a non-scalar element shape is unorderable — decline to the empty list
    };
    // Build the arr of (dup'd) element handles in canonical order, then fold it into a persistent vec. The
    // CHAMP stores each element ALREADY BOXED (a scalar's box-* leaf / a compound's handle), so the element
    // handle is stored as-is — no re-box — exactly the representation a `List a` element carries.
    let arr = op_arr_alloc(elems.len() as u32);
    for (i, &e) in elems.iter().enumerate() {
        op_dup(e); // the set still owns `e`; the vec takes an independent reference
        op_arr_set(arr, i as u32, e);
    }
    op_vec_of_arr(arr) // consumes the arr, yields the List handle
}

/// `map-to-list(m, desc)` — enumerate a MAP's entries as a runtime `List (Tuple k v)` (a persistent vec of
/// 2-element tuple handles) in CANONICAL KEY order (collections-and-text.md §A Map Renders As Its Entries In
/// Canonical Key Order). Reuses `map_entries_canonical` (the sorted walk value-encode renders from), so the
/// observable order matches the value form exactly. BORROWS `m` and `desc`; each `(key, value)` handle the
/// walk returns is BORROWED (the map owns them), so both are `dup`'d before being stored into the fresh owned
/// entry tuple (an `arr-alloc(2)` — the runtime representation of `(Tuple k v)`, key at slot 0, value at slot
/// 1), and the tuple handles are collected into the result vec. A malformed descriptor or a non-scalar
/// (unorderable) KEY shape yields the EMPTY vec (the never-trap total). The result is a `List (Tuple k v)` the
/// front-end consumes like any list of pairs.
pub(crate) fn op_map_to_list(map: Handle, desc: &[u8]) -> Handle {
    let Some(descriptor) = decode_descriptor(desc) else {
        return op_vec_empty();
    };
    let key_ix = match resolve_shape(&descriptor, descriptor.root) {
        Some(Shape::Map(k, _v)) => *k,
        _ => return op_vec_empty(),
    };
    let Some(entries) = map_entries_canonical(&descriptor, map, key_ix) else {
        return op_vec_empty(); // a non-scalar key shape is unorderable — decline to the empty list
    };
    let arr = op_arr_alloc(entries.len() as u32);
    for (i, &(k, v)) in entries.iter().enumerate() {
        // A fresh 2-element tuple `[key, value]` — the `(Tuple k v)` representation. Each component is
        // BORROWED from the map, so `dup` it: the entry tuple co-owns a reference alongside the map. Both
        // components are stored ALREADY BOXED (the CHAMP holds boxed handles), matching a tuple's slots.
        let entry = op_arr_alloc(2);
        op_dup(k);
        op_arr_set(entry, 0, k);
        op_dup(v);
        op_arr_set(entry, 1, v);
        op_arr_set(arr, i as u32, entry); // the tuple handle is owned by `arr` (moved in, no dup)
    }
    op_vec_of_arr(arr) // consumes the arr, yields the List (Tuple k v) handle
}

/// The NON-PROGRESS cap on the value walk — bounds a MALFORMED descriptor whose `Ref`/`Named` chain
/// cycles WITHOUT ever consuming a heap node (e.g. `Ref → Ref`, or `Named → Ref → Named …`), which would
/// otherwise spin the iterative walk forever building nothing. It counts only CONSECUTIVE non-consuming
/// transitions (`Ref` and `Named` both keep the SAME value `h`); it RESETS to 0 on any descent into a
/// child node (Tuple/List/Record/Sum reach a DIFFERENT heap node via `arr-get`/`sum-payload`, so they
/// make progress and cannot cycle on a well-formed acyclic value). It is therefore NOT a value-DEPTH
/// limit: because the walk is ITERATIVE (an explicit heap work stack — see `encode_value`), a genuinely
/// deep value (a long list, a deep tree) is bounded only by heap, never by the ~4.5 k-frame native/wasm
/// call stack a recursive walker would overflow. A real descriptor's `Ref`/`Named` runs are O(1) between
/// consuming steps, so this cap never fires on a well-formed value however deep.
pub(crate) const ENCODE_REF_CYCLE_CAP: u32 = 100_000;

/// One unit of pending work on the iterative encode's explicit stack. Modelled directly on the recursive
/// walk it replaces (below, as `encode_value_recursive` in the tests) so the SEQUENCE of `DocBuilder`
/// leaf/struct pushes — and therefore the document bytes — is IDENTICAL. `'d` borrows the descriptor's
/// interned names (the head/field/type strings), so no name is cloned.
// `'static` (no borrow of the descriptor) so the `work` stack can be REUSED from a thread-local across
// encodes (grow-once, like `ENCODE_OUT`/`ENCODE_BUILDER`) instead of a fresh heap Vec per call — the
// `work` stack grows O(depth) for a deep value (each container's assembler stays on the stack during
// child descent), so a fresh Vec's grow-chain cost O(log depth) reallocs PER encode. The three formerly
// borrowed fields (a record field's key `&str`, a `Named`'s type name `&str`, a `Framed`'s `&TypeNode`)
// are re-derived from `desc` at PROCESS time via the OWNING shape's table index — the name leaf is still
// built at process time, so emission order (byte-exactness) is unchanged.
pub(crate) enum EncodeWork {
    /// Dispatch on the shape of value `h` at table entry `shape_ix`; leaf shapes emit + produce one
    /// result, container shapes emit their head eagerly then push children (in reverse) + an assembler.
    /// `refs` = consecutive non-consuming `Ref`/`Named` hops taken to reach here (reset on child descent).
    Visit { h: Handle, shape_ix: u32, refs: u32 },
    /// A record FIELD: emit the key leaf+atom (BEFORE the field value, matching the recursive per-field
    /// order), then queue the value visit and a `Pair` assembler. The key `&str` is re-derived at process
    /// time from `desc.table[rec_ix]` (the `Shape::Record`) at `field_ix` — no borrow held on the stack.
    VisitField {
        h: Handle,
        shape_ix: u32,
        rec_ix: u32,
        field_ix: u32,
    },
    /// Assemble `list([head_s, <the top `nkids` results in child order>])` — the tuple/list/record/sum body.
    List { head_s: u32, nkids: usize },
    /// Assemble the `(: value Type)` frame: pop the inner value, emit the type-name leaf+atom AFTER it
    /// (matching the recursive order), then `list([colon_s, value, tname_s])`. The name `&str` is
    /// re-derived at process time from `desc.table[named_ix]` (the `Shape::Named`).
    Named { colon_s: u32, named_ix: u32 },
    /// Assemble a `(: value <type-node>)` frame — like `Named` but the type is an arbitrary (possibly
    /// NESTED) type node, re-derived at process time from `desc.table[framed_ix]` (the `Shape::Framed`).
    /// Pop the inner value, `render_type_node` the type, then `list([colon_s, value, type_node])`.
    Framed { colon_s: u32, framed_ix: u32 },
    /// Assemble one record field: pop the field value, `list([eq, katom, fval])` where `eq` is the M2
    /// FieldPair ctor-head atom (pre-M2 it was the `=` name atom). `eq` and the key atom are built PRE-order
    /// (before the value visit) so the leaf/struct pool matches canon's pre-order first-encounter — see
    /// `VisitField`. Structure `(FieldPair name value)`.
    Pair { eq: u32, katom: u32 },
    /// A MAP entry (M2): build the FieldPair ctor-head atom PRE-order (before the k/v subtrees, for canon
    /// first-encounter — the FieldPair leaf dedups), then queue the value + key visits and a `MapPair`
    /// assembler. Mirrors `VisitField` (a map key is a VALUE, so it is Visited, not a pre-built name atom).
    VisitMapEntry {
        k: Handle,
        v: Handle,
        key_shape: u32,
        val_shape: u32,
    },
    /// Assemble one MAP entry `(FieldPair key value)`: the key result is directly below the value result on
    /// `out` (key Visited before value). Pop value then key, build `list([fp_s, key, value])`. `fp_s` is the
    /// FieldPair ctor-head atom built PRE-order in `VisitMapEntry`.
    MapPair { fp_s: u32 },
    /// Assemble `(map (k1 v1) … (kn vn))` — the canonical Map value form. Pops the top `nentries` pair
    /// results (already in canonical KEY order), under the pre-emitted `map` `head_s`.
    MapOf { head_s: u32, nentries: usize },
}

/// Walk the runtime value `root_h` under table entry `root_shape`, appending its value-form structs to
/// `b`; return the root struct index. A `Ref` follows the table (where a recursive value re-enters the
/// sum's shape). `None` on a malformed descriptor / out-of-range disc / unrenderable shape / a `Ref`/
/// `Named` cycle exceeding `ENCODE_REF_CYCLE_CAP`. BORROWS the value; caller drops the root afterward.
///
/// ITERATIVE (an explicit heap work stack, not native recursion) — a deep recursive value (a long linked
/// list, a deep tree: the very shapes this op exists to encode) would overflow the ~4.5 k-frame native /
/// wasm call stack of the recursive walker and ABORT the guest, rather than honour the op's decline
/// contract. Same discipline as `op_drop`'s iterative free cascade. The push order reproduces the
/// recursive walk's leaf/struct emission EXACTLY, so the document is byte-identical (guarded by
/// `value_encode_iterative_matches_recursive_reference`). `refs` counts only consecutive non-consuming
/// `Ref`/`Named` hops (reset on every child descent), so the cap bounds a malformed cycle WITHOUT
/// limiting a well-formed value's genuine depth.
pub(crate) fn encode_value(
    desc: &Descriptor,
    b: &mut DocBuilder,
    out: &mut Vec<u32>,
    work: &mut Vec<EncodeWork>,
    root_h: Handle,
    root_shape: u32,
) -> Option<u32> {
    // `out` (completed struct indices) and `work` (the pending-task stack) are both REUSED thread-local
    // buffers, passed in by the caller (cleared here, capacity retained across encodes). `EncodeWork` is
    // now `'static` (no descriptor borrow — the key/name/type-node are re-derived from `desc` at process
    // time), so the `work` stack reuses like `out`/the builder instead of a fresh Vec per encode.
    out.clear();
    work.clear();
    work.push(EncodeWork::Visit {
        h: root_h,
        shape_ix: root_shape,
        refs: 0,
    });
    while let Some(task) = work.pop() {
        match task {
            EncodeWork::Visit { h, shape_ix, refs } => {
                if refs > ENCODE_REF_CYCLE_CAP {
                    return None; // a Ref/Named chain that never consumes a node — malformed descriptor cycle
                }
                match desc.table.get(shape_ix as usize)? {
                    Shape::Ref(target) => {
                        // Non-consuming: same `h`, no heap node reached → count toward the cycle cap.
                        work.push(EncodeWork::Visit {
                            h,
                            shape_ix: *target,
                            refs: refs + 1,
                        });
                    }
                    Shape::Int => {
                        let l = b.int_leaf(op_get_int(h));
                        out.push(b.atom(l));
                    }
                    Shape::BigInt => {
                        // Read the arbitrary-precision value via `unbox_bigint` (NOT `op_get_int`, which
                        // caps at i64) and render it as the SAME `KIND_INT` leaf — the leaf is already
                        // sign + arbitrary-width big-endian magnitude, so no new wire kind is needed.
                        let l = b.bigint_leaf(&unbox_bigint(h));
                        out.push(b.atom(l));
                    }
                    Shape::Rational => {
                        // seq-204 NATIVE rational (head+children): the list `(KIND_RATIONAL <num> <den>)` —
                        // the payloadless Rational tag head + two normalized BigInt components as ordinary Int
                        // leaves. Self-typing (bare). Byte-identical to Builder::rational + rust emit by
                        // construction. Children are known BigInts → build directly (no work-queue recursion).
                        let (num, den) = unbox_rational(h);
                        let tag_leaf = b.ctor_leaf(doc::KIND_RATIONAL);
                        let tag = b.atom(tag_leaf);
                        let num_leaf = b.bigint_leaf(&num);
                        let num_atom = b.atom(num_leaf);
                        let den_leaf = b.bigint_leaf(&den);
                        let den_atom = b.atom(den_leaf);
                        out.push(b.list_head_tail(tag, &[num_atom, den_atom]));
                    }
                    Shape::Bool => {
                        let l = b.bool_leaf(op_get_bool(h));
                        out.push(b.atom(l));
                    }
                    Shape::Char => {
                        // A char value is an immediate int (the code-point) — read it with `op_get_int` and
                        // emit a `KIND_CHAR` leaf (rendered as a `#\c` char literal on decode), mirroring how
                        // `Bool` emits `KIND_BOOL_*`. A code-point that is not a Unicode scalar is a malformed
                        // Char value → decline the encode (like a non-finite Float).
                        let c = char::from_u32(op_get_int(h) as u32)?;
                        let l = b.char_leaf(c);
                        out.push(b.atom(l));
                    }
                    Shape::Unit => {
                        let l = b.name_leaf("unit");
                        out.push(b.atom(l));
                    }
                    Shape::Str => {
                        // A String value may be a ROPE (a `String.concat`/`String.at`-slice builds concat/
                        // slice nodes, NOT a flat leaf), so MATERIALIZE it to a leaf first (`bytes_flatten`,
                        // iterative so no deep-rope stack overflow; content-preserving so unobservable on a
                        // borrowed/shared value) before reading `raw` — exactly as `Shape::Bytes` does. A
                        // flat string leaf stores its UTF-8 bytes in `raw` and flatten is a no-op there.
                        // Without the flatten a runtime string (a concat/slice) rendered its raw HANDLE
                        // bytes (garbage), losing the content.
                        bytes_flatten(h);
                        // Build the leaf DIRECTLY from the flattened node's borrowed raw slice — `str_leaf`
                        // stores it as an inline `Raw` for a short string (no `to_vec`). `with_node` returns
                        // the leaf index while the borrow is live; a null/missing node reads as empty.
                        let l = with_node(h, None, |n| Some(b.str_leaf(n.raw.as_slice())))
                            .unwrap_or_else(|| b.str_leaf(&[]));
                        out.push(b.atom(l));
                    }
                    Shape::Symbol => {
                        // A Symbol shares the String rep (materialize a rope first, exactly as `Shape::Str`)
                        // but renders its CONSTRUCTION form `((. Symbol of) "text")` — the #7694 Member-access
                        // member-compound, byte-matching `const_value_ast`:1979 (`list([member(Symbol,of),
                        // Str(text)])`) + the rust backend — NOT the bare `Str` leaf `"text"` (ambiguous with a
                        // real String, divergent from rust). All parts are known → build the fixed doc DIRECTLY
                        // (no work-queue recursion), like `Shape::Rational`. Inner `(. Symbol of)` =
                        // `list([KIND_MEMBER head, name "Symbol", name "of"])`; outer = `list([member, str])`.
                        bytes_flatten(h);
                        let str_leaf = with_node(h, None, |n| Some(b.str_leaf(n.raw.as_slice())))
                            .unwrap_or_else(|| b.str_leaf(&[]));
                        let str_atom = b.atom(str_leaf);
                        let member_kind = b.ctor_leaf(doc::KIND_MEMBER);
                        let member_atom = b.atom(member_kind);
                        let sym_leaf = b.name_leaf("Symbol");
                        let sym_atom = b.atom(sym_leaf);
                        let of_leaf = b.name_leaf("of");
                        let of_atom = b.atom(of_leaf);
                        let member = b.list(&[member_atom, sym_atom, of_atom]);
                        out.push(b.list(&[member, str_atom]));
                    }
                    Shape::Bytes => {
                        // A Bytes value may be a ROPE (concat/slice nodes) — materialize it to a leaf
                        // (iterative `bytes_flatten`, so no deep-rope stack overflow; content-preserving so
                        // UNOBSERVABLE even on a borrowed/shared value), then read the leaf's raw and emit a
                        // KIND_BYTES leaf. A leaf is already flat (flatten is a no-op there).
                        bytes_flatten(h);
                        let l = with_node(h, None, |n| Some(b.bytes_leaf(n.raw.as_slice())))
                            .unwrap_or_else(|| b.bytes_leaf(&[]));
                        out.push(b.atom(l));
                    }
                    Shape::Float => {
                        // Convert the runtime f64 to the codec's EXACT decimal (KIND_FLOAT). A NON-FINITE
                        // float (nan/inf) has no exact-decimal form, so `float_leaf` emits its dedicated
                        // payloadless word-form leaf (FloatNan/FloatInf) rather than declining — a non-finite
                        // float in a compound crosses the boundary instead of collapsing the whole compound.
                        let l = b.float_leaf(op_get_float(h))?;
                        out.push(b.atom(l));
                    }
                    Shape::Float32 => {
                        // Read the 4-byte Float32 and render the f32's OWN shortest decimal (not a promoted
                        // f64's). A non-finite f32 emits the same payloadless word-form leaf, like Float64.
                        let l = b.float32_leaf(op_get_float32(h))?;
                        out.push(b.atom(l));
                    }
                    Shape::Tuple(elems) => {
                        if elems.is_empty() {
                            // An EMPTY `(Tuple)`-typed value renders the HEADED empty tuple `(tuple)`, NOT
                            // `unit` (Ruling-B: `unit` and `(Tuple)` are DISTINCT types — 05-compound:9232-9239 —
                            // and a `(Tuple)`-typed value MUST render `(tuple)`, matching the rust
                            // cdz_render_expr path + the wasm const path). The physical handle is `imm_unit`
                            // (`op_arr_alloc(0)`) for BOTH a Unit and an empty-tuple value — they share one
                            // runtime handle, so the render MUST be driven by the SHAPE DESCRIPTOR, not the
                            // handle: `Shape::Unit` → `unit`, `Shape::Tuple([])` → `(tuple)`. Emit the same
                            // `tuple` head as the non-empty arm with ZERO children (`list_head_tail` over an
                            // empty slice yields the bare `(tuple)`). Paired with v-wasm-opt's shape_of change
                            // to emit `ShapeNode::Tuple([])` (not Unit) for an empty `Ty::Tuple` — BOTH needed.
                            let head = b.ctor_leaf(doc::KIND_TUPLE_CTOR);
                            let head_s = b.atom(head);
                            work.push(EncodeWork::List { head_s, nkids: 0 });
                        } else {
                            // TOTALITY: the descriptor declares `elems.len()` fields; verify the actual node
                            // has at least that arity BEFORE any `op_arr_get` (which TRAPS on OOB / an
                            // immediate). A well-formed descriptor always matches, but a malformed one must
                            // DECLINE (`None`) per this op's contract, not trap the guest.
                            if (op_arr_len(h) as usize) < elems.len() {
                                return None;
                            }
                            let head = b.ctor_leaf(doc::KIND_TUPLE_CTOR);
                            let head_s = b.atom(head);
                            work.push(EncodeWork::List {
                                head_s,
                                nkids: elems.len(),
                            });
                            // Push children in REVERSE so the LIFO stack visits them left→right; each
                            // completes to one `out` entry, in child order under the `List` assembler.
                            // A child is a DIFFERENT heap node (arr-get) → progress → reset `refs` to 0.
                            for (i, &es) in elems.iter().enumerate().rev() {
                                work.push(EncodeWork::Visit {
                                    h: op_arr_get(h, i as u32),
                                    shape_ix: es,
                                    refs: 0,
                                });
                            }
                        }
                    }
                    Shape::List(elem) => {
                        // A Cadenza `List` is an RRB `vec` (NOT a flat `arr` — a tuple/record is the arr),
                        // so read its length + elements with the `vec-*` ops. (`arr-len`/`arr-get` on a vec
                        // handle read the root node's arity, not the logical element count — the bug that
                        // rendered only the first element.)
                        let (elem, n) = (*elem, op_vec_len(h));
                        let head = b.ctor_leaf(doc::KIND_LIST_CTOR);
                        let head_s = b.atom(head);
                        work.push(EncodeWork::List {
                            head_s,
                            nkids: n as usize,
                        });
                        for i in (0..n).rev() {
                            work.push(EncodeWork::Visit {
                                h: op_vec_get(h, i),
                                shape_ix: elem,
                                refs: 0,
                            });
                        }
                    }
                    Shape::Record(fields) => {
                        // TOTALITY (as `Tuple`): a record is an arr of field values; verify the node's
                        // arity covers the descriptor's field count before any trapping `op_arr_get`.
                        if (op_arr_len(h) as usize) < fields.len() {
                            return None;
                        }
                        let head = b.ctor_leaf(doc::KIND_RECORD_CTOR);
                        let head_s = b.atom(head);
                        work.push(EncodeWork::List {
                            head_s,
                            nkids: fields.len(),
                        });
                        for (i, (_k, fs)) in fields.iter().enumerate().rev() {
                            work.push(EncodeWork::VisitField {
                                h: op_arr_get(h, i as u32),
                                shape_ix: *fs,
                                rec_ix: shape_ix, // the Record shape's own table index (re-derives the key)
                                field_ix: i as u32,
                            });
                        }
                    }
                    Shape::Sum(variants) => {
                        // `sum_disc_shaped` (not `op_sum_disc`): an all-nullary sum nested in a compound
                        // reaches render as an Int IMMEDIATE (box-int of a small disc → imm_int), and
                        // `op_sum_disc`→0 would render the FIRST variant for EVERY value (SOUNDNESS #43 render
                        // sibling of witness 4 — a runtime `(tuple (Tri.Hi unit) 5)` else renders `(Tri.Lo …)`).
                        let disc = sum_disc_shaped(h) as usize;
                        let (head, payload_shape) = variants.get(disc)?;
                        let head_leaf = b.name_leaf(head);
                        let head_s = b.atom(head_leaf);
                        let payload_shape = *payload_shape;
                        let payload_h = op_sum_payload(h);
                        // A MULTI-payload variant's payload is a `Spread`: the payload handle is the tuple
                        // arr of the boxed payloads, and the variant renders `(Variant p0 p1 …)` — the
                        // elements FLATTENED directly under the head, NOT wrapped in a `tuple` form. So
                        // splice the tuple's elements as the variant's children (one `arr-get` per element,
                        // like the `Tuple` walk) rather than visiting the single tuple shape.
                        if let Some(Shape::Spread(elems)) = desc.table.get(payload_shape as usize) {
                            let elems = elems.clone();
                            // TOTALITY (as `Tuple`): the payload arr must have ≥ the Spread's element count
                            // before any trapping `op_arr_get` — a malformed descriptor DECLINES, not traps.
                            if (op_arr_len(payload_h) as usize) < elems.len() {
                                return None;
                            }
                            work.push(EncodeWork::List {
                                head_s,
                                nkids: elems.len(),
                            });
                            for (i, &es) in elems.iter().enumerate().rev() {
                                work.push(EncodeWork::Visit {
                                    h: op_arr_get(payload_h, i as u32),
                                    shape_ix: es,
                                    refs: 0,
                                });
                            }
                        } else {
                            // A nullary variant's payload shape is `Unit` → the bare `unit` atom (the
                            // `(Variant unit)` form); a single-payload variant reaches its payload via
                            // `sum-payload` — a DIFFERENT heap node → progress → reset `refs`.
                            work.push(EncodeWork::List { head_s, nkids: 1 });
                            work.push(EncodeWork::Visit {
                                h: payload_h,
                                shape_ix: payload_shape,
                                refs: 0,
                            });
                        }
                    }
                    Shape::Named(_name, inner) => {
                        // The `(: <value> <Type>)` value-form frame — same `h`, no node consumed → count.
                        let inner = *inner;
                        let colon = b.name_leaf(":");
                        let colon_s = b.atom(colon);
                        work.push(EncodeWork::Named {
                            colon_s,
                            named_ix: shape_ix, // re-derives `name` from desc.table[named_ix] at process time
                        });
                        work.push(EncodeWork::Visit {
                            h,
                            shape_ix: inner,
                            refs: refs + 1,
                        });
                    }
                    Shape::Framed(_type_node, inner) => {
                        // The `(: <value> <type-node>)` frame — an arbitrary (possibly nested) type node.
                        // Same `h`, no node consumed → count toward the ref cap.
                        let inner = *inner;
                        let colon = b.name_leaf(":");
                        let colon_s = b.atom(colon);
                        work.push(EncodeWork::Framed {
                            colon_s,
                            framed_ix: shape_ix, // re-derives the TypeNode from desc.table[framed_ix]
                        });
                        work.push(EncodeWork::Visit {
                            h,
                            shape_ix: inner,
                            refs: refs + 1,
                        });
                    }
                    Shape::Set(elem) => {
                        // A Set renders `((. Set of) (list e1 … en))` with elements in CANONICAL key-VALUE
                        // order. The CHAMP iterates hash order, so collect + SORT by the element's canonical
                        // scalar value (matching the compiler's `const_key_order`). Only a SCALAR element is
                        // orderable/encodable; a non-scalar element shape declines (as `const_key_order` does).
                        // M2 head-first: a Set is a FLAT `(Ctor(Set) e1 … en)` — the Set ctor-leaf head atom +
                        // the sorted elements as direct children (NOT the pre-M2 `((. Set of) (list e…))`
                        // member-access-over-list form). Head interned PRE-order (canon first-encounter), then
                        // the elements visited in canonical order — reuse the plain `List` assembler.
                        let elem = *elem;
                        let sorted = set_elements_canonical(desc, h, elem)?;
                        let head = b.ctor_leaf(doc::KIND_SET_CTOR);
                        let head_s = b.atom(head);
                        work.push(EncodeWork::List {
                            head_s,
                            nkids: sorted.len(),
                        });
                        // Push in REVERSE so the LIFO stack encodes them in canonical order onto `out`. Each
                        // element is a DISTINCT heap node (a set member) → progress → reset `refs`.
                        for &e in sorted.iter().rev() {
                            work.push(EncodeWork::Visit {
                                h: e,
                                shape_ix: elem,
                                refs: 0,
                            });
                        }
                    }
                    Shape::Map(key, val) => {
                        // M2 head-first: a Map is `(Ctor(Map) (FieldPair k1 v1) … (FieldPair kn vn))` with
                        // entries in CANONICAL KEY order (CHAMP iterates hash order → collect + SORT by the
                        // key's canonical scalar value; only a SCALAR key is orderable/encodable, the value is
                        // any encodable shape). Map ctor head EAGER (pre-order); each entry is a FieldPair
                        // triple built by `VisitMapEntry` (which interns the FieldPair head PRE-order, before
                        // the k/v subtrees, for canon first-encounter — mirroring the record `VisitField`).
                        let (key, val) = (*key, *val);
                        let entries = map_entries_canonical(desc, h, key)?;
                        let map_head = b.ctor_leaf(doc::KIND_MAP_CTOR);
                        let head_s = b.atom(map_head);
                        work.push(EncodeWork::MapOf {
                            head_s,
                            nentries: entries.len(),
                        });
                        // Push entries in REVERSE (so `VisitMapEntry` pops in canonical order); each entry's
                        // handler builds its FieldPair head + visits key then value.
                        for &(k, v) in entries.iter().rev() {
                            work.push(EncodeWork::VisitMapEntry {
                                k,
                                v,
                                key_shape: key,
                                val_shape: val,
                            });
                        }
                    }
                    Shape::Spread(elems) => {
                        // A `Spread` is ONLY reached inline by the `Sum` walk (which splices its elements
                        // under the variant head). Visited DIRECTLY (a malformed descriptor that roots or
                        // nests a Spread outside a Sum variant), render it as an ordinary `tuple` — a safe
                        // fallback that never traps, matching the `Tuple` walk.
                        if elems.is_empty() {
                            let l = b.name_leaf("unit");
                            out.push(b.atom(l));
                        } else {
                            let elems = elems.clone();
                            let head = b.name_leaf("tuple");
                            let head_s = b.atom(head);
                            work.push(EncodeWork::List {
                                head_s,
                                nkids: elems.len(),
                            });
                            for (i, &es) in elems.iter().enumerate().rev() {
                                work.push(EncodeWork::Visit {
                                    h: op_arr_get(h, i as u32),
                                    shape_ix: es,
                                    refs: 0,
                                });
                            }
                        }
                    }
                }
            }
            EncodeWork::VisitField {
                h,
                shape_ix,
                rec_ix,
                field_ix,
            } => {
                // Key leaf+atom emitted BEFORE the field value; the `Pair` assembler runs AFTER it. The
                // field value is a fresh child node (arr-get already applied) → a new walk, `refs` 0.
                // Re-derive the key from the owning `Shape::Record` at `field_ix` (no borrow on the stack).
                let key = match desc.table.get(rec_ix as usize) {
                    Some(Shape::Record(fields)) => fields.get(field_ix as usize).map(|(k, _)| &**k),
                    _ => None,
                }?;
                // CANON CONVERGENCE: emit the `=` head atom, THEN the key atom, BOTH before descending into
                // the field value — matching canon's pre-order first-encounter (a field triple's children are
                // `[=, name, value]`, so canon interns `=` first, then name, then the value subtree). The
                // pre-Phase-B code built `=` in the `Pair` assembler AFTER the value, which interned `=` LATE
                // and made value-encode non-canonical vs `codec::encode(canon(tree))`. See canon.rs `visit`.
                // M2: a record field is `(FieldPair name value)` — the FieldPair ctor-leaf head + the key
                // name atom + the value (was the `=` name head pre-M2). Structure unchanged (head + 2 kids).
                let eq_leaf = b.ctor_leaf(doc::KIND_FIELD_PAIR);
                let eq = b.atom(eq_leaf);
                let kname = b.name_leaf(key);
                let katom = b.atom(kname);
                work.push(EncodeWork::Pair { eq, katom });
                work.push(EncodeWork::Visit {
                    h,
                    shape_ix,
                    refs: 0,
                });
            }
            EncodeWork::List { head_s, nkids } => {
                let base = out.len().checked_sub(nkids)?;
                // Build the list's range directly in the pool: head + the completed children in `out[base..]`
                // (already in child order — see the reverse push), no temporary Vec.
                let s = b.list_head_tail(head_s, &out[base..]);
                out.truncate(base);
                out.push(s);
            }
            EncodeWork::Named { colon_s, named_ix } => {
                let value = out.pop()?;
                // Re-derive the type name from the owning `Shape::Named` (no borrow on the stack).
                let name = match desc.table.get(named_ix as usize) {
                    Some(Shape::Named(name, _)) => &**name,
                    _ => return None,
                };
                let tname = b.name_leaf(name);
                let tname_s = b.atom(tname);
                out.push(b.list(&[colon_s, value, tname_s]));
            }
            EncodeWork::Framed { colon_s, framed_ix } => {
                let value = out.pop()?;
                // Re-derive the TypeNode from the owning `Shape::Framed` (no borrow on the stack).
                let type_node = match desc.table.get(framed_ix as usize) {
                    Some(Shape::Framed(tn, _)) => tn,
                    _ => return None,
                };
                let type_s = b.render_type_node(type_node);
                out.push(b.list(&[colon_s, value, type_s]));
            }
            EncodeWork::Pair { eq, katom } => {
                // Record field value-output form is the `(= name value)` ascription (record-type Phase B
                // full-symmetry migration — literals, patterns, AND value-output all spell `(= name value)`;
                // operator-ruled 2026-08-09). The `=` and key atoms were built PRE-order (in `VisitField`,
                // before the value) so the leaf/struct pool matches canon first-encounter; here we only
                // assemble the list once the field value result is on `out`.
                let fval = out.pop()?;
                out.push(b.list(&[eq, katom, fval]));
            }
            EncodeWork::VisitMapEntry {
                k,
                v,
                key_shape,
                val_shape,
            } => {
                // M2 map entry `(FieldPair key value)`: intern the FieldPair ctor-head atom PRE-order (before
                // the k/v subtrees, so the leaf/struct pool matches canon first-encounter — the FieldPair leaf
                // dedups across entries), then visit key BEFORE value (key below value on `out`, as `MapPair`
                // relies on). Mirrors the record `VisitField`.
                let fp = b.ctor_leaf(doc::KIND_FIELD_PAIR);
                let fp_s = b.atom(fp);
                work.push(EncodeWork::MapPair { fp_s });
                work.push(EncodeWork::Visit {
                    h: v,
                    shape_ix: val_shape,
                    refs: 0,
                });
                work.push(EncodeWork::Visit {
                    h: k,
                    shape_ix: key_shape,
                    refs: 0,
                });
            }
            EncodeWork::MapPair { fp_s } => {
                // Key was Visited before value, so on `out` the value is on top, key directly below.
                let val = out.pop()?;
                let key = out.pop()?;
                out.push(b.list(&[fp_s, key, val]));
            }
            EncodeWork::MapOf { head_s, nentries } => {
                // The top `nentries` results are the `(FieldPair key value)` entries in canonical KEY order.
                let base = out.len().checked_sub(nentries)?;
                let s = b.list_head_tail(head_s, &out[base..]);
                out.truncate(base);
                out.push(s);
            }
        }
    }
    // A well-formed walk leaves exactly the one root struct index.
    match out.len() {
        1 => out.pop(),
        _ => None,
    }
}

/// Render the runtime value `h` to its canonical binary-AST value-form document, under the shape
/// descriptor `desc` (compiler-baked bytes; see the module note). `None` on a malformed descriptor or an
/// unrenderable shape (a not-yet-supported Float/Str/Bytes payload). Does NOT drop `h` — the caller
/// (the escape `encode`) owns the release point.
pub(crate) fn op_value_encode_form(h: Handle, desc: &[u8]) -> Option<Vec<u8>> {
    // Decode the descriptor via the single-entry cache: on a hit (the same escape site's bytes as last
    // call — the common loop case) the decode + its Vec/String allocs are skipped entirely. On a miss,
    // decode once and store `(bytes.to_vec(), descriptor)` as the new entry. The whole encode runs while
    // the cache cell is borrowed, so the cached `Descriptor` is used in place (no clone). `decode_
    // descriptor` is a pure function of the bytes, so a byte-equal hit yields the identical descriptor.
    DESCRIPTOR_CACHE.with(|dcell| {
        let mut slot = dcell.borrow_mut();
        // Refresh the entry on a miss (empty, or different bytes than cached).
        if slot.as_ref().map(|(bytes, _)| bytes.as_slice()) != Some(desc) {
            let decoded = decode_descriptor(desc)?;
            *slot = Some((desc.to_vec(), decoded));
        }
        let descriptor = &slot.as_ref()?.1;
        // Reuse the thread-local builder + `out` + `work` stacks — `reset()`/`clear()` empties them but
        // retains capacity, so the leaf/struct/child-pool + result-stack + work-stack growth is paid ONCE
        // (not per encode). The result bytes are identical either way; the reuse is a pure allocation
        // optimisation (see `ENCODE_BUILDER`/`ENCODE_OUT`/`ENCODE_WORK`). The cells are distinct → never alias.
        ENCODE_BUILDER.with(|bcell| {
            ENCODE_OUT.with(|ocell| {
                ENCODE_WORK.with(|wcell| {
                    let b = &mut *bcell.borrow_mut();
                    let out = &mut *ocell.borrow_mut();
                    let work = &mut *wcell.borrow_mut();
                    b.reset();
                    let root = encode_value(descriptor, b, out, work, h, descriptor.root)?;
                    Some(b.finish(root))
                })
            })
        })
    })
}

// ─── value-decode (heap idx 90): the inverse of value-encode ──────────────────────────────
// Parse a canonical `cadenza-ast` value-form document (the exact bytes `op_value_encode_form` /
// `DocBuilder::finish` produces) and, guided by the SAME shape descriptor `value-encode` reads,
// CONSTRUCT a fresh owned heap value. Descriptor-guided + name/tag-free (field names / variant tags come
// from the descriptor, matched against the document, never invented). TOTAL: any shape/format mismatch
// returns `Handle::NULL` (0) — NEVER traps (the decode analogue of `op_value_encode_form`'s
// malformed-descriptor → empty-Bytes decline). See runtime.wit idx 90.

/// A parsed document leaf — the read-side mirror of `DocLeaf` (see `DocBuilder`). Owns its bytes so the
/// walk can build heap values without holding a borrow on the source Vec.
pub(crate) enum ParsedLeaf {
    /// (negative, big-endian magnitude, leading-zeros-stripped) — covers both `IntScalar` and `Int` on the
    /// wire (they encode to the identical `KIND_INT` framing); the walk picks i64 vs BigInt by SHAPE.
    Int(bool, Vec<u8>),
    Bool(bool),
    /// A Unicode-scalar Char leaf (`KIND_CHAR`) — the code-point; `decode_value` boxes it as an int.
    Char(char),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Bytes(Vec<u8>),
    /// (negative, exponent, big-endian base-256 significand) — the `KIND_FLOAT` exact-decimal parts.
    Float(bool, i64, Vec<u8>),
    /// The non-finite float value NaN (`KIND_FLOAT_NAN`) — `decode_value` boxes it as `f64::NAN`.
    FloatNan,
    /// A non-finite float infinity (`KIND_FLOAT_POS_INF`/`KIND_FLOAT_NEG_INF`); `negative` picks the sign.
    FloatInf(bool),
    /// An M2 payloadless ctor-head leaf — its `doc::KIND_*_CTOR`/`KIND_FIELD_PAIR`/`KIND_MEMBER` byte (20-26).
    /// The head atom of a native compound value's list; `doc_atom_ctor` reads its kind for the decode arms.
    Ctor(u8),
}

/// A parsed document struct — the read-side mirror of `DocStruct`. A `List`'s children are struct indices
/// (owned Vec here rather than a pooled range, since the reader has no shared child pool).
pub(crate) enum ParsedStruct {
    Atom(u32),      // → leaves[leaf_id]
    List(Vec<u32>), // → child struct indices
}

/// The parsed document: leaves + structs + the root struct index. `decode_value` walks it from `root`.
pub(crate) struct ParsedDoc {
    pub(crate) leaves: Vec<ParsedLeaf>,
    pub(crate) structs: Vec<ParsedStruct>,
    pub(crate) root: u32,
}

/// Read an unsigned LEB128 from `d` at `*pos`, advancing `*pos`. `None` on truncation or a >10-byte
/// (u64-overflowing) encoding — a malformed document declines, never panics.
pub(crate) fn doc_read_leb(d: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = *d.get(*pos)?;
        *pos += 1;
        if shift >= 64 {
            return None; // overflow — malformed
        }
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Some(value)
}

/// Read `len` bytes from `d` at `*pos`, advancing `*pos`. `None` on truncation.
pub(crate) fn doc_read_bytes<'a>(d: &'a [u8], pos: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = pos.checked_add(len)?;
    let slice = d.get(*pos..end)?;
    *pos = end;
    Some(slice)
}

/// Parse a value-form document (the inverse of `DocBuilder::finish`) into a `ParsedDoc`. Total: any
/// malformed framing (bad header, truncation, unknown kind/tag, out-of-range index) returns `None`.
pub(crate) fn parse_doc(d: &[u8]) -> Option<ParsedDoc> {
    let mut pos = 0usize;
    // Header.
    let header = doc_read_bytes(d, &mut pos, doc::SCHEMA_HEADER.len())?;
    if header != doc::SCHEMA_HEADER {
        return None;
    }
    // Leaves.
    let leaf_count = doc_read_leb(d, &mut pos)? as usize;
    let mut leaves = Vec::with_capacity(leaf_count.min(1 << 16));
    for _ in 0..leaf_count {
        let kind = *d.get(pos)?;
        pos += 1;
        let leaf = match kind {
            // KIND_INT_POS_DEC (0) / neg (0+3=3): [maglen LEB][BE mag].
            doc::KIND_INT_POS_DEC | 3 => {
                let neg = kind == doc::KIND_INT_POS_DEC + 3;
                let maglen = doc_read_leb(d, &mut pos)? as usize;
                let mag = doc_read_bytes(d, &mut pos, maglen)?.to_vec();
                ParsedLeaf::Int(neg, mag)
            }
            doc::KIND_FLOAT => {
                let neg = *d.get(pos)? != 0;
                pos += 1;
                let eb = doc_read_bytes(d, &mut pos, 8)?;
                let mut ebuf = [0u8; 8];
                ebuf.copy_from_slice(eb);
                let exp = i64::from_be_bytes(ebuf);
                let siglen = doc_read_leb(d, &mut pos)? as usize;
                let sig = doc_read_bytes(d, &mut pos, siglen)?.to_vec();
                ParsedLeaf::Float(neg, exp, sig)
            }
            // Non-finite floats — payloadless (no body to read), the inverse of the encode word-forms.
            doc::KIND_FLOAT_NAN => ParsedLeaf::FloatNan,
            doc::KIND_FLOAT_POS_INF => ParsedLeaf::FloatInf(false),
            doc::KIND_FLOAT_NEG_INF => ParsedLeaf::FloatInf(true),
            doc::KIND_STR => {
                let len = doc_read_leb(d, &mut pos)? as usize;
                ParsedLeaf::Str(doc_read_bytes(d, &mut pos, len)?.to_vec())
            }
            doc::KIND_BOOL_FALSE => ParsedLeaf::Bool(false),
            doc::KIND_BOOL_TRUE => ParsedLeaf::Bool(true),
            doc::KIND_CHAR => {
                // The scalar UTF-8-encoded (LEB len + 1-4 bytes), matching cadenza-ast codec's read_scalar:
                // read the body, parse as ONE Unicode scalar. A non-UTF-8 body or not-exactly-one-scalar
                // body is a malformed Char leaf → decline.
                let len = doc_read_leb(d, &mut pos)? as usize;
                let bytes = doc_read_bytes(d, &mut pos, len)?;
                let s = core::str::from_utf8(bytes).ok()?;
                let mut it = s.chars();
                let c = it.next()?;
                if it.next().is_some() {
                    return None; // more than one scalar in a char leaf — malformed
                }
                ParsedLeaf::Char(c)
            }
            doc::KIND_NAME => {
                let len = doc_read_leb(d, &mut pos)? as usize;
                ParsedLeaf::Name(doc_read_bytes(d, &mut pos, len)?.to_vec())
            }
            doc::KIND_BYTES => {
                let len = doc_read_leb(d, &mut pos)? as usize;
                ParsedLeaf::Bytes(doc_read_bytes(d, &mut pos, len)?.to_vec())
            }
            // M2 native-compound ctor-head kinds (20-27) — payloadless single kind byte (no body to read).
            doc::KIND_LIST_CTOR
            | doc::KIND_TUPLE_CTOR
            | doc::KIND_RECORD_CTOR
            | doc::KIND_MAP_CTOR
            | doc::KIND_SET_CTOR
            | doc::KIND_FIELD_PAIR
            | doc::KIND_MEMBER
            | doc::KIND_RATIONAL => ParsedLeaf::Ctor(kind), // 27: payloadless native-rational tag head (seq-204)
            _ => return None, // unknown kind — malformed
        };
        leaves.push(leaf);
    }
    // Structs.
    let struct_count = doc_read_leb(d, &mut pos)? as usize;
    let mut structs = Vec::with_capacity(struct_count.min(1 << 16));
    for _ in 0..struct_count {
        let tag = *d.get(pos)?;
        pos += 1;
        let s = match tag {
            doc::TAG_ATOM => {
                let id = doc_read_leb(d, &mut pos)? as u32;
                if id as usize >= leaves.len() {
                    return None; // dangling leaf index
                }
                ParsedStruct::Atom(id)
            }
            doc::TAG_LIST => {
                let len = doc_read_leb(d, &mut pos)? as usize;
                let mut kids = Vec::with_capacity(len.min(1 << 16));
                for _ in 0..len {
                    kids.push(doc_read_leb(d, &mut pos)? as u32);
                }
                ParsedStruct::List(kids)
            }
            _ => return None, // unknown tag — malformed
        };
        structs.push(s);
    }
    let root = doc_read_leb(d, &mut pos)? as u32;
    if root as usize >= structs.len() {
        return None; // dangling root
    }
    Some(ParsedDoc {
        leaves,
        structs,
        root,
    })
}

/// The single Atom leaf of a struct index, or `None` if that struct is a List (a shape/document mismatch
/// where a leaf was expected). Also range-checks the struct index.
pub(crate) fn doc_atom_leaf<'a>(doc: &'a ParsedDoc, struct_ix: u32) -> Option<&'a ParsedLeaf> {
    match doc.structs.get(struct_ix as usize)? {
        ParsedStruct::Atom(leaf_id) => doc.leaves.get(*leaf_id as usize),
        ParsedStruct::List(_) => None,
    }
}

/// The child struct indices of a List struct, or `None` if it is an Atom (mismatch). Range-checked.
pub(crate) fn doc_list_kids<'a>(doc: &'a ParsedDoc, struct_ix: u32) -> Option<&'a [u32]> {
    match doc.structs.get(struct_ix as usize)? {
        ParsedStruct::List(kids) => Some(kids),
        ParsedStruct::Atom(_) => None,
    }
}

/// The NAME-leaf text of an atom struct (a head/tag/name position), as `&str`. `None` if not a Name leaf
/// or not valid UTF-8.
pub(crate) fn doc_atom_name<'a>(doc: &'a ParsedDoc, struct_ix: u32) -> Option<&'a str> {
    match doc_atom_leaf(doc, struct_ix)? {
        ParsedLeaf::Name(bytes) => core::str::from_utf8(bytes).ok(),
        _ => None,
    }
}

/// The M2 ctor-head KIND byte of an atom struct (a `doc::KIND_*_CTOR`/`KIND_FIELD_PAIR`/`KIND_MEMBER`
/// head position), or `None` if the atom is not a `Ctor` leaf. The decode counterpart of `doc_atom_name`
/// for native-compound heads.
pub(crate) fn doc_atom_ctor(doc: &ParsedDoc, struct_ix: u32) -> Option<u8> {
    match doc_atom_leaf(doc, struct_ix)? {
        ParsedLeaf::Ctor(k) => Some(*k),
        _ => None,
    }
}

/// Max decode recursion depth — the same backstop class as `TYPE_NODE_DEPTH_CAP`/`ENCODE_REF_CYCLE_CAP`:
/// a compiler-baked value is shallow, but a malformed document/descriptor must DECLINE (return NULL), not
/// overflow the guest stack. Well above any real value nesting.
pub(crate) const DECODE_DEPTH_CAP: u32 = 512;

/// Reconstruct an `f64` from a `ParsedLeaf::Float`'s (neg, exp, big-endian base-256 significand) via the
/// exact decimal `[-]<sig>e<exp>` (base-256 → base-10 by repeated ÷10) parsed with Rust's correctly-rounded
/// `str::parse::<f64>` — the inverse of `float_leaf`. `None` if the decimal fails to parse or is non-finite.
pub(crate) fn float_from_parts(neg: bool, exp: i64, mag: &[u8]) -> Option<f64> {
    let s = float_decimal_string(neg, exp, mag);
    let f: f64 = s.parse().ok()?;
    if f.is_finite() { Some(f) } else { None }
}

/// The f32 twin of `float_from_parts` (parses the same decimal as `f32`).
pub(crate) fn float32_from_parts(neg: bool, exp: i64, mag: &[u8]) -> Option<f32> {
    let s = float_decimal_string(neg, exp, mag);
    let f: f32 = s.parse().ok()?;
    if f.is_finite() { Some(f) } else { None }
}

/// Build the `[-]<significand>e<exponent>` decimal string from a `KIND_FLOAT`'s parts: the significand is
/// the big-endian base-256 magnitude read as a base-10 integer (repeated ÷10, no width assumption).
pub(crate) fn float_decimal_string(neg: bool, exp: i64, mag: &[u8]) -> String {
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
        while limbs.first() == Some(&0) && limbs.len() > 1 {
            limbs.remove(0);
        }
    }
    let sig: String = if digits_rev.is_empty() {
        "0".into()
    } else {
        digits_rev.iter().rev().map(|&b| b as char).collect()
    };
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&sig);
    s.push('e');
    // i64 exponent as decimal (no_std-safe via itoa-free format through a small helper).
    s.push_str(&exp_to_string(exp));
    s
}

/// `i64` → decimal string without `format!`'s float machinery (kept explicit for the `no_std` wasm build).
pub(crate) fn exp_to_string(mut v: i64) -> String {
    if v == 0 {
        return "0".into();
    }
    let neg = v < 0;
    let mut digits: Vec<u8> = Vec::new();
    // Work in i128 to hold i64::MIN's magnitude without overflow.
    let mut n = (v as i128).unsigned_abs();
    let _ = &mut v;
    while n > 0 {
        digits.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.extend(digits.iter().rev().map(|&b| b as char));
    s
}

/// Build a big-endian magnitude + sign into the `[sign][little-endian magnitude]` form
/// `bigint::Big::from_sign_magnitude_bytes` expects: reverse the BE magnitude to LE and prepend the sign.
pub(crate) fn big_from_be_mag(neg: bool, be_mag: &[u8]) -> bigint::Big {
    let mut sm = Vec::with_capacity(be_mag.len() + 1);
    sm.push(neg as u8);
    sm.extend(be_mag.iter().rev().copied());
    bigint::Big::from_sign_magnitude_bytes(&sm)
}

/// The descriptor-guided construction walk: read the doc node at `struct_ix` as a value of shape
/// `shape_ix`, building a fresh OWNED heap handle. `Handle::NULL` on ANY mismatch (never traps). `depth`
/// bounds recursion (malformed-cycle backstop).
pub(crate) fn decode_value(
    desc: &Descriptor,
    doc: &ParsedDoc,
    struct_ix: u32,
    shape_ix: u32,
    depth: u32,
) -> Handle {
    decode_value_opt(desc, doc, struct_ix, shape_ix, depth).unwrap_or(Handle::NULL)
}

/// `decode_value`'s `Option` core (so `?` short-circuits a mismatch to `None` → `NULL`). Every arm that
/// builds a heap value on success returns `Some(handle)`; a shape/document mismatch returns `None`.
pub(crate) fn decode_value_opt(
    desc: &Descriptor,
    doc: &ParsedDoc,
    struct_ix: u32,
    shape_ix: u32,
    depth: u32,
) -> Option<Handle> {
    if depth > DECODE_DEPTH_CAP {
        return None;
    }
    match desc.table.get(shape_ix as usize)? {
        // Transparent wrappers: the value handle passes through unchanged. On the wire a Named/Ref adds no
        // struct level (encode reuses the same `h`), EXCEPT Named/Framed which wrap `(: value Type)`.
        Shape::Ref(target) => decode_value_opt(desc, doc, struct_ix, *target, depth + 1),
        Shape::Named(_, inner) | Shape::Framed(_, inner) => {
            // `(: <value> <Type>)` — a 3-element list; the value is element [1], decoded against `inner`.
            let kids = doc_list_kids(doc, struct_ix)?;
            if kids.len() != 3 || doc_atom_name(doc, kids[0])? != ":" {
                return None;
            }
            decode_value_opt(desc, doc, kids[1], *inner, depth + 1)
        }
        Shape::Int => {
            let ParsedLeaf::Int(neg, mag) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            // i64-bounded: rebuild via the BigInt magnitude then read as i64 (an >i64 magnitude here is a
            // malformed doc for an `Int` shape — decline).
            let big = big_from_be_mag(*neg, mag);
            let v = big.to_i64_checked()?;
            Some(op_box_int(v))
        }
        Shape::BigInt => {
            let ParsedLeaf::Int(neg, mag) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            Some(box_bigint(&big_from_be_mag(*neg, mag)))
        }
        Shape::Bool => {
            let ParsedLeaf::Bool(b) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            Some(op_box_bool(*b))
        }
        Shape::Char => {
            // A char value IS an int (the code-point) at runtime — box it with `op_box_int`, exactly as a
            // Bool boxes its 0/1. The wire leaf is `KIND_CHAR` (a scalar); the semantics are int.
            let ParsedLeaf::Char(c) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            Some(op_box_int(*c as i64))
        }
        Shape::Float => match doc_atom_leaf(doc, struct_ix)? {
            ParsedLeaf::Float(neg, exp, mag) => {
                Some(op_box_float(float_from_parts(*neg, *exp, mag)?))
            }
            // The inverse of the encode word-forms — box the non-finite f64 directly.
            ParsedLeaf::FloatNan => Some(op_box_float(f64::NAN)),
            ParsedLeaf::FloatInf(negative) => Some(op_box_float(if *negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            })),
            _ => None,
        },
        Shape::Float32 => match doc_atom_leaf(doc, struct_ix)? {
            ParsedLeaf::Float(neg, exp, mag) => {
                Some(op_box_float32(float32_from_parts(*neg, *exp, mag)?))
            }
            ParsedLeaf::FloatNan => Some(op_box_float32(f32::NAN)),
            ParsedLeaf::FloatInf(negative) => Some(op_box_float32(if *negative {
                f32::NEG_INFINITY
            } else {
                f32::INFINITY
            })),
            _ => None,
        },
        Shape::Str => {
            let ParsedLeaf::Str(bytes) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            let s = String::from_utf8(bytes.clone()).ok()?;
            Some(op_str_new(s))
        }
        Shape::Symbol => {
            // Inverse of the encode member-compound `((. Symbol of) "text")` = list([member-access, str]).
            // A Symbol shares the String runtime rep, so decode to the same string handle (`op_str_new`) —
            // the shape tag already says it is a Symbol; the doc structure is trusted (our own encode).
            // kids[0] = the `(. Symbol of)` member-access; kids[1] = the Str leaf carrying the text.
            let kids = doc_list_kids(doc, struct_ix)?;
            if kids.len() != 2 {
                return None;
            }
            let ParsedLeaf::Str(bytes) = doc_atom_leaf(doc, kids[1])? else {
                return None;
            };
            let s = String::from_utf8(bytes.clone()).ok()?;
            Some(op_str_new(s))
        }
        Shape::Bytes => {
            let ParsedLeaf::Bytes(bytes) = doc_atom_leaf(doc, struct_ix)? else {
                return None;
            };
            let buf = op_bytes_alloc(bytes.len() as u32);
            for (i, &b) in bytes.iter().enumerate() {
                op_bytes_set(buf, i as u32, b as u32);
            }
            Some(buf)
        }
        Shape::Unit => {
            // Encodes as the `unit` NAME atom.
            if doc_atom_name(doc, struct_ix)? != "unit" {
                return None;
            }
            Some(imm_unit())
        }
        Shape::Rational => {
            // seq-204 native head+children: the list `(KIND_RATIONAL <num-int> <den-int>)` — payloadless
            // Rational tag head + two ordinary Int children. Rebuild each Big (as Shape::Int/BigInt) then
            // re-normalize (defensive — a well-formed producer already emits lowest-terms/sign-on-num/den>0).
            let kids = doc_list_kids(doc, struct_ix)?;
            if kids.len() != 3 || doc_atom_ctor(doc, kids[0])? != doc::KIND_RATIONAL {
                return None;
            }
            let ParsedLeaf::Int(nneg, nmag) = doc_atom_leaf(doc, kids[1])? else {
                return None;
            };
            let ParsedLeaf::Int(dneg, dmag) = doc_atom_leaf(doc, kids[2])? else {
                return None;
            };
            let num = big_from_be_mag(*nneg, nmag);
            let den = big_from_be_mag(*dneg, dmag);
            Some(box_rational_normalized(&num, &den))
        }
        Shape::Tuple(elems) => {
            let elems = elems.clone();
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(Tuple) e0 e1 …)` — the Tuple ctor-head atom + one child per element.
            if kids.is_empty()
                || doc_atom_ctor(doc, kids[0])? != doc::KIND_TUPLE_CTOR
                || kids.len() - 1 != elems.len()
            {
                return None;
            }
            build_arr(desc, doc, &kids[1..], &elems, depth)
        }
        Shape::Spread(elems) => {
            // A Spread is only reached as a Sum variant's payload; the Sum arm splices its children, so a
            // direct decode of a Spread shape reads the same as a Tuple's element list WITHOUT a head (the
            // caller passed exactly the element child indices). Guard arity and build the arr.
            let elems = elems.clone();
            let kids = doc_list_kids(doc, struct_ix)?;
            if kids.len() != elems.len() {
                return None;
            }
            build_arr(desc, doc, kids, &elems, depth)
        }
        Shape::Record(fields) => {
            let fields = fields.clone();
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(Record) (FieldPair name value) …)` — Record ctor-head atom + one FieldPair triple
            // per field. Fields are in descriptor (sorted) order.
            if kids.is_empty()
                || doc_atom_ctor(doc, kids[0])? != doc::KIND_RECORD_CTOR
                || kids.len() - 1 != fields.len()
            {
                return None;
            }
            let arr = op_arr_alloc(fields.len() as u32);
            for (i, (fname, fshape)) in fields.iter().enumerate() {
                let field = doc_list_kids(doc, kids[1 + i])?;
                // M2 field form `(FieldPair name value)` — a 3-element list with the FieldPair ctor head.
                // (The legacy `(name value)` 2-element pair is still accepted for back-compat.) The value
                // child is the last element; the name is matched against the descriptor's field.
                let (name_ix, value_ix) = match field.len() {
                    3 if doc_atom_ctor(doc, field[0]) == Some(doc::KIND_FIELD_PAIR) => {
                        (field[1], field[2]) // (FieldPair name value)
                    }
                    2 => (field[0], field[1]), // (name value) — legacy
                    _ => {
                        op_drop(arr);
                        return None;
                    }
                };
                if doc_atom_name(doc, name_ix)? != &**fname {
                    op_drop(arr);
                    return None;
                }
                let fval = decode_value_opt(desc, doc, value_ix, *fshape, depth + 1);
                match fval {
                    Some(h) => {
                        op_arr_set(arr, i as u32, h);
                    }
                    None => {
                        op_drop(arr);
                        return None;
                    }
                }
            }
            Some(arr)
        }
        Shape::List(elem) => {
            let elem = *elem;
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(List) e…)` — the List ctor-head atom + the elements.
            if kids.is_empty() || doc_atom_ctor(doc, kids[0])? != doc::KIND_LIST_CTOR {
                return None;
            }
            let mut v = op_vec_empty();
            for &ck in &kids[1..] {
                match decode_value_opt(desc, doc, ck, elem, depth + 1) {
                    Some(h) => {
                        v = op_vec_push(v, h);
                    }
                    None => {
                        op_drop(v);
                        return None;
                    }
                }
            }
            Some(v)
        }
        Shape::Sum(variants) => {
            let variants = variants.clone();
            let kids = doc_list_kids(doc, struct_ix)?;
            // `(VariantName payload…)` — head atom is the variant name; match to its discriminant.
            if kids.is_empty() {
                return None;
            }
            let head = doc_atom_name(doc, kids[0])?;
            let (disc, (_, payload_shape)) = variants
                .iter()
                .enumerate()
                .find(|(_, (name, _))| &**name == head)?;
            let payload_shape = *payload_shape;
            // A MULTI-payload variant's payload is a `Spread`: its elements are the variant's children
            // (flattened) — build the payload arr from `kids[1..]` directly. A single/nullary payload
            // decodes the ONE payload node.
            match desc.table.get(payload_shape as usize) {
                Some(Shape::Spread(elems)) => {
                    let elems = elems.clone();
                    if kids.len() - 1 != elems.len() {
                        return None;
                    }
                    let payload = build_arr(desc, doc, &kids[1..], &elems, depth)?;
                    Some(op_sum_new(disc as u32, payload))
                }
                _ => {
                    // Single payload: exactly one child (a nullary variant's payload is `unit`).
                    if kids.len() != 2 {
                        return None;
                    }
                    let payload = decode_value_opt(desc, doc, kids[1], payload_shape, depth + 1)?;
                    Some(op_sum_new(disc as u32, payload))
                }
            }
        }
        Shape::Set(elem) => {
            let elem = *elem;
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(Set) e…)` — the Set ctor-head atom + the elements directly (was the nested
            // `((. Set of) (list e…))` member-access-over-list form).
            if kids.is_empty() || doc_atom_ctor(doc, kids[0])? != doc::KIND_SET_CTOR {
                return None;
            }
            let mut s = op_set_empty();
            for &ck in &kids[1..] {
                match decode_value_opt(desc, doc, ck, elem, depth + 1) {
                    Some(h) => {
                        s = op_set_insert(s, h);
                    }
                    None => {
                        op_drop(s);
                        return None;
                    }
                }
            }
            Some(s)
        }
        Shape::Map(key, val) => {
            let (key, val) = (*key, *val);
            let kids = doc_list_kids(doc, struct_ix)?;
            // M2 `(Ctor(Map) (FieldPair k v) …)` — Map ctor-head atom + one FieldPair triple per entry.
            if kids.is_empty() || doc_atom_ctor(doc, kids[0])? != doc::KIND_MAP_CTOR {
                return None;
            }
            let mut m = op_map_empty();
            for &pair_ix in &kids[1..] {
                let pair = doc_list_kids(doc, pair_ix)?;
                // `(FieldPair key value)` — 3 elems: the FieldPair ctor head + key + value.
                if pair.len() != 3 || doc_atom_ctor(doc, pair[0]) != Some(doc::KIND_FIELD_PAIR) {
                    op_drop(m);
                    return None;
                }
                let kh = match decode_value_opt(desc, doc, pair[1], key, depth + 1) {
                    Some(h) => h,
                    None => {
                        op_drop(m);
                        return None;
                    }
                };
                let vh = match decode_value_opt(desc, doc, pair[2], val, depth + 1) {
                    Some(h) => h,
                    None => {
                        op_drop(kh);
                        op_drop(m);
                        return None;
                    }
                };
                m = op_map_insert(m, kh, vh);
            }
            Some(m)
        }
    }
}

/// Build a fresh `arr` (the runtime rep of a tuple/record/spread) from `kids` (one doc child per element)
/// decoded against `shapes` (parallel element shape indices). On any element mismatch, drops the
/// partially-built arr and returns `None`. Caller guarantees `kids.len() == shapes.len()`.
pub(crate) fn build_arr(
    desc: &Descriptor,
    doc: &ParsedDoc,
    kids: &[u32],
    shapes: &[u32],
    depth: u32,
) -> Option<Handle> {
    let arr = op_arr_alloc(shapes.len() as u32);
    for (i, (&ck, &sh)) in kids.iter().zip(shapes.iter()).enumerate() {
        match decode_value_opt(desc, doc, ck, sh, depth + 1) {
            Some(h) => {
                op_arr_set(arr, i as u32, h);
            }
            None => {
                op_drop(arr);
                return None;
            }
        }
    }
    Some(arr)
}

/// Parse a base-10 decimal string (optional leading `-`) into a `bigint::Big`. `None` on a non-digit
/// character. Used for the Rational `num/den` name-leaf components.
pub(crate) fn big_from_decimal(s: &str) -> Option<bigint::Big> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ten = bigint::Big::from_i64(10);
    let mut acc = bigint::Big::zero();
    for b in digits.bytes() {
        acc = acc.mul(&ten);
        acc = acc.add(&bigint::Big::from_i64((b - b'0') as i64));
    }
    if neg {
        acc = acc.neg();
    }
    Some(acc)
}

/// value-decode (heap idx 90): parse the value-form `doc_bytes` and, guided by `desc`, construct a fresh
/// owned heap value. `Handle::NULL` on a malformed document / descriptor mismatch (never traps).
//= spec/contracts/deterministic-value-form.md#the-canonical-byte-form-has-a-decode-that-inverts-it
//# Decoding the canonical byte encoding of a value against the type of that value MUST yield a value equal, under the language's structural equality, to the value that was encoded.
//= spec/contracts/deterministic-value-form.md#decoding-refuses-bytes-that-are-not-a-value-of-the-expected-type
//# Decoding a byte sequence that is not the canonical byte encoding of any value of the expected type MUST be refused rather than yield a value, so that a decode never misinterprets bytes as a value they do not encode.
pub(crate) fn op_value_decode(doc_bytes: &[u8], desc: &[u8]) -> Handle {
    let Some(descriptor) = decode_descriptor(desc) else {
        return Handle::NULL;
    };
    let Some(parsed) = parse_doc(doc_bytes) else {
        return Handle::NULL;
    };
    decode_value(&descriptor, &parsed, parsed.root, descriptor.root, 0)
}

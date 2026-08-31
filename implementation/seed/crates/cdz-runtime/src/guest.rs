//! WIT Guest implementation
//!
//! The ONLY place u32 handles exist - the wasm component boundary.

use super::*;

#[cfg(target_arch = "wasm32")]
impl Handle {
    /// Narrow to the opaque public handle. Lossless on wasm32 (a `Node` address is 32-bit).
    ///
    /// The u32 IS a raw `Node` address into THIS runtime instance's heap, so it is meaningful only within
    /// the single run/instance that produced it — it never escapes as durable state the ABI transports;
    /// a host resuming a run reconstructs values through the runtime, not by carrying a handle across.
    //= spec/contracts/component-abi.md#a-runtime-value-crosses-as-an-opaque-handle
    //# A runtime handle MUST be meaningful only within the single run and runtime instance that produced it, so that a handle never escapes the run that produced it and a host that resumes a run by replaying it reconstructs the run's values through the runtime rather than by carrying a handle across the boundary (the handle is not durable state the ABI transports; whether and how a host replays is host policy — capabilities-and-effects.md §A Run Is A Deterministic Function Of Its Input And Responses).
    fn to_u32(self) -> u32 {
        self.0 as usize as u32
    }
    /// Widen a public handle back to a node pointer. Inverse of `to_u32` on wasm32.
    fn from_u32(x: u32) -> Handle {
        Handle(x as usize as *mut Node)
    }
}

/// `hash-blake3(bytes)` (heap index 91) — the BLAKE3 digest of `input`'s Bytes-leaf contents, as a fresh
/// 32-byte Bytes leaf. A GENERIC content hash (`bytes -> digest`): no tag, no prefix, no notion of a
/// "contract" — userspace prepends any domain separation before calling (DESIGN-compiler-primitives.md D7).
/// This is the RUNTIME half of the compiler's `Blake3.of`; the compile-time fold calls the SAME `blake3`
/// crate over the same bytes, so both produce byte-identical digests (that design's §9 load-bearing
/// invariant). BORROWS `input` (reads it, never drops it — an inspector, like `op_value_encode_form`) and
/// returns a fresh owned leaf. Reads `input` LOGICALLY via the index accessors so a rope Bytes value
/// flattens correctly, exactly as `op_value_decode` reads its document. TOTAL: an empty input hashes to
/// blake3's defined empty-input digest; never traps.
pub(crate) fn op_hash_blake3(input: Handle) -> Handle {
    let n = op_bytes_len(input);
    let mut buf = Vec::with_capacity(n as usize);
    for i in 0..n {
        buf.push(op_bytes_get(input, i) as u8);
    }
    let digest = blake3::hash(&buf);
    alloc(Vec::new(), digest.as_bytes().to_vec())
}

/// The 7 `Ast` variant discs the compiler conveys to `ast-print`/`ast-read`. The compiler looks these up
/// BY NAME from the (prelude-defined) `Ast` sum decl, so the runtime NEVER hardcodes them — they ride in
/// the `discs` Bytes, LEB-encoded in this fixed slot order: [int, float, bool, str, name, bytes, list].
pub(crate) struct AstDiscs {
    int: u32,
    float: u32,
    boolv: u32,
    strv: u32,
    name: u32,
    bytes: u32,
    list: u32,
}

/// Decode the baked disc descriptor: 7 LEB128 varints in `[int,float,bool,str,name,bytes,list]` order.
/// `None` on a truncated/malformed descriptor (the compiler always bakes a well-formed one, so not-reached).
pub(crate) fn read_ast_discs(discs: Handle) -> Option<AstDiscs> {
    let n = op_bytes_len(discs);
    let mut buf = Vec::with_capacity(n as usize);
    for i in 0..n {
        buf.push(op_bytes_get(discs, i) as u8);
    }
    let mut pos = 0usize;
    let mut next = || -> Option<u32> {
        let mut val: u32 = 0;
        let mut shift = 0u32;
        loop {
            let b = *buf.get(pos)?;
            pos += 1;
            val |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Some(val)
    };
    Some(AstDiscs {
        int: next()?,
        float: next()?,
        boolv: next()?,
        strv: next()?,
        name: next()?,
        bytes: next()?,
        list: next()?,
    })
}

/// Escape a string's contents for a `"…"` Ast.Str literal — the closed set `\n \t \r \\ \"` — mirroring the
/// compiler's `push_escaped_str` (rcdzc lower.rs) so `Ast.print` is byte-identical to the compile-time fold.
pub(crate) fn push_escaped_ast_str(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
}

/// Render a runtime `Ast` heap value to canonical re-readable s-expression text — BYTE-IDENTICAL to the
/// compiler's `print_ast_value` (rcdzc lower.rs): Int→BigInt decimal, Float→Rust shortest round-trip
/// decimal (forced `.0`), Bool→true/false, Str→escaped `"…"`, Name→bare, Bytes→`b"…"` (printable / named /
/// `\xNN` lower-hex), List→`(e e …)` space-separated recursive. An `Ast` variant carries exactly one
/// payload (a real heap sum node → `op_sum_disc` reads its stored disc). An unknown disc renders nothing.
pub(crate) fn render_ast(h: Handle, d: &AstDiscs, out: &mut String) {
    let disc = op_sum_disc(h);
    let payload = op_sum_payload(h);
    if disc == d.int {
        out.push_str(&unbox_bigint(payload).to_decimal_string());
    } else if disc == d.float {
        // Match `float_text` (rcdzc): Rust's `{}` shortest round-trip, forced to carry `.`/`e` so it
        // re-lexes as a float (a bare `3` would re-read as Ast.Int). f64's Display is core (no_std-ok).
        let s = alloc::format!("{}", op_get_float(payload));
        out.push_str(&s);
        if !(s.contains('.') || s.contains('e') || s.contains('E')) {
            out.push_str(".0");
        }
    } else if disc == d.boolv {
        out.push_str(if op_get_bool(payload) {
            "true"
        } else {
            "false"
        });
    } else if disc == d.strv {
        out.push('"');
        push_escaped_ast_str(out, &op_str_get(payload));
        out.push('"');
    } else if disc == d.name {
        out.push_str(&op_str_get(payload));
    } else if disc == d.bytes {
        out.push_str("b\"");
        let n = op_bytes_len(payload);
        for i in 0..n {
            let b = op_bytes_get(payload, i) as u8;
            match b {
                b'\n' => out.push_str("\\n"),
                b'\t' => out.push_str("\\t"),
                b'\r' => out.push_str("\\r"),
                b'\\' => out.push_str("\\\\"),
                b'"' => out.push_str("\\\""),
                0x20..=0x7e => out.push(b as char),
                _ => {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    out.push('\\');
                    out.push('x');
                    out.push(HEX[(b >> 4) as usize] as char);
                    out.push(HEX[(b & 0xf) as usize] as char);
                }
            }
        }
        out.push('"');
    } else if disc == d.list {
        // `Ast.List`'s payload is a Cadenza `(list …)` — a persistent RRB VECTOR, read by `vec-len`/
        // `vec-get` (NOT the `arr-*` tuple/record accessors: an RRB root node's `handles` arity is its
        // branch/leaf count, not the element count, so `arr-len` misreads a multi-element list as 1).
        out.push('(');
        let n = op_vec_len(payload);
        for i in 0..n {
            if i > 0 {
                out.push(' ');
            }
            render_ast(op_vec_get(payload, i), d, out);
        }
        out.push(')');
    }
}

/// `ast-print(handle, discs)` (heap op 92) — the runtime half of the compiler's `Ast.print`: render a
/// RUNTIME `Ast` heap value to its canonical re-readable s-expression text (a fresh owned String leaf),
/// byte-identical to the compile-time `print_ast_value` fold. BORROWS `handle` + `discs` (the caller owns
/// their release, like `value-encode`); the disc→variant mapping is read from the compiler-baked `discs`.
pub(crate) fn op_ast_print(handle: Handle, discs: Handle) -> Handle {
    let mut out = String::new();
    if let Some(d) = read_ast_discs(discs) {
        render_ast(handle, &d, &mut out);
    }
    op_str_new(out)
}

/// The NINE Ast-variant discriminants `ast-encode`/`ast-decode` need — the print descriptor's seven plus
/// `char` + `symbol` (encode/decode must round-trip EVERY variant, whereas print renders seven). A distinct
/// descriptor from `AstDiscs`: the shipped `ast-print` op bakes seven in its own order, so its reader stays
/// as-is. Field order mirrors the compiler's `AstDiscs` struct (`lower.rs`) — `[int, float, bool, str, name,
/// list, bytes, char, symbol]` — the order the compiler bakes the descriptor in.
pub(crate) struct AstEncDiscs {
    int: u32,
    float: u32,
    boolv: u32,
    strv: u32,
    name: u32,
    list: u32,
    bytes: u32,
    chr: u32,
    symbol: u32,
    // M2 (OPTION B) — the 7 native-collection reflected-Ast ctors, appended after `symbol`. The reflected
    // `Ast` sum gained `ListCtor`/`TupleCtor`/`RecordCtor`/`MapCtor`/`SetCtor` (each `(List Ast)`) and
    // `FieldPair`/`Member` (each `(Tuple Ast Ast)`); a compound decoded from a ctor-leaf head reflects to
    // the DISTINCT ctor, not a name-headed list. Baked positionally in this order by the descriptor synth.
    list_ctor: u32,
    tuple_ctor: u32,
    record_ctor: u32,
    map_ctor: u32,
    set_ctor: u32,
    field_pair: u32,
    member: u32,
    // The native RATIONAL literal (`3/2`) reflected variant — payload is a `(Tuple Ast Ast)` of num/den,
    // appended after `member` (matches the compiler's `AstDiscs`/bake field order).
    rational: u32,
}

/// Decode the baked 17-disc descriptor: 17 LEB128 varints in
/// `[int,float,bool,str,name,list,bytes,char,symbol, list_ctor,tuple_ctor,record_ctor,map_ctor,set_ctor,field_pair,member,rational]`
/// order (the 7 M2 native-collection ctors, then the native rational, appended last). `None` on a truncated
/// descriptor (the compiler always bakes a well-formed one; a pre-M2 9-disc descriptor truncates → `None`,
/// which is correct: a B runtime requires a B descriptor).
pub(crate) fn read_ast_enc_discs(discs: Handle) -> Option<AstEncDiscs> {
    let n = op_bytes_len(discs);
    let mut buf = Vec::with_capacity(n as usize);
    for i in 0..n {
        buf.push(op_bytes_get(discs, i) as u8);
    }
    let mut pos = 0usize;
    let mut next = || -> Option<u32> {
        let mut val: u32 = 0;
        let mut shift = 0u32;
        loop {
            let b = *buf.get(pos)?;
            pos += 1;
            val |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Some(val)
    };
    Some(AstEncDiscs {
        int: next()?,
        float: next()?,
        boolv: next()?,
        strv: next()?,
        name: next()?,
        list: next()?,
        bytes: next()?,
        chr: next()?,
        symbol: next()?,
        list_ctor: next()?,
        tuple_ctor: next()?,
        record_ctor: next()?,
        map_ctor: next()?,
        set_ctor: next()?,
        field_pair: next()?,
        member: next()?,
        rational: next()?,
    })
}

/// Bridge a runtime heap integer (`bigint::Big`) to the codec's `ast::IntValue{negative, magnitude:BE}`. The
/// heap leaf's sign-magnitude bytes are `[sign][LITTLE-endian magnitude, trailing-zeros-stripped]`; the codec
/// wants a BIG-endian magnitude with no leading zeros, so reverse the magnitude bytes (LE→BE; the LE form has
/// no trailing zeros, so the reversed BE form has no leading zeros — already canonical). Zero → `[0]` → empty
/// magnitude = `IntValue::zero`.
pub(crate) fn big_to_intvalue(b: &bigint::Big) -> crate::ast::IntValue {
    let sm = b.to_sign_magnitude_bytes();
    let negative = sm.first().copied() == Some(1);
    let mut magnitude: Vec<u8> = sm[1..].to_vec();
    magnitude.reverse();
    crate::ast::IntValue {
        negative,
        magnitude,
    }
}

/// Walk a runtime heap `Ast` value into the shared cadenza-ast `Builder` `b`, returning the built node's
/// `StructId` — the runtime twin of the compiler's `encode_ast_value` (rcdzc `lower.rs`), building the SAME
/// leaves/structs so `codec::encode` of the finished arena is BYTE-IDENTICAL to the compile-time fold. `None`
/// on an unknown disc (not reached for a well-typed Ast). A non-finite float has no finite `Decimal` and no
/// leaf yet (awaits the `KIND_FLOAT_{NAN,POS_INF,NEG_INF}` tags) — it declines here for now.
pub(crate) fn encode_ast_to_arenas(
    h: Handle,
    d: &AstEncDiscs,
    b: &mut crate::ast::Builder,
) -> Option<crate::ast::StructId> {
    let disc = op_sum_disc(h);
    let payload = op_sum_payload(h);
    if disc == d.int {
        Some(b.atom_leaf(crate::ast::Leaf::Int {
            value: big_to_intvalue(&unbox_bigint(payload)),
            radix: crate::ast::Radix::Dec,
        }))
    } else if disc == d.float {
        // A finite float encodes as the exact-decimal `Leaf::Float`; a NON-FINITE float has no finite
        // Decimal, so it rides its own payload-less leaf tag (17/18/19): NaN → FloatNan, +inf →
        // FloatInf{false}, -inf → FloatInf{true}. Byte-identical to the compiler's `encode_ast_value`
        // fold (both write the same shared codec tag), so `Ast.encode` of a non-finite Ast.Float agrees
        // compile-time and at runtime (the decode inverse is in `decode_arenas_to_ast`).
        let f = op_get_float(payload);
        let leaf = if f.is_nan() {
            crate::ast::Leaf::FloatNan
        } else if f.is_infinite() {
            crate::ast::Leaf::FloatInf { negative: f < 0.0 }
        } else {
            crate::ast::Leaf::Float(crate::ast::Decimal::from_f64(f)?)
        };
        Some(b.atom_leaf(leaf))
    } else if disc == d.boolv {
        Some(b.atom_leaf(crate::ast::Leaf::Bool(op_get_bool(payload))))
    } else if disc == d.strv {
        Some(b.atom_leaf(crate::ast::Leaf::Str(op_str_get(payload).into())))
    } else if disc == d.name {
        Some(b.atom_leaf(crate::ast::Leaf::Name(op_str_get(payload).into())))
    } else if disc == d.symbol {
        Some(b.atom_leaf(crate::ast::Leaf::Sym(op_str_get(payload).into())))
    } else if disc == d.chr {
        // A `Char` payload is a boxed i32 scalar code point (never a heap handle); a valid `Ast.Char` always
        // holds a real Unicode scalar, so `from_u32` succeeds.
        let c = char::from_u32(op_get_int(payload) as u32)?;
        Some(b.atom_leaf(crate::ast::Leaf::Char(c)))
    } else if disc == d.bytes {
        let n = op_bytes_len(payload);
        let mut raw = Vec::with_capacity(n as usize);
        for i in 0..n {
            raw.push(op_bytes_get(payload, i) as u8);
        }
        Some(b.atom_leaf(crate::ast::Leaf::Bytes(raw.into())))
    } else if disc == d.list {
        // A generic name-headed (or empty) list payload is a persistent RRB vector (`vec-*`, NOT `arr-*`);
        // each element is itself an Ast. Stays `Ast.List` (no ctor head) — the inverse of decode's fall-through.
        let n = op_vec_len(payload);
        let mut children = Vec::with_capacity(n as usize);
        for i in 0..n {
            children.push(encode_ast_to_arenas(op_vec_get(payload, i), d, b)?);
        }
        Some(b.list(children))
    } else if disc == d.list_ctor {
        // M2 (OPTION B): a reflected first-class compound-ctor value. Its payload is a `(List Ast)` RRB vector
        // of the reflected children (for Record/Map, those children are themselves `FieldPair` Ast values);
        // emit head-first via `Builder::compound`, whose head is the ctor LEAF KIND — byte-identical to the
        // compile-time `encode_ast_value` (both go through the shared cadenza-ast `Builder`) and the exact
        // inverse of `decode_arenas_to_ast`'s ctor-head arm.
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::List, &children))
    } else if disc == d.tuple_ctor {
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::Tuple, &children))
    } else if disc == d.record_ctor {
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::Record, &children))
    } else if disc == d.map_ctor {
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::Map, &children))
    } else if disc == d.set_ctor {
        let children = encode_ast_ctor_children(payload, d, b)?;
        Some(b.compound(crate::ast::CompoundCtor::Set, &children))
    } else if disc == d.field_pair {
        // FieldPair / Member payload is a `(Tuple Ast Ast)` = an `arr` of exactly two reflected children
        // (key,value for FieldPair; obj,key for Member).
        let k = encode_ast_to_arenas(op_arr_get(payload, 0), d, b)?;
        let val = encode_ast_to_arenas(op_arr_get(payload, 1), d, b)?;
        Some(b.field_pair(k, val))
    } else if disc == d.member {
        let obj = encode_ast_to_arenas(op_arr_get(payload, 0), d, b)?;
        let key = encode_ast_to_arenas(op_arr_get(payload, 1), d, b)?;
        Some(b.member(obj, key))
    } else if disc == d.rational {
        // Ast.Rational payload is a `(Tuple Ast Ast)` = an `arr` of the numerator/denominator (each an
        // `Ast.Int`); emit the native `(RationalTag <num> <den>)` node via `Builder::rational` — the exact
        // inverse of `decode_arenas_to_ast`'s rational-head arm, byte-identical to the compile-time fold.
        let num = encode_ast_to_arenas(op_arr_get(payload, 0), d, b)?;
        let den = encode_ast_to_arenas(op_arr_get(payload, 1), d, b)?;
        Some(b.rational(num, den))
    } else {
        None
    }
}

/// Encode the `(List Ast)` RRB-vector payload of a reflected compound-ctor value into the arena children
/// (each element recursively encoded), collected into a `Vec` so the mutable `Builder` borrow is released
/// before the caller's `Builder::compound` reborrows it. `None` propagates any child's encode failure.
pub(crate) fn encode_ast_ctor_children(
    payload: Handle,
    d: &AstEncDiscs,
    b: &mut crate::ast::Builder,
) -> Option<Vec<crate::ast::StructId>> {
    let n = op_vec_len(payload);
    let mut children = Vec::with_capacity(n as usize);
    for i in 0..n {
        children.push(encode_ast_to_arenas(op_vec_get(payload, i), d, b)?);
    }
    Some(children)
}

/// `ast-encode(handle, discs)` (heap op 93) — the runtime half of the compiler's `Ast.encode`: serialize a
/// RUNTIME `Ast` heap value to its canonical `cdzast` binary form (a fresh owned Bytes leaf), BYTE-IDENTICAL
/// to the compile-time `Ast.encode` fold (both run the shared `crate::codec::encode` over the same `Arenas`).
/// BORROWS `handle` + `discs`. An Ast that cannot be built (an unknown disc / a non-finite float pending its
/// tag) yields empty Bytes — not reached for a well-typed finite Ast.
pub(crate) fn op_ast_encode(handle: Handle, discs: Handle) -> Handle {
    let bytes = read_ast_enc_discs(discs)
        .and_then(|d| {
            let mut b = crate::ast::Builder::new();
            let root = encode_ast_to_arenas(handle, &d, &mut b)?;
            Some(crate::codec::encode(&b.finish(root)))
        })
        .unwrap_or_default();
    let buf = op_bytes_alloc(bytes.len() as u32);
    for (i, &v) in bytes.iter().enumerate() {
        op_bytes_set(buf, i as u32, v as u32);
    }
    buf
}

/// Inverse of [`big_to_intvalue`]: a codec `ast::IntValue{negative, magnitude:BE}` → a runtime heap
/// `bigint::Big`. `Big::from_sign_magnitude_bytes` wants `[sign][LITTLE-endian magnitude]`, so reverse the
/// big-endian magnitude back to little-endian and prepend the sign byte.
pub(crate) fn intvalue_to_big(iv: &crate::ast::IntValue) -> bigint::Big {
    let mut sm = Vec::with_capacity(1 + iv.magnitude.len());
    sm.push(iv.negative as u8);
    sm.extend(iv.magnitude.iter().rev().copied());
    bigint::Big::from_sign_magnitude_bytes(&sm)
}

/// Rebuild a heap `Ast` value from a node of a `codec::decode`d cadenza-ast `Arenas` — the runtime twin of
/// the compiler's `arenas_to_ast_value` (rcdzc `lower.rs`) and the inverse of [`encode_ast_to_arenas`].
/// Builds each node with `op_sum_new` at the descriptor's discs, boxing scalar payloads exactly as a
/// constructed `Ast` value does (bigint leaf / boxed float / boxed char scalar / RRB `vec-push` for a list).
/// `None` on an out-of-range id or a leaf with no `Ast` variant (`BadEscape`/`BadChar` markers — which a
/// well-formed `Ast.encode` never emits), so a malformed document decodes to the `Err` case, never a trap.
pub(crate) fn decode_arenas_to_ast(
    arenas: &crate::ast::Arenas,
    sid: crate::ast::StructId,
    d: &AstEncDiscs,
) -> Option<Handle> {
    match arenas.structure.get(sid.0 as usize)? {
        crate::ast::Struct::Atom(lid) => {
            let h = match arenas.leaves.get(lid.0 as usize)? {
                crate::ast::Leaf::Int { value, .. } => {
                    op_sum_new(d.int, box_bigint(&intvalue_to_big(value)))
                }
                crate::ast::Leaf::Float(dec) => {
                    op_sum_new(d.float, op_box_float(f64::from_bits(dec.to_f64_bits())))
                }
                // The non-finite float VALUES (codec tags 17/18/19) rebuild as an `Ast.Float` holding
                // the non-finite `f64` — the heap `Ast.Float` box carries any `f64`, so NaN / ±∞ are
                // ordinary boxed values (the inverse of `ast-encode` emitting the non-finite tag for a
                // non-finite `Ast.Float`).
                crate::ast::Leaf::FloatNan => op_sum_new(d.float, op_box_float(f64::NAN)),
                crate::ast::Leaf::FloatInf { negative } => op_sum_new(
                    d.float,
                    op_box_float(if *negative {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    }),
                ),
                crate::ast::Leaf::Bool(b) => op_sum_new(d.boolv, op_box_bool(*b)),
                crate::ast::Leaf::Str(s) => op_sum_new(d.strv, op_str_new(s.to_string())),
                crate::ast::Leaf::Name(s) => op_sum_new(d.name, op_str_new(s.to_string())),
                crate::ast::Leaf::Sym(s) => op_sum_new(d.symbol, op_str_new(s.to_string())),
                crate::ast::Leaf::Char(c) => op_sum_new(d.chr, op_box_int(*c as i64)),
                crate::ast::Leaf::Bytes(v) => {
                    let buf = op_bytes_alloc(v.len() as u32);
                    for (i, &b) in v.iter().enumerate() {
                        op_bytes_set(buf, i as u32, b as u32);
                    }
                    op_sum_new(d.bytes, buf)
                }
                // M2 (OPTION B): a compound-ctor head leaf (`Leaf::Ctor`/`FieldPair`/`Member`, codec kinds
                // 20-26) is NEVER a bare atom — it only ever appears as the HEAD of a `Struct::List` (handled
                // in the List arm below, dispatched to the DISTINCT reflected ctor). Reached as a standalone
                // atom it is a malformed document; decode is TOTAL (op94 → NULL on bad bytes, never a trap),
                // so fail cleanly.
                crate::ast::Leaf::Ctor(_)
                | crate::ast::Leaf::FieldPair
                | crate::ast::Leaf::Member
                // The rational-literal HEAD leaf (seq-204) is the same shape — a LIST head, never a bare
                // atom; its `(RationalTag num den)` node rebuilds to `Ast.Rational` in the List arm below. A
                // stray bare tag is likewise malformed → None.
                | crate::ast::Leaf::Rational => return None,
                crate::ast::Leaf::BadEscape(_) | crate::ast::Leaf::BadChar(_) => return None,
                // A type-suffixed numeric literal (`100N`/`0.5R`) is decoded to a plain Int/Float by the
                // codec, so it never appears in a decoded document; a stray occurrence fails cleanly
                // (decode is TOTAL), like the marker leaves above.
                crate::ast::Leaf::Suffixed { .. } => return None,
            };
            Some(h)
        }
        crate::ast::Struct::List(children) => {
            // M2 (OPTION B): if the list HEAD is a compound-ctor leaf, reflect to the DISTINCT first-class
            // reflected Ast ctor (native collections — no string head), built from the REMAINING children; a
            // generic name-headed (or empty) list stays `Ast.List`.
            if let Some(&head_sid) = children.first()
                && let Some(crate::ast::Struct::Atom(lid)) =
                    arenas.structure.get(head_sid.0 as usize)
                && let Some(head_leaf) = arenas.leaves.get(lid.0 as usize)
            {
                match head_leaf {
                    // The 5 collections carry a `(List Ast)` of their reflected tail elements.
                    crate::ast::Leaf::Ctor(c) => {
                        let disc = match c {
                            crate::ast::CompoundCtor::List => d.list_ctor,
                            crate::ast::CompoundCtor::Tuple => d.tuple_ctor,
                            crate::ast::CompoundCtor::Record => d.record_ctor,
                            crate::ast::CompoundCtor::Map => d.map_ctor,
                            crate::ast::CompoundCtor::Set => d.set_ctor,
                        };
                        let mut v = op_vec_empty();
                        for &ch in &children[1..] {
                            v = op_vec_push(v, decode_arenas_to_ast(arenas, ch, d)?);
                        }
                        return Some(op_sum_new(disc, v));
                    }
                    // FieldPair/Member carry a `(Tuple Ast Ast)` = (key,value) / (obj,key): exactly 2 elems.
                    crate::ast::Leaf::FieldPair | crate::ast::Leaf::Member => {
                        if children.len() != 3 {
                            return None; // malformed: a pair head needs exactly two elements
                        }
                        let a = decode_arenas_to_ast(arenas, children[1], d)?;
                        let b = decode_arenas_to_ast(arenas, children[2], d)?;
                        let tup = op_arr_alloc(2);
                        op_arr_set(tup, 0, a);
                        op_arr_set(tup, 1, b);
                        let disc = if matches!(head_leaf, crate::ast::Leaf::FieldPair) {
                            d.field_pair
                        } else {
                            d.member
                        };
                        return Some(op_sum_new(disc, tup));
                    }
                    // A native RATIONAL literal `(RationalTag <num> <den>)` → Ast.Rational of a `(Tuple Ast
                    // Ast)` of the two reflected Int children (exactly 2 elems), the inverse of the rational
                    // encode arm and the runtime twin of the compiler's `arenas_to_ast_value` rational arm.
                    crate::ast::Leaf::Rational => {
                        if children.len() != 3 {
                            return None; // malformed: a rational head needs exactly two children
                        }
                        let num = decode_arenas_to_ast(arenas, children[1], d)?;
                        let den = decode_arenas_to_ast(arenas, children[2], d)?;
                        let tup = op_arr_alloc(2);
                        op_arr_set(tup, 0, num);
                        op_arr_set(tup, 1, den);
                        return Some(op_sum_new(d.rational, tup));
                    }
                    _ => {} // a name/other head → the generic `Ast.List` below
                }
            }
            let mut v = op_vec_empty();
            for &c in children.iter() {
                v = op_vec_push(v, decode_arenas_to_ast(arenas, c, d)?);
            }
            Some(op_sum_new(d.list, v))
        }
    }
}

/// `ast-decode(bytes-handle, discs)` (heap op 94) — the runtime half of the compiler's `Ast.decode`, the
/// TOTAL inverse of `ast-encode`: parse a `Bytes` leaf as one canonical `cdzast` document (via the shared
/// `crate::codec::decode`) and rebuild the heap `Ast` value. Returns the `Ast` handle on success, or
/// `Handle::NULL` on any parse failure (wrong header / malformed / trailing bytes / a non-`Ast` leaf) — the
/// compiler's `Core::AstDecode` emit wraps the result (`h != null → Ok(h)`, else `Err`), so decode is total
/// (a bad byte sequence is DATA, never a trap). BORROWS both operands.
pub(crate) fn op_ast_decode(bytes_handle: Handle, discs: Handle) -> Handle {
    let Some(d) = read_ast_enc_discs(discs) else {
        return Handle::NULL;
    };
    let n = op_bytes_len(bytes_handle);
    let mut raw = Vec::with_capacity(n as usize);
    for i in 0..n {
        raw.push(op_bytes_get(bytes_handle, i) as u8);
    }
    match crate::codec::decode(&raw) {
        Some(arenas) => decode_arenas_to_ast(&arenas, arenas.root, &d).unwrap_or(Handle::NULL),
        None => Handle::NULL,
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct Component;

#[cfg(target_arch = "wasm32")]
impl Guest for Component {
    fn box_int(v: i64) -> u32 {
        op_box_int(v).to_u32()
    }
    fn get_int(handle: u32) -> i64 {
        op_get_int(Handle::from_u32(handle))
    }
    fn box_bool(v: bool) -> u32 {
        op_box_bool(v).to_u32()
    }
    fn get_bool(handle: u32) -> bool {
        op_get_bool(Handle::from_u32(handle))
    }
    fn box_float(v: f64) -> u32 {
        op_box_float(v).to_u32()
    }
    fn get_float(handle: u32) -> f64 {
        op_get_float(Handle::from_u32(handle))
    }
    fn box_float32(v: f32) -> u32 {
        op_box_float32(v).to_u32()
    }
    fn get_float32(handle: u32) -> f32 {
        op_get_float32(Handle::from_u32(handle))
    }
    fn bigint_of_i64(v: i64) -> u32 {
        op_bigint_of_i64(v).to_u32()
    }
    fn bigint_of_bytes(buf: u32) -> u32 {
        op_bigint_of_bytes(Handle::from_u32(buf)).to_u32()
    }
    fn bigint_to_i64_checked(handle: u32) -> i64 {
        op_bigint_to_i64_checked(Handle::from_u32(handle))
    }
    fn bigint_add(a: u32, b: u32) -> u32 {
        op_bigint_add(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bigint_sub(a: u32, b: u32) -> u32 {
        op_bigint_sub(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bigint_mul(a: u32, b: u32) -> u32 {
        op_bigint_mul(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bigint_div(a: u32, b: u32) -> u32 {
        op_bigint_div(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bigint_cmp(a: u32, b: u32) -> i64 {
        op_bigint_cmp(Handle::from_u32(a), Handle::from_u32(b))
    }
    fn arr_alloc(len: u32) -> u32 {
        op_arr_alloc(len).to_u32()
    }
    fn arr_set(arr: u32, index: u32, elem: u32) -> u32 {
        op_arr_set(Handle::from_u32(arr), index, Handle::from_u32(elem)).to_u32()
    }
    fn arr_get(arr: u32, index: u32) -> u32 {
        op_arr_get(Handle::from_u32(arr), index).to_u32()
    }
    fn arr_len(arr: u32) -> u32 {
        op_arr_len(Handle::from_u32(arr))
    }
    fn sum_new(disc: u32, payload: u32) -> u32 {
        op_sum_new(disc, Handle::from_u32(payload)).to_u32()
    }
    fn sum_disc(handle: u32) -> u32 {
        op_sum_disc(Handle::from_u32(handle))
    }
    fn sum_payload(handle: u32) -> u32 {
        op_sum_payload(Handle::from_u32(handle)).to_u32()
    }
    fn bytes_alloc(len: u32) -> u32 {
        op_bytes_alloc(len).to_u32()
    }
    fn bytes_set(buf: u32, index: u32, value: u32) -> u32 {
        op_bytes_set(Handle::from_u32(buf), index, value).to_u32()
    }
    fn bytes_get(buf: u32, index: u32) -> u32 {
        op_bytes_get(Handle::from_u32(buf), index)
    }
    fn bytes_len(buf: u32) -> u32 {
        op_bytes_len(Handle::from_u32(buf))
    }
    fn bytes_scalar_at(buf: u32, scalar_index: u32) -> u32 {
        op_bytes_scalar_at(Handle::from_u32(buf), scalar_index)
    }
    fn str_new(s: String) -> u32 {
        op_str_new(s).to_u32()
    }
    fn str_get(handle: u32) -> String {
        op_str_get(Handle::from_u32(handle))
    }
    fn map_alloc(len: u32) -> u32 {
        op_map_alloc(len).to_u32()
    }
    fn map_set(m: u32, index: u32, key: u32, value: u32) -> u32 {
        op_map_set(
            Handle::from_u32(m),
            index,
            Handle::from_u32(key),
            Handle::from_u32(value),
        )
        .to_u32()
    }
    fn map_key(m: u32, index: u32) -> u32 {
        op_map_key(Handle::from_u32(m), index).to_u32()
    }
    fn map_val(m: u32, index: u32) -> u32 {
        op_map_val(Handle::from_u32(m), index).to_u32()
    }
    fn map_len(m: u32) -> u32 {
        op_map_len(Handle::from_u32(m))
    }

    // ── CHAMP persistent map (§37–45) ────────────────────────────────────────────────────
    fn map_empty() -> u32 {
        op_map_empty().to_u32()
    }
    fn map_insert(m: u32, key: u32, val: u32) -> u32 {
        op_map_insert(
            Handle::from_u32(m),
            Handle::from_u32(key),
            Handle::from_u32(val),
        )
        .to_u32()
    }
    fn map_lookup(m: u32, key: u32) -> u32 {
        op_map_lookup(Handle::from_u32(m), Handle::from_u32(key)).to_u32()
    }
    fn map_remove(m: u32, key: u32) -> u32 {
        op_map_remove(Handle::from_u32(m), Handle::from_u32(key)).to_u32()
    }
    fn map_size(m: u32) -> u32 {
        op_map_size(Handle::from_u32(m))
    }
    fn map_iter(m: u32) -> u32 {
        op_map_iter(Handle::from_u32(m)).to_u32()
    }
    fn map_iter_next(cur: u32) -> u32 {
        op_map_iter_next(Handle::from_u32(cur)).to_u32()
    }
    fn map_iter_key(cur: u32) -> u32 {
        op_map_iter_key(Handle::from_u32(cur)).to_u32()
    }
    fn map_iter_val(cur: u32) -> u32 {
        op_map_iter_val(Handle::from_u32(cur)).to_u32()
    }

    // ── CHAMP persistent set (§46–53) ────────────────────────────────────────────────────
    fn set_empty() -> u32 {
        op_set_empty().to_u32()
    }
    fn set_insert(s: u32, elem: u32) -> u32 {
        op_set_insert(Handle::from_u32(s), Handle::from_u32(elem)).to_u32()
    }
    fn set_contains(s: u32, elem: u32) -> bool {
        op_set_contains(Handle::from_u32(s), Handle::from_u32(elem))
    }
    fn set_remove(s: u32, elem: u32) -> u32 {
        op_set_remove(Handle::from_u32(s), Handle::from_u32(elem)).to_u32()
    }
    fn set_size(s: u32) -> u32 {
        op_set_size(Handle::from_u32(s))
    }
    fn set_iter(s: u32) -> u32 {
        op_set_iter(Handle::from_u32(s)).to_u32()
    }
    fn set_iter_next(cur: u32) -> u32 {
        op_set_iter_next(Handle::from_u32(cur)).to_u32()
    }
    fn set_iter_elem(cur: u32) -> u32 {
        op_set_iter_elem(Handle::from_u32(cur)).to_u32()
    }
    fn set_union(a: u32, b: u32) -> u32 {
        op_set_union(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn set_intersection(a: u32, b: u32) -> u32 {
        op_set_intersection(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn set_difference(a: u32, b: u32) -> u32 {
        op_set_difference(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }

    fn dup(handle: u32) {
        op_dup(Handle::from_u32(handle))
    }
    fn drop(handle: u32) {
        op_drop(Handle::from_u32(handle))
    }
    fn reset(node: u32) -> u32 {
        op_reset(Handle::from_u32(node)).to_u32()
    }
    fn arr_alloc_reuse(len: u32, token: u32) -> u32 {
        op_arr_alloc_reuse(len, Handle::from_u32(token)).to_u32()
    }
    fn sum_new_reuse(disc: u32, payload: u32, token: u32) -> u32 {
        op_sum_new_reuse(disc, Handle::from_u32(payload), Handle::from_u32(token)).to_u32()
    }
    fn vec_empty() -> u32 {
        op_vec_empty().to_u32()
    }
    fn vec_len(v: u32) -> u32 {
        op_vec_len(Handle::from_u32(v))
    }
    fn vec_get(v: u32, index: u32) -> u32 {
        op_vec_get(Handle::from_u32(v), index).to_u32()
    }
    fn vec_push(v: u32, elem: u32) -> u32 {
        op_vec_push(Handle::from_u32(v), Handle::from_u32(elem)).to_u32()
    }
    fn vec_prepend(v: u32, elem: u32) -> u32 {
        op_vec_prepend(Handle::from_u32(v), Handle::from_u32(elem)).to_u32()
    }
    fn vec_update(v: u32, index: u32, elem: u32) -> u32 {
        op_vec_update(Handle::from_u32(v), index, Handle::from_u32(elem)).to_u32()
    }
    fn vec_concat(a: u32, b: u32) -> u32 {
        op_vec_concat(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn vec_split(v: u32, index: u32) -> (u32, u32) {
        let (l, r) = op_vec_split(Handle::from_u32(v), index);
        (l.to_u32(), r.to_u32())
    }
    fn vec_drop(v: u32, index: u32) -> u32 {
        // The tail `[index, len)` — builds ONLY the kept spine (no discarded left prefix). Byte-identical
        // to `split`+drop-left (guarded by `vec_drop_tail_matches_split_drop_left`), ~half the allocation.
        op_vec_drop_tail(Handle::from_u32(v), index).to_u32()
    }
    fn bigint_rem(a: u32, b: u32) -> u32 {
        op_bigint_rem(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_of(num: u32, den: u32) -> u32 {
        op_rational_of(Handle::from_u32(num), Handle::from_u32(den)).to_u32()
    }
    fn rational_num(r: u32) -> u32 {
        op_rational_num(Handle::from_u32(r)).to_u32()
    }
    fn rational_den(r: u32) -> u32 {
        op_rational_den(Handle::from_u32(r)).to_u32()
    }
    fn rational_add(a: u32, b: u32) -> u32 {
        op_rational_add(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_sub(a: u32, b: u32) -> u32 {
        op_rational_sub(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_mul(a: u32, b: u32) -> u32 {
        op_rational_mul(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_div(a: u32, b: u32) -> u32 {
        op_rational_div(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn rational_cmp(a: u32, b: u32) -> i64 {
        op_rational_cmp(Handle::from_u32(a), Handle::from_u32(b))
    }
    fn vec_of_arr(arr: u32) -> u32 {
        op_vec_of_arr(Handle::from_u32(arr)).to_u32()
    }
    // Structural value equality (index 61) — the deep heap walk behind `=` on two runtime compounds.
    // BORROWS both operands (an inspector, like `set-contains`): `champ_eq` reads without touching
    // either refcount, so the caller drops a temporary operand itself. This is the SAME tagless
    // structural comparison the map/set key path runs, exposed for the language's `=`.
    fn value_eq(a: u32, b: u32) -> bool {
        champ_eq(Handle::from_u32(a), Handle::from_u32(b))
    }
    // Value-form encode (index 62) — render a runtime value to its canonical binary-AST document,
    // guided by the compiler-baked shape descriptor `desc` (a Bytes handle). BORROWS both `v` and
    // `desc` (an inspector — the caller/escape owns the release of `v`; `desc` is a constant). Returns a
    // fresh owned Bytes. A malformed descriptor / unrenderable shape yields the empty Bytes (the
    // compiler only bakes a well-formed descriptor, so this is a defensive total, never a trap).
    fn value_encode(v: u32, desc: u32) -> u32 {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let doc = op_value_encode_form(Handle::from_u32(v), &bytes).unwrap_or_default();
        alloc(Vec::new(), doc).to_u32()
    }
    // Value-form DECODE (index 90) — the exact inverse of value-encode: read the canonical value-form
    // `bytes` document + the SAME shape `desc` value-encode reads, and CONSTRUCT a fresh owned heap value.
    // BORROWS `bytes` + `desc` (both constants/inputs the caller owns); returns a fresh owned handle (or the
    // NULL handle `0` on a shape/format mismatch — never traps, mirroring value-encode's malformed-desc
    // decline). See `op_value_decode`.
    fn value_decode(bytes: u32, desc: u32) -> u32 {
        let desc_h = Handle::from_u32(desc);
        let dn = op_bytes_len(desc_h);
        let mut desc_bytes = Vec::with_capacity(dn as usize);
        for i in 0..dn {
            desc_bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let doc_h = Handle::from_u32(bytes);
        let bn = op_bytes_len(doc_h);
        let mut doc_bytes = Vec::with_capacity(bn as usize);
        for i in 0..bn {
            doc_bytes.push(op_bytes_get(doc_h, i) as u8);
        }
        op_value_decode(&doc_bytes, &desc_bytes).to_u32()
    }
    // BLAKE3 content hash (index 91) — the digest of `bytes`'s Bytes-leaf contents as a fresh 32-byte Bytes
    // leaf. A generic `bytes -> digest` primitive (no tag/prefix — userspace owns domain separation, DESIGN-
    // compiler-primitives.md D7); the runtime half of the compiler's `Blake3.of`, sharing the one `blake3`
    // crate with the compile-time fold so both agree bit-for-bit. BORROWS `bytes` (an inspector); returns a
    // fresh owned handle the caller drops. See `op_hash_blake3`.
    fn hash_blake3(bytes: u32) -> u32 {
        op_hash_blake3(Handle::from_u32(bytes)).to_u32()
    }
    // Ast render (index 92) — the runtime half of `Ast.print`: a runtime Ast heap value → its canonical
    // s-expr text (a fresh String leaf), byte-identical to the compiler's print_ast_value. BORROWS both;
    // `discs` conveys the Ast variant discs (baked by the compiler, by-name — never hardcoded). See
    // `op_ast_print`.
    fn ast_print(handle: u32, discs: u32) -> u32 {
        op_ast_print(Handle::from_u32(handle), Handle::from_u32(discs)).to_u32()
    }
    fn ast_encode(handle: u32, discs: u32) -> u32 {
        op_ast_encode(Handle::from_u32(handle), Handle::from_u32(discs)).to_u32()
    }
    fn ast_decode(bytes_handle: u32, discs: u32) -> u32 {
        op_ast_decode(Handle::from_u32(bytes_handle), Handle::from_u32(discs)).to_u32()
    }
    // mark-immortal (index 95) — convert a build-once static heap node to IMMORTAL (dup/drop no-op +
    // census-excluded). See `op_mark_immortal`.
    fn mark_immortal(handle: u32) -> u32 {
        op_mark_immortal(Handle::from_u32(handle)).to_u32()
    }
    // Mark-immortal-DEEP (index 96) — transitively mark a heap value AND every node reachable through its
    // child handles IMMORTAL (RRB list interior+leaf nodes, CHAMP map interior nodes + `[k,v]` entries, and
    // the k/v/element payloads they own). The deep analogue of `mark-immortal` for a build-once static whose
    // value is a multi-node structure (a `>32` list, a map) with no compile-time per-node handle. See
    // `op_mark_immortal_deep`.
    fn mark_immortal_deep(handle: u32) -> u32 {
        op_mark_immortal_deep(Handle::from_u32(handle)).to_u32()
    }
    // Value-form COMPARE (index 86) — the blessed three-way order over two runtime compound values of the
    // same type, guided by the compiler-baked shape `desc` (read exactly as `value-encode` reads it). BORROWS
    // `a`, `b` (an inspector — the caller owns their release) and `desc` (a constant). Returns -1/0/1
    // (Less/Equal/Greater) or the sentinel 2 when the type offers no total order or the descriptor is
    // malformed (the compiler declines ordering for a non-orderable type, so 2 is a defensive not-reached).
    fn value_cmp(a: u32, b: u32, desc: u32) -> i32 {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let Some(descriptor) = decode_descriptor(&bytes) else {
            return 2; // malformed descriptor — unordered sentinel
        };
        match value_cmp_shaped(
            &descriptor,
            Handle::from_u32(a),
            Handle::from_u32(b),
            descriptor.root,
        ) {
            Some(core::cmp::Ordering::Less) => -1,
            Some(core::cmp::Ordering::Equal) => 0,
            Some(core::cmp::Ordering::Greater) => 1,
            None => 2, // a non-orderable shape (float/bytes/set/map leaf) — unordered sentinel
        }
    }
    // Value-form structural EQUALITY (index 88) — the descriptor-guided companion of `value-eq` (index 61).
    // `value-eq` is the tagless `champ_eq` PHYSICAL-byte walk (sound for a canonical-by-construction value);
    // this walks the shape descriptor element-by-element, so it is exact for a LIST (an RRB vector that is
    // element- but not shape-canonical) and for a FLOAT/BYTES leaf a list carries (byte-canonical equality —
    // nan==nan, -0.0≠+0.0 — which `value-cmp` DECLINES since a float offers equality but no total order).
    // BORROWS `a`, `b` (an inspector — the caller owns their release) and `desc` (a constant). A malformed
    // descriptor / unrepresentable shape reads as `false` (defensive total — the compiler bakes a well-formed
    // descriptor, so this is a not-reached). Consistent with `value-cmp`: `value-eq-shaped == true` iff
    // `value-cmp == 0` for an orderable type.
    fn value_eq_shaped(a: u32, b: u32, desc: u32) -> bool {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let Some(descriptor) = decode_descriptor(&bytes) else {
            return false; // malformed descriptor — defensive not-equal (never reached)
        };
        crate::value_eq_shaped(
            &descriptor,
            Handle::from_u32(a),
            Handle::from_u32(b),
            descriptor.root,
        )
        .unwrap_or(false) // an unrepresentable shape reads as not-equal (defensive total)
    }
    // Value CANONICALIZE (index 87) — the blessed canonical form of a runtime value of the type `desc`
    // describes: a fresh OWNED value byte-identical for any two values EQUAL as values, whatever their
    // construction. Emitted at a Map/Set KEY site for a list-typed (or list-containing) key so the tagless
    // CHAMP byte-walk (`champ_hash`/`champ_eq`) places construction-equal list keys in the SAME slot
    // (collections-and-text.md §162 — a key's identity is construction-independent). BORROWS `a` (the
    // caller retains/releases it) and `desc` (a constant); returns a fresh owned handle the caller drops
    // after a borrowing key op, exactly like a `bytes-compact`ed rope key. On a malformed descriptor the
    // canonicalize declines and we return a DUP of the input (identity — degrades to the pre-fix byte-walk,
    // never a trap, never a leak): the op is total.
    fn value_canonicalize(a: u32, desc: u32) -> u32 {
        let a_h = Handle::from_u32(a);
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        let out = match decode_descriptor(&bytes) {
            Some(descriptor) => value_canonicalize_shaped(&descriptor, a_h, descriptor.root),
            None => None,
        };
        match out {
            Some(h) => h.to_u32(),
            None => {
                op_dup(a_h); // decline → fresh owned identity (never trap/leak)
                a_h.to_u32()
            }
        }
    }
    // `set-to-list(s, desc)` (index 83) — a SET's elements as a `List a` in canonical element-value order,
    // and `map-to-list(m, desc)` (index 84) — a MAP's entries as a `List (Tuple k v)` in canonical KEY
    // order. Both BORROW their collection + the compiler-baked shape `desc` (a Bytes handle read the same
    // way `value-encode` reads it), reuse the sorted canonical walk value-encode renders from (so program
    // iteration order == the canonical byte form, collections-and-text.md:149), and return a fresh owned
    // `List` handle. A malformed descriptor / non-scalar unorderable key/element yields the empty list.
    fn set_to_list(s: u32, desc: u32) -> u32 {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        op_set_to_list(Handle::from_u32(s), &bytes).to_u32()
    }
    fn map_to_list(m: u32, desc: u32) -> u32 {
        let desc_h = Handle::from_u32(desc);
        let n = op_bytes_len(desc_h);
        let mut bytes = Vec::with_capacity(n as usize);
        for i in 0..n {
            bytes.push(op_bytes_get(desc_h, i) as u8);
        }
        op_map_to_list(Handle::from_u32(m), &bytes).to_u32()
    }
    fn bytes_concat(a: u32, b: u32) -> u32 {
        op_bytes_concat(Handle::from_u32(a), Handle::from_u32(b)).to_u32()
    }
    fn bytes_slice(buf: u32, start: u32, len: u32) -> u32 {
        op_bytes_slice(Handle::from_u32(buf), start, len).to_u32()
    }
    fn bytes_compact(buf: u32) -> u32 {
        op_bytes_compact(Handle::from_u32(buf)).to_u32()
    }
    fn str_nfc_normalize(s: u32) -> u32 {
        op_str_nfc(Handle::from_u32(s)).to_u32()
    }
    fn str_from_bytes(buf: u32) -> u32 {
        op_str_from_bytes(Handle::from_u32(buf)).to_u32()
    }
    // Debug leak oracle (index 54). The number of live heap objects, or 0 when the counter is not
    // compiled in (default build). Not imported by the compiler; a leak-check harness asserts it is 0
    // after a run to verify the Perceus dup/drop discipline balances.
    fn live_objects() -> u32 {
        live_object_count()
    }
}

// The rc-trace leak-attribution DRAIN EXPORT (the `debug-trace` WIT interface). Gated on the
// `rc-trace-export` feature (NOT `debug-counters`), so it is present ONLY in the rc-trace variant — the
// nix flake build that enables `rc-trace-export` AND targets `world runtime-debug` (heap + debug-trace),
// for `cdz-run --rc-trace`. It is CFG'D OUT of both (a) the release runtime (no feature, `world runtime`,
// 058B5h untouched) AND (b) the plain debug-counters leak-check runtime (feature off → `world runtime`,
// heap-only bindings with no `debug-trace` trait → no E0433 in xtask codegen / the gate). `rc-trace-export`
// implies `debug-counters`, so the instrumentation + the crate-root rc-trace fns this wraps are present.
// Thin wrappers over those fns (the serialization + enable/flag LOGIC is native-tested in lib.rs). The
// exact wit-bindgen trait PATH below is v-nix's debug-build compile-gate (the regenerated runtime-debug
// bindings are the authority — v-nix's build already validated it compiles; only rc-trace-enable's arg
// needed the WIT `on: bool`).
#[cfg(all(target_arch = "wasm32", feature = "rc-trace-export"))]
impl bindings::exports::cadenza::runtime::debug_trace::Guest for Component {
    fn rc_trace_enable(on: bool) {
        crate::rc_trace_enable(on);
    }
    fn rc_trace_drain() -> alloc::vec::Vec<u8> {
        crate::rc_trace_drain_bytes()
    }
    fn rc_trace_truncated() -> bool {
        crate::rc_trace_truncated_flag()
    }
}

#[cfg(target_arch = "wasm32")]
bindings::export!(Component with_types_in bindings);

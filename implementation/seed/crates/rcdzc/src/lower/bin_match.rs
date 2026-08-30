//! `lower::bin_match` — binary (`bin`) construction + pattern matching, split out of `lower.rs`. Lowers
//! `(bin <segment>…)` construction (`lower_bin_build`, incl. the CDZ0304 overrange diagnostic), and the
//! `bin` MATCH path: constant scrutinee folding, field decode (static/dynamic offsets, bit-fields,
//! size/len-dependent + UTF-8 fields), the runtime decode `if`-chain, and the per-arm predicate builder.
//! Behaviour-preserving move: all items are module-private (now `pub(super)`), reached across the tree
//! via a plain `use bin_match::*` re-import in `lower` (and the siblings' own `use super::*`).

use super::*;

/// Lower `(bin <segment>…)` in EXPRESSION position — construct a `Bytes`. Realizes the FIXED-WIDTH
/// INTEGER segments (`uNN`/`iNN`, big-endian, `le` modifier) and BIT-FIELDS (`bits v k`): a CONSTANT
/// segment folds to its encoded bytes, assembled across all segments into a single `Core::BytesOf` of
/// synthesized `UInt8` `Leaf::Int` elems — so a constant `(bin …)` bakes/compares/slices exactly like
/// `(Bytes.of (list …))`, no runtime op. An int emits its `w` two's-complement bytes (MSB-first, reversed
/// for `le`); a `bits k` shifts `k` bits MSB-first into a bit-accumulator that flushes whole bytes as
/// they close (the whole `bin` is byte-aligned — CDZ0220, checked in infer — so the accumulator is empty
/// at every int/bytes segment and at the end). A value OUT OF RANGE for its segment (`(u8 256)`, `(u8
/// -1)`, a `bits k` value ≥ 2^k) is a compile-provable trap (CDZ0304 — the build-fail companion of the
/// runtime "binary value does not fit segment" trap). `(bin)` (no segments) is the empty byte sequence.
/// A `bytes` splice, or a RUNTIME (non-constant) value, is not folded here yet — declines cleanly (BN4
/// dependent-bytes + the runtime path).
/// The rich CDZ0304 message for a CONSTANT value that does not fit its `(signed, bits)` bin INTEGER/BITS
/// segment — names the offending VALUE, the segment's width TYPE, and the VALID RANGE, mirroring the
/// annotation-position CDZ0302 ("the valid range is 0..=255") rather than the terse "binary value does
/// not fit segment". A `bits k` field is an UNSIGNED k-bit value (`signed=false`). `bits` is the segment's
/// value width in bits (a byte-aligned int segment is `w*8`; a `(bits k)` field is `k`). The width type is
/// spelled off the aliasing — a bound name for an aliased width (`UInt8`), the `(UInt k)` ctor form
/// otherwise (a bit-field's `(UInt 4)`), reusing `width_module_spelling` so the named type actually
/// resolves. The range clause is omitted only for a malformed width `int_width_range` can't render.
pub(super) fn bin_segment_overrange_message(
    v: &crate::ast::IntValue,
    signed: bool,
    bits: u32,
) -> String {
    let ty = crate::infer::width_module_spelling(&crate::ty::IntTy::fixed(signed, bits));
    let val = v.to_decimal_string();
    match crate::infer::int_width_range(signed, bits) {
        Some(range) => format!(
            "the value {val} does not fit this bin segment's {bits}-bit {ty} field (the valid range \
             is {range}) — a bin segment never truncates; narrow the value explicitly to fit",
        ),
        None => format!(
            "the value {val} does not fit this bin segment's {bits}-bit {ty} field — a bin segment \
             never truncates; narrow the value explicitly to fit",
        ),
    }
}

pub(super) fn lower_bin_build(
    db: &mut Db,
    id: StructId,
    segs: &[crate::resolved::Segment],
) -> Core {
    use crate::resolved::SegKind;
    // RUNTIME construction: if ANY segment's value is not a compile-time constant, the `bin` can't fold to
    // a baked `Core::BytesOf` — it builds at run time. This slice handles a `bin` of ONLY fixed-width
    // INTEGER segments (a runtime `bits`/`bytes` segment is a later increment); such a `bin` lowers to a
    // `Core::BinBuild` the backend emits (alloc + per-segment range-check-and-write). A constant segment
    // still range-checks here (a provable trap fails the build even alongside a runtime sibling). A `bin`
    // mixing a runtime value with a `bits`/`bytes` segment declines (not yet built).
    let any_runtime = segs.iter().any(|s| match &s.kind {
        // A runtime INT value (a param, not a `ConstInt`) — the segment builds at run time.
        SegKind::Int { .. } => !matches!(core_of(db, s.slot), Core::ConstInt(_) | Core::Poison(_)),
        // A `(bytes b)` splice whose `b` is not a compile-time-visible constant Bytes — spliced at run
        // time via `bytes-concat`. (`bin_const_scrutinee` = Some only for a visible `Core::BytesOf`.)
        SegKind::Bytes { .. } => bin_const_scrutinee(db, s.slot).is_none(),
        // A runtime bit-field value (a param, not a `ConstInt`) — the run packs at run time.
        SegKind::Bits { .. } => !matches!(core_of(db, s.slot), Core::ConstInt(_) | Core::Poison(_)),
        // A `utf8` segment is a PATTERN-only construct here — building a `(utf8 s n)` (splice a String's
        // bytes) is not yet lowered; route it to the const-build loop, which declines cleanly.
        SegKind::Utf8 { .. } => false,
    });
    if any_runtime {
        // Build the `bin` as a sequence of PIECES concatenated at run time (`Core::BytesConcat`): each
        // maximal RUN of fixed-width int segments is one `Core::BinBuild` piece, each maximal RUN of
        // bit-fields is one `Core::BinBitsBuild` piece (byte-aligned — CDZ0220 closes a `bits` run to a
        // whole byte before any int/bytes segment and at the end), and each `(bytes b)` SPLICE segment
        // contributes `b` directly. Composes headers/bit-flags with a runtime bytes body via `bytes-concat`.
        let mut pieces: Vec<StructId> = Vec::new();
        let mut int_run: Vec<crate::core::BinSeg> = Vec::new();
        let mut bits_run: Vec<crate::core::BinBitsField> = Vec::new();
        // Flush the current int-run as a `Core::BinBuild` piece (synthesized, so it emits standalone).
        let flush_ints =
            |db: &mut Db, run: &mut Vec<crate::core::BinSeg>, pieces: &mut Vec<StructId>| {
                if !run.is_empty() {
                    let piece = synth_core(
                        db,
                        Core::BinBuild {
                            segs: std::mem::take(run),
                        },
                        crate::ty::Ty::Bytes,
                    );
                    pieces.push(piece);
                }
            };
        // Flush the current bit-field run as a `Core::BinBitsBuild` piece (byte-aligned per CDZ0220).
        let flush_bits =
            |db: &mut Db, run: &mut Vec<crate::core::BinBitsField>, pieces: &mut Vec<StructId>| {
                if !run.is_empty() {
                    let piece = synth_core(
                        db,
                        Core::BinBitsBuild {
                            fields: std::mem::take(run),
                        },
                        crate::ty::Ty::Bytes,
                    );
                    pieces.push(piece);
                }
            };
        for seg in segs {
            match &seg.kind {
                SegKind::Int { width, signed } => {
                    if let Core::Poison(r) = core_of(db, seg.slot) {
                        return Core::Poison(r);
                    }
                    // An int segment is byte-aligned — close any open bit-field run first (order-preserving).
                    flush_bits(db, &mut bits_run, &mut pieces);
                    // A CONSTANT sibling still range-checks (a provable misfit fails the build).
                    if let Core::ConstInt(v) = core_of(db, seg.slot)
                        && !v.fits_width(*signed, (*width as u32) * 8)
                    {
                        return Core::Poison(
                            Reject::coded(
                                Code::ConstTrap,
                                bin_segment_overrange_message(&v, *signed, (*width as u32) * 8),
                            )
                            .at(seg.slot),
                        );
                    }
                    int_run.push(crate::core::BinSeg {
                        width: *width,
                        signed: *signed,
                        little_endian: seg.little_endian,
                        value: seg.slot,
                    });
                }
                // A `(bits v k)` bit-field: close any open int-run first, then extend the bit-field run.
                // The run is byte-aligned as a whole (CDZ0220), so it flushes to a `Core::BinBitsBuild`.
                SegKind::Bits { k } => {
                    if let Core::Poison(r) = core_of(db, seg.slot) {
                        return Core::Poison(r);
                    }
                    let k = *k;
                    // `k` must be a usable runtime field width (the u64 pack accumulator carries ≤ 7 open
                    // bits between flushes, so `7 + k <= 64` keeps the `acc << k` shift lossless). A wider
                    // runtime bit-field declines (the constant path still handles k ≤ 63).
                    if k == 0 || k > 56 {
                        return Core::Poison(Reject::decline(
                            "a runtime bin bit-field wider than 56 bits is not yet built",
                        ));
                    }
                    // A CONSTANT bit-field sibling still range-checks (a k-bit UNSIGNED field; misfit → trap).
                    if let Core::ConstInt(v) = core_of(db, seg.slot)
                        && !v.fits_width(false, k)
                    {
                        return Core::Poison(
                            Reject::coded(
                                Code::ConstTrap,
                                bin_segment_overrange_message(&v, false, k),
                            )
                            .at(seg.slot),
                        );
                    }
                    flush_ints(db, &mut int_run, &mut pieces);
                    bits_run.push(crate::core::BinBitsField { k, value: seg.slot });
                }
                // A `(bytes b)` splice: flush both runs, then splice `b` (a Bytes value). A dependent
                // size `(bytes b n)` on CONSTRUCTION is a length constraint the const path checks; a
                // RUNTIME sized splice (a runtime `b`/`n`) is not checked yet — decline it.
                SegKind::Bytes { size } => {
                    if let Core::Poison(r) = core_of(db, seg.slot) {
                        return Core::Poison(r);
                    }
                    if size.is_some() {
                        return Core::Poison(Reject::decline(
                            "a runtime sized (bytes b n) construction is not yet built",
                        ));
                    }
                    if crate::infer::type_of(db, seg.slot) != crate::ty::Ty::Bytes {
                        return Core::Poison(Reject::decline(
                            "a bin bytes splice operand is not a Bytes value",
                        ));
                    }
                    flush_ints(db, &mut int_run, &mut pieces);
                    flush_bits(db, &mut bits_run, &mut pieces);
                    pieces.push(seg.slot);
                }
                // Constructing a `(utf8 s n)` segment (splice a String's bytes) is not yet lowered — the
                // `utf8` segment is currently pattern-only (`bin_match_decode`). Decline cleanly.
                SegKind::Utf8 { .. } => {
                    return Core::Poison(Reject::decline(
                        "constructing a utf8 bin segment is not yet built (utf8 is pattern-only)",
                    ));
                }
            }
        }
        flush_ints(db, &mut int_run, &mut pieces);
        flush_bits(db, &mut bits_run, &mut pieces);
        // Concatenate the pieces left-to-right. Zero pieces = the empty bin (empty Bytes); one piece is
        // itself; else fold to a chain of `Core::BytesConcat`.
        let mut iter = pieces.into_iter();
        let Some(first) = iter.next() else {
            return Core::BytesOf {
                elems: std::rc::Rc::from([]),
            }; // (bin) with only… nothing — empty
        };
        let mut acc = first;
        for piece in iter {
            acc = synth_core(
                db,
                Core::BytesConcat {
                    lhs: acc,
                    rhs: piece,
                },
                crate::ty::Ty::Bytes,
            );
        }
        return core_of(db, acc);
    }
    let mut raw: Vec<u8> = Vec::new();
    // The open bit-accumulator between `bits` segments: `acc` holds `nbits` bits, MSB-first (the first
    // field's bits occupy the high end). Whole bytes are flushed to `raw` as soon as `nbits >= 8`.
    let mut acc: u64 = 0;
    let mut nbits: u32 = 0;
    for seg in segs {
        match &seg.kind {
            SegKind::Int { width, signed } => {
                let w = *width as u32;
                let bits = w * 8;
                match core_of(db, seg.slot) {
                    Core::Poison(r) => return Core::Poison(r),
                    Core::ConstInt(v) => {
                        // Range: the value must fit the segment's (signed, bits) width — else a provable
                        // trap (never truncate). `(u8 256)`/`(u8 -1)` fail here.
                        if !v.fits_width(*signed, bits) {
                            return Core::Poison(
                                Reject::coded(
                                    Code::ConstTrap,
                                    bin_segment_overrange_message(&v, *signed, bits),
                                )
                                .at(seg.slot),
                            );
                        }
                        // The low `w` bytes of the value's two's-complement representation, big-endian
                        // (MSB first). `to_i64_bits` gives the 64-bit two's-complement pattern; for a
                        // signed negative this already has the right high bits within `w` (checked to fit).
                        let word = v.to_i64_bits() as u64;
                        let mut be: Vec<u8> = (0..w)
                            .rev()
                            .map(|i| ((word >> (i * 8)) & 0xff) as u8)
                            .collect();
                        if seg.little_endian {
                            be.reverse();
                        }
                        raw.extend(be);
                    }
                    // A runtime integer value — the runtime construction path (BN4). Decline for now.
                    _ => {
                        return Core::Poison(Reject::decline(
                            "a bin segment with a runtime value is not yet built (constant segments only)",
                        ));
                    }
                }
            }
            // A bit-field `(bits v k)`: the low `k` bits of `v`, packed MSB-first into the accumulator.
            // `v` must fit `k` UNSIGNED bits (`bits 2 1` — 2 needs two bits, has one — traps). k ≤ 63 keeps
            // `acc` (a u64) from overflowing between flushes (a whole `bin` is byte-aligned, so ≤ 7 bits
            // are ever carried across a segment; a single field ≤ 63 bits fits with room to flush).
            SegKind::Bits { k } => {
                let k = *k;
                match core_of(db, seg.slot) {
                    Core::Poison(r) => return Core::Poison(r),
                    Core::ConstInt(v) => {
                        // A malformed bit width (0, or > 63 here) is a well-formedness fault (infer's
                        // CDZ0220 normally catches it); keep the terse message for that degenerate case.
                        if k == 0 || k > 63 {
                            return Core::Poison(Reject::coded(
                                Code::ConstTrap,
                                "binary value does not fit segment",
                            ));
                        }
                        // A bit-field is an unsigned k-bit value; out of range (or negative) → the rich
                        // "value V does not fit this K-bit (UInt K) field (valid range 0..=…)" message.
                        if !v.fits_width(false, k) {
                            return Core::Poison(
                                Reject::coded(
                                    Code::ConstTrap,
                                    bin_segment_overrange_message(&v, false, k),
                                )
                                .at(seg.slot),
                            );
                        }
                        let val = v.to_i64_bits() as u64 & ((1u64 << k) - 1);
                        acc = (acc << k) | val;
                        nbits += k;
                        // Flush every whole byte from the TOP of the accumulator (MSB-first).
                        while nbits >= 8 {
                            let shift = nbits - 8;
                            raw.push(((acc >> shift) & 0xff) as u8);
                            nbits -= 8;
                            acc &= (1u64 << nbits) - 1; // keep only the still-open low bits
                        }
                    }
                    _ => {
                        return Core::Poison(Reject::decline(
                            "a bin bit-field with a runtime value is not yet built (constant segments only)",
                        ));
                    }
                }
            }
            // A `(bytes b [n])` splice — append all of the constant byte sequence `b`. A dependent size
            // `n` (`(bytes b n)`) is a LENGTH CONSTRAINT on construction: `|b|` must equal `n`, else the
            // value does not fit its segment → trap (CDZ0304 for a constant). The whole `bin` is
            // byte-aligned (CDZ0220), so the accumulator is empty here.
            SegKind::Bytes { size } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at a bytes segment"
                );
                let Some(bytes) = bin_const_scrutinee(db, seg.slot) else {
                    if let Core::Poison(r) = core_of(db, seg.slot) {
                        return Core::Poison(r);
                    }
                    return Core::Poison(Reject::decline(
                        "a bin bytes segment with a runtime value is not yet built (constant only)",
                    ));
                };
                if let Some(n_occ) = size {
                    match core_of(db, *n_occ) {
                        Core::ConstInt(v) => {
                            if v.to_i64().filter(|n| *n >= 0) != Some(bytes.len() as i64) {
                                return Core::Poison(Reject::coded(
                                    Code::ConstTrap,
                                    "binary value does not fit segment",
                                ));
                            }
                        }
                        Core::Poison(r) => return Core::Poison(r),
                        _ => {
                            return Core::Poison(Reject::decline(
                                "a bin bytes segment size is not a compile-time constant (not yet built)",
                            ));
                        }
                    }
                }
                raw.extend(bytes);
            }
            // Constructing a `(utf8 s n)` segment (splice a String's UTF-8 bytes) is not yet lowered —
            // `utf8` is currently pattern-only (`bin_match_decode`). Decline cleanly.
            SegKind::Utf8 { .. } => {
                return Core::Poison(Reject::decline(
                    "constructing a utf8 bin segment is not yet built (utf8 is pattern-only)",
                ));
            }
        }
    }
    // A well-formed `bin` is byte-aligned, so no open bits remain here (CDZ0220 caught a mis-aligned one
    // in infer before this runs). Defensively: any residual open bits mean an ill-formed form slipped
    // through — decline rather than emit a wrong byte count.
    if nbits != 0 {
        return Core::Poison(Reject::coded(
            Code::IllFormedBinary,
            "a bin form's bit-fields must close to a whole number of bytes",
        ));
    }
    // Assemble the emitted bytes into a constant `Core::BytesOf` (synthesized UInt8 element leaves), the
    // same shape `b"…"`/`String.to-bytes` produce — so it rides the constant-Bytes fold/escape/equality.
    trace!(target: "rcdzc::lower", node = id.0, len = raw.len(), "bin construction folds to a constant Bytes");
    let elems: Vec<StructId> = raw
        .iter()
        .map(|&b| {
            db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i64(b as i64),
                radix: crate::ast::Radix::Dec,
            })
        })
        .collect();
    Core::BytesOf {
        elems: elems.into(),
    }
}

/// The constant bytes of `scrutinee` if it reduces to a compile-time-visible `Core::BytesOf` (a `(bin
/// …)`/`(Bytes.of …)`/`b"…"` constant) — `None` for a runtime Bytes (a parameter, a concat result), which
/// takes the BN4 runtime cursor path. Each element must be a `ConstInt` in `0..=255`.
pub(super) fn bin_const_scrutinee(db: &mut Db, scrutinee: StructId) -> Option<Vec<u8>> {
    let Core::BytesOf { elems } = core_of(db, scrutinee) else {
        return None;
    };
    let mut raw = Vec::with_capacity(elems.len());
    for e in elems.iter().copied() {
        match core_of(db, e) {
            Core::ConstInt(v) => raw.push(v.to_i64().filter(|n| (0..=255).contains(n))? as u8),
            _ => return None,
        }
    }
    Some(raw)
}

/// One decoded `bin` segment against a concrete byte sequence: an integer (an `Int`/`Bits` segment's
/// value) or a byte RANGE `[start, end)` into the scrutinee (a `Bytes` segment). Used both to decide a
/// match (literal probes + whole-scrutinee close) and to bind a segment binder (`decode_bin_field`).
pub(super) enum BinDecoded {
    /// A decoded integer segment's value, as an ARBITRARY-PRECISION `IntValue` — NOT an `i64`. A `u64`
    /// segment with the top bit set (a genuine `UInt64 > Int64.max`, e.g. bytes `[128,0,…,0,1]` = 2^63+1)
    /// has no `i64`, so an `i64` decode would store it as its WRAPPED NEGATIVE, and the const-fold path
    /// (`decode_bin_field` → `Core::ConstInt`, and the literal-probe compare) would fold the wrong
    /// (negative) value — the const-eval twin of the runtime `(bin (u64 n))` signed-binding miscompile
    /// (infer.rs `Resolved::BinField`). Carrying the full `IntValue` keeps the decode faithful for every
    /// width/sign; a size operand that needs a machine count still reads `.to_i64()` (a size is small).
    Int(IntValue),
    ByteRange(usize, usize),
    /// A `utf8` segment's decoded string — the byte range validated as strict UTF-8 (its match already
    /// required well-formedness, so this is a real `String`). Kept alongside the range so a binder can
    /// bind the decoded `String` directly.
    Str(String),
}

/// Run a `bin` PATTERN's segment automaton over the concrete bytes `raw`, left-to-right. Returns each
/// segment's decoded value if the pattern MATCHES the WHOLE sequence, else `None` (a non-match: a
/// fixed-width segment overruns the input, a bit-field run does not close, a dependent size overruns the
/// remainder, or bytes are left unconsumed with no trailing unsized `(bytes …)`). Handles fixed-width
/// ints, bit-fields, a FINAL unsized `(bytes rest)`, and a DEPENDENT-size `(bytes body n)` (`n` names an
/// earlier INT segment binder — resolved to that segment's already-decoded value). The literal-vs-binder
/// distinction is the CALLER's (a literal slot must equal the decoded int); here we decode every
/// segment's raw value + enforce widths/consumption.
/// Resolve a sized bin-segment's SIZE operand (`(bytes b SIZE)` / `(utf8 s SIZE)`) to a byte count against
/// the already-decoded prefix `out`. The size is EITHER a NAME referencing an earlier integer segment's
/// binder (a dependent size — the crown-jewel form), OR a CONSTANT INTEGER LITERAL (a fixed size — ruling
/// (a) 2026-07-21: a literal size is the most basic case, Erlang bit-syntax precedent, and MUST match, not
/// silently fall through). Returns the non-negative byte count, or `None` (non-match) if the operand is
/// neither a resolvable earlier-segment name nor a constant int, or the resolved value is negative.
pub(super) fn bin_decode_dependent_size(
    db: &Db,
    size_occ: StructId,
    segs: &[crate::resolved::Segment],
    seg_index: usize,
    out: &[BinDecoded],
) -> Option<usize> {
    let v = if let Some(name) = db.ast.as_name(size_occ) {
        // A NAME → the decoded Int of the earlier segment binding it (a forward / non-int ref → None).
        segs.iter()
            .take(seg_index)
            .position(|s| db.ast.as_name(s.slot) == Some(name))
            .and_then(|idx| match out.get(idx) {
                // A size is a byte count — small by construction; read it as a machine `i64` (a value
                // beyond i64 is not a plausible size → `to_i64` → None → non-match).
                Some(BinDecoded::Int(v)) => v.to_i64(),
                _ => None,
            })?
    } else {
        // A CONSTANT INTEGER LITERAL size (ruling (a)) — read it directly, no earlier segment needed.
        db.ast.as_int(size_occ)?.to_i64()?
    };
    (v >= 0).then_some(v as usize)
}

pub(super) fn bin_match_decode(
    db: &Db,
    raw: &[u8],
    segs: &[crate::resolved::Segment],
) -> Option<Vec<BinDecoded>> {
    use crate::resolved::SegKind;
    let mut out: Vec<BinDecoded> = Vec::with_capacity(segs.len());
    let mut off: usize = 0; // byte offset
    let mut acc: u64 = 0; // open bit-accumulator (MSB-first) between bit-fields
    let mut nbits: u32 = 0;
    for (i, seg) in segs.iter().enumerate() {
        match &seg.kind {
            SegKind::Int { width, signed } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at an int segment"
                );
                let w = *width as usize;
                if off + w > raw.len() {
                    return None; // overrun → non-match
                }
                // Assemble big-endian (MSB first); `le` reverses the byte order.
                let mut val: u64 = 0;
                for j in 0..w {
                    let b = if seg.little_endian {
                        raw[off + (w - 1 - j)]
                    } else {
                        raw[off + j]
                    };
                    val = (val << 8) | b as u64;
                }
                // Sign-extend a signed segment from its top bit; zero-extend an unsigned one.
                let bits = (w as u32) * 8;
                let decoded = if *signed && bits < 64 && (val >> (bits - 1)) & 1 == 1 {
                    // A signed narrow segment with its top bit set → sign-extend to a negative value.
                    IntValue::from_i64((val | !((1u64 << bits) - 1)) as i64)
                } else if *signed {
                    // A signed segment with the top bit clear (or width 64: `val` already carries the
                    // two's-complement i64) → the value as a signed `i64`.
                    IntValue::from_i64(val as i64)
                } else {
                    // An UNSIGNED segment → the raw magnitude, UNSIGNED. `from_u128` keeps a top-bit-set
                    // u64 (up to 2^64-1) as its true positive value rather than a wrapped negative `i64`.
                    IntValue::from_u128(val as u128)
                };
                out.push(BinDecoded::Int(decoded));
                off += w;
            }
            SegKind::Bits { k } => {
                let k = *k;
                // Pull `k` bits MSB-first from the byte stream, refilling the accumulator a byte at a time.
                while nbits < k {
                    if off >= raw.len() {
                        return None; // overrun
                    }
                    acc = (acc << 8) | raw[off] as u64;
                    off += 1;
                    nbits += 8;
                }
                let shift = nbits - k;
                let field = (acc >> shift) & ((1u64 << k) - 1);
                acc &= (1u64 << shift) - 1;
                nbits -= k;
                // A bit-field is `k ≤ 64` bits, always NON-NEGATIVE (an unsigned sub-byte value) → its
                // magnitude, unsigned. `from_u128` keeps a top-bit-set 64-bit field positive.
                out.push(BinDecoded::Int(IntValue::from_u128(field as u128)));
            }
            SegKind::Bytes { size: None } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at a bytes segment"
                );
                // A FINAL unsized bytes binds the remainder. (Well-formedness in infer guarantees it is
                // the last segment.) BN3 handles the final-rest form; a non-final would have been CDZ0220.
                if i + 1 != segs.len() {
                    return None;
                }
                out.push(BinDecoded::ByteRange(off, raw.len()));
                off = raw.len();
            }
            // A DEPENDENT-size `(bytes body n)`: `n` names an EARLIER integer segment binder — resolve it
            // to that segment's already-decoded value, then bind exactly `n` bytes at the cursor. `n == 0`
            // is a valid empty bind; `n` overrunning the remainder is a NON-MATCH (fall through). The
            // whole `bin` is byte-aligned here (CDZ0220), so the cursor is on a byte boundary.
            SegKind::Bytes { size: Some(n_occ) } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at a bytes segment"
                );
                // `n_occ` is EITHER a name referencing an earlier segment's binder (dependent size) OR a
                // constant integer literal (fixed size — ruling (a)). `bin_decode_dependent_size` resolves
                // both; an unresolvable/negative size is a non-match, conservatively.
                let n = bin_decode_dependent_size(db, *n_occ, segs, i, &out)?;
                if off + n > raw.len() {
                    return None; // the sized segment overruns the remaining bytes → non-match
                }
                out.push(BinDecoded::ByteRange(off, off + n));
                off += n;
            }
            // A UTF-8 string segment `(utf8 s n)`: read exactly `n` bytes (like a dependent `bytes`) then
            // DECODE them as strict UTF-8. Ill-formed bytes are a NON-MATCH (return `None`), never a trap —
            // exhaustiveness (a required catch-all) forces the caller to handle the bad case. `n` names an
            // earlier integer segment binder, resolved to its already-decoded value. This IS the
            // string-decoding PATTERN the spec requires: ill-formed UTF-8 is a non-match that falls through
            // to a later arm (a branch the exhaustiveness rule forces the program to carry), not a halt.
            //= spec/capabilities/collections-and-text.md#decoding-bytes-to-a-string-is-total-not-trapping
            //# A pattern that decodes a string from a byte sequence MUST treat ill-formed UTF-8 as a non-match that the match's exhaustiveness obligation forces the program to handle, so that the ill-formed case is covered by a branch rather than by a trap.
            SegKind::Utf8 { size } => {
                debug_assert_eq!(
                    nbits, 0,
                    "a well-formed bin is byte-aligned at a utf8 segment"
                );
                // `size` is EITHER a name (dependent size) OR a constant integer literal (fixed size —
                // ruling (a)); `bin_decode_dependent_size` resolves both, non-match on an unresolvable/negative.
                let n = bin_decode_dependent_size(db, *size, segs, i, &out)?;
                if off + n > raw.len() {
                    return None; // the sized segment overruns the remaining bytes → non-match
                }
                // Strict UTF-8 validation (matches `str::from_utf8` — rejects invalid leads, overlong
                // forms, surrogates, and code points > U+10FFFF). Ill-formed → non-match.
                let s = core::str::from_utf8(&raw[off..off + n]).ok()?;
                out.push(BinDecoded::Str(s.to_string()));
                off += n;
            }
        }
    }
    // Whole-scrutinee accounting: after the last segment, any open bits or leftover bytes are a non-match
    // (a `bin` pattern matches the ENTIRE sequence — leftover needs a trailing unsized `(bytes rest)`).
    if nbits != 0 || off != raw.len() {
        return None;
    }
    Some(out)
}

/// Lower a `bin` PATTERN binder reference — decode the bound segment's value from the (constant)
/// scrutinee. On a visible `Core::BytesOf` scrutinee, run the segment automaton and return this segment's
/// decoded value: an `Int` → `Core::ConstInt`; a `Bytes` → a synthesized constant `Core::BytesOf` of the
/// bound byte range (its core/ty pre-filled, like the slice-fold payload). A runtime scrutinee, or a
/// pattern the automaton can't decode here (a dependent-size `(bytes b n)`), declines — BN4. The arm was
/// already SELECTED by `lower_match_bin` (which ran the same decode + the literal probes), so this decode
/// is on a byte sequence the pattern matched; a defensive `None` still declines rather than miscompiles.
pub(super) fn decode_bin_field(
    db: &mut Db,
    scrutinee: StructId,
    segs: &[crate::resolved::Segment],
    seg_index: usize,
) -> Core {
    let Some(raw) = bin_const_scrutinee(db, scrutinee) else {
        // RUNTIME scrutinee — decode the segment directly from the runtime `Bytes` (the arm was already
        // selected by `lower_match_bin`'s runtime predicate, which guarded the length). Only a fixed-width
        // INTEGER segment at a STATIC offset is read this way; a bit-field or a (dependent) bytes segment
        // in a runtime match is a later slice.
        return decode_bin_field_runtime(db, scrutinee, segs, seg_index);
    };
    let Some(decoded) = bin_match_decode(db, &raw, segs) else {
        return Core::Poison(Reject::decline(
            "a bin pattern segment could not be decoded at compile time (dependent size / non-match)",
        ));
    };
    match decoded.get(seg_index) {
        Some(BinDecoded::Int(n)) => Core::ConstInt(n.clone()),
        // A `utf8` segment binds the decoded, already-validated string as a `Core::ConstStr` (typed
        // `Ty::String`) — the same rep a string literal lowers to, so it rides the constant path.
        Some(BinDecoded::Str(s)) => Core::ConstStr(s.clone().into()),
        Some(BinDecoded::ByteRange(s, e)) => {
            // A synthesized constant `Core::BytesOf` of the bound sub-range (same shape the Bytes.slice
            // fold produces): fresh UInt8 element leaves, core/ty pre-filled so it rides the constant path.
            let sub: Vec<StructId> = raw[*s..*e]
                .iter()
                .map(|&b| {
                    db.push_atom(crate::ast::Leaf::Int {
                        value: IntValue::from_i64(b as i64),
                        radix: crate::ast::Radix::Dec,
                    })
                })
                .collect();
            let payload = db.push_atom(crate::ast::Leaf::Bytes(raw[*s..*e].to_vec().into()));
            db.core.fill(payload, Core::BytesOf { elems: sub.into() });
            db.types.fill(payload, crate::ty::Ty::Bytes);
            core_of(db, payload)
        }
        None => Core::Poison(Reject::decline(
            "a bin pattern segment index is out of range",
        )),
    }
}

/// The STATIC byte offset of segment `seg_index`, plus a flag for whether ALL preceding segments are
/// fixed-offset (byte-aligned int/`bits`). `None` if a preceding segment makes the offset dynamic (a
/// dependent-size `(bytes b n)`) or the pattern has a bit-field the runtime path does not handle yet.
/// The runtime matcher (fixed-offset int segments) uses this to place a `BinIntRead`.
pub(super) fn bin_static_offset(
    segs: &[crate::resolved::Segment],
    seg_index: usize,
) -> Option<u32> {
    use crate::resolved::SegKind;
    let mut off: u32 = 0; // byte offset
    let mut bits: u32 = 0; // open sub-byte bits accumulated across a bit-field run (0 at a byte boundary)
    for seg in segs.iter().take(seg_index) {
        match &seg.kind {
            SegKind::Int { width, .. } => {
                // An int segment is byte-aligned — a well-formed bin (CDZ0220) has closed any bit-field run
                // to a whole byte before it, so `bits` is 0 here; be defensive and decline if not.
                if bits != 0 {
                    return None;
                }
                off += *width as u32;
            }
            // A BIT-FIELD contributes `k` sub-byte bits; a run closes to whole bytes (CDZ0220), so fold
            // every completed byte into `off`. A following int/bytes segment therefore reads at a STATIC
            // byte offset once the run is byte-aligned (the case that previously declined outright).
            SegKind::Bits { k } => {
                bits += *k;
                off += bits / 8;
                bits %= 8;
            }
            // A bytes / utf8 segment before the target makes the offset dynamic (variable length) — not
            // built yet. (A bit-field run mid-byte at this point would also be ill-formed; `bits != 0`
            // means the preceding structure did not byte-align, so decline.)
            SegKind::Bytes { .. } | SegKind::Utf8 { .. } => return None,
        }
    }
    // The target segment starts at a byte boundary only if the preceding bit-field run closed a whole
    // number of bytes; a mid-byte position (an odd bit-field run before a byte-aligned segment) is
    // ill-formed for a byte-offset read — decline.
    if bits != 0 {
        return None;
    }
    Some(off)
}

/// The read offset of segment `seg_index`, split into a STATIC base (the sum of fixed-width int widths +
/// bit-field bytes before it) and an OPTIONAL runtime addend `off_plus` — the total bytes any PRECEDING
/// DEPENDENT-SIZE `(bytes body n)` segments consume, as an i64-count `Core` node (a `BinIntRead` of each
/// size, summed). This is the §4a generalization of `bin_static_offset`: once a dependent-size segment
/// appears, every following segment reads at `static_base + off_plus` (§6.4 "constant + a bound local").
///
/// Returns `None` when the offset is not computable: a NON-FINAL UNSIZED `(bytes b)` before `seg_index` is
/// ill-formed (nothing can follow an open-ended rest — CDZ0220); a mid-byte bit-field position; a utf8
/// segment (not yet built); or a preceding dependent size whose own size field is not a fixed int at a
/// static offset (`bin_size_len_read` declines). `bytes_src` is the materialized scrutinee read (a
/// `LocalRef`) the size reads borrow. `off_plus` is `None` for a purely static offset (the common case,
/// identical to `bin_static_offset`).
pub(super) fn bin_dynamic_offset(
    db: &mut Db,
    bytes_src: StructId,
    segs: &[crate::resolved::Segment],
    seg_index: usize,
) -> Option<(u32, Option<StructId>)> {
    use crate::resolved::SegKind;
    let mut off: u32 = 0; // static byte base
    let mut bits: u32 = 0; // open sub-byte bits across a bit-field run (0 at a byte boundary)
    let mut off_plus: Option<StructId> = None; // runtime addend = Σ preceding dependent-size lengths
    for (j, seg) in segs.iter().take(seg_index).enumerate() {
        match &seg.kind {
            SegKind::Int { width, .. } => {
                if bits != 0 {
                    return None;
                }
                off += *width as u32;
            }
            SegKind::Bits { k } => {
                bits += *k;
                off += bits / 8;
                bits %= 8;
            }
            // A DEPENDENT-SIZE `(bytes body n)` consumes `n` runtime bytes — fold `n` into the addend. The
            // size read borrows the scrutinee (a scalar `BinIntRead`, no heap operand). A run must be
            // byte-aligned before it (a mid-byte bit-field position is ill-formed).
            SegKind::Bytes { size: Some(n_occ) } => {
                if bits != 0 {
                    return None;
                }
                let n_read = bin_size_len_read(db, bytes_src, segs, j, *n_occ)?;
                off_plus = Some(match off_plus {
                    None => n_read,
                    Some(prev) => synth_core(
                        db,
                        Core::Arith {
                            op: Prim::Add,
                            lhs: prev,
                            rhs: n_read,
                        },
                        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
                    ),
                });
            }
            // A `(utf8 s SIZE)` segment consumes SIZE bytes, exactly like a `(bytes … SIZE)`: a CONSTANT
            // literal size is a STATIC byte contribution (`off += C`); a DEPENDENT name size folds its
            // runtime `n` into the addend (a `BinIntRead` of the earlier segment). This is what lets a
            // segment FOLLOW a utf8 segment (the non-final utf8 case) — its offset is `static_base + Σn`.
            SegKind::Utf8 { size } => {
                if bits != 0 {
                    return None;
                }
                if let Some(c) = db.ast.as_int(*size).and_then(|v| v.to_i64()) {
                    if c < 0 {
                        return None;
                    }
                    off += c as u32;
                } else {
                    let n_read = bin_size_len_read(db, bytes_src, segs, j, *size)?;
                    off_plus = Some(match off_plus {
                        None => n_read,
                        Some(prev) => synth_core(
                            db,
                            Core::Arith {
                                op: Prim::Add,
                                lhs: prev,
                                rhs: n_read,
                            },
                            crate::ty::Ty::Int(crate::ty::IntTy::i64()),
                        ),
                    });
                }
            }
            // A NON-FINAL UNSIZED rest is ill-formed (CDZ0220).
            SegKind::Bytes { size: None } => return None,
        }
    }
    if bits != 0 {
        return None;
    }
    Some((off, off_plus))
}

/// A byte-aligned BIT-FIELD RUN containing segment `seg_index` (which must be a `(bits k)`): its start
/// BYTE offset, the run's total width in BITS (a whole number of bytes — CDZ0220 keeps a `bits` run
/// byte-aligned), and this field's bit position (from the run's MSB) + its own width `k`. A run is a
/// MAXIMAL consecutive sequence of `(bits …)` segments; the whole `bin` is byte-aligned, so a run's bits
/// sum to a multiple of 8. `None` if any segment BEFORE the run makes the byte offset dynamic (a preceding
/// bit-field the offset walk can't cross yet, or a bytes/utf8 segment). Used by the runtime decode to read
/// the run's bytes as one integer then shift+mask a field out (MSB-first, mirroring `bin_match_decode`).
///
/// Returns `(run_byte_off, run_bits, field_bit_pos, k)`: read `run_bits/8` bytes at `run_byte_off` as one
/// big-endian unsigned integer `R`, then `field = (R >> (run_bits - field_bit_pos - k)) & ((1<<k)-1)`.
pub(super) fn bin_bitfield_run(
    segs: &[crate::resolved::Segment],
    seg_index: usize,
) -> Option<(u32, u32, u32, u32)> {
    use crate::resolved::SegKind;
    let SegKind::Bits { k } = &segs.get(seg_index)?.kind else {
        return None;
    };
    let k = *k;
    // Byte offset up to the START of this bit-field's run: sum the widths of the segments BEFORE the run,
    // which must all be byte-aligned INT segments (a preceding bytes/utf8/bit-field run declines — a later
    // slice handles a run that is not the first non-int structure). Walk back to the run start: the run
    // begins at the first `(bits …)` in the maximal consecutive bit-field block ending at/containing
    // `seg_index`.
    let run_start = {
        let mut i = seg_index;
        while i > 0 && matches!(segs[i - 1].kind, SegKind::Bits { .. }) {
            i -= 1;
        }
        i
    };
    // Everything before the run must be a fixed-width int (byte-aligned), else the run's byte offset is
    // dynamic / unsupported.
    let mut byte_off: u32 = 0;
    for seg in segs.iter().take(run_start) {
        match &seg.kind {
            SegKind::Int { width, .. } => byte_off += *width as u32,
            _ => return None,
        }
    }
    // The run is the maximal consecutive `(bits …)` block from `run_start`. Sum its bits (must be a whole
    // number of bytes) and find `seg_index`'s bit position from the run's MSB.
    let mut run_bits: u32 = 0;
    let mut field_bit_pos: u32 = 0;
    let mut i = run_start;
    while let Some(seg) = segs.get(i) {
        let SegKind::Bits { k: kk } = &seg.kind else {
            break;
        };
        if i == seg_index {
            field_bit_pos = run_bits;
        }
        run_bits += *kk;
        i += 1;
    }
    // The run must be byte-aligned as a whole (CDZ0220 guarantees it for a well-formed bin) and fit a u64
    // read (≤ 64 bits). A field wider than the run, or a non-byte-aligned run, is malformed / unsupported.
    if run_bits == 0 || !run_bits.is_multiple_of(8) || run_bits > 64 || field_bit_pos + k > run_bits
    {
        return None;
    }
    Some((byte_off, run_bits, field_bit_pos, k))
}

/// Synthesize the runtime read of ONE bit-field out of a byte-aligned run: read the run's `run_bits/8`
/// bytes at `run_byte_off` as one big-endian unsigned integer, then shift the field down and mask to `k`
/// bits — `(R >> (run_bits - field_bit_pos - k)) & ((1<<k)-1)`, MSB-first (mirroring `bin_match_decode`).
/// `bytes_src` is the already-materialized scrutinee read (a `LocalRef` to the kept binding, or the
/// predicate path's `scrut_ref`) — used directly, NOT re-wrapped. Shared by the bit-field BINDER decode
/// (`decode_bin_field_runtime`'s `Bits` arm) and the dependent-SIZE read (`bin_size_len_read`, when the
/// size field is a bit-field). Reads the run unsigned so the mask makes signedness moot.
pub(super) fn bin_bitfield_read(
    db: &mut Db,
    bytes_src: StructId,
    run_byte_off: u32,
    run_bits: u32,
    field_bit_pos: u32,
    k: u32,
) -> StructId {
    // R = the run's bytes as one unsigned big-endian integer (i64 rep; run_bits ≤ 64).
    let run_read = synth_core(
        db,
        Core::BinIntRead {
            bytes: bytes_src,
            byte_offset: run_byte_off,
            // A bit-field run declines on any preceding dependent-size segment (`bin_bitfield_run` requires
            // fixed-int-only before the run), so its offset is always static.
            off_plus: None,
            width: (run_bits / 8) as u8,
            signed: false,
            little_endian: false,
        },
        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
    );
    // Shift the field down to the low bits: shift = run_bits - field_bit_pos - k.
    let shift = run_bits - field_bit_pos - k;
    let field_low = if shift == 0 {
        run_read
    } else {
        let shift_node = synth_core(
            db,
            Core::ConstInt(IntValue::from_i64(shift as i64)),
            crate::ty::Ty::Int(crate::ty::IntTy::i64()),
        );
        synth_core(
            db,
            Core::Arith {
                op: Prim::Shr,
                lhs: run_read,
                rhs: shift_node,
            },
            crate::ty::Ty::Int(crate::ty::IntTy::i64()),
        )
    };
    // Mask to k bits: & ((1<<k)-1). (k ≤ run_bits ≤ 64; the mask fits an i64 for k ≤ 63.)
    let mask_val: i64 = if k >= 63 { -1 } else { (1i64 << k) - 1 };
    let mask_node = synth_core(
        db,
        Core::ConstInt(IntValue::from_i64(mask_val)),
        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
    );
    synth_core(
        db,
        Core::Arith {
            op: Prim::BitAnd,
            lhs: field_low,
            rhs: mask_node,
        },
        crate::ty::Ty::Int(crate::ty::IntTy::i64()),
    )
}

/// Decode a `bin`-pattern INTEGER segment binder out of a RUNTIME `Bytes` scrutinee (a `BinIntRead` at
/// the segment's static offset). Only a fixed-width int segment at a fixed offset is supported; anything
/// else (a bit-field or bytes binder, or an offset made dynamic by a preceding dependent size) declines —
/// a later runtime-matching slice.
pub(super) fn decode_bin_field_runtime(
    db: &mut Db,
    scrutinee: StructId,
    segs: &[crate::resolved::Segment],
    seg_index: usize,
) -> Core {
    use crate::resolved::SegKind;
    let Some(seg) = segs.get(seg_index) else {
        return Core::Poison(Reject::decline(
            "a bin pattern segment index is out of range",
        ));
    };
    // The scrutinee read for a size field's `bin_size_len_read` / the offset walk (a `LocalRef` to the kept
    // binding). `bin_dynamic_offset` reads any preceding dependent size off this handle (a borrow).
    let off_src = synth_core(
        db,
        Core::LocalRef { binder: scrutinee },
        crate::ty::Ty::Bytes,
    );
    match &seg.kind {
        SegKind::Int { width, signed } => match bin_dynamic_offset(db, off_src, segs, seg_index) {
            Some((byte_offset, off_plus)) => {
                // `lower_match_bin` materialized the scrutinee as a KEPT binding, so read it through a
                // `LocalRef` (its own occurrence is the binding key) — NOT the raw scrutinee occurrence,
                // which would re-emit the `BinBuild` construction per binder read.
                let scrut_ref = synth_core(
                    db,
                    Core::LocalRef { binder: scrutinee },
                    crate::ty::Ty::Bytes,
                );
                Core::BinIntRead {
                    bytes: scrut_ref,
                    byte_offset,
                    off_plus,
                    width: *width,
                    signed: *signed,
                    little_endian: seg.little_endian,
                }
            }
            None => Core::Poison(Reject::decline(
                "a runtime bin int segment after a non-final unsized bytes / utf8 segment is not yet decoded",
            )),
        },
        // A FINAL unsized `(bytes rest)` binder — the tail after the fixed prefix. Read it as
        // `bytes-slice(scrutinee, off, len-off)` via `Core::BinRestRead`; `off = static_base + off_plus`
        // (a preceding dependent-size segment contributes the runtime `off_plus` — §4a).
        SegKind::Bytes { size: None } if seg_index + 1 == segs.len() => {
            match bin_dynamic_offset(db, off_src, segs, seg_index) {
                Some((byte_offset, off_plus)) => {
                    let scrut_ref = synth_core(
                        db,
                        Core::LocalRef { binder: scrutinee },
                        crate::ty::Ty::Bytes,
                    );
                    Core::BinRestRead {
                        bytes: scrut_ref,
                        byte_offset,
                        off_plus,
                    }
                }
                None => Core::Poison(Reject::decline(
                    "a runtime bin rest binder after a non-final unsized bytes / utf8 segment is not yet decoded",
                )),
            }
        }
        // A DEPENDENT-SIZE `(bytes payload n)` binder — exactly `n` bytes at `static_base + off_plus`, where
        // `n` is the RUNTIME value of an EARLIER integer segment (named by `n_occ`) and `off_plus` is the
        // total bytes any PRECEDING dependent-size segments consumed (§4a: a non-final dependent size is now
        // decoded — its offset is `static_base + off_plus`, no longer requiring `size: Some` to be final).
        // Read as `bytes-slice(scrutinee, off, n)` via `Core::BinSizedRead`, whose `len` child is a
        // `BinIntRead` of the named earlier segment. The caller's predicate guaranteed `bytes-len >= off + n`.
        SegKind::Bytes { size: Some(n_occ) } => {
            // The size `n` reads off its own materialized `LocalRef` to the kept scrutinee binding.
            let len_src = synth_core(
                db,
                Core::LocalRef { binder: scrutinee },
                crate::ty::Ty::Bytes,
            );
            match (
                bin_dynamic_offset(db, off_src, segs, seg_index),
                bin_size_len_read(db, len_src, segs, seg_index, *n_occ),
            ) {
                (Some((byte_offset, off_plus)), Some(len)) => {
                    let scrut_ref = synth_core(
                        db,
                        Core::LocalRef { binder: scrutinee },
                        crate::ty::Ty::Bytes,
                    );
                    Core::BinSizedRead {
                        bytes: scrut_ref,
                        byte_offset,
                        off_plus,
                        len,
                    }
                }
                _ => Core::Poison(Reject::decline(
                    "a runtime dependent-size bin binder needs a computable offset and a fixed-int size segment",
                )),
            }
        }
        // A BIT-FIELD `(bits k)` binder — read its byte-aligned RUN as one big-endian unsigned integer,
        // then shift+mask the field out MSB-first (mirroring `bin_match_decode`'s `(acc >> (nbits-k)) &
        // mask`). `bin_bitfield_run` gives the run's byte offset, total bits, and this field's bit position
        // from the run's MSB. `field = (R >> (run_bits - field_bit_pos - k)) & ((1<<k)-1)`, where `R` is the
        // run's `run_bits/8` bytes read unsigned. A run that is not byte-aligned, wider than 64 bits, or
        // preceded by a non-int structure declines (a later slice). The mask makes the read's signedness
        // moot, so `R` is read unsigned.
        SegKind::Bits { .. } => match bin_bitfield_run(segs, seg_index) {
            Some((run_byte_off, run_bits, field_bit_pos, k)) => {
                let scrut_ref = synth_core(
                    db,
                    Core::LocalRef { binder: scrutinee },
                    crate::ty::Ty::Bytes,
                );
                // Read the field out of its byte-aligned run: run-read + shift + mask (shared helper).
                let read =
                    bin_bitfield_read(db, scrut_ref, run_byte_off, run_bits, field_bit_pos, k);
                core_of(db, read)
            }
            None => Core::Poison(Reject::decline(
                "a runtime bin bit-field binder needs a byte-aligned run of ≤64 bits after fixed-int \
                 segments (a mid-stream or wide bit-field is a later slice)",
            )),
        },
        // A `(utf8 s SIZE)` binder (constant OR dependent size, at ANY position) — decode the byte range to
        // `Option String` and unwrap the `Some`. The arm's predicate already ANDed `is-some` (UTF-8
        // well-formedness), so this `SumExpect` is on a value proven present and never traps; it unboxes as
        // the decoded `String` (the node's solved type). The range read borrows a fresh `LocalRef` to the
        // RAW scrutinee (the int / rest / dependent-bytes binders above do the same), NOT `off_src` re-wrapped.
        SegKind::Utf8 { .. } => {
            let scrut_ref = synth_core(
                db,
                Core::LocalRef { binder: scrutinee },
                crate::ty::Ty::Bytes,
            );
            match bin_utf8_decode(db, scrut_ref, segs, seg_index) {
                Some((opt_node, disc_some, _)) => Core::SumExpect {
                    scrutinee: opt_node,
                    disc_present: disc_some,
                },
                None => Core::Poison(Reject::decline(
                    "a runtime bin utf8 binder needs a computable byte range (offset + size)",
                )),
            }
        }
        SegKind::Bytes { .. } => Core::Poison(Reject::decline(
            "a runtime bin non-final sized-bytes binder is not yet decoded",
        )),
    }
}

/// Build the `Core::BinIntRead` occurrence that reads the DEPENDENT-SIZE operand `n` of a `(bytes payload
/// n)` segment out of the runtime scrutinee: `n_occ` names an EARLIER integer segment binder, so find that
/// segment by name, confirm it is a fixed-width int at a static byte offset, and emit a `BinIntRead` of it
/// (typed Int64 — a `BinIntRead` always yields an i64). Returns `None` if `n_occ` is not a name / does not
/// resolve to an earlier fixed-int segment / that segment has no static offset (a dynamic offset from a
/// preceding dependent size). Mirrors the const path's by-name resolution (`bin_match_decode`).
pub(super) fn bin_size_len_read(
    db: &mut Db,
    bytes_src: StructId,
    segs: &[crate::resolved::Segment],
    seg_index: usize,
    n_occ: StructId,
) -> Option<StructId> {
    use crate::resolved::SegKind;
    let size_name = db.ast.as_name(n_occ)?;
    // Find the EARLIER segment whose binder is `size_name` — a fixed-width INT or a byte-aligned BIT-FIELD.
    let idx = segs
        .iter()
        .take(seg_index)
        .position(|s| db.ast.as_name(s.slot) == Some(size_name))?;
    // `bytes_src` is the already-materialized read of the scrutinee (a `LocalRef` to the kept binding, or
    // the predicate path's `scrut_ref`) — read the size field off it directly (do NOT re-wrap: wrapping a
    // ref in another `LocalRef` yields "no local slot").
    match &segs[idx].kind {
        SegKind::Int { width, signed, .. } => {
            // The size field's own offset may be dynamic (a chained `(u8 a)(bytes x a)(u8 b)(bytes y b)` —
            // `b` sits after dependent `x`). `bin_dynamic_offset` recurses only over EARLIER segments, so it
            // terminates. `off_plus` reads borrow `bytes_src` (the same materialized scrutinee handle).
            let (byte_offset, off_plus) = bin_dynamic_offset(db, bytes_src, segs, idx)?;
            Some(synth_core(
                db,
                Core::BinIntRead {
                    bytes: bytes_src,
                    byte_offset,
                    off_plus,
                    width: *width,
                    signed: *signed,
                    little_endian: segs[idx].little_endian,
                },
                crate::ty::Ty::int(),
            ))
        }
        // A BIT-FIELD size field `(bits n k)` feeding a dependent-size `(bytes payload n)` — read `n` out of
        // its byte-aligned run (run-read + shift + mask, the SAME extraction the bit-field binder decode
        // uses). The run must be byte-aligned + ≤64 bits (`bin_bitfield_run` — else the offset of the
        // dependent segment is not static; declines). Unsigned + masked, so a size is a small non-negative
        // int (the caller's `n >= 0` predicate guard still applies).
        SegKind::Bits { .. } => {
            let (run_byte_off, run_bits, field_bit_pos, k) = bin_bitfield_run(segs, idx)?;
            Some(bin_bitfield_read(
                db,
                bytes_src,
                run_byte_off,
                run_bits,
                field_bit_pos,
                k,
            ))
        }
        _ => None,
    }
}

/// Decode a `(utf8 s SIZE)` segment's byte range out of a RUNTIME `Bytes` scrutinee to an `Option String`
/// via the total UTF-8 decode `Core::StrFromBytes`, at ANY position and for either size form. The range is
/// `[off, off+len)` where `off` is a static base + a runtime `off_plus` addend from any preceding
/// dependent-size segments (`bin_dynamic_offset`), and `len` is `ConstInt(C)` for a CONSTANT literal size
/// `C` or a `BinIntRead` of the earlier named segment (`bin_size_len_read`) for a DEPENDENT name size.
/// Read as a `Core::BinSizedRead` (exactly `len` bytes at `off` — the same sized slice the dependent
/// `(bytes … n)` binder uses), which works for a FINAL as well as a NON-FINAL utf8 segment. The arm's
/// length predicate pins `bytes-len == total + Σn` (the utf8 width enters that accounting), so the range is
/// in bounds. `bytes_src` is the ALREADY-MATERIALIZED scrutinee read the range + size borrow (a `LocalRef`
/// to the kept binding, or the predicate path's `scrut_ref`) — passed in directly, NOT re-wrapped in
/// another `LocalRef` (which yields "no local slot"). Returns `(opt_node, disc_some, disc_none)`: the
/// `StrFromBytes` occurrence (typed `Option String`) plus the built-in Option variants' discriminants.
/// `None` if the offset / size is not computable (a shape whose plumbing declines) or the prelude `Option`
/// is absent (a prelude-less compile).
pub(super) fn bin_utf8_decode(
    db: &mut Db,
    bytes_src: StructId,
    segs: &[crate::resolved::Segment],
    seg_index: usize,
) -> Option<(StructId, u32, u32)> {
    use crate::resolved::SegKind;
    let SegKind::Utf8 { size } = &segs.get(seg_index)?.kind else {
        return None;
    };
    let size = *size;
    let (byte_offset, off_plus) = bin_dynamic_offset(db, bytes_src, segs, seg_index)?;
    // The segment's byte length: a constant literal `C`, or a `BinIntRead` of the earlier named segment.
    let len = if let Some(c) = db.ast.as_int(size).and_then(|v| v.to_i64()) {
        if c < 0 {
            return None;
        }
        synth_core(
            db,
            Core::ConstInt(IntValue::from_i64(c)),
            crate::ty::Ty::Int(crate::ty::IntTy::i64()),
        )
    } else {
        bin_size_len_read(db, bytes_src, segs, seg_index, size)?
    };
    // The built-in `Option String` type + its Some/None discriminants (read off the declaration by NAME,
    // never assumed positionally — the same discipline `option_discs` uses).
    let opt_ty = {
        let occ = db.type_decls.iter().find(|t| t.name == "Option")?.occ;
        db.normalize_sum(occ, vec![crate::ty::Ty::String])
    };
    let disc_some = variant_disc_by_name(db, &opt_ty, "Some")?;
    let disc_none = variant_disc_by_name(db, &opt_ty, "None")?;
    // Read exactly `len` bytes at `off (+ off_plus)` as a Bytes value off the materialized scrutinee read.
    let range = synth_core(
        db,
        Core::BinSizedRead {
            bytes: bytes_src,
            byte_offset,
            off_plus,
            len,
        },
        crate::ty::Ty::Bytes,
    );
    let opt_node = synth_core(
        db,
        Core::StrFromBytes {
            bytes: range,
            disc_some,
            disc_none,
        },
        opt_ty,
    );
    Some((opt_node, disc_some, disc_none))
}

/// A boolean `Core` occurrence that holds iff `opt_node` (an `Option`-typed value) is its `Some` variant —
/// a `MatchSum` yielding `true` on the `Some` disc and `false` on the default. Used to AND a `(utf8 …)`
/// segment's UTF-8 WELL-FORMEDNESS into the runtime arm predicate: an ill-formed byte range decodes to
/// `None`, so `is-some` is exactly the non-match / fall-through test the const path expresses as `.ok()?`.
pub(super) fn bin_option_is_some(db: &mut Db, opt_node: StructId, disc_some: u32) -> StructId {
    let t = synth_core(db, Core::ConstBool(true), crate::ty::Ty::Bool);
    let f = synth_core(db, Core::ConstBool(false), crate::ty::Ty::Bool);
    let root = std::rc::Rc::new(crate::core::SumCont::Switch {
        path: std::rc::Rc::from(Vec::<crate::core::PathStep>::new()),
        arms: vec![
            crate::core::SumArm {
                disc: Some(disc_some),
                cont: crate::core::SumCont::Leaf(t),
            },
            crate::core::SumArm {
                disc: None,
                cont: crate::core::SumCont::Leaf(f),
            },
        ],
    });
    synth_core(
        db,
        Core::MatchSum {
            scrutinee: opt_node,
            root,
        },
        crate::ty::Ty::Bool,
    )
}

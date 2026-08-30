//! Scalar operations: Int, Bool, Float, Float32, BigInt, Rational
//!
//! All scalar box/unbox operations + bigint/rational arithmetic.

use super::*;

// ─── Scalar leaves: box a primitive, read it back (TOTAL — reinterprets bytes, never traps) ──────

pub(crate) fn op_box_int(v: i64) -> Handle {
    // Normalize-on-construct (P2), THE single source of truth for the fixnum boundary: a value that
    // fits the inline window is ALWAYS an immediate, never boxed, so inline-3 and boxed-3 cannot
    // coexist (canonical form). Only out-of-window ints keep a heap Node.
    if fixnum_fits(v) {
        return imm_int(v);
    }
    alloc_raw(Vec::new(), Raw::inline(&(v as u64).to_le_bytes())) // 8-byte scalar: inline, no heap raw
}
pub(crate) fn op_get_int(h: Handle) -> i64 {
    if is_immediate(h) {
        return imm_as_int(h); // rep-agnostic decode; equals a boxed twin's `read_word`
    }
    with_node(h, 0, |n| read_word(&n.raw) as i64)
}
pub(crate) fn op_box_bool(v: bool) -> Handle {
    // Normalize-on-construct (P1b): a bool ALWAYS inlines, never boxes, so inline is the one
    // canonical representation. `imm_bool` carries the value in the tag bits — no heap Node.
    imm_bool(v)
}
pub(crate) fn op_get_bool(h: Handle) -> bool {
    if is_immediate(h) {
        return imm_as_bool(h); // rep-agnostic decode of an inline bool
    }
    with_node(h, false, |n| n.raw.first().is_some_and(|&b| b != 0))
}
pub(crate) fn is_whole_f64(f: f64) -> bool {
    let bits = f.to_bits();
    let biased_exp = ((bits >> 52) & 0x7ff) as i64;
    let mantissa = bits & 0x000f_ffff_ffff_ffff;
    if biased_exp == 0 {
        return mantissa == 0;
    }
    let e = biased_exp - 1023;
    if e >= 52 {
        return true;
    }
    if e < 0 {
        return false;
    }
    let frac_bits = 52 - e as u32;
    (mantissa & ((1u64 << frac_bits) - 1)) == 0
}
pub(crate) fn op_box_float(v: f64) -> Handle {
    // Normalize-on-construct to the CANONICAL byte form (deterministic-value-form.md §A Value Has One
    // Canonical Byte Form): every NaN — of ANY bit pattern (a distinct literal NaN, or a runtime
    // arithmetic NaN like `0.0/0.0` whose payload/sign wasm need not fix) — collapses to the ONE
    // canonical quiet NaN `f64::NAN.to_bits()`, exactly the pattern the compiler's `ConstFloatNan`
    // emits. Without this, two NaN values with differing bits would be DISTINCT map/set keys and
    // structurally-unequal under `champ_hash`/`champ_eq` (which compare raw bytes), whereas the spec's
    // canonical form makes every NaN equal to every NaN. A NON-NaN value (incl. ±0.0 and ±inf) keeps
    // its bits verbatim, so `-0.0` stays DISTINCT from `0.0` (their canonical forms genuinely differ).
    // This is the float twin of `op_box_int`'s normalize-on-construct: `box-float` is the SOLE producer
    // of a float leaf, so canonicalizing here guarantees every stored float has one byte form — one
    // canonical encoding per value, equal values (every NaN) sharing identical bytes and unequal values
    // (±0.0) keeping distinct bytes, which `champ_hash`/`champ_eq` compare rawly:
    //= spec/contracts/deterministic-value-form.md#a-value-has-one-canonical-byte-form
    //# Each serializable value MUST have exactly one canonical byte encoding.
    //= spec/contracts/deterministic-value-form.md#a-value-has-one-canonical-byte-form
    //# Two values that are equal under the language's structural equality MUST have identical canonical byte encodings.
    //= spec/contracts/deterministic-value-form.md#a-value-has-one-canonical-byte-form
    //# Two values that are not equal under the language's structural equality MUST have distinct canonical byte encodings.
    let bits = if v.is_nan() {
        f64::NAN.to_bits()
    } else {
        v.to_bits()
    };
    alloc_raw(Vec::new(), Raw::inline(&bits.to_le_bytes())) // 8-byte scalar: inline, no heap raw
}
pub(crate) fn op_get_float(h: Handle) -> f64 {
    if is_immediate(h) {
        return 0.0; // cross-kind totality: a float is never itself an immediate
    }
    with_node(h, 0.0, |n| f64::from_bits(read_word(&n.raw)))
}
/// Box a `Float32` in its NATURAL 4-byte form (distinct from `box-float`'s 8-byte Float64), so a
/// Float32's canonical byte form — and value-encode's shortest-decimal render — is the f32's, not a
/// promoted f64's. NaN-canonicalized on construction (the f32 twin of `op_box_float`): any NaN → the
/// one canonical quiet `f32::NAN.to_bits()`, so two NaN Float32s are the same map/set key. Non-NaN
/// (incl. ±0.0/±inf) keeps its bits, so `-0.0f32` stays distinct from `0.0f32`.
pub(crate) fn op_box_float32(v: f32) -> Handle {
    let bits = if v.is_nan() {
        f32::NAN.to_bits()
    } else {
        v.to_bits()
    };
    alloc_raw(Vec::new(), Raw::inline(&bits.to_le_bytes())) // 4-byte scalar: inline, no heap raw
}
pub(crate) fn op_get_float32(h: Handle) -> f32 {
    if is_immediate(h) {
        return 0.0; // cross-kind totality: a float is never itself an immediate
    }
    with_node(h, 0.0f32, |n| {
        // Read the low 4 bytes of the raw (zero-padded past the end — defensive, total).
        let mut buf = [0u8; 4];
        let k = n.raw.len().min(4);
        buf[..k].copy_from_slice(&n.raw[..k]);
        f32::from_bits(u32::from_le_bytes(buf))
    })
}

// ─── Arbitrary-precision integer (BigInt) — a sign-magnitude limb-array LEAF ─────────────────────
// A `BigInt` value is a raw-only heap leaf (zero handles), the `Bytes`-leaf shape: its `raw` holds the
// canonical sign-magnitude bytes of a `bigint::Big` (`to/from_sign_magnitude_bytes`). ALWAYS a heap leaf
// — never a fixnum immediate — because `BigInt` is a DISTINCT type from a fixed-width int: an immediate
// tag means "small int", and conflating the two would let a `BigInt` handle be misread as an `Int`. The
// arithmetic ops unbox both operands to `Big`, compute (the hand-written limb library), and re-box the
// normalized result. `op_dup`/`op_drop` need no change (a raw-only leaf is the cheapest node shape).

/// Box a `Big` as a BigInt heap leaf — its canonical sign-magnitude bytes in `raw`, zero handles.
pub(crate) fn box_bigint(b: &bigint::Big) -> Handle {
    // Fast path — a small BigInt (single/few limbs → ≤`INLINE_RAW_CAP` sign-magnitude bytes, the common
    // case) serializes DIRECTLY into an inline `Raw` with NO transient heap Vec (the `to_sign_magnitude_
    // bytes` + `Raw::from` path would allocate that Vec then free it once inlined — the transient-small-Vec
    // smell). A larger value falls back to the heap serialization. Byte-identical either way.
    let mut buf = [0u8; INLINE_RAW_CAP];
    if let Some(n) = b.to_sign_magnitude_bytes_into(&mut buf) {
        return alloc_raw(Vec::new(), Raw::inline(&buf[..n]));
    }
    alloc_raw(Vec::new(), Raw::from(b.to_sign_magnitude_bytes()))
}
/// Read a BigInt leaf back to a `Big`. Total: a null/mismatched node reads as zero (deterministic bits,
/// never a trap — the scalar-read discipline). A BigInt is never an immediate, so no immediate decode.
pub(crate) fn unbox_bigint(h: Handle) -> bigint::Big {
    with_node(h, bigint::Big::zero(), |n| {
        bigint::Big::from_sign_magnitude_bytes(&n.raw)
    })
}
/// `bigint-of-i64` — widen a fixed-width `i64` into a `BigInt` leaf (the `BigInt.of` target for a runtime
/// integer; a constant folds in the compiler and never calls this). Boxes the value DIRECTLY through the
/// i128 path (`box_bigint_i128`, which serializes to inline sign-magnitude bytes with NO `Big`) — an i64
/// trivially fits i128. This skips the transient `Big::from_i64` limb `Vec` the `box_bigint(&Big)` route
/// allocated-then-freed per call (the same transient-small-Vec smell `box_bigint`'s own inline fast path
/// avoids, reintroduced by the `Big` intermediate). Byte-identical leaf (both emit the canonical
/// `[sign][LE magnitude, trailing-zeros-stripped]` form — verified across the full i64 range incl. i64::MIN
/// + limb boundaries).
pub(crate) fn op_bigint_of_i64(v: i64) -> Handle {
    box_bigint_i128(v as i128)
}
/// `bigint-of-bytes` — build a BigInt leaf from a Bytes leaf holding the canonical sign-magnitude bytes
/// (`[sign][LE magnitude, trailing-zeros-stripped]`). The compiler emits this to materialize a CONSTANT
/// BigInt whose magnitude exceeds i64 range (too large for `bigint-of-i64`): it bakes the sign-magnitude
/// bytes as a Bytes leaf (`bytes-alloc`/`bytes-set`, like a constant string) then re-tags them here. The
/// input may be a rope (a concat/slice) in general, so FLATTEN it before reading `raw` — exactly as the
/// value-encode `Shape::Bytes` walker does. `from_sign_magnitude_bytes` re-normalizes (a malformed/empty
/// input decodes as zero — total), so `box_bigint` re-emits the canonical leaf form. CONSUMES `buf` (the
/// transient byte leaf is dropped after its content is read).
pub(crate) fn op_bigint_of_bytes(buf: Handle) -> Handle {
    bytes_flatten(buf);
    let big = with_node(buf, bigint::Big::zero(), |n| {
        bigint::Big::from_sign_magnitude_bytes(&n.raw)
    });
    let out = box_bigint(&big);
    op_drop(buf);
    out
}
/// `bigint-to-i64-checked` — the CHECKED narrowing back to `i64`: the value if it fits, else TRAP
/// (`options/numeric-model/explicit-checked.md` — `Int64.of` of an out-of-range BigInt traps). Reads the
/// leaf's sign-magnitude `raw` slice DIRECTLY (`Big::i64_checked_from_sign_magnitude_bytes`) — a narrowing
/// is READ-ONLY, so it needs NO `Big` (no limb `Vec`): allocation-free. A null node reads as zero.
pub(crate) fn op_bigint_to_i64_checked(h: Handle) -> i64 {
    let raw = unsafe { h.node_ref() }.map_or(&[][..], |n| n.raw.as_slice());
    match bigint::Big::i64_checked_from_sign_magnitude_bytes(raw) {
        Some(v) => v,
        None => trap_bigint_narrow(),
    }
}
#[cold]
#[inline(never)]
pub(crate) fn trap_bigint_narrow() -> ! {
    panic!("cdz-runtime: BigInt value out of range for the target integer type")
}
/// Read a BigInt leaf's raw sign-magnitude bytes as an `i128`, or `None` if the value exceeds i128 range
/// (needs the full `Big` path). Borrows the node's `raw` slice DIRECTLY — no `Big`, no limb `Vec`. A
/// null/missing node reads as the empty slice = canonical zero. The small-operand arithmetic fast path.
#[inline]
pub(crate) fn bigint_as_i128(h: Handle) -> Option<i128> {
    let raw = unsafe { h.node_ref() }.map_or(&[][..], |n| n.raw.as_slice());
    bigint::Big::i128_from_sign_magnitude_bytes(raw)
}
/// Box an `i128` result as a BigInt leaf directly from its sign-magnitude bytes — no intermediate `Big`.
/// An `i128`'s bytes are ≤17 (`[sign] + ≤16 magnitude`), which exceeds `INLINE_RAW_CAP` (12) only for a
/// value needing >11 magnitude bytes; such a value falls back to the heap `Raw`. Byte-identical to
/// `box_bigint(&Big::from_i128(v))`.
#[inline]
pub(crate) fn box_bigint_i128(v: i128) -> Handle {
    let mut buf = [0u8; 17]; // sign + 16 LE magnitude bytes (i128 max)
    let n = bigint::Big::i128_to_sign_magnitude_bytes_into(v, &mut buf)
        .expect("17-byte buf fits any i128");
    if n <= INLINE_RAW_CAP {
        alloc_raw(Vec::new(), Raw::inline(&buf[..n]))
    } else {
        alloc_raw(Vec::new(), Raw::from(buf[..n].to_vec()))
    }
}
/// `bigint-add`/`-sub`/`-mul` — the total (never-trapping) arithmetic. FAST PATH: when both operands fit
/// `i128` (the common case — a runtime BigInt is a BigInt by TYPE, its magnitude usually small) and the
/// native `checked_*` op does not overflow, compute + box the `i128` result with NO limb `Vec` on either
/// operand (was 2 unbox Vecs + a result Vec; now just the result node). SLOW PATH: an operand out of i128
/// range, or an overflowing result, falls back to the full `Big` path — byte-identical either way (both
/// produce the canonical sign-magnitude leaf; guarded by the `num-bigint` differential + the i128-boundary
/// differential test).
pub(crate) fn op_bigint_add(a: Handle, b: Handle) -> Handle {
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(r) = x.checked_add(y) {
            return box_bigint_i128(r);
        }
    }
    box_bigint(&unbox_bigint(a).add(&unbox_bigint(b)))
}
pub(crate) fn op_bigint_sub(a: Handle, b: Handle) -> Handle {
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(r) = x.checked_sub(y) {
            return box_bigint_i128(r);
        }
    }
    box_bigint(&unbox_bigint(a).sub(&unbox_bigint(b)))
}
pub(crate) fn op_bigint_mul(a: Handle, b: Handle) -> Handle {
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(r) = x.checked_mul(y) {
            return box_bigint_i128(r);
        }
    }
    box_bigint(&unbox_bigint(a).mul(&unbox_bigint(b)))
}
/// `bigint-div` — TRUNCATING integer division (quotient toward zero); TRAPS on a zero divisor (an
/// unbounded range does not give `n/0` a value — numeric-model.md). Returns the quotient.
pub(crate) fn op_bigint_div(a: Handle, b: Handle) -> Handle {
    // FAST PATH (mirrors add/sub/mul): both operands fit i128 (the common case — a runtime BigInt is a
    // BigInt by TYPE, magnitude usually small). Rust's `/` truncates toward zero — EXACTLY `divmod`'s
    // quotient (differential-verified byte-identical across all sign combos + i128 extremes). `checked_div`
    // returns `None` for the two non-representable cases — a ZERO divisor AND the `i128::MIN / -1` overflow
    // — and BOTH then fall through to the `Big` path, which produces the identical result (or traps on
    // zero via `divmod`'s `None`). So no separate zero-guard is needed: the fallback preserves the trap.
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(q) = x.checked_div(y) {
            return box_bigint_i128(q);
        }
    }
    match unbox_bigint(a).divmod(&unbox_bigint(b)) {
        Some((q, _r)) => box_bigint(&q),
        None => trap_bigint_div_zero(),
    }
}
#[cold]
#[inline(never)]
pub(crate) fn trap_bigint_div_zero() -> ! {
    panic!("cdz-runtime: BigInt division by zero")
}
/// `bigint-rem` — the REMAINDER of truncating division (`%`): `a - (a / b) * b`, so its sign is the
/// DIVIDEND's (numeric-model.md — `%` takes the dividend's sign, the companion of `bigint-div`'s
/// truncate-toward-zero). TRAPS on a zero divisor (same as `bigint-div`). `divmod` returns `(q, r)` with
/// exactly this remainder, so this is the `r` half — the whole reason `divmod` computes both at once.
pub(crate) fn op_bigint_rem(a: Handle, b: Handle) -> Handle {
    // FAST PATH: Rust's `%` takes the DIVIDEND's sign — EXACTLY `divmod`'s remainder (differential-verified
    // byte-identical). Like `div`, `checked_rem` returns `None` on a zero divisor and on `i128::MIN % -1`
    // (defined as 0 but the paired division overflows), both falling through to the `Big` path (identical
    // result, or the zero-divisor trap) — so no separate zero-guard is needed.
    if let (Some(x), Some(y)) = (bigint_as_i128(a), bigint_as_i128(b)) {
        if let Some(r) = x.checked_rem(y) {
            return box_bigint_i128(r);
        }
    }
    match unbox_bigint(a).divmod(&unbox_bigint(b)) {
        Some((_q, r)) => box_bigint(&r),
        None => trap_bigint_div_zero(),
    }
}
/// `bigint-cmp` — three-way compare: `-1`/`0`/`1` for `a < b`/`a = b`/`a > b` (the primitive the
/// comparison operators `<`/`>`/`=`/… lower to + a fixed compare). Compares the operands' canonical
/// sign-magnitude `raw` slices DIRECTLY (`Big::cmp_sign_magnitude_bytes`) — a comparison is READ-ONLY, so
/// it needs NO `Big` (no limb `Vec`): allocation-FREE, unlike the arithmetic ops which must build a
/// result. A null/mismatched node reads as the empty slice = canonical zero.
pub(crate) fn op_bigint_cmp(a: Handle, b: Handle) -> i64 {
    let av = unsafe { a.node_ref() };
    let bv = unsafe { b.node_ref() };
    let as_ = av.map_or(&[][..], |n| n.raw.as_slice());
    let bs = bv.map_or(&[][..], |n| n.raw.as_slice());
    match bigint::Big::cmp_sign_magnitude_bytes(as_, bs) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

// ─── Exact rational (Rational) — a NORMALIZED pair of BigInt handles ─────────────────────────────
// A `Rational` value is a 2-HANDLE node `[numerator, denominator]`, each child a BigInt leaf, kept in
// canonical NORMALIZED form: lowest terms (gcd-reduced), the sign on the numerator, the denominator
// strictly positive (`> 0`). So two equal rationals are byte-identical (`2/4` and `1/2` share one node
// shape), and `champ_eq`/`champ_hash` over the two child leaves compare by value. The runtime reuses the
// `bigint::Big` limb arithmetic for the component math; `op_dup`/`op_drop` already recurse into the two
// child handles (a rational is an ordinary 2-handle node), so refcounting needs no special case. A
// runtime Rational is built by `rational-of` from two BigInt handles (which it consumes: it reads both,
// normalizes, and re-boxes the canonical pair, dropping the inputs); the constant fold in the compiler
// never calls these (it emits the folded `num/den` value form directly).

/// Read a Rational node's `(num, den)` components as `i64`s DIRECTLY from the child leaves' raw bytes —
/// `None` if EITHER exceeds i64 range or the node is malformed. No `Big`, no limb `Vec`. The small-operand
/// fast path for the READ-ONLY `rational-cmp`: a runtime Rational built from i64 params (the common R3b
/// case) has i64 components, and then two i64 components cross-multiply into an i128 that CANNOT overflow
/// (|i64| · |i64| < 2¹²⁷), so the compare is exact native arithmetic with zero allocation.
pub(crate) fn rational_components_as_i64(h: Handle) -> Option<(i64, i64)> {
    let n = unsafe { h.node_ref() }?;
    if n.handles.len() != 2 {
        return None;
    }
    let read = |slot: usize| -> Option<i64> {
        let ch = n.handles.get(slot).copied()?;
        let raw = unsafe { ch.node_ref() }.map_or(&[][..], |cn| cn.raw.as_slice());
        bigint::Big::i64_checked_from_sign_magnitude_bytes(raw)
    };
    Some((read(0)?, read(1)?))
}

/// Read a Rational node's two children as `(num, den)` `Big`s. Total: a null/mismatched node reads as
/// `0/1` (deterministic, never a trap — the scalar-read discipline). Borrows the child leaves; does NOT
/// consume the handles.
pub(crate) fn unbox_rational(h: Handle) -> (bigint::Big, bigint::Big) {
    match unsafe { h.node_ref() } {
        Some(n) if n.handles.len() == 2 => (
            unbox_bigint(n.handles.get(0).copied().unwrap_or(Handle::NULL)),
            unbox_bigint(n.handles.get(1).copied().unwrap_or(Handle::NULL)),
        ),
        _ => (bigint::Big::zero(), bigint::Big::from_i64(1)),
    }
}

/// Box a NORMALIZED `(num, den)` pair as a Rational node — a 2-handle node holding the two BigInt leaves.
/// REQUIRES `den` already normalized (lowest terms, strictly positive) by the caller.
pub(crate) fn box_rational_normalized(num: &bigint::Big, den: &bigint::Big) -> Handle {
    alloc_raw(
        Handles::inline_from(&[box_bigint(num), box_bigint(den)]),
        Raw::inline(&[]),
    )
}

/// Normalize + box an i128 `(num, den)` Rational NATIVELY (no `Big`) — the small-operand arithmetic fast
/// path's write half. `den != 0` (the caller ensures). Reduces to lowest terms via an i128 gcd, moves the
/// sign onto the numerator (den strictly positive), then boxes each component via `box_bigint_i128`. Returns
/// `None` (→ caller falls back to the full `Big` path) if either component is `i128::MIN` (whose `abs`
/// overflows i128 — a value that anyway only arises from operands far outside the i64 fast-path domain).
/// Byte-identical to `box_rational_normalized(normalize_rational(Big(num), Big(den)))`.
pub(crate) fn rational_from_i128_pair(mut num: i128, mut den: i128) -> Option<Handle> {
    if num == i128::MIN || den == i128::MIN {
        return None; // abs would overflow — bail to the Big path (unreachable from i64-domain operands)
    }
    if den < 0 {
        num = -num;
        den = -den;
    }
    // i128 gcd (Euclid) over the magnitudes; gcd(0, d) = d.
    let (mut a, mut b) = (num.unsigned_abs(), den.unsigned_abs());
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    let g = a as i128; // g > 0 (den != 0)
    Some(box_rational_node(
        box_bigint_i128(num / g),
        box_bigint_i128(den / g),
    ))
}

/// Box two already-BigInt-handle children as a Rational node (the shared node-build for both the `Big` and
/// the i128 fast paths). CONSUMES the two handles into the node's `handles`.
pub(crate) fn box_rational_node(num: Handle, den: Handle) -> Handle {
    alloc_raw(Handles::inline_from(&[num, den]), Raw::inline(&[]))
}

/// Normalize `(num, den)` → lowest terms, denominator strictly positive, sign on the numerator. REQUIRES
/// `den != 0` (the caller — `rational-of` — traps on a zero denominator before this). `0/d` → `0/1`.
pub(crate) fn normalize_rational(
    num: &bigint::Big,
    den: &bigint::Big,
) -> (bigint::Big, bigint::Big) {
    let g = num.gcd(den); // non-negative; gcd(0, d) = |d|
    let (mut n, _) = num.divmod(&g).expect("gcd is nonzero when den != 0");
    let (mut d, _) = den.divmod(&g).expect("gcd is nonzero when den != 0");
    if d.neg {
        n = n.neg();
        d = d.neg();
    }
    (n, d)
}

/// `rational-of(num, den)` — CONSTRUCT a normalized rational from two BigInt handles. Normalizes (gcd-
/// reduce, sign on numerator, denom > 0). A ZERO denominator has no value → TRAPS. CONSUMES both operand
/// handles (reads then drops them — the caller transfers ownership in, matching the compiler's emit).
pub(crate) fn op_rational_of(num: Handle, den: Handle) -> Handle {
    let (n, d) = (unbox_bigint(num), unbox_bigint(den));
    op_drop(num);
    op_drop(den);
    if d.is_zero() {
        trap_rational_zero_denom();
    }
    let (nn, nd) = normalize_rational(&n, &d);
    box_rational_normalized(&nn, &nd)
}
#[cold]
#[inline(never)]
pub(crate) fn trap_rational_zero_denom() -> ! {
    panic!("cdz-runtime: rational with zero denominator")
}
/// `rational-num(r)` / `rational-den(r)` — the numerator / denominator as a fresh BigInt handle (a DUP of
/// the child leaf, so the rational stays intact — the child is borrowed, the returned handle owned). A
/// null/mismatched node yields the `0/1` components.
pub(crate) fn op_rational_num(r: Handle) -> Handle {
    let (n, _) = unbox_rational(r);
    box_bigint(&n)
}
pub(crate) fn op_rational_den(r: Handle) -> Handle {
    let (_, d) = unbox_rational(r);
    box_bigint(&d)
}
/// `rational-add`/`-sub`/`-mul`/`-div` — exact rational arithmetic over two normalized operands, re-
/// normalized: `a/b + c/d = (ad+cb)/(bd)`, `- = (ad-cb)/(bd)`, `* = (ac)/(bd)`, `÷ = (ad)/(bc)`. All BORROW
/// their operands (unbox reads the child leaves without consuming) and return a FRESH normalized rational.
/// `÷` by `0/1` gives a zero denominator → TRAPS (the rational analogue of ÷0). Never overflow (BigInt).
pub(crate) fn op_rational_add(a: Handle, b: Handle) -> Handle {
    // FAST PATH: all four components fit i64. A cross-product `an·bd`/`bn·ad` is i64·i64 → fits i128; the
    // numerator SUM can reach ±2¹²⁷ so use `checked_add` (overflow → the `Big` path). The denominator
    // `ad·bd` is i64·i64 → fits i128. Byte-identical result; ~23/op → the result node + 2 leaves only.
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        if let Some(num) = (an as i128 * bd as i128).checked_add(bn as i128 * ad as i128) {
            if let Some(h) = rational_from_i128_pair(num, ad as i128 * bd as i128) {
                return h;
            }
        }
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    let num = an.mul(&bd).add(&bn.mul(&ad));
    let den = ad.mul(&bd);
    let (n, d) = normalize_rational(&num, &den);
    box_rational_normalized(&n, &d)
}
pub(crate) fn op_rational_sub(a: Handle, b: Handle) -> Handle {
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        if let Some(num) = (an as i128 * bd as i128).checked_sub(bn as i128 * ad as i128) {
            if let Some(h) = rational_from_i128_pair(num, ad as i128 * bd as i128) {
                return h;
            }
        }
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    let num = an.mul(&bd).sub(&bn.mul(&ad));
    let den = ad.mul(&bd);
    let (n, d) = normalize_rational(&num, &den);
    box_rational_normalized(&n, &d)
}
pub(crate) fn op_rational_mul(a: Handle, b: Handle) -> Handle {
    // `an·bn / ad·bd` — both products are i64·i64 → fit i128, no overflow possible, so no `checked` guard.
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        if let Some(h) = rational_from_i128_pair(an as i128 * bn as i128, ad as i128 * bd as i128) {
            return h;
        }
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    let (n, d) = normalize_rational(&an.mul(&bn), &ad.mul(&bd));
    box_rational_normalized(&n, &d)
}
pub(crate) fn op_rational_div(a: Handle, b: Handle) -> Handle {
    // `an·bd / ad·bn` — both products i64·i64 → fit i128. A zero result-denominator (÷ by 0/1) TRAPS,
    // exactly as the `Big` path does (checked BEFORE the fast-path box, so the trap fires either way).
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        let den = ad as i128 * bn as i128;
        if den == 0 {
            trap_rational_zero_denom();
        }
        if let Some(h) = rational_from_i128_pair(an as i128 * bd as i128, den) {
            return h;
        }
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    let num = an.mul(&bd);
    let den = ad.mul(&bn);
    if den.is_zero() {
        trap_rational_zero_denom();
    }
    let (n, d) = normalize_rational(&num, &den);
    box_rational_normalized(&n, &d)
}
/// `rational-cmp(a, b)` — three-way `-1`/`0`/`1` for `a < b`/`a = b`/`a > b`. Both normalized with a
/// strictly-positive denominator, so `a/b <=> c/d` ⇔ `a·d <=> c·b` (cross-multiply, direction preserved).
/// Borrows both operands. FAST PATH: when all four components fit `i64` (the common case — a Rational
/// built from i64 params), the cross-products `an·bd` and `bn·ad` fit `i128` without overflow (i64·i64 <
/// 2¹²⁷), so the compare is exact NATIVE arithmetic with NO `Big`/limb `Vec` (was 6/op: 4 unbox Vecs + 2
/// mul Vecs → 0). A component out of i64 range falls back to the full `Big` cross-multiply — same result.
pub(crate) fn op_rational_cmp(a: Handle, b: Handle) -> i64 {
    if let (Some((an, ad)), Some((bn, bd))) =
        (rational_components_as_i64(a), rational_components_as_i64(b))
    {
        // an/ad <=> bn/bd ⇔ an·bd <=> bn·ad (both dens > 0). i64·i64 fits i128 exactly.
        let (lhs, rhs) = (an as i128 * bd as i128, bn as i128 * ad as i128);
        return match lhs.cmp(&rhs) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        };
    }
    let ((an, ad), (bn, bd)) = (unbox_rational(a), unbox_rational(b));
    match an.mul(&bd).cmp(&bn.mul(&ad)) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

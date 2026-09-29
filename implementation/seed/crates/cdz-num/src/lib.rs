//! `cdz-num` — the Cadenza arbitrary-precision numeric library for the RUST BACKEND.
//!
//! The `rcdzc` rust backend emits programs that use the SAME bignum (`Big`) the wasm runtime uses —
//! reuse, not a second implementation (the operator-directed numeric-tower approach). Both this crate
//! and `cdz-runtime` get `Big` from the shared `etude-bigint` crate (operator seq-1332), so the rust
//! backend and the wasm runtime agree on `Big` by construction.
//!
//! This crate deps `etude-bigint` DIRECTLY (a normal external crate dep) and re-exports its `Big`. It does
//! NOT `#[path]`-include `cdz-runtime`'s source (operator seq-1385: no path-includes), and it is NOT a
//! dependency of `cdz-runtime` — so it never enters the runtime's cross-crate-LTO set (that split broke
//! frozen-hash reproducibility in #459). cdz-num pins the etude-bigint rev carrying `Big: Ord`/`PartialOrd`
//! (PR #328), which the rust backend needs for BigInt-keyed `BTreeMap`/`BTreeSet`; that rev may LEAD
//! cdz-runtime's frozen-hash pin (separate Cargo.locks, and `Ord` is unused in the runtime → DCE'd → the
//! runtime wasm hash is unaffected). See [[runtime-cross-arch-determinism]] and DESIGN-rust-runtime-trait.md.

// `Rational` (below) keeps an inherent `cmp` alongside its `Ord` impl (which delegates to it), a pattern
// `clippy::should_implement_trait` flags; allow it — the inherent form is a convenient direct call site.
#![allow(clippy::should_implement_trait)]

// `Rational::to_display_string` builds an `alloc::string::String`, so `alloc` must be in scope. This crate
// is a plain host rlib (it keeps `std`); naming `alloc` explicitly is always valid under `std`.
extern crate alloc;

// The shared bignum, re-exported at the crate root: the rust backend emits `cdz_num::Big`. `Big` and its
// `Ord`/`PartialOrd` (a bignum is a canonical total order) come from `etude-bigint` itself (PR #328), so
// cdz-num needs no local trait impls — and could not add them, since `Big` is a foreign type.
pub use etude_bigint::Big;

/// An exact rational — a `Big` numerator over a `Big` denominator, held in the runtime's CANONICAL
/// NORMALIZED form: lowest terms (gcd-reduced), sign on the numerator, denominator strictly positive.
/// The rust backend emits `cdz_num::Rational` for a `Ty::Rational` value; the ops below MIRROR
/// cdz-runtime's `op_rational_*` (its `Big`-path — the i64 fast-path there is a pure perf optimization
/// producing BYTE-IDENTICAL values), so a Rust program's rational result equals the wasm oracle. Built on
/// `Big`'s public API (`gcd`/`divmod`/`add`/`sub`/`mul`/`neg`/`cmp`), so it stays a source-only value type
/// (no runtime Handle, no frozen-hash surface — the rational logic lives HERE, not source-shared from the
/// Handle-based runtime, which has no standalone `Rational`). A zero denominator TRAPS (panics) at
/// construction, matching the runtime's `trap_rational_zero_denom`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rational {
    pub num: Big,
    pub den: Big,
}

impl Rational {
    /// Build from a numerator/denominator pair, NORMALIZING to canonical form (gcd-reduce, sign on the
    /// numerator, denominator > 0). Mirrors the runtime `normalize_rational` + `box_rational_normalized`.
    /// A zero denominator has no value → panics (the runtime traps here).
    pub fn new(num: Big, den: Big) -> Rational {
        if den.is_zero() {
            panic!("Rational with zero denominator");
        }
        let (num, den) = normalize(num, den);
        Rational { num, den }
    }

    /// The whole rational `n/1` from an integer.
    pub fn from_big(n: Big) -> Rational {
        Rational {
            num: n,
            den: Big::from_i64(1),
        }
    }

    /// `a + b` = `(an·bd + bn·ad) / (ad·bd)`, normalized. Mirrors `op_rational_add`'s Big path.
    pub fn add(&self, other: &Rational) -> Rational {
        let num = self.num.mul(&other.den).add(&other.num.mul(&self.den));
        let den = self.den.mul(&other.den);
        Rational::new(num, den)
    }

    /// `a - b` = `(an·bd − bn·ad) / (ad·bd)`, normalized. Mirrors `op_rational_sub`.
    pub fn sub(&self, other: &Rational) -> Rational {
        let num = self.num.mul(&other.den).sub(&other.num.mul(&self.den));
        let den = self.den.mul(&other.den);
        Rational::new(num, den)
    }

    /// `a · b` = `(an·bn) / (ad·bd)`, normalized. Mirrors `op_rational_mul`.
    pub fn mul(&self, other: &Rational) -> Rational {
        Rational::new(self.num.mul(&other.num), self.den.mul(&other.den))
    }

    /// `a / b` = `(an·bd) / (ad·bn)`, normalized. TRAPS (panics) when `b` is zero (`ad·bn` zero). Mirrors
    /// `op_rational_div` (which checks the denominator `is_zero` before normalizing).
    pub fn div(&self, other: &Rational) -> Rational {
        let num = self.num.mul(&other.den);
        let den = self.den.mul(&other.num);
        if den.is_zero() {
            panic!("Rational divide by zero");
        }
        Rational::new(num, den)
    }

    /// Three-way compare. Both denominators are positive (normalized), so `an/ad <=> bn/bd` ⇔
    /// `an·bd <=> bn·ad`. Mirrors `op_rational_cmp`'s cross-multiply.
    pub fn cmp(&self, other: &Rational) -> core::cmp::Ordering {
        self.num.mul(&other.den).cmp(&other.num.mul(&self.den))
    }

    /// Render as cdz-run's canonical `n/d` text (matching the ML `print_display` rational form). A
    /// whole rational (`d == 1`) still renders `n/1` — the runtime keeps the explicit denominator.
    pub fn to_display_string(&self) -> alloc::string::String {
        let mut s = self.num.to_decimal_string();
        s.push('/');
        s.push_str(&self.den.to_decimal_string());
        s
    }
}

// `Rational` derives `Clone + PartialEq + Eq` (a normalized pair is byte-comparable — lowest terms + sign
// on num means equal values have identical (num, den)). It needs `Ord` too: a `Rational`-keyed
// `BTreeSet`/`BTreeMap` requires it, and `Rational::cmp` is a total order (cross-multiply, positive denoms).
// Provide the trait impls here (same rationale as `Big`'s — `Rational` is LOCAL to cdz-num, not orphan).
impl Ord for Rational {
    fn cmp(&self, other: &Rational) -> core::cmp::Ordering {
        Rational::cmp(self, other) // the inherent cross-multiply compare
    }
}
impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Rational) -> Option<core::cmp::Ordering> {
        Some(Ord::cmp(self, other))
    }
}

/// Normalize `(num, den)` to canonical form: gcd-reduce, then move the sign to the numerator so the
/// denominator is strictly positive. Mirrors cdz-runtime's `normalize_rational` exactly (same `Big::gcd`/
/// `divmod`/`neg` composition), so a built value is byte-identical to the runtime's. `den` is nonzero
/// (checked by the callers), so the gcd is nonzero and both `divmod`s succeed.
fn normalize(num: Big, den: Big) -> (Big, Big) {
    let g = num.gcd(&den); // non-negative; gcd(0, d) = |d|
    let (mut n, _) = num.divmod(&g).expect("gcd is nonzero when den != 0");
    let (mut d, _) = den.divmod(&g).expect("gcd is nonzero when den != 0");
    if d.is_negative() {
        n = n.neg();
        d = d.neg();
    }
    (n, d)
}

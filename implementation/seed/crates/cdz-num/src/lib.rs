//! `cdz-num` — the Cadenza arbitrary-precision numeric library for the RUST BACKEND.
//!
//! This crate exists so the `rcdzc` rust backend can emit programs that use the SAME bignum (`Big`) the
//! wasm runtime uses — reuse, not a second implementation (the operator-directed numeric-tower approach).
//!
//! # Why a SOURCE share (`#[path]`), not a shared linked crate
//! The obvious design — extract `bigint.rs` into a crate that BOTH `cdz-runtime` and this backend depend
//! on — was tried and REVERTED (#459): adding a second crate to `cdz-runtime`'s cross-crate-LTO set made
//! the frozen runtime wasm CROSS-MACHINE NON-DETERMINISTIC (a crate unit's `-Cmetadata` is baked into the
//! LTO'd symbol layout and varies by build env, so three machines produced three different
//! `REQUIRED_RUNTIME_HASH` values; debug matched because dev has no cross-crate LTO merge). v-runtime
//! (frozen-hash owner) RULED: keep `Big` PHYSICALLY IN `cdz-runtime` as a plain module (its single-crate
//! LTO stays reproducible, hash stays `def9d173`, nothing to re-freeze) and share the SOURCE here.
//!
//! So this crate brings in `cdz-runtime/src/bigint.rs` VERBATIM as a `#[path]` module: `Big` is compiled
//! as part of THIS crate's own build (a normal rlib the gate links via `--extern`), while `cdz-runtime`
//! keeps the exact same file as its own module in its own single-crate build. Zero SOURCE duplication
//! (one file, two compiles), and `cdz-runtime`'s wasm build is untouched — no determinism regression, no
//! hash churn. See [[runtime-cross-arch-determinism]] and DESIGN-rust-runtime-trait.md.

// The `#[path]`-included `bigint.rs` is v-runtime's SOURCE, physically owned by `cdz-runtime` (an
// excluded standalone workspace with its own lint posture — AND its bytes feed the frozen wasm content
// hash, so it must NOT be reformatted here). Pulling it into THIS (main-workspace) crate would otherwise
// subject it to `clippy -D warnings` (it trips a few style lints the excluded crate never gated) and to
// `cargo fmt --all` (which would rewrite its compact struct literals, churning the frozen hash). So this
// crate-level `#![allow(...)]` silences the borrowed source's style lints, and the module carries
// `#[rustfmt::skip]` (below) to leave the shared file byte-for-byte — neither touches bigint.rs.
#![allow(
    clippy::needless_range_loop,
    clippy::should_implement_trait,
    clippy::neg_multiply
)]

// `bigint.rs` writes fully-qualified `alloc::…` paths (it was authored as a `no_std` module of the
// `no_std` runtime), so `alloc` must be in scope here. Bringing it in explicitly works under `std` too
// (a `std` crate can always name `alloc`), so this crate needs no `#![no_std]` — it is a plain host rlib.
extern crate alloc;

// The bignum, verbatim from the runtime, brought in via `#[path]` as a real module file. `#[path]` (not
// `include!`) because `bigint.rs` opens with `//!` INNER doc comments (module docs): `include!` pastes
// tokens inline and rejects a leading `//!` (E0753 "inner doc comments can only appear before items"),
// whereas `#[path = "…"] mod big;` treats the file AS module `big`'s own source — exactly the role it
// plays in `cdz-runtime` — where its `//!` module docs are valid. `#[rustfmt::skip]` keeps `cargo fmt`
// from rewriting the shared file. Same file, same semantics, no edits to the shared source.
#[rustfmt::skip]
#[path = "../../cdz-runtime/src/bigint.rs"]
pub mod big;

// The rust backend emits `cdz_num::Big`, so surface `Big` at the crate root (the submodule is an
// include-mechanics detail, not part of the API shape the emit targets).
pub use big::Big;

// `Big` derives `Clone + PartialEq + Eq` in bigint.rs but NOT `Ord` (the runtime never needed a Rust
// `Ord` — it orders BigInt leaves by canonical bytes in its own CHAMP). The rust backend DOES need it:
// a `BigInt`-keyed `BTreeSet`/`BTreeMap` requires `Big: Ord`. `Big` already HAS a total three-way
// `cmp(&self, &Big) -> Ordering` (the signed comparison), so provide the trait impls here — in cdz-num,
// where `Big` is a LOCAL type (its `#[path]` module is ours), so this is NOT an orphan-rule violation
// and does NOT touch the frozen bigint.rs source. Consistent with `Eq` (the derived `==` agrees with
// `cmp == Equal` — same canonical-form value equality). This is why `types::ty_is_ord` treats `BigInt`
// as orderable.
impl Ord for Big {
    fn cmp(&self, other: &Big) -> core::cmp::Ordering {
        // Delegate to `Big`'s inherent signed three-way compare (defined in bigint.rs). Fully qualified
        // so it resolves to the INHERENT method, not this trait method (which would recurse).
        big::Big::cmp(self, other)
    }
}
impl PartialOrd for Big {
    // Canonical form (clippy::non_canonical_partial_ord_impl): defer to `Ord::cmp`, which holds the logic.
    fn partial_cmp(&self, other: &Big) -> Option<core::cmp::Ordering> {
        Some(Ord::cmp(self, other))
    }
}

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
    if d.neg {
        n = n.neg();
        d = d.neg();
    }
    (n, d)
}

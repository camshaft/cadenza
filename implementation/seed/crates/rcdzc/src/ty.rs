//! The solved-type universe — what inference determines and every pass below reads.
//!
//! A node's solved type is materialized once by inference into the type column and read downstream;
//! no later pass re-derives it (`reference-compiler.md` §Types Are Solved Once And Read Downstream).
//! A pass that must choose a value's *machine* representation reads it off this type
//! (`reference-compiler.md` §The Machine Representation Is A Read-Off Of The Solved Type).
//!
//! This type is **target-neutral**: it sits above the backend seam and carries NO wasm valtype byte,
//! no component-model encoding — those are a target's concern, computed by that target's backend
//! (`backends-and-targets.md` §The Boundary Layout Is Computed Once, Target-Neutrally, And Reused).
//! The wasm backend maps a `Ty` to its own valtypes (see `backend::wasm`); a second backend maps the
//! same `Ty` to its target's representation. What lives here is only the language-level type and the
//! names the value renderer supplies (`reference-compiler.md` §Rendering Walks A Static Shape And
//! Supplies The Names) — a language fact, not a target one.
//!
//! The universe is a CLOSED, exhaustively-matched set of variants: integers (width- and
//! sign-indexed), Bool, Unit, records, tuples, sums, function types, the type-value type, a
//! unification variable, and `Any`. Because it is closed, adding a type is adding a variant, which
//! forces every pass that reads a type to say what it does with it — the property that keeps a new
//! type honest as the language grows.
//!
//! **An integer type carries its width and signedness, not a fixed name.** `Int64` is not a type
//! unto itself — it is the signed, 64-bit instance of the one integer type `(Int width)`, whose
//! signedness and width are data:
//!
//= spec/capabilities/numeric-model.md#an-integer-type-is-indexed-by-a-compile-time-width
//# An integer type MUST be identified by a signedness and a bit width, so that two integer types of different width or signedness are distinct types that do not silently convert to one another.
//!
//! So a width is a value the compiler computes and unifies, never a new `Ty` variant per width — the
//! single [`IntTy`] carries both axes, and every named width (`Int8`, `UInt32`, …) is one instance.

/// The default bit width an integer literal grounds to when nothing fixes it — the width the backend
/// picks for an unresolved literal (`Int64`).
pub const DEFAULT_INT_WIDTH: u32 = 64;

/// The width of an integer type — the parameter that makes an intrinsic generic over the integer
/// type. Three states: a `Fixed` concrete width (`64` = `Int64`), a `Deferred` width a bare literal
/// carries until a constraint fixes it (numeric-literal polymorphism, grounds to the default), or a
/// `Var` unification variable an intrinsic's signature introduces so `+ : (Int w) → (Int w) → (Int
/// w)` unifies its operands' widths rather than hard-coding one (`build-order.md` §Stage 2 — generic
/// over the integer type). Inference resolves a `Var`/`Deferred` to a `Fixed`; the backend grounds a
/// still-unresolved width to the default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Width {
    Fixed(u32),
    Deferred,
    Var(u32),
}

/// The signedness of an integer type — the SAME three-state shape as [`Width`], because a bare integer
/// literal is polymorphic in its sign exactly as it is in its width. `Fixed(true)` = signed,
/// `Fixed(false)` = unsigned; `Deferred` = a bare literal's sign before anything constrains it (grounds
/// to signed); `Var` = a unification variable an intrinsic/annotation introduces. Making sign a
/// variable (not a baked `bool`) is what lets `(: 200 UInt8)` GROUND a literal to unsigned through
/// ordinary unification — "Annotations Constrain, Never Contradict" — rather than clashing with a
/// signed default. Inference resolves a `Var`/`Deferred` to a `Fixed`; the backend grounds a
/// still-unresolved sign to signed (the default literal type).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sign {
    Fixed(bool),
    Deferred,
    Var(u32),
}

/// An integer type: a [`Sign`] and a [`Width`]. `IntTy { sign: Fixed(true), width: Fixed(64) }` is
/// `Int64`. Both axes unify (unify only at equal sign and width, no implicit promotion) and both can be
/// deferred (a bare literal) or a variable (an operator generic over the integer type) — so a width
/// AND a signedness are data the compiler unifies, never a hard-coded case.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IntTy {
    pub sign: Sign,
    pub width: Width,
}

impl IntTy {
    /// A deferred integer — the type a bare integer literal takes before any constraint or defaulting
    /// fixes its sign and width. BOTH axes are deferred, so an annotation (or an operator's signature)
    /// can ground either.
    pub fn deferred() -> IntTy {
        IntTy {
            sign: Sign::Deferred,
            width: Width::Deferred,
        }
    }

    /// The signed 64-bit integer (`Int64`) — the concrete type an unresolved sign+width grounds to.
    pub fn i64() -> IntTy {
        IntTy {
            sign: Sign::Fixed(true),
            width: Width::Fixed(DEFAULT_INT_WIDTH),
        }
    }

    /// A concrete `(signed, width)` integer — the ordinary constructor for a fixed integer type.
    pub fn fixed(signed: bool, width: u32) -> IntTy {
        IntTy {
            sign: Sign::Fixed(signed),
            width: Width::Fixed(width),
        }
    }

    /// The concrete SIGNEDNESS this integer takes at the machine boundary — its fixed sign, or signed
    /// (the default) if still deferred or an unresolved variable. The backend/renderer reads THIS.
    pub fn ground_signed(self) -> bool {
        match self.sign {
            Sign::Fixed(s) => s,
            Sign::Deferred | Sign::Var(_) => true,
        }
    }

    /// The concrete width this integer takes at the machine boundary: its fixed width, or the default
    /// if still deferred or an unresolved variable. The backend reads THIS to pick a representation,
    /// so a literal whose width inference never constrained still lowers to a definite width.
    pub fn ground_width(self) -> u32 {
        match self.width {
            Width::Fixed(w) => w,
            Width::Deferred | Width::Var(_) => DEFAULT_INT_WIDTH,
        }
    }

    /// Whether this integer's width was FIXED by inference (an annotation `(Int 8)` / an operator
    /// signature), rather than DEFAULTED (a bare literal whose width nothing constrained, so it grounds
    /// to the default `Int64`). Distinguishes a literal that overflows a CHOSEN width (`(: 256 (Int 8))`
    /// — an out-of-range error, CDZ0302) from one that overflows the DEFAULT `Int64` with no width in
    /// sight (`9223372036854775808` — a malformed literal, CDZ0201): only a defaulted width has "no
    /// annotation to blame".
    pub fn width_is_fixed(self) -> bool {
        matches!(self.width, Width::Fixed(_))
    }

    /// The inclusive `[min, max]` value range of this integer at its ground (signed, width), in `i128`
    /// space so every 1..=64-bit bound is exact (`2^64` fits `i128`). Signed → `[-2^(w-1), 2^(w-1)-1]`;
    /// unsigned → `[0, 2^w - 1]`.
    fn ground_range(self) -> (i128, i128) {
        let w = self.ground_width();
        if self.ground_signed() {
            let half = 1i128 << (w - 1);
            (-half, half - 1)
        } else {
            (0, (1i128 << w) - 1)
        }
    }

    /// Whether EVERY value of this (source) integer type provably fits `target` — i.e. `self`'s value
    /// range is a subset of `target`'s. When true, a checked conversion `target.of(x : self)` can NEVER
    /// trap (every source value is in range), so it is a pure representation change identical to
    /// `target.wrap(x)` — the backend emits the extend-and-reinterpret with no range check. When false
    /// (a narrowing, or an unsigned→signed of equal width where the top value overflows), the conversion
    /// is genuinely checked and must trap at run time. Both ranges are computed in `i128`, so this is
    /// exact for all 1..=64-bit widths.
    pub fn fits_within(self, target: IntTy) -> bool {
        let (s_min, s_max) = self.ground_range();
        let (t_min, t_max) = target.ground_range();
        t_min <= s_min && s_max <= t_max
    }
}

/// The default float width a float literal grounds to when nothing fixes it (`Float64` — binary64).
pub const DEFAULT_FLOAT_WIDTH: u32 = 64;

/// The set of IEEE-754 binary float widths the numeric model admits — the widths a conforming wasm
/// runtime provides (`f32`/`f64`), pinned at `options/numeric-model/`. A `(Float N)` for any other `N`
/// fails the width constraint (CDZ0302), exactly as an out-of-range integer width does. This is
/// set-MEMBERSHIP, not a range (unlike integers, where every width in `1..=64` is a type): a float
/// width is an IEEE format, not an arbitrary bit count. The admitted-set check lives at the `Float`
/// constructor (`prelude::build_float_ty`), exactly as the integer `1..=64` check lives at `Int`.
pub const ADMITTED_FLOAT_WIDTHS: [u32; 2] = [32, 64];

/// The integer widths that have a pre-installed ALIAS name in the prelude (`Int8`/`Int16`/`Int32`/
/// `Int64` and their `UInt` twins — `prelude::install`). Every width in `1..=64` is a valid integer
/// TYPE, but only these have a BOUND module name; `(Int N)` for any other width is built on demand and
/// has no name to write. A diagnostic that suggests a conversion `(IntN.of …)` must restrict to THIS set
/// so the suggested name actually resolves — a non-aliased `Int48` would be an unbound name.
pub const ALIASED_INT_WIDTHS: [u32; 4] = [8, 16, 32, 64];

/// A floating-point type: a [`Width`] (there is no signedness axis — a float is inherently signed, so
/// this is [`IntTy`] minus its `Sign`). `FloatTy { width: Fixed(64) }` is `Float64` (binary64), the
/// type a bare float literal grounds to; `Fixed(32)` is `Float32`. The width is polymorphic exactly as
/// an integer's is: `Deferred` a bare literal, `Var` an operator generic over the float width (`+. :
/// (Float a) → (Float a) → (Float a)`), `Fixed` a concrete width. Crucially it REUSES the integer
/// [`Width`] machinery (`unify_width`/`apply_width`/freshen) — parametric "just like ints" — because a
/// width variable is a width variable regardless of whether it ends up bound in a float or integer
/// context, and a float only ever unifies with a float so the two never cross. Only the admitted SET
/// differs ({32,64} vs 1..=64), enforced at the constructor, not here.
///
/// A float type IS its width (there is no other axis), so two float types of different width are
/// distinct types that never silently unify; and the width is a compile-time [`Width`] (a literal or a
/// resolved variable), never a runtime value:
///
//= spec/capabilities/numeric-model.md#a-floating-point-type-is-indexed-by-a-compile-time-width
//# A floating-point type MUST be identified by a bit width drawn from the set of IEEE-754 binary formats the numeric model admits, so that two floating-point types of different width are distinct types that do not silently convert to one another.
///
//= spec/capabilities/numeric-model.md#a-floating-point-type-is-indexed-by-a-compile-time-width
//# The bit width of a floating-point type MUST be resolved from a compile-time value and MUST NOT be determined by runtime data, so that a floating-point type's width is fixed before the program runs rather than dependent on a value computed at runtime.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FloatTy {
    pub width: Width,
}

impl FloatTy {
    /// A deferred float — the type a bare float literal takes before any constraint or defaulting fixes
    /// its width. An annotation (or a float operator's signature) can ground it.
    pub fn deferred() -> FloatTy {
        FloatTy {
            width: Width::Deferred,
        }
    }

    /// The 64-bit float (`Float64`, binary64) — the concrete type an unresolved float width grounds to.
    pub fn f64() -> FloatTy {
        FloatTy {
            width: Width::Fixed(DEFAULT_FLOAT_WIDTH),
        }
    }

    /// A concrete-width float — the ordinary constructor for a fixed float type (`(Float 32)`/`(Float
    /// 64)`).
    pub fn fixed(width: u32) -> FloatTy {
        FloatTy {
            width: Width::Fixed(width),
        }
    }

    /// The concrete width this float takes at the machine boundary: its fixed width, or the default
    /// (`Float64`) if still deferred or an unresolved variable. The backend/renderer reads THIS.
    pub fn ground_width(self) -> u32 {
        match self.width {
            Width::Fixed(w) => w,
            Width::Deferred | Width::Var(_) => DEFAULT_FLOAT_WIDTH,
        }
    }

    /// Whether this float's width is CONCRETELY fixed (a `Float32`/`Float64`), not a deferred literal or
    /// an unresolved variable — the float twin of [`IntTy::width_is_fixed`]. Used to prefer a
    /// concrete-width operand over a deferred literal when typing `+`/`-`/`*`/`/` over floats.
    pub fn width_is_fixed(self) -> bool {
        matches!(self.width, Width::Fixed(_))
    }
}

/// A UNIT — an element of the free abelian group over named base dimensions (`units-of-measure.md` §A
/// Dimension Groups Interconvertible Units; `options/units-of-measure/`). A unit is a canonical map from
/// a base-dimension NAME to a signed integer EXPONENT, with every zero-exponent base DROPPED, so the
/// group identity `Unit.one` (a dimensionless quantity) is the EMPTY map and two units are the SAME
/// DIMENSION exactly when their maps are EQUAL — a finite, order-independent, solver-free compile-time
/// comparison (the F#-units model, `options/units-of-measure/erased-compile-time-quantity.md`). Held in
/// a `BTreeMap` so the canonical order is the key order (making `==` decide dimensional equality and the
/// render order deterministic) and cloning is cheap for the small maps units are.
///
/// The base-dimension name is a `String` in Layer 1 (the erasure-only dimensional core): the corpus
/// names base dimensions with a symbol literal `#"meter"`, read as its text. When the `Symbol` type
/// lands (Layer 2, families/prefixes), this key becomes a `resolved::Symbol`; nothing else about the
/// group algebra changes. A unit is a COMPILE-TIME value: it indexes `Ty::Qty` and is ERASED before
/// emission, so it never reaches the backend (`units-of-measure.md` §Dimensions Are Checked Then Erased).
/// A UNIT = a DIMENSION (an exponent map) together with a compile-time SCALE to that dimension's
/// reference (`units-of-measure.md` §A Unit Carries An Exact Scale To Its Dimension's Reference). Two
/// concepts, kept distinct on purpose:
///   - the DIMENSION `exp` — the free-abelian-group element (base name → signed exponent, zero-exponent
///     bases dropped). This alone gates COMPATIBILITY: `+`/`-`/comparison require EQUAL dimensions
///     (`same_dimension`), and `*`/`/` compose them. `meter` and `kilometer` are the SAME dimension.
///   - the SCALE `scale_num`/`scale_den` — a machine-integer ratio to the dimension's reference unit
///     (`meter`=1/1, `kilometer`=1000/1, `millimeter`=1/1000, `mebibyte`=2²⁰/1). This is what makes
///     `meter` and `kilometer` DISTINCT units (distinct types) of one dimension, and what a conversion
///     multiplies by. The scale is COMPILE-TIME METADATA, not a runtime `Rational`: a conversion emits
///     `value * num / den` in the quantity's OWN inner numeric type T (spec §48: it "los[es] precision
///     only where the underlying numeric type is itself inexact" — exact over `Int`/`Rational`, rounding
///     over `Float`), so NO arbitrary-precision value is needed. Every real unit scale fits `i128` with
///     enormous headroom (the largest, `tera`=10¹² / `tebi`=2⁴⁰ / `mile`=201168/125).
///
/// TYPE IDENTITY (`==`, `Ord`, `Hash`) compares BOTH map and scale, so `(Qty T meter)` and `(Qty T
/// kilometer)` are distinct static types (crossing needs an explicit conversion); DIMENSIONAL
/// compatibility (`same_dimension`) compares the map ALONE. A base-dimension name is a `String` in
/// Layer 1 (the corpus writes `#"meter"`); it becomes a `resolved::Symbol` when Symbols land. A unit is
/// COMPILE-TIME: it indexes `Ty::Qty` and is ERASED before emission (§Dimensions Are Checked Then
/// Erased) — only the scale MULTIPLY a mixed-unit combine denotes may survive, as ordinary T arithmetic.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Unit {
    /// The dimension: base name → signed exponent, zero-exponent bases dropped (the group element).
    exp: std::collections::BTreeMap<String, i64>,
    /// The scale to the dimension's reference unit, as a normalized machine-integer ratio (den > 0,
    /// lowest terms). `1/1` for a reference/base unit; `1000/1` for a kilo-prefixed unit; `1/1000` for a
    /// milli-prefixed one. Compile-time metadata a conversion multiplies by (in the inner type T).
    scale_num: i128,
    scale_den: i128,
}

impl Default for Unit {
    fn default() -> Unit {
        Unit::one()
    }
}

impl Unit {
    /// The dimensionless unit — the group identity (EMPTY exponent map, scale 1/1). `Unit.one`.
    pub fn one() -> Unit {
        Unit {
            exp: std::collections::BTreeMap::new(),
            scale_num: 1,
            scale_den: 1,
        }
    }

    /// A base dimension named `name`, to the first power, at the REFERENCE scale 1/1 — the single-entry
    /// map `{name: 1}`. `(Unit.base #"meter")`. A base unit IS the scale-1 reference of its dimension.
    pub fn base(name: impl Into<String>) -> Unit {
        let mut m = std::collections::BTreeMap::new();
        m.insert(name.into(), 1);
        Unit {
            exp: m,
            scale_num: 1,
            scale_den: 1,
        }
    }

    /// Whether this is the dimensionless unit (the empty map) — a `(Qty T Unit.one)` scaled result, or a
    /// ratio of like quantities that cancelled to no dimension. (Ignores the scale — a dimensionless
    /// SCALED value, e.g. a bare ratio, is still dimensionless.)
    pub fn is_dimensionless(&self) -> bool {
        self.exp.is_empty()
    }

    /// Whether two units share a DIMENSION — their exponent maps are equal, IGNORING scale. This gates
    /// combine/compare compatibility (`meter` + `kilometer` is well-formed; `meter` + `second` is
    /// CDZ0501) and is the relation `units-of-measure.md` §Combining Units Of One Dimension Is
    /// Well-Formed rests on. Distinct from `==` (type identity), which also requires equal scale.
    pub fn same_dimension(&self, other: &Unit) -> bool {
        self.exp == other.exp
    }

    /// This unit's scale to its dimension's reference, as a `(num, den)` machine-integer ratio. A
    /// conversion from this unit to another of the same dimension multiplies a value by `self.scale /
    /// other.scale` (in the inner numeric type). `1/1` for a reference unit.
    pub fn scale(&self) -> (i128, i128) {
        (self.scale_num, self.scale_den)
    }

    /// This unit at the REFERENCE scale (1/1) — its dimension with the scale dropped. The common unit a
    /// mixed-unit combine converts to (`units-of-measure.md` §the common (reference) unit), and the unit
    /// a conversion's result carries.
    pub fn at_reference(&self) -> Unit {
        Unit {
            exp: self.exp.clone(),
            scale_num: 1,
            scale_den: 1,
        }
    }

    /// This unit SCALED by a compile-time ratio `num/den` — a prefix (`kilo` = 1000/1, `milli` = 1/1000,
    /// `mebi` = 2²⁰/1) applied to a unit produces another unit of the SAME dimension differing only by
    /// that exact factor (`units-of-measure.md` §A Scaled Unit Is A Unit Scaled By An Exact Factor). The
    /// scales MULTIPLY (a `(Unit.prefix kilo meter)` squared is km², scale 10⁶). Normalizes the result
    /// ratio (lowest terms, positive denominator). `None` if `den == 0` or an intermediate overflows
    /// `i128` (no real prefix comes close).
    pub fn scaled(&self, num: i128, den: i128) -> Option<Unit> {
        let n = self.scale_num.checked_mul(num)?;
        let d = self.scale_den.checked_mul(den)?;
        let (n, d) = normalize_ratio(n, d)?;
        Some(Unit {
            exp: self.exp.clone(),
            scale_num: n,
            scale_den: d,
        })
    }

    /// The PRODUCT of two units — pointwise exponent ADD, dropping any base whose combined exponent is
    /// zero (the drop-zeros canonicalization). `(Unit.* meter meter)` = `{meter: 2}`; a base times its
    /// inverse cancels to `Unit.one`. The `*`/`/` dimensional rule (`units-of-measure.md` §An Operation
    /// That Derives A Dimension).
    pub fn mul(&self, other: &Unit) -> Unit {
        let mut m = self.exp.clone();
        for (k, e) in &other.exp {
            let entry = m.entry(k.clone()).or_insert(0);
            *entry += e;
            if *entry == 0 {
                m.remove(k);
            }
        }
        // The scales MULTIPLY (a product's scale is the product of the operand scales — `km·km` = 10⁶
        // m²); normalize, falling back to 1/1 on overflow (no real derived unit reaches i128).
        let (sn, sd) = normalize_ratio(
            self.scale_num.saturating_mul(other.scale_num),
            self.scale_den.saturating_mul(other.scale_den),
        )
        .unwrap_or((1, 1));
        Unit {
            exp: m,
            scale_num: sn,
            scale_den: sd,
        }
    }

    /// The QUOTIENT of two units — pointwise exponent SUBTRACT, dropping zeros. `(Unit./ meter second)` =
    /// `{meter: 1, second: -1}` (a velocity); `(Unit./ meter meter)` = `Unit.one` (a ratio of like
    /// quantities is dimensionless, decided by the exponent map going to all-zero).
    pub fn div(&self, other: &Unit) -> Unit {
        let mut m = self.exp.clone();
        for (k, e) in &other.exp {
            let entry = m.entry(k.clone()).or_insert(0);
            *entry -= e;
            if *entry == 0 {
                m.remove(k);
            }
        }
        // The scales DIVIDE (a quotient's scale is the quotient of the operand scales); normalize,
        // falling back to 1/1 on overflow.
        let (sn, sd) = normalize_ratio(
            self.scale_num.saturating_mul(other.scale_den),
            self.scale_den.saturating_mul(other.scale_num),
        )
        .unwrap_or((1, 1));
        Unit {
            exp: m,
            scale_num: sn,
            scale_den: sd,
        }
    }

    /// This unit raised to a compile-time integer power `n` (may be negative) — each exponent scaled by
    /// `n`, dropping zeros. `n = 0` yields `Unit.one` (any dimension to the zeroth power is dimensionless).
    /// `(Unit.^ meter 2)` = `{meter: 2}` (area), `(Unit.^ second -1)` = `{second: -1}` (frequency).
    pub fn pow(&self, n: i64) -> Unit {
        if n == 0 {
            return Unit::one();
        }
        let mut m = std::collections::BTreeMap::new();
        for (k, e) in &self.exp {
            let scaled = e * n;
            if scaled != 0 {
                m.insert(k.clone(), scaled);
            }
        }
        // The scale is raised to the SAME power: `(km)²` has scale 10⁶ (a positive power multiplies the
        // ratio n times; a negative power inverts). Normalize; fall back to 1/1 on overflow.
        let (base_n, base_d) = if n >= 0 {
            (self.scale_num, self.scale_den)
        } else {
            (self.scale_den, self.scale_num) // invert for a negative power
        };
        let times = n.unsigned_abs() as u32;
        let mut sn: i128 = 1;
        let mut sd: i128 = 1;
        let mut ok = true;
        for _ in 0..times {
            match (sn.checked_mul(base_n), sd.checked_mul(base_d)) {
                (Some(a), Some(b)) => {
                    sn = a;
                    sd = b;
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        let (sn, sd) = if ok {
            normalize_ratio(sn, sd).unwrap_or((1, 1))
        } else {
            (1, 1)
        };
        Unit {
            exp: m,
            scale_num: sn,
            scale_den: sd,
        }
    }

    /// The base→exponent pairs in canonical (sorted) order — for encoding the unit into an arena subtree
    /// and for rendering. (The scale is carried separately, via [`scale`].)
    pub fn entries(&self) -> impl Iterator<Item = (&String, &i64)> {
        self.exp.iter()
    }

    /// Render the unit for a `(Qty T <unit>)` type annotation — the canonical written form. The
    /// dimensionless unit renders `Unit.one`; a single base to the first power renders `(Unit.base
    /// #"name")`; a base to a power renders `(Unit.^ (Unit.base #"name") n)`; a product of several
    /// renders a left-nested `(Unit.* …)`. This mirrors the corpus surface so a rendered quantity type
    /// re-reads to the same unit.
    pub fn render(&self) -> String {
        if self.exp.is_empty() {
            return "Unit.one".to_string();
        }
        let mut factors: Vec<String> = Vec::new();
        for (name, &exp) in &self.exp {
            let base = format!("(Unit.base #\"{name}\")");
            if exp == 1 {
                factors.push(base);
            } else {
                factors.push(format!("(Unit.^ {base} {exp})"));
            }
        }
        // Left-nested product of the factors (a single factor is itself).
        let mut it = factors.into_iter();
        let mut acc = it.next().unwrap();
        for f in it {
            acc = format!("(Unit.* {acc} {f})");
        }
        acc
    }

    /// Render the unit as the canonical VALUE-form s-expression cdz-run prints inside a `(Qty.of <mag>
    /// <unit>)` result — byte-for-byte identical to what [`crate::lower`]'s `unit_value_ast` bakes (that
    /// builds an arena tree; this builds its text). Every member-access key is SUGARED to its bare dotted
    /// name (`Qty.of`/`Unit.base`/`Unit.one` — a name atom the printer emits verbatim, matching the
    /// operator-symbol members `Unit.*`/`Unit./`/`Unit.^`; seq-283 consistency fix — before it, only the
    /// operator-symbol members sugared while the identifier members rendered the unsugared `(. Unit base)`).
    /// It differs from [`render`] (the TYPE-annotation surface) in one way the corpus records demand:
    ///   - a unit with NEGATIVE exponents renders as a QUOTIENT `(Unit./ <numerator> <denominator>)` — the
    ///     velocity surface `(Unit./ meter second)`, NOT `render`'s `(Unit.* meter (Unit.^ second -1))`.
    ///
    /// The numerator is the positive-exponent factors as a left-nested `(Unit.* …)` product (`Unit.one`
    /// if none); the denominator the negative-exponent factors with exponents made positive. This is the note
    /// the Rust-backend gate splices verbatim, so it MUST track `unit_value_ast` exactly.
    pub fn render_value_form(&self) -> String {
        // Escape a base NAME for embedding in a `#"…"` symbol literal — the SAME closed set the canonical
        // printer's `literal::escape_string` uses (`\n \t \r \\ \"`), so the rendered symbol re-reads to the
        // same name AND stays a single-line token. `Unit.base` carries a RAW string (no bare-safe-subset
        // restriction — a base name may hold a quote, backslash, or control char), so an unescaped `name`
        // would emit an invalid s-expr AND could split the emitted `// cdz-unit[…]` note across lines,
        // breaking the Rust gate harness's string-literal splice (which the harness ALSO defends with the
        // same escape). `cadenza-syntax`'s `escape_string` is the authority but is a DEV-only dep here, so
        // inline the identical closed set. (A char OUTSIDE this set — a control char other than \n\t\r —
        // passes through verbatim, exactly as `escape_string` leaves it: the reader accepts it inside `#"…"`,
        // and the note is still one line since only \n/\r break a line.)
        fn escape_sym(name: &str) -> String {
            let mut out = String::with_capacity(name.len());
            for c in name.chars() {
                match c {
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    _ => out.push(c),
                }
            }
            out
        }
        // A single base at a (positive) exponent: `(Unit.base #"name")` or `(Unit.^ … k)`.
        fn factor(name: &str, exp: i64) -> String {
            let base = format!("(Unit.base #\"{}\")", escape_sym(name));
            if exp == 1 {
                base
            } else {
                format!("(Unit.^ {base} {exp})")
            }
        }
        // Left-nested product of a factor list, or `(. Unit one)` when empty.
        fn product(factors: &[(&String, i64)]) -> String {
            let Some((first, rest)) = factors.split_first() else {
                return "Unit.one".to_string();
            };
            let mut acc = factor(first.0, first.1);
            for (name, exp) in rest {
                acc = format!("(Unit.* {acc} {})", factor(name, *exp));
            }
            acc
        }
        // Split into positive (numerator) and negative (denominator, exponents made positive) factors, in
        // the `BTreeMap`'s sorted base-name order (deterministic, matching `unit_value_ast`'s `entries()`).
        let num: Vec<(&String, i64)> = self
            .exp
            .iter()
            .filter(|(_, e)| **e > 0)
            .map(|(n, &e)| (n, e))
            .collect();
        let den: Vec<(&String, i64)> = self
            .exp
            .iter()
            .filter(|(_, e)| **e < 0)
            .map(|(n, &e)| (n, -e))
            .collect();
        if den.is_empty() {
            // All positive (or empty → `(. Unit one)`) — a plain product / single factor / identity.
            return product(&num);
        }
        // A quotient `(Unit./ numerator denominator)` — the derived-unit surface.
        format!("(Unit./ {} {})", product(&num), product(&den))
    }

    /// A compact, HUMAN-readable rendering of the unit for a DIAGNOSTIC — the base names joined by `·`
    /// with `^n` exponents (`meter`, `meter^2`, `meter·second^-1`), and `dimensionless` for the empty
    /// unit. Unlike [`render`] (the round-tripping `(Unit.base …)` surface form), this is for a message
    /// where naming just the units — not the whole `(Qty T <unit>)` type — is what the reader needs.
    /// The `exp` map is a `BTreeMap`, so iteration is in sorted base-name order → deterministic.
    pub fn render_human(&self) -> String {
        if self.exp.is_empty() {
            return "dimensionless".to_string();
        }
        self.exp
            .iter()
            .map(|(name, &exp)| {
                if exp == 1 {
                    name.clone()
                } else {
                    format!("{name}^{exp}")
                }
            })
            .collect::<Vec<_>>()
            .join("·")
    }
}

/// Normalize a machine-integer ratio `num/den` to canonical form — lowest terms, denominator strictly
/// positive — the form a `Unit`'s scale is kept in (so two units of equal scale compare equal by field).
/// `None` if `den == 0` (no valid scale) or the sign flip of `i128::MIN` overflows. Shared by `scaled`/
/// `mul`/`div`/`pow`; a scale is always a well-defined positive-denominator ratio of small integers.
fn normalize_ratio(mut num: i128, mut den: i128) -> Option<(i128, i128)> {
    if den == 0 {
        return None;
    }
    if den < 0 {
        num = num.checked_neg()?;
        den = den.checked_neg()?;
    }
    let g = gcd_i128(num.unsigned_abs(), den.unsigned_abs());
    if g > 1 {
        num /= g as i128;
        den /= g as i128;
    }
    Some((num, den))
}

/// The greatest common divisor of two unsigned magnitudes (Euclid) — reduces a scale ratio to lowest
/// terms. `gcd(0, n) = n`.
fn gcd_i128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// A solved type — the closed variant set inference determines and every pass below reads.
///
/// `PartialEq` is HAND-WRITTEN (not derived) for ONE reason: a [`Ty::Nominal`]'s identity is its
/// `decl + args` (its fully-qualified name + instantiation, `type-system.md §A Nominal Type's Identity
/// Is Its Fully-Qualified Name`), NOT its structural `inner`. A RECURSIVE nominal's `inner` DIVERGES by
/// derivation path — the folded (annotation) path collapses the recursion to a `Ty::Sum{decl}` back-edge
/// while the unfolded (value) path holds a `Ty::Nominal{decl}` there — so comparing `inner` structurally
/// would make `Lst != Lst`. Comparing by `decl + args` makes them equal (the μ-type equality the recursion
/// needs) AND keeps generic instantiations distinct (`Box Int64 != Box Bool`, args differ). `inner` is a
/// machine-representation HINT (depth-1 shape for `valtype_of`/`box_op_ty`, which take `&Ty` and cannot
/// reach `Db`); it is derived from `decl + args` and NEVER compared. (`Ty` is never hashed / never used as
/// a map key — Phase 0 audit — so this custom eq carries no `Hash`-consistency obligation.)
#[derive(Clone, Debug)]
pub enum Ty {
    /// An integer of a given signedness and a possibly-deferred width.
    Int(IntTy),
    /// The boolean.
    Bool,
    /// The unit value: no information, no runtime slot.
    Unit,
    /// A record: a fixed SET of named fields, each with its own type. Held as a canonically-ordered
    /// `BTreeMap` so two records of the same field-set are the SAME type regardless of the order the
    /// fields were written (`core-semantics.md` §A Record Has A Fixed Set Of Named Fields), and a
    /// field's type is looked up by name in O(log n). Wrapped in an `Rc` so CLONING a `Ty::Record`
    /// (which `type_of` does on every memo read) is a refcount bump, not a deep map copy — a wide
    /// record read field-by-field was O(N²) in map clone. Faithful to the Cadenza port target, which
    /// is ref-counted throughout (a shared immutable value = one refcounted allocation).
    ///
    /// Record / [`Tuple`](Ty::Tuple) / [`Sum`](Ty::Sum) are THE structural types — a record of named
    /// fields, a tuple of positional elements, a sum of named variants. A STRUCTURAL type's identity is
    /// its SHAPE (its constituents in their defining positions), so two are equal exactly when those
    /// coincide — the hand-written `PartialEq` compares a record by its field-name→type map, a tuple by
    /// its ordered element types, a sum by `decl` (structural sums share one synthesized decl per shape).
    //= spec/capabilities/type-system.md#the-structural-types-are-record-tuple-and-sum
    //# A program MUST be able to form a structural type — a record of named fields, a tuple of positional elements, or a sum of named variants — whose identity is its shape, equal to any type of the same shape.
    //= spec/capabilities/type-system.md#the-structural-types-are-record-tuple-and-sum
    //# A structural type's shape MUST be its constituent types in their defining positions — a record's field names with their types, a tuple's element types in order, a sum's variant names with their payload types — so that two structural types are equal exactly when those constituents coincide.
    Record(std::rc::Rc<std::collections::BTreeMap<crate::resolved::Symbol, Ty>>),
    /// A tuple: a fixed-ARITY POSITIONAL product, each position with its own type. The arity AND the
    /// per-position types ARE the type — a tuple of a different arity, or with a differently-typed
    /// position, is a DIFFERENT type (`type-system.md` §The Structural Types Are Record, Tuple, And
    /// Sum). Distinct from `Record` (a set of NAMED fields): a tuple is projected by position. Held
    /// behind an `Rc<[Ty]>` (an immutable shared slice) so cloning a `Ty::Tuple` is a refcount bump,
    /// not a deep element copy: an N-element tuple type read once per projection (N projections off one
    /// shared tuple) was O(N²) in `Vec<Ty>` clone. Indexing/iterating are unchanged (it derefs to a
    /// slice); only construction differs (`.collect::<Vec<_>>().into()` or `vec![…].into()`).
    Tuple(std::rc::Rc<[Ty]>),
    /// A LIST: a homogeneous, variable-length sequence of one ELEMENT type. Unlike a tuple (fixed arity,
    /// per-position type), a list's length is a runtime property and every element shares ONE type — so
    /// `(list 1 true)` (mixed) is ill-typed (the elements do not unify) and `(list)` is a list of a
    /// deferred element type. `List Int64` and `List Bool` are distinct; the element type is held behind a
    /// `Box` (one type, unlike the tuple's slice). Backed at run time by the persistent `vec-*` RRB heap
    /// ops — an internal representation the language holds behind opaque handles, unobservable to the
    /// program (a list is only ever an i32 handle, inspected through the runtime's `vec-*` ops).
    //= spec/capabilities/collections-and-text.md#a-list-is-an-ordered-homogeneous-sequence
    //# A list MUST be an ordered sequence whose elements share one type.
    //= spec/capabilities/collections-and-text.md#a-list-is-an-ordered-homogeneous-sequence
    //# Two lists MUST be equal exactly when they have equal elements in the same order.
    //= spec/capabilities/collections-and-text.md#a-list-s-representation-is-unspecified-and-unobservable
    //# A conforming implementation MAY back a list with any internal representation — a contiguous array, a persistent tree, or a structure it selects and changes by size or usage — and MUST NOT let that choice be observable, so that two lists with equal elements in the same order are indistinguishable by every operation, including equality, length, indexing, and the list's canonical byte form, regardless of how each is stored.
    List(Box<Ty>),
    /// A MAP: a persistent association of KEYS of one type with VALUES of one type
    /// (`collections-and-text.md` §A Map Associates Keys With Values). PARAMETRIC in two types — the
    /// key type and the value type — held as two `Box`es (unlike `List`'s single element type). The
    /// map's KEY SET is runtime data, NOT part of its type: two maps with different keys but the same
    /// `(key, value)` types are the SAME type `Map<K,V>` (the crucial contrast with a `Record`, whose
    /// field-name SET IS its shape). So comparing two maps with different keys is well-typed (yields
    /// `false`), and a `List (Map K V)` whose elements have different key sets is homogeneous. `Map
    /// Int64 Int64` and `Map Int64 Bool` are distinct. Backed at run time by the persistent CHAMP
    /// `map-*` heap ops; keys are compared by VALUE under structural equality, and its canonical form
    /// renders entries in sorted key order. The CHAMP holds each key at most once (an add-or-replace,
    /// never a second entry), so a map is a key→value association with unique keys.
    //= spec/capabilities/collections-and-text.md#a-map-associates-keys-with-values
    //# A map MUST associate keys of one type with values of one type.
    //= spec/capabilities/collections-and-text.md#a-map-associates-keys-with-values
    //# A map MUST contain each key at most once.
    //= spec/capabilities/collections-and-text.md#a-map-associates-keys-with-values
    //# Two maps MUST be equal exactly when they associate the same keys with equal values, independent of insertion order.
    //= spec/capabilities/collections-and-text.md#keys-are-compared-by-value-not-representation
    //# Whether a map contains a key, and which entry a lookup or removal names, MUST be decided by the key's value under *core-semantics.md §Equality Is Structural* — two keys that are equal as values name the same entry regardless of how each was constructed or stored.
    // The CHAMP hash and the internal node placement it uses are NEVER exposed: the observable surface is
    // only the value-based ops (contains/lookup/insert/remove — membership + association), `size`, value
    // equality, and the canonical SORTED-KEY iteration (the deterministic order). No op reveals a key's
    // hash or its position in the trie, so a program can neither observe nor depend on the placement.
    //= spec/capabilities/collections-and-text.md#keys-are-compared-by-value-not-representation
    //# A map therefore MUST NOT expose or depend on any hashing, ordering, or internal placement of its keys as observable behavior; only membership, association, size, equality, and the deterministic iteration order below are observable.
    //= spec/capabilities/core-semantics.md#a-record-has-a-fixed-set-of-named-fields
    //# A map MUST associate keys with values as a dynamic homogeneous collection whose set of keys is not fixed by the value's form, distinct from a record's fixed field set.
    Map(Box<Ty>, Box<Ty>),
    /// A SET: a persistent UNORDERED collection of UNIQUE ELEMENTS of one type (`collections-and-text.md`
    /// §A Set Is A Collection Of Unique Elements). PARAMETRIC in ONE element type (like `List`, unlike
    /// `Map`'s two). The set's element SET is runtime data, NOT part of its type: two sets with different
    /// elements but the same element type are the SAME type `Set<T>` (the map analogue — the element set
    /// is a runtime collection, not a shape), so comparing them is well-typed (yields `false`). `Set
    /// Int64` and `Set Bool` are distinct. Backed at run time by the persistent CHAMP `set-*` heap ops
    /// (CHAMP-minus-value-column); elements are compared by VALUE under structural equality, and its
    /// canonical form renders `(Set.of (list …))` with elements in sorted order. The CHAMP holds each
    /// element at most once, so a set is a collection of UNIQUE elements of one type.
    //= spec/capabilities/collections-and-text.md#a-set-is-a-collection-of-unique-elements
    //# A set MUST be a collection of elements of one type.
    //= spec/capabilities/collections-and-text.md#a-set-is-a-collection-of-unique-elements
    //# A set MUST contain each element at most once.
    //= spec/capabilities/collections-and-text.md#a-set-is-a-collection-of-unique-elements
    //# Two sets MUST be equal exactly when they contain equal elements, independent of insertion order.
    Set(Box<Ty>),
    /// A BYTES sequence: a homogeneous, variable-length sequence of BYTES (`collections-and-text.md` §A
    /// Byte Sequence Is A Sequence Of Bytes). NOT parametric — a byte is a byte, so unlike `List` it
    /// carries no element type (a LEAF in the type universe). Distinct from `List Int64`: a `Bytes` packs
    /// its bytes and renders `b"…"` (the byte-string form), whereas a list renders `(list …)`; the
    /// compiler keeps them separate types even though a byte is an integer. Backed at run time by the
    /// persistent rope `bytes-*` heap ops; its observable form is the byte-string literal.
    Bytes,
    /// A STRING: an immutable sequence of Unicode text — a sequence of Unicode SCALAR VALUES, so its
    /// contents are independent of any byte encoding (the UTF-8 rope below is a representation, not the
    /// value). One monomorphic type (no element parameter, unlike `List`) — every string is `String`.
    /// Backed at run time by the same UTF-8 byte-rope the value heap uses for `Bytes`; only the STATIC
    /// type differs (a `String` renders `"…"` with the closed escape set, a `Bytes` renders `\xNN`). This
    /// increment realizes the CONSTANT string (a literal folds + equality); runtime string ops
    /// (`concat`/`len`/`at`) + string escape arrive later.
    //= spec/capabilities/collections-and-text.md#a-string-is-a-sequence-of-unicode-scalar-values
    //# A string MUST be a sequence of Unicode scalar values, so that its contents are independent of any byte encoding.
    String,
    /// A CHAR: a single Unicode scalar value — the element type of a string's scalar sequence. One
    /// monomorphic LEAF type (no parameter). Its value is its scalar, so equality is scalar equality and
    /// its order is the numeric order of its scalar value. A `#\a` literal is a `Char` constant;
    /// `Char.to-int`/`from-int` convert to/from `Int64` totally. This increment realizes the CONSTANT
    /// char (literal + equality/ordering). A `Char` is backed by Rust's `char`, which admits ONLY a valid
    /// scalar (never a surrogate), so it can never hold a non-scalar value.
    //= spec/capabilities/collections-and-text.md#a-char-is-a-single-unicode-scalar-value
    //# A char MUST be a single Unicode scalar value — a code point in the range `U+0000..=U+10FFFF` excluding the surrogate range `U+D800..=U+DFFF` — so that the element type of a string's scalar sequence is exactly a char and a char can never hold a value that is not a scalar.
    //= spec/capabilities/collections-and-text.md#a-char-is-a-single-unicode-scalar-value
    //# A char's ordering MUST be the numeric order of its scalar value, so that a char order and the string order defined on scalar values agree by construction.
    Char,
    /// A SYMBOL: an interned NAME value with O(1) equality (`options/symbol-interning/`; 17-symbols) — a
    /// NOMINAL wrapper over a `String`. `(Symbol.of s)` maps a String to a Symbol, and two Symbols are
    /// equal exactly when their underlying strings are equal (String equality lifted through the Symbol
    /// tag). A monomorphic LEAF type (no parameter, like `String`/`Char`); its IDENTITY is content-derived
    /// — a deterministic function of its text, NEVER allocation order (deterministic-value-form.md §A Value
    /// Has One Canonical Byte Form) — so interning is a pure representation optimization the runtime MAY
    /// perform invisibly. A `Symbol` never unifies with the `String` it wraps (a distinct `Ty` variant, so
    /// a String cannot be passed where a Symbol is required nor vice versa — the nominal boundary). This
    /// increment realizes the CONSTANT symbol: `Symbol.of`/`to-string` fold, and equality reuses the
    /// underlying string's constant equality (a constant symbol shares the `Core::ConstStr` rep; only the
    /// static type differs). The runtime symbol handle + `#"…"` reader-sugar equivalence arrive later.
    Symbol,
    /// An ARBITRARY-PRECISION signed integer — `BigInt`, of UNBOUNDED range (`numeric-model.md`
    /// §Arbitrary Precision; `options/numeric-model/explicit-checked.md` §Arbitrary-precision integer).
    /// A monomorphic LEAF type (no width parameter, unlike [`Ty::Int`]) — it is the signed, unbounded
    /// companion of the fixed-width family, opted into explicitly. It NEVER overflows or wraps: an
    /// arithmetic operation grows its representation as the result requires. It is a DISTINCT numeric
    /// type — `agrees_with` is true only `BigInt`↔`BigInt`, so a `BigInt`/fixed-width mix is a mismatch
    /// (CDZ0301) with no silent promotion, exactly as `Int64`/`Float64` is. A constant folds in the
    /// compiler (reusing `num-bigint`, already the backing of `ast::IntValue`); a runtime-valued `BigInt`
    /// is a sign-magnitude limb-array heap leaf (the `Bytes`-leaf shape — raw bytes, zero handles). Its
    /// boundary representation is a `list<u8>` in the pinned two's-complement encoding, so it MAY cross an
    /// exported signature. See `implementation/design/DESIGN-bigint-and-rational-rcdzc.md`. B0 adds the type
    /// through the closed universe (byte-neutral — nothing constructs one yet).
    BigInt,
    /// An EXACT RATIONAL number — `Rational`, a normalized pair of arbitrary-precision integers
    /// (`numeric-model.md` §Exact Arithmetic Is Exact; `options/numeric-model/explicit-checked.md` §Exact
    /// rational). A monomorphic LEAF type (no parameter) kept in a canonical NORMALIZED form: lowest terms
    /// (numerator and denominator share no common factor via `gcd`), the sign carried on the numerator,
    /// the denominator strictly positive. So `2/4` and `1/2` are ONE value with one canonical byte form,
    /// and equality is structural over the normalized pair. It is a DISTINCT numeric type — `agrees_with`
    /// is true only `Rational`↔`Rational`, so a `Rational`/integer mix is a mismatch (CDZ0301) with no
    /// silent promotion (crossing in is explicit via `Rational.of-int`), exactly as `BigInt`/`Int64` is. A
    /// zero denominator has no value (`Rational.of _ 0` traps). A constant folds in the compiler (the
    /// normalized `num`/`den` pair over `ast::IntValue` bignum arithmetic); a runtime-valued `Rational` is
    /// a two-`BigInt`-child compound, boundary-encoded as `record { numerator: list<u8>, denominator:
    /// list<u8> }`. It is one admissible inner numeric `T` for `(Qty T u)`, so a dimensioned exact
    /// quantity `(Qty Rational u)` composes the two orthogonally. See the design doc. B4-0 adds the type
    /// through the closed universe (byte-neutral — nothing constructs one yet).
    Rational,
    /// A FLOATING-POINT number, indexed by its bit WIDTH (`numeric-model.md` §A Floating-Point Type Is
    /// Indexed By A Compile-Time Width) — the float analogue of [`Ty::Int`], carrying a [`FloatTy`]
    /// (a possibly-deferred [`FloatWidth`]) rather than a fixed name. `Float64` is the signed 64-bit
    /// (binary64) instance a bare float literal grounds to; `Float32` is `(Float 32)`. Two float types
    /// of different width are DISTINCT (no silent promotion), and an integer never unifies with a float
    /// (`(+ 2 2.0)` → CDZ0301). Backed at the boundary by the component-model `f64`/`f32`.
    Float(FloatTy),
    /// A QUANTITY: an underlying numeric value of type `inner` carried with a COMPILE-TIME `unit`
    /// (`units-of-measure.md`; `options/units-of-measure/erased-compile-time-quantity.md`). `(Qty T u)`
    /// is an ordinary type-constructor application exactly as `(Int N)` is the integer constructor
    /// applied to a compile-time width — a quantity type is the same shape indexed by a compile-time
    /// UNIT instead of a natural. The unit tracks the DIMENSION and gates compatibility (`+`/`-`/
    /// comparison require equal units; `*`/`/` compose them by the free-abelian-group operation); the
    /// numeric core is untouched — `inner` keeps its width/overflow/no-promotion rules and the running
    /// arithmetic is the plain `inner` operation. Two quantities are the SAME type iff their `inner`
    /// types unify AND their units are EQUAL (a meter never unifies with a bare number nor with a
    /// second). The whole apparatus is CHECKED THEN ERASED before emission (`§Dimensions Are Checked
    /// Then Erased`): a `Ty::Qty` lowers to its `inner` (`lower` strips it), so it NEVER reaches the
    /// backend and `(Qty.of 5.0 meter)` is byte-identical to the bare `5.0`. The one piece of earlier
    /// Cadenza's identity that survives the clean room, buying verified dimensional correctness for zero
    /// runtime cost.
    Qty { inner: Box<Ty>, unit: Unit },
    /// A SUM: a value of one of a fixed set of named variants (`type-system.md` §The Structural Types
    /// Are Record, Tuple, And Sum — "a sum of named variants"). Declared by `(type NAME variant…)`,
    /// which tags it NOMINAL (`§Nominal Is An Orthogonal Modifier Over Any Structural Type`), so its
    /// identity is its fully-qualified NAME — "the module path in which it is declared together with
    /// its declared name" (`§A Nominal Type's Identity Is Its Fully-Qualified Name`) — NOT its shape.
    ///
    /// We realize that FQN identity as the DECLARATION's arena occurrence `decl` (the `TypeDecl.occ`),
    /// not the local name string. Two `(type Foo …)` declared in different modules are DISTINCT AST
    /// nodes, so they carry distinct `decl` ids and are distinct types (`§160` — distinct whenever
    /// their FQNs differ, even with identical structure and identical local name `Foo`). This is also
    /// IMPORT-SAFE by construction: package linking splices each file's arena into one `Db`, so every
    /// imported declaration keeps its own `StructId` — the "A's Foo ≠ B's Foo" property survives with
    /// no module-path plumbing. (It is the columns-model realization of the seed's `Rc::ptr_eq`
    /// identity — physical declaration identity, expressed as the node id everything is already keyed
    /// by.) `name` is the declared LOCAL name, carried for rendering (`(: (Some 5) Option)`) only;
    /// `decl` alone decides equality (a `decl` determines its `name`).
    ///
    /// The variant SET with its payload types — the shape a match's exhaustiveness (`§190`) and a
    /// constructor's payload check read — lives in `db.type_decls` (found by `decl`), NOT here, which
    /// also keeps `Ty` FINITE for a recursive sum (`(type Expr (Lit Int64) (Neg Expr))` — an eager
    /// payload map mentioning the sum itself would be an infinite type). The runtime representation is
    /// the heap sum handle (`sum-new`/`sum-disc`/`sum-payload`); the nominal tag is compile-time only
    /// (`§156` — it "adds nothing to the value's runtime representation").
    ///
    /// `args` are the sum's TYPE ARGUMENTS — the concrete types its (implicit) type parameters are
    /// instantiated at (`type-system.md §Generics Are Type-Valued Parameters`). A MONOMORPHIC sum
    /// (`(type Sign Neg Zero Pos)` — no free type variable in any payload) has `args: []`. A GENERIC sum
    /// `(type Option (Some a) None)` at a concrete instantiation carries them: `Option Int64` is `Sum {
    /// decl: <Option>, args: [Int64] }`. Two sums are the SAME type iff their `decl` AND
    /// their `args` agree, so `Option Int64 ≠ Option Bool` (same declaration, different instantiation) —
    /// the payload-type discrimination `type-system.md §the head Option agrees but the payload does not`
    /// requires. Args are positional, in the parameters' first-appearance order.
    Sum {
        decl: crate::ast::StructId,
        // `Rc<[Ty]>` (an immutable shared slice), NOT `Vec<Ty>`: a sum's type-arguments are cloned on
        // every `Ty::clone` (which `type_of`/`unify`/`subst.apply` do constantly), and a GENERIC sum
        // NESTED in another (`(Option (Option … Int64))`) held its inner sum in `args`, so a `Vec<Ty>`
        // clone DEEP-COPIED the whole nesting at every level — an O(depth) copy done O(depth) times
        // (`ty_at_path`/`payload_ty_at_instantiation` per match level) → O(depth³) (a deep-nested-Option
        // match: depth 800 = 610ms, ~3.9×/dbl). `Rc<[Ty]>` makes the clone a refcount bump, sharing the
        // nesting across levels — the SAME fix `Tuple`/`Record` (fix-33/50) and `Nominal.inner` (fix-53)
        // already carry. Indexing/iterating are unchanged (it derefs to a slice); only construction
        // differs (`.collect()`/`vec![…].into()`/`.into()`). Identity is still `decl + args` (the
        // hand-written `PartialEq` compares the slices elementwise).
        args: std::rc::Rc<[Ty]>,
    },
    /// A NOMINAL type — an underlying structural type `inner` tagged with a compile-time NAME
    /// (`type-system.md §Nominal Is An Orthogonal Modifier Over Any Structural Type`). Realized as the
    /// erased form of a SINGLE-VARIANT sum: `(type UserId (Mk Int64))` is a nominal Int64, `(type Point
    /// (Mk Int64 Int64))` a nominal `(Tuple Int64 Int64)`, `(type Marker (The))` a nominal `Unit`. The
    /// tag "adds nothing to the value's runtime representation" (§156) — at run time a `Ty::Nominal`
    /// value IS its `inner` value (no `sum-new` box, no discriminant), so its machine representation
    /// (`valtype_of`, boundary encode/decode) reads THROUGH `inner`.
    ///
    /// **Identity is `decl + args`; `inner` is a machine-rep hint.** `decl` is the nominal FQN identity
    /// (the declaration occurrence, exactly like `Ty::Sum::decl`) and `args` its type-argument
    /// instantiation (exactly like `Ty::Sum::args`): two `Nominal` types are the SAME iff their `decl` AND
    /// `args` match (the hand-written `PartialEq` / `agrees_with` / `unify`). So `UserId` never unifies
    /// with the bare `Int64` it wraps (a different `Ty` variant ⇒ mismatch, §Nominal Types Are Not
    /// Comparable Across Their Boundary); two same-shape declarations `(type A (Mk Int64))` / `(type B
    /// (Mk Int64))` are DISTINCT (distinct `decl`s); and a generic `Box Int64 != Box Bool` (same `decl`,
    /// different `args`). The declared LOCAL name is recovered from `decl` at render time (via
    /// `NameCtx`/`db.type_decl_by_occ`), no longer carried on the type — it was redundant render-only state.
    ///
    /// `inner` is the erased UNDERLYING type — the machine representation `valtype_of`/`box_op_ty`/field
    /// access read THROUGH (it takes `&Ty`, cannot reach `Db`). It is DERIVED from `decl + args`
    /// (`Db::normalize_sum` substitutes `args` into the stored template) and NEVER compared — because a
    /// RECURSIVE nominal's `inner` diverges by derivation path (folded `Ty::Sum{decl}` back-edge vs
    /// unfolded `Ty::Nominal{decl}`), and comparing it would make `Lst != Lst`. Excluding it from equality
    /// is what lets a recursive newtype erase (the μ-type equality problem dissolves).
    //= spec/capabilities/type-system.md#nominal-is-an-orthogonal-modifier-over-any-structural-type
    //# A program MUST be able to declare a nominal type by tagging any structural type — record, tuple, or sum — with a name, so that nominal-versus-structural is one orthogonal choice available over every structural type rather than a property of one kind of type.
    //= spec/capabilities/type-system.md#nominal-is-an-orthogonal-modifier-over-any-structural-type
    //# A nominal type MUST be represented as its underlying structural value together with a compile-time tag naming the type, so that a nominal type and its underlying structural type are one runtime mechanism and the tag adds nothing to the value's runtime representation.
    //= spec/capabilities/type-system.md#nominal-is-an-orthogonal-modifier-over-any-structural-type
    //# A nominal type's identity MUST be its fully-qualified name — the module path in which it is declared together with its declared name — so that its identity is unique across the whole program and does not depend on its shape.
    //= spec/capabilities/type-system.md#nominal-is-an-orthogonal-modifier-over-any-structural-type
    //# Two nominal types MUST be distinct whenever their fully-qualified names differ, even when their underlying structures and their declared local names are identical, so that a module cannot forge a value of another module's nominal type by re-declaring a same-shape same-name type.
    Nominal {
        decl: crate::ast::StructId,
        // `Rc<[Ty]>` (not `Vec<Ty>`), the sibling of `Ty::Sum::args` — a nominal's type-arguments are
        // cloned on every `Ty::clone` and a nested generic nominal held its child in `args`, so a
        // `Vec<Ty>` clone deep-copied the nesting per level; `Rc<[Ty]>` shares it (a refcount bump).
        args: std::rc::Rc<[Ty]>,
        // `Rc`, not `Box`: `inner` is the derived machine-rep template with `args` substituted, and for a
        // NESTED generic nominal (`(Box (Box … Int64))`) the child nominal is stored BOTH here and in
        // `args` — with a `Box`, each level deep-CLONED the child into `inner`, so the materialized `Ty`
        // doubled per nesting level = O(2^depth) (a depth-20 annotation hung the compiler). `Rc` SHARES the
        // child's allocation across `args`/`inner` and across levels, so a deep nesting is O(depth) nodes,
        // not O(2^depth). Behaviour-identical — `inner` is a hint that is never compared (only read for
        // layout/valtype), and its logical structure is unchanged; only the pointer is now shared.
        inner: std::rc::Rc<Ty>,
    },
    /// A function type `param → result`, curried (a multi-parameter operation is nested `Fn`s). What
    /// an operator's (and later a function's) `Meta.t` denotes; an application unifies the argument
    /// against `param` and takes `result`.
    Fn(Box<Ty>, Box<Ty>),
    /// The type of a type VALUE — the type of `Bool`, of `(Int 64)`, of the result of `(-> A B)`.
    /// Because a type is a first-class value, it has a type, and that type is `Type`. It is
    /// compile-time-only (a value of type `Type` is erased before the runtime boundary, like any
    /// type-value). A type is an ORDINARY value — bound to a name (`Int64`, a prelude type ctor), passed,
    /// and returned — so the language needs no separate term-and-type syntax; and its type being `Type`
    /// makes the kind level itself a type in the system rather than an untyped meta-level.
    //= spec/capabilities/type-system.md#types-are-first-class-values-whose-type-is-the-type-of-types
    //# A type MUST be expressible as an ordinary first-class value that can be bound, passed, and returned, so that the language needs no separate term-and-type syntax to name a type.
    //= spec/capabilities/type-system.md#types-are-first-class-values-whose-type-is-the-type-of-types
    //# The type of a type-value MUST be the type of types, so that the kind level is itself a type in the system rather than an untyped meta-level.
    Type,
    /// A unification variable — an as-yet-unsolved type inference introduces (e.g. a fresh operand
    /// type before it is constrained). Resolved to a concrete type by the Hindley-Milner solve in
    /// [`crate::unify`]; a variable that survives to the boundary is an undetermined type (a rejection,
    /// not a default). An operator's scheme carries one variable per generic axis so an application
    /// unifies its operands rather than hard-coding a type.
    Var(u32),
    /// The type of a node the compiler could not type — a poison's type. It is COMPATIBLE with every
    /// type, so a "no" never induces a spurious mismatch upward (the poison itself is the reported
    /// fault, not a type error it would otherwise cascade). Behaves as a top type in `agrees_with`
    /// and `unify`.
    Any,
    /// A reified DELIMITED CONTINUATION — the type of a `k` captured by a general (`ctl`-style) handler
    /// arm and used as a FIRST-CLASS VALUE (stored in a collection, passed to another function, resumed
    /// from a different activation). `Cont { resume, answer }`: applying it (`(k v)`, `resume` = `apply`)
    /// takes a `resume` value and runs the delimited region to an `answer` (the handler's answer type).
    /// This is the E5 STEP-3 type (general/stored continuations — the DES scheduler's `sleep` stores a
    /// `Cont`, resumes it later). RUNTIME REP: a defunctionalized frame-chain HANDLE — an ordinary
    /// value-heap value (so it stores + resumes later), `core_valtype` = the heap-handle `I32`. RESERVED
    /// here (increment 1, gate-neutral): the variant EXISTS + every exhaustive `Ty` match has an arm, but
    /// nothing constructs it yet (a general escaping-k arm still declines to lower) — the frame reification
    /// + `apply` dispatcher land in later increments (`DESIGN-general-continuations-e5.md` §8).
    Cont { resume: Box<Ty>, answer: Box<Ty> },
}

/// Structural equality, HAND-WRITTEN so a [`Ty::Nominal`] compares by `decl + args` (its identity), NOT
/// its `inner` — see the `Ty::Nominal` doc. Every other variant compares exactly as a derive would
/// (all fields). `Eq` is a marker (the relation is reflexive/symmetric/transitive: a nominal's
/// `decl + args` equality is, and every other arm is derive-equivalent).
impl PartialEq for Ty {
    fn eq(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Int(a), Ty::Int(b)) => a == b,
            (Ty::Bool, Ty::Bool)
            | (Ty::Unit, Ty::Unit)
            | (Ty::Bytes, Ty::Bytes)
            | (Ty::String, Ty::String)
            | (Ty::Char, Ty::Char)
            | (Ty::Symbol, Ty::Symbol)
            | (Ty::BigInt, Ty::BigInt)
            | (Ty::Rational, Ty::Rational)
            | (Ty::Type, Ty::Type)
            | (Ty::Any, Ty::Any) => true,
            (Ty::Float(a), Ty::Float(b)) => a == b,
            (Ty::Record(a), Ty::Record(b)) => a == b,
            (Ty::Tuple(a), Ty::Tuple(b)) => a == b,
            (Ty::List(a), Ty::List(b)) => a == b,
            (Ty::Set(a), Ty::Set(b)) => a == b,
            (Ty::Map(ak, av), Ty::Map(bk, bv)) => ak == bk && av == bv,
            (
                Ty::Sum {
                    decl: da, args: aa, ..
                },
                Ty::Sum {
                    decl: db, args: ab, ..
                },
            ) => da == db && aa == ab,
            // A NOMINAL compares by `decl + args` ONLY — NOT `inner` (which diverges by derivation path
            // for a recursive nominal; comparing it would make `Lst != Lst`). `name` is render-only.
            (
                Ty::Nominal {
                    decl: da, args: aa, ..
                },
                Ty::Nominal {
                    decl: db, args: ab, ..
                },
            ) => da == db && aa == ab,
            (Ty::Fn(ap, ar), Ty::Fn(bp, br)) => ap == bp && ar == br,
            (
                Ty::Cont {
                    resume: ar,
                    answer: aa,
                },
                Ty::Cont {
                    resume: br,
                    answer: ba,
                },
            ) => ar == br && aa == ba,
            (
                Ty::Qty {
                    inner: ai,
                    unit: au,
                },
                Ty::Qty {
                    inner: bi,
                    unit: bu,
                },
            ) => ai == bi && au == bu,
            (Ty::Var(a), Ty::Var(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Ty {}

/// A render-time NAME resolver for `Ty::render_name` and the diagnostic/encode render paths. A
/// `Ty::Sum`/`Ty::Nominal` carries only its declaration occurrence `decl` (identity = `decl + args`; the
/// spelled name is redundant render-only state). `NameCtx` maps `decl → name`, built ONCE from
/// `db.type_decls` at a render entry point and threaded by shared reference down `render_name`'s
/// recursion — so `Ty`/`unify`/`Subst::apply` stay db-free (the cheap-clone audit's point). Introduced
/// ahead of dropping the redundant `name` field; this sub-increment threads it but the render still reads
/// the existing field (a green no-op).
#[derive(Clone, Copy)]
pub struct NameCtx<'a> {
    decls: &'a [crate::db::TypeDecl],
}

impl<'a> NameCtx<'a> {
    pub fn new(decls: &'a [crate::db::TypeDecl]) -> NameCtx<'a> {
        NameCtx { decls }
    }
    /// The declared name of the type whose declaration occurrence is `decl`, or `None` for a
    /// synthesized/unresolved occurrence.
    pub fn name_of(&self, decl: crate::ast::StructId) -> Option<&'a str> {
        self.decls
            .iter()
            .find(|d| d.occ == decl)
            .map(|d| d.name.as_str())
    }
}

impl Ty {
    /// A fresh integer-literal type: signed, width deferred. Inference or the backend fixes the width.
    pub fn int() -> Ty {
        Ty::Int(IntTy::deferred())
    }

    /// The signed 64-bit integer type (`Int64`) — an integer literal grounded to its default width.
    pub fn int64() -> Ty {
        Ty::Int(IntTy::i64())
    }

    /// A fresh float-literal type: width deferred. Inference or the backend fixes the width (`Float64`).
    pub fn float() -> Ty {
        Ty::Float(FloatTy::deferred())
    }

    /// The 64-bit float type (`Float64`) — a float literal grounded to its default width.
    pub fn float64() -> Ty {
        Ty::Float(FloatTy::f64())
    }

    /// Peel every NOMINAL tag, returning the innermost underlying structural type. A nominal newtype is
    /// erased at run time (`type-system.md §156` — the tag adds nothing to the representation), so the
    /// BOUNDARY treats a nominal value exactly as its underlying value: a `(type UserId (Mk Int64))`
    /// crosses as `Int64`, a `(type Rec (Mk Int64 Int64 Int64))` as its `(Tuple …)`. A non-nominal type
    /// is returned unchanged. (Nesting is not produced this increment — the guard is sum-free — but peel
    /// through it anyway so the helper is robust.)
    pub fn strip_nominal(&self) -> &Ty {
        let mut t = self;
        while let Ty::Nominal { inner, .. } = t {
            t = inner;
        }
        t
    }

    /// Peel BOTH `Nominal` and `Qty` wrappers to the underlying representation type — the erased form a
    /// key/element crosses as (a `Qty Float64 <unit>` erases to `Float64`, a newtype to its inner). Ord-key
    /// wrapping (`__CdzF{N}`) keys on this repr, so a Qty-over-Float `BTreeMap`/`BTreeSet` key gets the
    /// total-order wrapper a bare float does (else a raw `f64` key → `f64: Ord` E0277). Loops so a
    /// `Nominal(Qty(…))` / `Qty(Nominal(…))` fully unwraps to the innermost representation.
    pub fn strip_nominal_and_qty(&self) -> &Ty {
        let mut t = self;
        loop {
            match t {
                Ty::Nominal { inner, .. } => t = inner,
                Ty::Qty { inner, .. } => t = inner,
                _ => return t,
            }
        }
    }

    /// Whether the type contains an UNRESOLVED type variable ([`Ty::Var`]) — a payload/element/parameter
    /// the inference solve never determined (a bare `(None)` is `Option ?0`; an empty `(list)` is `List
    /// ?0`). Such a type has no defined serialization, so a value of it reaching the HOST BOUNDARY is a
    /// rejection that should name the AMBIGUITY (annotate the type) rather than a boundary-SHAPE error.
    /// (Deferred integer width/sign are NOT free variables — they ground to a default; only a `Ty::Var`
    /// is genuinely undetermined.)
    /// Whether this type contains an `Any` anywhere — an unconstrained position the body inference could
    /// not pin (and grounded to `Any`, not a free `Var`). Used with [`has_free_var`] by call-site seeding
    /// to recognize a param whose type still has a hole to fill (`(Tuple Int64 Any)` — the body pinned the
    /// first field but left the second). Structurally mirrors `has_free_var`.
    pub fn has_any(&self) -> bool {
        match self {
            Ty::Any => true,
            Ty::Fn(p, r) => p.has_any() || r.has_any(),
            Ty::Tuple(elems) => elems.iter().any(|t| t.has_any()),
            Ty::List(elem) => elem.has_any(),
            Ty::Map(k, v) => k.has_any() || v.has_any(),
            Ty::Set(elem) => elem.has_any(),
            Ty::Record(fields) => fields.values().any(|t| t.has_any()),
            Ty::Sum { args, .. } => args.iter().any(|t| t.has_any()),
            Ty::Qty { inner, .. } => inner.has_any(),
            Ty::Nominal { args, .. } => args.iter().any(|t| t.has_any()),
            Ty::Cont { resume, answer } => resume.has_any() || answer.has_any(),
            Ty::Var(_)
            | Ty::Int(_)
            | Ty::Bool
            | Ty::Unit
            | Ty::Type
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Symbol
            | Ty::BigInt
            | Ty::Rational
            | Ty::Float(_) => false,
        }
    }

    /// Whether this type is FULLY SOLVED — no `Ty::Any`, no free `Ty::Var`, and no integer/float axis left
    /// `Deferred`/`Var` (i.e. every `IntTy`/`FloatTy` sign+width is `Fixed`). A fully-solved type has a
    /// determinate machine representation (a concrete valtype + box/unbox op set) at every position.
    ///
    /// The B2 sharing-aware-emit bind PLAN requires this: binding a shared heap node into a `Core::Let`
    /// slot gives the slot the node's type as its valtype; an UNSOLVED type (`Int` with `Deferred` sign or
    /// width — the shape v-rb's install dump showed on two Record.with shares) yields an indeterminate slot
    /// valtype and the emit fails with `let-binding reference has no local slot`. So a share whose type is
    /// not fully solved is NOT bind-safe. (Deferred int axes GROUND to a default at emit for a normal
    /// value, but a slot binding must fix the valtype up front — the grounding is not visible to the slot
    /// synth — so the plan must require a solved type rather than rely on the default.)
    pub fn is_fully_solved(&self) -> bool {
        let int_solved =
            |it: &IntTy| matches!(it.sign, Sign::Fixed(_)) && matches!(it.width, Width::Fixed(_));
        match self {
            Ty::Any | Ty::Var(_) => false,
            Ty::Int(it) => int_solved(it),
            Ty::Float(ft) => matches!(ft.width, Width::Fixed(_)),
            Ty::Fn(p, r) => p.is_fully_solved() && r.is_fully_solved(),
            Ty::Tuple(elems) => elems.iter().all(|t| t.is_fully_solved()),
            Ty::List(elem) => elem.is_fully_solved(),
            Ty::Map(k, v) => k.is_fully_solved() && v.is_fully_solved(),
            Ty::Set(elem) => elem.is_fully_solved(),
            Ty::Record(fields) => fields.values().all(|t| t.is_fully_solved()),
            Ty::Sum { args, .. } => args.iter().all(|t| t.is_fully_solved()),
            Ty::Qty { inner, .. } => inner.is_fully_solved(),
            Ty::Nominal { args, .. } => args.iter().all(|t| t.is_fully_solved()),
            Ty::Cont { resume, answer } => resume.is_fully_solved() && answer.is_fully_solved(),
            Ty::Bool
            | Ty::Unit
            | Ty::Type
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Symbol
            | Ty::BigInt
            | Ty::Rational => true,
        }
    }

    /// Whether this type contains an `Any` in a DATA-CONTAINER ELEMENT position — nested in a
    /// List/Tuple/Map/Set/Record/Sum/Nominal/Qty — but NOT reached through a function arrow (`Ty::Fn`).
    /// `(List Any)` and `(Tuple Int64 Any)` are true; `(-> Int64 (-> Any Any))` and a bare `Any` are
    /// false (the arrow's `Any` is a not-yet-solved closure domain/result, a legitimate intermediate the
    /// closure-tie machinery grounds — distinct from the self-nested-producer re-entrancy poison, which
    /// collapses a data element to `Any`). Used by the `type_of` memo guard to skip caching ONLY the
    /// producer-re-entrancy poison, leaving a curried-closure arrow signature cached (its `Any`s ground
    /// via the transformer-closure tie, and a module-member generic monomorphization depends on it).
    pub fn has_any_in_data_element(&self) -> bool {
        match self {
            // An arrow is NOT descended: its `Any`s are closure domain/result holes, not the data poison.
            Ty::Fn(_, _) => false,
            // A bare `Any` at THIS position is not a nested data element (the caller already excludes it).
            Ty::Any => false,
            Ty::Tuple(elems) => elems.iter().any(|t| t.contains_any_no_arrow()),
            Ty::List(elem) | Ty::Set(elem) => elem.contains_any_no_arrow(),
            Ty::Map(k, v) => k.contains_any_no_arrow() || v.contains_any_no_arrow(),
            Ty::Record(fields) => fields.values().any(|t| t.contains_any_no_arrow()),
            Ty::Sum { args, .. } | Ty::Nominal { args, .. } => {
                args.iter().any(|t| t.contains_any_no_arrow())
            }
            Ty::Qty { inner, .. } => inner.contains_any_no_arrow(),
            _ => false,
        }
    }

    /// Whether this type contains an `Any` ANYWHERE reachable WITHOUT crossing a function arrow — the
    /// element-position recursion used by [`has_any_in_data_element`]. An `Any` under a `Ty::Fn` is
    /// invisible here (that arm returns false); a data-container element's `Any` is visible.
    fn contains_any_no_arrow(&self) -> bool {
        match self {
            Ty::Any => true,
            Ty::Fn(_, _) => false,
            Ty::Tuple(elems) => elems.iter().any(|t| t.contains_any_no_arrow()),
            Ty::List(elem) | Ty::Set(elem) => elem.contains_any_no_arrow(),
            Ty::Map(k, v) => k.contains_any_no_arrow() || v.contains_any_no_arrow(),
            Ty::Record(fields) => fields.values().any(|t| t.contains_any_no_arrow()),
            Ty::Sum { args, .. } | Ty::Nominal { args, .. } => {
                args.iter().any(|t| t.contains_any_no_arrow())
            }
            Ty::Qty { inner, .. } => inner.contains_any_no_arrow(),
            _ => false,
        }
    }

    /// Whether this type carries a numeric whose WIDTH or SIGN is still UNGROUNDED (a `Width::Deferred`/
    /// `Width::Var` or `Sign::Deferred`/`Sign::Var`) — reachable at this position or in a no-arrow data
    /// element. A bare integer/decimal literal is deferred until something constrains it; a member of a
    /// mutual-recursion SCC solved WHILE a sibling is in-flight sees that sibling as `Any`, so a bare-literal
    /// return grounds to a still-DEFERRED numeric (no concrete peer yet) that the old scheme solve CACHED —
    /// freezing e.g. `v0`'s return `Int{Deferred}` (→ default `Int64`) while a sibling's `(: 3 UInt16)` base
    /// pins the group `UInt16`, so the two schemes disagree at the machine width and the emit is invalid wasm
    /// (#6049). `compute_def_scheme` uses this to DEFER such a reentrant result (uncached) so a later clean
    /// demand re-grounds the width against the now-concrete sibling. An arrow is NOT descended (a closure
    /// domain/result hole is not the return-position data numeric), matching `has_any_in_data_element`.
    pub fn has_ungrounded_width(&self) -> bool {
        // TOP-LEVEL numeric ONLY — deliberately NOT recursing into ANY compound. This predicate scopes the
        // mutual-recursion NUMERIC-width defer (#6049): a bare-literal recursive-group RETURN whose width
        // must adopt a concrete sibling. #6049's witness is a TOP-LEVEL bare `Int` return. Descending into a
        // compound made a nested deferred-default `Int64` leaf (e.g. `(GIter Int64)`, `(List Int64)`) look
        // ungrounded, so the reentrant skip-cache DEFERRED a COMPOUND node during a multi-def SCC solve; the
        // deferred node then re-derives against a not-yet-grounded polymorphic state and picks up an EXTRA
        // type-constructor layer (`b` → `GIter b`) or mis-grounds a leaf to `Unit` — the `zip`/`interleave`/
        // `int-list-eq` iterators regression from #6366 (CDZ0203 "`(GIter (GIter Int64))` expected" / "`(List
        // Unit)` vs `(List Int64)`"). A compound's element widths are a DIFFERENT fixpoint (its own inference
        // + generic monomorphization), NOT the mutual-recursion return-width case, so the defer must not fire
        // for a compound. A top-level bare numeric re-derives to the same numeric (safe); #6049's top-level
        // `Int{Deferred}` return still fires + defers as intended.
        matches!(
            self,
            Ty::Int(it) if !matches!(it.width, Width::Fixed(_)) || !matches!(it.sign, Sign::Fixed(_))
        ) || matches!(self, Ty::Float(ft) if !matches!(ft.width, Width::Fixed(_)))
    }

    /// Whether this type contains a TYPE-VALUE (`Ty::Type`, the kind of types) anywhere — itself, or
    /// nested in a compound (`(Tuple Type Int64)`, `(List Type)`, a record field, a sum/nominal arg). A
    /// type-value is COMPILE-TIME ONLY (`type-system.md §226`: a type-value never flows from runtime data),
    /// so a value whose type contains one cannot reach a runtime slot / cross the boundary — the check
    /// `compile::collect_faults` uses to report ONE coded reject for a type stored in a compound (rather
    /// than the emit path's uncoded no-runtime-form cascade). Structurally mirrors [`has_any`].
    pub fn has_type_value(&self) -> bool {
        match self {
            Ty::Type => true,
            Ty::Fn(p, r) => p.has_type_value() || r.has_type_value(),
            Ty::Tuple(elems) => elems.iter().any(|t| t.has_type_value()),
            Ty::List(elem) | Ty::Set(elem) => elem.has_type_value(),
            Ty::Map(k, v) => k.has_type_value() || v.has_type_value(),
            Ty::Record(fields) => fields.values().any(|t| t.has_type_value()),
            Ty::Sum { args, .. } | Ty::Nominal { args, .. } => {
                args.iter().any(|t| t.has_type_value())
            }
            Ty::Qty { inner, .. } => inner.has_type_value(),
            Ty::Cont { resume, answer } => resume.has_type_value() || answer.has_type_value(),
            Ty::Var(_)
            | Ty::Any
            | Ty::Int(_)
            | Ty::Bool
            | Ty::Unit
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Symbol
            | Ty::BigInt
            | Ty::Rational
            | Ty::Float(_) => false,
        }
    }

    /// Fill this type's HOLES (`Any` / a free `Var`) with the corresponding part of `concrete` — a
    /// structural merge that keeps `self`'s already-determined parts and takes `concrete`'s only where
    /// `self` is unconstrained. Used by call-site seeding: a body-solved param `(Tuple Int64 Any)` merged
    /// with a call site's `(Tuple Int64 Int64)` becomes `(Tuple Int64 Int64)` — the `Any` value field is
    /// filled while the pinned key field is preserved. Where the shapes AGREE the merge recurses
    /// element-wise; where `self` is a hole it takes `concrete` wholesale; otherwise `self` is kept (a
    /// genuine disagreement is a fault reported elsewhere — the merge never invents an incompatible type).
    pub fn fill_holes(&self, concrete: &Ty) -> Ty {
        match self {
            Ty::Any | Ty::Var(_) => concrete.clone(),
            Ty::Tuple(a) => match concrete {
                Ty::Tuple(b) if a.len() == b.len() => Ty::Tuple(
                    a.iter()
                        .zip(b.iter())
                        .map(|(x, y)| x.fill_holes(y))
                        .collect(),
                ),
                _ => self.clone(),
            },
            Ty::List(a) => match concrete {
                Ty::List(b) => Ty::List(Box::new(a.fill_holes(b))),
                _ => self.clone(),
            },
            Ty::Set(a) => match concrete {
                Ty::Set(b) => Ty::Set(Box::new(a.fill_holes(b))),
                _ => self.clone(),
            },
            Ty::Map(ka, va) => match concrete {
                Ty::Map(kb, vb) => {
                    Ty::Map(Box::new(ka.fill_holes(kb)), Box::new(va.fill_holes(vb)))
                }
                _ => self.clone(),
            },
            Ty::Record(a) => match concrete {
                Ty::Record(b) if self.agrees_with(concrete) => {
                    let merged = a
                        .iter()
                        .map(|(k, ta)| {
                            let t = b
                                .get(k)
                                .map(|tb| ta.fill_holes(tb))
                                .unwrap_or_else(|| ta.clone());
                            (k.clone(), t)
                        })
                        .collect();
                    Ty::Record(std::rc::Rc::new(merged))
                }
                _ => self.clone(),
            },
            Ty::Sum { decl, args: aa } => match concrete {
                Ty::Sum { args: ab, .. } if self.agrees_with(concrete) => Ty::Sum {
                    decl: *decl,
                    args: aa
                        .iter()
                        .zip(ab.iter())
                        .map(|(x, y)| x.fill_holes(y))
                        .collect(),
                },
                _ => self.clone(),
            },
            // Leaves and other determined shapes keep themselves (no hole to fill).
            _ => self.clone(),
        }
    }

    /// Whether this type is fully GROUND — contains NO substitutable variable of ANY kind (a type `Var`,
    /// an integer/float `Width::Var`, or a `Sign::Var`). A ground type is a FIXPOINT of `Subst::apply`
    /// (applying any substitution returns it unchanged), so `apply` short-circuits on it — cloning the
    /// (Rc-shared) type is then a refcount bump, not a deep rebuild of a wide `Record`/`Tuple`/`Sum`.
    /// STRONGER than `!has_free_var()`: that ignores width/sign vars (a `Ty::Int` with a `Width::Var` has
    /// no free TYPE var yet is not ground), which would make an `apply` fast-path keyed on it INCORRECT.
    pub fn is_ground(&self) -> bool {
        match self {
            Ty::Var(_) => false,
            Ty::Int(it) => {
                !matches!(it.width, crate::ty::Width::Var(_))
                    && !matches!(it.sign, crate::ty::Sign::Var(_))
            }
            Ty::Float(ft) => !matches!(ft.width, crate::ty::Width::Var(_)),
            Ty::Fn(p, r) => p.is_ground() && r.is_ground(),
            Ty::Tuple(elems) => elems.iter().all(|t| t.is_ground()),
            Ty::List(elem) => elem.is_ground(),
            Ty::Map(k, v) => k.is_ground() && v.is_ground(),
            Ty::Set(elem) => elem.is_ground(),
            Ty::Record(fields) => fields.values().all(|t| t.is_ground()),
            Ty::Sum { args, .. } => args.iter().all(|t| t.is_ground()),
            Ty::Qty { inner, .. } => inner.is_ground(),
            // `Nominal` mirrors `Subst::apply`: it substitutes into BOTH `args` and the `inner` machine-rep
            // hint, so both must be ground for the type to be an `apply` fixpoint.
            Ty::Nominal { args, inner, .. } => {
                args.iter().all(|t| t.is_ground()) && inner.is_ground()
            }
            Ty::Cont { resume, answer } => resume.is_ground() && answer.is_ground(),
            Ty::Bool
            | Ty::Unit
            | Ty::Type
            | Ty::Any
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Symbol
            | Ty::BigInt
            | Ty::Rational => true,
        }
    }

    pub fn has_free_var(&self) -> bool {
        match self {
            Ty::Var(_) => true,
            Ty::Fn(p, r) => p.has_free_var() || r.has_free_var(),
            Ty::Tuple(elems) => elems.iter().any(|t| t.has_free_var()),
            Ty::List(elem) => elem.has_free_var(),
            Ty::Map(k, v) => k.has_free_var() || v.has_free_var(),
            Ty::Set(elem) => elem.has_free_var(),
            Ty::Record(fields) => fields.values().any(|t| t.has_free_var()),
            Ty::Sum { args, .. } => args.iter().any(|t| t.has_free_var()),
            // A quantity's free variables are its INNER numeric type's — the unit is a concrete
            // compile-time value (a canonical exponent map), never a variable.
            Ty::Qty { inner, .. } => inner.has_free_var(),
            // A nominal's free variables are its type ARGS' (exactly like `Ty::Sum`) — a generic `Box ?0`
            // at an unsolved instantiation carries the free var in `args`. NOT `inner` (a recursive
            // nominal's inner holds a `Ty::Sum{decl}` back-edge that is not a free var anyway; `args` is
            // the identity/instantiation axis).
            Ty::Nominal { args, .. } => args.iter().any(|t| t.has_free_var()),
            Ty::Cont { resume, answer } => resume.has_free_var() || answer.has_free_var(),
            // Bytes, String, Char, Symbol, BigInt, and Rational are leaves — no inner type, no free var.
            Ty::Int(_)
            | Ty::Bool
            | Ty::Unit
            | Ty::Type
            | Ty::Any
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Symbol
            | Ty::BigInt
            | Ty::Rational
            | Ty::Float(_) => false,
        }
    }

    /// Whether this type carries an UNDETERMINED component that makes it unfit to serialize at the host
    /// boundary — a free type variable ([`has_free_var`]) OR a [`Ty::Any`]. An unconstrained escaping value
    /// leaves its type variable a `Var` in SOME shapes (a bare `(None)` → `(Option _)`), but an empty
    /// collection GROUNDS its element to `Ty::Any` instead (`(list)` → `(List Any)`) — the SAME
    /// undetermined-type fault, differently spelled. `has_free_var` sees only the `Var` form, so an
    /// `Any`-element escape (`(def (main) (list))`) slipped past the `cdz check` undetermined-escape reject
    /// and hit an uncoded emit decline (a check≡emit gap). This predicate unifies both so the coded CDZ0203
    /// "annotate it" fires for either grounding. Used ONLY behind [`crate::backend::wasm::
    /// crosses_as_resource_escape`], which admits only compound/heap shapes — so a bare top-level `Ty::Any`
    /// (a diverging body that never crosses as a resource) is not reached here, and every `Any` this sees is
    /// a nested element/payload/field the boundary walker cannot render. Structurally mirrors `has_free_var`.
    pub fn has_undetermined_escape_component(&self) -> bool {
        match self {
            Ty::Var(_) | Ty::Any => true,
            Ty::Fn(p, r) => {
                p.has_undetermined_escape_component() || r.has_undetermined_escape_component()
            }
            Ty::Tuple(elems) => elems.iter().any(|t| t.has_undetermined_escape_component()),
            Ty::List(elem) | Ty::Set(elem) => elem.has_undetermined_escape_component(),
            Ty::Map(k, v) => {
                k.has_undetermined_escape_component() || v.has_undetermined_escape_component()
            }
            Ty::Record(fields) => fields
                .values()
                .any(|t| t.has_undetermined_escape_component()),
            Ty::Sum { args, .. } => args.iter().any(|t| t.has_undetermined_escape_component()),
            Ty::Qty { inner, .. } => inner.has_undetermined_escape_component(),
            Ty::Nominal { args, .. } => args.iter().any(|t| t.has_undetermined_escape_component()),
            Ty::Cont { resume, answer } => {
                resume.has_undetermined_escape_component()
                    || answer.has_undetermined_escape_component()
            }
            Ty::Int(_)
            | Ty::Bool
            | Ty::Unit
            | Ty::Type
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Symbol
            | Ty::BigInt
            | Ty::Rational
            | Ty::Float(_) => false,
        }
    }

    /// Collect the distinct free type-variable numbers ([`Ty::Var`]) this type mentions, into `out` (in
    /// first-seen order, deduplicated). Used to GENERALIZE a recursive-generic def's signature into a
    /// [`Scheme`]'s `ty_vars` (recursive-generic monomorphization) — a parameter the body only threads is
    /// a `Ty::Var`, and the scheme quantifies over those so each call site instantiates fresh. Structurally
    /// mirrors [`has_free_var`]. Only whole-type vars are collected: a deferred integer width/sign grounds
    /// to a default (never a genuine free variable), so `width_vars`/`sign_vars` are left empty by the
    /// generalizer — a generic param is generic over its WHOLE type, not a numeric width alone.
    pub fn collect_free_vars(&self, out: &mut Vec<u32>) {
        match self {
            Ty::Var(n) => {
                if !out.contains(n) {
                    out.push(*n);
                }
            }
            Ty::Fn(p, r) => {
                p.collect_free_vars(out);
                r.collect_free_vars(out);
            }
            Ty::Tuple(elems) => elems.iter().for_each(|t| t.collect_free_vars(out)),
            Ty::List(elem) | Ty::Set(elem) => elem.collect_free_vars(out),
            Ty::Map(k, v) => {
                k.collect_free_vars(out);
                v.collect_free_vars(out);
            }
            Ty::Record(fields) => fields.values().for_each(|t| t.collect_free_vars(out)),
            Ty::Cont { resume, answer } => {
                resume.collect_free_vars(out);
                answer.collect_free_vars(out);
            }
            Ty::Sum { args, .. } | Ty::Nominal { args, .. } => {
                args.iter().for_each(|t| t.collect_free_vars(out))
            }
            Ty::Qty { inner, .. } => inner.collect_free_vars(out),
            Ty::Int(_)
            | Ty::Bool
            | Ty::Unit
            | Ty::Type
            | Ty::Any
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Symbol
            | Ty::BigInt
            | Ty::Rational
            | Ty::Float(_) => {}
        }
    }

    /// Whether two types are COMPATIBLE — a structural yes/no relation, distinct from [`crate::unify`]
    /// which solves variables into a substitution. This is the cheap check the passes that only need a
    /// verdict use (an annotation's declared vs inferred type, an `if`'s two branches, a match
    /// pattern's type vs its scrutinee); the solving inference uses `unify`. `Any` agrees with anything
    /// (a poison never disagrees); two integers agree if their signedness matches and their widths are
    /// compatible (a deferred/variable width or sign is compatible — it has not been fixed yet).
    pub fn agrees_with(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Any, _) | (_, Ty::Any) => true,
            // A variable is not yet solved, so it is compatible with anything (unification, not this
            // relation, is what actually resolves it).
            (Ty::Var(_), _) | (_, Ty::Var(_)) => true,
            (Ty::Int(a), Ty::Int(b)) => {
                // A fixed sign must match; a deferred/variable sign is compatible (not yet fixed).
                let sign_ok = match (a.sign, b.sign) {
                    (Sign::Fixed(sa), Sign::Fixed(sb)) => sa == sb,
                    _ => true,
                };
                let width_ok = match (a.width, b.width) {
                    (Width::Fixed(wa), Width::Fixed(wb)) => wa == wb,
                    // a deferred or variable width has not been fixed, so it is compatible.
                    _ => true,
                };
                sign_ok && width_ok
            }
            (Ty::Bool, Ty::Bool) => true,
            (Ty::Unit, Ty::Unit) => true,
            // Two function types agree iff their parameters and results agree.
            (Ty::Fn(pa, ra), Ty::Fn(pb, rb)) => pa.agrees_with(pb) && ra.agrees_with(rb),
            // Two records agree iff they have the same field-name set and each field's types agree.
            (Ty::Record(a), Ty::Record(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, ta)| match b.get(k) {
                        Some(tb) => ta.agrees_with(tb),
                        None => false,
                    })
            }
            // Two tuples agree iff they have the SAME ARITY and each position's types agree — a
            // different arity is a different type (the corpus `if`-branch cases: a 2-tuple and a 3-tuple
            // do not agree, nor a `(Tuple Int Int)` with a `(Tuple Int Bool)`).
            (Ty::Tuple(a), Ty::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(ta, tb)| ta.agrees_with(tb))
            }
            // Two lists agree iff their ELEMENT types agree — `List Int64` ≠ `List Bool`, and a list of a
            // deferred element type is compatible via the recursive `agrees_with` (the empty-list case).
            (Ty::List(a), Ty::List(b)) => a.agrees_with(b),
            // Two maps agree iff their KEY types agree AND their VALUE types agree — `Map Int64 Int64` ≠
            // `Map Int64 Bool`. A map's KEY SET is NOT compared here (it is runtime data, not shape): two
            // maps with different keys but the same `(key, value)` types are the SAME type and agree, so a
            // comparison between them is well-typed (yielding `false`) and a list of them is homogeneous —
            // the contrast with `Record` above (whose field-name set IS its shape). Deferred key/value
            // types (an empty map) are compatible via the recursive `agrees_with`.
            (Ty::Map(ka, va), Ty::Map(kb, vb)) => ka.agrees_with(kb) && va.agrees_with(vb),
            // Two sets agree iff their ELEMENT types agree — `Set Int64` ≠ `Set Bool`. The element SET is
            // NOT compared (runtime data, like a map's key set): two sets with different elements but the
            // same element type are the SAME type and agree (a well-typed comparison → `false`).
            (Ty::Set(a), Ty::Set(b)) => a.agrees_with(b),
            // Bytes is a leaf — a bytes agrees only with another bytes (no element type to compare).
            (Ty::Bytes, Ty::Bytes) => true,
            // Two sums agree iff their DECLARATIONS match AND their type ARGS agree pairwise — a sum's
            // identity is its declaration (`type-system.md §160`: distinct FQNs ⇒ distinct types, so
            // module A's `Foo` ≠ module B's `Foo`) TOGETHER WITH its instantiation (`Option Int64 ≠
            // Option Bool` — same declaration, different type argument; §the head agrees but the payload
            // does not). A monomorphic sum has empty `args` on both sides, so this reduces to the decl
            // check. A deferred arg (an as-yet-unsolved payload) is compatible via the recursive
            // `agrees_with`.
            (
                Ty::Sum {
                    decl: a, args: aa, ..
                },
                Ty::Sum {
                    decl: b, args: ab, ..
                },
            ) => {
                a == b
                    && aa.len() == ab.len()
                    && aa.iter().zip(ab.iter()).all(|(x, y)| x.agrees_with(y))
            }
            // Two nominals agree iff their DECLARATIONS match AND their type ARGS agree pairwise — the
            // exact `Ty::Sum` rule (decl is identity, args the instantiation). NOT `inner` — a recursive
            // nominal's inner diverges by derivation path, so comparing it would make `Lst` disagree with
            // `Lst`. A nominal never agrees with its bare underlying type (the `(_, _)` fallthrough handles
            // `(Nominal, Int)`), so `UserId` cannot pass where `Int64` is required (§Nominal Types Are Not
            // Comparable Across Their Boundary).
            (
                Ty::Nominal {
                    decl: a, args: aa, ..
                },
                Ty::Nominal {
                    decl: b, args: ab, ..
                },
            ) => {
                a == b
                    && aa.len() == ab.len()
                    && aa.iter().zip(ab.iter()).all(|(x, y)| x.agrees_with(y))
            }
            // A RECURSIVE NOMINAL, FOLDED (`Ty::Nominal{decl}`) vs UNFOLDED μ back-edge (`Ty::Sum{decl}`) —
            // the same recursive newtype reached by different derivation paths (see the matching `unify`
            // arm). A recursive field projected out of the compound is the bare-Sum back-edge; it must
            // AGREE with the folded declared type. Same `decl` (a nominal's back-edge reuses its own
            // declaration occurrence; a genuine sum's decl differs, so this never conflates two types) +
            // args pairwise. Without it, a recursive-newtype traversal's tail projection disagreed with
            // its own declared type.
            (
                Ty::Sum {
                    decl: a, args: aa, ..
                },
                Ty::Nominal {
                    decl: b, args: ab, ..
                },
            )
            | (
                Ty::Nominal {
                    decl: a, args: aa, ..
                },
                Ty::Sum {
                    decl: b, args: ab, ..
                },
            ) => {
                a == b
                    && aa.len() == ab.len()
                    && aa.iter().zip(ab.iter()).all(|(x, y)| x.agrees_with(y))
            }
            // `String` is monomorphic — the one string type agrees only with itself.
            (Ty::String, Ty::String) => true,
            // `Char` is monomorphic — the one char type agrees only with itself.
            (Ty::Char, Ty::Char) => true,
            // `Symbol` is monomorphic — the one symbol type agrees only with itself (and NOT with the
            // `String` it wraps: the nominal boundary, handled by the `_ => false` fallthrough).
            (Ty::Symbol, Ty::Symbol) => true,
            // `BigInt` is monomorphic and DISTINCT — the one arbitrary-precision integer type agrees only
            // with itself, NEVER with a fixed-width `Ty::Int` (no silent promotion: a `BigInt`/`Int64`
            // mix is CDZ0301, via the `_ => false` fallthrough), the same discipline as float-vs-int.
            (Ty::BigInt, Ty::BigInt) => true,
            // `Rational` is monomorphic and DISTINCT — the one exact-rational type agrees only with itself,
            // NEVER with an integer or a `BigInt` (crossing in is explicit via `Rational.of-int`; a
            // `Rational`/integer mix is CDZ0301 via the `_ => false` fallthrough), the same no-promotion
            // discipline as float-vs-int and BigInt-vs-int.
            (Ty::Rational, Ty::Rational) => true,
            // Two floats agree iff their WIDTHS agree — `Float32` ≠ `Float64` (no silent promotion), a
            // deferred/variable width is compatible (not yet fixed). A float never agrees with an integer
            // (numeric-model.md §Numeric Types Do Not Silently Promote). Mirrors the `Ty::Int` width check.
            (Ty::Float(a), Ty::Float(b)) => match (a.width, b.width) {
                (Width::Fixed(wa), Width::Fixed(wb)) => wa == wb,
                _ => true,
            },
            // Two quantities agree iff their INNER numeric types agree AND their UNITS are EQUAL — a
            // meter and a second (same inner `Float64`, different dimension) do NOT agree, and a meter and
            // a bare `Float64` do not agree (no implicit dimensionless coercion). A quantity never agrees
            // with a non-quantity; that falls to the `_ => false` below.
            (
                Ty::Qty {
                    inner: ia,
                    unit: ua,
                },
                Ty::Qty {
                    inner: ib,
                    unit: ub,
                },
            ) => ua == ub && ia.agrees_with(ib),
            _ => false,
        }
    }

    /// The "more defined" of two agreeing types — the join used to type an `if` from its branches:
    /// `Any` yields the other; a deferred-width int yields the branch that fixed its width. This is
    /// how the deferred width flows from a constrained branch to the whole conditional.
    pub fn join(&self, other: &Ty) -> Ty {
        match (self, other) {
            (Ty::Any, t) | (t, Ty::Any) => t.clone(),
            // A variable yields the other side (the more-defined type).
            (Ty::Var(_), t) | (t, Ty::Var(_)) => t.clone(),
            (Ty::Int(a), Ty::Int(b)) => {
                // Prefer whichever side fixed each axis (Fixed > Deferred/Var).
                let width = match (a.width, b.width) {
                    (Width::Fixed(w), _) | (_, Width::Fixed(w)) => Width::Fixed(w),
                    (Width::Deferred, _) | (_, Width::Deferred) => Width::Deferred,
                    _ => a.width,
                };
                let sign = match (a.sign, b.sign) {
                    (Sign::Fixed(s), _) | (_, Sign::Fixed(s)) => Sign::Fixed(s),
                    (Sign::Deferred, _) | (_, Sign::Deferred) => Sign::Deferred,
                    _ => a.sign,
                };
                Ty::Int(IntTy { sign, width })
            }
            // Two floats join by preferring the FIXED width (mirroring the Int arm) — so a DEFERRED float
            // literal (`2.0`, `Ty::float()`) in one arm ADAPTS to the sibling arm's `Float32`, exactly as a
            // deferred INT literal adapts to a sibling `Int8`. Without this the pair hit the `_ => self.clone()`
            // fallthrough and a deferred-float arm STAYED deferred → grounded to the Float64 DEFAULT, leaving an
            // f64-vs-f32 join whose emit lowered to INVALID WASM (v-cdz-smith match-arm float-width; the `if`
            // analog was latently the same, masked by const-cond dead-branch-elim). A deferred literal taking a
            // determined width is the literal's type being FIXED, NOT an implicit promotion (numeric-model
            // §a-declared-default-fixes-a-type-not-a-conversion). Two DIFFERENT fixed widths (`Float64` ⊔
            // `Float32`) still return `self` here but are REJECTED by the arms-agree check (`agrees_with` = false,
            // no silent promotion between float widths) — identical to the Int arm's two-fixed-widths behavior.
            (Ty::Float(a), Ty::Float(b)) => {
                let width = match (a.width, b.width) {
                    (Width::Fixed(w), _) | (_, Width::Fixed(w)) => Width::Fixed(w),
                    _ => a.width,
                };
                Ty::Float(FloatTy { width })
            }
            // Two agreeing records join field-wise (a deferred width in one branch's field is fixed by
            // the other). If they disagree, keep `self` — the branches-agree check reports the fault.
            (Ty::Record(a), Ty::Record(b)) if self.agrees_with(other) => {
                let joined = a
                    .iter()
                    .map(|(k, ta)| {
                        let t = b.get(k).map(|tb| ta.join(tb)).unwrap_or_else(|| ta.clone());
                        (k.clone(), t)
                    })
                    .collect();
                Ty::Record(std::rc::Rc::new(joined))
            }
            // Two agreeing tuples join position-wise (same arity, guaranteed by `agrees_with`).
            (Ty::Tuple(a), Ty::Tuple(b)) if self.agrees_with(other) => {
                Ty::Tuple(a.iter().zip(b.iter()).map(|(ta, tb)| ta.join(tb)).collect())
            }
            // Two agreeing SUMS join their type ARGS pairwise, so a payload resolved in EITHER branch
            // determines the joined arg — `(Option ?0)` ⊔ `(Option Int64)` = `(Option Int64)`. Without
            // this, an `if` whose branches are the SAME sum built two ways (a nullary variant `(None)` :
            // `Option ?0` in one branch, a payload variant `(Some n)` : `Option Int64` in the other) took
            // the `_ => self.clone()` fallthrough and kept the FIRST branch's type — so a leading `None`
            // pinned the result to `(Option ?0)` with `?0` free, and the value-heap layout then declined
            // "projecting a tuple element of type ?0 needs the value heap". Joining the args makes the
            // conditional's type ORDER-INDEPENDENT: the payload-carrying branch fixes the parameter in
            // either position. (`agrees_with` guarantees same `decl` + arg arity.)
            (Ty::Sum { decl, args: aa }, Ty::Sum { args: ab, .. }) if self.agrees_with(other) => {
                Ty::Sum {
                    decl: *decl,
                    args: aa.iter().zip(ab.iter()).map(|(x, y)| x.join(y)).collect(),
                }
            }
            // Two agreeing lists join their element type — a deferred element (`List ?0`, the empty list)
            // is fixed by the other branch's `List Int64`, the list analogue of the sum-arg join above.
            (Ty::List(a), Ty::List(b)) if self.agrees_with(other) => Ty::List(Box::new(a.join(b))),
            // Two agreeing sets join their element type — a deferred element (`Set ?0`, an empty set) is
            // fixed by the other branch's `Set Int64`, the set analogue of the list join.
            (Ty::Set(a), Ty::Set(b)) if self.agrees_with(other) => Ty::Set(Box::new(a.join(b))),
            // Two agreeing maps join their key type AND value type — a deferred key/value (`Map ?0 ?1`,
            // the empty map) is fixed by the other branch's `Map Int64 Int64`, the map analogue of the
            // list join above.
            (Ty::Map(ka, va), Ty::Map(kb, vb)) if self.agrees_with(other) => {
                Ty::Map(Box::new(ka.join(kb)), Box::new(va.join(vb)))
            }
            // Two agreeing quantities (same unit, guaranteed by `agrees_with`) join their INNER numeric
            // type — a deferred inner width in one `if` branch is fixed by the other, carrying the shared
            // unit through the conditional.
            (
                Ty::Qty {
                    inner: ia,
                    unit: ua,
                },
                Ty::Qty { inner: ib, .. },
            ) if self.agrees_with(other) => Ty::Qty {
                inner: Box::new(ia.join(ib)),
                unit: ua.clone(),
            },
            // Two agreeing nominals (same `decl`, guaranteed by `agrees_with`) join their type ARGS
            // pairwise — a deferred arg in one `if` branch is fixed by the other, the nominal analogue of
            // the sum-arg join. `inner` is re-derived from the joined args (the join of the two inners
            // would DIVERGE for a recursive nominal — `Ty::Sum{decl}` vs `Ty::Nominal{decl}` — so join by
            // args and keep this branch's inner, which the joined-args value makes representative).
            (
                Ty::Nominal {
                    decl,
                    args: aa,
                    inner,
                },
                Ty::Nominal { args: ab, .. },
            ) if self.agrees_with(other) => Ty::Nominal {
                decl: *decl,
                args: aa.iter().zip(ab.iter()).map(|(x, y)| x.join(y)).collect(),
                inner: inner.clone(),
            },
            _ => self.clone(),
        }
    }

    /// The type's name as it appears in a rendered value's annotation (e.g. the corpus `(: 42
    /// Int64)`). Supplied by the value renderer, which walks the static type; the runtime holds no
    /// such name. An integer's name is composed from its signedness and its GROUND width — a deferred
    /// width renders as its default — so an observed value's type is always concrete (`Int64`,
    /// `UInt32`, …). A language-level fact, target-neutral.
    pub fn render_name(&self, ncx: &NameCtx) -> String {
        // DEPTH GUARD (DoS-harden + readability): a type renders one recursive level per structural layer, so
        // an explosively-deep type — a self-application fixpoint building `(List (List … (-> Any Any)))` — can
        // recurse this renderer to a STACK OVERFLOW on the `rcdzc-compile` thread while BUILDING a CDZ0201
        // diagnostic (v-cdz-smith fuzzer DoS). Past a GENEROUS depth, truncate with `…`: a diagnostic never
        // needs deeper, no real (non-pathological) type reaches it, and this only affects MESSAGE TEXT — never
        // compile logic. (The type-BUILD/unify structural recursion is a separate bound, pending a reproducible
        // repro.) The counter is balanced by the RAII `Restore` on every exit path.
        thread_local! {
            static RENDER_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        }
        const MAX_RENDER_DEPTH: u32 = 24;
        let depth = RENDER_DEPTH.with(|c| c.get());
        if depth >= MAX_RENDER_DEPTH {
            return "…".to_string();
        }
        RENDER_DEPTH.with(|c| c.set(depth + 1));
        struct Restore(u32);
        impl Drop for Restore {
            fn drop(&mut self) {
                RENDER_DEPTH.with(|c| c.set(self.0));
            }
        }
        let _restore = Restore(depth);
        match self {
            Ty::Int(it) => {
                let stem = if it.ground_signed() { "Int" } else { "UInt" };
                format!("{stem}{}", it.ground_width())
            }
            Ty::Bool => "Bool".to_string(),
            Ty::Unit => "Unit".to_string(),
            // A string renders as `String` — one monomorphic type, no parameters.
            Ty::String => "String".to_string(),
            // A char renders as `Char` — one monomorphic type (its VALUES render `#\c`).
            Ty::Char => "Char".to_string(),
            Ty::Symbol => "Symbol".to_string(),
            // The arbitrary-precision integer renders as `BigInt` — one monomorphic type, no parameters.
            Ty::BigInt => "BigInt".to_string(),
            // The exact rational renders as `Rational` — one monomorphic type, no parameters.
            Ty::Rational => "Rational".to_string(),
            // A float renders as its aliased width name — `Float32`/`Float64`. Every admitted float
            // width ({32, 64}) has an alias, so an observed float type is always a concrete `FloatN`
            // (an unresolved width grounds to `Float64`), mirroring the integer `IntN`/`UIntN` render.
            Ty::Float(ft) => format!("Float{}", ft.ground_width()),
            // A record TYPE renders as `(Record (: name Type) …)` in canonical (sorted) field order — the
            // CAPITALIZED type-constructor head the author writes in an annotation (`(: r (Record (: a
            // Int64)))`), matching `Tuple`/`List`/`Map`/`Set` below (a lowercase `(record …)` in type
            // position is rejected as "not a type" — it is the VALUE constructor). Each field is the
            // canonical `(: name T)` ASCRIPTION node (RT3, DESIGN-record-type-syntax) — the same node a
            // param binder / `e: T` uses, not a bespoke pair. This is a TYPE renderer, so it must spell the
            // type the way the surface accepts it: a mismatch message naming `(record (: a Bool))` reads as
            // a different thing than the `(Record (: a Bool))` the author wrote.
            Ty::Record(fields) => {
                let mut s = String::from("(Record");
                for (k, t) in fields.iter() {
                    s.push_str(&format!(" (: {} {})", k.name, t.render_name(ncx)));
                }
                s.push(')');
                s
            }
            // A tuple renders as `(Tuple T0 T1 …)` in position order — the shape the renderer walks
            // (its arity and element types are its type).
            Ty::Tuple(elems) => {
                let mut s = String::from("(Tuple");
                for t in elems.iter() {
                    s.push(' ');
                    s.push_str(&t.render_name(ncx));
                }
                s.push(')');
                s
            }
            // A list renders as `(List Elem)` — the element type is its only type parameter.
            Ty::List(elem) => format!("(List {})", elem.render_name(ncx)),
            // A map renders as `(Map Key Value)` — its two type parameters, key first (the corpus `(:
            // (map (1 10) (2 20)) (Map Int64 Int64))` form). The key SET is runtime data, not the type.
            Ty::Map(k, v) => format!("(Map {} {})", k.render_name(ncx), v.render_name(ncx)),
            // A set renders as `(Set Elem)` — its one element type parameter (the corpus `(: (Set.of …)
            // (Set Int64))` form).
            Ty::Set(elem) => format!("(Set {})", elem.render_name(ncx)),
            // Bytes renders as the bare type name `Bytes` (its VALUES render `b"…"`, but the type
            // annotation is the name — the corpus `(: b"…" Bytes)` form).
            Ty::Bytes => "Bytes".to_string(),
            // A sum renders as its NOMINAL NAME, applied to its type ARGS when generic: a monomorphic
            // sum (`args: []`) is the bare name (`(: (Neg unit) Sign)`); a generic sum is `(Name arg…)`
            // — `(: (Some 5) (Option Int64))` (`type-system.md §158`; the corpus form). The variant set
            // is not part of the rendered type (a match reads it from `db.type_decls`).
            Ty::Sum { decl, args, .. } => {
                // The declared name is recovered from `decl` via the render-context (no longer carried on
                // the type — identity is `decl + args`). A synthesized/unresolved `decl` with no declared
                // name falls back to `<sum>` (never panics; a render is always a definite string).
                let name = ncx.name_of(*decl).unwrap_or("<sum>");
                if args.is_empty() {
                    name.to_string()
                } else {
                    let mut s = format!("({name}");
                    for a in args.iter() {
                        s.push(' ');
                        s.push_str(&a.render_name(ncx));
                    }
                    s.push(')');
                    s
                }
            }
            // A nominal renders as its declared NAME (`(: (Mk 42) UserId)`) — its identity is the name,
            // not its underlying shape (`type-system.md §A Nominal Type's Identity Is Its
            // Fully-Qualified Name`). A GENERIC nominal is the name APPLIED to its type args (`(Box a)`,
            // `(Box Int64)`) — the same `(Name arg…)` shape the `Ty::Sum` arm above uses and that #7380 /
            // `render_ty` show on the host query/hover/exports surfaces; rendering the args here keeps the
            // ERROR-message vocabulary consistent with those surfaces (a user seeing `(Box a)` in a hover
            // but bare `Box` in an error is a needless inconsistency). A monomorphic nominal (`args: []`)
            // is the bare name. Recovered from `decl`; a nameless synth decl falls back to `<nominal>`.
            Ty::Nominal { decl, args, .. } => {
                let name = ncx.name_of(*decl).unwrap_or("<nominal>");
                if args.is_empty() {
                    name.to_string()
                } else {
                    let mut s = format!("({name}");
                    for a in args.iter() {
                        s.push(' ');
                        s.push_str(&a.render_name(ncx));
                    }
                    s.push(')');
                    s
                }
            }
            Ty::Fn(p, r) => format!("(-> {} {})", p.render_name(ncx), r.render_name(ncx)),
            // A reified continuation renders as `(Cont resume answer)` — the type a stored/escaping `k`
            // would be named. Compile-time-only until the step-3 heap rep lands; a name for diagnostics.
            Ty::Cont { resume, answer } => {
                format!(
                    "(Cont {} {})",
                    resume.render_name(ncx),
                    answer.render_name(ncx)
                )
            }
            // A quantity renders as `(Qty <inner> <unit>)` — the corpus form `(: (Qty.of 5.0 meter) (Qty
            // Float64 (Unit.base #"meter")))`. The inner type renders as its ordinary name; the unit
            // renders via `Unit::render` (the canonical written form so the type re-reads to the same
            // unit).
            Ty::Qty { inner, unit } => {
                format!("(Qty {} {})", inner.render_name(ncx), unit.render())
            }
            Ty::Type => "Type".to_string(),
            // An UNSOLVED type variable — a payload/element type inference has not pinned (`(Result Int64
            // _)`, the error type of a bare `(Ok 1)`). Render it as `_`, the placeholder rustc uses for an
            // unknown type ("a value of type `Result<i32, _>`"), NOT the internal `?{n}` — the `n` is a
            // nondeterministic solver-assigned number that means nothing to the author and reads as an
            // internal-detail leak (the naive-HM leak the reporting discipline forbids). `_` is the stable,
            // meaningful "not determined here" placeholder, identical wherever an unsolved var surfaces.
            Ty::Var(_) => "_".to_string(),
            Ty::Any => "Any".to_string(),
        }
    }

    /// Like [`render_name`](Self::render_name), but renders each DISTINCT free type variable with a stable
    /// letter NAME (`a`, `b`, `c`, …) drawn from `names` (a var-number → letter map), so a reader sees which
    /// `_`s are the SAME quantified variable and which differ. `render_name` collapses every `Ty::Var` to a
    /// bare `_`, which is right for a diagnostic (an unknown type is just "some type") but HIDES the tie
    /// structure of a generic signature: `from-list`'s tied `(-> (List a) (Iter a))` and a broken
    /// `(-> (Iter a) (Iter b))` both print `(-> (Iter _) (Iter _))`, indistinguishable. Used by the
    /// scheme-aware [`Scheme::render_scheme`] (the `cdz type` surface), NOT by diagnostic messages — those
    /// keep the stable `_`. A var absent from `names` (should not happen for a well-formed scheme) falls
    /// back to `_`. Every non-`Var` arm delegates to the same shape `render_name` produces.
    pub(crate) fn render_named_vars(
        &self,
        names: &std::collections::BTreeMap<u32, String>,
        ncx: &NameCtx,
    ) -> String {
        match self {
            Ty::Var(n) => names.get(n).cloned().unwrap_or_else(|| "_".to_string()),
            Ty::Record(fields) => {
                let mut s = String::from("(Record");
                for (k, t) in fields.iter() {
                    // Canonical `(: name T)` ascription field (RT3), matching `render_name`.
                    s.push_str(&format!(
                        " (: {} {})",
                        k.name,
                        t.render_named_vars(names, ncx)
                    ));
                }
                s.push(')');
                s
            }
            Ty::Tuple(elems) => {
                let mut s = String::from("(Tuple");
                for t in elems.iter() {
                    s.push(' ');
                    s.push_str(&t.render_named_vars(names, ncx));
                }
                s.push(')');
                s
            }
            Ty::List(elem) => format!("(List {})", elem.render_named_vars(names, ncx)),
            Ty::Map(k, v) => format!(
                "(Map {} {})",
                k.render_named_vars(names, ncx),
                v.render_named_vars(names, ncx)
            ),
            Ty::Set(elem) => format!("(Set {})", elem.render_named_vars(names, ncx)),
            Ty::Sum { decl, args, .. } => {
                let name = ncx.name_of(*decl).unwrap_or("<sum>");
                if args.is_empty() {
                    name.to_string()
                } else {
                    let mut s = format!("({name}");
                    for a in args.iter() {
                        s.push(' ');
                        s.push_str(&a.render_named_vars(names, ncx));
                    }
                    s.push(')');
                    s
                }
            }
            Ty::Fn(p, r) => format!(
                "(-> {} {})",
                p.render_named_vars(names, ncx),
                r.render_named_vars(names, ncx)
            ),
            Ty::Qty { inner, unit } => {
                format!(
                    "(Qty {} {})",
                    inner.render_named_vars(names, ncx),
                    unit.render()
                )
            }
            // A generic nominal renders `(Name arg…)` with each arg through `render_named_vars`, so a
            // scheme surface (`cdz type`) shows the STABLE letter vars (`(Box a)`), not the `_` the
            // `render_name` fallthrough would collapse them to — mirroring the `Ty::Sum` arm above.
            Ty::Nominal { decl, args, .. } if !args.is_empty() => {
                let name = ncx.name_of(*decl).unwrap_or("<nominal>");
                let mut s = format!("({name}");
                for a in args.iter() {
                    s.push(' ');
                    s.push_str(&a.render_named_vars(names, ncx));
                }
                s.push(')');
                s
            }
            // Every scalar / monomorphic-nominal / non-var-bearing arm renders identically to `render_name`.
            _ => self.render_name(ncx),
        }
    }

    /// [`render_name`](Self::render_name) prefixed with the correct indefinite article — `an Int64`, `a
    /// String`, `a UInt8` — for a message that reads "found <this>" / "<this> and <that> are different
    /// types". The article is keyed off the type's SOUND, not merely its first letter: among the names
    /// `render_name` produces, only the signed integers (`Int8`/`Int16`/`Int32`/`Int64`, rendered stem
    /// `Int…`, "eye-nt") begin with a vowel sound and take `an`. `UInt…` and `Unit` begin with a "yoo"
    /// consonant sound and take `a`; every other type name (`Float…`, `Bool`, `Char`, `String`, `Bytes`,
    /// `Symbol`, `BigInt`, `Rational`, a compound `(Record …)`/`(List …)`, a user sum) is a plain
    /// consonant. A naive first-letter a/an would misfire on `UInt8` (would wrongly say "an UInt8"), which
    /// is why this keys on the rendered stem rather than the letter. Vowel-initial USER sum names are rare
    /// enough that defaulting them to `a` is acceptable — this helper serves the scalar/text mismatch
    /// messages where only the built-in scalar names reach it.
    pub fn render_with_article(&self, ncx: &NameCtx) -> String {
        let name = self.render_name(ncx);
        // "an" before a vowel SOUND. The signed ints (`Int…`) and other vowel-initial type names (`Ast`,
        // `Any`, `Option`, `Iter`, `Error`, a user `(type Elephant …)`) take "an"; the EXCEPTIONS are the
        // `U…` names whose leading `u` is the consonant "yoo" sound — `UInt…`, `Unit` — which take "a".
        let vowel_initial = name.starts_with(['A', 'E', 'I', 'O', 'a', 'e', 'i', 'o'])
            || (name.starts_with(['U', 'u'])
                && !name.starts_with("UInt")
                && !name.starts_with("Unit"));
        let article = if vowel_initial { "an" } else { "a" };
        format!("{article} {name}")
    }
}

/// A type SCHEME — a polymorphic type quantified over some type and width variables (`∀ vars. ty`).
/// What an operator's (and later a function's) `Meta.t` denotes. Instantiating a scheme replaces its
/// bound variables with FRESH ones, so each use is independent — the mechanism that makes `+` generic
/// over the integer type and `(id x)` polymorphic. Bound variables are identified by the `Ty::Var` /
/// `Width::Var` numbers appearing in `ty`; `ty_vars`/`width_vars` list which of those are quantified.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scheme {
    pub ty_vars: Vec<u32>,
    pub width_vars: Vec<u32>,
    pub sign_vars: Vec<u32>,
    pub ty: Ty,
}

impl Scheme {
    /// A monomorphic scheme — a plain type with nothing quantified.
    pub fn mono(ty: Ty) -> Scheme {
        Scheme {
            ty_vars: Vec::new(),
            width_vars: Vec::new(),
            sign_vars: Vec::new(),
            ty,
        }
    }

    /// Render the scheme's type with each DISTINCT quantified type variable shown as a stable letter
    /// (`a`, `b`, …) instead of the collapsed `_` [`Ty::render_name`] produces. This makes the TIE
    /// STRUCTURE of a generic signature visible — `from-list`'s tied `(-> (List a) (Iter a))` versus a
    /// broken producer/transformer whose element is untied `(-> (Iter a) (Iter b))` — where both would
    /// otherwise print `(-> (Iter _) (Iter _))`, indistinguishable. Used by the `cdz type` surface (the
    /// `Query::TypeOf` answer), the exact tool for diagnosing a recursive-generic monomorphization tie;
    /// diagnostic MESSAGES keep the stable `_`. Vars are named in FIRST-OCCURRENCE order over the type
    /// (left-to-right), so the naming is deterministic and reads like a written `∀a b. …` signature. A
    /// monomorphic scheme (no free vars) renders byte-identically to `render_name`.
    pub fn render_scheme(&self, ncx: &NameCtx) -> String {
        let mut order = Vec::new();
        self.ty.collect_free_vars(&mut order);
        let mut names = std::collections::BTreeMap::new();
        for (i, &v) in order.iter().enumerate() {
            // a, b, …, z, then a1, b1, … for a signature with >26 distinct vars (never in practice).
            let letter = (b'a' + (i % 26) as u8) as char;
            let suffix = i / 26;
            let name = if suffix == 0 {
                letter.to_string()
            } else {
                format!("{letter}{suffix}")
            };
            names.insert(v, name);
        }
        self.ty.render_named_vars(&names, ncx)
    }
}

#[cfg(test)]
mod tests {
    use super::Unit;
    use super::{FloatTy, Ty};

    // Regression (v-cdz-smith/v-rb match-arm float-width): a control-flow JOIN of a DEFERRED float literal
    // (`Ty::float()`) with a fixed `Float32` must yield `Float32` — the literal ADAPTS to the fixed width,
    // exactly as a deferred int literal joins to a sibling `Int8`. Without the `Float` arm in `join`, the
    // pair hit `_ => self.clone()` and a deferred-float arm stayed deferred → grounded to the Float64
    // default → an f64-vs-f32 join that emitted INVALID WASM. Two DIFFERENT fixed widths must NOT agree
    // (no silent float-width promotion), so the arms-agree check rejects them.
    #[test]
    fn join_of_a_deferred_float_and_a_fixed_float_adopts_the_fixed_width() {
        let f32 = Ty::Float(FloatTy::fixed(32));
        let deferred = Ty::float(); // a bare float literal's deferred-width type
        assert_eq!(
            deferred.join(&f32),
            f32,
            "deferred ⊔ Float32 = Float32 (order 1)"
        );
        assert_eq!(
            f32.join(&deferred),
            f32,
            "Float32 ⊔ deferred = Float32 (order 2)"
        );
        // Two DIFFERENT fixed widths do NOT agree — the arms-agree check rejects the mix (no silent promotion).
        assert!(
            !Ty::Float(FloatTy::fixed(64)).agrees_with(&f32),
            "Float64 and Float32 must NOT agree (no silent float-width promotion)"
        );
        assert!(
            deferred.agrees_with(&f32),
            "a deferred float agrees with Float32 (its width is not yet fixed)"
        );
    }

    #[test]
    fn render_value_form_escapes_a_base_name_for_the_symbol_literal() {
        // The value form embeds a base name in a `#"…"` symbol literal. `Unit.base` carries a RAW string, so
        // a name with a `"`/`\`/newline must be ESCAPED (the closed set `\n \t \r \\ \"`, matching the
        // canonical printer's `escape_string`) — else it emits an invalid s-expr AND can split the emitted
        // `// cdz-unit[…]` note across lines, breaking the gate harness's string-literal splice. (Copilot
        // review on PR #485.)
        assert_eq!(
            Unit::base("meter").render_value_form(),
            "(Unit.base #\"meter\")",
            "an ordinary name is unchanged"
        );
        assert_eq!(
            Unit::base("me\"ter").render_value_form(),
            "(Unit.base #\"me\\\"ter\")",
            "an embedded quote is escaped"
        );
        assert_eq!(
            Unit::base("a\\b").render_value_form(),
            "(Unit.base #\"a\\\\b\")",
            "a backslash is escaped"
        );
        // A newline in the name would otherwise split the single-line `// cdz-unit` note → escaped to `\n`.
        assert_eq!(
            Unit::base("x\ny").render_value_form(),
            "(Unit.base #\"x\\ny\")",
            "a newline is escaped so the note stays one line"
        );
        // The escape composes through a power and a quotient (each factor's base name is escaped).
        assert_eq!(
            Unit::base("m\"").pow(2).render_value_form(),
            "(Unit.^ (Unit.base #\"m\\\"\") 2)",
            "a powered factor escapes its base name"
        );
    }

    #[test]
    fn render_name_of_a_generic_nominal_applies_its_type_args() {
        // A GENERIC nominal must render `(Name arg…)` in error text — the same `(Box a)` shape #7380 /
        // `render_ty` show on the host query/hover surfaces — not the bare `Name` the arm used to collapse
        // to (the cross-surface inconsistency the concierge routed). Mirrors the `Ty::Sum` arm, which the
        // corpus generic-SUM cases already exercise; this pins the NOMINAL twin, for which no reachable
        // corpus case exists yet.
        use super::{NameCtx, Ty};
        use std::rc::Rc;
        let occ = crate::ast::StructId(0);
        let decls = vec![crate::db::TypeDecl {
            name: "Box".to_string(),
            occ,
            params: vec!["a".to_string()],
            variants: vec![],
            open_tail: None,
            synth: None,
            associated: vec![],
        }];
        let ncx = NameCtx::new(&decls);
        let mk = |args: Rc<[Ty]>| Ty::Nominal {
            decl: occ,
            args,
            inner: Rc::new(Ty::int64()),
        };
        // GENERIC instantiation → `(Box Int64)` (was bare `Box` before this fix).
        assert_eq!(mk(Rc::from([Ty::int64()])).render_name(&ncx), "(Box Int64)");
        // MONOMORPHIC (no args) → bare name, unchanged (a nominal newtype `(type UserId …)`).
        assert_eq!(mk(Rc::from([] as [Ty; 0])).render_name(&ncx), "Box");
        // A free var arg: `render_name` collapses it to `_` (a diagnostic's "some type"); the scheme-aware
        // `render_named_vars` shows the STABLE letter (`cdz type` surface), like the `Ty::Sum` twin.
        let gen_ty = mk(Rc::from([Ty::Var(0)]));
        assert_eq!(gen_ty.render_name(&ncx), "(Box _)");
        let names = std::collections::BTreeMap::from([(0u32, "a".to_string())]);
        assert_eq!(gen_ty.render_named_vars(&names, &ncx), "(Box a)");
    }

    // `is_fully_solved` is the B2 bind-safety gate P1 (core_analysis.rs) — a share whose type is NOT fully
    // solved is excluded from the sharing-aware-emit plan (an unsolved slot valtype → 'no local slot' emit
    // decline). It is an EXHAUSTIVE match over `Ty`, so a NEW `Ty` variant forces an arm choice — this test
    // pins the soundness-critical cases so a future variant (or a refactor) that lets an unsolved type
    // report solved fails HERE, not as a downstream miscompile. Mirrors the `has_free_var`/`has_any` twins.
    #[test]
    fn is_fully_solved_rejects_unresolved_axes_and_vars_recursively() {
        use super::{FloatTy, IntTy, Sign, Ty, Unit, Width};
        // SOLVED leaves + scalars.
        assert!(
            Ty::int64().is_fully_solved(),
            "a Fixed-sign Fixed-width int is solved"
        );
        assert!(Ty::Bool.is_fully_solved(), "Bool is solved");
        assert!(Ty::Unit.is_fully_solved(), "Unit is solved");
        assert!(Ty::String.is_fully_solved(), "String is solved");
        assert!(
            Ty::Float(FloatTy {
                width: Width::Fixed(64)
            })
            .is_fully_solved(),
            "a Fixed-width float is solved"
        );
        // UNSOLVED: a bare `Any`, a free `Var`, or an int/float axis left Deferred/Var.
        assert!(!Ty::Any.is_fully_solved(), "Any is unsolved");
        assert!(!Ty::Var(0).is_fully_solved(), "a free type Var is unsolved");
        assert!(
            !Ty::Int(IntTy::deferred()).is_fully_solved(),
            "a Deferred int axis is unsolved (the Record.with drift-guard shape)"
        );
        assert!(
            !Ty::Int(IntTy {
                sign: Sign::Var(0),
                width: Width::Fixed(64)
            })
            .is_fully_solved(),
            "a Var sign is unsolved even with a Fixed width"
        );
        assert!(
            !Ty::Int(IntTy {
                sign: Sign::Fixed(true),
                width: Width::Var(0)
            })
            .is_fully_solved(),
            "a Var width is unsolved even with a Fixed sign"
        );
        assert!(
            !Ty::Float(FloatTy {
                width: Width::Deferred
            })
            .is_fully_solved(),
            "a Deferred float width is unsolved"
        );
        // RECURSES into container element/field positions — an unsolved element makes the whole unsolved.
        assert!(
            Ty::List(Box::new(Ty::int64())).is_fully_solved(),
            "a list of a solved element is solved"
        );
        assert!(
            !Ty::List(Box::new(Ty::Int(IntTy::deferred()))).is_fully_solved(),
            "a list of a Deferred-int element is unsolved (recurses)"
        );
        assert!(
            !Ty::Tuple(vec![Ty::int64(), Ty::Any].into()).is_fully_solved(),
            "a tuple with an Any position is unsolved (recurses)"
        );
        assert!(
            Ty::Tuple(vec![Ty::int64(), Ty::Bool].into()).is_fully_solved(),
            "a tuple of solved positions is solved"
        );
        // QUANTITY: a `Ty::Qty` is solved IFF its numeric `inner` is — the unit is a compile-time index
        // erased before emission (`lower` strips the Qty to its inner), so it has no bearing on the slot
        // valtype. A Qty over a Deferred-int inner must report UNSOLVED so B2 gate-P1 excludes it (else the
        // slot binding gets an indeterminate valtype → 'no local slot' emit decline); a Qty over a solved
        // inner is bind-safe regardless of the unit. Pinned here so the erased-to-inner invariant can't
        // silently flip a future refactor into admitting an unsolved Qty share.
        assert!(
            Ty::Qty {
                inner: Box::new(Ty::int64()),
                unit: Unit::base("meter")
            }
            .is_fully_solved(),
            "a Qty over a solved inner is solved (the unit is erased, inner fixes the valtype)"
        );
        assert!(
            !Ty::Qty {
                inner: Box::new(Ty::Int(IntTy::deferred())),
                unit: Unit::base("meter")
            }
            .is_fully_solved(),
            "a Qty over a Deferred-int inner is unsolved (recurses to inner — B2 gate-P1 must exclude it)"
        );
    }
}

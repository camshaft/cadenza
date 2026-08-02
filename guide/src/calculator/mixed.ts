/// Mixed-number rendering for the calculator tape — an IMPROPER rational (`47/12`) shown as a mixed
/// number `3 + 11/12`. Operator decision (2026-08-01, Q-b): use the EXPLICIT-PLUS form, NOT a plain-space
/// `3 11/12`. The load-bearing reason is round-trip: `3 + 11/12` is itself VALID Cadenza arithmetic that
/// re-evaluates to `47/12`, so a reader can paste the tape output straight back into the calculator and
/// get the same value. `check-calculator.mjs` pins exactly that (the render is fed back through the
/// compiler and asserted equal to the original rational), so this transform can never drift into a form
/// that doesn't re-parse.
///
/// Pure + react-/wasm-free (like letChain.ts / classify.ts) so `node --test` can cover it directly.
///
/// Scope (deliberately narrow): only a bare rational — the WHOLE display string matching `-?n/d` — is
/// rewritten. A quantity (`47/12 meter`, has a space + unit), a bare integer (`42`, no `/`), a tuple, or
/// any other compound is left untouched, so the transform can never mangle a value it doesn't fully
/// understand. A PROPER fraction (`1/3`, `-5/8`) is already in its simplest readable form and is returned
/// unchanged; only an improper one (|numerator| > denominator) splits.

/// A bare rational as the compiler's display surface renders it: `n/d` with the sign (if any) on the
/// numerator and the denominator positive (a rational is always in lowest terms). No spaces.
const BARE_RATIONAL = /^(-?\d+)\/(\d+)$/;

/// Rewrite an improper bare-rational display into the explicit-plus mixed form; pass anything else
/// through unchanged. Uses BigInt so an arbitrary-precision rational (rationals are exact, not i64) never
/// loses digits.
///
/// The split is a single SIGNED formula — `whole = trunc(n/d)`, `rem = n - whole*d` — so the negative
/// case falls out symmetric with the positive one and needs no special-casing:
///   47/12  → whole 3,  rem 11  → `3 + 11/12`
///  -47/12  → whole -3, rem -11 → `-3 + -11/12`   (= -3 + (-11/12) = -47/12, still valid + pasteable)
/// Both re-parse to the original rational (verified in check-calculator.mjs).
export function toMixed(display: string): string {
  const m = BARE_RATIONAL.exec(display);
  if (!m) return display; // not a bare rational (quantity / integer / tuple / …) — leave it alone

  const n = BigInt(m[1]);
  const d = BigInt(m[2]);
  // A zero denominator is not a rational the compiler ever emits (a rational is in lowest terms with a
  // positive denominator), but `toMixed` is exported and called from scripts/tests on arbitrary strings,
  // so stay TOTAL: pass `n/0` through unchanged rather than dividing by zero.
  if (d === 0n) return display;
  // A whole rational (`5/1`) or a proper fraction (|n| < d, e.g. `1/3`) is already in its clearest form.
  // (In lowest terms d only divides n when d === 1, so the d === 1 guard also covers the whole case.)
  const abs = n < 0n ? -n : n;
  if (d === 1n || abs < d) return display;

  const whole = n / d; // BigInt division truncates toward zero
  const rem = n - whole * d; // carries the same sign as n, so `-3 + -11/12` renders symmetrically
  return `${whole} + ${rem}/${d}`;
}

# PR review comment — mirrored from GitHub PR #455 (Copilot inline)

- **PR:** #455 (MERGED)
- **File:** `spec/semantics/22-property-based-testing.sexp:111`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3593216199
- **Link:** https://github.com/camshaft/cadenza/pull/455#discussion_r3593216199

## Comment (verbatim)
> This case asserts a strict order-preservation property (`a < b ⇒ of(a) < of(b)`), but `Float64.of-int` is an IEEE `i64 → f64` conversion and is only monotonic *non-decreasing* due to rounding (e.g. adjacent integers beyond 2^53 can map to the same float). The predicate should use `<=` (or explicitly constrain the integer domain) so the stated property matches the conversion's actual behavior.

## Liaison triage — CONFIRMED (property is too strong)
`Float64.of-int` is IEEE i64→f64, which is monotonic NON-DECREASING but not strictly increasing:
two distinct i64 beyond 2^53 round to the SAME f64. So the property `a < b ⇒ of(a) < of(b)` is FALSE
for such inputs — a property-test generator that happens to pick two large adjacent-after-rounding
integers would (correctly) find a counterexample, OR (if the generator's range is small) the test is
vacuously passing while asserting a false property. FIX: use `of(a) <= of(b)` (non-strict), or constrain
the generated integer domain to |n| < 2^53 where the conversion IS strictly monotonic. Property-testing
corpus correctness (v-property-testing / corpus-bugfix). Fix on `trunk`. Quote + link in queue file.

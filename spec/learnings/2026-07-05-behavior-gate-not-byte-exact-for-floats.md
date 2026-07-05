# The behavior gate is not byte-exact for floats, so it cannot catch a canonical-form defect

*2026-07-05*

**What happened.** An adversarial-corpus `/loop` run probed float canonical-form rendering and found a
value-form defect the seed's renderer produces — and, more importantly, that the behavior gate
**structurally cannot catch it**. The seed renders a whole-valued float via `f as i64` (in both
`cdz-compiler`'s `display_float_text` and `cadenza-seed`'s `host::display_float`), and `f as i64`
*saturates*: every whole float at or beyond 2^63 (1e19, 1e20, 1e100, 1.5e300, …) becomes the same string
`9223372036854775807.0`. So distinct float values collapse to one canonical form. Yet the gate reports
these cases PASS — including a case that records `1e20`'s output literally as `1e19` (a demonstrably
different value: `(= 1e19 1e20)` is false). The reason: the gate's float comparison is not byte-exact. It
renders the *recorded* output form through the very same `host::display_float`
(`corpus.rs` `render_value_node` → `host::display_float`) and compares that against the component's output,
which also came from `display_float`. Both sides launder through the same saturating function, so they
agree on a wrong string, and the recorded text is effectively ignored for floats.

**Why.** Two specified requirements are unmet, and they mask each other:
- `contracts/deterministic-value-form.md` §"Numeric Values Serialize Deterministically": "Two
  floating-point values that are not equal under structural equality MUST have distinct canonical byte
  encodings." The `f as i64` renderer violates this — it is not injective over whole floats ≥ 2^63.
- `spec/semantics/README.md` states a case's expected output is **byte-exact** ("a case's expected output
  is byte-exact"; "Determinism is part of the check. A case's output is byte-exact"). The gate does not
  compare byte-exact: it re-derives both sides through the implementation's own float renderer, so a
  case cannot pin a float's canonical text independently of the implementation. A conformance gate that
  compares the implementation against itself cannot witness a canonical-form requirement — this is the
  "a modeled subsystem passes a shape check" failure mode this project already learned once
  ([2026-07-02-a-modeled-subsystem-passes-a-shape-check.md](./2026-07-02-a-modeled-subsystem-passes-a-shape-check.md)),
  recurring at the value-comparison layer.

The existing corpus case "a large whole-valued float renders its full value, not an integer saturation"
(01-literals.sexp), which records `1e19` → `10000000000000000000.0`, is therefore a **false guard**: it
passes not because the renderer produces that text but because both sides produce the same *saturated*
text, whatever it is. The case reads as coverage of the anti-saturation requirement while providing none.

**The requirement it drove.** Two separable changes, and BOTH LANDED 2026-07-05 (in the order that keeps
the gate honest: renderer first so the gate hardening turns nothing red):
1. **Fix the renderer (DONE).** The whole-float branch of both `codegen::display_float_text` and
   `host::display_float` now renders `format!("{f:.0}.0")` instead of `format!("{}.0", f as i64)`. `{:.0}`
   prints the exact f64 integer value with no fractional digits and is injective across all finite whole
   floats; the `f as i64` cast saturated at `i64::MAX` (`9223372036854775807`), collapsing every whole
   float ≥ 2^63 to one string. The two renderers are kept in lock-step. This restores
   `deterministic-value-form.md` injectivity: 1e19, 1e20, 1e100, 1.5e300 now render to distinct decimals.
2. **Make the gate independent for floats (DONE).** The gate harness (`corpus.rs::compare`) gained a second
   check for a float scalar output that must hold ALONGSIDE the render-string equality:
   `float_output_round_trips(form, observed)` requires the observed text to `parse::<f64>()` back to the
   recorded f64 *bit-identically* (NaN self-equal). `parse` is the inverse of the renderer's `format`,
   computed by DIFFERENT code, so it discharges the canonical-form requirement WITHOUT testing the renderer
   against itself — a saturating renderer emits `9223…807.0`, which parses to 9.22e18 ≠ 1e19 and fails the
   check even though both sides of the string compare agreed. This was chosen over "store the raw source
   text and compare verbatim" because parse-round-trip needs no reader change and tolerates
   representation-equivalent spellings (`1e19` vs `10000000000000000000.0`) while still being injective.
   The check is vacuously true for non-float / compound outputs (already byte-exact via the string compare).

The former false-guard case ("a large whole-valued float renders its full value, not an integer
saturation", 01-literals.sexp) is now a REAL guard — the round-trip oracle backs it, so a regression to a
saturating (or otherwise non-injective) renderer turns it RED. The general lesson stands and generalizes
beyond floats: **a canonical-form / injectivity requirement cannot be discharged by comparing two outputs
of the same function; the gate needs an oracle computed by an independent path (here, the parse inverse).**

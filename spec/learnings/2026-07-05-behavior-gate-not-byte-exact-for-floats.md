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

**The requirement it drove.** *Deferred to a clarity/gate-fix pass* (this entry is the hand-off, per the
operator's practice of documenting gaps for a follow-up agent). Two changes are needed and they are
separable:
1. **Fix the renderer** (implementation, tracked in the memory bug note
   `quote-vs-ast…`-style entry `float-render-saturates-…`): render a whole float without the `f as i64`
   cast — e.g. `format!("{:.0}", f)` with the `.0` suffix, which prints the exact f64 integer value and
   is injective — in both `codegen::display_float_text` and `host::display_float`, keeping the two in
   lock-step. This restores `deterministic-value-form.md` injectivity.
2. **Make the gate byte-exact for floats** (gate harness): compare a float output against the case's
   recorded *literal text* rather than re-rendering the recorded form through the implementation's own
   `display_float`. Only an independent comparison can discharge a canonical-form requirement — otherwise
   the gate tests the renderer against itself. A candidate: store the recorded float form's raw source
   text at parse time and compare it to the component's rendered output verbatim (with a defined
   normalization the spec pins), or have `deterministic-value-form.md` fix the exact float grammar and
   compare against that grammar rather than a Rust format call.

Until both land, the float canonical-form requirement is unwitnessed and the saturation defect is latent.
No corpus case can currently express it — a case recording the correct full-decimal form passes anyway,
because the gate laminates both sides through the buggy renderer.

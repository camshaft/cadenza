# Decision — Numeric Model

**The decision.** The concrete integer widths and overflow behavior, floating-point mode, and exact
and rational representations that realize the numeric behavior the specification states
technology-neutrally (numeric-model.md capability; deterministic-value-form.md; determinism-and-fuel.md).

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Numeric types do not silently promote; overflow is defined; exact arithmetic is exact
  (numeric-model.md).
- Floating-point follows the determinism contract (determinism-and-fuel.md).
- Numeric values serialize under the canonical value form (deterministic-value-form.md).

Because these choices fix bytes that cross the boundary and enter the canonical value form, a change
to a chosen byte form is an ABI-level change under the constitution's Governance Floors.

## Choices

- [`explicit-checked`](./explicit-checked.md) — checked-and-trapping signed-64 default integer,
  distinct wrapping and unsigned types, opt-in big-integer and normalized rational, deterministic
  binary64 float, no implicit promotion. **The default.**

DEFAULT: explicit-checked

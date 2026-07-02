# Decision — Diagnostics Schema

**The decision.** The concrete machine-readable diagnostic record that realizes the diagnostics.md
capability's requirements that every diagnostic carry a stable code, a precise span, and the rule it
enforces.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Every diagnostic has a stable machine-readable code, a precise span, and a rule reference
  (diagnostics.md; constitution XI).
- Diagnostics are emitted in a deterministic order (diagnostics.md).

Diagnostics are a compile-time and tool-time output, not part of a derived component, so tuning this
schema does not alter any component's bytes.

## Choices

- [`coded-span-record`](./coded-span-record.md) — a diagnostic record of code, severity, span (line/
  column/offset), message, rule reference, and related spans, serialized in the canonical encoding.
  **The default.**

DEFAULT: coded-span-record

# PR #1213 review comment — implementation/compiler-ml/src/emit-db.cdz (v-compiler-ml)

Mirrored from https://github.com/camshaft/cadenza/pull/1213 (PR: "cand: v-compiler-ml — 0329e04d6").

## Trap message drops the `op` value (Copilot, emit-db.cdz:650, also :655) — diagnostics
> The trap message drops the actual `op` value, which makes failures harder to diagnose (you can't
> tell which opcode declined). Include the `op` in the message using the existing `int-to-decimal`
> helper in this file.

Diagnostic quality: when this trap fires you can't tell which opcode declined. There's already an
`int-to-decimal` helper in the file — interpolate `op` into the trap message so a failure names the
offending opcode.

# Vertical-ready: quantity reference-normalized storage + `as/in` unwraps

**Design landed:** `implementation/design/DESIGN-quantity-reference-normalized-unwrap.md`
(committed `48dd8bb3` on `fleet/design-qty-unwrap`, merge-request sent to pr-sync 2026-07-15).

**Subsystem:** `rcdzc` (core: infer.rs / lower.rs / ty.rs render), plus `spec/` + corpus, plus
`cadenza-syntax` (the `as` surface keyword) and the `cdz-calc` display. A units-familiar `vertical`
agent should own it top-to-bottom.

**The model (operator-locked):**
1. A stored `Qty` is ALWAYS at the dimension's reference unit — no side-carried scale factor. Named
   non-reference units (`kilometer`/`foot`/`mbps`/`KiB`/prefixed) are construction SUGAR applied once
   at `Qty.of` (`magnitude × scale @ reference`).
2. `as/in <unit>` UNWRAPS to a bare dimensionless number (converts + strips the Qty), not a re-expressed Qty.
3. Mixed-unit combine result = reference unit (unchanged, now the only stored unit).

**First increment (Q1 — start here):** spec-first. Revise `spec/capabilities/units-of-measure.md`,
`options/units-of-measure/erased-compile-time-quantity.md`, and `spec/semantics/18-units-of-measure.sexp`
to the new contract (the `Unit.in` outputs become bare `(: N T)`; prefixed constructions display their
scaled reference magnitude). Then Q2 = eager-normalize `QtyOf` (fixes the reported calc relabel bug),
Q3 = `Unit.in` → bare, Q4 = `as` keyword, Q5 = calc render reuse + `* 1` unit preservation. Full plan +
seams in the design doc §3/§5.

**⚠ This REVISES landed passing behavior** (not a pure addition): the `Unit.in` return type (Qty→bare)
and prefixed-construction stored magnitudes change. Every corpus flip must be an intentional output
change with a matching spec edit — never a silent todo→fail. Spec-first (Q1) before the compiler moves.

**Coordinate:** Q2 also discharges the queued bugfix
`mlrepro-calc-bare-quantity-relabels-to-base-without-scaling.md` (that bug is the SYMPTOM; this is the
root-cause fix). PM: hold the narrow bugfix pending this vertical, OR ship the display-only patch first
if the calc must be correct sooner — don't double-land. See design doc §6.

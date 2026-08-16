# Open-row projection x effect dispatch (2026-08-10)

Angle: grep found ZERO overlap between 15-rows open-row projectors and 14-effects
dispatch — an uncovered seam. The projector's per-call-site slot resolution must
survive draws/arm-built values/record states.

All GREEN x3, hand-modeled first:
- or1: one open-row projector at TWO record widths, each row holding a fresh DRAW
  (slot resolution per instantiation while state advances) — 403/100
- or2: the ARM builds the record (op returns (Record (: x)(: t))), body projects it
  open-row at the 1-field width AND reads t — arm-built rows cross dispatch — 6053020/1003020
- or3: RECORD handler state projected open-row INSIDE the arm (arm's instantiation
  independent of body's), state row also rebuilt each dispatch — 113013/110010

Vocab: an op's RESULT TYPE spelling is `(Record (: x Int64))` (capital, type form) —
`(record ...)` (value form) in a signature parses as a record VALUE and errors
CDZ0201 "cannot apply a value of type (Record (: apply Any)...)".

No counterexample. Strong pin candidates (novel seam).

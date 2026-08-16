# List-of-lists handler state (2026-08-11)

Angle: (List (List Int64)) never crosses a dispatch in 14-effects (rows appended
per dispatch, two-level element reads).

GREEN x3 (pin candidate):
- ll1: each add dispatch pushes a fresh 3-element row; pick reads [i][j] through
  nested Option matches in the arm; drain reads middle/last/first rows — 400800/100200

DECLINE FENCE FOUND (match-chain vs let-chain sequencing):
- The SAME program sequenced with (match (Rows.add n) (_ ...)) chains DECLINES
  ("not yet reducible") at 3+ dispatches — with FLAT list state too, and
  regardless of arm mix (3 adds alone decline; 2 adds compile). Sequenced with
  LET chains it folds at any count. The wildcard-match discard chain is a
  SEQUENCING FORM the fold only tracks to depth 2; let-binding is the general
  form. (Landed pins mostly use match-chains at depth <= 2 or let-chains —
  consistent.) Banked for v-effects.

NOTE: my earlier corpus pins (bp*/mm1 etc.) use match-chains at exactly the
depths that fold — the fence explains why they were green.

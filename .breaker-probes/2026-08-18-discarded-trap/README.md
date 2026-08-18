# Discarded traps are elided (2026-08-18)

Surprise finding while probing trap-in-discarded-replay: a PURE trapping
form in DISCARD position never traps — uniform x3.

- `dsc1.sexp` — the control, no effects at all: (do (/ 60 (% n 3)) 7)
  returns 7 for BOTH seeds including the zero divisor. The discard law
  (CDZ0307's contract read literally: a valueless pure form "has none"
  [no effect] and is droppable — a trap is not an effect).
- `pyt6.sexp` — the composition: the double-replaying arm's FIRST replay
  runs a body whose division would trap on the zero seed, but that
  replay's value is discarded, the pure trapping tail is elided, and only
  the second replay's quotient survives (5 / 6 — note n=0 returns 60/10=6
  where value-position semantics would trap). The nonzero seed (5=60/11
  via replay-2) shows replay-1 otherwise runs and threads state.

Contrast pin: dbf1 (batch 323) proved discarded-replay EFFECTS still
fire; dsc1/pyt6 prove discarded-replay pure TRAPS do not. Effects are
kept, pure work (including trapping pure work) is droppable. This is
load-bearing spec behavior — worth the corpus pin so any move toward
strict-trap semantics is a deliberate flag-day, not an accident.

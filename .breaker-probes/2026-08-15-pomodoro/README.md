# pom — pomodoro timer: declines + a SECOND COMPILE HANG (2026-08-15, tick 1563)

| probe | shape | verdict |
|-------|-------|---------|
| pom1 | (wcount,brk,sessions) 3-tuple, 2-level cross-field guard, seed cmp in arm | DECLINE ×3 |
| pom2 | 2-tuple phase-merged, same-field 2-level guard, seed cmp in arm | DECLINE ×3 |
| pom3 | pom2 w/ cascade reordered high-to-low | DECLINE |
| pom4 | pom3 w/ seed comparison REMOVED (slen hardcoded) | **PASS** |
| pom5 | slen carried IN STATE (3-tuple), no n in arm | **COMPILE HANG** (gate timeout) |

Two findings in one family:
1. The pom2/pom3-vs-pom4 pair isolates a decline requiring the seed-equality
   guard `(= (+ phase 1) (+ (% n 3) 2))` inside a 3-level cascade — the
   n-compound in a mid-cascade equality is the delta (pom4 hardcodes 3 and
   passes).
2. pom5 — moving the seed compound into STATE (the natural fix!) — HANGS the
   compiler like cmb1. Second hang witness, different shape: 3-level
   same-field cascade + equality against a STATE field at 8 dispatches.

The natural-repair path (hoist seed to state) walks into the hang — worth
flagging on the cmb1 issue.

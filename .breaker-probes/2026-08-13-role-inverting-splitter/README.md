# 2026-08-13 role-inverting parity splitter (tick 1404)

- `rsp1.sexp` — (ev, od, flag) state: feed routes by PARITY XOR FLAG (a boolean
  equality of two comparisons selecting the branch), flip inverts the routing
  AND answers the packed pre-flip snapshot. Post-flip, even values land in the
  odd bucket: seed 3 sends 6→od (3+6=9), 3→ev (4+3=7); seed 8 stacks both
  pre-flip feeds in ev (4,12) then 6→od, 3→ev. The routing-inversion protocol
  (a mode bit REINTERPRETING the same op's dataflow) was unpinned; bwa1/pal1
  route by fixed parity. PASS ×3 (40304030907/41212000615).

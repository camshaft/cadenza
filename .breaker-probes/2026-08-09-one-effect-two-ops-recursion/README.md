# #14 SCOPE CORRECTION (tick 1033): not two-EFFECTS - TWO DRAWS PER ROUND forks the out-state

- ra4: ONE effect, TWO OPS drawn per round (next+probe), trailing draw -> 305 vs 329.
  Steps value CORRECT (3), trailing draw reads n (pre-recursion). Out-state dropped.
- ra5: ONE effect, SAME op drawn TWICE per round, trailing draw -> 205 vs 235. Same profile.
- ra6: one let draw + one BARE discarded draw per round -> 205 vs 235. Binding shape immaterial.
- ra3-min1 (single draw per round + trailing): STILL PASSES.
Corrected trigger: [recursive callee performing >=2 draws per ROUND (any op/effect mix)] x [continuation draw].
The two-effect ra3-min2/3 faces are the same bug. 147bd8ef4's one-cell threading covers exactly ONE
perform per recursion body; a second perform in the same body drops the out-state.
NOTE steps ARE correct in ra4/5/6 (the recursion's internal reads are fine) - contrast ra3 where
steps also died; that difference is second-effect-related (B's state feeding the exit test).

## Further scoping (tick 1034)
- ra7: NON-recursive helper with two draws, called once, trailing draw -> GREEN x3-checked-wasm.
  The fork is RECURSION-specific (a plain def body threads multi-perform out-state fine).
- ra8: recursion with ONE draw per non-exit round, TWO draws only at the EXIT LEAF -> GREEN.
  Per-round multi-perform is the trigger; a multi-perform exit leaf threads correctly.
Refined trigger: [>=2 draws on the RECURRING path of a recursive callee] x [continuation draw].

## Mutual face (tick 1035)
- ra9: MUTUAL pair each drawing once (2 draws/cycle), trailing draw -> DECLINES (honest todo).
  The mutual floor (row-mr) shields the mutual version of this shape - no silent fork there.
  #14 is confined to SELF-recursion with >=2 draws on the recurring path.
- ra10: multi-draw walk on E, trailing read on UNTOUCHED nested B -> GREEN (208/204).
  B's thread + the steps result both correct; ONLY the multi-drawn effect's own out-state is stale.
  The fork is CELL-SPECIFIC: exactly the cell that multi-performed per round loses its advances.

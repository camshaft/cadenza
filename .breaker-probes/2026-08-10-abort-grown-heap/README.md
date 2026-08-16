# Abort reading state grown by earlier same-handler dispatches (2026-08-10)

Angle: the landed heap-abort pins read the state at the SEED (Map.empty/list at
dispatch 1); an abort AFTER resumptive same-handler dispatches (reading the
GROWN state) was unpinned.

GREEN x3 (pin candidate):
- ag3: ONE resumptive put then halt — the abort arm reads the advanced scalar
  state (100*s), continuation (+7777 _) discarded — 300/0/-400

DECLINE FENCE (staged, honest — "not yet reducible by the tail-resumptive fold"):
- TWO+ resumptive dispatches before the abort decline, scalar or heap alike
  (/tmp/ag-one compiles, /tmp/ag-two declines; ag1 halt-op + ag2 conditional-
  abort with Map state also decline at 3 puts). The fence is the resumptive-
  dispatch COUNT preceding an abort on the same handler: 1 folds, 2+ declines.
  Width-independent of state shape. Banked for v-effects' fold frontier.

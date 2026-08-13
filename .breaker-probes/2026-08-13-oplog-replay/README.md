# 2026-08-13 op-log replay (tick 1431)

- `rpl1.sexp` — (value, log) state: apply advances AND logs its delta; replay
  folds the WHOLE log onto the current value, keeping the log intact — so a
  second replay after another apply compounds (log [3,4,1] re-applied on top of
  the already-replayed value). vs ldd1 (tag-filtered ledger read) and ckp1
  (snapshot copy): the log here is EXECUTABLE history whose re-application
  composes with the live value; the kept-log semantics distinguish it from
  a flush. PASS ×3 (307141523/812192028).

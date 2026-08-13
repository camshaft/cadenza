# 2026-08-13 idempotent-undo counter (tick 1379)

- `und1.sexp` — state (value, last-delta): apply adds and records the delta;
  undo reverses the LAST delta exactly once (answers it, zeroes the record),
  the SECOND undo answers 0 and threads unchanged. A negative delta (-2) is
  the undone one, so the undo answer is negative (sign-offset packing).
  Traffic-light FSM angle was coverage-killed (three-phase machine at 14b:8507);
  undo-with-idempotence is a fresh protocol shape: the arm's behavior depends on
  a FLAG FIELD it itself cleared one dispatch earlier. PASS ×3
  (81608115 / 253308132).

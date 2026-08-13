# 2026-08-13 length-prefixed frame protocol (tick 1394)

- `frm1.sexp` — a Bytes state as a FRAME QUEUE: two writer ops append
  [len, payload...] frames (widths 2 and 1), popf reads the length PREFIX,
  sums exactly that many body bytes (recursive bounded walk), and re-slices the
  REST as the next state; the drained pop answers -1. Composes: length-driven
  bounded reads + Bytes.slice rest-carving + multi-frame interleave through the
  thread. Draft trap (my own documented gotcha, hit anyway): Bytes.slice takes
  (start, LENGTH) not (start, end) — the wrong 3rd arg shifted every later
  frame; the gate's differential caught it (both seeds wrong the same way).
  PASS ×3 (3572911/3842911).

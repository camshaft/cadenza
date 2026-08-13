# 2026-08-13 UTF-8 mid-run validity flip (tick 1390)

- `utf1.sexp` — the THREADED Bytes state goes UTF-8-INVALID mid-run: a bare lead
  byte (0xC3) flips String.from-bytes to None, the completion byte (0xA9, seed 0)
  restores Some, a bad continuation (0x41 'A', seed 1) leaves it None. The landed
  from-bytes arm pins validate op ARGUMENTS one-shot; the validity of the
  ACCUMULATED STATE flipping across dispatches (Some→None→Some/None) is the new
  face. Model correction: my python used scalar count for the final decode —
  String.byte-len counts BYTES ("abé" = 4, answer 41 not 31); compiler agreed ×3
  and the model was fixed. PASS ×3 (21300441/21300400).

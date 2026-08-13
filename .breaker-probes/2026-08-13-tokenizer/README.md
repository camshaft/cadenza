# 2026-08-13 tokenizer protocol (tick 1413)

- `tok1.sexp` — next-tok: recursive skip (spaces) + scan (to space/end) over
  String.at, answers token byte-len, threads the REST via String.slice(j, END);
  multi-space runs and the drained-stream zero edge covered (trailing tok on
  seed 1 = 'w' len 1... rows +1-packed). Two seeds with different space patterns.
  Draft trap (the MIRROR of yesterday's frm1 trap): String.slice takes
  (start, END) — I wrote (start, LEN) this time; both backends agreed against
  the model, and a byte-len-of-rest debug probe exposed the shrinking windows.
  Slice-semantics pair: String=(start,END), Bytes=(start,LEN). PASS ×3 (3421/2321).

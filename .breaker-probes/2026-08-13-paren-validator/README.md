# 2026-08-13 paren-depth validator (tick 1440)

- `prn1.sexp` — (depth, bad) state: "(" and ")" chars (compared by string
  equality on 1-char strings crossing the op boundary) move the depth; an
  UNDERFLOW flips the sticky bad flag whose every later answer is -9 — the
  post-underflow "(" cannot revive the stream. Seed picks the stream: "(()"
  balanced-prefix (depths 1,2,1 then 2) vs "())" underflowing at char 3.
  String-classification chars-as-args + sticky-flag composition (stk2's sticky
  face is numeric-threshold; here it's a STRUCTURAL validity property).
  PASS ×3 (11121112/11100101).

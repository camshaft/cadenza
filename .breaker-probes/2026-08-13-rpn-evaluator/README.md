# 2026-08-13 RPN evaluator (tick 1437)

- `rpn1.sexp` — a stack machine over the effect protocol: push answers depth,
  addop/mulop read the top two (top2 via two len-relative List.at), rebuild the
  stack without them (drop2 prefix-copy walk), and push the result. The body
  runs the RPN program `n 3 + 2 *`; answers expose depths and intermediates
  ((n+3) then (n+3)*2). Two operator arms sharing helper defs; pop-two-push-one
  differs from cst1's cursor (single-cell) and pq1's drop-at (single removal):
  this removes a SUFFIX and appends. PASS ×3 (1207214/1213226).

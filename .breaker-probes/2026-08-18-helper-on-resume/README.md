# Recursive pure helper on the resume result (2026-08-18)

- `pyh1.sexp` — (dsum (resume s (+ s 3))) + hundredfold state toll, where
  dsum is a RECURSIVE pure def (digit sum). The helper recursion runs
  during the unwind on a value that does not exist until the tail
  completes — call-position over resume (function application, not a
  connective or binder), extending the post-resume ladder: arithmetic
  (pyr1/2), if (pyr4), match literal (pyr5), let binder (pyr3 fixed),
  match binder (pyr7 fixed), and now a recursive CALL. Seeds drive the
  two frames' digit-sums through different collapse depths (109: dsum(41)
  =5 +300 -> dsum(305)=8 +100... model: 109 / 6). PASS x3 at 2794fdc43
  (post-e6eb3831b trunk).

(case "eval of a quoted HUGE integer literal declines CDZ0201 — BigInt storage does not widen eval"
  (doc    "The huge-leaf face of the eval-width boundary (see 'eval of a quoted integer literal grounds
           to Int64 (BigInt is AST storage, not eval width)' — the small-literal twin): a quoted
           26-digit literal is stored losslessly (see 'an Ast.Int carries a BEYOND-64-bit literal
           losslessly through quote'), but `eval` re-infers at ordinary width, so the huge leaf
           overflows Int64 and REJECTS with CDZ0201 (naming the BigInt annotation escape hatch)
           rather than truncating. Pins the Part-1 storage-only scope: lossless in the AST, ordinary
           width in eval.")
  (input  (do
            (def (main) (eval (quote 99999999999999999999999999)))
            (export main)))
  (error  CDZ0201))

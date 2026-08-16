(case "print renders a HUGE Ast.Int losslessly — the full 26-digit decimal, no truncation"
  (doc    "The print face of the storage-vs-eval-width family: `print` of a BigInt-annotated Ast.Int
           renders the STORED value's exact decimal text (26 digits), unlike `eval` which re-infers
           at Int64 and declines (the huge-leaf eval case beside this one). Print reads the storage,
           so it inherits the losslessness of 'an Ast.Int carries a BEYOND-64-bit literal losslessly
           through quote'.")
  (input  (do
            (def (main) (= (print (Ast.Int (: 99999999999999999999999999 BigInt))) "99999999999999999999999999"))
            (export main)))
  (output (: true Bool)))

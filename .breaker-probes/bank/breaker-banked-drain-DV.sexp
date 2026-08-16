(case "a branch-SELECTED Ast value splices into a template compared structurally"
  (doc    "The RUNTIME-selected operand face of the computed-splice family (:3274ff pin let-bound and
           param-bound operands — both statically-known subtrees): here the spliced Ast ARRIVES
           through an if-join of two different quote values, so the graft site receives whichever
           subtree the runtime branch chose — verified structurally against the directly-written
           expected tree per branch (1/1). A splice resolution keyed to a statically-unique operand
           (or a template fold that specialized to one branch's subtree) breaks the other branch.
           (The EVAL of the same shape declines ×3 — eval is const-only; banked as a TODO flip.)")
  (input  (do
            (def (main (: b Bool))
              (let ((node (if b (quote (+ 1 2)) (quote (* 3 4)))))
                (if (= (quasiquote (+ 100 (unquote node)))
                       (if b (quote (+ 100 (+ 1 2))) (quote (+ 100 (* 3 4)))))
                  1 0)))
            (export main)))
  (call   main (: true Bool)) (output (: 1 Int64))
  (call   main (: false Bool)) (output (: 1 Int64)))

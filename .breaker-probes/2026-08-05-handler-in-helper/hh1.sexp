(case "hh1 a HANDLE inside a helper fn called from another handle's body (handler-in-callee composition)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (inner-unit (: n Int64))
              (handle B (* n 10)
                ((b (u) t (resume t t)))
                (+ (B.b) 1)))
            (def (main (: k Int64))
              (handle A k
                ((a (u) s (resume s s)))
                (+ (A.a) (inner-unit (A.a)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))

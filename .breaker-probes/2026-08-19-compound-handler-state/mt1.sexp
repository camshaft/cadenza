(case "mt1 a handler state of TWO tries (tuple) updates each independently across resumes"
  (input  (do
            (effect Tw (op addl (-> Int64 Int64)) (op addr (-> Int64 Int64)) (op sizes (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Tw (tuple Map.empty Map.empty)
                ((addl (v) s (match s ((tuple l r) (resume 0 (tuple (Map.insert l v v) r)))))
                 (addr (v) s (match s ((tuple l r) (resume 0 (tuple l (Map.insert r v v))))))
                 (sizes (u) s (match s ((tuple l r) (resume (+ (* 10 (Map.len l)) (Map.len r)) s)))))
                (do
                  (Tw.addl 1)
                  (Tw.addl 2)
                  (Tw.addr 10)
                  (Tw.sizes))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 21 Int64)))

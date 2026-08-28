(do (effect e (op o (-> Unit Unit))) (effect e2 (op p (-> Unit Unit))) (def (main) (host (e e2) (do (e.o) (e2.p)))) (export main))

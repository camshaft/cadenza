(case "nc1 THREE performs nested in one expression — each op's result is the next op's argument, three distinct arms and strides"
  (input  (do
            (effect S (op a (-> Int64 Int64)) (op b (-> Int64 Int64)) (op c (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((a (v) s (resume (+ v s) (+ s 1)))
                 (b (v) s (resume (* v 2) (+ s 10)))
                 (c (v) s (resume (- v s) (+ s 100))))
                (+ (S.c (S.b (S.a 5))) (* 10000 (S.a 1)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1150002 Int64))
  (call   main (: 0 Int64)) (output (: 1119999 Int64)))

(case "lr3 the REST binder resumes OUT of one dispatch and re-crosses INTO a second — the rest view survives two boundary crossings"
  (input  (do
            (effect S (op grab (-> (List Int64) (List Int64))) (op sum (-> (List Int64) Int64)))
            (def (sum-l (: xs (List Int64)) (: acc Int64))
              (match xs
                ((list h .. t) (sum-l t (+ acc h)))
                (_other acc)))
            (def (main (: n Int64))
              (handle S 0
                ((grab (xs) s
                  (match xs
                    ((list _a .. r) (resume r s))
                    (_other (resume xs s))))
                 (sum (ys) s (resume (sum-l ys 0) s)))
                (S.sum (S.grab (list n 10 20 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64))
  (call   main (: 0 Int64)) (output (: 60 Int64)))

(do (type R (Ok Int64) (Err Int64))
    (def (f (: x (Option R))) (match x ((Some (Ok v)) v) ((Some (Err e)) (- 0 e)) ((None _u) -99)))
    (def (main (: n Int64)) (f (if (> n 0) (Some (Ok n)) (if (< n -10) (None) (Some (Err (- 0 n)))))))
    (export main))

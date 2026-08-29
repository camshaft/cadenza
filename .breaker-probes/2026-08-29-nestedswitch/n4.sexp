(do (def (f (: oo (Option (Option Int64)))) (match oo ((guard (Some (Some v)) (> v 5)) 100) ((Some (Some v)) v) ((Some (None _u)) 1) ((None _u) -1)))
    (def (main (: n Int64)) (f (if (> n 0) (Some (Some n)) (if (< n -10) (None) (Some (None))))))
    (export main))

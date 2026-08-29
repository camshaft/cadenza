(do (def (f (: o (Option (Option (Option (Option (Option Int64))))))) (match o ((Some (Some (Some (Some (Some x))))) (* x 10)) (_ -1)))
    (def (main (: n Int64)) (f (if (> n 0) (Some (Some (Some (Some (Some n))))) (Some (Some (None))))))
    (export main))

(do
  (type Opt (Some a) (Nil unit))
  (def (get-or (: o (Opt Int64)) (: d Int64)) (match o ((Some v) v) ((Nil _u) d)))
  (def (main (: k Int64)) (+ (get-or (Some k) 0) (get-or (Nil unit) 99)))
  (export main))

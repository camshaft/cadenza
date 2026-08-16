(do
  (def (f (: c (Record (: k Int64)))) (. c k))
  (def (main (: n Int64)) (f (record (k n))))
  (export main))

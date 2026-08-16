(do
  (effect M (op step (-> (Record (: k Int64)) Int64)))
  (def (main (: n Int64))
    (handle M 0
      ((step (c) s (resume (. c k) s)))
      (M.step (record (k n)))))
  (export main))

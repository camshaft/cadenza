; F24 REJECT repro (compiler-ml self-host caught it; db-query-diff.cdz:26): the #st drain
; orphans a MATCH-TUPLE binder (root) used in a performing let-init body, under a growing-state
; handler. CLEAN TRUNK (no F24) = 51; WITH F24 trio = CDZ0101 unbound name root.
(effect Db (op resolve (-> Int64 Int64)) (op typeof (-> Int64 Int64)))
(def (main (: n Int64))
  (handle Db (list)
    ((resolve (r) s (resume r (List.push s r)))
     (typeof (r) s (resume (List.len s) s)))
    (match (tuple 5 7)
      ((tuple root tree)
       (let ((db (Db.resolve root)))
         (+ (Db.typeof root) (* 10 db)))))))
(export main)

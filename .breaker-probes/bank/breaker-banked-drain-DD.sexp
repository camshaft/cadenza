(case "a record-destructuring def PARAMETER binds a named subset of fields"
  (doc    "The record face of the binding-position pattern grant (:137 + 'a record pattern MAY name a
           subset of the fields, ignoring the rest' :217): `(def (getid (record (id k))) …)` names ONE
           field of a 3-field argument whose OTHER fields carry heap values (a list and a string) —
           the param binds k, keeps single-argument arity, and the unnamed heap fields drop cleanly
           (the def-param twin of the partial-record MATCH pin :15888). 50 at n=5. Together with the
           tuple def-param pin this maps the def-param pattern vocabulary; a record pattern here is
           irrefutable on its type (field presence is static), so the binding position admits it.")
  (input  (do
            (def (getid (record (id k))) (* k 10))
            (def (main (: n Int64))
              (getid (record (id n) (tags (list n)) (name "x"))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))

(case "a record-destructuring LET binder names a subset of a runtime result's fields"
  (doc    "The let-binder twin over a RUNTIME function result: `(let (((record (id k)) (mk n))) …)`
           projects one field out of the returned 2-field record (whose other field is a heap list,
           dropped unbound) → 105 at n=5. The record companion of the nested-tuple-let pin — a
           subset-naming record pattern in let position against a computed value.")
  (input  (do
            (def (mk (: n Int64)) (record (id n) (tags (list n (+ n 1)))))
            (def (main (: n Int64))
              (let (((record (id k)) (mk n)))
                (+ k 100)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))

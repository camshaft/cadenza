(case "Record.pop components with runtime leaves equal their directly-built twins"
  (doc    "The POP face completing the row-op construction-path family (without/with/merge/extend are
           pinned above): `(Record.pop {a:n, b:2, c:30} b)` yields `(tuple <popped> <rest>)` — the
           popped value must be the field's value (tens digit) and the REST record must be
           byte-canonical with the never-had-b record built directly (ones digit) → 11 ∀n. The rest
           component shares the deletion path-copy with Record.without but returns through the tuple
           path — a divergence in the tupled copy breaks equality while projections still read right.")
  (input  (do
            (def (main (: n Int64))
              (let ((p (Record.pop (record (a n) (b 2) (c 30)) b)))
                (+ (* 10 (if (= (. p 0) 2) 1 0))
                   (if (= (. p 1) (record (a n) (c 30))) 1 0))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 11 Int64)))

(case "a project-reached record with runtime leaves equals the directly-built record"
  (doc    "The PROJECT face: `(Record.project {a:n, b:2, c:30} (a c))` restricts to two fields — the
           result must equal the directly-written `{a:n, c:30}` (tens digit) and stay unequal to a
           decoy differing in one kept field's value (ones digit) → 10 ∀n. Projection assembles a NEW
           record from a subset of a runtime record's fields; landing on a different canonical field
           order or copying a stale leaf would flip a leg while `(. r a)` reads still pass.")
  (input  (do
            (def (main (: n Int64))
              (+ (* 10 (if (= (Record.project (record (a n) (b 2) (c 30)) (a c))
                             (record (a n) (c 30))) 1 0))
                 (if (= (Record.project (record (a n) (b 2) (c 30)) (a c))
                        (record (a n) (c 31))) 1 0)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 10 Int64)))

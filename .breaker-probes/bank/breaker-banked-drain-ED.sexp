(case "a RECORD of closures acts as a vtable — fields dispatch distinctly with distinct captures"
  (doc    "The ops-record/vtable idiom: a factory builds a record whose TWO fields are closures
           sharing ONE captured base — each field must dispatch its OWN body through the projection
           (`(. ops add)` vs `(. ops mul)`) while both read the same capture (8·100 + 15 = 815 at
           b=3). A field-to-code resolution that unified the two closure cells (both fields the same
           fn), or per-field env copies that diverged from the shared capture, breaks a digit. The
           record-container companion of the list-of-closures and map-of-closures dispatch pins —
           and the SHARED-capture face neither has (their closures capture distinct values).")
  (input  (do
            (def (mk-ops (: base Int64))
              (record (add (fn ((: v Int64)) (+ v base)))
                      (mul (fn ((: v Int64)) (* v base)))))
            (def (main (: b Int64))
              (let ((ops (mk-ops b)))
                (+ (* 100 ((. ops add) 5))
                   ((. ops mul) 5))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 815 Int64)))

(case "sa1 String.slice inside a handler arm on a rope STATE (view-of-state in arm context)"
  (input  (do
            (effect St (op peek (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (String.concat "ab" "cdef")
                ((peek (v) s
                  (resume (String.scalar-len (Option.expect (String.slice s 1 v) "in bounds")) s)))
                (+ (* 10 (St.peek 4)) (St.peek 2))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 31 Int64)))

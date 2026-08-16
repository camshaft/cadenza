(case "ta1 a fallible helper with a `?` called from INSIDE a handler ARM (success path)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (bump (: v Int64))
              (let ((x (try (Some v))))
                (Some (+ x 100))))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume (match (bump s) ((Some v) v) ((None _u) -1)) (+ s 1))))
                (+ (* 10 (St.next)) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1156 Int64)))

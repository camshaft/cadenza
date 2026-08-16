(case "rw5 escalation: heap-state accumulator loses the branch perform's push across recursion"
  (input  (do
            (effect Log (op add (-> Int64 Int64)))
            (def (walk (: n Int64)) (if (= n 0) 0 (+ (if true (Log.add n) 0) (walk (- n 1)))))
            (def (main)
              (handle Log (list)
                ((add (v) s (resume (List.len s) (List.push s v))))
                (walk 3)))
            (export main)))
  (output (: 3 Int64)))

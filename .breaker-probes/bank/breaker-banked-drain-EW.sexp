(case "a guard projecting a FIELD of the record payload it destructured gates the arm"
  (doc    "The guard-reads-through-projection face: the guard's condition is `(> (. r pri) 5)` — a
           MEMBER ACCESS on the record payload the pattern just bound — so the guard evaluation
           composes the payload binder with a field projection before the arm is chosen. Guard holds
           → id through the guarded arm (700); guard fails → the unguarded same-variant arm reads the
           SAME record's other field negated (-30 — the fall-through must re-bind r2 to the intact
           record, not a guard-consumed husk); Idle → 0 → 670. The :495 guard family reads the payload
           WHOLE (List.len xs); the projection-in-guard + fall-through-rereads-record composition is
           the job-queue priority-dispatch idiom.")
  (input  (do
            (type Job (Ready (Record (: pri Int64) (: id Int64))) (Idle))
            (def (pick (: j Job))
              (match j
                ((guard (Ready r) (> (. r pri) 5)) (. r id))
                ((Ready r2) (- 0 (. r2 id)))
                ((Idle) 0)))
            (def (main (: p Int64))
              (+ (* 100 (pick (Ready (record (pri p) (id 7)))))
                 (+ (* 10 (pick (Ready (record (pri 1) (id 3)))))
                    (pick (Idle)))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 670 Int64)))

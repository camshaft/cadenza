(case "du1 a DURATION newtype state accumulates seconds across dispatches — the arm unwraps, adds at nanosecond scale, rewraps"
  (input  (do
            (type Duration (Duration UInt64))
            (effect T (op tick (-> Int64 Int64)))
            (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
            (def (dur-ns (: d Duration)) (match d ((Duration.Duration ns) ns)))
            (def (main (: n Int64))
              (handle T (secs (UInt64.wrap 0))
                ((tick (v) s
                  (let ((nxt (Duration.Duration (+ (dur-ns s) (dur-ns (secs (UInt64.wrap v)))))))
                    (resume (Int64.of (/ (dur-ns nxt) 1000000000)) nxt))))
                (+ (T.tick 3) (* 100 (T.tick 4)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 703 Int64)))

(case "a closure capturing a record returns row-op-updated copies without mutating its capture"
  (doc    "The captured-record row-op factory: `mk-bumper` captures r0 and each application returns
           `(Record.with r #\"n\" …)` — a FRESH path-copied record per call reading the CAPTURED
           base. Two applications with different deltas both read the ORIGINAL captured n (7 then
           102 from base 2 — a capture that absorbed the first call's update gives 107), and r0
           OUTSIDE the closure is untouched (1s digit: persistence through the env) → 8021. The
           row-op face of closure captures: the env holds the record handle across applications
           while each with-copy is independent — the config-template factory idiom (base settings
           captured once, per-call overrides).")
  (input  (do
            (def (mk-bumper (: r (Record (: n Int64) (: tag Int64))))
              (fn ((: d Int64)) (Record.with r #"n" (+ (. r n) d))))
            (def (main (: base Int64))
              (let ((r0 (record (n base) (tag 9))))
                (let ((bump (mk-bumper r0)))
                  (+ (* 1000 (. (bump 5) n))
                     (+ (* 10 (. (bump 100) n))
                        (- (. r0 n) (- base 1)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 8021 Int64)))

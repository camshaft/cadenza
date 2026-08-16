(case "host calls issue only from the TAKEN arm of a fused match and in arm order"
  (doc    "Host delegation × the match-fusion seam: a fused match (call-result scrutinee) whose BOTH
           arms perform a host-delegated `io.get` — fusion clones each arm's host perform into the
           callee's branches, and the observable host-call sequence must stay EXACTLY ONE call (the
           taken arm's), consuming the single response: k=7 → Hi arm → 70+3=73 with [io.get] the
           whole trace. A clone that speculated the untaken arm's perform, or emitted the host call
           outside the branch dispatch, would issue TWO calls (the fixture would reject the trace) or
           consume the response in the wrong operand. The fused-arm companion of the host-calls-in-
           order pins (:196-area); wasm computes, rust targets todo (host-effect family).")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (host (io)
                (match (mk k)
                  ((Hi h) (+ (* 10 h) (io.get)))
                  ((Lo w) (- (io.get) w)))))
            (export main)))
  (host-responses (respond io.get (: 3 Int64)))
  (host-calls (call io.get))
  (call   main (: 7 Int64)) (output (: 73 Int64)))

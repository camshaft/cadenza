(case "a handler whose heap-list state is REPLACED each resume drops the stale list safely"
  (doc    "The state-REPLACEMENT face of heap handler state (the accumulator pins GROW the state via
           List.push s; here each resume abandons the old list wholesale — `(List.push (list v) v)`
           builds a FRESH 2-element list and the previous state becomes garbage): the resume value
           reads the OUTGOING state's len before it drops. Per-perform the state is len-1 (seed) then
           len-2 forever: n=3 → 1+2+2 = 5; n=1 → 1. An over-drop of the replaced state frees the list
           the resume value was just computed from (UAF window between the len read and the state
           swap); a leak accumulates n dead lists silently but the churn path is exercised. The
           Perceus face of handler-state threading the grow-only accumulators never hit.")
  (input  (do
            (effect Acc (op push (-> Int64 Int64)))
            (def (go (: n Int64))
              (if (= n 0) 0 (+ (Acc.push n) (go (- n 1)))))
            (def (main (: n Int64))
              (handle Acc (list 0)
                ((push (v) s (resume (List.len s) (List.push (list v) v))))
                (go n)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 5 Int64))
  (call   main (: 1 Int64)) (output (: 1 Int64)))

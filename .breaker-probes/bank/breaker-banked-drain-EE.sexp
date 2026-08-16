(case "a handler whose TUPLE state carries a closure slot threads both slots per resume"
  (doc    "The CLOSURE-slot upgrade of the tuple-state pin (:2736's two slots are both scalars):
           the threaded state packs a scalar counter WITH a closure — each resume applies the
           closure slot to the op argument (f(x) = 2x) and rebuilds the pair advancing only the
           counter, the closure slot threading UNCHANGED through three resumes: 10+20+2 = 32. A
           state rebuild that dropped the fn handle's refcount per resume (three re-thread cycles),
           or re-materialized the closure per frame, corrupts a later application; the counter slot
           proves the rebuild genuinely runs. The fn-in-state face the scalar tuple pin cannot see
           (a FULLY fn-typed state still declines — banked TODO — so the tuple carrier is the
           working spelling of stateful-closure threading).")
  (input  (do
            (effect Acc (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Acc (tuple 0 (fn ((: v Int64)) (* v 2)))
                ((step (x) st
                  (match st
                    ((tuple c f) (resume (f x) (tuple (+ c 1) f))))))
                (+ (Acc.step 5) (+ (Acc.step n) (Acc.step 1)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 32 Int64)))

(case "wx6 wrapping increments CHAINED hop-to-hop — MAX-1 crosses the seam inside a nested three-op chain, count pins the trips"
  (input  (do
            (effect E (op step (-> Int64 Int64)) (op count (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((step (x) s (resume (Int64.wrapping-add x 1) (+ s 1)))
                 (count () s (resume s s)))
                (let ((v (E.step (E.step (E.step 9223372036854775806)))))
                  (+ (if (= v -9223372036854775807) 100 900)
                     (+ (if (< v 0) 10 90) (E.count))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 113 Int64)))

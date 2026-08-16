(case "ed2 a handled def CALLED from inside another def's handle — the callee's region shadows the caller's mid-body"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (inner (: k Int64))
              (handle St (* k 100)
                ((next () s (resume s (+ s 7))))
                (+ (St.next) (St.next))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next) (+ (inner 2) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 418 Int64))
  (call   main (: 0 Int64)) (output (: 408 Int64)))

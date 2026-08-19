(case "pyl3 HEAP-LIST STATE THROUGH DIVERGENT REPLAYS — the discarded replay pushes a NINE and the survivor pushes a FIVE onto the shared starting list, the next dispatch reads slot one and finds the survivor's five never the discarded nine, and the seeded head rides both answers while the five stamps only the second"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (list (% n 3))
                ((tick () xs
                  (do (resume (List.len xs) (List.push xs 9))
                      (resume (+ (match (List.at xs 0)
                                   ((Some h) (* h 10))
                                   ((None) (: -1 Int64)))
                                 (match (List.at xs 1)
                                   ((Some t) t)
                                   ((None) (: 0 Int64))))
                              (List.push xs 5)))))
                (+ (E.tick) (* 100 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1510 Int64))
  (call   main (: 0 Int64)) (output (: 500 Int64)))

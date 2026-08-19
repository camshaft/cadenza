(case "pyl4 the HEAP TOMBSTONE READS THE ARM'S OWN PRE-REPLAY LIST — both replays push onto the thread yet the tombstone that survives reads the arm's ORIGINAL binding with length one and the seeded head, neither replay's push contaminating the captured list, the heap twin of the capture-not-live law"
  (input  (do
            (effect E (op grow (-> Int64)))
            (def (main (: n Int64))
              (handle E (list (% n 3))
                ((grow () xs
                  (do (resume (List.len xs) (List.push xs 9))
                      (resume (List.len xs) (List.push xs 5))
                      (match (List.at xs 0)
                        ((Some h) (+ (* h 100) (List.len xs)))
                        ((None) (: -1 Int64))))))
                (E.grow)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

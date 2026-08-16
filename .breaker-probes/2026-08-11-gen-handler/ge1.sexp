(case "ge1 a generator threaded as HANDLER STATE draws a reproducible stream through performs"
  (input  (do
            (effect Gen (op draw (-> Unit Int64)))
            (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
            (def (run (: seed Int64))
              (handle Gen seed ((draw (u) s (resume (% s 1000) (next s))))
                (+ (* 1000000 (Gen.draw)) (+ (* 1000 (Gen.draw)) (Gen.draw)))))
            (def (main (: seed Int64))
              (if (= (run seed) (run seed)) 1 0))
            (export main)))
  (call   main (: 42 Int64)) (output (: 1 Int64))
  (call   main (: 7 Int64)) (output (: 1 Int64)))

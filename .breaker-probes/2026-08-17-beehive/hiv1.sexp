(case "hiv1 a BEEHIVE with swarm pressure — a full party of three or more bees banks the whole yield, a skeleton crew banks HALF (integer division showing in the answer), an empty hive answers nine hundred, hatching past six bees SWARMS half the colony away (seven-hundred row with the survivors), the read packs honey bees and swarms, and the seed's starting colony swarms on the FIRST hatch for one run and the SECOND for the other so the forage tiers flip between"
  (input  (do
            (effect H
              (op forage (-> Int64 Int64))
              (op hatch (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle H (tuple (+ (: 2 Int64) (* (% n 3) 2)) (: 0 Int64) (: 0 Int64))
                ((forage (y) st
                  (match st
                    ((tuple bees honey sw)
                      (if (>= bees 3)
                          (resume (+ (* (+ honey y) 10) 1) (tuple bees (+ honey y) sw))
                          (if (>= bees 1)
                              (resume (+ (* (/ y 2) 10) 2) (tuple bees (+ honey (/ y 2)) sw))
                              (resume (: 900 Int64) st))))))
                 (hatch (k) st
                  (match st
                    ((tuple bees honey sw)
                      (if (> (+ bees k) 6)
                          (resume (+ (: 700 Int64) (/ (+ bees k) 2))
                                  (tuple (/ (+ bees k) 2) honey (+ sw 1)))
                          (resume (* (+ bees k) 10) (tuple (+ bees k) honey sw))))))
                 (read () st
                  (match st
                    ((tuple bees honey sw)
                      (resume (+ (* honey 100) (+ (* bees 10) sw)) st)))))
                (let ((a (H.forage (: 6 Int64))))
                  (let ((b (H.hatch (: 3 Int64))))
                    (let ((c (H.forage (: 8 Int64))))
                      (let ((d (H.hatch (: 2 Int64))))
                        (let ((f (H.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 617031410501451 Int64))
  (call   main (: 0 Int64)) (output (: 320501117031131 Int64)))

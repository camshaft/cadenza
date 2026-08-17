(case "sgb1 a SIGNAL BOX with interlocked levers — throwing the points needs the signal at DANGER (else the interlock refuses showing both fields), clearing the signal needs the points SET (else refused showing them), the read packs points signal and moves, and the seed's points let one box clear immediately then deadlock its lever behind its own signal while the other must throw first so the same four pulls thread opposite interlocks"
  (input  (do
            (effect S
              (op lever (-> Int64 Int64))
              (op pull (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (if (> (% n 3) 0) 1 0) (: 0 Int64) (: 0 Int64))
                ((lever (p) st
                  (match st
                    ((tuple pts sig mv)
                      (if (= sig 0)
                          (resume (+ (* p 10) (% (+ mv 1) 10)) (tuple p sig (+ mv 1)))
                          (resume (+ (: 800 Int64) (+ (* pts 10) sig)) st)))))
                 (pull () st
                  (match st
                    ((tuple pts sig mv)
                      (if (= pts 1)
                          (resume (+ (: 700 Int64) (% mv 10)) (tuple pts (: 1 Int64) mv))
                          (resume (+ (: 900 Int64) pts) st)))))
                 (read () st
                  (match st
                    ((tuple pts sig mv)
                      (resume (+ (* pts 100) (+ (* sig 10) mv)) st)))))
                (let ((a (S.pull)))
                  (let ((b (S.lever (: 1 Int64))))
                    (let ((c (S.pull)))
                      (let ((d (S.lever (: 0 Int64))))
                        (let ((f (S.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 7008117008110110 Int64))
  (call   main (: 0 Int64)) (output (: 9000117018110111 Int64)))

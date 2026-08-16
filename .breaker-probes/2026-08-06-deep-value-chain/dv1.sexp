(case "dv1 one perform result flows through let, record, projection, tuple, destructure, and match"
  (input  (do
            (effect St (op seed (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((seed (u) s (resume s (+ s 1))))
                (let ((v (St.seed)))
                  (let ((r (record (base v) (scale 3))))
                    (let ((p (tuple (. r base) (* (. r base) (. r scale)))))
                      (match p
                        ((tuple lo hi)
                          (match (> hi 10)
                            (true (+ lo hi))
                            (false 0)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20 Int64)))

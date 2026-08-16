(case "eh1 a LIST built from three performs ESCAPES the handle — read intact outside the region"
  (input  (do
            (effect Cfg (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (let ((xs (handle Cfg n
                          ((get (u) s (resume s (+ s 1))))
                          (list (Cfg.get) (Cfg.get) (Cfg.get)))))
                (+ (* 100 (List.len xs))
                   (match (List.at xs 2) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 307 Int64)))

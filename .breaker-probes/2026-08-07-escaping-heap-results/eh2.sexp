(case "eh2 a MAP keyed with perform results ESCAPES the handle — looked up outside the region"
  (input  (do
            (effect Cfg (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (let ((m (handle Cfg n
                         ((get (u) s (resume s (+ s 10))))
                         (Map.insert (Map.insert Map.empty "a" (Cfg.get)) "b" (Cfg.get)))))
                (+ (* 10 (match (Map.lookup m "a") ((Some a) a) ((None _u) -1)))
                   (match (Map.lookup m "b") ((Some b) b) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 65 Int64)))

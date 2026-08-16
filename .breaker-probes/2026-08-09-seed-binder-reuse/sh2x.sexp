(case "sh2x a let binder feeds a nested handle's SEED and is read again AFTER the region — the freshened seed reference and the tail reference are the same binder"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((k (+ n 3)))
                  (+ (handle B (* k 2)
                       ((get (u) t (resume t (+ t 1))))
                       (+ (B.get) (B.get)))
                     k))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 41 Int64))
  (call   main (: 0 Int64)) (output (: 16 Int64))
  (call   main (: -4 Int64)) (output (: -4 Int64)))

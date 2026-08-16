(case "nw3 compact result stored in a MAP then the rope re-read (alias through a collection)"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((m (Map.insert Map.empty 1 (Bytes.compact rope))))
                  (+ (match (Map.lookup m 1) ((Some b) (Bytes.len b)) ((None _u) -1))
                     (* 100 (Bytes.len rope))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1010 Int64))
  (call   main (: 2 Int64)) (output (: 202 Int64)))

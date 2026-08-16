(case "Char values as Map keys look up by scalar identity including multibyte"
  (input  (do
            (def (main (: k Int64))
              (let ((m (Map.insert (Map.insert Map.empty #\a 1) #\é 2)))
                (+ (* 100 (match (Map.lookup m #\a) ((Some v) v) ((None u) -1)))
                   (+ (* 10 (match (Map.lookup m #\é) ((Some v) v) ((None u) -1)))
                      (match (Map.lookup m #\b) ((Some v) v) ((None u) 0))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 120 Int64)))

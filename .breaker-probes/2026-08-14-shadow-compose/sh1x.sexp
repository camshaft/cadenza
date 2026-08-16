(case "sh1x a shadowed HEAP binding: inner shadows a list with a Map, both alive across the shadow"
  (input  (do
            (def (main (: n Int64))
              (do
                (def xs (list n 2 3))
                (def total
                  (let ((xs (Map.insert Map.empty 1 100)))
                    (Map.len xs)))
                (+ (* 10 (List.len xs)) total)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 31 Int64)))

(case "fx1 a value computed ACROSS the fixnum boundary equals and CHAMP-keys like its literal twin"
  (input  (do
            (def (main (: k Int64))
              (let ((boxed (+ 536870911 k)))
                (+ (* 100 (if (= boxed 536870912) 1 0))
                   (+ (* 10 (match (Map.lookup (Map.insert Map.empty 536870912 7) boxed) ((Some v) v) ((None _u) -1)))
                      (Set.len (Set.of (list boxed 536870912)))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 171 Int64)))

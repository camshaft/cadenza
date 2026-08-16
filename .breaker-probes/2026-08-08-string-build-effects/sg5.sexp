(case "sg5 the GROWING string state is a MAP KEY per dispatch — each draw looks up the current rope in a literal map"
  (input  (do
            (effect St (op adv (-> Int64)))
            (def (main (: n Int64))
              (handle St "a"
                ((adv () s (resume (match (Map.lookup (map ("a" 10) ("ab" 20) ("abb" 30)) s)
                                     ((Some v) v)
                                     ((None) -1))
                                   (String.concat s "b"))))
                (+ (St.adv) (+ (* 10 (St.adv)) (* 100 (St.adv))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3210 Int64)))

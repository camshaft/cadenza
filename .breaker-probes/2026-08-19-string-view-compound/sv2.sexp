(case "sv2 a multibyte String view as a SET element dedupes against its literal twin"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (String.concat "aé∀" "bçd"))
                (def w (match (String.slice rope 1 4)
                         ((Some v) v) ((None _u) "")))
                (def s (Set.of (list w "é∀b" "zz")))
                (+ (* 10 (Set.len s)) (if (Set.contains s "é∀b") 1 0))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 21 Int64)))

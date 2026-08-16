(case "sh11 escaped out-of-range UInt4 shift result feeds a CHAMP key"
  (input  (do
            (def (main (: k Int64))
              (Set.len (Set.of (list (<< ((. (UInt 4) wrap) 3) ((. (UInt 4) wrap) k)) ((. (UInt 4) wrap) 8)))))
            (export main)))
  (call   main (: 3 Int64)) (trap "overflow"))

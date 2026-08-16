(case "mg1 a MATCH SCRUTINEE that is itself a match (nested scrutinee position) over runtime sums"
  (input  (do
            (type R (Lo Int64) (Hi Int64))
            (def (main (: k Int64))
              (match (match (if (> k 10) (Hi k) (Lo k))
                       ((R.Lo v) (Lo (* v 2)))
                       ((R.Hi v) (Hi (+ v 100))))
                ((R.Lo v) v)
                ((R.Hi v) (- 0 v))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 20 Int64)) (output (: -120 Int64)))

(case "ds1 FOUR discarded draws in a do-chain before the kept one — every discarded dispatch still advances the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (do (E.next) (E.next) (E.next) (E.next) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: 4 Int64))
  (call   main (: -9 Int64)) (output (: -5 Int64)))

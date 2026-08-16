(case "ic4 a helper with a performing IF-condition called twice — each call re-evaluates the condition against the advanced state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (pick) (if (> (St.next) 6) (St.next) (- 0 (St.next))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 2))))
                (+ (pick) (* 100 (pick)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1309 Int64))
  (call   main (: 0 Int64)) (output (: -602 Int64))
  (call   main (: 3 Int64)) (output (: 895 Int64)))

(case "bc1 a comparison of a draw feeds a BOOL-taking op whose arm NEGATES the state — the bool crosses the dispatch boundary"
  (input  (do
            (effect E (op next (-> Int64)) (op judge (-> Bool Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (judge (b) s (resume (if b 100 200) (- 0 s)))
                 (probe () s (resume s s)))
                (+ (E.judge (< (E.next) 3)) (E.probe))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 97 Int64))
  (call   main (: 5 Int64)) (output (: 194 Int64))
  (call   main (: -1 Int64)) (output (: 100 Int64)))

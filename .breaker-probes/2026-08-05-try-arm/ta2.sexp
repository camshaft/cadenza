(case "ta2 simpler: helper with try called FROM an arm"
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (safe-div (: a Int64) (: b Int64))
              (: (if (= b 0) (None unit) (Some (/ a b))) (Option Int64)))
            (def (main (: k Int64))
              (handle Ask k ((get (_u) s (resume (match (safe-div 100 s) ((Some v) v) ((None _u) -1)) s)))
                (Ask.get)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 25 Int64))
  (call   main (: 0 Int64)) (output (: -1 Int64)))

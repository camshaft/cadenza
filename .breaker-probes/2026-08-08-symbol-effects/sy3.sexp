(case "sy3 the SYMBOL state is a MAP KEY per dispatch — the a→b→c walk ends at a missing key"
  (input  (do
            (effect R (op route (-> Int64)))
            (def (main (: n Int64))
              (handle R #"a"
                ((route () s (resume (match (Map.lookup (map (#"a" 10) (#"b" 20)) s)
                                       ((Some v) v)
                                       ((None) -1))
                                     (if (= s #"a") #"b" #"c"))))
                (+ (R.route) (+ (* 10 (R.route)) (* 100 (R.route))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 110 Int64)))

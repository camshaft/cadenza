(do
            (effect E (op fetch (-> (Option Int64))) (op probe (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((fetch () s (resume (if (= (% s 2) 0) (Some s) (None)) (+ s 2)))
                 (probe () s (resume s s)))
                (+ (* 10 (handle Bail 0
                           ((out (v) t (+ 500 v)))
                           (match (E.fetch)
                             ((None) (Bail.out 11))
                             ((Some a) (match (E.fetch)
                               ((None) (Bail.out 22))
                               ((Some b) (* a b)))))))
                   (- (E.probe) n))))
            (export main))

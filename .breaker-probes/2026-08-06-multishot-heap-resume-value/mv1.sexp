(case "mv1 a multi-shot arm resumes DIFFERENT HEAP LISTS per branch — each continuation reads its own"
  (input  (do
            (effect Amb (op pick (-> Unit (List Int64))))
            (def (main (: n Int64))
              (handle Amb 0
                ((pick (u) s (+ (resume (list n 2 9) s) (resume (list 7) s))))
                (let ((xs (Amb.pick)))
                  (+ (* 10 (List.len xs))
                     (match (List.at xs 0) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 52 Int64)))

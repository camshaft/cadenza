(case "pyl1 a HEAP LIST BUILT AND MEASURED INSIDE THE TOLL — each frame's post-resume toll constructs a list whose LENGTH depends on the captured state then charges a hundredfold of it, the heap allocation happens during the unwind, and the state-gated size means the seeded frame builds the longer list"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (+ (resume (* s 10) (+ s 1))
                     (* 100 (List.len (if (> s 0)
                                          (List.push (List.push (list) s) s)
                                          (List.push (list) s)))))))
                (+ (E.tick) (* 10 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 610 Int64))
  (call   main (: 0 Int64)) (output (: 400 Int64)))

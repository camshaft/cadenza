(case "pyif1 probe: the arm branches on state parity and RESUMES in BOTH if-branches with DIFFERENT answer and next-state — even s resumes (* s 100) threading (+ s 1), odd s resumes (* s 10) threading (+ s 3); three dispatches follow a data-dependent path through the two resume sites, so the tail fold must handle two distinct resume calls that reconverge (each its own answer AND its own state advance)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (if (= (% s 2) (: 0 Int64))
                      (resume (* s 100) (+ s 1))
                      (resume (* s 10) (+ s 3)))))
      (+ (E.tick) (+ (E.tick) (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 460 Int64))
  (call   main (: 0 Int64)) (output (: 410 Int64)))

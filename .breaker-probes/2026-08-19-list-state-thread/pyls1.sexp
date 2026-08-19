(case "pyls1 probe: a LIST-STATE handler accumulates via prepend across the seam — push(v) threads (List.prepend l v) as the next-state and answers the new length, while a head op reads the most-recently-pushed element; the seed-scaled final push makes the head reflect the seed"
  (input (do
  (effect E (op push (-> Int64 Int64)) (op head (-> Int64)))
  (def (main (: n Int64))
    (handle E (list (% n 3))
      ((push (v) l (resume (List.len (List.prepend l v)) (List.prepend l v)))
       (head () l (resume (match (List.at l (: 0 Int64)) ((Some x) x) ((None) (: -1 Int64))) l)))
      (+ (* 1000 (E.push (: 7 Int64)))
         (+ (* 100 (E.push (* (% n 3) (: 10 Int64))))
            (E.head)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 2310 Int64))
  (call   main (: 0 Int64)) (output (: 2300 Int64)))

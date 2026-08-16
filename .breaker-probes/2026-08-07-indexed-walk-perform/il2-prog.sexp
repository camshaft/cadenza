(do
  (effect Pick (op at (-> Int64 Int64)))
  (def (build (: xs (List Int64)) (: i Int64) (: out (List Int64)))
    (match (List.at xs i)
      ((Some v) (build xs (+ i 1) (List.push out (Pick.at v))))
      ((None _u) out)))
  (def (main (: n Int64))
    (handle Pick 0
      ((at (v) s (resume (* v 10) (+ s 1))))
      (let ((out (build (list 3 1 2) 0 (list))))
        (+ (* 100 (match (List.at out 0) ((Some a) a) ((None _u) -1)))
           (+ (* 10 (match (List.at out 1) ((Some b) b) ((None _u) -1)))
              (match (List.at out 2) ((Some c) c) ((None _u) -1)))))))
  (export main))

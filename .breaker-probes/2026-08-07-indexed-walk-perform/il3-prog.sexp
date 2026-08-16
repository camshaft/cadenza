(do
  (effect Keep (op test (-> Int64 Int64)))
  (def (sift (: xs (List Int64)) (: i Int64) (: out (List Int64)))
    (match (List.at xs i)
      ((Some v) (sift xs (+ i 1) (if (> (Keep.test v) 0) (List.push out v) out)))
      ((None _u) out)))
  (def (main (: n Int64))
    (handle Keep 0
      ((test (v) s (resume (if (> v s) 1 0) (+ s 2))))
      (let ((out (sift (list 1 4 2 9) 0 (list))))
        (+ (* 100 (List.len out))
           (match (List.at out 1) ((Some b) b) ((None _u) -1))))))
  (export main))

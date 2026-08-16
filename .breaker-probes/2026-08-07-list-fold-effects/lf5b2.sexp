(do
  (effect St (op w (-> Int64 Int64)))
  (def (zipwalk (: xs (List Int64)) (: ys (List Int64)) (: i Int64))
    (match (List.at xs i)
      ((Some x) (match (List.at ys i)
                  ((Some y) (+ (St.w (+ x y)) (zipwalk xs ys (+ i 1))))
                  ((None) 0)))
      ((None) 0)))
  (def (main (: n Int64))
    (handle St n
      ((w (v) s (resume (+ v s) (+ s 1))))
      (zipwalk (list 1 2 3) (list 10 20 30) 0)))
  (export main))

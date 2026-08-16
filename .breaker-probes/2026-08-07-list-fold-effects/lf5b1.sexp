(do
  (effect St (op mix (-> Int64 Int64 Int64)))
  (def (walk (: xs (List Int64)) (: i Int64))
    (match (List.at xs i)
      ((Some x) (+ (St.mix x i) (walk xs (+ i 1))))
      ((None) 0)))
  (def (main (: n Int64))
    (handle St n
      ((mix (a b) s (resume (+ (* a b) s) (+ s 1))))
      (walk (list 1 2 3) 0)))
  (export main))

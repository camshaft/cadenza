(case "m2 nested match on empty-front rev path"
  (input (do
        (def (rev (: xs (List Int64)) (: acc (List Int64)))
          (match xs
            ((list) acc)
            ((list h .. t) (rev t (List.concat (list h) acc)))))
        (def (deq (: f (List Int64)) (: b (List Int64)))
          (match f
            ((list h .. t) (tuple h t b))
            ((list)
              (match (rev b (list))
                ((list h .. t) (tuple h t (list)))
                ((list) (tuple -1 (list) (list)))))))
        (def (main (: n Int64))
          (match (deq (list) (list n 2)) ((tuple v f2 _b2) (+ v ((. List len) f2)))))
        (export main)))
  (call main (: 5 Int64)) (output (: 3 Int64)))

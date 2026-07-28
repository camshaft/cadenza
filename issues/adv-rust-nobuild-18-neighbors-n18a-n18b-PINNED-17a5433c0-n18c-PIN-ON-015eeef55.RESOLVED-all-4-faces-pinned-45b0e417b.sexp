(case "n18a match on recursive call in IF-branch inside arm"
  (input (do
        (def (rev (: xs (List Int64)) (: acc (List Int64)))
          (match xs
            ((list) acc)
            ((list h .. t) (rev t (List.concat (list h) acc)))))
        (def (pick (: f (List Int64)) (: b (List Int64)) (: n Int64))
          (match f
            ((list h .. t) (tuple h t b))
            ((list)
              (if (> n 0)
                  (match (rev b (list))
                    ((list h .. t) (tuple h t (list)))
                    ((list) (tuple -1 (list) (list))))
                  (tuple -2 (list) b)))))
        (def (main (: n Int64))
          (match (pick (list) (list n 2) n) ((tuple v f2 _b2) (+ v ((. List len) f2)))))
        (export main)))
  (call main (: 5 Int64)) (output (: 3 Int64))
  (call main (: -5 Int64)) (output (: -2 Int64)))

(case "n18b match on recursive call in arm, arms return LIST not tuple"
  (input (do
        (def (rev (: xs (List Int64)) (: acc (List Int64)))
          (match xs
            ((list) acc)
            ((list h .. t) (rev t (List.concat (list h) acc)))))
        (def (deq (: f (List Int64)) (: b (List Int64)))
          (match f
            ((list h .. t) (List.concat (list h) t))
            ((list)
              (match (rev b (list))
                ((list h .. t) (List.concat (list h) t))
                ((list) (list -1))))))
        (def (main (: n Int64))
          (do
            (def out (deq (list) (list n 2)))
            (+ (* (Option.expect (List.at out 0) "h") 10) ((. List len) out))))
        (export main)))
  (call main (: 5 Int64)) (output (: 22 Int64)))

(case "n18c match on recursive call in arm, tuple of TWO lists no scalar"
  (input (do
        (def (rev (: xs (List Int64)) (: acc (List Int64)))
          (match xs
            ((list) acc)
            ((list h .. t) (rev t (List.concat (list h) acc)))))
        (def (deq (: f (List Int64)) (: b (List Int64)))
          (match f
            ((list _h .. t) (tuple t b))
            ((list)
              (match (rev b (list))
                ((list _h .. t) (tuple t (list)))
                ((list) (tuple (list) (list)))))))
        (def (main (: n Int64))
          (match (deq (list) (list n 2)) ((tuple f2 b2) (+ (* ((. List len) f2) 10) ((. List len) b2)))))
        (export main)))
  (call main (: 5 Int64)) (output (: 10 Int64)))

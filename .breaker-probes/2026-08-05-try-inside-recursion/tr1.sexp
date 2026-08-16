(case "tr1 try (early-return Option) INSIDE self-recursion: the sibling face of the wp4 abort-in-recursion"
  (input  (do
            (def (find (: n Int64) (: xs (List Int64)))
              (match xs
                ((list) (Option.None))
                ((list h .. t) (if (= h n) (Option.Some h) (find n t)))))
            (def (main (: n Int64))
              (match (find n (list 3 7 9)) ((Option.Some v) v) ((Option.None) -1)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64)))

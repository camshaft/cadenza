(case "lr2 a recursive list-rest WALK over a handler-built list, performing per element visited"
  (input  (do
            (effect St (op a (-> Unit Int64)) (op w (-> Int64 Int64)))
            (def (walk (: xs (List Int64)))
              (match xs
                ((list) 0)
                ((list h .. t) (+ (St.w h) (walk t)))))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1)))
                 (w (v) s (resume (* v s) s)))
                (walk (list (St.a) (St.a)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 77 Int64)))

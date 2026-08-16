(case "lf4 a per-element dispatch comparing each element against the RISING state — amplify-or-pass per visit"
  (input  (do
            (effect St (op weigh (-> Int64 Int64)))
            (def (walk (: xs (List Int64)) (: i Int64))
              (match (List.at xs i)
                ((Some v) (+ (St.weigh v) (walk xs (+ i 1))))
                ((None) 0)))
            (def (main (: n Int64))
              (handle St n
                ((weigh (v) s (resume (if (> v s) (* v 100) v) (+ s 1))))
                (walk (list 2 9 4) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 906 Int64))
  (call   main (: 1 Int64)) (output (: 1500 Int64))
  (call   main (: 8 Int64)) (output (: 15 Int64)))

(case "mo5 LIST-state shadowing — inner and outer thread independent heap lists, growth interleaved"
  (input  (do
            (effect St (op push (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (list n)
                ((push (v) s (resume (List.len s) (List.push s v))))
                (+ (St.push 10)
                   (+ (handle St (list 7 8 9)
                        ((push (v) s (resume (+ (List.len s) 100) (List.push s v))))
                        (+ (St.push 1) (St.push 2)))
                      (St.push 20)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 210 Int64))
  (call   main (: 0 Int64)) (output (: 210 Int64)))

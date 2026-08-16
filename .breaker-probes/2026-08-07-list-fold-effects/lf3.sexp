(case "lf3 the arm pushes TWO elements per dispatch (both lengths read from the PRE-push list) — the third draw reads length 5"
  (input  (do
            (effect L (op push2 (-> Int64)))
            (def (main (: n Int64))
              (handle L (list n)
                ((push2 () s (resume (List.len s) (List.push (List.push s (List.len s)) (* (List.len s) 10)))))
                (do
                  (L.push2)
                  (L.push2)
                  (L.push2))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5 Int64)))

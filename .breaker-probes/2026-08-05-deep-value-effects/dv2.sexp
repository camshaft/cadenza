(case "dv2 the nested compound as HANDLER STATE, inner List grown through the tuple per perform"
  (input  (do
            (effect St (op push (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St (tuple 0 (list))
                ((push (v) s (resume (List.len (. s 1))
                                     (tuple (+ (. s 0) v) (List.push (. s 1) v)))))
                (do
                  (def l1 (St.push a))
                  (def l2 (St.push (+ a 1)))
                  (+ (* 100 l2) (St.push 0)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 102 Int64)))

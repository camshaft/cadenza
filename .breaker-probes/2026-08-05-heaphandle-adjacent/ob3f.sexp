(case "ob3f isolate: branch on STATE s, TWO performs"
  (input  (do
            (effect Src (op read (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Src 7
                ((read (v) s (if (> s 5) (resume v s) (resume -1 s))))
                (+ (Src.read n) (* 10 (Src.read (+ n 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 65 Int64)))

(case "ob3e isolate: branch on OP-PARAM v, ONE perform"
  (input  (do
            (effect Src (op read (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Src 0
                ((read (v) s (if (> v 0) (resume v s) (resume -1 s))))
                (Src.read n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))

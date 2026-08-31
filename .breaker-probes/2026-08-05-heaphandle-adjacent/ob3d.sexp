(case "ob3d control: BRANCHING arm resuming plain SCALAR (two resume sites, no Option)"
  (input  (do
            (effect Src (op read (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Src 0
                ((read (v) s
                  (if (> v 0) (resume v s) (resume -1 s))))
                (+ (Src.read n) (* 10 (Src.read (- 0 n))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -5 Int64)))

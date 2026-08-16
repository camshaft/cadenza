(case "hr4 a whole HANDLE region as another effect's op ARGUMENT — B's region computes the value A's dispatch consumes"
  (input  (do
            (effect A (op scale (-> Int64 Int64)))
            (effect B (op next (-> Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((scale (v) s (resume (* v s) (+ s 1))))
                (+ (A.scale (handle B n
                              ((next () t (resume t (* t 2))))
                              (+ (B.next) (B.next))))
                   (A.scale 1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 161 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64)))

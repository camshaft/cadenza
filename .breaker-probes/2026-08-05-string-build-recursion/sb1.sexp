(case "sb1 a recursive fn BUILDS a rope (200 concats) whose scalar-len is read under a handler"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (build (: n Int64) (: acc String))
              (if (= n 0) acc (build (- n 1) (String.concat acc "ab"))))
            (def (main (: n Int64))
              (handle Log 0
                ((emit (v) s (resume (+ s v) (+ s v))))
                (Log.emit (String.scalar-len (build n "")))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 400 Int64)))

(case "sftA shrink — ONE push with siftup only (double List.update swap in recursive def)"
  (input  (do
            (effect H (op push (-> Int64 Int64)))
            (def (getat (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None u) 0)))
            (def (siftup (: xs (List Int64)) (: i Int64))
              (if (= i 0)
                  xs
                  (if (< (getat xs i) (getat xs (/ (- i 1) 2)))
                      (siftup (List.update (List.update xs (/ (- i 1) 2) (getat xs i)) i (getat xs (/ (- i 1) 2))) (/ (- i 1) 2))
                      xs)))
            (def (main (: n Int64))
              (handle H (: (list n 4) (List Int64))
                ((push (v) st
                  (match (siftup (List.push st v) (- (List.len (List.push st v)) 1))
                    (h2 (resume (getat h2 0) h2)))))
                (H.push 2)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

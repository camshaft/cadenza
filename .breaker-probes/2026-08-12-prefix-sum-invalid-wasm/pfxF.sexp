(case "pfxF three adds: at-read of FIXED index 0 (not len-1), push present"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 0)
                ((add (v) pre
                  (let ((t (+ (match (List.at pre 0) ((Some x) x) ((None u) 0)) v)))
                    (resume t (List.push pre t)))))
                (let ((_a (S.add n)))
                  (let ((_b (S.add 4)))
                    (S.add 9)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9 Int64)))

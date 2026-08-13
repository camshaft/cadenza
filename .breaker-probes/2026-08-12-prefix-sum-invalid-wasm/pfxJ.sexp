(case "pfxJ finding-23 face: FOUR dispatches of the failing arm"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 0)
                ((add (v) pre
                  (let ((t (+ (match (List.at pre (- (List.len pre) 1)) ((Some x) x) ((None u) 0)) v)))
                    (resume t (List.push pre t)))))
                (let ((_a (S.add n)))
                  (let ((_b (S.add 4)))
                    (let ((_c (S.add 9)))
                      (S.add 1))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 17 Int64)))

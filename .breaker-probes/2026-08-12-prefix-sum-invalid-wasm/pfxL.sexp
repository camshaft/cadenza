(case "pfxL finding-23 face: seed length 3 + ONE dispatch (final length 4)"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 0 1 2)
                ((add (v) pre
                  (let ((t (+ (match (List.at pre (- (List.len pre) 1)) ((Some x) x) ((None u) 0)) v)))
                    (resume t (List.push pre t)))))
                (S.add n)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 5 Int64)))

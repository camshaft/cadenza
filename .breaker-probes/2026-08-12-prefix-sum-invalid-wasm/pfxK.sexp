(case "pfxK finding-23 face: seed length 2 + two dispatches (final length 4)"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 0 1)
                ((add (v) pre
                  (let ((t (+ (match (List.at pre (- (List.len pre) 1)) ((Some x) x) ((None u) 0)) v)))
                    (resume t (List.push pre t)))))
                (let ((_a (S.add n)))
                  (S.add 4))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 8 Int64)))

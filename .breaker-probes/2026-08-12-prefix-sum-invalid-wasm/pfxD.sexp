(case "pfxD three adds: helper+push but NO let (t inlined twice)"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (last (: xs (List Int64)))
              (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
            (def (main (: n Int64))
              (handle S (list 0)
                ((add (v) pre
                  (resume (+ (last pre) v) (List.push pre (+ (last pre) v)))))
                (let ((_a (S.add n)))
                  (let ((_b (S.add 4)))
                    (S.add 9)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 16 Int64)))

(case "pfxA three adds, arm WITHOUT last-helper (plain len as answer)"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 0)
                ((add (v) pre
                  (resume (List.len pre) (List.push pre v))))
                (let ((_a (S.add n)))
                  (let ((_b (S.add 4)))
                    (S.add 9)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3 Int64)))

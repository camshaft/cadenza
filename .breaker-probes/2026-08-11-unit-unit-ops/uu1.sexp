(case "uu1 a Unit-to-Unit op driven purely for its state side effect — two marks then a count reads 2"
  (input  (do
            (effect L (op mark (-> Unit Unit)) (op count (-> Int64)))
            (def (main (: n Int64))
              (handle L n
                ((mark (u) s (resume unit (+ s 1)))
                 (count () s (resume s s)))
                (let ((_a (L.mark unit)))
                  (let ((_b (L.mark unit)))
                    (L.count)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2 Int64))
  (call   main (: 40 Int64)) (output (: 42 Int64)))

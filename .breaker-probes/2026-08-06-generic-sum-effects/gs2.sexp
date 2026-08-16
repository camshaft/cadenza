(case "gs2 a generic sum instantiated at a HEAP payload ((Box (List Int64))) crosses resume"
  (input  (do
            (effect St (op grab (-> Int64 (Box (List Int64)))))
            (type (Box a) (Full a) (Empty))
            (def (main (: n Int64))
              (handle St 0
                ((grab (v) s (resume (if (> v 10) (Box.Full (list v v v)) (Box.Empty)) s)))
                (+ (match (St.grab 20) ((Box.Full xs) (List.len xs)) ((Box.Empty) -1))
                   (match (St.grab n) ((Box.Full xs) (List.len xs)) ((Box.Empty) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2 Int64)))

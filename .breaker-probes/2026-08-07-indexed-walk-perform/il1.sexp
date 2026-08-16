(case "il1 an indexed list walk performing per element — element × advancing draw, summed"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (go (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (go xs (+ i 1) (+ acc (* v (St.next)))))
                ((None _u) acc)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (go (list 1 2 3) 0 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 38 Int64)))

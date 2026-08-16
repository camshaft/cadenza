(case "gi1 a GENERIC helper instantiated at (List Int64) consuming perform-built lists in two roles"
  (input  (do
            (effect St (op mk (-> Unit (List Int64))))
            (def (first-or x d)
              (match x ((list) d) ((list h .. _t) h)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (list s (+ s 1)) (+ s 10))))
                (+ (* 10 (first-or (St.mk) -1))
                   (first-or (St.mk) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 65 Int64)))

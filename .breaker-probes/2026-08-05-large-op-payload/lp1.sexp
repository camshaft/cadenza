(case "lp1 a 100-element List crossing the op-ARG boundary and back as the RESUME value (large payload round-trip)"
  (input  (do
            (effect St (op echo (-> (List Int64) (List Int64))))
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (build (- i 1) (List.push acc i))))
            (def (main (: n Int64))
              (handle St 0
                ((echo (xs) s (resume (List.push xs 999) s)))
                (do
                  (def out (St.echo (build n (list))))
                  (+ (* 10 (List.len out))
                     (match (List.at out n) ((Option.Some v) (if (= v 999) 1 0)) ((Option.None) -1))))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 1011 Int64)))

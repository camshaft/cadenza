(case "lu2 the updated-back list keys a Map like the pristine build (RRB update leaves no key trace)"
  (input  (do
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (build (- i 1) (List.push acc i))))
            (def (main (: n Int64))
              (do
                (def base (build n (list)))
                (def restored (List.update (List.update base 35 999) 35 (match (List.at base 35) ((Some v) v) ((None _u) -1))))
                (match (Map.lookup (Map.insert Map.empty base 42) restored)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 42 Int64)))

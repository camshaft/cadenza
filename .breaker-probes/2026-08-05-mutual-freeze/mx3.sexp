(case "mx3 mutual recursion with a (List String) accumulator (heap non-sum element)"
  (input  (do
            (def (evens (: n Int64) (: acc (List String)))
              (if (= n 0) acc (odds (- n 1) (List.push acc "e"))))
            (def (odds (: n Int64) (: acc (List String)))
              (if (= n 0) acc (evens (- n 1) (List.push acc "o"))))
            (def (main (: k Int64))
              (List.len (evens k (list))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 6 Int64)))

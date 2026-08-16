(case "mx1 mutual recursion with a (List Int64) accumulator (scalar element control)"
  (input  (do
            (def (evens (: n Int64) (: acc (List Int64)))
              (if (= n 0) acc (odds (- n 1) (List.push acc n))))
            (def (odds (: n Int64) (: acc (List Int64)))
              (if (= n 0) acc (evens (- n 1) acc)))
            (def (main (: k Int64))
              (List.len (evens k (list))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 3 Int64)))

(case "mx2 mutual recursion with a (List <user-Sum>) accumulator (the filed freeze shape)"
  (input  (do
            (type Tok (A Int64) (B Int64))
            (def (evens (: n Int64) (: acc (List Tok)))
              (if (= n 0) acc (odds (- n 1) (List.push acc (A n)))))
            (def (odds (: n Int64) (: acc (List Tok)))
              (if (= n 0) acc (evens (- n 1) (List.push acc (B n)))))
            (def (main (: k Int64))
              (List.len (evens k (list))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 6 Int64)))

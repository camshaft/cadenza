(case "da1 THREE draws inside one list literal passed to a helper — left-to-right element order, the post-call draw sees all three"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (sum3 (: xs (List Int64)))
              (match (List.at xs 0) ((Some a) (match (List.at xs 1) ((Some b) (match (List.at xs 2) ((Some c) (+ a (+ b c))) ((None) 0))) ((None) 0))) ((None) 0)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (+ (sum3 (list (St.next) (St.next) (St.next))) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 75 Int64))
  (call   main (: 1 Int64)) (output (: 15 Int64)))

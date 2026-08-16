(case "sq1 a Set of RECORDS with heap-list fields dedupes by deep content"
  (input  (do
            (def (main (: n Int64))
              (Set.len (Set.of (list
                (record (xs (list n 2)) (tag "a"))
                (record (xs (list 1 2)) (tag "a"))
                (record (xs (list 1 2)) (tag (String.concat "a" "")))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 9 Int64)) (output (: 2 Int64)))

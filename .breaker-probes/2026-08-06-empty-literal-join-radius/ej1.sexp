(case "ej1 empty (list) in an IF join (not match) — same downstream evidence"
  (input  (do
            (def (main (: n Int64))
              (let ((xs (if (> n 100) (list 1 2) (list))))
                (List.len (List.push xs n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))

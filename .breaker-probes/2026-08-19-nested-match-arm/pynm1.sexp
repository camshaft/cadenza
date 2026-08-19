(case "pynm1 probe: op takes an (Option (Option Int64)) arg and the arm NESTED-MATCHES it two levels deep to pick the resume; Some(Some x) adds x, Some(None) doubles, None scales-by-10 — two dispatches hit different nested arms"
  (input (do
  (effect E (op cmd (-> (Option (Option Int64)) Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((cmd (m) s
        (match m
          ((Some inner)
            (match inner
              ((Some x) (resume (+ s x) (+ s 1)))
              ((None) (resume s (* s 2)))))
          ((None) (resume (* s 10) (+ s 3))))))
      (+ (* 100 (E.cmd (Some (Some 7)))) (E.cmd (None)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 820 Int64))
  (call   main (: 0 Int64)) (output (: 710 Int64)))

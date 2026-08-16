(case "odx8 the escaped out-of-range Int24 re-enters arithmetic"
  (input  (do
            (def (main (: k Int64))
              (let ((bad (/ ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) k))))
                (Int64.of (+ bad ((. (Int 24) wrap) 0)))))
            (export main)))
  (call   main (: -1 Int64)) (trap "overflow"))

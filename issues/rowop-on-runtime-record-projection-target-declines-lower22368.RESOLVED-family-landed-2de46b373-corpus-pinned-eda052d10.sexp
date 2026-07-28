(case "l6 nested-record param, Record.with inner update through it"
  (input (do
        (def (bump (: outer (Record (: pos (Record (: x Int64) (: y Int64))) (: vel (Record (: x Int64) (: y Int64))))) (: d Int64))
          (Record.with outer pos (Record.with (. outer pos) y (+ (. (. outer pos) y) d))))
        (def (main (: d Int64))
          (do
            (def p0 (record (pos (record (x 1) (y 2))) (vel (record (x 30) (y 40)))))
            (. (. (bump p0 d) pos) y)))
        (export main)))
  (call main (: 5 Int64)) (output (: 7 Int64)))

; breaker probe L — LEADING-rest list pattern: `(list .. init last)` — rest first, one trailing
; element. If supported: init = all-but-last, last = final element. xs=[1,2,3] → init len 2,
; last 3 → 23; xs=[7] → init len 0, last 7 → 7. Empty list falls to the (list) arm → -1.

(case "a list pattern with a leading rest and one trailing element binds the suffix"
  (input  (do
            (def (main (: xs (List Int64)))
              (match xs
                ((list .. init last) (+ (* 10 (List.len init)) last))
                ((list) -1)))
            (export main)))
  (call   main (list 1 2 3)) (output (: 23 Int64))
  (call   main (list 7)) (output (: 7 Int64))
  (call   main (list)) (output (: -1 Int64)))

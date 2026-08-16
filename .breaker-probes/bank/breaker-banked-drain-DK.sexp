(case "a nested list pattern in rest position is a shape error, not an unbound name"
  (doc    "RULED (v-inference 2026-08-02) + FIXED (PR#1206, wording #1250): the rest binder of a list
           pattern admits only a name or `_` — core-semantics.md grants nested patterns to ELEMENT
           positions only, and a binding position requires an irrefutable pattern (a nested list
           pattern is refutable on empty rest; the same name-only rule the map rest binder has). So
           `(list a .. (list b .. r))` REJECTS with the CDZ0201 SHAPE diagnostic naming the rest form
           (bind the tail to a name and destructure in a nested match) — NOT the pre-fix CDZ0101
           'unbound name' scoping leak this arc started from (adv-49: resolve treated the rest slot
           as a single name-binder and the compound's inner names fell through to scoping). The
           mirror of the map-rest shape reject; uniform across all targets.")
  (input  (do
            (def (main (: xs (List Int64)))
              (match xs
                ((list a .. (list b .. r)) (+ (* 100 a) (+ (* 10 b) (List.len r))))
                ((list a) (* a 1000))
                ((list) -1)))
            (export main)))
  (call   main (list 1 2 3))
  (error  CDZ0201))

; breaker probe F — Perceus dup/drop on a SHARED record threaded through recursion while each
; level projects a field AND builds a row-extended copy it then discards on one branch.
; Hand-derived: go r 3: acc pattern — each level adds (. r a)=10: 3 levels → 30 + base (. r b)=2 → 32.
;   The extended record (with c) is built every level and dropped unless n==1, where its c=(* 10 n)=10
;   is added: total = 10+10+10 + 10 (c at n==1) + 2 = 42.
; main → 42.

(case "shared record threaded through recursion with a per-level extended copy dropped on most branches"
  (input  (do
            (def (go (: r (Record (a Int64) (b Int64))) (: n Int64))
              (if (= n 0)
                (. r b)
                (let ((e (record (a (. r a)) (b (. r b)) (c (* 10 n)))))
                  (if (= n 1)
                    (+ (. r a) (+ (. e c) (go r (- n 1))))
                    (+ (. r a) (go r (- n 1)))))))
            (def (main) (go (record (a 10) (b 2)) 3))
            (export main)))
  (output (: 42 Int64)))

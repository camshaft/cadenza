; breaker probe AB2 — TRAP faces of the quoted-arithmetic eval family (002e7c3d1 pinned the value
; faces of * / %): a quoted computation whose value WOULD trap at run time is caught at COMPILE
; time by the evaluator as the compile-provable trap reject (CDZ0304) — never a wrapped scalar,
; never a compiler crash.

(case "eval of quoted division by zero rejects as a compile-provable trap"
  (doc    "`(eval (quasiquote (/ 1 0)))` — the compile-time evaluator folds the quoted division and
           hits the zero divisor AT COMPILE TIME. The sound outcome: the same CDZ0304 compile-provable
           trap reject a bare constant `(/ 1 0)` gets — the eval boundary does not launder a provable
           trap into UB, a wrapped value, or a compiler panic. The div-0 trap face of the eval
           arithmetic family (the value faces of * and / and % are pinned beside this).")
  (input  (do
            (def (main) (eval (quasiquote (/ 1 0))))
            (export main)))
  (call   main)
  (error  CDZ0304))

(case "eval of quoted overflowing addition rejects as a compile-provable trap"
  (doc    "The overflow twin: `(eval (quasiquote (+ 9223372036854775807 1)))` folds Int64.max + 1 in
           the compile-time evaluator — checked semantics reject CDZ0304 (compile-provable overflow),
           never a two's-complement wrap to Int64.min. Together with the div-0 face this pins that the
           eval fold's arithmetic is CHECKED, matching the run-time trap semantics exactly (a fold that
           wrapped would let a quoted computation observe UB the run-time forbids).")
  (input  (do
            (def (main) (eval (quasiquote (+ 9223372036854775807 1))))
            (export main)))
  (call   main)
  (error  CDZ0304))

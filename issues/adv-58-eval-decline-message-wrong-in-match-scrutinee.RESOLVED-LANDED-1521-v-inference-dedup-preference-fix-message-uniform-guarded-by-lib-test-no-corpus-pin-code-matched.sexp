; adv-58 (breaker, 2026-08-02, LOW diagnostics-quality — decline behavior CORRECT in all positions,
; message wrong-class in one): `eval` of a runtime-built (let-bound) AST correctly declines with the
; TEACHING diagnostic ("eval executes only a COMPILE-TIME-VISIBLE AST construction ...") in let-init,
; if-cond, and arith-operand positions — but the SAME program with the eval in MATCH-SCRUTINEE
; position falls to a bare "unbound name `eval`" (CDZ0101), which is FALSE (eval is a known form,
; just position-limited) and sends the user hunting for an import instead of explaining the
; compile-time-visibility rule. A match scrutinee resolves through a path that misses the eval
; special form entirely. Repro pair:
;   (let ((tree (quasiquote (+ (unquote k) 1)))) (+ (eval tree) 0))    -> good teaching message
;   (let ((tree (quasiquote (+ (unquote k) 1)))) (match (eval tree) (v v))) -> "unbound name eval"
; Control: (match (eval (quote (+ 1 2))) (v (+ v k))) COMPILES and runs (const-visible eval in
; scrutinee position works — so the resolver knows eval there; only the DECLINE path misroutes).
(case "adv-58 eval of a runtime AST in match-scrutinee position gets the teaching decline, not unbound-name"
  (input  (do
            (def (main (: k Int64))
              (let ((tree (quasiquote (+ (unquote k) 1))))
                (match (eval tree)
                  (v v))))
            (export main)))
  (error  CDZ0101))

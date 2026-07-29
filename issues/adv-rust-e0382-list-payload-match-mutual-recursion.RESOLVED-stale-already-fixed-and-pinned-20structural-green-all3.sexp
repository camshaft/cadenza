; FINDING (breaker, 2026-07-25): rust NO-BUILD (error[E0382]: borrow of moved value: `__ms19.0`)
; when a MUTUALLY-RECURSIVE tree-fold matches a REBUILT list with a payload-binding list pattern.
; wasm compiles + computes the same program correctly (105/102) - a backend-divergent no-build,
; the classic compiler-PASS shape (fold + fold-list rebuilding an Ast list bottom-up).
;
; MATRIX (all gated --target rust; wasm passes all):
;   x m2/m4/m5: (fold node) mutually recursive with (fold-list ...) that REBUILDS the list;
;       inner (match xs2 ((list (Ast.Int a)) ...) (_ ...)) with a PAYLOAD-binding element
;       pattern -> E0382 __ms19.0. Repros with a single-element pattern (m4) and even with
;       the fall-through returning a FRESH value (m5) - the xs2 REUSE is NOT required.
;   ok m3: the SAME nested match over a call result WITHOUT recursion -> builds + passes.
;   ok m6: the recursion WITHOUT the inner payload-binding list match -> builds + passes.
;   ok m1: the inner match shape as a standalone fn on a param -> builds + passes.
;
; So: RECURSION x (list-pattern with payload binder over a rebuilt list) = the rust emit
; moves the matched list into the probe chain and then re-borrows it (__ms19.0 = the match
; scrutinee temp). Neither half alone breaks. IMPACT: the natural bottom-up rewrite pass
; (constant-fold / simplify / rename with structural dispatch) no-builds on rust when written
; with element-payload dispatch; binder-free or non-recursive spellings work.
;
; Repro below = m4-minimal (expect 102: one fold, v=2 -> wasm gives 102; rust E0382).
(case "a mutually-recursive fold matching a rebuilt list with a payload binder builds on rust (FINDING repro)"
  (input (do
        (def (fold node)
          (match node
            ((Ast.List xs)
              (match (fold-list xs (list) 0)
                ((tuple xs2 k)
                  (match xs2
                    ((list (Ast.Int a)) (tuple (Ast.Int a) (+ k 1)))
                    (_ (tuple (Ast.List xs2) k))))))
            (other (tuple other 0))))
        (def (fold-list (: xs (List Ast)) (: acc (List Ast)) (: k Int64))
          (match xs
            ((list) (tuple acc k))
            ((list h .. t)
              (match (fold h)
                ((tuple h2 k2) (fold-list t (List.push acc h2) (+ k k2)))))))
        (def (main (: n Int64))
          (match (fold (Ast.List (list (Ast.Int (BigInt.of n)))))
            ((tuple (Ast.Int v) k) (+ (Int64.of v) (* k 100)))
            (_ -1)))
        (export main)))
  (call main (: 2 Int64)) (output (: 102 Int64)))

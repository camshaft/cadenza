; breaker const-eval sweep 3 — MIS-FOLD hunt: arithmetic edge semantics INSIDE the recursive const
; evaluator must match runtime semantics (trap-for-trap, value-for-value). Each case routes the edge
; op through a const-param recursion so the general const evaluator (not just the inline simplifier)
; computes it. SOUND outcomes: clean decline (-> runtime trap, case PASSES) or a fail-loud CDZ0304
; (compile reject, grades as mismatch — classified sound by hand). A folded VALUE = miscompile.

(case "cc01 Int64.max + 1 inside const recursion traps (never folds to a beyond-width value)"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (+ 9223372036854775807 1) (f (- n 1))))
            (def (main) (f 2))
            (export main)))
  (trap   "overflow"))

(case "cc02 division by zero inside const recursion traps (never folds)"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (/ 10 (- n n)) (f (- n 1))))
            (def (main) (f 2))
            (export main)))
  (trap   "divide by zero"))

(case "cc03 Int64.min / -1 inside const recursion traps (never folds)"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (/ -9223372036854775808 -1) (f (- n 1))))
            (def (main) (f 2))
            (export main)))
  (trap   "overflow"))

(case "cc04 shift count 64 inside const recursion traps or rejects (never masks)"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (<< 1 (+ n 64)) (f (- n 1))))
            (def (main) (f 2))
            (export main)))
  (trap   "shift"))

(case "cc05 UInt8 wrapping-add VALUE computed in const recursion is the wrapped value"
  (input  (do
            (def (f (const (: b UInt8)) (const (: n Int64)))
              (if (= n 0)
                  (if (= (UInt8.wrapping-add b (UInt8.wrap 5)) (UInt8.wrap 4))
                      (trap "cc05 wrapped to four")
                      (trap "cc05 WRONG wrap value"))
                  (f b (- n 1))))
            (def (main) (f (UInt8.wrap 255) 2))
            (export main)))
  (error  CDZ0304 (message "cc05 wrapped to four")))

(case "cc06 negative modulo sign computed in const recursion is truncated (dividend sign)"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0)
                  (if (= (% -7 3) -1)
                      (trap "cc06 truncated remainder")
                      (trap "cc06 WRONG remainder sign"))
                  (f (- n 1))))
            (def (main) (f 2))
            (export main)))
  (error  CDZ0304 (message "cc06 truncated remainder")))

(case "cc07 Int64 multiply overflow inside const recursion traps (never folds wide)"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (* 4611686018427387904 4) (f (- n 1))))
            (def (main) (f 2))
            (export main)))
  (trap   "overflow"))

(case "cc08 subtraction underflow at Int64.min inside const recursion traps"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (- -9223372036854775808 1) (f (- n 1))))
            (def (main) (f 2))
            (export main)))
  (trap   "overflow"))

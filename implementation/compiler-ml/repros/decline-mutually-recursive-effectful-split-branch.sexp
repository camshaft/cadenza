;; ✅ LEAK FIXED (2026-07-14, seed rcdzc — landed by THIS loop) → now a CLEAN DECLINE (still a Todo).
;; This shape used to `cdz compile` with an INTERNAL-looking `error [CDZ0101]: unbound name ev#eff3$s0`
;; (an effect-specialization mangled name leaking into resolution). It now declines cleanly:
;;   error: this handler is not yet reducible by the tail-resumptive fold (cross-function or non-tail
;;          resume arrives in a later increment)
;; The FEATURE (specializing this shape) is still unbuilt, but the confusing internal-name leak is gone.
;; FIX (`effects.rs::specialize_recursive`): a new syntactic guard
;; `perform_and_mutual_call_in_separate_branches` declines up front when a cycle def performs a discharged
;; op in ONE `if`/`match` branch while the mutual call is in a DIFFERENT branch — the shape the
;; branch-distributed state threading + cross-def memo knot cannot yet handle. Unit test
;; `a_state_mutual_recursion_with_perform_split_from_the_mutual_call_declines_cleanly` locks in that the
;; decline does NOT leak the internal name.
;;
;; TRIGGER (minimal, this file): `ev` and `od` are mutually recursive over an Int64; `ev` performs
;; `Fresh.next` in its BASE-CASE branch (`(if (= n 0) (Fresh.next) (od …))`), and the MUTUAL recursive
;; call `(od …)` is in the OTHER branch.
;;
;; SHARP BISECTION (2026-07-14) — mutual-recursive effect specialization mostly WORKS; the gap is narrow:
;;   - A SINGLE self-recursive effectful fn compiles + runs (self-recursion + effect is fine).
;;   - MUTUAL recursion where the perform and the mutual call are in the SAME strict expression WORKS,
;;     e.g. `(def (ev n) (if (= n 0) 0 (+ (Ctr.tick) (od (- n 1))))) (def (od n) (ev (- n 1)))` — the
;;     `ev#ctx`/`od#ctx` memo knot ties correctly (this is the seed's own passing test shape).
;;   - The FAILING shape: the perform sits in a DIFFERENT branch from the mutual recursive call — the
;;     effect is in the base case and the `(od …)` call is in the recursive branch (this file), OR the
;;     effect is in the PARTNER's base case with the mutual call in its recursive branch (both leak
;;     `…#eff…$s0`). So the defect is precisely: effect-and-mutual-call in SEPARATE branches of a
;;     cycle def — the branch-with-perform's state-threading is emitted but its `#ctx$s0` specialization
;;     name is left dangling.
;; So it is NOT "all mutual recursion fails" (the seed handles the same-branch case); a blanket decline
;; would regress the working shape. The fix needs to tie the memo knot for the separate-branch case too.
;;
;; IMPACT ON THE PORT: an effect for fresh-name / fresh-variable generation (the HM gensym — a `Fresh`
;; effect handled by a state counter) is the natural way to thread unique ids through a compiler pass,
;; and a pass over an `Ast` is ALWAYS mutually recursive (`relabel(node)` ↔ `relabel-list(children)`).
;; So an effectful AST-walking pass cannot be compiled today — it must fall back to threading the counter
;; explicitly as a parameter (`src/unify.cdz`'s `State`-style, which works). A single self-recursive
;; effectful loop is fine.
(do
  (effect Fresh (op next (-> Int64)))
  (def (ev (: n Int64)) (if (= n 0) (Fresh.next) (od (- n 1))))
  (def (od (: n Int64)) (if (= n 0) 0 (ev (- n 1))))
  (def (main) (handle Fresh 0 ((next () s (resume (+ s 1) s))) (ev 3)))
  (export main))

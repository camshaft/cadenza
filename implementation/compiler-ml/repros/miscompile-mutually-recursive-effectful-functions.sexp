;; MISCOMPILE / DECLINE (2026-07-14, seed rcdzc — effect specialization). `cdz check` is CLEAN (exit 0);
;; `cdz compile -t wasm` FAILS with an INTERNAL-looking error:
;;   error [CDZ0101]: unbound name `ev#eff3$s0` — did you mean `ev#eff3`?
;; The `#eff3$s0` suffix is an effect-SPECIALIZATION mangled name (the state-threaded specialization of an
;; effectful function). It escapes into name resolution as an "unbound name" when two MUTUALLY RECURSIVE
;; functions are specialized for an effect and at least one performs an operation of that effect.
;;
;; TRIGGER (minimal, this file): `ev` and `od` are mutually recursive over an Int64; `ev` performs
;; `Fresh.next` in its BASE-CASE branch (`(if (= n 0) (Fresh.next) (od …))`), and the MUTUAL recursive
;; call `(od …)` is in the OTHER branch. The specializer mints a state-threaded `ev#eff…$s0` but leaves a
;; dangling reference, so name resolution fails at emit. No user type / list is needed.
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

; BREAKER FINDING — metaprogramming HYGIENE / capture-avoidance bug (SILENT WRONG VALUE, both backends).
;
; An `(unquote x)` whose variable NAME collides with a binder introduced INSIDE the quasiquote template is
; CAPTURED by that binder, instead of splicing x's value (resolved in the quasiquote's enclosing scope).
;
; REPRO: `(let ((x 10)) (eval (quasiquote (let ((x 1)) (+ (unquote x) 99)))))`.
;   `(unquote x)` must splice x's VALUE 10 (the outer let), so the template is `(let ((x 1)) (+ 10 99))` →
;   the inner x=1 is dead, result = 109. But the compiler returns 100 = `(+ 1 99)` — the unquoted `x` was
;   re-inserted as the NAME `x` into the reconstructed eval source, and the template's inner `let x=1`
;   CAPTURED it. A hygienic unquote splices the already-resolved value, immune to template binders.
;
; NARROWED (recompute-before-crying-bug — the bug is NAME-COLLISION-specific):
;   (unquote x) into (let ((x …)) …)  [SAME name]      -> 100  WRONG (captured)
;   (unquote n) into (let ((x …)) …)  [DIFFERENT name] -> 109  correct (no collision)
;   (unquote (+ 5 5)) into (let ((x …)) …) [non-name]  -> 109  correct (nothing to capture)
;   (unquote x) into (let ((y …)) …)  [different binder]-> 11  correct (11 = 10+1, no capture)
; So it is specifically an unquoted NAME whose identifier matches a template-introduced binder — the
; classic macro variable-capture / hygiene failure. Both wasm AND rust return the captured value (100).
;
; ROOT (hypothesis): the eval-desugar reconstructs source from the AST and splices an unquoted VARIABLE as
; its NAME node rather than its resolved value; a same-named inner binder in the template then shadows the
; spliced name. The fix must splice the unquote's already-evaluated VALUE (a literal/const node) — or
; alpha-rename the template's colliding binder — so a template binder cannot capture an unquoted variable.
;
; The cases assert the CORRECT (hygienic) result; they FAIL today (return the captured value) on both
; backends, flipping to pass when unquote-splices-value (or template-binder-renaming) is fixed.

(case "adv hygiene: an unquoted variable is captured by a same-named binder inside the template (should splice its value)"
  (doc "`(let ((x 10)) (eval (quasiquote (let ((x 1)) (+ (unquote x) 99)))))`: (unquote x) must splice the
        OUTER x's value 10, giving (let ((x 1)) (+ 10 99)) = 109. The compiler returns 100 = (+ 1 99) — the
        unquoted x was captured by the template's inner (let x 1). A hygienic unquote splices the resolved
        value, uncapturable. WRONG on both backends today.")
  (input (do (def (main) (let ((x 10)) (eval (quasiquote (let ((x 1)) (+ (unquote x) 99)))))) (export main)))
  (output (: 109 Int64)))

(case "adv hygiene: an unquoted variable with a DIFFERENT name than the template binder is not captured (control)"
  (doc "The control that PASSES today: `(unquote n)` with n=10 spliced into a template that binds `x` — no
        name collision, so the value 10 embeds correctly: (let ((x 1)) (+ 10 99)) = 109. Pins that the bug
        is specifically the name COLLISION, not unquote-into-a-binding-form in general.")
  (input (do (def (main) (let ((n 10)) (eval (quasiquote (let ((x 1)) (+ (unquote n) 99)))))) (export main)))
  (output (: 109 Int64)))

(case "adv hygiene: an unquoted non-name expression is not captured by a same-shaped template binder (control)"
  (doc "The non-name control that PASSES today: `(unquote (+ 5 5))` splices the value 10 into a template
        binding `x` — an arithmetic expression has no name to capture, so it embeds as 10: 109. Together
        with the different-name control this pins that only an unquoted NAME matching a template binder is
        mis-captured.")
  (input (do (def (main) (eval (quasiquote (let ((x 1)) (+ (unquote (+ 5 5)) 99))))) (export main)))
  (output (: 109 Int64)))

## UPDATE 2026-07-16 (v-metaprogramming): CONFIRMED + localized. The constructed Ast is CORRECT (,x folds to Ast.Int 10); bug is PURELY in eval_ast::reconstruct — ,x reaches it as (ast-lift x) name-preserved, spliced as bare NAME, and reconstruct has NO binder-awareness so the template (let x 1) captures it. FIX = ALPHA-RENAME the capturing template binder (NOT splice-the-value — runtime x has no compile-time literal + also mis-captures). Implementing carefully. Keep open till it lands.

## BREAKER FOLLOW-UP 2026-07-16: the capture happens through a LAMBDA binder too, not only `let` — verified
## `(let ((z 7)) (eval (quasiquote ((fn (z) (+ (unquote z) 99)) 3))))` returns 102 = (+ 3 99) (captured to
## the lambda param 3) instead of 106 = (+ 7 99) (splice value 7). Both backends. So the alpha-rename must
## cover ALL template binders — `let`, `fn`/lambda params (and presumably `match`-arm binders + a `do`'s
## nested `def`). The extra case below pins the lambda-param face; when you alpha-rename, make it binder-
## kind-agnostic so a fn param can't capture an unquote either.

(case "adv hygiene: an unquoted variable is captured by a same-named LAMBDA parameter in the template"
  (doc "The lambda-binder companion of the let-binder capture above: `(let ((z 7)) (eval (quasiquote
        ((fn (z) (+ (unquote z) 99)) 3))))`. (unquote z) must splice the OUTER z's value 7 → the template
        ((fn (z) (+ 7 99)) 3) = 106 (the lambda's own z=3 is irrelevant to the spliced value). The compiler
        returns 102 = (+ 3 99) — the unquoted z was captured by the lambda param z. Confirms the capture is
        not `let`-specific: any template-introduced binder (here a fn param) captures a same-named unquote.
        The alpha-rename fix must be binder-kind-agnostic. WRONG on both backends today.")
  (input (do (def (main) (let ((z 7)) (eval (quasiquote ((fn (z) (+ (unquote z) 99)) 3))))) (export main)))
  (output (: 106 Int64)))

; ENHANCEMENT (v-effects, 2026-07-18, found by self-directed probing). NOT a miscompile — a clean DECLINE
; ("not yet reducible" todo, cdz check rc=0, no $s/#eff leak). An ABORTIVE perform inside a SHORT-CIRCUIT
; CONNECTIVE that is itself an if-CONDITION over-declines, though a BARE abort-in-condition folds fine.
;
; DECLINES:  (handle Bail 0 ((bail (n) s n)) (if (and b (> (Bail.bail 7) 0)) 100 200))
; FOLDS (→7): (handle Bail 0 ((bail (n) s n)) (if (> (Bail.bail 7) 0) 100 200))   [my 2f24898a abort-in-cond]
;
; So the bare abort-in-condition folds (the whole handle collapses to the abort value 7 when reached), but
; wrapping the abort's perform in a connective (and/or) in the condition position declines. Expected value for
; the connective form, b=true: the (and b (Bail.bail 7)) evaluates b (true) then (Bail.bail 7) which ABORTS →
; the whole handle = 7. b=false: short-circuits, cond=false → else 200.
;
; LIKELY ROOT: hoist Site 3 desugars (and b (Bail.bail 7)) → (if b (Bail.bail 7) false); the abort now sits in
; a branch of a condition-position if. The abortive guards (body_has_unsound_abortive_perform / the
; specialize decline) treat an abort under a conditional-in-a-condition as non-tail-unsound and decline,
; where the bare form's abort-in-condition fold (2f24898a) handled the un-connective'd shape. Site 5 (my
; recent connective-in-condition hoist) is tail-resumptive-only; the ABORTIVE analogue in condition position
; is not built. FIX (when prioritized): extend the abort hoist (hoist_conditional_abort) or Site-5-analogue to
; lift a connective-desugared abort in a condition to a foldable position, OR let the abort-in-condition fold
; see through the connective desugar. Gate adversarially (abort in and-lhs vs and-rhs vs or; b=true/false;
; abort value type consistency — abortive folds are miscompile-prone, always probe compound/non-tail after).
;
; SEVERITY: none (clean decline, safe). No forcing consumer. Low priority — an abort inside a connective
; condition is a rare hand-written shape. Promote if a real program needs it. v-effects territory.

(do
  (effect Bail (op bail (-> Int64 Int64)))
  (def (main (: b Bool))
    (handle Bail 0 ((bail (n) s n))
      (if (and b (> (Bail.bail 7) 0)) 100 200)))
  (export main))

;; REFINED ROOT-CAUSE (v-effects 2026-07-18): the And-desugar (effects.rs:2391) DOES fire —
;; `(and b (> (Bail.bail 7) 0))` → `(if b (> (Bail.bail 7) 0) false)`. But that lands as the
;; CONDITION of the enclosing `(if <cond> 100 200)`. hoist_once has no site for an `if` whose
;; CONDITION is itself an `if`-with-abort. Sound fix = distribute the outer if into the inner
;; branches: `(if (if b X false) t e)` ≡ `(if b (if X t e) (if false t e))`. Bounded + sound
;; (t/e duplicated statically, each path runs one copy). PARKED: no forcing consumer; won't build
;; a speculative hoist site. Bare abort-in-if-condition (no connective) already folds.
; RESOLVED (v-effects, 2026-07-18, MR a38bdd243): outer-if-through-condition hoist + tail-condition-abort capturable guard. b=true→7, b=false→200, adversarial sweep clean.

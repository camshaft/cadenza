;; FALSE REJECT (CDZ0101, NOT a miscompile — found by v-patterns adversarial probing 2026-07-15). A user
;; `(guard …)` on a list arm whose leading element is a NESTED LIST, whose guard cond reads the INNER list's
;; element binder — `(guard (list (list a .. r1) .. r2) (> a 3))` — falsely reports CDZ0101 "unbound name
;; `a`" at BOTH cdz check AND compile. A valid well-typed program declined.
;;
;; ISOLATION (trunk after Inc-35):
;;   - the SAME nested-list element in the BODY (no guard) `((list (list a .. r1) .. r2) a)` → WORKS (Inc 14).
;;   - a guard on a PLAIN list element `(guard (list a .. r) (> a 3))` → WORKS (Inc 5).
;;   - a guard on a nested TUPLE-in-list / ctor-in-list element → WORKS (Inc 35 / Inc 34).
;;   → only {a NESTED-LIST leading element} × {a user guard reading its inner binder} triggers it.
;;
;; ROOT (same CLASS as Inc-34 ctor-list-element + Inc-33 literal): `desugar_refutable_nested_list_elements`
;; (lower.rs, Inc 14) rewrites `(list (list a .. r1) .. r2)` to a fresh binder + an inner-LENGTH guard + a
;; BODY re-match that binds `a`/`r1`. The user's guard cond is combined at the OUTER guard level, where the
;; inner binders are NOT yet in scope (they bind only in the body re-match, AFTER the guard) → `a` unbound.
;; This is the nested-list twin of the ctor-list-element guard bug fixed in Inc-34 (commit 8d7415eb0): there
;; the fix folded the user cond into the innermost disc-test's matched arm (real pattern, payloads live).
;;
;; FIX DIRECTION (v-patterns seam, lower.rs `desugar_refutable_nested_list_elements`): mirror Inc-34 — fold
;; the user cond into the inner-length-guard's matched structure so the inner list's binders (`a`/`r1`) are
;; in scope for it, rather than ANDing the user cond outside where only the fresh outer binder is bound. NOT
;; a miscompile — declines honestly; LOW-MEDIUM value (a niche nested combination). Owned by v-patterns.
(do
  (def (f (: xs (List (List Int64))))
    (match xs
      ((guard (list (list a .. r1) .. r2) (> a 3)) a)
      (_ -1)))
  (def (mk (: k Int64)) (list (list k)))
  (def (main (: k Int64)) (f (mk k)))
  (export main))

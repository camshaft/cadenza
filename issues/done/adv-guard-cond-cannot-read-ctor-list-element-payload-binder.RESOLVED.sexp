;; FALSE REJECT (CDZ0101 over-rejection, NOT a miscompile — found by v-patterns adversarial probing
;; 2026-07-15). A user `(guard …)` on a list arm whose leading element is a REFUTABLE CTOR whose payload
;; binder the guard cond reads — `(guard (list (Op.Add n) .. r) (> n 3))` — falsely reports CDZ0101
;; "unbound name `n`" at BOTH `cdz check` AND `cdz compile` (a consistent resolve-side rejection, not a
;; check-vs-emit gap, not a trap). A valid, well-typed program is declined.
;;
;; ROOT: `desugar_refutable_ctor_list_elements` (lower.rs) replaces the ctor element `(Op.Add n)` with a
;; fresh binder `__lc`, folds a DISCRIMINANT-test into the arm guard, and DEFERS the payload binding (`n`)
;; to a BODY RE-MATCH `(match __lc ((Op.Add n) body) (_ trap))` — which runs AFTER the guard. The user's
;; guard cond `(> n 3)` is combined into the arm guard (`(and <disc-test> (> n 3))`), but `n` is only bound
;; in the body re-match, NOT in the guard scope — so the user cond sees `n` unbound.
;;
;; ISOLATION (all on trunk after Inc-33):
;;   - guard reads a PLAIN list-element binder `(guard (list n .. r) (> n 3))` → WORKS (Inc 5). [OK]
;;   - guard reads a CTOR-element PAYLOAD binder `(guard (list (Op.Add n) .. r) (> n 3))` → CDZ0101. [BUG]
;;   - the ctor element WITHOUT a user guard `(list (Op.Add n) .. r)` → WORKS (Inc 12). [OK]
;;   → only {refutable ctor list element} × {user guard reading its payload} triggers it.
;;
;; FIX DIRECTION (v-patterns seam, lower.rs): the ctor desugar must bind the ctor's payload for the USER
;; guard cond's scope, not only the body re-match. Options: (a) bind the payload in the guard structure
;; (e.g. the disc-test `(match __lc ((Op.Add n) (and true <user-cond>)) (_ false))` — fold the user cond
;; INTO the disc-test's matched arm so `n` is in scope), rather than ANDing the user cond outside; (b) a
;; resolve-side `guard_cond` case that descends a ctor-element payload binder (the analogue of Inc-5's
;; `guard_cond_list_binds`, but through the ctor element). Option (a) is likely cleaner (keeps the payload
;; binding local to the disc-test match). NOT a miscompile — declines honestly; MEDIUM value (the idiomatic
;; "match a tagged head and guard on its payload" shape). Owned by v-patterns.
(do
  (type Op (Add Int64) (Neg Int64))
  (def (f (: xs (List Op)))
    (match xs
      ((guard (list (Op.Add n) .. r) (> n 3)) n)
      (_ -1)))
  (def (main) (f (list (Op.Add 5))))
  (export main))

;; DECLINE (2026-07-14): "borrowing op operand has an ownership this backend cannot yet prove".
;; `cdz check` is CLEAN; `cdz compile` (and `cdz test`) DECLINE at emit.
;;
;; TRIGGER: a `String` that (a) originates from a `Map.lookup` (a borrowed heap value out of the map),
;; (b) is RETURNED from a function, then (c) is used as an operand of `String ==` in the CALLER.
;;   `f` returns the looked-up String; `(= (f …) "z")` in the test → decline.
;; CONTROL (PASSES): do the `==` INSIDE the lookup's match arm (`((Some s) (= s "z"))`) — never return
;; the borrowed String across the call boundary. So it is the returned-borrowed-String-then-compared
;; path the backend's borrow checker can't yet prove.
;;
;; Surfaced building the port's substitution pass (subst returns a Map-looked-up Ast whose payload a
;; test compared). Minimal form here uses `Map String String` — no user types needed.
;;
;; RELATED (a WRONG-VALUE, context-dependent sibling of this decline): a `String ==` on a value that
;; flowed through a MISSED `Map.lookup` (the `None → return node` branch) can also miscompile — but
;; only past a per-module def/test-count THRESHOLD (it passes standalone or with few siblings, fails in
;; a larger module, like the slot-alias bug). The port's `src/subst.cdz` sidesteps BOTH by checking a
;; result's SHAPE (`match … ((Ast.Name _) …)`) rather than `String ==`-ing an extracted payload.
(do
  (def (f (: m (Map String String)) (: k String))
    (match (Map.lookup m k)
      (((. Option Some) s) s)
      (((. Option None) _) "?")))
  (@ test (def (t) (if (= (f (Map.insert (map) "y" "z") "y") "z") unit (trap "expected z")))))

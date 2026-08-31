; BUG (edge-hunt, v-rcdzc-ts-2 batch-97; routed to v-ast-compound via v-rcdzc-test-shrink) —
; MISLEADING DIAGNOSTIC: a `#set(<binder>)` match PATTERN reports CDZ0101 "unbound name `a`"
; (treating the pattern's binders as value references) instead of a coded reject explaining that a
; set pattern cannot bind its elements (a set is unordered — there is no positional element to bind).
;
; Observed (trunk 170fded390, `cdz check`):
;   (match #set(1 2) (#set(a b) 9) (_ 0))     → CDZ0101 "unbound name `a`" (×2)   ← THIS filing
;   (match #set(1 2) (#set(a)   9) (_ 0))     → CDZ0101 "unbound name `a`"
;   (match #set(1 2) (#set(1 2) 9) (_ 0))     → OK (a LITERAL set pattern matches by containment)
;   (match 5         (#set(a)   9) (_ 0))     → CDZ0201 "a `(set …)` pattern matches only a set
;                                               scrutinee" (kind-check OK) + a cascaded CDZ0101 on `a`
;
; The literal-element `#set(…)` pattern is a valid CONTAINMENT (subset) matcher — now pinned in
; spec/semantics/19-sets.sexp ("Set patterns match by CONTAINMENT", batch-97). The defect is only the
; BINDER-element form: a set has no positional structure, so `#set(a)` cannot bind `a` to "an element",
; but the matcher lets the binder fall through to value scope → a spurious "unbound name". The RIGHT
; diagnostic is a coded, actionable reject in the family of the existing kind-check message ("a `(set …)`
; pattern matches only a set scrutinee") — e.g. "a set pattern cannot bind its elements; match on
; `Set.contains` / `Set.len` instead". Exact code + wording are v-ast-compound's call (their
; native-compound-pattern domain; sibling of the #map/#set matcher bugs they fixed).
;
; Severity: moderate diagnostic-quality — a user writing a natural (if unsupported) set-destructuring
; pattern is sent to chase a phantom unbound name rather than told the real constraint.
;
; The `#set(<binder>)` INPUT ML-round-trips (verified: roundtrip 1 ok / 0 fail), so this case is
; expressible as a corpus reject once the fix lands. GRADED AGAINST INTENT below (RED today: actual is
; CDZ0101 "unbound name"), flips GREEN when the coded set-cannot-bind reject lands — at which point
; v-rcdzc-ts-2 pins it into 19-sets next to the containment-pattern cases.

(case "a set pattern with binder elements is a coded reject, not a spurious unbound-name"
  (doc    "A `#set(<binder>)` pattern cannot bind (a set is unordered). It must be a coded, actionable
           reject naming the set-binding constraint — NOT the CDZ0101 'unbound name' it spuriously gives
           today by letting the binder fall through to value scope. Exact code/message TBD by
           v-ast-compound; asserted here as a set-kind reject that does not misreport an unbound name.")
  (input  (do (def (main) (match #set(1 2) (#set(a b) 9) (_ 0))) (export main)))
  (error  CDZ0201 (message "set") (not "unbound name")))

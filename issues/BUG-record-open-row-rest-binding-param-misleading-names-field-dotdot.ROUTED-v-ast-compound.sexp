; BUG (edge-hunt, v-rcdzc-ts-2 batch-102; routed to v-ast-compound via v-rcdzc-test-shrink) —
; MISLEADING DIAGNOSTIC + BINDING/MATCH ASYMMETRY: a record OPEN-ROW REST pattern `(.. rest)` in a
; BINDING position (a def/fn param) is rejected CDZ0203 claiming the value "names field `..`" — the
; binding-position path mis-reads the rest SYNTAX `..` as a FIELD NAME, rather than recognizing the
; open-row rest the match-arm path handles.
;
; Observed (trunk 1cbdadf16f, `cdz check`):
;   (def (get #record((= x a) (.. rest))) a)   → CDZ0203 "a record binding pattern names field `..`,
;                                                  which the bound value of type (Record …) does not have"
;   (def (get #record((.. rest))) 0)            → same CDZ0203 "names field `..`"
;   MATCH-arm form WORKS (spec-blessed, corpus 05 "a record pattern MAY end in a trailing `.. rest`"):
;       (match r (#record((= x a) (.. rest)) (+ a (. rest y))) (_ 0))   → clean, binds rest
;   LIST control — the analogous zero-leading rest WORKS as a binding param (irrefutable):
;       (def (f #list(.. rest)) 0)              → clean (binds the whole list)
;
; Two defects:
;   (a) DIAGNOSTIC: the message "names field `..`, which the value does not have" is misleading — `..` is
;       the open-row REST syntax, not a field the record lacks. The record-binding-param resolver does not
;       recognize `(.. rest)` and treats `..` as a field label.
;   (b) BEHAVIOR (needs an intent ruling): an open-row rest `#record((= x a) (.. rest))` — and the
;       rest-only `#record((.. rest))` — is arguably IRREFUTABLE (it matches every record having the named
;       fields, binding the residual to `rest`), exactly as the LIST zero-leading rest `#list(.. rest)` is
;       accepted in a binding position. If so it should be ACCEPTED there, not rejected. If the design
;       instead disallows a record rest in a binding position, the reject must name the REAL reason
;       (rest-in-binding refutability / unsupported), never "names field `..`".
;
; Severity: moderate — natural record-destructuring binding shape; the misleading message sends the author
; to chase a phantom field. v-ast-compound's native-compound-pattern domain (sibling of the #set-element
; binding-path CDZ0306 they fixed in #6693, and the wrong-kind-scrutinee binding-path class).
;
; NOT pinning an expectation here (fix shape TBD by v-ast-compound + v-spec-oracle, per the #set precedent
; where my initial hypothesis was spec-corrected). Post-fix, v-rcdzc-ts-2 pins the ruled behavior in 05.
;
; (No graded (case …) — the intended code/message is undecided; this is a routed-bug record. The MATCH-arm
; open-row rest is already pinned at 05 "a record pattern MAY end in a trailing `.. rest`".)

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
; INTENT RULING LANDED — v-spec-oracle #6723, core-semantics §"A Binding Position Accepts An Irrefutable
; Pattern". The binding-path fix-class (record trailing `.. rest`, tuple trailing `.. rest`, #set-element
; #6693) resolves as follows (my initial "should be accepted as irrefutable, like list #list(.. rest)"
; hypothesis was VINDICATED for the trailing-rest cases):
;   - record TRAILING `(.. rest)` binding param  → BINDS, runs (an ACCEPT/output case, NOT a reject)
;   - tuple TRAILING `.. rest` binding param      → BINDS, runs (ACCEPT/output)
;   - map-rest + list-LEADING-rest binding param  → CDZ0210 (stays a refutable-reject)
;   - a genuine arity/shape mismatch              → CDZ0201
;
; STILL BLOCKED: v-ast-compound has NOT yet landed the `check_binding_pattern` fix (as of trunk ff4a9668bd
; the record form still gives the spurious CDZ0203 "names field `..`" and the tuple form CDZ0201 "not a
; tuple/record/constructor"). POST-FIX, v-rcdzc-ts-2 pins the RUN cases in 05 (record + tuple trailing-rest
; binding params bind + run, output the bound value), citing §"A Binding Position Accepts An Irrefutable
; Pattern" / #6723. The MATCH-arm open-row rest is already pinned at 05 ("a record pattern MAY end in a
; trailing `.. rest`"). Turnkey post-fix pin targets:
;   (def (get #record((= x a) (.. rest))) a) applied to #record((= x 5) (= y 6))   → 5
;   (def (f #tuple(a .. rest)) a) applied to #tuple(3 4 5)                          → 3
;
; PROGRESS (trunk 1edcb142cf): the TUPLE half of check_binding_pattern LANDED — a tuple trailing-rest
; binding param now BINDS/runs. v-rcdzc-ts-2 PINNED it in 02-binding-and-control ("a tuple trailing-rest
; parameter binds the leading element (irrefutable binding position)" -> 3, + the rest->sub-tuple face -> 4),
; citing #6723. The list-leading-rest and map-rest reject faces (-> CDZ0210) verified still correct.
; STILL OPEN: the RECORD trailing-rest binding param STILL gives the spurious CDZ0203 "names field `..`"
; (the record half of check_binding_pattern is not yet landed). When it lands (record trailing-rest should
; BIND/run per #6723), v-rcdzc-ts-2 pins the record run case (-> 5) in 05 and renames this issue DONE-PINNED.
;
; RESOLVED (binding-rest fix-class COMPLETE): tuple half fixed #6732, record half fixed #6763. Both
; trailing-rest binding params now BIND/run (irrefutable) per core-semantics §"A Binding Position Accepts An
; Irrefutable Pattern" (#6723/#6750). v-ast-compound (fix-class owner) pinned BOTH corpus-05 witnesses
; themselves (tuple @05:580 "a tuple trailing-rest is IRREFUTABLE, so it binds in a BINDING-PARAM position";
; record @05:617 "a record pattern with a trailing rest binds the named fields and the residual record").
; The map-rest + list-LEADING-rest reject faces correctly stay CDZ0210. No double-pin from v-rcdzc-ts-2.
; (My batch-97 edge-hunt find → filed repro → routed → #6723 oracle ruling vindicated the irrefutability
; hypothesis → v-ast-compound fixed both slices + pinned inline. Loop closed.)

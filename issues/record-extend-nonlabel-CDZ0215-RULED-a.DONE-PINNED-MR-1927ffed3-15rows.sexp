;; HELD-FOR-RULING (corpus-bugfix, 2026-07-28): breaker QUESTION-grade (issue 000000017900).
;; Record.extend with a BARE IDENTIFIER as the #field-label operand PUNS the identifier into a static
;; label rather than rejecting a wrong-kind operand. CONFIRMED trunk 51c0a2983 both backends:
;;   (Record.extend (record (x 10)) fname k) with `fname` an UNDECLARED bare identifier -> COMPILES (no
;;   CDZ0101 unbound), creates a field literally named `fname`; (. wide fname) reads k back (=7). wasm+rust
;;   agree. No miscompile (consistent value) — a SEMANTIC FOOTGUN: a user passing a computed Symbol
;;   expecting dynamic field-naming silently gets a field named after their variable.
;; SPEC: 15-rows:496 "extending a record adds a new field" — the field operand is "a `#field` label operand
;;   (a static label, NOT a runtime value)". So the label position is statically a label; a bare identifier
;;   there is currently PUNNED to a label instead of (a) rejected as a non-label operand or (b) blessed.
;; LEAN (corpus-bugfix): (a) REJECT a non-`#label` operand with a coded error naming the static-label rule.
;;   The spec explicitly says "not a runtime value", and the read-;-as-Name finding is the same reinterpret-
;;   instead-of-reject class the operator has been closing. Silent pun = worst of both.
;; ROUTED to concierge for the ruling (ask sent 2026-07-28). ON RULING:
;;   (a) reject -> pin an (error CDZNNNN) case here + route the reject to v-inference/rcdzc resolve.
;;   (b) bless the pun -> pin a VALUE case documenting identifier-position = label (=7), doc the pun in 15-rows.
;; Same family as [[check-corpus-ruling-before-routing-a-reject-gap]]. No unilateral pin.

;; ============================================================================
;; RULED = (a) REJECT (concierge, 2026-07-29, answer 17913). Matches my lean; reject-safety CONFIRMED:
;; EVERY landed Record.extend/with pin uses an explicit #"label" operand (extend #"b", extend #"a"
;; reject-present, with #"b", extend-without-inverse #"b") -> a reject of non-#label name-INTRODUCTION
;; operands flips ZERO green pins. SCOPE: reject ONLY the name-introduction operand of extend/with; the
;; READ/DROP ops (. r x)/pop/without/project legitimately take a BARE label and MUST stay valid.
;; SEMANTIC OUTCOME: extend/with whose field-name operand is NOT a #field label (bare identifier or any
;; runtime-value expr) MUST reject at compile time with a coded diag naming the static-label rule.
;;
;; CURRENT trunk 514ef49d0: still PUNS (compiles, 658 bytes, NO reject) -> this is PIN-ON-FIX, not landable
;; now. CODE ANALYSIS (corpus-bugfix): existing CDZ021x row-op family — CDZ0211 present-field-conflict,
;; CDZ0212 AbsentField (a valid #label absent from the record). The pun is DISTINCT: the operand isn't a
;; valid static label at all (a bare identifier reinterpreted). Likely a NEW CDZ021x "field-name operand
;; must be a #field label, not a runtime value" resolve/check condition (or nearest-fit existing static-
;; label-violation code — v-inference/rcdzc's call, don't mint if one fits). ROUTED to v-inference/rcdzc
;; (Record.extend/with operand resolution). ON FIX: gate x3 -> (error CDZ0NNN); pin into 15-rows beside
;; the extend/with pins; baseline x3 (a correct reject keys as `pass`).

(case "extending a record with a non-#label (bare identifier) field-name operand is rejected"
  (input  (do (def (main (: k Int64)) (let ((wide (Record.extend (record (x 10)) fname k))) (. wide fname))) (export main)))
  (call   main (: 7 Int64)) (error CDZ0212))

;; UPDATE (2026-07-29): v-inference built the reject = CDZ0215 (RecordFieldNameNotLabel, commit f52d35f95).
;; My 'zero green-pin flip' claim was WRONG — the reject flips 9 EXISTING green cases (bare-name extend/with
;; call-sites in 15-rows + 20-structural). SEQUENCE (b, agreed): I landed a GATE-NEUTRAL #label migration
;; FIRST (MR e2f4e2af0 — the 9 sites -> #"label", byte-identical compile, --check 0 regressed x3, no baseline
;; change), THEN v-inference lands CDZ0215 on top (now gate-clean, no bare-name cases left), THEN I flip the
;; bare-name pin below to (error CDZ0215). Placeholder code in the case above updated to CDZ0215.
;; ON LAND (CDZ0215 reject, after migration integrates): gate x3 -> (error CDZ0215); pin into 15-rows; baseline x3.

;; MR'd (v-inference, 2026-07-29): combined with CDZ0214 in 364077bf4 (queued; gate CLEAN 5163/0/0 now that
;; my #label migration landed). ON LAND: gate the (error CDZ0215) bare-name reject x3; pin into 15-rows.

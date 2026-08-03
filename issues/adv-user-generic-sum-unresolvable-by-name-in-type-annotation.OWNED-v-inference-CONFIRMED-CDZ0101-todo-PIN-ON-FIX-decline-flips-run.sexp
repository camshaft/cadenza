; INFERENCE gap (v-inference-owned, filed 2026-08-03, MED-value/LOW-urgency): a USER-declared GENERIC
; sum is NOT resolvable by NAME in a TYPE-EXPRESSION position (param annotation OR variant payload),
; while built-in generics (Option/List) and monomorphic user sums resolve there fine, and the same
; user generic resolves fine in VALUE positions (construction + match + unannotated-param inference).
;
; CONFIRMED on trunk (corpus-bugfix): the case below DECLINES CDZ0101 "unbound name Container" at the
; (Container Int64) annotation; verdict todo (expected run 7). Also fails: bare (: b Container) →
; "unknown type Container"; variant payload (type W (Wrap (Container Int64))) → same.
; CONTRAST (all WORK): (: b (Option Int64)) / (: a (List Int64)); (: c Color) monomorphic user sum;
; the SAME (Container a) in value positions.
;
; ROOT (v-inference): a user GENERIC sum's type NAME misses the type-annotation/payload resolve path
; (bare or applied), unlike a monomorphic user sum (in the type-decl index) or a built-in generic
; (prelude). Pre-existing + untested corpus territory (no (: x (UserGeneric T)) annotation cases exist).
; v-inference OWNS the fix (resolve/infer, dedicated slice). Workaround: drop the annotation (inference
; handles it). PIN-ON-FIX: this case flips decline→run(7) when v-inference lands the fix — pin it then.
(case "a user-declared generic sum resolves by NAME in a parameter type annotation"
  (input  (do
            (type (Container a) (Full a))
            (def (unwrap (: b (Container Int64))) (match b ((Full v) v)))
            (def (main (: k Int64)) (unwrap (Full k)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64)))

; FINDING #48 (breaker, 2026-08-01) — gap-B SIBLING FACE: the unbound-type decl-validation gap
; covers RECORD declarations too, not just sum-variant payloads. cdz run-ml on trunk 6030aa26f:
; "value 42"; rcdzc rejects CDZ0101 on all 3 targets. Same root cause as gap B (no declared-
; type-name registry in the reader), filed so the registry fix walks record FIELD types as well
; as sum ctor payloads. Corpus pin HELD at breaker with the gap-B pin; both release together.
;
; Face sweep for the registry fix (all rcdzc-reject CDZ0101 x3):
;   value 42 (GAP):  (type R (record (: field NoSuchField)))   <- this file
;   value 42 (GAP):  (type Unused (Mk NoSuchPayload))          <- gap B, already routed
;   declined (ok):   unbound RETURN-annotation type; unbound type inside (List _) generic
;                    application; unbound effect-op ARG type — ML declines these, no diff.

(case "an unbound FIELD type in a never-constructed record type declaration is rejected"
  (input  (do
            (type R (record (: field NoSuchField)))
            (def (main) 42)
            (export main)))
  (error  CDZ0101))

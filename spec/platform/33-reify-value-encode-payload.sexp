; Platform-conformance suite — the R2 in-fold Value.encode boundary composes with the reify boundary: a reducer
; encodes a STRUCTURED value to Bytes IN-FOLD via Value.encode (the R2 canonical binary-AST encoder, ∀a. a → Bytes,
; total) and carries the result as the payload of a REIFIED async world-effect. Case 32 pins the reify of a bare-Bytes
; payload; this pins that a structured value (a record) can be canonically encoded in-fold and ride a reified effect —
; the composition the phase-2 flag-day CARVED OUT pending R2 (Structured payloads reify-declined without an in-fold
; value-encode primitive; R2 landed it, 057e19950). Value.encode(record) -> Bytes is a single-Bytes payload from the
; reify's view (no multi-arg / no descriptor-decline), so it reifies with a schema_descriptor -> the kernel hashes it ->
; req.schema_hash Some. Pins the R2-encode -> reify -> parse -> observe chain end-to-end from a .cdz-dialect-equivalent
; sexp reducer, closing the "Structured payload needs R2" carve-out for the reified-effect path.
;
; The reducer's apply RESULT-TYPE is the 4-field reify record {correlation, kind, payload, schema_descriptor}
; (name-sorted) — the descriptor-building-effect shape (Beat.beat: Bytes->Unit; the payload arg is the encoded Bytes).
; Asserts the effect is observed (family effect/Beat) carrying a schema_hash (present) — non-vacuous via the same
; producer-bake -> schema_hash pin case 32 established (a reify that dropped the descriptor would fail the schema-hash
; assertion; case 32's pre-fix FAIL is the standing witness).

(platform-case "a reducer in-fold Value.encodes a structured value and reifies an effect carrying the encoded payload (R2 composes with reify)"
  (doc "The R2-encode + reify composition: on its message kickoff the reducer builds a structured record
        (record (a 7) (b 9)), encodes it to Bytes IN-FOLD via (. Value encode) (the R2 canonical encoder), and
        performs (host-delegated) the target-free async world-effect Beat.beat with the encoded Bytes as the
        single payload arg. rcdzc reifies the perform to the 4-field request record (correlation None, kind
        effect/Beat, payload Some(encoded-bytes), schema_descriptor); the kernel decodes+hashes the descriptor
        -> req.schema_hash Some; the runner observes the effect + emits effect-schema-hash present. Asserts the
        observed effect (from s, family effect/Beat, schema-hash present) + quiescent. Pins that a STRUCTURED
        payload — the phase-2 carve-out pending R2 — now rides a reified effect via the in-fold Value.encode
        prim (057e19950). The payload byte-value is NOT pinned to a literal (the binary-AST encoding is v-rb's
        to own; present-schema-hash + observed-effect is the stable end-to-end pin, per the case-32 discipline).")
  (session "s" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (effect Beat (op beat (-> Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (Beat) (list (Beat.beat ((. Value encode) (record (a 7) (b 9))))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: schema_descriptor Bytes)))))
      (export apply))))
  (kickoff "s" (inbound "message" (: unit Unit)))
  (expect-effects (effect (from "s") (family "effect/Beat") (schema-hash present)))
  (end-state "s" (status quiescent)))

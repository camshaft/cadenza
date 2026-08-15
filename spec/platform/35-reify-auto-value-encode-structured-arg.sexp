; Platform-conformance suite — reify AUTO-value-encodes a STRUCTURED op-arg (the reify structured-payload carve-out,
; v-rust-backend's reify wiring). DISTINCT from case 33: there the reducer calls (. Value encode) EXPLICITLY in-fold
; and passes the resulting Bytes as a single-BYTES payload arg (reify sees Bytes, no wrapping). HERE the op-arg is
; STRUCTURED (a record) passed DIRECTLY — `Beat.beat((record (a 7) (b 9)))` where `beat`'s declared arg type is the
; record, not Bytes — and reify_effect_to_tuple itself inserts the Value.encode: a single structured (non-Bytes)
; payload arg reifies as `(Some ((. Value encode) arg))` -> Core::ValueEncode with desc = sum_shape_descriptor(arg-ty)
; (v-rb wiring 543c4c6b7). This is the co-gate for that wiring: PRE-wiring reify DECLINES a structured payload arg
; (the "needs an in-fold value-encode primitive" carve-out), so this case is a standing NON-VACUOUS witness — it
; DECLINES (session errors, no effect observed) without the reify auto-encode, and PASSES once it lands, exactly the
; case-32 pre-fix-FAIL discipline. The kernel treats the resulting payload OPAQUELY (parse_effect_request reads it as
; Payload::Inline, never decoding it — the effect-IDENTITY descriptor is the SEPARATE schema_descriptor field the
; reify emits + the kernel re-hashes -> req.schema_hash Some), so present-schema-hash + observed-effect is the stable
; end-to-end pin; the payload byte-value is v-rb's canonical-encoding to own, not pinned to a literal.
;
; This mirrors the real carve-out consumers agent_loop Model.request(ModelRequest record) + close/reply(structured):
; a reducer performing a world-effect whose op-arg is a structured value, which reify now auto-encodes rather than
; declining. Named `Beat` (target-free, host-delegated) like case 33 so the shape is minimal + descriptor-bearing.

(platform-case "reify auto-value-encodes a structured op-arg passed directly to a world-effect perform (reify structured-payload carve-out)"
  (doc "On its message kickoff the reducer performs the target-free async world-effect Beat.beat with a STRUCTURED
        record arg (record (a 7) (b 9)) passed DIRECTLY — beat's declared arg type is the record, NOT Bytes, and the
        reducer does NOT pre-encode it. rcdzc's reify (reify_effect_to_tuple) auto-inserts the in-fold Value.encode:
        the single structured payload arg reifies as (Some (Value.encode arg)) -> Core::ValueEncode, desc =
        sum_shape_descriptor(record-ty). The 4-field reify record {correlation None, kind effect/Beat, payload
        Some(encoded-bytes), schema_descriptor} rides to the kernel; parse decodes+hashes the descriptor -> schema_hash
        Some; the runner observes the effect. Asserts the observed effect (from s, family effect/Beat, schema-hash
        present) + quiescent. PRE-wiring this DECLINES (structured payload arg -> reify decline, no effect, session
        errors) — the standing non-vacuous witness; POST-wiring (v-rb 543c4c6b7) it PASSES. Payload byte-value NOT
        pinned (v-rb's canonical encoding); present-schema-hash + observed-effect is the stable pin (case-32 style).")
  (session "s" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (effect Beat (op beat (-> (Record (: a Int64) (: b Int64)) Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (Beat) (list (Beat.beat (record (a 7) (b 9)))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: schema_descriptor Bytes)))))
      (export apply))))
  (kickoff "s" (inbound "message" (: unit Unit)))
  (expect-effects (effect (from "s") (family "effect/Beat") (schema-hash present)))
  (end-state "s" (status quiescent)))

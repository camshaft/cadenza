; Platform-conformance suite — I2 slice-9: a handler REPLIES an Err outcome and the caller observes it, on the
; A1 bytes fold boundary. The full handler-sent-Err round-trip, unlocked by the err-reply host half (906aba179:
; the ReplyExecutor decodes the reply payload as an Ok/Err outcome value-form). A caller performs an "ask"
; effect; the handler serves it and replies Structured(Err(record{message, retryable: false})) — a permanent
; failure — via the guest Payload sum; the ReplyExecutor settles the caller with EffectOutcome::Err, surfaced
; on the effect-result's outcome child. The caller matches (. e outcome) -> Err(rec) and records iserr=1 +
; retry from the typed retryable bool. Complements 22 (Err via the unhandled-effect permanent error) and 13
; (Ok reply): this is the handler-CHOSEN Err — a served handler deliberately failing the effect, with a typed
; retryability the caller faithfully surfaces.

(platform-case "a handler replies an Err outcome (retryable false) and the caller observes the failure via the effect-result outcome"
  (doc "The handler-sent-Err round-trip: kickoff message -> caller performs an ask effect (forwarded to the
        handler that serves ask); the handler replies Structured(Err(record{message, retryable: false})) — a
        deliberate permanent failure; the ReplyExecutor settles the caller with EffectOutcome::Err, surfaced on
        the effect-result outcome child. The caller matches (. e outcome) -> Err(rec), records iserr=1 and
        retry=0 (the typed retryable bool). Asserts iserr=1 (the caller observed the handler's Err, not an Ok)
        + retry=0 (the handler chose permanent) + both quiescent. Distinct from 22 (Err via unhandled-effect,
        no handler) and 13 (handler replies Ok); this pins a served handler deliberately failing an effect.")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "effect-result")
             (host (kv)
               (match (. e outcome)
                 ((Some o) (match o
                   ((Ok _rp) (list))
                   ((Err rec)
                     (do
                       (kv.put ((. String to-bytes) "iserr") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                       (do
                         (if (. rec retryable)
                           (kv.put ((. String to-bytes) "retry") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                           (kv.put ((. String to-bytes) "retry") ((. Bytes of) ("list" ((. UInt8 wrap) 0)))))
                         (list))))
                   ((TimedOut) (list))))
                 ((None) (list))))
             (if (= (. (. e content-type) family) "message")
               (list (record (correlation (Some ((. String to-bytes) "c1"))) (kind "ask") (payload (None)) (target ((. String to-bytes) ""))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "handler" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type Payload (Raw Bytes) (Structured Outcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "effect-request/ask")
             (match (. e payload)
               ((Some frame) (match ((. Bytes slice) frame 40 32)
                 ((Some token) (list (record (correlation (None)) (kind "effect/reply") (payload (Some (Payload.Structured (Outcome.Err (record (message ((. String to-bytes) "nope")) (retryable false)))))) (target token))))
                 ((None) (list))))
               ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply)))
    (serves "ask"))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "caller") (family "ask")))
  (end-state "caller" (kv "iserr" (: 1 Int64)) (kv "retry" (: 0 Int64)) (status quiescent))
  (end-state "handler" (status quiescent)))

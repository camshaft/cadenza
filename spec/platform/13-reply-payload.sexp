; Platform-conformance suite — I2 slice-5: the effect/reply PAYLOAD round-trips to the caller, on the A1 bytes
; fold boundary. A caller performs an effect (kind "ask"); a handler serves it and replies with a DISTINCTIVE
; payload (list(42)); the caller, on the resulting effect-result event, reads e.payload and stores it. Asserts
; the caller's gotreply=42 — proving the handler's reply CONTENT crosses the effect/reply -> effect-result
; boundary intact, not merely that the caller resumed. The round-trip cases (02, 08, 11) only assert resumed=1
; and ignore the reply bytes; this pins that the payload a handler puts on its effect/reply is exactly what the
; caller observes as e.payload on resume (the value channel of the userspace-effect protocol).

(platform-case "a handler's effect/reply payload round-trips to the caller, observed as e.payload on the effect-result"
  (doc "The reply-payload round-trip: kickoff message -> caller performs an ask effect (deferred, forwarded to
        the handler that serves ask); the handler replies with payload list(42); the caller receives an
        effect-result whose e.payload carries that reply, and stores it under gotreply. Asserts gotreply=42
        (the reply CONTENT crossed intact, not just a resume signal) and both sessions quiescent (the effect
        was really settled by the reply). Distinct from 02/08/11 which only assert the caller resumed and never
        check the reply bytes; this pins the value channel of the effect/reply -> effect-result protocol.")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "effect-result")
             (match (. e payload)
               ((Some p) (host (kv) (do (kv.put ((. String to-bytes) "gotreply") p) (list))))
               ((None) (list)))
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
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "effect-request/ask")
             (match (. e payload)
               ((Some frame) (match ((. Bytes slice) frame 40 32)
                 ((Some token) (list (record (correlation (None)) (kind "effect/reply") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 42))))) (target token))))
                 ((None) (list))))
               ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "ask"))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "caller") (family "ask")))
  (end-state "caller" (kv "gotreply" (: 42 Int64)) (status quiescent))
  (end-state "handler" (status quiescent)))

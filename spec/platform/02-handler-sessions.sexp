; Platform-conformance suite — I2: effect-handler SESSIONS + the effect round-trip (seq359), on the
; post-A1 BYTES fold boundary (apply(list<u8>)->list<u8>).
;
; A CALLER reducer performs a userspace effect; a HANDLER session bound via (serves <family>) receives the
; DEFERRED+forwarded effect-request/<family> inbound, folds it, and emits an effect/reply that settles the
; caller's open effect so the caller RESUMES — the real in-process round-trip (the SAME machinery THE
; OUTPOST federates over the wire). A caller emits kind="weather" (unhandled → routed to the handler); a
; handler emits kind="effect/reply" with the reply-token (from the forwarded framing, Bytes.slice offset
; 40 len 32) as target. On resume the fold sees e.content-type.family == "effect-result".
;
; BYTES shape: reducers are (do …) with an UNHANDLED kv effect (host import), a single Event-record param
; (kebab content-type), and an annotated (List (Record …)) effect-list result. Both reducers use kv.PUT
; only (no kv.get), so they emit at the bytes boundary today (§3c GAP B); the userspace-effect deferral +
; effect/reply settle drive through cdz-session-run's CompositeExecutor exactly as before.

(platform-case "a worker performs a userspace effect served by a handler session, and resumes on the reply"
  (doc "The full I2 round-trip: kickoff -> worker performs a weather effect (deferred, forwarded to the
        sky handler which serves weather); sky records served=1 + replies effect/reply; the reply settles
        worker, which resumes (e.content-type.family==effect-result) + records resumed=1. Asserts the
        dispatched effect, both end-states, and worker QUIESCENT (its deferred effect was really settled).")
  (session "worker" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "effect-result")
             (host (kv) (do (kv.put ((. String to-bytes) "resumed") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
             (if (= (. (. e content-type) family) "message")
               (list (record (correlation (Some ((. String to-bytes) "w1"))) (kind "weather") (payload (None)) (target ((. String to-bytes) ""))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "sky" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "effect-request/weather")
             (match (. e payload)
               ((Some frame) (match ((. Bytes slice) frame 40 32)
                 ((Some token) (host (kv) (do (kv.put ((. String to-bytes) "served") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list (record (correlation (None)) (kind "effect/reply") (payload (Some ((. String to-bytes) "sunny"))) (target token))))))
                 ((None) (list))))
               ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "weather"))
  (kickoff "worker" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "worker") (family "weather")))
  (end-state "worker" (kv "resumed" (: 1 Int64)) (status quiescent))
  (end-state "sky" (kv "served" (: 1 Int64)) (status quiescent)))

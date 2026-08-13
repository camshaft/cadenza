; Platform-conformance suite — I2 slice-4: PARTIAL SETTLE — a handler serves two families but replies to only
; one, so one of the caller's two open effects settles while the other stays blocked. On the A1 bytes fold
; boundary. A caller performs alpha AND beta (two deferred effects in one fold); a handler serves both, records
; a=1/b=1, but replies ONLY to alpha (the beta branch records + returns an empty list, no effect/reply). The
; caller's alpha effect is settled (it resumes, writing resumed=1) while its beta effect stays open — so the
; caller ends BOTH resumed=1 (alpha round-tripped) AND `active` (beta obligation still open). Exercises that the
; platform settles multiple open effects INDEPENDENTLY: one reply resolves exactly its own effect and leaves the
; sibling blocked — the dimension neither 08 (both replied -> quiescent) nor 09 (one effect, none replied ->
; active) pins. The handler, having no open obligation of its own, ends quiescent.

(platform-case "a handler serves two families but replies to only one; the caller resumes one effect and stays active on the other"
  (doc "The partial-settle round-trip: kickoff message -> caller performs alpha AND beta (both deferred, both
        forwarded to the handler that serves both); the handler records a=1 + replies to alpha, and records
        b=1 but returns an empty list for beta (no reply). The alpha reply settles the caller's alpha effect
        so it resumes (resumed=1); the beta effect is never settled, so it stays open. Asserts both dispatched
        effects (alpha, beta in order), the handler's a=1 + b=1 + quiescent (no open obligation of its own),
        and the caller ends resumed=1 (alpha round-tripped) AND active (the unsettled beta keeps an obligation
        open). Pins independent per-effect settlement: one reply resolves exactly its effect, not the sibling.")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "effect-result")
             (host (kv) (do (kv.put ((. String to-bytes) "resumed") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
             (if (= (. (. e content-type) family) "message")
               (list
                 (record (correlation (Some ((. String to-bytes) "c1"))) (kind "alpha") (payload (None)) (target ((. String to-bytes) "")))
                 (record (correlation (Some ((. String to-bytes) "c2"))) (kind "beta") (payload (None)) (target ((. String to-bytes) ""))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "handler" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "effect-request/alpha")
             (match (. e payload)
               ((Some frame) (match ((. Bytes slice) frame 40 32)
                 ((Some token) (host (kv) (do (kv.put ((. String to-bytes) "a") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list (record (correlation (None)) (kind "effect/reply") (payload (Some ((. String to-bytes) "ra"))) (target token))))))
                 ((None) (list))))
               ((None) (list)))
             (if (= (. (. e content-type) family) "effect-request/beta")
               (host (kv) (do (kv.put ((. String to-bytes) "b") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "alpha") (serves "beta"))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "caller") (family "alpha"))
    (effect (from "caller") (family "beta")))
  (end-state "handler" (kv "a" (: 1 Int64)) (kv "b" (: 1 Int64)) (status quiescent))
  (end-state "caller" (kv "resumed" (: 1 Int64)) (status active)))

; Platform-conformance suite — I2 slice-7: an effect NO session serves is a PERMANENT error that resumes the
; caller, on the A1 bytes fold boundary. A caller performs an effect (kind "orphan") tagged correlation c1,
; but no session is bound to serve that family. The runtime does not hang or silently drop it: the unhandled
; effect is a permanent error, so the caller is resumed via an effect-result (carrying the original c1
; correlation in e.resumes) and folds its resume arm — ending quiescent with resumed=1 and corr=c1. Exercises
; the "no home" routing path — distinct from 09 (a handler EXISTS but declines to reply, leaving the caller
; active) and 11 (partial settle). Here there is no handler at all, and the contract is: resume, do not stall.

(platform-case "an effect no session serves is a permanent error that resumes the caller (no hang, no silent drop)"
  (doc "The unhandled-effect resume: kickoff message -> caller performs an orphan effect tagged correlation c1;
        no session serves orphan, so the effect is a permanent error and the runtime resumes the caller with an
        effect-result whose e.resumes carries the original c1. The caller folds its effect-result arm, writing
        resumed=1 and corr from e.resumes. Asserts resumed=1 (the caller WAS resumed, not left hanging), corr=c1
        (the original correlation is echoed on the error-resume, same matching channel as a real reply), and the
        session quiescent (the effect was really settled, not left open like 09's declined-reply). Pins that an
        unhandled effect is resolved as a permanent error that resumes, never a stall or a dropped obligation.")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "effect-result")
             (host (kv)
               (do
                 (kv.put ((. String to-bytes) "resumed") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                 (match (. e resumes)
                   ((Some r) (do (kv.put ((. String to-bytes) "corr") r) (list)))
                   ((None) (list)))))
             (if (= (. (. e content-type) family) "message")
               (list (record (correlation (Some ((. String to-bytes) "c1"))) (kind "orphan") (payload (None)) (target ((. String to-bytes) ""))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (end-state "caller" (kv "resumed" (: 1 Int64)) (kv "corr" (: "c1" String)) (status quiescent)))

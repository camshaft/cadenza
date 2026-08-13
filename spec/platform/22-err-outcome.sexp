; Platform-conformance suite — I2 slice-8: the effect-result OUTCOME discriminant — a failed effect surfaces as
; Err on the reducer Event, on the A1 bytes fold boundary. Since the err-reply caller-side seam landed, the
; Event carries a first-class outcome child: option<Ok(ReplyPayload) | Err(record{message: Bytes, retryable:
; Bool}) | TimedOut>, present on an effect-result, None otherwise. An UNHANDLED effect (no session serves it)
; is a permanent error, so its effect-result resume carries outcome = Some(Err{retryable: false}) — the caller
; can DISTINGUISH a failure from a success by matching (. e outcome), which case 16 could not (it only saw the
; resume). This pins the outcome-discriminant contract: a failed effect is observably Err (not Ok), and its
; retryable flag is false for a permanent error. Complements 16 (which asserts the resume + correlation); this
; asserts WHAT the outcome was. (A handler-sent Err reply awaits the ReplyExecutor Err-decode wiring; this uses
; the unhandled-effect permanent error, which surfaces Err today.)

(platform-case "an unhandled effect resumes the caller with an Err outcome (retryable false), observable via the Event outcome child"
  (doc "The err-outcome witness: kickoff message -> caller performs an orphan effect (no session serves it), a
        permanent error; the runtime resumes the caller with an effect-result whose outcome child is
        Some(Err{message, retryable}). The caller matches (. e outcome): on the Err arm it records iserr=1 and
        retry = the retryable bool (0 for this permanent error). Asserts iserr=1 (the outcome was observably
        Err, not Ok) and retry=0 (a permanent error is not retryable) and the session quiescent. Pins the
        effect-result outcome discriminant — a caller can tell a failure from a success — distinct from 16
        which only asserts the resume happened.")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "effect-result")
             (host (kv)
               (match (. e outcome)
                 ((Some o) (match o
                   ((Ok _p) (list))
                   ((Err rec) (do
                      (kv.put ((. String to-bytes) "iserr") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                      (do
                        (if (. rec retryable)
                          (kv.put ((. String to-bytes) "retry") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                          (kv.put ((. String to-bytes) "retry") ((. Bytes of) ("list" ((. UInt8 wrap) 0)))))
                        (list))))
                   ((TimedOut) (list))))
                 ((None) (list))))
             (if (= (. (. e content-type) family) "message")
               (list (record (correlation (Some ((. String to-bytes) "c1"))) (kind "orphan") (payload (None)) (target ((. String to-bytes) ""))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (end-state "caller" (kv "iserr" (: 1 Int64)) (kv "retry" (: 0 Int64)) (status quiescent)))

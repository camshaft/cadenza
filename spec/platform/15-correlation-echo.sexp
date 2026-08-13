; Platform-conformance suite — I2 slice-6: the effect's CORRELATION token is echoed back to the caller on the
; effect-result, on the A1 bytes fold boundary. When a caller performs an effect it tags it with a
; correlation (here "c1"); on the resulting effect-result the runtime carries that same correlation in
; e.resumes — the token the caller uses to match a resume to the specific effect it issued. This case reads
; e.resumes and stores it, asserting corr="c1" via a STRING value-form: it pins the CONTROL/matching channel
; of the userspace-effect protocol (distinct from 13, which pins the reply PAYLOAD in e.payload). It is also
; the suite's first String-valued end-state assertion, exercising the value-form decoder's String arm
; (raw-UTF-8-bytes) added alongside this case.

(platform-case "the effect correlation token is echoed to the caller on the effect-result (observed as e.resumes)"
  (doc "The correlation-echo round-trip: kickoff message -> caller performs an ask effect tagged correlation c1
        (deferred, forwarded to the handler that serves ask); the handler replies; the caller receives an
        effect-result whose e.resumes carries the original c1 correlation and stores it under corr. Asserts
        corr=c1 (String value-form: the correlation the caller issued is echoed back verbatim for matching)
        and both sessions quiescent. Distinct from 13 (reply payload in e.payload); this pins the correlation
        channel — how a caller with multiple in-flight effects tells which one just resumed.")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "effect-result")
             (match (. e resumes)
               ((Some r) (host (kv) (do (kv.put ((. String to-bytes) "corr") r) (list))))
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
                 ((Some token) (list (record (correlation (None)) (kind "effect/reply") (payload (Some ((. String to-bytes) "ok"))) (target token))))
                 ((None) (list))))
               ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "ask"))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "caller") (family "ask")))
  (end-state "caller" (kv "corr" (: "c1" String)) (status quiescent))
  (end-state "handler" (status quiescent)))

; Platform-conformance suite — I2 slice-5: the effect/reply PAYLOAD round-trips to the caller as an Ok outcome,
; on the A1 bytes fold boundary. A caller performs an effect (kind "ask"); a handler serves it and replies a
; success OUTCOME value-form carrying a distinctive payload — Structured(Ok(Inline(list(42)))) — via the guest
; Payload sum the kernel's parse_effect_list re-encodes to the value-form the ReplyExecutor's decode_reply_
; outcome reads (err-reply host half, 906aba179: the reply payload IS a discriminated Ok/Err outcome, no bare-
; payload path). The caller reads the effect-result's outcome child, matches Ok(Inline b), and stores b.
; Asserts gotreply=42 — the handler's reply CONTENT crosses the effect/reply -> effect-result boundary intact
; as the Ok arm's Inline payload, not merely that the caller resumed. Complements 22 (Err outcome via the
; unhandled-effect path); this is the Ok/success outcome via a handler-sent reply.

(platform-case "a handler's effect/reply Ok(Inline) payload round-trips to the caller, observed as the effect-result outcome"
  (doc "The reply-payload round-trip: kickoff message -> caller performs an ask effect (deferred, forwarded to
        the handler that serves ask); the handler replies Structured(Ok(Inline(list(42)))) — a success outcome
        carrying the response bytes; the ReplyExecutor decodes it and settles the caller with EffectOutcome::Ok,
        surfaced on the effect-result's outcome child. The caller matches (. e outcome) -> Ok(Inline b) and
        stores b under gotreply. Asserts gotreply=42 (the reply CONTENT crossed intact as the Ok Inline
        payload) and both sessions quiescent. Distinct from 02/08/11 (assert only the resume) and 22 (Err
        outcome via unhandled-effect); this pins the Ok-outcome value channel of a handler-sent reply.")
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
                   ((Ok rp) (match rp
                     ((Inline b) (do (kv.put ((. String to-bytes) "gotreply") b) (list)))
                     ((Blob _h) (list))))
                   ((Err _r) (list))
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
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (type Payload (Raw Bytes) (Structured Outcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "effect-request/ask")
             (match (. e payload)
               ((Some frame) (match ((. Bytes slice) frame 40 32)
                 ((Some token) (list (record (correlation (None)) (kind "effect/reply") (payload (Some (Payload.Structured (Outcome.Ok (ReplyPayload.Inline ((. Bytes of) ("list" ((. UInt8 wrap) 42)))))))) (target token))))
                 ((None) (list))))
               ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply)))
    (serves "ask"))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "caller") (family "ask")))
  (end-state "caller" (kv "gotreply" (: 42 Int64)) (status quiescent))
  (end-state "handler" (status quiescent)))

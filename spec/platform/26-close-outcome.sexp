; Platform-conformance suite — I4 slice-3: the close OUTCOME (Success vs Failure) is preserved and observable,
; on the A1 bytes fold boundary. A reducer self-closes with a control/close carrying a CloseOutcome — Success
; (clean completion) or Failure (a reason). §6 landed a close_outcome accessor (Session::close_outcome +
; StatusSnapshot.close_outcome), so the runner surfaces the chosen outcome as an end-close-outcome line — the
; two no longer collapse to the same `status closed` (which is all case 24 could assert). This case runs TWO
; sessions from a two-kickoff seed: "winner" self-closes Success, "loser" self-closes Failure, and asserts each
; session's DISTINCT close-outcome. Pins that a reducer's chosen close outcome is preserved end-to-end (not
; flattened) and is observable post-close.

(platform-case "two sessions self-close with distinct outcomes — one Success, one Failure — each observable"
  (doc "The close-outcome distinction: two kickoffs (winner go, loser go) seed both sessions; winner folds its
        go into a control/close Success(Inline(empty)); loser folds its go into a control/close Failure(reason
        bytes). Both end `status closed` — but the runner surfaces each session's CloseOutcome, so the case
        asserts (close-outcome Success) for winner and (close-outcome Failure) for loser: DISTINCT, not both
        collapsing to closed (which is all 24 pins). Pins that a reducer's chosen close outcome is preserved
        and observable post-close.")
  (session "winner" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type CloseOutcome (Success ReplyPayload) (Failure Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (type Payload (Raw Bytes) (Structured CloseOutcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "go")
             (list (record (correlation (None)) (kind "control/close") (payload (Some (Payload.Structured (CloseOutcome.Success (ReplyPayload.Inline ((. Bytes of) ("list"))))))) (target ((. String to-bytes) ""))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply))))
  (session "loser" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type CloseOutcome (Success ReplyPayload) (Failure Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (type Payload (Raw Bytes) (Structured CloseOutcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "go")
             (list (record (correlation (None)) (kind "control/close") (payload (Some (Payload.Structured (CloseOutcome.Failure ((. String to-bytes) "boom"))))) (target ((. String to-bytes) ""))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply))))
  (kickoff "winner" (inbound "go" (: unit Unit)))
  (kickoff "loser" (inbound "go" (: unit Unit)))
  (end-state "winner" (status closed) (close-outcome Success))
  (end-state "loser" (status closed) (close-outcome Failure)))

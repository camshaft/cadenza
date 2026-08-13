; Platform-conformance suite — I4 slice-1: a reducer SELF-CLOSES its session via a control/close signal, on the
; A1 bytes fold boundary. §6 supervision landed the WASM-guest self-close (b3aa9e76c / FoldOutput.close): a
; reducer emits a control/close effect whose payload is a CloseOutcome value-form (Success(ReplyPayload) |
; Failure(Bytes)); the fold adapter intercepts it and appends EventBody::Closed, so the session's terminal
; status is `closed` — distinct from `quiescent` (done for now, could resume) and `active` (obligation open).
; This is the suite's first `closed` end-state and its first lifecycle-transition case: on its kickoff the
; session emits control/close with a Success outcome and the runtime closes it. Pins that a reducer can end its
; own session's lifecycle and that the close is observable as the terminal status.

(platform-case "a reducer self-closes its session with a control/close Success signal, ending status closed"
  (doc "The self-close witness: kickoff message -> the reducer emits a control/close effect carrying a
        CloseOutcome Success(Inline(empty)) payload (via the guest Payload.Structured sum); the fold adapter
        intercepts the well-known control/close family and appends a Closed lifecycle event, so the session's
        terminal status snapshot is `closed`. Asserts the session ends `closed` — the first non-quiescent
        lifecycle terminal in the suite (quiescent = idle-but-live; closed = lifecycle-ended). Pins the §6
        WASM-guest self-close: a reducer can end its own session and the close surfaces as the terminal status.")
  (session "s" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type CloseOutcome (Success ReplyPayload) (Failure Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type Payload (Raw Bytes) (Structured CloseOutcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "message")
             (list (record (correlation (None)) (kind "control/close") (payload (Some (Payload.Structured (CloseOutcome.Success (ReplyPayload.Inline ((. Bytes of) ("list"))))))) (target ((. String to-bytes) ""))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply))))
  (kickoff "s" (inbound "message" (: unit Unit)))
  (end-state "s" (status closed)))

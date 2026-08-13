; Platform-conformance suite — the SettleUnbounded fault: an unbounded effect/reply ping-pong is a GRADED
; fault, never a hang, on the A1 bytes fold boundary. Session A performs a "ping" effect on its kickoff AND on
; every effect-result — so each resume re-performs, forever. Session B serves ping and replies to each request.
; The drive therefore never reaches a fixpoint: A performs -> B replies -> A resumes -> A performs -> ...
; cdz-session-run caps the drive at a step budget (the grader passes a small one for a fault-expecting case)
; and, on exceeding it, prints the observed-run-so-far then bails non-zero with a "SettleUnbounded" marker.
; This case asserts (expect-fault SettleUnbounded): the run MUST fault with that marker. It pins that a
; divergent constellation is caught and reported as a fault rather than hanging the platform — the first
; fault-asserting case in the suite (using corpus-bugfix's (expect-fault ..) reader clause).

(platform-case "an unbounded effect/reply ping-pong is caught as a SettleUnbounded fault, not a hang"
  (doc "The SettleUnbounded witness: kickoff message -> A performs a ping effect; B (serves ping) replies; A
        resumes on the effect-result and performs ping AGAIN; repeat forever. The fixpoint drive never
        settles, so cdz-session-run trips its step budget and bails with a SettleUnbounded fault. Asserts
        (expect-fault SettleUnbounded) — the run must exit non-zero with that stderr marker. Pins that an
        unbounded effect/reply loop is a graded fault, never a silent hang. No end-state is graded (the drive
        never reached a fixpoint); the fault IS the assertion.")
  (session "A" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "message")
             (list (record (correlation (Some ((. String to-bytes) "c1"))) (kind "ping") (payload (None)) (target ((. String to-bytes) ""))))
             (if (= (. (. e content-type) family) "effect-result")
               (list (record (correlation (Some ((. String to-bytes) "c1"))) (kind "ping") (payload (None)) (target ((. String to-bytes) ""))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "B" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "effect-request/ping")
             (match (. e payload)
               ((Some frame) (match ((. Bytes slice) frame 40 32)
                 ((Some token) (list (record (correlation (None)) (kind "effect/reply") (payload (Some ((. String to-bytes) "pong"))) (target token))))
                 ((None) (list))))
               ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "ping"))
  (kickoff "A" (inbound "message" (: unit Unit)))
  (expect-fault SettleUnbounded))

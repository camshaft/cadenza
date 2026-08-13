; Platform-conformance suite — I4 slice-5: the typed child-completed OUTCOME is DIFFERENTIAL — a supervisor routes
; Success and Failure children to DISTINCT state, on the A1 bytes fold boundary. Case 27 pins a single Success child
; decodes; this pins that the two ChildOutcome arms (Success(ReplyPayload) | Failure(Bytes)) are told apart end to end
; and do NOT collapse. One supervisor spawns TWO children whose hashes arrive concatenated in the platform/children
; seed (alias order: bad at [0..32], good at [32..64], sliced with Bytes.slice exactly like the fan-out peer seed of
; case 20): the "good" worker self-closes control/close Success, the "bad" worker self-closes control/close
; Failure(reason). On each reaped child-completed the supervisor reads (. e child-completed) -> Some({child, outcome})
; and branches: Success bumps kv[oks], Failure bumps kv[fails]. Both children run to completion, so the supervisor
; observes oks=1 AND fails=1 — DISTINCT, pinning that a guest supervisor can route per-child on the terminal outcome
; (the foundation for restart-on-failure / count-successes supervision policy), not merely observe that A child
; completed (case 25).

(platform-case "a supervisor routes two children by their typed terminal outcome — one Success, one Failure — to distinct state"
  (doc "The child-outcome differential: config seeds sup.KV[children] = bad-hash ++ good-hash (two --child-reducer
        registrations concatenated, alias order, 32 bytes each); the kickoff slices both and spawns them (two
        lifecycle/spawn effects); each spawn effect-result returns a child SessionId the supervisor messages to
        stimulate; the good worker self-closes Success, the bad worker self-closes Failure(reason). On each reaped
        child-completed the supervisor decodes (. e child-completed) and matches outcome: Success -> bump kv[oks],
        Failure -> bump kv[fails]. Asserts oks=1 AND fails=1 — the two outcomes are told apart and routed to distinct
        state (not collapsed), pinning per-child outcome routing. sup ends quiescent. Both workers are declared via
        (child ..) — spawned by the supervisor, not seeded kickoffs.")
  (session "sup" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (bump (: k Bytes)) (: (host (kv)
        (do (kv.put k (match (kv.get k)
              ((Some b) (match ((. Bytes at) b 0) ((Some v) ((. Bytes of) ("list" ((. UInt8 wrap) (+ v 1))))) ((None) ((. Bytes of) ("list" ((. UInt8 wrap) 1))))))
              ((None) ((. Bytes of) ("list" ((. UInt8 wrap) 1))))))
            (list)))
        (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/children")
             (match (. e payload) ((Some h) (host (kv) (do (kv.put ((. String to-bytes) "children") h) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "lifecycle/child-completed")
               (match (. e child-completed)
                 ((Some cc) (match (. cc outcome)
                   ((Success _rp) (bump ((. String to-bytes) "oks")))
                   ((Failure _r) (bump ((. String to-bytes) "fails")))))
                 ((None) (list)))
               (if (= (. (. e content-type) family) "effect-result")
                 (match (. e outcome)
                   ((Some o) (match o
                     ((Ok rp) (match rp
                       ((Inline childid) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 1))))) (target childid))))
                       ((Blob _b) (list))))
                     ((Err _r) (list))
                     ((TimedOut) (list))))
                   ((None) (list)))
                 (if (= (. (. e content-type) family) "message")
                   (host (kv) (match (kv.get ((. String to-bytes) "children"))
                     ((Some cs)
                       (match ((. Bytes slice) cs 0 32)
                         ((Some c0) (match ((. Bytes slice) cs 32 32)
                           ((Some c1) (list
                             (record (correlation (Some ((. String to-bytes) "c0"))) (kind "lifecycle/spawn") (payload (Some c0)) (target ((. String to-bytes) "")))
                             (record (correlation (Some ((. String to-bytes) "c1"))) (kind "lifecycle/spawn") (payload (Some c1)) (target ((. String to-bytes) "")))))
                           ((None) (list))))
                         ((None) (list))))
                     ((None) (list))))
                   (list)))))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (child "good" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type CloseOutcome (Success ReplyPayload) (Failure Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (type Payload (Raw Bytes) (Structured CloseOutcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (list (record (correlation (None)) (kind "control/close") (payload (Some (Payload.Structured (CloseOutcome.Success (ReplyPayload.Inline ((. Bytes of) ("list"))))))) (target ((. String to-bytes) ""))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply))))
  (child "bad" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type CloseOutcome (Success ReplyPayload) (Failure Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (type Payload (Raw Bytes) (Structured CloseOutcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (list (record (correlation (None)) (kind "control/close") (payload (Some (Payload.Structured (CloseOutcome.Failure ((. String to-bytes) "boom"))))) (target ((. String to-bytes) ""))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply))))
  (kickoff "sup" (inbound "message" (: unit Unit)))
  (end-state "sup" (kv "oks" (: 1 Int64)) (kv "fails" (: 1 Int64)) (status quiescent)))

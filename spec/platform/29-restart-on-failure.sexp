; Platform-conformance suite — I4 slice-6: a supervisor RESTARTS a child on Failure — the per-child terminal outcome
; DRIVES a corrective re-spawn mid-drive, on the A1 bytes fold boundary. Cases 27/28 pin that a supervisor DECODES the
; typed child-completed field and tells Success from Failure; this pins the supervision POLICY that reads on top of it:
; a Failure outcome is not merely counted but ACTED ON — the supervisor emits a fresh lifecycle/spawn (of a stable
; replacement worker) from within its child-completed fold, and the runner's FIFO materializes that restart on the next
; iteration (the loop drains lifecycle_rx at the top of EVERY iteration, not only post-kickoff — so a spawn emitted from
; a child-completed fold is honored). The two children arrive concatenated in the platform/children seed (alias order:
; flaky at [0..32], stable at [32..64], sliced like the fan-out peer seed). Flow: kickoff -> spawn flaky -> message it ->
; flaky self-closes Failure -> host reaps + delivers child-completed(Failure) -> supervisor bumps kv[restarts] AND emits
; lifecycle/spawn(stable) -> host spawns stable -> supervisor messages it -> stable self-closes Success -> child-completed
; (Success) -> supervisor records kv[recovered]=1. Asserts restarts=1 AND recovered=1 (the Failure drove exactly one
; restart, and the replacement succeeded) with the supervisor quiescent. Pins restart-on-failure supervision end to end.

(platform-case "a supervisor restarts a child on Failure — a Failure outcome drives a corrective re-spawn that then succeeds"
  (doc "Restart-on-failure supervision: config seeds sup.KV[children] = flaky-hash ++ stable-hash (two
        --child-reducer registrations concatenated, alias order, 32 bytes each). The kickoff spawns flaky and
        messages it; flaky self-closes control/close Failure(reason). On the reaped child-completed the supervisor
        decodes (. e child-completed) -> Some({child, outcome}), matches Failure -> bumps kv[restarts] AND emits a
        fresh lifecycle/spawn of the stable worker (sliced [32..64] from KV[children]); the runner materializes the
        restart next FIFO iteration; the supervisor messages the stable child (its id off the spawn effect-result);
        stable self-closes Success; on THAT child-completed the supervisor matches Success -> kv[recovered]=1. Asserts
        restarts=1 AND recovered=1 with sup quiescent: the Failure drove exactly one restart and the replacement
        succeeded. Both workers declared via (child ..) — spawned by the supervisor, not seeded kickoffs.")
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
      (def (spawn-slice (: off Int64)) (: (host (kv)
        (match (kv.get ((. String to-bytes) "children"))
          ((Some cs) (match ((. Bytes slice) cs off 32)
            ((Some h) (list (record (correlation (None)) (kind "lifecycle/spawn") (payload (Some h)) (target ((. String to-bytes) "")))))
            ((None) (list))))
          ((None) (list))))
        (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/children")
             (match (. e payload) ((Some h) (host (kv) (do (kv.put ((. String to-bytes) "children") h) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "lifecycle/child-completed")
               (match (. e child-completed)
                 ((Some cc) (match (. cc outcome)
                   ((Success _rp) (bump ((. String to-bytes) "recovered")))
                   ((Failure _r)
                     (host (kv)
                       (do (kv.put ((. String to-bytes) "restarts")
                             (match (kv.get ((. String to-bytes) "restarts"))
                               ((Some b) (match ((. Bytes at) b 0) ((Some v) ((. Bytes of) ("list" ((. UInt8 wrap) (+ v 1))))) ((None) ((. Bytes of) ("list" ((. UInt8 wrap) 1))))))
                               ((None) ((. Bytes of) ("list" ((. UInt8 wrap) 1))))))
                           (match (kv.get ((. String to-bytes) "children"))
                             ((Some cs) (match ((. Bytes slice) cs 32 32)
                               ((Some h) (list (record (correlation (None)) (kind "lifecycle/spawn") (payload (Some h)) (target ((. String to-bytes) "")))))
                               ((None) (list))))
                             ((None) (list))))))))
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
                   (spawn-slice 0)
                   (list)))))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (child "flaky" (reducer
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
  (child "stable" (reducer
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
  (kickoff "sup" (inbound "message" (: unit Unit)))
  (end-state "sup" (kv "restarts" (: 1 Int64)) (kv "recovered" (: 1 Int64)) (status quiescent)))

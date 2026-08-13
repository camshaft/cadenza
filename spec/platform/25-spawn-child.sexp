; Platform-conformance suite — I4 slice-2: a supervisor SPAWNS a child session, the child self-closes, and the
; supervisor observes its completion, on the A1 bytes fold boundary. §6 supervision end-to-end: the runner
; registers a spawnable child (--child-reducer, declared via the (child ..) clause) content-addressed by hash
; and seeds KV["child"] with that hash (symmetric with the peer-seed — a child reducer-hash is a build-time
; identity delivered as config). On its kickoff the supervisor reads KV["child"] and emits a lifecycle/spawn
; effect (child hash on the payload); the host materializes the child via spawn_child_with_nonce and returns
; the child's SessionId on the spawn effect-result; the supervisor messages the child (its id from the Ok
; outcome) to stimulate it; the child self-closes (control/close Success); the host reaps it and delivers a
; lifecycle/child-completed inbound to the supervisor, which folds it ESCALATE-ONLY (a .cdz guest cannot decode
; the opaque child-completed payload — no runtime guest value-decode of a nested payload) and records
; childdone=1. Pins the spawn -> child-runs -> self-close -> reap -> parent-notified supervision round-trip.

(platform-case "a supervisor spawns a child, messages it, the child self-closes, and the supervisor observes child-completed"
  (doc "The §6 spawn round-trip: config seeds sup.KV[child] = the worker reducer content-hash; kickoff message ->
        the supervisor reads KV[child] and emits lifecycle/spawn(hash on payload); the host spawns the worker
        and returns its SessionId on the spawn effect-result; the supervisor reads the id off the Ok outcome and
        emits a message to the worker; the worker self-closes (control/close Success); the host reaps it and
        delivers lifecycle/child-completed to the supervisor, which branches on the family (escalate-only, no
        payload decode) and records childdone=1. Asserts the supervisor's childdone=1 and quiescent (the child
        was spawned, ran, self-closed, and its completion was observed). The worker is declared via (child ..) —
        registered-but-not-kicked; it is spawned by the supervisor, not seeded a kickoff.")
  (session "sup" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/children")
             (match (. e payload) ((Some h) (host (kv) (do (kv.put ((. String to-bytes) "child") h) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "lifecycle/child-completed")
               (host (kv) (do (kv.put ((. String to-bytes) "childdone") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
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
                   (host (kv) (match (kv.get ((. String to-bytes) "child"))
                     ((Some h) (list (record (correlation (Some ((. String to-bytes) "s1"))) (kind "lifecycle/spawn") (payload (Some h)) (target ((. String to-bytes) "")))))
                     ((None) (list))))
                   (list)))))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (child "worker" (reducer
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
  (end-state "sup" (kv "childdone" (: 1 Int64)) (status quiescent)))

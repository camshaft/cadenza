; Platform-conformance suite — I4 slice-8: a supervisor routes per-child by the child's IDENTITY (the child field of
; child-completed), not just its outcome, on the A1 bytes fold boundary. Cases 27/28/29/30 all branch on the child-completed
; OUTCOME (Success vs Failure); none yet exercises the CHILD field — the completed child's 32-byte genesis hash, the
; per-child routing key a real supervisor keys restart/backoff state by. This case pins that the child field carries a
; DISTINCT identity per child: a supervisor spawns TWO children (both self-close Success); on each reaped child-completed it
; writes a KV slot keyed by "seen/" ++ (. cc child) = 1, then prefix-scans "seen/" and records the count. Two DISTINCT child
; ids -> two distinct keys -> count=2. If the two child-completed events carried the same id (or an empty/constant one) the
; keys would collide and count would be 1 — so count=2 pins that the kernel surfaces each child's own genesis hash as the
; child field and a guest can route per-child by it. Deliberately does NOT assert a specific id value (the spawn nonce is
; OS-random -> the genesis hash is non-deterministic; only the DISTINCTNESS + count are observable/deterministic).

(platform-case "a supervisor routes per-child by the child identity field — two distinct child ids yield two distinct keyed slots"
  (doc "Per-child-by-id routing: config seeds sup.KV[children] = c0-hash ++ c1-hash (two --child-reducer registrations
        concatenated, 32 bytes each). The kickoff slices both and spawns them; each spawn effect-result returns a child
        SessionId the supervisor messages; both workers self-close Success. On each reaped child-completed the supervisor
        reads (. e child-completed) -> Some({child, outcome}) and writes KV[\"seen/\" ++ child] = 1 (keying by the child's
        32-byte genesis hash), then prefix-scans \"seen/\" and stores the count under \"distinct\". Asserts distinct=2 with
        sup quiescent: the two child-completed events carried DISTINCT child ids (had they collided, the count would be 1),
        pinning that the child field is a usable per-child routing identity. Does NOT assert a specific id (spawn nonce is
        OS-random); only the distinctness/count is deterministic. Both workers declared via (child ..).")
  (session "sup" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)) (op prefix-scan (-> Bytes (List (Tuple Bytes Bytes)))))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/children")
             (match (. e payload) ((Some h) (host (kv) (do (kv.put ((. String to-bytes) "children") h) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "lifecycle/child-completed")
               (match (. e child-completed)
                 ((Some cc)
                   (host (kv)
                     (do (kv.put ((. Bytes concat) ((. String to-bytes) "seen/") (. cc child)) ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                         (let ((n ((. List len) (kv.prefix-scan ((. String to-bytes) "seen/")))))
                           (kv.put ((. String to-bytes) "distinct") ((. Bytes of) ("list" ((. UInt8 wrap) n)))))
                         (list))))
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
  (child "c0" (reducer
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
  (child "c1" (reducer
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
  (end-state "sup" (kv "distinct" (: 2 Int64)) (status quiescent)))

; Platform-conformance suite — I3 slice-8: FAN-IN — two INDEPENDENT senders converge on one receiver, on the
; A1 bytes fold boundary. The first case with MORE THAN ONE kick-off (operator-approved multi-kickoff, option
; A): both senders are seeded before the FIFO drives to a joint fixpoint, so the run stays single-fixpoint-
; deterministic — it just starts from two seed events instead of one. Each sender, on its own `go` kick-off,
; reads its config-seeded KV[peer]=r and emits a message to r; r folds each message by incrementing a counter
; (read-modify-write via kv.get). After both messages r's count=2. Complements fan-out (20, one sender to many):
; here it is many senders to one, exercising convergence + the multi-kickoff seeding + a read-modify-write
; accumulator that must see its own prior committed write across two message folds.

(platform-case "two independent senders converge on one receiver which accumulates both messages (fan-in)"
  (doc "The fan-in round-trip: config seeds s1.KV[peer]=r and s2.KV[peer]=r; TWO kick-offs (s1 go, s2 go) seed
        both senders before the drive; each sender emits a message (payload 1) to r; r, on each message, reads
        its count (kv.get, defaulting 0 when absent), increments, and stores it back — so after both messages
        r's count=2. Asserts both routed messages (s1->r and s2->r each carrying 1), r's count=2, and all three
        sessions quiescent. Pins multi-sender convergence on one receiver, the multi-kickoff seeding (both
        stimuli enqueued in declared order before the fixpoint), and a receiver accumulator that reads its own
        committed write across the two folds.")
  (session "s1" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "go")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 1))))) (target peer))))
                 ((None) (list))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "s2" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "go")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 1))))) (target peer))))
                 ((None) (list))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "r" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv)
               (let ((prev (match (kv.get ((. String to-bytes) "count"))
                             ((Some c) (match ((. Bytes at) c 0) ((Some b) b) ((None) 0)))
                             ((None) 0))))
                 (do (kv.put ((. String to-bytes) "count") ((. Bytes of) ("list" ((. UInt8 wrap) (+ prev 1))))) (list))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "s1" (inbound "go" (: unit Unit)))
  (kickoff "s2" (inbound "go" (: unit Unit)))
  (expect-messages
    (message (from "s1") (to "r") (family "message") (: 1 Int64))
    (message (from "s2") (to "r") (family "message") (: 1 Int64)))
  (end-state "r" (kv "count" (: 2 Int64)) (status quiescent))
  (end-state "s1" (status quiescent))
  (end-state "s2" (status quiescent)))

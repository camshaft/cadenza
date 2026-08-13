; Platform-conformance suite — I3 slice-3: cross-session PING-PONG (A -> B -> A), on the A1 bytes fold
; boundary. Two sessions message each other in sequence: the ping session (a) emits ping to the pong
; session (b) on its kick-off; b replies pong back to a; a folds the pong and stops (no re-emit) — a
; bounded 2-hop exchange, the messaging analog of a request/response. Exercises BIDIRECTIONAL peer
; addressing (a knows b AND b knows a) and that a routed message can itself trigger a reply message.
;
; MD1: each `(expect-message (from X) (to Y))` edge derives a `--peer X=Y` seed, so the two edges (a->b,
; b->a) seed BOTH sessions' KV[peer] pre-kick-off (folded from the platform/peers config inbound). Each
; reducer reads its own KV[peer] via kv.get to address its counterpart. Bytes-apply shape throughout.

(platform-case "two sessions ping-pong: a emits to b, b replies to a, a folds the reply and stops"
  (doc "The I3 ping-pong round-trip: config seeds a.KV[peer]=b-id-hex and b.KV[peer]=a-id-hex; kickoff
        `start` -> a reads KV[peer] + emits ping(1) to b; the EmitExecutor routes it as a message to b,
        which emits pong(2) back to a; a folds the pong and records pong=2 (no re-emit -> bounded). Asserts
        BOTH routed messages in order (a->b carries 1, b->a carries 2), a's pong=2 end-state, and both
        sessions quiescent (the exchange completed, nothing stuck).")
  (session "a" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "start")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 1))))) (target peer))))
                 ((None) (list))))
               (if (= (. (. e content-type) family) "message")
                 (host (kv) (do (kv.put ((. String to-bytes) "pong") ((. Bytes of) ("list" ((. UInt8 wrap) 2)))) (list)))
                 (list))))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "b" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "message")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 2))))) (target peer))))
                 ((None) (list))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "a" (inbound "start" (: unit Unit)))
  (expect-messages
    (message (from "a") (to "b") (family "message") (: 1 Int64))
    (message (from "b") (to "a") (family "message") (: 2 Int64)))
  (end-state "a" (kv "pong" (: 2 Int64)) (status quiescent))
  (end-state "b" (status quiescent)))

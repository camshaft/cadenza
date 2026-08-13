; Platform-conformance suite — I3 slice-4: MULTIPLE messages from one fold (a sender emits two messages to
; the same peer in a SINGLE apply), on the A1 bytes fold boundary. Exercises that a reducer's effect-list
; result can carry MORE THAN ONE emit (the value-encoded list is walked in order), that the runner routes
; each as its own `message` inbound in emission order, and that the receiver folds both (its end-state
; reflects the SECOND / last message it processed). Single-peer addressing (KV[peer]) — no fan-out; this
; pins the multi-emit-per-fold + ordered-delivery dimension the single-message cases don't.

(platform-case "a sender emits two messages to one peer in a single fold; the peer folds both in order"
  (doc "The I3 multi-emit round-trip: config seeds sender.KV[peer]=receiver-id-hex; kickoff message ->
        sender reads KV[peer] + emits TWO messages (payload 1 then 2) to the receiver in one apply; the
        EmitExecutor routes them as two message inbounds in order; the receiver folds each, storing the
        payload under `last`, so after both its last=2 (the second message). Asserts BOTH routed messages
        in order (sender->receiver carrying 1 then 2), receiver last=2, and both sessions quiescent.")
  (session "sender" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "message")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peer) (list
                   (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 1))))) (target peer))
                   (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 2))))) (target peer))))
                 ((None) (list))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "receiver" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "message")
             (match (. e payload) ((Some p) (host (kv) (do (kv.put ((. String to-bytes) "last") p) (list)))) ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "sender" (inbound "message" (: unit Unit)))
  (expect-messages
    (message (from "sender") (to "receiver") (family "message") (: 1 Int64))
    (message (from "sender") (to "receiver") (family "message") (: 2 Int64)))
  (end-state "sender" (status quiescent))
  (end-state "receiver" (kv "last" (: 2 Int64)) (status quiescent)))

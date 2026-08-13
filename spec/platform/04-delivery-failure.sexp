; Platform-conformance suite — I3 slice-2: cross-session DELIVERY-FAILURE bounce (seq358 / lifecycle-I5),
; on the post-A1 BYTES fold boundary. A sender emits a message to a peer that is NOT live; the send cannot
; be delivered, so the FIFO drive bounces a `delivery-failure`-family inbound back to the sender (a
; Failure-to-sender). The sender READS KV[peer] via kv.get (a valid-hex-but-absent id) — which the bytes
; boundary does not yet emit (§3c GAP C) — so the sender DECLINES and this case grades Todo until GAP C.

(platform-case "a message to an absent peer bounces a delivery-failure back to the sender, which folds it"
  (doc "The I3 delivery-failure round-trip: config seeds sender.KV[peer]=ghost-id-hex (ghost resolved but
        unspawned); kickoff message -> sender reads KV[peer] (kv.get) + emits kind=emit target=ghost
        payload=7; deliver-to-ghost returns None -> the drive bounces a delivery-failure inbound back to
        sender, which folds failed=1. TODO until §3c GAP C (kv.get option-result emit) lands.")
  (session "sender" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload)
               ((Some peerhex) (host (kv) (do (kv.put ((. String to-bytes) "peer") peerhex) (list))))
               ((None) (list)))
             (if (= (. (. e content-type) family) "delivery-failure")
               (host (kv) (do (kv.put ((. String to-bytes) "failed") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
               (if (= (. (. e content-type) family) "message")
                 (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                   ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 7))))) (target peer))))
                   ((None) (list))))
                 (list))))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "sender" (inbound "message" (: unit Unit)))
  (expect-messages
    (message (from "sender") (to "ghost") (family "message") (: 7 Int64)))
  (expect-delivery-failure (from "sender") (to "ghost"))
  (end-state "sender" (kv "failed" (: 1 Int64)) (status quiescent)))

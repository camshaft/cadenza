; Platform-conformance suite — I3 slice-6: a message carrying a MULTI-BYTE (String) payload round-trips
; through the inter-session message channel, on the A1 bytes fold boundary. Every prior I3 case (03-07) carries
; a single-byte payload (1, 2, 5, 6, 7); this pins that the message value channel routes an arbitrary-length
; byte payload intact, not just one byte. A sender reads its config-seeded KV[peer] and emits a message whose
; payload is the String "hi" (two bytes); the receiver stores e.payload. Asserts the routed message carries
; "hi" (via the String value-form on expect-messages) and the receiver's msg="hi" — exercising multi-byte
; payload delivery + the String value-form on both the message-sequence and end-state assertions.

(platform-case "a message carrying a multi-byte String payload round-trips to the receiver intact"
  (doc "The multi-byte-payload round-trip: config seeds sender.KV[peer]=receiver-id-hex; kickoff message ->
        sender reads KV[peer] and emits a message whose payload is the String hi (two bytes) to the receiver;
        the receiver folds it, storing e.payload under msg. Asserts the routed message sender->receiver carries
        the String hi, the receiver's msg=hi, and both sessions quiescent. Pins that the message value channel
        delivers an arbitrary-length byte payload intact (prior I3 cases only carry a single byte).")
  (session "sender" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "message")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. String to-bytes) "hi"))) (target peer))))
                 ((None) (list))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "receiver" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (match (. e payload) ((Some p) (host (kv) (do (kv.put ((. String to-bytes) "msg") p) (list)))) ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "sender" (inbound "message" (: unit Unit)))
  (expect-messages
    (message (from "sender") (to "receiver") (family "message") (: "hi" String)))
  (end-state "sender" (status quiescent))
  (end-state "receiver" (kv "msg" (: "hi" String)) (status quiescent)))

; Platform-conformance suite — I3 slice-5: a 3-session MESSAGE CHAIN (a -> b -> c), on the A1 bytes fold
; boundary. A message propagates across three sessions in sequence: a emits to b on its kick-off; b, on
; receiving, emits to c; c folds the arrival. Exercises MULTI-HOP routing (a routed message triggers a
; further routed message at the next hop, and again) — the transitive-propagation dimension the single-hop
; and 2-hop (ping-pong) cases don't pin — and that each hop's peer is addressed from that session's own
; config-seeded KV[peer]. Single-peer-per-hop addressing (no fan-out); each edge derives one --peer seed.

(platform-case "a message chains across three sessions a -> b -> c, each hop emitting to the next"
  (doc "The I3 chain round-trip: config seeds a.KV[peer]=b, b.KV[peer]=c; kickoff `start` -> a emits msg(5)
        to b; b folds it and emits msg(6) to c; c folds it and records arrived=1. Asserts BOTH hops in
        order (a->b carries 5, b->c carries 6), c's arrived=1 end-state, and all three sessions quiescent
        (the message propagated the full chain and settled).")
  (session "a" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "start")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 5))))) (target peer))))
                 ((None) (list))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "b" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "message")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 6))))) (target peer))))
                 ((None) (list))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "c" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv) (do (kv.put ((. String to-bytes) "arrived") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "a" (inbound "start" (: unit Unit)))
  (expect-messages
    (message (from "a") (to "b") (family "message") (: 5 Int64))
    (message (from "b") (to "c") (family "message") (: 6 Int64)))
  (end-state "c" (kv "arrived" (: 1 Int64)) (status quiescent))
  (end-state "a" (status quiescent))
  (end-state "b" (status quiescent)))

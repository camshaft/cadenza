; Platform-conformance suite — a single fold emits BOTH an inter-session message AND a deferred userspace
; effect, on the A1 bytes fold boundary. Every prior case's fold produces only one kind of outbound (all emits,
; or all effect-requests, or all replies); this pins that one fold's effect-list can MIX kinds — an emit routed
; to a peer AND a userspace effect deferred to a handler — and the runner dispatches each to its correct
; destination in one drive. The origin session, on its kickoff, emits msg(7) to its config-seeded peer AND
; performs an ask effect; the handler (serves ask) replies, resuming the origin; the receiver folds the message.
; This is also the first case to assert BOTH expect-effects and expect-messages together.

(platform-case "one fold emits a message to a peer and performs a userspace effect; both are routed in one drive"
  (doc "The mixed-outbound fold: config seeds origin.KV[peer]=receiver-id-hex; kickoff message -> origin's one
        fold returns an effect-list carrying TWO records: an emit of payload 7 to the receiver AND an ask
        userspace effect (deferred to the handler that serves ask). The runner routes the emit as a message
        inbound to the receiver and forwards the ask to the handler; the handler replies, so origin resumes
        (resumed=1); the receiver folds the message (got=7). Asserts the dispatched ask effect, the routed
        message origin->receiver carrying 7, origin resumed=1, receiver got=7, and all three quiescent. Pins
        that a single fold can mix an emit and a userspace effect and both reach their correct destination.")
  (session "origin" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "effect-result")
               (host (kv) (do (kv.put ((. String to-bytes) "resumed") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
               (if (= (. (. e content-type) family) "message")
                 (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                   ((Some peer) (list
                     (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 7))))) (target peer))
                     (record (correlation (Some ((. String to-bytes) "c1"))) (kind "ask") (payload (None)) (target ((. String to-bytes) "")))))
                   ((None) (list))))
                 (list))))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "handler" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "effect-request/ask")
             (match (. e payload)
               ((Some frame) (match ((. Bytes slice) frame 40 32)
                 ((Some token) (list (record (correlation (None)) (kind "effect/reply") (payload (Some ((. String to-bytes) "ok"))) (target token))))
                 ((None) (list))))
               ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "ask"))
  (session "receiver" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (match (. e payload) ((Some p) (host (kv) (do (kv.put ((. String to-bytes) "got") p) (list)))) ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "origin" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "origin") (family "ask")))
  (expect-messages
    (message (from "origin") (to "receiver") (family "message") (: 7 Int64)))
  (end-state "origin" (kv "resumed" (: 1 Int64)) (status quiescent))
  (end-state "receiver" (kv "got" (: 7 Int64)) (status quiescent))
  (end-state "handler" (status quiescent)))

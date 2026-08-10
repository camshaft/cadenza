; Platform-conformance suite — I3 slice-2: cross-session DELIVERY-FAILURE bounce (seq358 / §lifecycle-I5).
; A sender emits a message to a peer that is NOT live; the send cannot be delivered, so the host bounces a
; `delivery-failure`-family inbound back to the SENDER (a Failure-to-sender, never a silent drop) — the
; SAME machinery THE OUTPOST federates over the wire (an emit to a terminated/absent federated peer bounces
; identically). The runner replicates the async loop's §lifecycle-I5 bounce path in its deterministic FIFO
; drive: on a message whose target is absent (deliver returns None) and whose reply_to is the sender, it
; routes a `delivery-failure` inbound (echoed payload, reply_to cleared) back to the sender.
;
; The peer "ghost" is a VALID-HEX id the runner resolves (Hash::of(salt ++ "ghost")) but never spawns as a
; --session, so the emit targets a canonical-hex id that resolves to no live session → the deliver-None
; bounce path (a NON-hex target would instead be a permanent EmitExecutor error that routes NOTHING, not a
; bounce). MD1: the runner seeds the sender KV[peer]=ghost-id-hex pre-kick-off so the reducer emits by id.

(platform-case "a message to an absent peer bounces a delivery-failure back to the sender, which folds it"
  (doc "The I3 delivery-failure round-trip: config seeds sender.KV[peer]=ghost-id-hex (ghost is resolved but
        unspawned); kickoff message -> sender emits kind=emit target=ghost payload=7; deliver-to-ghost
        returns None -> the drive bounces a delivery-failure inbound (echoed payload) back to sender, which
        folds failed=1. Asserts the message was attempted (sender->ghost), the bounce (sender->ghost), the
        sender folded failed=1, and the sender is quiescent (it handled the failure, not stuck).")
  (session "sender" (reducer (do (effect Kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit))) (bind Kv "cadenza:agent-kernel/kv") (def (apply (: ct (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes))) (: (if (= (. ct family) "platform/peers") (match payload ((Some peer) (host (Kv) (do ((. Kv put) ((. String to-bytes) "peer") peer) ("list")))) ((None) ("list"))) (if (= (. ct family) "delivery-failure") (host (Kv) (do ((. Kv put) ((. String to-bytes) "failed") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) ("list"))) (if (= (. ct family) "message") (host (Kv) (match ((. Kv get) ((. String to-bytes) "peer")) ((Some peer) ("list" ("record" (= correlation (None)) (= kind "emit") (= payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 7))))) (= target peer)))) ((None) ("list")))) ("list")))) (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes))))) (export apply))))
  (kickoff "sender" (inbound "message" (: unit Unit)))
  (expect-messages
    (message (from "sender") (to "ghost") (family "message") (: 7 Int64)))
  (expect-delivery-failure (from "sender") (to "ghost"))
  (end-state "sender" (kv "failed" (: 1 Int64)) (status quiescent)))

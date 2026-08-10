; Platform-conformance suite — I3: multi-session MESSAGING (seq358). A sender session performs an `emit`
; effect targeting a PEER session; the EmitExecutor routes it as a `message`-family inbound into the peer's
; log, which the peer folds — one agent signalling another, the SAME machinery THE OUTPOST federates over
; the wire (in-process here = the deterministic conformance oracle for that behavior).
;
; MD1 peer-addressing: a case names peers by ALIAS, never by a content-derived genesis hash. The runner
; resolves each `(expect-message (from A) (to B))` edge to a `--peer A=B` binding and, BEFORE the single
; kick-off, delivers A a `platform/peers` CONFIGURATION inbound carrying B's SessionId-hex (folded into
; KV["peer"]). So A reads KV["peer"] and emits to B by the resolved id — the config sets the constellation
; up; the one kick-off starts the interaction (D5 single-kickoff-drives-to-fixpoint honored).
;
; The emitted message payload is a one-byte integer (Bytes.of [UInt8.wrap 7]) so the value decoder models
; it; the receiver records got=1 on delivery. Asserts the routed message (from/to + value), the sender +
; receiver end-states, and both QUIESCENT (the message was really delivered + folded).

(platform-case "a sender emits a message to a peer session, which the peer receives and folds"
  (doc "The I3 happy-path directed message: config seeds sender.KV[peer]=receiver-id-hex; kickoff message ->
        sender reads KV[peer] + emits kind=emit target=peer payload=7; the EmitExecutor routes it as a
        message inbound to receiver, which folds got=1. Asserts the routed sender->receiver message carries
        7, receiver got=1, and both sessions quiescent (the message was delivered, not stuck).")
  (session "sender" (reducer (do (effect Kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit))) (bind Kv "cadenza:agent-kernel/kv") (def (apply (: ct (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes))) (: (if (= (. ct family) "platform/peers") (match payload ((Some peer) (host (Kv) (do ((. Kv put) ((. String to-bytes) "peer") peer) ("list")))) ((None) ("list"))) (if (= (. ct family) "message") (host (Kv) (match ((. Kv get) ((. String to-bytes) "peer")) ((Some peer) ("list" ("record" (= correlation (None)) (= kind "emit") (= payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 7))))) (= target peer)))) ((None) ("list")))) ("list"))) (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes))))) (export apply))))
  (session "receiver" (reducer (do (effect Kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit))) (bind Kv "cadenza:agent-kernel/kv") (def (apply (: ct (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes))) (: (if (= (. ct family) "message") (host (Kv) (do ((. Kv put) ((. String to-bytes) "got") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) ("list"))) ("list")) (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes))))) (export apply))))
  (kickoff "sender" (inbound "message" (: unit Unit)))
  (expect-messages
    (message (from "sender") (to "receiver") (family "message") (: 7 Int64)))
  (end-state "sender" (status quiescent))
  (end-state "receiver" (kv "got" (: 1 Int64)) (status quiescent)))

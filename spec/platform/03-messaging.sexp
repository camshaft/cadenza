; Platform-conformance suite — I3 slice-1: cross-session MESSAGING (seq358), on the post-A1 BYTES fold
; boundary. A sender performs an `emit` effect targeting a PEER (its id-hex seeded into KV[peer] by the
; runner's pre-kickoff platform/peers config); the EmitExecutor routes it as a `message`-family inbound to
; the peer, which folds it. The sender READS KV[peer] via kv.get, which the bytes boundary does not yet
; emit (§3c GAP C, option<list<u8>> host result pending) — so the sender DECLINES and this case grades
; Todo until GAP C lands. The receiver is pure kv.put. MD1: --peer <holder>=<peer> derived from the
; expect-message edge seeds the holder's KV[peer] = peer SessionId-hex pre-kickoff.

(platform-case "a sender emits a message to a peer session, which the peer receives and folds"
  (doc "The I3 happy-path directed message: config seeds sender.KV[peer]=receiver-id-hex; kickoff message ->
        sender reads KV[peer] (kv.get) + emits kind=emit target=peer payload=7; the EmitExecutor routes it
        as a message inbound to receiver, which folds got=1. TODO until §3c GAP C (kv.get option-result
        emit) lands — the sender's kv.get declines at the bytes boundary today.")
  (session "sender" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv) (match (kv.get ((. String to-bytes) "peer"))
               ((Some peer) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 7))))) (target peer))))
               ((None) (list))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "receiver" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv) (do (kv.put ((. String to-bytes) "got") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "sender" (inbound "message" (: unit Unit)))
  (expect-messages
    (message (from "sender") (to "receiver") (family "message") (: 7 Int64)))
  (end-state "sender" (status quiescent))
  (end-state "receiver" (kv "got" (: 1 Int64)) (status quiescent)))

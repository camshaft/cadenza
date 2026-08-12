; Platform-conformance suite — I3 slice-7: FAN-OUT — one sender emits to TWO distinct peers in a single fold,
; on the A1 bytes fold boundary. Complements 06 (one sender, two messages to the SAME peer): here the two
; messages go to DIFFERENT receivers. The runner seeds a holder's multiple `--peer` edges as a concatenation of
; their 64-char SessionId hexes into one `platform/peers` payload (edge order), so the origin's fold slices
; peer 0 at [0..64] and peer 1 at [64..128] with Bytes.slice and emits a distinct payload to each. Asserts both
; routed messages (origin->r1 carrying 1, origin->r2 carrying 2, in order) and each receiver's own end-state.
; Pins that a single fold can address several distinct peers and the runner routes each to its own session.

(platform-case "one sender fans a distinct message out to two different peers in a single fold"
  (doc "The fan-out round-trip: config seeds origin.KV[peer] = r1-hex ++ r2-hex (two --peer edges concatenated,
        64 chars each); kickoff message -> origin's one fold slices peer 0 ([0..64]) and peer 1 ([64..128]) and
        emits payload 1 to r1 and payload 2 to r2. The runner routes each emit as a message inbound to its own
        receiver; r1 folds got=1, r2 folds got=2. Asserts both routed messages in order (origin->r1 carrying 1,
        origin->r2 carrying 2), r1 got=1, r2 got=2, and all three quiescent. Pins that a single fold addresses
        multiple distinct peers, each routed to its own session (distinct from 06's two messages to one peer).")
  (session "origin" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "platform/peers")
             (match (. e payload) ((Some ph) (host (kv) (do (kv.put ((. String to-bytes) "peer") ph) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "message")
               (host (kv) (match (kv.get ((. String to-bytes) "peer"))
                 ((Some peers)
                   (match ((. Bytes slice) peers 0 64)
                     ((Some p0) (match ((. Bytes slice) peers 64 64)
                       ((Some p1) (list
                         (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 1))))) (target p0))
                         (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 2))))) (target p1))))
                       ((None) (list))))
                     ((None) (list))))
                 ((None) (list))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "r1" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (match (. e payload) ((Some p) (host (kv) (do (kv.put ((. String to-bytes) "got") p) (list)))) ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "r2" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (match (. e payload) ((Some p) (host (kv) (do (kv.put ((. String to-bytes) "got") p) (list)))) ((None) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "origin" (inbound "message" (: unit Unit)))
  (expect-messages
    (message (from "origin") (to "r1") (family "message") (: 1 Int64))
    (message (from "origin") (to "r2") (family "message") (: 2 Int64)))
  (end-state "origin" (status quiescent))
  (end-state "r1" (kv "got" (: 1 Int64)) (status quiescent))
  (end-state "r2" (kv "got" (: 2 Int64)) (status quiescent)))

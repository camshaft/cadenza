; Platform-conformance suite — I2: effect-handler SESSIONS + the effect round-trip (seq359).
;
; A CALLER reducer performs a userspace effect; a HANDLER session bound via (serves <family>) receives
; the DEFERRED+forwarded effect-request/<family> inbound, folds it, and emits an effect/reply that
; settles the caller's open effect so the caller RESUMES — the real in-process round-trip (the SAME
; machinery THE OUTPOST federates over the wire). Runs FOR REAL post-binary-AST-B2 (c58a7a65e, seq374):
; the fold boundary is apply(list<u8>)->list<u8>, so a Cadenza reducer emits a FLAT effect-request record
; whose `kind` is an arbitrary STRING (register-by-string). A caller emits kind="weather" (unhandled →
; routed to the handler); a handler emits kind="effect/reply" with the reply-token (from the forwarded
; framing, Bytes.slice offset 40 len 32) as target (Bytes). On resume the fold sees ct.family=="effect-result".

(platform-case "a worker performs a userspace effect served by a handler session, and resumes on the reply"
  (doc "The full I2 round-trip: kickoff -> worker performs a weather effect (deferred, forwarded to the
        sky handler which serves weather); sky records seen=1 + replies effect/reply; the reply settles
        worker, which resumes (ct.family==effect-result) + records resumed=1. Asserts the dispatched
        effect, both end-states, and worker QUIESCENT (its deferred effect was really settled).")
  (session "worker" (reducer (do (effect Kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit))) (bind Kv "cadenza:agent-kernel/kv") (def (apply (: ct (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes))) (: (if (= (. ct family) "effect-result") (host (Kv) (do ((. Kv put) ((. String to-bytes) "resumed") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) ("list"))) (if (= (. ct family) "message") ("list" ("record" (= correlation (Some ((. String to-bytes) "w1"))) (= kind "weather") (= payload (None)) (= target ((. String to-bytes) "")))) ("list"))) (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes))))) (export apply))))
  (session "sky" (reducer (do (effect Kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit))) (bind Kv "cadenza:agent-kernel/kv") (def (apply (: ct (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes))) (: (if (= (. ct family) "effect-request/weather") (match payload ((Some frame) (match ((. Bytes slice) frame 40 32) ((Some token) (host (Kv) (do ((. Kv put) ((. String to-bytes) "served") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) ("list" ("record" (= correlation (None)) (= kind "effect/reply") (= payload (Some ((. String to-bytes) "sunny"))) (= target token)))))) ((None) ("list")))) ((None) ("list"))) ("list")) (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes))))) (export apply))) (serves "weather"))
  (kickoff "worker" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "worker") (family "weather")))
  (end-state "worker" (kv "resumed" (: 1 Int64)) (status quiescent))
  (end-state "sky" (kv "served" (: 1 Int64)) (status quiescent)))

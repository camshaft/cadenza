; Platform-conformance suite — I2 slice-2: a handler session that SERVES TWO effect families, on the A1
; bytes fold boundary. A caller performs TWO distinct userspace effects (kind "alpha" and "beta") in one
; fold; a single handler session bound via (serves "alpha") (serves "beta") receives each deferred+forwarded
; effect-request, dispatches ON the family (per-family branch), records + replies to each. Exercises the
; multi-`serves` dimension the single-family I2 case (02) doesn't: one session as the handler for several
; families, routed per-family, with two open effects on the caller settled independently.

(platform-case "one handler session serves two effect families; a caller performs both and resumes"
  (doc "The multi-family I2 round-trip: kickoff message -> caller performs an alpha effect AND a beta effect
        (both deferred, forwarded to the handler which serves both); the handler dispatches on
        e.content-type.family — effect-request/alpha records a=1 + replies, effect-request/beta records b=1
        + replies; each reply settles the caller's matching open effect, and the caller records resumed=1 on
        an effect-result. Asserts both dispatched effects (alpha, beta in order), the handler's a=1 + b=1,
        the caller's resumed=1, and both sessions quiescent (both deferred effects were really settled).")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "effect-result")
             (host (kv) (do (kv.put ((. String to-bytes) "resumed") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
             (if (= (. (. e content-type) family) "message")
               (list
                 (record (correlation (Some ((. String to-bytes) "c1"))) (kind "alpha") (payload (None)) (target ((. String to-bytes) "")))
                 (record (correlation (Some ((. String to-bytes) "c2"))) (kind "beta") (payload (None)) (target ((. String to-bytes) ""))))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "handler" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "effect-request/alpha")
             (match (. e payload)
               ((Some frame) (match ((. Bytes slice) frame 40 32)
                 ((Some token) (host (kv) (do (kv.put ((. String to-bytes) "a") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list (record (correlation (None)) (kind "effect/reply") (payload (Some ((. String to-bytes) "ra"))) (target token))))))
                 ((None) (list))))
               ((None) (list)))
             (if (= (. (. e content-type) family) "effect-request/beta")
               (match (. e payload)
                 ((Some frame) (match ((. Bytes slice) frame 40 32)
                   ((Some token) (host (kv) (do (kv.put ((. String to-bytes) "b") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list (record (correlation (None)) (kind "effect/reply") (payload (Some ((. String to-bytes) "rb"))) (target token))))))
                   ((None) (list))))
                 ((None) (list)))
               (list)))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "alpha") (serves "beta"))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "caller") (family "alpha"))
    (effect (from "caller") (family "beta")))
  (end-state "caller" (kv "resumed" (: 1 Int64)) (status quiescent))
  (end-state "handler" (kv "a" (: 1 Int64)) (kv "b" (: 1 Int64)) (status quiescent)))

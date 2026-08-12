; Platform-conformance suite — I2 slice-3: an UNANSWERED deferred effect leaves the caller BLOCKED, on the
; A1 bytes fold boundary. A caller performs an effect (kind "slow"); a handler session serves that family and
; receives the forwarded effect-request, records it, but DECLINES to reply (returns an empty effect-list). The
; caller's dispatched effect is never settled, so it ends `active` (an open, unsettled obligation) — while the
; handler, which has no open obligation of its own, ends `quiescent`. Exercises the LIVENESS dimension the
; round-trip cases (02, 08) don't: the platform neither silently DROPS an unanswered effect nor FALSELY
; settles it — the caller stays genuinely blocked. This is the suite's first non-quiescent end-state, pinning
; that a deferred effect with no reply keeps its caller's obligation open (status active, no clock).

(platform-case "an unanswered deferred effect leaves the caller active (blocked), the handler quiescent"
  (doc "The unanswered-effect witness: kickoff message -> caller performs a `slow` effect (deferred, forwarded
        to the handler that serves `slow`); the handler records saw=1 but returns an empty effect-list (no
        effect/reply), so the caller's dispatched effect is never settled. Asserts the one dispatched effect
        (caller, slow), the handler's saw=1 + quiescent (it did its work and has no open obligation), and the
        caller ends ACTIVE — the open unsettled effect keeps its obligation live (no clock, so an outstanding
        effect is Active, not Stalled). Pins that an unanswered effect is neither dropped nor falsely settled.")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (list
               (record (correlation (Some ((. String to-bytes) "c1"))) (kind "slow") (payload (None)) (target ((. String to-bytes) ""))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "handler" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "effect-request/slow")
             (host (kv) (do (kv.put ((. String to-bytes) "saw") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "slow"))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "caller") (family "slow")))
  (end-state "handler" (kv "saw" (: 1 Int64)) (status quiescent))
  (end-state "caller" (status active)))

; Platform-conformance suite — I1: a single Cadenza reducer session, one kick-off event, no effects.
;
; The runtime/platform analog of the compiler corpus (DESIGN-platform-conformance-suite.md, seq359):
; a (platform-case ..) declares reducer SESSIONS + exactly ONE (kickoff ..); the gate compiles each
; reducer to the cadenza:agent-kernel/fold world, drives it through the REAL kernel via cdz-session-run,
; and grades the observed end-state. I1 is the single-session, no-effect, drive-to-quiescence proof.

(platform-case "a counter session bumps kv count on its kick-off message"
  (doc "One session, no effects. The kick-off is an inbound `message`; the reducer reads kv[count]
        (absent -> 0), writes back byte (count+1) via its bound cadenza:agent-kernel/kv, and returns
        no effects. Folding one message on an empty session leaves kv[count] = 1, the session
        quiescent, and 2 events on the log (genesis seq0 + the one inbound).")
  (session "worker" (reducer
    (do
      (type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))
      (type EffectRequest
        (Mk (Record
          (: kind EffectKind)
          (: target String)
          (: payload (Option Bytes))
          (: correlation (Option Bytes)))))
      (effect Kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (bind Kv "cadenza:agent-kernel/kv")
      (def (bump-count (: prev (Option Bytes)))
        (: (let ((prev-byte (match prev
                              ((Some b) (match ((. Bytes at) b 0) ((Some v) v) ((None) 0)))
                              ((None) 0))))
             ((. Bytes of) ("list" ((. UInt8 wrap) (+ prev-byte 1)))))
           Bytes))
      (def (apply
             (: ct (Record (: family String) (: version (UInt 32))))
             (: payload (Option Bytes))
             (: resumes (Option Bytes)))
        (: (match resumes
             ((Some _) ("list"))
             ((None)
              (if (= (. ct family) "message")
                (host (Kv)
                  (do
                    ((. Kv put) ((. String to-bytes) "count")
                                (bump-count ((. Kv get) ((. String to-bytes) "count"))))
                    ("list")))
                ("list"))))
           (List EffectRequest)))
      (export apply))))
  (kickoff "worker" (inbound "message" (: unit Unit)))
  (end-state "worker" (kv "count" (: 1 Int64)) (status quiescent))
  (events-processed "worker" 2))

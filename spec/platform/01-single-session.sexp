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

(platform-case "the same counter IGNORES a non-message kick-off (else-branch writes no state)"
  (doc "The negative companion of the counter case: the SAME reducer, but the kick-off family is
        `tick`, which its `apply` does not match — so the else-branch returns no effects and writes
        NO kv. This pins that a no-op fold leaves the session quiescent with an EMPTY kv (no spurious
        `count` key) and still 2 events on the log (genesis + the one inbound). It witnesses the
        else-branch of the fold, and that the grader's kv assertions are a POSITIVE check (a case that
        asserts no kv key does not require one to exist).")
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
  (kickoff "worker" (inbound "tick" (: unit Unit)))
  (end-state "worker" (status quiescent))
  (events-processed "worker" 2))

(platform-case "a session writes TWO kv keys on its kick-off (multi-key end-state)"
  (doc "Exercises the grader's MULTI-KEY end-state path: on a `message` kick-off the reducer writes two
        distinct kv keys (a=7, b=9) via its bound kv, no effects. Pins that BOTH keys are asserted
        independently (a case with several (kv …) clauses requires every one to match), not just the
        first. Distinct one-byte values (07/09) also witness the value decoder at more than one number.")
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
                    ((. Kv put) ((. String to-bytes) "a") ((. Bytes of) ("list" ((. UInt8 wrap) 7))))
                    ((. Kv put) ((. String to-bytes) "b") ((. Bytes of) ("list" ((. UInt8 wrap) 9))))
                    ("list")))
                ("list"))))
           (List EffectRequest)))
      (export apply))))
  (kickoff "worker" (inbound "message" (: unit Unit)))
  (end-state "worker" (kv "a" (: 7 Int64)) (kv "b" (: 9 Int64)) (status quiescent))
  (events-processed "worker" 2))

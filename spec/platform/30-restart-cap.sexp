; Platform-conformance suite — I4 slice-7: a supervisor CAPS its restarts and GIVES UP — bounded restart-on-failure,
; on the A1 bytes fold boundary. Case 29 pins that a Failure drives a corrective re-spawn that then succeeds; this pins
; the OTHER supervision-policy half: when the replacement ALSO fails, the supervisor does NOT restart unboundedly (which
; would diverge / trip SettleUnbounded) but bounds its attempts and records giving up. The supervisor restarts the flaky
; worker exactly ONCE: it distinguishes the first Failure from the second by the presence of KV["restarts"] (absent on the
; first child-completed Failure -> set restarts=1 and re-spawn; present on the second -> set gaveup=1 and emit nothing).
; So the flaky worker is spawned twice, fails twice, and the drive SETTLES (no infinite spawn->fail->respawn loop) with
; the supervisor quiescent, restarts=1, gaveup=1. Pins that restart-on-failure is BOUNDED — the anti-divergence property a
; real supervisor needs — using only the kv.get Some/None option arm (no numeric comparison), symmetric with case 29.

(platform-case "a supervisor caps restarts and gives up — a repeatedly-failing child is restarted once then abandoned, the drive settles"
  (doc "Bounded restart-on-failure: config seeds sup.KV[children] = flaky-hash (one --child-reducer). Kickoff spawns
        flaky and messages it; flaky self-closes control/close Failure(reason). On the FIRST child-completed Failure
        the supervisor finds KV[restarts] ABSENT -> sets restarts=1 AND re-spawns flaky (a bounded retry). The
        re-spawned flaky fails again; on the SECOND child-completed Failure KV[restarts] is PRESENT -> the supervisor
        sets gaveup=1 and emits NOTHING (no further spawn). The drive settles: flaky ran twice, failed twice, and the
        supervisor did NOT loop forever. Asserts restarts=1 AND gaveup=1 with sup quiescent — restart-on-failure is
        BOUNDED (the anti-SettleUnbounded property). Uses only the kv.get Some/None arm to count the one allowed
        restart, no numeric comparison. flaky is declared via (child ..) — spawned by the supervisor, not kicked.")
  (session "sup" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (spawn-flaky) (: (host (kv)
        (match (kv.get ((. String to-bytes) "children"))
          ((Some cs) (match ((. Bytes slice) cs 0 32)
            ((Some h) (list (record (correlation (None)) (kind "lifecycle/spawn") (payload (Some h)) (target ((. String to-bytes) "")))))
            ((None) (list))))
          ((None) (list))))
        (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/children")
             (match (. e payload) ((Some h) (host (kv) (do (kv.put ((. String to-bytes) "children") h) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "lifecycle/child-completed")
               (match (. e child-completed)
                 ((Some cc) (match (. cc outcome)
                   ((Success _rp) (list))
                   ((Failure _r)
                     (host (kv)
                       (match (kv.get ((. String to-bytes) "restarts"))
                         ((Some _b)
                           (do (kv.put ((. String to-bytes) "gaveup") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
                         ((None)
                           (do (kv.put ((. String to-bytes) "restarts") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                               (match (kv.get ((. String to-bytes) "children"))
                                 ((Some cs) (match ((. Bytes slice) cs 0 32)
                                   ((Some h) (list (record (correlation (None)) (kind "lifecycle/spawn") (payload (Some h)) (target ((. String to-bytes) "")))))
                                   ((None) (list))))
                                 ((None) (list))))))))))
                 ((None) (list)))
               (if (= (. (. e content-type) family) "effect-result")
                 (match (. e outcome)
                   ((Some o) (match o
                     ((Ok rp) (match rp
                       ((Inline childid) (list (record (correlation (None)) (kind "emit") (payload (Some ((. Bytes of) ("list" ((. UInt8 wrap) 1))))) (target childid))))
                       ((Blob _b) (list))))
                     ((Err _r) (list))
                     ((TimedOut) (list))))
                   ((None) (list)))
                 (if (= (. (. e content-type) family) "message")
                   (spawn-flaky)
                   (list)))))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (child "flaky" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type CloseOutcome (Success ReplyPayload) (Failure Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (type Payload (Raw Bytes) (Structured CloseOutcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (list (record (correlation (None)) (kind "control/close") (payload (Some (Payload.Structured (CloseOutcome.Failure ((. String to-bytes) "boom"))))) (target ((. String to-bytes) ""))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply))))
  (kickoff "sup" (inbound "message" (: unit Unit)))
  (end-state "sup" (kv "restarts" (: 1 Int64)) (kv "gaveup" (: 1 Int64)) (status quiescent)))

; Platform-conformance suite — the I4 REPLAY-EQUALS-RECOVER equivalence: a session's whole event log is persisted
; through a durable file sink as it drives, then after the fixpoint the session is REMOVED from the host (the crash
; boundary) and RECOVERED from disk into a fresh host; the recovered end-state (KV + status + open-effect obligations)
; must EQUAL the live one. This is the platform/runtime analog of the kernel's loop_and_recovery.rs reference
; (recover reconstructs KV by re-folding the log through the reducer + preserves the dispatched-but-unsettled effect
; obligations the driver must re-drive). It rides the full I4 host surface v-agent-harness-host landed:
; AgentHost::recover_hosted_session (00b8e3f05) + HostedSession::with_persisted_sink (88ff19d6a) + recover_and_build
; dep-resolution (6b952aab4 — the recovered reducer's cadenza:runtime/heap dep resolves from the recovery blob + the
; ComponentStore's transitive nfc). The runner (--recover-check caller) populates a MemBlobStore with the reducer + its
; heap-dep bytes, persists genesis then drives, removes the live session (releasing the file sink), and recovers.
;
; The checked reducer (caller) does BOTH halves so the equivalence is NON-VACUOUS: on its message kickoff it (1) writes
; a KNOWN kv value (rk=7) AND (2) performs a DEFERRED effect (kind "slow") forwarded to a handler that DECLINES to reply
; (empty effect-list), so the caller's dispatched effect is never settled — it ends `active` with ONE open obligation.
; A recover that dropped the KV, lost the open effect, or mis-reconstructed the status would DIVERGE from the captured
; live state and the runner emits recover-mismatch (fail); recover-equal ok pins that all three (kv + active-status +
; open-effect count) survived the persist->crash->recover round-trip intact. The handler is NOT recover-checked (it is
; the unanswered-effect foil, mirroring case 09); only the caller — the session with both durable KV and a live
; obligation — is the equivalence subject.

(platform-case "a session's persisted log recovers to an end-state equal to the live one — KV + open-effect obligation intact (I4 replay-equals-recover)"
  (doc "The I4 replay-equals-recover equivalence: the caller on its message kickoff writes kv rk=7 AND performs a
        deferred `slow` effect (forwarded to a handler that records saw=1 but never replies), so the caller ends
        `active` with one open (unsettled) effect obligation and a durable KV write. The runner persists the caller's
        whole log through a file sink (genesis first, then every fold), drives to the fixpoint, captures the live
        end-state, REMOVES the caller (crash boundary — the file sink flushes+closes), then recovers it from disk into
        a fresh host via recover_hosted_session (reloading the reducer + resolving its heap dep from the recovery blob
        + ComponentStore) and asserts the recovered kv + status + open-effect count EQUAL the captured live ones.
        Non-vacuous: the caller has BOTH durable state (rk=7) and a live obligation (the open `slow` effect), so a
        recover that dropped either diverges. The handler is the unanswered-effect foil (case-09 shape), not checked.")
  (session "caller" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv) (do (kv.put ((. String to-bytes) "rk") ((. Bytes of) ("list" ((. UInt8 wrap) 7))))
               (list
                 (record (correlation (Some ((. String to-bytes) "c1"))) (kind "slow") (payload (None)) (target ((. String to-bytes) ""))))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (session "handler" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "effect-request/slow")
             (host (kv) (do (kv.put ((. String to-bytes) "saw") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply)))
    (serves "slow"))
  (kickoff "caller" (inbound "message" (: unit Unit)))
  (recover-check "caller")
  (expect-effects
    (effect (from "caller") (family "slow")))
  (end-state "handler" (kv "saw" (: 1 Int64)) (status quiescent))
  (end-state "caller" (kv "rk" (: 7 Int64)) (status active)))

; Platform-conformance suite — I4 slice-4: a supervisor DECODES the typed child-completed field (child + terminal
; outcome), not just the escalate-only family branch case 25 pins. §6 V2 landed the first-class typed Event field
; child-completed: Option(Record(child: Bytes, outcome: ChildOutcome)) surfaced by build_event_document (slice-B
; 7bfddf541 + v-ah-host delivery cd3bbfd04): both the reap-of-a-self-closed-child and terminate-I7 paths now emit
; EventBody::ChildCompleted, and the kernel surfaces the child's 32-byte genesis hash + its CloseOutcome value-form
; (ChildOutcome = Success(ReplyPayload) | Failure(Bytes)) as a TYPED field a .cdz supervisor value-decodes. This case
; upgrades the case-25 round-trip: instead of blindly recording childdone=1 on the family, the supervisor reads
; (. e child-completed) -> Some({child, outcome}), branches on outcome (Success here), and records child-ok=1 — pinning
; that the guest can now DECODE the terminal outcome per child (the foundation for per-child restart/route). It also
; witnesses the child id is present (non-empty) by recording its first byte length-proxy is unnecessary — the outcome
; branch alone proves decode; the child field is exercised structurally by the match binding it.

(platform-case "a supervisor DECODES the typed child-completed field and branches on the terminal outcome (Success)"
  (doc "The §6 V2 typed-field decode: same spawn round-trip as case 25 (config seeds sup.KV[child] = worker hash;
        kickoff -> read KV[child] -> emit lifecycle/spawn; host spawns worker, returns its id on the spawn
        effect-result; sup messages the worker; worker self-closes control/close Success; host reaps + delivers
        child-completed). But unlike 25's escalate-only family branch, the supervisor now reads (. e child-completed)
        -> Some({child, outcome}) and MATCHES on outcome: Success -> kv child-ok=1, Failure -> kv child-failed=1. The
        worker closes Success, so the supervisor observes child-ok=1 (NOT child-failed) — pinning that the guest
        value-decodes the terminal outcome per child, the capability slice-B + the delivery landing added over the
        opaque payload case 25 could only escalate on. Asserts sup kv child-ok=1 and quiescent.")
  (session "sup" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "platform/children")
             (match (. e payload) ((Some h) (host (kv) (do (kv.put ((. String to-bytes) "child") h) (list)))) ((None) (list)))
             (if (= (. (. e content-type) family) "lifecycle/child-completed")
               (match (. e child-completed)
                 ((Some cc) (match (. cc outcome)
                   ((Success _rp) (host (kv) (do (kv.put ((. String to-bytes) "child-ok") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list))))
                   ((Failure _r) (host (kv) (do (kv.put ((. String to-bytes) "child-failed") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))) (list))))))
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
                   (host (kv) (match (kv.get ((. String to-bytes) "child"))
                     ((Some h) (list (record (correlation (Some ((. String to-bytes) "s1"))) (kind "lifecycle/spawn") (payload (Some h)) (target ((. String to-bytes) "")))))
                     ((None) (list))))
                   (list)))))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (child "worker" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type CloseOutcome (Success ReplyPayload) (Failure Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (type Payload (Raw Bytes) (Structured CloseOutcome))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (list (record (correlation (None)) (kind "control/close") (payload (Some (Payload.Structured (CloseOutcome.Success (ReplyPayload.Inline ((. Bytes of) ("list"))))))) (target ((. String to-bytes) ""))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Payload)) (: target Bytes)))))
      (export apply))))
  (kickoff "sup" (inbound "message" (: unit Unit)))
  (end-state "sup" (kv "child-ok" (: 1 Int64)) (status quiescent)))

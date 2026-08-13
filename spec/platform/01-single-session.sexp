; Platform-conformance suite — I1: a single Cadenza reducer session, one kick-off event, no effects.
;
; The runtime/platform analog of the compiler corpus (DESIGN-platform-conformance-suite.md, seq359):
; a (platform-case ..) declares reducer SESSIONS + exactly ONE (kickoff ..); the gate compiles each
; reducer to the cadenza:agent-kernel/fold BYTES boundary (apply(list<u8>)->list<u8>, binary-abi A1) via
; the target WIT world (emit-wit-world), drives it through the REAL kernel via cdz-session-run, and grades
; the observed end-state. I1 is the single-session, no-effect, drive-to-quiescence proof.
;
; BYTES-BOUNDARY REDUCER SHAPE (post-A1 flip): apply takes ONE param — the Event value-record the kernel's
; build_event_document sends (record { content-type: { family, version }, payload: Option Bytes, resumes:
; Option Bytes }, kebab field names) — which the compiler value-DECODEs from the incoming list<u8>. The
; result is an effect-list, a value-form (List (Record (: correlation ..) (: kind ..) (: payload ..)
; (: target ..))) the compiler value-ENCODEs to list<u8>. The bound kv effect is UNHANDLED (no (bind ..)),
; so it crosses as the cadenza:agent-kernel/kv HOST IMPORT the kernel serves against the session KV.
;
; kv.GET is not yet emittable at the bytes boundary (§3c GAP C, option<list<u8>> host result pending), so a
; reducer that reads its KV DECLINES → the grader records Todo (coverage-not-yet). The two counter cases
; below use kv.get and stand as Todo witnesses until GAP C lands; the two pure-kv.put cases PASS today.

(platform-case "a counter session bumps kv count on its kick-off message"
  (doc "One session, no effects. The kick-off is an inbound `message`; the reducer reads kv[count]
        (absent -> 0), writes back byte (count+1) via its bound cadenza:agent-kernel/kv, and returns
        no effects. Folding one message on an empty session leaves kv[count] = 1, the session
        quiescent, and 2 events on the log (genesis seq0 + the one inbound). kv.GET keeps this a Todo
        witness until §3c GAP C lands the option-result host emit.")
  (session "worker" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv)
               (do (kv.put ((. String to-bytes) "count")
                     (match (kv.get ((. String to-bytes) "count"))
                       ((Some b) (match ((. Bytes at) b 0) ((Some v) ((. Bytes of) ("list" ((. UInt8 wrap) (+ v 1))))) ((None) ((. Bytes of) ("list" ((. UInt8 wrap) 1))))))
                       ((None) ((. Bytes of) ("list" ((. UInt8 wrap) 1))))))
                   (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "worker" (inbound "message" (: unit Unit)))
  (end-state "worker" (kv "count" (: 1 Int64)) (status quiescent))
  (events-processed "worker" 2))

(platform-case "a session writes TWO kv keys on its kick-off (multi-key end-state)"
  (doc "Exercises the grader's MULTI-KEY end-state path: on a `message` kick-off the reducer writes two
        distinct kv keys (a=7, b=9) via its bound kv, no effects. Pins that BOTH keys are asserted
        independently (a case with several (kv …) clauses requires every one to match), not just the
        first. Distinct one-byte values (07/09) also witness the value decoder at more than one number.
        Pure kv.PUT (no get) so it PASSES at the bytes boundary today.")
  (session "worker" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv)
               (do (kv.put ((. String to-bytes) "a") ((. Bytes of) ("list" ((. UInt8 wrap) 7))))
                   (kv.put ((. String to-bytes) "b") ((. Bytes of) ("list" ((. UInt8 wrap) 9))))
                   (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "worker" (inbound "message" (: unit Unit)))
  (end-state "worker" (kv "a" (: 7 Int64)) (kv "b" (: 9 Int64)) (status quiescent))
  (events-processed "worker" 2))

(platform-case "a session stores a fixed mid-range byte value (decoder past the counter's 1)"
  (doc "A reducer that writes a FIXED byte 42 under `answer` on its message kick-off (not a bump, not
        derived) — witnessing the value decoder at a mid-range number (0x2a) and a fresh key name, so the
        grader's (: n Int64) → one-byte hex path is pinned beyond the count=1 the other cases use. Guards
        that an arbitrary stored byte round-trips through the end-kv comparison, not just the value 1.
        Pure kv.PUT so it PASSES at the bytes boundary today.")
  (session "worker" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv)
               (do (kv.put ((. String to-bytes) "answer") ((. Bytes of) ("list" ((. UInt8 wrap) 42))))
                   (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "worker" (inbound "message" (: unit Unit)))
  (end-state "worker" (kv "answer" (: 42 Int64)) (status quiescent))
  (events-processed "worker" 2))

(platform-case "the same counter IGNORES a non-message kick-off (else-branch writes no state)"
  (doc "The negative companion of the counter case: the SAME reducer, but the kick-off family is
        `tick`, which its `apply` does not match — so the else-branch returns no effects and writes
        NO kv. This pins that a no-op fold leaves the session quiescent with an EMPTY kv (no spurious
        `count` key) and still 2 events on the log (genesis + the one inbound). kv.GET keeps this a Todo
        witness until §3c GAP C lands.")
  (session "worker" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv)
               (do (kv.put ((. String to-bytes) "count")
                     (match (kv.get ((. String to-bytes) "count"))
                       ((Some b) b)
                       ((None) ((. Bytes of) ("list" ((. UInt8 wrap) 1))))))
                   (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "worker" (inbound "tick" (: unit Unit)))
  (end-state "worker" (status quiescent))
  (events-processed "worker" 2))

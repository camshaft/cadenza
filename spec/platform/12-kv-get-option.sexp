; Platform-conformance suite — single-session slice: kv.get's OPTION result, BOTH arms, on the A1 bytes fold
; boundary. A reducer reads two keys via kv.get in one fold: an absent key ("missing") returns None, and a
; key it just put ("present"=9) returns Some. It witnesses each arm positively: the None arm writes sawnone=1,
; and the Some arm binds the read-back value v and stores it under "gotback" — so gotback=9 proves the Some
; payload was lifted FAITHFULLY (the actual bytes, not just a truthy tag) and that a fold reads its own
; uncommitted writes (read-your-writes). Exercises the option<list<u8>> lift's None branch and Some-payload
; round-trip end-to-end; the existing cases only ever hit kv.get on a pre-seeded (Some) key and never pin the
; None arm or a value read-back as their asserted behavior.

(platform-case "a reducer reads an absent key (kv.get None) and a just-put key (kv.get Some) and witnesses both option arms"
  (doc "The kv.get option round-trip: kickoff message -> the reducer kv.get(missing) which returns None so it
        writes sawnone=1; then puts present=9 and kv.get(present) which returns Some(9) so it binds the read-back
        value and writes gotback=<that value>. Asserts sawnone=1 (None arm ran), present=9 (the seed), gotback=9
        (Some arm ran AND the lifted payload equals what was put — faithful value round-trip + read-your-writes),
        and the session quiescent. Pins both arms of the option<list<u8>> lift crossing the fold boundary.")
  (session "s" (reducer
    (do
      (effect kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv)
               (do
                 (match (kv.get ((. String to-bytes) "missing"))
                   ((Some _v) (unit))
                   ((None) (kv.put ((. String to-bytes) "sawnone") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))))
                 (do
                   (kv.put ((. String to-bytes) "present") ((. Bytes of) ("list" ((. UInt8 wrap) 9))))
                   (do
                     (match (kv.get ((. String to-bytes) "present"))
                       ((Some v) (kv.put ((. String to-bytes) "gotback") v))
                       ((None) (unit)))
                     (list)))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "s" (inbound "message" (: unit Unit)))
  (end-state "s" (kv "sawnone" (: 1 Int64)) (kv "present" (: 9 Int64)) (kv "gotback" (: 9 Int64)) (status quiescent)))

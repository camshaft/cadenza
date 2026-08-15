; Platform-conformance suite — the R2 Value.decode round-trip: a reducer encodes a structured value to Bytes and
; DECODES it back in-fold, recovering the original. Case 33 pins the ENCODE half (Value.encode a record -> Bytes ->
; reified-effect payload); this pins the DECODE half — the inverse — so the full R2 in-fold codec round-trips at the
; A1 bytes boundary. Value.encode : ∀a. a -> Bytes (total) and Value.decode : ∀a. Bytes -> Option a (partial) are the
; single public canonical binary-AST codec (R2, operator-ruled); decode grounds its target from the enclosing
; annotation (v-inference f6f3be0d4) and reconstructs a genuine heap-handle value (Record/List/String/Map — NOT a bare
; scalar, whose unbox is a later increment). The reducer encodes (record (= x 7) (= y 9)) of a structural 2-field
; record, decodes the bytes back to (Option (Record ..)), and on the Some arm reads the x field (=7) into KV. Pins that
; a value survives a full encode->decode round-trip through the in-fold codec with its structure + field values intact.
;
; The decode TARGET is a STRUCTURAL inline record type in the annotation (: (Value.decode ..) (Option (Record ..))) —
; a genuine I32 heap handle (the descriptor-bearing rep decode reconstructs); a bare scalar target declines (R2's
; scalar-unbox is deferred), and a NOMINAL type ctor (type Pt ..) + bare Pt would trip CDZ0203 (needs a type arg), so
; the structural form is the one that grounds. Asserts kv rx=7 (the decoded x) — non-vacuous: a decode that returned
; None (or dropped the field) would write 0, and the round-trip must recover the exact encoded value.

(platform-case "a reducer Value.encodes a structured record then Value.decodes it back in-fold, recovering the field (R2 round-trip)"
  (doc "The R2 decode round-trip: on its message kickoff the reducer builds a structural record (record (= x 7)
        (= y 9)), Value.encodes it to Bytes, then Value.decodes those Bytes back with the enclosing annotation
        (: (Value.decode ..) (Option (Record (: x UInt8) (: y UInt8)))) grounding the target; on the Some arm it
        reads the recovered x field (7) and stores it under kv rx. Asserts rx=7 quiescent: the value survived the
        full in-fold encode->decode round-trip with its structure + field intact. Uses a STRUCTURAL record target
        (a genuine heap handle; a scalar target's unbox is a later R2 increment, and a nominal type ctor trips
        CDZ0203) and the (= field val) record-literal form. Non-vacuous: a None (decode failed) or dropped field
        writes 0, not 7. The DECODE complement of case 33's ENCODE-into-a-reified-effect pin.")
  (session "s" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv) (do (kv.put ((. String to-bytes) "rx")
               (match (: ((. Value decode) ((. Value encode) (: (record (= x 7) (= y 9)) (Record (: x UInt8) (: y UInt8))))) (Option (Record (: x UInt8) (: y UInt8))))
                 ((Some p) ((. Bytes of) ("list" (. p x))))
                 ((None) ((. Bytes of) ("list" ((. UInt8 wrap) 0)))))) (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "s" (inbound "message" (: unit Unit)))
  (end-state "s" (kv "rx" (: 7 Int64)) (status quiescent)))

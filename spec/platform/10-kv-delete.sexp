; Platform-conformance suite — single-session slice: the kv.delete host op + its BOOL result, on the A1 bytes
; fold boundary. A reducer declares `(op delete (-> Bytes Bool))` alongside put, and in one fold: puts key
; "k", deletes it (delete returns TRUE since it existed) and on the true branch writes removed=1; then deletes
; a never-seeded key "absent" (delete returns FALSE) and on the false branch writes missing=1. Exercises the
; kv.delete bool-scalar lift end-to-end through the platform boundary in BOTH outcomes — the first `(op delete)`
; use in the suite (kv-widening step 1, cdz-kernel 7c7d1632d added delete to reducer_world_artifact). The two
; witness keys are the positive proof (the grader asserts KV presence/value, not absence): removed=1 proves the
; true arm ran, missing=1 the false arm; the deleted "k" is simply absent from the end-state (unassertable).

(platform-case "a reducer deletes KV slots and branches on the delete bool: true for a present key, false for an absent one"
  (doc "The kv.delete round-trip: kickoff message -> the reducer puts k=1 then kv.delete(k) which returns TRUE
        (k existed) so it writes removed=1; then kv.delete(absent) which returns FALSE (never seeded) so it
        writes missing=1. Asserts removed=1 (the delete-true arm ran, so delete's bool lifted true), missing=1
        (the delete-false arm ran, so delete's bool lifted false), and the session quiescent. Pins both
        outcomes of the kv.delete bool-scalar result crossing the fold boundary; k itself is deleted, hence
        absent from the end-state (the grader has no absence assertion — the two witness keys carry the proof).")
  (session "s" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)) (op delete (-> Bytes Bool)))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv)
               (do
                 (kv.put ((. String to-bytes) "k") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                 (do
                   (if (kv.delete ((. String to-bytes) "k"))
                     (kv.put ((. String to-bytes) "removed") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                     (unit))
                   (do
                     (if (kv.delete ((. String to-bytes) "absent"))
                       (unit)
                       (kv.put ((. String to-bytes) "missing") ((. Bytes of) ("list" ((. UInt8 wrap) 1)))))
                     (list)))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "s" (inbound "message" (: unit Unit)))
  (end-state "s" (kv "removed" (: 1 Int64)) (kv "missing" (: 1 Int64)) (status quiescent)))

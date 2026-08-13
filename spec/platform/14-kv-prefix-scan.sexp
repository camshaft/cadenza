; Platform-conformance suite — single-session slice: the kv.prefix-scan host op + its nested
; list<tuple<list<u8>,list<u8>>> result, on the A1 bytes fold boundary. This completes the kv interface
; (get/put/delete/prefix-scan). A reducer puts three keys under an "item/" prefix and one key outside it
; ("other"), then kv.prefix-scan("item/") and stores the pair COUNT. Asserts count=3 — proving the nested
; list-of-byte-pairs lift returns the right number of elements AND that the prefix filter excludes the
; out-of-namespace key. NON-VACUOUS by construction: the count must be a specific nonzero (3), so a
; silent-empty lift (the exact bug my repro A/B/C surfaced, fixed in af74f912a "convert the built arr to a
; VEC") would fail this rather than pass. The op name is the WIT func name verbatim, kebab `prefix-scan`.

(platform-case "a reducer prefix-scans a namespaced key range and counts exactly the in-prefix pairs"
  (doc "The kv.prefix-scan round-trip: kickoff message -> the reducer puts item/a=1 item/b=2 item/c=3 and
        other=9, then kv.prefix-scan(item/) and stores List.len of the result under count. Asserts count=3
        (the scan returns exactly the three item/ pairs and excludes other, exercising the nested
        list<tuple<list<u8>,list<u8>>> lift end-to-end and the prefix filter), the four seeded keys present,
        and the session quiescent. NON-VACUOUS: count is a specific nonzero, so the silent-empty lift bug
        this suite surfaced (fixed af74f912a) would fail here.")
  (session "s" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)) (op prefix-scan (-> Bytes (List (Tuple Bytes Bytes)))))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)))))
        (: (if (= (. (. e content-type) family) "message")
             (host (kv)
               (do
                 (kv.put ((. String to-bytes) "item/a") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
                 (kv.put ((. String to-bytes) "item/b") ((. Bytes of) ("list" ((. UInt8 wrap) 2))))
                 (kv.put ((. String to-bytes) "item/c") ((. Bytes of) ("list" ((. UInt8 wrap) 3))))
                 (kv.put ((. String to-bytes) "other") ((. Bytes of) ("list" ((. UInt8 wrap) 9))))
                 (let ((n ((. List len) (kv.prefix-scan ((. String to-bytes) "item/")))))
                   (kv.put ((. String to-bytes) "count") ((. Bytes of) ("list" ((. UInt8 wrap) n)))))
                 (list)))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes)) (: target Bytes)))))
      (export apply))))
  (kickoff "s" (inbound "message" (: unit Unit)))
  (end-state "s"
    (kv "count" (: 3 Int64))
    (kv "item/a" (: 1 Int64)) (kv "item/b" (: 2 Int64)) (kv "item/c" (: 3 Int64)) (kv "other" (: 9 Int64))
    (status quiescent)))

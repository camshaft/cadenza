; BREAKER FINDING (differential, WRONG VALUE on wasm) — a RUNTIME-start Bytes.slice used as a CHAMP
; hash key (Map key or Set member, EITHER side of the lookup) hashes DIFFERENTLY from a flat Bytes of
; equal content on the WASM backend, so the lookup MISSES — while value `=` on the same pair says EQUAL
; (the equal-means-same-key contract is violated; sharing/representation is observable). rust and
; rust-async compute every face correctly. A CONST-start slice works on wasm too (it compacts/folds),
; so the divergence is specifically the RUNTIME-start slice VIEW representation reaching champ_hash
; without content-canonicalization — champ_hash likely hashes the view node (offset/parent) or the
; parent's leaf bytes rather than the flattened slice content, while value-eq compares content.
;
; Grades: wasm FAIL (wrong value) / rust PASS / rust-async PASS on every case below.
; Severity: silent wrong value on wasm (not a decline) — a content-addressed table keyed by parsed
; frame windows would silently drop entries.

(case "value-eq CONTROL: a runtime slice compares equal to a flat Bytes of the same content"
  (input (do
           (def (main (: a Int64))
             (match (Bytes.slice (Bytes.of (list 9 20 30 8)) a 2)
               ((Some s) (if (= s (Bytes.of (list 20 30))) 1 0))
               ((None u) -1)))
           (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case "a runtime slice PROBING a Map keyed by flat Bytes must hit by content"
  (input (do
           (def (main (: a Int64))
             (let ((m (Map.insert Map.empty (Bytes.of (list 20 30)) 42)))
               (match (Bytes.slice (Bytes.of (list 9 20 30 8)) a 2)
                 ((Some s) (match (Map.lookup m s) ((Some v) v) ((None u) -1)))
                 ((None u) -2))))
           (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64)))

(case "a runtime slice STORED as a Map key must be found by a flat Bytes probe"
  (input (do
           (def (main (: a Int64))
             (match (Bytes.slice (Bytes.of (list 9 20 30 8)) a 2)
               ((Some s)
                 (match (Map.lookup (Map.insert Map.empty s 42) (Bytes.of (list 20 30)))
                   ((Some v) v) ((None u) -1)))
               ((None u) -2)))
           (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64)))

(case "a runtime slice probes a Set of flat Bytes by content"
  (input (do
           (def (main (: a Int64))
             (match (Bytes.slice (Bytes.of (list 9 20 30 8)) a 2)
               ((Some s) (if (Set.contains (Set.of (list (Bytes.of (list 20 30)))) s) 1 0))
               ((None u) -2)))
           (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

; CONST-start control — WORKS on wasm today (the slice compacts at fold time):
(case "CONTROL: a CONST-start slice as a Map-lookup key hits on wasm"
  (input (do
           (def (main (: a Int64))
             (let ((m (Map.insert Map.empty (Bytes.of (list 20 30)) 42)))
               (match (Bytes.slice (Bytes.of (list 9 20 30 8)) 1 2)
                 ((Some s) (match (Map.lookup m s) ((Some v) v) ((None u) -1)))
                 ((None u) -2))))
           (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

; ADDENDUM (tick 228 scoping sweep): the divergence is EXCLUSIVE to the runtime-start Bytes.slice VIEW.
; Working controls on wasm, all verified this sweep:
;   - String.slice (runtime start) as a Map key: HITS both directions (probe and stored-key).
;   - Bytes.concat rope (runtime leaf) as a Map probe: HITS.
; So champ_hash flattens ropes and handles String slices; ONLY the Bytes slice-view node misses the
; content-canonicalization. (Runtime rope STRING key via concat inside the lookup declines — unrelated.)

; ADDENDUM 2 (tick 230): a TUPLE-WRAPPED slice as a Map key HITS on wasm — (Map.lookup m (tuple 1 s))
; with s the runtime slice finds the flat-keyed entry. So the COMPOUND-key champ descent canonicalizes
; the Bytes leaf correctly; ONLY the BARE-slice-key path misses. The fix is even narrower than addendum 1
; suggested: the top-level Bytes arm of champ key canonicalization (the compound descent's leaf handling
; already flattens — likely a reusable path).

; BREAKER FINDING 2026-07-20 (trunk 4a08a8a48, re-confirmed 7f6f074e3) — WASM-ONLY GAP (differential):
; Set.to-list over FLOAT64 elements (and Map.to-list over Float64 KEYS) DECLINES on wasm
; ("Set.to-list element shape has no orderable descriptor") while rust/rust-async COMPUTE.
; RULING RECEIVED (concierge answer, 2026-07-20): OPTION A — Float-element to-list should COMPUTE
; by CANONICAL BYTE order on BOTH backends. Fix = the wasm to-list sort compares canonical BYTES
; instead of taking the unordered value-cmp sentinel (spec collections-and-text#190 "Set Iteration
; Is Deterministic": order derived from elements, agreeing with the canonical byte form — which DOES
; totally order floats: NaN collapsed, +/-0.0 distinct; the same order `=` and hashing use).
;
; The FLOAT sibling of the fixed compound-element to-list gap (19-sets:786 regression witness):
; the orderable-descriptor gate excludes Float, but the ruling blesses canonical-byte order.
;
; OBSERVED canonical-byte order on rust (pin these when fixing — they are the CORRECT target order,
; NOT numeric/IEEE order):
;   {-1.0, 0.5, 2.5}  -> [0.5, 2.5, -1.0]   (NEGATIVES sort AFTER positives — sign bit is the high bit)
;   {-0.0, 0.0}       -> [0.0, -0.0]        (cardinality 2 — signed zeros are DISTINCT elements)
;   {NaN, 2.5, 1.5}   -> NaN LAST; (= f f) is TRUE for the enumerated NaN (canonical nan==nan)
;
; Float SETS otherwise work on wasm: contains/insert/len all pinned green (03-equality:463,
; 19-sets:953 box-float). ONLY the to-list enumeration declines.
;
; SEVERITY: backend divergence on a blessed-by-ruling surface; blocks the set→list→fold idiom over
; float data on wasm.
;
; Expected (per ruling A): all cases below compute on BOTH backends with the canonical-byte order.
(case "Set.to-list over Float64 elements enumerates (len is the cardinality)"
  (doc    "`(List.len (Set.to-list (Set.of (list x 2.5 1.5))))` with a runtime x:Float64 — the
           enumeration must compute (ruling: canonical-byte order per collections-and-text#190), so the
           length is 3. Currently wasm declines 'no orderable descriptor'; rust computes.")
  (input  (do
            (def (main (: x Float64))
              (List.len (Set.to-list (Set.of (list x 2.5 1.5)))))
            (export main)))
  (call   main (: 3.5 Float64)) (output (: 3 Int64)))

(case "Set.to-list over Float64 elements yields canonical BYTE order — negatives after positives"
  (doc    "The order face: canonical byte order sorts by the IEEE bit pattern's byte form, so a NEGATIVE
           float (sign bit set) sorts AFTER every positive — {-1.0, 0.5, 2.5} enumerates [0.5, 2.5, -1.0],
           index 0 is 0.5 (NOT -1.0 as numeric order would give). This is rust's already-computed order and
           the ruling's blessed target; a fix that sorts numerically instead would pass the len case above
           but fail here. Encodes: index0==0.5 -> 1.")
  (input  (do
            (def (main (: x Float64))
              (match (List.at (Set.to-list (Set.of (list x 0.5 2.5))) 0)
                ((Some f) (if (= f 0.5) 1 0))
                ((None u) -1)))
            (export main)))
  (call   main (: -1.0 Float64)) (output (: 1 Int64)))

(case "Map.to-list over Float64 KEYS enumerates (len is the entry count)"
  (doc    "The Map-key twin: `(List.len (Map.to-list (Map.insert (Map.insert Map.empty x 1) 2.5 2)))`
           with runtime x:Float64 — 2 entries. Same orderable-descriptor gate, same fix (the Map.to-list
           key sort takes canonical bytes).")
  (input  (do
            (def (main (: x Float64))
              (List.len (Map.to-list (Map.insert (Map.insert Map.empty x 1) 2.5 2))))
            (export main)))
  (call   main (: 3.5 Float64)) (output (: 2 Int64)))

; ===== PM triage (corpus-bugfix, 2026-07-20, VERIFIED trunk 7f6f074e3) =====
; Float64-element Set.to-list DECLINES on wasm ('element shape has no orderable descriptor') — confirmed
; live. Ruling A in hand (compute by canonical BYTE order both backends). ROUTED to v-runtime (sort-comparator
; Float leaf — compare_scalar_leaf returns Some(None) for Float; the Float sibling of the fixed compound gap
; 19-sets:786). NOT spawning a fix agent — runtime comparator change in v-runtime's lane. corpus-bugfix to PIN
; the 3 cases (len / order-face [0.5,2.5,-1.0] + signed-zero-distinct + NaN-last / Map-key twin) once landed.

; ===== ORDER CORRECTION (v-runtime root-cause, 2026-07-20) — supersedes my "NaN last" phrasing =====
; v-runtime confirmed the EXACT order = UNSIGNED u64 to_bits() cmp (matches rust __CdzF64), TWO loci both
; theirs: (1) compile-time orderable_leaf_or_compound (lower.rs:23386) returns false for Ty::Float — fix is
; a to-list-SCOPED canonical-byte predicate, NOT flipping the shared numeric-< one; (2) runtime
; compare_scalar_leaf (cdz-runtime lib.rs:5803) returns None for Shape::Float — a FROZEN-HASH BUMP.
; CONFIRMED ORDER: {-1.0,0.5,2.5}->[0.5,2.5,-1.0] (negatives LAST, sign=bit63); {-0.0,0.0}->[0.0,-0.0]
; (+0.0 first, distinct). ⚠ NaN CORRECTION: canonical NaN 0x7ff8000000000000 in UNSIGNED u64 order sorts
; AFTER positive finites but BEFORE negatives — NOT strictly "last", it's LAST-AMONG-POSITIVES. When pinning
; the NaN-position case, PIN TO RUST'S ACTUAL to-list OUTPUT (cargo xtask gate --target rust on the NaN case),
; NOT the "NaN last" phrasing — both backends use the identical u64 to_bits cmp so rust's output IS the target.
; TIMING: hash-bump, so v-runtime builds on a clean runway (after their queued strings MR + pr-sync healthy),
; then pings me to pin the 3 cases. Scheduled, not dropped.

; ===== NaN-PIN GOTCHAS (v-runtime, 2026-07-20) — refine the pin plan =====
; (1) a CONST NaN element is REJECTED at compile time ('(/ 0.0 0.0)' + NaN literals -> 'a floating-point
;     operation whose result is not finite has no value form yet'). The NaN set-element case MUST source NaN
;     at RUNTIME (a Float64 param passed as NaN, or a runtime float op yielding NaN into the set).
; (2) canonical NaN bits 0x7ff8000000000000 sort BETWEEN positive finites and negatives in u64-to_bits order
;     -> NaN is NOT strictly 'last', it's after-positives/before-negatives. Pin NaN position to rust's ACTUAL
;     to-list output (gate --target rust), not the phrasing.
; PIN PLAN: land the 3 FINITE cases first (len + finite order-face [0.5,2.5,-1.0] incl signed-zero-distinct +
;     Map-key twin — all solid), NaN-position as a FOLLOW-UP pin once a runtime-NaN construction path is settled.
;     v-runtime builds the finite (frozen-hash) fix on a clean runway, then pings me to pin.

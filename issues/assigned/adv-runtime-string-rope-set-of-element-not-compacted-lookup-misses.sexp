; ADVERSARIAL FINDING (producer, iter-367, 2026-07-14) — 🔴 MISCOMPILE / WRONG VALUE (the LAST remaining half
; of the runtime-String-rope champ-key family #353/#347/#343): `Set.of` (the of-arr BATCH construction path)
; does NOT compact a runtime String ROPE element before champ-insert, so a set built by `Set.of` from a
; concatenated string cannot be membership-tested with the equal FLAT string — `Set.contains` returns false.
;
; The champ-key compaction fix has landed incrementally: value-eq (earlier), Map.insert/lookup + Set.contains
; QUERY key + Set.INSERT element (all now compact). But `Set.of`'s of-arr element-insert path was MISSED — a
; rope element goes in uncompacted.
;
; REPRODUCER (returns 0 — WRONG; the element "hixxx" IS in the set, so contains "hixxx" must be true → 1):
;   (do (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
;       (def (main) (if (Set.contains (Set.of (list (rep "hi" 3))) "hixxx") 1 0))
;       (export main))
;   → 0   (the rope element `(rep "hi" 3)`="hixxx" via Set.of is not found by the flat literal "hixxx")
;
; ISOLATION (Set.INSERT is FIXED; Set.OF is NOT — the exact split):
;   Set.INSERT rope element, then Set.contains flat        → 1    [FIXED — insert path compacts]
;   Set.OF (list <rope>), then Set.contains flat "hixxx"   → 🔴 0  (of-arr element NOT compacted)
;   Set.OF (list <rope> "other"), Set.contains flat        → 🔴 0  (still missed; the other element is unrelated)
;   Set.OF (list <rope>), then Set.REMOVE flat → Set.len   → 🔴 1  (iter-386: a stronger twin — the flat-twin
;                                                                   remove QUERY (compacted) can't find the stored
;                                                                   rope, so nothing is removed; len stays 1 not 0.
;                                                                   Same root cause, and it shows the defect hits
;                                                                   Set.remove too, not just Set.contains.)
;   Map.insert rope-key, Map.lookup flat                   → 42   [FIXED]
;   Map.insert rope-key, Map.REMOVE flat → Map.size        → 0    [FIXED — insert-path map stores canonical]
;   Set.of (list <rope> <flat-twin>) → Set.len             → 1    [dedup: the of-arr COMPARE canonicalizes one
;                                                                   against the other, so a co-inserted flat twin
;                                                                   MASKS the bug — but a LONE rope stays a rope]
;   Set.of (list <rope>) then Set.INSERT flat-twin → len   → 1    [dedup works here too — the later insert's
;                                                                   compacted probe champ_eq-collapses against the
;                                                                   stored rope; only a QUERY (contains/remove)
;                                                                   whose key is compacted MISSES the raw rope]
;
; ROOT CAUSE (hypothesis): the champ-key compaction (a `bytes-compact` on a String key/element before
; champ_hash/champ_insert) was added to `Set.insert` / `Map.insert` / the Set query key, but NOT to `Set.of`'s
; of-arr batch element-insert. So `Set.of` champ-inserts each element with its raw (rope) bytes; a lone rope
; element lands under its rope champ_hash, and a later `Set.contains` of the flat twin (whose query key IS
; compacted) hashes elsewhere → miss. The `Set.of [rope, flat-twin]` dedup only works because the of-arr
; insertion COMPARES the two elements (champ_eq after compacting the second's probe), collapsing them — it does
; not reflect that a stored lone rope is canonical.
;
; FIX (hypothesis): compact a String element (and a String nested in a compound element) in `Set.of`'s of-arr
; element-insert path, the same `bytes-compact` `Set.insert`/`Map.insert` now apply — so a rope element built by
; `Set.of` is stored canonical and a flat query finds it. Mirror the Set.insert element compaction on Set.of.
;
; SEVERITY: 🔴 MISCOMPILE — a valid, well-typed program returns the WRONG value: a set built via `Set.of` from
; runtime-concatenated Strings (a seen-set of interned identifiers assembled in a batch) silently fails
; membership tests against literals. Not a crash (the earlier Set.insert invalid-wasm is fixed) — a silent
; wrong answer. Reachable from the idiomatic "collect assembled names into a set via Set.of, then test
; membership by a literal". Grades Fail (returns 0 where 1 is expected). The Set.insert twin is fixed; this is
; the Set.of remainder.

(case "a runtime string rope built into a set via Set.of is a member"
  (doc    "`(Set.contains (Set.of (list (rep \"hi\" 3))) \"hixxx\")` — a set built by `Set.of` from a runtime
           String rope `(rep \"hi\" 3)`=\"hixxx\" (three String.concat), membership-tested with the flat
           literal \"hixxx\". Must be true → 1. Instead returns 0 (false): `Set.of`'s of-arr element-insert
           does not compact the rope element, so it lands under its rope champ_hash and the flat query (whose
           key IS compacted) misses. `Set.insert` of the same rope + the same flat query → 1 (the insert path
           was fixed); Map.insert rope-key → 42 (fixed) — only Set.of's batch element-insert remains
           uncompacted. `Set.of [rope, flat-twin]` dedups to len 1 (the of-arr compare masks a lone-rope miss).
           Fix: compact a String element in Set.of's of-arr path, as Set.insert/Map.insert now do. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (Set.contains (Set.of (list (rep "hi" 3))) "hixxx") 1 0))
            (export main)))
  (output (: 1 Int64)))

;; ASSIGNED 2026-07-15: claimed by v-runtime (rope-canonicalization design). NOT a fix-agent job.

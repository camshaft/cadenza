; ADVERSARIAL FINDING (breaker, 2026-07-15) — 🔴 MISCOMPILE / SILENT WRONG VALUE: the runtime-String-ROPE
; compaction fix (13-strings.sexp: "a runtime string rope compares equal to its flat twin") canonicalizes a
; String operand at the TOP-LEVEL `=`/match/champ-key sites — but NOT inside the structural value-eq WALK.
; A rope NESTED IN A COMPOUND (tuple element, record field, Option payload) is compared by champ_eq's
; recursion with its raw rope bytes, so it compares UNEQUAL to its flat twin of identical content. The same
; miss hits a COMPOUND Map key containing a rope (the key's nested string leaf is not compacted), so a
; flat-twin lookup misses.
;
; The next face of the champ-key/value-eq compaction family (#353/#347/#343 + the Set.of remainder): every
; landed fix compacts the string when the string IS the operand/key/element; none compacts a string LEAF
; reached THROUGH a compound.
;
; REPRODUCERS (wasm backend, trunk@20cd2142, freshly built store; each hand-recomputed):
;   rep s n = n<1 ? s : rep (String.concat s "x") (n-1)   — rep "hi" 3 = "hixxx", rep "hi" 1 = "hix"
;
;   (= (tuple (rep "hi" 3) 1) (tuple "hixxx" 1))              → 🔴 false (0)  expected true (1)
;   (= (tuple (rep "hi" 1) 1) (tuple "hix" 1))                → 🔴 0          [MINIMAL: ONE concat suffices]
;   (= (Option.Some (rep "hi" 3)) (Option.Some "hixxx"))      → 🔴 0          (sum-payload face)
;   (= (record (f (rep "hi" 3)) (g 1)) (record (f "hixxx") (g 1))) → 🔴 0     (record-field face)
;   (Map.lookup (Map.insert Map.empty (tuple (rep "hi" 3) 1) 42) (tuple "hixxx" 1))
;                                                             → 🔴 None (-1)  expected (Some 42)
;                                                             (compound-KEY face: nested leaf not compacted
;                                                              at the champ-insert of the tuple key)
;
; CONTROLS (all pass — isolate the nested-rope face exactly):
;   (= (rep "hi" 3) "hixxx") at TOP level                     → 1   [the landed fix works there]
;   (= (tuple (rep "hi" 0) 1) (tuple "hi" 1))                 → 1   [a FLAT runtime string nested is fine —
;                                                                    it's the ROPE, not runtime-ness]
;   (= (tuple (rep "hi" 3) 1) (tuple (rep "hi" 3) 1))         → 1   [identically-built ropes both sides —
;                                                                    same physical shape masks it]
;   (= (tuple "hixxx" 1) (tuple "hixxx" 1)) constants          → 1   [folds]
;   list-nested `(= (list (rep "hi" 3)) (list "hixxx"))`       → declines (todo, not wrong)
;
; ROOT CAUSE (hypothesis): the landed compaction is applied to the string operand AT the =/match/key SITE
; (a `bytes-compact` before champ_hash/champ_eq when the operand's TYPE is String). When the operand is a
; TUPLE/RECORD/SUM, no compaction happens at the site, and champ_eq's recursive walk compares the nested
; string leaf PHYSICALLY (rope vs flat leaf bytes differ) instead of canonically. Fix faces: either
; (a) champ_eq/champ_hash canonicalize a string leaf during the walk (runtime half — matches how the walk
; already applies float canonical-byte rules to nested floats), or (b) the compiler compacts string leaves
; when CONSTRUCTING a compound from an owned rope (compile-time half; but that leaves later-built ropes
; reaching compounds via params uncovered — (a) is the sound general fix).
;
; SEVERITY: 🔴 MISCOMPILE — well-typed program, silent wrong value; equality of equal values answers false
; and a Map keyed by a compound containing an assembled string can't be looked up by literal. Reachable from
; the idiomatic "key a map by (name, arity) where name was assembled by concat". NOT the same bug as the
; Set.of remainder (adv-runtime-string-rope-set-of-element-not-compacted-lookup-misses.sexp — that's the
; batch of-arr ELEMENT path; this is the value-eq WALK + compound-KEY path), though one runtime-side fix
; (canonicalize string leaves in champ_eq/champ_hash) would close both key faces.
;
; NOTE the float precedent: the value-eq walk ALREADY applies canonical-byte comparison to a float nested in
; a compound (03-equality: "a NaN nested in a tuple compares equal under the canonical byte form") — the
; string leaf needs the same treatment (canonical = compacted bytes).
;
; Graded cases below: the minimal tuple face + the strongest (map-key) face.

(case "a one-concat rope nested in a tuple equals its flat twin"
  (doc    "`(= (tuple (rep \"hi\" 1) 1) (tuple \"hix\" 1))` — the left tuple's string element is a runtime
           ROPE (one String.concat, content \"hix\"), the right's is the flat literal \"hix\". Structural
           equality compares component-wise and string equality is by content (canonical bytes) — the
           landed top-level fix compacts a rope OPERAND of `=`, and the value-eq walk already applies the
           canonical-byte rule to a NESTED float (NaN-in-tuple case) — so the tuples are equal → 1.
           Instead returns 0: the walk compares the nested string leaf physically (rope bytes ≠ flat
           bytes). Top-level `(= (rep \"hi\" 1) \"hix\")` → 1 (fixed); a nested FLAT runtime string → 1;
           only a nested ROPE misses. Fix: canonicalize a string leaf inside champ_eq/champ_hash, as the
           float canonical-byte rule already does. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (tuple (rep "hi" 1) 1) (tuple "hix" 1)) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a compound map key containing a rope is found by its flat-twin key"
  (doc    "The champ-KEY face: `(Map.insert Map.empty (tuple (rep \"hi\" 3) 1) 42)` keys the map by a TUPLE
           whose string element is a runtime rope (content \"hixxx\"); `(Map.lookup … (tuple \"hixxx\" 1))`
           looks up with the flat-twin tuple. Equal keys → must find 42. Instead returns None (→ -1): the
           tuple key is champ-hashed with its nested rope leaf UNCOMPACTED, so it lands in a different slot
           than the flat-twin query key. The scalar string-key faces are fixed (13-strings map-key cases);
           only a string leaf NESTED in a compound key misses. Same root as the value-eq walk face.
           Expected: 42.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main)
              (match (Map.lookup (Map.insert Map.empty (tuple (rep "hi" 3) 1) 42) (tuple "hixxx" 1))
                ((Some v) v)
                ((None) (- 0 1))))
            (export main)))
  (output (: 42 Int64)))

(case "a rope in an Option payload equals its flat-twin payload"
  (doc    "The sum-payload face: `(= (Option.Some (rep \"hi\" 3)) (Option.Some \"hixxx\"))` — tags match
           (both Some), payloads are content-equal strings (rope vs flat \"hixxx\") → true → 1. Instead 0:
           the payload compare is physical. The float twin of this exact case passes (`(= (Some
           Float64.nan) (Some Float64.nan))` — canonical byte form applied to a sum payload). Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (Option.Some (rep "hi" 3)) (Option.Some "hixxx")) 1 0))
            (export main)))
  (output (: 1 Int64)))

(case "a rope in a record field equals its flat-twin field"
  (doc    "The record-field face: `(= (record (f (rep \"hi\" 3)) (g 1)) (record (f \"hixxx\") (g 1)))` —
           same field set, field `g` equal, field `f` content-equal strings → true → 1. Instead 0. The
           fourth face of the same nested-leaf miss. Expected: 1.")
  (input  (do
            (def (rep (: s String) (: n Int64))
              (if (< n 1) s (rep (String.concat s "x") (- n 1))))
            (def (main) (if (= (record (f (rep "hi" 3)) (g 1)) (record (f "hixxx") (g 1))) 1 0))
            (export main)))
  (output (: 1 Int64)))

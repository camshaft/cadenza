; FINDING (breaker, 2026-07-28): wasm emits an INVALID MODULE (i64 where i32 expected, same
; validator signature as the FIXED to-bytes br_table bug 4f9658803 but a DIFFERENT seam) for a
; recursive fn whose String param is REBOUND to a helper-built concat-of-two-slices, when BOTH:
;   (a) the helper takes the slice INDEX as its own Int64 param (slice bounds i / i+1), AND
;   (b) the recursion's exit test reads (String.scalar-len s) on the REBOUND param.
; Either ingredient alone is fine (m9: param-i helper + const bound OK; m10: const-index helper
; + scalar-len bound OK); together -> func 13 fails to validate "expected i32, found i64".
; rust computes 3 (ground truth: greedy drop-scalar walk from "aébcd" -> "éc" byte-len 3).
; Discovered probing a scalar-aware STRING SHRINKER (property-testing idiom) — the exact code a
; user writes for compound string shrinking.
;
; Matrix (all built during minimization):
;   FAIL m4/m11  helper(s,i) slice[0,i)++slice[i+1,scalar-len] + walk bound scalar-len(s)
;   ok   m9      same helper, walk bound CONST
;   ok   m10     helper const-index, walk bound scalar-len
;   ok   m7/m8   inline or helper concat-of-slices, const everything else
;   ok   m5/m6   concat-only or single-slice rebind
;   ok   m1      helper alone (no recursion)
;
; Smell: i64/i32 local-width confusion when the scalar-len(s) of a REBOUND (loop-carried) rope
; and the helper's Int64 index both feed the same recursive frame — same family as the fixed
; br_table slot bug but in the recursive-param spill path.
;
; GRADED REPRO (= post-fix pin; rust passes 3 today, wasm invalid-module):
(case "a recursive drop-scalar walk over a rope converges (string-shrinker shape)"
  (input  (do
        (def (d (: s String) (: i Int64))
          (String.concat (Option.expect (String.slice s 0 i) "lo")
                         (Option.expect (String.slice s (+ i 1) (String.scalar-len s)) "hi")))
        (def (walk (: s String) (: i Int64))
          (if (>= i (String.scalar-len s))
              s
              (walk (d s i) (+ i 1))))
        (def (main (: mode Int64))
          (String.byte-len (walk "aébcd" 0)))
        (export main)))
  (call   main (: 0 Int64)) (output (: 3 Int64)))

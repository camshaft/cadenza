; corpus-bugfix probe (trunk 9b3a0f47a) — 4th/5th width-descent holes, folded into v-inference's
; consolidated width-fit audit (pr-sync-endorsed) per the "route the next hole into the same audit" directive.
;
; The compound-payload width-descent ESCAPES the fit-check for MAP VALUES and MAP KEYS too (same class as
; the record-field / user-sum escape). Verified silent miscompile:
;   (: (map (1 999)) (Map Int64 Int8)) — value 999 as Int8 → Map.lookup 1 = -25 (silent truncation). Should REJECT CDZ0302.
;   (: (map (999 1)) (Map Int8 Int64)) — key 999 as Int8 compiles (should REJECT CDZ0302).
;
; FULL DESCENT-GAP SCOPE (corpus-bugfix mapped the matrix):
;   ESCAPE (bug, should reject): record fields, user-sum payloads, MAP VALUES, MAP KEYS.
;   CORRECT (rejects CDZ0302 today): Option payload, Result payload, TUPLE elements, LIST elements.
; So the descent covers the "linear" compound arms (Option/Result/tuple/list) but misses record-field,
; user-sum-payload, and BOTH map positions. Expected CDZ0302 uniformly.
;
; Reject cases to PIN once v-inference's audit lands (corpus-bugfix, all 3 backends):
(case "a map VALUE literal over its narrow annotated width is rejected (width descent reaches map values)"
  (input  (: (map (1 999)) (Map Int64 Int8)))
  (error CDZ0302))
(case "a map KEY literal over its narrow annotated width is rejected (width descent reaches map keys)"
  (input  (: (map (999 1)) (Map Int8 Int64)))
  (error CDZ0302))

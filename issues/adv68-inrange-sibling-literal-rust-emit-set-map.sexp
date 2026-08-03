; adv-68 (breaker, 2026-08-06): RUST backend — an IN-RANGE integer literal whose element/value
; type arrives from SIBLING UNIFICATION in a Set.of or Map.insert emits at its Int64 default
; spelling, producing E0308 (artifact does not build) while wasm computes correctly. The LIST
; face works on both backends (the #1766 list machinery grounds the join into the emit); the
; #1780 fix added the RANGE CHECK for these seams but the rust EMIT of the in-range literal
; still doesn't consult the sibling-solved width.
;
; Hand-verify: 41 fits UInt64; {1,41} len 2. wasm: 2. rust: E0308 on the 41 emit.
; Same shape for a Map VALUE (30 fits UInt8). Map KEY face untested in isolation (sw2's
; out-of-range key correctly rejects; the in-range key face likely shares the value face's gap).

(case "sw3 an in-range literal typed by a Set.of sibling compiles and runs"
  (input  (Set.len (Set.of (list (: 1 UInt64) 41))))
  (output (: 2 Int64)))

(case "sw5 an in-range literal typed by a Map.insert value sibling compiles and runs"
  (input  (Map.len (Map.insert (Map.insert Map.empty 1 (: 5 UInt8)) 2 30)))
  (output (: 2 Int64)))

(case "sw4 the LIST face control (works both backends)"
  (input  (List.len (list (: 1 UInt64) 41)))
  (output (: 2 Int64)))

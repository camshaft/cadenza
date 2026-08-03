; adv-67b (breaker, 2026-08-03): RUST backend — odd-width `<<` overflow does NOT trap; the
; overflow round-trip check runs at the MACHINE SLOT width, so a result exceeding the DECLARED
; width (but fitting the slot) passes and escapes. Same declared-vs-slot-width family as adv-67
; (Div MIN/-1 guard) — one fix arc likely covers both. wasm traps correctly on every face.
;
; ROOT (read, not fixed): backend/rust/expr.rs ~:4567 Prim::Shl guarded path:
;   let r = v << c; if (r >> c) != v { panic!("integer overflow in left shift") }
; v: {vty} = the SLOT type (i32 for UInt4/Int24). UInt4 3<<3 = 24 fits i32 and round-trips
; losslessly, so no panic — but 24 > UInt4 max 15 (wasm: integer overflow trap). Int24
; 4194304<<1 = 8388608 > Int24 max 8388607, same shape. MACHINE widths are correct on rust
; (UInt8 200<<1 traps — the round-trip at u8 catches the dropped bit; slot == declared).
; The COUNT guard is correct (it.ground_width() = declared width: UInt4 << 4 traps both
; backends). CONST face folds to CDZ0304 on both (correct). Only the runtime odd-width
; RESULT-overflow path splits.
;
; Hand-recompute: 3<<3 = 24 > 15 → must trap. 4194304<<1 = 8388608 > 8388607 → must trap.

(case "sh6 odd-width shift result exceeding the declared width traps (UInt4: 3<<3)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (<< ((. (UInt 4) wrap) 3) ((. (UInt 4) wrap) k))))
            (export main)))
  (call   main (: 3 Int64)) (trap "integer overflow")
  (call   main (: 2 Int64)) (output (: 12 Int64)))

(case "sh8 SIGNED odd-width shift overflow traps (Int24: 4194304<<1)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (<< ((. (Int 24) wrap) 4194304) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: 1 Int64)) (trap "integer overflow")
  (call   main (: 0 Int64)) (output (: 4194304 Int64)))

(case "sh11 the escaped out-of-range UInt4 shift result poisons a CHAMP Set (rust builds {24,8} len 2)"
  (input  (do
            (def (main (: k Int64))
              (Set.len (Set.of (list (<< ((. (UInt 4) wrap) 3) ((. (UInt 4) wrap) k)) ((. (UInt 4) wrap) 8)))))
            (export main)))
  (call   main (: 3 Int64)) (trap "integer overflow"))

; adv-67 (breaker, 2026-08-03): RUST backend — odd-width signed division MIN / -1 does NOT trap;
; returns +2^(N-1), an OUT-OF-RANGE value for the declared width, which then escapes into Int64
; arithmetic unchecked. wasm traps correctly ("integer overflow"). rust + rust-async both affected.
;
; ROOT (read, not fixed): backend/rust/expr.rs ~:4456 Prim::Div overflow guard emits
;   `else if l == {t}::MIN && r == -1 { panic!("division overflow") }`
; where {t} = types::rust_type(&Ty::Int(it)) — the MACHINE SLOT type (i32 for Int24, i64 for
; Int48). The declared-width minimum (-8388608 for Int24) never equals i32::MIN, so the guard
; never fires and `l / r` yields +8388608 (fits the slot; violates the Int24 invariant).
; +, -, * trap correctly at the declared width (their checked path re-checks the declared range);
; % is correct (MIN % -1 = 0 by the numeric model). Machine widths (Int8/16/32/64) are unaffected
; (slot MIN == declared MIN).
;
; Hand-recompute: Int24 min = -8388608; /-1 = +8388608 > Int24 max 8388607 → overflow, MUST trap.
; The CONST face folds to CDZ0304 on both backends (correct) — only the RUNTIME divisor path splits.

(case "od1 odd-width division overflow traps at the declared width (Int24 min / -1)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) k))))
            (export main)))
  (call   main (: -1 Int64)) (trap "integer overflow")
  (call   main (: 2 Int64)) (output (: -4194304 Int64)))

(case "odx4 Int48 min / -1 (odd width above the i32 slot, same wrong-value shape)"
  (input  (do
            (def (main (: k Int64))
              (Int64.of (/ ((. (Int 48) wrap) -140737488355328) ((. (Int 48) wrap) k))))
            (export main)))
  (call   main (: -1 Int64)) (trap "integer overflow"))

(case "odx8 the escaped out-of-range Int24 poisons downstream Int64 arithmetic (rust yields 8388608)"
  (input  (do
            (def (main (: k Int64))
              (let ((bad (/ ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) k))))
                (Int64.of (+ bad ((. (Int 24) wrap) 0)))))
            (export main)))
  (call   main (: -1 Int64)) (trap "integer overflow"))

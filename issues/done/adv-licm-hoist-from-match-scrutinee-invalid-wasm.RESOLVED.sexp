; ADVERSARIAL FINDING (producer, iter-383, 2026-07-14) — 🔴 CRASH / INVALID WASM (regression from 9bccb36a):
; LICM hoisting a loop-invariant subexpression out of a MATCH SCRUTINEE produces INVALID wasm — the module
; fails validation with `type mismatch: expected i32, found i64`, so `cdz-run` rejects the component
; ("invalid component: failed to compile: wasm[0]::function[1]"). A tail-recursive loop whose match scrutinee
; contains a loop-invariant subexpression (e.g. `(match (< i (+ n 1)) …)`, `(+ n 1)` invariant since `n`
; threads unchanged) can no longer be compiled at all. The `if`-condition twin `(if (< i (+ n 1)) …)` is
; FINE — the bug is specific to the MATCH-scrutinee hoisting position that 9bccb36a newly added to the frontier.
;
; REGRESSION: 9bccb36a ("LICM hoists a TRAPPING loop-invariant when it sits in an always-evaluated position")
; extended `collect_hoistable` to hoist an invariant when it is in the loop body's DOMINATING FRONTIER, which
; now INCLUDES the match scrutinee. The if-condition frontier position was already hoisted correctly (and still
; is); the match-scrutinee position was added by this commit and mis-wires the hoisted slot's type/read, emitting
; an i64 where the match dispatch expects an i32 (the bool discriminant / branch selector). Verified by
; checkout: at the parent bf5a1a1c the minimal case compiles to VALID wasm (181 bytes) and returns 5; at
; 9bccb36a+ (spec tip ad09810a) the same program is INVALID wasm (188 bytes, func 1 fails to validate).
;
; REPRODUCER (INVALID WASM — cannot run; must return 5):
;   (do (def (loop (: i Int64) (: n Int64))
;         (match (< i (+ n 1)) (true (loop (+ i 1) n)) (false i)))
;       (def (main) (loop 0 4))
;       (export main))
;   → cdz-run: invalid component: failed to compile: wasm[0]::function[1]
;   → wasm-tools validate: "func 1 failed to validate … type mismatch: expected i32, found i64"
;
; ISOLATION (the ONLY difference between valid and invalid is a HOISTABLE INVARIANT in the MATCH scrutinee):
;   MATCH (< i n)          [no invariant subexpr]              → 6    [OK — nothing to hoist]
;   MATCH (< i (+ i 1))    [i-dependent, NOT invariant]        → 6    [OK — not hoisted]
;   MATCH (< i (+ n 1))    [(+ n 1) invariant → HOISTED]       → 🔴 INVALID WASM  ← THE BUG
;   MATCH (< i (* n 2))    [the commit's own doc example]      → 🔴 INVALID WASM
;   IF    (< i (+ n 1))    [SAME invariant, if-condition]      → 5    [OK — if frontier hoist works]
;   IF    (< i (* n 2))    [the commit's if-form example]      → 28   [OK — documented, works]
;   let ((lim (+ n 1))) (match (< i lim) …)                    → 🔴 INVALID WASM (still hoisted/mis-wired)
;   → so: MATCH scrutinee + a hoistable invariant = invalid wasm; the IF twin and the no-invariant / varying
;     scrutinee cases all compile & run. Match-scrutinee-specific, invariant-triggered.
;
; ROOT CAUSE (hypothesis, backend/wasm — the LICM frontier + match lowering seam): 9bccb36a added the match
; scrutinee to `collect_dominating_frontier`, so `collect_hoistable` now hoists an invariant out of the
; scrutinee into a pre-loop slot and rewrites the scrutinee to read the slot. But the match dispatch consumes
; its scrutinee as an i32 (the bool discriminant / variant selector), while the hoisted-slot read is emitted
; as i64 (the invariant's Int64 arithmetic type) — the slot-read replacement is not coerced/typed to the
; discriminant width the match lowering expects. The if-condition path already handled this (its condition is
; consumed as i32 and the hoist wires correctly); the match-scrutinee path was not updated to match.
;
; FIX (hypothesis): in the match-scrutinee hoisting path, emit the hoisted slot-read at the width the match
; dispatch expects (the i32 discriminant), OR exclude the match scrutinee from the hoisting frontier until the
; slot-read wiring is width-correct there. The if-condition frontier hoist is the working reference.
;
; SEVERITY: 🔴 CRASH / INVALID WASM — a valid, well-typed, previously-compiling program can no longer be
; compiled: the emitted module fails wasm validation and the component is rejected at load. Reachable from the
; idiomatic tail-recursive loop written with `match` on a comparison whose bound is loop-invariant (e.g.
; `(match (< i (+ n 1)) (true …) (false …))`) — a common counted-loop shape. Regression from 9bccb36a; the
; parent bf5a1a1c compiles & runs it correctly (→5). Grades Fail (invalid wasm, no value produced).

(case "a loop-invariant subexpression in a match scrutinee compiles to valid wasm"
  (doc    "`(match (< i (+ n 1)) (true (loop (+ i 1) n)) (false i))` — a tail-recursive counted loop whose
           MATCH scrutinee `(< i (+ n 1))` contains the loop-invariant `(+ n 1)` (`n` threads unchanged). Must
           loop i:0→5 and return 5. Instead emits INVALID WASM: `wasm-tools validate` reports `func 1 failed
           to validate … type mismatch: expected i32, found i64`, and `cdz-run` rejects the component. The
           if-condition twin `(if (< i (+ n 1)) …)` returns 5 correctly; only the match-scrutinee position
           breaks. Regression from 9bccb36a, which added the match scrutinee to the LICM dominating frontier:
           the hoisted invariant's slot-read is emitted at i64 (the Int64 arithmetic width) where the match
           dispatch consumes its scrutinee as an i32 discriminant. Parent bf5a1a1c compiles & runs this → 5.
           Fix: coerce the hoisted match-scrutinee slot-read to the discriminant width, or exclude the match
           scrutinee from the hoist frontier. Expected: 5.")
  (input  (do
            (def (loop (: i Int64) (: n Int64))
              (match (< i (+ n 1)) (true (loop (+ i 1) n)) (false i)))
            (def (main) (loop 0 4))
            (export main)))
  (output (: 5 Int64)))

;; RESOLVED 2026-07-15 (trunk@2ac25eab): fix landed, gate PASSes. Agent self-removed.

; KNOWN LIMIT (v-compiler-perf, 2026-07-16, concierge-ratified PARK) — NOT a bug to fix now; a documented
; ceiling with a ready-to-build fix, awaiting a REAL trigger.
;
; WHAT: a match whose arms each test ≥2 LITERAL COLUMNS over a RUNTIME scrutinee (a transition-table /
; parser `(tuple state token payload)` dispatch, or a refined-list `(Some (list a b))` payload) emits an
; EXPONENTIALLY-SIZED wasm module. Measured (runtime 2-column `(match (tuple p q p) ((tuple i i c) …))`,
; `cdz compile -t wasm`): 8/12/16 arms = 15KB / 242KB / 4MB (×~4 per +2 arms). The emitted code is CORRECT
; — just large.
;
; WHAT IS ALREADY FIXED: the COMPILE-TIME O(2^arms) hang (cdz check / diagnostics / LSP) is fixed —
;   - S1 (multi-column, trunk `edded9e09`): build_tree shares the fall-through for non-refining Int/Str
;     probes → O(arms) build + an in-memory decision-DAG.
;   - S3 (refined-list, trunk `1db42a0fd`): `refine_listlen_to_passed` extends the sharing to the ListLen
;     passed-length world.
;   So `cdz check` on these shapes is now LINEAR. Only the wasm EMIT still expands the DAG inline.
;
; WHY PARKED (concierge ratified 2026-07-16, option b): it is CORRECT code on a RARE shape (a runtime
; ≥8-arm multi-column dispatch — a constant scrutinee FOLDS away, and check/LSP is already linear), and
; nothing real is biting the emit-size today. The FIX is a ~150-line wasm-backend change where a wrong
; `br` depth is a SILENT MISCOMPILE, so it deserves an ATTENDED session with a real trigger, not
; unprompted idle-filling.
;
; THE READY-TO-BUILD FIX (full design in the v-compiler-perf memory, perf-loop-state fires #22-24 / #45):
;   `backend/wasm/select.rs::emit_sum_cont` LitTest arm emits `if test { then_ } else { els }` INLINE, so a
;   shared `els` Rc (the decision-DAG's shared fall-through) is re-emitted per reference = 2^N. Fix: emit
;   each SHARED continuation (Rc::strong_count > 1) ONCE behind a labeled `block` join (reuse the existing
;   br_table `$join` pattern in emit_sum_match_arms) — two join blocks (`$done` for matched-body results,
;   `$els` for the shared fall-through), threading a `shared_els: Option<block-depth>` through
;   emit_sum_cont's ~10 call sites, with the Leaf arm emitting `br $done`. Gate: emitted-wasm-BYTES linear
;   (tp_N 242KB@12 → ~KB) + a byte lock-in test.
;
; TRIGGER TO UN-PARK: a real program (e.g. a generated parser/lexer state table, or a compiler-ml pass)
; whose emitted module blows a size budget on this shape. Then escalate to the attended session.
;
; REPRODUCER (runtime 2-column, compiles clean, emits exponential wasm):
(module m
  (def (f (: p Int64) (: q Int64))
    (match (tuple p q p)
      ((tuple 0 0 c) c)
      ((tuple 1 1 c) (+ c 1))
      ((tuple 2 2 c) (+ c 2))
      ((tuple 3 3 c) (+ c 3))
      ((tuple 4 4 c) (+ c 4))
      ((tuple 5 5 c) (+ c 5))
      ((tuple 6 6 c) (+ c 6))
      ((tuple 7 7 c) (+ c 7))
      (_ -1)))
  (export f))

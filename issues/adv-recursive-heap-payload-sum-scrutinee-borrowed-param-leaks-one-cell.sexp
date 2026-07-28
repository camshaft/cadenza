; FINDING (v-wasm-opt drift-guard, 2026-07-24): a self-RECURSIVE fn taking a sum with a HEAP payload
; (BigInt OR String) as a BORROWED param leaks exactly 1 live cell when the recursion unwinds. VALUE
; CORRECT (no UAF, no wrong value) — a pure LEAK. Surfaced co-verifying v-inference's FACE-B land
; (b9eb90e14) with the debug-counters ComposedRuntime live_objects() probe.
;
; ISOLATION (all under debug-counters runtime, live_objects after call):
;   ✗ recursive walk, BigInt payload, LITERAL probe ((Mk 1)…), const (Mk 1) arg      → val 40, live 1
;   ✗ recursive walk, BigInt payload, LITERAL probe, RUNTIME-built arg (Mk (BigInt.of k)) → val 40, live 1
;   ✗ recursive walk, BigInt payload, LITERAL probe, arm NEVER matches (falls through) → val -1, live 1
;   ✗ recursive walk, BigInt payload, BIND payload ((Mk x)…) — NO probe at all        → val 3,  live 1
;   ✗ recursive walk, STRING payload, BIND payload ((Mk x)…) — NO probe, NOT BigInt   → val 2,  live 1
;   ✓ Int64 payload (scalar, not heap) recursive walk, same shape                     → val 40, live 0
;   ✓ NON-recursive heap-payload sum, bind/probe                                      → live 0 (folds/clean)
;
; So: HEAP-payload-sum-specific (Int64 scalar payload = 0), RECURSIVE-fn-specific (non-recursive = 0),
; probe-INDEPENDENT (a plain (Mk x) bind leaks the same 1), const-vs-runtime-arg-INDEPENDENT. The heap
; payload sum `w` is a BORROWED self-recursion param; across the recursive tail-ish call the sum shell (or
; its payload leaf) is retained-but-not-dropped once when the frames unwind → 1 orphaned cell.
;
; NOT a v-wasm-opt or v-inference regression: independent of FACE-A (5505b5010 probe) AND FACE-B
; (77e8ca8b1 const nominal-peel) — the bind-only + String variants never touch either path, and both
; faces' VALUE results are correct. This is a Perceus reclaim gap in recursive-param heap-sum ownership.
; TERRITORY: v-memory-safety (dup/drop placement for a heap-payload sum held as a borrowed recursion param).
;
; Repro (minimal, bind-only — no probe machinery involved):
(case "a recursive fn holding a heap-payload sum as a borrowed param leaks one cell on unwind"
  (input (do
        (type W (Mk BigInt))
        (def (mk (: k Int64)) (Mk (BigInt.of k)))
        (def (walk (: n Int64) (: w W))
          (if (>= n 0) (walk (- n 1) w) (match w ((Mk x) (Int64.of x)))))
        (def (main) (walk 1 (mk 3)))
        (export main)))
  (output (: 3 Int64)))

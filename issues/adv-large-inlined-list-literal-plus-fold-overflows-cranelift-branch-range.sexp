; ADVERSARIAL FINDING (producer, iter-328, 2026-07-14) — RUNTIME CRASH via a compiler emission-size issue: a
; large INLINED list literal (~≥20k elements) consumed by a fold in the SAME function emits a single wasm
; function so large that cranelift (the cdz-run/wasmtime JIT) PANICS on a branch-offset range assertion:
;
;   thread 'main' panicked at cranelift-codegen-…/src/machinst/buffer.rs:1395:17:
;   assertion failed: (label_offset - offset) <= kind.max_pos_range()
;
; The emitted wasm VALIDATES (wasm-tools validate is clean) — it is not an invalid-wasm bug — but the single
; function is big enough (a ~20k-element list literal's element-by-element construction + the fold loop, all
; inlined into `main`, ~290KB wasm) that a conditional/loop branch's offset exceeds cranelift's
; `max_pos_range()`, so cranelift aborts while JIT-compiling it. So a VALID program crashes `cdz-run`.
;
; REPRODUCER (crashes cdz-run at ~≥20k; the exact threshold is ~15k–20k elements):
;   (do (def (sum (: xs (List Int64)) (: acc Int64))
;         (match xs ((list) acc) ((list x .. rest) (sum rest (+ acc x)))))
;       (def (main) (sum <a 20000-element (list 2 2 2 … 2)> 0)) (export main))
;
; ISOLATION (it is the INLINED-LITERAL SIZE, not the fold or construction alone):
;   len(50000-elem literal)                         → 50000  OK   [a huge literal alone is fine]
;   fold sum(10000 literal) / sum(15000 literal)     → OK (20000/30000)
;   fold sum(20000 / 30000 / 50000 literal)          → 🔴 cranelift panic (buffer.rs:1395)
;   build a 30000-list at RUNTIME (small main) + fold → 60000  OK   [runtime-built list ⇒ small main ⇒ fine]
;   → so the crash needs a LARGE LITERAL inlined WITH its consumer into one oversized function; a runtime-
;     built list (or a literal used trivially) stays under the branch-range limit.
;
; ROOT CAUSE (hypothesis): rcdzc inlines a constant list literal's construction (one push/set per element)
; AND the consuming expression into a single wasm function body. At ~20k+ elements the function's machine-code
; size pushes a forward branch (a loop/if internal to the fold or the build) past cranelift's signed
; branch-displacement range (`max_pos_range()`), and cranelift's `MachBuffer` asserts rather than falling back
; to a long-branch/island. rcdzc has no per-function emission-size bound, so it can hand cranelift a function
; cranelift refuses.
;
; FIX (hypothesis): bound the emitted function size — e.g. emit a large constant list literal via a separate
; helper/data-segment initializer (not element-by-element inlined into the consumer), or split an oversized
; function body, so no single function exceeds cranelift's branch range. (Alternatively, a compile-time cap
; with a clear diagnostic — "this literal is too large to inline; bind it or build it at runtime" — beats a
; cranelift assertion at run time.)
;
; SEVERITY: runtime CRASH (cdz-run aborts with a cranelift assertion) on a VALID, wasm-valid program. Niche
; (a ≥20k-element inlined LITERAL is unusual — real large lists are built at runtime, which works), but a
; large literal is a legitimate program and the failure mode (a cranelift internal assertion, no Cadenza
; diagnostic) is opaque. Grades Fail/Todo (the case cannot run). Not an rcdzc invalid-wasm bug — an
; emission-size-vs-JIT-limit issue; a compile-time size cap with a diagnostic would make it a clean reject.

(case "a large inlined list literal consumed by a fold runs (or rejects cleanly, not a JIT crash)"
  (doc    "A ~20k-element inlined `(list 2 2 … 2)` summed by a tail fold emits a single wasm function
           (~290KB) whose internal branch offset exceeds cranelift's max_pos_range, so cdz-run PANICS
           (cranelift buffer.rs:1395 assertion) rather than running. The wasm VALIDATES; a 50k literal used
           trivially (len) is fine; a runtime-BUILT 30k list + fold is fine (→60000) — only a large INLINED
           literal + its consumer in one oversized function crashes. Fix: bound emitted function size (emit
           a large literal via a helper/data segment, or split the function) so no function exceeds
           cranelift's branch range — or cap it at compile time with a clear diagnostic. Expected: the sum
           (40000 for 20000×2), or a clean compile-time reject, never a cranelift assertion at run time.")
  (input  (do
            (def (sum (: xs (List Int64)) (: acc Int64))
              (match xs ((list) acc) ((list x .. rest) (sum rest (+ acc x)))))
            (def (main) (sum (list 2 2 2 2 2 2 2 2 2 2) 0))
            (export main)))
  (output (: 20 Int64)))

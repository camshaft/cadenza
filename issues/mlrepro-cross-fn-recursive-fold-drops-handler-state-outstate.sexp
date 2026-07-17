; mlrepro (2026-07-17, v-effects — filed at reviewer's request re: dbe6922fb item-3). The RECURSIVE analogue
; of the inlined helper-call out-state drop that was fixed in 53d95f103 (batch 161, RESOLVED). That fix
; threads an INLINED helper's state-advancing effect through the caller's continuation. This one is the
; SPECIALIZED (recursive) callee: a handler-STATE accumulator does NOT thread across a CROSS-FUNCTION
; recursive fold.
;
; SHAPE: run-ops recursively performs Prim.run per list element; the handler arm advances state s -> s+1 per
; perform. A perform in the handle body's CONTINUATION, AFTER run-ops returns, should see the accumulated
; count — but sees FRESH state 0. The specialized run-ops#ctx recursion advances state internally (the 3
; performs DO fire + thread WITHIN the recursion — a base-case-observed read returns 3 correctly), but the
; recursion's FINAL out-state is dropped at the specialization-return boundary, so it never reaches the
; caller's do-continuation.
;
; PINNED bisection (all under the same one-op counter handler):
;   - run-ops [1 2 3] with the count read in the BASE CASE (Prim.total inside the recursion tail) -> 3  ✓
;   - run-ops [1 2 3] then (Prim.run 0) AFTER it, in the handle body's do                          -> 0  ✗  EXPECTED 3
;
; ⚠ SEVERITY: COMPILES + RUNS to a WRONG VALUE (silent), not a decline. Guarded fleet-wide by the #[test]
; a_handler_state_accumulator_across_a_cross_function_fold_is_a_known_miscompile (asserts the wrong 0, so a
; regression to a DIFFERENT wrong shape / a leak fails loudly). NOT currently blocking a consumer:
; v-agent-harness's agent-kernel executor (the surfacing consumer) uses per-op RESULTS (each exec/http/log
; perform returns its own result — which folds CORRECTLY across the cross-fn fold), NOT a threaded
; handler-state accumulator. So this is QUEUED, not urgent.
;
; THE FIX (substantial, E3): thread the specialized recursive callee's FINAL out-state back to the caller's
; continuation — the recursive analogue of the inlined-helper fix. Likely reuses the multi-value tuple-return
; machinery (thread_returning_tuple / build_value_state_tuple) that already threads a self-call's out-state to
; a LATER SIBLING, but extended so the out-state escapes the specialized def back to the CALLER's do-tail.
; Promote when a real consumer needs handler-state-accumulator-across-a-cross-fn-fold, OR active effects work
; clears. Tracked in v-effects task #15 + memory queued-branch-outstate-lost-to-later-perform.
;
; EXPECTED: main() == 3 (three Prim.run performs advance state 0->1->2->3; the trailing Prim.run 0 sees 3).
; ACTUAL:   main() returns 0 (the trailing perform sees fresh state 0 — the fold's out-state was dropped).

(do
  (effect Prim (op run (-> Int64 Int64)))
  (def (run-ops (: ops (List Int64)))
    (match ops
      ((list h .. rest) (do (Prim.run h) (run-ops rest)))
      (_ 0)))
  (def (main)
    (handle Prim 0 ((run (tag) s (resume s (+ s 1))))
      (do (run-ops (list 1 2 3)) (Prim.run 0))))
  (export main))

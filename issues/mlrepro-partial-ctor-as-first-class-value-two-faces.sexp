;; BUG (2026-07-15, v-runtime — extends the v-inference-filed partial-ctor-export finding) — a
;; PARTIALLY-APPLIED CONSTRUCTOR held as a genuine FIRST-CLASS VALUE (not completed to full arity at
;; compile time) is not lowered to a working runtime closure. TWO faces, both a lowering gap:
;;
;; FACE A (the originally-filed one) — EXPORTED / returned-bare partial ctor:
;;   (do (type T (Mk Int64 Int64)) (def (main) (T.Mk 1)) (export main))
;;   → cdz: error: a closure export produced no lifted lambda (backend/wasm/mod.rs) — a LEAKY INTERNAL
;;     error. cdz check PASSES; `cdz type main` = (-> Int64 T).
;;
;; FACE B (found while fixing A — a WORSE, pre-existing miscompile) — partial ctor as a COMPOUND ELEMENT
;; (tuple element / list element), projected and applied:
;;   (do (type T (Mk Int64 Int64))
;;       (def (main) (let ((p (tuple (T.Mk 10) 0))) (match ((. p 0) 5) ((T.Mk a b) (+ a b)))))
;;       (export main))
;;   → cdz-run: invalid component: failed to compile: wasm[0]::function[3]  — INVALID WASM (a miscompile,
;;     not a decline). Same with `(list (T.Mk 10))` + List.at retrieval. Expected 15.
;;
;; CONTROL (works) — the SAME shape with an EXPLICIT lambda instead of the partial ctor:
;;   (list (fn ((: y Int64)) (T.Mk 10 y)))  … applied → 15 CORRECT.
;; So a source lambda constructing the ctor lifts + flows through a compound fine; only the
;; partially-applied CTOR value miscompiles. The two should be identical.
;;
;; ALSO WORKS (completes at compile time, does NOT exercise the gap): a partial ctor that reaches FULL
;; arity via a ref/inline — `(let ((g (T.Mk 1))) (g x))`, `((mk) x)` — flattens to a flat SumNew through
;; `ctor_spine` (lower.rs ~1022) BEFORE any closure is needed. So the existing 05-compound-types partial-
;; ctor cases (4102/4115) pass without touching this gap.
;;
;; ROOT (lowering): a partial ctor spine that stops SHORT of arity needs a runtime ETA-CLOSURE
;; `(fn (__eta{k}..) (ctor supplied.. __eta{k}..))` — the partial analogue of eta_ctor_closure (lower.rs
;; ~8920, the full-arity bare-head expansion). ⚠ ATTEMPTED FIX (v-runtime, reverted): synthesizing that
;; closure by splicing the supplied arg nodes into the body + seeding remaining-param types lowered +
;; applied CORRECTLY for the direct/returned cases, but produced INVALID WASM (function[3]) exactly when
;; the closure flows through a COMPOUND (Face B) — the synthesized lifted lambda's functype/env differs
;; from a source lambda's in a way that breaks the projected-closure `call_indirect`. So the naive splice
;; is NOT sufficient; the synthesized closure must be byte-equivalent to the working explicit-lambda lift
;; (investigate: does the spliced supplied-arg node get its type seeded / captured correctly? compare the
;; lifted lambda + its `closure_call_types`/functype against the explicit `fn (y) (T.Mk 10 y)` case).
;; INTERIM: a clean `Reject::decline` for the short-arity spine (Face A → clean todo instead of the leaky
;; internal error) is safe, but does NOT fix Face B (the compound-element partial ctor takes a different
;; lowering path — lowering `(T.Mk 10)` AS a compound element — and still emits invalid wasm), so a decline
;; there is incomplete. The real fix is the correct eta-closure lift covering both faces.
;;
;; SEVERITY: Face B is a 🔴 MISCOMPILE (invalid wasm from a well-typed program); Face A is a leaky internal
;; error. Reachable from the idiomatic "list/table of partial constructors" (a parser combinator table, a
;; dispatch map of `Tag`-builders). Owned by v-runtime (lowering + closure-lift seam).
(do
  (type T (Mk Int64 Int64))
  (def (main)
    (let ((p (tuple (T.Mk 10) 0)))
      (match ((. p 0) 5) ((T.Mk a b) (+ a b)))))
  (export main))

; BREAKER-ADJACENT FINDING (routed by v-rust-backend, 2026-07-17) — a WASM-backend MISCOMPILE:
; matching a LIST ELEMENT that is a sum whose payload is a TUPLE traps 'unreachable' on an input
; that SHOULD match, instead of binding the tuple fields and taking the arm.
;
; VERIFIED on trunk 1c255812b (fresh build, store def9d173) by corpus-bugfix:
;   wasm run(3): TRAPS (wasm unreachable) — WRONG, should be 3+9 = 12
;   wasm run(0): 0 (correct — Nil arm / the _ fallthrough)
;   rust: DECLINES ('a nested list-element binder beyond a tuple projection is not rendered') — SOUND
;         (rust's sum-payload arm only handles the [Elem(i), Payload] terminal step; it declines the
;          deeper tuple-step rather than miscompiling — so this is a WASM-only wrong-behavior, not a
;          differential where both compile).
;
; NARROWED (isolates it to the list-element + tuple-payload COMBO):
;   A. direct (Pt (tuple a b)) match, NO list        -> wasm run(3) = 12  CORRECT
;   B. the SAME pattern as a LIST element            -> wasm run(3) TRAPS  <-- THE BUG
;   C. list element with SCALAR payload (Pt Int64)   -> wasm run(3) = 3   CORRECT (+ rust emits)
; So the break is specifically a sum-with-TUPLE-payload matched as a LIST ELEMENT — the wasm
; list-element sum-payload decision path (match-plan step [Elem(i), Payload, Elem(j)/TupleProj]).
; Scalar payload is fine; direct (non-list) tuple payload is fine.
;
; OWNER: wasm backend (rcdzc match-plan / list-element sum-payload lowering). NOT rust territory
; (rust soundly declines the deeper arm). v-rust-backend offers to take the DEEPER RUST arm
; ([Elem(i), Payload, Elem(j)] terminal) as a backlog slice ONCE the wasm fix + a corpus case land.
; Corpus 09-functions.sexp only pins the NON-list / scalar-payload faces — this list+tuple face is
; uncovered. Migrate this witness into spec/semantics when fixed.

(case "matching a list element that is a sum with a tuple payload binds the tuple fields"
  (doc "run(3): xs = (list (Pt (tuple 3 9))); the arm (list (Pt (tuple a b))) should bind a=3 b=9 -> 12.
        Currently TRAPS unreachable on wasm (the list-element sum-tuple-payload match-plan step is wrong).
        run(0): xs = (list (Nil)) -> the _ fallthrough -> 0 (correct).")
  (input (module m (type P (Pt (Tuple Int64 Int64)) (Nil))
    (def (build (: k Int64)) (if (< k 1) (list (Nil)) (list (Pt (tuple k 9)))))
    (def (f (: xs (List P))) (match xs ((list (Pt (tuple a b))) (+ a b)) (_ 0)))
    (def (run (: k Int64)) (f (build k))) (export run)))
  (call run 3)
  (output (: 12 Int64)))

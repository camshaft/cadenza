; adv-55 (breaker, 2026-08-02): wasm CSE hoists a REPEATED trapping subexpression out of a
; short-circuit `and`/`or` rhs — the shield is violated and a spurious trap fires.
;
; observed:  main(false, 0) TRAPS "integer divide by zero" on the wasm backend at EVERY opt
;            level O0..O3 (not the new O2 Core CSE — the always-on select.rs CSE).
; expected:  0 — the `and` lhs is false, so the rhs (and its divisions) must never run
;            (spec/capabilities/core-semantics.md#boolean-connectives-short-circuit; the
;            single-division corpus pin at 02-binding-and-control.sexp:1324 passes).
; delta:     the division appears TWICE. One division in the same position passes; the rust
;            backend passes both. Root-cause hypothesis: collect_dominating_frontier
;            (core_analysis.rs:148) special-cases only If/Match* as control flow — Core::And
;            descends via licm_children into BOTH operands, so the shielded rhs subexpr is
;            wrongly in the dominating frontier and CSE hoists it to the body root.
; or-twin:   (or (= x 0) (= (/ 100 x) (/ 100 x))) traps identically with x=0.
(case "adv-55 a repeated division in a short-circuit and's rhs stays shielded (CSE must not hoist past the connective)"
  (input  (do
            (def (main (: b Bool) (: d Int64))
              (if (and b (= (/ 10 d) (/ 10 d)))
                  1
                  0))
            (export main)))
  (call   main (: false Bool) (: 0 Int64)) (output (: 0 Int64))
  (call   main (: true Bool) (: 5 Int64)) (output (: 1 Int64)))

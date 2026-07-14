;; MISCOMPILE (2026-07-14): a PARAMETERIZED export returning a COMPOUND (tuple / record / sum) compiles
;; clean (`cdz check` passes; the component's WIT even shows `make: func(p0: s64) -> t`) but TRAPS at
;; run time — `cdz-run pt.wasm --arg 5` → "trap: expected 1 argument(s), got 0" (wasmtime's own arity
;; check). The export's parameter is NOT delivered to the resource-escape `make` at run time, so a
;; parameterized compound-return export cannot actually be CALLED with its argument.
;;
;; CONTROL: a NULLARY compound-return export works perfectly (see below / repros without a param).
;; EVERY compound return type triggers it — verified: tuple, record, sum, recursive sum, List, Set, Map
;; (nullary Set/Map escape fine → `(: ((. Set of) (list 1 2 3)) (Set Int64))`, but a param'd one traps).
;; So it is the escape `make`'s param-forwarding, independent of the compound type.
;; ⚠ There IS a seed test `a_parameterized_compound_return_export_compiles_via_the_resource_escape`
;; (rcdzc tests.rs) but it only asserts `compile_component(...).is_ok()` — it never RUNS the component
;; with an argument, so the runtime arg-forwarding gap is untested. This is the coverage hole.
(do
  (def (pair (: n Int64)) (tuple n (+ n 1)))
  (def (main (: n Int64)) (pair n))
  (export main))

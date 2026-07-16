; NESTED-LET face of the try-shortcircuit-drops-trapping-outer-init family (v-wasm-opt, 2026-07-16).
; The same-let form is pinned at 23-try-operator.sexp:146 (PR #409): a constant-failure `?`/`try`
; MUST NOT drop a trapping EARLIER binding — `a` is before the `?` cut and referenced, so the whole
; expression must trap CDZ0304 (÷0), NOT fold to None.
;
; This is the NESTED-LET shape: `a` is bound in an OUTER let, `x` (the failing `?`) in an INNER let,
; and `a` is referenced after. The same rule applies — the outer trapping init is sequenced before the
; `?` cut, so it must CDZ0304, not fold to None. v-wasm-opt reported the nested form folds to None
; (only a CDZ0305 warning) — a drop the :146 same-let pin does not cover. REJECT-OR-TRAP, don't drop.
; (Original .cdz mis-mixed ML let..in with s-expr application → parse error; re-surfaced as .sexp
; matching the same-let pin idiom by corpus-bugfix.)
(module m
  (def (main)
    (let ((a (/ 1 0)))
      (let ((x (try (None unit))))
        (Some (+ a x)))))
  (export main))

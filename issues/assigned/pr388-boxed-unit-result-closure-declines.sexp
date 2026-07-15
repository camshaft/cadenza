;; DECLINE (backend/wasm/select.rs closure_type_index, confirmed by corpus-bugfix triage 2026-07-15,
;; trunk@979a36f27). A BOXED/runtime closure whose full application yields Unit DECLINES with "no matching
;; function type". Root (Copilot PR #388, comment 3590046340, select.rs:4527): closure_type_index does
;; `let rv = valtype_of(&result_ty)?` which returns None for Ty::Unit (lir.rs valtype_of: Unit => None),
;; so the whole program is declined — but the serializer builds closure_call_functypes that treat Unit as
;; a valid ZERO-RESULT functype. Eager decline of a valid call_indirect. FIX: match the zero-result
;; functype shape for a Unit result instead of declining (mirror how the serializer handles it).
;; NB a DIRECTLY-inlined apply (def (apply f x) (f x)) over a Unit closure already COMPILES; only the
;; BOXED/runtime-dispatched path (closure stored in a sum, applied after a match) hits closure_type_index.
(case "a boxed runtime closure returning Unit applies without declining"
  (doc    "A closure (-> Int64 Unit) boxed in a sum, extracted by match, then applied — the runtime
           call_indirect path. Must compile: the serializer already treats Unit as a zero-result
           functype; closure_type_index must not decline on valtype_of(Unit)=None.")
  (input (do
    (type Box (C (-> Int64 Unit)))
    (def (run (: b Box) (: x Int64)) (match b (((. Box C) f) (f x))))
    (def (ignore (: n Int64)) unit)
    (def (main) (do (run ((. Box C) ignore) 5) 42))
    (export main)))
  (output (: 42 Int64)))

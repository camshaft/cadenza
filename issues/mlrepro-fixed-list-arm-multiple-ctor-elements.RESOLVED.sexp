;; ✅ FIXED (2026-07-14, seed rcdzc `lower.rs`) — REGRESSION WITNESS. A LIST pattern arm may now contain
;; SEVERAL refutable (constructor) elements. This file (`((list (A.I x) (A.N y) c) …)` — two ctor elements)
;; now COMPILES; it used to decline "a list arm with more than one refutable constructor element is not
;; yet supported". FIX: the list-refutable-element desugar (`rewrite_list_ctor_arms`) generalized from ONE
;; ctor position to N — each ctor element gets a fresh binder, all their discriminant tests are ANDed into
;; the arm guard, and the body re-matches NEST (innermost holds the original body, so every ctor payload is
;; in scope). Corpus: `spec/semantics/05-compound-types.sexp` ("a list arm with two constructor elements
;; binds both payloads" + the fall-through-on-second-tag case, gate-verified value 3 / -1). Unit test
;; `a_list_pattern_must_be_linear_and_refutable_elements_decline` updated to assert the two-ctor arm
;; compiles. The port's `src/fold.cdz` now uses the natural `[Ast.Name(op), Ast.Int(x), Ast.Int(y)]` arm
;; (three ctor elements) — the bind-all-then-nested-match workaround is gone.
(do
  (type A (I Int64) (N String))
  (def (f (: xs (List A)))
    (match xs
      ((list (A.I x) (A.N y) c) (+ x 0))
      (_ 0)))
  (def (main) (f (list)))
  (export main))

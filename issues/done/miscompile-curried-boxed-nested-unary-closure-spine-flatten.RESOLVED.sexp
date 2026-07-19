;; MISCOMPILE — COMPILES-THEN-TRAPS (root-caused by v-property-testing/closures probe, confirmed by
;; corpus-bugfix 2026-07-15, trunk@d283d44cb). A BOXED nested-unary curried closure returning a closure
;; ((-> Int64 (-> Int64 Int64)) stored in a sum), applied curried ((f x) y), compiles to VALID wasm but
;; TRAPS 'indirect call type mismatch' at runtime.
;;
;; ROOT (lower.rs:995-1005): the lowering SPINE-FLATTENS a curried application ((f x) y) into ONE
;; Core::CallClosure{args:[x,y]}, assuming f lifts to a single 2-param function. That holds for (fn (n m) …)
;; SUGAR (one lifted lambda, arity 2 — works). But a NESTED-UNARY closure (fn (n) (fn (m) …)) is TWO
;; DISTINCT lifted lambdas: outer (env,n)->i32 (closure handle), inner (env,m)->i64. The flattened call
;; emits ONE call_indirect with a (env:i32,i64,i64)->i64 functype — the module VALIDATES (structural
;; typing) but NO lifted body implements that shape, so it traps.
;;
;; FIX DIRECTION: the flattening at lower.rs:995 must NOT flatten across a closure-returning-closure
;; boundary — when the head closure's application at arity K returns another closure (result still Ty::Fn
;; after peeling K args and no single lifted lambda has the flattened arity), emit SEPARATE call_indirects
;; (dispatch outer→intermediate handle, then dispatch that). OR closure_type_index detects a
;; no-matching-flattened-lift and falls back to per-arrow dispatch. (The Unit variants are already correctly
;; DECLINED by the lambda_is_nested_in_lambda guard from the Unit-param fix; only the non-Unit curried case
;; is this live miscompile.) TERRITORY: v-effects / closure lowering.
(case "a boxed nested-unary curried closure applies without trapping"
  (doc    "A closure of type (-> Int64 (-> Int64 Int64)) written nested-unary (fn (n) (fn (m) (+ n m))),
           boxed in a sum, extracted and applied curried ((f x) y). The two arrows are distinct lifted
           lambdas; spine-flattening the call into one 2-arg call_indirect emits a functype no lifted body
           implements → 'indirect call type mismatch' at runtime. Must return 7 (add 3 4).")
  (input (do
    (type Box (C (-> Int64 (-> Int64 Int64))))
    (def (run (: b Box) (: x Int64) (: y Int64)) (match b (((. Box C) f) ((f x) y))))
    (def (add (: n Int64)) (fn (m) (+ n m)))
    (def (main) (run ((. Box C) add) 3 4))
    (export main)))
  (output (: 7 Int64)))

;; RESOLVED 2026-07-15 (trunk@0c7d182c1, fix 38b5c71f2): a boxed nested-unary curried closure (-> A (-> B R)) no longer traps "indirect call type mismatch". Fix = flatten a directly-nested curried lambda to ONE multi-param lift in lower_lambda_value (value-side; closure ABI untouched) so it matches the flat (env,a,b)->r shape the caller spine-flattens to. Wasmtime regression test + 09-functions.sexp pin. Returns 7.

(case "a nested redundant type annotation is transparent to the value and type"
  (doc    "`(: (: n Int64) Int64)` — an annotation stacked on an annotation, both naming the operand's
           solved type. Annotations are constraints, not conversions, so the nest is fully transparent:
           the value flows through both layers unchanged (41+1 → 42). Pins that a doubled constraint is
           idempotent — a reader/desugar that consumed only the outer layer, or an inference pass that
           re-grounded the inner one as a fresh unknown, would reject or mistype a shape macro-expanded
           code produces routinely (a splice that re-annotates an already-annotated operand).")
  (input  (do
            (def (main (: n Int64))
              (+ (: (: n Int64) Int64) (: 1 Int64)))
            (export main)))
  (call   main (: 41 Int64)) (output (: 42 Int64)))

(case "a nested CONFLICTING type annotation is rejected"
  (doc    "The conflict face: `(: (: n Int64) Float64)` constrains the same operand to Int64 (inner,
           matching the param) and Float64 (outer) — irreconcilable, so it rejects CDZ0203 (a genuine
           type mismatch between the layers, on all targets uniformly). Pins that the outer layer of a
           nested annotation is a REAL constraint (not shadowed/dropped by the inner one) — the reject
           twin of the transparent redundant nest above.")
  (input  (do
            (def (main (: n Int64))
              (: (: n Int64) Float64))
            (export main)))
  (call   main (: 41 Int64))
  (error  CDZ0203))

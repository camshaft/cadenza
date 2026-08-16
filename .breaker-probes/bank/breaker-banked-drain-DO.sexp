(case "runtime Float32 addition rounds per-op to binary32, not accumulated at f64"
  (doc    "The double-rounding discriminator: e = 2^-25 + 2^-30 (built by exact f32 divisions of
           powers of two) is just over half an ulp of 1.0 — a PER-OP binary32 add rounds x+e up to
           1+ulp and the second +e rounds back DOWN to... the tie-to-even dance nets exactly 1.0
           per-op, while an implementation that accumulated at f64 and rounded ONCE lands on 1+ulp
           (1.0000001) ≠ x. The predicate is TRUE (1) only under faithful per-op binary32 rounding —
           python-verified both strategies produce different bits. Pins the fixed narrower mode the
           :2923 family asserts per-op, with an input that actually DISCRIMINATES the two rounding
           schedules (the family's operands are exactly-representable, so both schedules agree there).")
  (input  (do
            (def (main (: x Float32))
              (let ((e (+ (/ (: 1.0 Float32) 33554432.0) (/ (: 1.0 Float32) 1073741824.0))))
                (if (= (+ (+ x e) e) x) 1 0)))
            (export main)))
  (call   main (: 1.0 Float32)) (output (: 1 Int64)))

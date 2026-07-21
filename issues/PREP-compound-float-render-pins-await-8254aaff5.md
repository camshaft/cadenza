# PREP (corpus-bugfix): compound + large-significand float-render pins — HOLD until v-runtime 8254aaff5 lands

The float_leaf full-expansion convergence (v-runtime 8254aaff5, MR'd, ruling (a)) makes compound
KIND_FLOAT elements render the FULL exact expansion (matching scalar + rust). WHEN IT LANDS: rebuild,
gate all 3, append to baselines, send. These cases FAIL on wasm today (compound still shortest), so do
NOT insert into the corpus until the fix is on trunk.

Canonical full-expansion values (from rust run-rust, already correct on all backends today):
- tuple: (tuple 340282349999999991754788743781432688640.0 1.0)
- list:  (list 340282349999999991754788743781432688640.0)
- Option:(Some 340282349999999991754788743781432688640.0)
- f64::MAX scalar (309-digit / 128-byte-significand guard): 179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858368.0

DRAFT CASES (target file: 01-literals.sexp, after the scalar top-of-exponent pin ~L431):

(case "a large-magnitude float renders its full expansion as a TUPLE element (compound matches scalar)"
  (doc "Compound-element companion of the scalar large-magnitude pin: a Float64 whose shortest form differs
        from its exact value, as a tuple element, renders the FULL decimal expansion — the same form the
        scalar path and rust emit — guarding the wasm KIND_FLOAT (float_leaf) renderer against diverging to
        shortest form. Fixed by v-runtime 8254aaff5 (all 3 encode paths converged to full-expansion for whole floats).")
  (input  (do (def (main) (tuple 3.4028235e38 1.0)) (export main)))
  (output (: (tuple 340282349999999991754788743781432688640.0 1.0) (Tuple Float64 Float64))))

(case "a large-magnitude float renders its full expansion as a LIST element"
  (doc "The list-element face of the compound float render (same KIND_FLOAT heap path as the tuple case).")
  (input  (do (def (main) (list 3.4028235e38)) (export main)))
  (output (: (list 340282349999999991754788743781432688640.0) (List Float64))))

(case "a large-magnitude float renders its full expansion as an OPTION (sum) payload"
  (doc "The sum-payload face of the compound float render — a boxed KIND_FLOAT inside Option.Some.")
  (input  (do (def (main) (Some 3.4028235e38)) (export main)))
  (output (: (Some 340282349999999991754788743781432688640.0) (Option Float64))))

(case "a float at f64::MAX renders its full 309-digit expansion (large-significand codec)"
  (doc "Guards the large-significand KIND_FLOAT doc-codec path (a 309-digit / 128-byte significand) that the
        shortest form never exercised (shortest sig <= 17 digits) — the latent readback bug v-runtime found +
        hardened alongside 8254aaff5 (LEB siglen + arbitrary-length limb magnitude). f64::MAX = 1.7976931348623157e308.")
  (input  (do (def (main) 1.7976931348623157e308) (export main)))
  (output (: 179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858368.0 Float64)))

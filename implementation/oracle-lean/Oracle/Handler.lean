/-
The oracle's evaluation entry point.

L0.1 is the declining skeleton: it realizes no semantics yet, so every trial is answered
`Outcome.unsupported`. This is a first-class, sound verdict — the harness (L1.2) skips it rather
than counting it as a differential mismatch — which is exactly what lets coverage integrate on day
one and grow monotonically as later increments teach `handle` to decode (L0.2) and evaluate (L1.1).

The two stages of design §1.1 (`reduce` = const-fold the program to minimal form; `execute` = run a
trial against it) land in L1.1; today `handle` short-circuits before either.
-/
import Oracle.Frame

namespace Oracle

open Oracle.Frame

/-- The reason string every L0.1 verdict carries; kept as a constant so the self-test and future
regression tests can pin it. -/
def skeletonReason : String := "L0.1 skeleton: no semantics modeled yet"

/-- Answer a request. L0.1: one `unsupported` verdict per trial, no host calls. -/
def handle (r : Request) : Response :=
  r.trials.map fun _ => { outcome := .unsupported skeletonReason }

end Oracle

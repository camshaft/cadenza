# 2026-08-13 list-of-closures pipeline state (tick 1391)

- `loc1.sexp` — the handler state is a `(List (-> Int64 Int64))` PIPELINE: addmul/
  addadd push param-capturing stage closures, run folds the input through every
  stage in order (recursive List.at walk applying each). Two runs bracket a
  third stage push, so the second run composes 3 stages where the first ran 2.
  The 14b sibling passes a closure LIST as an op ARG indexed by state; here the
  closure list IS the threaded state, GROWN across dispatches with mixed captures
  (k from two different arms). PASS ×3 (1213339/1220360).

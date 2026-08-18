# Tuple-state post-resume toll (2026-08-18)

- `pyr8.sexp` — the post-resume toll composed with TUPLE-state
  destructuring: the arm matches (tuple v k) out of state, resumes with a
  new tuple, then the toll packs BOTH match binders (* 1000 (+ (* v 10)
  k)). The binders must survive ACROSS the suspend into the post-resume
  expression with their pre-resume values, while the resume itself carries
  the updated tuple — a binder-lifetime probe layered on the unwind-order
  law (pyr1). Interesting boundary note: match binders over the STATE fold
  fine post-resume (this case), while match binders over the RESUME VALUE
  decline (pyr7) — the gap is specific to binding resume's result, not to
  match binders in non-tail-resume arms generally. PASS x3 at 3cd560c66.

# bar1 — barometer with storm threshold (2026-08-17, tick 1693)

Attack: the sgn HELPER CALLED THREE TIMES per dispatch with the same compound
arg `(- p pressure)` — in the storm test's equality-against-negative-one, and
in BOTH branches' trend rebuild + the plain answer's offset. Storm gate =
boolean-AND-as-nested-if where the LEFT conjunct compares a helper result to
a NEGATIVE literal (`(= (sgn ...) -1)`) and the right is seed-shifted
(`(< p (+ 27 (% n 3)))`). Negative-literal equality over a helper return is
the fresh face.

Differential: same four readings; storm line 28 vs 27: the 27-reading storms
on n=10 (901) but is a plain fall (270) on n=0; the 26-reading storms on both
but with DIFFERENT counts (902 vs 901). Reads 2620 vs 2610.

Hand model: n=10 → 2809012929022620; n=0 → 2802702929012610 (mixed base;
first design's seed shifted only the INITIAL pressure — row-1-only weak pin,
redesigned to shift the THRESHOLD).

Pass ×3 wasm + rust + rust-async on trunk e4b91e88b.

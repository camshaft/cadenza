; ADVERSARIAL REPRO — spurious CDZ0101 "unbound name k" from the COMPILE backend (specialization/closure
; lowering), NOT the front-end. `cdz check` reports CLEAN (exit 0) — inference agrees `k` is bound — but
; `cdz compile` fails with `CDZ0101: unbound name k` (no source location, arises during a compile pass).
;
; MINIMAL TRIGGER (all four needed; drop any one and it compiles):
;   1. a TWO-variant sum whose FIRST variant carries a CLOSURE-typed payload `(Fn (-> Int64 Int64))`;
;   2. a helper `mk` that PICKS the variant via `if` (returns the closure variant in one arm, the other
;      variant in the other) — a direct `(Fn …)` construct (no `if`) COMPILES (see i.sexp below);
;   3. a consumer `run` that MATCHES the sum and APPLIES the extracted closure `(f arg)`;
;   4. the caller reuses ITS OWN parameter `k` in BOTH argument positions of the consuming call:
;      `(run (mk k) k)` — passing a literal for the 2nd arg (`(run (mk k) 4)`) COMPILES.
;
; DISCRIMINATORS (each COMPILES, isolating the trigger):
;   - single-variant sum (no `Const` arm)                        → compiles
;   - scalar payload `(V Int64)` instead of closure              → compiles
;   - direct `(Fn (fn …))` at the call (no `mk`/`if`)            → compiles + runs → 12
;   - `(run (mk k) 4)` (2nd arg a literal, not the reused `k`)   → compiles
;   - plain `(add (dbl k) k)` (k twice, no closure/sum)          → compiles
;   - a HEAP `(List Int64)` payload instead of the closure       → compiles (so it is CLOSURE-SPECIFIC,
;                                                                   NOT heap-payload-general)
;   - the BUILT-IN `Option` carrying the closure (`(Some (fn …))` via the if-helper, `(run (mk k) k)`)
;                                                                → ALSO FAILS (so it is not the custom
;                                                                   `Box` sum; any sum with a closure payload)
; A `let ((b (mk k))) (run b k)` binding does NOT work around it (still fails) — so it is not a
; two-arg-position syntactic issue; the reused `k` flowing into both the closure-producing helper AND the
; apply-arg is what the closure-specialization β-copy loses.
;
; SMELL: resonates with the resolve.rs same-name-ctor β-COPY fix `b42821408` (a specialization-inlined
; synth node losing a binding) — likely the closure-specialization path re-derives resolution on a β-copy
; of `mk`'s body inlined at the call site and drops `k` from scope. OWNER: v-inference (specialization /
; emit-type-selection), NOT pattern-matching (front-end resolution + match semantics are correct; `cdz
; check` is clean). Repro'd on trunk ff38db305 with the clean trunk cdz binary. Filed by v-patterns during
; a match bug-hunt.
;
; Expected once fixed: k=4 selects the `Fn` arm → `(* 4 3)` = 12; k=-1 selects `Const` → 77.

(case "a closure-payload sum picked by an if-helper, then consumed with the caller's param reused as the apply arg, compiles"
  (doc    "See header. `(run (mk k) k)` must compile: `mk k` returns `(Fn (fn (x) (* x 3)))` for k>0, and
           `run` matches it and applies the closure to `arg`=k → (* 4 3) = 12 at k=4. Currently emits a
           spurious CDZ0101 `unbound name k` from the compile backend while `cdz check` is clean.")
  (input  (do
            (type Box (Fn (-> Int64 Int64)) (Const Int64))
            (def (mk (: k Int64)) (if (> k 0) (Fn (fn (x) (* x 3))) (Const 77)))
            (def (run (: b Box) (: arg Int64)) (match b ((Fn f) (f arg)) ((Const c) c)))
            (def (main (: k Int64)) (run (mk k) k))
            (export main)))
  (call   main (: 4 Int64)) (output (: 12 Int64)))

; ===== PM triage (corpus-bugfix, 2026-07-20) — CONFIRMED live on trunk ff38db305 =====
; VERIFIED: ML repro (type Box (Fn (Int64->Int64))(Const Int64); mk via if; run match-applies; main(k)=
; run(mk(fn(x)=>x+k), k)) -> cdz check CLEAN, cdz compile FAILS "CDZ0101: unbound name k". Genuine backend
; specialization/β-copy miscompile (check clean => not front-end/pattern-semantics). ROUTED to v-inference
; (v-patterns already noted them; I reinforced with independent confirmation). Same neighborhood as their
; b42821408 same-name-ctor β-copy fix. NOT a fix agent (deep specialization, owner lane). corpus-bugfix to
; pin a compile-succeeds+correct-value witness once fixed.

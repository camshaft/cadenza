; Property-based testing — witnesses property-based-testing.md. Property testing is a HARNESS
; behavior, not a new language construct: a property is an ordinary Cadenza predicate over generated
; inputs, generation is a seeded deterministic function, and shrinking is a terminating search for a
; minimal failing input. The capability spec fixes four things and each is a plain evaluable program in
; the language the seed already realizes: (1) generation is seeded and reproducible (§Generation Is
; Seeded And Reproducible), (2) a generator for a bounded/refined type produces only in-range values
; (§Refinements Constrain Generation), (3) a failing property shrinks to a minimal failing input by a
; search that terminates (§Shrinking Converges To A Minimal Failing Input), and (4) a stated
; postcondition — here, a fold's permutation-invariance — is usable as the property oracle
; (§A Postcondition Is Usable As A Property, §Permutation Invariance Is A Property). The generator is a
; linear-congruential step `next s = s*A + C (mod 2^64)` built from the two's-complement WRAPPING
; arithmetic the numeric model already realizes (`Int64.wrapping-mul`/`wrapping-add`, 06-numeric-model);
; nothing here needs a new primitive. Results are (: <value> <Type>).
;
; The MULTIPLIER 6364136223846793005 and INCREMENT 1442695040888963407 are Knuth's MMIX LCG constants
; — an arbitrary well-mixing pair; the property cases do not depend on the specific stream, only that
; the SAME seed reproduces the SAME stream (determinism) and DISTINCT seeds diverge (coverage). A
; property that generates its inputs at RUN TIME (a boundary `seed` parameter, via `(call …)`) cannot
; constant-fold, so those cases exercise the emitted component's real generation-and-check machinery,
; not a compile-time reduction — the distinction the corpus draws between a nullary folding entry and a
; boundary-parameter entry (README §"(call …)").

; --- §Generation Is Seeded And Reproducible ---------------------------------------------------------
; "A property run MUST be reproducible from its recorded seed, producing the same inputs on every
; conforming run." The generator is a pure function of the seed, so re-running it on the same seed
; yields the identical value. These pin determinism (same seed → same value) at BOTH the compile-time
; folding path (a constant seed) and the runtime-boundary path (a `seed` parameter that cannot fold).

(case "the generator is a deterministic function of its seed (same seed, same value) — folds"
  (doc    "Witnesses property-based-testing.md §Generation Is Seeded And Reproducible: the LCG step
           `next s = s*A + C (mod 2^64)` is pure, so `(next 42)` produces one fixed value and
           `(= (next 42) (next 42))` = true. A constant seed folds at compile time.")
  (input  (let ((next (fn (s) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))))
            (= (next 42) (next 42))))
  (output (: true Bool)))

(case "the generator reproduces its stream from a runtime seed (same seed, same value) — runs"
  (doc    "The runtime companion: the seed arrives across the boundary as a parameter, so the generation
           runs as real instructions (no constant fold). Re-generating from the same seed reproduces the
           value: `(= (gen seed) (gen seed))` = true for any seed. This is the reproducibility a recorded
           seed buys a property run.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (main (: seed Int64)) (= (next seed) (next seed)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: -9223372036854775808 Int64)) (output (: true Bool)))

(case "distinct seeds produce distinct generated values (the stream covers, it is not constant)"
  (doc    "Witnesses §Generation Is Seeded And Reproducible read the other way: reproducibility is only
           useful if different seeds actually explore different inputs. `(next 1)` ≠ `(next 2)`, so the
           generator is injective enough to cover — a degenerate generator that ignored its seed would
           make `=` true here and defeat the point of a seed.")
  (input  (let ((next (fn (s) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))))
            (= (next 1) (next 2))))
  (output (: false Bool)))

(case "a two-step seed sequence advances (successive draws differ)"
  (doc    "A property run draws a SEQUENCE of inputs by threading the seed: draw one value, then advance
           the seed and draw again. Successive draws from one seed thread differ — `next(seed)` ≠
           `next(next(seed))` — so a multi-draw property visits distinct inputs. Pins the seed-threading
           idiom a generator loop uses, at the runtime boundary.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (main (: seed Int64)) (let ((a (next seed))) (= a (next a))))
              (export main)))
  (call   main (: 7 Int64)) (output (: false Bool)))

; --- §Refinements Constrain Generation --------------------------------------------------------------
; "A generator for a value of a refined type MUST produce only values satisfying that type's
; refinement." A bounded generator masks the raw stream into a range: `roll s = next s & (2^k − 1)`
; produces a value in `0 .. 2^k`. These pin that the bounded generator's output satisfies its range
; refinement for arbitrary runtime seeds — the harness-side realization of "the generator respects the
; refinement" ahead of a first-class refinement-type surface.

(case "a bounded generator produces only in-range values (0 <= roll < 64) for a runtime seed"
  (doc    "Witnesses §Refinements Constrain Generation: masking the LCG output with `& 63` constrains it
           to the refinement `0 <= x < 64`. For any runtime seed the drawn value satisfies the bound —
           `(if (>= v 0) (< v 64) false)` = true. This is a bounded/refined-type generator producing
           only admissible values, checked at the boundary so the mask runs (a mask on a constant would
           fold).")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (roll (: s Int64)) (& (next s) 63))
              (def (main (: seed Int64)) (let ((v (roll seed))) (if (>= v 0) (< v 64) false)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: 999 Int64)) (output (: true Bool))
  (call   main (: -1 Int64)) (output (: true Bool)))

; --- §Shrinking Converges To A Minimal Failing Input ------------------------------------------------
; "When a property fails, the harness MUST search for a smaller input that still fails", "The shrinking
; search MUST terminate rather than search unboundedly", and "The shrinking search MUST report a
; minimal failing input." A shrinker is a bounded upward search from the smallest candidate: scan
; 0,1,2,… while the property HOLDS; the first candidate at which it fails is the minimal failing input.
; Fuel bounds the scan so it terminates even when nothing in range fails.

(case "shrinking reports the minimal failing input"
  (doc    "Witnesses §Shrinking Converges To A Minimal Failing Input (and §…Reports A Minimal Failing
           Input). Property `p x = x < 4`; the failing inputs are `x >= 4`. Scanning candidates upward
           from 0 while `p` holds, the search stops at the FIRST failing candidate = 4, the SMALLEST
           input that violates the property — exactly the minimal counterexample a shrinker reports.
           `start` is the fuel; 100 is ample to reach 4.")
  (input  (do (def (p (: x Int64)) (< x 4))
              (def (search (: cand Int64) (: fuel Int64))
                (if (= fuel 0) cand (if (p cand) (search (+ cand 1) (- fuel 1)) cand)))
              (def (main (: start Int64)) (search 0 start))
              (export main)))
  (call   main (: 100 Int64)) (output (: 4 Int64)))

(case "the shrinking search terminates via its fuel bound rather than searching unboundedly"
  (doc    "Witnesses §…MUST terminate rather than search unboundedly. When the property holds for every
           candidate the scan reaches, the fuel bound forces termination: with `p x = x < 1000000` and
           fuel 5, the search exhausts its fuel at candidate 5 (never having found a failure) and returns
           5 rather than looping forever. Pins that the search is TOTAL — bounded, not open-ended.")
  (input  (do (def (p (: x Int64)) (< x 1000000))
              (def (search (: cand Int64) (: fuel Int64))
                (if (= fuel 0) cand (if (p cand) (search (+ cand 1) (- fuel 1)) cand)))
              (def (main (: fuel Int64)) (search 0 fuel))
              (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))

; --- §A Postcondition Is Usable As A Property / §Permutation Invariance Is A Property ---------------
; "A declared postcondition MUST be usable as a property oracle" and "A statement that a permutation of
; a fold's inputs produces a byte-identical result MUST be expressible as a property the generator
; exercises." The load-bearing first application (learnings/2026-07-04-fold-order-independence): a
; fold's order-independence is checkable by generated permutations. `sum` is a COMMUTATIVE fold, so its
; result is invariant under any permutation of its inputs — the property `sum(S) = sum(π(S))` holds;
; and the property has DISCRIMINATING POWER, since an order-DEPENDENT computation fails it.

(case "permutation invariance of a commutative fold holds — the property oracle passes"
  (doc    "Witnesses §Permutation Invariance Is A Property and §A Postcondition Is Usable As A Property.
           `sum` is commutative, so for the inputs {1,2,3} and a permutation {3,1,2} the fold agrees:
           `(= (sum [1 2 3]) (sum [3 1 2]))` = true. This is the fold order-independence property
           (learnings/2026-07-04-fold-order-independence-is-a-verified-property) exercised as a plain
           predicate over two orderings — the property-testing rung of discharging order-independence.")
  (input  (do (def (sum (: xs (List Int64)))
                (match xs ((list) 0) ((list h .. t) (+ h (sum t)))))
              (def (main) (= (sum (list 1 2 3)) (sum (list 3 1 2))))
              (export main)))
  (output (: true Bool)))

(case "permutation invariance of a commutative fold holds for generated runtime inputs"
  (doc    "The generator-exercised form: the three inputs are DRAWN from a runtime seed (so nothing
           folds), then the fold is compared across two orderings of the SAME drawn values —
           `(= (sum [a b c]) (sum [c a b]))` = true. This is `sum(S) = sum(π(S))` checked on inputs the
           generator produced, exactly §Permutation Invariance Is A Property's `the generator exercises`.
           The list-reordering is done at the CALL site (the elements are scalars); the fold reduces each
           ordering to a scalar the seed's runtime `=` compares.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (sum (: xs (List Int64)))
                (match xs ((list) 0) ((list h .. t) (+ h (sum t)))))
              (def (main (: seed Int64))
                (let ((a (next seed)))
                  (let ((b (next a)))
                    (let ((c (next b)))
                      (= (sum (list a b c)) (sum (list c a b)))))))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: 42 Int64)) (output (: true Bool)))

(case "the permutation-invariance property has discriminating power — an order-dependent fold fails it"
  (doc    "The counterpoint that makes the property meaningful: a computation that DEPENDS on order does
           NOT satisfy permutation invariance. `first-wins a b = a` (a latest-wins/first-arg projection)
           gives `first-wins 3 7 = 3` but `first-wins 7 3 = 7`, so `(= (first-wins a b) (first-wins b a))`
           = false. Pins that the property genuinely distinguishes order-independent from order-dependent
           behavior — a fold author engaging it gets a real check, not a tautology.")
  (input  (do (def (first-wins (: a Int64) (: b Int64)) a)
              (def (main (: a Int64) (: b Int64)) (= (first-wins a b) (first-wins b a)))
              (export main)))
  (call   main (: 3 Int64) (: 7 Int64)) (output (: false Bool)))

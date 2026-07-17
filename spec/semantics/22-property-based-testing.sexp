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

(case "a COMPOUND generated value is reproducible from its seed (a tuple draws identically twice)"
  (doc    "Witnesses §Generation Is Seeded And Reproducible for a generator that produces a COMPOUND value,
           not just a scalar: `gen` draws a `(Tuple Int64 Bool)` from the seed (a masked int + a PARITY
           bool), and `(= (gen seed) (gen seed))` = true — the whole compound value re-generates identically.
           This exercises RUNTIME structural equality on a heap value (a tuple), realized for
           tuple/record/set/map (list equality is a separate later increment). Runs at the boundary so the
           generation + the compound compare are real instructions, not a compile-time fold. The parity bool
           `(= (% … 2) 0)` also guards a fixed wasm codegen bug (a const-divisor rem feeding a Bool element
           of a `=`-compared tuple once aliased an i32 slot as i64) from the property-testing angle.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (gen (: s Int64)) (tuple (& (next s) 255) (= (% (& (next (next s)) 255) 2) 0)))
              (def (main (: seed Int64)) (= (gen seed) (gen seed)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: -7 Int64)) (output (: true Bool)))

(case "a generated value with a BIGINT leaf is reproducible (compound = walks the bignum)"
  (doc    "Witnesses §Generation Is Seeded And Reproducible for a compound value carrying an ARBITRARY-
           PRECISION leaf: `gen` draws a `(Tuple BigInt Bool)` — the int is lifted to a `BigInt` via
           `BigInt.of` — and `(= (gen seed) (gen seed))` = true. This exercises runtime structural equality
           over a compound whose element is a bignum (a heap value whose `=` walks the digit limbs, not a
           fixed-width scalar compare); admitting a `BigInt`/`Rational` leaf into the whole-compound `=`
           walk is what makes it hold. Runs at the boundary so the lift + the compound compare are real
           instructions, not a fold.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (gen (: s Int64)) (tuple ((. BigInt of) (& (next s) 255)) (= (% (& (next (next s)) 255) 2) 0)))
              (def (main (: seed Int64)) (= (gen seed) (gen seed)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: -7 Int64)) (output (: true Bool)))

(case "a generated value with a FLOAT leaf is reproducible (compound = over the canonical float byte form)"
  (doc    "Witnesses §Generation Is Seeded And Reproducible for a compound carrying a FLOAT leaf: `gen`
           draws a `(Tuple Float64 Bool)` from the seed (an integer-valued float via `Float64.of-int` + a
           threshold bool), and `(= (gen seed) (gen seed))` = true. This exercises runtime structural
           equality over a compound whose element is a FLOAT — a per-element compare by the canonical byte
           form (core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form), distinct from
           the bignum-limb walk (BIGINT/RATIONAL cases) and the fixed-width scalar compare (the Int/Bool
           tuple case). The generator uses `of-int` (never NaN), so each drawn float compares equal to its
           twin. Runs at the boundary — the lift + the compound float-compare are real instructions.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (gen (: s Int64)) (tuple ((. Float64 of-int) (& (next s) 255)) (< (& (next (next s)) 255) 128)))
              (def (main (: seed Int64)) (= (gen seed) (gen seed)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: -7 Int64)) (output (: true Bool)))

(case "a generated value with a SYMBOL leaf is reproducible (compound = walks the interned-name byte leaf)"
  (doc    "Witnesses §Generation Is Seeded And Reproducible for a compound carrying a SYMBOL leaf — the
           last leaf type in the compound-`=`-reproducibility family (Int/Bool, BigInt, Rational, Float
           already witnessed). `gen` draws a `(Tuple Symbol Bool)` — a constant interned name `(Symbol.of
           \"tag\")` beside a seed-derived threshold bool — and `(= (gen seed) (gen seed))` = true. A Symbol
           is a String byte-leaf at run time (the tagless heap has no distinct `Sym` shape; identity is the
           interned CONTENT, exactly like a String), so the compound `=` walks it by the same byte-leaf
           compare as a String element — distinct from the fixed-width scalar compare (Int/Bool), the
           bignum-limb walk (BigInt/Rational), and the canonical-byte float compare. Runs at the boundary,
           so the intern + the compound byte-leaf compare are real instructions, not a fold.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (gen (: s Int64)) (tuple ((. Symbol of) "tag") (< (& (next s) 255) 128)))
              (def (main (: seed Int64)) (= (gen seed) (gen seed)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: -7 Int64)) (output (: true Bool)))

(case "a SYMBOL compound = has DISCRIMINATING power (two runtime-selected interned names separate)"
  (doc    "The counterpart that makes the Symbol-leaf compare meaningful: the compound `=` must SEPARATE
           genuinely different interned names, not just equate identical ones. To keep the compare a real
           RUNTIME byte-leaf walk (a literal `(= (Symbol.of \"a\") (Symbol.of \"b\"))` would CONST-FOLD to
           false at compile time, witnessing nothing), the symbol is SELECTED at runtime from a seed-derived
           bool: `pick b = if b then (Symbol.of \"alpha\") else (Symbol.of \"beta\")`. Then a tuple carrying
           `(pick x)` versus one carrying `(pick (not x))` holds two DIFFERENT symbols, so `=` is false; and
           a tuple carrying `(pick x)` twice holds the SAME symbol, so `=` is true. The `n=0` vs `n=1` calls
           below drive `x` to both branches. Pins that the Symbol compound `=` has power in BOTH directions
           on a value inference cannot pre-decide.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (pick (: b Bool)) (if b ((. Symbol of) "alpha") ((. Symbol of) "beta")))
              (def (differ (: seed Int64)) (let ((x (< (& (next seed) 255) 128))) (= (tuple (pick x) 0) (tuple (pick (not x)) 0))))
              (def (same (: seed Int64)) (let ((x (< (& (next seed) 255) 128))) (= (tuple (pick x) 0) (tuple (pick x) 0))))
              (def (main (: seed Int64)) (if (differ seed) false (same seed)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: -7 Int64)) (output (: true Bool)))

(case "a generated RECORD value is reproducible from its seed (compound = walks the record fields)"
  (doc    "Witnesses §Generation Is Seeded And Reproducible for a generator that produces a RECORD — the
           other named product container beside the tuple. The doc of the tuple case promises structural
           equality is realized for `tuple/record/set/map`; the tuple, set (via the CRDT cases), and map
           (below) containers are witnessed — this pins the RECORD one. `gen` draws a `(record (x Int64)
           (y Bool))` from the seed (a masked int field + a threshold bool field), and `(= (gen seed) (gen
           seed))` = true — the whole record re-generates identically and the compound `=` walks it FIELD
           by field (a named-field product walk, distinct from the positional tuple walk). Runs at the
           boundary so the generation + the record field-walk compare are real instructions, not a fold.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (gen (: s Int64)) (record (x (& (next s) 255)) (y (< (& (next (next s)) 255) 128))))
              (def (main (: seed Int64)) (= (gen seed) (gen seed)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: -7 Int64)) (output (: true Bool)))

(case "a RATIONAL leaf in a compound compares by CANONICAL form (2n/4 = n/2 inside a tuple)"
  (doc    "Deepens the compound-`=`-over-a-bignum-leaf witness with RATIONAL's distinguishing feature:
           equality is by NORMALIZED (reduced) form, not by the stored numerator/denominator. Two tuples
           whose `Rational` element is written differently but denotes the SAME value — `2n/4` and `n/2` —
           compare EQUAL, because `Rational.of` reduces to lowest terms and the compound `=` walks that
           canonical form. This is a discriminating property (it would FAIL if `=` compared raw num/den):
           `(= (tuple (Rational.of 2n 4) …) (tuple (Rational.of n 2) …))` = true. Runs at the boundary.")
  (input  (do (def (main (: n Int64))
                (= (tuple ((. Rational of) (Int64.wrapping-mul n 2) 4) true)
                   (tuple ((. Rational of) n 2) true)))
              (export main)))
  (call   main (: 1 Int64)) (output (: true Bool))
  (call   main (: 3 Int64)) (output (: true Bool)))

(case "a RATIONAL compound = has DISCRIMINATING power (1/2 ≠ 1/3, but 0/2 = 0/3)"
  (doc    "The counterpart to the canonical-form case: normalizing equality must still DISTINGUISH genuinely
           different values, or it would be vacuous. `1/2` and `1/3` are different rationals, so a tuple
           carrying each compares NOT-equal — `(= (tuple (Rational.of n 2) …) (tuple (Rational.of n 3) …))`
           = false at `n=1`. But `0/2` and `0/3` both reduce to `0`, so at `n=0` the same expression is
           true — the reduction equates the zeros. Pins that the Rational compound `=` has power in BOTH
           directions (equates equal values, separates unequal ones).")
  (input  (do (def (main (: n Int64))
                (= (tuple ((. Rational of) n 2) true) (tuple ((. Rational of) n 3) true)))
              (export main)))
  (call   main (: 1 Int64)) (output (: false Bool))
  (call   main (: 0 Int64)) (output (: true Bool)))

(case "generated FLOAT values are totally ordered (a trichotomy property over two generated floats)"
  (doc    "A property over GENERATED floats using the runtime float ordering: `of` draws a float from the
           seed (an integer-valued float via `Float64.of-int`, which never produces NaN), so for any two
           generated floats exactly one of `<`, `=`, `>` holds — `(if (< x y) true (if (= x y) true (< y x)))`
           = true. This is trichotomy, the total-order law that would FAIL for NaN (all comparisons false);
           the generator producing only non-NaN floats is what makes it hold. Exercises the runtime float
           ordering (`<`) over the compiler's float generator at the boundary, not a fold.")
  (input  (do (def (of (: n Int64)) ((. Float64 of-int) n))
              (def (main (: a Int64) (: b Int64))
                (let ((x (of a)))
                  (let ((y (of b)))
                    (if (< x y) true (if (= x y) true (< y x))))))
              (export main)))
  (call   main (: 3 Int64) (: 7 Int64)) (output (: true Bool))
  (call   main (: 5 Int64) (: 5 Int64)) (output (: true Bool))
  (call   main (: 9 Int64) (: -2 Int64)) (output (: true Bool)))

(case "the int-to-float generator is order-preserving (a < b implies of a <= of b — NON-decreasing)"
  (doc    "A cross-type ordering property tying the integer generator to its FLOAT image: `Float64.of-int`
           is monotonic NON-DECREASING (the IEEE i64->f64 round-to-nearest), so whenever `a < b` as
           integers, `of(a) <= of(b)` as floats. It is NOT strictly increasing: two DISTINCT i64 beyond
           2^53 round to the SAME f64 (the mantissa runs out of bits), so `of(a) < of(b)` is FALSE there —
           `of(2^53) == of(2^53 + 1)`. The correct invariant is `<=`, and the last call PINS exactly that
           boundary (a<b holds but the floats are EQUAL, so a strict `<` would wrongly report false — this
           case would be a vacuous false-green if it only tested small ints). Distinct from trichotomy
           (a within-float total order): this pins the generator's Int->Float mapping preserves order
           NON-strictly, exercising the integer `<` and the runtime float `<=` at the boundary. Holds for
           negatives (of-int is signed) and across the 2^53 rounding threshold.")
  (input  (do (def (of (: n Int64)) ((. Float64 of-int) n))
              (def (main (: a Int64) (: b Int64))
                (if (< a b) (<= (of a) (of b)) true))
              (export main)))
  (call   main (: 3 Int64) (: 7 Int64)) (output (: true Bool))
  (call   main (: 9 Int64) (: 2 Int64)) (output (: true Bool))
  (call   main (: -8 Int64) (: -2 Int64)) (output (: true Bool))
  (call   main (: 9007199254740992 Int64) (: 9007199254740993 Int64)) (output (: true Bool)))

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

; --- §Permutation Invariance Is A Property, over a CRDT-style SET MERGE ------------------------------
; The permutation-invariance cases above reduce a list to a SCALAR (sum) before comparing, because the
; seed's runtime `=` over two heap LISTS is not yet realized. A `Set` sidesteps that entirely: a set's
; equality is ORDER-INDEPENDENT by construction (19-sets.sexp §order-independent equality), so a set
; merge is the natural first-class witness of a commutative-convergence property — exactly the
; CRDT-style grow-only-set merge the fold-order-independence learning names
; (learnings/2026-07-04-fold-order-independence-is-a-verified-property). These check the DEFINING
; convergence laws — commutativity, idempotence, associativity — over inputs the GENERATOR produced at
; run time, so the set built from a generated stream is compared, not a constant that would fold. A
; grow-only set whose merge is `Set.union` converges regardless of the order/duplication with which
; elements arrive — the property a property-testing run exercises before any proof effort.

(case "a set built from generated values is equal regardless of insertion order (order-independent equality on generated inputs)"
  (doc    "Witnesses §Permutation Invariance Is A Property directly on a set: a set drawn from a runtime
           seed in one order equals the SAME elements inserted in a PERMUTED order —
           `(= (Set.of [a b c]) (Set.of [c a b]))` = true. Unlike the list-sum case this needs no scalar
           reduction: set equality is order-independent by construction, so the set IS the
           permutation-invariant value. The elements are drawn from a runtime seed (nothing folds).")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (main (: seed Int64))
                (let ((a (next seed)))
                  (let ((b (next a)))
                    (let ((c (next b)))
                      (= (Set.of (list a b c)) (Set.of (list c a b)))))))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: 777 Int64)) (output (: true Bool)))

(case "set union is commutative on generated inputs (a grow-only-set CRDT merge converges)"
  (doc    "The convergence law a grow-only-set CRDT relies on: `A ∪ B = B ∪ A`, so two replicas that
           merge each other's states in EITHER order reach the same set. Checked on two sets drawn from a
           generated stream — `(= (union (Set.of [a b]) (Set.of [c d])) (union (Set.of [c d]) (Set.of
           [a b])))` = true. This is the property-testing rung of discharging order-independence for a set
           merge (learnings/2026-07-04-fold-order-independence), exercised on generated inputs.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (main (: seed Int64))
                (let ((a (next seed)))
                  (let ((b (next a)))
                    (let ((c (next b)))
                      (let ((d (next c)))
                        (= (Set.union (Set.of (list a b)) (Set.of (list c d)))
                           (Set.union (Set.of (list c d)) (Set.of (list a b)))))))))
              (export main)))
  (call   main (: 777 Int64)) (output (: true Bool)))

(case "set union is idempotent on generated inputs (re-delivering a set changes nothing)"
  (doc    "The other CRDT-merge law: `A ∪ A = A`, so at-least-once / duplicate delivery of the same state
           does not change the merged set — `(= (union s s) s)` = true for a set drawn from a generated
           seed. Commutativity + idempotence (+ associativity below) are exactly the convergence
           properties order-independence requires, checked by generated inputs.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (main (: seed Int64))
                (let ((a (next seed)))
                  (let ((b (next a)))
                    (let ((s1 (Set.of (list a b))))
                      (= (Set.union s1 s1) s1)))))
              (export main)))
  (call   main (: 42 Int64)) (output (: true Bool)))

(case "set union is associative on generated inputs (the order merges are grouped does not matter)"
  (doc    "The third convergence law: `(A ∪ B) ∪ C = A ∪ (B ∪ C)`, so replicas that combine partial
           merges in different groupings still converge. Checked on three singleton sets drawn from a
           generated seed. Together with commutativity and idempotence this is the full commutative-
           monoid-with-idempotence structure a grow-only set has — the property a CRDT convergence check
           exercises over generated event sets.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (main (: seed Int64))
                (let ((a (next seed)))
                  (let ((b (next a)))
                    (let ((c (next b)))
                      (let ((sa (Set.of (list a))))
                        (let ((sb (Set.of (list b))))
                          (let ((sc (Set.of (list c))))
                            (= (Set.union (Set.union sa sb) sc)
                               (Set.union sa (Set.union sb sc))))))))))
              (export main)))
  (call   main (: 314159 Int64)) (output (: true Bool)))

(case "the generated-set convergence property has discriminating power — two different generated sets are NOT equal"
  (doc    "The counterpoint keeping the set-convergence cases honest: distinct generated values build
           DISTINCT sets, so `(= (Set.of [a]) (Set.of [b]))` = false when `a ≠ b`. Pins that set equality
           is real content equality — a degenerate `=` that called everything equal would make the
           convergence cases vacuously pass, and this case would catch it.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (main (: seed Int64))
                (let ((a (next seed)))
                  (let ((b (next a)))
                    (= (Set.of (list a)) (Set.of (list b))))))
              (export main)))
  (call   main (: 5 Int64)) (output (: false Bool)))

(case "a MAP built from generated pairs is equal regardless of insertion order (the Map analogue of set convergence)"
  (doc    "Witnesses §Permutation Invariance Is A Property on a MAP — the key/value container the tuple
           case's doc names alongside tuple/record/set. Like a set, a map's equality is INSERTION-ORDER-
           INDEPENDENT by construction (the CHAMP canonicalizes, 19-maps): the same two generated key/value
           pairs inserted in one order equal the same pairs inserted in the REVERSED order — `(= (insert
           (insert empty a 1) b 2) (insert (insert empty b 2) a 1))` = true. This is the map analogue of the
           order-independent-set-equality case, and it needs no scalar reduction: the map `=` compares the
           canonical entry set directly. Both keys are DRAWN from the seed at run time, so nothing folds —
           the two maps are built and compared as real instructions. (`a` and `b` are masked to distinct
           bytes only incidentally; even were they equal the second insert would overwrite consistently on
           both sides, so the equality still holds — order-independence is the load-bearing property.)")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (main (: seed Int64))
                (let ((a (& (next seed) 255)))
                  (let ((b (& (next (next seed)) 255)))
                    (= (Map.insert (Map.insert (Map.empty) a 1) b 2)
                       (Map.insert (Map.insert (Map.empty) b 2) a 1)))))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: -7 Int64)) (output (: true Bool)))

; --- §Refinements Constrain Generation, by REJECTION SAMPLING ---------------------------------------
; The masked-generator case above constrains generation by CONSTRUCTION (a bitmask can only produce an
; in-range value). The general way a harness makes "the generator produces only values satisfying the
; refinement" hold for a predicate a mask cannot express is REJECTION SAMPLING: draw from the seed
; stream, keep the draw if it satisfies the refinement, otherwise ADVANCE the seed and draw again —
; bounded by fuel so the search is TOTAL (the same fuel discipline the shrinking search uses). These pin
; that a rejection-sampled draw satisfies its refinement, and that the loop terminates on fuel
; exhaustion rather than searching unboundedly (a refinement no draw in range satisfies must not hang).

(case "rejection sampling draws until the refinement holds — an only-even generator produces an even value"
  (doc    "Witnesses §Refinements Constrain Generation for a predicate a bitmask cannot directly impose:
           `draw-even` advances the seed until an EVEN value appears, then returns it. The result's low
           bit is 0 for any seed — `(& (draw-even seed 50) 1)` = 0 — so the generator produces only values
           satisfying the `even` refinement. Fuel-bounded (50 draws) so the loop is total. Runs at the
           boundary (the seed is a parameter), so the search executes rather than folding.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (draw-even (: s Int64) (: fuel Int64))
                (let ((v (next s)))
                  (if (= (& v 1) 0) v (if (= fuel 0) v (draw-even v (- fuel 1))))))
              (def (main (: seed Int64)) (& (draw-even seed 50) 1))
              (export main)))
  (call   main (: 12345 Int64)) (output (: 0 Int64))
  (call   main (: 999 Int64)) (output (: 0 Int64)))

(case "rejection sampling into a narrow range refinement produces only in-range values"
  (doc    "Rejection into a range the mask alone cannot give: mask to `0..16` then REJECT anything `>= 10`,
           re-drawing until the value lands in `0..10`. The returned value satisfies `0 <= v < 10` for any
           seed — `(if (>= v 0) (< v 10) false)` = true. This is the harness-side realization of
           §Refinements Constrain Generation for an arbitrary decidable refinement, fuel-bounded (100
           draws).")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (draw (: s Int64) (: fuel Int64))
                (let ((v (& (next s) 15)))
                  (if (< v 10) v (if (= fuel 0) 0 (draw (next s) (- fuel 1))))))
              (def (main (: seed Int64)) (let ((v (draw seed 100))) (if (>= v 0) (< v 10) false)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool))
  (call   main (: 42 Int64)) (output (: true Bool))
  (call   main (: 7 Int64)) (output (: true Bool)))

(case "rejection sampling terminates on fuel exhaustion when the refinement is unsatisfiable"
  (doc    "The termination guarantee: a refinement NO draw can satisfy (`v < 0` for a non-negative masked
           draw) must not hang. The fuel bound fires and the search returns its fallback (-1) after 5
           draws rather than looping forever — the total-search discipline §Shrinking …MUST terminate
           applies equally to a rejection-sampling generator. Pins that a mis-stated refinement fails
           closed (bounded) rather than diverging.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (draw (: s Int64) (: fuel Int64))
                (let ((v (& (next s) 15)))
                  (if (< v 0) v (if (= fuel 0) -1 (draw (next s) (- fuel 1))))))
              (def (main (: seed Int64)) (draw seed 5))
              (export main)))
  (call   main (: 12345 Int64)) (output (: -1 Int64)))

; --- A Bool generator (the enum-shaped generator) ---------------------------------------------------
; Generation is not only over integers: a property over a Bool (or a small enum) needs a generator for
; that type. The seed's low bit is a Bool generator — `gen-bool s = (next s & 1) = 0`. These pin it is
; reproducible (same seed → same Bool, §Generation Is Seeded And Reproducible) and covers both values
; across seeds (not a constant), the two properties a usable generator for a finite type must have.

(case "a Bool generator derived from the seed is reproducible (same seed, same Bool)"
  (doc    "Witnesses §Generation Is Seeded And Reproducible for a Bool-typed generator: `gen-bool` reads
           the low bit of the LCG output, so `(= (gen-bool seed) (gen-bool seed))` = true — the generated
           Bool is a deterministic function of the seed, exactly as the integer generator is. This is the
           generator a property quantified over a Bool draws from.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (gen-bool (: s Int64)) (= (& (next s) 1) 0))
              (def (main (: seed Int64)) (= (gen-bool seed) (gen-bool seed)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: true Bool)))

(case "the Bool generator covers both values across seeds (it is not a constant)"
  (doc    "The coverage counterpart: two seeds whose LCG low bits differ generate DIFFERENT Bools, so
           `(= (gen-bool s1) (gen-bool s2))` = false — the generator explores both `true` and `false`
           rather than pinning one. A degenerate Bool generator that ignored its seed would make this
           true and never test the `false` branch of a property; this case pins that it does not.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (gen-bool (: s Int64)) (= (& (next s) 1) 0))
              (def (main (: s1 Int64) (: s2 Int64)) (= (gen-bool s1) (gen-bool s2)))
              (export main)))
  (call   main (: 1 Int64) (: 2 Int64)) (output (: false Bool)))

; --- §Shrinking Converges To A Minimal Failing Input (end-to-end: find THEN shrink) ------------------
; The two spec halves — a property RUN that surfaces a failing input, and a SHRINK that minimizes it —
; composed into the full harness loop. `find` draws byte-masked values from the seed stream until one
; VIOLATES the property `p x = x < 50` (i.e. the first `x >= 50` the stream produces), fuel-bounded so
; the search is total; `shrink` then walks UP from 0 to the SMALLEST value that still violates `p`
; (again fuel-bounded), which is exactly 50 regardless of which counterexample `find` surfaced. This is
; the §Shrinking Converges guarantee end-to-end: a run finds SOME failing input, and shrinking converges
; to the minimal one — independent of the seed that started it. Runs at the RUNTIME boundary (a `seed`
; parameter) so it exercises the emitted generate-check-shrink machinery, not a compile-time fold.

(case "end-to-end: a property run finds a failing input, then the shrinker converges to the minimal failing value"
  (doc    "Witnesses §Shrinking Converges To A Minimal Failing Input composed with the property RUN:
           `find` scans the seed stream for the first byte-masked draw that VIOLATES `p x = x < 50`
           (a value >= 50), fuel-bounded to 200 draws so it is total; `shrink` then searches UPWARD from
           0 for the smallest value still violating `p`, converging to 50. The minimal failing input (50)
           does NOT depend on WHICH counterexample `find` surfaced — every failing run shrinks to the
           same minimum — which is the defining property of a convergent shrinker. A runtime `seed`
           parameter keeps it off the folding path, so the emitted find→shrink loop actually runs.")
  (input  (do (def (next (: s Int64)) (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
              (def (p (: x Int64)) (< x 50))
              (def (find (: s Int64) (: n Int64))
                (if (= n 0) -1 (let ((v (& (next s) 255))) (if (p v) (find (next s) (- n 1)) v))))
              (def (shrink (: cand Int64) (: fuel Int64))
                (if (= fuel 0) cand (if (p cand) (shrink (+ cand 1) (- fuel 1)) cand)))
              (def (main (: seed Int64))
                (let ((cx (find seed 200)))
                  (if (>= cx 50) (shrink 0 100) -1)))
              (export main)))
  (call   main (: 12345 Int64)) (output (: 50 Int64)))

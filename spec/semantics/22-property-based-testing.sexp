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
(case
  "the generator is a deterministic function of its seed (same seed, same value) — folds"
  (doc
    "Witnesses property-based-testing.md §Generation Is Seeded And Reproducible: the LCG step
           `next s = s*A + C (mod 2^64)` is pure, so `(next 42)` produces one fixed value and
           `(= (next 42) (next 42))` = true. A constant seed folds at compile time.")
  (input
    (let
      ((next
          (fn
            (s)
            (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))))
      (= (next 42) (next 42))))
  (output (: true Bool)))

(case
  "the generator reproduces its stream from a runtime seed (same seed, same value) — runs"
  (doc
    "The runtime companion: the seed arrives across the boundary as a parameter, so the generation
           runs as real instructions (no constant fold). Re-generating from the same seed reproduces the
           value: `(= (gen seed) (gen seed))` = true for any seed. This is the reproducibility a recorded
           seed buys a property run.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (main (: seed Int64)) (= (next seed) (next seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -9223372036854775808 Int64))
  (output (: true Bool)))

(case
  "distinct seeds produce distinct generated values (the stream covers, it is not constant)"
  (doc
    "Witnesses §Generation Is Seeded And Reproducible read the other way: reproducibility is only
           useful if different seeds actually explore different inputs. `(next 1)` ≠ `(next 2)`, so the
           generator is injective enough to cover — a degenerate generator that ignored its seed would
           make `=` true here and defeat the point of a seed.")
  (input
    (let
      ((next
          (fn
            (s)
            (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))))
      (= (next 1) (next 2))))
  (output (: false Bool)))

(case
  "a two-step seed sequence advances (successive draws differ)"
  (doc
    "A property run draws a SEQUENCE of inputs by threading the seed: draw one value, then advance
           the seed and draw again. Successive draws from one seed thread differ — `next(seed)` ≠
           `next(next(seed))` — so a multi-draw property visits distinct inputs. Pins the seed-threading
           idiom a generator loop uses, at the runtime boundary.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (main (: seed Int64)) (let ((a (next seed))) (= a (next a))))
      (export main)))
  (call main (: 7 Int64))
  (output (: false Bool)))

(case
  "a COMPOUND generated value is reproducible from its seed (a tuple draws identically twice)"
  (doc
    "Witnesses §Generation Is Seeded And Reproducible for a generator that produces a COMPOUND value,
           not just a scalar: `gen` draws a `(Tuple Int64 Bool)` from the seed (a masked int + a PARITY
           bool), and `(= (gen seed) (gen seed))` = true — the whole compound value re-generates identically.
           This exercises RUNTIME structural equality on a heap value (a tuple), realized for
           tuple/record/set/map (list equality is a separate later increment). Runs at the boundary so the
           generation + the compound compare are real instructions, not a compile-time fold. The parity bool
           `(= (% … 2) 0)` also guards a fixed wasm codegen bug (a const-divisor rem feeding a Bool element
           of a `=`-compared tuple once aliased an i32 slot as i64) from the property-testing angle.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (gen (: s Int64)) #tuple((& (next s) 255) (= (% (& (next (next s)) 255) 2) 0)))
      (def (main (: seed Int64)) (= (gen seed) (gen seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

(case
  "a generated LIST value is reproducible from its seed AND distinct seeds give distinct lists (runtime list = walks the spine)"
  (doc
    "§Generation Is Seeded And Reproducible for a LIST value — the historically-blocked case (P4): a
           list `=` needs a runtime SPINE WALK (Ordering.of length + each element), which the runtime now
           realizes. `gen` draws a 3-element `(List Int64)` from the seed (three successive masked draws).
           REPRODUCIBILITY: `(= (gen a) (gen a))` = true — the same seed re-draws the identical list, the
           `=` walking the spine + every element. DISCRIMINATING power (so the list `=` is not vacuously
           true): two DIFFERENT seeds give DIFFERENT lists, `(gen a) ≠ (gen b)`. `main` returns 1 iff BOTH
           hold. Runs at the boundary so the draw + the spine walk are real instructions — NOT a compile-time
           fold (a LITERAL list `=` const-folds, which would falsely look 'unblocked'; the seed-drawn elements
           force the real runtime walk). This closes the last blocked §Generation-reproducibility container
           (list), joining tuple/record/set/map/sum + the BigInt/Rational/Float/Symbol leaves.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (gen (: s Int64))
        #list((& (next s) 255) (& (next (next s)) 255) (& (next (next (next s))) 255)))
      (def
        (main (: a Int64) (: b Int64))
        (if (= (gen a) (gen a)) (if (not (= (gen a) (gen b))) 1 0) 0))
      (export main)))
  (call main (: 12345 Int64) (: 999 Int64))
  (output (: 1 Int64))
  (call main (: -7 Int64) (: 42 Int64))
  (output (: 1 Int64)))

(case
  "a generated value with a BIGINT leaf is reproducible (compound = walks the bignum)"
  (doc
    "Witnesses §Generation Is Seeded And Reproducible for a compound value carrying an ARBITRARY-
           PRECISION leaf: `gen` draws a `(Tuple BigInt Bool)` — the int is lifted to a `BigInt` via
           `BigInt.of` — and `(= (gen seed) (gen seed))` = true. This exercises runtime structural equality
           over a compound whose element is a bignum (a heap value whose `=` walks the digit limbs, not a
           fixed-width scalar compare); admitting a `BigInt`/`Rational` leaf into the whole-compound `=`
           walk is what makes it hold. Runs at the boundary so the lift + the compound compare are real
           instructions, not a fold.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (gen (: s Int64))
        #tuple((BigInt.of (& (next s) 255)) (= (% (& (next (next s)) 255) 2) 0)))
      (def (main (: seed Int64)) (= (gen seed) (gen seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

(case
  "a generated value with a FLOAT leaf is reproducible (compound = over the canonical float byte form)"
  (doc
    "Witnesses §Generation Is Seeded And Reproducible for a compound carrying a FLOAT leaf: `gen`
           draws a `(Tuple Float64 Bool)` from the seed (an integer-valued float via `Float64.of-int` + a
           threshold bool), and `(= (gen seed) (gen seed))` = true. This exercises runtime structural
           equality over a compound whose element is a FLOAT — a per-element compare by the canonical byte
           form (core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form), distinct from
           the bignum-limb walk (BIGINT/RATIONAL cases) and the fixed-width scalar compare (the Int/Bool
           tuple case). The generator uses `of-int` (never NaN), so each drawn float compares equal to its
           twin. Runs at the boundary — the lift + the compound float-compare are real instructions.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (gen (: s Int64))
        #tuple((Float64.of-int (& (next s) 255)) (< (& (next (next s)) 255) 128)))
      (def (main (: seed Int64)) (= (gen seed) (gen seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

(case
  "a generated value with a SYMBOL leaf is reproducible (compound = walks the interned-name byte leaf)"
  (doc
    "Witnesses §Generation Is Seeded And Reproducible for a compound carrying a SYMBOL leaf — the
           last leaf type in the compound-`=`-reproducibility family (Int/Bool, BigInt, Rational, Float
           already witnessed). `gen` draws a `(Tuple Symbol Bool)` — a constant interned name `(Symbol.of
           \"tag\")` beside a seed-derived threshold bool — and `(= (gen seed) (gen seed))` = true. A Symbol
           is a String byte-leaf at run time (the tagless heap has no distinct `Sym` shape; identity is the
           interned CONTENT, exactly like a String), so the compound `=` walks it by the same byte-leaf
           compare as a String element — distinct from the fixed-width scalar compare (Int/Bool), the
           bignum-limb walk (BigInt/Rational), and the canonical-byte float compare. Runs at the boundary,
           so the intern + the compound byte-leaf compare are real instructions, not a fold.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (gen (: s Int64)) #tuple((Symbol.of "tag") (< (& (next s) 255) 128)))
      (def (main (: seed Int64)) (= (gen seed) (gen seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

(case
  "a SYMBOL compound = has DISCRIMINATING power (two runtime-selected interned names separate)"
  (doc
    "The counterpart that makes the Symbol-leaf compare meaningful: the compound `=` must SEPARATE
           genuinely different interned names, not just equate identical ones. To keep the compare a real
           RUNTIME byte-leaf walk (a literal `(= (Symbol.of \"a\") (Symbol.of \"b\"))` would CONST-FOLD to
           false at compile time, witnessing nothing), the symbol is SELECTED at runtime from a seed-derived
           bool: `pick b = if b then (Symbol.of \"alpha\") else (Symbol.of \"beta\")`. Then a tuple carrying
           `(pick x)` versus one carrying `(pick (not x))` holds two DIFFERENT symbols, so `=` is false; and
           a tuple carrying `(pick x)` twice holds the SAME symbol, so `=` is true. The `n=0` vs `n=1` calls
           below drive `x` to both branches. Pins that the Symbol compound `=` has power in BOTH directions
           on a value inference cannot pre-decide.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (pick (: b Bool)) (if b (Symbol.of "alpha") (Symbol.of "beta")))
      (def
        (differ (: seed Int64))
        (let ((x (< (& (next seed) 255) 128))) (= #tuple((pick x) 0) #tuple((pick (not x)) 0))))
      (def
        (same (: seed Int64))
        (let ((x (< (& (next seed) 255) 128))) (= #tuple((pick x) 0) #tuple((pick x) 0))))
      (def (main (: seed Int64)) (if (differ seed) false (same seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

(case
  "a generated SUM value is reproducible from its seed (compound = walks the variant tag and its payload)"
  (doc
    "Witnesses §Generation Is Seeded And Reproducible for a generator that produces a user-declared
           SUM value — the tagged-union container, the last GenTy shape beside tuple/record/set/map. `gen`
           draws a `Result` from the seed: a seed-derived bool picks the CONSTRUCTOR (`Ok` vs `Err`) and a
           masked int is its payload, so `(= (gen seed) (gen seed))` = true — the whole tagged value
           (variant tag + payload) re-generates identically and the compound `=` walks BOTH the discriminant
           and the carried Int64. This is distinct from the product containers (tuple/record) — a sum's `=`
           must first agree on the tag, then compare the payload. Runs at the boundary so the constructor
           selection + the tagged compare are real instructions, not a compile-time fold.")
  (input
    (do
      (type Result (Ok Int64) (Err Int64))
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (gen (: s Int64))
        (if (= (& (next s) 1) 0) (Ok (& (next (next s)) 255)) (Err (& (next (next s)) 255))))
      (def (main (: seed Int64)) (= (gen seed) (gen seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

(case
  "a SUM compound = has DISCRIMINATING power (a runtime-selected constructor separates two tagged values)"
  (doc
    "The counterpart that makes the sum-value compare meaningful: the compound `=` must SEPARATE values
           that carry the SAME payload but a DIFFERENT variant tag, not just equate identical ones. To keep
           the compare a real RUNTIME tagged walk (a literal `(= (Ok v) (Err v))` could const-fold once both
           constructors are known at compile time), the constructor is SELECTED at runtime: `mk tag v = if
           tag then (Ok v) else (Err v)`, with `tag` and `v` both seed-derived. Then `(mk tag v)` equals
           itself (same tag, same payload → true), and `(mk tag v)` vs `(mk (not tag) v)` holds the SAME
           payload under DIFFERENT tags, so `=` is false. `main` returns true iff BOTH hold — pinning that a
           sum `=` has power in both directions (equates equal tagged values, separates ones that differ only
           in the discriminant) on a value inference cannot pre-decide.")
  (input
    (do
      (type Result (Ok Int64) (Err Int64))
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (mk (: tag Bool) (: v Int64)) (if tag (Ok v) (Err v)))
      (def
        (main (: seed Int64))
        (let
          ((tag (= (& (next seed) 1) 0)) (v (& (next (next seed)) 255)))
          (if (= (mk tag v) (mk tag v)) (not (= (mk tag v) (mk (not tag) v))) false)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

(case
  "a generated MIXED payload+nullary SUM value is reproducible from its seed (a 3-variant sum with a bare-name nullary variant)"
  (doc
    "The SUM cases above use `Result (Ok Int64) (Err Int64)` — every variant PAYLOADED. This one
           witnesses the shape the property-test generator gained bare-name-nullary support for: a MIXED sum
           whose variants are two PAYLOADED (`Circle Int64` / `Square Int64`) plus one NULLARY (`Point`, a
           bare-name variant). A seed-derived selector `(& (next s) 3)` (the low 2 bits mask directly to
           0..3) — expressed here as nested `if`s — picks the constructor; a masked int is the payload for the payloaded
           arms, and the nullary arm carries none. `(= (gen seed) (gen seed))` re-draws the SAME tagged value
           and the compound `=` walks the discriminant AND (for a payloaded arm) the carried Int64 — so a
           sum that MIXES arities compares correctly (a nullary draw equals a nullary draw; a payloaded draw
           equals the same-tag same-payload draw). Runs at the boundary so the constructor selection + tagged
           compare are real instructions. Pins mixed payload+nullary sum generation (the class the generator
           previously declined for a bare-name nullary variant) as a reproducible, storeless-graded witness.")
  (input
    (do
      (type Shape (Circle Int64) (Square Int64) (Point))
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (gen (: s Int64))
        (let
          ((sel (& (next s) 3)))
          (if
            (= sel 0)
            (Circle (& (next (next s)) 255))
            (if (= sel 1) (Square (& (next (next s)) 255)) Point))))
      (def (main (: seed Int64)) (= (gen seed) (gen seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: 777 Int64))
  (output (: true Bool)))

(case
  "a MIXED payload+nullary SUM compound = has DISCRIMINATING power (separates tags AND a payloaded vs a nullary variant)"
  (doc
    "The vacuity guard for the mixed-sum reproducibility case above: the compound `=` over a sum that
           MIXES payloaded and nullary variants must SEPARATE values that differ, not just equate identical
           ones — otherwise the reproducibility case (which only asserts `x = x`) could pass on a `=` that
           returns true for everything. Distinct from the all-payloaded `Result Ok/Err` discriminating case:
           this pins the two mixed-arity distinctions the bare-name-nullary generation needs to hold —
           (1) SAME tag same payload → equal (`Circle v = Circle v`); (2) DIFFERENT tag same payload →
           unequal (`Circle v ≠ Square v` — the discriminant decides even when the carried Int64 matches);
           (3) a PAYLOADED variant ≠ a NULLARY variant (`Circle v ≠ Point` — a heap-carrying value vs a bare
           nullary tag never compare equal). The constructor is SELECTED at runtime via `mk tag v` so no
           comparison const-folds (a literal `(= (Circle v) (Point))` could be decided at compile time);
           `main` returns 1 iff all three hold. Pins that mixed payload+nullary sum `=` has power in every
           direction on a value inference cannot pre-decide.")
  (input
    (do
      (type Shape (Circle Int64) (Square Int64) (Point))
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (mk (: tag Int64) (: v Int64)) (if (= tag 0) (Circle v) (if (= tag 1) (Square v) Point)))
      (def
        (main (: seed Int64))
        (let
          ((v (& (next seed) 255)))
          (if
            (= (mk 0 v) (mk 0 v))
            (if (not (= (mk 0 v) (mk 1 v))) (if (not (= (mk 0 v) (mk 2 v))) 1 0) 0)
            0)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: 1 Int64))
  (call main (: 777 Int64))
  (output (: 1 Int64)))

(case
  "a generated RECORD value is reproducible from its seed (compound = walks the record fields)"
  (doc
    "Witnesses §Generation Is Seeded And Reproducible for a generator that produces a RECORD — the
           other named product container beside the tuple. The doc of the tuple case promises structural
           equality is realized for `tuple/record/set/map`; the tuple, set (via the CRDT cases), and map
           (below) containers are witnessed — this pins the RECORD one. `gen` draws a `(record (x Int64)
           (y Bool))` from the seed (a masked int field + a threshold bool field), and `(= (gen seed) (gen
           seed))` = true — the whole record re-generates identically and the compound `=` walks it FIELD
           by field (a named-field product walk, distinct from the positional tuple walk). Runs at the
           boundary so the generation + the record field-walk compare are real instructions, not a fold.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (gen (: s Int64)) #record((= x (& (next s) 255)) (= y (< (& (next (next s)) 255) 128))))
      (def (main (: seed Int64)) (= (gen seed) (gen seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

(case
  "a RATIONAL leaf in a compound compares by CANONICAL form (2n/4 = n/2 inside a tuple)"
  (doc
    "Deepens the compound-`=`-over-a-bignum-leaf witness with RATIONAL's distinguishing feature:
           equality is by NORMALIZED (reduced) form, not by the stored numerator/denominator. Two tuples
           whose `Rational` element is written differently but denotes the SAME value — `2n/4` and `n/2` —
           compare EQUAL, because `Rational.of` reduces to lowest terms and the compound `=` walks that
           canonical form. This is a discriminating property (it would FAIL if `=` compared raw num/den):
           `(= (tuple (Rational.of 2n 4) …) (tuple (Rational.of n 2) …))` = true. Runs at the boundary.")
  (input
    (do
      (def
        (main (: n Int64))
        (= #tuple((Rational.of (Int64.wrapping-mul n 2) 4) true) #tuple((Rational.of n 2) true)))
      (export main)))
  (call main (: 1 Int64))
  (output (: true Bool))
  (call main (: 3 Int64))
  (output (: true Bool)))

(case
  "a RATIONAL compound = has DISCRIMINATING power (1/2 ≠ 1/3, but 0/2 = 0/3)"
  (doc
    "The counterpart to the canonical-form case: normalizing equality must still DISTINGUISH genuinely
           different values, or it would be vacuous. `1/2` and `1/3` are different rationals, so a tuple
           carrying each compares NOT-equal — `(= (tuple (Rational.of n 2) …) (tuple (Rational.of n 3) …))`
           = false at `n=1`. But `0/2` and `0/3` both reduce to `0`, so at `n=0` the same expression is
           true — the reduction equates the zeros. Pins that the Rational compound `=` has power in BOTH
           directions (equates equal values, separates unequal ones).")
  (input
    (do
      (def (main (: n Int64)) (= #tuple((Rational.of n 2) true) #tuple((Rational.of n 3) true)))
      (export main)))
  (call main (: 1 Int64))
  (output (: false Bool))
  (call main (: 0 Int64))
  (output (: true Bool)))

(case
  "a generated Rational's equality is INDEPENDENT of sign placement (n/d = -n/-d)"
  (doc
    "A property over runtime-constructed rationals: `Rational.of` normalizes the sign onto the
           NUMERATOR (denominator forced strictly positive), so where a caller writes the sign does not
           matter — `n/d` and `-n/-d` are the SAME rational for any seed-drawn `n`, `d` (d≠0). This is
           the sign-placement analogue of the canonical-form (lowest-terms) property above: equality is
           by the normalized form, not the stored operands. `(= (Rational.of n d) (Rational.of -n -d))`
           = true for every nonzero-denominator draw. Both operand pairs are true entry parameters (no
           fold folds them), so it exercises the runtime construction+normalize path (widen→BigInt, gcd,
           sign-onto-numerator) that landed as the runtime-operand sign-normalize case — a property, not a
           fixed pair. The `d=0` seed is discarded (a zero denominator has no rational value / traps),
           mirroring the rejection-sampling guard used elsewhere in this file.")
  (input
    (do
      (def
        (main (: n Int64) (: d Int64))
        (if
          (= d 0)
          true
          (= (Rational.of n d) (Rational.of (Int64.wrapping-sub 0 n) (Int64.wrapping-sub 0 d)))))
      (export main)))
  (call main (: 3 Int64) (: -7 Int64))
  (output (: true Bool))
  (call main (: 5 Int64) (: 8 Int64))
  (output (: true Bool))
  (call main (: -9 Int64) (: 4 Int64))
  (output (: true Bool)))

(case
  "generated FLOAT values are totally ordered (a trichotomy property over two generated floats)"
  (doc
    "A property over GENERATED floats using the runtime float ordering: `of` draws a float from the
           seed (an integer-valued float via `Float64.of-int`, which never produces NaN), so for any two
           generated floats exactly one of `<`, `=`, `>` holds — `(if (< x y) true (if (= x y) true (< y x)))`
           = true. This is trichotomy, the total-order law that would FAIL for NaN (all comparisons false);
           the generator producing only non-NaN floats is what makes it hold. Exercises the runtime float
           ordering (`<`) over the compiler's float generator at the boundary, not a fold.")
  (input
    (do
      (def (of (: n Int64)) (Float64.of-int n))
      (def
        (main (: a Int64) (: b Int64))
        (let ((x (of a))) (let ((y (of b))) (if (< x y) true (if (= x y) true (< y x))))))
      (export main)))
  (call main (: 3 Int64) (: 7 Int64))
  (output (: true Bool))
  (call main (: 5 Int64) (: 5 Int64))
  (output (: true Bool))
  (call main (: 9 Int64) (: -2 Int64))
  (output (: true Bool)))

(case
  "the int-to-float generator is order-preserving (a < b implies of a <= of b — NON-decreasing)"
  (doc
    "A cross-type ordering property tying the integer generator to its FLOAT image: `Float64.of-int`
           is monotonic NON-DECREASING (the IEEE i64->f64 round-to-nearest), so whenever `a < b` as
           integers, `of(a) <= of(b)` as floats. It is NOT strictly increasing: two DISTINCT i64 beyond
           2^53 round to the SAME f64 (the mantissa runs out of bits), so `of(a) < of(b)` is FALSE there —
           `of(2^53) == of(2^53 + 1)`. The correct invariant is `<=`, and the last call PINS exactly that
           boundary (a<b holds but the floats are EQUAL, so a strict `<` would wrongly report false — this
           case would be a vacuous false-green if it only tested small ints). Distinct from trichotomy
           (a within-float total order): this pins the generator's Int->Float mapping preserves order
           NON-strictly, exercising the integer `<` and the runtime float `<=` at the boundary. Holds for
           negatives (of-int is signed) and across the 2^53 rounding threshold.")
  (input
    (do
      (def (of (: n Int64)) (Float64.of-int n))
      (def (main (: a Int64) (: b Int64)) (if (< a b) (<= (of a) (of b)) true))
      (export main)))
  (call main (: 3 Int64) (: 7 Int64))
  (output (: true Bool))
  (call main (: 9 Int64) (: 2 Int64))
  (output (: true Bool))
  (call main (: -8 Int64) (: -2 Int64))
  (output (: true Bool))
  (call main (: 9007199254740992 Int64) (: 9007199254740993 Int64))
  (output (: true Bool)))

; --- §Refinements Constrain Generation --------------------------------------------------------------
; "A generator for a value of a refined type MUST produce only values satisfying that type's
; refinement." A bounded generator masks the raw stream into a range: `roll s = next s & (2^k − 1)`
; produces a value in `0 .. 2^k`. These pin that the bounded generator's output satisfies its range
; refinement for arbitrary runtime seeds — the harness-side realization of "the generator respects the
; refinement" ahead of a first-class refinement-type surface.
(case
  "a bounded generator produces only in-range values (0 <= roll < 64) for a runtime seed"
  (doc
    "Witnesses §Refinements Constrain Generation: masking the LCG output with `& 63` constrains it
           to the refinement `0 <= x < 64`. For any runtime seed the drawn value satisfies the bound —
           `(if (>= v 0) (< v 64) false)` = true. This is a bounded/refined-type generator producing
           only admissible values, checked at the boundary so the mask runs (a mask on a constant would
           fold).")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (roll (: s Int64)) (& (next s) 63))
      (def (main (: seed Int64)) (let ((v (roll seed))) (if (>= v 0) (< v 64) false)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: 999 Int64))
  (output (: true Bool))
  (call main (: -1 Int64))
  (output (: true Bool)))

; --- §Shrinking Converges To A Minimal Failing Input ------------------------------------------------
; "When a property fails, the harness MUST search for a smaller input that still fails", "The shrinking
; search MUST terminate rather than search unboundedly", and "The shrinking search MUST report a
; minimal failing input." A shrinker is a bounded upward search from the smallest candidate: scan
; 0,1,2,… while the property HOLDS; the first candidate at which it fails is the minimal failing input.
; Fuel bounds the scan so it terminates even when nothing in range fails.
(case
  "shrinking reports the minimal failing input"
  (doc
    "Witnesses §Shrinking Converges To A Minimal Failing Input (and §…Reports A Minimal Failing
           Input). Property `p x = x < 4`; the failing inputs are `x >= 4`. Scanning candidates upward
           from 0 while `p` holds, the search stops at the FIRST failing candidate = 4, the SMALLEST
           input that violates the property — exactly the minimal counterexample a shrinker reports.
           `start` is the fuel; 100 is ample to reach 4.")
  (input
    (do
      (def (p (: x Int64)) (< x 4))
      (def
        (search (: cand Int64) (: fuel Int64))
        (if (= fuel 0) cand (if (p cand) (search (+ cand 1) (- fuel 1)) cand)))
      (def (main (: start Int64)) (search 0 start))
      (export main)))
  (call main (: 100 Int64))
  (output (: 4 Int64)))

(case
  "the shrinking search terminates via its fuel bound rather than searching unboundedly"
  (doc
    "Witnesses §…MUST terminate rather than search unboundedly. When the property holds for every
           candidate the scan reaches, the fuel bound forces termination: with `p x = x < 1000000` and
           fuel 5, the search exhausts its fuel at candidate 5 (never having found a failure) and returns
           5 rather than looping forever. Pins that the search is TOTAL — bounded, not open-ended.")
  (input
    (do
      (def (p (: x Int64)) (< x 1000000))
      (def
        (search (: cand Int64) (: fuel Int64))
        (if (= fuel 0) cand (if (p cand) (search (+ cand 1) (- fuel 1)) cand)))
      (def (main (: fuel Int64)) (search 0 fuel))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

; --- §A Postcondition Is Usable As A Property / §Permutation Invariance Is A Property ---------------
; "A declared postcondition MUST be usable as a property oracle" and "A statement that a permutation of
; a fold's inputs produces a byte-identical result MUST be expressible as a property the generator
; exercises." The load-bearing first application (learnings/2026-07-04-fold-order-independence): a
; fold's order-independence is checkable by generated permutations. `sum` is a COMMUTATIVE fold, so its
; result is invariant under any permutation of its inputs — the property `sum(S) = sum(π(S))` holds;
; and the property has DISCRIMINATING POWER, since an order-DEPENDENT computation fails it.
(case
  "permutation invariance of a commutative fold holds — the property oracle passes"
  (doc
    "Witnesses §Permutation Invariance Is A Property and §A Postcondition Is Usable As A Property.
           `sum` is commutative, so for the inputs {1,2,3} and a permutation {3,1,2} the fold agrees:
           `(= (sum [1 2 3]) (sum [3 1 2]))` = true. This is the fold order-independence property
           (learnings/2026-07-04-fold-order-independence-is-a-verified-property) exercised as a plain
           predicate over two orderings — the property-testing rung of discharging order-independence.")
  (input
    (do
      (def (sum (: xs (List Int64))) (match xs (#list() 0) (#list(h (.. t)) (+ h (sum t)))))
      (def (main) (= (sum #list(1 2 3)) (sum #list(3 1 2))))
      (export main)))
  (output (: true Bool))
  (live-objects 0))

(case
  "permutation invariance of a commutative fold holds for generated runtime inputs"
  (doc
    "The generator-exercised form: the three inputs are DRAWN from a runtime seed (so nothing
           folds), then the fold is compared across two orderings of the SAME drawn values —
           `(= (sum [a b c]) (sum [c a b]))` = true. This is `sum(S) = sum(π(S))` checked on inputs the
           generator produced, exactly §Permutation Invariance Is A Property's `the generator exercises`.
           The list-reordering is done at the CALL site (the elements are scalars); the fold reduces each
           ordering to a scalar the seed's runtime `=` compares. The fold uses `Int64.wrapping-add` — the
           NAMED wrapping opt-in — because the drawn values are full-range LCG outputs whose partial sums
           can exceed Int64: the ORDINARY `+` TRAPS on overflow (numeric-model.md #Overflow, operator ruling
           2026-08-29 — trap on any overflow unless the caller explicitly asks for wrapping), and a trapping
           fold is NOT permutation-invariant (at seed 42, b+c overflows in the `[a b c]` order but a+b does
           not in `[c a b]`, so one ordering would trap and the other would not). Wrapping addition is
           associative and commutative modulo 2^64, so `sum(S) = sum(π(S))` holds unconditionally — the
           property the case witnesses. This is the explicit-wrapping companion of the LCG's own
           `wrapping-add`/`wrapping-mul`.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (sum (: xs (List Int64)))
        (match xs (#list() 0) (#list(h (.. t)) (Int64.wrapping-add h (sum t)))))
      (def
        (main (: seed Int64))
        (let
          ((a (next seed)))
          (let ((b (next a))) (let ((c (next b))) (= (sum #list(a b c)) (sum #list(c a b)))))))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: 42 Int64))
  (output (: true Bool))
  (live-objects 0))

(case
  "the permutation-invariance property has discriminating power — an order-dependent fold fails it"
  (doc
    "The counterpoint that makes the property meaningful: a computation that DEPENDS on order does
           NOT satisfy permutation invariance. `first-wins a b = a` (a latest-wins/first-arg projection)
           gives `first-wins 3 7 = 3` but `first-wins 7 3 = 7`, so `(= (first-wins a b) (first-wins b a))`
           = false. Pins that the property genuinely distinguishes order-independent from order-dependent
           behavior — a fold author engaging it gets a real check, not a tautology.")
  (input
    (do
      (def (first-wins (: a Int64) (: b Int64)) a)
      (def (main (: a Int64) (: b Int64)) (= (first-wins a b) (first-wins b a)))
      (export main)))
  (call main (: 3 Int64) (: 7 Int64))
  (output (: false Bool)))

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
(case
  "a set built from generated values is equal regardless of insertion order (order-independent equality on generated inputs)"
  (doc
    "Witnesses §Permutation Invariance Is A Property directly on a set: a set drawn from a runtime
           seed in one order equals the SAME elements inserted in a PERMUTED order —
           `(= (Set.of [a b c]) (Set.of [c a b]))` = true. Unlike the list-sum case this needs no scalar
           reduction: set equality is order-independent by construction, so the set IS the
           permutation-invariant value. The elements are drawn from a runtime seed (nothing folds).")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (main (: seed Int64))
        (let
          ((a (next seed)))
          (let ((b (next a))) (let ((c (next b))) (= #set(a b c) #set(c a b))))))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: 777 Int64))
  (output (: true Bool)))

(case
  "set union is commutative on generated inputs (a grow-only-set CRDT merge converges)"
  (doc
    "The convergence law a grow-only-set CRDT relies on: `A ∪ B = B ∪ A`, so two replicas that
           merge each other's states in EITHER order reach the same set. Checked on two sets drawn from a
           generated stream — `(= (union (Set.of [a b]) (Set.of [c d])) (union (Set.of [c d]) (Set.of
           [a b])))` = true. This is the property-testing rung of discharging order-independence for a set
           merge (learnings/2026-07-04-fold-order-independence), exercised on generated inputs.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (main (: seed Int64))
        (let
          ((a (next seed)))
          (let
            ((b (next a)))
            (let
              ((c (next b)))
              (let
                ((d (next c)))
                (= (Set.union #set(a b) #set(c d)) (Set.union #set(c d) #set(a b))))))))
      (export main)))
  (call main (: 777 Int64))
  (output (: true Bool)))

(case
  "set union is idempotent on generated inputs (re-delivering a set changes nothing)"
  (doc
    "The other CRDT-merge law: `A ∪ A = A`, so at-least-once / duplicate delivery of the same state
           does not change the merged set — `(= (union s s) s)` = true for a set drawn from a generated
           seed. Commutativity + idempotence (+ associativity below) are exactly the convergence
           properties order-independence requires, checked by generated inputs.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (main (: seed Int64))
        (let ((a (next seed))) (let ((b (next a))) (let ((s1 #set(a b))) (= (Set.union s1 s1) s1)))))
      (export main)))
  (call main (: 42 Int64))
  (output (: true Bool)))

(case
  "set union is associative on generated inputs (the order merges are grouped does not matter)"
  (doc
    "The third convergence law: `(A ∪ B) ∪ C = A ∪ (B ∪ C)`, so replicas that combine partial
           merges in different groupings still converge. Checked on three singleton sets drawn from a
           generated seed. Together with commutativity and idempotence this is the full commutative-
           monoid-with-idempotence structure a grow-only set has — the property a CRDT convergence check
           exercises over generated event sets.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (main (: seed Int64))
        (let
          ((a (next seed)))
          (let
            ((b (next a)))
            (let
              ((c (next b)))
              (let
                ((sa #set(a)))
                (let
                  ((sb #set(b)))
                  (let
                    ((sc #set(c)))
                    (= (Set.union (Set.union sa sb) sc) (Set.union sa (Set.union sb sc))))))))))
      (export main)))
  (call main (: 314159 Int64))
  (output (: true Bool)))

(case
  "the generated-set convergence property has discriminating power — two different generated sets are NOT equal"
  (doc
    "The counterpoint keeping the set-convergence cases honest: distinct generated values build
           DISTINCT sets, so `(= (Set.of [a]) (Set.of [b]))` = false when `a ≠ b`. Pins that set equality
           is real content equality — a degenerate `=` that called everything equal would make the
           convergence cases vacuously pass, and this case would catch it.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (main (: seed Int64)) (let ((a (next seed))) (let ((b (next a))) (= #set(a) #set(b)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: false Bool)))

(case
  "a MAP built from generated pairs is equal regardless of insertion order (the Map analogue of set convergence)"
  (doc
    "Witnesses §Permutation Invariance Is A Property on a MAP — the key/value container the tuple
           case's doc names alongside tuple/record/set. Like a set, a map's equality is INSERTION-ORDER-
           INDEPENDENT by construction (the CHAMP canonicalizes, 19-maps): the same two generated key/value
           pairs inserted in one order equal the same pairs inserted in the REVERSED order — `(= (insert
           (insert empty a 1) b 2) (insert (insert empty b 2) a 1))` = true. This is the map analogue of the
           order-independent-set-equality case, and it needs no scalar reduction: the map `=` compares the
           canonical entry set directly. Both keys are DRAWN from the seed at run time, so nothing folds —
           the two maps are built and compared as real instructions. (`a` and `b` are masked to distinct
           bytes only incidentally; even were they equal the second insert would overwrite consistently on
           both sides, so the equality still holds — order-independence is the load-bearing property.)")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (main (: seed Int64))
        (let
          ((a (& (next seed) 255)))
          (let
            ((b (& (next (next seed)) 255)))
            (=
              (Map.insert (Map.insert (Map.empty) a 1) b 2)
              (Map.insert (Map.insert (Map.empty) b 2) a 1)))))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: -7 Int64))
  (output (: true Bool)))

; --- §Refinements Constrain Generation, by REJECTION SAMPLING ---------------------------------------
; The masked-generator case above constrains generation by CONSTRUCTION (a bitmask can only produce an
; in-range value). The general way a harness makes "the generator produces only values satisfying the
; refinement" hold for a predicate a mask cannot express is REJECTION SAMPLING: draw from the seed
; stream, keep the draw if it satisfies the refinement, otherwise ADVANCE the seed and draw again —
; bounded by fuel so the search is TOTAL (the same fuel discipline the shrinking search uses). These pin
; that a rejection-sampled draw satisfies its refinement, and that the loop terminates on fuel
; exhaustion rather than searching unboundedly (a refinement no draw in range satisfies must not hang).
(case
  "rejection sampling draws until the refinement holds — an only-even generator produces an even value"
  (doc
    "Witnesses §Refinements Constrain Generation for a predicate a bitmask cannot directly impose:
           `draw-even` advances the seed until an EVEN value appears, then returns it. The result's low
           bit is 0 for any seed — `(& (draw-even seed 50) 1)` = 0 — so the generator produces only values
           satisfying the `even` refinement. Fuel-bounded (50 draws) so the loop is total. Runs at the
           boundary (the seed is a parameter), so the search executes rather than folding.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (draw-even (: s Int64) (: fuel Int64))
        (let ((v (next s))) (if (= (& v 1) 0) v (if (= fuel 0) v (draw-even v (- fuel 1))))))
      (def (main (: seed Int64)) (& (draw-even seed 50) 1))
      (export main)))
  (call main (: 12345 Int64))
  (output (: 0 Int64))
  (call main (: 999 Int64))
  (output (: 0 Int64)))

(case
  "rejection sampling into a narrow range refinement produces only in-range values"
  (doc
    "Rejection into a range the mask alone cannot give: mask to `0..16` then REJECT anything `>= 10`,
           re-drawing until the value lands in `0..10`. The returned value satisfies `0 <= v < 10` for any
           seed — `(if (>= v 0) (< v 10) false)` = true. This is the harness-side realization of
           §Refinements Constrain Generation for an arbitrary decidable refinement, fuel-bounded (100
           draws).")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (draw (: s Int64) (: fuel Int64))
        (let ((v (& (next s) 15))) (if (< v 10) v (if (= fuel 0) 0 (draw (next s) (- fuel 1))))))
      (def (main (: seed Int64)) (let ((v (draw seed 100))) (if (>= v 0) (< v 10) false)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool))
  (call main (: 42 Int64))
  (output (: true Bool))
  (call main (: 7 Int64))
  (output (: true Bool)))

(case
  "rejection sampling terminates on fuel exhaustion when the refinement is unsatisfiable"
  (doc
    "The termination guarantee: a refinement NO draw can satisfy (`v < 0` for a non-negative masked
           draw) must not hang. The fuel bound fires and the search returns its fallback (-1) after 5
           draws rather than looping forever — the total-search discipline §Shrinking …MUST terminate
           applies equally to a rejection-sampling generator. Pins that a mis-stated refinement fails
           closed (bounded) rather than diverging.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (draw (: s Int64) (: fuel Int64))
        (let ((v (& (next s) 15))) (if (< v 0) v (if (= fuel 0) -1 (draw (next s) (- fuel 1))))))
      (def (main (: seed Int64)) (draw seed 5))
      (export main)))
  (call main (: 12345 Int64))
  (output (: -1 Int64)))

; --- A Bool generator (the enum-shaped generator) ---------------------------------------------------
; Generation is not only over integers: a property over a Bool (or a small enum) needs a generator for
; that type. The seed's low bit is a Bool generator — `gen-bool s = (next s & 1) = 0`. These pin it is
; reproducible (same seed → same Bool, §Generation Is Seeded And Reproducible) and covers both values
; across seeds (not a constant), the two properties a usable generator for a finite type must have.
(case
  "a Bool generator derived from the seed is reproducible (same seed, same Bool)"
  (doc
    "Witnesses §Generation Is Seeded And Reproducible for a Bool-typed generator: `gen-bool` reads
           the low bit of the LCG output, so `(= (gen-bool seed) (gen-bool seed))` = true — the generated
           Bool is a deterministic function of the seed, exactly as the integer generator is. This is the
           generator a property quantified over a Bool draws from.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (gen-bool (: s Int64)) (= (& (next s) 1) 0))
      (def (main (: seed Int64)) (= (gen-bool seed) (gen-bool seed)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: true Bool)))

(case
  "the Bool generator covers both values across seeds (it is not a constant)"
  (doc
    "The coverage counterpart: two seeds whose LCG low bits differ generate DIFFERENT Bools, so
           `(= (gen-bool s1) (gen-bool s2))` = false — the generator explores both `true` and `false`
           rather than pinning one. A degenerate Bool generator that ignored its seed would make this
           true and never test the `false` branch of a property; this case pins that it does not.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (gen-bool (: s Int64)) (= (& (next s) 1) 0))
      (def (main (: s1 Int64) (: s2 Int64)) (= (gen-bool s1) (gen-bool s2)))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: false Bool)))

; --- §Shrinking Converges To A Minimal Failing Input (end-to-end: find THEN shrink) ------------------
; The two spec halves — a property RUN that surfaces a failing input, and a SHRINK that minimizes it —
; composed into the full harness loop. `find` draws byte-masked values from the seed stream until one
; VIOLATES the property `p x = x < 50` (i.e. the first `x >= 50` the stream produces), fuel-bounded so
; the search is total; `shrink` then walks UP from 0 to the SMALLEST value that still violates `p`
; (again fuel-bounded), which is exactly 50 regardless of which counterexample `find` surfaced. This is
; the §Shrinking Converges guarantee end-to-end: a run finds SOME failing input, and shrinking converges
; to the minimal one — independent of the seed that started it. Runs at the RUNTIME boundary (a `seed`
; parameter) so it exercises the emitted generate-check-shrink machinery, not a compile-time fold.
(case
  "end-to-end: a property run finds a failing input, then the shrinker converges to the minimal failing value"
  (doc
    "Witnesses §Shrinking Converges To A Minimal Failing Input composed with the property RUN:
           `find` scans the seed stream for the first byte-masked draw that VIOLATES `p x = x < 50`
           (a value >= 50), fuel-bounded to 200 draws so it is total; `shrink` then searches UPWARD from
           0 for the smallest value still violating `p`, converging to 50. The minimal failing input (50)
           does NOT depend on WHICH counterexample `find` surfaced — every failing run shrinks to the
           same minimum — which is the defining property of a convergent shrinker. A runtime `seed`
           parameter keeps it off the folding path, so the emitted find→shrink loop actually runs.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (p (: x Int64)) (< x 50))
      (def
        (find (: s Int64) (: n Int64))
        (if (= n 0) -1 (let ((v (& (next s) 255))) (if (p v) (find (next s) (- n 1)) v))))
      (def
        (shrink (: cand Int64) (: fuel Int64))
        (if (= fuel 0) cand (if (p cand) (shrink (+ cand 1) (- fuel 1)) cand)))
      (def (main (: seed Int64)) (let ((cx (find seed 200))) (if (>= cx 50) (shrink 0 100) -1)))
      (export main)))
  (call main (: 12345 Int64))
  (output (: 50 Int64)))

; --- MODEL-based properties: the implementation agrees with a simple abstract model ---------------
; The properties above are algebraic laws over ONE structure (commutativity, idempotence, involution
; shapes). A MODEL-based property drives the REAL structure and a trivially-correct abstract model with
; the SAME generated operation sequence and asserts they agree at the end — the strongest generic oracle
; a property harness offers (it catches any divergence, not just a law violation).
(case
  "a model-oracle property — CHAMP Map.len agrees with a counting model over generated inserts"
  (doc
    "Twenty seeded-LCG-generated inserts (keys masked to 0..7, guaranteeing collisions) drive the
           real CHAMP map AND an abstract count-distinct model in one fold: the model increments only
           when `Map.lookup` misses (a fresh key), the map absorbs every insert. At the end `Map.len m`
           must equal the model count — an overwrite that grew the map, or an insert that lost an entry,
           diverges. Two seeds witness two operation sequences through one compiled loop. This is the
           model-based-testing idiom (real vs abstract state agreeing under a generated workload) the
           algebraic-law cases above cannot express.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (drive (: s Int64) (: n Int64) (: m (Map Int64 Int64)) (: cnt Int64))
        (if
          (< n 1)
          (if (= (Map.len m) cnt) 1 0)
          (let
            ((k (& (next s) 7)))
            (drive
              (next s)
              (- n 1)
              (Map.insert m k 1)
              (match (Map.lookup m k) ((Some v) cnt) ((None u) (+ cnt 1)))))))
      (def (main (: seed Int64)) (drive seed 20 Map.empty 0))
      (export main)))
  (call main (: 12345 Int64))
  (output (: 1 Int64))
  (call main (: 999 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "the model-oracle property has DISCRIMINATING power — a BROKEN model (counts every insert) diverges from Map.len"
  (doc
    "The counterpoint that makes the count-model oracle above meaningful: a model that MISCOUNTS
           does NOT agree with the real structure, so the oracle catches it. Same generated workload
           (20 seeded inserts, keys masked 0..7 so collisions are guaranteed and the real CHAMP map holds
           at most 8 distinct keys) drives the SAME real `Map.insert`, but the model here counts EVERY
           insert (`(+ cnt 1)` unconditionally) instead of only the distinct-key misses. `Map.len m`
           (<= 8) can never equal a 20-count, so the agreement check is false and main returns 0 on both
           seeds. Pins that the model-oracle idiom genuinely detects a real-vs-abstract divergence — a
           model-based property author gets a real check, not a tautology that a trivially-true agreement
           would pass. The vacuity guard the algebraic-law families have (permutation/set-convergence/=)
           but the model-oracle family lacked.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (drive (: s Int64) (: n Int64) (: m (Map Int64 Int64)) (: cnt Int64))
        (if
          (< n 1)
          (if (= (Map.len m) cnt) 1 0)
          (let ((k (& (next s) 7))) (drive (next s) (- n 1) (Map.insert m k 1) (+ cnt 1)))))
      (def (main (: seed Int64)) (drive seed 20 Map.empty 0))
      (export main)))
  (call main (: 12345 Int64))
  (output (: 0 Int64))
  (call main (: 999 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a generated map workload agrees with a linear-scan model at EVERY key of the domain"
  (doc
    "The exhaustive-agreement upgrade of the count-model pin above (which checks ONE aggregate):
           30 generated inserts over a 16-key masked domain (overwrites guaranteed), then EVERY key
           0..15 is checked — `Map.lookup` against a linear scan of the map's OWN to-list, misses
           included (both answer -1). 16 agreements or -999 on the first divergence; two seeds drive two
           workloads through one compiled verifier. The strongest per-key oracle: any lost overwrite,
           phantom entry, or enumeration drift shows as a point disagreement the aggregate count can
           miss.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (scan (: ps (List (Tuple Int64 Int64))) (: k Int64))
        (match
          ps
          (#list() -1)
          (#list(h (.. t)) (match h (#tuple(pk pv) (if (= pk k) pv (scan t k)))))))
      (def
        (drive (: s Int64) (: n Int64) (: m (Map Int64 Int64)))
        (if (< n 1) m (drive (next s) (- n 1) (Map.insert m (& (next s) 15) n))))
      (def
        (verify (: m (Map Int64 Int64)) (: ps (List (Tuple Int64 Int64))) (: k Int64) (: acc Int64))
        (if
          (> k 15)
          acc
          (verify
            m
            ps
            (+ k 1)
            (if (= (match (Map.lookup m k) ((Some v) v) ((None u) -1)) (scan ps k)) (+ acc 1) -999))))
      (def
        (main (: seed Int64))
        (let ((m (drive seed 30 Map.empty))) (verify m (Map.to-list m) 0 0)))
      (export main)))
  (call main (: 42 Int64))
  (output (: 16 Int64))
  (call main (: 777 Int64))
  (output (: 16 Int64))
  (live-objects known-leak))

(case
  "the per-key linear-scan model oracle has DISCRIMINATING power — a value-forgetting model diverges at every present key"
  (doc
    "The vacuity guard for the per-key model oracle above: a model that MISREADS the stored value does
           NOT agree with the real structure, so the per-key oracle catches it. Same generated workload (30
           seeded inserts over the 16-key masked domain) drives the SAME real `Map.insert`, but the model
           here is a BROKEN value-forgetting scan that returns a constant 0 for EVERY key (ignoring both the
           to-list and the key) instead of the map's stored value. The real per-key answer is `Map.lookup`'s
           value at a PRESENT key and -1 at a MISSING key (the `(None u) -1` arm), and `drive` inserts value
           `n` (the countdown 30..1 — ALWAYS >= 1, never 0). So the model's constant 0 disagrees with the
           real answer at EITHER kind of key (>= 1 at a present key, -1 at a missing key), and `verify`
           diverges (-999) at the FIRST key it checks (key 0), present or missing. Two seeds return -999.
           Pins that the STRONGEST oracle (per-key agreement) genuinely detects a value-level real-vs-model
           disagreement, not a tautology — the discriminating counterpart the count-model family (line 643)
           has but the exhaustive per-key family lacked.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (broken-scan (: ps (List (Tuple Int64 Int64))) (: k Int64)) 0)
      (def
        (drive (: s Int64) (: n Int64) (: m (Map Int64 Int64)))
        (if (< n 1) m (drive (next s) (- n 1) (Map.insert m (& (next s) 15) n))))
      (def
        (verify (: m (Map Int64 Int64)) (: ps (List (Tuple Int64 Int64))) (: k Int64) (: acc Int64))
        (if
          (> k 15)
          acc
          (verify
            m
            ps
            (+ k 1)
            (if
              (= (match (Map.lookup m k) ((Some v) v) ((None u) -1)) (broken-scan ps k))
              (+ acc 1)
              -999))))
      (def
        (main (: seed Int64))
        (let ((m (drive seed 30 Map.empty))) (verify m (Map.to-list m) 0 0)))
      (export main)))
  (call main (: 42 Int64))
  (output (: -999 Int64))
  (call main (: 777 Int64))
  (output (: -999 Int64))
  (live-objects known-leak))

(case
  "a generated insert/remove Set workload agrees with a BITMASK model at every step's end"
  (doc
    "The model-oracles above drive INSERT-only workloads; this one mixes DELETIONS: the seeded
           stream drives Set.insert/Set.remove (key = s&7, op = bit 8) against a bitmask model
           compared by popcount at the end — the CHAMP remove path's node collapse under a generated
           adversarial sequence, including dedup-then-remove-then-reinsert orbits. Three seeds:
           41/41/51 (len·10 + agree; the seed-7 face reaches a different live-set size, so the
           encode is not seed-degenerate).")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (popcount (: b Int64) (: acc Int64))
        (if (= b 0) acc (popcount (>> b 1) (+ acc (& b 1)))))
      (def
        (run (: s Int64) (: n Int64) (: st (Set Int64)) (: mask Int64))
        (if
          (= n 0)
          (+ (* (Set.len st) 10) (if (= (Set.len st) (popcount mask 0)) 1 0))
          (do
            (def s2 (next s))
            (def k (& s2 7))
            (if
              (= (& (>> s2 8) 1) 1)
              (run s2 (- n 1) (Set.insert st k) (| mask (<< 1 k)))
              (run s2 (- n 1) (Set.remove st k) (& mask (^ (<< 1 k) -1)))))))
      (def (main (: seed Int64) (: n Int64)) (run seed n #set() 0))
      (export main)))
  (call main (: 12345 Int64) (: 40 Int64))
  (output (: 41 Int64))
  (call main (: 99 Int64) (: 25 Int64))
  (output (: 41 Int64))
  (call main (: 7 Int64) (: 12 Int64))
  (output (: 51 Int64))
  (live-objects known-leak))

(case
  "generated small-alphabet strings dedup in a Set by content across per-draw construction"
  (doc
    "A STRING generator (seeded picks from a 4-letter alphabet, 1-or-2-char words via a draw
           bit, built by per-draw CONCAT) driving Set-of-STRING dedup — each inserted string is a
           fresh heap value whose content may collide with a prior draw's (champ_eq byte-walk under
           a generated workload). 3 seed/length faces with distinct counts.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (pick (: s Int64)) (match (& s 3) (0 "a") (1 "b") (2 "c") (_ "d")))
      (def
        (run (: s Int64) (: n Int64) (: seen (Set String)))
        (if
          (= n 0)
          (Set.len seen)
          (do
            (def s2 (next s))
            (def c1 (pick s2))
            (def two (& (>> s2 4) 1))
            (def s3 (next s2))
            (def w (if (= two 1) (String.concat c1 (pick s3)) c1))
            (run s3 (- n 1) (Set.insert seen w)))))
      (def (main (: seed Int64) (: n Int64)) (run seed n #set()))
      (export main)))
  (call main (: 12345 Int64) (: 3 Int64))
  (output (: 3 Int64))
  (call main (: 7 Int64) (: 5 Int64))
  (output (: 3 Int64))
  (call main (: 99 Int64) (: 2 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "generated string keys OVERWRITE by content and the first word's final value is observable"
  (doc
    "The value-side companion: word→draw-index inserted per draw (collided keys OVERWRITE by
           content), then the FIRST word is RE-DERIVED from the seed and its final value read — the
           deterministic-generator property composed with map overwrite (31 = w1 never collided;
           43 = w1's slot overwritten by draw 3).")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (pick (: s Int64)) (match (& s 3) (0 "a") (1 "b") (2 "c") (_ "d")))
      (def
        (word (: s Int64))
        (do
          (def s2 (next s))
          (def c1 (pick s2))
          (def two (& (>> s2 4) 1))
          (def s3 (next s2))
          #tuple((if (= two 1) (String.concat c1 (pick s3)) c1) s3)))
      (def
        (run (: s Int64) (: i Int64) (: n Int64) (: m (Map String Int64)))
        (if (> i n) m (match (word s) (#tuple(w s2) (run s2 (+ i 1) n (Map.insert m w i))))))
      (def
        (main (: seed Int64) (: n Int64))
        (do
          (def m (run seed 1 n Map.empty))
          (def w1 (match (word seed) (#tuple(w _s) w)))
          (+ (* (Map.len m) 10) (match (Map.lookup m w1) ((Some v) v) ((None _u) 0)))))
      (export main)))
  (call main (: 12345 Int64) (: 3 Int64))
  (output (: 31 Int64))
  (call main (: 99 Int64) (: 4 Int64))
  (output (: 41 Int64))
  (call main (: 11 Int64) (: 6 Int64))
  (output (: 43 Int64))
  (live-objects known-leak))

(case
  "symbols interned from GENERATED strings dedup by content in a symbol set"
  (doc
    "The symbol-intern analogue: Symbol.of over generator-produced strings dedups by CONTENT
           in a (Set Symbol) — each Symbol.of canonicalizes a fresh runtime string; an allocation-
           order identity would never dedup (count = n). 3 seed faces.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (pick (: s Int64)) (match (& s 3) (0 "a") (1 "b") (2 "c") (_ "d")))
      (def
        (run (: s Int64) (: n Int64) (: seen (Set Symbol)))
        (if
          (= n 0)
          (Set.len seen)
          (do (def s2 (next s)) (run s2 (- n 1) (Set.insert seen (Symbol.of (pick s2)))))))
      (def (main (: seed Int64) (: n Int64)) (run seed n #set()))
      (export main)))
  (call main (: 12345 Int64) (: 3 Int64))
  (output (: 3 Int64))
  (call main (: 99 Int64) (: 6 Int64))
  (output (: 4 Int64))
  (call main (: 5 Int64) (: 2 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a LIST shrinker drops elements greedily and converges to a minimal failing sublist"
  (doc
    "COMPOUND shrinking (the scalar shrink pins above search upward over integers): greedy
           drop-one-element with RESTART-on-success over a failing list (sum >= 100) converges from
           [60 50 30] to the minimal failing SUBLIST [60 50] — every further single-drop passes,
           the 1-minimality that defines a convergent compound shrinker. The never-fails control
           face returns the ok marker.")
  (input
    (do
      (def
        (sum-l (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def (fails (: xs (List Int64))) (>= (sum-l xs 0) 100))
      (def
        (drop-at (: xs (List Int64)) (: i Int64) (: j Int64) (: acc (List Int64)))
        (match
          xs
          (#list() acc)
          (#list(h (.. t))
            (if (= j i) (drop-at t i (+ j 1) acc) (drop-at t i (+ j 1) (List.push acc h))))))
      (def
        (try-drops (: xs (List Int64)) (: i Int64))
        (if
          (>= i (List.len xs))
          xs
          (do
            (def cand (drop-at xs i 0 #list()))
            (if (fails cand) (try-drops cand 0) (try-drops xs (+ i 1))))))
      (def
        (main (: mode Int64))
        (do
          (def xs (if (= mode 1) #list(60 50 30) #list(10 20 30)))
          (if (fails xs) (do (def m (try-drops xs 0)) (+ (* (sum-l m 0) 10) (List.len m))) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1102 Int64))
  (call main (: 2 Int64))
  (output (: -1 Int64))
  (live-objects known-leak))

(case
  "a generated list reverses twice to itself — an involution property over generated content"
  (doc
    "The involution law over GENERATED content: an 8-element list of masked LCG draws, reversed
           twice, equals itself — `rev` is a fold whose accumulator prepends via `List.concat (list h)`,
           so the double application must restore both length and order for whatever content the seed
           produced. The classic reverse-involution property, here witnessing the RRB list's push/concat/
           destructure round-trip under a generated workload rather than a literal.")
  (input
    (do
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def
        (gen (: s Int64) (: n Int64) (: acc (List Int64)))
        (if (< n 1) acc (gen (next s) (- n 1) (List.push acc (& (next s) 63)))))
      (def
        (rev (: xs (List Int64)) (: acc (List Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (rev t (List.concat #list(h) acc)))))
      (def
        (main (: seed Int64))
        (let ((xs (gen seed 8 #list()))) (if (= (rev (rev xs #list()) #list()) xs) 1 0)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a set over a NULLARY-SUM key enumerates canonically — permutation-invariant AND discriminant-ordered"
  (doc
    "Witnesses §Permutation Invariance Is A Property on a set whose element is a NULLARY SUM (a bare
           enum constructor), the tagged-union analogue of the Int64-keyed set-convergence case above. Two
           facts must hold for `Set.to-list` over such keys, and this pins BOTH: (1) permutation invariance —
           the same three levels inserted in a DIFFERENT order enumerate to the SAME list; (2) canonical
           discriminant order — the enumeration is sorted by the constructor's discriminant (Lo<Mid<Hi), not
           insertion order. Fact (2) is load-bearing and non-obvious: a nullary sum boxes via box-int, and a
           small discriminant is a fixnum IMMEDIATE, so the descriptor-guided value-compare that orders the
           set's to-list must decode the discriminant FROM the immediate — reading it as 0 for every key
           (the immediate-totality default) would collapse all keys to Equal and a stable sort would silently
           preserve insertion order (core-semantics.md #Sum Values Compare By Discriminant Then Payload). The
           three levels are SELECTED at run time from the seed (a masked `% 3` over successive LCG draws), so
           the set construction + the ordered enumeration are real instructions, never a compile-time fold.
           `sorted` walks the enumerated list asserting each discriminant is >= the previous; `main` returns 1
           iff the two insertion orders agree AND the enumeration is discriminant-sorted.")
  (input
    (do
      (type Level (Lo) (Mid) (Hi))
      (def
        (next (: s Int64))
        (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
      (def (lvl (: s Int64)) (let ((m (% (& s 255) 3))) (if (= m 0) (Lo) (if (= m 1) (Mid) (Hi)))))
      (def (di (: v Level)) (match v ((Lo) 0) ((Mid) 1) ((Hi) 2)))
      (def
        (sorted (: xs (List Level)) (: prev Int64))
        (match xs (#list() true) (#list(h (.. t)) (if (< (di h) prev) false (sorted t (di h))))))
      (def
        (main (: seed Int64))
        (let
          ((a (lvl (next seed))))
          (let
            ((b (lvl (next (next seed)))))
            (let
              ((c (lvl (next (next (next seed))))))
              (if
                (= (Set.to-list #set(a b c)) (Set.to-list #set(c a b)))
                (if (sorted (Set.to-list #set(a b c)) 0) 1 0)
                0)))))
      (export main)))
  (call main (: 12345 Int64))
  (output (: 1 Int64))
  (call main (: 777 Int64))
  (output (: 1 Int64))
  (call main (: -7 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a scalar-aware string shrinker converges to the 1-minimal failing string"
  (doc
    "The STRING sibling of the list-shrinker pin (:782) and the original program behind the
           recursive-slice invalid-module fix (597e0ff7d): greedy drop-one-SCALAR with
           restart-on-failure over `fails = scalar-len >= 3`. From the rope \"aébc\" it drops `a`
           (\"ébc\" still fails → restart) and then every single-drop of \"ébc\" passes — the
           1-minimal failing string, whose multibyte é makes scalar-len 3 ≠ byte-len 4 (304). The
           never-fails control returns the ok marker (202). Composes the shrinker recursion, the
           drop-scalar slice-concat helper, and a scalar-len exit on the loop-carried rope — the
           exact shape that emitted an invalid module before the slice-bound high-water fix.")
  (input
    (do
      (def
        (drop-sc (: s String) (: i Int64))
        (String.concat
          (Option.expect (String.slice s 0 i) "lo")
          (Option.expect (String.slice s (+ i 1) (String.scalar-len s)) "hi")))
      (def (fails (: s String)) (>= (String.scalar-len s) 3))
      (def
        (try-drops (: s String) (: i Int64))
        (if
          (>= i (String.scalar-len s))
          s
          (do (def cand (drop-sc s i)) (if (fails cand) (try-drops cand 0) (try-drops s (+ i 1))))))
      (def
        (main (: mode Int64))
        (do
          (def start (if (= mode 1) (String.concat "aé" "bc") "xy"))
          (def r (if (fails start) (try-drops start 0) "ok"))
          (+ (* 100 (String.scalar-len r)) (String.byte-len r))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 304 Int64))
  (call main (: 0 Int64))
  (output (: 202 Int64))
  (live-objects known-leak))

; --- The 2-D coordinate-descent pair shrinker. ---
(case
  "a PAIR shrinker coordinate-descends each component to a jointly 1-minimal failing pair"
  (doc
    "The shrink pins are 1-D (scalar scan, list greedy-drop, string scalar-aware); this shrinks a 2-D input whose components INTERACT through the predicate (fails iff x*y >= 12): shrinking x re-slackens y, and the fixpoint (2,6) from (10,8) is jointly 1-minimal — (1,6) and (2,5) both pass. Downward decrement-while-fails descent, fuel-bounded both axes.")
  (input
    (do
      (def (fails (: x Int64) (: y Int64)) (>= (* x y) 12))
      (def
        (shrink-x (: x Int64) (: y Int64) (: fuel Int64))
        (if (= fuel 0) x (if (fails (- x 1) y) (shrink-x (- x 1) y (- fuel 1)) x)))
      (def
        (shrink-y (: x Int64) (: y Int64) (: fuel Int64))
        (if (= fuel 0) y (if (fails x (- y 1)) (shrink-y x (- y 1) (- fuel 1)) y)))
      (def
        (main (: sx Int64) (: sy Int64))
        (do (def mx (shrink-x sx sy 100)) (def my (shrink-y mx sy 100)) (+ (* 100 mx) my)))
      (export main)))
  (call main (: 10 Int64) (: 8 Int64))
  (output (: 206 Int64)))

; --- The SUM-payload shrinker (re-homed from the deleted CLI e2e: shrink a tagged variant's payload). ---
(case
  "a SUM-payload shrinker descends a tagged variant's payload to its minimal failing value, preserving the variant"
  (doc
    "The existing shrink pins descend a BARE scalar, a LIST spine, a STRING's scalars, or a 2-D pair —
           none shrinks the PAYLOAD of a sum VARIANT while keeping the tag. This re-homes the deleted
           `a_failing_sum_payload_guard_property_shrinks_to_an_in_domain_payload` e2e as a seed-path corpus
           case: the failing predicate is `fails g = (match g ((Small n) (>= n 5)) ((Big n) (>= n 5)))`, and
           the shrinker decrement-while-fails descends the payload, RECONSTRUCTING the same variant each step
           (`(Small (- n 1))` / `(Big (- n 1))`) so it genuinely walks the tag and rebuilds the payload, not
           just an untagged int. From payload 40 it converges to the 1-minimal failing payload 5 (payload 4
           passes), and the variant is UNCHANGED. DISCRIMINATING across the two calls: a Small start returns
           `(* 10 5)` = 50 and a Big start `(+ (* 10 5) 1)` = 51 — proving BOTH that the payload reached
           exactly the minimal failing 5 AND that the shrink preserved the variant tag (50 vs 51 separate).
           Fuel-bounded so a mis-shrink cannot loop.")
  (input
    (do
      (type Guess (Small Int64) (Big Int64))
      (def (fails (: g Guess)) (match g ((Small n) (>= n 5)) ((Big n) (>= n 5))))
      (def
        (shrink (: g Guess) (: fuel Int64))
        (if
          (= fuel 0)
          g
          (match
            g
            ((Small n) (if (fails (Small (- n 1))) (shrink (Small (- n 1)) (- fuel 1)) g))
            ((Big n) (if (fails (Big (- n 1))) (shrink (Big (- n 1)) (- fuel 1)) g)))))
      (def
        (main (: which Bool))
        (let
          ((start (if which (Small 40) (Big 40))))
          (let ((m (shrink start 100))) (match m ((Small n) (* 10 n)) ((Big n) (+ (* 10 n) 1))))))
      (export main)))
  (call main (: true Bool))
  (output (: 50 Int64))
  (call main (: false Bool))
  (output (: 51 Int64))
  (live-objects known-leak))

; --- In-domain shrinking: the shrink search stays within a refinement's window (Refinements × Shrinking). ---
(case
  "a refined-newtype shrink stays IN-DOMAIN — it converges to the minimal value the invariant still admits, not below the floor"
  (doc
    "The existing shrink pins (scalar #22, list #47, string #50, pair #51, sum-payload) all descend an
           UNCONSTRAINED domain to the global minimal failing input. This re-homes the deleted
           `a_failing_invariant_property_shrinks_to_the_minimal_in_domain_value` e2e: when the generated type
           carries a range @invariant (`Percent`, `10 <= n <= 100`), the shrinker must not walk BELOW the
           domain floor — a shrunk candidate that violates the invariant is not a valid counterexample. The
           failing predicate is `fails n = (>= n 40)`; a naive decrement-while-fails would reach 40 (the
           1-minimal failing value), but the DOMAIN floor here is 10 and the interaction we pin is that the
           shrink CLAMPS at the domain floor when the whole in-domain window fails. Two calls discriminate the
           two regimes: (mode 0) floor 10 < 40 so the shrink stops at the in-domain 1-minimal 40; (mode 1) a
           tighter fails `(>= n 5)` whose failing set covers the ENTIRE domain [10,100], so the shrink clamps
           at the floor 10 (going to 4 would leave the domain) — proving the shrink respects the refinement.
           `main` returns the shrunk value directly. Fuel-bounded so a floor-miss cannot underflow-loop.")
  (input
    (do
      (def (dom-lo) 10)
      (def (fails-a (: n Int64)) (>= n 40))
      (def (fails-b (: n Int64)) (>= n 5))
      ; shrink DOWNWARD while the candidate both FAILS and stays >= the domain floor; clamp at floor.
      (def
        (shrink-a (: n Int64) (: fuel Int64))
        (if
          (= fuel 0)
          n
          (if (and (> n (dom-lo)) (fails-a (- n 1))) (shrink-a (- n 1) (- fuel 1)) n)))
      (def
        (shrink-b (: n Int64) (: fuel Int64))
        (if
          (= fuel 0)
          n
          (if (and (> n (dom-lo)) (fails-b (- n 1))) (shrink-b (- n 1) (- fuel 1)) n)))
      (def (main (: mode Int64)) (if (= mode 0) (shrink-a 100 200) (shrink-b 100 200)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 40 Int64))
  (call main (: 1 Int64))
  (output (: 10 Int64)))

; --- The serialization ROUND-TRIP property: decode . encode = Some (Value.encode/decode, R2 binary-AST). ---
(case
  "a Value.encode then Value.decode round-trips a compound value back to Some of the SAME structure"
  (doc
    "The canonical serialization property `Value.decode (Value.encode v) == Some v` — the R2 binary-AST
           encoder/decoder (`Value.encode : forall a. a -> Bytes` TOTAL, `Value.decode : forall a. Bytes ->
           Option a` PARTIAL, `a` grounded by the call-site expected type). This is the property-testing angle
           on the newly-runnable encoder: a value is generated from the seed `n`, encoded to its binary-AST
           bytes, and decoded back at the SAME type via the enclosing `(: (Value.decode ..) (Option T))`
           annotation (which threads `a := T` into the decode node so its descriptor is concrete — an
           unsolved `a` would decline). The round-tripped value must be `Some` of a STRUCTURALLY-EQUAL value.
           The generated value is a `(Tuple Int64 Int64)` (a heap-handle compound — the R2 v1 carve-out domain
           is handle-typed values; a scalar-erased newtype still declines the emit, a later increment). To
           prove BOTH elements survive the round-trip in the right POSITIONS (not dropped, swapped, or
           zeroed), the match folds them positionally as `(+ (* a 1000) c)` over `(tuple n (+ n 100))`, and
           two calls discriminate: n=7 -> `(tuple 7 107)` -> 7*1000+107 = 7107; n=3 -> `(tuple 3 103)` ->
           3103. A decode that lost the second element would give 7000/3000; one that swapped them 107007/…;
           an all-`None` decode would give -1 twice (indistinguishable calls) — so the two distinct outputs
           pin a faithful structural round-trip. Runs at the boundary so the decoded heap value is realized.
           NOTE (target coverage): the `Value` emit is wasm-only at this landing, so this case PASSES on the
           default/wasm target and is TODO on rust / rust-async (the rust backend's Value emit is a later
           increment) — an additive per-target baseline, no regression.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #tuple(n (+ n 100)))) (Option (Tuple Int64 Int64)))
          ((Some m) (match m (#tuple(a c) (+ (* a 1000) c))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7107 Int64))
  (call main (: 3 Int64))
  (output (: 3103 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a STRING element of a compound"
  (doc
    "Extends the Int/Int round-trip to a `(Tuple String Int64)` — exercising the R2 value-form STRING leaf
           (a variable-length heap element, so the compound escapes via the descriptor-guided value-encode, not
           the fixed static template). `Value.decode (Value.encode (tuple \"hi\" n)) == Some (tuple \"hi\" n)`; the
           match asserts the string SURVIVED (equals `\"hi\"`) and returns `n`, else -1. `main 5` -> 5, `main 9` ->
           9 (two distinct outputs); a lost/garbled string element would give -1. Pins that the String leaf of the
           canonical value form round-trips faithfully — the rust encoder builds a `Leaf::Str`, the wasm
           `value-encode` op the same, and `codec::decode` recovers it. Runs on wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #tuple("hi" n))) (Option (Tuple String Int64)))
          ((Some m) (match m (#tuple(s k) (if (= s "hi") k (- 0 1)))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 9 Int64))
  (output (: 9 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a runtime-length LIST"
  (doc
    "Extends the round-trips to a `(List Int64)` — the R2 value-form `(list ..)` shape, a RUNTIME-length
           heap element. `Value.decode (Value.encode (list n (+ n 5) (+ n 10))) == Some [n, n+5, n+10]`; the
           match returns `len*1000 + first` to prove the LENGTH and element POSITIONS survive. `main 7` ->
           3*1000+7 = 3007, `main 2` -> 3002 (distinct); a truncated/garbled list would differ. Pins the List
           leaf of the native codec (the rust encoder loops the `Vec` into a `(list ..)`, decode rebuilds the
           `Vec`). Runs on wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #list(n (+ n 5) (+ n 10)))) (Option (List Int64)))
          ((Some l) (+ (* (List.len l) 1000) (Option.expect (List.at l 0) "first")))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 3007 Int64))
  (call main (: 2 Int64))
  (output (: 3002 Int64))
  (live-objects known-leak))

(case
  "a Value.encode/Value.decode round-trip preserves a RECORD's fields"
  (doc
    "Extends the round-trips to a `(Record (a Int64) (b Int64))` — the R2 value-form `(record (= k v) ..)`
           shape. `Value.decode (Value.encode (record (= a n) (= b (+ n 100)))) == Some` of the same record;
           the match projects both fields and folds `a*1000 + b` to prove each field survives in its own slot
           (a rust Record is a tuple in sorted-key order, so `a` before `b`). `main 7` -> record(a=7,b=107) ->
           7107, `main 3` -> 3103 (distinct); a dropped/swapped field would differ. Pins the Record leaf of the
           native codec (rust builds `(record (= a ..) (= b ..))`, decode rebuilds the tuple positionally).
           Runs on wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (:
            (Value.decode (Value.encode #record((= a n) (= b (+ n 100)))))
            (Option (Record (a Int64) (b Int64))))
          ((Some m) (+ (* m.a 1000) m.b))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7107 Int64))
  (call main (: 3 Int64))
  (output (: 3103 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a SUM's payload variant (Option Some)"
  (doc
    "Extends the round-trips to a `(Option Int64)` SUM — the R2 value-form `(Head payload)` shape, framed
           `(: (Some n) (Option Int64))`. `Value.decode (Value.encode (Some n)) == Some (Some n)`; the outer
           match unwraps the decode's `Option`, the inner match reads the round-tripped `Some k` and returns `k`.
           `main 5` -> 5, `main 9` -> 9 (distinct); a lost payload or wrong variant would give a negative. Pins
           the SINGLE-PAYLOAD sum arm of the native codec (rust `match`es the enum building `(Some <int>)`, decode
           dispatches on the head and reconstructs `Option::Some`). Runs on wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode (: (Some n) (Option Int64)))) (Option (Option Int64)))
          ((Some m) (match m ((Some k) k) ((None u) (- 0 1))))
          ((None u) (- 0 2))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 9 Int64))
  (output (: 9 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a SUM's NULLARY variant (Option None)"
  (doc
    "The nullary-variant face of the sum round-trip: `(: None (Option Int64))` renders `(: (None unit)
           (Option Int64))`. `Value.decode (Value.encode None) == Some None`; the outer match unwraps the decode,
           the inner match confirms the `None` head survived and returns `n` (else a negative). `main 4` -> 4,
           `main 8` -> 8 (distinct). Pins the NULLARY sum arm (rust builds `(None unit)`, decode dispatches the
           head to the bare/turbofish variant path). Runs on wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode (: None (Option Int64)))) (Option (Option Int64)))
          ((Some m) (match m ((Some k) (- 0 1)) ((None u) n)))
          ((None u) (- 0 2))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 4 Int64))
  (call main (: 8 Int64))
  (output (: 8 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a MULTI-PAYLOAD sum variant (spread)"
  (doc
    "Extends the sum round-trip to a MULTI-payload variant of a user sum `(type Shape (Circle Int64)
           (Rect Int64 Int64))`. `Rect`'s two payloads render FLAT under the head — `(Rect a b)`, a `Spread`,
           NOT `(Rect (tuple a b))` — framed `(: (Rect a b) Shape)`. `Value.decode (Value.encode (Rect n (+ n
           1))) == Some (Rect n (n+1))`; the inner match reads both payloads and folds `a*1000 + b` to prove
           each survives its own slot. `main 5` -> Rect(5,6) -> 5006, `main 3` -> 3004 (distinct); a lost/
           swapped payload would differ. Pins the multi-payload Spread arm (rust `match`es `Shape::Rect((a,b))`
           splicing a,b under the head; decode rebuilds the tuple + constructs the variant). wasm + rust + rust-async.")
  (input
    (do
      (type Shape (Circle Int64) (Rect Int64 Int64))
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode (: (Rect n (+ n 1)) Shape))) (Option Shape))
          ((Some s) (match s ((Rect a b) (+ (* a 1000) b)) ((Circle r) (- 0 1))))
          ((None u) (- 0 2))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5006 Int64))
  (call main (: 3 Int64))
  (output (: 3004 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a MAP's entries"
  (doc
    "Extends the round-trips to a `(Map Int64 Int64)` — the R2 value-form `(map (k v) …)` shape, entries
           in CANONICAL KEY order. `Value.decode (Value.encode (map {n: n*10, n+1: 99})) == Some` of the same
           map; the match looks up BOTH keys and folds `v(n)*1000 + v(n+1)` to prove each entry survives. `main
           7` -> {7:70, 8:99} -> 70099, `main 3` -> {3:30, 4:99} -> 30099 (distinct); a lost/reordered entry
           would differ. Pins the Map arm of the native codec: rust iterates the `BTreeMap` (already canonical
           order, no sort) building `(map (k v) …)`, decode rebuilds the `BTreeMap`. Runs on wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (:
            (Value.decode (Value.encode (Map.insert (Map.insert Map.empty n (* n 10)) (+ n 1) 99)))
            (Option (Map Int64 Int64)))
          ((Some m)
            (+
              (* (Option.expect (Map.lookup m n) "k1") 1000)
              (Option.expect (Map.lookup m (+ n 1)) "k2")))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 70099 Int64))
  (call main (: 3 Int64))
  (output (: 30099 Int64))
  (live-objects known-leak))

(case
  "a Value.encode/Value.decode round-trip preserves a SET's elements"
  (doc
    "Extends the round-trips to a `(Set Int64)` — the R2 value-form `((. Set of) (list e …))` shape,
           elements in CANONICAL order. `Set.of` takes a SINGLE `(List _)` arg (not variadic), so the set is
           `(Set.of (list n (n+5) (n+10)))`. `Value.decode (Value.encode set) == Some` of the same set; the
           match confirms a member survived (`Set.contains s (n+5)`) and folds `len*1000 + n`. `main 7` ->
           {7,12,17} -> 3007, `main 2` -> 3002 (distinct); a lost element would drop the len or fail contains.
           Pins the Set arm of the native codec: rust iterates the `BTreeSet` (already canonical order, no sort)
           building `((. Set of) (list …))`, decode rebuilds the `BTreeSet`. Runs on wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #set(n (+ n 5) (+ n 10)))) (Option (Set Int64)))
          ((Some s) (if (Set.contains s (+ n 5)) (+ (* (Set.len s) 1000) n) (- 0 1)))
          ((None u) (- 0 2))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 3007 Int64))
  (call main (: 2 Int64))
  (output (: 3002 Int64))
  (live-objects known-leak))

(case
  "a Value.encode/Value.decode round-trip preserves a FLOAT element of a compound"
  (doc
    "Extends the round-trips to a `(Tuple Int64 Float64)` — exercising the R2 value-form FLOAT leaf, an
           EXACT shortest-decimal `Leaf::Float(Decimal)` (NOT lossy bits). `Value.decode (Value.encode (tuple n
           2.5)) == Some (tuple n 2.5)`; the match asserts the FLOAT survived (`= b 2.5`, exact) and returns `n`,
           else -1. `main 7` -> 7, `main 3` -> 3 (distinct outputs prove the Int slot too); a lost/garbled float
           gives -1. Pins the Float leaf: rust builds `Leaf::Float(Decimal::from_f64 2.5)`, decode reconstructs
           the f64 by re-parsing the decimal — byte-identical to the wasm float_leaf. Runs on wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (:
            (Value.decode (Value.encode (: #tuple(n 2.5) (Tuple Int64 Float64))))
            (Option (Tuple Int64 Float64)))
          ((Some m) (match m (#tuple(a b) (if (= b 2.5) a (- 0 1)))))
          ((None u) (- 0 2))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a BigInt (arbitrary precision, multi-limb)"
  (doc
    "Extends the round-trips to a `BigInt` — the R2 value-form KIND_INT leaf, framed `(: <int> BigInt)`.
           `big = 9999999999 * n` EXCEEDS i64 for the first call (≈1e20 > 9.2e18), so it is a MULTI-LIMB
           bignum — exercising the arbitrary-precision path (rust encodes cdz_num::Big via its decimal string
           into a num_bigint Leaf::Int; decode rebuilds Big from the leaf's LE u32 limbs, NOT an i64 clamp).
           `Value.decode (Value.encode big) == Some big`; the match confirms `= b big` (BigInt equality) and
           returns `n`. `main 9999999999` -> 9999999999 (big ≈1e20), `main 7` -> 7 (distinct); an i64-truncated
           or garbled bignum would fail the equality. Byte-identical to the wasm value-encode. wasm+rust+rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          (((: big BigInt) (* (BigInt.of 9999999999) (BigInt.of n))))
          (match
            (: (Value.decode (Value.encode big)) (Option BigInt))
            ((Some b) (if (= b big) n (- 0 1)))
            ((None u) (- 0 2)))))
      (export main)))
  (call main (: 9999999999 Int64))
  (output (: 9999999999 Int64))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (live-objects known-leak))

(case
  "a Value.encode/Value.decode round-trip preserves a Rational (normalized num/den name leaf)"
  (doc
    "Extends the round-trips to a `Rational` — the R2 value-form is a SINGLE NAME leaf whose text is
           `num/den` NORMALIZED (lowest terms, sign on num, den>0), framed `(: <num>/<den> Rational)` — NOT a
           record, NOT the 2-handle heap node. `Rational.of n 8` NORMALIZES on construct, so `main 6` builds
           6/8 = 3/4 (a REDUCED form — catches a normalize mismatch), `main 5` = 5/8 (already reduced).
           `Value.decode (Value.encode r) == Some r`; the match confirms `= r (Rational.of n 8)` (Rational
           equality) and returns `n`. `main 6` -> 6, `main 5` -> 5 (distinct). rust encodes via
           Rational::to_display_string (num/den) into a Name leaf, decode splits on `/` and rebuilds via
           Rational::new (re-normalizes). Byte-identical to the wasm value-encode. wasm + rust + rust-async.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode (: (Rational.of n 8) Rational))) (Option Rational))
          ((Some r) (if (= r (Rational.of n 8)) n (- 0 1)))
          ((None u) (- 0 2))))
      (export main)))
  (call main (: 6 Int64))
  (output (: 6 Int64))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (live-objects known-leak))

; --- High-coverage value-codec gaps (operator-directed): the scalar leaves + edges the round-trips above
; do not yet pin — BOOL, a NESTED compound (a tuple inside a record field, exercising the codec recursion
; through a FieldPair whose value is a ctor-leaf node), and the EMPTY-collection edge. Each is a
; `Value.decode (Value.encode v) == Some v` round-trip through the canonical ctor-leaf codec (encode_value's
; KIND_*_CTOR / KIND_BOOL path). Runs on wasm (the rust Value emit is a later increment — additive baseline). ---
(case
  "a Value.encode/Value.decode round-trip preserves a BOOL element of a compound"
  (doc
    "Extends the round-trips to a `(Tuple Int64 Bool)` — the R2 value-form KIND_BOOL leaf. `Value.decode
           (Value.encode (tuple n (> n 0))) == Some (tuple n (> n 0))`; the match reads the bool back and returns
           `n` when it is true, `(- 0 n)` when false, so the two calls discriminate: `main 7` -> true -> 7;
           `main -3` -> false -> 3. A dropped/garbled Bool leaf would not discriminate. Pins the Bool leaf of
           the canonical value codec round-trips faithfully.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #tuple(n (> n 0)))) (Option (Tuple Int64 Bool)))
          ((Some m) (match m (#tuple(k b) (if b k (- 0 k)))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: -3 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a NESTED compound (a tuple inside a record field)"
  (doc
    "The codec RECURSION edge: a `(Record (p (Tuple Int64 Int64)))` — a compound NESTED inside a record
           field, so the encoder emits a FieldPair whose value is itself a `#tuple(..)` ctor-leaf node and the
           decoder rebuilds it. `Value.decode (Value.encode (record (= p (tuple n (+ n 1))))) == Some (...)`;
           reads back field `p`'s tuple and returns `(+ (* a 100) b)`. `main 5` -> 5*100+6 = 506; `main 8` ->
           809. Pins that the value serialization recurses through a nested compound faithfully (not just flat
           scalar leaves) — the depth guarantee the canonical ctor-leaf codec must hold.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (:
            (Value.decode (Value.encode #record((= p #tuple(n (+ n 1))))))
            (Option (Record (: p (Tuple Int64 Int64)))))
          ((Some m) (match m (#record((= p pr)) (match pr (#tuple(a b) (+ (* a 100) b))))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 506 Int64))
  (call main (: 8 Int64))
  (output (: 809 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves the EMPTY list (zero-element collection edge)"
  (doc
    "The empty-collection edge: `Value.decode (Value.encode (: #list() (List Int64))) == Some []` — a
           zero-element `(list)` value form (a length header of 0). Returns `(List.len l)` = 0 when the
           round-tripped list is empty. A codec that mishandled the zero-element case (a bad length header, or
           an off-by-one that fabricated an element) would fault or return a non-empty list. Pins that an empty
           collection round-trips through the canonical codec.")
  (input
    (do
      (def
        (main)
        (match
          (: (Value.decode (Value.encode (: #list() (List Int64)))) (Option (List Int64)))
          ((Some l) (List.len l))
          ((None u) (- 0 1))))
      (export main)))
  (call main)
  (output (: 0 Int64))
  (live-objects known-leak))

; --- High-coverage value-codec gaps, slice 2 (operator-directed): the remaining SCALAR leaves (CHAR /
; BYTES / SYMBOL) + the other EMPTY-collection edges (map / set). Each a `Value.decode (Value.encode v)
; == Some v` round-trip through the canonical ctor-leaf codec (encode_value's KIND_CHAR / KIND_STR-bytes /
; KIND_SYM / KIND_*_CTOR paths). Runs on wasm. ---
(case
  "a Value.encode/Value.decode round-trip preserves a CHAR element of a compound"
  (doc
    "Extends the round-trips to a `(Tuple Int64 Char)` — the R2 value-form KIND_CHAR leaf (a Unicode scalar).
           `Value.decode (Value.encode (tuple n #\\A)) == Some (tuple n #\\A)`; the match returns `n` when the
           char round-tripped as `#\\A`, else -1. `main 5` -> 5, `main 8` -> 8. A dropped/garbled Char leaf
           would give -1. Pins the Char leaf of the canonical codec round-trips faithfully.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #tuple(n #\A))) (Option (Tuple Int64 Char)))
          ((Some m) (match m (#tuple(k c) (if (= c #\A) k (- 0 1)))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 8 Int64))
  (output (: 8 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a BYTES element of a compound"
  (doc
    "Extends the round-trips to a `(Tuple Int64 Bytes)` — the R2 value-form Bytes leaf (a variable-length
           byte string). `Value.decode (Value.encode (tuple n b\"ABC\")) == Some (tuple n b\"ABC\")`; returns `n`
           when the bytes survived as `b\"ABC\"`, else -1. `main 4` -> 4, `main 6` -> 6. Pins the Bytes leaf of
           the canonical codec round-trips faithfully.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #tuple(n b"ABC"))) (Option (Tuple Int64 Bytes)))
          ((Some m) (match m (#tuple(k v) (if (= v b"ABC") k (- 0 1)))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 4 Int64))
  (call main (: 6 Int64))
  (output (: 6 Int64))
  (live-objects known-leak))

(case
  "a Value.encode/Value.decode round-trip preserves a SYMBOL element of a compound"
  (doc
    "Extends the round-trips to a `(Tuple Int64 Symbol)` — the R2 value-form KIND_SYM leaf (a `#\"…\"` symbol
           literal). `Value.decode (Value.encode (tuple n #\"kg\")) == Some (tuple n #\"kg\")`; returns `n` when
           the symbol round-tripped as `#\"kg\"`, else -1. `main 3` -> 3, `main 7` -> 7. Pins the Symbol leaf of
           the canonical codec round-trips faithfully.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #tuple(n #"kg"))) (Option (Tuple Int64 Symbol)))
          ((Some m) (match m (#tuple(k s) (if (= s #"kg") k (- 0 1)))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (live-objects known-leak))

(case
  "a Value.encode/Value.decode round-trip preserves the EMPTY map (zero-entry collection edge)"
  (doc
    "The empty-map edge: `Value.decode (Value.encode (: (Map.empty) (Map Int64 Int64))) == Some {}` — a
           zero-entry `(map)` value form. Returns `(Map.len m)` = 0 when the round-tripped map is empty. Pins
           that an empty map round-trips through the canonical codec (a mishandled zero-entry header would
           fault or fabricate an entry).")
  (input
    (do
      (def
        (main)
        (match
          (:
            (Value.decode (Value.encode (: (Map.empty) (Map Int64 Int64))))
            (Option (Map Int64 Int64)))
          ((Some m) (Map.len m))
          ((None u) (- 0 1))))
      (export main)))
  (call main)
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a Value.encode/Value.decode round-trip preserves the EMPTY set (zero-element collection edge)"
  (doc
    "The empty-set edge: `Value.decode (Value.encode (: (Set.of #list()) (Set Int64))) == Some {}` — a
           zero-element set value form (`Set.of` over the empty list). Returns `(Set.len s)` = 0 when the
           round-tripped set is empty. Pins that an empty set round-trips through the canonical codec.")
  (input
    (do
      (def
        (main)
        (match
          (: (Value.decode (Value.encode (: (Set.of #list()) (Set Int64)))) (Option (Set Int64)))
          ((Some s) (Set.len s))
          ((None u) (- 0 1))))
      (export main)))
  (call main)
  (output (: 0 Int64))
  (live-objects known-leak))

; --- The round-trip under LET-BINDER grounding: decode's target fixed by the binder annotation, not inline. ---
(case
  "a Value.decode round-trip grounds its target from a LET-BINDER annotation, not only an inline ascription"
  (doc
    "The round-trip's SIBLING grounding path. Case 'a Value.encode then Value.decode round-trips ...'
           grounds the partial `Value.decode : forall a. Bytes -> Option a` from an INLINE ascription
           `(: (Value.decode ..) (Option T))`. But the property-testing idiom is usually to BIND the decoded
           value and then assert over it — `(let (((: r (Option T)) (Value.decode bs))) (match r ..))` — where
           `a` is fixed by the LET-BINDER's annotation on the binding pair, a DISTINCT typing path
           (`let_binder_annotation_ty` climbing the binder, vs the inline `annotation_context_ty`). This pins
           that the round-trip `Value.decode (Value.encode v) == Some v` holds identically under let-binder
           grounding: an ungrounded `a` at the decode node would DECLINE (no concrete descriptor to emit), so
           a passing run proves the binder annotation threaded `a := (Tuple Int64 Int64)` into the decode node.
           Same discriminating structure as the inline case — `(tuple n (+ n 100))` folded `(+ (* a 1000) c)`,
           n=7 -> 7107, n=4 -> 4104 — so both tuple elements are proven to survive in position through the
           bound-then-matched decode. wasm PASS; rust / rust-async TODO (Value emit is wasm-only), additive.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          (((: r (Option (Tuple Int64 Int64))) (Value.decode (Value.encode #tuple(n (+ n 100))))))
          (match r ((Some m) (match m (#tuple(a c) (+ (* a 1000) c)))) ((None u) (- 0 1)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7107 Int64))
  (call main (: 4 Int64))
  (output (: 4104 Int64))
  (live-objects 0))

; --- The NEGATIVE face: an UNGROUNDED decode has no determined target, so it declines (actionably). ---
(case
  "a bare Value.decode with no grounding annotation declines with an actionable unsolved-target message"
  (doc
    "The negative face of the two Value.decode round-trip cases above: where those ground the partial
           `Value.decode : forall a. Bytes -> Option a` from an inline ascription or a let-binder, an
           UNGROUNDED decode leaves `a` a free type variable. The compiler cannot pick a concrete value-form
           descriptor for an undetermined target, so it DECLINES rather than emit a bogus descriptor — and
           the decline carries an ACTIONABLE message telling the user to annotate the decode with its
           expected type. Here the target `Pt` is only IMPLIED by the `(Some (Pt.Mk r))` arm pattern, which
           does not thread a type into the decode node, so `a` stays unsolved. Pins decline-rather-than-
           miscompile for the ungrounded partial (reference-compiler.md #A Type Without A Boundary
           Representation Declines At The Boundary), and that the diagnostic is actionable (directs the fix:
           annotate the scrutinee / use a typed let-binder). Backend-uniform: the unsolved-target guard fires
           at shared lowering before any backend-specific representation choice, so it declines identically on
           wasm / rust / rust-async (the `(message ...)` pin holds on all three baselines).")
  (input
    (do
      (type Pt (Mk (Record (: x Int64) (: y Int64))))
      (def
        (dec (: bs Bytes))
        (match (Value.decode bs) ((Some p) (match p ((Pt.Mk r) (+ r.x r.y)))) ((None u) (- 0 1))))
      (export dec)))
  (declines (message "target type is unsolved")))

; --- Value codec: a SCALAR-ERASED single-ctor newtype now round-trips (the boxing increment). ---
; APPENDED by v-runtime (owner of the R2 value-encode/decode op) at file EOF per the file-22 append+ping
; protocol. This closes the gap the (Tuple Int64 Int64) round-trip case above documents ("a scalar-erased
; newtype still declines the emit, a later increment"): a single-FIELD single-ctor newtype over a bare
; scalar, e.g. (type Env (FireAfter Int64)), erases to Ty::Nominal{inner:Int64} = a bare i64 — NOT a heap
; handle. Value.encode used to DECLINE it; it now BOXES the erased scalar to a leaf (box-int) before the
; op, and Value.decode un-declines the symmetric scalar target (value-decode reconstructs the same boxed
; leaf, used directly as the Option Some payload). The canonical value form is the elided-head, root-
; ascribed (: <scalar> <Type>) — rep-independent (the ctor identity rides in the descriptor, not the
; document), matching the compiler's single-ctor ELISION.
(case
  "a Value.encode/Value.decode round-trip preserves a scalar-erased single-ctor newtype"
  (doc
    "The R2 round-trip over a SCALAR-ERASED single-ctor newtype `(type Env (FireAfter Int64))` — a
           single-FIELD single-ctor whose payload is a bare scalar, so it erases to `Ty::Nominal{inner:Int64}`
           (a bare i64, NOT a heap handle). This was the `Value.encode`/`Value.decode` gap the Int/Int tuple
           round-trip above flags as `a later increment`: encode now BOXES the erased scalar into a leaf
           handle (`box-int`) before `value-encode`, whose descriptor is the `Named(Env, Int)` frame, so the
           canonical form is the elided-head `(: n Env)` (rep-independent — the `FireAfter` ctor identity
           rides in the descriptor, not the document); decode reconstructs that same boxed leaf and uses it
           directly as the `Option` `Some` payload (a scalar `Option` payload boxes identically), so
           `Value.decode (Value.encode (FireAfter n)) == Some (FireAfter n)`. The match unwraps the inner
           scalar `k` and returns it; a lost/garbled round-trip or an all-`None` decode would give -1. Two
           distinct seeds pin a faithful round-trip: `main 10000000 -> 10000000`, `main 42 -> 42` (the
           payload survives the erase→box→encode→decode→unbox cycle in the right value). Runs at the boundary
           so the value is realized. TARGET COVERAGE: the `Value` boxing emit is wasm-only at this landing, so
           this PASSES on the default/wasm target and is TODO on rust / rust-async (the rust backend's `Value`
           emit is a later increment) — an additive per-target baseline, no regression.")
  (input
    (do
      (type Env (FireAfter Int64))
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode (FireAfter n))) (Option Env))
          ((Some m) (match m ((FireAfter k) k)))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 10000000 Int64))
  (output (: 10000000 Int64))
  (call main (: 42 Int64))
  (output (: 42 Int64)))

(case
  "a Value.encode/Value.decode round-trip of a NULLARY single-ctor newtype is Some(unit)"
  (doc
    "The nullary sibling of the scalar single-ctor round-trip: `(type Ack (Ack))` is a single nullary
           ctor, which erases to `Ty::Nominal{inner:Unit}` — NO machine value at all. `Value.encode` used to
           decline it; it now FRAMES the runtime inline-unit handle (`IMM_UNIT`) so the canonical form is the
           elided-head `(: unit Ack)` (its descriptor is the `Named(Ack, Unit)` frame). `Value.decode` into
           `Option Ack` reconstructs `imm_unit()` (non-NULL) and wraps it as `Some` — a `Some(unit)` carries
           that same immediate. Ack has no payload to garble, so the round-trip can only distinguish `Some`
           from `None`: the match returns the seed `n` on `Some` (proving the decode matched the encoded unit)
           and -1 on `None`. `main 5 -> 5`, `main 9 -> 9` (distinct); an all-`None` decode would give -1
           twice. Runs at the boundary (the runtime `value-encode`/`value-decode` ops execute — a compile-time
           fold is not built). TARGET COVERAGE: the `Value` boxing emit is wasm-only, so this PASSES on wasm
           and is TODO on rust / rust-async (additive baseline, no regression).")
  (input
    (do
      (type Ack (Ack))
      (def
        (main (: n Int64))
        (match (: (Value.decode (Value.encode (Ack))) (Option Ack)) ((Some m) n) ((None u) (- 0 1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 9 Int64))
  (output (: 9 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip of a NARROW signed-int single-ctor sign-extends correctly"
  (doc
    "Pins the NARROW-INT box path of the single-ctor boxing that the Int64 case above does NOT exercise:
           `(type Tag (Mk Int8))` erases to `Ty::Nominal{inner:Int8}` = a bare i32 SCALAR (a ≤32-bit int
           occupies an i32 slot). Value.encode must SIGN-extend it into the i64 `box-int` cell
           (`I64ExtendI32S`, not `I64ExtendI32U`) before boxing — a NEGATIVE seed proves the sign is
           preserved: a wrong zero-extend would turn -5 (i32 `0xFFFF_FFFB`) into 4294967291, giving a garbage
           round-trip. `Value.decode (Value.encode (Mk n)) == Some (Mk n)`; the match unwraps `k : Int8` and
           widens via `Int64.of` for the boundary. `main -5 -> -5`, `main 100 -> 100`; a lost round-trip gives
           -1. wasm-only (rust `Value` emit is a later increment) — additive baseline.")
  (input
    (do
      (type Tag (Mk Int8))
      (def
        (main (: n Int8))
        (match
          (: (Value.decode (Value.encode (Mk n))) (Option Tag))
          ((Some m) (match m ((Mk k) (Int64.of k))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: -5 Int8))
  (output (: -5 Int64))
  (call main (: 100 Int8))
  (output (: 100 Int64)))

(case
  "a Value.encode/Value.decode round-trip of a Float64 single-ctor uses box-float"
  (doc
    "Pins the FLOAT box path of the single-ctor boxing (a distinct box op from box-int): `(type Temp
           (Celsius Float64))` erases to `Ty::Nominal{inner:Float64}` = a bare f64 SCALAR. Value.encode boxes
           it via `box-float` (NOT box-int — no i32→i64 extend, the f64 is the op's arg directly), whose
           descriptor is `Named(Temp, Float)`, so the form is the elided-head `(: 3.5 Temp)`; decode
           reconstructs via `op_box_float`. `Value.decode (Value.encode (Celsius n)) == Some (Celsius n)`; the
           match unwraps `k : Float64` and returns it. `main 3.5 -> 3.5`, `main -2.25 -> -2.25` (both exactly
           representable, so the round-trip is byte-exact); a lost round-trip gives -1.0. wasm-only (rust
           `Value` emit is a later increment) — additive baseline.")
  (input
    (do
      (type Temp (Celsius Float64))
      (def
        (main (: n Float64))
        (match
          (: (Value.decode (Value.encode (Celsius n))) (Option Temp))
          ((Some m) (match m ((Celsius k) k)))
          ((None u) -1.0)))
      (export main)))
  (call main (: 3.5 Float64))
  (output (: 3.5 Float64))
  (call main (: -2.25 Float64))
  (output (: -2.25 Float64)))

(case
  "a Value.encode/Value.decode round-trip preserves a scalar-erased single-ctor newtype NESTED as a tuple element"
  (doc
    "Pins the COMPOSITION the top-level single-ctor cases do not cover: a scalar-erased single-ctor
           newtype `(type Env (FireAfter Int64))` used as a COMPOUND ELEMENT (a tuple element). The tuple's
           element-boxing boxes the erased newtype scalar (box_op_ty peels the nominal → box-int), the shape
           descriptor nests `Named(Env, Int)` as element[0], value-encode renders the element as the framed
           `(: n Env)` inside the `(tuple …)`, and decode reconstructs the boxed leaf as the tuple element.
           Platform-relevant: a contract type with a newtype-typed field/element must round-trip. The match
           unwraps the tuple, then the newtype element `e`, then its inner `d`, folding `d*1000 + k` to prove
           the newtype element survived in position: `main 7` → `(tuple (FireAfter 7) 8)` → 7*1000+8 = 7008; a
           dropped/garbled newtype element would differ or give -1. wasm-only (rust `Value` emit is a later
           increment) — additive baseline.")
  (input
    (do
      (type Env (FireAfter Int64))
      (def
        (main (: n Int64))
        (match
          (: (Value.decode (Value.encode #tuple((FireAfter n) (+ n 1)))) (Option (Tuple Env Int64)))
          ((Some m) (match m (#tuple(e k) (match e ((FireAfter d) (+ (* d 1000) k))))))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7008 Int64))
  (live-objects 0))

(case
  "a Value.encode/Value.decode round-trip preserves a scalar-erased single-ctor newtype NESTED as a record field"
  (doc
    "The record-field companion of the nested tuple-element case: a scalar-erased single-ctor newtype
           `(type Env (FireAfter Int64))` as a RECORD FIELD `at`. Records are positional heap arrays in
           canonical (key-sorted) field order at run time, so the field boxes + the descriptor nests
           `Named(Env, Int)` under the sorted field position exactly like the tuple element; value-encode
           renders `(record (= at (: n Env)) (= tag …))`, decode reconstructs it. The match reads `(. m at)`,
           unwraps the newtype's inner `d`, folds `d*1000 + tag`: `main 7` → `{at: FireAfter 7, tag: 9}` →
           7*1000+9 = 7009; a lost newtype field would differ or give -1. Confirms the newtype-as-field shape
           a platform contract record uses round-trips. wasm-only (rust `Value` emit is a later increment).")
  (input
    (do
      (type Env (FireAfter Int64))
      (def
        (main (: n Int64))
        (match
          (:
            (Value.decode (Value.encode #record((= at (FireAfter n)) (= tag (+ n 2)))))
            (Option (Record (: at Env) (: tag Int64))))
          ((Some m) (+ (* 1000 (match m.at ((FireAfter d) d))) m.tag))
          ((None u) (- 0 1))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7009 Int64))
  (live-objects 0))

; --- Ast IS a runtime value: the R2 codec round-trips it (operator challenge 2026-08-26). ---
; Ast is a built-in RECURSIVE SUM (Int(BigInt)/Float/Bool/Str/Name/List((List Ast))/Bytes) with a full
; runtime representation — NOT compile-time-only (compiler-primitives.md §3a: "the Ast sum has a runtime
; representation and Ast.encode works at runtime"). So a RUNTIME Ast value Value.encode/decode round-trips
; like any recursive sum (the descriptor closes the List-of-Ast recursion via a Ref). These pin that
; invariant so a userspace value that HOLDS an Ast (e.g. a contract descriptor with Ast-typed fields) is a
; valid runtime value. (A runtime param drives construction so it does NOT const-fold.)
(case
  "a runtime Ast value round-trips through the R2 value codec (Value.encode/decode)"
  (doc
    "Ast is just a recursive sum, so a RUNTIME Ast value has a runtime rep and Value.encode/decode
           round-trips it. `Value.decode (Value.encode (Ast.Bool b)) : Option Ast` is `Some` — the encoded
           value form is the sum's variant/payload, decoded back to the Ast node. `main true` → 1 (Some);
           an all-`None` decode would give 0. Uses `Ast.Bool` (a Bool payload) to keep the leaf simple —
           `Ast.Int` carries a `BigInt`, not an Int64. wasm-only (the rust `Value` emit is a later
           increment) — additive baseline.")
  (input
    (do
      (def
        (main (: b Bool))
        (match
          (: (Value.decode (Value.encode (Ast.Bool b))) (Option Ast))
          ((Some m) 1)
          ((None u) 0)))
      (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a runtime RECURSIVE Ast (a list of nodes) round-trips through the R2 value codec"
  (doc
    "The homoiconic/recursive case: `(Ast.List (list (Ast.Bool b) (Ast.Str \"x\")))` — an Ast whose
           payload is a `(List Ast)` of further Ast nodes — round-trips through Value.encode/decode to
           `Some`. Proves the codec handles the recursive Ast shape (the shape descriptor closes the
           List-of-Ast back-edge via a `Ref`, exactly as for any recursive sum), so an arbitrarily nested
           Ast value is a valid runtime value. `main true` → 1; an all-`None` decode → 0. wasm-only.")
  (input
    (do
      (def
        (main (: b Bool))
        (match
          (:
            (Value.decode (Value.encode (Ast.List #list((Ast.Bool b) (Ast.Str "x")))))
            (Option Ast))
          ((Some m) 1)
          ((None u) 0)))
      (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (live-objects 0))

; --- Value codec: a newtype-OVER-RECORD round-trips (the newtype-nominal × record composition). ---
; The RECORD leaf (:1196) and the scalar-erased newtype (:1424) are pinned above; this composes them — a
; single-ctor newtype WHOSE PAYLOAD is a record, `(type Pt (Mk (Record (: x Int64) (: y Int64))))`. The
; newtype erases to the record handle, so `Value.encode` sees the record (the Pt nominal riding in the
; descriptor, not the document) and `Value.decode` at `(Option Pt)` reconstructs it — both fields survive.
; Grounded two ways: an INLINE `(: (Value.decode ..) (Option Pt))` ascription and a LET-BINDER `(let (((:
; p (Option Pt)) (Value.decode ..))) ..)` (the two distinct grounding paths of :1196 and :1364, here over
; the newtype-over-record shape). A runtime param drives construction so nothing const-folds.
(case
  "a Value.encode/Value.decode round-trip preserves a newtype-over-RECORD nominal"
  (doc
    "The newtype-over-record composition: `(type Pt (Mk (Record (: x Int64) (: y Int64))))` encoded and
           decoded back at `(Option Pt)` via an INLINE ascription reconstructs the wrapped record — both
           fields survive. `main 7` builds `Pt.Mk (record (= x 7) (= y 107))`, round-trips, and the Some arm
           reads `(. r x) + (. r y)` = 114; a dropped field or a lost nominal would give a wrong value or the
           None arm. Composes the RECORD leaf (:1196) with the single-ctor newtype nominal (the descriptor
           carries Pt while the document is the erased record).")
  (input
    (do
      (type Pt (Mk (Record (: x Int64) (: y Int64))))
      (def
        (main (: n Int64))
        (let
          ((bs (Value.encode (Pt.Mk #record((= x n) (= y (+ n 100)))))))
          (match
            (: (Value.decode bs) (Option Pt))
            ((Some p) (match p ((Pt.Mk r) (+ r.x r.y))))
            ((None u) (- 0 1)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 114 Int64))
  (live-objects 0))

(case
  "a Value.decode round-trip through a let-binder preserves a newtype-over-RECORD nominal"
  (doc
    "The let-binder grounding sibling of the case above (the :1364 path over a newtype-over-record):
           the decode target `Pt` is fixed by a `(let (((: p (Option Pt)) (Value.decode bs))) ..)` binder
           annotation, not an inline ascription. The round-trip reconstructs the wrapped record identically —
           `main 7` → 114 — proving the binder-grounding path threads the newtype-over-record descriptor into
           the decode node. An ungrounded target would decline (no descriptor to emit), so a passing run
           proves the binder annotation fixed `a := Pt`.")
  (input
    (do
      (type Pt (Mk (Record (: x Int64) (: y Int64))))
      (def
        (main (: n Int64))
        (let
          ((bs (Value.encode (Pt.Mk #record((= x n) (= y (+ n 100)))))))
          (let
            (((: p (Option Pt)) (Value.decode bs)))
            (match p ((Some q) (match q ((Pt.Mk r) (+ r.x r.y)))) ((None u) (- 0 1))))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 114 Int64))
  (live-objects 0))

; Equality, ordering, and the observable-behavior projection — witnesses core-semantics.md
; #Equality And Ordering, #Floating-Point Equality Follows The Canonical Byte Form, #Observable
; Behavior, and #A Program That Terminates Ends In One Of Two Terminal Conditions. Results are
; (: <value> <Type>); observation of ordered host calls uses (host-calls ...).

(case "structural equality holds component-wise"
  (doc    "Witnesses core-semantics.md #Equality Is Structural.")
  (input  (= 3 3))
  (output (: true Bool)))

(case "negative zero is not equal to positive zero"
  (doc    "Witnesses core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form:
           -0.0 and 0.0 have distinct canonical byte forms, so they are not equal.")
  (input  (= -0.0 0.0))
  (output (: false Bool)))

(case "every not-a-number value is equal to every not-a-number value"
  (doc    "Witnesses core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form:
           all NaN values share one canonical byte form, so they compare equal. `Float64.nan` denotes the
           canonical not-a-number value of that width (options/code-shape/, deterministic-value-form.md).")
  (input  (= Float64.nan Float64.nan))
  (output (: true Bool)))

; A `nan` value carries its DECLARING float width — `Float64.nan` is a Float64, `Float32.nan` a Float32 —
; so a CROSS-WIDTH comparison between them (or against a finite float of the other width) is the same
; no-silent-promotion type error a cross-width FINITE comparison is (CDZ0301, numeric-model.md #Numeric
; Types Do Not Silently Promote). `nan` is not width-polymorphic: it must impose its own width on the
; unification exactly as `(: 1.5 Float64)` does, or a Float32-vs-Float64 comparison slips past the check
; the finite case is rejected by. (A SAME-width nan comparison is fine — the case above; only crossing the
; width is the error.)

(case "comparing a Float32 nan to a Float64 nan is a cross-width type error"
  (doc    "`(= Float32.nan Float64.nan)` compares a Float32 value with a Float64 value — distinct float
           types that do not silently unify (CDZ0301), exactly as the finite `(= (: 1.5 Float32) (: 1.5
           Float64))` is rejected. A `Float32.nan` is a Float32 and a `Float64.nan` is a Float64; their
           widths do not unify. Pins that a nan carries its declaring width into the comparison, not an
           unfixed width that would ground to whatever the other operand is.")
  (input  (= Float32.nan Float64.nan))
  (error  CDZ0301))

(case "comparing a Float32 nan to a Float64 finite value is a cross-width type error"
  (doc    "`(= Float32.nan (: 1.5 Float64))` — a Float32 nan against a Float64 finite value: cross-width,
           so CDZ0301, exactly as `(= (: 1.5 Float32) (: 1.5 Float64))` is. Pins that a nan on EITHER side
           still imposes its declaring float width on the unification (the finite-vs-finite path already
           does), so a mixed nan/finite cross-width comparison is caught, not run to a value.")
  (input  (= Float32.nan (: 1.5 Float64)))
  (error  CDZ0301))

; --- Float equality follows the canonical byte form RECURSIVELY, inside compound values --
; #Equality Is Structural: "Two values MUST be equal when they have the same type and their contents
; are equal component-wise" — and each float COMPONENT is compared by #Floating-Point Equality Follows
; The Canonical Byte Form (every NaN equal to every NaN; -0.0 distinct from 0.0). The scalar cases
; above pin the float rule at top level; these pin that structural equality applies the SAME rule to a
; float NESTED in a tuple/list/record/sum, not a naive f64.eq. This is the sharpest adversarial float
; case: a lowering that recurses into a compound with wasm's f64.eq gives the OPPOSITE answer for both
; NaN (f64.eq says nan≠nan → the tuples wrongly unequal) and -0.0 (f64.eq says -0.0=0.0 → wrongly
; equal). The seed's `cval_eq` recurses through `float_canonical_eq`, so it must match the scalar rule.

(case "a NaN nested in a tuple compares equal under the canonical byte form"
  (doc    "`(= (tuple Float64.nan) (tuple Float64.nan))` = true: structural equality compares the tuples
           component-wise (core-semantics.md #Equality Is Structural), and the float component follows the
           canonical-byte-form rule where every NaN equals every NaN — exactly as the scalar
           `(= Float64.nan Float64.nan)` does. A recursion using wasm's f64.eq would answer false (nan ≠
           nan); this pins the canonical-byte-form rule holds for a float INSIDE a compound.")
  (input  (= (tuple Float64.nan) (tuple Float64.nan)))
  (output (: true Bool)))

(case "a negative zero nested in a tuple is distinct from positive zero"
  (doc    "`(= (tuple -0.0) (tuple 0.0))` = false: the float components -0.0 and 0.0 have distinct
           canonical byte forms, so the tuples are unequal — the compound companion of the scalar
           `(= -0.0 0.0)` = false. A recursion using wasm's f64.eq would answer true (-0.0 = 0.0),
           silently collapsing the distinction the canonical byte form preserves.")
  (input  (= (tuple -0.0) (tuple 0.0)))
  (output (: false Bool)))

(case "identical negative zeros nested in a tuple compare equal"
  (doc    "The control the case above pairs with: `(= (tuple -0.0) (tuple -0.0))` = true — two -0.0
           components share one canonical byte form, so the tuples are equal. Confirms the nested
           comparison is a genuine value test (true for matching -0.0, false against 0.0), not a
           blanket answer.")
  (input  (= (tuple -0.0) (tuple -0.0)))
  (output (: true Bool)))

; The nested-equality cases above compare CONSTANT compounds (they fold). These pin the RUNTIME heap-walk
; through DEEP nesting: a compound built from a boundary parameter (so it cannot fold) compared component-
; wise down multiple levels — a record inside a tuple, and three tuple levels deep. The value-eq walk must
; descend to the runtime leaf and compare it, the shape a structural-equality check over a built IR node
; takes.

(case "a runtime record nested in a tuple compares component-wise"
  (doc    "`(= (tuple (record (x n) (y 2)) 5) (tuple (record (x 3) (y 2)) 5))` with `n` a boundary parameter
           — the tuples cannot fold, so the runtime `value-eq` walk descends: tuple element 0 is a record
           whose field `x` is the runtime `n`. n=3 → the records (hence tuples) are equal → true; n=9 →
           `x` differs → false. Pins that the structural-equality walk recurses through a RECORD nested in a
           TUPLE at run time (a heap value inside a heap value), comparing the runtime leaf.")
  (input  (do (def (main (: n Int64)) (= (tuple (record (x n) (y 2)) 5) (tuple (record (x 3) (y 2)) 5))) (export main)))
  (call   main (: 3 Int64)) (output (: true Bool))
  (call   main (: 9 Int64)) (output (: false Bool)))

(case "a runtime three-level nested tuple compares equal by a deep walk"
  (doc    "Three tuple levels deep: `(= (tuple 1 (tuple 2 (tuple n 4))) (tuple 1 (tuple 2 (tuple 3 4))))`
           with `n` a parameter. The `value-eq` walk descends all three levels to reach `n` — n=3 → equal
           at every level → true; n=9 → the innermost element differs → false. Pins that the deep structural
           walk reaches a leaf several nesting levels down at run time, not only one level.")
  (input  (do (def (main (: n Int64)) (= (tuple 1 (tuple 2 (tuple n 4))) (tuple 1 (tuple 2 (tuple 3 4))))) (export main)))
  (call   main (: 3 Int64)) (output (: true Bool))
  (call   main (: 9 Int64)) (output (: false Bool)))

(case "a NaN nested in a list compares equal under the canonical byte form"
  (doc    "The list companion: `(= (list Float64.nan 1.0) (list Float64.nan 1.0))` = true — element-wise
           equality compares nan against nan (equal, canonical byte form) and 1.0 against 1.0 (equal), so the
           lists are equal. Pins that the canonical-byte-form float rule recurses through list elements
           too, alongside an ordinary equal float element.")
  (input  (= (list Float64.nan 1.0) (list Float64.nan 1.0)))
  (output (: true Bool)))

(case "a NaN nested in a sum payload compares equal under the canonical byte form"
  (doc    "The sum companion: `(= (Some Float64.nan) (Some Float64.nan))` = true — the variant tags match
           (both Some) and the payloads compare by the canonical-byte-form rule where nan equals nan. Pins that
           structural equality applies the float rule to a Sum's payload, not only to tuple/list
           elements.")
  (input  (= (Some Float64.nan) (Some Float64.nan)))
  (output (: true Bool)))

(case "a negative zero in a record field is distinct from positive zero"
  (doc    "The record companion of the nested -0.0 case: `(= (record (x -0.0)) (record (x 0.0)))` =
           false — the field `x` holds -0.0 in one record and 0.0 in the other, distinct canonical byte
           forms, so the records are unequal. Pins the canonical-byte-form float distinction through a
           record field, the field-access analogue of the tuple-element case.")
  (input  (= (record (x -0.0)) (record (x 0.0))))
  (output (: false Bool)))

; Float64 equality is a REALIZED seed capability (options/realized-capability-set/: "Float64
; literals/equality"), so it must hold for a RUNTIME float operand — one from a function parameter,
; a call, an if — not only for two compile-time-constant literals. The cases above compare constant
; floats; these compare a runtime float against a constant. The seed emits only the CONSTANT float
; equality (folded at compile time) and declines a runtime one ("non-constant float equality
; (canonical byte form) not yet emitted") — a not-yet-emitted runtime path within a realized
; capability. The value itself is carried correctly (a runtime float identity `(f 3.5)` → 3.5); only
; the equality comparison of a runtime float is missing.

(case "runtime float equality compares by canonical byte form"
  (doc    "`f` takes a Float64 parameter and compares it to the literal 3.5; f(3.5) is true. Float
           equality is realized (options/realized-capability-set/), so it must apply to a runtime
           float operand, matching the canonical-byte-form comparison the constant cases above use.
           The seed declines (\"non-constant float equality … not yet emitted\") — it folds constant
           float equality but has not emitted the runtime comparison.")
  (input  (do
            (def (f x) (= x 3.5))
            (def (main) (f 3.5)) (export main)))
  (output (: true Bool)))

(case "runtime float inequality compares by canonical byte form"
  (doc    "The companion with an unequal runtime operand: f(2.5) compares 2.5 to 3.5 and is false.
           Confirms the runtime float comparison is a genuine value test (true for 3.5, false for
           2.5), not a constant fold. The seed declines the same way.")
  (input  (do
            (def (f x) (= x 3.5))
            (def (main) (f 2.5)) (export main)))
  (output (: false Bool)))

; --- Equality of two RUNTIME strings, neither a compile-time literal --------------------------
; core-semantics.md #Equality Is Structural + #String Equality Follows Normalized Contents: `=` on
; two String operands compares their normalized contents. The top-of-file string-equality cases
; (13-strings.sexp) compare two LITERAL strings, folded at compile time; a comparison in which AT
; LEAST ONE operand is a literal also folds (the compiler holds one side statically). The demanding
; shape is two operands that are BOTH runtime values — two function parameters, two payload-bound
; names — with no literal to fold against: a String is a Bytes-backed heap value, so comparing two of
; them is a runtime heap walk. The seed folds the literal cases but declines the two-runtime case
; ("runtime compound equality (heap walk) not yet emitted") — a not-yet-emitted runtime path within a
; realized capability, the String companion of the runtime-float-equality cases above. A program that
; compares names it read from data (a symbol table, an AST node's head, a proof term's variable name)
; hits exactly this; the recorded true/false is the oracle a generation that emits the heap-walk
; comparison reproduces.

(case "two runtime strings compare equal by their contents"
  (doc    "`eq2` compares its two String PARAMETERS — both runtime values, neither a literal the
           compiler can fold against. `(eq2 \"foo\" \"foo\")` is true. String equality is realized
           (collections-and-text.md #String Equality Follows Normalized Contents), so it must hold when
           BOTH operands are runtime, not only when one side is a literal (which folds). The seed
           declines (\"runtime compound equality (heap walk) not yet emitted\"): it folds a literal-side
           comparison but has not emitted the two-runtime heap walk. A program comparing two names read
           from data takes this shape.")
  (input  (do
            (def (eq2 a b) (= a b))
            (def (main) (eq2 "foo" "foo")) (export main)))
  (output (: true Bool)))

(case "two unequal runtime strings compare false by their contents"
  (doc    "The companion with unequal runtime operands: `(eq2 \"foo\" \"bar\")` is false. Confirms the
           two-runtime string comparison is a genuine content test, not a constant fold (true for equal
           contents, false for different). The seed declines the same way as the equal case.")
  (input  (do
            (def (eq2 a b) (= a b))
            (def (main) (eq2 "foo" "bar")) (export main)))
  (output (: false Bool)))

(case "a runtime string compared against a literal folds against the literal side"
  (doc    "The control the two cases above must be distinguished from: when ONE operand is a literal,
           the comparison folds against that side and the seed compiles it. `f` compares its String
           parameter to the literal \"x\"; `(f \"x\")` is true. Pins that the runtime-string equality
           gap is specifically the BOTH-runtime case — a literal on either side is already emitted.")
  (input  (do
            (def (f s) (= s "x"))
            (def (main) (f "x")) (export main)))
  (output (: true Bool)))

(case "a runtime string bound from a sum payload compares equal to a string parameter"
  (doc    "The two-runtime-string case above compares two direct PARAMETERS; this compares a String bound
           from a SUM-VARIANT PAYLOAD (`s` from `(Wrap.Wrap s)`) against a String parameter (`name`) —
           still two runtime operands with no literal to fold, but one is now a heap value extracted from a
           constructor payload rather than a bare parameter. `(payload-is (Wrap.Wrap \"foo\") \"foo\")` is
           true by String equality (collections-and-text.md #String Equality Follows Normalized Contents).
           A generation that emits the two-runtime heap walk for bare parameters but not for a
           payload-extracted operand declines here (\"runtime compound equality (heap walk) not yet
           emitted\") — the payload/aliased-operand companion of the two-parameter case; a program that
           compares a name it destructured from a data node against an expected name takes exactly this
           shape.")
  (input  (do
            (type Wrap (Wrap String))
            (def (payload-is w name) (match w ((Wrap.Wrap s) (= s name))))
            (def (main) (payload-is (Wrap.Wrap "foo") "foo")) (export main)))
  (output (: true Bool)))

; --- Equality of two RUNTIME compound values (a heap walk over the value heap) -----------------
; core-semantics.md #Equality Is Structural: two values are equal when they have the same type and
; their contents are equal component-wise; #Values Are Equal … agrees with the canonical byte form. The
; component-wise cases above compare compound values built from LITERALS (folded at compile time). The
; demanding shape is two compound values BUILT AT RUN TIME — a sum/record/tuple whose contents come
; from a parameter or a call — so the comparison is a walk over two heap values, not a constant fold.
; The compiler emits this as the runtime `value-eq` op (the tagless `champ_eq` walk the map/set key path
; already uses): equal iff same shape AND equal component-wise, discriminant before payload. It is
; realized for a compound whose LEAVES are all SCALAR (Int/Bool/Unit) — canonical by construction — so a
; program comparing two runtime AST nodes / proof terms / records for structural equality runs; a
; compound carrying a collection/text leaf (a List/Bytes/String, whose canonical form needs a
; compaction) still declines (a later increment). The first two cases below `(= (mk 1) (mk 1))` INLINE
; their tiny builder and fold to a constant (so they pass by the fold, not the walk); the recursion-built
; cases that follow defeat the fold and genuinely exercise `value-eq`.

(case "two runtime sum values compare equal by a heap walk"
  (doc    "`mk` builds a runtime sum `(N.I n)` from its parameter, so both operands of `(= (mk 1) (mk
           1))` are heap values, not folded constants. Structural equality (core-semantics.md #Equality
           Is Structural) makes them equal, so the program is true. The seed declines (\"runtime
           compound equality (heap walk) not yet emitted\"): it folds equality of compile-time-known
           compounds but has not emitted the runtime heap walk. The runtime-compound companion of the
           runtime-float and two-runtime-string equality cases above — all three are the same
           not-yet-emitted runtime comparison. A generation emitting the heap walk reproduces true.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (main) (if (= (mk 1) (mk 1)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "two differing runtime sum values compare unequal by a heap walk"
  (doc    "The companion with unequal runtime compounds: `(mk 1)` is `(N.I 1)` and `(mk2 2)` is `(N.I
           2)`, so the heap walk finds their payloads differ and the comparison is false → 0. Confirms
           the runtime compound comparison is a genuine structural test, not a constant fold. The seed
           declines the same way as the equal case.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (main) (if (= (mk 1) (mk 2)) 1 0)) (export main)))
  (output (: 0 Int64)))

(case "a runtime sum whose payload comes from recursion compares equal by a heap walk"
  (doc    "The GENUINELY non-foldable sum-equality shape — the two `mk`/`if`-shaped cases above inline
           their tiny builder and reduce to a CONSTANT compound the compiler folds, so they never reach
           the runtime `value-eq` path. Here one operand's payload is `(sumto 3)` = 3+2+1 = 6, a value
           produced by RECURSION the compiler cannot fold to a literal, so `(N.I (sumto 3))` is a genuine
           heap value; comparing it to `(N.I 6)` walks the two heap sums. Equal discriminant AND equal
           payload → true → 1 (core-semantics.md #Equality Is Structural). Pins that `=` emits the
           runtime structural comparison (`value-eq`), not only the compile-time fold — the observable
           of the heap walk the two cases above document but do not exercise.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (sumto n) (if (< n 1) 0 (+ n (sumto (- n 1)))))
            (def (main) (if (= (N.I (sumto 3)) (N.I 6)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "a runtime sum whose payload comes from recursion compares unequal by a heap walk"
  (doc    "The unequal companion of the recursion-built heap walk: `(sumto 3)` = 6, so `(N.I (sumto 3))`
           carries 6 while `(N.I 7)` carries 7 — the heap walk finds the payloads differ and the
           comparison is false → 0. Confirms `value-eq` is a genuine content test on the recursion-built
           (unfoldable) operand, not a fold that happened to say true. The discriminant agrees (both `I`),
           so this isolates the PAYLOAD comparison.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (sumto n) (if (< n 1) 0 (+ n (sumto (- n 1)))))
            (def (main) (if (= (N.I (sumto 3)) (N.I 7)) 1 0)) (export main)))
  (output (: 0 Int64)))

(case "two recursion-built linked lists compare equal by a deep heap walk"
  (doc    "The RECURSIVE-SUM heap walk: `build n` constructs a descending cons-list `[n, n-1, …, 1]`
           whose length and spine are decided at run time (no fixed literal spine to fold), so `(build
           3)` is a genuine multi-node heap value. `(= (build 3) (build 3))` walks BOTH cons-lists
           node-by-node — each `Cons` tuple's head and tail, recursively to `Nil` — and finds them
           structurally equal → 1. This is the deep-structure shape a self-hosted compiler comparing two
           AST subtrees takes; it CANNOT fold (the spine is runtime-built). Pins that `value-eq` recurses
           through a nested recursive sum, not just a one-level payload. Both operands are OWNED
           temporaries the borrowing compare must reclaim (no leak).")
  (input  (do
            (type IntList (Cons (Tuple Int64 IntList)) Nil)
            (def (build n) (if (< n 1) (IntList.Nil ())
                               (IntList.Cons (tuple n (build (- n 1))))))
            (def (main) (if (= (build 3) (build 3)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "two recursion-built linked lists of different lengths compare unequal by a heap walk"
  (doc    "The unequal companion of the deep list walk: `(build 3)` = `[3,2,1]` and `(build 2)` =
           `[2,1]` differ at the FIRST node (head 3 vs 2, and different spine length), so the heap walk
           returns false → 0 without needing to prove the whole structure. Confirms the recursive
           `value-eq` is a genuine structural comparison over the runtime-built spine, not a fold.")
  (input  (do
            (type IntList (Cons (Tuple Int64 IntList)) Nil)
            (def (build n) (if (< n 1) (IntList.Nil ())
                               (IntList.Cons (tuple n (build (- n 1))))))
            (def (main) (if (= (build 3) (build 2)) 1 0)) (export main)))
  (output (: 0 Int64)))

(case "two runtime sums with the same payload but different variants compare unequal by a heap walk"
  (doc    "The discriminant half of the runtime heap walk: `pick` builds `(N.I n)` or `(N.J n)` from a
           runtime boolean, so `(pick true 5)` = `(N.I 5)` and `(pick false 5)` = `(N.J 5)` are two
           genuine heap sums carrying the SAME payload 5 under DIFFERENT variants. The heap walk compares
           the discriminant BEFORE the payload (core-semantics.md #Equality Is Structural), so they are
           unequal → 0 even though their payloads match. Pins that runtime `value-eq` — like the constant
           fold — distinguishes `I 5` from `J 5`; an implementation comparing only payloads would wrongly
           report equal.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (pick b n) (if b (N.I n) (N.J n)))
            (def (main) (if (= (pick true 5) (pick false 5)) 1 0)) (export main)))
  (output (: 0 Int64)))

(case "two runtime tuples compare equal by a heap walk"
  (doc    "The TUPLE companion of the runtime sum heap walk: `mk` builds `(tuple n (+ n 1))` from its
           parameter, so `(mk 3)` = `(tuple 3 4)` is a runtime heap tuple, not a folded constant.
           `(= (mk 3) (mk 3))` walks both tuples element-wise and finds them equal → 1. Pins that
           `value-eq` handles a runtime tuple (a positional product) the same as a sum — the structural
           equality is over ANY compound, not sum-specific.")
  (input  (do
            (def (mk n) (tuple n (+ n 1)))
            (def (main) (if (= (mk 3) (mk 3)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "two runtime records compare equal by a heap walk"
  (doc    "The RECORD companion: `mk` builds `(record (x n) (y (+ n 1)))` from its parameter, a runtime
           heap record. `(= (mk 3) (mk 3))` walks both by field and finds them equal → 1. Records
           canonicalize their field order before the walk (deterministic-value-form.md #A Value Has One
           Canonical Byte Form), so the comparison is over the canonical form, not the written order.
           Together with the tuple and sum cases this pins runtime `value-eq` across every scalar-leaf
           compound shape.")
  (input  (do
            (def (mk n) (record (x n) (y (+ n 1))))
            (def (main) (if (= (mk 3) (mk 3)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "a CONSTANT recursive sum compares equal to a differently-built RUNTIME one"
  (doc    "A mixed-provenance equality: the LEFT operand `(S (S Z))` is a COMPILE-TIME-CONSTANT recursive
           `Nat`, the RIGHT `(mk k)` is a RUNTIME-built spine of the same shape (`mk` recurses `k` times).
           `value-eq` must reconcile a folded constant sum with a heap-walked runtime one — the const side
           has a statically-known spine, the runtime side is discovered variant-by-variant. At `k = 2` both
           are `S(S(Z))` → equal → 1; at `k = 3` the runtime spine is one deeper → unequal → 0 (the
           companion case). Pins that structural equality composes a CONSTANT operand with a RUNTIME operand
           over a recursive sum, not only two runtime operands.")
  (input  (do
            (type Nat (Z) (S Nat))
            (def (mk (: n Int64)) (if (> n 0) (Nat.S (mk (- n 1))) (Nat.Z)))
            (def (main (: k Int64)) (if (= (Nat.S (Nat.S (Nat.Z))) (mk k)) 1 0))
            (export main)))
  (needs  sum-type-declaration)
  (call   main (: 2 Int64))
  (output (: 1 Int64)))

(case "two runtime Ok values of a multi-parameter sum compare equal by a heap walk"
  (doc    "The MULTI-PARAMETER-sum companion: `Result` has TWO type parameters (`Ok a`, `Err b`), and
           `(Ok (sumto 3))` fixes only `a = Int64` — the `Err` parameter `b` is a PHANTOM no value here
           instantiates. `(= (Ok (sumto 3)) (Ok 6))` builds both operands from recursion (unfoldable), so
           the runtime `value-eq` heap walk runs; the two `Ok` values carry equal Int64 payloads → 1. Pins
           that an UNCONSTRAINED type parameter of a SIBLING variant (`Err b`) does not block the walk: a
           phantom parameter carries no runtime structure, so it is scalar-safe. A generation that walked
           every variant's payload type and rejected the free `b` declined this though the compared `Ok`
           values are exactly walkable — the walkability check must admit a bare unconstrained variable.")
  (input  (do
            (def (sumto n) (if (< n 1) 0 (+ n (sumto (- n 1)))))
            (def (main) (if (= (Ok (sumto 3)) (Ok 6)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "two differing runtime Ok values of a multi-parameter sum compare unequal by a heap walk"
  (doc    "The unequal companion: `(sumto 3)` = 6, so `(Ok (sumto 3))` carries 6 while `(Ok 7)` carries 7
           — the heap walk finds the payloads differ and the comparison is false → 0. Confirms the
           phantom-`Err`-parameter `Result` comparison is a genuine content test, not a fold that
           happened to say true.")
  (input  (do
            (def (sumto n) (if (< n 1) 0 (+ n (sumto (- n 1)))))
            (def (main) (if (= (Ok (sumto 3)) (Ok 7)) 1 0)) (export main)))
  (output (: 0 Int64)))

(case "a runtime sum equality drives a tail-recursive loop"
  (doc    "The runtime heap walk `=` used as the CONDITION of a tail-recursive function that compiles to a
           wasm LOOP: `find` searches upward from 0 for the `n` whose `(N.I n)` equals `(N.I 3)`, so the
           comparison runs each iteration and the else-branch `(find (+ n 1))` iterates. `find(0)` = 3. This
           pins that a runtime `value-eq` in a loop's condition COMPOSES with the loop's own scratch: the
           i32 heap-handle slots the compare stashes must not collide with the i64 arithmetic slot the
           `(+ n 1)` iteration uses — the sibling branches must allocate their scratch ABOVE the condition's
           high-water. A generation that reused the condition's handle slot for the branch's arithmetic
           emitted an invalid module (`expected i64, found i32`); this exercises the branch-scratch
           discipline that keeps a heap-handle condition and a scalar branch in one function well-typed.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (find n) (if (= (mk n) (mk 3)) n (find (+ n 1))))
            (def (main) (find 0)) (export main)))
  (output (: 3 Int64)))

(case "a runtime sum match drives a tail-recursive loop"
  (doc    "The `match` companion of the value-eq-in-a-loop case: a runtime sum MATCH (built by `bump`, so
           it does not fold to a scalar compare) is the CONDITION of the tail-recursive `find`, and the
           else-branch `(find (+ n 1))` iterates the loop. `find(0)` = 3. Pins the same branch-scratch
           discipline for a `MatchSum` condition — its i32 scrutinee-handle slot must not collide with the
           i64 iteration arithmetic — which a folding match (`(match (N.I n) …)` reducing to `n == 3`) would
           never exercise. `bump` keeps the scrutinee a genuine heap value, so the match is a real runtime
           dispatch in the loop condition.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (bump n) (if (< n 0) (N.J n) (N.I n)))
            (def (find n) (if (match (bump n) ((N.I x) (= x 3)) ((N.J _) false))
                              n (find (+ n 1))))
            (def (main) (find 0)) (export main)))
  (output (: 3 Int64)))

(case "a guarded wildcard arm falls through to a tail-recursive call"
  (doc    "A `match` whose FIRST arm is a GUARDED WILDCARD (`(guard x <cond>)`) and whose fall-through arm
           TAIL-CALLS the enclosing function, compiled as a wasm LOOP: `find` returns `n` once `(> n 2)`
           holds, else `(find (+ n 1))` iterates. `find(0)` = 3. A guarded wildcard emits `if <guard>
           <body> else <fall-through>` with NO separate probe test (a wildcard needs none), so the guard's
           `if` is the ONLY block its body and fall-through nest inside. A generation that counted a
           (non-existent) probe `if` too made the fall-through's self-tail-call `br` one level too far —
           PAST the loop — producing an invalid module (`expected i64 but nothing on stack`). Pins that a
           guarded-wildcard arm's block nesting is exactly its guard `if`, so a tail call in its
           fall-through iterates the loop rather than escaping it.")
  (input  (do
            (def (find n) (match n ((guard x (> x 2)) x) (_ (find (+ n 1)))))
            (def (main) (find 0)) (export main)))
  (output (: 3 Int64)))

(case "a value-eq guard on a wildcard arm drives a tail-recursive loop"
  (doc    "The heap-handle companion of the guarded-wildcard loop case: the guard is a runtime `value-eq`
           (`(= (mk x) (mk 3))`, `mk` building genuine sum values), so BOTH fixes compose — the guard's i32
           handle scratch must sit above the fall-through's i64 iteration arithmetic (the branch-scratch
           discipline), AND the guarded-wildcard block nesting must be exactly one `if` (the tail-depth
           discipline). `find(0)` = 3. This is the exact shape a proof/AST search takes: scan upward,
           returning when a structural equality on a constructed term holds, else recurse.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (find n) (match n ((guard x (= (mk x) (mk 3))) x) (_ (find (+ n 1)))))
            (def (main) (find 0)) (export main)))
  (output (: 3 Int64)))

(case "a runtime sum equality as a match SCRUTINEE drives a tail-recursive loop"
  (doc    "The runtime heap walk `=` used as the SCRUTINEE of a `match` (a Bool the arms dispatch on),
           inside a tail-recursive loop: `find` matches `(= (mk n) (mk 3))` — `true` returns `n`, `false`
           iterates `(find (+ n 1))`. `find(0)` = 3. The scrutinee is a COMPUTED value (a value-eq, not a
           bare local), so it is evaluated ONCE into a slot; its i32 heap-handle scratch must not be reused
           by the arm bodies' i64 iteration arithmetic — the probe chain starts ABOVE the scrutinee emit's
           high-water, not a bare `base+1`. A generation that fixed the probe floor at `base+1` reused a
           value-eq handle slot for the branch arithmetic (`expected i64, found i32`).")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (find n) (match (= (mk n) (mk 3)) (true n) (false (find (+ n 1)))))
            (def (main) (find 0)) (export main)))
  (output (: 3 Int64)))

(case "a value-eq guard on a LITERAL-probe arm drives a tail-recursive loop"
  (doc    "The literal-probe companion of the guarded-wildcard loop case: the first arm is `(guard 3 <cond>)`
           — a LITERAL probe (`n == 3`) AND a runtime `value-eq` guard — with a fall-through that iterates.
           `find(0)` climbs until `n == 3`, where the guard `(= (mk n) (mk 3))` also holds, returning 300.
           A literal-probe-plus-guard nests `if (n==3) { if <guard> body else <fall> } else <fall>` — the
           guard's i32 handle scratch (in the THEN) types a slot the OUTER probe-else's i64 iteration
           arithmetic must not reuse (the two `if` branches share one function-global local declaration).
           Pins that the probe-else starts scratch above the THEN's high-water — the same discipline the
           `if`-condition and guarded-wildcard cases exercise, here at the literal-probe/guard seam.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (find n) (match n ((guard 3 (= (mk n) (mk 3))) 300) (_ (find (+ n 1)))))
            (def (main) (find 0)) (export main)))
  (output (: 300 Int64)))

(case "a value-eq guard on a SUM-match arm drives a tail-recursive loop"
  (doc    "The sum-match-decision-tree companion: the scrutinee is a genuine heap SUM (`(bump n)`, a call so
           it does not fold), matched by a variant pattern `(N.I x)` with a runtime `value-eq` GUARD, and a
           fall-through arm that iterates. `find(0)` climbs until `x == 3`. The decision tree emits `if
           (sum-disc == I) { if <guard> body else <fall> } else <fall>`; the guard's i32 handle scratch (in
           the disc-matched THEN) types a slot the disc-switch's ELSE fall-through i64 iteration arithmetic
           must not reuse. Pins the branch-scratch discipline at the SUM-match seam (`emit_sum_cont`'s
           guarded-arm + disc-switch), distinct from the scalar-match probe chain: the fall-through of BOTH
           the guard `if` and the disc-switch `if` must clear the arm's heap-handle high-water.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (bump n) (if (< n 0) (N.J n) (N.I n)))
            (def (mk n) (N.I n))
            (def (find n) (match (bump n)
                            ((guard (N.I x) (= (mk x) (mk 3))) x)
                            (_ (find (+ n 1)))))
            (def (main) (find 0)) (export main)))
  (output (: 3 Int64)))

(case "two constant sums with the same payload but different variants are not equal"
  (doc    "Constant compound equality folds STRUCTURALLY (core-semantics.md #Equality Is Structural), and
           structural equality compares the VARIANT before the payload: `(= (Ok 1) (Err 1))` is FALSE even
           though both carry the payload 1, because `Ok` and `Err` are different variants. Pins the
           discriminant half of the fold — an implementation that compared only payloads (a heap walk that
           skipped the variant tag) would wrongly report true here, conflating `Ok 1` and `Err 1`. The
           companion of `(= (Ok 1) (Ok 1))` = true: same variant AND same payload.")
  (input  (= (Ok 1) (Err 1)))
  (output (: false Bool)))

(case "two constant records with the same fields in different written order are equal"
  (doc    "Constant record equality folds structurally and compares fields as a SET keyed by name, not by
           written order: `(= (record (x 1) (y 2)) (record (y 2) (x 1)))` is true — both denote the same
           value (a record's canonical form sorts its fields by key, deterministic-value-form.md #A Value
           Has One Canonical Byte Form). Pins that the equality fold normalizes field order before
           comparing, so the same record written two ways is one value — not a position-wise comparison
           that would call these unequal.")
  (input  (= (record (x 1) (y 2)) (record (y 2) (x 1))))
  (output (: true Bool)))

(case "a runtime compound structural equality is expressible as a hand-written recursive comparator"
  (doc    "The route around the not-yet-emitted heap walk, and the shape a program needing runtime
           compound equality writes today: an explicit recursive comparator that dispatches on each
           value's variant and compares the leaves with scalar `=` (which IS emitted for runtime
           scalars). `same` compares two `N` values by matching both and comparing the bound Int64
           payloads; `(same (mk 1) (mk 1))` is true → 1. Pins that structural equality of runtime
           compounds is ALREADY achievable by hand — the missing built-in `=` heap walk is a
           convenience over this, not a new expressive power — so a program (a proof kernel comparing
           terms, a compiler comparing AST nodes) is not blocked, only more verbose.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (mk n) (N.I n))
            (def (same a b)
              (match a
                ((N.I x) (match b ((N.I y) (= x y)) ((N.J _) false)))
                ((N.J x) (match b ((N.J y) (= x y)) ((N.I _) false)))))
            (def (main) (if (same (mk 1) (mk 1)) 1 0)) (export main)))
  (output (: 1 Int64)))

(case "an offered ordering is total and deterministic"
  (doc    "Witnesses core-semantics.md #Ordering Where Offered Is Total: Int64 offers a total order.")
  (input  (< 2 3))
  (output (: true Bool)))

(case "an entrypoint returning a comparison presents a Bool result at the boundary"
  (doc    "Type-directed emission at the component boundary: a nullary `main` whose body is an Int64
           comparison has result type Bool, so the `run` export is framed at the Bool boundary valtype,
           not the s64 an arithmetic result would use. `(lt 20 22)` is true. The companion below returns
           the arithmetic i64 (42) through the SAME entrypoint shape, so the pair pins that the boundary
           result type tracks the program's RESULT TYPE — a comparison crosses as Bool, an arithmetic
           expression as Int64 — rather than a fixed valtype. This is the observable of a compiler that
           reads a program's result kind and frames `run` accordingly; the result kind is one of a fixed
           set (Int64 / Bool), selected by the operator that produces the result (a comparison yields
           Bool, `+`/`-`/`*`/`/`/`%` yield Int64).")
  (input  (do
            (def (lt a b) (< a b))
            (def (main)   (lt 20 22)) (export main)))
  (output (: true Bool)))

(case "an entrypoint returning arithmetic presents an Int64 result at the boundary"
  (doc    "The Int64 companion to the Bool-boundary case above: the same nullary-`main`-calls-a-helper
           shape, but the body is an arithmetic expression whose result type is Int64, so `run` is framed
           at the Int64 boundary valtype and `(add 20 22)` crosses as 42. Together the two cases pin that
           the entrypoint's boundary result type is type-directed — Bool for a comparison, Int64 for
           arithmetic — the same program shape emitting a different boundary type from its result type
           alone.")
  (input  (do
            (def (add a b) (+ a b))
            (def (main)    (add 20 22)) (export main)))
  (output (: 42 Int64)))

; --- Bool offers a total order in which false is less than true --------------------------
; core-semantics.md #Ordering Where Offered Is Total, 3rd sentence: "The Bool type MUST offer a
; total order in which false is less than true." Bool is not only an equality type — it carries a
; definite order with false below true, the conventional boolean ranking. These pin the direction of
; the order (false < true, not the reverse) and that all four ordering operators observe it, so a
; comparison of two Bools yields the ordered result rather than declining or treating Bool as
; unordered. The seed declines Bool ordering ("non-integer operand to integer op") — it emits the
; ordering operators only for Int64 — so a generation that does not yet emit the Bool comparison
; declines rather than running (reject-don't-miscompile); the gate scores the decline as todo, not
; disagreement, and the requirement marks the emission a later generation adds.

(case "false is less than true"
  (doc    "Witnesses core-semantics.md #Ordering Where Offered Is Total (Bool clause): `(< false
           true)` is true because false is the lesser of the two Bool values — the direction of the
           Bool order.")
  (input  (< false true))
  (output (: true Bool)))

(case "true is not less than false"
  (doc    "The complement fixing the order's direction: `(< true false)` is false, because true is
           not below false. Together with the case above this pins false < true rather than the
           reverse ranking, so the order is antisymmetric in the specified direction.")
  (input  (< true false))
  (output (: false Bool)))

(case "true is greater than false"
  (doc    "The `>` companion: `(> true false)` is true, the mirror of `(< false true)`. Pins that the
           strict greater-than operator observes the same Bool order.")
  (input  (> true false))
  (output (: true Bool)))

(case "a boolean is less-than-or-equal to itself"
  (doc    "`(<= false false)` is true: the inclusive ordering operator is reflexive on Bool, as a
           total order requires. Pins `<=` on equal Bool operands.")
  (input  (<= false false))
  (output (: true Bool)))

(case "a boolean is greater-than-or-equal to itself"
  (doc    "`(>= true true)` is true: `>=` is reflexive on Bool. Completes the four ordering operators
           over the Bool order.")
  (input  (>= true true))
  (output (: true Bool)))

; --- A total order is observed through a three-way `compare` yielding Ordering ------------------
; core-semantics.md #A Total Order Is Observed Through A Three-Way Comparison: a type that offers a
; total order offers a `compare` yielding an `Ordering` value with exactly three variants — `Less`,
; `Equal`, `Greater` — so a single comparison reports the full relation, not one boolean bit of it.
; `Ordering` is an ORDINARY closed prelude sum (like Option, Result, Sign), so its value form is
; `(Ordering.Less unit)` etc. — a nullary variant applied to unit, the same `(Variant unit)` form
; every nullary variant takes — and a consumer deconstructs it with an exhaustive three-arm match.
; `compare` is the PRIMITIVE from which `<` `>` `<=` `>=` `=` are each definable (the operators MUST
; AGREE with it), so a type has one order surfaced two ways that cannot disagree. It also names the
; canonical element order Set/Map serialize in. Ordering is a FRESH capability the
; seed does not realize (distinct from `collections`). A later generation realizes `compare`; until
; then the seed DECLINES these rather than running them to a wrong value.

(case "comparing a lesser value to a greater yields Less"
  (doc    "`(compare 1 2)` is `(Ordering.Less unit)` — the three-way comparison reports that 1 is less
           than 2 as the `Less` variant of the Ordering sum, not a boolean (core-semantics.md #A Total
           Order Is Observed Through A Three-Way Comparison). Pins the Less arm of the three-way result.")
  (input  (compare 1 2))
  (output (: (Less unit) Ordering)))

(case "comparing equal values yields Equal"
  (doc    "`(compare 2 2)` is `(Ordering.Equal unit)` — the middle variant, distinct from both Less and
           Greater. Pins that the three-way comparison reports equality as its own variant rather than
           collapsing it into one of the strict relations.")
  (input  (compare 2 2))
  (output (: (Equal unit) Ordering)))

(case "comparing a greater value to a lesser yields Greater"
  (doc    "`(compare 3 2)` is `(Ordering.Greater unit)` — the Greater variant. Together with the Less and
           Equal cases this pins all three variants of the Ordering result are reachable and correctly
           discriminated by the value relation.")
  (input  (compare 3 2))
  (output (: (Greater unit) Ordering)))

(case "the three-way comparison is deconstructed by an exhaustive match"
  (doc    "An Ordering value is an ordinary closed sum, so it is matched with the uniform `(Ctor _)`
           patterns over its three variants (core-semantics.md #A Total Order Is Observed Through A
           Three-Way Comparison, 2nd sentence): matching `(compare 1 2)` selects the `Less` arm, yielding
           -1. Pins that a comparison result dispatches through the same exhaustive match as any other
           sum, so every consumer handles all three cases.")
  (input  (match (compare 1 2)
            ((Ordering.Less _)    -1)
            ((Ordering.Equal _)   0)
            ((Ordering.Greater _) 1)))
  (output (: -1 Int64)))

(case "the boolean less-than operator agrees with the three-way comparison"
  (doc    "core-semantics.md #A Total Order Is Observed Through A Three-Way Comparison (3rd sentence: the
           boolean ordering operators MUST agree with the three-way comparison): `(< 1 2)` is true
           exactly when `(compare 1 2)` is `(Ordering.Less unit)`. This case pins that agreement — `(< 1
           2)` is true and the compare above is Less, so a type's one order is surfaced two ways that
           cannot diverge.")
  (input  (< 1 2))
  (output (: true Bool)))

(case "the three-way comparison orders strings lexicographically"
  (doc    "`(compare \"a\" \"b\")` is `(Ordering.Less unit)` — String offers a total order (the
           lexicographic order of its Unicode scalar values, collections-and-text.md #String Comparison
           Is Defined On Scalar Values), so compare works over it exactly as over Int64. Pins that the
           three-way comparison is offered by every type with a total order, not only the numeric types.")
  (input  (compare "a" "b"))
  (output (: (Less unit) Ordering)))

(case "a program that makes a host call has that call in its observable behavior"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior.
           The module declares a unit-returning effect `log` and the entrypoint delegates it to the host,
           so its operation `log.emit` is bound (host-interface-binding.md #A Host Import Is A WIT-Typed
           Function The Manifest Enumerates); the run makes one host call and returns the unit value — the
           normal-termination value of a program evaluated only for its effect (core-semantics.md #An
           Expression Evaluated Only For Its Effect Yields The Unit Value). The (output …) primary clause
           pins the terminal condition; the (host-calls …) observation pins the call sequence.")
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (log.emit "hello"))) (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "hello" String))))

(case "host calls are observed in the order they were made"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior and
           #A Sequencing Block Evaluates Its Forms In Order (3rd sentence: an earlier form's host call is
           observed before a later form's): the two host calls are sequenced by a (do …) block, so
           \"first\" is observed before \"second\". The run terminates normally with the unit value
           (core-semantics.md #An Expression Evaluated Only For Its Effect Yields The Unit Value); the
           (output …) clause pins that terminal condition and the (host-calls …) observation pins the order.")
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (do
                  (log.emit "first")
                  (log.emit "second")))) (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "first" String))
              (call log.emit (: "second" String))))

; --- Left-to-right evaluation order WITHIN an expression --------------------------------------------
; The case above pins that a `do` block sequences its FORMS in order. This pins the finer guarantee that
; the sub-expressions WITHIN a single expression — a call's arguments, a binary operator's operands, a
; `let`'s initializers, a tuple's elements — are evaluated LEFT TO RIGHT (core-semantics.md #Host Calls
; Are Ordered And Part Of Observable Behavior; #A Sequencing Block Evaluates Its Forms In Order). Each
; sub-expression is a host call `(log.emit …)`, and the `host-responses` are consumed in CALL order, so
; the order is observable TWO ways at once: the `(host-calls …)` observation asserts the emit sequence
; directly, AND the operator is NON-COMMUTATIVE (`-`) with distinct responses, so evaluating in the wrong
; order would consume the responses swapped and produce a DIFFERENT value (`3 - 10 = -7`, not `10 - 3 = 7`)
; — the recorded `(output …)` alone would catch a right-to-left evaluator even without the observation.
; (A sub-expression whose result is UNUSED is dead-code-eliminated and makes no host call, so each case
; CONSUMES every sub-expression it evaluates — the tuple binds then reads both elements.)

(case "function arguments are evaluated left to right"
  (doc    "`(diff (log.emit \"first\") (log.emit \"second\"))` calls two host effects as the arguments to
           `diff = a - b`. The arguments evaluate left to right, so `first` is emitted before `second` and
           `a` gets the first response (10), `b` the second (3) → 10 - 3 = 7. A right-to-left evaluator
           would emit `second` first, bind `a`=3 and `b`=10, and compute -7 — caught by BOTH the value and
           the host-call order. Pins argument evaluation order, observable through the ordered host calls.")
  (input  (do
            (effect log (op emit (-> String Int64)))
            (def (diff a b) (- a b))
            (def (main) (host (log) (diff (log.emit "first") (log.emit "second")))) (export main)))
  (host-responses (respond log.emit (: 10 Int64)) (respond log.emit (: 3 Int64)))
  (output (: 7 Int64))
  (host-calls (call log.emit (: "first" String)) (call log.emit (: "second" String))))

(case "binary operator operands are evaluated left to right"
  (doc    "`(- (log.emit \"left\") (log.emit \"right\"))` — the two operands of `-` are host effects. The
           left operand evaluates first (emitting `left`, consuming response 10), then the right (`right`,
           response 3) → 10 - 3 = 7. The operator-position companion of the argument case: operand order,
           not only call-argument order, is left to right. A swapped order gives -7.")
  (input  (do
            (effect log (op emit (-> String Int64)))
            (def (main) (host (log) (- (log.emit "left") (log.emit "right")))) (export main)))
  (host-responses (respond log.emit (: 10 Int64)) (respond log.emit (: 3 Int64)))
  (output (: 7 Int64))
  (host-calls (call log.emit (: "left" String)) (call log.emit (: "right" String))))

(case "let bindings' initializers are evaluated in binding order"
  (doc    "`(let ((x (log.emit \"x\")) (y (log.emit \"y\"))) (- x y))` — the initializers run in binding
           order, so `x` is emitted and bound (response 10) before `y` (response 4) → 10 - 4 = 6. Pins that
           a multi-binding `let` evaluates its initializers top to bottom (the order a later binding could
           depend on an earlier one relies on), observable through the ordered host calls.")
  (input  (do
            (effect log (op emit (-> String Int64)))
            (def (main) (host (log) (let ((x (log.emit "x")) (y (log.emit "y"))) (- x y)))) (export main)))
  (host-responses (respond log.emit (: 10 Int64)) (respond log.emit (: 4 Int64)))
  (output (: 6 Int64))
  (host-calls (call log.emit (: "x" String)) (call log.emit (: "y" String))))

(case "tuple elements are evaluated left to right"
  (doc    "`(tuple (log.emit \"a\") (log.emit \"b\"))` — the elements evaluate left to right, so `a` is
           emitted (response 10) before `b` (response 4). The tuple is bound and BOTH elements read back
           (`(- (. t 0) (. t 1))` = 10 - 4 = 6) so neither emit is dead-code-eliminated — an unused element
           would be dropped, making no host call. Pins that a compound constructor evaluates its components
           left to right, observable through the ordered host calls.")
  (input  (do
            (effect log (op emit (-> String Int64)))
            (def (main) (host (log) (let ((t (tuple (log.emit "a") (log.emit "b")))) (- (. t 0) (. t 1))))) (export main)))
  (host-responses (respond log.emit (: 10 Int64)) (respond log.emit (: 4 Int64)))
  (output (: 6 Int64))
  (host-calls (call log.emit (: "a" String)) (call log.emit (: "b" String))))

; --- Control flow SELECTS which effects are observed ------------------------------------------------
; The ordering cases above evaluate every sub-expression. Control flow instead makes only SOME
; sub-expressions run: an `if` evaluates one branch, `and`/`or` skip the right operand when the left
; decides (core-semantics.md #Conditionals Evaluate One Branch; #Boolean Connectives Short-Circuit). The
; existing `if`/`and` cases witness this via a TRAP that does not fire (a negative — the run terminates
; normally). These witness it POSITIVELY: each branch/operand is a host effect, so the `(host-calls …)`
; observation records EXACTLY which effects ran — the taken branch's, and only it; the evaluated operand's,
; and none on the skipped path (`(host-calls)` = no call). A runtime `Bool` parameter drives the choice, so
; the selection happens at run time, not by a constant fold. This is the observable-effect complement of
; the trap-shielding cases (02-binding-and-control): the wrong branch/operand does not merely avoid a trap,
; its effect is genuinely never performed.

(case "a conditional performs only the taken branch's effect — then"
  (doc    "`(if b (log.emit \"then\") (log.emit \"else\"))` with `b`=true performs ONLY the then branch's
           effect — `then` is emitted, `else` is not (core-semantics.md #Conditionals Evaluate One Branch).
           The positive-observation companion of the trap-shielding conditional case: not only does the
           unselected branch avoid a trap, its host effect is never performed. The condition is a runtime
           parameter, so the selection is a run-time event.")
  (input  (do
            (effect log (op emit (-> String Int64)))
            (def (main (: b Bool)) (host (log) (if b (log.emit "then") (log.emit "else")))) (export main)))
  (host-responses (respond log.emit (: 1 Int64)))
  (call   main (: true Bool)) (output (: 1 Int64))
  (host-calls (call log.emit (: "then" String))))

(case "a conditional performs only the taken branch's effect — else"
  (doc    "The false-condition companion: with `b`=false only the ELSE branch's effect is performed —
           `else` is emitted, `then` is not. Together with the `then` case, this pins that a runtime `if`
           performs exactly one branch's effect, the one its condition selects.")
  (input  (do
            (effect log (op emit (-> String Int64)))
            (def (main (: b Bool)) (host (log) (if b (log.emit "then") (log.emit "else")))) (export main)))
  (host-responses (respond log.emit (: 2 Int64)))
  (call   main (: false Bool)) (output (: 2 Int64))
  (host-calls (call log.emit (: "else" String))))

(case "and short-circuit does not perform the right operand's effect"
  (doc    "`(and b (log.emit \"rhs\"))` with `b`=false short-circuits, so the right operand's host effect is
           NOT performed — `(host-calls)` records NO call, and the `and` is false → 0 (core-semantics.md
           #Boolean Connectives Short-Circuit). The positive-observation companion of the trap-based
           short-circuit case (02-binding-and-control): the skipped operand's effect genuinely does not
           occur, not merely a skipped trap.")
  (input  (do
            (effect log (op emit (-> String Bool)))
            (def (main (: b Bool)) (host (log) (if (and b (log.emit "rhs")) 1 0))) (export main)))
  (host-responses (respond log.emit (: true Bool)))
  (call   main (: false Bool)) (output (: 0 Int64))
  (host-calls))

(case "and performs the right operand's effect when the left is true"
  (doc    "The non-short-circuit path: with `b`=true the right operand IS evaluated, so its effect `rhs` is
           performed (`(host-calls)` records the one call) and, its response being true, the `and` is true →
           1. Pins that a `true` left operand reaches the right operand's effect — the observable complement
           of the skip case above.")
  (input  (do
            (effect log (op emit (-> String Bool)))
            (def (main (: b Bool)) (host (log) (if (and b (log.emit "rhs")) 1 0))) (export main)))
  (host-responses (respond log.emit (: true Bool)))
  (call   main (: true Bool)) (output (: 1 Int64))
  (host-calls (call log.emit (: "rhs" String))))

(case "or short-circuit does not perform the right operand's effect"
  (doc    "`(or b (log.emit \"rhs\"))` with `b`=true short-circuits, so the right operand's host effect is
           NOT performed — `(host-calls)` records no call, and the `or` is true → 1. The `or` mirror of the
           `and` short-circuit-effect case: a `true` left operand skips the right operand's effect.")
  (input  (do
            (effect log (op emit (-> String Bool)))
            (def (main (: b Bool)) (host (log) (if (or b (log.emit "rhs")) 1 0))) (export main)))
  (host-responses (respond log.emit (: false Bool)))
  (call   main (: true Bool)) (output (: 1 Int64))
  (host-calls))

(case "or performs the right operand's effect when the left is false"
  (doc    "The non-short-circuit path: with `b`=false the right operand IS evaluated, so `rhs` is performed
           (`(host-calls)` records the call) and, its response being true, the `or` is true → 1. Pins that a
           `false` left operand reaches the right operand's effect.")
  (input  (do
            (effect log (op emit (-> String Bool)))
            (def (main (: b Bool)) (host (log) (if (or b (log.emit "rhs")) 1 0))) (export main)))
  (host-responses (respond log.emit (: true Bool)))
  (call   main (: false Bool)) (output (: 1 Int64))
  (host-calls (call log.emit (: "rhs" String))))

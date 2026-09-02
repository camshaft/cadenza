; Binding, scope, and control flow — witnesses core-semantics.md. Cases are s-expressions
; in the canonical homoiconic representation (options/code-shape/); a result is (: <value> <Type>),
; a rejected program records its diagnostic code (options/diagnostics-schema/), a runtime halt
; records a trap. See README.md for the case vocabulary.
(case
  "a let binding is in scope in its body"
  (doc
    "Witnesses core-semantics.md #Binding Is Lexical — a name resolves to its enclosing binding.")
  (input (let ((x 10)) x))
  (output (: 10 Int64)))

(case
  "a name resolves to the nearest enclosing binding"
  (doc "Witnesses core-semantics.md #Binding Is Lexical.")
  (input (let ((x 1)) (let ((x 2)) x)))
  (output (: 2 Int64)))

(case
  "an inner binding shadows an outer one only within its scope"
  (doc
    "Witnesses core-semantics.md #Shadowing Is Well-Defined (which defers to the corpus):
           the inner x is 2 inside its let; the outer x is still 1 outside it, so the sum is 3.")
  (input (+ (let ((x 2)) x) (let ((x 1)) x)))
  (output (: 3 Int64)))

; The intro case above shadows within one type. Shadowing is also well-defined across DIFFERENT types:
; an inner `let` binder shadowing an outer `let` binder of a different type resolves each occurrence at its
; own binding's type, and the outer binder survives after the inner scope closes (the differently-typed
; shadow gets a FRESH slot — reusing the outer's slot would emit an invalid component). The two faces below
; pin the let-vs-let different-type shadow at the VALUE level (the parameter-shadow comment below covers the
; param face; these are the let-binder faces, exercised cross-backend).
(case
  "a nested let shadows an outer let of a different type and both resolve, outer surviving"
  (doc
    "`(let ((x 5)) (let ((y (let ((x true)) (if x 1 0)))) (+ x y)))` — the innermost `x` is Bool
           (used by `(if x 1 0)` → 1, bound to `y`), while the outer `x` stays Int64 (5) and survives the
           inner Bool shadow's scope, so `(+ x y)` = 6. Pins that a let binder shadowing an outer let binder
           of a DIFFERENT type resolves each `x` at its own binding's type and the outer survives — a fresh
           slot for the Bool shadow, not the outer Int64's slot (which would emit an invalid component).")
  (input
    (do (def (main) (let ((x 5)) (let ((y (let ((x true)) (if x 1 0)))) (+ x y)))) (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "an outer Int64 let binder survives an inner String shadow of the same name"
  (doc
    "The THIRD-type face: `(let ((x 10)) (let ((z (let ((x \"hi\")) (String.byte-len x)))) (+ x z)))` —
           the inner `x` is a String (`String.byte-len \"hi\"` = 2, bound to `z`), and the outer `x` stays
           Int64 (10) and survives, so `(+ x z)` = 12. Pins the outer binder survives a shadow by a
           heap-typed (String) value, not only a scalar Bool — the String companion of the case above.")
  (input
    (do
      (def (main) (let ((x 10)) (let ((z (let ((x "hi")) (String.byte-len x)))) (+ x z))))
      (export main)))
  (call main)
  (output (: 12 Int64)))

; A `let` may shadow a FUNCTION PARAMETER with a value of a DIFFERENT TYPE, and the inner binding's type
; governs its references (core-semantics.md #Shadowing Is Well-Defined: a shadowing binding takes effect
; for references in its scope). `(def (f x) (let ((x true)) x))` binds parameter `x` (used at type Int64
; by the call `(f 99)`) and then shadows it with `x = true` (Bool); the body returns the inner `x`, so
; `f` returns the Bool `true` regardless of its argument. The shadow is well-defined and the program is
; well-typed — the different-name analogue `(def (f x) (let ((y true)) y))` returns `true`, and a
; non-parameter nested shadow `(let ((x 99)) (let ((x true)) x))` returns `true`. A compiler that reuses
; the parameter's local SLOT (typed for the parameter, e.g. i64 for an Int64 argument) for the shadowing
; binding's differently-typed value emits a component that fails wasm validation — an invalid component,
; the worst outcome (self-hosting-and-bootstrap.md #An Unsupported Construct Is Declined, Not Miscompiled:
; a not-yet-handled construct MUST decline, never emit an invalid or divergent component). A generation
; that cannot yet allocate a fresh slot for a differently-typed shadow of a parameter declines rather than
; emitting an invalid component.
(case
  "an arm binder SHADOWS an outer heap list and the outer survives the shadow's scope"
  (doc
    "Two same-named HEAP handles with disjoint live ranges: an arm binder named xs shadows an
           outer heap list, the arm consumes the SHADOW (sum 18), then the OUTER xs is read after the
           arm's scope closes — the shadow's reclaim at arm-exit must not touch the outer's handle.")
  (input
    (do
      (def
        (sum-l (: l (List Int64)) (: acc Int64))
        (match l (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: n Int64))
        (do
          (def xs #list(1 2 n))
          (def o (Some #list(9 9)))
          (def inner (match o ((Some xs) (sum-l xs 0)) ((None _u) -1)))
          (+ (* inner 100) (sum-l xs 0))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1806 Int64))
  (call main (: 0 Int64))
  (output (: 1803 Int64))
  (live-objects 0))

(case
  "a LET shadow of a do-def heap binding closes its scope and the do-def survives"
  (doc
    "The mixed-BINDER-KIND interleave: do-def xs at do-scope, a let re-binds xs for an inner
           expression, the do-scope xs read after the let closes — the two binder forms take
           different lowering paths (statement- vs expression-position), so the shadow crosses
           lowering kinds; both handles are heap lists with distinct reclaim points.")
  (input
    (do
      (def
        (sum-l (: l (List Int64)) (: acc Int64))
        (match l (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: n Int64))
        (do
          (def xs #list(1 n))
          (def inner (let ((xs #list(7))) (sum-l xs 0)))
          (+ (* inner 100) (sum-l xs 0))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 706 Int64))
  (call main (: 0 Int64))
  (output (: 701 Int64))
  (live-objects 0))

(case
  "a closure captures a heap binding BEFORE a shadow and applies AFTER seeing the original"
  (doc
    "The face where capture-time vs apply-time name resolution DIVERGE observably: f captures
           the OUTER heap xs, a second (def xs …) shadows it, f applies AFTER — the capture cell must
           hold the ORIGINAL handle (dynamic scoping would see the shadow).")
  (input
    (do
      (def
        (sum-l (: l (List Int64)) (: acc Int64))
        (match l (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: n Int64))
        (do
          (def xs #list(1 n))
          (def f (fn ((: _u Int64)) (sum-l xs 0)))
          (def xs #list(7 8))
          (+ (* (f 0) 100) (sum-l xs 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 315 Int64))
  (call main (: 0 Int64))
  (output (: 115 Int64))
  (live-objects 0))

(case
  "TWO closures capture DIFFERENT generations of one shadowed name and each sees its own"
  (doc
    "Two live capture cells holding DIFFERENT heap handles under ONE source name: f captures
           gen-1 xs, the shadow rebinds, g captures gen-2, BOTH apply after — a capture keyed by NAME
           rather than binding-instance would alias them (both seeing gen-2 → 1515 not 315).")
  (input
    (do
      (def
        (sum-l (: l (List Int64)) (: acc Int64))
        (match l (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: n Int64))
        (do
          (def xs #list(1 n))
          (def f (fn ((: _u Int64)) (sum-l xs 0)))
          (def xs #list(7 8))
          (def g (fn ((: _u Int64)) (sum-l xs 0)))
          (+ (* (f 0) 100) (g 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 315 Int64))
  (call main (: 0 Int64))
  (output (: 115 Int64))
  (live-objects 0))

(case
  "a let shadowing a parameter with a differently-typed value is not an invalid component"
  (doc
    "`(def (f x) (let ((x true)) x))` shadows the Int64 parameter `x` with the Bool `x = true`; the
           body returns the inner `x`, so `(f 99)` = `true`. The shadow is well-defined (core-semantics.md
           #Shadowing Is Well-Defined) and the program is well-typed — the different-name form `(let ((y
           true)) y)` and the non-parameter nested shadow both return `true`. The compiler MUST compute
           `true` or DECLINE, never emit a component that fails wasm validation by reusing the parameter's
           local slot for the differently-typed shadow. Pins that a differently-typed shadow of a parameter
           gets its own slot rather than colliding with the parameter's, so the result is a valid component
           (the inline and different-name shadows already work; this is the same-name parameter-shadow
           case). A generation that cannot yet do so declines rather than emitting an invalid component.")
  (input (do (def (f x) (let ((x true)) x)) (def (main) (f 99)) (export main)))
  (output (: true Bool)))

(case
  "a let shadowing a parameter with a same-typed value runs, not miscompiles"
  (doc
    "The same-type companion of the differently-typed shadow above: `(def (f x) (let ((x 7)) x))`
           shadows the Int64 parameter `x` with another Int64 `x = 7`, so `(f 99)` = 7. Distinct from
           the Bool shadow because the types AGREE, yet it exercises the same binder-substitution hazard:
           when a function is inlined, β-reduction must NOT substitute the argument into the let's BINDER
           occurrence `x` (which resolves up to the same-named parameter). A generation that did so turned
           the binding into `(99 7)` — losing the name — so the body's `x` found no binding; here it
           additionally reused the parameter's slot, an outcome that MISCOMPILED to an invalid component.
           A binder names a binding and is copied, never substituted, so the inner `7` is returned.")
  (input (do (def (f x) (let ((x 7)) x)) (def (main) (f 99)) (export main)))
  (output (: 7 Int64)))

(case
  "a match-arm binder shadowing a parameter binds the scrutinee, not the argument"
  (doc
    "A match-arm PATTERN binder is a binding site, like a let binder: `(def (f x) (match 5 (x x)))`
           binds `x` to the scrutinee 5 for the arm's scope (core-semantics.md #Bindings Introduced By A
           Pattern Are Scoped To Its Branch), shadowing the parameter `x`; `(f 99)` = 5. When `f` inlines,
           β-reduction must copy the arm's binder occurrence `x` rather than substitute the argument for
           it (the binder resolves up to the same-named param) — else the arm binds nothing and the body's
           `x` is spuriously unbound. Pins that binder protection covers match-arm patterns, not only let
           bindings.")
  (input (do (def (f x) (match 5 (x x))) (def (main) (f 99)) (export main)))
  (output (: 5 Int64)))

(case
  "a match-arm binder CAPTURED BY A LAMBDA survives inlining the helper that owns it"
  (doc
    "The lambda-capture extension of the case above (v-cdz-smith type-oracle FIRST false-reject, an
           rcdzc scope bug — well-typed per the Lean oracle). A match-arm payload binder `i2` is read
           from INSIDE a lambda `(fn (i4) i2)` in a HELPER def `f0`; `(f0 2)` = 2 (the lambda ignores its
           arg and returns the captured `i2` = the Some-payload). The SAME capture in `main`, or for a
           let-binder / a helper param, compiles — it is SPECIFICALLY a match-arm payload binder captured
           by a lambda inside an INLINED helper that breaks: β-reduction copies f0's body into the call
           site, and the copied lambda-body reference to the match-arm SumPayload binder resolves UNBOUND
           (spurious CDZ0101) — the copy orphans it. Idealistically it compiles and computes 2; locked in
           as the spec target (TODO->pass when the β-copy preserves the lambda-captured match-arm binder's
           resolution, rcdzc scope/beta_reduce lane). Uncalled (`export f0`) it already compiles; only the
           inlined call site declines.")
  (input
    (do
      (def
        (f0 (: i1 Int64))
        (match (Some i1)
          ((Some i2) ((fn ((: i4 Int64)) i2) i2))
          ((None) 0)))
      (def (main) (f0 2))
      (export main)))
  (output (: 2 Int64)))

(case
  "sibling match arms each let-binding a DIFFERENT-WIDTH value get disjoint scratch slots, not an invalid component"
  (doc
    "The width-partition of the let-binder scratch claim (rcdzc wasm c443bd48d). Sibling match arms
           each RESET their scratch floor to the same `base`, so arm A's Int64 let-binder and arm B's
           Int32 let-binder both targeted `base` — but a single wasm local cannot be declared at two
           widths, so arm A's `LocalSet` stored i64 into an i32-declared slot → 'invalid component:
           function[N]: type mismatch: expected i64, found i32' (at inlining scale this is where the
           self-host emit-db.cdz tripped func[58]). The fix reuses a slot only when it is FREE or already
           recorded at THIS binder's width; a genuine width conflict spills to a fresh slot. Here arm A
           binds `(: 9000000000 Int64)` (needs i64, exceeds i32) and arm B binds `(: 42 Int32)`; a
           collision would emit an invalid component. `(pick A)` = 9000000000, `(pick B)` = 42 — both
           arms compute at their own width, valid component, both backends. The sibling-match-arm
           companion of the differently-typed-shadow slot cases above.")
  (input
    (do
      (type Sel (A) (B))
      (def
        (pick (: s Sel))
        (match
          s
          ((A) (let ((x (: 9000000000 Int64))) x))
          ((B) (let ((y (: 42 Int32))) (Int64.of y)))))
      (def (main (: k Int64)) (pick (if (> k 0) (Sel.A) (Sel.B))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 9000000000 Int64))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

(case
  "a let shadowing a parameter whose initializer references that parameter computes"
  (doc
    "The demanding shadow: the shadowing binding's INITIALIZER references the shadowed parameter.
           `(def (f x) (let ((x (+ x 1))) (* x 2)))` — the initializer `(+ x 1)` is written before the
           new `x` binding takes effect, so its `x` is the PARAMETER (core-semantics.md:53: an
           initializer observes the bindings written before it, not the one it introduces); the body's
           `(* x 2)` then reads the new local. `(f 20)` = (20+1)*2 = 42. This combines the two β-reduce
           hazards: the binder occurrence `x` must be copied not substituted (else the binding name is
           lost), AND the initializer's `x` reference must still be substituted with the argument (it IS
           a value reference to the param). A generation that lost the parameter binding when the local
           shared its name rejected CDZ0101 'unbound name `x`'. The different-name form (`(let ((y (+ x
           1))) …)`) and the let-over-let form both worked — only a same-name PARAM shadow broke.")
  (input (do (def (f x) (let ((x (+ x 1))) (* x 2))) (def (main (: n Int64)) (f n)) (export main)))
  (call main (: 20 Int64))
  (output (: 42 Int64)))

(case
  "a param-shadowing let with a param-referencing initializer folds at a constant argument"
  (doc
    "The constant-argument companion of the case above: `(f 20)` folds to 42 the same way, so the
           fix is not specific to a runtime argument — the parameter binding survives β-reduction for
           the initializer whether the argument is constant or runtime. Pins the fold path of the
           binder-copy / reference-substitute split.")
  (input (do (def (f x) (let ((x (+ x 1))) (* x 2))) (def (main) (f 20)) (export main)))
  (output (: 42 Int64)))

(case
  "a local let binding shadows a same-named top-level definition"
  (doc
    "A `let` binding named `f` shadows a top-level `(def (f) …)` of the same name for the extent of
           its scope: name resolution consults the lexical scope FIRST and the top-level def index only on
           a scope miss, so the body's `f` is the local 7, not the def's 99. Pins that resolution keys on
           the OCCURRENCE + its scope, never on the flat name index alone — same-named bindings at
           different scopes resolve independently (the invariant a nested-module / import rework must
           preserve).")
  (input (do (def (f) 99) (def (main) (let ((f 7)) f)) (export main)))
  (output (: 7 Int64)))

(case
  "a let binding whose value references a parameter compiles under a call"
  (doc
    "`(def (g n) (let ((x (+ n 1))) (+ x x)))` — the `let` value USES the parameter `n` (not a
           shadow). Calling `(g 10)` inlines g's body; the reduction must substitute `n`→`10` in the
           binding's initializer AND keep the body's references to the binding pointing at that
           substituted initializer. `x = 10+1 = 11`, so `(+ x x)` = 22. Pins that β-reduction copies a
           `let` inside a called function consistently — the body's binding references resolve to the
           COPY's substituted initializer, not the original (a name occurrence carried through a copy must
           re-resolve against the copied scope). A generation that shared the original unsubstituted
           initializer would surface an unsubstituted parameter with no local slot.")
  (input (do (def (g n) (let ((x (+ n 1))) (+ x x))) (def (main) (g 10)) (export main)))
  (output (: 22 Int64)))

(case
  "a nested if on the same condition collapses the inner test to the known branch"
  (doc
    "core-semantics.md #Conditionals Evaluate One Branch: inside the ELSE of `(if c … …)` the
           condition `c` is known false, so a nested `(if c B D)` there always takes `D`. `(if c 1 (if c 2
           3))` therefore never yields 2: `c` = true → 1, `c` = false → the outer else, where the inner `c`
           is false → 3. A compiler that constant-propagates the outer condition into the nested test folds
           the inner `if` away to `D`; this pins the observable result of that propagation is the same as
           re-evaluating `c` — the inner branch `2` is dead.")
  (input (do (def (main (: c Bool)) (if c 1 (if c 2 3))) (export main)))
  (call main (: false Bool))
  (output (: 3 Int64)))

(case
  "a redundant relational check inside its own truthy branch is known-true and its dead branch is eliminated"
  (doc
    "The FLAGSHIP value-facts demonstrator (operator directive, DESIGN-flow-sensitive-value-facts.md
           §3.1a): a flow-sensitive INTERVAL fact — not a boolean const-propagation — decides a nested
           relational comparison. Inside the truthy branch of `(if (> x 0) …)`, `x` is refined to `[1, MAX]`,
           so a nested `(if (> x 0) T F)` is KNOWN TRUE and folds to `T` with the dead branch `F` eliminated.
           Distinct from the boolean `(if c 1 (if c 2 3))` collapse above (which propagates a Bool VALUE): here
           the inner comparison is re-decided by the refined RANGE of `x`, the mechanism the value-facts work
           generalizes. Three shapes the analysis proves, each pinned by observable value:
             (a) SAME test:    `(if (> x 0) (if (> x 0) 1 2) 3)` — inner known true → never yields 2.
             (b) IMPLIED test: `(if (>= x 5) (if (> x 0) 1 2) 3)` — `x >= 5 ⇒ x > 0`, inner true → never 2.
             (c) MADE-FALSE:   `(if (< x 0) (if (> x 10) 1 2) 3)` — `x < 0 ⇒ x > 10` false → inner yields 2.
           The Lir-level elimination (inner compare gone, dead-branch constant gone) is unit-pinned in
           rcdzc `a_branch_refinement_folds_a_redundant_nested_comparison_and_eliminates_its_dead_branch`;
           this corpus case pins the OBSERVABLE-VALUE parity fleet-wide on both backends, so a future change
           that drops the refinement can't silently regress the fold.")
  (input
    (do
      (def (same (: x Int64)) (if (> x 0) (if (> x 0) 1 2) 3))
      (def (implied (: x Int64)) (if (>= x 5) (if (> x 0) 1 2) 3))
      (def (made-false (: x Int64)) (if (< x 0) (if (> x 10) 1 2) 3))
      (export same)
      (export implied)
      (export made-false)))
  ; (a) same test: positive x takes both truthy branches → 1; x <= 0 → outer else → 3 (inner `2` unreachable)
  (call same (: 5 Int64))
  (output (: 1 Int64))
  (call same (: 0 Int64))
  (output (: 3 Int64))
  ; (b) implied: x >= 5 makes the inner `x > 0` known true → 1; x < 5 → 3 (inner `2` unreachable)
  (call implied (: 10 Int64))
  (output (: 1 Int64))
  (call implied (: 5 Int64))
  (output (: 1 Int64))
  (call implied (: 3 Int64))
  (output (: 3 Int64))
  ; (c) made-false: x < 0 makes the inner `x > 10` known false → 2; x >= 0 → outer else → 3
  (call made-false (: -1 Int64))
  (output (: 2 Int64))
  (call made-false (: 0 Int64))
  (output (: 3 Int64))
  (call made-false (: 20 Int64))
  (output (: 3 Int64)))

(case
  "an UNSIGNED branch refinement decides a nested comparison and its soundness twin must not over-refine"
  (doc
    "The GAP-A unsigned companion of the flagship above (value-facts slice 2, rcdzc 77ac05508): the
           interval refinement now fires for an UNSIGNED comparison too, so a redundant nested `<` test on a
           `UInt32` folds — while an UNDECIDED nested test must NOT fold (the value-correctness twin). Before
           GAP-A the unsigned comparison refined nothing, so both compares always stayed. Three shapes:
             (a) SAME test:    `(if (< x 8) (if (< x 8) 1 2) 3)` — inner known true under x∈[0,7] → never 2.
             (b) IMPLIED test: `(if (< x 4) (if (< x 8) 1 2) 3)` — `x < 4 ⇒ x < 8`, inner true → never 2.
             (c) SOUNDNESS TWIN: `(if (< x 8) (if (< x 4) 1 2) 3)` — `x < 8` does NOT decide `x < 4`, so BOTH
                 compares MUST remain; over-refining here would flip x=6 from 2 to a wrong value (a miscompile).
           All scalar `UInt32`, so they gate cleanly on both backends. Pins that the unsigned refinement folds
           the decided cases AND leaves the undecided one intact fleet-wide.")
  (input
    (do
      (def (same (: x UInt32)) (if (< x 8) (if (< x 8) 1 2) 3))
      (def (implied (: x UInt32)) (if (< x 4) (if (< x 8) 1 2) 3))
      (def (twin (: x UInt32)) (if (< x 8) (if (< x 4) 1 2) 3))
      (export same)
      (export implied)
      (export twin)))
  ; (a) same: x < 8 makes the inner `x < 8` known true → 1; x >= 8 → outer else → 3 (inner `2` unreachable)
  (call same (: 3 UInt32))
  (output (: 1 Int64))
  (call same (: 50 UInt32))
  (output (: 3 Int64))
  ; (b) implied: x < 4 makes the inner `x < 8` known true → 1; x >= 4 → 3 (inner `2` unreachable)
  (call implied (: 2 UInt32))
  (output (: 1 Int64))
  (call implied (: 6 UInt32))
  (output (: 3 Int64))
  ; (c) twin: x < 8 does NOT decide x < 4 → both remain; x=2 → inner true → 1, x=6 → inner false → 2
  (call twin (: 2 UInt32))
  (output (: 1 Int64))
  (call twin (: 6 UInt32))
  (output (: 2 Int64)))

(case
  "an UNSIGNED lower-bound refinement must not fabricate an i64::MAX ceiling for a UInt64 above it"
  (doc
    "The UInt64-ceiling soundness pin for value-facts GAP-A (rcdzc 070a403d7). A lower-bound
           refinement `(> x 8)` must NOT conclude `x <= i64::MAX` — a UInt64 ranges past i64::MAX. So the
           nested `(> x 9223372036854775807)` (i.e. `> i64::MAX`) must stay LIVE, not fold to false. The
           fold operand i64::MAX is itself i64-representable, so a buggy refinement CAN fire the fold — the
           load-bearing case. At x = 2^63 = 9223372036854775808 (a valid UInt64 one past i64::MAX) the inner
           test is TRUE => 1; the earlier miscompiling fold yielded 2. Root fix seeds the interval from
           resolved_int_bounds (UInt64 hi = None), not a hardcoded i64::MAX. The UInt64 companion of the
           UInt32 same/implied/twin case above (that surface was always sound — this bug only bit types whose
           max exceeds i64::MAX).")
  (input (do (def (f (: x UInt64)) (if (> x 8) (if (> x 9223372036854775807) 1 2) 0)) (export f)))
  (call f (: 9223372036854775808 UInt64))
  (output (: 1 Int64))
  ; control: a genuinely small x takes neither refined path -> 0
  (call f (: 5 UInt64))
  (output (: 0 Int64)))

(case
  "SIGNED branch refinement over NEGATIVE bounds must not over-refine the inner compare"
  (doc
    "The negative-bound face of the interval-refinement soundness family (the flagship + unsigned
           pins refine non-negative ranges; nothing pins refinement arithmetic below zero): `(if (> x -8)
           (if (> x -4) 1 2) 3)` — `x > -8` does NOT decide `x > -4` (x = -6 satisfies the outer but not
           the inner), so BOTH compares must survive: 0 → 1, -6 → 2, -10 → 3. A refinement pass whose
           interval arithmetic mishandled negative endpoints (e.g. compared magnitudes, or seeded the
           lower bound at 0 as the unsigned path does) would fold the inner test and flip the -6 call.")
  (input (do (def (main (: x Int64)) (if (> x -8) (if (> x -4) 1 2) 3)) (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: -6 Int64))
  (output (: 2 Int64))
  (call main (: -10 Int64))
  (output (: 3 Int64)))

(case
  "an INCLUSIVE ordering refinement folds at the boundary but must not over-refine one past it"
  (doc
    "The off-by-one soundness twin for the INCLUSIVE (`<=`/`>=`) clamp math in the interval refinement
           (refine_from_comparison, diverge.rs): an inclusive bound refines to EXACTLY the constant
           (`Le → clamp(c)`, `Ge → clamp(c)`), one endpoint tighter than the strict form (`Lt → clamp(c-1)`,
           `Gt → clamp(c+1)`). The existing flagship pins the STRICT (`>`,`<`) fold; nothing pinned that an
           inclusive bound lands on the boundary and not one past it. A silent `clamp(c)`↔`clamp(c±1)` swap
           there is a real miscompile that value-pins alone would miss. Two faces:
             `fold`: `(if (<= x 5) (if (< x 6) 1 2) 3)` — `x <= 5` ⇒ `x < 6` is always TRUE, so the inner
                     compare folds and the dead `2` arm is eliminated (x=5→1, x=3→1, x=10→3). The FOLD
                     itself is unit-pinned in rcdzc
                     `an_inclusive_ordering_refinement_folds_at_the_boundary_but_never_one_past_it`.
             `edge`: `(if (<= x 5) (if (< x 5) 1 2) 3)` — `x <= 5` does NOT decide `x < 5` (x=5 satisfies the
                     outer but not the inner), so BOTH compares survive and x=5 takes the LIVE inner else → 2.
                     An establisher that over-refined `x<=5` to `[MIN,4]` would wrongly fold this and return 1
                     at x=5 — the SOUNDNESS TWIN. `edge-ge` is the symmetric `>=` face (over-refine to [6,MAX]).
           All Int64 scalar, so it gates on both backends. The x=5 boundary calls are the discriminators.")
  (input
    (do
      (def (fold (: x Int64)) (if (<= x 5) (if (< x 6) 1 2) 3))
      (def (edge (: x Int64)) (if (<= x 5) (if (< x 5) 1 2) 3))
      (def (edge-ge (: x Int64)) (if (>= x 5) (if (> x 5) 1 2) 3))
      (export fold)
      (export edge)
      (export edge-ge)))
  ; fold: x<=5 decides x<6 true → inner folds; value unchanged.
  (call fold (: 5 Int64))
  (output (: 1 Int64))
  (call fold (: 3 Int64))
  (output (: 1 Int64))
  (call fold (: 10 Int64))
  (output (: 3 Int64))
  ; edge: x<=5 does NOT decide x<5 — the x=5 boundary must take the inner else (2), NOT fold to 1.
  (call edge (: 5 Int64))
  (output (: 2 Int64))
  (call edge (: 3 Int64))
  (output (: 1 Int64))
  (call edge (: 10 Int64))
  (output (: 3 Int64))
  ; edge-ge: symmetric — x>=5 does NOT decide x>5; x=5 boundary takes the inner else (2).
  (call edge-ge (: 5 Int64))
  (output (: 2 Int64))
  (call edge-ge (: 7 Int64))
  (output (: 1 Int64))
  (call edge-ge (: 0 Int64))
  (output (: 3 Int64)))

(case
  "an EQUALITY test refines the same test in its own else to known-false"
  (doc
    "The equality face of branch refinement: inside the ELSE of `(if (= x 5) …)` the fact `x ≠ 5`
           holds, so a repeated `(= x 5)` there is known-false and its then-branch (99) is dead — x = 7 →
           0, never 99; the then-side control (x = 5 → x+1 = 6) confirms the refinement is branch-scoped.
           The `=`-fact companion of the relational refinements (an equality yields a point fact, not an
           interval).")
  (input (do (def (main (: x Int64)) (if (= x 5) (+ x 1) (if (= x 5) 99 0))) (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: 7 Int64))
  (output (: 0 Int64)))

(case
  "an equality point fact sheds a then-branch arithmetic overflow guard, but the trap survives unguarded"
  (doc
    "The EQUALITY companion of the interval underflow-elision case below: an `(= x c)` guard yields a
           POINT fact `x ∈ [c, c]` (not an interval), and that is enough to shed a checked-arith overflow
           guard when the result at `x == c` provably fits the NARROW type. `shed`: inside the then-branch of
           `(if (= x 5) …)`, `x` is pinned to `[5, 5]`, so `(+ x 1) = 6` cannot overflow Int8 and its
           overflow guard is dropped (value unchanged: x = 5 → 6, and the else covers everything else → 0).
           `raw`: the SAME `(+ x 1)` on Int8 WITHOUT the `(= x 5)` guard is unrefined, so at x = 127 it must
           still TRAP (127 + 1 overflows Int8) — the SOUNDNESS TWIN proving the elision is licensed by the
           POINT fact, not by luck. The Lir-level guard-drop is unit-pinned in rcdzc
           `an_equality_point_fact_elides_the_then_branch_arith_overflow_guard`; this pins the value + trap
           parity fleet-wide on both backends. Distinct from the ORDERING refinements (this is the point-fact
           face) and from the Eq-else known-false pin above (this elides an ARITHMETIC guard, not a compare).")
  (input
    (do
      (def (shed (: x Int8)) (if (= x 5) (: (+ x 1) Int8) 0))
      (def (raw (: x Int8)) (: (+ x 1) Int8))
      (export shed)
      (export raw)))
  ; shed: x == 5 pins x to [5,5], so (+ x 1) sheds its overflow guard; x = 5 → 6, any other x → else → 0.
  (call shed (: 5 Int8))
  (output (: 6 Int64))
  (call shed (: 100 Int8))
  (output (: 0 Int64))
  ; raw: unguarded (+ x 1) is value-correct for small x...
  (call raw (: 3 Int8))
  (output (: 4 Int64))
  ; ...and MUST still trap at x = 127 (Int8 overflow) — the trap the elision must NOT have dropped.
  (call raw (: 127 Int8))
  (trap "integer overflow"))

(case
  "an equality point fact folds an inner range comparison, both directions — the point-fact analogue of the ordering fold"
  (doc
    "The COMPARISON-FOLD face of the equality point fact: an `(= x c)` guard pins `x ∈ [c, c]`, and a
           point range decides EVERY ordinary comparison of `x` against a constant — so an inner `(if (> x k) …)`
           folds to a constant and its dead arm is eliminated. This is the point-fact analogue of the ORDERING
           `fold`/`implied` cases above (`x <= 5 ⇒ x < 6`), but the interval is a single point so it decides
           the compare in BOTH directions, which the two faces pin:
             `hi`: `(if (= x 5) (if (> x 3) 1 2) 0)` — under `x == 5`, `5 > 3` is always TRUE, so the inner
                   compare folds to the THEN arm and the `2` arm is dead (x=5 → 1; any other x → outer else → 0).
             `lo`: `(if (= x 5) (if (> x 5) 1 2) 0)` — under `x == 5`, `5 > 5` is always FALSE, so the inner
                   compare folds to the ELSE arm and the `1` arm is dead (x=5 → 2; any other x → 0).
           The `lo` face is the SOUNDNESS discriminator: a broken point fact that seeded `[5, MAX]` instead of
           `[5, 5]` would leave `> 5` undecided (or wrongly fold it true) and return 1 at x=5 — so the x=5 →
           2 output is what proves the fact is the tight POINT, not a half-open lower bound. Both Int64 scalar
           (gates on both backends). The rcdzc Lir counterpart is
           `an_equality_point_fact_folds_an_inner_range_comparison`. Distinct from the arith-guard elision above
           (this folds a COMPARE, not an arithmetic overflow guard) and from the Eq-else known-false pin (that
           refines the SAME test in its else; this decides a DIFFERENT ordering test in the then).")
  (input
    (do
      (def (hi (: x Int64)) (if (= x 5) (if (> x 3) 1 2) 0))
      (def (lo (: x Int64)) (if (= x 5) (if (> x 5) 1 2) 0))
      (export hi)
      (export lo)))
  ; hi: x == 5 ⇒ (> x 3) always TRUE → inner folds to 1, the 2 arm is dead.
  (call hi (: 5 Int64))
  (output (: 1 Int64))
  (call hi (: 4 Int64))
  (output (: 0 Int64))
  ; lo: x == 5 ⇒ (> x 5) always FALSE → inner folds to 2, the 1 arm is dead (the [5,5]-tightness discriminator).
  (call lo (: 5 Int64))
  (output (: 2 Int64))
  (call lo (: 9 Int64))
  (output (: 0 Int64)))

(case
  "a two-sided squeeze refines to an exact point and folds an inner comparison — the intersection face of the point fact"
  (doc
    "The INTERVAL-INTERSECTION face of the point fact: a `[c,c]` point can arise WITHOUT a syntactic
           `(= x c)` guard — a two-sided squeeze `x >= c AND x <= c` intersects to the single point `[c,c]`,
           and that decides an inner comparison exactly as the `Eq` arm does. `refine_from_comparison`
           (diverge.rs) intersects each new bound with the existing frame bound (`lo.max(nl)`, `hi.min(nh)`),
           and select.rs lowers each branch body with the refined frame as its base, so BOTH surface forms
           reach the point:
             `nested`: `(if (>= x 5) (if (<= x 5) (if (> x 5) 1 2) 3) 4)` — the inner `<= 5` intersects the
                       parent's `[5, MAX]` (from `>= 5`) down to `[5, 5]`, so the innermost `(> x 5)` is decided
                       FALSE and folds to its else (the `1` arm is dead). x=5 → 2; x=4 fails `>=5` → 4; x=9 fails
                       `<=5` → 3.
             `anded`:  `(if (and (>= x 5) (<= x 5)) (if (> x 5) 1 2) 3)` — the `and`-arm applies BOTH operands
                       into one frame → `[5, 5]`, same fold. x=5 → 2; x=6 → 3.
           SOUNDNESS: a regression that stopped INTERSECTING (kept only the last bound) would leave `[5, MAX]`,
           NOT decide `> 5`, and return 1 at x=5 — so the x=5 → 2 outputs prove the intersection produced the
           tight POINT. Pins that the point fact is reachable via the INTERSECTION path, not only the syntactic
           `Eq` arm (the `(= x c)` cases above). Both Int64 scalar (gates on both backends). The rcdzc Lir
           counterpart is `a_two_sided_squeeze_refines_to_an_exact_point_and_folds_an_inner_comparison`.")
  (input
    (do
      (def (nested (: x Int64)) (if (>= x 5) (if (<= x 5) (if (> x 5) 1 2) 3) 4))
      (def (anded (: x Int64)) (if (and (>= x 5) (<= x 5)) (if (> x 5) 1 2) 3))
      (export nested)
      (export anded)))
  ; nested: only x=5 satisfies >=5 AND <=5; there the squeezed [5,5] decides (> x 5) FALSE → inner else (2).
  (call nested (: 5 Int64))
  (output (: 2 Int64))
  (call nested (: 4 Int64))
  (output (: 4 Int64))
  (call nested (: 9 Int64))
  (output (: 3 Int64))
  ; anded: the (and …) squeeze reaches the same [5,5] point and folds the inner (> x 5) the same way.
  (call anded (: 5 Int64))
  (output (: 2 Int64))
  (call anded (: 6 Int64))
  (output (: 3 Int64)))

(case
  "an equality point fact decides a DIFFERENT equality test in the then branch — the point-fact eq-fold face"
  (doc
    "The EQUALITY-FOLD face of the point fact: inside the then-branch of `(if (= x 5) …)` the fact pins
           `x` to `[5, 5]`, and `refined_comparison_const` (lower.rs) folds an inner `(= x k)` — TRUE when the
           range PINS x to {k} (k == 5), FALSE when k is OUTSIDE [5, 5] (k != 5). Two faces:
             `outside`: `(if (= x 5) (if (= x 3) 1 2) 0)` — under x == 5 the inner `(= x 3)` is decided FALSE
                        (3 ∉ [5,5]) → inner folds to its else, the `1` arm is dead. x=5 → 2; x=3 fails the outer
                        `= 5` → 0.
             `pinned`:  `(if (= x 5) (if (= x 5) 1 2) 0)` — under x == 5 the repeated `(= x 5)` is decided TRUE
                        (pins to {5}) → inner folds to its then, the `2` arm is dead. x=5 → 1; x=7 → 0.
           Distinct from the Eq-else known-false pin above (which refines the SAME `(= x 5)` in its OWN else via
           the negation — x ≠ 5 there); here the point in the THEN decides a DIFFERENT-constant equality (the
           `outside` face) and confirms the same-constant re-test (the `pinned` face). The compiler has NO
           `Prim::Ne` (Lt/Gt/Le/Ge/Eq only), so a `!=` desugars to `(not (= …))` and its not-equal fold rides
           the SAME Eq arm — the `outside` leg covers it. Both Int64 scalar (both backends). The rcdzc Lir
           counterpart is `a_point_fact_decides_a_different_equality_test_in_the_then_branch`.")
  (input
    (do
      (def (outside (: x Int64)) (if (= x 5) (if (= x 3) 1 2) 0))
      (def (pinned (: x Int64)) (if (= x 5) (if (= x 5) 1 2) 0))
      (export outside)
      (export pinned)))
  ; outside: under x==5 the inner (= x 3) is FALSE → inner else (2); the 1 arm is dead.
  (call outside (: 5 Int64))
  (output (: 2 Int64))
  (call outside (: 3 Int64))
  (output (: 0 Int64))
  ; pinned: under x==5 the repeated (= x 5) is TRUE → inner then (1); the 2 arm is dead.
  (call pinned (: 5 Int64))
  (output (: 1 Int64))
  (call pinned (: 7 Int64))
  (output (: 0 Int64)))

(case
  "a NEGATIVE-constant point fact folds an inner comparison with the correct SIGN — the negative-point twin"
  (doc
    "The NEGATIVE-constant face of the point fact: every point-fact case above pins a POSITIVE constant
           (`[5,5]`); this pins that a NEGATIVE `(= x -3)` refines `x` to the exact point `[-3,-3]`
           (refine_from_comparison's Eq arm uses the i64 constant directly — no magnitude/clamp mishandling)
           and that the inner compare folds with the CORRECT SIGNED ordering. Two faces:
             `above`: `(if (= x -3) (if (> x -5) 1 2) 0)` — under x == -3, `-3 > -5` is TRUE (signed: -3 is
                      ABOVE -5) → inner folds to 1, the `2` arm is dead. x=-3 → 1; x=-2 fails the outer → 0.
             `below`: `(if (= x -3) (if (> x -1) 1 2) 0)` — under x == -3, `-3 > -1` is FALSE → inner folds to
                      2, the `1` arm is dead. x=-3 → 2; x=0 → 0.
           The `below` face is the SIGN DISCRIMINATOR: a magnitude bug (treating |-3|=3 > |-1|=1) would wrongly
           fold `below` to 1 at x = -3 — so the x=-3 → 2 output proves the point fact carries the signed value,
           not its magnitude. Companion of the SIGNED-negative ORDERING refinement case above (this is the
           negative-POINT twin). Int64 scalar (both backends). The rcdzc Lir counterpart is
           `a_negative_constant_point_fact_folds_an_inner_comparison_with_the_correct_sign`.")
  (input
    (do
      (def (above (: x Int64)) (if (= x -3) (if (> x -5) 1 2) 0))
      (def (below (: x Int64)) (if (= x -3) (if (> x -1) 1 2) 0))
      (export above)
      (export below)))
  ; above: under x==-3 the point [-3,-3] decides -3 > -5 TRUE → inner then (1); the 2 arm is dead.
  (call above (: -3 Int64))
  (output (: 1 Int64))
  (call above (: -2 Int64))
  (output (: 0 Int64))
  ; below: under x==-3, -3 > -1 is FALSE → inner else (2); the 1 arm is dead (the signed-vs-magnitude discriminator).
  (call below (: -3 Int64))
  (output (: 2 Int64))
  (call below (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a point fact flows through a let binding into a compare fold — the point-fact composition face"
  (doc
    "The COMPOSITION face of the point fact: the refinement must survive through a kept `let` binding
           and an arith op into a downstream compare fold. `value_range` threads a `LocalRef` to its
           initializer's range and composes arith ranges, so inside `(if (= n 5) (let ((y (+ n 1))) …) …)` the
           refinement `n ∈ [5, 5]` flows into `y = (+ n 1)` (arith range `[6, 6]`), and the inner `(> y 3)`
           folds TRUE — even though the compared variable `y` is a let-bound DERIVATIVE of the refined `n`, not
           `n` itself. n=5 → the then computes y=6, `> y 3` holds → 1; any other n → outer else → 0. Pins the
           point-fact ⇄ let-binding ⇄ arith-range ⇄ compare-fold composition on both backends (each piece is
           pinned individually above; their composition through a `let` was not). The rcdzc Lir counterpart is
           `a_point_fact_flows_through_a_let_binding_into_an_arm_body_compare_fold`.")
  (input (do (def (f (: n Int64)) (if (= n 5) (let ((y (+ n 1))) (if (> y 3) 1 2)) 0)) (export f)))
  ; n==5 → y = n+1 = 6, so (> y 3) folds TRUE → 1 (the 2 arm is dead); the fact flowed n→y through the let+arith.
  (call f (: 5 Int64))
  (output (: 1 Int64))
  (call f (: 4 Int64))
  (output (: 0 Int64)))

(case
  "a literal match arm pins the scrutinee to a point, folding an arm-body compare and shedding an arith guard — the match-arm face of the point fact"
  (doc
    "The PATTERN-MATCH face of the point fact: a literal `Int` probe over a variable scrutinee means the
           scrutinee EQUALS that literal in the arm BODY, so `refined_frame_for_match_arm` (select.rs; the rust
           backend mirrors it) pins it to the tightest `[c, c]` — the match-arm analogue of the `(if (= x c) …)`
           if-guard point fact pinned above. Two faces in the `(5 …)` arm, where `n` is known `== 5`:
             `fold`: `(match n (5 (if (> n 3) 1 2) …) …)` — the arm-body `(> n 3)` is decided TRUE by the
                     `[5,5]` point and folds to 1 (the `2` arm is dead). n=5 → 1.
             `shed`: `(match n (5 (: (+ n 1) Int8) …) …)` on Int8 — under `n == 5`, `(+ n 1) = 6` provably fits
                     Int8, so its overflow guard is dropped (value 6).
           SOUNDNESS: the WILDCARD arm is NOT refined (n is unknown there), so its own `(+ n 1)` on Int8 must
           still TRAP at n = 127 — the twin proving the elision is licensed by the arm's point fact, not by luck.
           The wasm-Lir guard-drop is unit-pinned in rcdzc
           `a_scalar_match_literal_arm_refines_the_scrutinee_to_the_matched_value`; this corpus case pins the
           value + trap + fold parity on BOTH backends (the unit test is wasm-Lir only). Distinct from the
           if-guard point-fact cases above (this is the match-arm face).")
  (input
    (do
      (def (fold (: n Int64)) (match n (5 (if (> n 3) 1 2)) (_ 0)))
      (def (shed (: n Int8)) (match n (5 (: (+ n 1) Int8)) (_ 0)))
      (def (raw (: n Int8)) (match n (5 0) (_ (: (+ n 1) Int8))))
      (export fold)
      (export shed)
      (export raw)))
  ; fold: the (5 …) arm knows n==5, so (> n 3) folds TRUE → 1; the 2 arm is dead. Any other n → wildcard → 0.
  (call fold (: 5 Int64))
  (output (: 1 Int64))
  (call fold (: 8 Int64))
  (output (: 0 Int64))
  ; shed: n==5 in the arm → (+ n 1) = 6 sheds its Int8 overflow guard (value unchanged).
  (call shed (: 5 Int8))
  (output (: 6 Int64))
  (call shed (: 9 Int8))
  (output (: 0 Int64))
  ; raw: the WILDCARD (+ n 1) is unrefined — value-correct for small n...
  (call raw (: 3 Int8))
  (output (: 4 Int64))
  ; ...and MUST still trap at n = 127 (Int8 overflow) — the trap the arm-point elision must NOT have dropped.
  (call raw (: 127 Int8))
  (trap "integer overflow"))

(case
  "an unsigned branch refinement elides an underflow guard — the operator's if x>0 example on an unsigned type"
  (doc
    "The operator's motivating value-facts example (`if x > 0` ⇒ `x - 1` cannot underflow) on an
           UNSIGNED type, which value-facts slice 2 (GAP-A) newly enables — before it, the unsigned `(> x 0)`
           comparison refined nothing, so the guard always stayed. `dec`: inside the truthy branch of
           `(if (> x 0) …)`, `x` refines to `[1, 2^32−1]`, so `(- x 1)` cannot underflow and its wrap/trap
           guard is dropped; the value is unchanged (x=5 → 4, and the else covers x=0 → 0). `raw`: the SAME
           `(- x 1)` WITHOUT the `(> x 0)` guard is unrefined, so at x=0 it must still TRAP (unsigned
           underflow = integer overflow) — the SOUNDNESS TWIN proving the elision is licensed by the FACT,
           not by luck. Pins the operator's headline use case for unsigned + its trap-preservation twin on
           both backends. Distinct from the redundant-compare pins above (this elides an ARITHMETIC guard).")
  (input
    (do
      (def (dec (: x UInt32)) (if (> x 0) (: (- x 1) UInt32) 0))
      (def (raw (: x UInt32)) (: (- x 1) UInt32))
      (export dec)
      (export raw)))
  ; dec: x > 0 refines x to [1, MAX], so (- x 1) sheds its underflow guard; x=0 takes the else → 0.
  (call dec (: 5 UInt32))
  (output (: 4 Int64))
  (call dec (: 0 UInt32))
  (output (: 0 Int64))
  ; raw: unguarded (- x 1) computes for x>0 but MUST trap at x=0 (unsigned underflow) — the soundness twin.
  (call raw (: 3 UInt32))
  (output (: 2 Int64))
  (call raw (: 0 UInt32))
  (trap "integer overflow"))

(case
  "a flow refinement folds a compare over a no-overflow arith operand, but the trap survives when overflow is possible"
  (doc
    "The value-facts slice 6c face (rcdzc 621e71135): `refined_comparison_const` now folds a comparison
           whose operand is a CHECKED-ARITH node (`(+ x 1)`), not just a directly-refined variable — but ONLY
           when that arith is PROVABLY-NO-OVERFLOW under the flow refinement, because the fold DISCARDS the
           operand and a checked `+`/`-`/`*` is not trap-free. Two defs, and the pair is the whole point:
             (a) FOLDS: `(if (and (>= x 0) (< x 10)) (if (< (+ x 1) 11) 1 2) 3)` — under x∈[0,9], (+ x 1)∈[1,10]
                 cannot overflow AND is always < 11, so the inner compare folds true and the `2` arm dies. The
                 fold is licensed precisely because dropping the checked add is trap-safe here. Value-only
                 observable (x=5→1, x=9→1, x=20→3) — the Lir dead-arm elision is unit-pinned in rcdzc
                 `a_flow_refinement_propagates_through_a_no_overflow_arith_into_a_compare_fold`.
             (b) SOUNDNESS TWIN — the trap must survive: `(if (> x 0) (if (< (+ x 1) 11) 1 2) 3)`. Under just
                 `(> x 0)` (x∈[1, i64::MAX] — bounded below by the refinement, only the type max above) `(+ x 1)` CAN overflow (x = i64::MAX), so it is
                 NOT provably_no_overflow → the fold DECLINES, the checked add stays, and at x = i64::MAX the
                 `(+ x 1)` overflow trap is REACHED. This is the gate-visible witness that the fold does not
                 over-broaden `discardable` and silently drop a reachable trap — the twin lived only in the
                 `--lib` test; this pins the trap-preservation fleet-wide on both backends.")
  (input
    (do
      (def (folded (: x Int64)) (if (and (>= x 0) (< x 10)) (if (< (+ x 1) 11) 1 2) 3))
      (def (twin (: x Int64)) (if (> x 0) (if (< (+ x 1) 11) 1 2) 3))
      (export folded)
      (export twin)))
  ; (a) folded: under x∈[0,9], (+ x 1)<11 always → 1; outside the guard → 3. The dropped `2` arm is unreachable.
  (call folded (: 5 Int64))
  (output (: 1 Int64))
  (call folded (: 9 Int64))
  (output (: 1 Int64))
  (call folded (: 20 Int64))
  (output (: 3 Int64))
  ; (b) twin: x>0 does NOT bound x above, so the add is not provably-no-overflow → it stays; small x is value-correct...
  (call twin (: 5 Int64))
  (output (: 1 Int64))
  (call twin (: 0 Int64))
  (output (: 3 Int64))
  ; ...and at x = i64::MAX the surviving (+ x 1) overflows — the trap the fold must NOT have dropped.
  (call twin (: 9223372036854775807 Int64))
  (trap "integer overflow"))

(case
  "a below-len guard lets List.at shed its own bounds check, yet an out-of-range index still returns None"
  (doc
    "The operator-greenlit BOUNDED-INDEX (below-len) facet: inside the then-branch of
           `(< i (List.len xs))`, the index `i` is flow-known `< len(xs)`, so a `List.at xs i` there sheds
           its OWN redundant `index < len` bounds check (the enclosing guard already proved it) — the bounds
           analogue of the interval facet's overflow-guard elision. KEYED ON COLLECTION IDENTITY: the fact is
           `below_len[i] = xs`, so it never licenses eliding a check on a DIFFERENT list. The Lir-level
           elision (List.at's `vec-len` gone, confirmed against a facet-disabled baseline of 2→1) is
           unit-pinned in rcdzc `a_below_len_guard_elides_the_matching_list_ats_own_bounds_check_but_not_a_different_lists`.
           Here the value parity + the SOUNDNESS EDGES are pinned:
             `guarded`: under the guard, an in-range index reads its element (i=0→10, i=2→30). The guard
                 FALSE path (i=3, `3 < 3` false) takes the else → −2. And the negative-index edge (i=−1: the
                 guard `−1 < 3` is TRUE, so we enter the then-branch and `List.at xs −1` runs WITH its lower
                 `index >= 0` half still intact — the facet elides only the UPPER half — so it returns None →
                 −1, NOT a wild read). This is the soundness twin: eliding `< len` must not also drop `>= 0`.
             `unguarded`: the SAME access with no guard — value-identical for every index (i=3→None→−1,
                 i=−1→None→−1), proving the guarded elision changed no observable result.
           The list is RUNTIME-length (`(if … (list 10 20) (list 10 20 30))`, length depends on `i`): a
           const-length list folds `List.len` to a constant (the interval facet's domain), so a genuine
           runtime `Core::ListLen` is required to exercise this facet. Both backends.")
  (input
    (do
      (def
        (guarded (: i Int64))
        (let
          ((xs (if (> i 100) #list(10 20) #list(10 20 30))))
          (if (< i (List.len xs)) (match (List.at xs i) ((Some v) v) ((None _) -1)) -2)))
      (def
        (unguarded (: i Int64))
        (let
          ((xs (if (> i 100) #list(10 20) #list(10 20 30))))
          (match (List.at xs i) ((Some v) v) ((None _) -1))))
      (export guarded)
      (export unguarded)))
  ; guarded: in-range reads (0→10, 2→30); guard-false else (3→−2); negative-index edge stays None (−1→−1).
  (call guarded (: 0 Int64))
  (output (: 10 Int64))
  (call guarded (: 2 Int64))
  (output (: 30 Int64))
  (call guarded (: 3 Int64))
  (output (: -2 Int64))
  (call guarded (: -1 Int64))
  (output (: -1 Int64))
  ; unguarded: same access, no guard — value-identical for every index (the elision is observably neutral).
  (call unguarded (: 0 Int64))
  (output (: 10 Int64))
  (call unguarded (: 2 Int64))
  (output (: 30 Int64))
  (call unguarded (: 3 Int64))
  (output (: -1 Int64))
  (call unguarded (: -1 Int64))
  (output (: -1 Int64)))

(case
  "a match guard reads a HEAP payload's length and the arm body reuses the same binder"
  (doc
    "A guard that reads the heap payload it just destructured: (guard (Some xs) (> (List.len
           xs) 2)) BORROWS xs for the predicate, then the SAME binder is consumed by whichever arm
           wins — a guard that consumed the payload on failure would break the fall-through arm's
           own List.len read. Faces: pass→sum 18 / fail→fall-through -1 / None→0.")
  (input
    (do
      (def
        (sum-l (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (f (: o (Option (List Int64))))
        (match
          o
          ((guard (Some xs) (> (List.len xs) 2)) (sum-l xs 0))
          ((Some xs) (- 0 (List.len xs)))
          ((None _u) 0)))
      (def
        (main (: mode Int64))
        (f (if (= mode 1) (Some #list(5 6 7)) (if (= mode 2) (Some #list(5)) (None unit)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 18 Int64))
  (call main (: 2 Int64))
  (output (: -1 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a match guard reads a NESTED record-field binder (a record inside the matched Some)"
  (doc
    "A guard cond reads a binder bound by a `(record …)` sub-pattern NESTED inside the matched
           variant: `((guard (Some (record (= x a) (= y b))) (> a 5)) …)` — the guard `(> a 5)` reads `a`,
           the record field `x` destructured from the `Some` payload. The guard-cond twin of the body's
           nested-record binder read (Case 6rec-nested): a guard reads EVERY binder its pattern binds,
           including one nested in a record inside a variant, exactly as the arm body does. Was a spurious
           CDZ0101 `unbound name a` AT THE GUARD COND (guard-cond scope only reached TOP-LEVEL pattern
           binders + a top-level record, not a record NESTED in a variant), while the identical binder read
           in an arm BODY compiled. Faces: n=7 → guard `7>5` true → `7*100 + 3` = 703; n=3 → guard `3>5`
           false → falls to the bare `(Some r)` arm → `r.y` = 3; n=-1 → `None` → -1.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (if (> n 0) (Some #record((= x n) (= y 3))) (None))
          ((guard (Some #record((= x a) (= y b))) (> a 5)) (+ (* a 100) b))
          ((Some r) r.y)
          ((None u) -1)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 703 Int64))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (call main (: -1 Int64))
  (output (: -1 Int64)))

(case
  "THREE stacked guards on one constructor classify a heap payload into bands in order"
  (doc
    "The stacked face: three guards on ONE constructor classify by length bands (>4/>2/>0 →
           3/2/1, bare (Some _) → 0). Each failing guard must RE-borrow xs for the next guard's
           List.len — the payload survives N guard evaluations — and band order is first-match
           semantics over overlapping predicates (a reorder or a consume-at-guard-1 breaks bands
           2/3). Runtime-built list per call.")
  (input
    (do
      (def
        (build (: n Int64) (: acc (List Int64)))
        (if (= n 0) acc (build (- n 1) (List.push acc 1))))
      (def
        (band (: o (Option (List Int64))))
        (match
          o
          ((guard (Some xs) (> (List.len xs) 4)) 3)
          ((guard (Some xs) (> (List.len xs) 2)) 2)
          ((guard (Some xs) (> (List.len xs) 0)) 1)
          ((Some _xs) 0)
          ((None _u) -1)))
      (def (main (: n Int64)) (band (Some (build n #list()))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (call main (: 3 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a match guard performs a MAP lookup keyed by the pattern binder (guard x CHAMP)"
  (doc
    "The guard-predicate cases above read the destructured payload directly (List.len bands); this
           one sends the binder through a CHAMP descent INSIDE the guard: `(guard (Some id) (match
           (Map.lookup prices id) ((Some p) (> p 60)) ((None _u) false)))` — the guard's truth is decided
           by a lookup in an enclosing-scope map, with the inner match's own binder `p` scoped to the
           predicate. k=1 finds 100 (>60) -> guard passes -> 1; k=2 finds 50 (fails) -> falls through to
           the bare (Some _id) arm -> 2; k=9 misses (None -> false) -> same fall-through -> 2. Pins the
           nested-match-as-predicate composition (a guard is an arbitrary Bool expression, including one
           with its own binders) and that a MISS and a FAILING hit take the same fall-through edge.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def prices #map((= 1 100) (= 2 50)))
          (match
            (Some k)
            ((guard (Some id) (match (Map.lookup prices id) ((Some p) (> p 60)) ((None _u) false)))
              1)
            ((Some _id) 2)
            ((None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 2 Int64)))

(case
  "an IF nested in a match arm re-tests the binder with a MAP lookup (the unguarded twin)"
  (doc
    "The control beside the guard x CHAMP case above: the same lookup-decides-band logic spelled as
           an ordinary `if` INSIDE the arm body instead of a guard — `(if <lookup id> 1 (if (= id 2) 2
           0))`. Semantically the same classification (k=1 -> 1, k=2 -> 2, k=9 -> 0 here since the arm
           body distinguishes the miss), but structurally the arm is UNCONDITIONALLY taken and the test
           lives in its body — no fall-through edge, no re-match. Pins that the two spellings agree where
           they overlap (hit-pass and hit-fail) and that moving the predicate INTO the arm changes only
           what the author writes for a miss, not the lookup semantics.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def prices #map((= 1 100) (= 2 50)))
          (match
            (Some k)
            ((Some id)
              (if
                (match (Map.lookup prices id) ((Some p) (> p 60)) ((None _u) false))
                1
                (if (= id 2) 2 0)))
            (None -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 0 Int64)))

(case
  "a MATCH as a NON-FINAL multi-binding let init selects the binding value per call"
  (doc
    "No landed case puts a match in a NON-FINAL init of a multi-binding let: the match
           selects the first binding's value, the second binding evaluates after, both read in the
           body.")
  (input
    (do (def (main (: n Int64)) (let ((a (match n (0 10) (_ 20))) (b 5)) (+ a b))) (export main)))
  (call main (: 0 Int64))
  (output (: 15 Int64))
  (call main (: 3 Int64))
  (output (: 25 Int64)))

(case
  "a HANDLE as a NON-FINAL multi-binding let init completes before the sibling binding"
  (doc
    "The effects twin: a FULL handle (performing init seed, one perform, teardown) runs as
           the FIRST binding, the sibling scalar binds after — the handler's install/discharge/exit
           is bounded by one init slot (a scope leak past the binding breaks the sibling).")
  (input
    (do
      (effect C (op t (-> Unit Int64)))
      (def
        (main (: n Int64))
        (let ((a (handle C (+ 100 n) ((t (_u) s (resume s s))) (C.t))) (b 5)) (+ a b)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 107 Int64))
  (call main (: 0 Int64))
  (output (: 105 Int64)))

(case
  "TWO match inits in one let SEQUENCE - the second scrutinizes the first's binding"
  (doc
    "Sequential init dependency through two match lowerings in binding position (a = match n,
           b = match a) — a parallel-binding lowering or an init reorder breaks b.")
  (input
    (do
      (def
        (main (: n Int64))
        (let ((a (match n (0 10) (_ 20))) (b (match a (10 1) (_ 2)))) (+ a b)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64))
  (call main (: 3 Int64))
  (output (: 22 Int64)))

(case
  "a match init binds a per-arm HEAP list and the sibling init consumes it"
  (doc
    "Heap flow between sequential inits: a match init binds a PER-ARM heap list (different
           lengths per arm), the sibling init folds it, the body reads BOTH — three use-sites across
           the binding sequence; the len digit catches an arm mix-up beyond the sum.")
  (input
    (do
      (def
        (sum-l (: l (List Int64)) (: acc Int64))
        (match l (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: n Int64))
        (let
          ((xs (match n (0 #list(1 2 3)) (_ #list(9)))) (s (sum-l xs 0)))
          (+ (* s 10) (List.len xs))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 63 Int64))
  (call main (: 5 Int64))
  (output (: 91 Int64))
  (live-objects 0))

(case
  "unsigned branch refinement stays value-correct at the domain edges (0-lower-bound tautologies)"
  (doc
    "The soundness BOUNDARY of the unsigned interval refinement (value-facts GAP-A): an unsigned
           value is always ≥ 0, so a comparison against the domain's lower edge is a tautology the
           refinement must handle WITHOUT fabricating a bogus/inverted interval. Three edge shapes, all
           value-pinned (an over-refinement would flip one):
             (a) `(if (< x 0) 1 9)` — `x < 0` is UNSATISFIABLE for a UInt32 (x ≥ 0), so the then-arm is
                 unreachable and every call returns 9 (the else). A refinement that produced an inverted
                 `[0,-1]` interval and mis-decided the branch would wrongly return 1.
             (b) `(if (>= x 0) 7 1)` — `x >= 0` is ALWAYS TRUE for unsigned, so every call returns 7.
             (c) `(if (> x 0) (if (< x 0) 1 2) 3)` — inside `x > 0` (so `x ∈ [1, MAX]`), the nested
                 `(< x 0)` is provably FALSE, so the inner takes its else → 2 (x>0) / 3 (x=0). Pins that the
                 refinement composes correctly at the edge instead of contradicting itself.
           All scalar UInt32, both backends. Guards the GAP-A unsigned path against a degenerate-edge
           miscompile (the sibling of the UInt64-ceiling pin — both are unsigned-domain soundness edges).")
  (input
    (do
      (def (lt0 (: x UInt32)) (if (< x 0) 1 9))
      (def (ge0 (: x UInt32)) (if (>= x 0) 7 1))
      (def (nest (: x UInt32)) (if (> x 0) (if (< x 0) 1 2) 3))
      (export lt0)
      (export ge0)
      (export nest)))
  ; (a) x < 0 unsatisfiable for unsigned → always the else (9)
  (call lt0 (: 0 UInt32))
  (output (: 9 Int64))
  (call lt0 (: 5 UInt32))
  (output (: 9 Int64))
  ; (b) x >= 0 always true for unsigned → always the then (7)
  (call ge0 (: 0 UInt32))
  (output (: 7 Int64))
  (call ge0 (: 5 UInt32))
  (output (: 7 Int64))
  ; (c) nested (< x 0) under (> x 0) is provably false → inner else; x>0 → 2, x=0 → outer else 3
  (call nest (: 5 UInt32))
  (output (: 2 Int64))
  (call nest (: 0 UInt32))
  (output (: 3 Int64)))

(case
  "conditional propagation respects a shadowing rebind of the condition variable"
  (doc
    "The propagation must track the condition's VALUE in scope, not match its text: `(let ((c (< n
           5))) (if c 1 (let ((c true)) (if c 2 3))))` with n = 10 has the OUTER `c` = false (10 < 5 is
           false), so the outer `if` takes its else; there the INNER `c` is a fresh binding = true, so the
           inner `if` takes 2. The two `c`s are textually identical but denote different values — a
           propagation that folded the inner `(if c …)` to the outer `c`'s known-false value would wrongly
           yield 3. Pins that the constant propagation is scope-aware (it stops at a rebinding of the
           condition name), the control-flow analogue of the lexical-shadowing binding rule.")
  (input
    (do
      (def (main (: n Int64)) (let ((c (< n 5))) (if c 1 (let ((c true)) (if c 2 3)))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 2 Int64)))

(case
  "a two-arm match whose wildcard binds the scrutinee and applies a trap-free op — selects, binder reads it"
  (doc
    "`(def (main (: n Int64)) (match n (0 -1) (m (& m 255))))` — a two-arm scalar match: the `0`
           probe yields -1, else the wildcard binds the scrutinee as `m` and returns `(& m 255)` (the low
           byte). Both arms are trap-free, so the compiler emits a branchless `select` (the match analogue
           of the `if`→`select` conversion); the `m` binder reads the scrutinee's spilled value, which is
           materialized before either arm. Called with 300: not 0, so `300 & 255 = 44`. Pins that the
           select-converted match's scrutinee binder still reads the runtime scrutinee value correctly.")
  (input (do (def (main (: n Int64)) (match n (0 -1) (m (& m 255)))) (export main)))
  (call main (: 300 Int64))
  (output (: 44 Int64)))

(case
  "a two-arm match whose wildcard binds the scrutinee — the zero-probe arm"
  (doc
    "The probe-hit companion of the select-converted binding match `(match n (0 -1) (m (& m 255)))`:
           called with 0, the `0` probe matches and yields -1 (the `m` arm is not selected, though the
           branchless `select` evaluates both arm values). Confirms value parity of the select-converted
           match with the structured probe chain on the probe-hit path too.")
  (input (do (def (main (: n Int64)) (match n (0 -1) (m (& m 255)))) (export main)))
  (call main (: 0 Int64))
  (output (: -1 Int64)))

; `_` is a binding-position WILDCARD (it discards the bound value in a pattern or a discarded `let` binder).
; Using it as a VALUE — `(+ _ 1)`, an argument `(g _)`, a bare `_` body — is a category misuse, not an
; unbound name (a "did you mean?" typo suggestion is nonsense for it): CDZ0201 naming the misuse ("`_` is a
; wildcard … only in a binding position … discards the value"), not the generic unbound-name error. A
; LEGITIMATE `_` in a binding position (a wildcard match arm, a discarded `let` binder) compiles clean, and a
; `_`-LED name (`_x`) is an ordinary silenced binder, never the bare wildcard. (Migrated from rcdzc
; a_wildcard_used_as_a_value_names_the_binding_position_misuse.)
(case
  "a wildcard used as an operator operand names the binding-position misuse"
  (input (do (def x (+ _ 1)) (export x)))
  (error
    CDZ0201
    (message "`_` is a wildcard")
    (message "only in a binding position")
    (message "discards the value")))

(case
  "a wildcard used as a function argument names the binding-position misuse"
  (input (do (def (g a) a) (def x (g _)) (export x)))
  (error CDZ0201 (message "`_` is a wildcard")))

(case
  "a bare wildcard used as a def body names the binding-position misuse"
  (input (do (def (f) _) (export f)))
  (error CDZ0201 (message "`_` is a wildcard")))

(case
  "a legitimate wildcard match arm compiles and selects"
  (input (do (def (f (: n Int64)) (match n (0 1) (_ 2))) (def (main) (f 5)) (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a discarded let binder _ compiles clean"
  (input (do (def (f) (let ((_ 5)) 3)) (def (main) (f)) (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a _-led name is an ordinary silenced binder, not the bare wildcard"
  (input (do (def (f (: _x Int64)) _x) (def (main) (f 9)) (export main)))
  (call main)
  (output (: 9 Int64)))

(case
  "a sparse three-arm match's terminal pair dispatches branchlessly — the middle probe"
  (doc
    "`(def (main (: x Int64)) (match x (0 10) (100 20) (_ 30)))` — a SPARSE three-arm scalar match
           (the 0/100 values are too far apart for a jump table), so it compiles to a probe chain whose
           TERMINAL pair `(100 20) / (_ 30)` is a two-arm select shape and emits a branchless `select`
           (both arms are constants). Called with 100: the terminal select's condition `x == 100` holds →
           20. Pins that the tail of an N-arm sparse chain dispatches branchlessly, not only a standalone
           two-arm match.")
  (input (do (def (main (: x Int64)) (match x (0 10) (100 20) (_ 30))) (export main)))
  (call main (: 100 Int64))
  (output (: 20 Int64)))

(case
  "a sparse three-arm match's terminal pair dispatches branchlessly — the default"
  (doc
    "The default path of the sparse three-arm match `(match x (0 10) (100 20) (_ 30))`: called with
           7, which matches neither 0 nor 100, so the wildcard 30 is selected (the terminal `select`'s
           `x == 100` condition is false). Together with the middle-probe case this pins value parity of
           the branchless terminal pair with the structured probe chain it replaces.")
  (input (do (def (main (: x Int64)) (match x (0 10) (100 20) (_ 30))) (export main)))
  (call main (: 7 Int64))
  (output (: 30 Int64)))

(case
  "a two-arm if selecting between two runtime FLOAT values computes the chosen one (float select-ification)"
  (doc
    "The select-ification cases above select between INT leaves; this pins the FLOAT-leaf face. A
           2-arm `(if (> b 0) x y)` over runtime Float64 `x`/`y` may be lowered to a branchless `select`
           (an `f64.select` on wasm / a Rust conditional move) — both operands are trap-free float values,
           so evaluating both is safe. b=5 → the then-value `x` = 1.5; b=-5 → the else-value `y` = 2.5. Pins
           that the if→select conversion carries the correct FLOAT operand to the result (a select on the
           wrong width, or one that swapped the operands, would return the other value), both backends. The
           float leaf exercises the `f64.select` emit path distinct from the int-leaf select cases above.")
  (input (do (def (main (: b Int64) (: x Float64) (: y Float64)) (if (> b 0) x y)) (export main)))
  (call main (: 5 Int64) (: 1.5 Float64) (: 2.5 Float64))
  (output (: 1.5 Float64))
  (call main (: -5 Int64) (: 1.5 Float64) (: 2.5 Float64))
  (output (: 2.5 Float64)))

(case
  "a two-arm if selecting between two runtime NARROW UInt8 values computes the chosen one at the payload width"
  (doc
    "The narrow-int-leaf face of select-ification (the int cases above select Int64 leaves, the case
           above selects Float64): a 2-arm `(if (> b 0) x y)` over runtime `UInt8` `x`/`y` may lower to a
           branchless `select` at the NARROW payload width — both operands are trap-free, so evaluating both
           is safe. `x`/`y` are 100/200 (200 has the high bit set, where a select at the wrong width could
           corrupt or sign-extend): b=5 → 100, b=-5 → 200. Pins the if→select carries the correct UInt8
           operand at its own width (a select hardcoded to i32/i64, or one that swapped the operands, would
           give a wrong value) — the narrow-width companion of the Int64/Float64 select cases, and the
           select-shape twin of the sum-match-scrutinee-spill-at-payload-width fix. Both backends.")
  (input (do (def (main (: b Int64) (: x UInt8) (: y UInt8)) (if (> b 0) x y)) (export main)))
  (call main (: 5 Int64) (: 100 UInt8) (: 200 UInt8))
  (output (: 100 UInt8))
  (call main (: -5 Int64) (: 100 UInt8) (: 200 UInt8))
  (output (: 200 UInt8)))

(case
  "a two-arm if selecting between two runtime SIGNED Int8 values preserves the sign of the chosen leaf"
  (doc
    "The SIGNED-narrow face of select-ification: a 2-arm `(if (> b 0) x y)` over runtime `Int8` `x`/`y`
           lowers to a branchless `select` at the narrow payload width, and must preserve the operand's SIGN
           — a select that zero-extended (instead of sign-extending) the narrow leaf would corrupt a NEGATIVE
           value. `x` = -50 (sign bit set), `y` = 120: b=5 → -50 (the negative then-value survives intact),
           b=-5 → 120. Pins the if→select carries the correct SIGNED Int8 operand at its own width with its
           sign — the signed companion of the UInt8 select above (which used an unsigned high-bit value); the
           negative leaf is what a sign-mishandling select would get wrong. Both backends.")
  (input (do (def (main (: b Int64) (: x Int8) (: y Int8)) (if (> b 0) x y)) (export main)))
  (call main (: 5 Int64) (: -50 Int8) (: 120 Int8))
  (output (: -50 Int8))
  (call main (: -5 Int64) (: -50 Int8) (: 120 Int8))
  (output (: 120 Int8)))

(case
  "a dense zero-based jump-table match routes an out-of-range scrutinee to the default"
  (doc
    "`(def (main (: x Int64)) (match x (0 10) (1 20) (2 30) (3 40) (_ 50)))` — a dense match over
           0..3 compiles to a `br_table` jump. Because the covered range starts at 0, the table index is
           the scrutinee directly (the `x - 0` shift is elided). Called with -1: a NEGATIVE scrutinee, as
           an unsigned table index, is huge and out of range, so the out-of-range guard routes it to the
           default 50 — NOT a spurious in-range hit. Pins that eliding the zero-shift keeps the unsigned
           out-of-range guard intact (a negative or too-large scrutinee still defaults).")
  (input (do (def (main (: x Int64)) (match x (0 10) (1 20) (2 30) (3 40) (_ 50))) (export main)))
  (call main (: -1 Int64))
  (output (: 50 Int64)))

(case
  "a dense zero-based jump-table match hits a covered arm"
  (doc
    "The covered-arm companion of the zero-based `br_table` match `(match x (0 10) (1 20) (2 30)
           (3 40) (_ 50))`: called with 2, the table index 2 (the scrutinee used directly, no shift)
           selects arm 2 → 30. Together with the out-of-range case this pins value parity of the
           zero-shift-elided jump table with the shifted form.")
  (input (do (def (main (: x Int64)) (match x (0 10) (1 20) (2 30) (3 40) (_ 50))) (export main)))
  (call main (: 2 Int64))
  (output (: 30 Int64)))

(case
  "a loop whose bound is an invariant computation runs correctly (the bound is hoisted)"
  (doc
    "`(def (go (: i Int64) (: n Int64) (: acc Int64)) (if (< i (* n 2)) (go (+ i 1) n (+ acc i)) acc))`
           — the loop bound `(* n 2)` is loop-INVARIANT (n threads unchanged), so the compiler hoists it
           out of the loop, computing it ONCE before the loop instead of every iteration. Called via
           `(go 0 x 0)` with x = 4: the loop runs for i in [0, 8), summing 0+1+…+7 = 28. Pins that a
           hoisted invariant loop-bound still drives the loop correctly.")
  (input
    (do
      (def
        (go (: i Int64) (: n Int64) (: acc Int64))
        (if (< i (* n 2)) (go (+ i 1) n (+ acc i)) acc))
      (def (main (: x Int64)) (go 0 x 0))
      (export main)))
  (call main (: 4 Int64))
  (output (: 28 Int64)))

(case
  "a hoisted trapping invariant loop-bound still traps when it overflows, even at zero iterations"
  (doc
    "The same invariant-bound loop `(if (< i (* n 2)) …)`, but called with x so large that `x * 2`
           overflows Int64. The loop would run ZERO body iterations (i=0 is not < a would-be-huge bound),
           yet the program MUST still trap: the loop CONDITION evaluates `(* n 2)` on the entry check
           regardless, and that checked multiply overflows. Pins that hoisting the trapping invariant out
           of the loop is TRAP-EQUIVALENT — it traps on the entry check exactly as the un-hoisted form
           would, so the zero-iteration case is not silently spared its overflow trap.")
  (input
    (do
      (def
        (go (: i Int64) (: n Int64) (: acc Int64))
        (if (< i (* n 2)) (go (+ i 1) n (+ acc i)) acc))
      (def (main (: x Int64)) (go 0 x 0))
      (export main)))
  (call main (: 5000000000000000000 Int64))
  (trap "integer overflow"))

(case
  "a loop-invariant computed in both the condition and the body is shared, computing correctly"
  (doc
    "`(def (go (: i Int64) (: n Int64) (: acc Int64)) (if (< i (* n 2)) (go (+ i 1) n (+ acc (* n 2))) acc))`
           — the invariant `(* n 2)` appears in BOTH the loop condition and the body accumulation. LICM
           computes it once before the loop and BOTH occurrences read that one value (value-numbered
           hoist), so the body no longer recomputes `n * 2` each iteration. Called via `(go 0 x 0)` with
           x = 3: the bound is 6, the loop runs for i in [0, 6), adding `n*2 = 6` each iteration → 6 * 6 =
           36. Pins that sharing the invariant across the condition and body preserves the value.")
  (input
    (do
      (def
        (go (: i Int64) (: n Int64) (: acc Int64))
        (if (< i (* n 2)) (go (+ i 1) n (+ acc (* n 2))) acc))
      (def (main (: x Int64)) (go 0 x 0))
      (export main)))
  (call main (: 3 Int64))
  (output (: 36 Int64)))

; ── A PURE straight-line SHARED SUBEXPRESSION preserves its value however it is optimized ────────────
; `(+ (* x x) (* x x))` computes the SAME pure subexpression `(* x x)` twice over a runtime `x`. The wasm
; backend's dominator CSE value-numbers it into one slot computed once (backend-independent in RESULT: the
; VALUE must not change); the RUST backend emits the expression as written and lets `rustc` do the CSE
; downstream. Either way the observed value is identical — this is why the pure-CSE optimization is NOT a
; separate Core pass the rust backend re-implements (rustc covers the rust path), distinct from the
; EFFECTFUL-CSE soundness pins in 14-effects (where a shared node must NOT be reused). x=5 → 25+25 = 50,
; x=-4 → 16+16 = 32. A value-parity pin: the shared subexpression computes the same result on both backends.
(case
  "a pure shared subexpression computes the same value however the backend optimizes it"
  (doc
    "`(+ (* x x) (* x x))` over a runtime `x` shares the pure subexpression `(* x x)`. Whether the
           backend value-numbers it into one computed slot (wasm dominator CSE) or emits it twice and lets
           the downstream compiler fold it (rust → rustc), the observed VALUE is unchanged: x=5 → 50, x=-4
           → 32. Pins that a pure CSE candidate preserves its value on both backends — the pure companion
           of the effectful-CSE must-NOT-share soundness pins (14-effects-and-handlers), and the reason a
           pure-CSE is left to each backend's own optimizer rather than lifted to a Core pass.")
  (input (do (def (main (: x Int64)) (+ (* x x) (* x x))) (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64))
  (call main (: -4 Int64))
  (output (: 32 Int64)))

; ── A COMMON OPERATOR/COMPARISON hoisted out of both if arms is value-transparent (build/compute once) ─
; `(if c (op a …) (op b …))` shares the operator across both arms; the backend may hoist it to `(op (if c
; a b) …)` — the op applied ONCE over the SELECTED operand (the differing operand becomes a branchless
; select). Value-transparent, and crucially operand SELECTION not eager evaluation of BOTH arms: a checked
; op's overflow guard fires iff the TAKEN arm overflows, so the untaken arm's would-be trap is never
; evaluated. These pin the observable value/trap across the arith, integer-compare, and float
; equality/ordering (incl. NaN) faces, plus the repeated-compare CSE.
(case
  "a common operator hoisted out of both if arms selects the operand and computes once"
  (doc
    "`(if c (+ a 1) (+ b 1))` shares `(+ _ 1)`, hoistable to `(+ (if c a b) 1)` — the checked add and
           its overflow guard applied ONCE over the selected operand. Value both directions: c=true,a=5 -> 6;
           c=false,b=9 -> 10. And it is operand SELECTION, not eager evaluation of both: c=true with
           a=Int64.max TRAPS (the taken (+ a 1) overflows), but c=false with the SAME a=Int64.max does NOT
           trap (b is selected; the untaken arm's overflow is never evaluated). A hoist that eagerly added
           both operands would wrongly trap the c=false case.")
  (input (do (def (main (: c Bool) (: a Int64) (: b Int64)) (if c (+ a 1) (+ b 1))) (export main)))
  (call main (: true Bool) (: 5 Int64) (: 9 Int64))
  (output (: 6 Int64))
  (call main (: false Bool) (: 5 Int64) (: 9 Int64))
  (output (: 10 Int64))
  (call main (: true Bool) (: 9223372036854775807 Int64) (: 9 Int64))
  (trap "overflow")
  (call main (: false Bool) (: 9223372036854775807 Int64) (: 9 Int64))
  (output (: 10 Int64)))

(case
  "a common integer comparison hoisted out of both if arms compares once over the selected operand"
  (doc
    "`(if (if c (< a k) (< b k)) 1 0)` shares `(< _ k)`, hoistable to `(< (if c a b) k)`. A comparison
           is total, so the hoist is unconditionally value-safe: (c,a,b,k)=(true,3,9,5) -> (< 3 5)=true -> 1;
           (false,3,9,5) -> (< 9 5)=false -> 0; (true,9,3,5) -> (< 9 5)=false -> 0.")
  (input
    (do
      (def (main (: c Bool) (: a Int64) (: b Int64) (: k Int64)) (if (if c (< a k) (< b k)) 1 0))
      (export main)))
  (call main (: true Bool) (: 3 Int64) (: 9 Int64) (: 5 Int64))
  (output (: 1 Int64))
  (call main (: false Bool) (: 3 Int64) (: 9 Int64) (: 5 Int64))
  (output (: 0 Int64))
  (call main (: true Bool) (: 9 Int64) (: 3 Int64) (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a common float equality hoisted out of both if arms compares once over the selected operand"
  (doc
    "The Float64 face: `(if (if c (= a k) (= b k)) 1 0)` shares `(= _ k)`, hoistable to `(= (if c a b)
           k)` — one canonical-byte float compare over the selected operand (total, never traps). (c,a,b,k):
           (true,1.0,9.0,1.0) -> (= 1 1)=true -> 1; (false,1.0,9.0,1.0) -> (= 9 1)=false -> 0;
           (false,1.0,2.0,2.0) -> (= 2 2)=true -> 1; (true,9.0,2.0,2.0) -> (= 9 2)=false -> 0.")
  (input
    (do
      (def
        (main (: c Bool) (: a Float64) (: b Float64) (: k Float64))
        (if (if c (= a k) (= b k)) 1 0))
      (export main)))
  (call main (: true Bool) (: 1.0 Float64) (: 9.0 Float64) (: 1.0 Float64))
  (output (: 1 Int64))
  (call main (: false Bool) (: 1.0 Float64) (: 9.0 Float64) (: 1.0 Float64))
  (output (: 0 Int64))
  (call main (: false Bool) (: 1.0 Float64) (: 2.0 Float64) (: 2.0 Float64))
  (output (: 1 Int64))
  (call main (: true Bool) (: 9.0 Float64) (: 2.0 Float64) (: 2.0 Float64))
  (output (: 0 Int64)))

(case
  "a common float ordering hoist reproduces the IEEE partial order including NaN"
  (doc
    "`(if (if c (< a k) (< b k)) 1 0)` over Float64 hoists to `(< (if c a b) k)` — one f64.lt over the
           selected operand. f64.lt is total (NaN compares false, no trap), so the hoist is value-identical
           for every input including NaN: (true,1,9,5) -> (< 1 5)=true -> 1; (false,1,9,5) -> (< 9 5)=false
           -> 0; (true,NaN,1,5) selects a=NaN -> (< NaN 5)=false -> 0; (false,1,NaN,5) selects b=NaN -> (<
           NaN 5)=false -> 0.")
  (input
    (do
      (def
        (main (: c Bool) (: a Float64) (: b Float64) (: k Float64))
        (if (if c (< a k) (< b k)) 1 0))
      (export main)))
  (call main (: true Bool) (: 1.0 Float64) (: 9.0 Float64) (: 5.0 Float64))
  (output (: 1 Int64))
  (call main (: false Bool) (: 1.0 Float64) (: 9.0 Float64) (: 5.0 Float64))
  (output (: 0 Int64))
  (call main (: true Bool) (: NaN Float64) (: 1.0 Float64) (: 5.0 Float64))
  (output (: 0 Int64))
  (call main (: false Bool) (: 1.0 Float64) (: NaN Float64) (: 5.0 Float64))
  (output (: 0 Int64)))

(case
  "a repeated float comparison is computed once and reused (value-transparent CSE)"
  (doc
    "`(+ (if (< a b) 10 0) (if (< a b) 20 0))` uses the same `(< a b)` twice; the backend value-numbers
           it to one f64.lt computed once and reused by both consumers — value-transparent. a=1,b=2 -> (< 1
           2)=true -> 10 + 20 = 30; a=2,b=1 -> false -> 0 + 0 = 0.")
  (input
    (do
      (def (main (: a Float64) (: b Float64)) (+ (if (< a b) 10 0) (if (< a b) 20 0)))
      (export main)))
  (call main (: 1.0 Float64) (: 2.0 Float64))
  (output (: 30 Int64))
  (call main (: 2.0 Float64) (: 1.0 Float64))
  (output (: 0 Int64)))

; The DEEP UNIFORM companion of the shallow `(* x x)` share above. A deep left-nested accumulator chain
; `(+ (+ … (+ (+ p (* p 0)) (* p 1)) …) (* p 7))` is the shape the wasm CSE class-partition buckets by a
; full-depth structural hash: every `(+ …)` node has the SAME shallow key (`Arith(Add)` over `[Arith,Arith]`)
; and every `(* p k)` the SAME shallow key (`Arith(Mul)` over `[Param,ConstInt]`), so a SHALLOW bucket hash
; would collide the whole chain into one bucket and the within-bucket `core_eq` scan degrades to O(N²) deep
; compares. The full-depth hash distinguishes each `(* p k)` and each chain prefix → distinct buckets → linear
; partition. That LINEARITY is guarded by an `rcdzc --lib` perf pin (a compare-count bound) — but `xtask gate`
; SKIPS `--lib`, so this case is the fleet-visible VALUE witness of the same shape: whatever the partition
; does, the emitted result must stay correct at every opt level. p=2 → 2 + Σ(2·k for k=0..7) = 2 + 2·28 = 58;
; p=0 → 0. A value-parity pin over the deep-uniform CSE shape (the deep companion of `(* x x)` above).
(case
  "a deep uniform arith accumulator chain (CSE-partition shape) computes the same value across opt levels"
  (doc
    "`(+ (+ (+ … (+ p (* p 0)) (* p 1)) …) (* p 7))` — a deep left-nested chain of the SAME-shaped
           `(* p k)` terms, the uniform shape the wasm CSE partition full-depth-hash-buckets (a shallow hash
           collides all terms into one bucket → O(N²) within-bucket compares; the full-depth hash keeps them
           on distinct buckets → linear). Its LINEARITY is a `--lib` perf pin, but the gate skips `--lib`, so
           this pins the fleet-visible VALUE: p=2 → 2 + (2·0 + 2·1 + … + 2·7) = 2 + 56 = 58; p=0 → 0. The
           emitted output must be correct however the partition buckets — the deep-uniform companion of the
           shallow `(* x x)` share above.")
  (input
    (do
      (def
        (main (: p Int64))
        (+
          (+ (+ (+ (+ (+ (+ (+ p (* p 0)) (* p 1)) (* p 2)) (* p 3)) (* p 4)) (* p 5)) (* p 6))
          (* p 7)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 58 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a loop-invariant subexpression in a MATCH scrutinee is hoisted to valid wasm"
  (doc
    "`(match (< i (+ n 1)) (true (loop (+ i 1) n)) (false i))` — a tail-recursive counted loop whose
           MATCH scrutinee `(< i (+ n 1))` contains the loop-invariant `(+ n 1)` (`n` threads unchanged).
           The match scrutinee is an ALWAYS-EVALUATED (dominating-frontier) position, so LICM hoists `(+ n
           1)` into a pre-loop slot. REGRESSION guard (9bccb36a): the hoisted checked-add's transient
           overflow-guard slot was left inside the body's reusable scratch range, so the loop body reused it
           for the i32 bool discriminant while the hoist recorded it at i64 — the one wasm local was declared
           at two widths and the module failed validation (`func 1 … type mismatch: expected i32, found
           i64`), rejecting the component at load. The if-condition twin `(if (< i (+ n 1)) …)` was fine;
           only the match-scrutinee position mis-wired. `loop 0 4` counts i:0→5 while `i < n+1 (=5)` and
           returns 5. Fix: raise the body scratch floor past ALL scratch the invariant's emit touched, not
           just the persistent value slot.")
  (input
    (do
      (def (loop (: i Int64) (: n Int64)) (match (< i (+ n 1)) (true (loop (+ i 1) n)) (false i)))
      (def (main) (loop 0 4))
      (export main)))
  (output (: 5 Int64)))

; A lexical binding wins in APPLICATION-HEAD position too, not only value position — even when its
; name coincides with a built-in constructor form like `list`/`tuple`/`record`/`map`. core-semantics.md
; #Binding Is Lexical: "A name MUST resolve to the nearest enclosing binding of that name." A `let`
; binding named `list` shadows the built-in list constructor for the extent of its scope, so `(list 3 4)`
; in its body applies the bound function (the nearest binding), yielding 7 — not the built-in list value
; `(list 3 4)`. A compiler that dispatches an application by matching the head STRING against its built-in
; forms before consulting the environment never sees the shadowing binding in head position: it resolves
; a bare `list` reference to the binding (value position) but silently prefers the built-in when `list`
; heads an application — resolving one name two ways by syntactic position, which #Binding Is Lexical
; forbids. (The value-position companion — `(let ((list 99)) list)` = 99 — already holds; this pins the
; head-position half.)
(case
  "a let binding shadows a built-in constructor name in application-head position"
  (doc
    "`(let ((list (fn (a b) (+ a b)))) (list 3 4))` binds `list` to a function, then applies it in
           head position: `list` MUST resolve to the nearest enclosing binding (core-semantics.md #Binding
           Is Lexical), so `(list 3 4)` = 7, NOT the built-in list value `(list 3 4)`. Pins that
           application-head resolution consults the lexical environment before the built-in constructor
           forms — a compiler matching the head name `list` against its built-ins first ignores the
           shadowing binding and builds a two-element list, resolving `list` to the binding in value
           position but to the built-in in head position (the same name, two ways). A generation that does
           not realize a shadowing built-in name declines rather than choosing the built-in.")
  (input (let ((list (fn (a b) (+ a b)))) (list 3 4)))
  (output (: 7 Int64)))

(case
  "a let binding shadows a built-in type-module name in value position"
  (doc
    "Name resolution is ONE ordered lookup (prelude-and-resolution.md §Name Resolution Is One Ordered
           Lookup): the scope-first rule means a `let`-bound `Int64` HIDES the built-in `Int64` module for
           the extent of its scope, with no special case for built-in type names. `(let ((Int64 5)) Int64)`
           = 5, the binding — not the module. The value-position, type-module-name companion of the
           constructor-head shadow above (which shadows `list`/`tuple`/`record` in head position).")
  (input (let ((Int64 5)) Int64))
  (output (: 5 Int64)))

; The SAME head-position shadowing holds for `tuple` and `record` — the compound-VALUE constructors.
; They are ordinary shadowable names bound in the prelude (aliases for the primitive symbol
; constructors `(,)` and `{}`, which a program cannot spell), so a `let`/`def`/parameter binding of
; `tuple`/`record` shadows the built-in exactly as one of `list` does: `(tuple 3 4)` in the binding's
; scope applies the bound function, yielding 7 — NOT the built-in tuple value `(tuple 3 4)`. A compiler
; that dispatches a `tuple`/`record` head STRUCTURALLY (matching the head name before consulting the
; environment) resolves the name two ways by syntactic position — the built-in in head position, the
; binding in value position — which #Binding Is Lexical forbids. (core-semantics.md §A Compound Value
; Has A Symbol Constructor And A Shadowable Alias: the name is looked up like any other; the primitive
; is the symbol.)
(case
  "a let binding shadows the tuple constructor in application-head position"
  (doc
    "The `tuple` sibling of the recorded `list` head-position shadow: `(let ((tuple (fn (a b) (+ a
           b)))) (tuple 3 4))` applies the nearest binding, yielding 7 — not the built-in tuple value
           `(tuple 3 4)`. `tuple` is a shadowable prelude alias for the primitive symbol constructor
           `(,)`, so a binding named `tuple` shadows it; head-position resolution consults the lexical
           environment first. Earlier the seed answered `(tuple 3 4)` — the structural grammar dispatch
           on the head name won over the binding (a wrong value, the one-name-two-resolutions bug).")
  (input (do (def (main) (let ((tuple (fn (a b) (+ a b)))) (tuple 3 4))) (export main)))
  (output (: 7 Int64)))

(case
  "a let binding shadows the record constructor in application-head position"
  (doc
    "The `record` sibling: `(let ((record (fn (a b) (+ a b)))) (record 3 4))` applies the bound
           function in its scope, yielding 7 — `record` is a shadowable prelude alias for the primitive
           symbol constructor `{}`. Earlier the seed instead REJECTED with CDZ0201 'record field must be
           (key value)' — the built-in record form's shape check fired on an application of a lexically
           bound function: a spurious rejection of a well-typed program, the same head-vs-value split.")
  (input (do (def (main) (let ((record (fn (a b) (+ a b)))) (record 3 4))) (export main)))
  (output (: 7 Int64)))

(case
  "the string primitive tuple constructor is not shadowed by a same-named binding"
  (doc
    "The converse of the alias-shadowing cases above: the primitive tuple constructor is the
           UNSPELLABLE symbol, and the STRING literal `\"tuple\"` names it directly — a distinct leaf kind
           from the shadowable NAME alias `tuple`. So even when the name `tuple` is shadowed by a binding,
           the string primitive `\"tuple\"` still builds a tuple: `(let ((tuple (fn (a b) (+ a b)))) (. (\"tuple\"
           7 8) 0))` = 7 (the first element of the built tuple), NOT the bound function. A binding shadows
           the alias name, never the string primitive (core-semantics.md §A Compound Value Has A Symbol
           Constructor And A Shadowable Alias — the string spelling reaches the symbol constructor itself).")
  (input (do (def (main) (let ((tuple (fn (a b) (+ a b)))) (. #tuple(7 8) 0))) (export main)))
  (output (: 7 Int64)))

(case
  "a parameter named tuple is applied as the bound function"
  (doc
    "The parameter companion: `(def (f tuple) (tuple 3 4))` — the formal `tuple` is the nearest
           binding, so applying it calls the argument function. `(f (fn (a b) (* a b)))` = 12. Pins that
           a parameter shadows the `tuple` alias exactly as a `let` binding does — the name resolves to
           the parameter in head position, not the built-in constructor.")
  (input (do (def (f tuple) (tuple 3 4)) (def (main) (f (fn (a b) (* a b)))) (export main)))
  (output (: 12 Int64)))

(case
  "a shadowed-constructor application types at the binding's return type"
  (doc
    "The head-position misresolution was a TYPE-soundness bug too: the shadowing binding returns
           Int64, so `(+ (let ((tuple (fn (a b) (+ a b)))) (tuple 3 4)) 1)` = (3+4)+1 = 8. Earlier the
           seed REJECTED with CDZ0203 'cannot unify Int64 with (Tuple Int64 Int64)' — inference resolved
           the head to the built-in tuple constructor, typing the application as a Tuple, so the outer
           `+ … 1` failed to unify. Resolving the head to the lexical binding fixes the value AND the
           type: the same name no longer has two types by syntactic position.")
  (input (do (def (main) (+ (let ((tuple (fn (a b) (+ a b)))) (tuple 3 4)) 1)) (export main)))
  (output (: 8 Int64)))

; --- The bindings of one `let` take effect in order (let*, not parallel) --------------------
; core-semantics.md #The Bindings Of One `let` Take Effect In Order: each binding's initializer sees
; the bindings written before it in the SAME let, so `(let ((x 1) (y (+ x 1))) y)` is 2 — `y`'s
; initializer observes `x`. Under a PARALLEL reading `y`'s initializer would evaluate in the enclosing
; scope where `x` is unbound (a CDZ0101 rejection); the sequential reading, which the seed realizes,
; is the recorded oracle.
(case
  "a later let binding sees an earlier one in the same let"
  (doc
    "`(let ((x 1) (y (+ x 1))) y)` = 2: the second binding's initializer `(+ x 1)` observes the
           first binding `x`, so the bindings of one `let` take effect in order (core-semantics.md
           #The Bindings Of One `let` Take Effect In Order), not in parallel where `x` would be unbound
           in `y`'s initializer.")
  (input (let ((x 1) (y (+ x 1))) y))
  (output (: 2 Int64)))

(case
  "a chain of let bindings each referencing the immediately-preceding one accumulates"
  (doc
    "The transitive form of in-order binding: a `let` whose every binding references the one written
           just before it — realistic accumulation code. Each `v_i` initializer sees `v_{i-1}` (and only
           the earlier ones), so `v3` is the running sum 0+1+2+3 = 6. Pins that each binder in a chain
           resolves to its immediate predecessor, none mis-attributed to a later same-scope binding.")
  (input (let ((v0 0) (v1 (+ v0 1)) (v2 (+ v1 2)) (v3 (+ v2 3))) v3))
  (output (: 6 Int64)))

(case
  "a repeated let binding shadows the earlier one for what follows"
  (doc
    "`(let ((x 1) (x (+ x 10))) x)` = 11: the second binding of `x` shadows the first for the
           initializers and body that follow, and its initializer `(+ x 10)` sees the first `x` = 1
           (core-semantics.md #The Bindings Of One `let` Take Effect In Order + #Shadowing Is
           Well-Defined). The sequential companion of the case above at a repeated name.")
  (input (let ((x 1) (x (+ x 10))) x))
  (output (: 11 Int64)))

(case
  "a nested-let chain that reuses each binding folds to one value"
  (doc
    "Each `let` binding is referenced TWICE by the next, ten deep: `a = 1+1`, `b = a+a`, …,
           result `j+j`. Every binding is used more than once, so a compiler that re-evaluates a
           binding's initializer on each reference does exponential (2^depth) work; folding each
           binding ONCE and reusing its value is linear. `(+ j j)` = 2·2^10 = 2048. Pins that a
           `let` binding denotes a single value shared by all its references (core-semantics.md
           #The Bindings Of One `let` Take Effect In Order) — the same value whether read once or
           ten times — so the answer is independent of how the compiler memoizes the fold. (The
           observable is the value; the doubling structure is what makes a non-memoizing fold blow
           up, so this doubles as a compile-time-cost regression guard.)")
  (input
    (let
      ((a (+ 1 1)))
      (let
        ((b (+ a a)))
        (let
          ((c (+ b b)))
          (let
            ((d (+ c c)))
            (let
              ((e (+ d d)))
              (let
                ((f (+ e e)))
                (let
                  ((g (+ f f)))
                  (let ((h (+ g g))) (let ((i (+ h h))) (let ((j (+ i i))) (+ j j))))))))))))
  (output (: 2048 Int64)))

(case
  "a deep chain of runtime-list let-bindings compiles and returns the final length"
  (doc
    "The RUNTIME (heap-valued) companion of the fold above: twelve nested `let`s, each binding a
           runtime `list` grown from the previous by `List.push`, ending in `(List.len l12)` = 12. Each
           binding is a genuine value-heap handle (not a compile-time constant), so it is materialized
           as a real local — but the compiler captures the enclosing scope at each `let` for name
           resolution, and if that capture DEEP-CLONES the environment, the nested captures nest
           ~2^depth copies and compilation blows its memory (the 'compile is 2ⁿ in `let` nesting'
           ceiling). Sharing the captured environment makes the cost linear in depth. Pins that a deep
           chain of runtime-compound `let`s compiles at all (and to the right value) — the shape a
           compiler's threaded state / accumulator passes take. The observable is 12; the DEPTH is the
           compile-time-cost regression guard (this depth exhausted memory before the fix).")
  (input
    (let
      ((l1 (List.push #list() 1)))
      (let
        ((l2 (List.push l1 2)))
        (let
          ((l3 (List.push l2 3)))
          (let
            ((l4 (List.push l3 4)))
            (let
              ((l5 (List.push l4 5)))
              (let
                ((l6 (List.push l5 6)))
                (let
                  ((l7 (List.push l6 7)))
                  (let
                    ((l8 (List.push l7 8)))
                    (let
                      ((l9 (List.push l8 9)))
                      (let
                        ((l10 (List.push l9 10)))
                        (let
                          ((l11 (List.push l10 11)))
                          (let ((l12 (List.push l11 12))) (List.len l12))))))))))))))
  (output (: 12 Int64)))

(case
  "resolving a name in a shadowing environment returns the innermost binding's slot"
  (doc
    "The compiler-internal SCOPE-RESOLUTION idiom behind lexical shadowing (the value-level cases
           above pin the observable; this pins how a name resolver realizes it). A name environment is a
           list of bound names in scope order (a self-hosted compiler holds parameters and `let`
           bindings this way, resolving a name reference to a local slot). When a name is bound twice —
           an inner `let` shadowing an outer binding of the same name — resolution must return the
           INNERMOST (latest, highest-slot) binding, not the first. `pos` searches the environment
           deepest-first and returns the last matching position: for env `[5, 7, 5]` (name 5 bound at
           slot 0, shadowed at slot 2), looking up 5 yields 2 — the shadowing binding — not 0. Pins that
           a recursive deepest-first environment search realizes lexical shadowing correctly (a
           first-match search would wrongly return the shadowed outer slot 0). Absence is an
           `(Option Int64)` — `(None unit)` at the empty environment, `(Some k)` at a hit — not an
           in-band sentinel: `pos` returns a typed Option and the recursion propagates a deeper `Some`
           or falls back to the current frame, so `main` matches the result (the present name yields
           `(Some 2)`; the `None` arm is unreachable in this witness → trap). This is the `bytes →
           local-slot` name resolution a reader performs, the runtime dual of the `let`-shadowing
           value semantics above.")
  (input
    (do
      (type Env ENil (ECons (Tuple Int64 Env)))
      (def
        (pos xs target k)
        (match
          xs
          ((Env.ENil _) (None unit))
          ((Env.ECons #tuple(h t))
            (match
              (pos t target (+ k 1))
              ((Some d) (Some d))
              ((None _u) (if (= h target) (Some k) (None unit)))))))
      (def
        (main)
        (match
          (pos (Env.ECons #tuple(5 (Env.ECons #tuple(7 (Env.ECons #tuple(5 (Env.ENil ()))))))) 5 0)
          ((Some p) p)
          ((None _u) (trap "unreachable"))))
      (export main)))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "a reference to an unbound name is rejected before running"
  (doc
    "Witnesses core-semantics.md #Binding Is Lexical: a reference to a name with no enclosing
           binding is refused. This is a front-end rejection every generation makes — scope resolution
           needs no static typing — so (error CDZ0101) is the recorded outcome.")
  (input y)
  (error CDZ0101))

; A REACHABLE unbound name is found by BOTH the type-check walk AND the reached-poison walk, so it must be
; reported ONCE (deduped by code+node), not twice — `(def (main) nope)` yields exactly one CDZ0101. But TWO
; DISTINCT occurrences of the same unbound name (`(+ nope nope)`, at different source nodes) are NOT
; duplicates: each has its own source location, so both are reported. (Migrated from rcdzc
; the_same_fault_is_reported_once_even_when_two_passes_find_it.)
(case
  "a reachable unbound name found by two passes is reported exactly once"
  (input (do (def (main) nope) (export main)))
  (error CDZ0101 (count 1))
  (diagnostic-quality))

(case
  "two distinct occurrences of the same unbound name are each reported, not merged to one"
  (input (do (def (main) (+ nope nope)) (export main)))
  (error CDZ0101 (count 2))
  (diagnostic-quality))

; The unbound-name check (and well-formedness generally) applies to EVERY definition in a module, not
; only the ones reachable from `main`. core-semantics.md #Binding Is Lexical: "A reference to a name with
; no enclosing binding MUST be a compile-time error" — unconditionally, with no reachability qualifier.
; And a module's definitions are its EXPORTS, each reachable by member access (#A Module Evaluates To A
; Record Of Its Exports; #A Module's Exported Definition Is Reachable By Member Access), so a `(def (bad)
; nonexistent)` is not dead code — it is an export `(. m bad)` whose body must resolve. A compiler that
; type-checks and scope-checks only the functions transitively CALLED by `main` lets an ill-formed unused
; definition through: `(module m (def (bad) nonexistent) (def (main) 42))` compiles and runs to 42, its
; `bad` body's unbound `nonexistent` never checked. That contradicts the unconditional binding rule (and
; #A Program That Is Not Well-Typed Is Rejected — "every expression has a statically determined type",
; which includes every definition's body). An inner-module sibling in the same shape IS checked today
; (its unused ill-typed export is rejected), so this is specifically the TOP-LEVEL module's uncalled defs.
(case
  "an unbound name in an uncalled sibling definition is still rejected"
  (doc
    "`(def (bad) nonexistent)` references the unbound name `nonexistent`; even though `main` never
           calls `bad`, the program MUST be rejected (CDZ0101, core-semantics.md #Binding Is Lexical — the
           unbound-name rule is unconditional, not gated on reachability from `main`). A module's
           definitions are its exports, each reachable by member access, so `bad`'s body is not dead code
           and must resolve. A compiler that scope-checks only the functions `main` transitively calls lets
           an ill-formed uncalled definition through, running to 42 instead of rejecting. Pins that every
           top-level definition's body is checked, exactly as an inner-module sibling's already is.")
  (input (do (def (bad) nonexistent) (def (main) 42) (export main)))
  (error CDZ0101))

; The unbound-name check also reaches into an UNSELECTED conditional branch, not only uncalled top-level
; definitions. core-semantics.md #Binding Is Lexical: "A reference to a name with no enclosing binding
; MUST be a compile-time error" (unconditional); #Conditionals Evaluate One Branch: "Every branch … MUST
; be type-checked whether or not it is evaluated." So `(if true 1 undefined-name)` MUST be rejected
; (CDZ0101) even though the constant condition selects the `1` branch and the `undefined-name` branch is
; never evaluated — an unevaluated branch cannot carry a deferred scope error any more than a deferred
; type error. A compiler that const-folds the conditional to its taken branch and scope-checks only that
; branch lets the unbound reference in the dropped branch slip through, running to 1. (The `if` form
; already catches a TYPE error in an unselected branch — `(if true 1 (+ 1 true))` is rejected — so the
; scope check must reach the same unselected branch the type check already does.)
(case
  "an unbound name in an unselected conditional branch is still rejected"
  (doc
    "`(if true 1 undefined-name)` references the unbound name `undefined-name` in the else-branch;
           even though the constant condition `true` selects the `1` branch, the program MUST be rejected
           (CDZ0101, core-semantics.md #Binding Is Lexical — unconditional — with #Conditionals Evaluate
           One Branch: every branch type-checked whether or not evaluated). An unevaluated branch cannot
           carry a deferred scope error. A compiler that const-folds the conditional to its taken branch and
           scope-checks only that branch runs to 1 instead of rejecting. Pins that scope resolution reaches
           an unselected branch, exactly as the type check already does (`(if true 1 (+ 1 true))` is
           rejected). A generation that does not yet scope-check the dropped branch declines.")
  (input (if true 1 undefined-name))
  (error CDZ0101))

; The same unbound-name check reaches a boolean connective's SHORT-CIRCUITED operand, exactly as it
; reaches an unselected conditional branch — the spec makes the two identical. core-semantics.md #Boolean
; Connectives Short-Circuit: "a connective shields a trapping or effectful right operand exactly as the
; unselected branch of a conditional does", and "Each operand of a boolean connective MUST be type-checked
; as a boolean whether or not it is evaluated, so that an unevaluated operand cannot carry a deferred
; error, exactly as every branch of a conditional is type-checked." So `(and false undefined-name)` MUST
; be rejected (CDZ0101): `false` short-circuits the conjunction so the right operand is not evaluated, but
; an unevaluated operand cannot carry a deferred SCOPE error any more than a deferred type error. The seed
; already type-checks the dead operand — `(and false (+ 1 1))` is rejected "operand is not a Bool" and
; `(and false (+ 1 true))` "operation on mismatched types" — so, exactly as for the `if` case above
; (`(if true 1 (+ 1 true))` type-checks the dropped branch, and `(if true 1 undefined-name)` now
; scope-checks it too), the scope check MUST reach the same short-circuited operand the type check already
; does. A compiler that emits only the taken side of the short-circuit and scope-checks only that operand
; lets the unbound reference in the dead operand slip through, running `(and false undefined-name)` to
; `false`. A generation that does not yet scope-check the dead operand declines.
(case
  "an unbound name in a short-circuited boolean operand is still rejected"
  (doc
    "`(and false undefined-name)` references the unbound name `undefined-name` in the conjunction's
           right operand; even though the constant left operand `false` short-circuits the `and` so the
           right is never evaluated, the program MUST be rejected (CDZ0101, core-semantics.md #Binding Is
           Lexical — unconditional — with #Boolean Connectives Short-Circuit: each operand is type-checked
           whether or not it is evaluated, EXACTLY AS every branch of a conditional is). An unevaluated
           operand cannot carry a deferred scope error, the connective companion of the unselected-branch
           case above (`(if true 1 undefined-name)`). The seed already type-checks the dead operand
           (`(and false (+ 1 1))` rejects \"operand is not a Bool\"), so the scope check must reach the
           same operand the type check does. A compiler that scope-checks only the evaluated side runs to
           `false` instead of rejecting. A generation that does not yet scope-check the short-circuited
           operand declines rather than answering `false`.")
  (input (and false undefined-name))
  (error CDZ0101))

; The fixed-arity grammar forms `if`/`and`/`or`/`not` reject a wrong operand count (CDZ0201, "takes
; exactly …"). TOO MANY operands carries a delete-the-surplus fix — the same surplus-arg delete an
; over-applied operator / a too-many-operand quote gets. TOO FEW carries NO fix (nothing to delete).
; (Migrated from rcdzc a_fixed_arity_grammar_form_with_too_many_operands_offers_a_delete_fix.)
(case
  "an `if` with too many operands offers a delete-the-surplus fix"
  (input (do (def (main) (if true 1 2 3)) (export main)))
  (error CDZ0201 (message "takes exactly") (fix (kind delete))))

(case
  "an `and` with too many operands offers a delete-the-surplus fix"
  (input (do (def (main) (and true false true)) (export main)))
  (error CDZ0201 (message "takes exactly") (fix (kind delete))))

(case
  "an `or` with too many operands offers a delete-the-surplus fix"
  (input (do (def (main) (or true false true)) (export main)))
  (error CDZ0201 (message "takes exactly") (fix (kind delete))))

(case
  "a `not` with too many operands offers a delete-the-surplus fix"
  (input (do (def (main) (not true false)) (export main)))
  (error CDZ0201 (message "takes exactly") (fix (kind delete))))

(case
  "a binary arithmetic operator with too many operands offers a delete-the-surplus fix"
  (doc
    "The ARITHMETIC-operator sibling of the grammar-form over-application cluster above: `(+ n 1 2)`
           applies the binary `+` to THREE operands — a clear operator-specific CDZ0201 'takes exactly 2
           operands' (not the generic arity phrasing), carrying the SAME delete-the-surplus fix. Contrast the
           one-operand under-application `(+ n)`, which CURRIES into a partial application (07-type-system),
           not an arity error. (Migrated from rcdzc
           a_binary_operator_over_or_under_application_on_a_function_param_surfaces_in_the_query.)")
  (input (do (def (g (: n Int64)) (+ n 1 2)) (export g)))
  (error CDZ0201 (message "takes exactly 2 operands") (fix (kind delete))))

(case
  "a too-FEW-operand `and` carries no fix (nothing to delete)"
  (input (do (def (main) (and true)) (export main)))
  (error CDZ0201 (message "takes exactly") (no-fix)))

; A 2-operand `if` (`(if b then)` — the reflex of a statement-`if` language) is a wrong-arity `if`, but
; `if` is an EXPRESSION here (both branches must yield a value), so rather than the generic count nit it
; NAMES the missing else + why, and carries an INSERT fix appending a `(trap "TODO")` else — `trap` inhabits
; any type, so the completed `(if b then (trap "TODO"))` type-checks against the then-branch. A 1-operand
; `(if b)` (no then EITHER) is not a clean add-else, so it stays the generic arity message with no fix.
; (Migrated from rcdzc an_if_missing_its_else_branch_offers_to_add_one.)
(case
  "an `if` missing its else branch names the missing else and offers to add a trap placeholder"
  (input (do (def (f (: b Bool)) (if b 1)) (export f)))
  (error
    CDZ0201
    (message "no else branch")
    (message "expression")
    (fix (kind insert-into) (replacement-contains "(trap \"TODO\")"))))

(case
  "an `if` with the added trap-placeholder else branch compiles and runs"
  (input (do (def (f (: b Bool)) (if b 1 (trap "TODO"))) (def (main) (f true)) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a lone-condition `if` (no then either) keeps the generic arity message with no fix"
  (input (do (def (f (: b Bool)) (if b)) (export f)))
  (error CDZ0201 (message "takes exactly 3 operands") (no-fix)))

; A `(guard <pattern> <cond>)` match-arm head is a fixed-arity form (2 tail elements). A SURPLUS third
; element routes through the same fixed-arity reject — a delete-the-surplus fix (fix-parity with
; `if`/`and`/member). Too FEW (a lone `(guard x)`) has nothing to delete → no fix. (Migrated from rcdzc
; a_guarded_pattern_with_a_surplus_element_offers_a_delete_fix.)
(case
  "a guarded pattern with a surplus element offers a delete-the-surplus fix"
  (input (do (def (f (: n Int64)) (match n ((guard x (> x 0) extra) 1) (_ 0))) (export f)))
  (error CDZ0201 (message "a guarded pattern must be") (fix (kind delete))))

(case
  "a too-few-element guarded pattern carries no fix (nothing to delete)"
  (input (do (def (f (: n Int64)) (match n ((guard x) 1) (_ 0))) (export f)))
  (error CDZ0201 (message "a guarded pattern must be") (no-fix)))

; A `let`/`fn` whose bindings/params are present but whose trailing BODY is missing (`(let ((x 5)))`,
; `(fn (x))`) is CDZ0201 ("has no body"), and carries an INSERT fix appending a `(trap "TODO")` body —
; `trap` inhabits any type, so the completed form type-checks wherever used (the `let`/`fn` twin of the
; missing-if-else add-fix). A DEGENERATE `(let)` (no bindings AND no body) is not a one-shot add (appending
; a body still leaves a malformed bindings list), so it is message-only, no fix. (Migrated from rcdzc
; a_let_or_fn_missing_its_body_offers_to_add_one.)
(case
  "a let missing its body offers to add a trap-placeholder body"
  (input (do (def (f) (let ((x 5)))) (export f)))
  (error
    CDZ0201
    (message "has no body")
    (fix (kind insert-into) (replacement-contains "(trap \"TODO\")"))))

(case
  "a fn missing its body offers to add a trap-placeholder body"
  (input (do (def (f) ((fn (x)) 5)) (export f)))
  (error
    CDZ0201
    (message "has no body")
    (fix (kind insert-into) (replacement-contains "(trap \"TODO\")"))))

(case
  "a let with the added trap-placeholder body compiles and runs"
  (input (do (def (f) (let ((x 5)) (trap "TODO"))) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a degenerate empty let (no bindings and no body) carries no fix"
  (input (do (def (f) (let)) (export f)))
  (error CDZ0201 (message "no bindings and no body") (no-fix)))

(case
  "a let-bound variable is in scope inside a boolean connective operand"
  (doc
    "The complement of the short-circuited-unbound case above, and the boundary its scope check
           must not over-reach into: a `let`-bound (or parameter) name used in an `and`/`or` operand is
           IN SCOPE and resolves normally (core-semantics.md #Binding Is Lexical: a name resolves to its
           nearest enclosing binding). `(let ((x 3)) (and (> x 0) (< x 9)))` binds `x` and uses it in
           BOTH conjuncts, yielding true. A compiler that scope-checks a connective operand against a
           scope MISSING the enclosing `let`/parameter binders (e.g. a whole-tree type-check pass that
           does not thread block-local bindings) wrongly rejects `x` as unbound — the pair to the
           unbound case: an unbound name is rejected, a bound one is NOT. This idiom (`(let (…) (and
           (>= i 0) …))`) is pervasive in a self-hosting compiler's bounds/range guards, so the scope
           check must run where the operand's lexical environment is complete.")
  (input
    (do (def (f k) (let ((x k)) (and (> x 0) (< x 9)))) (def (main) (if (f 3) 1 0)) (export main)))
  (output (: 1 Int64)))

; --- Boolean connectives SHORT-CIRCUIT at RUN TIME --------------------------------------------------
; The cases above pin that a short-circuited operand is still SCOPE- and TYPE-checked (a dead operand
; carries no deferred error). This pins the RUNTIME half of #Boolean Connectives Short-Circuit: when the
; LEFT operand determines the result — `false` for `and`, `true` for `or` — the RIGHT operand is NOT
; EVALUATED, so its side effects (here, a runtime trap) do not occur; when the left does NOT determine the
; result, the right IS evaluated and its trap fires. The right operand is `(< (/ 10 d) 5)` with `d` a
; boundary parameter, so a `d`=0 divide traps at RUN TIME — a genuinely runtime trap (a CONSTANT `(/ 10 0)`
; would be the compile-time `CDZ0304`, caught before any short-circuit, so the divisor MUST be a parameter
; to reach the runtime connective). Each case pairs the two paths: the short-circuit path (left decides →
; right skipped → `d`=0 does NOT trap, the `if` yields the left-decided branch) and the evaluate path (left
; does not decide → right runs → `d`=0 TRAPS). A compiler that eagerly evaluated both operands would trap
; on the skip path; one that never evaluated the right would answer wrong on the evaluate path.
(case
  "and short-circuits at run time: a false left operand skips the trapping right operand"
  (doc
    "`(and b (< (/ 10 d) 5))` with `b`=false short-circuits — the right operand is NOT evaluated — so
           the runtime divide `(/ 10 d)` with `d`=0 does NOT trap, and the whole `and` is `false`, taking
           the `if`'s else branch → 0 (core-semantics.md #Boolean Connectives Short-Circuit). With `b`=true
           the left does not decide the conjunction, so the right IS evaluated and `(/ 10 0)` TRAPS at run
           time. Pins the runtime short-circuit of `and`: a `false` left skips the right operand's effects,
           a `true` left reaches them. The divisor is a parameter so the divide is a RUNTIME trap, not the
           compile-time CDZ0304 a constant `(/ 10 0)` would raise before the connective runs.")
  (input (do (def (main (: b Bool) (: d Int64)) (if (and b (< (/ 10 d) 5)) 1 0)) (export main)))
  (call main (: false Bool) (: 0 Int64))
  (output (: 0 Int64))
  (call main (: true Bool) (: 0 Int64))
  (trap "division by zero"))

(case
  "and evaluates the right operand when the left is true"
  (doc
    "The non-short-circuit path of `and` with a SAFE divisor: `b`=true so the right operand runs and
           the result DEPENDS on it — `d`=5 makes `(/ 10 5)` = 2 < 5 true, so the conjunction is true → 1;
           `d`=2 makes `(/ 10 2)` = 5, and `5 < 5` is false, so the conjunction is false → 0. Pins that a
           `true` left operand genuinely evaluates the right (the two divisors give different answers), the
           value companion of the trap-fires path above — the right operand is reached, not skipped.")
  (input (do (def (main (: b Bool) (: d Int64)) (if (and b (< (/ 10 d) 5)) 1 0)) (export main)))
  (call main (: true Bool) (: 5 Int64))
  (output (: 1 Int64))
  (call main (: true Bool) (: 2 Int64))
  (output (: 0 Int64)))

(case
  "or short-circuits at run time: a true left operand skips the trapping right operand"
  (doc
    "`(or b (< (/ 10 d) 5))` with `b`=true short-circuits — the right operand is NOT evaluated — so
           the runtime divide with `d`=0 does NOT trap, and the whole `or` is `true`, taking the `if`'s then
           branch → 1. With `b`=false the left does not decide the disjunction, so the right IS evaluated and
           `(/ 10 0)` TRAPS. The `or` mirror of the `and` case: a `true` left skips the right operand's
           effects, a `false` left reaches them.")
  (input (do (def (main (: b Bool) (: d Int64)) (if (or b (< (/ 10 d) 5)) 1 0)) (export main)))
  (call main (: true Bool) (: 0 Int64))
  (output (: 1 Int64))
  (call main (: false Bool) (: 0 Int64))
  (trap "division by zero"))

(case
  "or evaluates the right operand when the left is false"
  (doc
    "The non-short-circuit path of `or` with a SAFE divisor: `b`=false so the right operand runs and
           the result DEPENDS on it — `d`=5 makes `(/ 10 5)` = 2 < 5 true, so the disjunction is true → 1;
           `d`=2 makes `(/ 10 2)` = 5, and `5 < 5` is false, so the disjunction is false → 0. Pins that a
           `false` left operand genuinely evaluates the right (the two divisors give different answers), the
           value companion of the `or` trap-fires path above.")
  (input (do (def (main (: b Bool) (: d Int64)) (if (or b (< (/ 10 d) 5)) 1 0)) (export main)))
  (call main (: false Bool) (: 5 Int64))
  (output (: 1 Int64))
  (call main (: false Bool) (: 2 Int64))
  (output (: 0 Int64)))

(case
  "chained and short-circuits through nesting: a false outer-left skips a NESTED trapping operand"
  (doc
    "Short-circuit is RECURSIVE, not one-deep: an outer `and` whose left is false skips its ENTIRE
           right operand — including a NESTED `and` and the trapping divide buried inside it.
           `(and a (and b (< (/ 10 d) 5)))` with `a`=false, `d`=0: the outer `and` short-circuits on the
           false `a`, so the whole `(and b (/ 10 0))` subtree is never evaluated — the inner `(/ 10 0)` does
           NOT trap — and the result is false → 0. The single-level short-circuit cases above (a false left
           skips ONE trapping operand) cannot catch a shield that only reaches one level: an implementation
           that evaluated the outer's right operand `(and b …)` eagerly enough to reach the inner divide
           would trap here. Pins that the connective's laziness propagates through the whole operand
           subtree, the chained companion of the single-level `and`/`or` short-circuit cases.")
  (input
    (do
      (def (main (: a Bool) (: b Bool) (: d Int64)) (if (and a (and b (< (/ 10 d) 5))) 1 0))
      (export main)))
  (call main (: false Bool) (: true Bool) (: 0 Int64))
  (output (: 0 Int64)))

; The short-circuit shield must survive OPTIMIZATION, not just naive evaluation. When a trapping
; runtime subexpression appears TWICE inside a short-circuited operand, a common-subexpression
; elimination (CSE) pass may compute it once and hoist that single evaluation ABOVE the connective —
; and if the CSE frontier treats an `and`/`or` right operand as unconditionally reached (rather than
; guarded by the left), the hoisted divide runs even on the short-circuit path, resurrecting the very
; trap the connective shields (adv-55: a wasm CSE hoisted `(/ 10 d)` out of the `and` rhs, so
; `main(false, 0)` spuriously trapped divide-by-zero at O0..O3 while the rust backend stayed correct).
; The single-division cases above cannot catch this: the bug needs the divide REPEATED so CSE has two
; occurrences to coalesce. Each case pins that the connective's laziness binds the OPTIMIZER — the
; shielded operand's trap must not fire on the skip path no matter how the duplicate is folded.
(case
  "a repeated trapping divide in a short-circuited and's right operand stays shielded (CSE must not hoist it past the connective)"
  (doc
    "`(and b (= (/ 10 d) (/ 10 d)))` with `b`=false short-circuits, so the right operand — and BOTH
           of its `(/ 10 d)` divides — must not be evaluated: `main(false, 0)` is 0, NOT a divide-by-zero
           trap (core-semantics.md #Boolean Connectives Short-Circuit). With `b`=true the right runs and
           `(= (/ 10 5) (/ 10 5))` is true → 1. The divide is DUPLICATED so a common-subexpression pass has
           two occurrences to coalesce; a CSE that treats the `and` rhs as unconditionally reached would
           hoist the shared divide above the connective and trap on the false-left skip path (adv-55). Pins
           that CSE respects the short-circuit frontier — the companion to the single-division `and` case,
           at the OPTIMIZER level.")
  (input
    (do (def (main (: b Bool) (: d Int64)) (if (and b (= (/ 10 d) (/ 10 d))) 1 0)) (export main)))
  (call main (: false Bool) (: 0 Int64))
  (output (: 0 Int64))
  (call main (: true Bool) (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a repeated trapping divide in a short-circuited or's right operand stays shielded (CSE must not hoist it past the connective)"
  (doc
    "The `or` twin of the CSE-hoist shield above: `(or (= x 0) (= (/ 100 x) (/ 100 x)))` with `x`=0
           short-circuits on the true left, so the right operand's duplicated `(/ 100 x)` divides must not
           run — `main(0)` is 1, not a divide-by-zero trap. With `x`=5 the left is false, the right runs, and
           `(= (/ 100 5) (/ 100 5))` is true → 1. Both connectives lower to one `Core::And` node, so the
           and-case pins the mechanism and this is belt-and-suspenders that the same CSE-frontier fix holds
           for the disjunction spelling too (adv-55 or-twin).")
  (input (do (def (main (: x Int64)) (if (or (= x 0) (= (/ 100 x) (/ 100 x))) 1 0)) (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a repeated trapping divide inside one if-arm is not speculated past the branch"
  (doc
    "The if-branch companion of the connective CSE-shield cases above: `(if c (+ (/ a b) (/ a b)) 5)`
           uses the checked `(/ a b)` TWICE, but only on the THEN path. Straight-line CSE is gated to a
           body with no `if`/`match` (so every shared node is unconditionally reached), so a body WITH an
           `if` is ineligible — `(/ a b)` stays inside the then-arm, never hoisted to the function top.
           `c`=false takes `5` and must NOT evaluate the divide (`b`=0 stays safe → no trap); `c`=true with
           a safe divisor computes `(a/b)+(a/b)` (20/4 + 20/4 = 10); `c`=true with `b`=0 traps on the TAKEN
           divide. Pins that speculatively hoisting a repeated trapping node past an `if` branch does not
           fire the trap on the skip path (the CSE dual of the LICM frontier restriction).")
  (input (do (def (f (: c Bool) (: a Int64) (: b Int64)) (if c (+ (/ a b) (/ a b)) 5)) (export f)))
  (call f (: false Bool) (: 9 Int64) (: 0 Int64))
  (output (: 5 Int64))
  (call f (: true Bool) (: 20 Int64) (: 4 Int64))
  (output (: 10 Int64))
  (call f (: true Bool) (: 9 Int64) (: 0 Int64))
  (trap "division by zero"))

(case
  "a self-shielding or does not hoist its trapping right operand past the short-circuit"
  (doc
    "`(or (= x 0) (< (/ 100 x) 5))` — the LEFT operand `(= x 0)` is true EXACTLY at the value (x=0)
           where the right operand's `(/ 100 x)` would divide by zero, so the short-circuit shields it: x=0
           -> lhs true -> the `||` is true (1) and the divide never runs (NO trap). CSE must not hoist `(/
           100 x)` before the short-circuit branch. x=4 -> lhs false -> `(< 25 5)` false -> 0; x=50 -> `(< 2
           5)` true -> 1. The single-divide self-shielding companion of the repeated-divide or-shield above.")
  (input (do (def (f (: x Int64)) (if (or (= x 0) (< (/ 100 x) 5)) 1 0)) (export f)))
  (call f (: 0 Int64))
  (output (: 1 Int64))
  (call f (: 4 Int64))
  (output (: 0 Int64))
  (call f (: 50 Int64))
  (output (: 1 Int64)))

(case
  "a repeated trapping divide inside one match arm is not hoisted past the match"
  (doc
    "The match companion of the if-arm and connective CSE-shield cases above: `(match k (0 100) (_ (+
           (/ 10 d) (/ 10 d))))` uses the checked `(/ 10 d)` twice, but only in the WILDCARD arm. A match
           runs only the SELECTED arm, so an arm-local repeated node must stay inside its arm, never hoisted
           to the function top. main(0, 0) selects the `k=0` constant arm -> 100, and the wildcard arm's `(/
           10 0)` must NOT run (no divide-by-zero). Pins the Match conditional-position CSE frontier, the
           last unwitnessed position after And/Or/If.")
  (input
    (do
      (def (main (: k Int64) (: d Int64)) (match k (0 100) (_ (+ (/ 10 d) (/ 10 d)))))
      (export main)))
  (call main (: 0 Int64) (: 0 Int64))
  (output (: 100 Int64)))

(case
  "a sequencing block yields the value of its last form"
  (doc
    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order (2nd sentence:
           a block evaluates to its last form's value). The earlier forms are pure here, so the block's
           only observable result is the last form; ordering of effects is witnessed in
           03-equality-and-observation.sexp.")
  (input (do 1 2 3))
  (output (: 3 Int64)))

(case
  "a sequencing block discards a pure compound intermediate"
  (doc
    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order (\"evaluate each
           of its forms\" then \"evaluate to the value of its last form\"): a non-final form is
           evaluated and its value discarded, whatever its type. A pure compound value — a record here —
           in a non-final position has no observable effect, so the block yields its last form (42). The
           earlier `do` cases only drop scalars; this pins that a COMPOUND intermediate is dropped the
           same way rather than blocking the block.")
  (input (do #record((= a 1)) 42))
  (output (: 42 Int64)))

(case
  "a sequencing block discards a pure list intermediate"
  (doc
    "Companion of the case above with a list intermediate: `(do (list 1 2 3) 7)` evaluates the
           list, discards it (no effect), and yields the last form 7.")
  (input (do #list(1 2 3) 7))
  (output (: 7 Int64)))

; --- A declaration in a sequencing block binds for the following forms -------------------
; core-semantics.md #A Declaration In A Sequencing Block Is Scoped To The Forms That Follow It:
; "A declaration form in a sequencing block MUST bind its name for the forms that follow it in
; that block, so that a name a declaration introduces is in scope without a separate binding
; form." This is how a module declaration binds its name (11-modules.sexp relies on it for
; `(do (module m …) <uses-m>)`), and it applies to a `def` declaration too — a `def` in a `do`
; binds its name for the later forms, no enclosing `let` needed. The seed does not yet recognize
; `def` as a declaration in do-block position: it treats the `def` head as a name to resolve and
; declines "unbound name: def" (a misleading code — `def` is a declaration keyword, not a name).
(case
  "a value declaration in a do block is in scope for the following forms"
  (doc
    "Witnesses core-semantics.md #A Declaration In A Sequencing Block Is Scoped To The Forms
           That Follow It: `(def x 5)` as a form of a `do` binds `x` for the following form, so
           `(+ x 1)` sees it without a `let`. The block yields the last form's value, 6. This is the
           same declaration-binds-its-name rule a module declaration uses; a `def` declaration in a
           sequencing block is in scope exactly like one.")
  (input (do (def x 5) (+ x 1)))
  (output (: 6 Int64)))

(case
  "a function declaration in a do block is callable by the following forms"
  (doc
    "The function-declaration companion: `(def (f n) (+ n 1))` in a `do` binds `f` for the
           following forms, so `(f 9)` calls it and the block yields 10. A declaration introduces its
           name into the rest of the block without a separate binding form, whether it declares a
           value or a function.")
  (input (do (def (f n) (+ n 1)) (f 9)))
  (output (: 10 Int64)))

; The two cases above declare ONE name and use it in a later form. The scoping rule is that a
; declaration binds its name for EVERY following form — including a LATER DECLARATION, so a chain of
; `def`s each sees the ones before it (core-semantics.md #A Declaration In A Sequencing Block Is Scoped
; To The Forms That Follow It). These pin the chain: a `def` whose value references an earlier `def`, a
; `def`-fn whose body calls an earlier sibling `def`, and a `def` that shadows an outer `let` binding —
; the declaration-scope behavior a prelude or a group of top-level helpers relies on.
(case
  "a later declaration in a do block sees an earlier one"
  (doc
    "`(do (def x 5) (def y (+ x 1)) y)`: the second declaration's value `(+ x 1)` references `x`
           from the first declaration, so `y` = 6 and the block yields 6. Pins that a declaration is in
           scope for a LATER DECLARATION, not only for a plain expression form — the chaining that makes
           a sequence of `def`s (a prelude) resolve.")
  (input (do (def x 5) (def y (+ x 1)) y))
  (output (: 6 Int64)))

(case
  "a function declaration in a do block calls an earlier sibling declaration"
  (doc
    "`(do (def base 10) (def (add-base n) (+ n base)) (add-base 5))`: the function `add-base`
           closes over the earlier declaration `base`, so `(add-base 5)` = 15. Pins that a `def`-fn's
           body sees the declarations that precede it in the block, exactly as a module function sees
           its siblings.")
  (input (do (def base 10) (def (add-base n) (+ n base)) (add-base 5)))
  (output (: 15 Int64)))

; A do-local FUNCTION declaration is in scope in its OWN body (self-recursion) and in a sibling
; function's body regardless of order (mutual recursion) — a function group in a `do` is mutually
; visible, exactly like a module's members or the top-level defs, not strictly sequential like a VALUE
; binding (whose scope stays backward-only: `(do (def x 5) (def x (+ x 10)) x)` = 15, the second `x`
; seeing only the first). A recursive do-local function is registered as a standalone emittable function,
; so its recursive call lowers to a runtime call — the same lowering a top-level or module-member
; recursive function gets. A compiler that scopes a do-local declaration strictly sequentially reports
; the self-name (or a forward sibling) unbound; one that models the function group runs the recursion.
(case
  "a do-local function declaration is recursive"
  (doc
    "A do-local `(def (fac n) …)` calls ITSELF: the function is in scope in its own body (like a
           top-level or module-member recursive def), and the self-call lowers to a runtime call. fac(5)
           = 120. Pins that a do-local function group is self-visible, not strictly sequential — a value
           declaration's backward-only scope does not constrain a function's recursion.")
  (input (do (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))) (fac 5)))
  (output (: 120 Int64)))

(case
  "two do-local function declarations are mutually recursive"
  (doc
    "`ev` calls `od`, `od` calls `ev` — a do-local function is visible in a sibling function's body
           regardless of declaration order (mutual visibility, like a module's members). Neither reaches
           a normal form by inlining, so both lower to standalone runtime functions calling each other.
           ev(10) is true → 1. Pins that a do-local function group is mutually visible, so `ev`'s body
           sees `od` declared AFTER it (a forward reference a strictly-sequential scope would reject).")
  (input
    (do
      (def (ev n) (if (= n 0) true (od (- n 1))))
      (def (od n) (if (= n 0) false (ev (- n 1))))
      (if (ev 10) 1 0)))
  (output (: 1 Int64)))

(case
  "several independent do-local functions each resolve their own call"
  (doc
    "A do block declaring MANY same-shaped do-local functions — `(def (g_i x) (+ x i))` — each called
           once: every call must resolve to its OWN declaration, never a same-shaped sibling. g0..g4 at 0
           add their own index, so the sum is 0+1+2+3+4 = 10. Pins that a do-block's function declarations
           are each independently resolvable (no cross-attribution across the group), the do-local analogue
           of the module/top-level same-shaped-def resolution.")
  (input
    (do
      (def (g0 (: x Int64)) (+ x 0))
      (def (g1 (: x Int64)) (+ x 1))
      (def (g2 (: x Int64)) (+ x 2))
      (def (g3 (: x Int64)) (+ x 3))
      (def (g4 (: x Int64)) (+ x 4))
      (+ (g0 0) (+ (g1 0) (+ (g2 0) (+ (g3 0) (g4 0)))))))
  (output (: 10 Int64)))

; A do-local function that CAPTURES a sibling local from its enclosing scope lowers by lambda-lift: the
; captured free variable is threaded into the lifted function. The capture must cover a sibling local
; whose value is RUNTIME-COMPUTED (not a parameter, not a constant) — when the enclosing function inlines,
; β-reduction copies the do/let and the sibling binding's value-RHS becomes a SYNTH node; a free-var scan
; that only pins USER-node captures drops the synth value binder, so the copied capture re-resolves in the
; orphaned body and reports the sibling unbound (a false-negative CDZ0101 REJECT of a valid program — the
; FINDING #19 class, rcdzc lambda-lift, fixed by pinning a synth captured value binder). Parameter capture
; and constant-local capture always worked (the binder is a user node / folds to a constant); only a
; runtime-computed synth value binder slipped through.
(case
  "a do-local fn capturing a runtime-computed sibling local survives the enclosing fn's inlining"
  (doc
    "FINDING #19 (breaker), rcdzc lambda-lift synth-captured-value-binder fix. `outer` has a do-local
           `(def m (* n 3))` (runtime-computed from param `n`) and a do-local `(def (inner x) (+ x m))` that
           CAPTURES `m`; `outer` inlines at `main`, β-copying the do so `m`'s RHS is a synth node. The lift
           must still pin `m` into `inner`'s captured env — else the copied `inner` sees `m` unbound and
           rejects CDZ0101. `(outer 5)` = inner(5) = 5 + (5*3) = 20. The runtime-computed-capture face that
           param-capture and constant-capture (below) do not exercise.")
  (input
    (do
      (def (outer (: n Int64)) (do (def m (* n 3)) (def (inner (: x Int64)) (+ x m)) (inner n)))
      (def (main (: n Int64)) (outer n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64)))

(case
  "a do-local fn capturing a runtime-computed sibling via a LET binding survives inlining"
  (doc
    "The let-binder twin of the do-def capture above: the runtime-computed sibling is a `(let ((m (* n
           3))) …)` binding rather than a `(def m …)`; both binder forms produce a synth value binder when
           `outer` inlines, and both must be pinned into `inner`'s capture. `(outer 5)` = 20. Pins that the
           synth-captured-value-binder fix covers the let-binding form, not only do-def.")
  (input
    (do
      (def (outer (: n Int64)) (let ((m (* n 3))) (do (def (inner (: x Int64)) (+ x m)) (inner n))))
      (def (main (: n Int64)) (outer n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64)))

(case
  "a do-local fn capturing the enclosing fn PARAMETER directly lowers (capture guard)"
  (doc
    "FINDING #19 guard c2: `inner` captures the PARAMETER `n` directly (not a computed sibling). The
           binder is a user node so the free-var scan always pinned it — this always worked and stays a
           passing guard alongside the runtime-computed-capture fix. `(outer 5)` = inner(1) = 1 + 5 = 6.")
  (input
    (do
      (def (outer (: n Int64)) (do (def (inner (: x Int64)) (+ x n)) (inner 1)))
      (def (main (: n Int64)) (outer n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a do-local fn capturing a CONSTANT sibling local lowers (capture guard)"
  (doc
    "FINDING #19 guard c3: `inner` captures a CONSTANT sibling `(def m 3)` — the binder folds to a
           constant, so the capture always resolved. Stays a passing guard beside the runtime-computed
           case; distinguishes 'constant sibling' (always worked) from 'runtime-computed sibling' (the fix).
           `(outer 5)` = inner(5) = 5 + 3 = 8.")
  (input
    (do
      (def (outer (: n Int64)) (do (def m 3) (def (inner (: x Int64)) (+ x m)) (inner n)))
      (def (main (: n Int64)) (outer n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 8 Int64)))

(case
  "a top-level do-local fn capturing a constant lowers (capture control)"
  (doc
    "FINDING #19 control c1: a TOP-LEVEL do-local fn `(def (addb n) (+ n base))` capturing a top-level
           constant `(def base 10)` — no enclosing-fn inlining involved, so no synth-binder copy; the
           baseline capture path. `(addb 5)` = 5 + 10 = 15. Confirms the top-level capture never regressed.")
  (input
    (do
      (def base 10)
      (def (addb (: n Int64)) (+ n base))
      (def (main (: n Int64)) (addb n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

; A recursive do-local function nested INSIDE a HELPER that is itself INLINED at its call site still
; recurses: β-reduction COPIES the helper's body (fresh occurrences), so the copied recursive self-call
; must still lower to a runtime call — the copy's do-local function is registered as an emittable function
; exactly as the original is. A compiler that registers only the LOAD-TIME occurrence declines the copied
; call "needs runtime specialization"; one that registers the reduced copy's do-local functions runs it.
(case
  "a recursive do-local function nested in an inlined helper recurses"
  (doc
    "`helper` carries a do-local recursive `fac`; `(helper 5)` inlines `helper`, COPYING its body —
           so the copied `(fac (- n 1))` self-call must still resolve to an emittable function and lower to
           a runtime call. fac(5) = 120, and `helper` folds away. Pins that recursion survives β-copy of an
           enclosing function: the reduced copy's do-local function is registered like the original, not
           left as an un-lowerable copy.")
  (input
    (do
      (def (helper x) (do (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))) (fac x)))
      (def (main) (helper 5))
      (export main)))
  (output (: 120 Int64)))

(case
  "a recursive do-local function survives two inlinings of its helper"
  (doc
    "The helper is called TWICE — `(helper 5)` and `(helper 3)` — so its body (with the do-local
           recursive `fac`) is copied twice, each copy's `fac` its own emittable function. fac(5)+fac(3) =
           120 + 6 = 126. Pins that EACH β-copy of the enclosing helper registers its own copy of the
           recursive function (one call site's copy is not confused for another's).")
  (input
    (do
      (def (helper x) (do (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))) (fac x)))
      (def (main) (+ (helper 5) (helper 3)))
      (export main)))
  (output (: 126 Int64)))

(case
  "mutually-recursive do-local functions nested in an inlined helper recurse"
  (doc
    "The mutual-recursion face of the inlined-helper cases above: `helper` carries a do-local
           `ev`/`od` pair that call each other; `(helper 10)` inlines `helper`, β-copying both — so each
           copied function must lower to a runtime call and reach its sibling copy. ev(10) is true → 1.
           Pins that a whole EACH-OTHER call group (not only a single self-recursive function) survives the
           β-copy of its enclosing helper.")
  (input
    (do
      (def
        (helper x)
        (do
          (def (ev n) (if (= n 0) true (od (- n 1))))
          (def (od n) (if (= n 0) false (ev (- n 1))))
          (if (ev x) 1 0)))
      (def (main) (helper 10))
      (export main)))
  (output (: 1 Int64)))

(case
  "recursion is detected through a nested do around the self-call"
  (doc
    "A self-call inside a nested `(do …)` is a real recursion edge. A `do` collapses to its last
           form (intermediates discarded as pure), which would hide a self-call in a do item from the
           recursion walk — so the callee walk must descend every do item by raw AST. `(do 7 (+ n (sum-to
           (- n 1))))` puts the self-call as the do's last item after a discarded intermediate; a walk
           that read the collapsed `do` as non-recursive would inline `sum-to` without end (a hang) or
           miscompile it. sum-to(3) = 3+2+1+0 = 6.")
  (input
    (do
      (def (sum-to n) (if (= n 0) 0 (do 7 (+ n (sum-to (- n 1))))))
      (def (main) (sum-to 3))
      (export main)))
  (output (: 6 Int64)))

; The recursive cases above run at a small CONSTANT depth (fac(5)), which the compiler may fold. A
; self-hosted compiler instead recurses over the SIZE of the program it compiles — a depth decided at run
; time and often large. These drive a recursion to a LARGE N supplied as a boundary argument (so it cannot
; fold), pinning that the compiled recursion runs at scale in CONSTANT STACK: the wasm-backend loop
; transform turns a tail-recursive (and an accumulable non-tail) self-call into a loop, so 100000–1000000
; iterations complete without exhausting the wasm stack. A generation that lowered the self-call as a plain
; recursive wasm CALL would overflow the stack at these depths; the recorded value is the exact accumulation.
(case
  "a tail-recursive accumulator loop runs to a large runtime N in constant stack"
  (doc
    "`(go i n acc) = (if (< i n) (go (+ i 1) n (+ acc i)) acc)` summed over 0..n-1, driven to n =
           100000 by a boundary argument — so it cannot fold and runs as an emitted loop. The sum
           0+1+…+99999 = 4999950000 (which exceeds Int32, so it also pins the Int64 accumulator). Completing
           without a stack overflow pins that the tail-recursive self-call became a CONSTANT-STACK loop (a
           plain recursive call would blow the wasm stack at 100000 deep). n=0 and n=1 pin the empty and
           single-step boundaries (both 0, since the last index summed is n-1).")
  (input
    (do
      (def (go i n acc) (if (< i n) (go (+ i 1) n (+ acc i)) acc))
      (def (main (: n Int64)) (go 0 n 0))
      (export main)))
  (call main (: 100000 Int64))
  (output (: 4999950000 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 1 Int64))
  (output (: 0 Int64)))

(case
  "a tail-recursive countdown loop runs to a very large runtime N"
  (doc
    "`(go n acc) = (if (= n 0) acc (go (- n 1) (+ acc 1)))` counts down from n = 1000000, incrementing
           the accumulator each step → 1000000. A million-deep tail recursion completing pins the constant-
           stack loop at an order of magnitude beyond the sum case — the self-hosting scale where a
           per-call stack frame would certainly overflow.")
  (input
    (do
      (def (go n acc) (if (= n 0) acc (go (- n 1) (+ acc 1))))
      (def (main (: n Int64)) (go n 0))
      (export main)))
  (call main (: 1000000 Int64))
  (output (: 1000000 Int64)))

(case
  "accumulator introduction preserves the zero-iteration exit and the body's trap"
  (doc
    "The TRAP-SAFETY face of the loop transform, bracketing the exactness (non-associative alt)
           and scale (100000 constant-stack) pins: `(sum-div n d)` recurses non-tail with a body
           containing `(/ 100 d)`. Three verdicts: n = 0, d = 0 → 0 — the transform must NOT hoist the
           trapping divide ahead of the zero-iteration exit (an accumulator loop that evaluated the body
           once before checking the bound would trap here); n = 3, d = 5 → 60 — the value is exact; and
           n = 3, d = 0 → the trap FIRES when an iteration genuinely reaches the divide (the transform
           must not elide it either). Together: the rewrite moves no trap in either direction.")
  (input
    (do
      (def (sum-div (: n Int64) (: d Int64)) (if (= n 0) 0 (+ (/ 100 d) (sum-div (- n 1) d))))
      (def (main (: n Int64) (: d Int64)) (sum-div n d))
      (export main)))
  (call main (: 0 Int64) (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: 60 Int64))
  (call main (: 3 Int64) (: 0 Int64))
  (trap "divide by zero"))

(case
  "a non-tail accumulable recursion runs to a large runtime N in constant stack"
  (doc
    "`(go n) = (if (= n 0) 0 (+ 1 (go (- n 1))))` — the self-call is NOT in tail position (its result
           is fed to `(+ 1 …)`), but the accumulation is associative, so the backend's accumulator
           introduction turns it into a constant-stack loop too. Driven to n = 100000 it returns 100000
           without a stack overflow. Pins that the loop transform covers the accumulable non-tail shape (not
           only strict tail calls) at scale — the shape a naive `1 + recurse` count/length takes.")
  (input
    (do
      (def (go n) (if (= n 0) 0 (+ 1 (go (- n 1)))))
      (def (main (: n Int64)) (go n))
      (export main)))
  (call main (: 100000 Int64))
  (output (: 100000 Int64)))

(case
  "a tail-recursive list fold over a large runtime list runs in constant stack"
  (doc
    "The LIST-CONSUMING face of the loop transform: `(sa xs acc) = (match xs ((list) acc) ((list x ..
           rest) (sa rest (+ acc x))))` is a self-tail-call inside a `MatchList` CONS arm (the loop transform
           threads tail position through list-match arms, not only scalar `if`s). The list is built at run
           time by a push-loop so the fold walks a genuine heap list. Summing [0,100000) = 4999950000 (also
           pins the Int64 accumulator, exceeding Int32) COMPLETES without a stack overflow — a recursive
           `call` at 100000 deep would blow the wasm stack — proving the MatchList tail-call became a
           constant-stack loop. [0,100) = 4950 is the small control. Relocated (RUN half) from rcdzc
           `a_tail_recursive_list_fold_compiles_to_a_constant_stack_loop`; its Lir `loop`/no-self-call witness
           stays in rcdzc.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: out (List Int64)))
        (if (< i n) (build (+ i 1) n (List.push out i)) out))
      (def
        (sa (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(x (.. rest)) (sa rest (+ acc x)))))
      (def (main (: n Int64)) (sa (build 0 n #list()) 0))
      (export main)))
  (call main (: 100 Int64))
  (output (: 4950 Int64))
  (call main (: 100000 Int64))
  (output (: 4999950000 Int64))
  (live-objects 0))

(case
  "a non-tail list fold over a large runtime list is accumulator-transformed to constant stack"
  (doc
    "The user's natural non-tail list sum: `(sum xs) = (match xs ((list) 0) ((list x .. rest) (+ x
           (sum rest))))` — the self-call `(sum rest)` sits in a `+` OPERAND, so it is NOT a tail call and
           would grow the stack. The backend's accumulator introduction recognizes the list-fold shape (empty
           arm = the `+` identity, cons arm = combine, self-call threading `rest` through the scrutinee) and
           rewrites it to a tail accumulator, which the MatchList loop transform then compiles to a loop. Over
           a runtime list of [0,100000) it returns 4999950000 without a stack overflow — proving the non-tail
           list fold became O(1) stack. [0,100) = 4950 is the control. Relocated (RUN half) from rcdzc
           `a_non_tail_list_fold_is_accumulator_transformed_into_a_constant_stack_loop`; its `sum$acc`
           synthesis + Lir `loop` witness stays in rcdzc.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: out (List Int64)))
        (if (< i n) (build (+ i 1) n (List.push out i)) out))
      (def (sum (: xs (List Int64))) (match xs (#list() 0) (#list(x (.. rest)) (+ x (sum rest)))))
      (def (main (: n Int64)) (sum (build 0 n #list())))
      (export main)))
  (call main (: 100 Int64))
  (output (: 4950 Int64))
  (call main (: 100000 Int64))
  (output (: 4999950000 Int64))
  (live-objects 0))

(case
  "a nullary do-local def followed by a use of it computes over its result"
  (doc
    "The `def helper … then use it` idiom: `main`'s body is a do-block with a nullary do-local
           `(def (a) 10)` followed by `(+ (a) 5)` = 15. Pins the intended semantics of a def-body sequence
           whose declaration ends in a NUMBER and whose next statement STARTS with a name — the exact shape
           the ML surface's unit-quantity sugar (`5 feet` → Qty) corrupted by greedily reading the def RHS
           `10` and the next statement's leading `a` as one quantity `(Qty.of 10 (Unit.of #\"a\"))`, dropping
           main's real tail. The ML reader now gates that sugar to a single line (no crossing a newline /
           statement boundary), so this program's ML spelling parses like this s-expr and runs to 15. The
           s-expr surface was always correct (it has no juxtaposition sugar); this is the semantics witness.")
  (input (do (def (main) (do (def (a) 10) (+ (a) 5))) (export main)))
  (call main)
  (output (: 15 Int64)))

; An ARGUMENT to a user-function call is an expression evaluated in the CALL SITE's scope, and its
; names bind there — a compiler that reduces a call by substituting the argument into the callee's
; body must not thereby resolve the argument's names in the callee's scope. The witnesses below pin
; a let-bound name, a let-bound lambda's argument, and a call's own result each passed as an argument
; to another user call: every one keeps the binding in effect where it was written (core-semantics.md
; #Binding Is Lexical). The passing anchors (a literal argument, a direct reference with no call) sit
; among the other let/def cases in this file; these add the call-argument position specifically.
(case
  "a let-bound variable passed as a function-call argument resolves at the call site"
  (doc
    "`(let ((k 10)) (inc k))` binds `k` = 10, then applies the top-level `inc` to it, yielding
           11. The argument `k` is a reference to the caller's `let` binding; reducing `(inc k)` by
           substituting `k` into `inc`'s body must keep `k` bound at the call site, not resolve it in
           `inc`'s scope (where it is unbound). A literal argument `(inc 10)` and a direct reference
           `(let ((k 10)) (+ k 1))` both already resolve; this pins the call-argument position.")
  (input (do (def (inc x) (+ x 1)) (def (main) (let ((k 10)) (inc k))) (export main)))
  (output (: 11 Int64)))

(case
  "a let-bound variable passed to a let-bound lambda resolves at the call site"
  (doc
    "The lambda sibling: `(let ((k 10) (f (fn (x) (+ x 1)))) (f k))` applies the let-bound `f`
           to the let-bound `k`, yielding 11. Both names are bound by the same `let`; the argument `k`
           passed to `f` resolves against that `let`, not inside `f`'s body.")
  (input (do (def (main) (let ((k 10) (f (fn (x) (+ x 1)))) (f k))) (export main)))
  (output (: 11 Int64)))

(case
  "a nested application of a let-bound lambda resolves each argument at its call site"
  (doc
    "`(let ((f (fn (x) (+ x 1)))) (f (f 0)))` = 2: the inner `(f 0)` yields 1 and is the
           argument to the outer `f`. The inner call's result, substituted into the outer application,
           keeps `f` bound by the enclosing `let` — nesting one call as another's argument does not
           lose the binding.")
  (input (do (def (main) (let ((f (fn (x) (+ x 1)))) (f (f 0)))) (export main)))
  (output (: 2 Int64)))

(case
  "a let-bound variable derived from a runtime parameter passed as a call argument"
  (doc
    "The runtime companion: `(let ((k (+ n 1))) (inc k))` binds `k` from the runtime parameter
           `n` and passes it to `inc`; with n = 40, k = 41 and the result is 42. The binding is
           resolved at the call site whether the let value is a constant or a runtime expression — it
           is the call-argument resolution that matters, not the value's staticness.")
  (input
    (do (def (inc x) (+ x 1)) (def (main (: n Int64)) (let ((k (+ n 1))) (inc k))) (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64)))

; The CONVERSE of the call-argument cases above: those pin that a caller's binding flows INTO a callee
; (an argument keeps its call-site binding under substitution). This pins the other direction — a
; callee's body resolves its OWN free names in the CALLEE's lexical scope, and is NOT captured by a
; same-name binding live at the CALL SITE. `helper`'s body references `base`, which lexically names the
; top-level `(def (base) 5)`; a `let` in `main` that rebinds `base` to a different function is invisible
; to `helper`. So `(helper)` = 5 + 10 = 15 regardless of main's local `base`, and main = 15 + 100 = 115.
; A compiler that resolves a called def's body under the CALLER's environment (dynamic scope) would let
; main's `base` = 100 capture `helper`'s reference, computing 110 + 100 = 210 — a silent miscompile. This
; is the value-discriminating witness of core-semantics.md #Binding Is Lexical for the callee-body
; direction (helper bodies are first-order: they see no caller bindings), the twin of the unbound-name
; cases (which pin that a callee body name with NO lexical binding is CDZ0101, never dynamically found).
(case
  "a called function's body resolves its free names lexically, not in the caller's scope"
  (doc
    "`helper`'s body references `base`, which lexically resolves to the top-level `(def (base) 5)`.
           `main` introduces a `let` binding `base` = `(fn () 100)` and then calls `(helper)`. Under
           lexical scope `helper` cannot see main's local `base`: `(helper)` = 5 + 10 = 15, and main =
           15 + main's `(base)` (100) = 115. A compiler that resolves the called body under the CALLER's
           environment (dynamic scope) would have main's `base` = 100 capture `helper`'s reference,
           computing 110 + 100 = 210 — a silent miscompile. Pins that a callee body is first-order (sees
           no caller bindings), the value-discriminating converse of the call-argument cases above (which
           pin a caller binding flowing INTO the callee) and of the unbound-name cases (a callee free
           name with no lexical binding is CDZ0101, never dynamically resolved). core-semantics.md
           #Binding Is Lexical.")
  (input
    (do
      (def (base) 5)
      (def (helper) (+ (base) 10))
      (def (main) (let ((base (fn () 100))) (+ (helper) (base))))
      (export main)))
  (call main)
  (output (: 115 Int64)))

; The PARAMETERIZED companion of the case above: that one used a NULLARY helper, so its body is emitted
; without an argument substitution. This exercises the β-reduction / argument-splice path — a helper
; that BOTH takes an argument (`x`, substituted at the call) AND references a free top-level name
; (`scale`) that COLLIDES with a caller `let` binding. Substituting the argument must not drag the
; callee body's OWN free names into the caller's scope: `helper`'s `scale` still resolves to the
; top-level `(def (scale) 3)`, so `(helper 4)` = 4 * 3 = 12, and main = 12 + main's `(scale)` (1000) =
; 1012. A dynamic-scope compiler that resolves the substituted body under the caller env would have
; main's `scale` = 1000 capture `helper`'s reference: 4 * 1000 + 1000 = 5000 — a silent miscompile on
; the arg-substitution path specifically (distinct emit from the nullary case).
(case
  "a parameterized called function's free names resolve lexically while its argument substitutes"
  (doc
    "`helper` takes an argument `x` and references a free top-level name `scale` = `(def (scale)
           3)`. `main` binds a local `scale` = `(fn () 1000)` and calls `(helper 4)`. Under lexical
           scope the argument `4` substitutes for `x` while `scale` still resolves to the top-level def:
           `(helper 4)` = 4 * 3 = 12, and main = 12 + main's `(scale)` (1000) = 1012. A compiler that
           resolves the substituted body under the CALLER's environment (dynamic scope) would have
           main's `scale` = 1000 capture `helper`'s reference: 4 * 1000 + 1000 = 5000 — a silent
           miscompile. The parameterized companion of the nullary callee-body case above: it pins the
           same first-order-body invariant on the β-reduction / argument-splice emit path, where a
           dynamic-scope leak could ride in with the substitution. core-semantics.md #Binding Is Lexical.")
  (input
    (do
      (def (scale) 3)
      (def (helper (: x Int64)) (* x (scale)))
      (def (main) (let ((scale (fn () 1000))) (+ (helper 4) (scale))))
      (export main)))
  (call main)
  (output (: 1012 Int64)))

(case
  "a declaration in a do block shadows an outer binding"
  (doc
    "`(let ((x 1)) (do (def x 99) x))`: the `def x 99` inside the `do` shadows the outer `let`
           binding of `x` for the forms that follow it, so the block yields 99. Pins that a do-block
           declaration follows the same lexical shadowing rules as any other binding (core-semantics.md
           #Shadowing Is Well-Defined), taking effect for references in its scope.")
  (input (let ((x 1)) (do (def x 99) x)))
  (output (: 99 Int64)))

(case
  "a repeated do-local declaration shadows the earlier one for what follows"
  (doc
    "The do-block twin of the repeated-let-binding shadow: a second `(def x …)` shadows the earlier
           one for the forms that follow (last-wins), and its RHS sees the OLD binding (the scope stays
           backward-only). `(do (def x 5) (def x (+ x 10)) x)` — the second `x`'s value `(+ x 10)` reads
           the first `x` = 5 → 15, and the trailing `x` is that second binding = 15. A generation that
           unbound the earlier `x` at the shadow (rather than shadowing for what follows) would fault the
           second declaration's RHS.")
  (input (do (def x 5) (def x (+ x 10)) x))
  (output (: 15 Int64)))

(case
  "a single-form body admits a sequence by holding a do block"
  (doc
    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order in a
           single-form body position: a `let` body is one form, so a sequence of forms is written as a
           `(do …)` there. The prefix form is pure, so the block yields the value of its last form (the
           binding x), showing the do is the sequencing point and let scope is unchanged.")
  (input (let ((x 4)) (do (+ x 1) x)))
  (output (: 4 Int64)))

(case
  "a sequencing block whose last form is unit yields unit"
  (doc
    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order together with
           #An Effect-Only Expression Yields The Unit Value: a `do` yields its last form's value, and
           when that is `unit` the block — and the program — yields the unit value. The earlier form is
           pure and dropped. This is the shape of every effect-only body: a sequence of effects ending
           in unit; it must run and yield unit as the normal-termination value.")
  (input (do 1 unit))
  (output (: unit Unit)))

; A `do` block must END IN A VALUE FORM: an empty `(do)` has no value, and a `do` whose last form is a
; DECLARATION (a `def`) is valueless — both are CDZ0201 well-formedness faults, caught even in a
; parameterized (non-exported-nullary) body. (Migrated from rcdzc
; a_malformed_do_block_surfaces_in_the_diagnostics_query_on_any_body — the observable-reject faces.)
(case
  "an empty do block in a parameterized def body is rejected as valueless"
  (input (do (def (g (: n Int64)) (do)) (export g)))
  (error CDZ0201 (message "empty `do` block has no value")))

(case
  "a do block whose last form is a declaration rather than a value form is rejected"
  (input (do (def (g) (do (def x 5))) (export g)))
  (error CDZ0201 (message "must end in a value form, not a declaration")))

(case
  "a let body of unit yields unit"
  (doc
    "Witnesses core-semantics.md #An Effect-Only Expression Yields The Unit Value: binding a
           value and then yielding `unit` produces the unit value as the program result. Unit is an
           ordinary value that a binding form can carry to the run boundary.")
  (input (let ((x 1)) unit))
  (output (: unit Unit)))

(case
  "a conditional whose branches are unit yields unit"
  (doc
    "Witnesses core-semantics.md #Conditionals Evaluate One Branch with a unit result: both
           branches yield the unit value, so the conditional yields unit whichever is taken. Pins that
           the unit value flows through `if` and crosses the run boundary as the program's result.")
  (input (if true unit unit))
  (output (: unit Unit)))

(case
  "a conditional evaluates only the selected branch"
  (doc
    "Witnesses core-semantics.md #Conditionals Evaluate One Branch. The unselected branch would
           trap on overflow if it were evaluated; the normal result proves it was not.")
  (input (if true 1 (+ Int64.max 1)))
  (output (: 1 Int64)))

(case
  "a conditional selects the false branch when the condition is false"
  (doc "Witnesses core-semantics.md #Conditionals Evaluate One Branch.")
  (input (if false 1 2))
  (output (: 2 Int64)))

; The single-level case above shields a top-level unselected branch. The guarantee holds at DEPTH too:
; a trapping expression inside a NESTED unselected branch must not be evaluated either — and, dually, a
; conditional's CONDITION may itself be a conditional (an ordinary Bool-valued expression). These pin
; #Conditionals Evaluate One Branch where the single-level case cannot: the shielding is recursive, and
; the condition position accepts a computed Bool, not only a literal or a direct comparison.
(case
  "a conditional shields a trap in a nested unselected branch"
  (doc
    "`(if true (if true 5 (/ 1 0)) 9)`: the outer `if` selects its then-branch, which is another
           `if` selecting 5; the innermost else `(/ 1 0)` (a division-by-zero trap) is in a branch that
           is never selected at either level, so it is NOT evaluated and the result is 5. Pins that
           #Conditionals Evaluate One Branch shields a trap NESTED two levels deep, not only a
           top-level unselected branch (the `(+ Int64.max 1)` case above).")
  (input (if true (if true 5 (/ 1 0)) 9))
  (output (: 5 Int64)))

; The shield cases above use CONSTANT-selected branches with PROVABLE traps (folded/shielded at compile
; time). The RUNTIME face: a branch selected by a runtime `Bool`, whose body divides by a RUNTIME divisor
; (not statically provable, so a runtime trap not CDZ0304). The trap fires ONLY when that branch is
; actually taken AND the divisor is zero — the branch-body companion of the runtime-div-in-the-CONDITION
; case (`(if (> (/ 10 z) 0) …)`) elsewhere in this file. Pins that #Conditionals Evaluate One Branch holds
; at run time: the untaken branch's potential trap does not fire.
(case
  "a runtime-selected branch with a runtime-divisor div-by-zero traps only when taken"
  (doc
    "`(if b (/ 10 z) 42)` with runtime `b`/`z`: the trapping expression `(/ 10 z)` has a RUNTIME divisor
           (not a provable ÷0, so not CDZ0304 — a runtime trap). Called with `b = true, z = 0` the trapping
           branch is selected and z = 0, so it traps 'divide by zero'. Pins that a runtime-selected branch
           evaluates its body — the branch-body companion of the runtime-div-in-the-condition trap case.")
  (input (do (def (main (: b Bool) (: z Int64)) (if b (/ 10 z) 42)) (export main)))
  (call main (: true Bool) (: 0 Int64))
  (trap "divide by zero"))

(case
  "the untaken branch's runtime div-by-zero does not fire"
  (doc
    "The one-branch guarantee at run time: the SAME `(if b (/ 10 z) 42)` with `b = false, z = 0` takes
           the ELSE branch, so `(/ 10 z)` — which would trap at z = 0 — is NOT evaluated, and the result is
           42. Pins that #Conditionals Evaluate One Branch shields a RUNTIME trap in the unselected branch,
           the runtime companion of the constant-shield cases above (which fold the trap away at compile
           time; here the shielding is a run-time branch choice).")
  (input (do (def (main (: b Bool) (: z Int64)) (if b (/ 10 z) 42)) (export main)))
  (call main (: false Bool) (: 0 Int64))
  (output (: 42 Int64)))

(case
  "a runtime-selected trapping branch with a non-zero divisor computes normally"
  (doc
    "The no-trap control: `(if b (/ 10 z) 42)` with `b = true, z = 2` selects the division branch and
           z is non-zero, so `10 / 2` = 5. Rules out a spurious trap on the taken branch when the divisor is
           valid — the trap in the taken-branch case is the z = 0 divisor, not the branch selection itself.")
  (input (do (def (main (: b Bool) (: z Int64)) (if b (/ 10 z) 42)) (export main)))
  (call main (: true Bool) (: 2 Int64))
  (output (: 5 Int64)))

; The shield cases above use INDEPENDENT condition/divisor (`(if b (/ 10 z) 42)`) or COMPOUND arms
; (record/Option payloads). The select-ification pass (if→branchless select over two same-typed bare
; scalar arms — Int64/Float64/narrow-UInt8, pinned elsewhere) is only sound when BOTH arms are trap-free;
; a select evaluates both. These pin the NEGATIVE case that pass must respect: a BARE-SCALAR `if` whose
; condition GUARDS a trapping arm (`(if (= d 0) 0 (/ 100 d))` — the guard is correlated to the divisor)
; must NOT be select-ified, or the guarded divide would evaluate at d=0 and trap, defeating the guard.
; This is the exact shape select-ification most aggressively targets (two Int64 arms, cheap condition), so
; the `is_trap_free` guard on the select is what keeps it a branch — the trap-safety complement of the
; landed narrow/Int64/Float64 select pins (which all use trap-free value arms).
(case
  "a bare-scalar if guarding a div-by-zero is not select-ified — the divisor guard shields the trap"
  (doc
    "`(if (= d 0) 0 (/ 100 d))` — the condition `(= d 0)` guards the else arm's `(/ 100 d)`, which
           would trap at d = 0. Both arms are bare Int64, the shape select-ification targets, but the else
           arm is NOT trap-free, so the compiler must keep a BRANCH (not a branchless select that evaluates
           both). d = 0 → 0 (the guard shields the ÷0, no trap); d = 5 → 100/5 = 20 (the divide arm runs).
           A select-ification that fired here would trap at d = 0, defeating the guard — the trap-safety
           complement of the landed bare-scalar select cases.")
  (input (do (def (main (: d Int64)) (if (= d 0) 0 (/ 100 d))) (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 20 Int64)))

(case
  "a bare-scalar if guarding a MIN/-1 overflow is not select-ified"
  (doc
    "The overflow-guard companion: `(if (= d -1) 0 (/ Int64.min d))` guards the OVERFLOW trap
           `Int64.min / -1` (the quotient +2^63 is out of range). d = -1 → 0 (guard shields the overflow);
           d = 2 → Int64.min/2 = -4611686018427387904 (the divide runs). Pins the select-ification trap
           guard covers an overflow-trapping arm too, not only ÷0 — both are traps a branchless select
           would fire unconditionally.")
  (input (do (def (main (: d Int64)) (if (= d -1) 0 (/ -9223372036854775808 d))) (export main)))
  (call main (: -1 Int64))
  (output (: 0 Int64))
  (call main (: 2 Int64))
  (output (: -4611686018427387904 Int64)))

(case
  "a narrow-UInt8-guarded div is not select-ified — the guard shields the trap"
  (doc
    "The narrow-condition companion: a UInt8 `d` guards a divide `(/ 100 (Int64.of d))`. Since the
           narrow-value select-ification pins a UInt8 two-arm if lowering to a branchless select, this pins
           that a narrow-guarded TRAPPING arm still branches. d = 0 → 0 (guard shields ÷0); d = 4 → 100/4 =
           25. Pins the trap-safety holds when the guard condition is over a narrow width.")
  (input (do (def (main (: d UInt8)) (if (= d 0) 0 (/ 100 (Int64.of d)))) (export main)))
  (call main (: 0 UInt8))
  (output (: 0 Int64))
  (call main (: 4 UInt8))
  (output (: 25 Int64)))

(case
  "a repeated trapping division in a guarded arm stays shielded — a CSE class must not hoist past the branch"
  (doc
    "`(if (= d 0) 0 (+ (/ 100 d) (/ 100 d)))` — the else arm repeats `(/ 100 d)` TWICE, forming a
           common-subexpression class. A CSE that hoists the shared division to the body root (above the
           branch) would trap at d = 0, defeating the guard; the sharing must stay INSIDE the arm (or not
           fire). d = 0 → 0 (the guard shields the ÷0); d = 5 → 20+20 = 40. The REPEATED-occurrence
           companion of the single-division select-ification pin above: the single-div case guards
           select-ification, this guards the CSE hoist — a distinct pass with the same trap-safety
           obligation (adv-55 found the and/or-rhs face of this failing; this pins the if-arm face that
           holds).")
  (input (do (def (main (: d Int64)) (if (= d 0) 0 (+ (/ 100 d) (/ 100 d)))) (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 40 Int64)))

(case
  "a conditional's condition may itself be a conditional"
  (doc
    "`(if (if true false true) 1 2)`: the condition is an `if` that evaluates to `false`, so the
           outer conditional selects its else-branch, yielding 2. Pins that the condition position
           accepts an arbitrary Bool-valued expression — here a nested `if` — not only a literal or a
           direct comparison (core-semantics.md #Conditionals Evaluate One Branch: a conditional selects
           by its condition, whatever Bool expression computes it).")
  (input (if (if true false true) 1 2))
  (output (: 2 Int64)))

(case
  "a conditional whose condition folds to a constant still drops the untaken trapping branch"
  (doc
    "`(if (< 1 2) 7 (% 5 0))`: the condition is a COMPARISON that a constant-folding compiler
           reduces to true at compile time, after which the conditional selects its then-branch (7) and
           the untaken else-branch `(% 5 0)` — a modulo-by-zero that would trap — is never evaluated,
           so the result is 7. Pins that folding a conditional whose CONDITION became a constant is
           short-circuit-preserving: it becomes the taken branch and DROPS the other, exactly as a
           run-time conditional shields an unselected branch (core-semantics.md #Conditionals Evaluate
           One Branch). This is the dual of the divisor-folds-to-zero case (06-numeric-model.sexp): there
           a fold must not ERASE a trap the source denotes; here a fold must not MANUFACTURE a trap the
           source shields. Distinct from the literal-`true` shielding case above in that the shielding
           holds only AFTER the condition itself folds — a fold that evaluated both branches, or kept
           the trapping one, would wrongly trap.")
  (input (do (def (main) (if (< 1 2) 7 (% 5 0))) (export main)))
  (output (: 7 Int64)))

(case
  "a conditional selects a branch by a runtime value that is not known at compile time"
  (doc
    "`(def (f x) (if (< x 10) x (* x 2)))`: the condition `(< x 10)` depends on the runtime
           parameter `x`, so it CANNOT fold — the conditional must emit a real runtime branch that
           selects `x` (then) or `(* x 2)` (else) by the value computed at run time. `f(21)`: 21 is not
           < 10, so the else-branch yields 42. Pins the runtime conditional — a condition that is a
           genuine runtime value, not a literal or a fold — which a compiler lowers to a structured
           branch (push the condition, then a then/else region each leaving one value of the branches'
           shared type on the stack). Distinct from every conditional case above, whose condition is
           known at compile time (a literal, a nested `if`, or a foldable comparison): here the selection
           happens at run time. The companion `f(3)` (3 < 10) takes the then-branch and yields 3.")
  (input (do (def (f x) (if (< x 10) x (* x 2))) (def (main) (f 21)) (export main)))
  (output (: 42 Int64)))

(case
  "a runtime conditional selects its then-branch when the runtime condition holds"
  (doc
    "The then-branch companion to the runtime-conditional case above: with `x` = 3, `(< x 10)` is
           true at run time, so `(if (< x 10) x (* x 2))` selects `x` and yields 3. Together the pair
           pins that a runtime conditional selects EITHER branch by the run-time condition value (42 when
           false, 3 when true), so the structured branch is a genuine two-way selection, not a folded
           constant.")
  (input (do (def (f x) (if (< x 10) x (* x 2))) (def (main) (f 3)) (export main)))
  (output (: 3 Int64)))

(case
  "a conditional on a negated runtime condition selects the correct branch and shields the other"
  (doc
    "A conditional whose condition is `(not c)` may be lowered by SWAPPING the then/else branches and
           dropping the negation (rather than computing `not` then branching): `(if (not c) T E)` becomes
           `(if c E T)`. That rewrite must preserve BOTH the selection and the shielding. `(if (not b) 7 (/
           1 z))` with `b` = false: `(not false)` is true, so the THEN branch (7) is selected and the else
           `(/ 1 z)` (a division by zero at z = 0) is NOT evaluated — the result is 7, not a trap. A swap
           that mis-mapped the branches would select `(/ 1 z)` and trap; one that evaluated both would trap
           too. The anchor: with `b` = true, `(not true)` is false, so the else `(/ 1 z)` IS selected and
           traps. Pins the negated-if branch swap keeps the untaken branch shielded and the condition
           correctly inverted.")
  (input (do (def (main (: b Bool) (: z Int64)) (if (not b) 7 (/ 1 z))) (export main)))
  (call main (: false Bool) (: 0 Int64))
  (output (: 7 Int64)))

; ── More `if`-simplification Core rewrites (lower.rs Resolved::If) — backend-independent, both inherit ─
; Beyond the negated-condition branch swap above, the `if` lowering does three more pure rewrites on a
; RUNTIME condition, each behavior-preserving and inherited by both backends:
;  - DOUBLE-NEGATION unwind: `(if (not (not c)) T E)` → `(if c T E)` — each swap cancels a `not` layer.
;  - CONDITIONAL CONSTANT PROPAGATION on a repeated condition: within the then-branch `c` is known TRUE,
;    within the else-branch FALSE, so a branch that is ITSELF `(if c A B)` collapses to its `A`/`B` arm
;    (`(if c (if c A B) E)` → `(if c A E)`). The inner condition must be the SAME pure `c`.
;  - IDENTICAL-BRANCHES collapse: `(if c V V)` → `V` when the branches are core-equivalent — BUT only when
;    `c` is TRAP-FREE, since dropping the `if` drops the condition's evaluation (a trapping `c` must still
;    trap). These pin each on a runtime condition (a constant `c` takes the dead-branch-elimination path).
(case
  "a double-negated condition unwinds to the original selection"
  (doc
    "`(if (not (not (> b 0))) 10 20)` — two negations cancel (each branch-swap drops one `not`
           layer), so it selects exactly as `(if (> b 0) 10 20)`: b=5 → 10, b=-5 → 20. Pins the
           double-negation unwind on a runtime condition, both backends (the iterated companion of the
           single negated-condition swap above).")
  (input (do (def (main (: b Int64)) (if (not (not (> b 0))) 10 20)) (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64))
  (call main (: -5 Int64))
  (output (: 20 Int64)))

(case
  "a repeated condition inside a branch propagates the known truth value"
  (doc
    "Within the then-branch of `(if c …)` the condition `c` is known TRUE; within the else-branch,
           FALSE. So a nested `(if c …)` with the SAME condition collapses to the appropriate arm. Both
           faces in one program: `(if (> b 0) (if (> b 0) 1 2) (if (> b 0) 8 3))` — the then-side inner
           takes its true arm (1), the else-side inner takes its false arm (3), so the redundant inner
           tests are never re-evaluated. b=5 → 1, b=-5 → 3. Pins conditional constant propagation on a
           repeated pure condition, both backends (an inner arm that survived would give 2 or 8).")
  (input (do (def (main (: b Int64)) (if (> b 0) (if (> b 0) 1 2) (if (> b 0) 8 3))) (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: -5 Int64))
  (output (: 3 Int64)))

(case
  "an if with identical branches folds to the branch when the condition is trap-free"
  (doc
    "`(if (> b 0) 42 42)` — both branches are the same value, so the `if` collapses to `42`
           regardless of `b` (the condition `(> b 0)` is trap-free, so eliding it drops nothing). b=1 →
           42, b=-5 → 42. Pins the identical-branches collapse on a runtime, trap-free condition.")
  (input (do (def (main (: b Int64)) (if (> b 0) 42 42)) (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: -5 Int64))
  (output (: 42 Int64)))

(case
  "an if with identical branches still evaluates a TRAPPING condition"
  (doc
    "The trap-preservation anchor: `(if (> (/ 10 z) 0) 42 42)` has identical branches, but the
           condition `(> (/ 10 z) 0)` divides by zero at z=0 — so the collapse to `42` MUST NOT drop the
           condition's evaluation. The `if` folds its branches only when the condition is trap-free; a
           trapping condition keeps being evaluated → z=0 traps. Pins that the identical-branches fold is
           guarded on a trap-free condition, both backends.")
  (input (do (def (main (: z Int64)) (if (> (/ 10 z) 0) 42 42)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

; ── The SAME if-simplification rewrites over a FLOAT partial-order condition (NaN → false) ────────────
; The double-negation unwind, identical-branches collapse, and connective folds above all use an INTEGER
; `>` condition, which is a total order (classical two-valued Bool). A float ordering (`< <= > >=`, landed
; as an IEEE PARTIAL order) is the adversarial case for these SAME rewrites: a NaN operand makes the
; comparison FALSE, so the condition is not classically-complete (`(< x y)` and `(< y x)` and `(= x y)`
; can ALL be false at once). A fold that assumed a total-order/two-valued condition — e.g. rewriting
; `not (not (< a b))` by a boolean-algebra identity that presumes `c ∨ ¬c`, or collapsing a branch on the
; ASSUMPTION the float condition partitions the space — would MISCOMPILE the NaN case. These pin that each
; rewrite stays behavior-preserving with a float partial-order condition, on BOTH backends: the fold acts
; on the `if`/`not` STRUCTURE, never on assumed condition completeness.
(case
  "an if with identical branches folds even when the condition is a NaN-false float compare"
  (doc
    "`(if (< a b) 7 7)` — identical branches collapse to `7` regardless of the condition, and the
           condition `(< a b)` is a TRAP-FREE float compare (float ops never trap; NaN → false, not a
           halt), so eliding it drops nothing. Finite (1.0,2.0) → 7 AND the unordered NaN case (nan,nan,
           where `<` is false) → 7. Pins the identical-branches fold does not depend on the float
           condition's value — the NaN-false partial order is still trap-free, both backends.")
  (input (do (def (run (: a Float64) (: b Float64)) (if (< a b) 7 7)) (export run)))
  (call run (: 1.0 Float64) (: 2.0 Float64))
  (output (: 7 Int64))
  (call run (: nan Float64) (: nan Float64))
  (output (: 7 Int64)))

(case
  "a double-negated float compare unwinds to the compare, preserving NaN-false"
  (doc
    "`(if (not (not (< a b))) 1 0)` — the two `not` layers cancel to `(if (< a b) 1 0)`, and that
           must hold for the PARTIAL order: (1.0,2.0) → `1<2` true → 1, and (nan,1.0) → `nan<1` FALSE → 0.
           A boolean-algebra double-negation that folded via a `c ∨ ¬c`-style identity (assuming a
           two-valued condition) would wrongly flip the NaN case; pins the unwind acts on the `not`
           structure only, so NaN → false survives, both backends.")
  (input (do (def (run (: a Float64) (: b Float64)) (if (not (not (< a b))) 1 0)) (export run)))
  (call run (: 1.0 Float64) (: 2.0 Float64))
  (output (: 1 Int64))
  (call run (: nan Float64) (: 1.0 Float64))
  (output (: 0 Int64)))

(case
  "a conjunction of two float compares preserves NaN-false through the connective fold"
  (doc
    "`(if (and (< a b) (> b a)) 1 0)` — the short-circuit `and` (a nested conditional) of two float
           partial-order compares. Finite ordered (1.0,2.0): `1<2` and `2>1` both true → 1. The NaN case
           (nan,1.0): `nan<1` is false → the `and` short-circuits false → 0 (a de-morgan/connective fold
           must not turn the unordered pair true). Reversed finite (2.0,1.0): `2<1` false → 0. Pins the
           connective fold over float compares preserves the NaN-false partial order, both backends.")
  (input (do (def (run (: a Float64) (: b Float64)) (if (and (< a b) (> b a)) 1 0)) (export run)))
  (call run (: 1.0 Float64) (: 2.0 Float64))
  (output (: 1 Int64))
  (call run (: nan Float64) (: 1.0 Float64))
  (output (: 0 Int64))
  (call run (: 2.0 Float64) (: 1.0 Float64))
  (output (: 0 Int64)))

(case
  "a conjunction guards a let over a runtime value inside a conditional"
  (doc
    "An INTEGRATION case: several control constructs composed in one function over a runtime
           parameter, the way a real program (not an isolated feature test) uses the language.
           `classify x = (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0)` composes: a
           short-circuit `and` of two comparisons as the condition (each operand a runtime `>`/`<`), a
           `let` binding a RUNTIME value `(* x x)` in the then-branch (so it must emit a real local, not
           a compile-time alias), the outer conditional selecting Int64 branches, and the arithmetic —
           all driven by the runtime argument. `classify 4`: `0 < 4` and `4 < 10` both hold, so
           `(let ((y (* 4 4))) (- y 1))` = 16 - 1 = 15. Pins that these constructs COMPOSE in one
           function — the short-circuit `and` (which desugars to a nested conditional), a runtime `let`,
           and the enclosing `if` nest correctly and thread their values — not merely that each works in
           isolation. The out-of-range companion below takes the else-branch.")
  (input
    (do
      (def (classify x) (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0))
      (def (main) (classify 4))
      (export main)))
  (output (: 15 Int64)))

(case
  "the guarded-let conditional takes its else-branch when the conjunction is false"
  (doc
    "The else companion of the integration case above: `classify 20` — `20 < 10` is false, so the
           short-circuit `and` is false and the outer conditional selects its else-branch 0, never
           evaluating the `let`. Together the pair pins that the composed `and`/`let`/`if` selects by the
           runtime value in both directions (15 in range, 0 out of range), and that the short-circuit
           `and` shields the `let`-bearing then-branch when the guard fails.")
  (input
    (do
      (def (classify x) (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0))
      (def (main) (classify 20))
      (export main)))
  (output (: 0 Int64)))

; --- A conditional's branches must have the same type ------------------------------------
; core-semantics.md #Conditionals Evaluate One Branch, 2nd sentence: "Every branch of a
; conditional MUST be type-checked whether or not it is evaluated, so that an unevaluated
; branch cannot carry a deferred error." So a conditional whose branches have DIFFERENT types
; is ill-typed even when the condition is a compile-time constant that never evaluates the
; mismatched branch — the compiler MUST reject it (CDZ0203, a type mismatch). The rejection is the recorded
; outcome; the program does not run, so it has no branch value. A generation that does not yet
; type-check the unevaluated branch declines rather than emitting a component
; (reject-don't-miscompile).
(case
  "a conditional with an integer then-branch and a boolean else-branch is a type error"
  (doc
    "The then-branch is Int64, the else-branch is Bool — different types. Even with a constant
           condition selecting the Int64 branch, the compiler MUST type-check BOTH branches and reject
           the mismatch (CDZ0203) rather than run the program.")
  (input (if true 1 false))
  (error CDZ0203))

(case
  "a conditional type error is caught even when the mismatched branch is the one taken"
  (doc
    "The companion with the condition false, selecting the Bool branch: the branches still
           disagree in type (Int64 vs Bool), so the compiler MUST reject (CDZ0203). Pins that the
           check is on the pair of branch types, not on which branch would run.")
  (input (if false 1 false))
  (error CDZ0203))

(case
  "a conditional with a compound branch and a scalar branch is a type error even when the compound branch is dead"
  (doc
    "`(if false (record (a 1)) 7)` — the then-branch is a compound (a record), the else-branch is a
           scalar (Int64); they have different types, so the conditional is ill-typed and the compiler MUST
           reject it (CDZ0203). The constant condition `false` selects the SCALAR branch, so a compiler that
           const-folds the conditional to its taken branch would discard the compound then-branch WITHOUT
           type-checking it and silently accept an ill-typed program — a miscompile. The type-check is on the
           PAIR of branches, so it must happen BEFORE (or independently of) any fold that eliminates a branch:
           an unevaluated branch cannot carry a deferred type error. This pins the compound-vs-scalar instance
           of the dead-branch check, which the scalar-vs-scalar cases above do not exercise (folding a compound
           branch away is where the check is easiest to skip).")
  (input (if false #record((= a 1)) 7))
  (error CDZ0203))

; The branch-type-agreement check must fire when the conditional is INSIDE A FUNCTION BODY with a
; constant condition, not only at the top-level entry expression. The cases above pair mismatched
; branches in `main`'s own body and are correctly rejected; but the same mismatch inside a `def`ed
; function with a compile-time-constant condition SLIPS — the const-condition fold in a function body
; discards the untaken branch WITHOUT the pair-of-branches type-check that #Conditionals Evaluate One
; Branch requires ("every branch … type-checked whether or not it is evaluated"). `(def (f) (if true 1
; false))` pairs an Int64 then-branch with a Bool else-branch — ill-typed exactly as the top-level `(if
; true 1 false)` is (CDZ0203) — yet the seed accepts it and `f` returns 1, an ill-typed program run
; (and it composes: `(+ (f) 0)` = 1). Worse, when the surviving (taken) branch is a COMPUTED expression
; rather than a constant — `(def (f n) (if true (+ n 1) false))`, whose `(+ n 1)` cannot fold to a
; literal — the unchecked Int/Bool branch-representation mismatch makes the compiler emit an INVALID
; wasm component (it fails validation), not merely a wrong value: a fold that drops a branch dropped its
; type-check, and the two branches' incompatible representations reach code generation. The internal
; checks of the dropped branch DO survive the fold (an else `(+ 1 true)` is still rejected "operation on
; mismatched types", an unbound else name is still CDZ0101) — only the branch-type-AGREEMENT check is
; lost, which is the one these cases pin. This is the in-function companion of the top-level dead-branch
; cases: the fold that eliminates a branch must not eliminate the agreement check, wherever the `if` sits.
(case
  "a conditional inside a function with a constant condition and mismatched branches is a type error"
  (doc
    "`(def (f) (if true 1 false))` pairs an Int64 then-branch with a Bool else-branch — different
           types, ill-typed exactly as the top-level `(if true 1 false)` above (CDZ0203). But the `if` is
           inside a function body with a constant condition, and the seed's const-condition fold in a
           function body discards the untaken `false` branch WITHOUT the pair-of-branches type-check
           (core-semantics.md #Conditionals Evaluate One Branch: every branch type-checked whether or not
           evaluated). The seed accepts it and `f` returns 1 — an ill-typed program run — where the
           identical `if` at the top-level entry expression is correctly rejected. (When the surviving
           branch is a COMPUTED expression, `(def (f n) (if true (+ n 1) false))`, the unchecked Int/Bool
           branch-representation mismatch makes the compiler emit an INVALID component rather than a wrong
           value.) Pins that the dead-branch agreement check fires inside a function body too, not only at
           the top-level entry — the internal checks of the dropped branch already survive the fold; only
           the branch-type-agreement check is lost. A generation that type-checks the pair of branches
           before folding declines rather than running the ill-typed program or emitting invalid code.")
  (input (do (def (f) (if true 1 false)) (def (main) (f)) (export main)))
  (error CDZ0203))

(case
  "a conditional with integer and floating-point branches is a type error"
  (doc
    "Int64 and Float64 are distinct numeric types that do not silently unify (numeric-model.md
           #Numeric Types Do Not Silently Promote). A conditional with an Int64 branch and a Float64
           branch is therefore ill-typed and the compiler MUST reject it (CDZ0201).")
  (input (if true 1 3.5))
  (error CDZ0201))

; The branch-type-agreement check must compare branches STRUCTURALLY, not only by coarse kind: two
; branches that are both tuples but of DIFFERENT ARITY are different types (a tuple's arity is part of
; its type, type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix), so the conditional
; is ill-typed even though both branches are "a tuple." `(if true (tuple 1 2) (tuple 3 4 5))` pairs a
; two-tuple with a three-tuple; the whole `if` has no single type, so the compiler MUST reject it
; (CDZ0203) — a check that compares only the branches' KIND (tuple vs tuple) and not their arity accepts
; the mismatch and returns whichever branch the constant condition selects, an unevaluated branch carrying
; a deferred type error (core-semantics.md #Conditionals Evaluate One Branch — every branch type-checked).
; A generation that does not yet compare branch shapes structurally declines rather than accepting.
(case
  "a conditional with two tuple branches of different arity is a type error"
  (doc
    "`(if true (tuple 1 2) (tuple 3 4 5))` pairs a two-element tuple with a three-element tuple —
           different types, since a tuple's arity is part of its type. The whole conditional has no single
           type, so it is ill-typed and the compiler MUST reject it (CDZ0203), exactly as the Int/Bool and
           compound/scalar branch-mismatch cases above. Pins that branch-type agreement is checked
           STRUCTURALLY, not only at coarse kind (both branches being 'a tuple' is not enough) — a compiler
           comparing only branch kinds accepts this and returns the two-tuple, an ill-typed program run.")
  (input (if true #tuple(1 2) #tuple(3 4 5)))
  (error CDZ0203))

(case
  "a conditional with two tuple branches of different element type is a type error"
  (doc
    "`(if true (tuple 1 2) (tuple 1 true))` pairs `(Tuple Int64 Int64)` with `(Tuple Int64 Bool)` —
           same arity but a different element type at position 1, so different types. The conditional is
           ill-typed (CDZ0203), the element-type companion of the arity case above. Pins that the structural
           branch-type comparison descends into a tuple's element types, not only its arity — the same
           depth the list-element homogeneity check already applies.")
  (input (if true #tuple(1 2) #tuple(1 true)))
  (error CDZ0203))

; The structural branch-type check must NOT treat a list's LENGTH as part of its type — unlike a tuple's
; arity. A list is a variable-length sequence typed by its element type (collections-and-text.md #A List
; Is An Ordered Homogeneous Sequence, #A List Is Grown By Functional Construction), so two list branches
; of the SAME element type but DIFFERENT LENGTH are the SAME type `(List Int64)`, and the conditional is
; well-typed — its value is whichever list the condition selects. `(if true (list 1 2) (list 3 4 5))`
; yields `(list 1 2)`. A compiler that reuses the tuple-arity branch-shape check on lists wrongly rejects
; this well-typed conditional as "branches have different shapes" — length is a tuple's type distinction,
; not a list's. (The genuinely ill-typed list-branch case is two lists of different ELEMENT type, e.g.
; `(if … (list 1 2) (list true false))` — those are different types and rejected, exactly as any type
; mismatch is; only the different-LENGTH same-element-type case must be accepted.)
(case
  "a conditional with two list branches of different length is well-typed"
  (doc
    "`(if true (list 1 2) (list 3 4 5))` pairs a two-element list with a three-element list — the
           SAME type `(List Int64)`, since a list's length is not part of its type (a list is
           variable-length; collections-and-text.md #A List Is An Ordered Homogeneous Sequence). The
           conditional is well-typed and yields the selected branch `(list 1 2)`. This is the list
           counterpoint to the tuple-arity branch case above: a tuple's arity IS part of its type (so
           different-arity tuple branches are rejected), but a list's length is NOT, so different-length
           list branches MUST be accepted. Pins that the branch-shape check does not treat list length as
           a shape mismatch — a compiler reusing the tuple-arity check on lists wrongly rejects this.")
  (input (if true #list(1 2) #list(3 4 5)))
  (output (: #list(1 2) (List Int64))))

(case
  "a RUNTIME if-chain selects among four different-length heap lists"
  (doc
    "The runtime/depth upgrade of the const two-branch case above: a 3-deep nested `if` chain over a
           runtime `n` selects among FOUR list literals of lengths 3/2/1/0 — every branch constructs a heap
           value, only the selected branch's construction is observable, and the four calls exercise every
           arm (20→3, 7→2, 3→1, -1→0 via List.len). Pins the classify-into-buckets idiom (a threshold
           ladder returning collections): each arm's heap construction is confined to its branch (an emit
           hoisting all four constructions, or unifying the branches to one shared list, would still pass
           len checks — but a branch-confusion in the nested selects would misroute the middle calls).")
  (input
    (do
      (def
        (main (: n Int64))
        (List.len (if (> n 10) #list(1 2 3) (if (> n 5) #list(1 2) (if (> n 0) #list(1) #list())))))
      (export main)))
  (call main (: 20 Int64))
  (output (: 3 Int64))
  (call main (: 7 Int64))
  (output (: 2 Int64))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: -1 Int64))
  (output (: 0 Int64)))

; --- A conditional's condition must be a Bool --------------------------------------------
; core-semantics.md #Conditionals Evaluate One Branch: a conditional selects a branch by its
; condition, which is a Bool. A condition of any other type is ill-typed — the compiler MUST
; reject it (CDZ0203). A COMPOUND condition (a tuple/record/list) must be rejected as a not-a-Bool
; type error with the constructor `tuple`/`record`/`list` intact — it is a recognized form (it
; builds a value everywhere else), so a diagnostic of "unbound name: tuple" would be a misleading
; code (CDZ0101) for what is plainly a not-a-Bool type error, the same wrong-diagnostic class as an
; out-of-range integer literal reported as an unbound name (01-literals.sexp).
(case
  "an integer if condition is a type error, not a running conditional"
  (doc
    "1 is Int64, not Bool. A conditional's condition selects a branch and MUST be a Bool; an
           Int64 condition is ill-typed (CDZ0203). A C-like language treats a nonzero int as true —
           Cadenza does not silently coerce (numeric-model.md #Numeric Types Do Not Silently
           Promote); there is no truthiness. A generation that does not yet wire the CDZ0203 code
           declines rather than running the program (reject-don't-miscompile).")
  (input (if 1 10 20))
  (error CDZ0203))

(case
  "a compound if condition is a type error, not an unbound name"
  (doc
    "A tuple is not a Bool, so `(if (tuple 1 2) …)` is ill-typed (CDZ0203). The constructor
           `tuple` is a recognized form — `(tuple 1 2)` builds a value in every other position — so
           reporting `unbound name: tuple` (CDZ0101) would mistake a not-a-Bool type error for a name
           resolution failure. The condition's type is what is wrong, not the spelling of a name.
           Pins that a compound condition is rejected as a type error with the constructor intact,
           the same misleading-diagnostic class as an out-of-range literal reported as unbound.")
  (input (if #tuple(1 2) 10 20))
  (error CDZ0203))

(case
  "a pattern binds a name scoped to its branch"
  (doc
    "Witnesses core-semantics.md #Bindings Introduced By A Pattern Are Scoped To Its Branch.
           Option is declared where used as (Some <value> | None) (options/code-shape/); the Some
           branch binds n to the payload, in scope only in that branch. Patterns are uniform:
           (Some n) for unary, (None _) for nullary — both single-arity.")
  (input (match (Some 5) ((Some n) n) ((None _) 0)))
  (output (: 5 Int64)))

(case
  "matching on integer literals"
  (doc
    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: a match can branch on
           literal values, not just constructors. Integer literal patterns match by equality. The
           compiler uses this to dispatch on instruction opcodes and section IDs.")
  (input (match 2 (0 "zero") (1 "one") (2 "two") (_ "many")))
  (output (: "two" String)))

; --- A literal pattern's type must match the scrutinee's type ----------------------------
; A literal pattern matches the scrutinee by equality (above), and equality is only defined between
; values of the SAME type (core-semantics.md #Equality Is Structural; a cross-type comparison is a
; type error). So a literal pattern whose type differs from the scrutinee's — a `true` (Bool) pattern
; against an Int64 scrutinee, an integer pattern against a Bool scrutinee — can never meaningfully
; match: it is a static type mismatch between the arm and the scrutinee, a type error (CDZ0201), the
; same class as a tuple pattern of the wrong arity or a `(Some x)` pattern against an Int64. The
; compiler rejects the ill-typed arm; a generation that does not yet check the pattern's type against
; the scrutinee's declines rather than running the program (reject-don't-miscompile).
(case
  "a boolean literal pattern against an integer scrutinee is a type error"
  (doc
    "The scrutinee `5` is Int64; the pattern `true` is Bool. A literal pattern matches by
           equality, which is only defined within one type, so a Bool pattern can never match an Int64
           value — the arm is ill-typed and the compiler MUST reject the match (CDZ0201). Pins that a
           literal pattern's type is checked against the scrutinee's, not silently failed to match.")
  (input (match 5 (true 1) (_ 0)))
  (error CDZ0201))

(case
  "an integer literal pattern against a boolean scrutinee is a type error"
  (doc
    "The mirror: scrutinee `true` is Bool, pattern `5` is Int64 — a type mismatch, so the arm is
           ill-typed (CDZ0201). Pins the check in both directions — the scrutinee and every literal
           pattern must share a type.")
  (input (match true (5 1) (_ 0)))
  (error CDZ0201))

(case
  "matching on string literals"
  (doc
    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: string literal patterns
           match by equality. The compiler uses this heavily to dispatch on instruction tags like
           'i64.const', 'i64.add', etc. — replacing nested if/= chains with readable match.")
  (input (match "hello" ("hello" 1) ("world" 2) (_ 0)))
  (output (: 1 Int64)))

(case
  "matching on a string produced by an expression"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected: string literal patterns match by
           equality against the scrutinee's VALUE, whether the scrutinee is written as a bare literal
           (the case above) or produced by an expression. `(String.concat \"a\" \"b\")` evaluates to
           \"ab\", which the \"ab\" arm matches, yielding 100 — not the wildcard. (That the two strings
           are equal is independently witnessed: `(= (String.concat \"a\" \"b\") \"ab\")` is true. A
           bare and a let-bound \"ab\" scrutinee already select the arm; a string-valued expression
           must behave identically — the common compiler idiom of dispatching on a computed
           instruction name.)")
  (input (match (String.concat "a" "b") ("ab" 100) (_ 200)))
  (output (: 100 Int64)))

(case
  "matching on a sliced string selects the literal arm"
  (doc
    "Companion using another string-producing operation: `(String.slice \"hello\" 0 2)` yields Some
           \"he\"; `expect` unwraps the in-bounds slice to \"he\", which the \"he\" arm matches, yielding
           100. A slice result is fallible (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping), so the program names the in-bounds expectation before matching the substring.")
  (input (match (Option.expect (String.slice "hello" 0 2) "slice is in bounds") ("he" 100) (_ 200)))
  (output (: 100 Int64)))

; --- Requiring the value of an optional at run time -------------------------------------------
; core-semantics.md #Requiring The Value Of An Optional Traps On Absence: `Option.expect` (and its
; Result twin) unwraps the present variant's payload or traps on absence. The cases above exercise
; it only on COMPILE-TIME-CONSTANT optionals (a literal slice/index). These pin it on a RUNTIME
; optional — a parameter, or a value a runtime operation produced — where present/absent is decided
; at run time by the sum's discriminant, not folded. This is the compiler's unwrap-or-trap idiom:
; assert a `List.at`/`Bytes.at`/`checked-*` result is present, taking its value or trapping.
(case
  "expect unwraps the present case of a runtime optional"
  (doc
    "`(g (Some 7))` calls `(g o) = (Option.expect o \"m\")` on a RUNTIME optional (the parameter
           `o`, not a constant): the discriminant says Some at run time, so expect yields its payload 7.
           Pins expect on an optional whose present/absent is decided at run time — the unwrap-or-trap
           idiom over a value the compiler cannot fold, distinct from expect on a literal optional.")
  (input (do (def (g o) (Option.expect o "m")) (def (main) (g (Some 7))) (export main)))
  (output (: 7 Int64)))

(case
  "expect traps on the absent case of a runtime optional"
  (doc
    "The absent companion: `(g (None unit))` on the same `(Option.expect o \"m\")` sees the None
           discriminant at run time, so expect traps rather than producing a value (core-semantics.md
           #Requiring The Value Of An Optional Traps On Absence). The terminal condition is the trap.")
  (input (do (def (g o) (Option.expect o "m")) (def (main) (g (None unit))) (export main)))
  (trap "m"))

(case
  "expect on a RUNTIME-absent optional traps with the canonical unreachable kind"
  (doc
    "The runtime (non-const-folded) absent expect: `main`'s parameter feeds a runtime `Option Int64`
           that is always `None`, so `(Option.expect o \"…\")` sees the None discriminant AT RUN TIME and
           traps (core-semantics.md #Requiring The Value Of An Optional Traps On Absence). The trap's
           canonical KIND is `unreachable` — the SAME on every backend: wasm's `SumExpect` absent branch is
           an `unreachable` instruction, and the Rust backend panics with a reason classifying as
           `unreachable` (matching the explicit-`trap` lowering). Pins that a RUNTIME expect-on-absent traps
           consistently across backends (distinct from the const-folded case above, whose recorded message
           is a custom string the trap-kind grader does not classify).")
  (input
    (do
      (def (g (: o (Option Int64))) (Option.expect o "boom"))
      (def (main (: k Int64)) (g (if (> k 0) (Option.None) (Option.None))))
      (export main)))
  (call main (: 5 Int64))
  (trap "unreachable"))

(case
  "expect makes a checked-arithmetic result trap on overflow"
  (doc
    "The compiler idiom expect exists for: turn a non-trapping `Int64.checked-add` into a TRAPPING
           add. `(add-ck a b) = (Option.expect (Int64.checked-add a b) \"overflow\")` yields the sum when
           in range — `(add-ck 20 22)` = 42, usable directly in arithmetic. Pins expect on a RUNTIME
           `Option<Int64>` a runtime operation produced, unboxing to the Int64 payload.")
  (input
    (do
      (def (add-ck a b) (Option.expect (Int64.checked-add a b) "overflow"))
      (def (main) (+ (add-ck 20 22) (add-ck 1 1)))
      (export main)))
  (output (: 44 Int64)))

(case
  "expect on an overflowing checked add traps"
  (doc
    "`(add-ck Int64.max 1)`: `Int64.checked-add` OVERFLOWS, so it returns `None` WITHOUT trapping (that is
           the point of a checked add), and `Option.expect None …` then traps on the ABSENCE — a bare
           `unreachable` (the expect message `\"overflow\"` is the author's label, DROPPED at the boundary; the
           trap KIND is the expect-absence unreachable, NOT an arithmetic overflow). Contrast
           `(Int64.wrapping-add Int64.max 1)`, which wraps to MIN without trapping at all. (Corrected from a
           stale `(trap \"overflow\")` expectation that never matched — surfaced once a trap-vs-trap KIND
           mismatch became a hard fail instead of a hidden todo.)")
  (input
    (do
      (def (add-ck a b) (Option.expect (Int64.checked-add a b) "overflow"))
      (def (main) (add-ck Int64.max 1))
      (export main)))
  (trap "unreachable"))

(case
  "expect unwraps the ok case of a runtime result"
  (doc
    "`Result.expect` is the Result twin of `Option.expect`: `(g (Ok 99))` on `(Result.expect r \"m\")`
           sees the Ok discriminant at run time and yields its payload 99; the Err case would trap. Pins
           expect on a runtime Result, the same unwrap-or-trap accessor over the two-variant Result sum.")
  (input (do (def (g r) (Result.expect r "m")) (def (main) (g (Ok 99))) (export main)))
  (output (: 99 Int64)))

(case
  "expect traps on the err case of a RUNTIME result"
  (doc
    "The Result absent companion (the Err twin of the Option-None expect-trap): a runtime `Result
           Int64 Int64` that is always `Err` feeds `(Result.expect r \"…\")`, which sees the Err
           discriminant AT RUN TIME and traps (core-semantics.md #Requiring The Value Of An Optional Traps
           On Absence, extended to Result's Err). The trap's canonical KIND is `unreachable` on every
           backend — wasm's `SumExpect` absent branch is an `unreachable` instruction and the Rust backend
           panics with a reason classifying the same way. Pins that `Result.expect` on Err traps
           consistently across backends, the two-variant-Result companion of the Option-None trap.")
  (input
    (do
      (def (g (: r (Result Int64 Int64))) (Result.expect r "boom"))
      (def (main (: k Int64)) (g (if (> k 0) (Result.Err 1) (Result.Err 2))))
      (export main)))
  (call main (: 5 Int64))
  (trap "unreachable"))

(case
  "matching falls through to else when no literal matches"
  (doc
    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: when no literal pattern
           matches, the else (wildcard) catches it. Without else, a non-exhaustive match traps.")
  (input (match 99 (0 "zero") (1 "one") (_ "other")))
  (output (: "other" String)))

; --- A match arm may carry a guard ---------------------------------------------------------
; core-semantics.md #Matching Is Exhaustive Or Rejected: an arm's pattern may carry a boolean GUARD
; `pattern if <guard>` — the arm is selected only when the pattern matches AND the guard, a pure
; expression evaluated with the pattern's bindings in scope, is true. A failing guard falls through
; to the following arms exactly as a non-matching pattern does. The guard is an ordinary expression
; (it can read the names the pattern binds); it refines WHICH values an arm accepts without changing
; the pattern's shape. A guard does NOT count toward exhaustiveness (a guarded arm might not fire), so
; a match whose only arms are guarded is non-exhaustive and rejected — the cases below pin selection,
; fall-through, binding-visibility, and the exhaustiveness rule.
(case
  "a guarded arm is selected when its guard holds"
  (doc
    "The arm `x if x < 0` binds `x` to the scrutinee and is selected only if the guard `x < 0`
           is true. For scrutinee -5 the guard holds, so the arm fires and the result is -1. Pins that
           a guard `pattern if <expr>` gates its arm on a boolean condition evaluated with the
           pattern's bindings in scope (core-semantics.md #Matching Is Exhaustive Or Rejected).")
  (input (match -5 ((guard x (< x 0)) -1) (_ 1)))
  (output (: -1 Int64)))

(case
  "a failing guard falls through to a later arm"
  (doc
    "The mirror: for scrutinee 5 the guard `x < 0` is false, so the guarded arm does NOT fire and
           the match falls through to the wildcard, yielding 1 — exactly as a non-matching pattern
           falls through. Pins that a false guard skips its arm rather than trapping or forcing it.")
  (input (match 5 ((guard x (< x 0)) -1) (_ 1)))
  (output (: 1 Int64)))

(case
  "a guard sees the names its pattern binds and arms are tried in order"
  (doc
    "Two guarded arms binding `n`: for scrutinee 7 the first guard `n = 0` is false, the second
           `n < 10` is true, so the second arm fires and returns `n` (7). Pins that a guard reads the
           pattern's binding (`n` is in scope in the guard) and that guarded arms are tried top-to-bottom,
           the first whose pattern-and-guard both hold winning.")
  (input (match 7 ((guard n (= n 0)) 100) ((guard n (< n 10)) n) (_ 999)))
  (output (: 7 Int64)))

(case
  "a runtime scrutinee gates through a multi-guard chain and falls through to the wildcard"
  (doc
    "The runtime (non-folded) face of the multi-guard chain: `classify` matches its RUNTIME param `n`
           through two guarded arms then a wildcard — none folds, so the emitted probe chain tests each
           guard in order and falls through on failure. classify(-5) = -1 (first guard `< 0` holds);
           classify(7) = 7 (first fails → second `> 0` holds, returning the bound scrutinee); classify(0) =
           0 (both guards fail → the unguarded wildcard tail). Pins the runtime multi-guard gate + ordered
           fall-through to the wildcard over one runtime scalar, distinct from the constant-scrutinee cases
           above (which fold) and the single-guard runtime case below.")
  (input
    (do
      (def (classify (: n Int64)) (match n ((guard x (< x 0)) -1) ((guard x (> x 0)) x) (_ 0)))
      (def (main (: n Int64)) (classify n))
      (export main)))
  (call main (: -5 Int64))
  (output (: -1 Int64))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a guarded match over a runtime scrutinee with constant arms computes correctly"
  (doc
    "`(def (main (: x Int64)) (match x ((guard n (> n 100)) 1) (_ 0)))` — a guarded scalar match over
           a RUNTIME argument. This is semantically `(if (> x 100) 1 0)`; the guard `(> n 100)` reads the
           binder `n` (the scrutinee value) and the two arms are constants, so the compiler compiles it to
           the same branchless form the plain `if` gets. Called with 150 (> 100) → 1. Pins that a
           runtime guarded match with a binder-reading guard evaluates the guard against the scrutinee and
           selects the right arm.")
  (input (do (def (main (: x Int64)) (match x ((guard n (> n 100)) 1) (_ 0))) (export main)))
  (call main (: 150 Int64))
  (output (: 1 Int64)))

(case
  "a guarded match over a runtime scrutinee whose guard fails takes the wildcard"
  (doc
    "The false-guard companion of the runtime guarded match `(match x ((guard n (> n 100)) 1) (_ 0))`:
           called with 50 (not > 100), so the guard fails and the unguarded wildcard 0 is taken. Together
           with the true case this pins value parity of the desugared branchless form with the guarded
           probe chain it replaced.")
  (input (do (def (main (: x Int64)) (match x ((guard n (> n 100)) 1) (_ 0))) (export main)))
  (call main (: 50 Int64))
  (output (: 0 Int64)))

(case
  "a bare-binder guard over a STRING (heap) param scrutinee in a helper binds and evaluates"
  (doc
    "The HEAP-param face of the bare-binder guard: the scalar cases above guard an Int64 scrutinee;
           this guards a `String` (heap-typed) parameter in a NON-entry helper. `(match s ((guard t (< t
           \"m\")) 1) (_ 3))` with `s : String` a helper param must bind `t` to the scrutinee and run the
           guard — \"apple\" < \"m\" → 1 (core-semantics.md #A Binding Position Accepts An Irrefutable
           Pattern; the guard closes over the bound `t`). Formerly all 3 targets over-rejected CDZ0101 (the
           guard binder orphaned): the finding-#46 fix wrapped the guarded-SCALAR desugar in a binder let,
           but the sibling runtime-STRING-match desugar built the arm's then-branch WITHOUT that let-wrap,
           so `t` never bound → the extracted `if` severed it from its `(guard …)` ancestor. This pins the
           heap/string face now binds (v-inference lambda/guard-desugar fix), the String companion of the
           Int64 guarded-scalar cases.")
  (input
    (do
      (def (band (: s String)) (match s ((guard t (< t "m")) 1) (_ 3)))
      (def (main (: k Int64)) (band "apple"))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a guard reads a binding from the enclosing scope, not only its pattern's"
  (doc
    "A guard is an ordinary expression evaluated in the arm's full scope, so it reads names from the
           ENCLOSING scope too, not only the ones its pattern binds: `classify` guards `v if v < limit`
           where `v` is the pattern binder but `limit` is a FUNCTION PARAMETER. The arm fires when the
           scrutinee is below the dynamic threshold — the common real-world guard (Ordering.of a matched value
           against a runtime bound, not a literal). For x below `limit` it returns 0, at or above it falls
           through to 1. Every other guard case compares a binder to a LITERAL; this pins that a guard also
           closes over the enclosing bindings. Both operands runtime (call args), so nothing folds.")
  (input
    (do
      (def (classify (: x Int64) (: limit Int64)) (match x ((guard v (< v limit)) 0) (_ 1)))
      (def (main (: x Int64) (: limit Int64)) (classify x limit))
      (export main)))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 8 Int64) (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a match whose only arm is guarded is non-exhaustive"
  (doc
    "A guard does not count toward exhaustiveness: a guarded arm might not fire (its guard may be
           false), so it cannot be the coverage for any value. A match on an Int64 whose sole arm is
           `x if x < 0` — with no unconditional arm or wildcard — therefore covers no value unconditionally
           and is non-exhaustive; the compiler MUST reject it (CDZ0210), the same rejection as a match
           missing a case. Pins that guarded arms are excluded from the exhaustiveness check. A generation
           that does not yet check runtime exhaustiveness declines rather than emitting a component.")
  (input (match 5 ((guard x (< x 0)) 1)))
  (error CDZ0210))

(case
  "a match whose only arm is guarded by a literally-true condition is still non-exhaustive"
  (doc
    "The exhaustiveness check treats EVERY guard as opaque — it does not reason about whether the
           guard condition is true. `(match 5 ((guard x true) 1))` has a guard whose condition is the
           literal `true`, so the arm always fires at run time; but the checker MUST still reject it
           (CDZ0210) as non-exhaustive, exactly as the `(< x 0)` case above. A checker that 'optimized' by
           recognizing a literally-true guard as an unconditional arm would wrongly ACCEPT this match, then
           the same reasoning would have to extend to arbitrarily complex always-true conditions — the
           conservative rule is simpler and sound: a guarded arm never counts toward coverage, whatever its
           condition. Pins that guard truth is not analyzed for exhaustiveness.")
  (input (match 5 ((guard x true) 1)))
  (error CDZ0210))

; --- A guard CONDITION must be Bool, and faults inside it surface ---------------------------------
; A guarded arm `(guard <pattern> <cond>)` gates the arm on the boolean predicate `<cond>`, so
; `<cond>` must be Bool — exactly like an `if` condition. A non-Bool guard condition is a type error
; (CDZ0203, "guard condition must be Bool", naming the offending type), and a fault INSIDE the guard
; condition (e.g. an unbound name) surfaces rather than being silently swallowed — the condition is
; walked. A well-typed Bool guard compiles and runs clean. Migrated from rcdzc match_engine
; `a_guard_condition_must_be_bool_and_its_faults_surface`.
(case
  "a non-Bool guard condition is a type error naming the offending type"
  (doc
    "`(guard x (+ x 1))` uses an Int64 as the arm's boolean predicate. The guard condition must be
           Bool like an `if` condition, so this is CDZ0203 'guard condition must be Bool', naming the
           offending type (Int64). A generation that used a non-boolean as a branch condition would wrongly
           accept it.")
  (input (do (def (g (: n Int64)) (match n ((guard x (+ x 1)) x) (_ 0))) (export g)))
  (error CDZ0203 (message "guard condition must be Bool") (message "Int64")))

(case
  "a String guard condition is likewise rejected — the Bool check is general"
  (doc
    "A String guard `(guard x \"y\")` is rejected with the same CDZ0203 'guard condition must be Bool'
           — the check is general over the condition's type, not int-specific.")
  (input (do (def (g (: n Int64)) (match n ((guard x "y") x) (_ 0))) (export g)))
  (error CDZ0203 (message "guard condition must be Bool")))

(case
  "a fault inside a guard condition surfaces — the condition is walked"
  (doc
    "An unbound name inside a guard condition `(guard x (> x zzz))` surfaces as CDZ0101 rather than
           being silently accepted. The guard condition is walked, so faults within it are reported.")
  (input (do (def (g (: n Int64)) (match n ((guard x (> x zzz)) x) (_ 0))) (export g)))
  (error CDZ0101))

(case
  "a well-typed Bool guard condition compiles and runs clean"
  (doc
    "The no-false-positive control: a Bool guard `(guard x (> x 0))` is well-typed and produces no
           fault. For n = 5 the guard 5 > 0 holds, so the guarded arm fires and returns n = 5.")
  (input (do (def (g (: n Int64)) (match n ((guard x (> x 0)) x) (_ 0))) (export g)))
  (call g (: 5 Int64))
  (output (: 5 Int64)))

; --- A guard may refine a VARIANT pattern ---------------------------------------------------------
; A guard composes with a variant (sum) pattern, not only a bare binder: `(guard (Some x) <cond>)`
; fires when the scrutinee is `Some` AND `<cond>` (which reads the payload binder `x`) holds. On a
; false guard the arm falls through to a LATER arm — including a later arm of the SAME variant — just
; as a scalar guard does. The payload binder is in scope for the guard cond (resolved through the
; `(guard …)` wrapper to the inner variant pattern), and a guarded variant arm does NOT count toward
; exhaustiveness (so a match whose only `Some` arm is guarded, with no `Some` fall-through, is
; non-exhaustive). These pin the guard-over-variant surface end to end.
(case
  "a guard over a variant pattern gates on the payload"
  (doc
    "`(match o ((guard (Some x) (> x 0)) x) ((Some y) (- 0 y)) ((None) 0))` — the natural `(Some x)
           if x > 0` shape: the arm fires when the Option is `Some` AND its payload is positive, binding
           `x` to the payload. For `(Some 5)` the guard `5 > 0` holds, so the arm returns x = 5. The
           payload binder `x` is in scope for the guard condition (through the `(guard …)` wrapper). Was a
           spurious CDZ0101 'unbound name x' before guarded sum-match support landed.")
  (input
    (do
      (def
        (f (: o (Option Int64)))
        (match o ((guard (Some x) (> x 0)) x) ((Some y) (- 0 y)) ((None) 0)))
      (def (main (: n Int64)) (f (Some n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a guarded variant arm falls through when the guard fails"
  (doc
    "The fall-through face of the same program: for `(Some -3)` the guard `x > 0` is false, so the
           guarded `(Some x)` arm does NOT fire and the match falls through to the plain `(Some y)` arm,
           which negates: `-(-3)` = 3. Pins that a guarded VARIANT arm falls through to a LATER arm of the
           same variant exactly as a bare-binder guard falls through — the per-variant fall-through the
           decision tree threads.")
  (input
    (do
      (def
        (f (: o (Option Int64)))
        (match o ((guard (Some x) (> x 0)) x) ((Some y) (- 0 y)) ((None) 0)))
      (def (main (: n Int64)) (f (Some n)))
      (export main)))
  (call main (: -3 Int64))
  (output (: 3 Int64)))

(case
  "a variant guard reads the payload binder AND an enclosing binding"
  (doc
    "A guard over a variant reads both its payload binder and the enclosing scope: `pick` guards
           `(Some v) if v < limit` where `v` is the payload binder and `limit` is a FUNCTION PARAMETER.
           When the payload is below the dynamic threshold the arm returns it; otherwise it falls through
           to the plain `(Some y)` arm (0). For `(Some 3)` with limit 5 the guard `3 < 5` holds → 3; for
           `(Some 9)` it fails → 0. The variant-guard companion of the scalar enclosing-scope case: a
           guarded variant arm's condition closes over the enclosing bindings, not only the payload it
           binds. Both operands runtime, so nothing folds.")
  (input
    (do
      (def
        (pick (: o (Option Int64)) (: limit Int64))
        (match o ((guard (Some v) (< v limit)) v) ((Some y) 0) ((None) -1)))
      (def (main (: n Int64) (: limit Int64)) (pick (Some n) limit))
      (export main)))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: 3 Int64))
  (call main (: 9 Int64) (: 5 Int64))
  (output (: 0 Int64)))

(case
  "chained guards of the same variant are tried in order"
  (doc
    "Two guarded `Some` arms then a plain `(Some z)`: `(guard (Some x) (> x 10))`, `(guard (Some y)
           (> y 0))`, `(Some z)`. Each guard is tried top-to-bottom, falling through on failure. For
           `(Some 5)` the first guard `5 > 10` fails and the second `5 > 0` holds, so the result is 1.
           Pins that multiple guarded arms of the SAME variant chain their fall-through correctly.")
  (input
    (do
      (def
        (f (: o (Option Int64)))
        (match
          o
          ((guard (Some x) (> x 10)) 100)
          ((guard (Some y) (> y 0)) 1)
          ((Some z) 0)
          ((None) -1)))
      (def (main (: n Int64)) (f (Some n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a match whose only variant arm is guarded is non-exhaustive"
  (doc
    "A guarded VARIANT arm covers no value unconditionally, so `(match o ((guard (Some x) (> x 0))
           x) ((None) 0))` — whose only `Some` arm is guarded, with no unguarded `Some` fall-through —
           leaves `Some` uncovered and is non-exhaustive: the compiler MUST reject it (CDZ0210), exactly
           as a guarded scalar arm is excluded from coverage. Pins that a guarded variant arm does not
           satisfy exhaustiveness for its variant.")
  (input
    (do
      (def (f (: o (Option Int64))) (match o ((guard (Some x) (> x 0)) x) ((None) 0)))
      (def (main (: n Int64)) (f (Some n)))
      (export main)))
  (error CDZ0210))

; The rustc-gold non-exhaustive diagnostic: a plain missing-arm sum match is CDZ0210, NAMES the uncovered
; variant(s), and carries an add-arms INSERT fix that appends a covering arm per missing variant (a `trap`
; placeholder body, so the synthesized arm type-checks against the sibling arms and is a heuristic — not
; verified). A nullary missing variant → `(V (trap "TODO: V"))`; multiple missing → all arms space-joined.
; (migrated from rcdzc a_non_exhaustive_sum_match_is_rejected /
; a_non_exhaustive_sum_match_names_the_missing_variants_and_offers_an_add_arms_fix /
; a_non_exhaustive_match_synthesizes_payload_binders_and_lists_multiple_missing.)
(case
  "a non-exhaustive sum match names the uncovered variant and offers an add-arms fix"
  (input
    (do
      (type Option (Some Int64) None)
      (def (f (: s Int64)) (match (Option.Some s) ((Option.Some x) x)))
      (export f)))
  (error
    CDZ0210
    (message "`None`")
    (message "not covered")
    (fix (kind insert-into) (replacement "(None (trap \"TODO: None\"))"))))

(case
  "a non-exhaustive match lists MULTIPLE missing variants and appends an arm for each"
  (input (do (type T (A Int64) B C) (def (f (: t T)) (match t ((A x) x))) (export f)))
  (error
    CDZ0210
    (message "`B`")
    (message "`C`")
    (fix (kind insert-into) (replacement "(B (trap \"TODO: B\")) (C (trap \"TODO: C\"))"))))

; A BOOL scrutinee is a FINITE two-value gap, so a non-exhaustive Bool match names the missing LITERAL
; (`false`/`true`) and inserts exactly that arm — `(false (trap "TODO: false"))`, not a generic wildcard —
; the same precision as a missing sum variant above. The `trap` body inhabits any type, so the completed
; match type-checks against a sibling arm's non-Unit result (a `unit` body would clash). Symmetric on which
; literal is missing. (Migrated from rcdzc a_bool_match_missing_a_literal_offers_the_specific_missing_arm.)
(case
  "a non-exhaustive Bool match names the missing literal and inserts exactly that arm"
  (input (do (def (f (: b Bool)) (match b (true 1))) (export f)))
  (error
    CDZ0210
    (message "`false`")
    (message "not covered")
    (fix (kind insert-into) (replacement "(false (trap \"TODO: false\"))"))))

(case
  "the non-exhaustive Bool match arm-insert is symmetric on the missing literal"
  (input (do (def (f (: b Bool)) (match b (false 2))) (export f)))
  (error
    CDZ0210
    (message "`true`")
    (message "not covered")
    (fix (kind insert-into) (replacement "(true (trap \"TODO: true\"))"))))

(case
  "a false variant guard shields its arm's trapping body"
  (doc
    "A guarded arm's body runs only when the guard holds (core-semantics.md #Boolean Connectives
           Short-Circuit, applied to a guard): `(Some x) if x > 0` over `(Some 0)` must NOT evaluate its
           body `(/ 10 x)` — the guard `0 > 0` is false, so the arm is skipped and the match falls through
           to `(Some y) -1`. The division by the zero payload never happens. A generation that folds a
           guarded body regardless of its guard raises a spurious compile-time divide-by-zero (CDZ0304)
           for an arm that never runs; the fold must evaluate the guard FIRST and skip the body when it is
           false. The variant-guard sibling of the scalar shielding cases above.")
  (input (match (Some 0) ((guard (Some x) (> x 0)) (/ 10 x)) ((Some y) -1) ((None) -2)))
  (output (: -1 Int64)))

; The stronger sibling of the shielded-body case above: there, the guard's VALUE is false and it shields
; the BODY. Here the invariant is that a guard is NOT EVALUATED AT ALL when its own PATTERN does not match
; — the runtime order is pattern-test → (only if it matches) guard-eval → (only if the guard holds) body.
; The witness makes the guard itself TRAPPING (`(/ 100 d)` with a RUNTIME `d`=0, so it cannot const-fold):
; on an arm whose literal pattern MISMATCHES, that trapping guard must never run. `classify` arm 1 is
; `(guard 0 (> (/ 100 d) 0))`; with n=7 the literal-`0` pattern mismatches, so the div-by-zero guard is
; NEVER evaluated and control reaches arm 2 `(guard x (> x 5))` → 2. (Companion runtime behavior, not
; encodable as a single value here: with n=0 the literal-`0` arm MATCHES, so the guard DOES run and
; `100/0` TRAPS — confirming the guard runs exactly when its pattern matches.) A guard-hoisting optimizer
; that evaluated arm 1's guard before testing its pattern would wrongly trap here. Pins the
; pattern-gates-guard evaluation order.
(case
  "a trapping guard on a non-matching literal arm is never evaluated"
  (doc
    "See the comment above. `classify n 0` with n=7: arm 1 `(guard 0 (> (/ 100 d) 0))` has a literal
           `0` pattern that MISMATCHES 7, so its guard — which would divide by the runtime `d`=0 and trap —
           is NOT evaluated; control falls to arm 2 `(guard x (> x 5))`, and 7 > 5 → 2. Runtime `d` so the
           guard cannot const-fold. Pins that a guard is evaluated only when its pattern matches (the
           pattern-test gates the guard), so a would-trap guard on a skipped arm is harmless. Expected: 2.")
  (input
    (do
      (def
        (classify (: n Int64) (: d Int64))
        (match n ((guard 0 (> (/ 100 d) 0)) 1) ((guard x (> x 5)) 2) (_ 3)))
      (def (main (: n Int64) (: d Int64)) (classify n d))
      (export main)))
  (call main (: 7 Int64) (: 0 Int64))
  (output (: 2 Int64)))

; --- Finding #46: a guard's bare binder resolves over a COMPUTED scrutinee in a non-entry helper.
; The guarded-scalar desugar extracts the guard cond+body into a bare `(if <cond> <body> …)`, which
; severed the named wildcard binder `w` from its `(guard)` ancestor: when the scrutinee is a computed
; expression (needs a temp) inside a NON-entry fn, the arm reduced to an orphan `if` where `w` could
; not recover the scrutinee → a spurious CDZ0101 'unbound w' (both targets). The fix wraps the arm in
; `(let ((w <scrutinee>)) …)` + forget_subtree so `w` binds the let and outer names re-resolve (fixed
; v-inference 0f79b082f). The sibling of the guarded-sum CDZ0101 at :2357 — same false-unbound class,
; scalar-guard face. Trigger needs BOTH a non-entry helper AND a non-bare-param scrutinee; the raw-param
; control below always compiled.
(case
  "a guard binder resolves over a COMPUTED scrutinee inside a helper fn"
  (doc
    "Finding #46 regression witness (breaker; fix v-inference 0f79b082f). `classify` is a NON-entry
           helper matching a COMPUTED scrutinee `(* q 1)` with a bare-binder guard `(guard w (> w 10))`.
           Was a spurious CDZ0101 'unbound w' on both targets — the guarded-scalar desugar orphaned the
           `w` binder from its scrutinee when the scrutinee needed a temp in a non-entry frame. Now binds
           and runs: at q=15 the guard 15>10 holds → 1; at q=5 it fails → 0. The computed-scrutinee /
           scalar-guard sibling of the guarded-variant CDZ0101 at :2357.")
  (input
    (do
      (def (classify (: q Int64)) (match (* q 1) ((guard w (> w 10)) 1) (_ 0)))
      (def (main (: x Int64)) (classify x))
      (export main)))
  (call main (: 15 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a guard binder resolves over a raw param scrutinee inside a helper fn"
  (doc
    "Finding #46 control (green even pre-fix): the same bare-binder guard in the same non-entry
           helper, but matching the RAW param `q` directly rather than a computed expression. A bare-param
           scrutinee needs no temp, so the guard binder never orphaned — this face always compiled. Pins
           that the fix's trigger was specifically the COMPUTED scrutinee, not the guard-in-helper shape.")
  (input
    (do
      (def (classify (: q Int64)) (match q ((guard w (> w 10)) 1) (_ 0)))
      (def (main (: x Int64)) (classify x))
      (export main)))
  (call main (: 15 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

; --- A match must cover every value of the scrutinee's type ------------------------------
; core-semantics.md #Matching Is Exhaustive Or Rejected: "A match whose patterns do not cover
; every value of the scrutinee's type MUST be a compile-time error." A Bool has exactly two
; values, true and false, so a match on a Bool that arms only ONE of them (and has no wildcard)
; is non-exhaustive and the compiler MUST reject it (CDZ0210) — even though the missing case would
; only be reached for one of the two inputs. The rejection is the recorded outcome; the program
; does not run. A generation that does not yet check runtime-bool exhaustiveness declines rather
; than emitting a component (reject-don't-miscompile).
(case
  "a bool match missing the false arm is non-exhaustive"
  (doc
    "The scrutinee `b` is a Bool — its type has exactly two values. A match arming only `true`
           leaves `false` uncovered and has no wildcard, so it is non-exhaustive and the compiler MUST
           reject it (CDZ0210, coded-span-record.md). The rejection is the recorded outcome; the
           program does not run. Pins runtime-bool exhaustiveness against a match whose scrutinee is a
           function parameter, not a compile-time constant.")
  (input (do (def (f b) (match b (true 1))) (def (main) (f false)) (export main)))
  (error CDZ0210))

(case
  "a bool match missing the true arm is non-exhaustive"
  (doc
    "The mirror of the case above: a match on a Bool arming only `false` leaves `true`
           uncovered and the compiler MUST reject it as non-exhaustive (CDZ0210). Pins that
           exhaustiveness is checked for BOTH bool values, not only the one the sole arm happens to
           name.")
  (input (do (def (f b) (match b (false 0))) (def (main) (f true)) (export main)))
  (error CDZ0210))

(case
  "a bool match on a constant scrutinee is non-exhaustive even when the constant hits the sole arm"
  (doc
    "`(match true (true 1))` — the scrutinee is the COMPILE-TIME CONSTANT `true`, and the sole arm
           `true` is exactly the value it holds. Exhaustiveness is still checked against the TYPE's value
           set (both `true` and `false`), not against which value the constant scrutinee happens to be:
           the arm set leaves `false` uncovered and there is no wildcard, so the match is non-exhaustive
           and the compiler MUST reject it (CDZ0210). This is the constant-scrutinee, present-arm form —
           distinct from the parameter-scrutinee cases above (a dynamic scrutinee) and the companion of
           the constant-sum present-arm case below: a static-scrutinee compile path that returns the arm
           the constant matches must NOT skip the arm-set-vs-type exhaustiveness check just because the
           constant hit a present arm. Exhaustiveness is a property of the arm set against the type, not
           of the scrutinee's value.")
  (input (match true (true 1)))
  (error CDZ0210))

; A sum type's value set is its variant set, so exhaustiveness for a sum match is checked against
; ALL its variants — not just the scrutinee's runtime value. `Option` has variants Some and None;
; a match arming only `Some` leaves `None` uncovered, so it is non-exhaustive and the compiler MUST
; reject it (CDZ0210) EVEN when the scrutinee happens to be a `Some`. Exhaustiveness is a
; compile-time property of the arm set against the sum's variant set, not of which variant the
; scrutinee holds. The bool cases above are the two-value instance of the same rule; these are the
; general sum instance.
(case
  "a sum match missing a variant is non-exhaustive even when the scrutinee is the covered one"
  (doc
    "`Option` has variants Some and None. `(match (Some 5) ((Some x) x))` arms only Some, leaving
           None uncovered and having no wildcard — non-exhaustive, so the compiler MUST reject it
           (CDZ0210), independent of the scrutinee being a Some. Exhaustiveness is a compile-time
           property of the arm set against the sum's variant set, not of which variant the scrutinee
           holds.")
  (input (match (Some 5) ((Some x) x)))
  (error CDZ0210))

(case
  "a Sign match missing two of three variants is non-exhaustive"
  (doc
    "Sign has three variants (Neg | Zero | Pos). `(match (Sign.Pos unit) ((Sign.Pos _) 1))`
           arms only Pos, leaving Neg and Zero uncovered — non-exhaustive, so the compiler MUST reject
           it (CDZ0210). Pins that a sum's exhaustiveness covers every declared variant, not only the
           one the constant scrutinee names — a three-variant sum with a single arm is rejected just
           as a two-variant one is.")
  (input (match (Sign.Pos unit) ((Sign.Pos _) 1)))
  (error CDZ0210))

; An Int64's value set is all 2^64 of its values, so no finite set of literal arms covers it — a match
; on an Int64 with only literal arms and no wildcard is non-exhaustive exactly as a Bool match missing an
; arm or a sum match missing a variant is. The rule is the same one the bool and sum cases above pin
; (core-semantics.md #Matching Is Exhaustive Or Rejected: "cover every value of the scrutinee's type"),
; applied to the third scrutinee kind: exhaustiveness is a property of the ARM SET against the TYPE, not
; of the scrutinee's value. In particular a COMPILE-TIME CONSTANT Int64 scrutinee that hits a present arm
; does NOT excuse the missing coverage: `(match 5 (5 1))` folds the scrutinee to `5` and the sole arm
; names `5`, but the arm set still leaves every other Int64 uncovered and there is no wildcard, so the
; match is non-exhaustive and MUST be rejected (CDZ0210). This is the Int64 companion of the
; constant-scrutinee, present-arm bool case (§"a bool match on a constant scrutinee is non-exhaustive
; even when the constant hits the sole arm") and sum case above: a static-scrutinee compile path that
; returns the arm the constant matches must NOT skip the arm-set-vs-type exhaustiveness check just
; because the constant hit a present arm. The DYNAMIC-scrutinee form — `(match x (5 1))` for a parameter
; `x` — is already rejected; the constant-scrutinee form is the one a value-driven shortcut mis-accepts.
(case
  "an int match on a constant scrutinee is non-exhaustive even when the constant hits the sole arm"
  (doc
    "`(match 5 (5 1))` — the scrutinee is the COMPILE-TIME CONSTANT `5`, and the sole arm `5` is
           exactly the value it holds. Exhaustiveness is still checked against the TYPE's value set (all
           2^64 Int64 values), not against which value the constant scrutinee happens to be: a finite set
           of literal arms cannot cover Int64, and there is no wildcard, so the match is non-exhaustive
           and the compiler MUST reject it (CDZ0210). This is the Int64 companion of the constant-scrutinee
           present-arm bool case and sum case above (core-semantics.md #Matching Is Exhaustive Or
           Rejected). A static-scrutinee compile path that returns the arm the constant matches must NOT
           skip the arm-set-vs-type exhaustiveness check just because the constant hit a present arm — the
           same value-driven shortcut the bool path had before it was fixed. The dynamic-scrutinee form
           `(match x (5 1))` for a parameter `x` is already rejected; the constant-scrutinee present-arm
           form is the one this pins. A generation that does not yet check int-literal exhaustiveness on a
           constant scrutinee declines rather than emitting a component (reject-don't-miscompile).")
  (input (match 5 (5 1)))
  (error CDZ0210))

(case
  "a non-exhaustive scalar match names the wildcard gap and offers an add-wildcard-arm fix"
  (doc
    "An open Int scalar match with literal arms and no wildcard — `(match n (0 1) (1 2))` over a
           parameter `n` — is non-exhaustive (a finite literal set cannot cover Int64), CDZ0210, and the
           message names the missing `wildcard`. It carries an INSERT fix appending a `(_ (trap \"TODO\"))`
           wildcard arm — the scalar twin of the sum add-arms fix. The body is a `trap` (∀a. String → a),
           NOT `unit`, so the added arm type-checks against sibling arms of ANY result type in ONE shot (a
           bare `unit` would cascade to a CDZ0203 'match arms differ'). Heuristic (placeholder body →
           unverified). (Migrated from rcdzc a_non_exhaustive_scalar_match_offers_a_wildcard_arm_fix.)")
  (input (do (def (f (: n Int64)) (match n (0 1) (1 2))) (export f)))
  (error
    CDZ0210
    (message "wildcard")
    (fix (kind insert-into) (replacement "(_ (trap \"TODO\"))") (unverified))))

; Exhaustiveness composes into NESTED patterns, not only the top-level variant set. core-semantics.md
; #Patterns Compose (a constructor pattern's binder MAY itself be a constructor pattern, matched
; recursively) with #Matching Is Exhaustive Or Rejected ("cover every value of the scrutinee's type"):
; a value of type `Option (Option Int64)` ranges over `(Some (Some _))`, `(Some (None _))`, and `(None
; _)`, so a match arming `(Some (Some x))` and `(None _)` — but NOT `(Some (None _))` — leaves a value of
; the type uncovered and is non-exhaustive (CDZ0210), exactly as a flat match missing a top-level variant
; is. The check must descend into the nested constructor position: the OUTER `Some` is covered, but its
; payload's own variant set (`Some | None`) is not, so the composed arm set does not cover the type. A
; compiler that checks exhaustiveness only at the top level (outer `Some`/`None` both named) accepts the
; ill-typed program; worse, one that checks against the CONSTANT scrutinee's nested shape rather than the
; TYPE accepts `(match (Some (Some 5)) …)` because the constant hits `(Some (Some x))` — the same
; value-driven shortcut the constant-scrutinee cases above pin, here at the nested level. (The dynamic
; form and the constant-is-the-uncovered-case form are already rejected — `(match (Some (None unit))
; ((Some (Some x)) x) ((None _) -1))` declines; the constant-hits-a-covered-arm form is the one this pins.)
; A generation that does not yet check nested exhaustiveness declines rather than emitting a component.
(case
  "a nested sum match missing an inner variant is non-exhaustive"
  (doc
    "`(match (Some (Some 5)) ((Some (Some x)) x) ((None _) -1))` arms the outer `Some` (with an inner
           `Some`) and the outer `None`, but leaves `(Some (None _))` uncovered — a value of the scrutinee
           type `Option (Option Int64)` that no arm matches and no wildcard catches, so the match is
           non-exhaustive and MUST be rejected (CDZ0210, core-semantics.md #Matching Is Exhaustive Or
           Rejected with #Patterns Compose: exhaustiveness composes into the nested constructor position).
           Pins that the exhaustiveness check descends into a nested pattern — the outer `Some` is covered,
           but its payload's variant set `Some | None` is not, so the composed arm set does not cover the
           type. This is the nested companion of the flat sum-missing-a-variant case above; a compiler that
           checks only the top-level variant set, or checks against the constant scrutinee's nested shape
           (`(Some (Some 5))` hits `(Some (Some x))`) rather than the type, accepts the ill-typed program.
           A generation that does not yet check nested exhaustiveness declines rather than emitting.")
  (input (match (Some (Some 5)) ((Some (Some x)) x) ((None _) -1)))
  (error CDZ0210))

(case
  "nested patterns deconstruct recursively"
  (doc
    "Witnesses core-semantics.md #Pattern Matching: patterns can nest — a constructor pattern
           inside another constructor pattern. (Some (tuple a b)) matches a Some whose payload is a
           tuple, binding both elements. The compiler uses this to deconstruct nested AST structures.")
  (input (match (Some #tuple(3 7)) ((Some #tuple(a b)) (+ a b)) ((None _) 0)))
  (output (: 10 Int64)))

(case
  "nested patterns with literals"
  (doc
    "Witnesses core-semantics.md #Pattern Matching: nested patterns can combine constructors
           and literals. (Some 0) matches Some carrying exactly 0 — the literal refines the match.")
  (input (match (Some 0) ((Some 0) "zero") ((Some _) "nonzero") ((None _) "none")))
  (output (: "zero" String)))

(case
  "a literal inside a constructor pattern matches a runtime payload"
  (doc
    "core-semantics.md #Pattern Matching + #Matching Is Exhaustive Or Rejected: a literal nested
           inside a constructor pattern must be tested against the payload's RUNTIME value, exactly as
           a top-level literal pattern is. Here the payload `n` is a function parameter (not known at
           compile time); `(Some n)` with n=0 must match `(Some 0)` and yield 100, not fall through to
           the binding arm `(Some k)`. Companion to \"nested patterns with literals\" above, whose
           scrutinee `(Some 0)` is a compile-time constant — this one pins the same refinement when the
           payload is only known at run time. The `((None _) …)` arm is present because exhaustiveness
           is against the TYPE's variant set, not the scrutinee's known variant (the sibling case \"a sum
           match missing a variant is non-exhaustive even when the scrutinee is the covered one\").")
  (input
    (do
      (def (f n) (match (Some n) ((Some 0) 100) ((Some k) k) ((None _) -1)))
      (def (main) (f 0))
      (export main)))
  (output (: 100 Int64)))

(case
  "a non-matching literal inside a constructor pattern binds the runtime payload"
  (doc
    "The companion of the case above: with n=7 the literal arm `(Some 0)` does not match, so the
           binding arm `(Some k)` binds k=7 and yields 7. Confirms the nested literal is a genuine
           runtime test (matching for 0, falling through otherwise) rather than always-taken or
           always-skipped. The `((None _) …)` arm keeps the match exhaustive against `Option`'s variant
           set (see the case above).")
  (input
    (do
      (def (f n) (match (Some n) ((Some 0) 100) ((Some k) k) ((None _) -1)))
      (def (main) (f 7))
      (export main)))
  (output (: 7 Int64)))

(case
  "a boolean literal inside a constructor pattern refines the match"
  (doc
    "The bool-payload companion: a variant carrying a `Bool` payload can be matched against a
           boolean LITERAL. `(F.S true)` matches `F.S` carrying exactly `true`; `(F.S k)` binds otherwise
           (core-semantics.md #Pattern Matching, the literal refines the match). For a runtime `b=true`
           the `(F.S true)` arm fires → 1. Pins that a literal payload test works for a Bool payload, not
           only Int — the get-bool + i32 compare sibling of the Int literal test.")
  (input
    (do
      (type F (S Bool) C)
      (def (f b) (match (F.S b) ((F.S true) 1) ((F.S k) 0) ((F.C _) -1)))
      (def (main) (f true))
      (export main)))
  (output (: 1 Int64)))

(case
  "a literal inside an Ok pattern refines a Result match"
  (doc
    "The Result companion: `(Ok 0)` matches `Ok` carrying exactly `0`, `(Ok k)` binds otherwise,
           `(Err e)` covers the error variant. For a runtime `n=3` the literal arm `(Ok 0)` does not
           match, so `(Ok k)` binds k=3 → 3. Pins that a literal payload test composes with the
           two-variant Result sum exactly as with Option.")
  (input
    (do
      (def (f n) (match (Ok n) ((Ok 0) 100) ((Ok k) k) ((Err e) -1)))
      (def (main) (f 3))
      (export main)))
  (output (: 3 Int64)))

(case
  "a literal inside a NESTED constructor pattern refines the match"
  (doc
    "The nested-literal companion: `(Some (Some 0))` tests the INNER payload against the literal
           `0`. `(Some (Some 0))` fires only when the doubly-wrapped value is exactly 0; `(Some (Some x))`
           binds otherwise. For a runtime n=7 the literal arm does not match, so the binder arm yields 7.
           Pins that a literal test at a DEEP payload path (`[Payload, Payload]`) works — the literal
           refinement composes with the decision tree's nested descent.")
  (input
    (do
      (def
        (f n)
        (match
          (Some (Some n))
          ((Some (Some 0)) 99)
          ((Some (Some x)) x)
          ((Some (None _)) -1)
          ((None _) -2)))
      (def (main) (f 7))
      (export main)))
  (output (: 7 Int64)))

(case
  "a literal inside a tuple pattern matches a runtime element"
  (doc
    "core-semantics.md #Pattern Matching: the same refinement inside a tuple pattern. `(tuple n
           9)` with a runtime n; the arm `(tuple 0 y)` matches only when the first element is 0. With
           n=0 it matches and yields 100; the literal element is tested against the runtime value, not
           treated as a binder.")
  (input
    (do
      (def (f n) (match #tuple(n 9) (#tuple(0 y) 100) (#tuple(x y) x)))
      (def (main) (f 0))
      (export main)))
  (output (: 100 Int64)))

; --- A tuple pattern's arity must match the scrutinee's tuple arity ----------------------
; core-semantics.md #A Tuple Is Deconstructible By Pattern Matching (`(tuple a b)` binds the
; elements): a tuple pattern deconstructs a tuple of the SAME arity. A pattern `(tuple a b c)` has a
; three-element tuple shape, which can NEVER match a two-element tuple scrutinee — the pattern and
; scrutinee shapes are statically incompatible, a type error (CDZ0201), exactly as a `(Some x)`
; pattern against an Int64 scrutinee is. A wrong-arity tuple pattern is ill-typed, not a runtime
; non-match: the compiler rejects it, and a generation that does not yet check a tuple pattern's
; arity against the scrutinee's declines rather than running the program (reject-don't-miscompile).
(case
  "a tuple pattern of the wrong arity is a type error"
  (doc
    "`(tuple a b c)` is a three-element tuple pattern; the scrutinee `(tuple 1 2)` is a
           two-tuple. A three-element pattern can never match a two-element tuple — their shapes are
           statically incompatible, so the arm is ill-typed and the compiler MUST reject the match
           (CDZ0201). Pins that a tuple pattern's arity is checked against the scrutinee's, not
           silently failed.")
  (input (match #tuple(1 2) (#tuple(a b c) a) (_ 0)))
  (error CDZ0201))

(case
  "a one-element tuple pattern against a two-tuple is a type error"
  (doc
    "The other direction: `(tuple a)` is a one-element tuple pattern, which cannot match the
           two-tuple `(tuple 1 2)` — a static shape mismatch, CDZ0201. Pins that BOTH too-many and
           too-few pattern elements are a type error, not a runtime non-match.")
  (input (match #tuple(1 2) (#tuple(a) a) (_ 0)))
  (error CDZ0201))

; The tuple-pattern-arity rule applies RECURSIVELY, at every nesting depth, not only to the outermost
; tuple pattern. core-semantics.md #Patterns Compose: a tuple pattern MUST admit any pattern in each of
; its binder positions — its element "MAY itself be … a tuple pattern … matched recursively to any depth." So a
; nested `(tuple b c d)` at a position whose scrutinee element is a two-tuple `(tuple 2 3)` is the same
; wrong-arity shape mismatch the top-level cases above pin — a three-element tuple pattern can never
; match a two-element tuple — and MUST be rejected CDZ0201, not silently fail and fall through to a
; wildcard. A compiler that checks only the OUTERMOST tuple pattern's arity against the scrutinee's
; (and not the arity of each nested tuple pattern against the corresponding nested scrutinee element)
; lets the ill-typed nested arm slip past: `(match (tuple 1 (tuple 2 3)) ((tuple a (tuple b c d)) 9)
; (_ 0))` runs to 0 (the arm silently not-matching) where it MUST reject, exactly the "silent non-match"
; the flat cases forbid. A generation that does not yet check nested pattern arity declines rather than
; running the program (reject-don't-miscompile).
(case
  "a nested tuple pattern of the wrong arity is a type error"
  (doc
    "`(tuple a (tuple b c d))` is a tuple pattern whose second element is a three-element tuple
           pattern; matched against `(tuple 1 (tuple 2 3))`, that nested pattern faces a two-element
           tuple — a static shape mismatch, CDZ0201, exactly as the flat `(tuple a b c)` vs `(tuple 1 2)`
           case above. The arity rule composes recursively (core-semantics.md #Patterns Compose — a tuple pattern's element MAY itself be a tuple
           pattern, matched to any depth), so the nested arm is ill-typed and MUST be rejected, not
           silently fail and fall through to the wildcard yielding 0. Pins that a compiler checking only
           the OUTERMOST tuple pattern's arity does not let a nested wrong-arity pattern slip past as a
           runtime non-match.")
  (input (match #tuple(1 #tuple(2 3)) (#tuple(a #tuple(b c d)) 9) (_ 0)))
  (error CDZ0201))

; The CONSTRUCTOR twin of the tuple-pattern-shape rule: a user-sum constructor pattern must bind exactly the
; ctor's field arity — an over-arity `(Mk a b c)` on a 2-field `Mk`, or several binders on a single-value
; carrier, is the same static shape mismatch (CDZ0201), and the message NAMES the constructor + counts its
; FIELDS (not the internal "payload" term the compiler once leaked). Migrated from rcdzc
; a_multi_payload_pattern_of_wrong_arity_is_rejected.
(case
  "a constructor pattern of the wrong arity is a type error naming the field count"
  (doc
    "`(Pair.Mk a b c)` binds THREE elements against a two-field `Mk` — a nonexistent third element, a
           static shape mismatch CDZ0201 (never bind `c` past the field tuple → a wrong value / invalid wasm).")
  (input
    (do
      (type Pair (Mk Int64 Int64))
      (def (main (: n Int64)) (match (Pair.Mk n n) ((Pair.Mk a b c) (+ a b))))
      (export main)))
  (error CDZ0201 (message "this pattern binds 3 elements for `Mk`, but `Mk` carries 2 fields")))

(case
  "a multi-binder pattern on a single-value constructor points at the one-sub-pattern form"
  (doc
    "A single-value-carrier variant `(Mk Int64)` matched with SEVERAL binders `(Mk x y)` is the same
           shape mismatch; the message points at the one-sub-pattern form `(Mk x)`.")
  (input
    (do (type P (Mk Int64) (Other)) (def (f (: p P)) (match p ((Mk x y) x) ((Other) 0))) (export f)))
  (error
    CDZ0201
    (message "`Mk` carries a single value of type Int64 — bind it with one sub-pattern `(Mk x)`")))

; The recursion covers a nested LITERAL pattern's type too, not only a nested tuple's arity. A literal
; pattern matches by equality, defined only WITHIN one type (core-semantics.md #Equality Is Structural),
; so a literal pattern whose type differs from the value at its position can never match — CDZ0201 at the
; top level (§"a literal pattern's type must match the scrutinee's"), and the same at every nested binder
; position (core-semantics.md #Patterns Compose — a tuple pattern's element MAY itself be a literal pattern,
; checked recursively). `(tuple true b)` puts a Bool literal `true` at position 0, whose scrutinee element
; is the Int64 `1`; the arm is ill-typed and MUST be rejected, not silently fail to the wildcard yielding 0.
(case
  "a nested literal pattern of the wrong type is a type error"
  (doc
    "`(tuple true b)` matched against `(tuple 1 2)` puts the Bool literal `true` at a position whose
           scrutinee element is the Int64 `1` — a literal-pattern-type mismatch (core-semantics.md #Equality
           Is Structural: equality is within one type), CDZ0201, exactly as the top-level `(match 5 (true 1)
           …)` case is rejected. The rule composes to nested binder positions (core-semantics.md #Patterns
           Compose), so the nested literal type is checked against the corresponding scrutinee element, not
           only the outermost. Pins that a compiler checking only the top-level literal pattern's type does
           not let a nested wrong-type literal slip past as a runtime non-match falling to the wildcard.")
  (input (match #tuple(1 2) (#tuple(true b) 9) (_ 0)))
  (error CDZ0201))

; The recursion must also enter a tuple pattern nested UNDER A CONSTRUCTOR pattern, not only one at the
; arm's root. A constructor pattern's binder MAY itself be a tuple pattern (core-semantics.md #Patterns
; Compose), so `(Some (tuple a b c))` carries a three-element tuple pattern in `Some`'s payload position.
; Matched against `(Some (tuple 1 2))`, whose payload is a two-element tuple, that nested tuple pattern is
; the same wrong-arity shape mismatch the flat and tuple-nested cases pin — CDZ0201 — reached through the
; constructor's binder rather than a tuple element. A compiler whose shape check descends only through
; tuple patterns (entering only when the arm's pattern is a `(tuple …)` at the root) never reaches a tuple
; pattern sitting under a `Some`/`Ok`/user constructor, and lets the ill-typed arm slip past to a wildcard.
(case
  "a wrong-arity tuple pattern nested under a constructor pattern is a type error"
  (doc
    "`(Some (tuple a b c))` carries a three-element tuple pattern in `Some`'s payload binder; matched
           against `(Some (tuple 1 2))`, whose payload is a two-element tuple, the nested pattern faces a
           two-tuple — a static arity mismatch (CDZ0201), the same rule as the tuple-nested and flat cases,
           reached through a constructor's binder (core-semantics.md #Patterns Compose — a constructor
           pattern's binder MAY itself be a tuple pattern, matched to any depth). Pins that the recursive
           shape check enters a tuple pattern nested under a constructor pattern, not only one at the arm's
           root, so the ill-typed arm is rejected rather than silently failing to the wildcard yielding 0.")
  (input (match (Some #tuple(1 2)) ((Some #tuple(a b c)) 9) (_ 0)))
  (error CDZ0201))

; A pattern's KIND must also match the scrutinee's kind, not only a tuple's arity: a tuple pattern
; against a SUM scrutinee (or a sum/constructor pattern against a tuple) is a static shape mismatch.
; A `(tuple a b)` pattern deconstructs a tuple; a `Some`/`Ok`/`Sign.Pos` value is a sum, so the tuple
; pattern can never match it — CDZ0201, the same shape-mismatch class as a wrong-arity tuple pattern
; or a type-mismatched literal pattern above. (A literal pattern vs a sum/tuple scrutinee, and a
; constructor pattern vs a tuple/scalar scrutinee, are already rejected; this pins the tuple-pattern-
; vs-sum-scrutinee direction.)
(case
  "a tuple pattern against a sum scrutinee is a type error"
  (doc
    "`(tuple a b)` is a tuple pattern; the scrutinee `(Some 5)` is a sum value. A tuple pattern
           deconstructs a tuple, so it can never match a sum — the arm's shape is statically
           incompatible with the scrutinee, a type error (CDZ0201). Pins the pattern-KIND check
           (tuple vs sum), the companion of the tuple-ARITY check above.")
  (input (match (Some 5) (#tuple(a b) a) (_ 0)))
  (error CDZ0201))

(case
  "a tuple pattern against a Sign scrutinee is a type error"
  (doc
    "The companion with a user-facing sum: `(Sign.Pos unit)` is a sum value, so a `(tuple a b)`
           pattern against it is a shape mismatch (CDZ0201). Pins that the tuple-pattern-vs-sum check
           holds for every sum, not only Option.")
  (input (match (Sign.Pos unit) (#tuple(a b) a) (_ 0)))
  (error CDZ0201))

(case
  "deeply nested pattern matching"
  (doc
    "The compiler pattern-matches over nested AST: a list node containing a name node.
           Patterns nest arbitrarily deep.")
  (input
    (do
      (type Expr (Lit Int64) (Add (Tuple Expr Expr)))
      (let
        ((e (Expr.Add #tuple((Expr.Lit 1) (Expr.Lit 2)))))
        (match
          e
          ((Expr.Lit n) n)
          ((Expr.Add #tuple((Expr.Lit a) (Expr.Lit b))) (+ a b))
          ((Expr.Add _) 0)))))
  (output (: 3 Int64)))

; --- Matching a RUNTIME scrutinee ---------------------------------------------------
; Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected for scrutinees whose
; value is NOT known at compile time — a function parameter or a computed expression. The
; matching arm must be selected from the scrutinee's RUNTIME value, exactly as when the
; scrutinee is an inline literal (cases above). These are core (functions + match are core):
; the compiler that dispatches instruction opcodes matches on runtime-computed byte values.
(case
  "an integer literal pattern matches a runtime scrutinee"
  (doc
    "The scrutinee `n` is a function parameter — its value (0) is not known until run
           time. The first arm's literal pattern 0 must match the runtime value 0 and select
           its body, exactly as it would for an inline literal scrutinee. This is the base-case
           dispatch every recursive function over integers relies on.")
  (input
    (do
      (def (classify n) (match n (0 100) (1 200) (_ 900)))
      (def (main) (classify 0))
      (export main)))
  (output (: 100 Int64)))

(case
  "a two-arm match does not evaluate the unselected arm's trapping body"
  (doc
    "A 2-arm `match` with leaf-value bodies may be lowered to a branchless `select` (both bodies on
           the stack, the discriminant chooses) — but ONLY when both bodies are trap-free. `(match n (0 (/
           1 z)) (_ 99))` has a trapping body `(/ 1 z)` in the first arm, so it MUST keep the branch: with
           n = 5 the wildcard arm is selected → 99, and the first arm's division by zero (z = 0) is NOT
           evaluated. A naive branchless-select that evaluated both bodies would trap here. Pins that the
           2-arm-match-to-select optimization does not treat a trapping arm body as a select leaf — the
           match evaluates only the selected arm (core-semantics.md #Matching Is Exhaustive Or Rejected +
           the trap-observation rule). The anchor: with n = 0 the first arm IS selected and it traps.")
  (input (do (def (main (: n Int64) (: z Int64)) (match n (0 (/ 1 z)) (_ 99))) (export main)))
  (call main (: 5 Int64) (: 0 Int64))
  (output (: 99 Int64)))

(case
  "a runtime scrutinee selects a non-first literal arm"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected: arms are tried top-to-bottom
           and the first whose pattern matches the runtime value wins. Here the runtime value 2
           skips the 0 and 1 arms and selects the 2 arm — not the else.")
  (input
    (do
      (def (classify n) (match n (0 10) (1 20) (2 30) (_ 99)))
      (def (main) (classify 2))
      (export main)))
  (output (: 30 Int64)))

(case
  "a negative integer literal pattern matches a runtime scrutinee"
  (doc
    "A negative literal pattern matches by equality against the runtime value, like any
           other integer literal.")
  (input
    (do (def (classify n) (match n (-1 100) (_ 200))) (def (main) (classify -1)) (export main)))
  (output (: 100 Int64)))

(case
  "a WIDE integer literal pattern (beyond ±2^31) matches a runtime scrutinee by equality"
  (doc
    "The wide-magnitude companion of the negative-literal case above: a `match` arm probes a
           literal whose magnitude EXCEEDS ±2^31, so the emitted `i64.const` it compares against needs a
           MULTI-BYTE sleb128 encoding — the exact place a sign-extension miscompile hides (a truncated /
           wrongly sign-extended constant would compare against the wrong 64-bit value and mis-dispatch).
           `classify` probes `5000000000` and `-5000000000` (both past the i32 range) against a runtime
           `n`. Built from a runtime arg so the match cannot fold: n=5000000000 hits the positive-wide arm
           → 111; n=-5000000000 hits the negative-wide arm → 222; n=0 (and any near-miss like
           5000000001) falls through → 0. Pins that a wide/negative i64 literal in PATTERN position
           encodes its comparison constant with correct sleb128 sign-extension. Expected (n=5000000000):
           111.")
  (input
    (do
      (def (classify (: n Int64)) (match n (5000000000 111) (-5000000000 222) (_ 0)))
      (def (main (: n Int64)) (classify n))
      (export main)))
  (call main (: 5000000000 Int64))
  (output (: 111 Int64)))

(case
  "a wide NEGATIVE integer literal pattern matches its runtime scrutinee"
  (doc
    "The negative-arm companion of the wide-literal case above, selecting the OTHER wide arm to pin
           the negative multi-byte sleb128 path independently: the same `classify` called with
           `-5000000000` hits the `-5000000000` arm → 222. A sign-extension bug in the negative wide
           `i64.const` would compare against a wrong value and fall through to 0. Expected: 222.")
  (input
    (do
      (def (classify (: n Int64)) (match n (5000000000 111) (-5000000000 222) (_ 0)))
      (def (main (: n Int64)) (classify n))
      (export main)))
  (call main (: -5000000000 Int64))
  (output (: 222 Int64)))

(case
  "an earlier literal arm is chosen over a later name-binding arm for a runtime scrutinee"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected + #Bindings Introduced By A
           Pattern Are Scoped To Its Branch: a bare name pattern `k` matches anything and binds
           the whole scrutinee, but only if reached. With the runtime value 0, the earlier
           literal arm `0` matches first, so the name arm is never entered.")
  (input (do (def (f n) (match n (0 100) (k (+ k 1)))) (def (main) (f 0)) (export main)))
  (output (: 100 Int64)))

(case
  "a name pattern binds the runtime scrutinee when no literal arm matches"
  (doc
    "The companion to the case above: with the runtime value 41 no literal arm matches,
           so the name arm `k` binds k=41 and its body computes 42. Confirms the name arm and
           the literal arm are selected consistently from the same runtime value.")
  (input (do (def (f n) (match n (0 100) (k (+ k 1)))) (def (main) (f 41)) (export main)))
  (output (: 42 Int64)))

(case
  "a name pattern binds a RUNTIME scrutinee read from an exported parameter"
  (doc
    "The binder cases above pass the scrutinee as a compile-time-known argument `(f 41)`; this reads it
           from an EXPORTED annotated parameter so the scrutinee is a genuine runtime value (not const-folded
           away). `(match n (0 100) (k (+ k 1)))` over `(: n Int64)`: n=7 → the name arm binds k=7, body → 8.
           Pins that the bare-name arm reads the parameter's slot at run time.")
  (input (do (def (f (: n Int64)) (match n (0 100) (k (+ k 1)))) (export f)))
  (call f (: 7 Int64))
  (output (: 8 Int64))
  (call f (: 0 Int64))
  (output (: 100 Int64)))

(case
  "a name pattern over a NARROW-width scrutinee normalizes the literal arm to the match's width"
  (doc
    "A bare-name arm beside a bare-LITERAL arm over a NARROW (UInt8) scrutinee. Every arm produces the
           match's result type, so the literal arm (default Int64 on its own) must take the UInt8 result
           width — else a default-i64 arm beside the narrow-i32 binder arm is a mismatched block that wasm
           rejects. This was a MISCOMPILE (invalid component). `(match x (0 100) (n n))`: x=5 → the binder arm
           yields 5, x=0 → the literal arm yields 100.")
  (input (do (def (main (: x UInt8)) (match x (0 100) (n n))) (export main)))
  (call main (: 5 UInt8))
  (output (: 5 UInt8))
  (call main (: 0 UInt8))
  (output (: 100 UInt8)))

(case
  "a narrow-width name binder is usable in a downstream op at its width"
  (doc
    "The bound NARROW value feeds a downstream arithmetic op at the same width: `(match x (0 0) (n (+ n
           x)))` over `(: x UInt8)`. x=50 → the binder `n` = 50, `(+ n x)` = 100 at UInt8. Pins that the
           width-normalized binder is a usable value, not merely a passthrough.")
  (input (do (def (main (: x UInt8)) (match x (0 0) (n (+ n x)))) (export main)))
  (call main (: 50 UInt8))
  (output (: 100 UInt8)))

; A `_`-PREFIXED match-arm binder (`_x`) is a real, USABLE binding — the `_` prefix only SILENCES the
; unused-binding warning (CDZ0306), it does NOT turn the name into a bare `_` wildcard that drops the value.
; So a `(Some _x)` arm whose body REFERENCES `_x` binds the payload and reads it normally. This is the
; match-arm companion of the `let`/param `_x` cases in 01-literals (which bind `_x` in binding/param
; position); pins it in a MATCH ARM specifically. A generation that treated any `_`-leading pattern name as
; an unbindable wildcard would leave `_x` UNBOUND (a spurious CDZ0101) — the fault this guards.
(case
  "an underscore-prefixed match-arm binder is a usable binding, not a wildcard"
  (doc
    "`(match o ((Some _x) _x) ((None) -1))` — the payload binder `_x` is `_`-PREFIXED, which only
           silences the unused-binding warning; it is still a real binding its body can reference. Over
           `(Some 8)` the arm binds `_x` = 8 and returns it → 8. Contrast a bare `_` (a wildcard that binds
           nothing). Pins that the `_` prefix is a warning-silencer, not a wildcard, in match-arm position.
           Expected: 8.")
  (input
    (do
      (def (probe (: o (Option Int64))) (match o ((Some _x) _x) ((None) -1)))
      (def (main (: k Int64)) (probe (Some k)))
      (export main)))
  (call main (: 8 Int64))
  (output (: 8 Int64)))

(case
  "an earlier name-binding arm shadows a later literal arm — first-match-wins, not specificity-ordered"
  (doc
    "The precedence direction the two cases above do NOT cover: an earlier GENERAL (name-binding) arm
           makes a later MORE-SPECIFIC (literal) arm DEAD. `(match n (x (+ x 100)) (5 999))` — the binding
           `x` matches ANY value including 5, and arms are tried top-to-bottom with the FIRST match winning
           (core-semantics.md #Matching Is Exhaustive Or Rejected), so the runtime value 5 takes the FIRST
           arm → 5 + 100 = 105, NOT the later literal-5 arm (999, which is unreachable for EVERY input).
           n = 7 likewise takes the binding arm → 107. This is the witness that Cadenza matches in SOURCE
           ORDER, not by pattern specificity: a specificity-ordered matcher (most-specific arm first) would
           let the literal-5 arm win at n = 5 → 999. The prior cases put the literal FIRST (so the binding
           arm is genuinely reachable); only a binding-arm-BEFORE-a-matching-literal case distinguishes
           source-order from specificity-order.")
  (input (do (def (main (: n Int64)) (match n (x (+ x 100)) (5 999))) (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64))
  (call main (: 7 Int64))
  (output (: 107 Int64)))

(case
  "a match arm an earlier arm already fully covers compiles but earns a CDZ0213 unreachable-arm warning"
  (doc
    "The dead-arm warning of the source-order rule above: `(match n (0 1) (0 2) (_ 3))` repeats the
           literal `0`, so the second `(0 2)` arm can never be reached — the first `(0 0)` arm wins every
           value it would match (first-match-wins, core-semantics.md #Matching Is Exhaustive Or Rejected).
           The program still COMPILES and runs (`(f 0)` = 1, the first arm), but the build surfaces a CDZ0213
           `unreachable` WARNING rather than silently keeping the dead arm — the same code-quality/dead-code
           band as the unused binding (CDZ0306) and dead trap (CDZ0305). The (warns ..) pins the stable
           message lead (`this match arm is unreachable`); the arm the warning names is the second `(0 2)`.
           Wasm-graded (warnings ride the shared compile stage = target-independent; the rust/rust-async run
           paths cannot observe compile stderr, so the (warns ..) check is skipped there, not failed). Portable
           companion of the rcdzc a_duplicate_or_shadowed_match_arm_warns test; that test additionally pins
           exactly-one-warning across four shapes (variant/literal/after-catch-all/Option) and the delete
           fix, so it is KEPT — the (warns ..) substring clause expresses neither the count nor the fix.")
  (input (do (def (f (: n Int64)) (match n (0 1) (0 2) (_ 3))) (def (main) (f 0)) (export main)))
  (output (: 1 Int64))
  (warns CDZ0213 (message "this match arm is unreachable")))

(case
  "a catch-all after the specific arms already cover a finite type is unreachable and earns a CDZ0213 warning"
  (doc
    "The exhaustiveness-saturation face of the unreachable-arm warning — a DISTINCT detector from the
           duplicate-literal case above (which is caught by first-match-shadowing). Here `(match b (true 1)
           (false 2) (_ 3))` on a `Bool`: the `true` and `false` arms exhaust the finite type, so the
           trailing `_` catch-all can never match — dead by SATURATION, not by an earlier duplicate. The
           program compiles and runs (`(f true)` = 1) but the build surfaces the CDZ0213 `unreachable`
           warning. Pins that the redundancy pass detects a catch-all after a complete specific cover of a
           finite scrutinee (all booleans, or all variants of a sum), not only a literal/pattern duplicate.
           Wasm-graded (the run paths skip the (warns ..) check, not fail it). Companion of the rcdzc
           a_catch_all_after_the_specific_arms_saturate_a_finite_type_is_redundant test.")
  (input
    (do (def (f (: b Bool)) (match b (true 1) (false 2) (_ 3))) (def (main) (f true)) (export main)))
  (output (: 1 Int64))
  (warns CDZ0213 (message "this match arm is unreachable")))

(case
  "a refining arm shadowed by an earlier full-variant cover is unreachable and earns a CDZ0213 warning"
  (doc
    "The variant-refinement-subsumption face — a third DISTINCT detector (after duplicate-literal and
           finite-saturation): a BROADER earlier arm shadows a NARROWER later arm of the SAME variant.
           `(match o ((Some _) 0) ((Some (Some x)) x) ((None) -1))` on `(Option (Option Int64))`: the first
           `(Some _)` matches EVERY `Some` value, so the later `(Some (Some x))` — a refinement of the same
           `Some` variant — can never be reached. This is broader than an exact-duplicate arm: an earlier
           full-variant cover subsumes any later same-variant refinement. The program compiles and runs
           (`(f (None))` = -1, the `None` arm) but the build surfaces the CDZ0213 `unreachable` warning on
           the dead refining arm. Wasm-graded (the run paths skip the (warns ..) check, not fail it).
           Companion of the rcdzc a_refining_arm_shadowed_by_an_earlier_full_variant_cover_is_redundant test.")
  (input
    (do
      (def (f (: o (Option (Option Int64)))) (match o ((Some _) 0) ((Some (Some x)) x) ((None) -1)))
      (def (main) (f (None)))
      (export main)))
  (call main)
  (output (: -1 Int64))
  (warns CDZ0213 (message "this match arm is unreachable")))

(case
  "a structurally-duplicate tuple arm is unreachable and earns a CDZ0213 warning"
  (doc
    "The structural-shape-duplicate face — a fourth DISTINCT detector (after duplicate-literal, finite-
           saturation, and refinement-subsumption): two arms of the same STRUCTURAL SHAPE (binders normalized
           to `_`, literals compared by value) match the same region, so the later is unreachable.
           `(match t ((tuple true a) a) ((tuple true b) b) ((tuple false c) c))` — the first two arms are both
           `(tuple true _)` (the binder name `a`/`b` does not distinguish them), so the second is dead. The
           program compiles and runs (`(f (tuple true 1))` = 1, the first arm) but the build surfaces the
           CDZ0213 `unreachable` warning on the duplicate tuple arm. Pins that the redundancy pass compares
           arm SHAPES, not just variant tags or literals. Wasm-graded (the run paths skip the (warns ..)
           check, not fail it). Companion of the rcdzc
           a_structurally_duplicate_tuple_or_nested_ctor_arm_is_redundant test.")
  (input
    (do
      (def
        (f (: t (Tuple Bool Int64)))
        (match t (#tuple(true a) a) (#tuple(true b) b) (#tuple(false c) c)))
      (def (main) (f #tuple(true 1)))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (warns CDZ0213 (message "this match arm is unreachable")))

(case
  "an all-wildcard tuple arm is an irrefutable catch-all that shadows later arms and earns a CDZ0213 warning"
  (doc
    "The product-subsumption face — a fifth DISTINCT detector: an ALL-IRREFUTABLE tuple pattern
           `(tuple _ _)` (every element a wildcard or binder) matches EVERY value of its tuple type, so it is
           a whole-type catch-all (`is_irrefutable_cover`) and any arm after it is unreachable — even a
           broader tuple arm, not only a bare `_`. `(match t ((tuple _ _) 0) ((tuple true c) c))`: the first
           arm covers the whole `(Tuple Bool Int64)`, so the later `(tuple true c)` is dead. The program
           compiles and runs (`(f (tuple true 1))` = 0, the first arm) but the build surfaces the CDZ0213
           `unreachable` warning. Pins that irrefutability is detected through a PRODUCT pattern, not only a
           bare binder. Wasm-graded (the run paths skip the (warns ..) check, not fail it). Companion of the
           rcdzc an_all_wildcard_tuple_arm_is_a_catch_all_that_shadows_later_arms test.")
  (input
    (do
      (def (f (: t (Tuple Bool Int64))) (match t (#tuple(_ _) 0) (#tuple(true c) c)))
      (def (main) (f #tuple(true 1)))
      (export main)))
  (call main)
  (output (: 0 Int64))
  (warns CDZ0213 (message "this match arm is unreachable")))

(case
  "a match arm whose list length an earlier arm already covers is unreachable and earns a CDZ0213 warning"
  (doc
    "The list-length-subsumption face — a sixth DISTINCT detector: a list-match arm covers a LENGTH
           (an exact `(list a)` = length 1, or a `.. r` rest ray = length ≥ k), and a later arm whose lengths
           are all already covered is unreachable. `(match xs ((list a) a) ((list b) 9) (_ 0))` — both
           `(list a)` and `(list b)` match a length-1 list, so the second is dead by length coverage (not by
           variant, literal, tuple-shape, or irrefutability — the list-arity axis). The program compiles and
           runs (`(f (list 1))` = 1, the first arm) but the build surfaces the CDZ0213 `unreachable` warning.
           Pins that the redundancy pass reasons about list-length coverage (exact and `≥ k` ray). Wasm-graded
           (the run paths skip the (warns ..) check, not fail it). Companion of the rcdzc
           a_duplicate_or_shadowed_list_length_arm_is_redundant test.")
  (input
    (do
      (def (f (: xs (List Int64))) (match xs (#list(a) a) (#list(b) 9) (_ 0)))
      (def (main) (f #list(1)))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (warns CDZ0213 (message "this match arm is unreachable")))

(case
  "a match on a computed runtime value dispatches on the result"
  (doc
    "The scrutinee is the expression `(% n 2)`, computed at run time. Its value (0 for an
           even n) selects the literal arm 0. Exercises a match whose scrutinee is neither a
           literal nor a variable but an arbitrary runtime expression — the parity dispatch a
           LEB128 encoder performs.")
  (input (do (def (parity n) (match (% n 2) (0 0) (_ 1))) (def (main) (parity 4)) (export main)))
  (output (: 0 Int64)))

(case
  "a match on a record-field-access scrutinee dispatches on the field value"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected + #Member Access Projects A Record
           Field: the match scrutinee is `(. r n)`, a member access whose value is 5. The literal arm
           5 must match that value and yield 100 — the scrutinee's value is what is matched, whether it
           is written as a literal, a variable, an arithmetic expression, or a field projection.
           (Binding the field to a name first and matching that already works; matching the projection
           directly must behave identically.)")
  (input (let ((r #record((= n 5)))) (match r.n (5 100) (_ 200))))
  (output (: 100 Int64)))

(case
  "a match on a tuple-element-access scrutinee dispatches on the element value"
  (doc
    "The tuple companion of the case above: the scrutinee `(. t 0)` projects element 0 (value
           5), which the literal arm 5 must match, yielding 100. A positional access is a scrutinee
           value like any other.")
  (input (let ((t #tuple(5 9))) (match (. t 0) (5 100) (_ 200))))
  (output (: 100 Int64)))

(case
  "a match on a record field selects a later literal arm"
  (doc
    "Confirms the field-access scrutinee is matched against EACH literal arm, not just skipped to
           the wildcard: with r.n = 6, the 5 arm is passed over and the 6 arm selected, yielding 300.")
  (input (let ((r #record((= n 6)))) (match r.n (5 100) (6 300) (_ 200))))
  (output (: 300 Int64)))

(case
  "a nested match on a runtime scrutinee"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected: a match body may itself be a
           match on the same runtime scrutinee. Both selections are driven by the runtime value
           0, so the inner match's 0 arm is chosen and the result is 7.")
  (input
    (do (def (f n) (match n (0 (match n (0 7) (_ 8))) (_ 9))) (def (main) (f 0)) (export main)))
  (output (: 7 Int64)))

; The case above nests a match in a match ARM (both on the same scrutinee). A match may also take
; another match's RESULT as its SCRUTINEE — `(match (match …) …)` — the outer match dispatching on the
; value the inner match produced. This is the compiler idiom of dispatching on a sub-dispatch's result
; (classify, then act on the classification). The inner match's selected value crosses into the outer as
; an ordinary scrutinee value; core-semantics.md #Matching Is Exhaustive Or Rejected applies at each
; level. Distinct from the same-scrutinee nesting above: here the inner match is EVALUATED and its value
; consumed, not a body reached after the outer already matched.
(case
  "a match takes another match's result as its scrutinee"
  (doc
    "The scrutinee of the outer match is itself a match: `(match 1 (1 (Some 7)) (_ (None unit)))`
           evaluates to `(Some 7)`, which the outer match deconstructs, binding x=7. Pins that a match's
           scrutinee may be a match RESULT — the sub-dispatch is evaluated and its value consumed as an
           ordinary scrutinee, the compiler idiom of dispatching on a classification.")
  (input (match (match 1 (1 (Some 7)) (_ (None unit))) ((Some x) x) ((None _) 0)))
  (output (: 7 Int64)))

(case
  "a wildcard in a nested pattern position ignores that element"
  (doc
    "core-semantics.md #Pattern Matching: a `_` wildcard may appear at a NESTED position, matching
           anything there without binding. `(Some (tuple _ b))` matches a Some whose payload is a pair,
           ignoring the first element and binding `b` to the second — here 2. Pins that the wildcard is
           positional inside a compound pattern, not only a top-level catch-all arm.")
  (input (match (Some #tuple(1 2)) ((Some #tuple(_ b)) b) ((None _) 0)))
  (output (: 2 Int64)))

(case
  "a runtime scrutinee matching no arm traps"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected: a match on an Int64 arming only 1
           and 2, with no wildcard/else, cannot be proven to cover every Int64 value, so it is
           non-exhaustive and the compiler MUST reject it at compile time (CDZ0210) rather than emit a
           component that could trap at run time. The rejection is the recorded outcome; the program
           does not run.")
  (input (do (def (f n) (match n (1 10) (2 20))) (def (main) (f 3)) (export main)))
  (error CDZ0210))

(case
  "a boolean literal pattern matches a runtime scrutinee"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected over the two Bool values, with
           the scrutinee a runtime function parameter. `not` is a total match on true/false —
           exhaustive, so no else is needed and no generation rejects it.")
  (input
    (do
      (def (negate b) (match b (true false) (false true)))
      (def (main) (negate true))
      (export main)))
  (output (: false Bool)))

(case
  "a two-arm Bool match selects its second (false) arm"
  (doc
    "The else-branch companion of the `negate` case above: `(negate false)` takes the `false`
           arm, yielding `true`. A wildcard-less exhaustive Bool match emits its LAST arm as the
           unconditional else (once the `true` probe fails, `false` is the only value left), so this
           pins that the second arm's value is produced — not a dangling fallthrough. Together with the
           `(negate true)` case it exercises both selections of the two-arm Bool match.")
  (input
    (do
      (def (negate b) (match b (true false) (false true)))
      (def (main) (negate false))
      (export main)))
  (output (: true Bool)))

; A scalar `match` with MANY literal arms (≥4) lowers to a jump table rather than an if/probe chain. These
; pin the two positions the table lowering must get right: (1) in TAIL position — the match value IS the
; result — each arm is selected by the runtime scrutinee and produces its value; (2) in NON-TAIL position
; — the match value is CONSUMED by surrounding code — the match must YIELD into the enclosing expression,
; so the following code runs. The ≥4-arm NON-TAIL case was once miscompiled — a jump-table arm branched
; ONE BLOCK PAST the match's result-join to the function result, escaping the consumer; each arm now
; branches to the match's own `$join` block (`n_arms - k`, not `n_arms - k + 1`), so the following code
; runs in every position. These pin the ≥4-arm non-tail case (the fix), the ≤3-arm non-tail case (a
; distinct if/probe-chain lowering), and the ≥4-arm TAIL case — the whole boundary.
(case
  "a many-arm scalar match in tail position selects each arm by a runtime scrutinee"
  (doc
    "A FOUR-arm scalar match `(match a (0 10) (1 20) (2 30) (_ 40))` as the whole function body (TAIL
           position), driven by a runtime scrutinee `a`: each literal arm and the wildcard is selected in
           turn — a=0 → 10, a=1 → 20, a=2 → 30, a=9 → 40. Pins that the many-arm (jump-table) lowering
           dispatches to the correct arm for every scrutinee and produces that arm's value as the result —
           the opcode/tag-dispatch idiom a compiler's evaluator leans on, exercised across all arms.")
  (input (do (def (main (: a Int64)) (match a (0 10) (1 20) (2 30) (_ 40))) (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (call main (: 1 Int64))
  (output (: 20 Int64))
  (call main (: 2 Int64))
  (output (: 30 Int64))
  (call main (: 9 Int64))
  (output (: 40 Int64)))

(case
  "a many-arm match consumed in non-tail position yields into the enclosing expression"
  (doc
    "A FOUR-arm match `(match a (0 10) (1 20) (2 30) (_ 40))` (a jump-table lowering) consumed by
           `(+ … 100)` — its value is NOT the function result, so it must yield into the addition and
           `+ 100` must run: a=0 → 110, a=2 → 130, a=9 → 140. This was a SILENT WRONG-VALUE miscompile
           (valid wasm): a jump-table arm branched ONE BLOCK PAST the match's result-join to the FUNCTION
           result, so the arm value became the whole result and `+ 100` never ran (a=0 → 10). The default
           arm, which falls through to the join with no branch, was unaffected (a=9 → 140 was already
           right) — masking the bug. Fixed: each arm branches to the match's own `$join` block. The 3-arm
           operand case above (a different lowering) and the ≥4-arm TAIL case both worked throughout — this
           pins the ≥4-arm NON-tail position, the shape a compiler's 4+-way tag dispatch used as an operand
           takes.")
  (input (do (def (main (: a Int64)) (+ (match a (0 10) (1 20) (2 30) (_ 40)) 100)) (export main)))
  (call main (: 0 Int64))
  (output (: 110 Int64))
  (call main (: 2 Int64))
  (output (: 130 Int64))
  (call main (: 9 Int64))
  (output (: 140 Int64)))

(case
  "a many-arm match let-bound then consumed yields into the enclosing expression"
  (doc
    "The same jump-table lowering reached through a LET binding: `(let ((m (match a …4 arms…)))
           (+ m 100))`. The escape was not operand-specific — a let-bound then-used ≥4-arm match dropped the
           `+ 100` too (a=1 → 20 instead of 120), because the arm branch still escaped the match's join.
           Fixed alongside the operand case. a=1 → 120, a=9 → 140. Pins that the fix covers a match whose
           value is bound and later consumed, not only one directly in an operator's operand slot.")
  (input
    (do
      (def (main (: a Int64)) (let ((m (match a (0 10) (1 20) (2 30) (_ 40)))) (+ m 100)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 120 Int64))
  (call main (: 9 Int64))
  (output (: 140 Int64)))

(case
  "a three-arm match consumed as an operand yields into the enclosing expression"
  (doc
    "A THREE-arm match `(match a (0 10) (1 20) (_ 40))` consumed by `(+ … 100)` — the match value is
           NOT the function result, so the match must yield into the addition and `+ 100` must run: a=0 →
           110, a=1 → 120, a=9 → 140. Pins that a match in NON-TAIL (operand) position produces its value
           into the enclosing expression rather than escaping — for the ≤3-arm (if/probe-chain) lowering, a
           DISTINCT path from the ≥4-arm jump table (the case above), so both lowerings are pinned in
           non-tail position.")
  (input (do (def (main (: a Int64)) (+ (match a (0 10) (1 20) (_ 40)) 100)) (export main)))
  (call main (: 0 Int64))
  (output (: 110 Int64))
  (call main (: 1 Int64))
  (output (: 120 Int64))
  (call main (: 9 Int64))
  (output (: 140 Int64)))

(case
  "a many-arm string match consumed by concat beside a recursive call keeps both operands"
  (doc
    "The HEAP-operand face of the ≥4-arm non-tail escape (the scalar `+ 100` cases above pin the
           scalar face). A FOUR-arm String match `(d …)` (a jump-table lowering, its arm bodies are heap
           Strings) is consumed by `(String.concat (go (/ n 3)) (d (% n 3)))` — its SIBLING operand a
           RECURSIVE call `(go …)`. The escape bit HERE too: the recursive left operand was dropped and
           only the right `(d …)` survived — `go 4` returned \"b\" (byte-len 1) instead of \"bb\" (2),
           because the br_table arm branched one block PAST the concat to the function result, discarding
           the left handle. The recursive-call sibling is load-bearing: with the sibling a constant the
           drop is masked (the surviving arm is the result anyway). Fixed with the arithmetic cases (each
           arm branches to the match's own `$join`); pins that a heap-producing many-arm match yields its
           value into a consuming op even when the sibling is a recursive call — go(2)=\"c\"→1,
           go(4)=concat(\"b\",\"b\")→2, go(9)=concat(concat(\"b\",\"a\"),\"a\")→3.")
  (input
    (do
      (def (d (: v Int64)) (match v (0 "a") (1 "b") (2 "c") (_ "?")))
      (def (go (: n Int64)) (if (< n 3) (d n) (String.concat (go (/ n 3)) (d (% n 3)))))
      (def (main (: n Int64)) (String.byte-len (go n)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (call main (: 4 Int64))
  (output (: 2 Int64))
  (call main (: 9 Int64))
  (output (: 3 Int64)))

(case
  "a five-arm match consumed in non-tail position yields into the enclosing expression"
  (doc
    "The non-tail escape hit ANY ≥4-arm (jump-table) match, not exactly four: a FIVE-arm `(match a (0
           10) (1 20) (2 30) (3 50) (_ 40))` consumed by `(+ … 100)` must also yield into the addition —
           a=3 → 150 (arm value 50 plus 100), not 50 (the escaped arm value). Extends the four-arm non-tail
           case above to a wider jump table.")
  (input
    (do (def (main (: a Int64)) (+ (match a (0 10) (1 20) (2 30) (3 50) (_ 40)) 100)) (export main)))
  (call main (: 3 Int64))
  (output (: 150 Int64)))

(case
  "a dense many-arm match dispatches by value to a covered arm and to the default"
  (doc
    "The runtime value parity of a DENSE ≥4-int-arm match (the jump-table fast path): `(match k (0 10)
           (1 20) (2 30) (3 40) (_ 99))` selects arm 2 at k=2 → 30 and the wildcard default at an uncovered
           k=7 → 99. The density choice (jump table vs probe chain) is an emit detail; the dispatched value
           is invariant.")
  (input (do (def (f (: k Int64)) (match k (0 10) (1 20) (2 30) (3 40) (_ 99))) (export f)))
  (call f (: 2 Int64))
  (output (: 30 Int64))
  (call f (: 7 Int64))
  (output (: 99 Int64)))

(case
  "a sparse many-arm match dispatches identically to its dense sibling"
  (doc
    "The SAME arm count and shape as the dense case but with the literals spread (`0,1000,2000,3000`),
           so a jump table would be mostly-empty slots and the compiler falls back to the linear probe
           chain. The dispatched value is identical to the dense form: k=2000 → 30 (a covered arm), k=7 → 99
           (the default, not a covered slot). Pins that the density fallback is a pure emit choice with no
           value effect.")
  (input
    (do (def (f (: k Int64)) (match k (0 10) (1000 20) (2000 30) (3000 40) (_ 99))) (export f)))
  (call f (: 2000 Int64))
  (output (: 30 Int64))
  (call f (: 7 Int64))
  (output (: 99 Int64)))

(case
  "a scalar match beyond the jump-table cap falls back to a probe cascade and still dispatches"
  (doc
    "A scalar match with MORE arms than the jump-table density cap (>256 int arms) is INELIGIBLE for
           the br_table (a dense table holds at most 256 distinct values), so it emits the linear `if (== k)`
           probe cascade. This drives that fallback end-to-end and pins that a matched value still reaches
           the right arm through the long chain: 300 consecutive arms `k -> k*2` plus a wildcard `-1`.
           k=0 -> 0 (first arm), k=150 -> 300 (mid-chain), k=299 -> 598 (last arm), k=9999 -> -1 (default).")
  (input
    (do
      (def
        (f (: k Int64))
        (match
          k
          (0 0)
          (1 2)
          (2 4)
          (3 6)
          (4 8)
          (5 10)
          (6 12)
          (7 14)
          (8 16)
          (9 18)
          (10 20)
          (11 22)
          (12 24)
          (13 26)
          (14 28)
          (15 30)
          (16 32)
          (17 34)
          (18 36)
          (19 38)
          (20 40)
          (21 42)
          (22 44)
          (23 46)
          (24 48)
          (25 50)
          (26 52)
          (27 54)
          (28 56)
          (29 58)
          (30 60)
          (31 62)
          (32 64)
          (33 66)
          (34 68)
          (35 70)
          (36 72)
          (37 74)
          (38 76)
          (39 78)
          (40 80)
          (41 82)
          (42 84)
          (43 86)
          (44 88)
          (45 90)
          (46 92)
          (47 94)
          (48 96)
          (49 98)
          (50 100)
          (51 102)
          (52 104)
          (53 106)
          (54 108)
          (55 110)
          (56 112)
          (57 114)
          (58 116)
          (59 118)
          (60 120)
          (61 122)
          (62 124)
          (63 126)
          (64 128)
          (65 130)
          (66 132)
          (67 134)
          (68 136)
          (69 138)
          (70 140)
          (71 142)
          (72 144)
          (73 146)
          (74 148)
          (75 150)
          (76 152)
          (77 154)
          (78 156)
          (79 158)
          (80 160)
          (81 162)
          (82 164)
          (83 166)
          (84 168)
          (85 170)
          (86 172)
          (87 174)
          (88 176)
          (89 178)
          (90 180)
          (91 182)
          (92 184)
          (93 186)
          (94 188)
          (95 190)
          (96 192)
          (97 194)
          (98 196)
          (99 198)
          (100 200)
          (101 202)
          (102 204)
          (103 206)
          (104 208)
          (105 210)
          (106 212)
          (107 214)
          (108 216)
          (109 218)
          (110 220)
          (111 222)
          (112 224)
          (113 226)
          (114 228)
          (115 230)
          (116 232)
          (117 234)
          (118 236)
          (119 238)
          (120 240)
          (121 242)
          (122 244)
          (123 246)
          (124 248)
          (125 250)
          (126 252)
          (127 254)
          (128 256)
          (129 258)
          (130 260)
          (131 262)
          (132 264)
          (133 266)
          (134 268)
          (135 270)
          (136 272)
          (137 274)
          (138 276)
          (139 278)
          (140 280)
          (141 282)
          (142 284)
          (143 286)
          (144 288)
          (145 290)
          (146 292)
          (147 294)
          (148 296)
          (149 298)
          (150 300)
          (151 302)
          (152 304)
          (153 306)
          (154 308)
          (155 310)
          (156 312)
          (157 314)
          (158 316)
          (159 318)
          (160 320)
          (161 322)
          (162 324)
          (163 326)
          (164 328)
          (165 330)
          (166 332)
          (167 334)
          (168 336)
          (169 338)
          (170 340)
          (171 342)
          (172 344)
          (173 346)
          (174 348)
          (175 350)
          (176 352)
          (177 354)
          (178 356)
          (179 358)
          (180 360)
          (181 362)
          (182 364)
          (183 366)
          (184 368)
          (185 370)
          (186 372)
          (187 374)
          (188 376)
          (189 378)
          (190 380)
          (191 382)
          (192 384)
          (193 386)
          (194 388)
          (195 390)
          (196 392)
          (197 394)
          (198 396)
          (199 398)
          (200 400)
          (201 402)
          (202 404)
          (203 406)
          (204 408)
          (205 410)
          (206 412)
          (207 414)
          (208 416)
          (209 418)
          (210 420)
          (211 422)
          (212 424)
          (213 426)
          (214 428)
          (215 430)
          (216 432)
          (217 434)
          (218 436)
          (219 438)
          (220 440)
          (221 442)
          (222 444)
          (223 446)
          (224 448)
          (225 450)
          (226 452)
          (227 454)
          (228 456)
          (229 458)
          (230 460)
          (231 462)
          (232 464)
          (233 466)
          (234 468)
          (235 470)
          (236 472)
          (237 474)
          (238 476)
          (239 478)
          (240 480)
          (241 482)
          (242 484)
          (243 486)
          (244 488)
          (245 490)
          (246 492)
          (247 494)
          (248 496)
          (249 498)
          (250 500)
          (251 502)
          (252 504)
          (253 506)
          (254 508)
          (255 510)
          (256 512)
          (257 514)
          (258 516)
          (259 518)
          (260 520)
          (261 522)
          (262 524)
          (263 526)
          (264 528)
          (265 530)
          (266 532)
          (267 534)
          (268 536)
          (269 538)
          (270 540)
          (271 542)
          (272 544)
          (273 546)
          (274 548)
          (275 550)
          (276 552)
          (277 554)
          (278 556)
          (279 558)
          (280 560)
          (281 562)
          (282 564)
          (283 566)
          (284 568)
          (285 570)
          (286 572)
          (287 574)
          (288 576)
          (289 578)
          (290 580)
          (291 582)
          (292 584)
          (293 586)
          (294 588)
          (295 590)
          (296 592)
          (297 594)
          (298 596)
          (299 598)
          (_ -1)))
      (export f)))
  (call f (: 0 Int64))
  (output (: 0 Int64))
  (call f (: 150 Int64))
  (output (: 300 Int64))
  (call f (: 299 Int64))
  (output (: 598 Int64))
  (call f (: 9999 Int64))
  (output (: -1 Int64)))

; ── DEAD-ARM ELIMINATION is value-transparent ────────────────────────────────────────────────────────
; An arm the scrutinee's provable range can never reach is dropped (probe + body) by the backend. The
; elimination is a pure emit optimization — a dead arm was unreachable anyway — so dispatch is IDENTICAL
; whether or not it fires. These pin the observable VALUE across the shapes that trigger the drop: a
; masked scrutinee (`& x 7` in [0,7]), a wide-unsigned probe that only LOOKS out of range (kept), a
; guarded arm, a flow-refined scrutinee (`(> n 100)`), and the all-arms-dead collapse to the wildcard.
(case
  "a match over a masked scrutinee whose out-of-range arm is unreachable dispatches to the live arms"
  (doc
    "`(match (& x 7) (100 111) (0 222) (_ 333))` — `(& x 7)` is always in [0,7], so the `100` arm can
           never match (it is dropped by dead-arm elimination, but that is unobservable). Dispatch: x=8 →
           8&7=0 → the `0` arm → 222; x=5 → 5&7=5 → the wildcard → 333.")
  (input (do (def (f (: x Int64)) (match (: (& x 7) Int64) (100 111) (0 222) (_ 333))) (export f)))
  (call f (: 8 Int64))
  (output (: 222 Int64))
  (call f (: 5 Int64))
  (output (: 333 Int64)))

(case
  "a match over a masked scrutinee keeps an in-range arm at the range boundary"
  (doc
    "The live-arm companion: `(match (& x 7) (7 111) (100 222) (_ 333))` — `7` IS in [0,7] so the `7`
           arm is LIVE (kept) while `100` is dead. x=15 → 15&7=7 → the `7` arm → 111; x=8 → 8&7=0 → the
           wildcard → 333. Pins that only the genuinely-unreachable arm is elided, not the boundary-live one.")
  (input (do (def (f (: x Int64)) (match (: (& x 7) Int64) (7 111) (100 222) (_ 333))) (export f)))
  (call f (: 15 Int64))
  (output (: 111 Int64))
  (call f (: 8 Int64))
  (output (: 333 Int64)))

(case
  "a wide-unsigned match probe past i64 is not mistaken for out-of-range and still fires"
  (doc
    "SOUNDNESS: a `UInt64` probe of 2^63 has a NEGATIVE i64 bit pattern but a value legitimately in
           the UInt64 range — it must NOT be treated as out-of-range and dropped. `(match x (2^63 111) (0
           222) (_ 333))` over a UInt64: x=2^63 → the arm fires → 111; x=0 → 222. (Bodies are Int64 literals,
           so the result reads back as Int64.)")
  (input (do (def (f (: x UInt64)) (match x (9223372036854775808 111) (0 222) (_ 333))) (export f)))
  (call f (: 9223372036854775808 UInt64))
  (output (: 111 Int64))
  (call f (: 0 UInt64))
  (output (: 222 Int64)))

(case
  "a guarded arm over a masked scrutinee stays gated by its guard when the probe is in range"
  (doc
    "A LIVE guarded arm is kept and its guard still gates it: `(match (& x 7) ((guard 7 (> x 100)) 111)
           (100 222) (_ 333))` — the `7` probe is in [0,7] (live), so the arm fires only when x&7==7 AND
           x>100. x=127 → 127&7=7 and 127>100 → 111; x=7 → 7&7=7 but 7>100 false → wildcard 333; x=8 →
           8&7=0 ≠ 7 → wildcard 333. (The dead-arm elimination drops a GUARDED out-of-range arm whole —
           guard included — but that arm, `100` here, is unobservable; this pins the live guarded arm.)")
  (input
    (do
      (def (f (: x Int64)) (match (: (& x 7) Int64) ((guard 7 (> x 100)) 111) (100 222) (_ 333)))
      (export f)))
  (call f (: 127 Int64))
  (output (: 111 Int64))
  (call f (: 7 Int64))
  (output (: 333 Int64))
  (call f (: 8 Int64))
  (output (: 333 Int64)))

(case
  "a flow-refined match scrutinee dispatches with the refinement-dead arm elided"
  (doc
    "Inside the THEN branch of `(> n 100)` the scrutinee `n` is refined to [101, MAX], so the `5` arm
           is dead (dropped by dead-arm elimination, unobservable). `(if (> n 100) (match n (5 111) (200
           222) (_ 333)) 0)`: n=200 → >100 and ==200 → 222; n=150 → >100, not 200 → wildcard 333; n=5 →
           !(>100) → the else 0.")
  (input (do (def (f (: n Int64)) (if (> n 100) (match n (5 111) (200 222) (_ 333)) 0)) (export f)))
  (call f (: 200 Int64))
  (output (: 222 Int64))
  (call f (: 150 Int64))
  (output (: 333 Int64))
  (call f (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a match all of whose non-wildcard arms are unreachable collapses to the wildcard body"
  (doc
    "When EVERY non-wildcard arm is dead, the match collapses to the wildcard body, returned for every
           input. `(match (& x 7) (100 111) (200 222) (_ 333))` — `(& x 7)` is always in [0,7], so neither
           `100` nor `200` can ever match; every x yields the wildcard 333: x=0 → 333, x=7 → 333, x=999 →
           999&7=7 → still 333.")
  (input
    (do (def (f (: x Int64)) (match (: (& x 7) Int64) (100 111) (200 222) (_ 333))) (export f)))
  (call f (: 0 Int64))
  (output (: 333 Int64))
  (call f (: 7 Int64))
  (output (: 333 Int64))
  (call f (: 999 Int64))
  (output (: 333 Int64)))

(case
  "a Bool match with its arms in either order is exhaustive"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected: exhaustiveness of a Bool match is a
           property of the arm-value SET {true, false}, not the arm order. `(match b (false 2) (true
           1))` covers both values with the arms reversed, so it needs no wildcard. Exercised at BOTH
           runtime selections: `b` = true takes the `true` arm (1), `b` = false takes the `false` arm
           (2). Pins that the checker accepts the reversed order exactly as it accepts `(true …) (false
           …)` — the wildcard requirement is for OPEN types (Int64), never for a Bool covered by both
           literals — and that both branches select correctly at run time.")
  (input (do (def (main (: b Bool)) (match b (false 2) (true 1))) (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 2 Int64)))

(case
  "a wildcard-less Bool match returning Bool negates its scrutinee"
  (doc
    "The Bool-RESULT companion of the reversed-order case above: `(match b (true false) (false true))`
           is exhaustive without a wildcard (both Bool literals present) and its arm bodies are Bool, so the
           match computes `!b`. The wildcard-less match emits its LAST arm as the unconditional else, so the
           `false → true` arm is genuinely reached, not a dangling fallthrough. b=true → false, b=false →
           true.")
  (input
    (do
      (def (negate (: b Bool)) (match b (true false) (false true)))
      (def (main (: b Bool)) (negate b))
      (export main)))
  (call main (: true Bool))
  (output (: false Bool))
  (call main (: false Bool))
  (output (: true Bool)))

(case
  "a Bool match with only the true arm is non-exhaustive"
  (doc
    "The negative control that pins the Bool-exhaustiveness relaxation does NOT over-accept:
           `(match b (true 1))` covers only `true`, leaving `false` unhandled — genuinely
           non-exhaustive, so it MUST reject (CDZ0210) exactly as an Int64 match without a wildcard
           does. A Bool match is exhaustive only when BOTH `true` and `false` arms are present; a single
           Bool literal is not enough. (An Int64 match without a wildcard likewise stays rejected — the
           relaxation is specific to a Bool scrutinee covered by both of its two values.)")
  (input (do (def (main (: b Bool)) (match b (true 1))) (export main)))
  (error CDZ0210))

(case
  "a match on a runtime integer scrutinee producing a boolean"
  (doc
    "core-semantics.md #Matching Is Exhaustive Or Rejected: the scrutinee is a runtime integer
           but the arm bodies are Bool — a match is an expression of whatever type its arms yield,
           not restricted to the scrutinee's type. `is-zero` maps 0 → true, else → false; is-zero(0)
           = true. The Bool result must cross the run boundary as the program's value (Ordering.of the
           Bool-returning function cases in 09-functions.sexp — same result-kind requirement, reached
           through a match rather than a call).")
  (input (do (def (is-zero n) (match n (0 true) (_ false))) (def (main) (is-zero 0)) (export main)))
  (output (: true Bool)))

(case
  "two sum arms with textually identical bodies still bind their own variant's payload"
  (doc
    "An optimization that collapses a match whose arm bodies are all the same to that one body must
           not treat two arms as identical when their bodies REFERENCE a per-arm binder: `((N.I x) (+ x 1))`
           and `((N.J x) (+ x 1))` are textually the same `(+ x 1)`, but `x` binds the `I` payload in the
           first arm and the `J` payload in the second — they are NOT the same body. With `b` = false the
           scrutinee is `(N.J 9)`, so the taken arm's `x` is 9 and the result is 10; a collapse that fused
           the two arms and read the first arm's payload slot would wrongly yield 6 (the `I` payload 5 + 1).
           Pins that the all-same-body collapse is keyed on the body AFTER binder resolution, so an arm that
           binds a different sub-value is a distinct body.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (main (: b Bool)) (match (if b (N.I 5) (N.J 9)) ((N.I x) (+ x 1)) ((N.J x) (+ x 1))))
      (export main)))
  (call main (: false Bool))
  (output (: 10 Int64)))

; A match is an expression of ONE type — all its arm bodies must agree, exactly as a conditional's two
; branches must (core-semantics.md #Matching Is Exhaustive Or Rejected makes a match an expression whose
; type is what its arms yield; #Conditionals Evaluate One Branch requires "every branch … type-checked
; whether or not it is evaluated"). So arm bodies of DIFFERENT type — a `1` (Int64) arm and a `true`
; (Bool) arm — make the match ill-typed (CDZ0203), whether or not the constant scrutinee selects one of
; them. A compiler that CONST-FOLDS a match on a literal scrutinee to its matching arm and emits only that
; arm — without type-checking the OTHER arms — silently accepts `(match 5 (5 1) (_ true))` and runs it to
; 1, an unevaluated arm carrying a deferred type error. This is the match analogue of the conditional
; branch-agreement check (which rejects `(if (= 5 5) 1 true)` even though the constant condition selects
; the Int64 branch): the arm-type check is on the SET of arm bodies, independent of which the scrutinee
; hits. A RUNTIME-scrutinee match already checks this ("runtime match arms differ in kind"); the gap is the
; const-folded path. A generation that does not yet check the unselected arms' types declines rather than
; emitting the arm it folded to.
(case
  "a match whose arm bodies have different types is a type error even when a constant scrutinee selects one"
  (doc
    "`(match 5 (5 1) (_ true))` has an Int64 arm body `1` and a Bool arm body `true` — a match is an
           expression of one type, so disagreeing arm bodies are ill-typed (CDZ0203), the match analogue of
           the conditional branch-agreement cases (`(if … 1 true)` is rejected). The constant scrutinee `5`
           selects the Int64 arm, so a compiler that const-folds the match to its matching arm and emits
           only that arm — without type-checking the other arms — silently accepts this and runs it to 1,
           an unevaluated arm carrying a deferred type error (core-semantics.md #Conditionals Evaluate One
           Branch: every branch is type-checked whether or not evaluated; the same for a match's arms). A
           runtime-scrutinee match already rejects arms that differ in kind; this pins the const-folded
           path. A generation that does not yet check the unselected arms declines rather than emitting the
           folded arm.")
  (input (match 5 (5 1) (_ true)))
  (error CDZ0203))

; The unselected-arm check must type-check each arm's BODY for internal errors, not only compare the arms'
; RESULT types. The case above pins arm-type-AGREEMENT (an Int64 arm vs a Bool arm); this pins that an
; unselected arm whose body is INTERNALLY ill-typed — `(+ 1 true)`, an Int64/Bool arithmetic mismatch —
; is rejected too, even though the constant scrutinee selects the other arm. core-semantics.md
; #Conditionals Evaluate One Branch: "Every branch … MUST be type-checked whether or not it is evaluated,
; so that an unevaluated branch cannot carry a deferred error" — the same for a match's arms. The `if`
; form already catches this in its unselected branch (`(if true 1 (+ 1 true))` is rejected "operation on
; mismatched types"); the const-folded match does not — it takes the unselected arm's result type
; superficially (here Int64, which agrees with the selected arm) WITHOUT checking the body, so the internal
; `(+ 1 true)` deferred type error slips through and the program runs to 1. A generation that does not yet
; type-check an unselected arm body declines rather than emitting the folded arm.
(case
  "an internally ill-typed unselected match arm body is a type error"
  (doc
    "`(match 5 (5 1) (_ (+ 1 true)))` — the unselected `_` arm body `(+ 1 true)` mixes Int64 and Bool,
           an internal type error the compiler MUST reject (CDZ0203), even though the constant scrutinee `5`
           selects the `1` arm. Distinct from the arm-type-AGREEMENT case above: there the two arms' result
           types disagree; here an arm's BODY is internally ill-typed while its result type (Int64) agrees
           with the selected arm. core-semantics.md #Conditionals Evaluate One Branch requires every branch
           type-checked whether or not evaluated, and the same holds for a match's arms — the `if` form
           already rejects `(if true 1 (+ 1 true))`. Pins that the const-folded match type-checks each arm's
           BODY, not only compares arm result types. A generation that does not yet check the unselected
           arm's body declines rather than emitting the folded arm.")
  (input (match 5 (5 1) (_ (+ 1 true))))
  (error CDZ0203))

; The unselected-arm check must reach a SCOPE error, not only a type error. The two cases above pin that
; a const-folded match type-checks its unselected arms (agreement + internal type); this pins that it
; also SCOPE-checks them — an unbound name in an unselected arm is rejected (CDZ0101), exactly as the
; `if` form already rejects an unbound name in its unselected branch (`(if true 1 undefined-name)` above).
; core-semantics.md #Binding Is Lexical: "A reference to a name with no enclosing binding MUST be a
; compile-time error" (unconditional); #Conditionals Evaluate One Branch: every branch type-checked
; whether or not evaluated — the same for a match's arms, and scope resolution reaches an unselected arm
; as the type check does. `(match 2 (1 undefined-z) (_ 99))` selects the `_` arm (scrutinee 2 ≠ 1), but
; the `1` arm references the unbound `undefined-z`; the program MUST be rejected CDZ0101, not run to 99.
; The seed const-folds the match to its selected arm and scope-checks ONLY that arm, so the unbound
; reference in the dropped arm slips and it runs to 99 — the scope-check analogue of the type-check the
; case above pins, and the match companion of the `if` unselected-branch scope case. This is the more
; fundamental gap: it swallows CDZ0101, the front-end check every generation makes ("scope resolution
; needs no static typing"), on the const-folded match path. (The `if` form scope-checks its dropped
; branch correctly; only the const-folded match drops the unselected arm's scope check.) A generation
; that scope-checks every arm before folding declines rather than emitting the folded arm.
(case
  "an unbound name in an unselected match arm is still rejected"
  (doc
    "`(match 2 (1 undefined-z) (_ 99))` references the unbound name `undefined-z` in the `1` arm;
           the scrutinee 2 selects the `_` arm, but the program MUST be rejected (CDZ0101,
           core-semantics.md #Binding Is Lexical — unconditional — with #Conditionals Evaluate One Branch:
           every arm checked whether or not evaluated). An unevaluated arm cannot carry a deferred scope
           error, exactly as an unevaluated `if` branch cannot (`(if true 1 undefined-name)` is rejected
           above). The seed const-folds the match to its selected arm and scope-checks only that arm, so
           the unbound reference in the dropped `1` arm slips and it runs to 99 — swallowing CDZ0101, the
           front-end check every generation makes. This is the match companion of the `if`
           unselected-branch scope case, and the scope-check analogue of the unselected-arm TYPE cases
           above (which the seed already enforces). A generation that scope-checks every arm before
           folding declines rather than emitting the folded arm.")
  (input (match 2 (1 undefined-z) (_ 99)))
  (error CDZ0101))

; The arm-type-agreement check must fire when a RUNTIME scrutinee's first arm body is a bare PAYLOAD
; BINDER — the const-folded cases above assume "a runtime-scrutinee match already rejects arms that
; differ in kind," but that check SLIPS when the first arm's body is just the payload variable it binds,
; and the match then MISCOMPILES rather than merely running. `(match o ((Some x) x) ((None _) true))`
; over a runtime `o : Option Int64` has a `Some` arm body `x` (Int64, the payload) and a `None` arm body
; `true` (Bool) — disagreeing arm types, so the match is ill-typed (CDZ0201) exactly as `(match 5 (5 1)
; (_ true))` is, and as the conditional `(if … 1 true)` is. But the seed accepts it and, worse, the
; Int64 payload is REINTERPRETED as a Bool across the run boundary: `(f (Some 5))` yields `true`,
; `(f (Some 42))` yields `false` — the payload's bits read as a boolean, neither the true Int value nor a
; rejection. The tell that isolates the gap: make the first arm body ANYTHING but a bare binder and the
; check fires — a literal `((Some x) 99)` rejects "match arm bodies have different types", an arithmetic
; `((Some x) (+ x 0))` rejects "runtime sum match arms differ in kind" — only the bare-binder first arm
; `((Some x) x)` slips. And an inline constant scrutinee (`(match (Some 5) ((Some x) x) ((None _) true))`)
; const-folds to the correct Int 5, so the defect is specific to the RUNTIME-scrutinee + bare-binder-first-
; arm path — the exact path the "runtime match already checks this" assumption relies on. It is a wrong
; VALUE, not only a missed rejection: an Int64 payload emerges as a Bool. A generation that checks the arm
; result types on this path declines the ill-typed program rather than reinterpreting the payload's bits.
(case
  "a runtime-scrutinee match with a bare-binder first arm and a differently-typed second arm is a type error"
  (doc
    "`(match o ((Some x) x) ((None _) true))` over a runtime `o : Option Int64` has a `Some` arm
           body `x` of type Int64 (the payload) and a `None` arm body `true` of type Bool — disagreeing
           arm types, so the match is ill-typed (CDZ0203), the same arm-agreement rule as `(match 5 (5 1)
           (_ true))` and the conditional `(if … 1 true)`. The seed accepts it and REINTERPRETS the Int64
           payload as a Bool: `(f (Some 5))` yields `true`, `(f (Some 42))` yields `false` — a wrong value
           (the payload's bits read as a boolean), not merely a missed rejection. The check fires when the
           first arm body is a literal (`((Some x) 99)` → \"match arm bodies have different types\") or an
           expression (`((Some x) (+ x 0))` → \"runtime sum match arms differ in kind\"); ONLY a bare
           payload-binder first arm slips, and only on a runtime scrutinee (an inline constant scrutinee
           const-folds to the correct Int). Falsifies the assumption that a runtime-scrutinee match already
           checks arm-type agreement. A generation that checks the arm result types declines rather than
           reinterpreting the payload.")
  (input
    (do (def (f o) (match o ((Some x) x) ((None _) true))) (def (main) (f (Some 5))) (export main)))
  (error CDZ0203))

; --- Boolean connectives (short-circuit) -------------------------------------------------
; core-semantics.md #Boolean Connectives Short-Circuit: the language offers conjunction, disjunction,
; and negation over Bool. Conjunction evaluates its right operand ONLY when the left is true;
; disjunction ONLY when the left is false — so a connective shields a trapping or effectful right
; operand exactly as an unselected conditional branch does (#Conditionals Evaluate One Branch). Each
; operand is type-checked as a Bool whether or not it is evaluated. The seed does not yet realize
; `and`/`or`/`not`, so it DECLINES these until a generation adds them; they
; desugar to short-circuit conditionals (`(and a b)` = `(if a b false)`, `(or a b)` = `(if a true b)`,
; `(not a)` = `(if a false true)`), which the seed already lowers.
(case
  "conjunction is true exactly when both operands are true"
  (doc
    "The `and` value table over the four Bool pairs, folded to one witness: only true∧true is
           true (core-semantics.md #Boolean Connectives Short-Circuit).")
  (input
    (do
      (def (row a b) (if (and a b) 1 0))
      (def (main) (+ (+ (row true true) (row true false)) (+ (row false true) (row false false))))
      (export main)))
  (output (: 1 Int64)))

(case
  "disjunction is false exactly when both operands are false"
  (doc
    "The `or` value table: only false∨false is false, so three of the four pairs are true
           (core-semantics.md #Boolean Connectives Short-Circuit).")
  (input
    (do
      (def (row a b) (if (or a b) 1 0))
      (def (main) (+ (+ (row true true) (row true false)) (+ (row false true) (row false false))))
      (export main)))
  (output (: 3 Int64)))

(case
  "negation inverts a boolean"
  (doc
    "`(not true)` is false and `(not false)` is true (core-semantics.md #Boolean Connectives
           Short-Circuit).")
  (input (do (def (main) (if (not false) (not true) true)) (export main)))
  (output (: false Bool)))

; ── BOOLEAN-COERCION folds: (if c false true) → ¬c and (if c true false) → c, on a RUNTIME condition ──
; A conditional selecting between the two boolean CONSTANTS is a boolean coercion of its condition, which
; `lower` folds structurally: `(if c false true)` is the negation of `c` (the seed's `(not c)` desugars to
; exactly this, and it lowers to a `Core::Not` — an `i32.eqz` — NOT a two-arm branch), and `(if c true
; false)` is `c` itself (an `if` that returns its condition's truth value). Both are backend-independent
; Core rewrites both backends inherit; `Core::Not` arises ONLY from the `(if c false true)` fold, so these
; are its sole corpus witnesses. Pinned on a RUNTIME condition (a constant `c` folds to the constant Bool).
(case
  "an if returning false/true folds to the negation of a runtime condition"
  (doc
    "`(if (> b 0) false true)` selects between the two Bool constants, so it IS the negation of the
           condition — `lower` folds it to `Core::Not` (an `i32.eqz` over the compare), not a branch.
           b = 5: `(> 5 0)` true → false; b = -5: false → true. Pins the `(if c false true)` → ¬c
           boolean-coercion fold on a runtime condition, both backends (this is how the seed's `not`
           lowers, so it is `Core::Not`'s witness).")
  (input (do (def (main (: b Int64)) (if (> b 0) false true)) (export main)))
  (call main (: 5 Int64))
  (output (: false Bool))
  (call main (: -5 Int64))
  (output (: true Bool)))

(case
  "an if returning true/false folds to the runtime condition itself"
  (doc
    "The dual: `(if (> b 0) true false)` returns the condition's OWN truth value — a boolean
           coercion that folds to `c` itself (no branch, no negation). b = 5 → true, b = -5 → false.
           Pins the `(if c true false)` → c fold on a runtime condition, both backends — the identity
           companion of the negation fold above.")
  (input (do (def (main (: b Int64)) (if (> b 0) true false)) (export main)))
  (call main (: 5 Int64))
  (output (: true Bool))
  (call main (: -5 Int64))
  (output (: false Bool)))

(case
  "conjunction shields a trapping right operand when the left is false"
  (doc
    "`(and false (< (/ 1 0) 2))`: `and` evaluates its right operand ONLY when the left is true,
           so with the left false the division-by-zero trap in the right operand is NOT evaluated and
           the result is false — the connective shields the trap exactly as an unselected conditional
           branch does (core-semantics.md #Boolean Connectives Short-Circuit). Without short-circuit
           this would trap.")
  (input (and false (< (/ 1 0) 2)))
  (output (: false Bool)))

(case
  "disjunction shields a trapping right operand when the left is true"
  (doc
    "`(or true (< (/ 1 0) 2))`: `or` evaluates its right operand ONLY when the left is false, so
           with the left true the trap in the right operand is NOT evaluated and the result is true.
           The dual of the `and` shielding case (core-semantics.md #Boolean Connectives Short-Circuit).")
  (input (or true (< (/ 1 0) 2)))
  (output (: true Bool)))

; NESTED boolean connectives compose over runtime operands: `and`/`or`/`not` each desugar to a short-
; circuit conditional (`(and a b)`=`(if a b false)`, `(or a b)`=`(if a true b)`, `(not a)`=`(if a false
; true)`), so nesting them (a `not` of an `and`, an `and` of two `not`s — the De Morgan shapes) exercises
; the conditional nesting the single-operator cases don't. The VALUE is the ordinary boolean composition;
; these pin that the nested desugaring threads correctly on both backends (not that the compiler applies
; De Morgan — it need not; the two forms just compute the same truth table).
(case
  "a not of a runtime conjunction computes the negated conjunction"
  (doc
    "`(not (and (> a 0) (> b 0)))` over runtime params: the `and` short-circuits (right skipped when
           left false), and the `not` inverts the result. (1,1) both-positive → and=true → not=false;
           (1,-1) → and=false → not=true. Pins a `not` composing over a runtime short-circuit `and`, both
           backends.")
  (input (do (def (main (: a Int64) (: b Int64)) (not (and (> a 0) (> b 0)))) (export main)))
  (call main (: 1 Int64) (: 1 Int64))
  (output (: false Bool))
  (call main (: 1 Int64) (: -1 Int64))
  (output (: true Bool)))

(case
  "an and of two runtime negations computes their conjunction"
  (doc
    "The De Morgan twin: `(and (not (> a 0)) (not (> b 0)))` — an `and` whose BOTH operands are
           negations, over runtime params. Both non-positive → both nots true → and=true; one positive →
           its not false → and=false (the other not short-circuited away). (-1,-1) → true; (1,-1) → false.
           Pins two `not`s composing under a short-circuit `and`, both backends.")
  (input (do (def (main (: a Int64) (: b Int64)) (and (not (> a 0)) (not (> b 0)))) (export main)))
  (call main (: -1 Int64) (: -1 Int64))
  (output (: true Bool))
  (call main (: 1 Int64) (: -1 Int64))
  (output (: false Bool)))

(case
  "a runtime conjunction still shields a comparison right operand whose subexpression traps"
  (doc
    "The shielding must survive the branchless emit: an `and`/`or` whose right operand is a
           trap-free COMPARISON may be lowered to a branchless `select` (both operands evaluated) — but
           ONLY when the comparison's own operands are trap-free. `(and (= a 1) (< (/ 1 z) 5))` has a
           right operand `(< (/ 1 z) 5)` whose subexpression `(/ 1 z)` can trap, so the connective MUST
           keep short-circuiting: with `a = 99` the left `(= a 1)` is false, so the right operand is NOT
           evaluated and the division by zero (z = 0) is shielded — the result is false, not a trap. Pins
           that the branchless-connective optimization does not treat a comparison with a trapping
           subexpression as a trap-free leaf; the left operand is a RUNTIME value, so this is the emit-path
           shielding the constant-fold cases above cannot witness.")
  (input
    (do (def (main (: a Int64) (: z Int64)) (if (and (= a 1) (< (/ 1 z) 5)) 1 0)) (export main)))
  (call main (: 99 Int64) (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a boolean connective with a non-boolean operand is a type error"
  (doc
    "`(and true 1)` gives an Int64 where a Bool operand is required. core-semantics.md #Boolean
           Connectives Short-Circuit: each operand is type-checked as a Bool whether or not it is
           evaluated, so the compiler MUST reject the non-Bool operand (CDZ0201) rather than run — the
           same discipline as a conditional's branch type-check, applied to a connective's operand.")
  (input (and true 1))
  (error CDZ0201))

(case
  "an or connective with a non-boolean operand is a type error"
  (doc
    "The `or` companion of the `and` case above: `(or 5 false)` gives an Int64 where a Bool operand
           is required. Each operand of a boolean connective is type-checked as a Bool whether or not it is
           evaluated (core-semantics.md #Boolean Connectives Short-Circuit), so the non-Bool operand is
           rejected CDZ0201 — the rule holds for `or`, not only `and`.")
  (input (or 5 false))
  (error CDZ0201))

(case
  "a not connective with a non-boolean operand is a type error"
  (doc
    "The unary `not` companion: `(not 7)` negates an Int64, but `not` requires a Bool operand, so it
           is rejected CDZ0201. Pins that the Bool-operand discipline covers the unary connective too, not
           only the binary `and`/`or`.")
  (input (not 7))
  (error CDZ0201))

(case
  "a recursive function that threads a tuple accumulator returns it"
  (doc
    "A recursive function whose result is a TUPLE in every branch — a `(value, cursor)` accumulator
           threaded through the recursion — MUST compile and return that tuple. `go` returns `(tuple acc 0)`
           at the base and, in the recursive branch, matches a helper's tuple `(pair n)` and recurses with an
           updated accumulator; the result kind is a tuple on both branches, so the function is tuple-valued
           throughout. `(go 3 0)` sums 3+2+1 into `acc`, yielding `(tuple 6 0)`, and `a` = 6. A generation
           whose return-kind inference does not recognize the recursive branch as tuple-valued declines
           (\"runtime sum match without a constructor arm\" — the tuple match is misread as a sum match when
           the tuple comes from a call and the arm recurses); but a tuple-threading recursion is an ordinary
           function, load-bearing for any recursive-descent walk that threads a (node, position) cursor.")
  (input
    (do
      (def
        (go n acc)
        (if (= n 0) #tuple(acc 0) (match (pair n) (#tuple(v k) (go (- n 1) (+ acc v))))))
      (def (pair n) #tuple(n n))
      (def (main) (match (go 3 0) (#tuple(a b) a)))
      (export main)))
  (output (: 6 Int64))
  (live-objects 0))

(case
  "a tail-recursive function returning a tuple is tuple-valued"
  (doc
    "The MINIMAL isolation of the case above — no accumulator, no helper, no heap: a tail-recursive
           function whose branches are both a TUPLE MUST be tuple-valued, so a match on its result
           destructures the tuple. `(go 3)` recurses to the base `(tuple 0 0)`; `(+ a b)` = 0. A generation
           whose return-kind inference does not carry the base branch's tuple kind back through the
           tail-recursive call declines (\"runtime sum match without a constructor arm\" — the recursive call
           site is 'unknown tuple shape', so the result's tuple match is misread as a sum match). The trigger
           is precisely TAIL-RECURSION + a TUPLE return: a non-recursive tuple return compiles, and a
           NON-tail recursive function that WRAPS its recursive result in a new tuple compiles; only the
           tail-recursive tuple return does not. This is the return-kind companion of the tail-recursive
           SCALAR accumulator inference (realized) — a tuple result must infer the same way a scalar does.")
  (input
    (do
      (def (go n) (if (< n 1) #tuple(0 0) (go (- n 1))))
      (def (main) (match (go 3) (#tuple(a b) (+ a b))))
      (export main)))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a mutually-recursive decoder returns a heap value and cursor and its heap slot is dispatched"
  (doc
    "The MUTUAL-RECURSION sibling of the tail-recursive tuple return above. `dn` (decode-node) and
           `dac` (decode-children) are mutually recursive: `dn` returns `(tuple <Ast> <cursor>)` — a HEAP
           sum value paired with an Int cursor — and `dac` matches `dn`'s tuple `((tuple child nx) …)` and
           recurses. `top` destructures `dn`'s result to the HEAP slot `ast`, and `main` dispatches on it
           with CONSTRUCTOR patterns. The return-kind inference must carry the tuple's per-slot kinds — a
           heap slot (the `Ast`) stays Heap, a scalar slot (the cursor) is Int — back through the MUTUAL
           recursion, so `top`'s result is a runtime sum (Heap) and the caller's `((AInt n) …)` takes the
           runtime-sum-match path. A generation that infers the heap slot as a scalar declines 'runtime
           match with a non-literal pattern' (the constructor pattern is read against a scalar) or 'cannot
           infer runtime compound result shape' — ask-77, the mutual-recursion face of the tail-recursive
           tuple return. `(dn (list 42 7) 0)` at i=0 yields `(tuple (AInt 42) 1)`; `top` returns `(AInt 42)`;
           `main` reads 42. `List.at`+`Option.expect` keeps the element a genuine runtime value (unfolded).")
  (input
    (do
      (type Ast (AInt Int64) ALeaf (AList (List Ast)))
      (def
        (dn b i)
        (if
          (= i 0)
          #tuple((AInt (Option.expect (List.at b 0) "in range")) (+ i 1))
          #tuple((AList (dac b i (- i 1) #list())) (+ i 1))))
      (def
        (dac b i n acc)
        (if
          (< n 1)
          acc
          (match (dn b i) (#tuple(child nx) (dac b nx (- n 1) (List.push acc child))))))
      (def (top b) (match (dn b 0) (#tuple(ast pos) ast)))
      (def (main) (match (top #list(42 7)) ((AInt n) n) (_ -1)))
      (export main)))
  (output (: 42 Int64))
  (live-objects known-leak))

; The recursive-descent PARSER face of the mutual-recursion cursor thread: the decoder above destructures
; the returned (value, cursor) tuple with a tuple PATTERN in a match arm; a hand-written precedence parser
; instead PROJECTS the returned tuple by member access (`(. inner 0)`/`(. inner 1)`) and REBUILDS a fresh
; tuple that pairs a boxed-sum node built from the value slot with the threaded cursor slot — `(tuple (Neg
; (. inner 0)) (. inner 1))`. So a boxed-sum payload is projected out of one recursive-return tuple, wrapped
; in a NEW sum ctor, and returned in a new tuple ACROSS the mutual `pa↔pb` edge while the cursor from the
; other projection is threaded on. This is the slot-alias-prone shape (a boxed sum projected from a tuple
; and re-threaded through a self/mutual loop — the compiler-ml decode/parser stress, self-hosting-surface.md
; #The Reader Is Written In Cadenza); pinning it green guards the tuple-projection slot allocator against a
; refcount/slot change that would alias the projected-sum handle with the cursor arith temp.
(case
  "a mutually-recursive parser projects a return tuple and rebuilds one with a boxed sum across the edge"
  (doc
    "`pa`/`pb` mutually recurse over a token list, each returning `(Expr, next-index)`. `pa` at index
           i reads the token: on `0` it recurses via `pb(i+1)`, PROJECTS that return with `(. inner 0)` /
           `(. inner 1)`, and rebuilds `(tuple (Expr.Neg (. inner 0)) (. inner 1))` — a boxed sum built from
           the projected value slot paired with the threaded cursor slot; otherwise it returns `(tuple
           (Expr.Lit t) (+ i 1))`. `run` parses two factors in sequence, threading the cursor `(. a 1)` into
           the second parse, and sums their evaluated values. toks `[0,7,0,7]`: `pa(0)` sees 0 → `Neg(pb(1))`
           = `Neg(Lit 7)` at cursor 2, `pa(2)` sees 0 → `Neg(Lit 7)` at cursor 4, so `ev` gives -7 + -7 =
           -14. Pins that a boxed-sum payload projected from a recursive-return tuple, re-wrapped in a new
           ctor, and re-threaded across the mutual-recursion edge stays correct — the recursive-descent
           parser shape the self-hosted compiler takes, and a slot-allocator guard for the projected-sum /
           cursor-temp seam.")
  (input
    (do
      (type Expr (Lit Int64) (Neg Expr))
      (def (toks) #list(0 7 0 7))
      (def
        (pa (: i Int64))
        (let
          ((t (match (List.at (toks) i) ((Some x) x) ((None _) -1))))
          (if
            (= t 0)
            (let ((inner (pb (+ i 1)))) #tuple((Expr.Neg (. inner 0)) (. inner 1)))
            #tuple((Expr.Lit t) (+ i 1)))))
      (def (pb (: i Int64)) (pa i))
      (def (ev (: e Expr)) (match e ((Expr.Lit n) n) ((Expr.Neg x) (- 0 (ev x)))))
      (def (run) (let ((a (pa 0))) (let ((b (pa (. a 1)))) (+ (ev (. a 0)) (ev (. b 0))))))
      (export run)))
  (output (: -14 Int64))
  (live-objects known-leak))

; --- A binding position accepts an irrefutable pattern ---------------------------------------
; core-semantics.md #A Binding Position Accepts An Irrefutable Pattern: a `let` binder (and a parameter)
; MAY hold an irrefutable pattern in place of a bare name, binding the names it introduces to the
; corresponding sub-values of the bound value — exactly as the same pattern would in a single match arm
; over that value. A bare name and a wildcard are the trivial irrefutable patterns; a tuple pattern whose
; every element is irrefutable is irrefutable, recursively to any depth (#Patterns Compose). This is the
; ergonomic form of the bind-then-rematch idiom the decoder cases above pay by hand — `(let ((r v)) (match
; r ((tuple a b) …)))` becomes `(let (((tuple a b) v)) …)`.
(case
  "a let binder may be a tuple pattern that destructures the value"
  (doc
    "`(let (((tuple a b) (tuple 3 4))) (+ a b))` binds `a` and `b` to the two elements of the bound
           pair (core-semantics.md #A Binding Position Accepts An Irrefutable Pattern) — the same binding a
           `(match (tuple 3 4) ((tuple a b) (+ a b)))` arm makes, written at the binder. Pins that a tuple
           pattern in a `let` binder position destructures the value rather than requiring a bind-then-match.")
  (input (let ((#tuple(a b) #tuple(3 4))) (+ a b)))
  (output (: 7 Int64)))

(case
  "a tuple binding pattern nests to any depth"
  (doc
    "`(let (((tuple a (tuple b c)) (tuple 1 (tuple 2 3)))) …)` — a tuple pattern whose second element
           is itself a tuple pattern, bound recursively (core-semantics.md #A Binding Position Accepts An
           Irrefutable Pattern / #Patterns Compose: a binder position admits any pattern). Pins that a
           binding pattern composes to any depth, exactly as a match-arm pattern does.")
  (input (let ((#tuple(a #tuple(b c)) #tuple(1 #tuple(2 3)))) (+ a (+ b c))))
  (output (: 6 Int64)))

(case
  "a let tuple pattern destructures a helper CALL's runtime tuple result"
  (doc
    "The multi-return idiom: `divmod` returns `(tuple (/ a b) (% a b))` — a RUNTIME tuple, not a
           literal — and the caller destructures it at the binder: `(let (((tuple q r) (divmod a b))) …)`.
           47/10 → q=4, r=7 → 407. The const-tuple binder cases above fold; here the RHS is a live call
           whose compound result must materialize and destructure at run time — the two-values-out shape
           every quotient/remainder, min/max, or split-pair helper takes.")
  (input
    (do
      (def (divmod (: a Int64) (: b Int64)) #tuple((/ a b) (% a b)))
      (def (main (: a Int64) (: b Int64)) (let ((#tuple(q r) (divmod a b))) (+ (* 100 q) r)))
      (export main)))
  (call main (: 47 Int64) (: 10 Int64))
  (output (: 407 Int64)))

(case
  "a NESTED tuple-of-tuples let pattern destructures a call's runtime result to all four leaves"
  (doc
    "The depth-2 upgrade of the tuple-of-call destructure above (flat pair) composed with the
           nests-to-any-depth pin (const RHS): `make-pair` returns `((n, n+1), (2n, 3n))` — a runtime
           tuple OF tuples — and the binder `(tuple (tuple a b) (tuple c d))` reaches all FOUR leaves in
           one destructure: 2346 at n=2 (a=2,b=3,c=4,d=6). Each inner tuple is a live heap value the
           nested pattern must open at run time — the matrix-row / interval-pair return shape.")
  (input
    (do
      (def (make-pair (: n Int64)) #tuple(#tuple(n (+ n 1)) #tuple((* n 2) (* n 3))))
      (def
        (main (: n Int64))
        (let
          ((#tuple(#tuple(a b) #tuple(c d)) (make-pair n)))
          (+ (* 1000 a) (+ (* 100 b) (+ (* 10 c) d)))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2346 Int64)))

(case
  "a recursive fold returns a MIXED-representation tuple destructured at the caller"
  (doc
    "The multi-accumulator return shape: `stats` threads an Int64 sum AND a String rope through a
           recursive walk and returns `(tuple sum txt (List.len xs))` — an i64, a rope handle, and a
           second i64 in one product crossing the return. The caller destructures all three (6·100 +
           3·10 + 3 = 633). A return convention that boxed the scalar by the rope's slot kind (or
           mis-ordered mixed slots) corrupts a component — the RETURN-position companion of the
           mixed-representation generic and effects-argument pins.")
  (input
    (do
      (def
        (stats (: xs (List Int64)) (: i Int64) (: n Int64) (: sum Int64) (: txt String))
        (if
          (>= i n)
          #tuple(sum txt (List.len xs))
          (match
            (List.at xs i)
            ((Some v) (stats xs (+ i 1) n (+ sum v) (String.concat txt "x")))
            ((None u) #tuple(-1 txt -1)))))
      (def
        (main (: a Int64))
        (match
          (stats #list(a 2 3) 0 3 0 "")
          (#tuple(s t len) (+ (* 100 s) (+ (* 10 (String.byte-len t)) len)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 633 Int64))
  (live-objects known-leak))

; A RECORD binding pattern. A record is a fixed-shape product like a tuple, so `(record (x a) (y b))` in a
; binder position destructures the value BY FIELD — binding `a`/`b` to the `x`/`y` fields — with NO
; discriminant test, hence IRREFUTABLE iff each named field's sub-pattern is (core-semantics.md #A Binding
; Position Accepts An Irrefutable Pattern). Unlike a tuple, a record pattern is projected by NAME, so it
; MAY name fields in a different order than the value writes them and MAY name a SUBSET of the fields (a
; partial pattern). A field binder resolves to a PROJECTION of the bound value (`a` ≡ `(. v x)`), the
; record analogue of a tuple element's positional read.
(case
  "a let binder may be a record pattern that destructures by field"
  (doc
    "`(let (((record (x a) (y b)) (record (x 3) (y 4)))) (+ a b))` binds `a`/`b` to the `x`/`y` fields
           of the bound record (core-semantics.md #A Binding Position Accepts An Irrefutable Pattern) — the
           record analogue of the tuple destructure above, and the same binding a `(match r ((record (x a)
           (y b)) …))` arm would make. A record field is read by name, so this destructure reuses the
           ordinary member-access projection. `(+ a b)` = 7.")
  (input (let ((#record((= x a) (= y b)) #record((= x 3) (= y 4)))) (+ a b)))
  (output (: 7 Int64)))

(case
  "a record binding pattern binds fields by name, out of order and partial"
  (doc
    "A record pattern projects each field by NAME, not position: `(let (((record (z b) (a c)) (record
           (a 10) (z 20)))) …)` names `z`/`a` in the OPPOSITE order the value writes them and each binder
           still reads its own field — `c` = field `a` = 10, `b` = field `z` = 20 → 100*10+20 = 1020. This
           is the flexibility a tuple lacks (positional), and a partial pattern that names a subset of the
           fields is equally valid. Pins field-order-independence + partiality of a record binding pattern.")
  (input (let ((#record((= z b) (= a c)) #record((= a 10) (= z 20)))) (+ (* 100 c) b)))
  (output (: 1020 Int64)))

(case
  "a let record pattern destructures a helper CALL's runtime record result"
  (doc
    "The record twin of the tuple-of-call destructure: `stats` returns a RUNTIME record — `(record
           (lo …) (hi …))` ordering its two arguments — and the caller destructures it at the binder:
           `(let (((record (lo l) (hi h)) (stats a b))) …)`. (7,3) → lo 3, hi 7 → 307; (2,9) → 209. The
           literal-RHS record binder cases fold; here the record materializes from a live call and the
           by-NAME field binding must read the runtime heap record.")
  (input
    (do
      (def (stats (: a Int64) (: b Int64)) #record((= lo (if (< a b) a b)) (= hi (if (< a b) b a))))
      (def
        (main (: a Int64) (: b Int64))
        (let ((#record((= lo l) (= hi h)) (stats a b))) (+ (* 100 l) h)))
      (export main)))
  (call main (: 7 Int64) (: 3 Int64))
  (output (: 307 Int64))
  (call main (: 2 Int64) (: 9 Int64))
  (output (: 209 Int64)))

(case
  "a record MATCH pattern with a LITERAL field sub-pattern dispatches on the field value"
  (doc
    "A record pattern in MATCH position whose `tag` sub-pattern is a LITERAL — `((record (tag 1)
           (v x)) x)` — dispatches on the runtime field value while BINDING the other field: tag 1 → the
           bound `v` (10); tag 2 → the second arm's transform (100); anything else → the wildcard (-1).
           The field-access-then-match pin scrutinizes ONE projected field; this matches the WHOLE record
           with per-field literal/binder sub-patterns — the tagged-record dispatch idiom (a struct-like
           message with a discriminating field).")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          #record((= tag n) (= v 10))
          (#record((= tag 1) (= v x)) x)
          (#record((= tag 2) (= v x)) (* x 10))
          (_ -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 10 Int64))
  (call main (: 2 Int64))
  (output (: 100 Int64))
  (call main (: 9 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a def parameter may be a record pattern that destructures by field"
  (doc
    "`(def (f (record (x a) (y b))) (+ a b))` destructures its single record argument by field,
           keeping arity 1 (core-semantics.md #A Binding Position Accepts An Irrefutable Pattern: a
           destructuring parameter occupies one argument position and names its parts). The parameter
           desugars to a destructuring `let`, so `a`/`b` read the runtime record's fields — the same
           binding-path the let-binder cases above exercise. `(f (record (x 3) (y 4)))` = 7. The record twin
           of the list-rest / tuple param patterns; its ML surface `def f({ x = a, y = b })` round-trips now
           that v-syntax admits a record pattern in a parameter slot (the surface gap this case waited on).")
  (input
    (do
      (def (f #record((= x a) (= y b))) (+ a b))
      (def (main) (f #record((= x 3) (= y 4))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a tuple-destructuring fn (lambda) parameter binds its parts"
  (doc
    "The `fn` (lambda) face of the destructuring parameter: core-semantics.md #A Binding Position
           Accepts An Irrefutable Pattern names \"a `let` binder, a function or `fn` parameter\" — so a `fn`
           parameter accepts an irrefutable tuple pattern exactly as a `def` parameter does (the case above)
           and a `let` binder does. `(fn ((tuple x y)) (+ (* x 10) y))` binds `x`/`y` from the tuple argument;
           `(f (tuple 3 4))` = 3*10+4 = 34. Before the lambda-param desugar reached this position the fn face
           rejected CDZ0101 (the pattern's names fell through to scoping, unbound) while the def-param + let
           faces worked — this pins the fn face now binds, closing the binding-position family across all three
           positions the spec names (v-inference lambda-param desugar).")
  (input
    (do
      (def (main (: a Int64)) (let ((f (fn (#tuple(x y)) (+ (* x 10) y)))) (f #tuple(a 4))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 34 Int64)))

(case
  "a record binding pattern naming an absent field is rejected"
  (doc
    "The contrast: a record binding pattern that names a field the bound value's record type does NOT
           have is a type mismatch (CDZ0203), not a silent miss. `(let (((record (nope a)) (record (x 3) (y
           4)))) a)` names `nope`, absent from `(Record (: x Int64) (: y Int64))`. Pins that a record binding
           pattern's fields must exist on the value — the record analogue of a tuple pattern's arity check.")
  (input (let ((#record((= nope a)) #record((= x 3) (= y 4)))) a))
  (error CDZ0203))

(case
  "a destructuring record let over a runtime value binds its fields"
  (doc
    "`(def (f p) (let (((record (x a) (y b)) p)) (+ a b)))` destructures the RUNTIME parameter `p` (a
           record built by `(mk 10)`, not a literal) at the binder, then `(f (mk 10))` reads its fields at
           run time — `a`+`b` = 10+11 = 21 (core-semantics.md #A Binding Position Accepts An Irrefutable
           Pattern). Pins that a record destructure reads the bound value's fields at RUN TIME, not only
           when the record folds to a constant — the record companion of the runtime tuple/list destructures.")
  (input
    (do
      (def (mk n) #record((= x n) (= y (+ n 1))))
      (def (f p) (let ((#record((= x a) (= y b)) p)) (+ a b)))
      (def (main) (f (mk 10)))
      (export main)))
  (output (: 21 Int64)))

(case
  "a record binding pattern may leave a field's value a wildcard"
  (doc
    "A record field's value sub-pattern MAY be a wildcard `_`, which binds nothing — `(let (((record
           (x a) (y _)) (record (x 7) (y 4)))) a)` binds only `a` = 7 and ignores `y` (core-semantics.md #A
           Binding Position Accepts An Irrefutable Pattern: `_` is a trivial irrefutable sub-pattern). This
           is the field-level companion of the partial pattern (which OMITS a field) — here the field is
           named but its value discarded. Pins that a wildcard field value is irrefutable and binds nothing.")
  (input (let ((#record((= x a) (= y _)) #record((= x 7) (= y 4)))) a))
  (output (: 7 Int64)))

(case
  "a later let binding sees an earlier record pattern's field binders"
  (doc
    "`(let (((record (x a) (y b)) (record (x 3) (y 4))) (c (* a b))) c)` — the second binding's
           initializer `(* a b)` references `a`/`b`, the field binders the first (record-destructuring)
           binding introduced (core-semantics.md #The Bindings Of One `let` Take Effect In Order). `a`*`b` =
           3*4 = 12. The record twin of the tuple case above — pins that record field binders are in scope
           for the bindings that follow.")
  (input (let ((#record((= x a) (= y b)) #record((= x 3) (= y 4))) (c (* a b))) c))
  (output (: 12 Int64)))

(case
  "a let binder may be a single-variant-sum pattern that destructures the payload"
  (doc
    "A SINGLE-VARIANT sum's sole constructor ALWAYS matches, so it is an IRREFUTABLE pattern — valid
           in a `let` binder position (core-semantics.md #A Binding Position Accepts An Irrefutable
           Pattern), exactly as a tuple pattern is. `(let (((Id.Mk n) (Id.Mk 42))) n)` binds `n` to the
           `Mk` payload — the same binding a `(match (Id.Mk 42) ((Id.Mk n) n))` arm makes, written at the
           binder. Pins that a one-variant sum destructures in a binding position (a MULTI-variant sum
           there is refutable → CDZ0210, the rejection below), the sum companion of the tuple destructure.")
  (input (do (type Id (Mk Int64)) (def (main) (let (((Id.Mk n) (Id.Mk 42))) n)) (export main)))
  (output (: 42 Int64)))

(case
  "a single-variant-sum binding pattern destructures a multi-payload constructor positionally"
  (doc
    "The multi-payload companion: `(let (((P.Mk a b) (P.Mk 5 6))) (+ a b))` binds `a` and `b` to the
           two payloads of the single-variant `P.Mk` (its payloads box as one tuple, matched positionally,
           exactly as a `(P.Mk a b)` match arm does). Pins that a single-variant binding pattern binds each
           payload position, not only a one-payload newtype.")
  (input
    (do
      (type P (Mk Int64 Int64))
      (def (main) (let (((P.Mk a b) (P.Mk 5 6))) (+ a b)))
      (export main)))
  (output (: 11 Int64)))

(case
  "a single-variant-sum binding pattern nests inside another"
  (doc
    "A single-variant pattern nests, like a tuple one: `(let (((W.Wrap (Id.Mk n)) (W.Wrap (Id.Mk 9))))
           …)` destructures the outer `Wrap` then the inner `Mk`, binding `n` two payload levels deep
           (core-semantics.md #Patterns Compose). `n + 1` = 10.")
  (input
    (do
      (type Id (Mk Int64))
      (type W (Wrap Id))
      (def (main) (let (((W.Wrap (Id.Mk n)) (W.Wrap (Id.Mk 9)))) (+ n 1)))
      (export main)))
  (output (: 10 Int64)))

(case
  "a multi-variant-sum binding pattern is refutable and rejected"
  (doc
    "The contrast to the single-variant cases above: a MULTI-variant sum's constructor pattern in a
           binding position is REFUTABLE — the other variants are uncovered and there is no alternative arm
           — so it is rejected (CDZ0210), not accepted. `(let (((C.A n) (C.A 5))) n)` over `(type C (A
           Int64) B)` leaves `B` uncovered. Only a single-variant sum earns the binding-position exemption;
           a many-variant sum's destructure must be a `match`. Pins the refutability boundary.")
  (input (do (type C (A Int64) B) (def (main) (let (((C.A n) (C.A 5))) n)) (export main)))
  (error CDZ0210))

(case
  "a later let binding sees an earlier pattern's binders"
  (doc
    "`(let (((tuple a b) (tuple 3 4)) (c (+ a b))) c)` — the second binding's initializer `(+ a b)`
           references `a` and `b`, the binders the first (destructuring) binding introduced
           (core-semantics.md #The Bindings Of One `let` Take Effect In Order: each initializer observes the
           bindings written before it). Pins that a destructuring binder is in scope for the bindings that
           follow, the multi-binding-let idiom the decoder threads.")
  (input (let ((#tuple(a b) #tuple(3 4)) (c (+ a b))) c))
  (output (: 7 Int64)))

(case
  "a destructuring let over a runtime value binds its parts"
  (doc
    "`(def (f p) (let (((tuple a b) p)) (+ a b)))` destructures the RUNTIME parameter `p` (not a
           literal tuple) at the binder, then `(f (tuple 10 20))` = 30 (core-semantics.md #A Binding
           Position Accepts An Irrefutable Pattern). Pins that the destructure reads the bound value at run
           time, not only when it folds to a constant.")
  (input
    (do (def (f p) (let ((#tuple(a b) p)) (+ a b))) (def (main) (f #tuple(10 20))) (export main)))
  (output (: 30 Int64)))

; A LIST binding pattern. A list pattern is irrefutable ONLY in the ZERO-LEADING rest form `(list .. rest)`
; — that form matches EVERY list (the empty list included), binding `rest` to the whole list, so it may
; bind in a `let` binder or a `def`/`fn` parameter (the rest binder resolves to `SumPayload{RestFrom(0)}`
; reading the whole bound value; core-semantics.md #A Binding Position Accepts An Irrefutable Pattern / #A
; List Is Deconstructed By Element Patterns With An Optional Rest). A LEADING-element rest `(list a .. rest)`
; is REFUTABLE — it requires at least one element, so it does NOT match the empty list (§147: only the
; zero-leading form matches every list) → CDZ0210 in a binding position, exactly like the FIXED-ARITY
; `(list a b)` form (which matches only its exact length). A possibly-empty leading-element destructure
; belongs in a `match`, whose arms cover the empty case. Both refutable forms are the rejections below.
(case
  "a zero-leading list rest pattern binds the whole list in a def parameter"
  (doc
    "`(def (all (list .. rest)) rest)` — the ONLY irrefutable list binding form: `(list .. rest)` with
           NO leading element matches EVERY list (empty included), binding `rest` to the whole list
           (core-semantics.md #A Binding Position Accepts An Irrefutable Pattern). The parameter desugars to
           a destructuring `let` and `rest` resolves to `SumPayload{RestFrom(0)}` = the whole runtime list;
           `sum` over it folds `(list 7 8 9)` → 24. Pins that the zero-leading rest form earns the
           binding-position exemption (a leading-element rest does not — see the CDZ0210 case below).")
  (input
    (do
      (def (sum (: xs (List Int64))) (match xs (#list() 0) (#list(x (.. rest)) (+ x (sum rest)))))
      (def (all (: ys (List Int64))) (let ((#list((.. rest)) ys)) (sum rest)))
      (def (main) (all #list(7 8 9)))
      (export main)))
  (output (: 24 Int64))
  (live-objects 0))

(case
  "a leading-element list rest pattern in a def parameter is refutable and rejected"
  (doc
    "`(def (head (list x .. rest)) x)` binds a LEADING element `x` before the rest — a REFUTABLE
           pattern: `(list x .. rest)` requires at least one element, so it does NOT match the EMPTY list
           (core-semantics.md §147 — only the zero-leading `(list .. rest)` matches every list). A binding
           position has no alternative arm, so a refutable pattern MUST be a compile-time error: CDZ0210
           (§139), the same rule the fixed-arity form gets. Were it accepted (the earlier unsound behavior),
           `(head (list))` would TRAP at runtime reading element 0 of an empty list — a fault the type
           system must reject up front. A leading-element destructure of a possibly-empty list belongs in a
           `match`. Pins that ONLY the zero-leading rest form is irrefutable in a binding position.")
  (input (do (def (head #list(x (.. rest))) x) (def (main) (head #list(7 8 9))) (export main)))
  (error CDZ0210))

(case
  "a leading-element list rest pattern in a let binder is refutable and rejected"
  (doc
    "The `let` twin of the parameter rejection: `(let (((list a b .. rest) ys)) …)` binds two LEADING
           elements before the rest, so it does not match lists shorter than 2 (nor the empty list) — a
           REFUTABLE pattern in a binding position → CDZ0210 (core-semantics.md §139/§147). The spec-correct
           idiom for a possibly-short list is a `match` with an empty/short arm, not a `let` destructure.
           Pins the leading-element rest boundary in the `let` position, mirroring the parameter case.")
  (input (do (def (main) (let ((#list(a b (.. rest)) #list(1 2 3 4))) a)) (export main)))
  (error CDZ0210))

(case
  "a fixed-arity list binding pattern is refutable and rejected"
  (doc
    "The contrast to the rest form: a FIXED-ARITY `(list a b)` binding pattern matches ONLY lists of
           that exact length, so it is REFUTABLE — a binding position has no alternative arm, so it is the
           non-exhaustive error the equivalent single-arm match raises (CDZ0210, core-semantics.md #A
           Binding Position Accepts An Irrefutable Pattern). Only the rest form `(list p… .. rest)`, which
           matches any length ≥ the leading count, earns the binding-position exemption; a length-fixed
           destructure must be a `match`. Pins the list refutability boundary.")
  (input (do (def (main) (let ((#list(a b) #list(1 2))) (+ a b))) (export main)))
  (error CDZ0210 (message "ZERO-LEADING") (message "is itself refutable")))

; Further ill-formed list bindings (migrated from rcdzc an_ill_formed_list_binding_pattern_is_rejected): an
; EMPTY `#list()` binder matches only the empty list (refutable, CDZ0210); a REST binding with a refutable
; LEADING element — a literal `#list(0 .. rest)` — is CDZ0210 (the rest exemption covers length, not a
; refutable leading element); a NON-LINEAR list binder `#list(a a .. rest)` repeats `a` → CDZ0102
; (linearity is checked BEFORE the leading-element refutability guard, so the non-linear code wins).
(case
  "an empty list binding pattern is refutable and rejected"
  (input (do (def (main) (let ((#list() #list())) 0)) (export main)))
  (error CDZ0210))

(case
  "a rest binding with a refutable literal leading element is rejected"
  (input (do (def (main) (let ((#list(0 (.. rest)) #list(0 1))) 42)) (export main)))
  (error CDZ0210))

(case
  "a non-linear list binding pattern is rejected CDZ0102 before the refutability guard"
  (input (do (def (main) (let ((#list(a a (.. rest)) #list(1 2 3))) a)) (export main)))
  (error CDZ0102))

(case
  "a non-linear tuple match pattern is rejected CDZ0102 with a rename fix"
  (doc
    "A repeated binder in a MATCH pattern — `(#tuple(a a) a)` binds `a` twice — is the nonlinear reject
           CDZ0102, and it carries the mechanical repair: RENAME the repeated binder to a fresh
           non-colliding name (`a` → `a2`), making the pattern linear. Heuristic (unverified — the rename
           clears the hard error but the fresh binder is then unused until the author wires it up). The
           match-pattern companion of the non-linear list-binder and duplicate-parameter cases. (Migrated
           from rcdzc a_non_linear_pattern_binder_carries_a_rename_fix.)")
  (input (match #tuple(1 2) (#tuple(a a) a)))
  (error CDZ0102 (fix (kind replace) (replacement "a2") (unverified))))

; A misspelled variant ctor as a LIST-ELEMENT pattern draws its did-you-mean from the element sum's
; variants: `#list((Ad) .. r)` on `(List Op)` where `(type Op (Add) (Sub))` — a near-miss `Ad` for `Add` is
; CDZ0201 "did you mean `Add`?" + a rename fix on the variant name. A FAR miss lists the closest matches
; (no baseless fix). (Migrated from rcdzc a_misspelled_variant_in_a_list_element_pattern_suggests_the_near_variant.)
(case
  "a misspelled variant in a list-element pattern suggests the near variant with a rename fix"
  (input
    (do
      (type Op (Add) (Sub))
      (def (f (: xs (List Op))) (match xs (#list((Ad) (.. r)) 1) (_ 0)))
      (export f)))
  (error CDZ0201 (message "did you mean `Add`?") (fix (kind replace) (replacement "Add"))))

(case
  "a far-miss variant in a list-element pattern lists the closest matches with no fix"
  (input
    (do
      (type Op (Add) (Sub))
      (def (f (: xs (List Op))) (match xs (#list((Zzz) (.. r)) 1) (_ 0)))
      (export f)))
  (error CDZ0201 (message "closest matches") (no-fix)))

(case
  "a constructor-pattern list element over a NON-SUM element type gets no spurious variant suggestion"
  (doc
    "The variant did-you-mean fires only when the element type is a SUM whose variant set the closed
           suggestion pool can search. A `(Foo)` constructor pattern over a NON-SUM element type (here the
           list is `(List Int64)`, a scalar) is the ordinary 'not a tuple, record, or constructor' reject
           (CDZ0201) and carries NO spurious variant did-you-mean — there is no sum to draw a candidate from.
           Pinned by `(not \"did you mean\")`. (Migrated from rcdzc
           a_non_sum_list_element_pattern_gets_no_spurious_variant_suggestion.)")
  (input (do (def (f (: xs (List Int64))) (match xs (#list((Foo) (.. r)) 1) (_ 0))) (export f)))
  (error CDZ0201 (message "constructor") (not "did you mean")))

; The refutable / ill-shaped / non-linear rejections. A binding position has no alternative arm, so its
; pattern MUST be irrefutable and its shape MUST match the value's type (core-semantics.md #A Binding
; Position Accepts An Irrefutable Pattern).
(case
  "a refutable constructor pattern in a let binder is rejected"
  (doc
    "`(let (((Some x) (Some 5))) x)` — a `Some` pattern is refutable (the `None` variant is
           uncovered), and a binding position has no alternative arm, so it is the non-exhaustive error the
           equivalent single-arm `(match (Some 5) ((Some x) x))` raises: CDZ0210 (core-semantics.md #A
           Binding Position Accepts An Irrefutable Pattern / #Matching Is Exhaustive Or Rejected). Pins that
           a multi-variant constructor cannot bind a value in a `let`.")
  (input (let (((Some x) (Some 5))) x))
  (error CDZ0210))

(case
  "a literal in a let binder is refutable and rejected"
  (doc
    "`(let ((0 5)) 42)` — a literal pattern matches one value, not every value of its type, so it is
           refutable and rejected in a binding position (CDZ0210, core-semantics.md #A Binding Position
           Accepts An Irrefutable Pattern). Pins that a literal cannot stand where a binder is expected.")
  (input (do (def (main) (let ((0 5)) 42)) (export main)))
  (error CDZ0210))

; Refutability is checked RECURSIVELY, at every nesting depth — a refutable sub-pattern nested inside a
; tuple binding position is rejected exactly as the top-level one is (core-semantics.md #A Binding Position
; Accepts An Irrefutable Pattern: "a tuple pattern is irrefutable ONLY when every element is"). The
; refutability check must not stop at the top level: a literal or multi-variant-constructor element makes
; the whole binding refutable, so it is CDZ0210, not a silent no-op that drops the refutable sub-pattern.
(case
  "a literal nested in a tuple let-binder is refutable and rejected"
  (doc
    "`(let (((tuple 0 b) (tuple 0 9))) b)` puts the literal `0` in the first element of a tuple
           BINDING pattern. A literal is refutable, so a binding position rejects it (CDZ0210) exactly as the
           top-level `(let ((0 5)) 42)` does — the check recurses into tuple sub-patterns. A compiler that
           stopped at the top level ran it to 9, silently treating the literal element as a no-op.")
  (input (do (def (main) (let ((#tuple(0 b) #tuple(0 9))) b)) (export main)))
  (error CDZ0210))

(case
  "a literal nested in a tuple def-parameter is refutable and rejected"
  (doc
    "`(def (f (tuple 0 b)) b)` — a tuple-pattern parameter desugars to a `(let (((tuple 0 b) p)) …)`
           binder, so the literal `0` in the first element is refutable and rejects CDZ0210. Calling
           `(f (tuple 9 5))` with a first element that does NOT equal 0 must not run to 5 (no compile
           rejection, no runtime trap) — the parameter's binding position enforces irrefutability like a
           `let` binder.")
  (input (do (def (f #tuple(0 b)) b) (def (main) (f #tuple(9 5))) (export main)))
  (error CDZ0210))

; The top-level constructor + non-linear faces of a def-parameter binding (migrated from rcdzc
; an_ill_formed_def_parameter_pattern_is_rejected): a refutable multi-variant CONSTRUCTOR parameter is
; CDZ0210 (the None variant is uncovered — the binding position has no fall-through arm); a NON-LINEAR
; tuple parameter (a binder repeated inside the tuple pattern) is CDZ0102, checked before refutability.
(case
  "a refutable constructor def-parameter is rejected"
  (input (do (def (f (Some x)) x) (def (main) 0) (export main)))
  (error CDZ0210))

(case
  "a non-linear tuple def-parameter is rejected CDZ0102"
  (input (do (def (f #tuple(x x)) x) (def (main) 0) (export main)))
  (error CDZ0102))

; The INLINE-LAMBDA face: a refutable binding inside an INLINE / let-bound lambda's body or parameter is the
; same CDZ0210 a def-body binding gets (a lambda parameter desugars to a body `let`). An earlier over-accept
; let these through — an inline lambda body was not walked for binding refutability — so these pin that the
; irrefutability check reaches INSIDE inline lambdas, to any nesting depth, while a generic/irrefutable body
; stays clean. (migrated from rcdzc a_refutable_binding_pattern_inside_an_inline_lambda_rejects_cdz0210.)
(case
  "a refutable let inside an inline lambda body is rejected"
  (input (do (def (main) (let ((f (fn (p) (let (((Some x) p)) x)))) (f (Some 3)))) (export main)))
  (error CDZ0210))

(case
  "a refutable constructor parameter on an inline lambda is rejected"
  (input (do (def (main) (let ((f (fn ((Some x)) x))) (f (Some 3)))) (export main)))
  (error CDZ0210))

(case
  "a refutable let nested two inline-lambda levels deep is rejected"
  (input
    (do
      (def (main) (let ((outer (fn (x) (let ((inner (fn (z) (let ((5 y)) y)))) 3)))) 9))
      (export main)))
  (error CDZ0210))

(case
  "an irrefutable tuple parameter on an inline lambda is legal and applies (the control)"
  (input (do (def (main) (let ((f (fn (#tuple(a b)) (+ a b)))) (f #tuple(3 4)))) (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a generic bare-parameter inline lambda compiles clean (no spurious binding fault at depth)"
  (input (do (def (main) (let ((id (fn (x) x))) (id 5))) (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a multi-variant constructor nested in a tuple let-binder is refutable and rejected"
  (doc
    "`(let (((tuple (Some x) b) (tuple (Some 5) 9))) x)` puts the multi-variant constructor pattern
           `(Some x)` in a tuple binding element. A multi-variant ctor is refutable (the `None` variant is
           uncovered) — the top-level `(let (((Some x) (Some 5))) x)` rejects CDZ0210, so the nested form
           does too. The recursion classifies each element with the same rule the top-level binder uses.")
  (input (do (def (main) (let ((#tuple((Some x) b) #tuple((Some 5) 9))) x)) (export main)))
  (error CDZ0210))

(case
  "a deeply nested literal in a tuple let-binder is refutable and rejected"
  (doc
    "`(let (((tuple a (tuple 0 b)) (tuple 1 (tuple 0 3)))) (+ a b))` — the literal `0` is TWO tuple
           levels deep, in the second element's own tuple pattern. Refutability recurses to any depth, so
           the deep literal is CDZ0210 exactly as a top-level one is. Pins that the recursion does not stop
           after one tuple level (contrast the irrefutable `(tuple a (tuple b c))` binder above, which
           composes to any depth and RUNS).")
  (input
    (do (def (main) (let ((#tuple(a #tuple(0 b)) #tuple(1 #tuple(0 3)))) (+ a b))) (export main)))
  (error CDZ0210))

(case
  "a wrong-arity tuple binding pattern is a shape error"
  (doc
    "`(let (((tuple a b c) (tuple 1 2))) a)` — a three-element tuple pattern cannot match a
           two-element value: a static shape mismatch (CDZ0201, core-semantics.md #A Binding Position
           Accepts An Irrefutable Pattern), the same code the wrong-arity tuple MATCH arm gets. Pins that a
           binding pattern's arity is checked against the bound value's type.")
  (input (let ((#tuple(a b c) #tuple(1 2))) a))
  (error
    CDZ0201
    (message "this tuple pattern binds 3 elements, but the value is a tuple with 2 elements")))

(case
  "a tuple binding pattern over a non-tuple value is a shape error"
  (doc
    "`(let (((tuple a b) 5)) a)` destructures a tuple pattern over a NON-tuple value (Int64) — a shape
           mismatch, CDZ0201. The message says the tuple pattern cannot destructure a value of type Int64
           (it does NOT call the bound value a `payload`, the earlier conflated phrasing). Distinct from the
           wrong-ARITY case above (a tuple value of the wrong length). (migrated from rcdzc
           an_ill_formed_let_binding_pattern_is_rejected_not_miscompiled.)")
  (input (do (def (main) (let ((#tuple(a b) 5)) a)) (export main)))
  (error CDZ0201 (message "this tuple pattern cannot destructure a value of type Int64")))

; A `let`/`fn` takes EXACTLY ONE body — `(let (binds) b1 b2)` / `(fn (params) b1 b2)` with a trailing form
; is malformed (the surplus form was silently DROPPED — a miscompile). Rejected CDZ0201 naming the form +
; the `(do …)` sequencing hint, with a delete-the-surplus fix. A single body, or one body that is itself a
; `(do …)` sequence, is well-formed. (migrated from rcdzc a_let_or_fn_with_more_than_one_body_is_cdz0201.)
(case
  "a let with more than one body is rejected with a delete-the-surplus fix"
  (input (do (def (main) (let ((x 1)) x 99)) (export main)))
  (error CDZ0201 (message "more than one body") (fix (kind delete))))

(case
  "an inline lambda with more than one body is rejected"
  (input (do (def (main) ((fn (x) x 99) 5)) (export main)))
  (error CDZ0201 (message "more than one body")))

(case
  "a single-body let whose one body is a (do …) sequence is well-formed and runs"
  (input (do (def (main) (let ((x 1)) (do (+ x 1) x))) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a tuple binding pattern against a non-tuple value is a shape error"
  (doc
    "`(let (((tuple a b) 5)) a)` — a tuple pattern cannot match a scalar `Int64` value: a kind
           mismatch (CDZ0201, core-semantics.md #A Binding Position Accepts An Irrefutable Pattern). Pins
           that a tuple binding pattern requires a tuple value.")
  (input (let ((#tuple(a b) 5)) a))
  (error CDZ0201))

(case
  "a non-linear tuple binding pattern is rejected"
  (doc
    "`(let (((tuple x x) (tuple 1 2))) x)` binds `x` twice in one binding pattern — not linear, so it
           is the same CDZ0102 error a non-linear MATCH pattern gets (core-semantics.md #A Binding Position
           Accepts An Irrefutable Pattern / #Bindings Introduced By A Pattern Are Scoped To Its Branch).
           Pins that linearity is enforced in binding position, not only in a match arm.")
  (input (let ((#tuple(x x) #tuple(1 2))) x))
  (error CDZ0102))

; A binding pattern MAY carry a type ANNOTATION `(: <pat> <Type>)` (type-system.md #Annotations Constrain,
; Never Contradict): the annotation constrains the bound value's type and the inner pattern is the real
; binder. A contradiction is CDZ0203, the same code any annotation-vs-value mismatch gets.
(case
  "an annotated let binder constrains the value's type"
  (doc
    "`(let (((: x Int64) 5)) x)` — the binder `x` is annotated `Int64`, which agrees with the value
           `5`, so `x` binds 5 (type-system.md #Annotations Constrain, Never Contradict). Pins that a `let`
           binder MAY carry a `(: <name> <Type>)` annotation, the binder analogue of an annotated
           parameter `(def (f (: x Int64)) …)`.")
  (input (let (((: x Int64) 5)) x))
  (output (: 5 Int64)))

(case
  "an annotated destructuring let binder"
  (doc
    "`(let (((: (tuple a b) (Tuple Int64 Int64)) (tuple 3 4))) (+ a b))` — the annotation constrains
           the whole tuple before the pattern takes it apart, then `a`/`b` bind its elements (7). Pins that
           the annotation wraps a DESTRUCTURING binder, not only a bare name.")
  (input (let (((: #tuple(a b) (Tuple Int64 Int64)) #tuple(3 4))) (+ a b)))
  (output (: 7 Int64)))

(case
  "an annotated let binder that contradicts the value is rejected"
  (doc
    "`(let (((: x Bool) 5)) x)` annotates `x` `Bool` but binds it to the Int64 `5` — a contradiction
           the compiler MUST reject (CDZ0203, type-system.md #Annotations Constrain, Never Contradict: an
           annotation participates in inference as a constraint, and a value that cannot satisfy it is a
           type error). Pins that a binder's annotation is CHECKED against the value, not merely recorded.")
  (input (do (def (main) (let (((: x Bool) 5)) x)) (export main)))
  (error CDZ0203))

(case
  "an annotated destructuring let binder that contradicts an element type is rejected"
  (doc
    "`(let (((: (tuple a b) (Tuple Int64 Bool)) (tuple 3 4))) a)` annotates the pair `(Tuple Int64 Bool)`
           but binds it to `(tuple 3 4)`, whose second element `4` is Int64, not Bool — a per-ELEMENT
           contradiction the compiler MUST reject (CDZ0203). The destructuring companion of the scalar Bool/Int
           contradiction above (and the negative of the positive destructuring binder `(Tuple Int64 Int64)`): an
           annotation on a DESTRUCTURING binder is checked element-wise against the value, not only whole-shape.")
  (input (let (((: #tuple(a b) (Tuple Int64 Bool)) #tuple(3 4))) a))
  (error CDZ0203))

(case
  "an annotated let binder narrower than its literal value is rejected (int width)"
  (doc
    "`(let (((: x Int8) 999)) x)` — the binder's `Int8` annotation grounds the bound literal, and 999
           overflows Int8 (valid range -128..=127) → CDZ0302 'does not fit', exactly as `(: 999 Int8)` is.
           The width companion of the Bool/Int contradiction above: a binder annotation is a WIDTH constraint
           on the value too, not only a type-shape constraint — without the check the binding would smuggle
           an out-of-range value under a narrow name.")
  (input (let (((: x Int8) 999)) x))
  (error CDZ0302))

(case
  "an annotated let binder narrower than its literal value is rejected (float width)"
  (doc
    "The float twin: `(let (((: x Float32) 1.0e300)) x)` — `1.0e300` is finite as Float64 but overflows
           binary32 (as an f32 it would be ±inf, a malformed value with no written form) → CDZ0302. Pins that
           the binder-annotation width check covers FLOAT widths as well as integer ones. The fitting twin
           below computes — the check must not over-reject.")
  (input
    (let
      (((: x Float32)
          1000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000.0))
      x))
  (error CDZ0302))

(case
  "an annotated let binder at a fitting narrow float computes"
  (doc
    "The no-over-reject control: `(let (((: x Float32) 0.5)) x)` — 0.5 is exactly representable in
           binary32, so the binder annotation grounds the literal at Float32 and the binding computes → 0.5
           at Float32. Guards the width check above against rejecting every narrow-float binder.")
  (input (let (((: x Float32) 0.5)) x))
  (output (: 0.5 Float32)))

; A FUNCTION PARAMETER is a binding position too (core-semantics.md #A Binding Position Accepts An
; Irrefutable Pattern): `(def (f (tuple a b)) …)` names the two halves of its single pair argument, keeping
; ARITY ONE. The compiler realizes this by a load-time rewrite to a fresh whole-value parameter + a
; destructuring `let` over the body — the SAME desugar the annotated variant `(: (tuple a b) T)` takes,
; keeping the annotation on the fresh binder and the tuple destructuring on its value. The ML syntax
; surface parses `def f((a, b)) = …` and its annotated form `def f((a, b): T) = …`, so these survive the
; `sexpr → ml → sexpr` round-trip gate.
(case
  "a tuple-pattern parameter binds the halves of its pair argument"
  (doc
    "`(def (f (tuple a b)) (+ a b))` — a destructuring parameter names the two elements of its single
           pair argument, keeping arity one, exactly as the equivalent `let` binder `(let (((tuple a b) p))
           …)` does. Calling `(f (tuple 3 4))` binds `a`=3, `b`=4 and yields 7. Pins the parameter face of
           the binding-pattern capability the `let` cases above witness.")
  (input (do (def (f #tuple(a b)) (+ a b)) (def (main) (f #tuple(3 4))) (export main)))
  (output (: 7 Int64)))

; The tuple-pattern-parameter cases here pass a CONSTANT tuple `(f (tuple 3 4))` from a nullary entry, so
; the tuple folds and the destructure is compile-time. These pin the RUNTIME face: the argument tuple is
; built from a boundary parameter (so it cannot fold — a real heap tuple), and the parameter pattern
; destructures it at run time (a `tuple<…>` read back into its binders). The runtime companion of the
; constant destructure, the shape a compiler pass takes when a callee receives a (node, cursor) pair.
(case
  "a tuple-pattern parameter destructures a runtime-built tuple"
  (doc
    "`(def (add (tuple a b)) (+ a b))` applied to a tuple built from a boundary parameter
           `(add (tuple x (+ x 1)))` — the tuple cannot fold, so `add`'s parameter destructures a real heap
           tuple at run time, binding `a`=x and `b`=x+1. x=5 → 5+6 = 11; x=100 → 201. Pins the runtime face
           of tuple-parameter destructuring, distinct from the constant `(f (tuple 3 4))` fold above.")
  (input
    (do
      (def (add #tuple(a b)) (+ a b))
      (def (main (: x Int64)) (add #tuple(x (+ x 1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64))
  (call main (: 100 Int64))
  (output (: 201 Int64)))

(case
  "a nested tuple-pattern parameter destructures a runtime tuple"
  (doc
    "The nested form: `(def (f (tuple a (tuple b c))) …)` destructures a runtime `(tuple x (tuple (+ x
           1) (+ x 2)))` — the outer pair's second element is itself a pair, bound `b`=x+1, `c`=x+2. x=5 →
           5 + (6 + 7) = 18. Pins that a nested destructuring parameter reads a nested heap tuple at run
           time, its inner binders resolving down the extended access path.")
  (input
    (do
      (def (f #tuple(a #tuple(b c))) (+ a (+ b c)))
      (def (main (: x Int64)) (f #tuple(x #tuple((+ x 1) (+ x 2)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 18 Int64)))

(case
  "a tuple-pattern parameter over a tuple threaded from a helper call"
  (doc
    "The destructured tuple arrives from ANOTHER function's return, not built inline: `mk(x)` returns
           `(tuple x (- 0 x))` and `sum`'s tuple parameter destructures it — `(sum (mk x))` = x + (-x) = 0
           for every x. Pins that a callee's tuple-pattern parameter destructures a tuple produced by a
           prior call (the (node, cursor)-pair-threaded-through-a-pass shape), the return-boundary companion
           of the inline runtime destructure.")
  (input
    (do
      (def (mk (: x Int64)) #tuple(x (- 0 x)))
      (def (sum #tuple(a b)) (+ a b))
      (def (main (: x Int64)) (sum (mk x)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 40 Int64))
  (output (: 0 Int64)))

; A TUPLE-pattern parameter MAY end in a trailing `.. rest`: a tuple's arity is fixed and statically known,
; so a leading-element-plus-rest tuple pattern is IRREFUTABLE in a binding position (it matches every tuple
; of the parameter's type) and is ACCEPTED — binding the leading element(s) and the trailing sub-tuple to
; `rest`. Witnesses core-semantics.md §"A Binding Position Accepts An Irrefutable Pattern" (v-spec-oracle
; #6723): unlike a LEADING-rest list pattern (which does not match the empty list → CDZ0210) or a keyed-map
; pattern (refutable → CDZ0210), a tuple trailing-rest is total. `(def (f #tuple(a .. rest)) …)` over
; `#tuple(3 4 5)` binds `a`=3 and `rest`=`(tuple 4 5)`. (Formerly the binding-position path did not recognize
; a tuple-with-rest head and gave a spurious CDZ0201 "not a tuple/record/constructor"; fixed by
; v-ast-compound's check_binding_pattern. The MATCH-arm form is pinned in 05.)
(case
  "a tuple trailing-rest parameter binds the leading element (irrefutable binding position)"
  (input (do (def (f #tuple(a (.. rest))) a) (def (main) (f #tuple(3 4 5))) (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a tuple trailing-rest parameter binds the trailing sub-tuple to rest"
  (input (do (def (f #tuple(a (.. rest))) (. rest 0)) (def (main) (f #tuple(3 4 5))) (export main)))
  (call main)
  (output (: 4 Int64)))

(case
  "a signature mixing a plain parameter and a tuple-pattern parameter binds both"
  (doc
    "A def signature may MIX an ordinary name parameter with a destructuring tuple-pattern parameter:
           `(def (f x (tuple a b)) (+ x (+ a b)))` — `x` is a plain binder and `(tuple a b)` destructures
           the SECOND argument. Keeping arity two (one per parameter), `x` and the tuple's halves all bind
           without disturbing each other's slots. `(f x (tuple 2 4))` at x=10 = 10 + (2+4) = 16. Pins that a
           plain binder and a destructuring binder coexist in one parameter list.")
  (input
    (do
      (def (f x #tuple(a b)) (+ x (+ a b)))
      (def (main (: x Int64)) (f x #tuple(2 4)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 16 Int64)))

(case
  "an annotated tuple-pattern parameter binds its pattern's names"
  (doc
    "`(def (f (: (tuple a b) (Tuple Int64 Int64))) (+ a b))` is a destructuring tuple parameter that
           ALSO carries a type annotation. Its binders `a`/`b` must be in scope in the body, exactly as the
           un-annotated `(def (f (tuple a b)) …)` binds them. The annotated form desugars to a fresh
           annotated binder `(: p T)` plus a destructuring `let` over the inner tuple, so the annotation
           constrains the argument AND the halves bind. Calling `(f (tuple 3 4))` gives 7. (Without peeling
           the `(: pattern T)` annotation the desugar left `a`/`b` unbound — CDZ0101 — even though the
           un-annotated and the annotated-plain-binder forms both work; only their combination broke, and
           the ML printer emits exactly this form.)")
  (input
    (do
      (def (f (: #tuple(a b) (Tuple Int64 Int64))) (+ a b))
      (def (main) (f #tuple(3 4)))
      (export main)))
  (output (: 7 Int64)))

(case
  "an annotated tuple-pattern parameter still checks its annotation against the argument"
  (doc
    "The annotation on a destructuring parameter is ENFORCED, not silently dropped: `(def (f (: (tuple
           a b) (Tuple Int64 Bool))) a)` declares the second element `Bool`, but `(f (tuple 3 4))` passes an
           Int64 there — a contradiction (CDZ0203, type-system.md #Annotations Constrain, Never Contradict),
           exactly as an annotated `let` binder `(let (((: x Bool) 5)) x)` is rejected. Pins that peeling the
           annotation to reach the tuple pattern keeps the annotation live on the fresh binder.")
  (input
    (do (def (f (: #tuple(a b) (Tuple Int64 Bool))) a) (def (main) (f #tuple(3 4))) (export main)))
  (error CDZ0203))

; --- Jump-table index integrity for HIGH-BIT scrutinees (adversarial guards) ----------------------
; A dense match lowers to a `br_table` whose index operand is i32. The i64 scrutinee must be
; range-guarded on its FULL width BEFORE any wrap to i32: a scrutinee whose LOW 32 bits collide with a
; table index (2^32 + k has low bits k) must take the default, never arm k. The negative case (-1) is
; pinned above; these pin the wrap-collision faces a truncate-then-guard emit gets wrong.
(case
  "a scrutinee of two to the sixty-fourth-adjacent magnitude misses a zero-based jump table"
  (doc
    "`(match x (0 10) (1 20) (2 30) (_ 99))` called with x = 2^32 (4294967296): out of the covered
           range 0..2, so the default arm → 99. The low 32 bits of 2^32 are ZERO — an emit that wrapped
           the scrutinee to i32 (`i32.wrap_i64`) before the range guard would compute table index 0 and
           wrongly return 10 (arm 0). Pins that the br_table range guard tests the full i64, not the
           wrapped index. The wrap-collision companion of the negative-scrutinee default case above.")
  (input (do (def (main (: x Int64)) (match x (0 10) (1 20) (2 30) (_ 99))) (export main)))
  (call main (: 4294967296 Int64))
  (output (: 99 Int64)))

(case
  "a high-bit scrutinee misses an offset jump table whose bias cancels its low bits"
  (doc
    "The OFFSET-table companion: `(match x (100 10) (101 20) (102 30) (_ 99))` covers 100..102, so
           the emitted table index is `x - 100`. Called with x = 2^32 + 100 (4294967396): the true index
           2^32 is out of range → default 99. But the low 32 bits of `x - 100` are ZERO — a wrap before
           the guard hits arm 100 → 10. Pins the full-width guard survives the bias subtraction.")
  (input (do (def (main (: x Int64)) (match x (100 10) (101 20) (102 30) (_ 99))) (export main)))
  (call main (: 4294967396 Int64))
  (output (: 99 Int64)))

(case
  "the minimum integer scrutinee misses a zero-based jump table"
  (doc
    "`(match x (0 10) (1 20) (2 30) (_ 99))` at x = Int64.min: the sign-extreme scrutinee (low 32
           bits zero, like 2^32) must default → 99, whether the guard compares signed or unsigned. The
           extreme companion of the 2^32 and -1 default cases — together they pin the guard at both
           wrap-collision faces and both sign extremes.")
  (input (do (def (main (: x Int64)) (match x (0 10) (1 20) (2 30) (_ 99))) (export main)))
  (call main (: -9223372036854775808 Int64))
  (output (: 99 Int64)))

(case
  "a loop-invariant trapping expression is not evaluated when the loop runs zero iterations"
  (doc
    "`(go 0 0 d 5)` where go's body adds `(+ acc (/ 100 d))` per iteration: the bound n = 0 means
           ZERO iterations, so the loop-invariant `(/ 100 d)` is NEVER evaluated and the accumulator 5
           returns unchanged — even at d = 0, where evaluating it would trap. LICM may hoist the invariant
           out of the loop only BELOW the iteration guard (or guarded by it): a hoist above the `(< i n)`
           test evaluates `(/ 100 0)` speculatively and traps a program that must return 5. The
           trap-freedom complement of the hoisted-overflow-bound case above (there the bound itself is
           evaluated pre-loop and MUST trap; here the invariant belongs to the body and must NOT).")
  (input
    (do
      (def
        (go (: i Int64) (: n Int64) (: d Int64) (: acc Int64))
        (if (< i n) (go (+ i 1) n d (+ acc (/ 100 d))) acc))
      (def (main (: d Int64)) (go 0 0 d 5))
      (export main)))
  (call main (: 0 Int64))
  (output (: 5 Int64)))

; ── LICM invariance is PURITY-scoped, and a conditional trap keeps its per-iteration gate ─────────────
; Two more hoist-legality boundaries the zero-iteration case above cannot witness. FIRST: a
; syntactically loop-invariant PERFORM — `(Fresh.next)` names no loop variable, so a lexical
; invariance test calls it hoistable, but each iteration's perform reads a DIFFERENT handler state.
; Hoisting it to one pre-loop perform (or batching the reads) changes the VALUE, not just a trace.
; SECOND: a trapping expression inside the loop body gated by an ITERATION-dependent condition — the
; trap is reached only at sufficient depth, so a hoist may neither lift the trap above its `(= i 2)`
; gate (a shallow run must complete) nor drop it (a deep run must trap). Together with the
; zero-iteration case these pin the three hoist-legality axes: iteration count, effectfulness, and
; the iteration-indexed guard.
(case
  "a syntactically loop-invariant perform advances per iteration — effectful is not hoistable"
  (doc
    "`go` adds `(Fresh.next)` per iteration over a RUNTIME count n. The perform is syntactically
           invariant (it names neither `i` nor `n`), but each iteration reads an advancing handler state:
           seeded 10 at n = 3 the reads are 10, 11, 12 → 33. A LICM that hoisted the perform to one
           pre-loop read (3×10 = 30) or evaluated it speculatively at n = 0 (advancing state that must
           stay 0 → the n=0 call must yield 0) changes observable VALUES. Loop-invariance licences
           motion only for PURE computations — the effect-side companion of the zero-iteration
           trap-freedom case above, and the loop face of the unreferenced-effectful-binding pin
           (§dead-binding cluster). Expected: 33 (n=3), 0 (n=0).")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (go (: i Int64) (: n Int64) (: acc Int64))
        (if (< i n) (go (+ i 1) n (+ acc (Fresh.next))) acc))
      (def (main (: n Int64)) (handle Fresh 10 ((next (u) s (resume s (+ s 1)))) (go 0 n 0)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 33 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a conditionally-reached trap in a loop body fires exactly when its iteration arrives"
  (doc
    "The loop body adds `(if (= i 2) (/ 100 x) 1)` — the division is reachable only at iteration
           i = 2. At n = 2 (iterations 0,1) the trap site is never reached: the program must complete
           and yield 2, even at x = 0. At n = 4 iteration 2 arrives and `(/ 100 0)` must trap. A hoist
           that lifts the division above its `(= i 2)` gate traps the n = 2 run (wrong); one that
           replaces the guarded division with a pre-computed value drops the n = 4 trap (also wrong).
           The iteration-indexed companion of the zero-iteration case: there the guard is the loop
           bound itself, here it is a condition INSIDE the body that only some iterations satisfy.
           Expected: 2 (n=2, x=0); trap (n=4, x=0).")
  (input
    (do
      (def
        (go (: i Int64) (: n Int64) (: x Int64) (: acc Int64))
        (if (< i n) (go (+ i 1) n x (+ acc (if (= i 2) (/ 100 x) 1))) acc))
      (def (main (: n Int64) (: x Int64)) (go 0 n x 0))
      (export main)))
  (call main (: 2 Int64) (: 0 Int64))
  (output (: 2 Int64))
  (call main (: 4 Int64) (: 0 Int64))
  (trap "integer divide by zero"))

; --- The common-constructor if-arm hoist preserves arm guarding (adversarial pins) ----------------
; `(if c (K …p) (K …q))` with the SAME constructor K both arms is rewritten to build K ONCE with
; per-payload `(if c pᵢ qᵢ)` selections (one heap build instead of two). The rewrite is sound only if
; each payload stays guarded by the condition (a trap in the untaken arm's payload must not fire) and
; the taken arm's trap still does. These pin the guard at every constructor shape the hoist covers —
; sum, tuple, record — with a RUNTIME condition (the constant-fold untaken-branch cases above never
; reach the hoist).
(case
  "a trapping payload in the untaken arm of a same-constructor if does not trap"
  (doc
    "`(if (> d 0) (Some (/ 100 d)) (Some 42))` at d = 0: both arms build `Some`, the hoist's
           target shape. The else arm is taken → the match yields 42; the then-payload `(/ 100 0)` must
           stay UNEVALUATED behind the condition — a hoist that lifts the payload `if` out but evaluates
           both payload alternatives (a select over payloads) would trap a program that must return 42.
           The sum-shape guard pin for the common-constructor hoist.")
  (input
    (do
      (def
        (main (: d Int64))
        (match (if (> d 0) (Option.Some (/ 100 d)) (Option.Some 42)) ((Option.Some v) v) (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

(case
  "a trapping element in the untaken arm of a same-arity tuple if does not trap"
  (doc
    "The tuple-shape companion: `(. (if (> d 0) (tuple (/ 100 d) 1) (tuple 7 2)) 0)` at d = 0
           takes the else tuple → element 0 is 7; the then-element `(/ 100 0)` stays behind the guard.
           Pins the per-element `(if c pᵢ qᵢ)` selections the hoist introduces are real conditionals
           (or trap-gated selects), never both-sides evaluation.")
  (input
    (do (def (main (: d Int64)) (. (if (> d 0) #tuple((/ 100 d) 1) #tuple(7 2)) 0)) (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a trapping field in the untaken arm of a same-shape record if does not trap"
  (doc
    "The record-shape companion (the hoist's record extension): `(. (if (> d 0) (record (a (/ 100
           d))) (record (a 5))) a)` at d = 0 takes the else record → field a = 5; the then-field's
           divide-by-zero stays guarded. Completes the guard pin across all three hoisted shapes.")
  (input
    (do
      (def (main (: d Int64)) (. (if (> d 0) #record((= a (/ 100 d))) #record((= a 5))) a))
      (export main)))
  (call main (: 0 Int64))
  (output (: 5 Int64)))

; ── Projecting/matching through an if- or match-selected compound folds the compound away (no heap) ───
; A single projection/member-read/match over a compound built through an `if`/`match` pushes the read INTO
; the branches, folding the throwaway compound away entirely (no per-call arr-alloc, no value-heap import).
; The transform is value-transparent — the read still selects the branch's element/field/arm — so these
; pin the observable VALUE; the "no runtime import" fold witness stays a white-box rcdzc assertion.
(case
  "a projection-only runtime tuple folds to the sum of its elements"
  (doc
    "`(let ((t (tuple a b))) (+ (. t 0) (. t 1)))` over runtime params: the tuple is only projected,
           never escaped, so it folds to `(+ a b)` — no heap. pair-sum(20, 22) = 42.")
  (input
    (do
      (def (pair-sum (: a Int64) (: b Int64)) (let ((t #tuple(a b))) (+ (. t 0) (. t 1))))
      (export pair-sum)))
  (call pair-sum (: 20 Int64) (: 22 Int64))
  (output (: 42 Int64)))

(case
  "a projection of an if-selected tuple selects the branch's element"
  (doc
    "`(. (if p (tuple a b) (tuple b a)) 0)` pushes the projection into the branches, folding to `(if p
           a b)`: p=true selects the then-tuple's element 0 (a), p=false selects the else-tuple's element 0
           (b, the swapped position). pick(true,10,20)=10; pick(false,10,20)=20.")
  (input
    (do
      (def (pick (: p Bool) (: a Int64) (: b Int64)) (. (if p #tuple(a b) #tuple(b a)) 0))
      (export pick)))
  (call pick (: true Bool) (: 10 Int64) (: 20 Int64))
  (output (: 10 Int64))
  (call pick (: false Bool) (: 10 Int64) (: 20 Int64))
  (output (: 20 Int64)))

(case
  "a member read of an if-selected record selects the branch's field by name"
  (doc
    "The record companion, with fields written OUT of sorted order to confirm the fold is by KEY not
           slot: `(. (if p (record (y b) (x a)) (record (y a) (x b))) x)` folds to `(if p a b)`.
           pick(true,10,20)=10; pick(false,10,20)=20.")
  (input
    (do
      (def
        (pick (: p Bool) (: a Int64) (: b Int64))
        (. (if p #record((= y b) (= x a)) #record((= y a) (= x b))) x))
      (export pick)))
  (call pick (: true Bool) (: 10 Int64) (: 20 Int64))
  (output (: 10 Int64))
  (call pick (: false Bool) (: 10 Int64) (: 20 Int64))
  (output (: 20 Int64)))

(case
  "a match over an if-selected sum folds each branch's constructor to its arm body"
  (doc
    "`(match (if (> x 0) (Some x) None) ((Some v) v) (None 0))` pushes the match into each branch and
           folds each constant constructor to its arm body, giving `(if (> x 0) x 0)` — no throwaway sum
           build. x=5 -> Some 5 -> v=5; x=-3 -> None -> 0.")
  (input
    (do
      (type Option (Some Int64) None)
      (def
        (f (: x Int64))
        (match (if (> x 0) (Option.Some x) Option.None) ((Option.Some v) v) (Option.None 0)))
      (export f)))
  (call f (: 5 Int64))
  (output (: 5 Int64))
  (call f (: -3 Int64))
  (output (: 0 Int64)))

(case
  "a match over a match-selected sum folds into the inner arms"
  (doc
    "The match-of-match fusion: `(match (match (> n 0) (true (Some n)) (false None)) ((Some v) v) (None
           0))` pushes the outer match into each inner arm body, folding each throwaway constructor away to
           `(match (> n 0) (true n) (false 0))`. n=5 -> Some 5 -> v=5; n=-3 -> None -> 0.")
  (input
    (do
      (type Option (Some Int64) None)
      (def
        (f (: n Int64))
        (match
          (match (> n 0) (true (Option.Some n)) (false Option.None))
          ((Option.Some v) v)
          (Option.None 0)))
      (export f)))
  (call f (: 5 Int64))
  (output (: 5 Int64))
  (call f (: -3 Int64))
  (output (: 0 Int64)))

(case
  "a trapping payload in the TAKEN arm of a same-constructor if still traps"
  (doc
    "The complement: `(if (= d 0) (Some (/ 100 d)) (Some 42))` at d = 0 takes the THEN arm, so its
           payload `(/ 100 0)` IS evaluated and must trap — a hoist that over-guards (never evaluates a
           trapping payload, or folds the taken arm away) silently returns where the program must fail.
           Together with the untaken-arm cases this pins the guard is exactly the condition, not a
           blanket trap suppression.")
  (input
    (do
      (def
        (main (: d Int64))
        (match (if (= d 0) (Option.Some (/ 100 d)) (Option.Some 42)) ((Option.Some v) v) (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "a trapping condition of a same-constructor if traps before either arm"
  (doc
    "`(if (> (/ 100 d) 0) (tuple 1 2) (tuple 3 4))` at d = 0: the CONDITION itself traps, so
           neither arm is reached. The hoist moves the condition into per-payload selections — if that
           duplication re-evaluated the condition per payload the trap would fire twice (observable
           under an effectful condition; here it pins at minimum that the trap still fires), and if the
           rewrite dropped the condition eval for equal payload slots it would not fire at all. The
           condition-integrity pin for the hoist.")
  (input
    (do (def (main (: d Int64)) (. (if (> (/ 100 d) 0) #tuple(1 2) #tuple(3 4)) 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "a trapping shared payload before the differing position must not preempt a trapping cond"
  (doc
    "`(if (< (+ i64::MAX e) 5) (tuple (/ 10 d) 1) (tuple (/ 10 d) 2))` — element 0 is the SHARED
           `(/ 10 d)`, element 1 is the sole DIFFERING position. The original `if` evaluates the cond
           FIRST; a checked `+` overflow means the program must trap 'integer overflow' at every d. The
           common-constructor hoist builds the shared element OUTSIDE the per-position `if`, so it would
           evaluate `(/ 10 d)` BEFORE the cond — at d = 0 that div-by-zero would preempt the cond's
           overflow (the WRONG trap). The hoist declines a possibly-trapping cond unless every shared
           payload before the differing position is trap-free, keeping the cond's trap observed first.
           This pins the ORDER obligation the diff-count guard alone did not cover (Copilot review on PR
           #375, r3589185980).")
  (input
    (do
      (def
        (main (: d Int64) (: e Int64))
        (if (< (+ 9223372036854775807 e) 5) #tuple((/ 10 d) 1) #tuple((/ 10 d) 2)))
      (export main)))
  (call main (: 0 Int64) (: 1 Int64))
  (trap "integer overflow"))

; The hoist COMPOSES through NESTED `if`s: `(if c1 (K a) (if c2 (K b) (K c)))` builds `K` ONCE across the
; whole decision tree (each per-`if` fold fires bottom-up — the inner `if` hoists `K` out, then the outer
; `if` sees both arms as `K` and hoists again), so the payload becomes a nested select/`if` and a single
; construct is emitted for all three arms. This pins the composition (which the flat two-arm cases above
; do not exercise): the value must be the matched arm's payload in every direction.
(case
  "a nested if of a common constructor builds it once and dispatches to each arm"
  (doc
    "`(if c1 (Some a) (if c2 (Some b) (Some d)))` — a three-way nested `if` whose every leaf builds
           `Some`. The common-constructor hoist composes bottom-up (inner `if` first, then the outer),
           so ONE `Some` is built around a nested payload select rather than three separate `sum-new`s.
           Observed through an outer match, the payload is the selected arm's in every direction: c1 →
           a (10), else-then-c2 → b (20), else-else → d (30). Pins the nested/composing case the flat
           two-arm common-constructor pins above do not reach.")
  (input
    (do
      (def
        (main (: c1 Bool) (: c2 Bool) (: a Int64) (: b Int64) (: d Int64))
        (match
          (if c1 (Option.Some a) (if c2 (Option.Some b) (Option.Some d)))
          ((Option.Some v) v)
          (_ -1)))
      (export main)))
  (call main (: true Bool) (: false Bool) (: 10 Int64) (: 20 Int64) (: 30 Int64))
  (output (: 10 Int64))
  (call main (: false Bool) (: true Bool) (: 10 Int64) (: 20 Int64) (: 30 Int64))
  (output (: 20 Int64))
  (call main (: false Bool) (: false Bool) (: 10 Int64) (: 20 Int64) (: 30 Int64))
  (output (: 30 Int64)))

; The hoist COMPOSES through the differing position it SYNTHESIZES: when the hoist pushes a differing
; field into a fresh `(if c pᵢ qᵢ)` and that field is ITSELF a common constructor across the arms, the
; synthesized `if` is re-run through the hoist so the nested constructor is hoisted too (arbitrary depth).
; `(if c1 (tuple (Some a) 1) (if c2 (tuple (Some b) 1) (tuple (Some a) 1)))` hoists the outer tuple ONCE,
; and its differing field-0 — a nested-if of `Some` — hoists to one `Some` (one tuple + one sum-new, not
; three of each). Value parity in every arm direction pins the deep composition and its Perceus (only the
; selected payload materializes). This pins the compose-through-synthesized-position case (a unit test
; covers it; this gives the whole fleet's gate the same protection).
(case
  "a common constructor composes through a hoisted tuple field to build once"
  (doc
    "`(if c1 (tuple (Some a) 1) (if c2 (tuple (Some b) 1) (tuple (Some a) 1)))` — the hoist builds
           the outer tuple once and, because its differing field-0 is itself a nested-if of `Some`, that
           field hoists to a single `Some` too (arbitrary-depth composition). Read as element-0's payload
           plus the shared element-1 (=1): c1 → a+1, else-c2 → b+1, else-else → a+1. Kept opaque via a
           recursive helper so the tuple/Some are genuine runtime heap values.")
  (input
    (do
      (def
        (mk (: c1 Bool) (: c2 Bool) (: a Int64) (: b Int64) (: n Int64))
        (if
          (< n 0)
          (mk c1 c2 a b (+ n 1))
          (if
            c1
            #tuple((Option.Some a) 1)
            (if c2 #tuple((Option.Some b) 1) #tuple((Option.Some a) 1)))))
      (def
        (main (: c1 Bool) (: c2 Bool) (: a Int64) (: b Int64))
        (match (. (mk c1 c2 a b 0) 0) ((Option.Some v) (+ v (. (mk c1 c2 a b 0) 1))) (_ -1)))
      (export main)))
  (call main (: true Bool) (: false Bool) (: 10 Int64) (: 20 Int64))
  (output (: 11 Int64))
  (call main (: false Bool) (: true Bool) (: 10 Int64) (: 20 Int64))
  (output (: 21 Int64))
  (call main (: false Bool) (: false Bool) (: 10 Int64) (: 20 Int64))
  (output (: 11 Int64))
  (live-objects known-leak))

; --- The list face of the common-constructor hoist (same-length ListNew arms) ---------------------
; The hoist's list extension: `(if c (list …p) (list …q))` with SAME-length arms builds one list with
; per-element selections. Same guard obligations as the sum/tuple/record pins above, plus two faces
; the other shapes don't have: a LENGTH mismatch must decline the hoist (length is part of a list's
; value), and the vec-push element chain must respect Perceus retains inside a hoisted element.
(case
  "a trapping element in the untaken arm of a same-length list if does not trap"
  (doc
    "`(if (> d 0) (list (/ 100 d) 1) (list 7 2))` at d = 0: both arms build a 2-list (the hoist's
           target), the else arm is taken → element 0 is 7; the then-element `(/ 100 0)` stays behind the
           condition. The list-shape guard pin, completing the sum/tuple/record set above.")
  (input
    (do
      (def
        (main (: d Int64))
        (Option.expect (List.at (if (> d 0) #list((/ 100 d) 1) #list(7 2)) 0) "v"))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a trapping element in the taken arm of a same-length list if still traps"
  (doc
    "The complement: `(if (= d 0) (list (/ 100 d) 1) (list 7 2))` at d = 0 takes the THEN arm, so
           its element `(/ 100 0)` IS evaluated and must trap. With the untaken-arm case this pins the
           per-element selections are guarded by exactly the condition.")
  (input
    (do
      (def
        (main (: d Int64))
        (Option.expect (List.at (if (= d 0) #list((/ 100 d) 1) #list(7 2)) 0) "v"))
      (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "an if over different-length list arms keeps each branch's own list"
  (doc
    "`(if (> d 0) (list 1 2 3) (list 9))` — the arms build lists of DIFFERENT lengths (3 vs 1), so
           the hoist must DECLINE (a list's length is part of its value; there is no per-element
           alignment) and keep the `if`: d = 0 → the 1-list (len 1), d = 1 → the 3-list (len 3). A hoist
           that force-aligned the shorter arm (padding or truncating) would corrupt one branch's value.
           The decline pin for the list hoist's same-length guard.")
  (input (do (def (main (: d Int64)) (List.len (if (> d 0) #list(1 2 3) #list(9)))) (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 3 Int64)))

(case
  "a shared and a differing element hoist across same-length list arms"
  (doc
    "`(if (> d 0) (list x 5) (list y 5))` — element 1 is IDENTICAL across the arms (shared
           directly, no select), element 0 differs (selected by the condition). d = 1 → (x, 5) = 3 + 5 =
           8; d = 0 → (y, 5) = 9 + 5 = 14. Pins the aligned per-position rewrite: the differing slot
           genuinely selects (both branch directions verified) and the shared slot is not disturbed.")
  (input
    (do
      (def
        (main (: d Int64) (: x Int64) (: y Int64))
        (let
          ((t (if (> d 0) #list(x 5) #list(y 5))))
          (+ (Option.expect (List.at t 0) "v") (Option.expect (List.at t 1) "v"))))
      (export main)))
  (call main (: 1 Int64) (: 3 Int64) (: 9 Int64))
  (output (: 8 Int64))
  (call main (: 0 Int64) (: 3 Int64) (: 9 Int64))
  (output (: 14 Int64)))

(case
  "heap string elements of a hoisted list if select by branch"
  (doc
    "`(if (> d 0) (list (rep \"a\" d)) (list \"bb\"))` — singleton lists whose element is a HEAP
           value (a runtime String rope vs a flat literal). d = 0 → \"bb\" (byte-len 2); d = 3 →
           \"axxx\" (byte-len 4). The per-element select carries a heap HANDLE, not a scalar — pins the
           hoist is sound when the selected element is a reference-counted value (the select must move
           exactly one arm's handle into the list; duplicating or dropping either corrupts refcounts).")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (main (: d Int64))
        (String.byte-len
          (Option.expect (List.at (if (> d 0) #list((rep "a" d)) #list("bb")) 0) "v")))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (call main (: 3 Int64))
  (output (: 4 Int64)))

(case
  "a consuming op inside a hoisted list element respects a still-live binding"
  (doc
    "The Perceus interaction: `xs = [7]` is a multi-use binding; ONE hoisted element consumes it
           (`(List.len (List.push xs 9))`) while `xs` is read again after the `if`. d = 1 → the consuming
           element is selected: push path-copies (retain), so element 0 = 2 and `(List.len xs)` = 1 → 3;
           d = 0 → the constant arm: 0 + 1 = 1. Pins that moving the consuming expression from an if arm
           into a hoisted per-element selection preserves its dup site (the retain analysis must see the
           consume-under-condition the same either way).")
  (input
    (do
      (def
        (main (: d Int64))
        (let
          ((xs (List.push #list() 7)))
          (let
            ((t (if (> d 0) #list((List.len (List.push xs 9)) 0) #list(0 0))))
            (+ (Option.expect (List.at t 0) "v") (List.len xs)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

; --- The common-constructor sink across MATCH arms (the multi-arm hoist analogue) -----------------
; `(match k (0 (K …p₀)) (1 (K …p₁)) (_ (K …p_d)))` with every unguarded arm building the SAME
; constructor is rewritten to build K ONCE, each differing field position becoming its own per-position
; match over the same scrutinee (a `core_equiv` position is shared directly). The rewrite SPLITS one
; match into several — so the scrutinee must still be evaluated exactly once (sharpest under an
; effectful scrutinee: the split matches must all probe ONE performed value), untaken-arm traps must
; stay behind their probes, a guarded arm must keep first-match-wins, and a Perceus dup site inside one
; arm's field must survive the split. These pin each obligation at the shapes the sink covers.
(case
  "an effectful match scrutinee is performed exactly once across a two-position sink"
  (doc
    "`(match (Ctr.tick) (0 (tuple 1 2)) (1 (tuple 3 4)) (_ (tuple 5 6)))` — every arm builds a
           2-tuple and BOTH positions differ, so the sink emits one tuple whose two elements each match
           over the scrutinee. The counter arm `(tick (_) s (resume s (+ s 1)))` returns the count and
           threads +1: the first perform returns 0 → both position-matches must probe THAT value → t =
           (1, 2); the trailing `(Ctr.tick)` returns 1 (state advanced exactly once). 100·1 + 10·2 + 1 =
           121. A sink that RE-EVALUATES the scrutinee per position performs tick twice: position 0
           probes 0 → 1, position 1 probes 1 → 4, trailing tick returns 2 → 142. Pins that the split
           matches share ONE scrutinee evaluation.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          0
          ((tick (_) s (resume s (+ s 1))))
          (let
            ((t (match (Ctr.tick unit) (0 #tuple(1 2)) (1 #tuple(3 4)) (_ #tuple(5 6)))))
            (+ (+ (* 100 (. t 0)) (* 10 (. t 1))) (Ctr.tick unit)))))
      (export main)))
  (call main)
  (output (: 121 Int64)))

(case
  "a trapping payload in a non-taken arm of a same-constructor match does not trap"
  (doc
    "`(match k (0 (Some (/ 100 (- k 1)))) (1 (Some 20)) (_ (Some 30)))` at k = 1: every arm builds
           `Some`, the sink's target; arm 1 is taken → 20, and arm 0's payload `(/ 100 (- k 1))` = `(/
           100 0)` at this k WOULD trap had it been evaluated, but stays behind its probe. The divisor is
           `(- k 1)` — a RUNTIME expression that is 0 exactly at the k this case calls — NOT a literal `(/
           100 0)`, which is a compile-time divide-by-zero poison (CDZ0304) that would reject the whole
           program before any sink. So a sink that eagerly evaluated arm 0's payload would TRAP at k = 1
           instead of yielding 20 — the test now genuinely exercises the guard (an earlier version used
           `(/ 100 k)`, which at k = 1 is `(/ 100 1)` = 100 and never traps, making the pin vacuous — PR
           #381 review). The match analogue of the if-hoist untaken-arm pins above.")
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (match k (0 (Option.Some (/ 100 (- k 1)))) (1 (Option.Some 20)) (_ (Option.Some 30)))
          ((Option.Some v) v)
          (_ -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 20 Int64)))

(case
  "a trapping payload in the taken arm of a same-constructor match still traps"
  (doc
    "The complement at k = 0: the taken arm's payload `(/ 100 0)` IS evaluated and must trap —
           no over-guarding. With the non-taken case this pins the sunk payload evaluates exactly when
           its arm is selected.")
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (match k (0 (Option.Some (/ 100 k))) (1 (Option.Some 20)) (_ (Option.Some 30)))
          ((Option.Some v) v)
          (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "a guarded arm among same-constructor match arms keeps first-match-wins"
  (doc
    "`(match k ((guard x (> x 3)) (Some 50)) (1 (Some 20)) (_ (Some 30)))` — a GUARDED first arm
           makes the arm's coverage conditional, so the sink must decline (or preserve order exactly):
           k = 5 → the guard passes → 50; k = 1 → the guard fails, fall through to the literal arm → 20.
           A sink that reordered probes or dropped the guard's runtime condition breaks one of the two
           calls. The guard-interaction pin for the multi-arm sink.")
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (match k ((guard x (> x 3)) (Option.Some 50)) (1 (Option.Some 20)) (_ (Option.Some 30)))
          ((Option.Some v) v)
          (_ -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64))
  (call main (: 1 Int64))
  (output (: 20 Int64)))

(case
  "same-length list match arms share an equal element and sink the differing one"
  (doc
    "`(match k (0 (list 10 7)) (1 (list 20 7)) (_ (list 30 7)))` — element 1 is `core_equiv`
           across ALL arms (shared directly), element 0 differs (sunk into a per-position match). k = 1
           → (20, 7) → 27; k = 9 → the default (30, 7) → 37. The list-shape pin for the multi-arm sink,
           including the default arm's participation in the per-position match.")
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((t (match k (0 #list(10 7)) (1 #list(20 7)) (_ #list(30 7)))))
          (+ (Option.expect (List.at t 0) "v") (Option.expect (List.at t 1) "v"))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 27 Int64))
  (call main (: 9 Int64))
  (output (: 37 Int64)))

(case
  "same-key-set record match arms share an equal field and sink the differing one"
  (doc
    "`(match k (0 (record (a 10) (b 9))) (1 (record (a 20) (b 9))) (_ (record (a 30) (b 9))))` —
           field `b` is equal across arms, field `a` differs. k = 2 → the default {a:30, b:9} → 39;
           k = 0 → {a:10, b:9} → 19. The record-shape (keyed alignment) pin for the multi-arm sink.")
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((r
              (match
                k
                (0 #record((= a 10) (= b 9)))
                (1 #record((= a 20) (= b 9)))
                (_ #record((= a 30) (= b 9))))))
          (+ r.a r.b)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 39 Int64))
  (call main (: 0 Int64))
  (output (: 19 Int64)))

(case
  "a consuming op on a still-live binding inside one match arm's payload keeps its retain"
  (doc
    "The Perceus interaction for the match sink: `xs = [7]` is multi-use — arm 1's payload
           consumes it (`(List.len (List.push xs 9))`) and `xs` is read again after the match. k = 1 →
           the consuming payload is selected: push path-copies (its dup site must survive the payload
           being sunk into a per-position match) → Some 2, then `(List.len xs)` = 1 → 3; k = 0 → the
           constant arm → 0 + 1 = 1. Pins that moving an arm payload into a sunk position match
           preserves the consume-under-probe the retain analysis placed.")
  (input
    (do
      (def
        (main (: k Int64))
        (let
          ((xs (List.push #list() 7)))
          (let
            ((o (match k (1 (Option.Some (List.len (List.push xs 9)))) (_ (Option.Some 0)))))
            (+ (Option.expect o "v") (List.len xs)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a trapping scrutinee of a same-constructor match traps before any arm"
  (doc
    "`(match (/ 100 d) (0 (tuple 1 2)) (_ (tuple 3 4)))` at d = 0: the SCRUTINEE traps, so no arm
           is reached. The sink multiplies the scrutinee's syntactic occurrences (one per differing
           position) — this pins the trap still fires (and, with the effectful-once case above, fires
           exactly once): the split matches must probe a single bound evaluation, never re-run it.")
  (input
    (do
      (def (main (: d Int64)) (. (match (/ 100 d) (0 #tuple(1 2)) (_ #tuple(3 4))) 0))
      (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "a computed match scrutinee is evaluated once and its overflow guard still fires"
  (doc
    "`(match (+ a b) (0 10) (1 20) (_ 30))` — the scrutinee is a CHECKED add, evaluated ONCE into a
           slot that every probe reads (not a recomputed add). Dispatch: (a+b)=0 → 10, =1 → 20, =5 →
           wildcard 30. The single evaluation must still CHECK: an a+b overflowing Int64 traps before any
           probe runs (the compute-once must not drop the scrutinee's own overflow guard).")
  (input (do (def (f (: a Int64) (: b Int64)) (match (+ a b) (0 10) (1 20) (_ 30))) (export f)))
  (call f (: -2 Int64) (: 2 Int64))
  (output (: 10 Int64))
  (call f (: 1 Int64) (: 0 Int64))
  (output (: 20 Int64))
  (call f (: 3 Int64) (: 2 Int64))
  (output (: 30 Int64))
  (call f (: 9223372036854775807 Int64) (: 1 Int64))
  (trap "overflow"))

(case
  "a match every unguarded arm of which yields the same value collapses to that value"
  (doc
    "`(match a (1 x) (2 x) (_ x))` always yields `x` regardless of the scrutinee `a` (the match
           analogue of `(if c x x)` -> `x`) — every arm is the SAME trap-free body, so the probe chain is
           dropped and the value is just `x`. Verified across a matched literal and the wildcard: a=1 -> 7,
           a=9 -> 7. (Sound because the scrutinee `a` is a trap-free parameter; a trapping scrutinee must
           still be evaluated — the case below.)")
  (input (do (def (f (: a Int64) (: x Int64)) (match a (1 x) (2 x) (_ x))) (export f)))
  (call f (: 1 Int64) (: 7 Int64))
  (output (: 7 Int64))
  (call f (: 9 Int64) (: 7 Int64))
  (output (: 7 Int64)))

(case
  "a trapping scrutinee of an all-same-body match is still evaluated"
  (doc
    "The all-same-body collapse (above) drops the probe chain, but the scrutinee was evaluated to
           drive the now-gone probes, so a scrutinee that could TRAP must still be evaluated. `(match (/ 10
           b) (1 x) (_ x))` always yields `x`, yet the division must still run: b=2 -> 7 (the common body),
           b=0 -> a divide-by-zero trap, not a silent `x`. Pins that the collapse preserves the scrutinee's
           effects.")
  (input (do (def (f (: b Int64) (: x Int64)) (match (/ 10 b) (1 x) (_ x))) (export f)))
  (call f (: 2 Int64) (: 7 Int64))
  (output (: 7 Int64))
  (call f (: 0 Int64) (: 7 Int64))
  (trap "divide by zero"))

; Two additions completing the multi-arm sink's shape coverage above: the TUPLE shared-plus-differing
; case (the sum/list/record cases pin the same-arm alignment, but the tuple shape's shared/differing
; split was uncovered), and a bare sum-PAYLOAD dispatch across three arms (the pure `Some`-payload
; direction — the sink builds one `Some` around a per-payload match).
(case
  "same-arity tuple match arms share an equal element and sink the differing one"
  (doc
    "The tuple companion of the list/record shared-and-differing cases: `(match k (0 (tuple x 5))
           (1 (tuple y 5)) (_ (tuple 99 5)))` — element 1 is `core_equiv` across ALL arms (shared
           directly, no per-position match), element 0 differs (sunk into a match on k). Read as
           element0 + element1: k=0 → x+5, k=1 → y+5, any other → 99+5. Pins the tuple shape's aligned
           per-position rewrite — the differing slot dispatches in every arm direction (incl. the
           default) and the shared slot is untouched.")
  (input
    (do
      (def
        (main (: k Int64) (: x Int64) (: y Int64))
        (let ((t (match k (0 #tuple(x 5)) (1 #tuple(y 5)) (_ #tuple(99 5))))) (+ (. t 0) (. t 1))))
      (export main)))
  (call main (: 0 Int64) (: 3 Int64) (: 8 Int64))
  (output (: 8 Int64))
  (call main (: 1 Int64) (: 3 Int64) (: 8 Int64))
  (output (: 13 Int64))
  (call main (: 5 Int64) (: 3 Int64) (: 8 Int64))
  (output (: 104 Int64)))

(case
  "a match building one Some per arm sinks to a single Some around a payload match"
  (doc
    "The bare sum-payload dispatch: `(match k (0 (Some 10)) (1 (Some 20)) (_ (Some 30)))` — every
           arm builds `Some`, so the sink builds ONE `Some` around a per-payload match on k, rather than
           a `sum-new` per arm. Observed through an outer match, the payload must be the matched arm's in
           every direction: k=0→10, k=1→20, any other→30 (the default). The value-parity pin for the
           sum-payload face of the multi-arm sink, complementing the trap/effect/Perceus cases above.")
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (match k (0 (Option.Some 10)) (1 (Option.Some 20)) (_ (Option.Some 30)))
          ((Option.Some v) v)
          (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (call main (: 1 Int64))
  (output (: 20 Int64))
  (call main (: 9 Int64))
  (output (: 30 Int64)))

(case
  "a literal arm wins over a later guard arm that also matches"
  (doc
    "Literal arms INTERLEAVED with a guard arm: `(match k (0 0) (1 10) (2 20) (3 30) (5 50)
           ((guard x (> x 3)) 99) (_ -1))`. k = 5 satisfies BOTH the literal `5` arm and the later
           guard `(> x 3)` — first-match-wins selects the literal → 50; k = 4 misses every literal and
           takes the guard → 99; k = 2 hits its literal before the guard is reached → 20. The guarded
           twin-arm case above pins guard-vs-guard order; this pins LITERAL-vs-guard order under the
           guarded-scalar if-chain desugar — a lowering that partitions literals into a jump table and
           appends guards (or vice versa) without preserving source order answers 99 for k = 5.")
  (input
    (do
      (def
        (main (: k Int64))
        (match k (0 0) (1 10) (2 20) (3 30) (5 50) ((guard x (> x 3)) 99) (_ -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64))
  (call main (: 4 Int64))
  (output (: 99 Int64))
  (call main (: 2 Int64))
  (output (: 20 Int64)))

; --- Evaluation-order anchors around the hoist/sink rewrites (which trap fires) --------------------
; 06dadcb75 pinned that a trapping SHARED payload must not preempt a trapping COND. These anchor the
; rest of the order contract the rewrites must preserve: strict left-to-right within the taken arm,
; and the projected-vs-unprojected distinction (a projection that discards a trapping element is the
; OPEN dead-trap spec question — not graded here; the PROJECTED trapping element is, and must trap).
(case
  "the left payload of the taken arm traps before the right payload"
  (doc
    "`(tuple (/ 10 d) (+ x 1))` in the taken arm with BOTH elements trapping for these inputs
           (d = 0 → divide by zero; x = Int64.max → overflow): elements evaluate left-to-right, so the
           DIVIDE trap fires — a rewrite (hoist, sink, or per-element select) that reorders payload
           evaluation surfaces the wrong trap. The projection reads element 0, so the trapping element
           is also the demanded one (no dead-trap ambiguity).")
  (input
    (do
      (def (main (: x Int64) (: d Int64)) (. (if (< x 5) #tuple((/ 10 d) (+ x 1)) #tuple(0 0)) 0))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64))
  (trap "divide by zero"))

(case
  "a projected trapping element of a hoisted same-constructor if traps"
  (doc
    "`(. (if (< x 5) (tuple (/ 10 d) 1) (tuple (/ 10 d) 2)) 0)` — the arms share the trapping
           element at position 0 (a hoist builds it once, outside the per-position select) and the
           projection DEMANDS that position. At x = 1, d = 0 the taken arm's `(/ 10 0)` must trap. Pins
           that a shared-and-hoisted payload keeps its trap when demanded — the demanded-element
           complement of the shared-payload-vs-cond order case (the UNdemanded-element face is the open
           dead-trap-on-discard spec question, deliberately not graded).")
  (input
    (do
      (def (main (: x Int64) (: d Int64)) (. (if (< x 5) #tuple((/ 10 d) 1) #tuple((/ 10 d) 2)) 0))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64))
  (trap "divide by zero"))

; The UNDEMANDED-element face the case above deferred is now SETTLED: the same §283 dead-init ruling that
; lets a `?` short-circuit elide an unobserved trapping let-init (23-try-operator) — and that lets an
; unreferenced `let` binding's trap be elided (the dead-binding cases below) — makes a tuple element the
; projection DISCARDS unobserved too. So projecting the SAFE position of a tuple whose OTHER position would
; trap does NOT trap: observation, not construction, forces a trap (core-semantics.md §A Trap Occurs Only
; Where Its Computation Is Observed). This grades the discard face — the complement of the demanded-element
; trap above — so a fold that eagerly evaluated the whole tuple (materializing the discarded trapping
; element) would be caught FLIPPING this from its value to a trap.
(case
  "projecting the safe element of a tuple elides the discarded trapping element's trap"
  (doc
    "`(. (tuple (/ 10 d) 1) 1)` projects position 1 (the `1`) and DISCARDS position 0 (`(/ 10 d)`).
           At d = 0 the discarded element WOULD divide by zero, but the projection never observes it, so
           per §A Trap Occurs Only Where Its Computation Is Observed (the same rule that elides an unused
           `let` init and a `?`-short-circuited earlier init) the trap is ELIDED — the result is `1`, not a
           trap. Settles the undemanded-element dead-trap-on-discard question the demanded-element case
           above deferred; the discarded element's evaluation is not forced by tuple CONSTRUCTION, only by
           projection of THAT position. `d` is a runtime parameter so this is emitted code, both backends.")
  (input (do (def (main (: d Int64)) (. #tuple((/ 10 d) 1) 1)) (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a discarded trapping item in a do sequence is elided per the dead-init ruling"
  (doc
    "The do-sequence member of the §283 discard family: `(do (/ 1 n) 42)` — the non-final item's
           value is discarded, so its trap is UNOBSERVED and elided; the do yields 42 at n=0 exactly as
           at n=1 (observation, not construction, forces a trap). The OBSERVED control: `(let ((q (/ 1
           n))) (do q (+ q 41)))` — q reaches the final item's arithmetic, so the n=0 trap FIRES. One
           binding-vs-discard pair pinning both sides of the ruling on the do spine (the tuple-projection
           and dead-let members are pinned nearby; note the FOREIGN-perform exception — a discarded item
           reaching a PERFORM is preserved, 14-effects' do-fold family).")
  (input (do (def (main (: n Int64)) (do (/ 1 n) 42)) (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64))
  (call main (: 1 Int64))
  (output (: 42 Int64)))

(case
  "a do item OBSERVED by the final expression keeps its trap"
  (doc
    "The observed twin: the trapping quotient is LET-bound and the final do item reads it — the
           trap is demanded, so n=0 traps and n=1 computes 42. Brackets the elide case above: the SAME
           quotient, elided when discarded, trapping when observed.")
  (input (do (def (main (: n Int64)) (let ((q (/ 1 n))) (do q (+ q 41)))) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero")
  (call main (: 1 Int64))
  (output (: 42 Int64)))

(case
  "an escaping tuple evaluates its trapping element"
  (doc
    "`(tuple (/ 10 d) 1)` RETURNED whole (no projection): every element is demanded by the
           escape, so the d = 0 divide trap must fire — no fold may discard it. The escape control the
           projection-discard question is measured against: whatever the strict-let/dead-trap ruling
           decides for a DISCARDED element, a demanded one is unambiguous.")
  (input (do (def (main (: d Int64)) #tuple((/ 10 d) 1)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

; ── A `let` binding is observed only when it is REFERENCED — an unused binding's trap is elided ───────
; core-semantics.md §A Trap Occurs Only Where Its Computation Is Observed names, in the same breath as an
; un-projected tuple element, "a `let` binding that is never referenced" as unobserved: constructing the
; surrounding scope does not require evaluating it, so an implementation MAY elide it AND the trap it
; would have raised. This settles the open "strict-let / dead-trap ruling" the escaping-tuple case above
; measures against: `let` is NOT "strict" enough to make an unused binding's trap observable — observation,
; not the `let` keyword, is what forces a trap, exactly as for an un-projected tuple element. The dual
; ANCHOR pins that the moment the binding IS referenced, its value flows out and the trap fires. A
; parameter-driven overflow (not a constant fold — the arg crosses the boundary at run time) makes this a
; genuine emitted-code question on BOTH backends. (A binding the compiler PROVES traps and elides also
; earns the non-error CDZ0305 diagnostic; the build still succeeds with the recorded value.)
(case
  "an unused let binding whose init would overflow is elided, so its trap does not occur"
  (doc
    "`(let ((y (+ x 1))) x)` with x = Int64.max: the binding `y = x + 1` overflows Int64, but the
           body returns `x`, never referencing `y`. `y`'s value is unobserved, so the binding need not be
           evaluated and its overflow trap does not occur — the program yields Int64.max. Uses a runtime
           parameter (the arg crosses the boundary, not a constant fold) so this exercises the emitted
           code on both backends. The anchor below pins that referencing `y` DOES trap. This is the
           binding-form companion of the un-projected tuple element (05-compound-types.sexp) and the
           unused-argument (09-functions.sexp) elisions — observation, not the `let` keyword, forces a
           trap (core-semantics.md §A Trap Occurs Only Where Its Computation Is Observed).")
  (input (do (def (main (: x Int64)) (let ((y (+ x 1))) x)) (export main)))
  (call main (: 9223372036854775807 Int64))
  (output (: 9223372036854775807 Int64)))

(case
  "a referenced let binding whose init overflows IS observed, so its trap occurs (the anchor)"
  (doc
    "The control: the SAME `(let ((y (+ x 1))) …)` but the body returns `y`, so `y`'s value flows
           out as the result — observed — and the overflowing `+` must trap. Contrast the elision case
           above where `y` is never referenced. Confirms the elision is specifically about an UNREFERENCED
           binding: a referenced binding is a strict, observed computation whose trap fires. The
           binding-form dual of the projected-tuple-element anchor in 05-compound-types.sexp.")
  (input (do (def (main (: x Int64)) (let ((y (+ x 1))) y)) (export main)))
  (call main (: 9223372036854775807 Int64))
  (trap "integer overflow"))

(case
  "a discarded do-statement whose value would overflow is elided, so its trap does not occur"
  (doc
    "The sequencing face: `(do (+ x 1) x)` with x = Int64.max evaluates the non-final statement
           `(+ x 1)` only for its effect and discards its value. The statement is PURE (it reaches no host
           call) and its overflowing value is never observed, so an implementation need not evaluate it —
           its trap does not occur and the block yields its tail `x` = Int64.max. Pins that a discarded
           pure do-statement is unobserved exactly as an unreferenced let binding is; a non-final statement
           whose value cannot affect observable behavior (no host call, discarded value) need not run.")
  (input (do (def (main (: x Int64)) (do (+ x 1) x)) (export main)))
  (call main (: 9223372036854775807 Int64))
  (output (: 9223372036854775807 Int64)))

(case
  "a pure non-final do-statement whose value is discarded compiles but earns a CDZ0307 discarded-value warning"
  (doc
    "The dead-code warning of the discarded-do-statement rule above: `(do (inc 8) (* n 2))` evaluates
           the non-final `(inc 8)` only for its effect, but it is PURE, so its computed value is thrown away
           — in a pure language almost always a bug (a call whose result the author forgot to use). The block
           still COMPILES and runs (its value is the tail `(* n 2)`; `(main 5)` = 10), but the build surfaces
           a CDZ0307 `discarded value` WARNING anchored at the dead `(inc 8)` form — the same code-quality/
           dead-code band as the unused binding (CDZ0306), unreachable arm (CDZ0213), and dead trap (CDZ0305).
           The (warns ..) pins the stable message lead (`computed but discarded`). Contrast the FINAL form and
           a Unit-typed non-final form, which discard nothing and do not warn. Wasm-graded (warnings ride the
           shared compile stage = target-independent; the rust/rust-async run paths cannot observe compile
           stderr, so the (warns ..) check is skipped there, not failed). Portable companion of the rcdzc
           a_pure_non_final_do_form_that_discards_a_value_warns test; that test additionally asserts the
           discarded form's user NODE and a delete FIX — a structural + HAS-FIX shape the (warns ..) substring
           clause cannot express — so it is KEPT.")
  (input
    (do (def (inc (: n Int64)) (+ n 1)) (def (main (: n Int64)) (do (inc 8) (* n 2))) (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64))
  (warns CDZ0307 (message "computed but discarded")))

(case
  "an ill-typed non-final do-statement is still caught though its value is discarded"
  (doc
    "A non-final `do` form is EVALUATED for effect (value discarded), but the fault walk descends into
           EVERY form, not only the tail — so an ill-typed intermediate is still rejected. `(do (if 5 1 2)
           42)` has a non-Bool `if` condition in the discarded first position → CDZ0203 'if condition must
           be Bool', not silently skipped because its value is unused. (migrated from rcdzc
           a_do_block_with_an_ill_typed_intermediate_is_still_caught.)")
  (input (do (def (main) (do (if 5 1 2) 42)) (export main)))
  (error CDZ0203 (message "condition must be Bool")))

(case
  "an ill-typed condition inside a let body is still caught (the check descends through the let)"
  (doc
    "The let-body companion of the do-descent above: `(let ((x 5)) (if x 1 2))` binds `x` = 5 (Int64)
           then uses it as an `if` CONDITION — the check walks into the let BODY, so the non-Bool condition
           is still reported → CDZ0203 'if condition must be Bool', not skipped because it is nested under a
           let. Pins that the type-fault walk descends through `let` bodies. (migrated from rcdzc
           a_type_fault_inside_a_let_body_is_still_caught.)")
  (input (do (def (main) (let ((x 5)) (if x 1 2))) (export main)))
  (error CDZ0203 (message "condition must be Bool")))

; A near-miss of an IN-SCOPE name gets a did-you-mean suggestion with a REPLACE fix (a heuristic
; nearest-name guess, edit-distance ≤ cutoff): a `let` binder `counter` referenced as `countr`, and a
; top-level def `compute` called as `computee`. Both reject CDZ0101 and carry a replace-fix to the near
; name. (migrated from rcdzc an_unbound_name_close_to_a_let_binding_suggests_the_binding +
; an_unbound_name_close_to_a_def_suggests_it_with_a_heuristic_fix; the latter's fix-ANCHOR pin — the fix
; targets the faulting node — stays a rust residue the corpus can't assert.)
(case
  "an unbound name close to a let binding suggests the binding with a replace fix"
  (input (do (def (main) (let ((counter 5)) (+ countr 1))) (export main)))
  (error CDZ0101 (fix (kind replace) (replacement "counter"))))

(case
  "an unbound name close to a top-level def suggests it with a replace fix"
  (input (do (def (compute x) x) (def (main) (computee 1)) (export main)))
  (error CDZ0101 (message "did you mean `compute`?") (fix (kind replace) (replacement "compute"))))

; A misspelled GRAMMAR keyword in HEAD position is an unbound name — but a correctly-spelled keyword is
; dispatched structurally, so the keywords only join the candidate pool in HEAD position. A head typo
; therefore names the keyword (`mtch`→`match`, `iff`→`if`, `le`→`let`, `annd`→`and`); a real DEF the typo
; is NEARER to still wins (`matchee` is distance 1 from `matcher`, distance 3 from `match`). (migrated from
; rcdzc a_misspelled_form_keyword_head_suggests_the_grammar_keyword — the argument-position NON-suggestion
; face stays a rust residue, a suggestion-ABSENCE the corpus grades only todo.)
(case
  "a misspelled match keyword in head position suggests match"
  (input (do (def (f (: n Int64)) (mtch n (0 1) (_ 2))) (export f)))
  (error CDZ0101 (message "did you mean `match`?")))

(case
  "a misspelled if keyword in head position suggests if"
  (input (do (def (f (: b Bool)) (iff b 1 2)) (export f)))
  (error CDZ0101 (message "did you mean `if`?")))

(case
  "a misspelled let keyword in head position suggests let"
  (input (do (def (f) (le ((x 5)) x)) (export f)))
  (error CDZ0101 (message "did you mean `let`?")))

(case
  "a misspelled and keyword in head position suggests and"
  (input (do (def (f (: b Bool)) (annd b b)) (export f)))
  (error CDZ0101 (message "did you mean `and`?")))

(case
  "a head typo nearer to a real def suggests the def over a grammar keyword"
  (input (do (def (matcher x) x) (def (f) (matchee 5)) (export f)))
  (error CDZ0101 (message "did you mean `matcher`?")))

; The DIVIDE-BY-ZERO face of the same elision: the trap-observation rule is about WHETHER the value is
; observed, not WHICH trap it would raise. An unused binding whose init is a divide-by-zero (`(/ 100 d)`
; at d = 0) is elided exactly as the overflow one above — the ÷0 trap does not occur and the body's value
; is returned. Pins that the ruling covers every DEFINED trap kind (÷0, %0, zero-denominator Rational.of),
; not only overflow, so an agent probing the div/rem/rational faces sees the conformant behavior witnessed
; rather than re-discovering it as a "miscompile." Uses a runtime parameter so it is a real emitted-code
; question on both backends. (The constant-fold form `(/ 100 0)` additionally earns the non-error CDZ0305
; provably-would-trap diagnostic — asserted by a compiler unit test — while still yielding its value.)
(case
  "an unused let binding whose init would divide by zero is elided, so its trap does not occur"
  (doc
    "`(let ((q (/ 100 d))) 1)` with d = 0: the binding `q = 100 / d` would trap (integer divide by
           zero), but the body returns the constant `1`, never referencing `q`. `q`'s value is unobserved,
           so the binding need not be evaluated and its ÷0 trap does not occur — the program yields 1. The
           divide-by-zero companion of the overflow elision above: the trap-observation rule
           (core-semantics.md §A Trap Occurs Only Where Its Computation Is Observed) is about observation,
           not the trap kind, so it covers ÷0/%0/zero-denominator `Rational.of` identically. A runtime
           parameter (the arg crosses the boundary) keeps it a genuine emitted-code question on both
           backends; the referenced-binding anchor above pins that observing such a binding DOES trap.")
  (input (do (def (main (: d Int64)) (let ((q (/ 100 d))) 1)) (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

; The CONSTANT-FOLD face of the dead-trap elision earns a DIAGNOSTIC, not silence: when the unobserved
; init PROVABLY traps at compile time (a constant `(/ 100 0)`, not a runtime `d`), the compiler both
; elides it (the value is unobserved, so the program still yields its result) AND surfaces the non-error
; CDZ0305 "provably-would-trap" WARNING — an unused element/binding/argument that always traps is likely
; a bug worth flagging, so it rides alongside the produced artifact rather than denying the build. The
; runtime-parameter cases above cannot pin this (the trap is not compile-time provable there); this pins
; the constant form portably with the (warns ..) clause. (Previously asserted only by an rcdzc unit test.)
(case
  "an unprojected tuple element that provably traps is elided but earns a CDZ0305 dead-trap warning"
  (doc
    "`(. (tuple 42 (/ 100 0)) 0)` projects element 0 (= 42); element 1's `(/ 100 0)` is a CONSTANT
           divide-by-zero that the fold proves traps, but it is never projected, so its value is unobserved
           — the program compiles and yields 42 (the trap-observation rule, core-semantics.md §A Trap Occurs
           Only Where Its Computation Is Observed). Because the elided computation PROVABLY traps (a compile-
           time constant, unlike the runtime-`d` elisions above), the build additionally surfaces the non-
           error CDZ0305 warning — a dead computation that always traps is likely a bug. The (warns ..) pins
           the stable message lead; a runtime-provable-only regression that dropped the warning flips this.
           Graded on the wasm target (warnings ride the shared compile stage = target-independent; the rust/
           rust-async run paths cannot observe compile stderr, so the (warns ..) check is skipped there, not
           failed). The portable companion of the rcdzc dead-trap unit test.")
  (input (do (def (main) (. #tuple(42 (/ 100 0)) 0)) (export main)))
  (output (: 42 Int64))
  (warns CDZ0305 (message "always traps but its value is never used")))

; A CONSTANT operation that ALWAYS traps, sitting in a branch whose reachability depends on a RUNTIME value
; (`(if (> n 0) 7 <const-trap>)`), earns the non-error CDZ0309 "potentially reachable trap" WARNING that NAMES
; the specific trap kind — divide-by-zero / overflow / shift-out-of-range — so the reader knows what would
; trap and can guard it. The program still compiles + runs the taken branch (`main 1` = 7). Wasm-graded (the
; run paths skip the warns check). (Migrated from rcdzc a_reachable_const_trap_warning_names_the_specific_trap_kind.)
(case
  "a reachable constant divide-by-zero in a runtime branch earns a CDZ0309 warning naming the trap kind"
  (input (do (def (f (: n Int64)) (if (> n 0) 7 (/ 1 0))) (def (main) (f 1)) (export main)))
  (output (: 7 Int64))
  (warning CDZ0309 (message "potentially reachable trap") (message "divide by zero")))

(case
  "a reachable constant overflow in a runtime branch earns a CDZ0309 warning naming the trap kind"
  (input (do (def (f (: n Int64)) (if (> n 0) 7 (* 9223372036854775807 9223372036854775807))) (def (main) (f 1)) (export main)))
  (output (: 7 Int64))
  (warning CDZ0309 (message "potentially reachable trap") (message "overflows Int64")))

(case
  "a reachable constant shift-out-of-range in a runtime branch earns a CDZ0309 warning naming the trap kind"
  (input (do (def (f (: n Int64)) (if (> n 0) 7 (<< 1 100))) (def (main) (f 1)) (export main)))
  (output (: 7 Int64))
  (warning CDZ0309 (message "potentially reachable trap") (message "out of range")))

(case
  "an unused NON-NORMALIZING let init is eliminated but earns a CDZ0305 warning"
  (doc
    "The dead-computation warning fires for a DIVERGING (non-terminating / explosively-growing) init too,
           not only a provably-trapping one: `_y`'s init is an omega-style self-application that never reduces
           to a value; since `_y` is unused the init is eliminated (the program runs to 0), but the build
           surfaces the non-error CDZ0305 `does not reduce to a value` warning — a dead non-normalizing
           computation is likely a bug. `_y` (underscore) silences the unused-BINDING CDZ0306 so only the
           CDZ0305 remains. (Migrated from rcdzc an_unused_non_normalizing_let_init_warns_but_still_compiles.)")
  (input (do (def (main) (let ((_y ((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1)))))) 0)) (export main)))
  (output (: 0 Int64))
  (count 1)
  (warning CDZ0305 (message "does not reduce to a value")))

(case
  "a USED non-normalizing binding is a hard CDZ0999 error, not merely warned"
  (doc
    "DCE consistency's other side: the SAME omega-style non-normalizing term, when its value IS used, is
           not dead code that can be elided — the reduction hits the compiler's limit and the component is
           DENIED with the hard CDZ0999 error (not a warning). Caught by the reduction limit, so it errors
           cleanly rather than overflowing. (Migrated from rcdzc a_non_normalizing … USED-term facet.)")
  (input (do (def (main) (let ((y ((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1)))))) y)) (export main)))
  (error CDZ0999 (message "does not reduce to a value")))

(case
  "a NORMAL unused let init is elided with NO dead-computation warning"
  (doc
    "The false-positive guard: a NORMALIZING unused init (`(+ 1 2)`) is ordinary dead code — elided, the
           program runs to 0, and NO CDZ0305 dead-computation warning fires (only a non-normalizing / trapping
           dead init earns CDZ0305). `_y` silences the unused-binding CDZ0306. (Migrated from rcdzc
           a_non_normalizing … no-false-positive facet.)")
  (input (do (def (main) (let ((_y (+ 1 2))) 0)) (export main)))
  (output (: 0 Int64))
  (no-diagnostic "does not reduce to a value"))

; The dead-trap warning fires for EVERY provably-trapping constant, not only integer ÷0 — the trap-
; observation elision + its CDZ0305 diagnostic are about WHETHER the value is observed, not WHICH trap the
; dead computation would raise. These two pin the other trap kinds as unprojected tuple elements (the clean
; CDZ0305-only carrier — a `let`/argument shape would also draw a CDZ0306 unused-binding/param warning),
; so a fold change that dropped the trap-proof for one kind (modulo, a constant overflow) is caught here.
(case
  "an unprojected element whose constant modulo-by-zero traps is elided but earns a CDZ0305 warning"
  (doc
    "The modulo face of the dead-trap warning: `(. (tuple 42 (% 100 0)) 0)` yields 42 (element 1's
           `(% 100 0)` is a constant modulo-by-zero the fold proves traps, but it is never projected, so it
           is unobserved and elided per core-semantics.md §A Trap Occurs Only Where Its Computation Is
           Observed). Because the elided computation PROVABLY traps, the build surfaces the non-error CDZ0305
           dead-trap warning exactly as the ÷0 case above. Pins that the dead-trap diagnostic covers %0, not
           only ÷0. Wasm-graded (warnings ride the shared compile stage; the run paths skip the (warns ..)
           check, not fail it).")
  (input (do (def (main) (. #tuple(42 (% 100 0)) 0)) (export main)))
  (output (: 42 Int64))
  (warns CDZ0305 (message "always traps but its value is never used")))

(case
  "an unprojected element whose constant overflow traps is elided but earns a CDZ0305 warning"
  (doc
    "The overflow face of the dead-trap warning: `(. (tuple 42 (+ 9223372036854775807 1)) 0)` yields 42
           (element 1's `Int64.max + 1` is a constant overflow the fold proves traps, but it is never
           projected, so it is unobserved and elided). The build surfaces the non-error CDZ0305 dead-trap
           warning as the ÷0 and %0 cases do. Pins that the dead-trap diagnostic covers a provable overflow,
           not only the divide/modulo family — the trap-KIND axis of core-semantics.md §285. Wasm-graded (the
           run paths skip the (warns ..) check, not fail it).")
  (input (do (def (main) (. #tuple(42 (+ 9223372036854775807 1)) 0)) (export main)))
  (output (: 42 Int64))
  (warns CDZ0305 (message "always traps but its value is never used")))

(case
  "an unprojected element whose constant zero-denominator Rational.of traps is elided but earns a CDZ0305 warning"
  (doc
    "The rational face of the dead-trap warning, completing the trap-KIND axis: `(. (tuple 42
           (Rational.of 1 0)) 0)` yields 42 (element 1's `(Rational.of 1 0)` has a zero denominator, which
           the fold proves has no rational value and traps, but it is never projected, so it is unobserved
           and elided). The build surfaces the non-error CDZ0305 dead-trap warning as the ÷0/%0/overflow
           cases do. Pins that the dead-trap diagnostic covers the zero-denominator rational construction —
           the fourth provable trap kind of core-semantics.md §285 — so the ruling is about observation, not
           the trap kind. Wasm-graded (the run paths skip the (warns ..) check, not fail it).")
  (input (do (def (main) (. #tuple(42 (Rational.of 1 0)) 0)) (export main)))
  (output (: 42 Int64))
  (warns CDZ0305 (message "always traps but its value is never used")))

; The POSITION axis of the dead-trap warning (the tuple cases above are the ELEMENT position): a provably-
; trapping constant in ANY unobserved slot — a dropped RECORD field, an unused LET init, or an unused
; ARGUMENT — is elided (the program runs) and earns exactly one CDZ0305. (Migrated from rcdzc
; an_eliminated_provable_trap_warns_but_still_compiles position sweep.)
(case
  "a provably-trapping dropped RECORD field is elided but earns one CDZ0305 dead-trap warning"
  (input (do (def (main) (. #record((= a 42) (= b (/ 100 0))) a)) (export main)))
  (output (: 42 Int64))
  (count 1)
  (warning CDZ0305 (message "always traps but its value is never used")))

(case
  "a provably-trapping unused LET init is elided but earns one CDZ0305 dead-trap warning"
  (input (do (def (main) (let ((_t (/ 100 0))) 5)) (export main)))
  (output (: 5 Int64))
  (count 1)
  (warning CDZ0305 (message "always traps but its value is never used")))

(case
  "a provably-trapping unused ARGUMENT is elided but earns one CDZ0305 dead-trap warning"
  (input (do (def (f x _y) x) (def (main) (f 7 (/ 100 0))) (export main)))
  (output (: 7 Int64))
  (count 1)
  (warning CDZ0305 (message "always traps but its value is never used")))

; The COMPLEMENT of the dead-trap warning: it fires only for a PROVABLY-trapping dropped computation. A
; clean program, and an UNPROVABLE (runtime-valued) computation in a dropped position, must NOT warn — the
; fold cannot prove the runtime `(/ 100 x)` traps, so no CDZ0305 (a false dead-trap warning on runtime code
; would be noise). (Migrated from rcdzc a_clean_program_and_an_unprovable_trap_do_not_warn.)
(case
  "a clean program with no dead computation earns no dead-trap warning"
  (input (do (def (main) (. #tuple(42 7) 0)) (export main)))
  (output (: 42 Int64))
  (no-diagnostic "always traps"))

(case
  "an UNPROVABLE (runtime) trap in a dropped position earns NO dead-trap warning"
  (input (do (def (f (: x Int64)) (. #tuple(42 (/ 100 x)) 0)) (def (main) (f 3)) (export main)))
  (output (: 42 Int64))
  (no-diagnostic "always traps"))

; The dead-COMPUTATION warning covers a computation with NO VALUE, not only a trapping one: the same
; CDZ0305 code + elision applies when an unobserved init does not REDUCE to a value (a non-terminating or
; explosively-growing reduction) — DCE consistency, an un-observed non-normalizing binding is elided
; exactly as an un-observed trap is. The message differs (`does not reduce to a value` vs `always traps`),
; so this is a distinct sub-face worth pinning. (The SAME term USED is a hard error, not a warning.)
(case
  "an unused let binding whose init does not reduce to a value is elided but earns a CDZ0305 warning"
  (doc
    "The no-normal-form face of the dead-computation warning: `(let ((y ((fn (v0) (v0 v0)) (fn (v1)
           (v1 (v1 v1)))))) 0)` binds `y` to a non-normalizing self-application (no normal form — the
           reduction grows without bound), but the body returns the constant `0`, never referencing `y`.
           `y`'s value is unobserved, so the fold ELIDES the binding rather than diverging — the program
           compiles and yields 0 — AND the build surfaces the non-error CDZ0305 warning, whose message names
           the non-normalizing reason (`does not reduce to a value`) rather than the trap wording of the
           cases above. This is DCE consistency: an un-observed non-normalizing binding is elided exactly as
           an un-observed trap is. (The SAME term USED — `… y` as the body — is a hard CDZ0999 error, the
           component is DENIED; only the UNUSED case warns.) Wasm-graded (the run paths skip the (warns ..)
           check, not fail it). Portable companion of the rcdzc
           an_unused_non_normalizing_let_init_warns_but_still_compiles test, which additionally pins the
           exactly-one count and the used-term-is-an-error contrast.")
  (input (do (def (main) (let ((y ((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1)))))) 0)) (export main)))
  (output (: 0 Int64))
  (warns CDZ0305 (message "does not reduce to a value")))

; ── The elision ruling is PURE-only: an unused binding whose init PERFORMS is NOT elidable ───────────
; The cases above establish that an unreferenced binding's init MAY be elided — for PURE inits, whose
; only observable is the value (and the trap observation would force). An init that PERFORMS an effect
; is different in kind: its VALUE may be dead, but its handler-state advance is observed by every later
; perform under the same handler. Eliding it would change those later values — so the boundary of the
; dead-binding rule is effectfulness, not referencedness alone. These pin that boundary with a counter
; arm whose state a second perform reads back: the unused binding's perform MUST still advance the
; state. This is exactly the edge a dead-code-elimination pass over bindings could get wrong (elide by
; use-count without an effect check), witnessed as a VALUE, not a trace.
(case
  "an unused let binding whose init performs still advances the handler state"
  (doc
    "`(let ((y (Fresh.next))) (Fresh.next))` under the counter arm seeded 0: the binding `y` is
           never referenced, but its init PERFORMS — the first `Fresh.next` reads 0 and advances the
           state to 1, so the SECOND perform (the body) reads 1. An implementation that elided the unused
           binding — applying the pure dead-binding rule above without an effect check — would return 0.
           Pins that the §A Trap Occurs Only Where Its Computation Is Observed elision licence is
           PURE-only: an effectful init is observed THROUGH THE HANDLER STATE even when its value is
           dead. Expected: 1.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (let ((y (Fresh.next))) (Fresh.next))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a discarded do-statement that performs still advances the handler state"
  (doc
    "The sequencing twin: `(do (Fresh.next) (Fresh.next))` seeded 0 — the non-final statement's
           value is discarded, but its perform advances the state, so the tail perform reads 1. Contrast
           the discarded PURE do-statement above (`(do (+ x 1) x)`), which need not run at all: a
           discarded statement is elidable exactly when it is pure. Expected: 1.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def (main) (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (do (Fresh.next) (Fresh.next))))
      (export main)))
  (output (: 1 Int64)))

; --- The common-OPERATOR if-arm hoist preserves trap and order semantics ---------------------------
; ba26196c9 hoists a common operator out of both if arms — `(if c (+ a 1) (+ b 1))` → `(+ (if c a b)
; 1)` — one checked op + one guard instead of two. Unlike the constructor hoist (payloads stay
; guarded), the OP itself moves BELOW the select, so its trap now fires after the selection: sound
; exactly when the trap depends only on the SELECTED operands. These pin that equivalence at the
; faces where it could break: overflow at one arm's operand, divisor selection, cond-vs-op trap
; order, effectful-cond evaluation count, and the mixed-operator decline.
(case
  "the taken arm's checked add still overflows under the operator hoist"
  (doc
    "`(if (> c 0) (+ a 1) (+ b 1))` with the TAKEN arm's operand at Int64.max: the hoisted
           single add receives the selected a = max and must trap 'integer overflow' exactly as the
           un-hoisted taken arm would. Pins the hoist keeps the guard on the shared op.")
  (input
    (do (def (main (: c Int64) (: a Int64) (: b Int64)) (if (> c 0) (+ a 1) (+ b 1))) (export main)))
  (call main (: 1 Int64) (: 9223372036854775807 Int64) (: 5 Int64))
  (trap "integer overflow"))

(case
  "an untaken arm's overflowing operand does not trap under the operator hoist"
  (doc
    "The complement: b = Int64.max in the UNTAKEN arm while the taken arm computes 1 + 1 = 2.
           The hoisted add sees only the SELECTED operand (a = 1), so no trap — the operand selection
           is what makes moving the op below the select value-preserving. A rewrite that evaluated
           both adds before selecting (op-first) would trap a program that must return 2.")
  (input
    (do (def (main (: c Int64) (: a Int64) (: b Int64)) (if (> c 0) (+ a 1) (+ b 1))) (export main)))
  (call main (: 1 Int64) (: 1 Int64) (: 9223372036854775807 Int64))
  (output (: 2 Int64)))

(case
  "a hoisted division traps by the selected divisor only"
  (doc
    "`(if (> c 0) (/ 100 a) (/ 100 b))` → the hoisted `(/ 100 (if c a b))`: with c false and
           a = 0, the selected divisor is b = 5 → 20 (the zero divisor sits in the untaken arm and
           must not trap); with c true the selected divisor IS the zero → 'divide by zero'. Both
           directions in one case pin that the division's partiality follows the SELECTION.")
  (input
    (do
      (def (main (: c Int64) (: a Int64) (: b Int64)) (if (> c 0) (/ 100 a) (/ 100 b)))
      (export main)))
  (call main (: 0 Int64) (: 0 Int64) (: 5 Int64))
  (output (: 20 Int64))
  (call main (: 1 Int64) (: 0 Int64) (: 5 Int64))
  (trap "divide by zero"))

(case
  "a trapping condition preempts the hoisted operator's trap"
  (doc
    "`(if (< (+ x 1) 5) (/ 100 d) (/ 100 d))` at x = Int64.max, d = 0 — BOTH the condition and
           the (fully shared) hoisted op trap for these inputs. Source order evaluates the condition
           first → 'integer overflow', never 'divide by zero'. The operator-hoist twin of the
           constructor hoist's shared-payload-vs-cond order pin: with every operand position equal,
           the hoist degenerates to `(/ 100 d)` guarded only by the cond's evaluation — which must
           still happen first.")
  (input
    (do (def (main (: x Int64) (: d Int64)) (if (< (+ x 1) 5) (/ 100 d) (/ 100 d))) (export main)))
  (call main (: 9223372036854775807 Int64) (: 0 Int64))
  (trap "integer overflow"))

(case
  "a shared trapping operand before the differing one does not preempt a trapping condition"
  (doc
    "`(if (< (+ e Int64.max) 0) (+ (/ 100 d) a) (+ (/ 100 d) b))` at e = 1, d = 0 — the single
           DIFFERING operand is the rhs (a vs b), so the hoist becomes `(+ (/ 100 d) (if c a b))` and
           the SHARED lhs `(/ 100 d)` is lifted OUTSIDE the per-operand select, ahead of the operand
           `if`. Both the condition (`e + Int64.max` overflows) and that shared lhs (÷0) trap for
           these inputs; source order evaluates the condition FIRST → 'integer overflow', never
           'divide by zero'. The diff==1 twin of the fully-shared order pin above: a trapping cond
           must not be hoisted past a shared preceding operand that also traps. Regression witness for
           the hoist_common_arith order guard (it was written with only the count check, mirroring the
           constructor-hoist order fix).")
  (input
    (do
      (def
        (main (: e Int64) (: d Int64) (: a Int64) (: b Int64))
        (if (< (+ e 9223372036854775807) 0) (+ (/ 100 d) a) (+ (/ 100 d) b)))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64) (: 5 Int64) (: 7 Int64))
  (trap "integer overflow"))

(case
  "a shared trapping operand before the differing one does not preempt a trapping condition (compare head)"
  (doc
    "The Compare-head twin of the Arith order pin above. `(if (< (+ e Int64.max) 0) (< (/ 100 d) a)
           (< (/ 100 d) b))` at e = 1, d = 0: the hoist becomes `(< (/ 100 d) (if c a b))`, lifting the
           SHARED lhs `(/ 100 d)` (÷0) outside the operand select, ahead of the operand `if`. A comparison
           is total (its VALUE never traps), which is why the Compare arm was added — but its OPERANDS can
           still be trapping arith, so the same trapping cond vs shared-operand order hazard applies. Source
           order evaluates the condition FIRST → 'integer overflow', never 'divide by zero'. Pins that the
           single order guard in hoist_common_arith covers the Compare head, not just Arith.")
  (input
    (do
      (def
        (main (: e Int64) (: d Int64) (: a Int64) (: b Int64))
        (if (< (+ e 9223372036854775807) 0) (< (/ 100 d) a) (< (/ 100 d) b)))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64) (: 5 Int64) (: 7 Int64))
  (trap "integer overflow"))

(case
  "an effectful condition is performed exactly once under the operator hoist"
  (doc
    "`(if (< (Ctr.tick) 1) (+ v 10) (+ v 20))` under a counter handler: the first perform
           returns 0 → the +10 arm → 15; the trailing `(Ctr.tick)` returns 1 (the state advanced
           exactly once) → 16. A hoist that re-evaluated the condition for the operand select AND the
           (degenerate) operator selection would skew the trailing read. The evaluate-once pin for
           the operator hoist, mirroring the constructor-hoist and match-sink counterparts.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: v Int64))
        (handle
          Ctr
          0
          ((tick (_) s (resume s (+ s 1))))
          (+ (if (< (Ctr.tick unit) 1) (+ v 10) (+ v 20)) (Ctr.tick unit))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 16 Int64)))

(case
  "mixed operators across the if arms keep their own arms"
  (doc
    "`(if (> c 0) (+ a 1) (- a 1))` — DIFFERENT operators, so the hoist must decline (there is
           no common op to lift; only the operand happens to agree): c false → 5 - 1 = 4, c true →
           5 + 1 = 6. A hoist keyed on operand agreement rather than operator identity would merge
           the arms into one op and get one direction wrong. The decline pin for the operator hoist.")
  (input (do (def (main (: c Int64) (: a Int64)) (if (> c 0) (+ a 1) (- a 1))) (export main)))
  (call main (: 0 Int64) (: 5 Int64))
  (output (: 4 Int64))
  (call main (: 1 Int64) (: 5 Int64))
  (output (: 6 Int64)))

; --- Boolean-identity folds preserve the left operand's traps and effects --------------------------
; fe142112b folds `(not (not x))` → x and and/or with a CONSTANT RIGHT operand: neutral (`and p
; true` / `or p false`) → p; absorbing (`and p false` / `or p true`) → the constant, but ONLY when
; the discarded p is trap-free (the `x * 0` discipline — the left operand is the always-evaluated
; condition). These grade the fold's contract from the running program's side: the absorbing fold
; must KEEP a trapping/effectful p, the neutral fold must keep p's value AND trap, and negation
; parity must survive composition.
(case
  "an absorbing and-false keeps its trapping left operand"
  (doc
    "`(and (> (/ 10 n) 0) false)` at n = 0: the conjunction's VALUE is decided (false), but the
           left operand traps — and the left of a connective is its always-evaluated condition, so the
           div-by-zero MUST still fire (core-semantics: partial operations have a defined outcome; the
           absorbing fold applies only to a trap-free left). A fold that discarded p unconditionally
           returns 0 where the program must trap.")
  (input (do (def (main (: n Int64)) (if (and (> (/ 10 n) 0) false) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "an absorbing and-false discards a trap-free left operand's value"
  (doc
    "The fold's positive face: `(and (> n 0) false)` with a TRAP-FREE left is constantly false
           — n = 2 (left true) still answers 0. Together with the trapping case above this pins the
           absorbing fold's exact gate: value-discard is fine, evaluation-discard is not.")
  (input (do (def (main (: n Int64)) (if (and (> n 0) false) 1 0)) (export main)))
  (call main (: 2 Int64))
  (output (: 0 Int64)))

(case
  "an absorbing or-true keeps an effectful left operand's perform"
  (doc
    "The EFFECT twin of the trap case: `(or (< (Ctr.tick) 99) true)` is constantly true, but the
           left operand PERFORMS — the counter must advance exactly once before the trailing read.
           tick returns 0 (condition true → 10), the trailing `(Ctr.tick)` returns 1 → 11. A fold that
           discarded the effectful left answers 10 (the trailing tick reads 0); one that duplicated it
           answers 12. Observable state pins evaluate-exactly-once through the absorbing fold.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: d Int64))
        (handle
          Ctr
          0
          ((tick (_) s (resume s (+ s 1))))
          (+ (if (or (< (Ctr.tick unit) 99) true) 10 20) (Ctr.tick unit))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64)))

(case
  "a neutral and-true keeps both the trap and the value of its left operand"
  (doc
    "`(and p true)` → p — the NEUTRAL fold hands back the left operand whole: its trap fires at
           n = 0 (the div), and its VALUE decides the conditional at trap-free inputs — n = 20 →
           `(> 0 0)` false → 0; n = 5 → `(> 2 0)` true → 1. Three calls pin that the neutral fold is
           the identity on p (not a constant-true absorption, not a trap-suppressing rewrite).")
  (input (do (def (main (: n Int64)) (if (and (> (/ 10 n) 0) true) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero")
  (call main (: 20 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a neutral or-false keeps both the trap and the value of its left operand"
  (doc
    "`(or p false)` → p — `false` is `or`'s identity, so the NEUTRAL fold hands back the left
           operand whole: its trap fires at n = 0 (the div), and its VALUE decides the conditional at
           trap-free inputs — n = 20 → `(> 0 0)` false → 0; n = 5 → `(> 2 0)` true → 1. The `or`
           companion of the neutral and-true fold above (not a constant absorption, not a trap-suppress).")
  (input (do (def (main (: n Int64)) (if (or (> (/ 10 n) 0) false) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero")
  (call main (: 20 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "double negation of a runtime boolean is the identity"
  (doc
    "`(not (not (> n 0)))` = `(> n 0)` — both truth values verified (n = 5 → 1, n = -1 → 0).
           The fold cancels the pair; parity (not the mere presence of `not`) must decide the answer.")
  (input (do (def (main (: n Int64)) (if (not (not (> n 0))) 1 0)) (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: -1 Int64))
  (output (: 0 Int64)))

(case
  "triple negation of a runtime boolean is a single negation"
  (doc
    "`(not (not (not (> n 0))))` — the ODD-parity companion: the fold may cancel exactly one
           pair, leaving ONE live negation (n = 5 → 0, n = -1 → 1). A cancellation that consumed all
           three (treating the fold as 'strip every not') flips both answers.")
  (input (do (def (main (: n Int64)) (if (not (not (not (> n 0)))) 1 0)) (export main)))
  (call main (: 5 Int64))
  (output (: 0 Int64))
  (call main (: -1 Int64))
  (output (: 1 Int64)))

(case
  "a short-circuit and with identical operands folds idempotently to the operand"
  (doc
    "IDEMPOTENCE: `(and a a)` → `a` (the redundant re-evaluation is dropped; `a` is the
           short-circuit condition, evaluated once). The value is just `a`: true → 1, false → 0.")
  (input (do (def (main (: a Bool)) (if (and a a) 1 0)) (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 0 Int64)))

(case
  "a short-circuit or with identical operands folds idempotently to the operand"
  (doc "The `or` companion: `(or a a)` → `a`. true → 1, false → 0.")
  (input (do (def (main (: a Bool)) (if (or a a) 1 0)) (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 0 Int64)))

(case
  "the idempotence fold keeps a repeated trapping operand's trap"
  (doc
    "The idempotent fold drops the redundant re-evaluation but KEEPS the operand's effects/traps —
           `a` is still evaluated once as the short-circuit condition. `(and (> (/ 10 n) 0) (> (/ 10 n)
           0))` at n = 0 traps on the div-by-zero (the fold must not eliminate `a` entirely); n = 2 → 1.")
  (input (do (def (main (: n Int64)) (if (and (> (/ 10 n) 0) (> (/ 10 n) 0)) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero")
  (call main (: 2 Int64))
  (output (: 1 Int64)))

(case
  "a short-circuit and of a boolean and its negation folds to false"
  (doc
    "BOOLEAN COMPLEMENT LAW: `(and a (not a))` → false — a boolean and its negation are exclusive,
           so the conjunction is always false regardless of `a`. true → 0, false → 0.")
  (input (do (def (main (: a Bool)) (if (and a (not a)) 1 0)) (export main)))
  (call main (: true Bool))
  (output (: 0 Int64))
  (call main (: false Bool))
  (output (: 0 Int64)))

(case
  "a short-circuit or of a boolean and its negation folds to true"
  (doc
    "The exhaustive complement law: `(or a (not a))` → true — a boolean and its negation are
           exhaustive. true → 1, false → 1.")
  (input (do (def (main (: a Bool)) (if (or a (not a)) 1 0)) (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 1 Int64)))

(case
  "the boolean complement-law fold keeps a trapping operand's trap"
  (doc
    "The complement-law fold discards both operands (it answers a constant), so it is gated on
           trap-freedom — a trapping operand keeps the runtime form and traps. `(and (> (/ 10 n) 0) (not
           (> (/ 10 n) 0)))` at n = 0 traps on the div; n = 2 → 0 (the conjunction is false).")
  (input
    (do (def (main (: n Int64)) (if (and (> (/ 10 n) 0) (not (> (/ 10 n) 0))) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero")
  (call main (: 2 Int64))
  (output (: 0 Int64)))

(case
  "a same-direction and of two upper-bound comparisons subsumes to the tighter bound"
  (doc
    "`(and (< x 5) (< x 10))` — both are upper bounds; the AND keeps the TIGHTER `(< x 5)`. x = 3 →
           true, x = 7 → false (not < 5), x = 12 → false.")
  (input (do (def (main (: x Int64)) (if (and (< x 5) (< x 10)) 1 0)) (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 7 Int64))
  (output (: 0 Int64))
  (call main (: 12 Int64))
  (output (: 0 Int64)))

(case
  "a same-direction or of two upper-bound comparisons subsumes to the looser bound"
  (doc
    "`(or (< x 5) (< x 10))` — the OR keeps the LOOSER `(< x 10)`. x = 3 → true, x = 7 → true (< 10),
           x = 12 → false.")
  (input (do (def (main (: x Int64)) (if (or (< x 5) (< x 10)) 1 0)) (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 7 Int64))
  (output (: 1 Int64))
  (call main (: 12 Int64))
  (output (: 0 Int64)))

(case
  "the same-direction subsumption fold keeps a trapping operand's trap"
  (doc
    "The kept comparison still evaluates the shared operand, so a trapping `(/ 100 z)` traps.
           `(and (< (/ 100 z) 5) (< (/ 100 z) 10))` at z = 0 traps on the div.")
  (input
    (do (def (main (: z Int64)) (if (and (< (/ 100 z) 5) (< (/ 100 z) 10)) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "two inclusive bounds at the same point collapse to equality"
  (doc
    "`(and (>= x 5) (<= x 5))` is satisfied only at x = 5 — the two inclusive bounds collapse to
           `(= x 5)`. 4 → 0, 5 → 1, 6 → 0.")
  (input (do (def (main (: x Int64)) (if (and (>= x 5) (<= x 5)) 1 0)) (export main)))
  (call main (: 4 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 6 Int64))
  (output (: 0 Int64)))

(case
  "two inclusive bounds collapse to equality over an unsigned width"
  (doc
    "The UInt64 face of the inclusive-bounds collapse: `(and (>= x 5) (<= x 5))` = `(= x 5)` — 4 → 0,
           5 → 1, 6 → 0.")
  (input (do (def (main (: x UInt64)) (if (and (>= x 5) (<= x 5)) 1 0)) (export main)))
  (call main (: 4 UInt64))
  (output (: 0 Int64))
  (call main (: 5 UInt64))
  (output (: 1 Int64))
  (call main (: 6 UInt64))
  (output (: 0 Int64)))

(case
  "two inclusive bounds collapse to equality at a negative point"
  (doc "`(and (>= x -3) (<= x -3))` = `(= x -3)` — -2 → 0, -3 → 1, -4 → 0.")
  (input (do (def (main (: x Int64)) (if (and (>= x -3) (<= x -3)) 1 0)) (export main)))
  (call main (: -2 Int64))
  (output (: 0 Int64))
  (call main (: -3 Int64))
  (output (: 1 Int64))
  (call main (: -4 Int64))
  (output (: 0 Int64)))

(case
  "the inclusive-bounds collapse keeps a trapping operand's trap"
  (doc
    "`(and (>= (/ 100 z) 5) (<= (/ 100 z) 5))` at z = 0 traps — the discarded second bound's trap is
           preserved (both bounds share the trapping `/`).")
  (input
    (do (def (main (: z Int64)) (if (and (>= (/ 100 z) 5) (<= (/ 100 z) 5)) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "an equality combined with a satisfiable range folds to the equality"
  (doc
    "`(and (= x 5) (> x 0))` — the equality pins x = 5, which satisfies `(> x 0)`, so the AND is
           just `(= x 5)`: x = 5 → true, x = 3 → false, x = 200 → false.")
  (input (do (def (main (: x Int64)) (if (and (= x 5) (> x 0)) 1 0)) (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64))
  (call main (: 200 Int64))
  (output (: 0 Int64)))

(case
  "an equality combined with a contradicting range folds to false"
  (doc
    "`(and (= x 5) (> x 100))` — x = 5 cannot also exceed 100, a contradiction → always false (5, 3,
           200 all → 0).")
  (input (do (def (main (: x Int64)) (if (and (= x 5) (> x 100)) 1 0)) (export main)))
  (call main (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64))
  (call main (: 200 Int64))
  (output (: 0 Int64)))

(case
  "an equality or-ed with a covering range subsumes to the range"
  (doc
    "`(or (= x 5) (>= x 0))` — the point x = 5 is already inside `(>= x 0)`, so the OR subsumes to
           `(>= x 0)`: x = 5 → true, x = 3 → true, x = -5 → false. The not-covered companion `(or (= x 5)
           (> x 100))` keeps the extra point: x = 5 → true, x = 50 → false.")
  (input (do (def (main (: x Int64)) (if (or (= x 5) (>= x 0)) 1 0)) (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: -5 Int64))
  (output (: 0 Int64)))

(case
  "an equality or-ed with a non-covering range keeps the extra point"
  (doc
    "`(or (= x 5) (> x 100))` — 5 is NOT inside `(> x 100)`, so the equality point survives: x = 5 →
           true (the point), x = 50 → false.")
  (input (do (def (main (: x Int64)) (if (or (= x 5) (> x 100)) 1 0)) (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 50 Int64))
  (output (: 0 Int64)))

(case
  "the equality-and-range contradiction fold keeps a trapping operand's trap"
  (doc
    "`(and (= (/ 10 z) 5) (> (/ 10 z) 100))` at z = 0 traps — the contradiction fold must not
           discard the trapping shared operand.")
  (input (do (def (main (: z Int64)) (if (and (= (/ 10 z) 5) (> (/ 10 z) 100)) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "opposite-direction comparisons over a disjoint gap fold their and to false"
  (doc
    "`(and (< x 5) (> x 10))` — nothing is both below 5 and above 10 (a disjoint gap), so the AND is
           always false: 0, 3, 5, 12 all → 0.")
  (input (do (def (main (: x Int64)) (if (and (< x 5) (> x 10)) 1 0)) (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64))
  (call main (: 12 Int64))
  (output (: 0 Int64)))

(case
  "opposite-direction comparisons that cover the line fold their or to true"
  (doc
    "`(or (< x 5) (> x 3))` — every value is below 5 OR above 3 (the ranges overlap and cover the
           line), so the OR is always true: 0, 4, 100, -5 all → 1.")
  (input (do (def (main (: x Int64)) (if (or (< x 5) (> x 3)) 1 0)) (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 4 Int64))
  (output (: 1 Int64))
  (call main (: 100 Int64))
  (output (: 1 Int64))
  (call main (: -5 Int64))
  (output (: 1 Int64)))

(case
  "an opposite-direction or with a real gap computes its non-constant value"
  (doc
    "`(or (< x 3) (> x 10))` leaves a real gap [3,10] and does NOT fold to a constant — a value in
           the gap is false: x = 5 → 0.")
  (input (do (def (main (: x Int64)) (if (or (< x 3) (> x 10)) 1 0)) (export main)))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "an opposite-direction and over a non-empty interval computes its non-constant value"
  (doc
    "`(and (< x 10) (> x 5))` is the non-empty interval 5 < x < 10 (does NOT fold to false): x = 7 →
           1, x = 3 → 0.")
  (input (do (def (main (: x Int64)) (if (and (< x 10) (> x 5)) 1 0)) (export main)))
  (call main (: 7 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64))
  (output (: 0 Int64)))

(case
  "the opposite-direction disjoint-and fold keeps a trapping operand's trap"
  (doc
    "`(and (< (/ 100 z) 5) (> (/ 100 z) 10))` at z = 0 traps — the disjoint-and fold (which answers
           the constant false) must not discard the trapping shared operand.")
  (input
    (do (def (main (: z Int64)) (if (and (< (/ 100 z) 5) (> (/ 100 z) 10)) 1 0)) (export main)))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "an absorbed conjunction composes as a live disjunction's operand"
  (doc
    "`(or (and (> n 0) false) (> m 0))` — the inner conjunction folds to constant false, which
           is the DISJUNCTION's neutral element, so the whole expression folds to `(> m 0)`: m decides
           (m = 0 → 0, m = 3 → 1), n never does. Pins the identities COMPOSING: the absorbing fold's
           output feeds the neutral fold correctly rather than latching the outer connective.")
  (input
    (do
      (def (main (: n Int64) (: m Int64)) (if (or (and (> n 0) false) (> m 0)) 1 0))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 0 Int64) (: 3 Int64))
  (output (: 1 Int64)))

; ── The BOOLEAN ABSORPTION LAWS: `a && (a || b)` → a and `a || (a && b)` → a ──────────────────────────
; The logical analogue of the bitwise absorption law (`x & (x|y) → x`): a value combined with the DUAL
; connective of itself-with-anything absorbs to itself. `arith_identity`/`bool_absorption_operand`
; (lower.rs) folds `(and a (or a b))` → a and `(or a (and a b))` → a — the result depends ONLY on `a`,
; never on `b`. Observed via `(if … 1 0)` on runtime comparison operands (so the connectives are EMITTED,
; not const-folded): with `a` = `(> a 0)`, the answer tracks `a` alone regardless of `b`. Both backends.
(case
  "a repeated conjunct absorbs to the plain conjunction"
  (doc
    "`(and (and a b) a)` — the repeated `a` is absorbed; the value is just `a && b`. (T,T) → 1,
           (T,F) → 0, (F,T) → 0.")
  (input (do (def (main (: a Bool) (: b Bool)) (if (and (and a b) a) 1 0)) (export main)))
  (call main (: true Bool) (: true Bool))
  (output (: 1 Int64))
  (call main (: true Bool) (: false Bool))
  (output (: 0 Int64))
  (call main (: false Bool) (: true Bool))
  (output (: 0 Int64)))

(case
  "a repeated disjunct absorbs to the plain disjunction"
  (doc
    "`(or (or a b) b)` — the repeated `b` is absorbed; the value is `a || b`. (T,F) → 1, (F,T) → 1,
           (F,F) → 0.")
  (input (do (def (main (: a Bool) (: b Bool)) (if (or (or a b) b) 1 0)) (export main)))
  (call main (: true Bool) (: false Bool))
  (output (: 1 Int64))
  (call main (: false Bool) (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool) (: false Bool))
  (output (: 0 Int64)))

(case
  "the repeated-conjunct absorb keeps a trapping nested operand's trap"
  (doc
    "The nested node is retained, so a trapping operand in it still traps. `(and (and (> (/ 10 n) 0)
           b) (> (/ 10 n) 0))` at n = 0 traps on the div.")
  (input
    (do
      (def (main (: n Int64) (: b Bool)) (if (and (and (> (/ 10 n) 0) b) (> (/ 10 n) 0)) 1 0))
      (export main)))
  (call main (: 0 Int64) (: true Bool))
  (trap "divide by zero"))

(case
  "a comparison pair split across a nested connective reassociates and folds to the interval"
  (doc
    "`(and (and (> x 0) (< x 100)) (> x 5))` — `(> x 0)` is subsumed by `(> x 5)` across the nested
           `and`, folding to the interval 5 < x < 100. x = 6 → 1, 99 → 1, 5 → 0, 100 → 0, -5 → 0.")
  (input (do (def (main (: x Int64)) (if (and (and (> x 0) (< x 100)) (> x 5)) 1 0)) (export main)))
  (call main (: 6 Int64))
  (output (: 1 Int64))
  (call main (: 99 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 100 Int64))
  (output (: 0 Int64))
  (call main (: -5 Int64))
  (output (: 0 Int64)))

(case
  "a complementary comparison split across a nested connective folds to false"
  (doc
    "`(and (and (< x y) (> x 0)) (>= x y))` — `(< x y)` and `(>= x y)` are complements reassociated
           across the nested `and`, so the conjunction is always false: (3,5) → 0, (5,3) → 0, (10,2) → 0.")
  (input
    (do
      (def (main (: x Int64) (: y Int64)) (if (and (and (< x y) (> x 0)) (>= x y)) 1 0))
      (export main)))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64) (: 3 Int64))
  (output (: 0 Int64))
  (call main (: 10 Int64) (: 2 Int64))
  (output (: 0 Int64)))

(case
  "the split-comparison reassociation declines a trapping leaf and keeps its trap"
  (doc
    "The reassociation needs all leaves trap-free, so a trapping leaf declines it, keeping the
           runtime form. `(and (and (> (/ 10 n) 0) (< x 100)) (> (/ 10 n) 5))` at n = 0 traps.")
  (input
    (do
      (def
        (main (: n Int64) (: x Int64))
        (if (and (and (> (/ 10 n) 0) (< x 100)) (> (/ 10 n) 5)) 1 0))
      (export main)))
  (call main (: 0 Int64) (: 50 Int64))
  (trap "divide by zero"))

(case
  "the boolean absorption laws absorb with swapped operand order too"
  (doc
    "The commuted faces of the absorption laws (operand order swapped from the canonical `(and a (or
           a b))` / `(or a (and a b))` below): `(or (and a b) a)` → a and `(and (or a b) a)` → a. `main`
           returns `(tuple (if (or (and (> a 0) (> b 0)) (> a 0)) 1 0) (if (and (or (> a 0) (> b 0)) (> a
           0)) 1 0))` = `(> a 0)` in both, so `b` is irrelevant: (5,-1) → (1,1), (-1,5) → (0,0).")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        #tuple((if (or (and (> a 0) (> b 0)) (> a 0)) 1 0)
          (if (and (or (> a 0) (> b 0)) (> a 0)) 1 0)))
      (export main)))
  (call main (: 5 Int64) (: -1 Int64))
  (output (: (tuple 1 1) (Tuple Int64 Int64)))
  (call main (: -1 Int64) (: 5 Int64))
  (output (: (tuple 0 0) (Tuple Int64 Int64)))
  (live-objects known-leak))

(case
  "the dual-absorption fold keeps a trapping absorbed operand's short-circuit form"
  (doc
    "When the absorbed-away operand carries a trap, the fold's trap-free guard declines, leaving the
           real short-circuit form. `(and (or a (> (/ 10 n) 0)) a)`: a = false forces the `(or …)` right
           operand → the trapping `/` fires at n = 0; a = true short-circuits before the trap → 1.")
  (input
    (do (def (main (: a Bool) (: n Int64)) (if (and (or a (> (/ 10 n) 0)) a) 1 0)) (export main)))
  (call main (: false Bool) (: 0 Int64))
  (trap "divide by zero")
  (call main (: true Bool) (: 0 Int64))
  (output (: 1 Int64)))

(case
  "the boolean absorption laws reduce a-and-(a-or-b) and a-or-(a-and-b) to a"
  (doc
    "`(and a (or a b))` → a and `(or a (and a b))` → a: a boolean combined with the DUAL connective
           of itself-with-anything absorbs to itself, so `b` is irrelevant. `main` returns `(tuple (and (>
           a 0) (or (> a 0) (> b 0))) (or (> a 0) (and (> a 0) (> b 0))))` as two conditionals folded to
           `(> a 0)`: at (a=5, b=-1) → (1, 1) (a>0 true), at (a=-1, b=5) → (0, 0) (a>0 false) — b flips
           across the two calls yet never changes the answer. Pins both boolean absorption laws on runtime
           operands, both backends.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        #tuple((if (and (> a 0) (or (> a 0) (> b 0))) 1 0)
          (if (or (> a 0) (and (> a 0) (> b 0))) 1 0)))
      (export main)))
  (call main (: 5 Int64) (: -1 Int64))
  (output (: (tuple 1 1) (Tuple Int64 Int64)))
  (call main (: -1 Int64) (: 5 Int64))
  (output (: (tuple 0 0) (Tuple Int64 Int64)))
  (live-objects known-leak))

; ── COMPLEMENTARY COMPARISONS: two ordering tests that PARTITION every value fold to true / false ─────
; When two comparisons on the SAME operand pair are exact complements over the total order — `<`/`>=` or
; `<=`/`>` (same operand order) — they partition every value: their `or` is exhaustive (always TRUE) and
; their `and` is disjoint (always FALSE). `complementary_comparisons` (lower.rs) folds `(or (< a b) (>= a
; b))` → true and `(and (< a b) (>= a b))` → false (caller trap-guards the discard). Observed via `(if …
; 1 0)` on runtime operands so the comparisons EMIT; the answer is fixed regardless of a/b — pinned for
; BOTH complement pairs including the boundary `a == b` (where `<=/>` must still partition). Both backends.
(case
  "complementary ordering comparisons fold their or to true and their and to false"
  (doc
    "`(or (< a b) (>= a b))` → true (exhaustive) and `(and (< a b) (>= a b))` → false (disjoint), and
           the same for the `<=`/`>` pair. `main` = `(tuple (if (or (< a b) (>= a b)) 1 0) (if (and (< a b)
           (>= a b)) 1 0) (if (or (<= a b) (> a b)) 1 0) (if (and (<= a b) (> a b)) 1 0))`. At (a=3, b=5) →
           (1, 0, 1, 0) and at (a=5, b=5) (the EQUAL boundary) → (1, 0, 1, 0): the or is always 1, the and
           always 0, at every relation of a to b. Pins that complementary comparisons partition every value
           (or exhaustive, and disjoint) for both `<`/`>=` and `<=`/`>`, both backends.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64))
        #tuple((if (or (< a b) (>= a b)) 1 0)
          (if (and (< a b) (>= a b)) 1 0)
          (if (or (<= a b) (> a b)) 1 0)
          (if (and (<= a b) (> a b)) 1 0)))
      (export main)))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: (tuple 1 0 1 0) (Tuple Int64 Int64 Int64 Int64)))
  (call main (: 5 Int64) (: 5 Int64))
  (output (: (tuple 1 0 1 0) (Tuple Int64 Int64 Int64 Int64)))
  ; The returned tuple is fully CONSTANT (the complementary comparisons fold to 1/0 regardless of a,b), so it
  ; now hoists build-once (WIT static encoding) to a census-excluded immortal — the former per-call leak is gone.
  (live-objects 0))

; The complementary-comparison fold above fires on IDENTICAL operand pairs. These pin its GUARDS —
; the faces where a fold keyed on comparison SHAPE rather than operand identity (or one that dropped
; the discarded operand's evaluation) would miscompile. The fold's own comment says "caller
; trap-guards the discard"; these witness both that guard and the two identity preconditions.
(case
  "a mirrored equality pair under and folds by commutativity to one equality"
  (doc
    "`=` is COMMUTATIVE, so `(= a b)` and `(= b a)` are the SAME predicate; `(and (= a b) (= b a))`
           idempotently folds to `(= a b)`. The VALUE is `a == b`: (3,3) → 1, (3,5) → 0, (0,0) → 1. The
           `and`-connective mirrored-equality fold.")
  (input (do (def (main (: a Int64) (: b Int64)) (if (and (= a b) (= b a)) 1 0)) (export main)))
  (call main (: 3 Int64) (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 0 Int64) (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a mirrored equality pair under or folds by commutativity to one equality"
  (doc
    "The `or` companion: `(or (= a b) (= b a))` also folds to `(= a b)` (commutative, idempotent).
           Value `a == b`: (3,3) → 1, (3,5) → 0, (0,0) → 1.")
  (input (do (def (main (: a Int64) (: b Int64)) (if (or (= a b) (= b a)) 1 0)) (export main)))
  (call main (: 3 Int64) (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 0 Int64) (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a swapped-operand ordering pair is a contradiction that does not mis-fold to one comparison"
  (doc
    "Ordering is NOT commutative: `(< a b)` and `(< b a)` are DIFFERENT (a disjoint pair), so `(and
           (< a b) (< b a))` is a CONTRADICTION — always false (a and b cannot each be strictly less than
           the other). A bogus swap-idempotence that collapsed them to one `<` would be wrong; the value
           is false at every relation: (3,5) → 0, (5,3) → 0, (4,4) → 0.")
  (input (do (def (main (: a Int64) (: b Int64)) (if (and (< a b) (< b a)) 1 0)) (export main)))
  (call main (: 3 Int64) (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64) (: 3 Int64))
  (output (: 0 Int64))
  (call main (: 4 Int64) (: 4 Int64))
  (output (: 0 Int64)))

(case
  "the commutative equality swap-fold declines a trapping operand and keeps both compares"
  (doc
    "The commutative-swap fold (`(= a b)` ~ `(= b a)`) requires a TRAP-FREE operand, so a trapping
           operand declines the swap — both compares stay and the trap fires. `(and (= (/ 10 a) b) (= b
           (/ 10 a)))` at a = 0: the `(/ 10 0)` traps (the fold must not drop it by mis-swapping). Pins
           the trap-safety guard of the mirrored-equality fold.")
  (input
    (do
      (def (main (: a Int64) (: b Int64)) (if (and (= (/ 10 a) b) (= b (/ 10 a))) 1 0))
      (export main)))
  (call main (: 0 Int64) (: 5 Int64))
  (trap "divide by zero"))

(case
  "complementary comparisons over different operand pairs do not fold to a tautology"
  (doc
    "`(or (< a b) (>= a c))` with b ≠ c is NOT exhaustive — at a=5,b=3,c=9 BOTH disjuncts are false
           (5 ≮ 3, 5 < 9) → 0; at a=5,b=9,c=9 the first holds → 1. A fold matching only the comparison
           SHAPE `(or (< _ _) (>= _ _))` without checking the operand pairs are IDENTICAL folds this to
           true and returns 1 at the first call. The operand-identity precondition of the tautology fold.
           Expected: 0, 1.")
  (input
    (do
      (def (main (: a Int64) (: b Int64) (: c Int64)) (if (or (< a b) (>= a c)) 1 0))
      (export main)))
  (call main (: 5 Int64) (: 3 Int64) (: 9 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64) (: 9 Int64) (: 9 Int64))
  (output (: 1 Int64)))

(case
  "the tautology fold keeps a trapping shared operand"
  (doc
    "`(or (< (/ a z) 5) (>= (/ a z) 5))` IS the exhaustive pair — the fold may answer true — but
           the shared operand `(/ a z)` carries a divide-by-zero at z = 0: the fold must not DISCARD its
           evaluation (the trap is a defined outcome; the same is_trap_free discipline as the x·0
           annihilator). z=2 → 1 (folded or not); z=0 → trap. Expected: 1, trap.")
  (input
    (do
      (def (main (: a Int64) (: z Int64)) (if (or (< (/ a z) 5) (>= (/ a z) 5)) 1 0))
      (export main)))
  (call main (: 10 Int64) (: 2 Int64))
  (output (: 1 Int64))
  (call main (: 10 Int64) (: 0 Int64))
  (trap "integer divide by zero"))

(case
  "swapped-operand complements do not fold — less-than or flipped greater-equal is not a tautology"
  (doc
    "`(or (< a b) (>= b a))` LOOKS complementary but `(>= b a)` is `(<= a b)`, not `(>= a b)` — the
           pair is exhaustive only at a ≠ b... in fact `(< a b) ∨ (<= a b)` = `(<= a b)`, which is FALSE
           at a > b: a=7,b=3 → 0 (both disjuncts false); a=3,b=7 → 1. A fold that matched the operator
           pair but ignored operand ORDER would fold to true and return 1 at the first call. The
           operand-order precondition, completing the guard set with the different-pairs and trap faces
           above. Expected: 1, 0.")
  (input (do (def (main (: a Int64) (: b Int64)) (if (or (< a b) (>= b a)) 1 0)) (export main)))
  (call main (: 3 Int64) (: 7 Int64))
  (output (: 1 Int64))
  (call main (: 7 Int64) (: 3 Int64))
  (output (: 0 Int64)))

; ── REASSOCIATION lets a complementary pair fold even when NESTED past a third operand ────────────────
; The complementary-comparison fold above fires on an ADJACENT pair. `reassociate_comparison_pair`
; (lower.rs) extends it: when a comparison `c` is `and`/`or`-ed with a SAME-connective nested pair
; `(op p q)`, it reassociates so `c` folds against `p` (or `q`) — so a complementary pair SEPARATED by a
; third operand still collapses. `(and (< a b) (and (>= a b) (> c 0)))`: reassociating `(< a b)` with the
; nested `(>= a b)` gives the exhaustive-FALSE `and`, so the whole conjunction is false regardless of `c`;
; the `or` dual is true regardless. All operands are pure comparisons (trap-free), so the regrouping is
; unobservable. Both backends.
(case
  "a complementary comparison pair nested past a third operand still folds via reassociation"
  (doc
    "`(and (< a b) (and (>= a b) (> c 0)))` → false and `(or (< a b) (or (>= a b) (> c 0)))` → true:
           `reassociate_comparison_pair` regroups the outer comparison with the matching nested leaf so the
           complementary `< a b`/`>= a b` pair folds (`and` disjoint → false, `or` exhaustive → true), and
           the third operand `(> c 0)` is then irrelevant. `main` = `(tuple (if (and (< a b) (and (>= a b)
           (> c 0))) 1 0) (if (or (< a b) (or (>= a b) (> c 0))) 1 0)`. At (a=3, b=5, c=9) → (0, 1) and at
           (a=5, b=3, c=-9) → (0, 1): a/b/c all vary yet the answer is fixed (0 for the and, 1 for the or).
           Pins that reassociation lets a complementary pair fold across a nested third operand, both
           backends.")
  (input
    (do
      (def
        (main (: a Int64) (: b Int64) (: c Int64))
        #tuple((if (and (< a b) (and (>= a b) (> c 0))) 1 0)
          (if (or (< a b) (or (>= a b) (> c 0))) 1 0)))
      (export main)))
  (call main (: 3 Int64) (: 5 Int64) (: 9 Int64))
  (output (: (tuple 0 1) (Tuple Int64 Int64)))
  (call main (: 5 Int64) (: 3 Int64) (: -9 Int64))
  (output (: (tuple 0 1) (Tuple Int64 Int64)))
  ; Fully-constant folded tuple → build-once immortal (WIT static encoding), census-excluded, no per-call leak.
  (live-objects 0))

; ── EQUALITY does not subsume: two `=` to different constants are a contradiction / a 2-point set ─────
; The same-direction subsumption fold (upper/lower half-lines collapse to the tighter/looser bound) is
; keyed on ORDERING operators only. A miscompile once let `Eq` in, so `(and (= x 5) (= x 6))` "subsumed"
; to `(= x 6)` (returns 1 at x=6) — but two equalities to DIFFERENT constants are a CONTRADICTION under
; `and` (x cannot equal both) and a 2-point set under `or` (neither keeps just one). Same-CONSTANT
; equality still folds idempotently, and legitimate ordering subsumption is unaffected. Regression pins.
(case
  "two equalities to different constants are an and-contradiction that does not subsume"
  (doc
    "`(and (= x 5) (= x 6))` is ALWAYS false — x cannot equal both 5 and 6 — including at the two
           miscompiled points x=5 and x=6. A subsumption fold that wrongly admitted `Eq` collapsed this to
           `(= x 6)` and returned 1 at x=6. Value is 0 everywhere: x=5,6,7,0 → 0. The Eq-exclusion of the
           same-direction subsumption fold.")
  (input (do (def (f (: x Int64)) (if (and (= x 5) (= x 6)) 1 0)) (export f)))
  (call f (: 5 Int64))
  (output (: 0 Int64))
  (call f (: 6 Int64))
  (output (: 0 Int64))
  (call f (: 7 Int64))
  (output (: 0 Int64))
  (call f (: 0 Int64))
  (output (: 0 Int64)))

(case
  "two equalities to different constants under or are the 2-point set neither collapses to"
  (doc
    "`(or (= x 5) (= x 6))` is the 2-point set {5, 6} — x=5 → 1, x=6 → 1, x=7 → 0. A subsumption
           fold admitting `Eq` would drop one point. Complements the and-contradiction above.")
  (input (do (def (f (: x Int64)) (if (or (= x 5) (= x 6)) 1 0)) (export f)))
  (call f (: 5 Int64))
  (output (: 1 Int64))
  (call f (: 6 Int64))
  (output (: 1 Int64))
  (call f (: 7 Int64))
  (output (: 0 Int64)))

(case
  "a same-constant equality pair still folds idempotently and ordering subsumption is unaffected"
  (doc
    "The Eq-exclusion is precise: same-CONSTANT equality is idempotence, not subsumption, so `(and
           (= x 5) (= x 5))` still folds to `x == 5` (x=5 → 1, x=6 → 0); and a legitimate ordering
           subsumption `(and (>= x 5) (>= x 10))` = `(>= x 10)` is unchanged (x=7 → 0, x=12 → 1). `main` =
           `(tuple (if (and (= x 5) (= x 5)) 1 0) (if (and (>= x 5) (>= x 10)) 1 0))`.")
  (input
    (do
      (def
        (main (: x Int64))
        #tuple((if (and (= x 5) (= x 5)) 1 0) (if (and (>= x 5) (>= x 10)) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: (tuple 1 0) (Tuple Int64 Int64)))
  (call main (: 6 Int64))
  (output (: (tuple 0 0) (Tuple Int64 Int64)))
  (call main (: 12 Int64))
  (output (: (tuple 0 1) (Tuple Int64 Int64)))
  (live-objects known-leak))

; ── BRANCHLESS boolean connectives over trap-free operands (value parity of the no-short-circuit emit) ─
; `(and p q)` / `(or p q)` over cheap trap-free operands (leaves or comparisons) need no short-circuit
; branch: booleans are canonical i32 0/1, so they emit a branchless `i32.and`/`i32.or`. Short-circuit only
; matters to skip an effecting/trapping right operand (that case keeps the `if` — see the trapping-rhs
; short-circuit cases above). These pin the VALUE parity of the branchless emit over the full truth table.
(case
  "a branchless and over two leaf booleans is the full conjunction truth table"
  (doc
    "`(and p q)` over two Bool leaves emits a branchless `i32.and` (no short-circuit `if`); the value
           is `p && q` over all four rows: (T,T) → true, (T,F) → false, (F,T) → false, (F,F) → false.")
  (input (do (def (f (: p Bool) (: q Bool)) (and p q)) (export f)))
  (call f (: true Bool) (: true Bool))
  (output (: true Bool))
  (call f (: true Bool) (: false Bool))
  (output (: false Bool))
  (call f (: false Bool) (: true Bool))
  (output (: false Bool))
  (call f (: false Bool) (: false Bool))
  (output (: false Bool)))

(case
  "a branchless or over two leaf booleans is the full disjunction truth table"
  (doc
    "`(or p q)` over two Bool leaves emits a branchless `i32.or`; the value is `p || q`: (T,T) → true,
           (T,F) → true, (F,T) → true, (F,F) → false. The disjunction companion of the branchless and.")
  (input (do (def (f (: p Bool) (: q Bool)) (or p q)) (export f)))
  (call f (: true Bool) (: true Bool))
  (output (: true Bool))
  (call f (: true Bool) (: false Bool))
  (output (: true Bool))
  (call f (: false Bool) (: true Bool))
  (output (: true Bool))
  (call f (: false Bool) (: false Bool))
  (output (: false Bool)))

(case
  "a branchless and over two comparisons is the conjunction of the two relations"
  (doc
    "`(and (< a b) (< c d))` — the operands are COMPARISONS, not bare leaves, but a comparison can
           neither trap nor effect, so it too emits a branchless `i32.and` (no short-circuit). The value is
           `(a<b) && (c<d)`: (1,2,3,4) → true, (1,2,4,3) → false, (2,1,3,4) → false, (2,1,4,3) → false.")
  (input
    (do (def (f (: a Int64) (: b Int64) (: c Int64) (: d Int64)) (and (< a b) (< c d))) (export f)))
  (call f (: 1 Int64) (: 2 Int64) (: 3 Int64) (: 4 Int64))
  (output (: true Bool))
  (call f (: 1 Int64) (: 2 Int64) (: 4 Int64) (: 3 Int64))
  (output (: false Bool))
  (call f (: 2 Int64) (: 1 Int64) (: 3 Int64) (: 4 Int64))
  (output (: false Bool))
  (call f (: 2 Int64) (: 1 Int64) (: 4 Int64) (: 3 Int64))
  (output (: false Bool)))

; --- Zero-equality instruction selection (eqz) keys on VALUE and width -----------------------------
; e316ef2cd selects `(= x 0)` to a single `eqz` at the Compare emit site. The selection must key on
; a VALUE zero in either operand order, test the NORMALIZED narrow value for a masked width (the
; UInt8.wrap+match zero-probe case in 09-functions pins the match path; this pins `=` directly), and
; compose with the negation fold. All three graded from the running program's side.
(case
  "equality against zero answers by value in both operand orders"
  (doc
    "`(= n 0)` and the commuted `(= 0 n)` in one body (10 + 1 = 11 at n = 0; 0 at n = 5): the
           zero-equality selection recognizes a constant zero on EITHER side and answers by the
           runtime operand's value. An emit keyed on 'right operand is literal zero' only misses the
           commuted form; one comparing the operand SLOT (not the value) misfires on nonzero.")
  (input (do (def (main (: n Int64)) (+ (if (= n 0) 10 0) (if (= 0 n) 1 0))) (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a wrapped byte's zero equality tests the masked value, not the wide slot"
  (doc
    "`(= (UInt8.wrap n) (UInt8.wrap 0))` at n = 256: the low byte is 0x00, so the equality is
           TRUE — though the wide i64 slot carried 256 (nonzero). n = 255 → low byte 0xFF → false.
           The `=`-form companion of the match zero-probe case (09-functions): an eqz applied to the
           un-masked wide slot answers false at n = 256.")
  (input (do (def (main (: n Int64)) (if (= (UInt8.wrap n) (UInt8.wrap 0)) 1 0)) (export main)))
  (call main (: 256 Int64))
  (output (: 1 Int64))
  (call main (: 255 Int64))
  (output (: 0 Int64)))

(case
  "a negated zero equality composes with the negation fold"
  (doc
    "`(not (= n 0))` — the zero-equality selection (eqz) composed with boolean negation (itself
           an eqz on i32): n = 0 → the equality is true, negated → 0; n = 7 → 1. Pins the two
           single-instruction selections stack without cancelling each other (a peephole that folded
           eqz;eqz as double negation ACROSS the width change would flip these).")
  (input (do (def (main (: n Int64)) (if (not (= n 0)) 1 0)) (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 7 Int64))
  (output (: 1 Int64)))

; --- Remaining faces of the arith/compare hoist order guard (position, complement, selection) ------
; The first-position shared-trapping-operand faces are pinned (arith + compare heads); these pin the
; neighbors: second position, the trap-free-cond complement, selection-only trapping for DIFFERING
; operands, the effect-timing face, and the comparison extension's selection/decline obligations.
(case
  "a shared trapping second operand does not preempt a trapping condition"
  (doc
    "`(if (< (+ x 1) 5) (+ a (/ 10 d)) (+ b (/ 10 d)))` at x = Int64.max, d = 0 — the shared
           trapping divide sits AFTER the differing operand this time. Source order: the cond's
           overflow fires first → 'integer overflow'. The position complement of the first-position
           order-guard case: one that hoisted any shared operand above cond regardless of position
           would surface 'divide by zero'.")
  (input
    (do
      (def
        (main (: x Int64) (: d Int64) (: a Int64) (: b Int64))
        (if (< (+ x 1) 5) (+ a (/ 10 d)) (+ b (/ 10 d))))
      (export main)))
  (call main (: 9223372036854775807 Int64) (: 0 Int64) (: 1 Int64) (: 2 Int64))
  (trap "integer overflow"))

(case
  "a shared trapping operand traps under a trap-free condition"
  (doc
    "The complement that keeps the order guard honest: same shared `(/ 10 d)`, but the condition
           `(< x 5)` is trap-free — the taken arm evaluates and the divide-by-zero IS the program's
           outcome. A guard that declined the hoist AND suppressed the arm's own evaluation would miss
           the trap: the guard protects ORDER, never reachability.")
  (input
    (do
      (def
        (main (: x Int64) (: d Int64) (: a Int64) (: b Int64))
        (if (< x 5) (+ (/ 10 d) a) (+ (/ 10 d) b)))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64) (: 1 Int64) (: 2 Int64))
  (trap "divide by zero"))

(case
  "differing trapping operands trap by selection only under the arith hoist"
  (doc
    "`(if (> c 0) (+ (/ 10 a) 1) (+ (/ 10 b) 1))` — the DIFFERING operands are the trapping
           ones (the shared `1` is inert): the per-operand `(if c (/ 10 a) (/ 10 b))` must evaluate
           only the SELECTED divide. c false, a = 0, b = 2 → 5 + 1 = 6 (the zero divisor is untaken);
           c true → 'divide by zero'.")
  (input
    (do
      (def (main (: c Int64) (: a Int64) (: b Int64)) (if (> c 0) (+ (/ 10 a) 1) (+ (/ 10 b) 1)))
      (export main)))
  (call main (: 0 Int64) (: 0 Int64) (: 2 Int64))
  (output (: 6 Int64))
  (call main (: 1 Int64) (: 0 Int64) (: 2 Int64))
  (trap "divide by zero"))

(case
  "an effectful shared operand performs after the condition, exactly once"
  (doc
    "The EFFECT face of the order guard, counter-observable end to end: `(if (< (Ctr.tick) 1)
           (+ (Ctr.tick) v) (+ (Ctr.tick) w))` — the arm-shared `(Ctr.tick)` is core_equiv across
           arms, so the hoist would love to share it; but it must perform AFTER the cond's own tick
           and exactly once. Order: cond tick returns 0 (→ then-arm), arm tick returns 1, + v = 100 →
           101, trailing tick returns 2 → 103. A hoist that evaluated the shared tick BEFORE the cond
           flips the branch (the cond's tick then returns 1, not 0) and skews every subsequent read —
           any wrong order or count misses 103.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: v Int64) (: w Int64))
        (handle
          Ctr
          0
          ((tick (_) s (resume s (+ s 1))))
          (+ (if (< (Ctr.tick unit) 1) (+ (Ctr.tick unit) v) (+ (Ctr.tick unit) w)) (Ctr.tick unit))))
      (export main)))
  (call main (: 100 Int64) (: 200 Int64))
  (output (: 103 Int64)))

(case
  "the selected operand decides a hoisted comparison"
  (doc
    "`(if (> c 0) (< a 10) (< b 10))` → the hoisted `(< (if c a b) 10)`: c = 1 selects a = 5 →
           true → 1; c = 0 selects b = 50 → false → 0. Both branch directions pin that the single
           compare receives the SELECTED operand (a positional mispairing answers the other arm's
           boolean).")
  (input
    (do
      (def (main (: c Int64) (: a Int64) (: b Int64)) (if (if (> c 0) (< a 10) (< b 10)) 1 0))
      (export main)))
  (call main (: 1 Int64) (: 5 Int64) (: 50 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64) (: 5 Int64) (: 50 Int64))
  (output (: 0 Int64)))

(case
  "mixed comparison operators across if arms keep their own arms"
  (doc
    "`(if (> c 0) (< a 10) (> a 10))` — SAME operand, DIFFERENT comparison operators: the hoist
           must decline (operator identity, not operand agreement, is the trigger). a = 5: c = 1 →
           `(< 5 10)` true → 1; c = 0 → `(> 5 10)` false → 0. A hoist keyed on the shared operand
           would emit ONE comparison and answer one direction wrong.")
  (input
    (do (def (main (: c Int64) (: a Int64)) (if (if (> c 0) (< a 10) (> a 10)) 1 0)) (export main)))
  (call main (: 1 Int64) (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64) (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a comparison's trapping shared bound fires under a trap-free condition"
  (doc
    "The compare-head reachability complement (the trapping-cond order face is pinned by the
           compare-head order-guard case): the cond `(< x 5)` is trap-free, the taken arm evaluates,
           and the shared bound's divide-by-zero IS the outcome. Order protection must not become
           trap suppression on the comparison shape either.")
  (input
    (do
      (def
        (main (: x Int64) (: d Int64) (: a Int64) (: b Int64))
        (if (if (< x 5) (< a (/ 10 d)) (< b (/ 10 d))) 1 0))
      (export main)))
  (call main (: 1 Int64) (: 0 Int64) (: 1 Int64) (: 2 Int64))
  (trap "divide by zero"))

; --- Constant propagation vs shadowing: the fold must respect the re-binding -----------------------
; The ML port's constprop just fixed exactly this (a shadowing non-constant let folded the body's x
; to the OUTER constant — 3c7f75dd9's soundness bug). These grade the seed's own fold at the same
; seams, promoted from passing breaker probes: a constant binding followed by a same-named RUNTIME
; re-binding must not leak the constant into the shadowed scope, at every binder kind.
(case
  "an inner runtime shadow defeats the outer constant fold"
  (doc
    "`(let ((x 5)) (let ((x w)) (+ x 1)))` at w = -1 → 0 — the inner `x` re-binds to the
           RUNTIME w, so the body's x is w, not the foldable outer 5 (a fold propagating the outer
           constant under the same environment answers 6). The exact shape of the ML constprop
           soundness bug, graded against the seed.")
  (input (do (def (main (: w Int64)) (let ((x 5)) (let ((x w)) (+ x 1)))) (export main)))
  (call main (: -1 Int64))
  (output (: 0 Int64)))

(case
  "a lambda parameter shadows the outer constant for its body"
  (doc
    "`(let ((x 5)) ((fn (x) (+ x 1)) w))` at w = -1 → 0 — the lambda's parameter is a NEW
           binder; its body's x is the argument, never the enclosing constant (a fold substituting
           into the lambda body before β answers 6). The function-binder face of the shadow-vs-fold
           discipline.")
  (input (do (def (main (: w Int64)) (let ((x 5)) ((fn (x) (+ x 1)) w))) (export main)))
  (call main (: -1 Int64))
  (output (: 0 Int64)))

(case
  "a captured pre-shadow value survives the re-binding"
  (doc
    "`(let ((x 1)) (let ((y x)) (let ((x w)) (+ x y))))` at w = 7 → 8 — `y` captures the OLD x
           (1) before the re-binding; the final body reads the NEW x (w) plus the captured y. Pins
           the fold's environment as a proper scope chain: the constant is usable exactly until the
           shadow, and values bound FROM it are independent of the re-binding.")
  (input
    (do (def (main (: w Int64)) (let ((x 1)) (let ((y x)) (let ((x w)) (+ x y))))) (export main)))
  (call main (: 7 Int64))
  (output (: 8 Int64)))

; --- Leading-rest list patterns are REFUTABLE by length (the memory-flagged unsoundness, resolved) --
; A `(list a b .. r)` arm requires AT LEAST 2 elements — it is REFUTABLE, not irrefutable (a
; too-short list must fall through, and a let-binder that cannot match must trap, never run the body
; on unbound head elements). These pin that length-refutation, promoted from passing breaker probes.
(case
  "a too-short list refutes a leading-rest arm and falls through"
  (doc
    "`(match (list 5) ((list a b .. r) 1) (_ 2))` → 2 — the 1-element list has FEWER than the
           two head elements the pattern binds, so the leading-rest arm REFUTES and control reaches
           the wildcard. Pins that a leading-rest pattern is length-refutable (an implementation
           treating it as irrefutable — binding `a`=5 and `b`/`r` from thin air — would return 1 on
           garbage; the historically-flagged unsoundness).")
  (input (do (def (main (: d Int64)) (match #list(5) (#list(a b (.. r)) 1) (_ 2))) (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

(case
  "a leading-rest arm matches an exactly-minimum list with an empty rest"
  (doc
    "`(match (list 7 8) ((list a b .. r) (+ (* (+ a b) 10) (List.len r))) (_ -1))` → 150: the
           2-element list is exactly the pattern's minimum, so it MATCHES with `a`=7, `b`=8, and
           `r`=[] (len 0) → (7+8)·10 + 0. The boundary between refute and match — one element fewer
           refutes (above), exactly-minimum matches with an empty rest.")
  (input
    (do
      (def
        (main (: d Int64))
        (match #list(7 8) (#list(a b (.. r)) (+ (* (+ a b) 10) (List.len r))) (_ -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 150 Int64)))

(case
  "a scalar-literal pattern over a List scrutinee is a coded CDZ0203 — the literal type cannot match the list"
  (doc
    "`(match #list(1 2) (8 99))` — the arm pattern `8` is an Int64 scalar literal, but the scrutinee is a
           `List Int64`. A scalar-literal pattern can NEVER match a list value, so the pattern and scrutinee
           types cannot unify — a PERMANENT type-mismatch reject (CDZ0203), reworded off the former codeless
           \"not supported\" (seq-280): this is ill-typed, not a not-yet lowering gap. Contrast the sibling
           arms that DO type-check: a bare binder `(x 99)` binds the whole list, and an element pattern
           `(#list(a b) …)` destructures it — only a non-list scalar/bool/string literal pattern is the type
           error. (Surfaced by the v-cdz-smith reachability sweep; coded by #7351.)")
  (input (do (def (main) (match #list(1 2) (8 99))) (export main)))
  (error CDZ0203 (message "scalar-literal pattern cannot match a `List` scrutinee")))

(case
  "a runtime-length list dispatches a leading-rest arm by its actual length"
  (doc
    "The length test is a RUNTIME check: a list built to length n by a recursive appender
           takes the `(list a b .. r)` arm only when n ≥ 2. n=4 → a+b = the two heads (7 over
           [4,3,2,1]... a=4? built by push so [4,3,2,1], heads 4+3=7); n=1 → refutes → -1. Pins that
           the refutation is decided at run time against the actual length, not a compile-time
           assumption.")
  (input
    (do
      (def
        (build (: n Int64) (: acc (List Int64)))
        (if (= n 0) acc (build (- n 1) (List.push acc n))))
      (def (main (: n Int64)) (match (build n #list()) (#list(a b (.. r)) (+ a b)) (_ -1)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 7 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64)))

(case
  "a dense if-equality chain dispatches every value including the default"
  (doc
    "A nested `(if (= n k) …)` chain over one integer parameter is an integer dispatch a user
           may write as chained `if`s instead of a `match`. Semantically it tests each constant in
           order and takes the first that matches, else the final else. Pins the OBSERVED VALUE for a
           matched arm (n=0/1/2/3 → 100/101/102/103) and the default (n outside → 999) — the
           equivalence a backend that lifts the chain to a jump table (`br_table`) must preserve. The
           lowering is a pure optimization: the value is exactly the if-chain's, only the dispatch is
           O(1) instead of an O(n) equality cascade.")
  (input
    (do
      (def
        (classify (: n Int64))
        (if (= n 0) 100 (if (= n 1) 101 (if (= n 2) 102 (if (= n 3) 103 999)))))
      (export classify)))
  (call classify (: 0 Int64))
  (output (: 100 Int64))
  (call classify (: 2 Int64))
  (output (: 102 Int64))
  (call classify (: 3 Int64))
  (output (: 103 Int64))
  (call classify (: 7 Int64))
  (output (: 999 Int64))
  (call classify (: -1 Int64))
  (output (: 999 Int64)))

(case
  "a dense if-equality chain dispatches on a let-bound value"
  (doc
    "The if-chain integer dispatch works on a `let`-bound scrutinee (a `LocalRef`), not only a
           bare parameter: `(let ((y (+ x 1))) (if (= y 0) … (if (= y 1) … (if (= y 2) … default))))`
           tests the SAME let-binding `y` in every arm. Pins the observed values (y=x+1, so x=-1→y=0→100,
           x=0→y=1→101, x=1→y=2→102, else 999) — the same first-wins semantics a backend lifting the
           chain to a jump table over the reusable local must preserve.")
  (input
    (do
      (def
        (classify (: x Int64))
        (let ((y (+ x 1))) (if (= y 0) 100 (if (= y 1) 101 (if (= y 2) 102 999)))))
      (export classify)))
  (call classify (: -1 Int64))
  (output (: 100 Int64))
  (call classify (: 0 Int64))
  (output (: 101 Int64))
  (call classify (: 1 Int64))
  (output (: 102 Int64))
  (call classify (: 9 Int64))
  (output (: 999 Int64)))

; --- A guard whose condition is a user-fn call on the heap payload. ---
(case
  "a match guard CALLS a user helper on the variant's heap payload to decide arm selection"
  (doc
    "The guard pins above use INLINE predicates; this guard's condition is a USER-FN CALL on the variant's heap payload ((is-vowel c) — the classifier idiom). On a helper-returned false the fall-through must leave the payload intact for the next arm's re-bind — the :495 borrow discipline through a CALL FRAME. Faces: vowel-hit arm 1, helper-false arm 2, None arm 0.")
  (input
    (do
      (def
        (is-vowel (: s String))
        (if (= s "a") true (if (= s "e") true (if (= s "i") true (if (= s "o") true (= s "u"))))))
      (def
        (classify (: s String))
        (match
          (String.at s 0)
          ((guard (Option.Some c) (is-vowel c)) 1)
          ((Option.Some _c) 2)
          ((Option.None _u) 0)))
      (def
        (main (: k Int64))
        (+
          (* 100 (classify (String.concat "ap" "e")))
          (+ (* 10 (classify (String.concat "xy" (if (= k 1) "z" "w")))) (classify ""))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 120 Int64))
  (live-objects known-leak))

; --- The do-def shadow WORKING perimeter (banked as box-the-fix pins around the v-inference
; do-def-shadow-over-param/let unbind fix): the shapes that were CORRECT before and after —
; shadows over match binders, prior defs, arm-scrutinee composition, and chained/nested
; re-shadowing each reading the PRIOR binding. ---
(case
  "a do-def shadows a MATCH BINDER and a prior def correctly"
  (doc
    "The WORKING half of the def-shadow matrix (param and let faces are the filed finding):
           a do-def over a MATCH BINDER rebinds properly — `(Some v)` arm binds v=k, `(def v (* v 2))`
           reads the binder and rebinds, trailing v = 2k (10 at k=5) — and the def-over-DEF spelling
           (:1267) reads the prior binding (15). Pins the two binder kinds the resolver already
           handles so the param/let fix extends the SAME treatment (a fix that re-scoped ALL shadows
           uniformly wrong would trip these).")
  (input
    (do
      (def (f (: k Int64)) (match (Some k) ((Some v) (do (def v (* v 2)) v)) ((None _u) -1)))
      (def (main (: k Int64)) (+ (* 100 (f k)) (do (def x 5) (def x (+ x 10)) x)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1015 Int64))
  (call main (: 0 Int64))
  (output (: 15 Int64)))

(case
  "an arm-body shadow of the param composes with the scrutinee's pre-shadow read"
  (doc
    "Shadow × match: the scrutinee `(+ v 1)` reads the PARAM, the irrefutable arm binds w,
           and the ARM BODY shadows v to 10w — both the shadow and the binder feed the result
           (11(k+1): 66 at k=5, 11 at k=0). Composes the fixed param-shadow with arm scope: a
           resolver that lets the arm's shadow leak backward into the scrutinee (re-evaluating
           it as 10w+1) or forward-scoped w wrongly breaks the multiple. Completes the shadow-fix
           perimeter across binder positions: do-chain, nested-do, capture, and match-arm.")
  (input
    (do
      (def (f (: v Int64)) (match (+ v 1) (w (do (def v (* w 10)) (+ v w)))))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 66 Int64))
  (call main (: 0 Int64))
  (output (: 11 Int64)))

(case
  "chained and nested do-def shadows over a param each read the prior binding"
  (doc
    "The composition perimeter of the def-shadow fix: THREE successive shadows of one param —
           two sequential in the outer do (each RHS reading the PRIOR binding: v→2v→2v+1) and a
           third inside a NESTED do (·10) — compute the full chain (110 at k=5, 10 at k=0). A scope
           fix that rebound only the FIRST shadow (or reset the chain at the nested-do boundary)
           truncates the pipeline; backward-only sequential visibility must hold at every link.")
  (input
    (do
      (def (f (: v Int64)) (do (def v (* v 2)) (def v (+ v 1)) (def w (do (def v (* v 10)) v)) w))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64))
  (call main (: 0 Int64))
  (output (: 10 Int64)))

; --- Validation-walk completeness probes (the cadenza-ml differential classifies each): an
; unbound name in an uncalled LAMBDA body. ---
(case
  "an unbound name inside an UNCALLED lambda body is still rejected"
  (doc
    "The ML-differential probe adjacent to KNOWN_ML_DIFFS #1 (unbound-in-uncalled-def): here the def's VALUE is a lambda that is never applied — a reachability walk that skips uncalled sibling defs may or may not descend into a bound-but-unapplied fn value. rcdzc rejects; the differential classifies ML.")
  (input
    (do (def unused-lambda (fn ((: x Int64)) (+ x undefined-name))) (def (main) 42) (export main)))
  (error CDZ0101))

; --- Non-linear binding, param-list face (annotated variant; adv-47) + uncalled-def match
; faces (arm body / arm guard — positions a scope walk may skip). ---
(case
  "a function with two SAME-NAMED parameters is a non-linear binding rejection"
  (doc
    "The param-list face of the CDZ0102 non-linearity rule (let and match faces pinned elsewhere; 05-compound pins the UNTYPED (def (f x x)) shape) — here both params carry Int64 annotations, so the reject must fire on the repeated NAME, not the annotation path. The self-hosted front-end accepted this and silently last-wins-shadowed ((f 1 2) returned 2) — adv-47, fixed b80c1d374.")
  (input (do (def (f (: x Int64) (: x Int64)) x) (def (main) (f 1 2)) (export main)))
  (error CDZ0102))

(case
  "an unbound name in an uncalled def's match-ARM BODY is rejected"
  (doc
    "The match-arm-body face of the uncalled-def scope walk: the unbound name sits inside an arm of a match in a never-called def — a walk that descends def bodies but not into match arms runs to 42. rcdzc rejects CDZ0101.")
  (input
    (do (def (unused (: x Int64)) (match x ((1) no-such-name) (_ 0))) (def (main) 42) (export main)))
  (error CDZ0101))

(case
  "an unbound name in an uncalled def's match-arm GUARD is rejected"
  (doc
    "The GUARD face: the unbound name is in an arm's guard expression, a position evaluated only during match dispatch — a scope walk that skips guard expressions in uncalled defs runs to 42. rcdzc rejects CDZ0101.")
  (input
    (do
      (def (unused (: x Int64)) (match x ((y (> y no-such-guard)) 1) (_ 0)))
      (def (main) 42)
      (export main)))
  (error CDZ0101))

(case
  "pure guards on fused-match arms read the payload binder and an enclosing capture"
  (doc
    "Guards × the match-fusion seam: the scrutinee is a CALL result (`mk`, whose arms clone
           into the callee's branches) and the guards read BOTH the arm's SumPayload binder AND an
           enclosing let-bound capture (`lim`) — with guarded and unguarded arms interleaved on the
           SAME variant (Hi guarded then Hi unguarded = the fall-through). k=9 → 9>8 → 90; k=7 →
           guard fails → unguarded Hi arm → 7; k=2 → Lo guard 2<8 → 200. A fused clone that
           mis-classified either binder read (copy the capture / share the payload) breaks a guard's
           value; the guard-position companion of the fused direct-read pins.")
  (input
    (do
      (type Sz (Hi Int64) (Lo Int64))
      (def (mk x) (if (> x 5) (Hi x) (Lo x)))
      (def
        (main (: k Int64))
        (let
          ((lim 8))
          (match
            (mk k)
            ((guard (Hi h) (> h lim)) (* h 10))
            ((Hi h2) h2)
            ((guard (Lo w) (< w lim)) (* w 100))
            (_ -999))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 90 Int64))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: 2 Int64))
  (output (: 200 Int64)))

(case
  "an unused let binding compiles and runs but the build surfaces a CDZ0306 unused-binding warning"
  (doc
    "Witnesses dead-code surfacing (the code-quality/dead-code warning band): a let binder that is
           never referenced does not change the program's value (`main` still returns 42), but the compiler
           emits a CDZ0306 `unused binding` WARNING rather than silently keeping the dead binding. This is
           the FIRST use of the portable (warns ..) clause — a case that compiles CLEAN and runs to a value
           can additionally assert an expected compiler warning (code + a message substring), orthogonal to
           its (output ..). The binder NAME is in the warning message's dynamic tail (`unused binding `x``),
           so only the stable lead `unused binding` is pinned. Graded on the wasm target (warnings are
           emitted in the shared compile stage = target-independent, so wasm is a sufficient witness; the
           rust/rust-async run paths cannot observe compile stderr, so the (warns ..) check is skipped there,
           not failed).")
  (input (do (def (main) (let ((unused 99)) 42)) (export main)))
  (output (: 42 Int64))
  (warns CDZ0306 (message "unused binding")))

(case
  "a reference buried under many non-binding forms resolves to its outer let binder"
  (doc
    "The lexical-scope skip index hops over the non-binding spine (record / if / application) to the
           binding let. `k` (=5) sits under three non-binding ifs and a record projection above the let that
           binds it; each `(< k N)` is true so the innermost `k` runs, projected back = 5.")
  (input
    (do
      (def
        (main)
        (let ((k 5)) (if (< k 30) (if (< k 20) (if (< k 10) (. #record((= a k)) a) 1) 2) 3)))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "shadowing resolves nearest-wins through the scope-skip index"
  (doc
    "An inner let rebinding a name an outer let also binds must reach the INNER one (nearest-wins).
           `(let ((x 1)) (let ((x 2)) x))` = 2, not 1 (a skip jumping past the inner let would return 1).")
  (input (do (def (main) (let ((x 1)) (let ((x 2)) x))) (export main)))
  (call main)
  (output (: 2 Int64)))

; ── Branchless select (if with two cheap leaf branches) computes the right value (migrated from rcdzc) ──
(case
  "an integer min via (if (< a b) a b) returns the smaller in either operand order"
  (doc
    "The branchless-select base case: `(if (< a b) a b)` with two cheap leaf branches lowers to wasm's
           branchless `select` (pops [then, else, cond], pushes then iff cond nonzero). It must compute the
           SAME value the structured block would — the smaller of a,b — in either order, so the emitted
           operand order (then, else, cond) matches the if's truth sense (a swapped order would silently
           return the wrong branch, which no instruction-count check catches). min(3,7)=3, min(7,3)=3,
           min(-5,-5)=-5.")
  (input (do (def (f (: a Int64) (: b Int64)) (if (< a b) a b)) (export f)))
  (call f (: 3 Int64) (: 7 Int64))
  (output (: 3 Int64))
  (call f (: 7 Int64) (: 3 Int64))
  (output (: 3 Int64))
  (call f (: -5 Int64) (: -5 Int64))
  (output (: -5 Int64)))

(case
  "a value-pick (if p a b) selects the arg matching the runtime condition's truth value"
  (doc
    "The value-picking companion of the min select: `(if p a b)` over a runtime Bool and two Int64
           args picks a when p is true, b when false — both cheap leaves, so it select-ifies, and the pick
           must honor the condition's truth sense (a swapped select operand order would return the wrong
           arg). p=true -> a=11; p=false -> b=22.")
  (input (do (def (f (: p Bool) (: a Int64) (: b Int64)) (if p a b)) (export f)))
  (call f (: true Bool) (: 11 Int64) (: 22 Int64))
  (output (: 11 Int64))
  (call f (: false Bool) (: 11 Int64) (: 22 Int64))
  (output (: 22 Int64)))

; ── Match-arm fusion is reclaim-neutral + UAF-free (migrated from rcdzc): a fused arm reading a heap binder twice matches the non-fused shape's reclaim; the anchor pins the residual is the Some-shell gap ──
(case
  "a fused match arm reading a heap binder twice is UAF-free and reclaim-neutral vs the non-fused shape"
  (doc
    "Inlining `inner` into the Some-arm triggers the fusion arm-clone that classifies `w` (the heap
           List Ok-payload, read within the clone, twice). Reading `w` twice must be UAF-free — a wrong SHARE
           of the binder in the cloned arm would double-drop it, so the second read sees a freed handle
           (garbage/trap). len([0,1,2]) twice = 6. The fused clone must be RECLAIM-NEUTRAL vs the non-fused
           Some-shell shape (same known-leak 3, the pre-existing compound-Some-shell residual): an over-copy
           would push it above 3, a wrong share would double-free (UAF/wrong value). Paired with the
           non-fused control below (same 3) this pins the neutrality.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc (List Int64)))
        (if (< i n) (build (+ i 1) n (List.push acc i)) acc))
      (def
        (inner (: r (Result (List Int64) Int64)))
        (match r ((Ok w) (+ (List.len w) (List.len w))) ((Err e) e)))
      (def
        (f (: c Bool) (: n Int64))
        (match
          (if c (Some (build 0 n #list())) (None))
          ((Some v) (inner (if c (Ok v) (Err 0))))
          ((None) 0)))
      (def (main (: n Int64)) (f true n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 6 Int64))
  (live-objects 0))

(case
  "the non-fused Some-shell control reads a heap binder twice with the same reclaim (no fusion)"
  (doc
    "The non-fused control for the fusion neutrality pin above: the SAME Some-wrapped heap list with
           `v` used twice DIRECTLY in the arm (no inner match to fuse), isolating the fused arm-clone as the
           only difference. Same value 6, same known-leak 3 (the compound-Some-shell residual) — so the fused
           case's equal 3 proves fusing changed neither value nor reclaim.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc (List Int64)))
        (if (< i n) (build (+ i 1) n (List.push acc i)) acc))
      (def
        (f (: c Bool) (: n Int64))
        (match
          (if c (Some (build 0 n #list())) (None))
          ((Some v) (+ (List.len v) (List.len v)))
          ((None) 0)))
      (def (main (: n Int64)) (f true n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 6 Int64))
  (live-objects 0))

(case
  "a twice-used heap binder without a Some shell reclaims fully (the fusion-neutrality anchor)"
  (doc
    "The anchor proving the fused/non-fused residual (3) is the pre-existing compound-Some-shell
           known-gap, NOT the twice-used heap binder's own dup/drop: the SAME heap list let-bound WITHOUT the
           Some shell, used twice, then dropped, reclaims FULLY — live-objects 0. Value 6. If this leaked, the
           residual would be mis-attributed to the binder rather than the shell.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc (List Int64)))
        (if (< i n) (build (+ i 1) n (List.push acc i)) acc))
      (def (f (: n Int64)) (let ((v (build 0 n #list()))) (+ (List.len v) (List.len v))))
      (def (main (: n Int64)) (f n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 6 Int64))
  (live-objects 0))

(case
  "rbw1 a three-hundred-arm integer match dispatches to the right arm"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          n
          (0 0)
          (1 2)
          (2 4)
          (3 6)
          (4 8)
          (5 10)
          (6 12)
          (7 14)
          (8 16)
          (9 18)
          (10 20)
          (11 22)
          (12 24)
          (13 26)
          (14 28)
          (15 30)
          (16 32)
          (17 34)
          (18 36)
          (19 38)
          (20 40)
          (21 42)
          (22 44)
          (23 46)
          (24 48)
          (25 50)
          (26 52)
          (27 54)
          (28 56)
          (29 58)
          (30 60)
          (31 62)
          (32 64)
          (33 66)
          (34 68)
          (35 70)
          (36 72)
          (37 74)
          (38 76)
          (39 78)
          (40 80)
          (41 82)
          (42 84)
          (43 86)
          (44 88)
          (45 90)
          (46 92)
          (47 94)
          (48 96)
          (49 98)
          (50 100)
          (51 102)
          (52 104)
          (53 106)
          (54 108)
          (55 110)
          (56 112)
          (57 114)
          (58 116)
          (59 118)
          (60 120)
          (61 122)
          (62 124)
          (63 126)
          (64 128)
          (65 130)
          (66 132)
          (67 134)
          (68 136)
          (69 138)
          (70 140)
          (71 142)
          (72 144)
          (73 146)
          (74 148)
          (75 150)
          (76 152)
          (77 154)
          (78 156)
          (79 158)
          (80 160)
          (81 162)
          (82 164)
          (83 166)
          (84 168)
          (85 170)
          (86 172)
          (87 174)
          (88 176)
          (89 178)
          (90 180)
          (91 182)
          (92 184)
          (93 186)
          (94 188)
          (95 190)
          (96 192)
          (97 194)
          (98 196)
          (99 198)
          (100 200)
          (101 202)
          (102 204)
          (103 206)
          (104 208)
          (105 210)
          (106 212)
          (107 214)
          (108 216)
          (109 218)
          (110 220)
          (111 222)
          (112 224)
          (113 226)
          (114 228)
          (115 230)
          (116 232)
          (117 234)
          (118 236)
          (119 238)
          (120 240)
          (121 242)
          (122 244)
          (123 246)
          (124 248)
          (125 250)
          (126 252)
          (127 254)
          (128 256)
          (129 258)
          (130 260)
          (131 262)
          (132 264)
          (133 266)
          (134 268)
          (135 270)
          (136 272)
          (137 274)
          (138 276)
          (139 278)
          (140 280)
          (141 282)
          (142 284)
          (143 286)
          (144 288)
          (145 290)
          (146 292)
          (147 294)
          (148 296)
          (149 298)
          (150 300)
          (151 302)
          (152 304)
          (153 306)
          (154 308)
          (155 310)
          (156 312)
          (157 314)
          (158 316)
          (159 318)
          (160 320)
          (161 322)
          (162 324)
          (163 326)
          (164 328)
          (165 330)
          (166 332)
          (167 334)
          (168 336)
          (169 338)
          (170 340)
          (171 342)
          (172 344)
          (173 346)
          (174 348)
          (175 350)
          (176 352)
          (177 354)
          (178 356)
          (179 358)
          (180 360)
          (181 362)
          (182 364)
          (183 366)
          (184 368)
          (185 370)
          (186 372)
          (187 374)
          (188 376)
          (189 378)
          (190 380)
          (191 382)
          (192 384)
          (193 386)
          (194 388)
          (195 390)
          (196 392)
          (197 394)
          (198 396)
          (199 398)
          (200 400)
          (201 402)
          (202 404)
          (203 406)
          (204 408)
          (205 410)
          (206 412)
          (207 414)
          (208 416)
          (209 418)
          (210 420)
          (211 422)
          (212 424)
          (213 426)
          (214 428)
          (215 430)
          (216 432)
          (217 434)
          (218 436)
          (219 438)
          (220 440)
          (221 442)
          (222 444)
          (223 446)
          (224 448)
          (225 450)
          (226 452)
          (227 454)
          (228 456)
          (229 458)
          (230 460)
          (231 462)
          (232 464)
          (233 466)
          (234 468)
          (235 470)
          (236 472)
          (237 474)
          (238 476)
          (239 478)
          (240 480)
          (241 482)
          (242 484)
          (243 486)
          (244 488)
          (245 490)
          (246 492)
          (247 494)
          (248 496)
          (249 498)
          (250 500)
          (251 502)
          (252 504)
          (253 506)
          (254 508)
          (255 510)
          (256 512)
          (257 514)
          (258 516)
          (259 518)
          (260 520)
          (261 522)
          (262 524)
          (263 526)
          (264 528)
          (265 530)
          (266 532)
          (267 534)
          (268 536)
          (269 538)
          (270 540)
          (271 542)
          (272 544)
          (273 546)
          (274 548)
          (275 550)
          (276 552)
          (277 554)
          (278 556)
          (279 558)
          (280 560)
          (281 562)
          (282 564)
          (283 566)
          (284 568)
          (285 570)
          (286 572)
          (287 574)
          (288 576)
          (289 578)
          (290 580)
          (291 582)
          (292 584)
          (293 586)
          (294 588)
          (295 590)
          (296 592)
          (297 594)
          (298 596)
          (299 598)
          (_ -1)))
      (export main)))
  (call main (: 150 Int64))
  (output (: 300 Int64)))

(case
  "rbw2 a four-hundred-binding let chain threads sequentially"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((x0 n)
            (x1 (+ x0 1))
            (x2 (+ x1 1))
            (x3 (+ x2 1))
            (x4 (+ x3 1))
            (x5 (+ x4 1))
            (x6 (+ x5 1))
            (x7 (+ x6 1))
            (x8 (+ x7 1))
            (x9 (+ x8 1))
            (x10 (+ x9 1))
            (x11 (+ x10 1))
            (x12 (+ x11 1))
            (x13 (+ x12 1))
            (x14 (+ x13 1))
            (x15 (+ x14 1))
            (x16 (+ x15 1))
            (x17 (+ x16 1))
            (x18 (+ x17 1))
            (x19 (+ x18 1))
            (x20 (+ x19 1))
            (x21 (+ x20 1))
            (x22 (+ x21 1))
            (x23 (+ x22 1))
            (x24 (+ x23 1))
            (x25 (+ x24 1))
            (x26 (+ x25 1))
            (x27 (+ x26 1))
            (x28 (+ x27 1))
            (x29 (+ x28 1))
            (x30 (+ x29 1))
            (x31 (+ x30 1))
            (x32 (+ x31 1))
            (x33 (+ x32 1))
            (x34 (+ x33 1))
            (x35 (+ x34 1))
            (x36 (+ x35 1))
            (x37 (+ x36 1))
            (x38 (+ x37 1))
            (x39 (+ x38 1))
            (x40 (+ x39 1))
            (x41 (+ x40 1))
            (x42 (+ x41 1))
            (x43 (+ x42 1))
            (x44 (+ x43 1))
            (x45 (+ x44 1))
            (x46 (+ x45 1))
            (x47 (+ x46 1))
            (x48 (+ x47 1))
            (x49 (+ x48 1))
            (x50 (+ x49 1))
            (x51 (+ x50 1))
            (x52 (+ x51 1))
            (x53 (+ x52 1))
            (x54 (+ x53 1))
            (x55 (+ x54 1))
            (x56 (+ x55 1))
            (x57 (+ x56 1))
            (x58 (+ x57 1))
            (x59 (+ x58 1))
            (x60 (+ x59 1))
            (x61 (+ x60 1))
            (x62 (+ x61 1))
            (x63 (+ x62 1))
            (x64 (+ x63 1))
            (x65 (+ x64 1))
            (x66 (+ x65 1))
            (x67 (+ x66 1))
            (x68 (+ x67 1))
            (x69 (+ x68 1))
            (x70 (+ x69 1))
            (x71 (+ x70 1))
            (x72 (+ x71 1))
            (x73 (+ x72 1))
            (x74 (+ x73 1))
            (x75 (+ x74 1))
            (x76 (+ x75 1))
            (x77 (+ x76 1))
            (x78 (+ x77 1))
            (x79 (+ x78 1))
            (x80 (+ x79 1))
            (x81 (+ x80 1))
            (x82 (+ x81 1))
            (x83 (+ x82 1))
            (x84 (+ x83 1))
            (x85 (+ x84 1))
            (x86 (+ x85 1))
            (x87 (+ x86 1))
            (x88 (+ x87 1))
            (x89 (+ x88 1))
            (x90 (+ x89 1))
            (x91 (+ x90 1))
            (x92 (+ x91 1))
            (x93 (+ x92 1))
            (x94 (+ x93 1))
            (x95 (+ x94 1))
            (x96 (+ x95 1))
            (x97 (+ x96 1))
            (x98 (+ x97 1))
            (x99 (+ x98 1))
            (x100 (+ x99 1))
            (x101 (+ x100 1))
            (x102 (+ x101 1))
            (x103 (+ x102 1))
            (x104 (+ x103 1))
            (x105 (+ x104 1))
            (x106 (+ x105 1))
            (x107 (+ x106 1))
            (x108 (+ x107 1))
            (x109 (+ x108 1))
            (x110 (+ x109 1))
            (x111 (+ x110 1))
            (x112 (+ x111 1))
            (x113 (+ x112 1))
            (x114 (+ x113 1))
            (x115 (+ x114 1))
            (x116 (+ x115 1))
            (x117 (+ x116 1))
            (x118 (+ x117 1))
            (x119 (+ x118 1))
            (x120 (+ x119 1))
            (x121 (+ x120 1))
            (x122 (+ x121 1))
            (x123 (+ x122 1))
            (x124 (+ x123 1))
            (x125 (+ x124 1))
            (x126 (+ x125 1))
            (x127 (+ x126 1))
            (x128 (+ x127 1))
            (x129 (+ x128 1))
            (x130 (+ x129 1))
            (x131 (+ x130 1))
            (x132 (+ x131 1))
            (x133 (+ x132 1))
            (x134 (+ x133 1))
            (x135 (+ x134 1))
            (x136 (+ x135 1))
            (x137 (+ x136 1))
            (x138 (+ x137 1))
            (x139 (+ x138 1))
            (x140 (+ x139 1))
            (x141 (+ x140 1))
            (x142 (+ x141 1))
            (x143 (+ x142 1))
            (x144 (+ x143 1))
            (x145 (+ x144 1))
            (x146 (+ x145 1))
            (x147 (+ x146 1))
            (x148 (+ x147 1))
            (x149 (+ x148 1))
            (x150 (+ x149 1))
            (x151 (+ x150 1))
            (x152 (+ x151 1))
            (x153 (+ x152 1))
            (x154 (+ x153 1))
            (x155 (+ x154 1))
            (x156 (+ x155 1))
            (x157 (+ x156 1))
            (x158 (+ x157 1))
            (x159 (+ x158 1))
            (x160 (+ x159 1))
            (x161 (+ x160 1))
            (x162 (+ x161 1))
            (x163 (+ x162 1))
            (x164 (+ x163 1))
            (x165 (+ x164 1))
            (x166 (+ x165 1))
            (x167 (+ x166 1))
            (x168 (+ x167 1))
            (x169 (+ x168 1))
            (x170 (+ x169 1))
            (x171 (+ x170 1))
            (x172 (+ x171 1))
            (x173 (+ x172 1))
            (x174 (+ x173 1))
            (x175 (+ x174 1))
            (x176 (+ x175 1))
            (x177 (+ x176 1))
            (x178 (+ x177 1))
            (x179 (+ x178 1))
            (x180 (+ x179 1))
            (x181 (+ x180 1))
            (x182 (+ x181 1))
            (x183 (+ x182 1))
            (x184 (+ x183 1))
            (x185 (+ x184 1))
            (x186 (+ x185 1))
            (x187 (+ x186 1))
            (x188 (+ x187 1))
            (x189 (+ x188 1))
            (x190 (+ x189 1))
            (x191 (+ x190 1))
            (x192 (+ x191 1))
            (x193 (+ x192 1))
            (x194 (+ x193 1))
            (x195 (+ x194 1))
            (x196 (+ x195 1))
            (x197 (+ x196 1))
            (x198 (+ x197 1))
            (x199 (+ x198 1))
            (x200 (+ x199 1))
            (x201 (+ x200 1))
            (x202 (+ x201 1))
            (x203 (+ x202 1))
            (x204 (+ x203 1))
            (x205 (+ x204 1))
            (x206 (+ x205 1))
            (x207 (+ x206 1))
            (x208 (+ x207 1))
            (x209 (+ x208 1))
            (x210 (+ x209 1))
            (x211 (+ x210 1))
            (x212 (+ x211 1))
            (x213 (+ x212 1))
            (x214 (+ x213 1))
            (x215 (+ x214 1))
            (x216 (+ x215 1))
            (x217 (+ x216 1))
            (x218 (+ x217 1))
            (x219 (+ x218 1))
            (x220 (+ x219 1))
            (x221 (+ x220 1))
            (x222 (+ x221 1))
            (x223 (+ x222 1))
            (x224 (+ x223 1))
            (x225 (+ x224 1))
            (x226 (+ x225 1))
            (x227 (+ x226 1))
            (x228 (+ x227 1))
            (x229 (+ x228 1))
            (x230 (+ x229 1))
            (x231 (+ x230 1))
            (x232 (+ x231 1))
            (x233 (+ x232 1))
            (x234 (+ x233 1))
            (x235 (+ x234 1))
            (x236 (+ x235 1))
            (x237 (+ x236 1))
            (x238 (+ x237 1))
            (x239 (+ x238 1))
            (x240 (+ x239 1))
            (x241 (+ x240 1))
            (x242 (+ x241 1))
            (x243 (+ x242 1))
            (x244 (+ x243 1))
            (x245 (+ x244 1))
            (x246 (+ x245 1))
            (x247 (+ x246 1))
            (x248 (+ x247 1))
            (x249 (+ x248 1))
            (x250 (+ x249 1))
            (x251 (+ x250 1))
            (x252 (+ x251 1))
            (x253 (+ x252 1))
            (x254 (+ x253 1))
            (x255 (+ x254 1))
            (x256 (+ x255 1))
            (x257 (+ x256 1))
            (x258 (+ x257 1))
            (x259 (+ x258 1))
            (x260 (+ x259 1))
            (x261 (+ x260 1))
            (x262 (+ x261 1))
            (x263 (+ x262 1))
            (x264 (+ x263 1))
            (x265 (+ x264 1))
            (x266 (+ x265 1))
            (x267 (+ x266 1))
            (x268 (+ x267 1))
            (x269 (+ x268 1))
            (x270 (+ x269 1))
            (x271 (+ x270 1))
            (x272 (+ x271 1))
            (x273 (+ x272 1))
            (x274 (+ x273 1))
            (x275 (+ x274 1))
            (x276 (+ x275 1))
            (x277 (+ x276 1))
            (x278 (+ x277 1))
            (x279 (+ x278 1))
            (x280 (+ x279 1))
            (x281 (+ x280 1))
            (x282 (+ x281 1))
            (x283 (+ x282 1))
            (x284 (+ x283 1))
            (x285 (+ x284 1))
            (x286 (+ x285 1))
            (x287 (+ x286 1))
            (x288 (+ x287 1))
            (x289 (+ x288 1))
            (x290 (+ x289 1))
            (x291 (+ x290 1))
            (x292 (+ x291 1))
            (x293 (+ x292 1))
            (x294 (+ x293 1))
            (x295 (+ x294 1))
            (x296 (+ x295 1))
            (x297 (+ x296 1))
            (x298 (+ x297 1))
            (x299 (+ x298 1))
            (x300 (+ x299 1))
            (x301 (+ x300 1))
            (x302 (+ x301 1))
            (x303 (+ x302 1))
            (x304 (+ x303 1))
            (x305 (+ x304 1))
            (x306 (+ x305 1))
            (x307 (+ x306 1))
            (x308 (+ x307 1))
            (x309 (+ x308 1))
            (x310 (+ x309 1))
            (x311 (+ x310 1))
            (x312 (+ x311 1))
            (x313 (+ x312 1))
            (x314 (+ x313 1))
            (x315 (+ x314 1))
            (x316 (+ x315 1))
            (x317 (+ x316 1))
            (x318 (+ x317 1))
            (x319 (+ x318 1))
            (x320 (+ x319 1))
            (x321 (+ x320 1))
            (x322 (+ x321 1))
            (x323 (+ x322 1))
            (x324 (+ x323 1))
            (x325 (+ x324 1))
            (x326 (+ x325 1))
            (x327 (+ x326 1))
            (x328 (+ x327 1))
            (x329 (+ x328 1))
            (x330 (+ x329 1))
            (x331 (+ x330 1))
            (x332 (+ x331 1))
            (x333 (+ x332 1))
            (x334 (+ x333 1))
            (x335 (+ x334 1))
            (x336 (+ x335 1))
            (x337 (+ x336 1))
            (x338 (+ x337 1))
            (x339 (+ x338 1))
            (x340 (+ x339 1))
            (x341 (+ x340 1))
            (x342 (+ x341 1))
            (x343 (+ x342 1))
            (x344 (+ x343 1))
            (x345 (+ x344 1))
            (x346 (+ x345 1))
            (x347 (+ x346 1))
            (x348 (+ x347 1))
            (x349 (+ x348 1))
            (x350 (+ x349 1))
            (x351 (+ x350 1))
            (x352 (+ x351 1))
            (x353 (+ x352 1))
            (x354 (+ x353 1))
            (x355 (+ x354 1))
            (x356 (+ x355 1))
            (x357 (+ x356 1))
            (x358 (+ x357 1))
            (x359 (+ x358 1))
            (x360 (+ x359 1))
            (x361 (+ x360 1))
            (x362 (+ x361 1))
            (x363 (+ x362 1))
            (x364 (+ x363 1))
            (x365 (+ x364 1))
            (x366 (+ x365 1))
            (x367 (+ x366 1))
            (x368 (+ x367 1))
            (x369 (+ x368 1))
            (x370 (+ x369 1))
            (x371 (+ x370 1))
            (x372 (+ x371 1))
            (x373 (+ x372 1))
            (x374 (+ x373 1))
            (x375 (+ x374 1))
            (x376 (+ x375 1))
            (x377 (+ x376 1))
            (x378 (+ x377 1))
            (x379 (+ x378 1))
            (x380 (+ x379 1))
            (x381 (+ x380 1))
            (x382 (+ x381 1))
            (x383 (+ x382 1))
            (x384 (+ x383 1))
            (x385 (+ x384 1))
            (x386 (+ x385 1))
            (x387 (+ x386 1))
            (x388 (+ x387 1))
            (x389 (+ x388 1))
            (x390 (+ x389 1))
            (x391 (+ x390 1))
            (x392 (+ x391 1))
            (x393 (+ x392 1))
            (x394 (+ x393 1))
            (x395 (+ x394 1))
            (x396 (+ x395 1))
            (x397 (+ x396 1))
            (x398 (+ x397 1))
            (x399 (+ x398 1)))
          x399))
      (export main)))
  (call main (: 5 Int64))
  (output (: 404 Int64)))

(case
  "rbw3 a one-million-frame add-one recursion answers (the non-tail spine does not exhaust the stack)"
  (input
    (do
      (def (down (: k Int64)) (if (= k 0) 0 (+ 1 (down (- k 1)))))
      (def (main (: n Int64)) (down n))
      (export main)))
  (call main (: 1000000 Int64))
  (output (: 1000000 Int64)))

; -- FLOW-SENSITIVE guard / mask elision value+trap parity (behavioral halves migrated from rcdzc
; 2026-08-27; the white-box Lir guard-count / bit-op-count inspections stay wasmtime-free rcdzc unit
; tests). A branch/range condition refines a variable's interval in the taken branch, licensing the
; compiler to shed a provably-dead overflow guard or a redundant mask/or — the VALUE must be unchanged,
; a genuinely-live guard must still trap, and a fold that DISCARDS an operand must keep that operand's
; trap. These use flow-refined `(if (and/or …) …)` surfaces distinct from the pure-bitwise identity
; cases in 06-numeric-model.
(case
  "a branch condition refines a variable's range so a dead underflow guard is elided (value + trap parity)"
  (doc
    "Under `(if (> n 0) …)` the then knows n>=1, so `(- n 1)` cannot underflow — value unchanged and
           no FALSE trap (n=MIN takes the else). A `<`-guard refines the else the same way. A `(+ n 1)`
           under n>=1 CAN overflow at n=MAX and must STILL trap (the live guard is kept). rcdzc:
           a_branch_condition_refines_a_variables_range_and_elides_a_dead_guard.")
  (input
    (do
      (def (sub (: n Int64)) (if (> n 0) (- n 1) 0))
      (def (add (: n Int64)) (if (> n 0) (+ n 1) 0))
      (def (esub (: n Int64)) (if (< n 1) 0 (- n 1)))
      (export sub)
      (export add)
      (export esub)))
  (call sub (: 5 Int64))
  (output (: 4 Int64))
  (call sub (: 0 Int64))
  (output (: 0 Int64))
  (call sub (: -9223372036854775808 Int64))
  (output (: 0 Int64))
  (call add (: 5 Int64))
  (output (: 6 Int64))
  (call add (: 9223372036854775807 Int64))
  (trap "overflow")
  (call esub (: 3 Int64))
  (output (: 2 Int64)))

(case
  "a branch refinement elides a redundant AND-mask covering the refined range (value parity)"
  (doc
    "Under `(if (and (>= x 0) (< x 256)) …)` x is refined to [0,255], so `(& x 255)` == x (the mask
           covers x's whole range and is a no-op); out of range takes the else. A PARTIAL `(& x 15)` still
           masks. rcdzc: a_branch_refinement_elides_a_redundant_and_mask.")
  (input
    (do
      (def (full (: x Int64)) (if (and (>= x 0) (< x 256)) (& x 255) x))
      (def (part (: x Int64)) (if (and (>= x 0) (< x 256)) (& x 15) x))
      (export full)
      (export part)))
  (call full (: 200 Int64))
  (output (: 200 Int64))
  (call full (: 0 Int64))
  (output (: 0 Int64))
  (call full (: 1000 Int64))
  (output (: 1000 Int64))
  (call part (: 200 Int64))
  (output (: 8 Int64)))

(case
  "a saturating OR-mask over a refined range folds to the constant (value + trap parity)"
  (doc
    "Under x∈[0,255], `(| x 255)` == 255 (every bit x could set is already set, so the OR adds
           nothing); out of range takes the else. A PARTIAL `(| x 15)` still ORs. The fold DISCARDS x, so
           a trapping operand keeps its divide-by-zero trap. rcdzc: a_saturating_or_mask_folds_to_the_constant.")
  (input
    (do
      (def (sat (: x Int64)) (if (and (>= x 0) (< x 256)) (| x 255) x))
      (def (part (: x Int64)) (if (and (>= x 0) (< x 256)) (| x 15) x))
      (def (divz (: z Int64)) (| (: (& (: (/ 100 z) Int64) 7) Int64) 255))
      (export sat)
      (export part)
      (export divz)))
  (call sat (: 200 Int64))
  (output (: 255 Int64))
  (call sat (: 0 Int64))
  (output (: 255 Int64))
  (call sat (: 1000 Int64))
  (output (: 1000 Int64))
  (call part (: 200 Int64))
  (output (: 207 Int64))
  (call divz (: 0 Int64))
  (trap "divide by zero"))

(case
  "a conjunction/disjunction range condition refines both bounds so guards are elided (value + trap parity)"
  (doc
    "`(and (> n 0) (< n 100))` bounds n∈[1,99] in the then, so `(- n 1)` and `(+ n 1)` shed their
           guards; `(or (< n 1) (> n 99))`'s ELSE refines the same via De Morgan; an `and` over two vars
           refines both. The WRONG polarity — an `(or …)` in the then — gives no single-variable bound, so
           `(- n 1)` keeps its guard and STILL traps at MIN. rcdzc:
           a_conjunction_or_disjunction_condition_refines_both_variable_bounds.")
  (input
    (do
      (def (asub (: n Int64)) (if (and (> n 0) (< n 100)) (- n 1) 0))
      (def (aadd (: n Int64)) (if (and (> n 0) (< n 100)) (+ n 1) 0))
      (def (oelse (: n Int64)) (if (or (< n 1) (> n 99)) 0 (- n 1)))
      (def (two (: a Int64) (: b Int64)) (if (and (> a 0) (> b 0)) (+ (- a 1) (- b 1)) 0))
      (def (othen (: n Int64)) (if (or (> n 0) (< n -100)) (- n 1) 0))
      (export asub)
      (export aadd)
      (export oelse)
      (export two)
      (export othen)))
  (call asub (: 50 Int64))
  (output (: 49 Int64))
  (call aadd (: 99 Int64))
  (output (: 100 Int64))
  (call oelse (: 50 Int64))
  (output (: 49 Int64))
  (call two (: 5 Int64) (: 3 Int64))
  (output (: 6 Int64))
  (call othen (: -9223372036854775808 Int64))
  (trap "overflow"))

; -- range-vs-range fold + unsigned equality-guard arith-shed (behavioral halves migrated from rcdzc
; 2026-08-27; the white-box Lir compare-count / guard-count inspections stay wasmtime-free rcdzc unit tests).
(case
  "a branch refinement folds a comparison of two disjoint refined ranges (value parity)"
  (doc
    "When two DIFFERENT variables are each refined by an enclosing branch so their ranges are
           DISJOINT, a comparison BETWEEN them is decided: under `a > 100` and `b < 50`, `b < a` is always
           true, so the innermost `if` collapses. An OVERLAPPING refinement (`b < 500`, so b∈[…,499] and
           a∈[101,…] overlap) leaves `b < a` undecided and computes normally. rcdzc:
           a_branch_refinement_folds_a_comparison_of_two_disjoint_refined_ranges.")
  (input
    (do
      (def (dj (: a Int64) (: b Int64)) (if (> a 100) (if (< b 50) (if (< b a) 1 0) 0) 0))
      (def (ov (: a Int64) (: b Int64)) (if (> a 100) (if (< b 500) (if (< b a) 1 0) 0) 0))
      (export dj)
      (export ov)))
  (call dj (: 200 Int64) (: 10 Int64))
  (output (: 1 Int64))
  (call dj (: 200 Int64) (: 49 Int64))
  (output (: 1 Int64))
  (call dj (: 200 Int64) (: 60 Int64))
  (output (: 0 Int64))
  (call dj (: 50 Int64) (: 10 Int64))
  (output (: 0 Int64))
  (call ov (: 400 Int64) (: 300 Int64))
  (output (: 1 Int64))
  (call ov (: 200 Int64) (: 300 Int64))
  (output (: 0 Int64)))

(case
  "an unsigned equality guard pins the exact value so a then-branch arith sheds its guard (value parity)"
  (doc
    "The UNSIGNED companion of the equality point-fact arith-guard shed: under `(= x 200)` on a
           UInt8, `x` pins to [200,200], so `(+ x 1) = 201` provably fits UInt8 and sheds its overflow
           guard; any other x takes the else 0. rcdzc:
           an_equality_guard_pins_the_variable_to_the_exact_value_in_the_then_branch (unsigned face).")
  (input (do (def (main (: x UInt8)) (: (if (= x 200) (: (+ x 1) UInt8) 0) UInt8)) (export main)))
  (call main (: 200 UInt8))
  (output (: 201 UInt8))
  (call main (: 50 UInt8))
  (output (: 0 UInt8)))

; -- MIXED-operator same-direction comparison subsumption (migrated from rcdzc
; a_mixed_operator_same_direction_comparison_pair_subsumes; the Lir compare-count subsumption inspection
; stays a wasmtime-free rcdzc unit test): `(< x 5)` and `(<= x 4)` both mean x<=4, so a same-direction
; pair subsumes to ONE compare REGARDLESS of which of </<= (or >/>=) each side uses — `and` keeps the
; tighter inclusive bound, `or` the looser. The value must land on the exact inclusive boundary.
(case
  "csm1 a mixed-operator and (< with <=) subsumes to the tighter inclusive bound"
  (doc
    "`(and (< x 5) (<= x 4))` normalizes both to x<=4 and keeps one compare: 3→1, 4→1 (the inclusive
           boundary), 5→0, 6→0.")
  (input (do (def (main (: x Int64)) (if (and (< x 5) (<= x 4)) 1 0)) (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 4 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64))
  (call main (: 6 Int64))
  (output (: 0 Int64)))

(case
  "csm2 a mixed-operator or (<= with <) subsumes to the looser inclusive bound"
  (doc
    "`(or (<= x 10) (< x 5))` keeps the looser x<=10: 5→1, 10→1 (inclusive boundary), 11→0, 12→0.")
  (input (do (def (main (: x Int64)) (if (or (<= x 10) (< x 5)) 1 0)) (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (call main (: 10 Int64))
  (output (: 1 Int64))
  (call main (: 11 Int64))
  (output (: 0 Int64))
  (call main (: 12 Int64))
  (output (: 0 Int64)))

(case
  "csm3 a mixed-operator lower-bound or (> with >=) subsumes to the looser bound"
  (doc "`(or (> x 5) (>= x 3))` keeps the looser x>=3: 2→0, 3→1 (inclusive boundary), 4→1, 6→1.")
  (input (do (def (main (: x Int64)) (if (or (> x 5) (>= x 3)) 1 0)) (export main)))
  (call main (: 2 Int64))
  (output (: 0 Int64))
  (call main (: 3 Int64))
  (output (: 1 Int64))
  (call main (: 4 Int64))
  (output (: 1 Int64))
  (call main (: 6 Int64))
  (output (: 1 Int64)))

(case
  "csm4 the mixed-operator subsumption keeps a trapping operand's trap"
  (doc
    "The surviving compare still evaluates the shared operand: `(and (< (/ 100 z) 5) (<= (/ 100 z) 4))`
           at z=0 divides by zero and traps; z=100 → (/100 100)=1, 1<5 and 1<=4 → 1.")
  (input
    (do (def (main (: z Int64)) (if (and (< (/ 100 z) 5) (<= (/ 100 z) 4)) 1 0)) (export main)))
  (call main (: 100 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "a let binder shadowing a parameter rebinds within its scope (same type)"
  (doc
    "`(let ((x 7)) x)` inside `(def (f (: x Int64)) …)` shadows the Int64 param x with the inner 7 —
           the trailing x reads the inner binding, not the substituted argument. f(n) = 7 for any n.")
  (input (do (def (f (: x Int64)) (let ((x 7)) x)) (def (main (: n Int64)) (f n)) (export main)))
  (call main (: 99 Int64))
  (output (: 7 Int64)))

(case
  "a let binder shadowing a parameter with a DIFFERENT type rebinds correctly"
  (doc
    "The inner `(let ((x true)) …)` shadows the Int64 param x with a Bool — the inner x is Bool.
           f(n) = (if x 1 0) with x=true = 1 for any n (was an invalid component before the fix).")
  (input
    (do
      (def (f (: x Int64)) (let ((x true)) (if x 1 0)))
      (def (main (: n Int64)) (f n))
      (export main)))
  (call main (: 99 Int64))
  (output (: 1 Int64)))

(case
  "a do-local def shadowing a parameter rebinds the name (does not unbind it)"
  (doc
    "`(do (def v (* v 2)) v)` inside `(def (f (: v Int64)) …)`: the do-def's RHS reads the outer
           param v, the trailing v reads the do-def — a rebind, not a spurious CDZ0101 unbound. f(v)=2v.")
  (input
    (do (def (f (: v Int64)) (do (def v (* v 2)) v)) (def (main (: v Int64)) (f v)) (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a do-local def function references a sibling do-local def function by name"
  (doc
    "A do-local `(def (f …) …)` FUNCTION references a sibling do-local `(def (dbl …) …)` function by
           bare name: `dbl` binds lexically to a Lambda, so its use inside `f`'s body is a captured free
           var that must be pinned before β-reduction (exactly as a module sibling is), else the copied
           body re-resolves it against an orphan scope (a spurious CDZ0101). f(3) = dbl(3) + 1 = 7.")
  (input (do (def (dbl x) (* x 2)) (def (f x) (+ (dbl x) 1)) (f 3)))
  (output (: 7 Int64)))

; -- a literal payload pattern refines a sum match, falling through to a same-variant binder (migrated
; from rcdzc a_literal_payload_pattern_refines_a_sum_match; the (Some 0)->100 hit is covered @3176/@3185):
; a non-matching literal payload falls through to the same-variant binder arm, and a literal-only arm with
; no binder fall-through is non-exhaustive (the literal does not cover the variant).
(case
  "lps1 a non-matching literal payload falls through to the same-variant binder"
  (doc
    "`(match (Some n) ((Some 0) 100) ((Some k) k) ((None _) -1))` with n=5: the literal `(Some 0)` arm
           does not match, so it falls through to the binder `(Some k)` -> 5.")
  (input
    (do
      (def (f n) (match (Some n) ((Some 0) 100) ((Some k) k) ((None _) -1)))
      (def (main) (f 5))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "lps2 a literal-only sum arm with no binder fall-through is non-exhaustive"
  (doc
    "`(match (Some n) ((Some 0) 100) ((None _) -1))` covers only `(Some 0)`, not a `Some` carrying
           any other value, and has no same-variant binder fall-through -> non-exhaustive, CDZ0210.")
  (input
    (do (def (f n) (match (Some n) ((Some 0) 100) ((None _) -1))) (def (main) (f 5)) (export main)))
  (error CDZ0210))

; -- a guarded-wildcard arm falling through to a self-tail-call iterates as a loop (migrated from rcdzc
; a_guarded_wildcard_arm_falling_through_to_a_tail_call_iterates_the_loop): a match whose first arm is a
; GUARDED WILDCARD and whose fall-through arm SELF-TAIL-CALLS compiles to a wasm loop. Two composed
; miscompiles (tail-depth: a guarded wildcard has no probe if, so the fall-through br'd past the loop;
; branch-scratch: a heap-handle guard's scratch slot must sit above the iteration arithmetic) made these
; emit INVALID wasm — running find(0) is the witness.
(case
  "gwl1 a scalar-guarded wildcard falling through to a self-tail-call iterates (tail-depth)"
  (input
    (do (def (find (: n Int64)) (match n ((guard x (> x 2)) x) (_ (find (+ n 1))))) (export find)))
  (call find (: 0 Int64))
  (output (: 3 Int64)))

(case
  "gwl2 a value-eq-guarded wildcard falling through to a tail-call iterates (tail-depth + scratch)"
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk (: n Int64)) (N.I n))
      (def (find (: n Int64)) (match n ((guard x (= (mk x) (mk 3))) x) (_ (find (+ n 1)))))
      (export find)))
  (call find (: 0 Int64))
  (output (: 3 Int64)))

(case
  "gwl3 a value-eq match scrutinee in a self-tail-call loop (scrutinee scratch)"
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk (: n Int64)) (N.I n))
      (def (find (: n Int64)) (match (= (mk n) (mk 3)) (true n) (false (find (+ n 1)))))
      (export find)))
  (call find (: 0 Int64))
  (output (: 3 Int64)))

(case
  "gwl4 a value-eq guard on a literal-probe arm in a tail-call loop (probe-else scratch)"
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk (: n Int64)) (N.I n))
      (def (find (: n Int64)) (match n ((guard 3 (= (mk n) (mk 3))) 300) (_ (find (+ n 1)))))
      (export find)))
  (call find (: 0 Int64))
  (output (: 300 Int64)))

(case
  "gwl5 a value-eq guard on a sum-match arm with a call scrutinee in a tail-call loop (sum-cont scratch)"
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (bump (: n Int64)) (if (< n 0) (N.J n) (N.I n)))
      (def (mk (: n Int64)) (N.I n))
      (def
        (find (: n Int64))
        (match (bump n) ((guard (N.I x) (= (mk x) (mk 3))) x) (_ (find (+ n 1)))))
      (export find)))
  (call find (: 0 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

; --- An int-literal-vs-float branch clash in `if`/`match` offers a float-literal retype fix ----
; A branch clash between an INTEGER LITERAL and a FLOAT branch carries the same one-shot repair the list-
; element / annotation sites give: rewrite the int literal `n` as a float `n.0` so both branches unify at the
; float type (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). Either branch may
; hold the int literal, and the fix targets whichever one it is. NOTE the code differs by construct: an `if`
; branch clash surfaces as CDZ0201 (a malformed conditional), a `match` arm clash as CDZ0203 (an arm-type
; mismatch) — both carry the retype fix. A cross-KIND clash (int-vs-bool) is NOT coercible, so no fix is
; offered. (Migrated from rcdzc if_and_match_int_literal_vs_float_offer_a_float_literal_retype_fix.)
(case
  "an if with an int-literal then-branch and a float else-branch retypes the int literal up"
  (input (do (def (f (: b Bool)) (if b 1 2.0)) (export f)))
  (error CDZ0201 (fix (kind replace) (replacement "1.0"))))

(case
  "an if with a float then-branch and an int-literal else-branch retypes the else int literal up"
  (input (do (def (f (: b Bool)) (if b 1.0 2)) (export f)))
  (error CDZ0201 (fix (kind replace) (replacement "2.0"))))

(case
  "a match with an int-literal arm and a later float arm retypes the int-literal arm up"
  (input (do (def (f (: x Int64)) (match x (0 1) (_ 2.0))) (export f)))
  (error CDZ0203 (fix (kind replace) (replacement "1.0"))))

(case
  "a match with a float arm and a later int-literal arm retypes the int-literal arm up"
  (input (do (def (f (: x Int64)) (match x (0 1.0) (_ 2))) (export f)))
  (error CDZ0203 (fix (kind replace) (replacement "2.0"))))

(case
  "an int-vs-bool if branch clash is not coercible, so no float-retype fix is offered"
  (doc
    "`(if b 1 true)` clashes an Int64 branch with a Bool branch — a cross-KIND clash with no shared
           numeric type, so unlike the int-vs-float cases it carries NO literal-retype fix (there is no
           `1`→`1.0` that would unify a number with a boolean). Pins that the retype fix is offered only for a
           genuinely coercible int-literal-vs-float clash.")
  (input (do (def (f (: b Bool)) (if b 1 true)) (export f)))
  (error CDZ0203 (no-fix)))

; A literal-arm match with no wildcard is NON-EXHAUSTIVE over its scalar domain (CDZ0210), regardless of
; whether the runtime scrutinee would hit an arm. For Bool the relaxation is precise: a match is exhaustive
; ONLY with BOTH `true` and `false` arms — a single Bool literal, or two of the SAME literal, still leaves a
; value uncovered. An Int64 literal match without a wildcard stays rejected (the both-literals relaxation is
; Bool-specific). (Migrated from rcdzc a_non_wildcard_pattern_after_a_literal_still_needs_a_wildcard +
; a_bool_match_missing_a_literal_is_still_non_exhaustive.)
(case
  "an integer literal match with no wildcard is non-exhaustive"
  (input (do (def (f (: n Int64)) (match n (0 1) (1 2))) (export f)))
  (error CDZ0210))

(case
  "a Bool match with only the true arm is non-exhaustive"
  (input (do (def (main (: b Bool)) (match b (true 1))) (export main)))
  (error CDZ0210))

(case
  "a Bool match with only the false arm is non-exhaustive"
  (input (do (def (main (: b Bool)) (match b (false 2))) (export main)))
  (error CDZ0210))

(case
  "a Bool match with two of the same literal is non-exhaustive"
  (input (do (def (main (: b Bool)) (match b (true 1) (true 2))) (export main)))
  (error CDZ0210))

(case
  "a Bool match with both true and false arms is exhaustive and runs"
  (input (do (def (main) (match true (true 1) (false 2))) (export main)))
  (call main)
  (output (: 1 Int64)))

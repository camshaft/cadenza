; The `?` / `try` fallible short-circuit operator — witnesses DESIGN-try-operator-rcdzc.md. `(try e)` is
; the canonical s-expr form of the ML postfix `e?`: on the success variant it UNWRAPS the payload
; (`Some a` / `Ok a` → `a`), and on the failure variant it SHORT-CIRCUITS the enclosing fallible boundary
; (the enclosing function's `Result`/`Option` result type), making the boundary's value the failure
; itself. It is NOT a monad — it desugars onto the effects system's within-function abortive lowering (a
; `Core::Block` boundary + a `Core::Break` short-circuit), so it adds no user-visible effect and nothing to
; the effect row. See README.md for the case vocabulary.
;
; STAGE STATUS (2026-07-16). LARGELY LANDED. Front-half: `(try e)` is carried first-class through
; resolve/infer — its type is the operand's success payload; an operand that is not a fallible sum is
; CDZ0203; a wrong-arity `(try …)` is CDZ0201; a `?` with no enclosing `Result`/`Option` function boundary
; is CDZ0230; a `Result`-`?` under an `Option` boundary (or vice-versa) is CDZ0203 (no coercion). Lowering:
; a CONSTANT `?` compiles and EXECUTES both paths — the success fold (BRICK 2a) and the constant-failure
; short-circuit (BRICK 3a, via `Core::Block`/`Break` + the `lower_let` break-fold), so the executing Option
; cases below PASS (a value comes out the far side). REMAINING: a RUNTIME `?` (a non-constant operand →
; the `Core::MatchSum` / `block`-`br` emit, BRICK 3b) still DECLINES (scored *todo*); the ML postfix `?`
; surface; the `try { }` block boundary (v2); and the T3 conversion-idiom prelude ops.

; ── T0a: rejections (these PASS today) ──────────────────────────────────────────────────────────────

(case "a `?` on a non-fallible operand is a type error"
  (doc    "`(try 5)` — a `?` on a plain `Int64` has nothing to unwrap. The operand of `?` must be a
           fallible `Result`/`Option`; anything else is the ordinary type mismatch CDZ0203, anchored at
           the operand. Pins the operand-shape half of the `?` type rule
           (DESIGN-try-operator-rcdzc.md §5): `?` unwraps a fallible sum, so a non-sum operand has no
           well-typed result and is rejected rather than run.")
  (input  (do (def (main) (try 5)) (export main)))
  (error  CDZ0203))

(case "a `?` on a String operand is a type error"
  (doc    "`(try \"hi\")` — the String companion of the `(try 5)` case: a `?` on any definite
           non-fallible type is CDZ0203. Pins that the operand-shape check is by the sum's VARIANT set
           (Some/None, Ok/Err), not by the operand happening to be a number.")
  (input  (do (def (main) (try "hi")) (export main)))
  (error  CDZ0203))

(case "`try` takes exactly one operand — a zero-operand form is malformed"
  (doc    "`(try)` has nothing to unwrap — `try` is a fixed one-operand form (like `not`/`quote`), so a
           zero-operand `(try)` is malformed, CDZ0201. Pins the arity check of `resolve_try`.")
  (input  (do (def (main) (try)) (export main)))
  (error  CDZ0201))

(case "`try` takes exactly one operand — a two-operand form is malformed"
  (doc    "`(try (Ok 1) (Ok 2))` supplies a surplus operand; `try` takes EXACTLY one (the fallible
           expression it unwraps), so it is malformed, CDZ0201, with the surplus-delete fix path shared
           with `quote`. Pins the upper arity bound of `resolve_try`.")
  (input  (do (def (main) (try (Ok 1) (Ok 2))) (export main)))
  (error  CDZ0201))

(case "a `?` with no fallible enclosing function boundary is rejected"
  (doc    "`(def (main) (try (Ok 1)))` — main's body IS the `?`, whose type is the UNWRAPPED `Int64`, so
           main returns `Int64`, neither `Result` nor `Option`. A `?` short-circuits the enclosing
           function's fallible result type (DESIGN-try-operator-rcdzc.md §4/§6); with no fallible boundary
           to exit to, the program is rejected, CDZ0230. Pins the boundary half of the `?` rule — distinct
           from CDZ0203 (a `?` on a non-fallible OPERAND); here the operand IS fallible but the BOUNDARY
           is not.")
  (input  (do (def (main) (try (Ok 1))) (export main)))
  (error  CDZ0230))

(case "a Result-valued `?` under an Option boundary is a type error"
  (doc    "`(def (main) (let ((x (try (Ok 1)))) (Some x)))` — the body's tail `(Some x)` makes main's
           result type `Option`, but the `?`'s operand `(Ok 1)` is a `Result`. A `Result`-`?` cannot
           short-circuit an `Option` boundary — the kinds disagree and Cadenza has NO auto-conversion
           (§5.1, against Rust's `?`-via-`From`), so it is CDZ0203. The explicit idiom is to `match` the
           `Result` and drop its error (`(Err _) => (None unit)`, `(Ok x) => (Some x)`) before the `?`
           (a prelude `Result.map-err`/`Option.ok-or` is the T3 increment — not yet in the prelude, so the
           CDZ0203 hint names the `match` re-wrap that exists today, not an absent op).")
  (input  (do (def (main) (let ((x (try (Ok 1)))) (Some x))) (export main)))
  (error  CDZ0203))

; ── T1 executing cases (the operator's actual ask — these PASS: a value comes out the far side) ───────
; The nested-`match`-collapse shapes, executed through wasmtime. Both operands here fold at compile time
; (a checked-arith over constants → `Some v` / `None`), so the constant `?` desugar selects the arm and
; the whole program folds to its value — the happy path and the short-circuit path both produce a value.
; (A RUNTIME `?` — a non-constant operand — is BRICK 3b, still todo.)

(case "`?` on the success variant unwraps the payload (Option, happy path)"
  (doc    "`parse-pair`-shaped Option chain collapsed with `?`: both `?`s see a `Some`, so the boundary
           falls through to the body's `Some`. `(Int64.checked-add 20 22)` = `(Some 42)`, so `x` = 42;
           `(Int64.checked-add 40 2)` = `(Some 42)`, so `y` = 42; the function returns `(Some (+ x y))` =
           `(Some 84)`. Witnesses DESIGN-try-operator-rcdzc.md §3.2 (the Option desugar) + §4 v1 (the
           enclosing-function boundary) on the happy path.")
  (input  (do
            (def (main)
              (let ((x (try (Int64.checked-add 20 22))))
                (let ((y (try (Int64.checked-add 40 2))))
                  (Some (+ x y)))))
            (export main)))
  (output (: (Some 84) (Option Int64))))

(case "`?` on the failure variant short-circuits the boundary (Option, None path)"
  (doc    "The short-circuit companion: the FIRST `?` sees a `None` (an overflowing checked-add), so it
           BREAKS the enclosing boundary — the function's value becomes `None` and the second `?` and the
           body never run. `(Int64.checked-add Int64.max-value 1)` overflows → `None`, so `(try …)` bails
           and `main` = `(None unit)`. Witnesses the abortive path of §3.2/§4: `?` exits the lexically
           enclosing function, contributing nothing to the effect row.")
  (input  (do
            (def (main)
              (let ((x (try (Int64.checked-add Int64.max 1))))
                (let ((y (try (Int64.checked-add 40 2))))
                  (Some (+ x y)))))
            (export main)))
  (output (: (None unit) (Option Int64))))

; ── T1a gate pins: invariants the constant-fold desugar must hold (all PASS today) ───────────────────
; These pin now-passing behaviors so a future change to the `?` desugar (or the BRICK sequence) cannot
; silently flip them. Added by v-try-operator after adversarially probing the landed BRICK 2a/3a folds.

(case "`?` unwraps an Ok payload under a Result boundary (happy path)"
  (doc    "The Result companion of the Option happy path: `(try (Ok 42))` under a `(Result Int64 Int64)`
           boundary unwraps the `Ok` payload to `42`, and the body's tail `(Ok x)` re-wraps it, so `main`
           = `(Ok 42)`. The result type is annotated so the `Err` type is determined (a bare `(Ok 42)`
           leaves `Err` unsolved — CDZ0203). Pins that the success fold reads the `Ok` disc off a Result
           exactly as it reads `Some` off an Option (`success_disc_of`, by variant NAME).")
  (input  (do (def (main) (: (let ((x (try (Ok 42)))) (Ok x)) (Result Int64 Int64))) (export main)))
  (output (: (Ok 42) (Result Int64 Int64))))

(case "two `?`s in one boundary both unwrap (nested happy path)"
  (doc    "The `parse-pair` shape with constant operands: `(let ((x (try (Some 20)))) (let ((y (try (Some
           22)))) (Some (+ x y))))` — both `?`s see a `Some`, so `x` = 20, `y` = 22, and the boundary
           falls through to `(Some 42)`. Pins that MULTIPLE `?`s under one boundary each unwrap
           independently and the happy path threads through to the body's tail — the nested-match collapse
           the operator asked for.")
  (input  (do
            (def (main)
              (let ((x (try (Some 20)))) (let ((y (try (Some 22)))) (Some (+ x y)))))
            (export main)))
  (output (: (Some 42) (Option Int64))))

(case "`?` unwraps a COMPOUND (tuple) payload"
  (doc    "`(try (Some (tuple 1 2)))` unwraps the tuple payload whole, so `(Some x)` = `(Some (tuple 1
           2))`. Pins that the payload the `?` binds is not restricted to a scalar — a compound (tuple/
           record/sum) payload flows through the success fold intact, its type preserved
           (`(Option (Tuple Int64 Int64))`).")
  (input  (do (def (main) (let ((x (try (Some (tuple 1 2))))) (Some x))) (export main)))
  (output (: (Some (tuple 1 2)) (Option (Tuple Int64 Int64)))))

(case "a `?` result is usable mid-body, not only in tail position"
  (doc    "`(let ((x (try (Some 10)))) (Some (+ x 5)))` — the unwrapped `x` = 10 feeds an arithmetic op
           BEFORE the boundary's tail, giving `(Some 15)`. Pins that `?` UNWRAPS to an ordinary value the
           rest of the body computes with (it is not confined to a tail `(Some …)` re-wrap): the success
           payload is a first-class value in its continuation.")
  (input  (do (def (main) (let ((x (try (Some 10)))) (Some (+ x 5)))) (export main)))
  (output (: (Some 15) (Option Int64))))

(case "a constant-failure `?` short-circuit ELIDES an earlier trapping let-init whose value it discards"
  (doc    "OPERATOR §283 RULING (2026-07-16): `we don't emit the trap unless it's reachable; a detected
           unreachable trap is a WARNING.` `(let ((a (/ 1 0)) (x (try (None unit)))) (Some (+ a x)))` — `a`
           traps (÷0) and is referenced only in `(+ a x)`, but `x`'s `?` sees a constant `None` and SHORT-
           CIRCUITS, so `(+ a x)` never runs and `a`'s value is UNOBSERVED (§285 laziness of an unselected
           branch — its value reaches neither the result nor a host call). So the trap is ELIDED, the whole
           expression folds to `(None unit)`, and the ÷0 is a §285 SHOULD-diagnose CDZ0305 WARNING (build
           succeeds), NOT a CDZ0304 reject. (Earlier this pinned CDZ0304 — an over-strict `is_trap_free`
           guard the operator ruling reverted; a host call, being observable, still bails the fold.) This
           keeps the same-let, nested-let, and `if false` shapes CONSISTENT-elide with the landed §283 DCE.")
  (input  (do (def (main) (let ((a (/ 1 0)) (x (try (None unit)))) (Some (+ a x)))) (export main)))
  (output (: (None unit) (Option Int64))))

(case "a constant-failure `?` in a NESTED let elides a trapping OUTER-let init it discards"
  (doc    "The nested-let companion (same §283 operator ruling): `(let ((a (/ 1 0))) (let ((x (try (None
           unit)))) (Some (+ a x))))` — `a` is bound in the OUTER let, referenced only in `(+ a x)`, and
           the inner `?` short-circuits before it runs, so `a`'s value is UNOBSERVED. Its ÷0 trap is ELIDED
           (→ `(None unit)`) with a CDZ0305 warning, exactly like the same-let case — observation, not the
           syntactic nesting or evaluation-order, governs (§285). Consistent-elide.")
  (input  (do (def (main) (let ((a (/ 1 0))) (let ((x (try (None unit)))) (Some (+ a x))))) (export main)))
  (output (: (None unit) (Option Int64))))

; The §283 elision above applies ONLY to an UNOBSERVED trapping init (the `?` short-circuits before its
; value is used). The NEGATIVE boundary: when the trapping init's value IS observed, the trap still FIRES —
; it is not silently dropped. These pin both observation shapes: a SUCCESS `?` (no short-circuit, so the
; init is used in the result) and an init observed IN the `?`'s own operand (used before the short-circuit
; could even happen). Both keep the provable ÷0 as a CDZ0304 reject, guarding that the elision does not
; over-reach into observable traps (the observable-trap-is-preserved axis, sibling of the trap-ordering rule).

(case "a success `?` observes an earlier trapping let-init so the trap is not elided"
  (doc    "The negative boundary of the §283 elision: `(let ((a (/ 1 0)) (x (try (Some 5)))) (Some (+ a x)))`
           — the `?` sees a `Some` and does NOT short-circuit, so `(+ a x)` runs and `a`'s ÷0 IS observed. The
           trap is therefore NOT elided: the provable ÷0 is a CDZ0304 reject, exactly as it is without any
           `?`. Pins that the elision fires only when the `?` short-circuits AWAY from the trapping value —
           a SUCCESS `?` leaves the value observed, so the trap stands. The complement of the elide cases
           above (which all short-circuit on a constant None).")
  (input  (do (def (main) (let ((a (/ 1 0)) (x (try (Some 5)))) (Some (+ a x)))) (export main)))
  (error  CDZ0304))

(case "a trapping init observed inside the `?`'s own operand is not elided"
  (doc    "`(let ((a (/ 1 0)) (x (try (Some a)))) (Some x))` — `a`'s value flows INTO the `?`'s operand
           `(Some a)`, so it is observed BEFORE the short-circuit could occur (the operand must be built to
           be matched). The ÷0 is observed regardless of the `?`'s success/failure, so the trap is not
           elided: CDZ0304. Pins that a value consumed by the `?` operand itself is observed, distinct from
           a value used only in a body the `?` may short-circuit past.")
  (input  (do (def (main) (let ((a (/ 1 0)) (x (try (Some a)))) (Some x))) (export main)))
  (error  CDZ0304))

(case "a `?` in a CALLED (inlined, non-exported) helper finds its boundary"
  (doc    "Regression pin: `(def (f) (let ((x (try (Some 7)))) (Some (+ x 3))))` is CALLED by `main` (only
           `main` is exported), so `f` is INLINED at the call site. `f`'s result type IS `Option`, so the
           `?` is well-formed — but a bug made the boundary walk (`enclosing_boundary_ty`) fall off the
           inlined COPY's re-parented tree and FALSELY reject CDZ0230 (`no fallible boundary`). The boundary
           walk is now INCONCLUSIVE when it falls off a re-parented copy (raises nothing); the genuine
           non-fallible-boundary reject still fires from the original body's walk. `f` = `(Some 10)`, so
           `main` = `(Some 10)`. Pins that a `?` in a called helper compiles, not spuriously rejects.")
  (input  (do
            (def (f) (let ((x (try (Some 7)))) (Some (+ x 3))))
            (def (main) (f))
            (export main)))
  (output (: (Some 10) (Option Int64))))

; ── T1a gate pins (round 2): the short-circuit SKIPS subsequent work + `?` under if/match ────────────
; Added by v-try-operator after adversarial probing of the BRICK 3a constant-failure fold. All PASS.

(case "a failure `?` short-circuits BEFORE later computation runs"
  (doc    "`(let ((x (try (None unit)))) (let ((y 100)) (Some (+ x y))))` — the FIRST binding's `?` sees a
           `None`, so the `let` short-circuits to `None` and the inner `let` + `(+ x y)` NEVER run. Pins
           that the short-circuit abandons the continuation (not just unwraps): the `(+ x y)` using the
           unbound-on-failure `x` is skipped, so the result is `(None unit)`, never a use of a missing
           payload.")
  (input  (do (def (main) (let ((x (try (None unit)))) (let ((y 100)) (Some (+ x y))))) (export main)))
  (output (: (None unit) (Option Int64))))

(case "the first failing `?` short-circuits; a later `?` never runs"
  (doc    "`(let ((x (try (None unit)))) (let ((y (try (Some 7)))) (Some (+ x y))))` — the FIRST `?`
           fails, so the boundary short-circuits to `None` and the SECOND `?` (`(try (Some 7))`) is never
           evaluated. Pins left-to-right short-circuit order across multiple `?`s: the first failure wins.")
  (input  (do (def (main) (let ((x (try (None unit)))) (let ((y (try (Some 7)))) (Some (+ x y))))) (export main)))
  (output (: (None unit) (Option Int64))))

(case "a `?` inside an if-branch resolves against the enclosing function boundary"
  (doc    "`(if true (let ((x (try (Some 5)))) (Some (+ x 1))) (None unit))` — the `?` in the THEN branch
           finds its boundary through the enclosing `if` up to `main`'s `Option` result (the if's branches
           are both `Option`). The taken branch unwraps `x` = 5 → `(Some 6)`. Pins that a `?` nested in a
           conditional still resolves the enclosing function as its boundary.")
  (input  (do (def (main) (if true (let ((x (try (Some 5)))) (Some (+ x 1))) (None unit))) (export main)))
  (output (: (Some 6) (Option Int64))))

(case "a `?` inside a match-arm resolves against the enclosing function boundary"
  (doc    "`(match 0 (0 (let ((x (try (Some 9)))) (Some x))) (_ (None unit)))` — the `?` in the first arm
           finds `main`'s `Option` boundary through the enclosing `match`. Arm 0 is selected, `x` = 9 →
           `(Some 9)`. The match-arm companion of the if-branch case.")
  (input  (do (def (main) (match 0 (0 (let ((x (try (Some 9)))) (Some x))) (_ (None unit)))) (export main)))
  (output (: (Some 9) (Option Int64))))

; --- The strict spine around a short-circuiting `?`: effects, ordering, and the cut point ----------
; The trapping-earlier-init pin above grades the compile-provable face (CDZ0304). These grade the
; RUNTIME spine: an effectful init BEFORE a failing `?` is observed (performs exactly once), a
; success-`?` then a failure-`?` cuts at the second, and an init AFTER the failing `?` — including a
; provably-trapping one — never evaluates (the short-circuit is the spine's cut point; only earlier
; inits are observed). Promoted from passing breaker probes.

(case "an effectful init before a failing `?` performs exactly once"
  (doc    "`(let ((a (Ctr.tick)) (x (try (None unit)))) (Some (+ a x)))` under a counter handler —
           the tick sits on the strict spine BEFORE the `?`, so it performs (state advances 0→1)
           and THEN the boundary short-circuits to None (→ -1); the trailing `(Ctr.tick)` reads 1 →
           0. A fold that discarded the earlier effectful init answers 1·(-1) + 0 = -1; one that
           duplicated it answers 1. The runtime-effect companion of the trapping-earlier-init
           CDZ0304 pin.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (opt) (let ((a (Ctr.tick unit)) (x (try (None unit)))) (Some (+ a x))))
            (def (main)
              (handle Ctr 0 ((tick (_) s (resume s (+ s 1))))
                (+ (match (opt) ((Some v) v) ((None _) -1))
                   (Ctr.tick unit))))
            (export main)))
  (output (: 0 Int64)))

(case "a success `?` then a failure `?` short-circuits at the second"
  (doc    "`x = (try (Some 1))` unwraps (the happy path); the SECOND `?` sees None and cuts the
           boundary → the caller matches None → -1. Pins the chain semantics: each `?` is its own cut
           point, and a successful unwrap does not immunize the rest of the body (nor does the
           short-circuit rewind the already-bound x).")
  (input  (do
            (def (opt) (let ((x (try (Some 1)))) (let ((y (try (None unit)))) (Some (+ x y)))))
            (def (main)
              (match (opt) ((Some v) v) ((None _) -1)))
            (export main)))
  (output (: -1 Int64)))

(case "an init after the failing `?` never evaluates — even a provably-trapping one"
  (doc    "`(let ((x (try (None unit)))) (let ((y (/ 1 0))) …))` — the `?` cuts the spine FIRST, so
           the later `(/ 1 0)` init is genuinely unreachable: the function yields None → -1, no trap
           and no CDZ0304 (contrast the EARLIER-init pin above, where the same ÷0 before the `?` must
           fail the build). Together the two pins locate the cut point exactly: earlier inits are
           observed, later inits are dead.")
  (input  (do
            (def (opt) (let ((x (try (None unit)))) (let ((y (/ 1 0))) (Some (+ x y)))))
            (def (main)
              (match (opt) ((Some v) v) ((None _) -1)))
            (export main)))
  (output (: -1 Int64)))

(case "a `?` whose Result error type disagrees with the boundary's is rejected"
  (doc    "SOUNDNESS pin: `(def (main) (: (let ((y (try (Err true)))) (Ok y)) (Result Int64 Int64)))` —
           the `?`'s operand `(Err true)` is a `Result _ Bool` (error type `Bool`), but the enclosing
           function's declared error type is `Int64`. A `?` short-circuits by passing its `Err` OUT
           UNCHANGED as the boundary value, so the error types MUST agree (§5: the error type unifies with
           the boundary's; Cadenza has no automatic error conversion). Without this check the `Bool` `true`
           escaped as a claimed `Int64` error — a soundness hole (the ordinary `(: (Err true) (Result Int64
           Int64))` annotation path already rejects it). CDZ0203.")
  (input  (do (def (main) (: (let ((y (try (Err true)))) (Ok y)) (Result Int64 Int64))) (export main)))
  (error  CDZ0203))

(case "an agreeing Result error type short-circuits through the boundary"
  (doc    "The positive control of the error-type soundness reject: `(try (Err 7))` under a
           `(Result Int64 Int64)` boundary — the error type AGREES, so the `?` short-circuits and
           the caller's Err arm reads 7. Pinned beside the disagreeing-type CDZ0203 so the check is
           graded from both sides (an over-tight fix that rejected agreeing error types breaks this).")
  (input  (do
            (def (f) (: (let ((y (try (Err 7)))) (Ok y)) (Result Int64 Int64)))
            (def (main)
              (match (f) ((Ok v) v) ((Err e) e)))
            (export main)))
  (output (: 7 Int64)))

(case "a failure `?` preserves a COMPOUND error value through the short-circuit"
  (doc    "The Err-value case above carries a scalar 7; this carries a COMPOUND: `(try (Err (tuple 3 4)))`
           under a `(Result Int64 (Tuple Int64 Int64))` boundary short-circuits, and the caller's Err arm
           destructures the tuple `(3, 4)` → 7. Pins that `?` propagates the WHOLE error payload (a compound,
           not just a scalar) unchanged through the abortive short-circuit — the error analogue of the
           compound-Ok-payload unwrap.")
  (input  (do
            (def (f) (: (let ((y (try (Err (tuple 3 4))))) (Ok y)) (Result Int64 (Tuple Int64 Int64))))
            (def (main) (match (f) ((Ok v) 0) ((Err (tuple a b)) (+ a b))))
            (export main)))
  (output (: 7 Int64)))

(case "a failure `?` propagates through TWO nested fallible boundaries"
  (doc    "An Err bubbles up MORE than one `?` boundary: `inner` returns `Err 9`; `outer`'s `(try (inner))`
           short-circuits and re-propagates the Err to its OWN boundary; `main` reads 9. Pins that the
           abortive `?` composes across nested fallible functions — the failure exits inner's boundary,
           becomes outer's `(try …)` operand, and short-circuits outer's boundary too, carrying 9 the whole
           way. A `?` that only exited one level (or lost the value across the second boundary) would give a
           wrong result. (The single-boundary short-circuit + the same-boundary two-`?` cases are pinned
           above; this is the cross-boundary composition.)")
  (input  (do
            (def (inner) (: (let ((y (try (Err 9)))) (Ok y)) (Result Int64 Int64)))
            (def (outer) (: (let ((z (try (inner)))) (Ok (+ z 1))) (Result Int64 Int64)))
            (def (main) (match (outer) ((Ok v) v) ((Err e) e)))
            (export main)))
  (output (: 9 Int64)))

(case "effect state threads across a successful `?`"
  (doc    "The straddle: `(let ((a (Ctr.tick)) (x (try (Some 0))) (b (Ctr.tick))) (Some (+ a b)))` — a
           perform BEFORE the `?` (a = 0), a SUCCESSFUL `?` unwrapping Some 0 (no short-circuit), then
           a perform AFTER (b = 1) → Some 1, and the trailing tick reads 2 → 3. The complement of the
           effectful-init-before-a-FAILING-? pin: on the success path the `?` is transparent to the
           effect spine, so the counter advances 0→1→2 straight through it. A `?` that reset or
           re-entered the handler on the success path would skew b or the trailing read.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (opt) (let ((a (Ctr.tick unit)) (x (try (Option.Some 0))) (b (Ctr.tick unit))) (Option.Some (+ a b))))
            (def (main)
              (handle Ctr 0 ((tick (_) s (resume s (+ s 1))))
                (+ (match (opt) ((Some v) v) ((None _) -1)) (Ctr.tick unit))))
            (export main)))
  (output (: 3 Int64)))
(case "a `?` on an ill-typed operand reports the operand's error, not a `?`-shape cascade"
  (doc    "`(let ((x (try (+ 1 2.0)))) (Some x))` — the operand `(+ 1 2.0)` is itself ill-typed (a numeric
           mismatch, CDZ0301). The `?`-operand-shape check must NOT pile a confusing `?` operand must be a
           fallible Result/Option, found Float64` on top: the operand's own fault is the primary `no`. Pins
           that the `?` collect arm collects the operand's faults FIRST and suppresses its shape/boundary
           checks when the operand already carries a coded fault (the `Member`-arm operand-is-poison
           discipline). Grades on CDZ0301 — the operand mismatch — not the suppressed `?` cascade.")
  (input  (do (def (main) (let ((x (try (+ 1 2.0)))) (Some x))) (export main)))
  (error  CDZ0301))

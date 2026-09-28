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
; short-circuit (BRICK 3a, via `Core::Block`/`Break` + the `lower_let` break-fold). And BRICK 3b now lands:
; a RUNTIME-DISC `?` (a non-constant operand whose Some/Ok-vs-None/Err variant is decided at run time)
; desugars in `lower_let` to a `Core::MatchSum` short-circuit — the success arm continues the body (the
; payload binder resolves to a `Core::SumPayload` of the materialized operand), the failure arm re-wraps
; the failure value (`None` / `Err r`) — so the executing Option AND runtime-disc cases below PASS on all
; three backends (a value comes out the far side on both paths). The desugar fires for a SINGLE-binding
; `let` in boundary-tail position (the `let x = e? in …` / `do (def x e?) …` idiom); a `?` under a
; non-fallible/non-tail wrapper is the same accepted v1 limitation the constant path has, and a runtime `?`
; in a MULTI-binding `let` or a non-let expression position stays a clean decline (scored *todo*).
; REMAINING: the runtime-disc `?` inside a stored CLOSURE (a lambda-boundary infer gap, still *todo* —
; CDZ0230 there); the ML postfix `?` surface; the `try { }` block boundary (v2); and the T3
; conversion-idiom prelude ops.
; ── T0a: rejections (these PASS today) ──────────────────────────────────────────────────────────────
(diagnostic-quality)

(case
  "a `?` on a non-fallible operand is a type error"
  (doc
    "`(try 5)` — a `?` on a plain `Int64` has nothing to unwrap. The operand of `?` must be a
           fallible `Result`/`Option`; anything else is the ordinary type mismatch CDZ0203, anchored at
           the operand. Pins the operand-shape half of the `?` type rule
           (DESIGN-try-operator-rcdzc.md §5): `?` unwraps a fallible sum, so a non-sum operand has no
           well-typed result and is rejected rather than run. The diagnostic says the operand must be
           `fallible`. (Enhanced from rcdzc try_on_a_non_fallible_operand_is_a_type_mismatch.)")
  (input (do (def (main) (try 5)) (export main)))
  (error CDZ0203 (message "fallible") (count 1)))

(case
  "a `?` on a String operand is a type error"
  (doc
    "`(try \"hi\")` — the String companion of the `(try 5)` case: a `?` on any definite
           non-fallible type is CDZ0203. Pins that the operand-shape check is by the sum's VARIANT set
           (Some/None, Ok/Err), not by the operand happening to be a number.")
  (input (do (def (main) (try "hi")) (export main)))
  (error CDZ0203))

(case
  "`try` takes exactly one operand — a zero-operand form is malformed"
  (doc
    "`(try)` has nothing to unwrap — `try` is a fixed one-operand form (like `not`/`quote`), so a
           zero-operand `(try)` is malformed, CDZ0201. Pins the arity check of `resolve_try`.")
  (input (do (def (main) (try)) (export main)))
  (error CDZ0201))

(case
  "`try` takes exactly one operand — a two-operand form is malformed"
  (doc
    "`(try (Ok 1) (Ok 2))` supplies a surplus operand; `try` takes EXACTLY one (the fallible
           expression it unwraps), so it is malformed, CDZ0201, with the surplus-delete fix path shared
           with `quote`. Pins the upper arity bound of `resolve_try`.")
  (input (do (def (main) (try (Ok 1) (Ok 2))) (export main)))
  (error CDZ0201))

; The s-expr surface head for the try operator is the keyword `try` (`(try e)`), NOT the `?` sigil many
; languages use — the diagnostics call it "`?`/`try`", so an author may reach for `(? e)`. A `?` in HEAD
; position resolves to no binding: CDZ0101, but the message names the real spelling (`(try <expression>)`,
; "write `try`") rather than a nonsensical did-you-mean over a sigil. A bare `?` NOT in head position
; (`(list ?)`) keeps the ORDINARY "unbound name `?`" with no try hint — only a `(? …)` head is the
; reachable-for-try mistake. (Migrated from rcdzc the_sigil_question_mark_as_a_head_points_at_the_try_spelling;
; the `?`→`try` head-rewrite FIX stays a white-box residual pending a corpus fix-grade.)
(case
  "the `?` sigil in head position names the `try` spelling, not a bare unbound name"
  (input (do (def (main) (? (Ok 1))) (export main)))
  (error CDZ0101 (message "(try <expression>)") (message "write `try`") (fix (kind replace))))

(case
  "a bare `?` NOT in head position stays an ordinary unbound name with no try hint"
  (input (do (def (main) #list(?)) (export main)))
  (error CDZ0101 (message "unbound name `?`") (not "try")))

(case
  "a `?` with no fallible enclosing function boundary is rejected"
  (doc
    "`(def (main) (try (Ok 1)))` — main's body IS the `?`, whose type is the UNWRAPPED `Int64`, so
           main returns `Int64`, neither `Result` nor `Option`. A `?` short-circuits the enclosing
           function's fallible result type (DESIGN-try-operator-rcdzc.md §4/§6); with no fallible boundary
           to exit to, the program is rejected, CDZ0230. Pins the boundary half of the `?` rule — distinct
           from CDZ0203 (a `?` on a non-fallible OPERAND); here the operand IS fallible but the BOUNDARY
           is not.")
  (input (do (def (main) (try (Ok 1))) (export main)))
  (error CDZ0230 (message "boundary") (message "(Result Int64")))

; The CDZ0230 boundary hint names the CONCRETE fallible type the `?`'d operand requires when the operand kind
; is definite (a `(try (Ok 1))` is a `Result Int64 _`, so it suggests `(Result Int64 _)` above). When the
; operand's kind is NOT yet definite — a bare unannotated param `x` in `(try x)` — the hint names BOTH forms,
; `(Result _ e)` and `(Option _)`, so either annotation is offered. (Migrated from rcdzc
; a_try_with_no_fallible_enclosing_function_is_cdz0230; the balanced-backtick message-shape check is the
; inexpressible remainder kept as a white-box residual.)
(case
  "a `?` whose operand kind is not yet definite names both fallible forms in the boundary hint"
  (input (do (def (f x) (+ 1 (try x))) (export f)))
  (error CDZ0230 (message "(Result _ e)") (message "(Option _)")))

(case
  "a `?` mid-body under a provably non-fallible boundary is rejected"
  (doc
    "`(def (main) (+ 1 (try (Some 2))))` — the `?` unwraps to `Int64` and is added to `1`, so main's
           result type is `Int64` (a definite non-fallible type inferred from the body, not merely an
           un-annotated top). Distinct from the `?`-IS-the-whole-body CDZ0230 case above: here the `?` sits
           in a SUBEXPRESSION and the boundary is provably a plain `Int64`, yet the boundary rule is the same
           — a `?` needs a fallible enclosing result to short-circuit to, and there is none, so CDZ0230.
           Pins that `enclosing_boundary_ty` walks to the function result regardless of the `?`'s syntactic
           position in the body (DESIGN-try-operator-rcdzc.md §4/§6).")
  (input (do (def (main) (+ 1 (try (Some 2)))) (export main)))
  (error CDZ0230))

(case
  "a `?` in an anonymous LAMBDA with a non-fallible result is rejected (CDZ0230)"
  (doc
    "The reject twin of the anonymous-lambda-boundary EXECUTING cases below: `((fn () (try (Some 1))))`
           — the immediately-applied lambda's body IS the `?`, whose type is the UNWRAPPED `Int64`, so the
           lambda's result type is `Int64`, neither `Result` nor `Option`. A lambda IS a function boundary
           exactly like a `def` (§6 v1), with NO auto-wrap (§5.1 fork B — the lambda's result is NOT promoted
           to `Option` just because its body has a top-level `?`), so this is CDZ0230, the SAME rule as the
           `?`-is-the-whole-def-body case above. Pins that the boundary check reaches an APPLIED anonymous
           lambda's body (it was once silently missed — the `?` was checked only on the β-reduced inlined
           copy, whose parentless walk hit the inlined-called-helper inconclusive tolerance; the fix descends
           the original parented applied-lambda body). One rule for every function body, named or anonymous.")
  (input (do (def (main) ((fn () (try (Some 1))))) (export main)))
  (error CDZ0230))

(case
  "a Result-valued `?` under an Option boundary is a type error"
  (doc
    "`(def (main) (let ((x (try (Ok 1)))) (Some x)))` — the body's tail `(Some x)` makes main's
           result type `Option`, but the `?`'s operand `(Ok 1)` is a `Result`. A `Result`-`?` cannot
           short-circuit an `Option` boundary — the kinds disagree and Cadenza has NO auto-conversion
           (§5.1, against Rust's `?`-via-`From`), so it is CDZ0203. The explicit idiom is to `match` the
           `Result` and drop its error (`(Err _) => (None unit)`, `(Ok x) => (Some x)`) before the `?`
           (a prelude `Result.map-err`/`Option.ok-or` is the T3 increment — not yet in the prelude, so the
           CDZ0203 hint names the `match` re-wrap that exists today, not an absent op).")
  (input (do (def (main) (let ((x (try (Ok 1)))) (Some x))) (export main)))
  (error CDZ0203))

; ── T1 executing cases (the operator's actual ask — these PASS: a value comes out the far side) ───────
; The nested-`match`-collapse shapes, executed through wasmtime. Both operands here fold at compile time
; (a checked-arith over constants → `Some v` / `None`), so the constant `?` desugar selects the arm and
; the whole program folds to its value — the happy path and the short-circuit path both produce a value.
; (A RUNTIME `?` — a non-constant operand — is BRICK 3b, still todo.)
(case
  "`?` on the success variant unwraps the payload (Option, happy path)"
  (doc
    "`parse-pair`-shaped Option chain collapsed with `?`: both `?`s see a `Some`, so the boundary
           falls through to the body's `Some`. `(Int64.checked-add 20 22)` = `(Some 42)`, so `x` = 42;
           `(Int64.checked-add 40 2)` = `(Some 42)`, so `y` = 42; the function returns `(Some (+ x y))` =
           `(Some 84)`. Witnesses DESIGN-try-operator-rcdzc.md §3.2 (the Option desugar) + §4 v1 (the
           enclosing-function boundary) on the happy path.")
  (input
    (do
      (def
        (main)
        (let
          ((x (try (Int64.checked-add 20 22))))
          (let ((y (try (Int64.checked-add 40 2)))) (Some (+ x y)))))
      (export main)))
  (output (: (Some 84) (Option Int64))))

(case
  "`?` on the failure variant short-circuits the boundary (Option, None path)"
  (doc
    "The short-circuit companion: the FIRST `?` sees a `None` (an overflowing checked-add), so it
           BREAKS the enclosing boundary — the function's value becomes `None` and the second `?` and the
           body never run. `(Int64.checked-add Int64.max-value 1)` overflows → `None`, so `(try …)` bails
           and `main` = `(None unit)`. Witnesses the abortive path of §3.2/§4: `?` exits the lexically
           enclosing function, contributing nothing to the effect row.")
  (input
    (do
      (def
        (main)
        (let
          ((x (try (Int64.checked-add Int64.max 1))))
          (let ((y (try (Int64.checked-add 40 2)))) (Some (+ x y)))))
      (export main)))
  (output (: (None unit) (Option Int64))))

; ── BRICK 3b: the RUNTIME-DISC `?` (the operand's Some/Ok-vs-None/Err variant is decided at RUN TIME) ──
; These are the operator-reported bug: a `?` over a NON-CONSTANT operand whose DISCRIMINANT depends on a
; runtime value (`Int64.checked-add(Int64.max, v)` for a runtime `v` overflows — or not — per call). Unlike
; the constant / static-ctor pins above (which const-fold the disc or fire the success fold at a known
; disc), here NEITHER path is compile-time-known: the SUCCESS arm unwraps the payload and the FAILURE arm
; short-circuits the enclosing boundary, and the compiled body must answer BOTH by the run-time disc. The
; `?` desugars to a `Core::MatchSum` over the operand — exactly the nested `match` the `?` collapses
; (DESIGN §0/§3.2) — the success arm continuing the body, the default arm re-wrapping the failure value.
; The `(live-objects known-leak)` clauses track the SHARED fresh-Option/Result-scrutinee reclaim gap: an
; equivalent HAND-WRITTEN `(match (Int64.checked-add …) …)` leaks the same scrutinee shell (v-memory-safety-
; owned), so the `?` desugar is leak-identical to the match it lowers to — not a `?`-specific leak.
(case
  "a RUNTIME-DISC `?` unwraps or short-circuits by a per-call value (Option — the operator repro)"
  (doc
    "The operator's reported bug, verbatim: `?` over a RUNTIME operand whose variant is decided per
           call. `(try (Int64.checked-add Int64.max v))` — for `v = -100` the add is in range → `Some
           (max-100)`, so `x` unwraps to `max-100`; the second `?` over the constant `(Int64.checked-add 40
           2)` = `Some 42` unwraps to `42`; the body returns `(Some (+ x y))` = `(Some (max-58))`. For `v =
           1` the add OVERFLOWS → `None`, so the FIRST `?` short-circuits the boundary — `main` = `(None
           unit)` and the second `?` + body never run. One compiled body answers BOTH by the run-time
           discriminant, which the constant / static-ctor folds above cannot (the disc is not known at
           compile time). Witnesses DESIGN-try-operator-rcdzc.md §3.2 (the Option desugar) + §4 v1 on a
           genuinely runtime operand — the `Core::MatchSum` short-circuit (BRICK 3b).")
  (input
    (do
      (def
        (main (: v Int64))
        (let
          ((x (try (Int64.checked-add Int64.max v))))
          (let ((y (try (Int64.checked-add 40 2)))) (Some (+ x y)))))
      (export main)))
  (call main (: -100 Int64))
  (output (: (Some 9223372036854775749) (Option Int64)))
  (call main (: 1 Int64))
  (output (: (None unit) (Option Int64)))
  (live-objects 0))

(case
  "a RUNTIME-DISC `?` unwraps Ok or short-circuits Err by a per-call value (Result)"
  (doc
    "The Result companion of the runtime-disc repro: `safe` returns `(Ok n)` or `(Err \"neg\")` picked
           by a run-time `if`, so `(try (safe v))`'s discriminant is decided per call. For `v = 5` it is
           `Ok 5` → `x` unwraps to `5`, the body re-wraps `(Ok (+ x 1))` = `(Ok 6)`. For `v = -3` it is
           `(Err \"neg\")` → the `?` short-circuits, and `main` = `(Err \"neg\")` — the error VALUE (the
           heap String payload) preserved unchanged through the re-wrap, not the scrutinee re-read. Pins
           that the runtime-disc desugar reads the `Ok` disc by NAME and reconstructs `Err r` with the
           error payload extracted from the matched scrutinee (DESIGN §3.2 Result arm).")
  (input
    (do
      (def (safe (: n Int64)) (: (if (>= n 0) (Ok n) (Err "neg")) (Result Int64 String)))
      (def (main (: v Int64)) (: (let ((x (try (safe v)))) (Ok (+ x 1))) (Result Int64 String)))
      (export main)))
  (call main (: 5 Int64))
  (output (: (Ok 6) (Result Int64 String)))
  (call main (: -3 Int64))
  (output (: (Err "neg") (Result Int64 String)))
  (live-objects known-leak))

(case
  "a `?`-boundary Option RETURNED to the host carries the value-escape-encode husk (the try-abort reclaim itself is clean)"
  (doc
    "A fallible boundary whose Option result ESCAPES to the host: `main` binds a rope `s`, runs a
           runtime-disc `?`, and RETURNS the `Some`/`None` to the host. v>0 overflows the checked-add →
           `None` short-circuit; v=-100 is in range → `(Some (max-100 + byte-len s))`. Value holds both
           paths, both backends, O0..O2, no double-free. The escape husk here is
           the value-ESCAPE-ENCODE residual of returning an Option to the host (the make / t-encode /
           resource-new escape-emit path), NOT the enclosing binding `s` and NOT a `?`-abort gap: (1) the
           IN-BODY companion below (boundary in a helper, result CONSUMED by main's match, scalar returned,
           no host escape) reclaims to live-objects 0 — the try-abort `MatchSum` reclaim is clean; and (2)
           `s` here const-folds to an immortal constant (both `if` arms are literals), so it never allocates.
           The husk routes to the escape-emit lane (v-cdz escv co-design), the same residual any host-returned
           heap-carrying Option shows. Under drop-before-census (operator 2026-09-19) the host drops the
           escaped owned Option before the census queries live-objects, so `(live-objects 0)`. (Breaker
           probe, re-attributed after v-memory-safety triage.)")
  (input
    (do
      (def
        (main (: v Int64))
        (let ((s (String.concat "live" (if (> v 0) "X" "Y"))))
          (let ((x (try (Int64.checked-add Int64.max v))))
            (Some (+ x (String.byte-len s))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: (None unit) (Option Int64)))
  (call main (: -100 Int64))
  (output (: (Some 9223372036854775712) (Option Int64)))
  (live-objects 0))

(case
  "a `?` short-circuit with a live runtime-rope binding, boundary CONSUMED in-body (no host escape), reclaims to 0"
  (doc
    "The isolating companion that pins the `?`-abort reclaim is CLEAN: `s` is a GENUINE non-immortal
           runtime rope (`rep \"live\" v` builds a v-deep concat rope, off the constant path), and the fallible
           boundary lives in an inner `helper` whose Option result is CONSUMED by `main`'s `match` — `main`
           returns a scalar, so nothing heap escapes to the host and the escape-encode husk of the case above
           is absent. At v=3 the checked-add overflows → the `?` short-circuits `helper` with `s` live in scope,
           and `s` IS reclaimed on the abortive path: live-objects 0. At v=-100 the add is in range → `helper`
           returns `(Some (max-100 + byte-len (rep \"live\" -100)))` = `Some (max-96)`, `main` unwraps to the
           scalar. Together with the host-escape case above this localizes the sole residual to the escape-emit
           lane, not the try-abort lowering. (Adversarial pin from a breaker probe.)")
  (input
    (do
      (def (rep (: s String) (: k Int64)) (if (< k 1) s (rep (String.concat s "x") (- k 1))))
      (def
        (helper (: v Int64))
        (let ((s (rep "live" v)))
          (let ((x (try (Int64.checked-add Int64.max v))))
            (Some (+ x (String.byte-len s))))))
      (def (main (: v Int64)) (match (helper v) ((Some r) r) ((None _u) -1)))
      (export main)))
  (call main (: 3 Int64))
  (output (: -1 Int64))
  (call main (: -100 Int64))
  (output (: 9223372036854775711 Int64))
  (live-objects 0))

; ── T1a gate pins: invariants the constant-fold desugar must hold (all PASS today) ───────────────────
; These pin now-passing behaviors so a future change to the `?` desugar (or the BRICK sequence) cannot
; silently flip them. Added by v-try-operator after adversarially probing the landed BRICK 2a/3a folds.
(case
  "`?` unwraps an Ok payload under a Result boundary (happy path)"
  (doc
    "The Result companion of the Option happy path: `(try (Ok 42))` under a `(Result Int64 Int64)`
           boundary unwraps the `Ok` payload to `42`, and the body's tail `(Ok x)` re-wraps it, so `main`
           = `(Ok 42)`. The result type is annotated so the `Err` type is determined (a bare `(Ok 42)`
           leaves `Err` unsolved — CDZ0203). Pins that the success fold reads the `Ok` disc off a Result
           exactly as it reads `Some` off an Option (`success_disc_of`, by variant NAME).")
  (input (do (def (main) (: (let ((x (try (Ok 42)))) (Ok x)) (Result Int64 Int64))) (export main)))
  (output (: (Ok 42) (Result Int64 Int64))))

(case
  "a success `?` unwraps a RUNTIME payload (static ctor, per-call value)"
  (doc
    "The first RUNTIME-value pin in this file: every other case's `?` operand is a compile-time
           constant, so the whole chain could in principle grade by const-folding alone. Here the operand
           is `(Some a)` for a boundary parameter `a` — the ctor is statically `Some` (so the success fold
           still fires) but the PAYLOAD is per-call: one compiled body must answer `(Some 10)` at a=5 and
           `(Some -14)` at a=-7. A desugar that snapshots the payload at fold time, or that only handles
           literal payloads, answers one of the two wrong. (A runtime-DISC operand — `(try (f a))` where
           `f` picks the variant at run time — remains BRICK 3b todo; this pins the half that works.)")
  (input (do (def (main (: a Int64)) (let ((x (try (Some a)))) (Some (* x 2)))) (export main)))
  (call main (: 5 Int64))
  (output (: (Some 10) (Option Int64)))
  (call main (: -7 Int64))
  (output (: (Some -14) (Option Int64)))
  (live-objects 0))

(case
  "a success `?` unwraps a RUNTIME Ok payload under a Result boundary"
  (doc
    "The Result companion of the runtime-payload pin: `(try (Ok a))` under an annotated
           `(Result Int64 String)` boundary unwraps the per-call `a`, and the tail re-wraps `(+ x 1)`, so
           `main 41` = `(Ok 42)`. Pins that the success fold's payload path is value-polymorphic for
           Result exactly as for Option — the disc is read by NAME at compile time while the payload
           stays a runtime operand.")
  (input
    (do
      (def (main (: a Int64)) (: (let ((x (try (Ok a)))) (Ok (+ x 1))) (Result Int64 String)))
      (export main)))
  (call main (: 41 Int64))
  (output (: (Ok 42) (Result Int64 String)))
  (live-objects 0))

(case
  "two `?`s in one boundary both unwrap (nested happy path)"
  (doc
    "The `parse-pair` shape with constant operands: `(let ((x (try (Some 20)))) (let ((y (try (Some
           22)))) (Some (+ x y))))` — both `?`s see a `Some`, so `x` = 20, `y` = 22, and the boundary
           falls through to `(Some 42)`. Pins that MULTIPLE `?`s under one boundary each unwrap
           independently and the happy path threads through to the body's tail — the nested-match collapse
           the operator asked for.")
  (input
    (do
      (def (main) (let ((x (try (Some 20)))) (let ((y (try (Some 22)))) (Some (+ x y)))))
      (export main)))
  (output (: (Some 42) (Option Int64))))

(case
  "two sequential RUNTIME-DISC `?`s: the FIRST failure short-circuits with ITS OWN error value (first-failure-wins ordering)"
  (doc
    "The failure-ordering companion of the happy-path two-`?` case above, over RUNTIME-DISC operands
           (BRICK 3b's `Core::MatchSum` short-circuit, not a const fold). `safe n` returns `(Ok n)` for n≥0
           and `(Err n)` for n<0; `chain a b` sequences `(let ((x (try (safe a)))) (let ((y (try (safe b))))
           (Ok (+ x y))))`. The failure value carries the FAILING input, so which `?` short-circuited is
           observable: `main` decodes the boundary `Result` to a scalar (`Ok v` → 1000+v, `Err e` → 2000+e).
           Both ok: chain(5,7) = Ok 12 → 1012. First fails: chain(-3,7) short-circuits at the first `?` →
           Err -3 → 1997. Only second fails: chain(5,-7) → Err -7 → 1993. The DECISIVE case is BOTH failing,
           chain(-3,-7): a correct short-circuit stops at the FIRST `?` and propagates Err -3 → 1997, so the
           second `?` on `(safe -7)` never wins — an implementation that evaluated both and returned the LAST
           error, or otherwise mis-ordered, would give 1993. Pins first-failure-wins + that the short-circuit
           precedes the later operand.")
  (input
    (do
      (def (safe (: n Int64)) (: (if (>= n 0) (Ok n) (Err n)) (Result Int64 Int64)))
      (def (chain (: a Int64) (: b Int64))
        (: (let ((x (try (safe a)))) (let ((y (try (safe b)))) (Ok (+ x y)))) (Result Int64 Int64)))
      (def (main (: a Int64) (: b Int64))
        (match (chain a b) ((Ok v) (+ 1000 v)) ((Err e) (+ 2000 e))))
      (export main)))
  (call main (: 5 Int64) (: 7 Int64))
  (output (: 1012 Int64))
  (call main (: -3 Int64) (: 7 Int64))
  (output (: 1997 Int64))
  (call main (: 5 Int64) (: -7 Int64))
  (output (: 1993 Int64))
  (call main (: -3 Int64) (: -7 Int64))
  (output (: 1997 Int64)))

(case
  "`?` unwraps a COMPOUND (tuple) payload"
  (doc
    "`(try (Some (tuple 1 2)))` unwraps the tuple payload whole, so `(Some x)` = `(Some (tuple 1
           2))`. Pins that the payload the `?` binds is not restricted to a scalar — a compound (tuple/
           record/sum) payload flows through the success fold intact, its type preserved
           (`(Option (Tuple Int64 Int64))`).")
  (input (do (def (main) (let ((x (try (Some #tuple(1 2))))) (Some x))) (export main)))
  (output (: (Some #tuple(1 2)) (Option (Tuple Int64 Int64)))))

(case
  "a `?` result is usable mid-body, not only in tail position"
  (doc
    "`(let ((x (try (Some 10)))) (Some (+ x 5)))` — the unwrapped `x` = 10 feeds an arithmetic op
           BEFORE the boundary's tail, giving `(Some 15)`. Pins that `?` UNWRAPS to an ordinary value the
           rest of the body computes with (it is not confined to a tail `(Some …)` re-wrap): the success
           payload is a first-class value in its continuation.")
  (input (do (def (main) (let ((x (try (Some 10)))) (Some (+ x 5)))) (export main)))
  (output (: (Some 15) (Option Int64))))

(case
  "a `?` in a GENERIC function monomorphizes correctly at two element types"
  (doc
    "`(def (wrap v) (let ((x (try (Some v)))) (Some x)))` — a POLYMORPHIC function (unannotated `v`,
           so `wrap : ∀a. a → (Option a)`) whose body wraps `v` in `Some`, `?`s it, and re-wraps. `main`
           calls it at TWO distinct types — `(wrap 7)` : `(Option Int64)` and `(wrap true)` : `(Option
           Bool)` — in one tuple. Pins that the `?` success desugar stays correct under MONOMORPHIZATION:
           each instantiation unwraps + re-wraps its own payload type, giving `(tuple (Some 7) (Some
           true))`. Every other executing case `?`s a monomorphic operand; this is the only one that
           witnesses the desugar surviving two instantiations of one generic body (the recursive-generic
           driver tie is a SEPARATE, parked v-inference concern — this is the plain two-type case).")
  (input
    (do
      (def (wrap v) (let ((x (try (Some v)))) (Some x)))
      (def (main) (: #tuple((wrap 7) (wrap true)) (Tuple (Option Int64) (Option Bool))))
      (export main)))
  (output (: #tuple((Some 7) (Some true)) (Tuple (Option Int64) (Option Bool)))))

(case
  "a constant success `?` folds INLINE in a subexpression with no let-binding"
  (doc
    "`(Some (+ (try (Some 3)) 10))` — the `?` sits DIRECTLY in an argument subexpression, never bound
           by a `let`. This exercises the try-NODE success fold (BRICK 2a, the `Resolved::Try` arm in
           `core_of`) rather than the `lower_let` `try_let_desugar` path every other executing case above
           routes through (each binds `(let ((x (try …))) …)` first). The constant `Some 3` folds to its
           payload `3` in place, so `(+ 3 10)` = 13 and `main` = `(Some 13)`. Pins that a constant success
           `?` unwraps in ANY expression position, not only as a let initializer — the two lowering paths
           agree on the happy path.")
  (input (do (def (main) (Some (+ (try (Some 3)) 10))) (export main)))
  (output (: (Some 13) (Option Int64))))

(case
  "a success `?` on a LET-BOUND variable operand unwraps through the binding"
  (doc
    "`(let ((o (Some 9))) (let ((x (try o))) (Some x)))` — the `?`'s operand is a VARIABLE `o`, not a
           literal `(Some …)` written in place. Every other executing case `?`s a syntactic `SumNew`
           literal (or a call that folds to one); here the constant `Some 9` reaches the `?` only after the
           binding `o` is copy-propagated into the operand, so this pins that the success fold reads the
           disc/payload off the RESOLVED operand core (post-substitution), not off a syntactic literal at
           the `?` site. `o` folds to `(Some 9)`, the `?` unwraps `x` = 9, and `main` = `(Some 9)`.")
  (input (do (def (main) (let ((o (Some 9))) (let ((x (try o))) (Some x)))) (export main)))
  (output (: (Some 9) (Option Int64))))

(case
  "a constant-failure `?` short-circuit ELIDES an earlier trapping let-init whose value it discards"
  (doc
    "OPERATOR §283 RULING (2026-07-16): `we don't emit the trap unless it's reachable; a detected
           unreachable trap is a WARNING.` `(let ((a (/ 1 0)) (x (try (None unit)))) (Some (+ a x)))` — `a`
           traps (÷0) and is referenced only in `(+ a x)`, but `x`'s `?` sees a constant `None` and SHORT-
           CIRCUITS, so `(+ a x)` never runs and `a`'s value is UNOBSERVED (§285 laziness of an unselected
           branch — its value reaches neither the result nor a host call). So the trap is ELIDED, the whole
           expression folds to `(None unit)`, and the ÷0 is a §285 SHOULD-diagnose CDZ0305 WARNING (build
           succeeds), NOT a CDZ0304 reject. (Earlier this pinned CDZ0304 — an over-strict `is_trap_free`
           guard the operator ruling reverted; a host call, being observable, still bails the fold.) This
           keeps the same-let, nested-let, and `if false` shapes CONSISTENT-elide with the landed §283 DCE.")
  (input (do (def (main) (let ((a (/ 1 0)) (x (try (None unit)))) (Some (+ a x)))) (export main)))
  (output (: (None unit) (Option Int64))))

(case
  "a constant-failure `?` in a NESTED let elides a trapping OUTER-let init it discards"
  (doc
    "The nested-let companion (same §283 operator ruling): `(let ((a (/ 1 0))) (let ((x (try (None
           unit)))) (Some (+ a x))))` — `a` is bound in the OUTER let, referenced only in `(+ a x)`, and
           the inner `?` short-circuits before it runs, so `a`'s value is UNOBSERVED. Its ÷0 trap is ELIDED
           (→ `(None unit)`) with a CDZ0305 warning, exactly like the same-let case — observation, not the
           syntactic nesting or evaluation-order, governs (§285). Consistent-elide.")
  (input
    (do (def (main) (let ((a (/ 1 0))) (let ((x (try (None unit)))) (Some (+ a x))))) (export main)))
  (output (: (None unit) (Option Int64))))

; The §283 elision above applies ONLY to an UNOBSERVED trapping init (the `?` short-circuits before its
; value is used). The NEGATIVE boundary: when the trapping init's value IS observed, the trap still FIRES —
; it is not silently dropped. These pin both observation shapes: a SUCCESS `?` (no short-circuit, so the
; init is used in the result) and an init observed IN the `?`'s own operand (used before the short-circuit
; could even happen). Both keep the provable ÷0 as a CDZ0304 reject, guarding that the elision does not
; over-reach into observable traps (the observable-trap-is-preserved axis, sibling of the trap-ordering rule).
(case
  "a success `?` observes an earlier trapping let-init so the trap is not elided"
  (doc
    "The negative boundary of the §283 elision: `(let ((a (/ 1 0)) (x (try (Some 5)))) (Some (+ a x)))`
           — the `?` sees a `Some` and does NOT short-circuit, so `(+ a x)` runs and `a`'s ÷0 IS observed. The
           trap is therefore NOT elided: the provable ÷0 is a CDZ0304 reject, exactly as it is without any
           `?`. Pins that the elision fires only when the `?` short-circuits AWAY from the trapping value —
           a SUCCESS `?` leaves the value observed, so the trap stands. The complement of the elide cases
           above (which all short-circuit on a constant None).")
  (input (do (def (main) (let ((a (/ 1 0)) (x (try (Some 5)))) (Some (+ a x)))) (export main)))
  (error CDZ0304))

(case
  "a trapping init observed inside the `?`'s own operand is not elided"
  (doc
    "`(let ((a (/ 1 0)) (x (try (Some a)))) (Some x))` — `a`'s value flows INTO the `?`'s operand
           `(Some a)`, so it is observed BEFORE the short-circuit could occur (the operand must be built to
           be matched). The ÷0 is observed regardless of the `?`'s success/failure, so the trap is not
           elided: CDZ0304. Pins that a value consumed by the `?` operand itself is observed, distinct from
           a value used only in a body the `?` may short-circuit past.")
  (input (do (def (main) (let ((a (/ 1 0)) (x (try (Some a)))) (Some x))) (export main)))
  (error CDZ0304))

(case
  "a `?` in a CALLED (inlined, non-exported) helper finds its boundary"
  (doc
    "Regression pin: `(def (f) (let ((x (try (Some 7)))) (Some (+ x 3))))` is CALLED by `main` (only
           `main` is exported), so `f` is INLINED at the call site. `f`'s result type IS `Option`, so the
           `?` is well-formed — but a bug made the boundary walk (`enclosing_boundary_ty`) fall off the
           inlined COPY's re-parented tree and FALSELY reject CDZ0230 (`no fallible boundary`). The boundary
           walk is now INCONCLUSIVE when it falls off a re-parented copy (raises nothing); the genuine
           non-fallible-boundary reject still fires from the original body's walk. `f` = `(Some 10)`, so
           `main` = `(Some 10)`. Pins that a `?` in a called helper compiles, not spuriously rejects.")
  (input (do (def (f) (let ((x (try (Some 7)))) (Some (+ x 3)))) (def (main) (f)) (export main)))
  (output (: (Some 10) (Option Int64))))

; ── T1a gate pins (round 2): the short-circuit SKIPS subsequent work + `?` under if/match ────────────
; Added by v-try-operator after adversarial probing of the BRICK 3a constant-failure fold. All PASS.
(case
  "a failure `?` short-circuits BEFORE later computation runs"
  (doc
    "`(let ((x (try (None unit)))) (let ((y 100)) (Some (+ x y))))` — the FIRST binding's `?` sees a
           `None`, so the `let` short-circuits to `None` and the inner `let` + `(+ x y)` NEVER run. Pins
           that the short-circuit abandons the continuation (not just unwraps): the `(+ x y)` using the
           unbound-on-failure `x` is skipped, so the result is `(None unit)`, never a use of a missing
           payload.")
  (input
    (do (def (main) (let ((x (try (None unit)))) (let ((y 100)) (Some (+ x y))))) (export main)))
  (output (: (None unit) (Option Int64))))

(case
  "the first failing `?` short-circuits; a later `?` never runs"
  (doc
    "`(let ((x (try (None unit)))) (let ((y (try (Some 7)))) (Some (+ x y))))` — the FIRST `?`
           fails, so the boundary short-circuits to `None` and the SECOND `?` (`(try (Some 7))`) is never
           evaluated. Pins left-to-right short-circuit order across multiple `?`s: the first failure wins.")
  (input
    (do
      (def (main) (let ((x (try (None unit)))) (let ((y (try (Some 7)))) (Some (+ x y)))))
      (export main)))
  (output (: (None unit) (Option Int64))))

(case
  "a `?` inside an if-branch resolves against the enclosing function boundary"
  (doc
    "`(if true (let ((x (try (Some 5)))) (Some (+ x 1))) (None unit))` — the `?` in the THEN branch
           finds its boundary through the enclosing `if` up to `main`'s `Option` result (the if's branches
           are both `Option`). The taken branch unwraps `x` = 5 → `(Some 6)`. Pins that a `?` nested in a
           conditional still resolves the enclosing function as its boundary.")
  (input
    (do (def (main) (if true (let ((x (try (Some 5)))) (Some (+ x 1))) (None unit))) (export main)))
  (output (: (Some 6) (Option Int64))))

(case
  "a `?` inside a match-arm resolves against the enclosing function boundary"
  (doc
    "`(match 0 (0 (let ((x (try (Some 9)))) (Some x))) (_ (None unit)))` — the `?` in the first arm
           finds `main`'s `Option` boundary through the enclosing `match`. Arm 0 is selected, `x` = 9 →
           `(Some 9)`. The match-arm companion of the if-branch case.")
  (input
    (do
      (def (main) (match 0 (0 (let ((x (try (Some 9)))) (Some x))) (_ (None unit))))
      (export main)))
  (output (: (Some 9) (Option Int64))))

(case
  "runtime-payload `?`s in BOTH arms of a RUNTIME-scrutinee match each resolve the boundary"
  (doc
    "The runtime upgrade of the match-arm case above (const scrutinee, const payload, one arm with a
           `?`): the scrutinee is the boundary parameter AND both arms carry their own runtime-payload
           `?` — n=0 takes arm 0 (`(try (Some n))` unwraps 0 → `(Some 100)`), n=21 falls to the default
           arm (`(try (Some (* n 2)))` unwraps 42). One compiled body, two live desugared `?`s in
           different arms, arm selection at run time — a desugar that wired both `?`s to one arm's
           continuation (or resolved the boundary only for the first arm) breaks one call.")
  (input
    (do
      (def
        (main (: n Int64))
        (match
          n
          (0 (let ((x (try (Some n)))) (Some (+ x 100))))
          (_ (let ((y (try (Some (* n 2))))) (Some y)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: (Some 100) (Option Int64)))
  (call main (: 21 Int64))
  (output (: (Some 42) (Option Int64)))
  (live-objects 0))

(case
  "runtime-payload `?`s in BOTH branches of a runtime if take their own continuations"
  (doc
    "The if twin: each branch has its own `?` and its own continuation (`+1` vs `-1`), the branch
           picked by a runtime Bool. true at 41 → 42 through the then-`?`; false at 43 → 42 through the
           else-`?`. Pins per-branch desugar independence under a runtime condition (the const if-branch
           case above always takes `true`).")
  (input
    (do
      (def
        (main (: b Bool) (: n Int64))
        (if b (let ((x (try (Some n)))) (Some (+ x 1))) (let ((y (try (Some n)))) (Some (- y 1)))))
      (export main)))
  (call main (: true Bool) (: 41 Int64))
  (output (: (Some 42) (Option Int64)))
  (call main (: false Bool) (: 43 Int64))
  (output (: (Some 42) (Option Int64)))
  (live-objects 0))

(case
  "a runtime-payload `?` result feeds a SECOND runtime-payload `?` in sequence"
  (doc
    "The chained-data face: the first `?` unwraps the runtime `n`, and the SECOND `?`'s operand is
           built FROM that result (`(Some (+ x 2))`) — the unwrapped value flows across the desugared
           seam into the next `?`'s operand, then out (19 → 21 → 42). The two-`?` const case pins
           independent unwraps; this pins the DATA DEPENDENCE between them over a runtime value.")
  (input
    (do
      (def
        (main (: n Int64))
        (let ((x (try (Some n)))) (let ((y (try (Some (+ x 2))))) (Some (* y 2)))))
      (export main)))
  (call main (: 19 Int64))
  (output (: (Some 42) (Option Int64)))
  (live-objects 0))

(case
  "a `?` in an anonymous LAMBDA body resolves the lambda as its boundary (happy path)"
  (doc
    "`((fn () (let ((x (try (Some 7)))) (Some (+ x 1)))))` — the `?` sits inside an IMMEDIATELY-APPLIED
           anonymous `(fn () …)`, not a named `def`. A `?` short-circuits the enclosing FUNCTION's fallible
           result, and a lambda IS a function (`enclosing_boundary_ty` walks to a `(fn params body)` body,
           not only a `def` body — DESIGN-try-operator-rcdzc.md §4). The lambda's result infers `(Option
           Int64)`, so the `?` unwraps `x` = 7 and the lambda (hence `main`) returns `(Some 8)`. Pins that
           the boundary walk resolves an ANONYMOUS-lambda boundary, the executing companion of the
           def-boundary cases above.")
  (input (do (def (main) ((fn () (let ((x (try (Some 7)))) (Some (+ x 1)))))) (export main)))
  (output (: (Some 8) (Option Int64))))

(case
  "a failure `?` short-circuits the enclosing LAMBDA, not the outer function"
  (doc
    "The short-circuit companion of the lambda-boundary case: `((fn () (let ((x (try (None unit))))
           (Some x))))` — the `?` sees a constant `None`, so it BREAKS the enclosing `(fn () …)` boundary
           (the NEAREST enclosing function body), making the lambda's value `(None unit)`; `main` returns
           that. Pins that a `?`'s short-circuit targets the innermost enclosing lambda boundary, exactly as
           it targets a def boundary — the abortive path of the anonymous-lambda boundary.")
  (input
    (do
      (def (main) (: ((fn () (let ((x (try (None unit)))) (Some x)))) (Option Int64)))
      (export main)))
  (output (: (None unit) (Option Int64))))

; --- The strict spine around a short-circuiting `?`: effects, ordering, and the cut point ----------
; The trapping-earlier-init pin above grades the compile-provable face (CDZ0304). These grade the
; RUNTIME spine: an effectful init BEFORE a failing `?` is observed (performs exactly once), a
; success-`?` then a failure-`?` cuts at the second, and an init AFTER the failing `?` — including a
; provably-trapping one — never evaluates (the short-circuit is the spine's cut point; only earlier
; inits are observed). Promoted from passing breaker probes.
(case
  "an effectful init before a failing `?` performs exactly once"
  (doc
    "`(let ((a (Ctr.tick)) (x (try (None unit)))) (Some (+ a x)))` under a counter handler —
           the tick sits on the strict spine BEFORE the `?`, so it performs (state advances 0→1)
           and THEN the boundary short-circuits to None (→ -1); the trailing `(Ctr.tick)` reads 1 →
           0. A fold that discarded the earlier effectful init answers 1·(-1) + 0 = -1; one that
           duplicated it answers 1. The runtime-effect companion of the trapping-earlier-init
           CDZ0304 pin.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (opt) (let ((a (Ctr.tick unit)) (x (try (None unit)))) (Some (+ a x))))
      (def
        (main)
        (handle
          Ctr
          0
          ((tick (_) s (resume s (+ s 1))))
          (+ (match (opt) ((Some v) v) ((None _) -1)) (Ctr.tick unit))))
      (export main)))
  (output (: 0 Int64)))

(case
  "a perform BETWEEN two `?`s advances state that survives the second's cut"
  (doc
    "The effectful-init pin performs BEFORE the first ?; this performs BETWEEN a succeeding ?
           and a failing one — the state advance from the mid-spine perform must SURVIVE the second
           ?'s cut (the abortive try must not roll back the effect discipline's state; the trailing
           tick reads 1 → -9).")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (opt)
        (:
          (let
            ((x (try (Some 10))))
            (let ((a (Ctr.tick unit))) (let ((y (try (None unit)))) (Some (+ x (+ a y))))))
          (Option Int64)))
      (def
        (main)
        (handle
          Ctr
          0
          ((tick (_u) s (resume s (+ s 1))))
          (+ (* (match (opt) ((Some v) v) ((None _u) -1)) 10) (Ctr.tick unit))))
      (export main)))
  (output (: -9 Int64)))

(case
  "a success `?` then a failure `?` short-circuits at the second"
  (doc
    "`x = (try (Some 1))` unwraps (the happy path); the SECOND `?` sees None and cuts the
           boundary → the caller matches None → -1. Pins the chain semantics: each `?` is its own cut
           point, and a successful unwrap does not immunize the rest of the body (nor does the
           short-circuit rewind the already-bound x).")
  (input
    (do
      (def (opt) (let ((x (try (Some 1)))) (let ((y (try (None unit)))) (Some (+ x y)))))
      (def (main) (match (opt) ((Some v) v) ((None _) -1)))
      (export main)))
  (output (: -1 Int64)))

(case
  "an init after the failing `?` never evaluates — even a provably-trapping one"
  (doc
    "`(let ((x (try (None unit)))) (let ((y (/ 1 0))) …))` — the `?` cuts the spine FIRST, so
           the later `(/ 1 0)` init is genuinely unreachable: the function yields None → -1, no trap
           and no CDZ0304 (contrast the EARLIER-init pin above, where the same ÷0 before the `?` must
           fail the build). Together the two pins locate the cut point exactly: earlier inits are
           observed, later inits are dead.")
  (input
    (do
      (def (opt) (let ((x (try (None unit)))) (let ((y (/ 1 0))) (Some (+ x y)))))
      (def (main) (match (opt) ((Some v) v) ((None _) -1)))
      (export main)))
  (output (: -1 Int64)))

(case
  "a `?` whose Result error type disagrees with the boundary's is rejected"
  (doc
    "SOUNDNESS pin: `(def (main) (: (let ((y (try (Err true)))) (Ok y)) (Result Int64 Int64)))` —
           the `?`'s operand `(Err true)` is a `Result _ Bool` (error type `Bool`), but the enclosing
           function's declared error type is `Int64`. A `?` short-circuits by passing its `Err` OUT
           UNCHANGED as the boundary value, so the error types MUST agree (§5: the error type unifies with
           the boundary's; Cadenza has no automatic error conversion). Without this check the `Bool` `true`
           escaped as a claimed `Int64` error — a soundness hole (the ordinary `(: (Err true) (Result Int64
           Int64))` annotation path already rejects it). CDZ0203.")
  (input
    (do (def (main) (: (let ((y (try (Err true)))) (Ok y)) (Result Int64 Int64))) (export main)))
  (error CDZ0203 (message "error type")))

(case
  "an agreeing Result error type short-circuits through the boundary"
  (doc
    "The positive control of the error-type soundness reject: `(try (Err 7))` under a
           `(Result Int64 Int64)` boundary — the error type AGREES, so the `?` short-circuits and
           the caller's Err arm reads 7. Pinned beside the disagreeing-type CDZ0203 so the check is
           graded from both sides (an over-tight fix that rejected agreeing error types breaks this).")
  (input
    (do
      (def (f) (: (let ((y (try (Err 7)))) (Ok y)) (Result Int64 Int64)))
      (def (main) (match (f) ((Ok v) v) ((Err e) e)))
      (export main)))
  (output (: 7 Int64)))

(case
  "a failure `?` short-circuits a RUNTIME error payload (static Err ctor, per-call value)"
  (doc
    "The failure-side complement of the runtime-payload SUCCESS pins above: those unwrap `(try (Some
           a))`/`(try (Ok a))` for a per-call `a`; this short-circuits `(try (Err a))` where the error
           payload is a boundary parameter. The ctor is statically `Err` (so the constant-failure fold still
           selects the short-circuit arm) but the PAYLOAD is per-call — one compiled body must flow `Err 7`
           out at a=7 and `Err -3` at a=-3, each read by the caller's `Err` arm. A desugar that snapshots the
           error payload at fold time answers one of the two wrong. Pins that the abortive break carries the
           runtime error VALUE unchanged, the failure analogue of the success runtime-payload path.")
  (input
    (do
      (def (f (: a Int64)) (: (let ((y (try (Err a)))) (Ok y)) (Result Int64 Int64)))
      (def (main (: a Int64)) (match (f a) ((Ok v) v) ((Err e) e)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: -3 Int64))
  (output (: -3 Int64)))

(case
  "a failure `?` preserves a COMPOUND error value through the short-circuit"
  (doc
    "The Err-value case above carries a scalar 7; this carries a COMPOUND: `(try (Err (tuple 3 4)))`
           under a `(Result Int64 (Tuple Int64 Int64))` boundary short-circuits, and the caller's Err arm
           destructures the tuple `(3, 4)` → 7. Pins that `?` propagates the WHOLE error payload (a compound,
           not just a scalar) unchanged through the abortive short-circuit — the error analogue of the
           compound-Ok-payload unwrap.")
  (input
    (do
      (def (f) (: (let ((y (try (Err #tuple(3 4))))) (Ok y)) (Result Int64 (Tuple Int64 Int64))))
      (def (main) (match (f) ((Ok v) 0) ((Err #tuple(a b)) (+ a b))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a failure `?` preserves a USER-SUM error value (a ctor + payload) through the short-circuit"
  (doc
    "The compound-Err case above carries a built-in `tuple`; this carries a USER-DECLARED SUM error
           type: `(type MyErr (Code Int64) (Bad))`, and `(try (Err (Code 404)))` under a `(Result Int64
           MyErr)` boundary short-circuits, flowing the `Err (Code 404)` out as the boundary value UNCHANGED
           — the `Code 404` ctor + its `Int64` payload survive the abortive break intact. Pins that the `?`
           failure short-circuit is transparent to a user-declared sum error carrying a payload (not only a
           built-in tuple/scalar Err), witnessing that the boundary value is the operand's `Err` verbatim
           regardless of the error type's shape (DESIGN-try-operator-rcdzc.md §5 — `?` passes its `Err` out
           unchanged, no coercion).")
  (input
    (do
      (type MyErr (Code Int64) (Bad))
      (def (main) (: (let ((x (try (Err (Code 404))))) (Ok x)) (Result Int64 MyErr)))
      (export main)))
  (output (: (Err (Code 404)) (Result Int64 MyErr))))

(case
  "a failure `?` propagates through TWO nested fallible boundaries"
  (doc
    "An Err bubbles up MORE than one `?` boundary: `inner` returns `Err 9`; `outer`'s `(try (inner))`
           short-circuits and re-propagates the Err to its OWN boundary; `main` reads 9. Pins that the
           abortive `?` composes across nested fallible functions — the failure exits inner's boundary,
           becomes outer's `(try …)` operand, and short-circuits outer's boundary too, carrying 9 the whole
           way. A `?` that only exited one level (or lost the value across the second boundary) would give a
           wrong result. (The single-boundary short-circuit + the same-boundary two-`?` cases are pinned
           above; this is the cross-boundary composition.)")
  (input
    (do
      (def (inner) (: (let ((y (try (Err 9)))) (Ok y)) (Result Int64 Int64)))
      (def (outer) (: (let ((z (try (inner)))) (Ok (+ z 1))) (Result Int64 Int64)))
      (def (main) (match (outer) ((Ok v) v) ((Err e) e)))
      (export main)))
  (output (: 9 Int64)))

(case
  "effect state threads across a successful `?`"
  (doc
    "The straddle: `(let ((a (Ctr.tick)) (x (try (Some 0))) (b (Ctr.tick))) (Some (+ a b)))` — a
           perform BEFORE the `?` (a = 0), a SUCCESSFUL `?` unwrapping Some 0 (no short-circuit), then
           a perform AFTER (b = 1) → Some 1, and the trailing tick reads 2 → 3. The complement of the
           effectful-init-before-a-FAILING-? pin: on the success path the `?` is transparent to the
           effect spine, so the counter advances 0→1→2 straight through it. A `?` that reset or
           re-entered the handler on the success path would skew b or the trailing read.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (opt)
        (let
          ((a (Ctr.tick unit)) (x (try (Option.Some 0))) (b (Ctr.tick unit)))
          (Option.Some (+ a b))))
      (def
        (main)
        (handle
          Ctr
          0
          ((tick (_) s (resume s (+ s 1))))
          (+ (match (opt) ((Some v) v) ((None _) -1)) (Ctr.tick unit))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a fallible helper with a runtime-payload `?` is called from INSIDE a handle body"
  (doc
    "The effects-composition of the runtime-payload `?`: `run` (a fallible Option-boundary def
           whose `?` unwraps the boundary parameter) is called from within a `handle` body, its result
           matched beside a perform — 0 + (5+100) = 105 per call. The `?` desugar's Core::Block boundary
           lives INSIDE the handler's context; the two abortive machineries (the `?` break and the
           handler dispatch) must nest without confusing their exit paths. (The effect-state straddle pin
           above uses const `?` operands inline in the handled body; this is the helper-boundary +
           runtime-payload face.)")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def (run (: n Int64)) (let ((x (try (Some n)))) (Some (+ x 100))))
      (def
        (main (: n Int64))
        (handle
          Ctr
          0
          ((next (u) s (resume s (+ s 1))))
          (+ (Ctr.next unit) (match (run n) ((Some v) v) ((None u) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64)))

(case
  "chained `?`s unwrap three List.at reads over a sufficient list"
  (doc
    "The collection-lookup chain (the checked-add pins use arithmetic Options): three `(try
           (List.at xs i))` reads in sequence over a 3-element list all unwrap — 10+2+30 = 42. The
           safe-multi-index idiom: `List.at`'s totality-as-Option composes with the `?` desugar so
           indexed reads chain without per-read match ladders. (The short-circuit twin below drives the
           SAME helper over a 2-element list; two calls to one fallible helper in one body still
           decline, so the faces pin separately.)")
  (input
    (do
      (def
        (get3 (: xs (List Int64)))
        (let
          ((a (try (List.at xs 0))))
          (let ((b (try (List.at xs 1)))) (let ((c (try (List.at xs 2)))) (Some (+ a (+ b c)))))))
      (def (main (: n Int64)) (match (get3 #list(n 2 30)) ((Some v) v) ((None u) -1)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 42 Int64)))

(case
  "the third `?` List.at read SHORT-CIRCUITS over a two-element list"
  (doc
    "The short-circuit twin: the same three-read chain over a 2-ELEMENT list — reads 0 and 1
           unwrap, read 2's None exits the boundary before the sum (-1, not a defaulted partial). Proves
           the third read drives the exit; a chain that defaulted a missing element to 0 would answer
           n+2.")
  (input
    (do
      (def
        (get3 (: xs (List Int64)))
        (let
          ((a (try (List.at xs 0))))
          (let ((b (try (List.at xs 1)))) (let ((c (try (List.at xs 2)))) (Some (+ a (+ b c)))))))
      (def (main (: n Int64)) (match (get3 #list(n 2)) ((Some v) v) ((None u) -1)))
      (export main)))
  (call main (: 10 Int64))
  (output (: -1 Int64)))

(case
  "a `?` on an ill-typed operand reports the operand's error, not a `?`-shape cascade"
  (doc
    "`(let ((x (try (+ 1 2.0)))) (Some x))` — the operand `(+ 1 2.0)` is itself ill-typed (a numeric
           mismatch, CDZ0301). The `?`-operand-shape check must NOT pile a confusing `?` operand must be a
           fallible Result/Option, found Float64` on top: the operand's own fault is the primary `no`. Pins
           that the `?` collect arm collects the operand's faults FIRST and suppresses its shape/boundary
           checks when the operand already carries a coded fault (the `Member`-arm operand-is-poison
           discipline). Grades on CDZ0301 — the operand mismatch — not the suppressed `?` cascade;
           `(no-other-errors)` pins that NO second coded fault (in particular no CDZ0203 `?`-shape reject)
           accompanies it. (Enhanced from rcdzc
           try_on_an_ill_typed_operand_reports_the_operand_error_not_a_fallible_cascade.)")
  (input (do (def (main) (let ((x (try (+ 1 2.0)))) (Some x))) (export main)))
  (error CDZ0301 (no-other-errors)))

(case
  "a runtime-DISC `?` inside a stored closure applies per-call"
  (doc
    "A runtime-DISC `?` inside a STORED closure applied twice, under the closure's OWN Option
           boundary: `(try (find m q))` — the operand's VARIANT is decided at run time by the lookup.
           f(1) unwraps 10 (+k), f(9) short-circuits the CLOSURE (not the outer def) to None — 1499 at
           k=5, 999 at k=0. Computes correctly WITHOUT a distinct block-br emit: the closure's inner
           `(do (def v (try …)) …)` is rewritten to `(let ((v (try …))) …)` by the try-do-def desugar,
           so the runtime-disc `?` rides the inline-robust BRICK-3b `lower_let` `Core::MatchSum`
           short-circuit; and the `?`'s boundary resolves to the CLOSURE's `(Option Int64)` result (the
           `fn`-body arm of `enclosing_boundary_ty`) rather than mis-walking past the `fn` to the outer
           def — the false-CDZ0230 that had been the sole thing blocking this shape. Pins the
           closure-boundary + double-application composition in one case.")
  (input
    (do
      (def (find (: m (Map Int64 Int64)) (: k Int64)) (Map.lookup m k))
      (def
        (main (: k Int64))
        (do
          (def m (Map.insert (Map.insert Map.empty 1 10) 2 20))
          (def f (fn ((: q Int64)) (do (def v (try (find m q))) (Some (+ v k)))))
          (+
            (* 100 (match (f 1) ((Some v) v) ((None _u) -1)))
            (match (f 9) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1499 Int64))
  (call main (: 0 Int64))
  (output (: 999 Int64)))

(case
  "tryh1 fifty try-unwraps of a HEAP (list) Ok payload under a Result boundary reclaim to zero"
  (doc
    "The census face of the success `?`/try fold on a HEAP payload: the existing runtime-payload
           pins use scalar Ok payloads; this unwraps a runtime-built list per frame, reads its length, and
           re-wraps — fifty frames leave NO live cell (the unwrapped list is consumed by List.len, the
           re-wrap is a fresh scalar Result). The runtime-Err short-circuit face stays BRICK-3b-gated
           (operator-owned, v-try-operator) — this pins only the success-path heap reclaim.")
  (input
    (do
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def
        (step (: k Int64))
        (: (let ((xs (try (Ok (bld 3))))) (Ok (List.len xs))) (Result Int64 String)))
      (def
        (frames (: k Int64))
        (if (= k 0) 0 (+ (match (step k) ((Ok v) v) ((Err e) -1)) (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 150 Int64))
  (live-objects 0))

(case
  "tdd1 a runtime-operand `?` under the do-def binding idiom computes like its let twin (should-work: both boundary-tail idioms are the documented form)"
  (doc
    "A runtime-operand `?` under the do-def idiom `(do (def x (try e)) body)` computes exactly like its
     `(let ((x (try e))) body)` twin — this file's header documents BOTH as the landed boundary-tail idiom.
     Here `quarter` is INLINED into `main`, which is the path that regressed: the do-def form is fragile
     under inlining (a do-local reference misresolves through the nested β-copy, missing the copied `try`'s
     BRICK-3b `core_override`), so the copied MatchSum's success arm re-declined CDZ0900 — and un-declining
     it naively MISCOMPILED the failure leg (n=5 gave 0, not -1). FIXED by the load-time `try_desugar` pass
     (`rcdzc/src/try_desugar.rs`): a two-form `(do (def x (try e)) body)` is rewritten to `(let ((x (try
     e))) body)` before resolution, so the runtime-`?` uses the inline-ROBUST `lower_let` short-circuit path
     (identical MatchSum to the `let` twin) whether inlined or not. This case now COMPUTES (not a fence) and
     GUARDS the inline path against regressing back to the CDZ0900 decline or the 0-not-`-1` miscompile.
     Derivation (matching the `let` twin): half halves evens else None; quarter chains two halvings through
     `?`: n=8 -> 2 (unwrap twice), n=6 -> -1 (3 is odd: inner fail), n=5 -> -1 (short-circuit at the first
     `?`).")
  (input
    (do
      (def (half (: k Int64)) (if (= (% k 2) 0) (Some (/ k 2)) (None)))
      (def (quarter (: k Int64)) (do (def h (try (half k))) (half h)))
      (def (main (: n Int64)) (match (quarter n) ((Some v) v) ((None _u) -1)))
      (export main)))
  (call main (: 8 Int64))
  (output (: 2 Int64))
  (call main (: 6 Int64))
  (output (: -1 Int64))
  (call main (: 5 Int64))
  (output (: -1 Int64)))

(case
  "trr1 a runtime-Result `?` in a fallible-boundary fn unwraps Ok and PROPAGATES Err"
  (doc
    "A `?`/`try` over a RUNTIME-selected `Result` operand in EXPRESSION position — `step`'s param `r` fed a
     runtime `(if … (Ok 41) (Err …))` — UNWRAPS on the Ok leg (42 = 41+1, re-wrapped through the `(Result …)`
     boundary) and PROPAGATES on the Err leg (`(try r)` short-circuits `step` to the Err, which the outer
     match maps to -1). The `?` sits inside `(Ok (+ 1 •))`, not a binding tail, so it computes via the BRICK-3
     expression-position hoist (`try_desugar::desugar_try_expr_position`): `(Ok (+ 1 (try r)))` is rewritten
     to `(let ((x (try r))) (Ok (+ 1 x)))`, which then rides the SAME inline-safe `lower_let` runtime-`?`
     `Core::MatchSum` short-circuit as the do-def / stored-closure cases (NO `Core::Block`/`Break`, no
     non-local jump — inline-safe where an emit-time block wrap is not, since `step` inlines into `main`).
     Verified tri-path leak-clean (live-objects 0 on both the Ok and the String-carrying Err leg). The
     CONST-arg twin `(step (Ok 41))` folds 42 (the fold path is unaffected).")
  (input
    (do
      (def (step (: r (Result Int64 String))) (: (Ok (+ 1 (try r))) (Result Int64 String)))
      (def
        (main (: k Int64))
        (match (step (if (> k 0) (Ok 41) (Err "nope"))) ((Ok v) v) ((Err _s) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64)))

(case
  "trr2 an expression-position `?` in a TAIL if-arm hoists into the arm and short-circuits"
  (doc
    "The control-flow face of the expression-position `?` (BRICK 3 slice 2a): the `?` sits inside the ELSE
     arm of a tail `if` — `(if b (Ok 100) (Ok (+ 1 (try r))))` — not a binding tail and not the whole body.
     The hoist descends TAIL positions (both `if` arms are tail) and rewrites the containing arm to
     `(let ((x (try r))) (Ok (+ 1 x)))`, which rides the same inline-safe `lower_let` runtime-`?`
     `Core::MatchSum` short-circuit — the `?`'s boundary is the fn's `(Result …)` result even though it is
     nested in an arm. b=true takes the `(Ok 100)` arm (100, the `?` never evaluated); b=false unwraps Ok
     (42 = 41+1) or PROPAGATES Err (short-circuits `step` to the Err, mapped to -1). Verified leak-clean
     (live-objects 0 on every path, incl. the String-carrying Err leg). The condition `b` is NOT a tail
     position, so a `?` in the condition would stay a clean CDZ0900 decline (a later slice).")
  (input
    (do
      (def
        (step (: r (Result Int64 String)) (: b Bool))
        (: (if b (Ok 100) (Ok (+ 1 (try r)))) (Result Int64 String)))
      (def
        (main (: k Int64))
        (match (step (if (> k 0) (Ok 41) (Err "no")) (= k 2)) ((Ok v) v) ((Err _s) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (call main (: 2 Int64))
  (output (: 100 Int64)))

(case
  "trl1 expression-position `?`s as `#list` elements hoist left-to-right and short-circuit"
  (doc
    "The compound-constructor face of the expression-position `?` (BRICK 3 slice 2c): two `?`s appear as
     ELEMENTS of a `#list` — `(Ok #list((try r) (try s)))` — under a `(Result (List Int64) String)` boundary.
     A `#list` head is a `Leaf::Ctor`, not a name, so the hoist descends it as a pure container (elements
     evaluate left-to-right) and, by fixpoint, lifts BOTH `?`s to nested boundary `let`s in order —
     `(let ((a (try r))) (let ((b (try s))) (Ok #list(a b))))` — each riding the inline-safe `lower_let`
     short-circuit. Both Ok → the two-element list (len 2); the FIRST Err short-circuits `mk` to that Err
     (mapped to -1), leaving the second `?` unevaluated. Verified leak-clean (live-objects 0 every path).")
  (input
    (do
      (def
        (mk (: r (Result Int64 String)) (: s (Result Int64 String)))
        (: (Ok #list((try r) (try s))) (Result (List Int64) String)))
      (def
        (main (: k Int64))
        (match
          (mk (if (> k 0) (Ok 7) (Err "a")) (if (> k 1) (Ok 9) (Err "b")))
          ((Ok xs) (List.len xs))
          ((Err _e) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  ; reclaim lock (v-memory-safety): the `(Ok #list(a b))` heap value built via the hoisted `?` boundary lets
  ; must reclaim on every path -- the both-Ok path (list built + List.len) and the Err short-circuits (list never built, Ok payloads unwind). Enforces the doc's "leak-clean (live-objects 0 every
  ; path)" (was value-only). BRICK-3 compound-ctor `?` reclaim family.
  (live-objects 0))

(case
  "trt1 expression-position `?`s as `#tuple` elements hoist left-to-right and short-circuit"
  (doc
    "The `#tuple` sibling of trl1 (BRICK 3 slice 2c compound-constructor descent): two `?`s appear as
     ELEMENTS of a `#tuple` — `(Ok #tuple((try r) (try s)))` — under a `(Result (Tuple Int64 Int64) String)`
     boundary. Like `#list`, a `#tuple` head is a `Leaf::Ctor` (not a name), so the hoist descends it as a
     pure container whose elements evaluate left-to-right and, by fixpoint, lifts BOTH `?`s to nested
     boundary `let`s in order — `(let ((a (try r))) (let ((b (try s))) (Ok #tuple(a b))))` — each riding the
     inline-safe `lower_let` short-circuit. Both Ok → the tuple `(7 9)`, summed to 16; the FIRST failing `?`
     short-circuits `mk` to its Err (mapped to -1), leaving the later `?` unevaluated (k=1: second Err; k=0:
     first Err). Verified leak-clean (live-objects 0 every path). Pins the Tuple arm of the compound-ctor
     descent beside the List arm (trl1) and the Record arm (trr3); a `map` element `?` remains a clean
     CDZ0900 decline (its `(= k v)` entries evaluate the KEY too, needing per-entry key-then-value ordering —
     a deliberate later slice).")
  (input
    (do
      (def
        (mk (: r (Result Int64 String)) (: s (Result Int64 String)))
        (: (Ok #tuple((try r) (try s))) (Result (Tuple Int64 Int64) String)))
      (def
        (main (: k Int64))
        (match
          (mk (if (> k 0) (Ok 7) (Err "a")) (if (> k 1) (Ok 9) (Err "b")))
          ((Ok t) (+ (. t 0) (. t 1)))
          ((Err _e) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 16 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  ; reclaim lock (v-memory-safety): the `(Ok #tuple(a b))` heap value built via the hoisted `?` boundary lets
  ; must reclaim on every path -- the both-Ok path (tuple built + projected) and the Err short-circuits (tuple never built, payloads unwind). Enforces the doc's "leak-clean (live-objects 0 every
  ; path)" (was value-only). BRICK-3 compound-ctor `?` reclaim family.
  (live-objects 0))

(case
  "trr3 expression-position `?`s as `#record` FIELD values hoist left-to-right and short-circuit"
  (doc
    "The `#record` sibling of trl1/trt1 (BRICK 3 compound-constructor descent, extended to records): two
     `?`s appear as FIELD VALUES of a `#record` — `(Ok #record((= a (try r)) (= b (try s))))` — under a
     `(Result (Record (: a Int64) (: b Int64)) String)` boundary. A `#record`'s kids are `(= field value)`
     PAIRS: the field name is STATIC (not evaluated), the value is the evaluated sub-expression, so the hoist
     descends into each pair's VALUE (never treating a `(= k v)` pair as an operand) and, by fixpoint, lifts
     BOTH `?`s to nested boundary `let`s in field order — `(let ((a (try r))) (let ((b (try s))) (Ok
     #record((= a a) (= b b)))))` — each riding the inline-safe `lower_let` short-circuit. Both Ok → the
     record `{a=7, b=9}`, summed to 16; the FIRST failing `?` short-circuits `mk` to its Err (mapped to -1),
     leaving the later `?` unevaluated (k=1: second Err; k=0: first Err). An IMPURE earlier field value is
     bound as a prefix `let` ahead of the `?` so field-evaluation order is preserved. Verified leak-clean
     (live-objects 0 every path). Completes the compound-ctor element `?` family (list/tuple/set/record); a
     `map` field `?` stays a clean CDZ0900 decline (its key is evaluated too — a later slice).")
  (input
    (do
      (def
        (mk (: r (Result Int64 String)) (: s (Result Int64 String)))
        (:
          (Ok #record((= a (try r)) (= b (try s))))
          (Result (Record (: a Int64) (: b Int64)) String)))
      (def
        (main (: k Int64))
        (match
          (mk (if (> k 0) (Ok 7) (Err "a")) (if (> k 1) (Ok 9) (Err "b")))
          ((Ok rec) (+ rec.a rec.b))
          ((Err _e) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 16 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  ; reclaim lock (v-memory-safety): the `(Ok #record((= a a)(= b b)))` heap value built via the hoisted `?` boundary lets
  ; must reclaim on every path -- the both-Ok path (record built + fields read) and the Err short-circuits (record never built, payloads unwind). Enforces the doc's "leak-clean (live-objects 0 every
  ; path)" (was value-only). BRICK-3 compound-ctor `?` reclaim family.
  (live-objects 0))

(case
  "trn1 two `?`s NESTED across compound constructors (record + list inside a tuple) each short-circuit"
  (doc
    "The recursive/compositional face of the compound-constructor descent: the `?`s are not siblings in one
     container but live at DIFFERENT nesting depths — `(Ok #tuple(#record((= a (try r))) #list((try s))))`
     under a `(Result (Tuple (Record (: a Int64)) (List Int64)) String)` boundary. The hoist descends the
     outer `#tuple`, then into the `#record` field value and the `#list` element (via the recursive
     `find_hoistable_try` + fixpoint), lifting BOTH `?`s to nested boundary `let`s in evaluation order — the
     record's `?` (first tuple element) before the list's `?` (second). Both Ok → tuple `({a=3}, [4])`,
     read as `3 + len[4] = 4`; the FIRST failing `?` short-circuits `mk` to its Err (mapped to -1), leaving
     the later one unevaluated (k=1: the list's Err; k=0: the record's Err). Verified leak-clean
     (live-objects 0 every path). Pins that the list/tuple/set/record element-`?` descent COMPOSES through
     arbitrary nesting, not just one level.")
  (input
    (do
      (def
        (mk (: r (Result Int64 String)) (: s (Result Int64 String)))
        (:
          (Ok #tuple(#record((= a (try r))) #list((try s))))
          (Result (Tuple (Record (: a Int64)) (List Int64)) String)))
      (def
        (main (: k Int64))
        (match
          (mk (if (> k 0) (Ok 3) (Err "a")) (if (> k 1) (Ok 4) (Err "b")))
          ((Ok t) (+ (. (. t 0) a) (List.len (. t 1))))
          ((Err _e) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 4 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  ; reclaim lock (v-memory-safety): the `(Ok #tuple(#record #list))` heap value built via the hoisted `?` boundary lets
  ; must reclaim on every path -- the both-Ok path (nested tuple/record/list built + read) and the Err short-circuits (nothing built, payloads unwind). Enforces the doc's "leak-clean (live-objects 0 every
  ; path)" (was value-only). BRICK-3 compound-ctor `?` reclaim family.
  (live-objects 0))

(case
  "trc1 two `?`s as CALL ARGUMENTS hoist left-to-right and short-circuit at the FIRST failure"
  (doc
    "The plain-application face of the expression-position `?` (the generic operand descent, distinct from the
     dedicated `#list`/`#tuple`/`#record` compound-ctor arms): two `?`s are ARGUMENTS of an ordinary function
     call — `(Ok (add (try r) (try s)))` under a `(Result Int64 Int64)` boundary. `find_hoistable_try`
     descends the name-head application left-to-right and, by fixpoint, lifts BOTH `?`s to nested boundary
     `let`s in argument order — `(let ((a (try r))) (let ((b (try s))) (Ok (add a b))))`. Both Ok → `add 3 4
     = 7`; the FIRST failing `?` short-circuits `mk` to its Err, leaving the later one unevaluated — pinned
     with DISTINCT Err payloads: at k=0 the first arg's `?` fails (→ Err 111), at k=1 the second (→ Err 222),
     proving left-to-right evaluation order. Verified leak-clean (live-objects 0 every path).")
  (input
    (do
      (def (add (: a Int64) (: b Int64)) (+ a b))
      (def
        (mk (: r (Result Int64 Int64)) (: s (Result Int64 Int64)))
        (: (Ok (add (try r) (try s))) (Result Int64 Int64)))
      (def
        (main (: k Int64))
        (match
          (mk (if (> k 0) (Ok 3) (Err 111)) (if (> k 1) (Ok 4) (Err 222)))
          ((Ok v) v)
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 222 Int64))
  (call main (: 2 Int64))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "trmc1 a `?` as the argument of a MODULE-MEMBER call unwraps then calls"
  (doc
    "The member-call face of the expression-position `?`: a `?` as the argument of a module-member call —
     `(Ok (List.len (try r)))` under a `(Result Int64 String)` boundary. A member call `List.len` desugars to
     a `.`-headed member-access node, so the head is not a plain name; the hoist's dedicated member-call arm
     (`find_hoistable_try`) recognizes a `.`-head with a SIMPLE receiver (`List`, a pure name atom — re-read,
     not re-evaluated, in the continuation) and descends its ARGUMENTS like operator operands, hoisting to
     `(let ((h (try r))) (Ok (List.len h)))`. k>0 → Ok payload `#list(10 20 30)`, `List.len` = 3; k=0 → Err
     short-circuits `mk` to -1. Verified leak-clean (live-objects 0 both paths). A COMPOUND receiver (a nested
     module path or a computed expression) stays a clean CDZ0900 decline — it is not descended, so no
     non-value head is mis-bound and no receiver effect is reordered.")
  (input
    (do
      (def
        (mk (: r (Result (List Int64) String)))
        (: (Ok (List.len (try r))) (Result Int64 String)))
      (def
        (main (: k Int64))
        (match (mk (if (> k 0) (Ok #list(10 20 30)) (Err "e"))) ((Ok v) v) ((Err _e) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "trmd1 `?`s as `#map` KEY and VALUE hoist KEY-BEFORE-VALUE and short-circuit in evaluation order"
  (doc
    "The `#map` completion of the compound-constructor element-`?` DESCENT (list/tuple/set/record already
     covered): a `#map`'s kids are `(= key value)` PAIRS where BOTH the key and the value are evaluated —
     unlike a `#record` (static field name) — KEY before VALUE, entries left-to-right (the runtime builds a
     `Map.insert(…, k, v)` chain whose args evaluate left-to-right). The hoist flattens the entries to their
     eval-order positions `[k0, v0, …]` and lifts the FIRST `?` with the earlier positions bound as a prefix.
     Here ONE entry has a `?` in BOTH its key and its value with DISTINCT Err payloads: at k=0 both fail and
     the KEY's `?` short-circuits FIRST (→ Err 111), pinning key-before-value ordering; at k=1 the key is Ok
     and the VALUE's `?` fails (→ Err 222); at k=2 both Ok → the one-entry map (len 1). Verified leak-clean
     (live-objects 0 every path) — the CHAMP re-wrapped-husk reclaim landed (62750d2b57). Complements
     v-memory-safety's trmp1 shell-reclaim lock: trmd1 exercises the DESCENT (both `?`s evaluated + hoisted).")
  (input
    (do
      (def
        (mk (: rk (Result Int64 Int64)) (: rv (Result Int64 Int64)))
        (: (Ok #map((= (try rk) (try rv)))) (Result (Map Int64 Int64) Int64)))
      (def
        (main (: k Int64))
        (match
          (mk (if (> k 0) (Ok 1) (Err 111)) (if (> k 1) (Ok 2) (Err 222)))
          ((Ok m) (Map.len m))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 222 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "trsd1 two `?`s as `#set` elements hoist left-to-right and short-circuit at the FIRST failure"
  (doc
    "The `#set` element-`?` descent (Set was already in the compound-ctor descendable set but had no fence and
     latent-leaked before the CHAMP fix): two `?`s as ELEMENTS of a `#set` — `(Ok #set((try r) (try s)))`
     under a `(Result (Set Int64) String)` boundary — hoist left-to-right to nested boundary `let`s and
     short-circuit at the first failure. Both Ok → the two-element set (len 2); k=1 short-circuits on the
     second Err, k=0 on the first (→ -1). Verified leak-clean (live-objects 0 every path) — Set is a CHAMP
     collection, so the re-wrapped Err husk on the short-circuit path is reclaimed by 62750d2b57.
     Complements v-memory-safety's trst1 shell-reclaim lock: trsd1 exercises the element DESCENT.")
  (input
    (do
      (def
        (mk (: r (Result Int64 String)) (: s (Result Int64 String)))
        (: (Ok #set((try r) (try s))) (Result (Set Int64) String)))
      (def
        (main (: k Int64))
        (match
          (mk (if (> k 0) (Ok 3) (Err "a")) (if (> k 1) (Ok 4) (Err "b")))
          ((Ok st) (Set.len st))
          ((Err _e) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 1 Int64))
  (output (: -1 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (live-objects 0))

; trx1: try-unwind THROUGH a handle whose LIST seed is read by a MATCH-shaped arm, with a leading
; tick — the four-factor conjunction (list seed x match-in-arm x pre-try perform x try early-return).
; FIXED (v-effects): previously REJECTED with `CDZ0101: unbound name #seed<n>` — under this conjunction
; the resumptive fold DUPLICATES the continuation and some `#seed` reference COPIES were memoized UNBOUND
; at threading time (before `apply_seed_wrap` minted the binder), sitting OUTSIDE the drained-`#st`
; `pending_inits` set `reduce_handle` forgets, so they stayed unbound. Fix = a targeted `forget_name_refs`
; that clears the stale `#seed{init}` NAME refs throughout the wrapped result so they re-resolve against
; the grafted binder (effects/reduce.rs). It now COMPILES and folds; and the handler-completion seam
; reclaims the final threaded list-seed on the success path (v-memory-safety's compound-payload sum-shell
; reclaim fix, #8105 — live-objects 0, confirmed under --guarded-all). Idealistic values from the
; scalar-seed twin + the arm's List.at-0 semantics (a=10 on both ticks): Ok path 5+10 = 15, Err path
; unwinds to -1. (breaker probe tr3, filed to v-effects tick 1475.)
(case
  "a try early-return unwinds a handle with a list seed and a match-shaped arm"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def (inner (: b Bool)) (: (if b (Ok 5) (Err "bad")) (Result Int64 String)))
      (def
        (f (: b Bool))
        (handle
          C
          #list(10)
          ((tick
              ()
              s
              (match
                (List.at s 0)
                ((Some a) (resume a (List.push s (+ a (List.len s)))))
                ((None) (resume -9 s)))))
          (do (C.tick) (let ((v (try (inner b)))) (Ok (+ v (C.tick)))))))
      (def (main (: n Int64)) (match (f (> n 0)) ((Ok v) v) ((Err _e) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 15 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (live-objects 0))

; tmx1: the MUTUAL-RECURSION face of the BRICK-3b runtime-operand gap — `try` over a call whose
; callee is one half of a mutually-recursive pair (ev/od Result-returning accumulators, the operand
; `(inner (not (= n 3)))` a runtime-variant user fn). Declines today on ALL THREE targets ("lowers
; only a constant operand" / run-rust declined / cadenza Poison) — the operand is opaque exactly
; like the stored-closure and do-def faces above, but the flip adds a distinct emit obligation: the
; Err unwind must break OUT of the mutual pair's CONVERTED LOOP (mtx1 pins the pair is a loop, not
; frames — so the short-circuit is a loop-exit branch, not a frame unwind). Idealistic values by
; hand-derivation matching the constructor-visible twin semantics: ev adds 1+try, od adds 2+try,
; try(inner)=5 while n!=3; main(2) completes ev(2)->od(1)->ev(0) = Ok 13; main(10) counts down into
; n=3 where inner Errs -> the whole loop short-circuits -> -1. Auto-flips with BRICK 3b (then also
; pins loop-exit-unwind composition in one case). (breaker probe tm1.)
(case
  "a try over a mutual-recursive callee short-circuits out of the converted loop"
  (doc
    "The expression-position `?` in a MUTUAL-recursion argument (BRICK 3 slice 2b): `ev`/`od` alternate,
     each threading `(+ 1 (try (inner …)))` (resp. `(+ 2 …)`) into the OTHER's accumulator argument. The
     `?` sits in the ELSE arm of the tail `if` (a tail position — slice 2a) inside `(od (- n 1) (+ acc …))`,
     where the FIRST argument `(- n 1)` is evaluated before it and can trap (a non-atom prefix). The hoist
     binds that impure prefix to its OWN `let` ahead of the `?`'s `let` — `(let ((t (- n 1))) (let ((x (try
     (inner …)))) (od t (+ acc (+ 1 x)))))` — preserving evaluation order and trap timing, then both `let`s
     ride the inline-safe `lower_let` short-circuit. inner is Ok(5) except at n=3 (Err \"stop\"). n=2: the
     recursion reaches n=0 without hitting n=3, summing acc = (1+5)+(2+5) = 13; n=10: passes through n=3, the
     `?` short-circuits the whole mutual loop to Err, mapped to -1. Verified leak-clean (live-objects 0).")
  (input
    (do
      (def (inner (: b Bool)) (: (if b (Ok 5) (Err "stop")) (Result Int64 String)))
      (def
        (ev (: n Int64) (: acc Int64))
        (:
          (if (= n 0) (Ok acc) (od (- n 1) (+ acc (+ 1 (try (inner (not (= n 3))))))))
          (Result Int64 String)))
      (def
        (od (: n Int64) (: acc Int64))
        (:
          (if (= n 0) (Ok acc) (ev (- n 1) (+ acc (+ 2 (try (inner (not (= n 3))))))))
          (Result Int64 String)))
      (def (main (: n Int64)) (match (ev n 0) ((Ok v) v) ((Err _e) -1)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 13 Int64))
  (call main (: 10 Int64))
  (output (: -1 Int64)))

; tbx1/tbx2: the try x BIN-MATCH x EFFECTS conjunction, pinned as a twin pair. tbx1 (PASS) is the
; MATCH-spelled control: a performing bin-parse helper ((bin (u8 137) (u8 x)) whose Ok payload adds
; a tick) matched under the handler, the Ok arm performing a second tick — folds exact
; (-99579 + 11n: Ok leg (42+n)*10 + (n+1), Err leg -1). tbx2 (TODO) is the SAME program with the
; match spelled as `try` + boundary re-wrap: it declines CDZ0900 today (the try distribution over
; the performing helper exceeds the tail-resumptive fold), even though try x bin WITHOUT effects
; folds (verified) and the match twin folds — the gap is precisely try x performing-operand under a
; handler. Idealistic values identical to the twin by construction. Flips with the try-under-effects
; fold increment; the twin guards the match spelling meanwhile. (breaker probes tb1-tb3, tick 1502;
; twin verified tri-target exact + byte-idempotent + census TRUE-0 on the fresh debug runtime.)
(case
  "a performing bin-parse matched under its handler folds with both ticks threaded"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def
        (parse (: b Bytes))
        (:
          (match b ((bin (u8 137) (u8 x)) (Ok (+ (Int64.of x) (C.tick)))) (_ (Err "bad")))
          (Result Int64 String)))
      (def
        (run (: n Int64) (: k Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (match
            (parse (Bytes.of #list((UInt8.of k) 42)))
            ((Ok v) (+ (* v 10) (C.tick)))
            ((Err _e) -1))))
      (def (main (: n Int64)) (+ (run n 137) (* 100000 (run n 1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: -99579 Int64))
  (call main (: 3 Int64))
  (output (: -99546 Int64)))

(case
  "the try spelling of the performing bin-parse folds once try-under-effects lands"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def
        (parse (: b Bytes))
        (:
          (match b ((bin (u8 137) (u8 x)) (Ok (+ (Int64.of x) (C.tick)))) (_ (Err "bad")))
          (Result Int64 String)))
      (def
        (run (: n Int64) (: k Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (match
            (:
              (let ((v (try (parse (Bytes.of #list((UInt8.of k) 42)))))) (Ok (+ (* v 10) (C.tick))))
              (Result Int64 String))
            ((Ok v) v)
            ((Err _e) -1))))
      (def (main (: n Int64)) (+ (run n 137) (* 100000 (run n 1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: -99579 Int64))
  (call main (: 3 Int64))
  (output (: -99546 Int64)))

(case
  "trmp1 a `?`-bound value in a `(Ok #map)` Ok arm reclaims on the Err short-circuit and the Ok path"
  (doc
    "The CHAMP-collection (Map) face of the compound-ctor `?` reclaim family: a `?`-bound scalar is used
     to BUILD a `#map` under the Ok arm of a `(Result (Map Int64 Int64) Int64)` boundary — `(do (def x
     (try r)) (Ok #map((= x x))))`. When `r` is Err the `try` short-circuits `mk` to a fresh re-wrapped
     `(Err …)` husk of the MAP-Ok sum type; when `r` is Ok the map is built and `Map.len` reads it. The
     enclosing `main` match extracts `e`/reads `Map.len m` and must drop the sum shell on BOTH paths.
     REGRESSION WITNESS (v-memory-safety): the re-wrapped Err husk LEAKED one cell because `Map.len m` in
     the Ok arm was mis-classified as CONSUMING its `m` operand (the CHAMP length ops `MapSize`/`SetLen`
     were absent from `arm_borrows_heap_subvalue`'s borrowing-read relax list, while `List.len`/`ListLen`
     reclaimed clean) → the escape check blocked the enclosing `MatchSum` shell-reclaim. Fixed by relaxing
     the `MapSize`/`SetLen` operand to `borrowed` exactly like `ListLen` (they `map-size`/`set-size` BORROW,
     O(1), returning a scalar and retaining no handle). Verified leak-clean (live-objects 0 every path).")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #map((= x x)))) (Result (Map Int64 Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok m) (Map.len m))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "trst1 a `?`-bound value in a `(Ok #set)` Ok arm reclaims on the Err short-circuit and the Ok path"
  (doc
    "The Set sibling of trmp1 (CHAMP-collection compound-ctor `?` reclaim family): a `?`-bound scalar
     builds a `#set` under the Ok arm of a `(Result (Set Int64) Int64)` boundary. `Set.len`/`SetLen`
     BORROWS (`set-size`, O(1), scalar result) exactly like `Map.len`/`MapSize` and `List.len`/`ListLen`;
     the same `arm_borrows_heap_subvalue` borrow-relax fix reclaims the re-wrapped Err husk (Err path) and
     the built set (Ok path). Verified leak-clean (live-objects 0 every path).")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #set(x))) (Result (Set Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok m) (Set.len m))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "trsc1 a `?`-bound Set probed by Set.contains in the Ok arm reclaims the try shell on both paths"
  (doc
    "The Set.contains (scalar-bool borrow) reclaim companion of trst1: a `?`-bound scalar builds a `#set`
     under the Ok arm of a `(Result (Set Int64) Int64)` boundary, then the arm PROBES the matched Set
     payload with `(if (Set.contains s 5) 1 0)`. `Set.contains` BORROWS both operands and returns a scalar
     bool — `op_set_contains` is the only bool-returning CHAMP op and retains NO handle — so the set operand
     is genuinely borrowed exactly like `Set.len`. Before the `arm_borrows_heap_subvalue` fix relaxed the
     SetContains SET operand to borrowed (it had been left CONSUMING — a copy of the Map.lookup/remove
     pattern, which legitimately keep the collection consuming because their Option/removed result can alias
     it), the probe was mis-read as consuming the set, blocking the enclosing `MatchSum` shell-reclaim so the
     re-wrapped Err husk (Err path) and the built set (Ok path) leaked. Verified leak-clean (live-objects 0
     every path). The `Set.contains` reclaim sibling of the trmp1/trst1 (Map.len/Set.len) locks.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #set(x))) (Result (Set Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok s) (if (Set.contains s 5) 1 0))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "trmtl1 a `?`-bound Map enumerated by Map.to-list in the Ok arm reclaims the try shell on both paths"
  (doc
    "The Map.to-list (heap-result borrow-producer) reclaim companion of trmp1 (Map.len): a `?`-bound scalar
     builds a `#map` under the Ok arm of a `(Result (Map Int64 Int64) Int64)` boundary, then the arm reads
     the matched Map payload with `(List.len (Map.to-list m))`. `Map.to-list` BORROWS the map (an O(n)
     enumeration) and yields a FRESH List whose entries independently co-own the cells — so the map operand
     is genuinely borrowed, exactly like `Map.len`. Two fixes compose to make this leak-clean: (1)
     v-core-opt's `arm_borrows_heap_subvalue` MapToList relax (936d26fae8) un-blocks the enclosing
     `MatchSum` shell-reclaim so the re-wrapped Err husk reclaims (Err path); (2) the emit-side twin — the
     `Core::MapToList` arm reclaims the child-dup'd BORROWED source (the try-materialized `Core::SumPayload`
     payload-view is child-dup'd at its materialization since it is a `dup_sites` node, but `map-to-list`
     only BORROWS it, so the preservation-dup was unbalanced → the +1 Ok-path residual). rc-trace confirms
     the map node is DUP'd 1->2->3 and the added drop lands on a live handle (no double-free). Verified
     leak-clean (live-objects 0) every path.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #map((= x x)))) (Result (Map Int64 Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok m) (List.len (Map.to-list m)))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "trstl1 a `?`-bound Set enumerated by Set.to-list in the Ok arm reclaims the try shell on both paths"
  (doc
    "The Set.to-list sibling of trmtl1 (same heap-result borrow-producer reclaim family): a `?`-bound scalar
     builds a `#set` under the Ok arm of a `(Result (Set Int64) Int64)` boundary, then the arm reads the
     matched Set payload with `(List.len (Set.to-list s))`. `Set.to-list` BORROWS the set (op_set_to_list
     dups each element into the fresh List), so the set operand is genuinely borrowed; the `arm_borrows`
     SetToList relax + the emit-side `Core::SetToList` child-dup'd-borrowed-source reclaim compose to un-block
     the shell-reclaim so the re-wrapped Err husk (Err path) + the built set and its enumeration
     preservation-dup (Ok path) reclaim. UNLIKE `Set.contains` (trsc1) — a scalar-returning borrow whose set
     operand is never a `dup_sites` child-dup — to-list's heap result forces the preservation-dup, so this
     needs the emit reclaim where contains does not. Verified leak-clean (live-objects 0) every path.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #set(x))) (Result (Set Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok s) (List.len (Set.to-list s)))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "trml1 a `?`-bound Map read by Map.lookup in the Ok arm reclaims the try shell + payload on both paths"
  (doc
    "The Map.lookup (fallible-extraction returning a Some(value) view) reclaim companion of trmp1/trmtl1: a
     `?`-bound scalar builds a `#map` under the Ok arm of a `(Result (Map Int64 Int64) Int64)` boundary, then
     the arm reads the payload via `(match (Map.lookup m 5) ((Some v) v) ((None) -9))`. Map.lookup BORROWS the
     map and returns the stored value handle (dup'd so the map keeps its own ref) — a scalar Int64 here — so
     the extracted view is dead-after. The node#6-nonlen shell-reclaim admit (v-core-opt: Owned + dead-after
     Core::MatchSum in the fresh-producer set) reclaims the try-materialized scrutinee shell + Map payload on
     BOTH paths (Err short-circuit re-wraps 111; Ok reads then reclaims). Verified live-objects 0 every path.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #map((= x x)))) (Result (Map Int64 Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok m) (match (Map.lookup m 5) ((Some v) v) ((None) -9)))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "trla1 a `?`-bound List read by List.at in the Ok arm reclaims the try shell + payload on both paths"
  (doc
    "The List.at sibling of trml1: a `?`-bound scalar builds a `#list` under the Ok arm of a
     `(Result (List Int64) Int64)` boundary, read via `(match (List.at xs 0) ((Some v) v) ((None) -9))`.
     List.at BORROWS the list and returns the element handle (a scalar Int64 here), dead-after. The
     node#6-nonlen admit reclaims the try shell + List payload on both paths. Verified live-objects 0.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #list(x x x))) (Result (List Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok xs) (match (List.at xs 0) ((Some v) v) ((None) -9)))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "trsa1 a `?`-bound String read by String.at in the Ok arm reclaims the try shell + payload on both paths"
  (doc
    "The String.at (fallible-extraction returning a Some(char) view) reclaim companion: a `?`-bound scalar
     builds a runtime String (via String.from-bytes) under the Ok arm of a `(Result String Int64)` boundary,
     read via `(match (String.at s 0) ((Some c) (String.scalar-len c)) ((None) -9))`. String.at BORROWS the
     String and returns a char view read by a scalar-len borrow, dead-after. The node#6-nonlen admit reclaims
     the try shell + String payload on both paths. Verified live-objects 0.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r))
               (def s (match (String.from-bytes (Bytes.of #list((UInt8.of x)))) ((Some ss) ss) ((None) "")))
               (Ok s)) (Result String Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok s) (match (String.at s 0) ((Some c) (String.scalar-len c)) ((None) -9)))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "trss1 a `?`-bound String read by String.slice in the Ok arm reclaims the try shell + payload on both paths"
  (doc
    "The String.slice sibling of trsa1: a `?`-bound scalar builds a runtime String under the Ok arm of a
     `(Result String Int64)` boundary, read via `(match (String.slice s 0 1) ((Some sub) (String.scalar-len
     sub)) ((None) -9))`. String.slice returns a Some(sub) view read by a scalar-len borrow, dead-after (unlike
     Bytes.slice it does not buffer-share-collide with the shell deep-drop — verified clean under the fix). The
     node#6-nonlen admit reclaims the try shell + String payload on both paths. Verified live-objects 0.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r))
               (def s (match (String.from-bytes (Bytes.of #list((UInt8.of x) (UInt8.of x)))) ((Some ss) ss) ((None) "")))
               (Ok s)) (Result String Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok s) (match (String.slice s 0 1) ((Some sub) (String.scalar-len sub)) ((None) -9)))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "trsi1 a `?`-bound Set CONSUMED by Set.insert in the Ok arm reclaims the try shell on both paths"
  (doc
    "The consumed-payload (P7) companion: a `?`-bound scalar builds a `#set` under the Ok arm of a
     `(Result (Set Int64) Int64)` boundary, then the arm CONSUMES the payload via `(Set.len (Set.insert s 9))`
     — Set.insert consumes the matched Set and returns a fresh one. The try-materialized scrutinee shell +
     payload base ref leaked (the shell-reclaim was BLOCKED because the inlined MatchSum scrutinee wasn't in
     the fresh-producer set) until the node#6-nonlen admit. Verified live-objects 0 both paths.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #set(x))) (Result (Set Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok s) (Set.len (Set.insert s 9)))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "trmi1 a `?`-bound Map CONSUMED by Map.insert in the Ok arm reclaims the try shell on both paths"
  (doc
    "The Map sibling of trsi1 (consumed-payload P7): a `?`-bound scalar builds a `#map` under the Ok arm of a
     `(Result (Map Int64 Int64) Int64)` boundary, CONSUMED via `(Map.len (Map.insert m 9 9))`. Same shell-
     reclaim block until the node#6-nonlen admit; verified live-objects 0 both paths.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #map((= x x)))) (Result (Map Int64 Int64) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok m) (Map.len (Map.insert m 9 9)))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (live-objects 0))

; --- OPTION-BOUNDARY companions (found tick #205: v-core-opt's (b)/(c) fix covers the Option `?` boundary
;     too — the inlined Option MatchSum is ALSO Owned+dead-after → admitted. Both clean under 372c0bd5a0;
;     O2-consume was +2 on clean main. Payloadless None failure arm needs no equalize; the CONSUME case
;     still needs the shell-reclaim admit, which the fix provides.) ---

(case
  "trob1 an Option `?`-bound Set read by Set.len in the Some arm reclaims the try shell on both paths"
  (doc
    "The Option-boundary borrow companion of trsi1/trmi1: a `?`-bound scalar builds a `#set` under the Some
     arm of an `(Option (Set Int64))` boundary (payloadless None failure arm), read via `(Set.len s)`. The
     node#6-nonlen admit reclaims the try-materialized Option MatchSum shell + Set payload on both paths
     (None short-circuit → -1; Some → borrow-then-reclaim). Verified live-objects 0.")
  (input
    (do
      (def (mk (: o (Option Int64)))
        (: (do (def x (try o)) (Some #set(x))) (Option (Set Int64))))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Some 5) None))
          ((Some s) (Set.len s))
          ((None) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "troc1 an Option `?`-bound Set CONSUMED by Set.insert in the Some arm reclaims the try shell on both paths"
  (doc
    "The Option-boundary CONSUME companion (the (c) P7 shape over an Option `?`): a `?`-bound scalar builds a
     `#set` under the Some arm of an `(Option (Set Int64))` boundary, CONSUMED via `(Set.len (Set.insert s
     9))`. The try-materialized Option MatchSum scrutinee shell + payload base ref leaked (+2 on the Some
     path) until the node#6-nonlen admit (the inlined Option MatchSum is Owned + dead-after → admitted to the
     fresh-producer set, same as the Result boundary). Verified live-objects 0 both paths.")
  (input
    (do
      (def (mk (: o (Option Int64)))
        (: (do (def x (try o)) (Some #set(x))) (Option (Set Int64))))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Some 5) None))
          ((Some s) (Set.len (Set.insert s 9)))
          ((None) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: -1 Int64))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "trbs1 a `?`-bound Bytes sliced by Bytes.slice in the Ok arm — KNOWN-LEAK (queue (e) Bytes.slice buffer-share)"
  (doc
    "The Bytes.slice sibling of the node#6-nonlen family — DELIBERATELY a KNOWN-LEAK, not live-objects 0. A
     `?`-bound scalar builds a runtime Bytes under the Ok arm of a `(Result Bytes Int64)` boundary, read via
     `(match (Bytes.slice bs 0 2) ((Some sl) (Bytes.len sl)) ((None) -9))`. UNLIKE the other fallible-
     extractions (Map.lookup/List.at/String.at/String.slice — all admitted to the try-shell reclaim, live-
     objects 0), Bytes.slice is EXCLUDED from the node#6-nonlen admit: op_bytes_slice returns a view SHARING
     bs's buffer without an independent buffer rc, so admitting the shell deep-drop DOUBLE-FREES the buffer the
     slice view still points at (traps even view-unused — v-memory-safety caught this UAF on the pre-push fix,
     tick #197/#198). Leak-over-UAF: BytesSlice stays consuming/declines, so the shell + payload safe-LEAK
     rather than trap. Pinned known-leak to LOCK the exclusion (it must never silently start trapping again);
     flips to live-objects 0 when queue (e) gives op_bytes_slice a clean independent buffer rc.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok (Bytes.of #list((UInt8.of x) (UInt8.of x) (UInt8.of x))))) (Result Bytes Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok bs) (match (Bytes.slice bs 0 2) ((Some sl) (Bytes.len sl)) ((None) -9)))
          ((Err e) e)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "trct1 a `?`-bound COMPOUND (tuple carrying a Set) Ok-arm payload reclaims the try shell + nested heap on both paths"
  (doc
    "The COMPOUND-payload sibling of the node#6-nonlen family (trml1..trsi1 are single-level: the
     payload IS the collection). Here a `?`-bound scalar builds a `(tuple x #set(x))` under the Ok arm
     of a `(Result (Tuple Int64 (Set Int64)) Int64)` boundary — a HEAP tuple carrying a NESTED heap
     Set (a 2-level payload). The Ok arm destructures the tuple and reads both fields
     (`(+ a (Set.len s))`). This exercises the try-shell reclaim's DEEP-DROP cascading THROUGH the
     tuple into the nested Set — distinct from the single-level locks, which never recurse a compound.
     The node#6-nonlen admit reclaims the try-materialized scrutinee shell + tuple + nested Set on BOTH
     paths (Err short-circuit re-wraps 111; Ok destructures then reclaims). Verified live-objects 0.")
  (input
    (do
      (def (mk (: r (Result Int64 Int64)))
        (: (do (def x (try r)) (Ok #tuple(x #set(x)))) (Result (Tuple Int64 (Set Int64)) Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok 5) (Err 111)))
          ((Ok t) (match t (#tuple(a s) (+ a (Set.len s)))))
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 6 Int64))
  (live-objects 0))

(case
  "trae1 a `?`-bound heap Result returned BARE as the try-boundary result equalizes the alias-husk (no leak on the failure short-circuit)"
  (doc
    "The node#6-nonlen sub-case (a) alias-husk equalize (the LAST holdout, unblocking v-try-operator's
     trnt1). `mk` returns the `?`-bound value BARE: `(do (def ir (try rr)) ir)` lowers to a divergent
     try-desugar MatchSum over `rr` — the SUCCESS (Ok) arm is a bare `SumPayload` VIEW of rr's payload
     (Borrowed), the FAILURE (Err) arm a fresh payload-carrying `SumNew` (Owned). The arm-blind
     ownership join reads the MatchSum Borrowed, so the scrutinee-shell reclaim is SUPPRESSED and the
     owned Err husk on the failure short-circuit LEAKED one cell (k=0 outer-Err). v-core-opt's
     `divergent_alias_arm_dupable` classifier + v-mem's 3-point emit equalize (stack-dup the success
     view so the join reads Owned, then the shell deep-drop nets the view 2→1) reclaims the husk on the
     failure path (leak→0) without freeing the returned view. SCALAR error only: a HEAP failure payload
     is a scrutinee view too, so the classifier S2 fence declines it (stays a safe leak, leak-over-UAF).
     Verified live-objects 0 both paths.")
  (input
    (do
      (def (mk (: rr (Result (Result Int64 Int64) Int64)))
        (: (do (def ir (try rr)) ir) (Result Int64 Int64)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok (Ok 7)) (Err 111)))
          ((Ok v) v)
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "trae2 a `?`-bound bare-alias Result with a HEAP Err payload — KNOWN-LEAK (the S2 fence declines the equalize; must never TRAP)"
  (doc
    "The HEAP-ERROR sibling of trae1 — DELIBERATELY a KNOWN-LEAK, guarding the UAF boundary v-core-opt's
     RED-review caught. `mk` returns the `?`-bound value bare over a `(Result Int64 Bytes)` boundary, so
     the try-desugar FAILURE value re-wraps the error as a bare `SumPayload` VIEW of the scrutinee too
     (runtime_try_failure_value, lower.rs:5007-5019) — NOT independently owned. If the (a) equalize fired
     here it would deep-drop the scrutinee shell while the failure husk still holds that heap Bytes error
     view → a double-free/UAF on the outer-Err short-circuit. The classifier S2 fence (v-core-opt
     a99b600472: every failure SumNew payload must be SCALAR) DECLINES the equalize for a heap error, so
     the pre-(a) leak persists (one cell on the k=0 outer-Err path) — a SAFE leak (leak-over-UAF). Pinned
     known-leak to LOCK the fence: it must never silently start reclaiming (→ the UAF trap). Flips to
     live-objects 0 only if the failure view is ALSO dup'd (the S2-relax follow-up). A SCALAR error is
     trae1 (live-objects 0).")
  (input
    (do
      (def (mk (: rr (Result (Result Int64 Bytes) Bytes)))
        (: (do (def ir (try rr)) ir) (Result Int64 Bytes)))
      (def (main (: k Int64))
        (match (mk (if (> k 0) (Ok (Ok 7)) (Err (Bytes.of #list((UInt8.of 9))))))
          ((Ok v) v)
          ((Err b) (Bytes.len b))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 7 Int64))
  (live-objects known-leak))

; trnt1: the NESTED try `(try (try rr))` — a `?` whose OPERAND is itself a `?`. Lowered by inner-first
; hoisting in `find_hoistable_try`'s `(try e)` arm: the operand is descended FIRST, so `(try (try rr))`
; lifts to `(let ((a (try rr))) (let ((b (try a))) …))` — each `?` level its own binding-tail `let` riding
; `lower_let`, inner unwrap before outer. The intermediate `a` is a bare-alias inner-Result whose shell
; reclaims via the unified chained-`?` reclaim (the producing-side `(try rr)` reclaims rr's outer shell + the
; dup-backed nested-match reclaims `a` — mem-safety a1e26895c3, fenced to an OWNED scrutinee: a chained-`?`
; on a borrowed-param scrutinee stays a clean decline, leak-over-UAF). Verified leak-clean (live-objects 0
; every path) with SCALAR Int64 errors; a HEAP error type on this shape stays a fenced safe-leak decline.
(case
  "trnt1 a NESTED `?` `(try (try rr))` unwraps twice, each level short-circuiting to the boundary"
  (doc
    "The nested-`?` face of the expression-position operator: the operand of a `?` is ITSELF a `?`. For
     `rr : (Result (Result Int64 Int64) Int64)`, the inner `(try rr)` unwraps the OUTER Result (short-
     circuiting the whole boundary on the outer Err), and the outer `(try …)` unwraps the resulting inner
     Result (short-circuiting on the inner Err) — so `(Ok (try (try rr)))` under a `(Result Int64 Int64)`
     boundary yields the doubly-unwrapped payload, with two distinct short-circuit points. Pinned with
     DISTINCT Err payloads: at k=0 the OUTER Result is Err (→ 111, inner-`?` short-circuits first); at k=1
     it is `(Ok (Err 222))` (→ 222, outer-`?` short-circuits); at k=2 `(Ok (Ok 7))` → 7. Verified leak-clean
     (live-objects 0 every path): the inner-first hoist rides `lower_let` and the intermediate bare-alias
     inner-Result reclaims via the unified chained-`?` shell reclaim (see the trnt1 comment above).")
  (input
    (do
      (def
        (mk (: rr (Result (Result Int64 Int64) Int64)))
        (: (Ok (try (try rr))) (Result Int64 Int64)))
      (def
        (main (: k Int64))
        (match
          (mk (if (> k 0)
                  (if (> k 1)
                      (: (Ok (Ok 7)) (Result (Result Int64 Int64) Int64))
                      (: (Ok (Err 222)) (Result (Result Int64 Int64) Int64)))
                  (: (Err 111) (Result (Result Int64 Int64) Int64))))
          ((Ok v) v)
          ((Err e) e)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: 222 Int64))
  (call main (: 2 Int64))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "trhe1 a `?` short-circuit propagates a HEAP Err payload through the boundary re-wrap, reclaim-clean"
  (doc
    "The heap-Err-payload face of the runtime `?`: unlike the scalar-Err fences (trc1/trmc1/trl1 use an
     Int64 or String Err), here the boundary is `(Result Int64 (List Int64))` — the ERR arm carries a HEAP
     `List`. `(Ok (+ (try r) 1))` in a fallible-boundary body hoists to `(let ((x (try r))) (Ok (+ x 1)))`;
     on the Ok path `x` unwraps and the result is `10 + 1 = 11`; on the Err path the `?` short-circuits,
     re-wrapping the caught heap-`List` Err into `mk`'s Result type and propagating it — the caller reads
     `List.len` of the payload (3). Verified leak-clean (live-objects 0 both paths): the try short-circuit's
     `Core::Block`/`Break` emit threads and reclaims a HEAP Err payload correctly. Complements the nested-`?`
     chained-reclaim case (trnt1) — this locks the ORTHOGONAL axis (heap Err, scalar Ok), guarding the
     Err-propagation path alongside the chained-`?` shell reclaim.")
  (input
    (do
      (def
        (mk (: r (Result Int64 (List Int64))))
        (: (Ok (+ (try r) 1)) (Result Int64 (List Int64))))
      (def
        (main (: k Int64))
        (match (mk (if (> k 0) (Ok 10) (Err #list(7 8 9))))
          ((Ok v) v)
          ((Err xs) (List.len xs))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 0 Int64))
  (output (: 3 Int64))
  (live-objects 0))

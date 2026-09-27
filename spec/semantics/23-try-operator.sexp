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

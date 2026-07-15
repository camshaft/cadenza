; The `?` / `try` fallible short-circuit operator — witnesses DESIGN-try-operator-rcdzc.md. `(try e)` is
; the canonical s-expr form of the ML postfix `e?`: on the success variant it UNWRAPS the payload
; (`Some a` / `Ok a` → `a`), and on the failure variant it SHORT-CIRCUITS the enclosing fallible boundary
; (the enclosing function's `Result`/`Option` result type), making the boundary's value the failure
; itself. It is NOT a monad — it desugars onto the effects system's within-function abortive lowering (a
; synthesized `Mir::Block` + `Mir::Break`), so it adds no user-visible effect and nothing to the effect
; row. See README.md for the case vocabulary.
;
; STAGE STATUS. T0a+T0b (landed): `(try e)` is carried first-class through resolve/infer — its type is
; the operand's success payload; an operand that is not a fallible sum is CDZ0203; a wrong-arity `(try …)`
; is CDZ0201; a `?` with no enclosing `Result`/`Option` function boundary is CDZ0230; and a `Result`-`?`
; under an `Option` boundary (or vice-versa) is CDZ0203 (no coercion). The function-boundary DESUGAR (a
; value executing through wasmtime, both the happy and the short-circuit path) is the next slice; until it
; lands a well-formed `(try e)` DECLINES (scored *todo* by the gate, never a miscompile). The executing
; cases below are the ones that matter most once T1 lands — a value must come out the far side, since `?`
; is control.

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
           (§5.1, against Rust's `?`-via-`From`), so it is CDZ0203. The explicit idiom is
           `Option.ok-or` / `Result.map-err` to reconcile the types first.")
  (input  (do (def (main) (let ((x (try (Ok 1)))) (Some x))) (export main)))
  (error  CDZ0203))

; ── T1 target: executing cases (DECLINE → *todo* until the boundary desugar lands) ──────────────────
; These are the operator's actual ask — the nested-`match`-collapse shapes, run through wasmtime. They
; are recorded here now so the gate pins the intended VALUE; the current generation declines them (the
; desugar is the next slice), which the differential gate scores as *todo*, not disagreement. When T1
; lands they flip from todo to pass with no corpus edit.

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

(case "a trapping earlier let-init is NOT dropped by a constant-failure `?` short-circuit"
  (doc    "Regression pin (PR #409): `(let ((a (/ 1 0)) (x (try (None unit)))) (Some (+ a x)))` — the FIRST
           binding `a` provably traps (÷0), and `x`'s init is a `?` on a constant `None` that would
           short-circuit the boundary. `a` is on the UNCONDITIONAL strict spine BEFORE the `?`, so its trap
           is OBSERVED and MUST fire (§283/§285, dead-binding-drops-a-defined-trap /
           trap-kind-is-observable) — the short-circuit MUST NOT elide it. A bug guarded the fast-path fold
           only on host-call-freedom, so a trapping earlier init was folded away, yielding `(None unit)`
           instead of trapping. The `÷0` is compile-provable, so it is CDZ0304 (a compile-provable trap
           fails the build); the fold now requires earlier inits to be trap-free too.")
  (input  (do (def (main) (let ((a (/ 1 0)) (x (try (None unit)))) (Some (+ a x)))) (export main)))
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

; Closures across the HOST boundary — witnesses DESIGN-closure-host-resource-rcdzc.md (C-HOST-1). A
; Cadenza closure that crosses the component boundary becomes a component-model RESOURCE, monomorphized
; per closure SIGNATURE `(-> A B)`, exposing a `call` method the host invokes. The closure's heap-cell
; handle IS the resource rep (both i32); the `call_indirect` stays INSIDE Cadenza — the host is a
; CUSTODIAN of an opaque handle, not an implementor. The handle ALWAYS originates in-guest: a closure
; crosses as the RESULT of an export, and `resource.new` is spliced at the boundary-return.
;
; The host (cdz-run) drives such a program by calling `make()` (→ the closure resource handle) then
; `call(handle, args…)` — so a case's `(call main <arg>)` supplies the CLOSURE's argument (not a bare
; function's), and the `(output …)` is what the closure returns. `own<t>` consumes the handle per call
; (single-use per handle in this increment; a `borrow<t>` handle for repeated calls is a later step), so
; each case drives one `make`+`call`.
;
; SCOPE (C-HOST-1): a NO-CAPTURE closure `(-> Int64 Int64)` exported directly, its `call` dispatched
; through the guest's own funcref table. Capturing closures + parameterized exports + multi-arg
; signatures + the host handing a closure back are later increments (C-HOST-2..4).

(case "a closure exported to the host is called by the host"
  (doc    "`(def (main) (fn (x) (+ x 1)))` returns a closure whose result type is `(-> Int64 Int64)`, so
           the whole program crosses as a component-model resource `closure-s64-s64` with a `call` method.
           The host calls `make()` to obtain the closure handle, then `call(handle, 5)`, which dispatches
           `(fn (x) (+ x 1))` through the guest's own `call_indirect` — returning 6. The closure logic
           never leaves Cadenza; the host only holds the opaque handle and invokes it. Pins that a Cadenza
           closure crosses to the host as a callable resource.")
  (input  (do (def (main) (fn ((: x Int64)) (+ x 1))) (export main)))
  (call   main (: 5 Int64))
  (output (: 6 Int64)))

; The SAME exported closure invoked with a different argument — the host mints a fresh handle (`make`) and
; calls it, showing the resource + its `call` dispatch are reusable, and that the result tracks the input.

(case "a host-called closure applied to a different argument tracks the input"
  (doc    "The same `(fn (x) (+ x 1))` closure export, called with 41 → 42. The host `make`s a fresh
           closure handle and `call`s it (each `own<t>` handle is one-shot). Pins that the closure's
           dispatch is reusable across handles and its result follows the argument.")
  (input  (do (def (main) (fn ((: x Int64)) (+ x 1))) (export main)))
  (call   main (: 41 Int64))
  (output (: 42 Int64)))

; A closure whose body MULTIPLIES rather than adds — a different lifted code selected through the same
; call_indirect boundary, proving the resource carries the RIGHT closure code (its funcref-table slot).

(case "a host-called closure with a different body dispatches the right code"
  (doc    "`(fn (x) (* x 3))` exported and called with 4 → 12. The closure's own lifted code (a distinct
           funcref-table slot) is what `call` dispatches, so a different closure body yields a different
           result through the identical boundary. Pins that the closure resource carries its code, not a
           fixed operation.")
  (input  (do (def (main) (fn ((: x Int64)) (* x 3))) (export main)))
  (call   main (: 4 Int64))
  (output (: 12 Int64)))

; C-HOST-2 — a PARAMETERIZED export returning a CAPTURING closure. `(def (adder (: k Int64)) (fn (x) (+ x
; k)))` returns a closure that captures `k`, so the whole export crosses as `adder : (Int64) ->
; own<closure-s64-s64>`. The host computes a DISTINCT closure per input: `make(k)` runs the export body
; (closing over `k` into the cell), then `call(handle, x)` reads `k` back from the cell inside the
; dispatch. The handle genuinely originates in-guest, computed from the host's input. The corpus `(call
; …)` args are SPLIT by `make`'s arity: the first (here `k`) goes to `make`, the rest (here `x`) to `call`.

(case "a parameterized export returning a capturing closure is made and called by the host"
  (doc    "`(def (adder (: k Int64)) (fn (x) (+ x k)))` — the host calls `make(10)` (building a closure
           that captured k=10), then `call(handle, 5)` = 5 + 10 = 15. Pins that the closure handle is
           computed from the host's input (make forwards the export param) AND the captured environment
           rides in the cell, read back inside the closure's `call` dispatch. The first `(call …)` arg
           (10) is make's `k`, the second (5) is the closure's `x`.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
  (call   adder (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

; The same capturing closure with a different capture AND a different call argument — the result tracks
; both, confirming `make`'s input flows into the captured cell and `call`'s input into the dispatch.

(case "a capturing closure export tracks both the captured value and the call argument"
  (doc    "`adder(100)` then `call(7)` = 7 + 100 = 107. A different `k` (100) captured, a different `x` (7)
           applied — the result follows both, so the captured value is genuinely per-`make` and the call
           argument per-`call`.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
  (call   adder (: 100 Int64) (: 7 Int64))
  (output (: 107 Int64)))

; C-HOST-3 — a MULTI-ARGUMENT closure. `(-> Int64 (-> Int64 Int64))` (curried sugar `(fn (a b) …)`)
; crosses as a resource whose `call` takes BOTH arguments: `call : (self, a: s64, b: s64) -> s64`. The
; guest's lifted body is `(env, a, b) -> result`, so `call` pushes both args before the `call_indirect`.
; The `call` method's arity generalizes past one argument (C-HOST-1/2 were single-arg).

(case "a two-argument closure exported to the host is called with both arguments"
  (doc    "`(fn (a b) (+ a b))` crosses as a resource whose `call` takes two Int64 args. The host calls
           `make()` then `call(handle, 3, 4)` = 7 — both args pushed to the guest's `call_indirect`. Pins
           that a closure's `call` method carries more than one argument.")
  (input  (do (def (main) (fn ((: a Int64) (: b Int64)) (+ a b))) (export main)))
  (call   main (: 3 Int64) (: 4 Int64))
  (output (: 7 Int64)))

(case "a three-argument closure exported to the host is called with all three"
  (doc    "`(fn (a b c) (+ (+ a b) c))` → `call(handle, 2, 3, 4)` = 9. Pins that the `call` arity is not
           special-cased to two — any number of scalar args crosses.")
  (input  (do (def (main) (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ a b) c))) (export main)))
  (call   main (: 2 Int64) (: 3 Int64) (: 4 Int64))
  (output (: 9 Int64)))

; A PARAMETERIZED export returning a MULTI-ARG CAPTURING closure — C-HOST-2 (capture + make-forwarding)
; composed with C-HOST-3 (multi-arg call). `make`'s param (k) and the closure's two args (a, b) are all
; supplied through the split `(call …)` list: the first (k) to `make`, the rest (a, b) to `call`.

(case "a parameterized export returning a multi-argument capturing closure"
  (doc    "`(def (adder3 (: k Int64)) (fn (a b) (+ (+ a b) k)))` — `make(100)` builds a closure capturing
           k=100, then `call(handle, 2, 3)` = 2 + 3 + 100 = 105. Composes make-param forwarding, a
           captured env, and a two-argument `call`.")
  (input  (do (def (adder3 (: k Int64)) (fn ((: a Int64) (: b Int64)) (+ (+ a b) k))) (export adder3)))
  (call   adder3 (: 100 Int64) (: 2 Int64) (: 3 Int64))
  (output (: 105 Int64)))

; A closure whose RESULT type is Bool — `(-> Int64 Bool)`. The `call` method returns a boolean; the host
; renders it. Pins that the closure's result valtype is not fixed to an integer.

(case "a closure returning a boolean is called by the host"
  (doc    "`(fn (x) (= x 0))` is a `(-> Int64 Bool)` closure; `make()` then `call(handle, 0)` = true (0
           equals 0), `call(handle, 5)` = false. The `call` method's result crosses as a boolean.")
  (input  (do (def (main) (fn ((: x Int64)) (= x 0))) (export main)))
  (call   main (: 0 Int64))
  (output (: true Bool)))

; A closure that PERFORMS AN EFFECT cannot escape to the host — the scope fence for this whole feature. A
; closure's effects are discharged by the `handle`/`(host …)` frame that is DYNAMICALLY OPEN where the
; closure is built; a host-held closure is invoked LATER, outside that frame, so the effect would have no
; home when the host calls it. Here `ask` IS delegated (`(host (ask) …)`), so the effect has a home at the
; export's TOP — but the closure the export RETURNS carries the `ask.ask` past that delegation, out to the
; host, where the delegation no longer applies. We reject this INTENTIONALLY (CDZ0406) rather than compile a
; closure whose effect silently loses its handler. (An effect fully HANDLED inside the closure — reduced to
; plain code with no residual host call — is unaffected; only an effect that would escape is rejected.)

(case "a closure that performs a delegated effect cannot cross the host boundary"
  (doc    "`(def (main) (host (ask) (fn (x) (+ x (ask.ask)))))` returns a closure whose body performs the
           delegated effect `ask.ask`. The delegation `(host (ask) …)` gives the effect a home at the
           export's top, but the RETURNED closure carries `ask.ask` out to the host, to be run when the host
           later invokes `call` — outside the delegation's dynamic extent, where the effect has no home. A
           closure's handler context does not travel with it across the boundary, so this is rejected
           (CDZ0406): closures escaping effects are not supported. Pins the scope fence that a host-held
           closure must be effect-free.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (fn ((: x Int64)) (+ x (ask.ask))))) (export main)))
  (error  CDZ0406))

; --- An exported closure's BODY is type-checked, like an ordinary def / an in-guest-applied lambda ------
; A `(def (a) (fn …))` exported as a host closure crosses the boundary and is NEVER applied in-guest, so
; its body is never β-reduced. An ill-typed body must still be a compile-time rejection — the same CDZ0203
; an ordinary def `(def (main (: x Int64)) (: x Bool))` or an applied `((fn …) 5)` gives — not a silently-
; emitted invalid component. (The closure-export lowering runs the body's type-error collection before emit;
; the closure's params are bound, so an annotation/unification fault in the body surfaces exactly as in an
; ordinary definition.)

(case "an exported closure with an annotation-mismatched body is rejected, not emitted invalid"
  (doc    "`(fn ((: x Int64)) (: x Bool))` — the body annotates an Int64 value as Bool, a type error. An
           ordinary def / an in-guest applied `(fn …)` rejects it CDZ0203; exporting the SAME closure must
           too, rather than skip the body's type-check and emit an invalid component. The closure-export
           path runs the body's `type_errors` before emit.")
  (input  (do (def (a) (fn ((: x Int64)) (: x Bool))) (export a)))
  (error  CDZ0203))

(case "an exported closure with a narrow-arg wide-result mismatched body is rejected, not miscompiled"
  (doc    "`(fn ((: x Int8)) (: (+ x 100) Int64))` — the `(+ x 100)` over an Int8 param is Int8, annotated
           Int64: an annotation mismatch (CDZ0203). Previously this ill-typed body ESCAPED the type-check
           and emitted an INVALID component (the `call` body left an i32 where the result declared i64:
           'type mismatch: expected i64, found i32'). Now the body is type-checked first, so it rejects
           CDZ0203 — the ill-typed program is caught, not miscompiled. (A WELL-TYPED narrow-arg/wide-result
           closure would use an explicit conversion, e.g. `(fn ((: x Int8)) (Int64.of x))`.)")
  (input  (do (def (a) (fn ((: x Int8)) (: (+ x 100) Int64))) (export a)))
  (error  CDZ0203))

; RICHER CAPTURING closures — the C-HOST-2 make-forwarding + captured-cell machinery is arity- and
; body-shape-agnostic, so a closure that captures SEVERAL values, drives control flow off a captured
; Bool, binds a `let` in its body, or calls a top-level helper all cross the boundary and are invoked
; by the host with no additional compiler support. Each `make(captures…)` builds the cell (closing over
; the export's params), and `call(x)` dispatches the lifted body through the guest's `call_indirect`,
; reading the captured environment back from the cell. These witness the CAPTURE path end-to-end past
; the single-scalar-capture cases above.

(case "a closure capturing two values is made and called by the host"
  (doc    "`(def (both (: a Int64) (: b Int64)) (fn (x) (+ (+ a b) x)))` — the closure captures BOTH `a`
           and `b`. The host `make(10, 20)` (closing over a=10, b=20 into the cell), then `call(5)` =
           10 + 20 + 5 = 35. Pins that a closure cell carries MORE THAN ONE captured value, each read
           back inside the `call` dispatch. The first two `(call …)` args are make's captures, the last
           is the closure's argument.")
  (input  (do (def (both (: a Int64) (: b Int64)) (fn ((: x Int64)) (+ (+ a b) x))) (export both)))
  (call   both (: 10 Int64) (: 20 Int64) (: 5 Int64))
  (output (: 35 Int64)))

(case "a capturing closure whose body uses the capture after an inner computation"
  (doc    "`(def (scale (: k Int64)) (fn (x) (* (+ x 1) k)))` — the captured `k` multiplies an inner
           `(+ x 1)`, so it is used AFTER a nested subexpression rather than as the first operand. The host
           calls `make(k=4)` then `call(x=3)` = (3 + 1) * 4 = 16. Pins that the captured value flows
           through a nested subexpression unchanged.")
  (input  (do (def (scale (: k Int64)) (fn ((: x Int64)) (* (+ x 1) k))) (export scale)))
  (call   scale (: 4 Int64) (: 3 Int64))
  (output (: 16 Int64)))

(case "a capturing closure with a let binding in its body"
  (doc    "`(def (f (: k Int64)) (fn (x) (let ((y (* x 2))) (+ y k))))` — the closure body binds a local
           `y` then adds the captured `k`. The host `make(100)` then `call(7)` = (7*2) + 100 = 114. Pins
           that a `let` inside an escaping closure body lowers correctly alongside the captured env.")
  (input  (do (def (f (: k Int64)) (fn ((: x Int64)) (let ((y (* x 2))) (+ y k)))) (export f)))
  (call   f (: 100 Int64) (: 7 Int64))
  (output (: 114 Int64)))

(case "a closure driving control flow off a captured boolean"
  (doc    "`(def (g (: flag Bool)) (fn (x) (if flag (+ x 1) (- x 1))))` — the closure captures a Bool and
           branches on it. The host `make(true)` then `call(10)` = 11 (the then-branch); a `make(false)`
           would yield 9. Pins that a captured Bool drives an `if` inside the `call` dispatch — the
           capture is not restricted to a numeric accumulator.")
  (input  (do (def (g (: flag Bool)) (fn ((: x Int64)) (if flag (+ x 1) (- x 1)))) (export g)))
  (call   g (: true Bool) (: 10 Int64))
  (output (: 11 Int64)))

(case "a closure whose body calls a top-level helper function"
  (doc    "`(def (dbl (: n Int64)) (* n 2))` `(def (h (: k Int64)) (fn (x) (+ (dbl x) k)))` — the escaping
           closure body CALLS the top-level `dbl`. The host `make(5)` (capturing k=5) then `call(3)` =
           (dbl 3) + 5 = 6 + 5 = 11. Pins that a closure crossing the boundary can call another in-program
           function (the helper is emitted as an ordinary reachable def, called directly from the lifted
           closure body).")
  (input  (do (def (dbl (: n Int64)) (* n 2)) (def (h (: k Int64)) (fn ((: x Int64)) (+ (dbl x) k))) (export h)))
  (call   h (: 5 Int64) (: 3 Int64))
  (output (: 11 Int64)))

; The scope fence is SCOPED to the returned closure's body — a BUILD-TIME delegated effect whose result
; the closure merely CAPTURES does NOT escape and must not be rejected. The distinction is where the
; `ask.ask` runs: INSIDE the returned closure (above — escapes, run later outside the delegation) versus
; in the export body PROPER (below — run at export-execution time, while the `(host (ask) …)` delegation
; is still in dynamic scope, its result captured as a plain value). The escape check flags a
; `Core::HostCall` only in the LIFTED closure bodies, not the whole export body — so the build-time case
; is allowed, exactly as the intra-program analogue `(handle … (let ((v (E.get))) (fn (x) (+ x v))))`
; compiles. (Running this needs the export-time host-call boundary — a later increment — so it declines
; today; the point pinned here is that the COMPILE-TIME outcome is NOT the CDZ0406 over-rejection.)

(case "a build-time delegated effect whose result a returned closure captures does not escape"
  (doc    "`(def (main) (host (ask) (let ((v (ask.ask))) (fn (x) (+ x v)))))` performs `ask.ask` in the
           `let` initializer — at export-execution time, inside the `(host (ask) …)` delegation's dynamic
           extent, where the effect has a home — and returns a closure that captures only the plain result
           `v`. The returned closure is effect-free; nothing crosses the host boundary performing `ask`, so
           this is NOT an escaping effect and must not be rejected CDZ0406 (contrast the escaping case
           above, where `ask.ask` is INSIDE the returned closure). The escape check scans the returned
           closure's body, not the whole export body. With `ask.ask` responding 10 and the call argument 3,
           the result is 3 + 10 = 13; running it needs the export-time host-call boundary (a later
           increment), so a generation without it declines rather than over-rejecting CDZ0406.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (let ((v (ask.ask)))
                  (fn ((: x Int64)) (+ x v))))) (export main)))
  (call   main (: 3 Int64))
  (host-responses (respond ask.ask (: 10 Int64)))
  (output (: 13 Int64)))

; MULTI-EXPORT closures — a program that exports SEVERAL closures of the same signature crosses as ONE
; resource type with a `make-<name>` per export (`make-inc`, `make-triple`) sharing ONE `call` method. The
; shared `call` is the load-bearing realization: the closure's code slot rides in the resource rep, so
; `resource.rep` → `call_indirect` at call time dispatches WHICHEVER closure the handle names, regardless
; of which `make` built it. The corpus `(call <name> …)` picks which `make-<name>` the host invokes, then
; drives the shared `call` — so `(call inc 5)` runs `make-inc()` then `call(5)`, and `(call triple 5)` runs
; `make-triple()` then the SAME `call(5)`. (Distinct-signature multi-export — N resource types — and a
; closure exported alongside a non-closure export are later increments; both decline cleanly.)

(case "one of several same-signature closure exports is made and called by the host"
  (doc    "Two closure exports `(def (inc) (fn (x) (+ x 1)))` and `(def (triple) (fn (x) (* x 3)))` cross
           together as one resource with `make-inc`/`make-triple` + a shared `call`. Calling `inc` drives
           `make-inc()` then `call(5)` = 6. Pins that several closures coexist as one resource and the
           named `make` selects the right one.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (triple) (fn ((: x Int64)) (* x 3)))
              (export inc) (export triple)))
  (call   inc (: 5 Int64))
  (output (: 6 Int64)))

(case "a second same-signature closure export shares the one call method"
  (doc    "The SAME two-export program, now calling `triple`: `make-triple()` then the SHARED `call(5)` =
           15. The single `call` dispatches `(* x 3)` here and `(+ x 1)` above — proving one `call` serves
           every same-signature export (the code slot travels in the resource rep, recovered per call).")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (triple) (fn ((: x Int64)) (* x 3)))
              (export inc) (export triple)))
  (call   triple (: 5 Int64))
  (output (: 15 Int64)))

(case "a multi-export set of parameterized capturing closures is driven per export"
  (doc    "Three closure exports that each CAPTURE their param: `add` (+ x k), `mul` (* x k), `sub` (- x k),
           all `(Int64) -> (-> Int64 Int64)`. Calling `mul` drives `make-mul(4)` (capturing k=4) then
           `call(5)` = 20. Pins that make-forwarding (the captured param) composes with multi-export: each
           `make-<name>` forwards its own export's parameter into its own cell, and the shared `call` reads
           whichever capture the handle carries.")
  (input  (do (def (add (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (mul (: k Int64)) (fn ((: x Int64)) (* x k)))
              (def (sub (: k Int64)) (fn ((: x Int64)) (- x k)))
              (export add) (export mul) (export sub)))
  (call   mul (: 4 Int64) (: 5 Int64))
  (output (: 20 Int64)))

; THE ROUND-TRIP (C-HOST-4, Direction 2) — the host produces a closure from one export and hands it BACK
; into another. A PRODUCER export's result is a closure (`make-adder : (Int64) -> (-> Int64 Int64)`); a
; CONSUMER export takes that closure as a PARAMETER (`apply-it : ((-> Int64 Int64), Int64) -> Int64`) and
; applies it. Both cross as ONE resource type: the producer mints a handle (`resource.new`), the consumer
; recovers it (`resource.rep`) and dispatches via the guest's own `call_indirect` — the closure logic never
; leaves Cadenza; the host is a CUSTODIAN threading an opaque handle from one call to the next. The corpus
; `(call <consumer> <producer-args…> <consumer-args…>)` names the consumer; the driver calls the sole
; PRODUCER with the leading args (its params), then the consumer with the produced handle + the rest. So
; `(call apply-it 10 5)` runs `make-adder(10)` → a handle, then `apply-it(handle, 5)` = 5 + 10 = 15. This is
; the missing half of first-class functions: a Cadenza closure the host stores and drives, handed back as a
; callback. (A CONSUMER-ONLY program — the host fabricating a Cadenza closure — stays out of scope: it
; declines, since no producer mints the handle.)

(case "a produced closure is handed back into a consumer export (the round trip)"
  (doc    "`(def (make-adder (: k Int64)) (fn (x) (+ x k)))` PRODUCES a closure capturing k; `(def (apply-it
           (: g (-> Int64 Int64)) (: x Int64)) (g x))` CONSUMES one. The host produces a handle from
           `make-adder(10)`, then threads it back into `apply-it(handle, 5)` = 5 + 10 = 15 — a closure
           crossing OUT of one export call and back IN to another, applied via the guest's own
           `call_indirect`. Pins host-as-custodian: the producer's `resource.new` handle is recovered by the
           consumer's `resource.rep` and dispatched.")
  (input  (do (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (export make-adder) (export apply-it)))
  (call   apply-it (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

; The same round trip with a different capture and argument — the handle genuinely carries the per-produce
; captured environment across the boundary and back, and the consumer's dispatch reads it.

(case "the round trip tracks the produced closure's captured value"
  (doc    "`make-adder(100)` produces a closure capturing k=100; `apply-it(handle, 7)` = 7 + 100 = 107. A
           different capture (100) and consumer argument (7) — the result follows both, so the captured
           environment rides in the handle the host hands back, not in any shared state.")
  (input  (do (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (export make-adder) (export apply-it)))
  (call   apply-it (: 100 Int64) (: 7 Int64))
  (output (: 107 Int64)))

; A consumer that does MORE than apply the closure once — it applies it and adds a constant — showing the
; consumer body is ordinary Cadenza code with the handed-back closure as a first-class value in it.

(case "a consumer applies the handed-back closure inside a larger expression"
  (doc    "`(def (twice-plus (: g (-> Int64 Int64)) (: x Int64)) (+ (g x) (g x)))` applies the handed-back
           closure TWICE and sums. With `make-adder(1)` producing `(+ x 1)`, `twice-plus(handle, 5)` =
           (5+1) + (5+1) = 12. Pins that the consumer body is ordinary code — the closure param is a
           first-class value it may apply more than once (the `own<t>` handle serves the whole consumer
           call; it is consumed once, at the boundary, not per in-body application).")
  (input  (do (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (twice-plus (: g (-> Int64 Int64)) (: x Int64)) (+ (g x) (g x)))
              (export make-adder) (export twice-plus)))
  (call   twice-plus (: 1 Int64) (: 5 Int64))
  (output (: 12 Int64)))

; WIDER SCALAR WIDTHS — a closure's `call` boundary crosses EVERY aliased-width scalar the ordinary export
; boundary supports, not just the u32/s64/bool/f64 the value-heap runtime ops model. The closure functype
; is a plain component functype (component primitive byte via `comp_valtype_of` + core valtype via
; `valtype_of`), independent of the runtime-op ABI table — so `(-> Int32 Int32)`, `(-> UInt64 UInt64)`,
; `(-> Int8 Int8)`, a `Float32` closure, and a mixed-width `(-> Int32 Bool)` all cross and dispatch.

(case "a 32-bit-integer closure crosses the host boundary"
  (doc    "`(fn (x) (+ x 1))` at `(-> Int32 Int32)` — the closure's arg and result cross as the component
           `s32` primitive (core i32), narrower than the s64 the value-heap ops use. `call(5)` = 6. Pins
           that a 32-bit closure signature crosses the `call` boundary (the boundary byte comes from
           `comp_valtype_of`, wider than the runtime-op ABI table).")
  (input  (do (def (main) (fn ((: x Int32)) (+ x 1))) (export main)))
  (call   main (: 5 Int32))
  (output (: 6 Int32)))

(case "a 64-bit-unsigned closure crosses the host boundary"
  (doc    "`(fn (x) (* x 2))` at `(-> UInt64 UInt64)` — crosses as the component `u64` primitive. `call(21)`
           = 42. Pins the UNSIGNED 64-bit width (distinct from the signed s64 the runtime ops model).")
  (input  (do (def (main) (fn ((: x UInt64)) (* x 2))) (export main)))
  (call   main (: 21 UInt64))
  (output (: 42 UInt64)))

(case "an 8-bit-integer closure crosses the host boundary"
  (doc    "`(fn (x) (- x 1))` at `(-> Int8 Int8)` — the narrowest aliased width, crossing as component `s8`
           (core i32). `call(10)` = 9. Pins that a narrow width crosses (the runtime-op ABI table has no s8,
           but the closure functype does not need it).")
  (input  (do (def (main) (fn ((: x Int8)) (- x 1))) (export main)))
  (call   main (: 10 Int8))
  (output (: 9 Int8)))

(case "a 32-bit-float closure crosses the host boundary"
  (doc    "`(fn (x) (+. x 1.5))` at `(-> Float32 Float32)` — crosses as component `f32` (core f32), narrower
           than the f64 the runtime ops use. `call(2.5)` = 4.0. Pins the 32-bit float width.")
  (input  (do (def (main) (fn ((: x Float32)) (+. x 1.5))) (export main)))
  (call   main (: 2.5 Float32))
  (output (: 4.0 Float32)))

(case "a capturing 32-bit-integer closure crosses and is called"
  (doc    "`(def (adder (: k Int32)) (fn (x) (+ x k)))` — a capturing closure at the narrower Int32 width.
           `make(100)` then `call(7)` = 107. Pins that make-forwarding + the captured cell compose with a
           widened scalar width, exactly as at Int64.")
  (input  (do (def (adder (: k Int32)) (fn ((: x Int32)) (+ x k))) (export adder)))
  (call   adder (: 100 Int32) (: 7 Int32))
  (output (: 107 Int32)))

(case "a UInt64 closure round-trips through a consumer export"
  (doc    "The round trip at a widened width: `(def (make-adder (: k UInt64)) (fn (x) (+ x k)))` produces a
           `(-> UInt64 UInt64)` closure; `(def (apply-it (: g (-> UInt64 UInt64)) (: x UInt64)) (g x))`
           consumes one. `make-adder(100)` → a handle → `apply-it(handle, 7)` = 107. Pins that the
           producer/consumer boundary (own<t> + resource.rep dispatch) crosses a non-Int64 scalar width.")
  (input  (do (def (make-adder (: k UInt64)) (fn ((: x UInt64)) (+ x k)))
              (def (apply-it (: g (-> UInt64 UInt64)) (: x UInt64)) (g x))
              (export make-adder) (export apply-it)))
  (call   apply-it (: 100 UInt64) (: 7 UInt64))
  (output (: 107 UInt64)))

; A CONSUMER whose closure parameter is NOT FIRST — the consumer's component functype follows SOURCE order,
; so a scalar-then-closure `(def (app (: x Int64) (: g (-> Int64 Int64))) (g x))` crosses as `app : (s64,
; own<t>) -> s64`, not a closure-first shape. (An earlier cut hardcoded the closure as the first param and
; emitted an INVALID component when it wasn't; the functype now mirrors the params in order.) The driver
; threads the produced handle into the closure position and the scalar into its position.

(case "a consumer takes the handed-back closure as its SECOND parameter"
  (doc    "`(def (mk) (fn (x) (+ x 1)))` produces the closure; `(def (app (: x Int64) (: g (-> Int64
           Int64))) (g x))` takes a scalar `x` FIRST, then the closure `g`. `mk()` → a handle, then
           `app(5, handle)` = `(g 5)` = 6. Pins that the consumer's boundary functype follows source
           param order (closure not required to be first).")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 1)))
              (def (app (: x Int64) (: g (-> Int64 Int64))) (g x))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 6 Int64)))

; A consumer taking MORE THAN ONE closure parameter — both of the same signature, so both cross as
; `own<t>` of the ONE resource type. The host produces a fresh handle per closure param (own<t> is consumed
; per call) and threads each into its position. `(def (app2 (: f …) (: g …) (: x Int64)) (+ (f x) (g x)))`.

(case "a consumer applies TWO handed-back closures"
  (doc    "`(def (app2 (: f (-> Int64 Int64)) (: g (-> Int64 Int64)) (: x Int64)) (+ (f x) (g x)))` takes
           TWO closures + a scalar. With `mk` producing `(+ x 1)`, the host produces two handles and calls
           `app2(h1, h2, 5)` = (5+1) + (5+1) = 12. Pins that several closure params of the same signature
           cross as own<t> of the one resource type, each threaded independently.")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 1)))
              (def (app2 (: f (-> Int64 Int64)) (: g (-> Int64 Int64)) (: x Int64)) (+ (f x) (g x)))
              (export mk) (export app2)))
  (call   app2 (: 5 Int64))
  (output (: 12 Int64)))

; A consumer whose RESULT type differs from the closure's — the consumer functype's result is the
; CONSUMER's own result (`Bool` here), not the applied closure's (`Int64`). `(def (is-pos (: g …) (: x
; Int64)) (> (g x) 0))` returns Bool.

(case "a consumer returns a different type than the closure it applies"
  (doc    "`(def (is-pos (: g (-> Int64 Int64)) (: x Int64)) (> (g x) 0))` applies an `(-> Int64 Int64)`
           closure but RETURNS `Bool`. With `mk` producing `(+ x 1)`, `is-pos(handle, 5)` = (6 > 0) = true.
           Pins that the consumer's boundary result byte is the CONSUMER's result type, not the closure's.")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 1)))
              (def (is-pos (: g (-> Int64 Int64)) (: x Int64)) (> (g x) 0))
              (export mk) (export is-pos)))
  (call   is-pos (: 5 Int64))
  (output (: true Bool)))

; DISTINCT-SIGNATURE multi-export — a program exporting closures of DIFFERENT signatures crosses as one
; resource type PER signature. `inc : (-> Int64 Int64)` and `isz : (-> Int64 Bool)` become resources `t0`
; and `t1`, each with its own `make-<name>` + `call-g<n>` (the group's shared call). The host picks a
; closure export by name; the driver calls `make-<name>` → a handle, then the `call-g<n>` whose `self`
; resource type matches. Each group gets its own `resource.new`/`resource.rep` intrinsics (a core
; `resource.new` is typed to ONE resource); both closures still share the guest funcref table.

(case "one of two DIFFERENT-signature closure exports is made and called"
  (doc    "`(def (inc) (fn (x) (+ x 1)))` is `(-> Int64 Int64)` and `(def (isz) (fn (x) (= x 0)))` is
           `(-> Int64 Bool)` — DIFFERENT signatures, so they cross as two resource types. Calling `inc`
           drives `make-inc()` (resource t0) then its `call`(5) = 6. Pins that distinct signatures each get
           their own resource type + make/call, published in one interface.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (export inc) (export isz)))
  (call   inc (: 5 Int64))
  (output (: 6 Int64)))

(case "the second distinct-signature closure export returns its own type"
  (doc    "The SAME two-export program, now calling `isz` (resource t1, a `(-> Int64 Bool)` closure):
           `make-isz()` then its `call`(0) = true. The `isz` group's `call` returns Bool, distinct from
           `inc`'s Int64 — proving the two resource types carry independent signatures and results.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (export inc) (export isz)))
  (call   isz (: 0 Int64))
  (output (: true Bool)))

(case "three distinct closure signatures cross as three resource types"
  (doc    "`inc : (-> Int64 Int64)`, `isz : (-> Int64 Bool)`, `dbl : (-> Int64 Int64)` — note `inc` and
           `dbl` SHARE a signature (one resource type, two makes), while `isz` is distinct (its own).
           Calling `dbl`(7) = 14 exercises the shared-signature group alongside the distinct one. Pins that
           grouping-by-signature composes: same-signature exports share a resource, distinct ones don't.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (dbl) (fn ((: x Int64)) (* x 2)))
              (export inc) (export isz) (export dbl)))
  (call   dbl (: 7 Int64))
  (output (: 14 Int64)))

; DISTINCT-SIGNATURE composed with MULTI-ARG and CAPTURE — the grouping-by-signature path (each signature
; its own resource type) composes with the arity/capture machinery, no new compiler work. `add : (-> Int64
; (-> Int64 Int64))` (two args) and `isz : (-> Int64 Bool)` are distinct signatures → two resource types;
; `add`'s `call` takes both args. And two CAPTURING producers of distinct signatures (`adder`/`eq`) each
; forward their captured param through their own resource.

(case "a multi-argument closure among distinct-signature exports"
  (doc    "`(def (add) (fn (a b) (+ a b)))` is `(-> Int64 (-> Int64 Int64))` (two-arg) and `(def (isz) (fn
           (x) (= x 0)))` is `(-> Int64 Bool)` — distinct signatures, two resource types. Calling `add`
           drives `make-add()` then its `call(3, 4)` = 7 — the two-arg `call` on its own resource, alongside
           the distinct `isz`. Pins that multi-arg composes with distinct-signature grouping.")
  (input  (do (def (add) (fn ((: a Int64) (: b Int64)) (+ a b)))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (export add) (export isz)))
  (call   add (: 3 Int64) (: 4 Int64))
  (output (: 7 Int64)))

(case "distinct-signature capturing producers"
  (doc    "`(def (adder (: k Int64)) (fn (x) (+ x k)))` → `(-> Int64 Int64)` and `(def (eq (: k Int64)) (fn
           (x) (= x k)))` → `(-> Int64 Bool)` — distinct signatures, both CAPTURING their `k`. Calling `eq`
           drives `make-eq(5)` (capturing k=5) then its `call(5)` = true. Pins that make-param capture rides
           through the per-signature resource, distinct from `adder`'s.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (eq (: k Int64)) (fn ((: x Int64)) (= x k)))
              (export adder) (export eq)))
  (call   eq (: 5 Int64) (: 5 Int64))
  (output (: true Bool)))

; ROUND-TRIP composed with MULTI-ARG and a WIDENED width — the producer/consumer path is arity- and
; width-agnostic (the consumer's `call_indirect` dispatches the guest lifted body over the ONE table).

(case "a multi-argument closure round-trips through a consumer"
  (doc    "`(def (mk) (fn (a b) (+ a b)))` produces a two-arg `(-> Int64 (-> Int64 Int64))` closure; `(def
           (app (: g (-> Int64 (-> Int64 Int64))) (: a Int64) (: b Int64)) (g a b))` applies it with BOTH
           args. The host `mk()` → a handle → `app(handle, 3, 4)` = 7. Pins that the round trip threads a
           MULTI-ARG closure back (the consumer's dispatch pushes both args).")
  (input  (do (def (mk) (fn ((: a Int64) (: b Int64)) (+ a b)))
              (def (app (: g (-> Int64 (-> Int64 Int64))) (: a Int64) (: b Int64)) (g a b))
              (export mk) (export app)))
  (call   app (: 3 Int64) (: 4 Int64))
  (output (: 7 Int64)))

(case "a round-trip at a widened scalar width (UInt32)"
  (doc    "The round trip at UInt32, not Int64: `(def (mk (: k UInt32)) (fn (x) (+ x k)))` produces a `(->
           UInt32 UInt32)` closure; `(def (app (: g (-> UInt32 UInt32)) (: x UInt32)) (g x))` applies it.
           `mk(100)` → a handle → `app(handle, 7)` = 107. Pins that the producer/consumer boundary crosses
           a widened scalar width (own<t> + resource.rep dispatch is width-agnostic).")
  (input  (do (def (mk (: k UInt32)) (fn ((: x UInt32)) (+ x k)))
              (def (app (: g (-> UInt32 UInt32)) (: x UInt32)) (g x))
              (export mk) (export app)))
  (call   app (: 100 UInt32) (: 7 UInt32))
  (output (: 107 UInt32)))

; STRESS the multi-export paths at higher fan-out — THREE distinct signatures (three resource types, one
; with a narrower width) and FOUR same-signature exports (one resource, four makes sharing the call) — plus
; a consumer whose ONLY use of the handed-back closure is to apply it to an INTERNAL constant. Adversarial
; witnesses that the grouping/sharing machinery holds past the two-export cases above.

(case "three distinct closure signatures cross as three resource types"
  (doc    "`p : (-> Int64 Int64)`, `q : (-> Int64 Bool)`, `r : (-> Int32 Int32)` — THREE distinct
           signatures (note `r`'s narrower Int32 width) → three resource types. Calling `r` drives its
           `make`+`call(5)` = 10. Pins that grouping-by-signature scales past two groups and mixes widths.")
  (input  (do (def (p) (fn ((: x Int64)) (+ x 1)))
              (def (q) (fn ((: x Int64)) (= x 0)))
              (def (r) (fn ((: x Int32)) (* x 2)))
              (export p) (export q) (export r)))
  (call   r (: 5 Int32))
  (output (: 10 Int32)))

(case "four same-signature closure exports share one resource"
  (doc    "`a`,`b`,`cc`,`dd` are all `(-> Int64 Int64)` → ONE resource type with four `make-<name>`s sharing
           the one `call`. Calling `cc` drives `make-cc()` then the shared `call(10)` = 13. Pins that the
           shared-call multi-export scales past two same-signature exports.")
  (input  (do (def (a) (fn ((: x Int64)) (+ x 1)))
              (def (b) (fn ((: x Int64)) (+ x 2)))
              (def (cc) (fn ((: x Int64)) (+ x 3)))
              (def (dd) (fn ((: x Int64)) (+ x 4)))
              (export a) (export b) (export cc) (export dd)))
  (call   cc (: 10 Int64))
  (output (: 13 Int64)))

(case "a consumer applies the handed-back closure to an internal constant"
  (doc    "`(def (app (: g (-> Int64 Int64))) (g 99))` — the consumer takes ONLY a closure param and applies
           it to a fixed 99 (no scalar param of its own). With `mk(1)` producing `(+ x 1)`, the host `mk(1)`
           → a handle → `app(handle)` = (g 99) = 99 + 1 = 100. Pins a consumer whose sole boundary param is
           the closure (the arg it applies is internal, not a boundary scalar).")
  (input  (do (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (app (: g (-> Int64 Int64))) (g 99))
              (export mk) (export app)))
  (call   app (: 1 Int64))
  (output (: 100 Int64)))

; CLOSURE BODY RICHNESS — the boundary machinery is agnostic to what the closure's body DOES; these witness
; body constructs (a `match`, a multi-binding `let`, several captures + args at once) crossing and
; dispatching correctly, a dimension distinct from the arity/capture/multi-export shapes above.

(case "an escaping closure captures two values and takes three arguments"
  (doc    "`(def (main (: k Int64)) (fn (a b c) (+ (+ (+ a b) c) k)))` — the export param `k` is captured
           while the closure takes THREE args. `make(100)` (capturing k=100) then `call(1, 2, 3)` = 1 + 2 +
           3 + 100 = 106. Pins capture composing with a 3-arg call.")
  (input  (do (def (main (: k Int64)) (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ (+ a b) c) k))) (export main)))
  (call   main (: 100 Int64) (: 1 Int64) (: 2 Int64) (: 3 Int64))
  (output (: 106 Int64)))

(case "an escaping closure whose body is a match hits the literal arm"
  (doc    "`(fn (x) (match x (0 100) (_ x)))` — the closure body is a `match`. `call(0)` takes the literal
           arm → 100. Pins that a control-flow body (`match`) lowers and dispatches through the closure
           boundary.")
  (input  (do (def (main) (fn ((: x Int64)) (match x (0 100) (_ x)))) (export main)))
  (call   main (: 0 Int64))
  (output (: 100 Int64)))

(case "an escaping closure whose body is a match hits the wildcard arm"
  (doc    "The same match-bodied closure, `call(5)` → the wildcard arm → 5. Pins both arms of the closure's
           `match` dispatch across the boundary.")
  (input  (do (def (main) (fn ((: x Int64)) (match x (0 100) (_ x)))) (export main)))
  (call   main (: 5 Int64))
  (output (: 5 Int64)))

(case "an escaping closure whose body binds a multi-variable let"
  (doc    "`(def (main (: k Int64)) (fn (x) (let ((a (* x 2)) (b (+ x k))) (+ a b))))` — the body binds two
           locals (one using the captured `k`) then sums. `make(10)` then `call(5)` = (5*2) + (5+10) = 10 +
           15 = 25. Pins a multi-binding `let` body composing with capture.")
  (input  (do (def (main (: k Int64)) (fn ((: x Int64)) (let ((a (* x 2)) (b (+ x k))) (+ a b)))) (export main)))
  (call   main (: 10 Int64) (: 5 Int64))
  (output (: 25 Int64)))

; SOUNDNESS: distinct component signatures that COLLAPSE to the same CORE valtype shape. `a : (-> Int64
; Int64)` and `b : (-> Int64 UInt64)` are DISTINCT at the component boundary (s64 vs u64 result) — two
; resource types — yet both lower to the SAME core functype `(i32 env, i64) -> i64`. Each must still
; dispatch its OWN lifted body: the code slot rides in the resource rep (make-a → a t0 handle whose cell
; points at a's slot; make-b → a t1 handle at b's slot), recovered per call, so the shared core functype
; index is immaterial to WHICH body runs. If the two ever collided, `b` would run `a`'s body.

(case "distinct signatures sharing a core valtype shape dispatch distinct bodies"
  (doc    "`a : (-> Int64 Int64)` returns `x + 1000`; `b : (-> Int64 UInt64)` returns `x * 7` — distinct
           component signatures (s64 vs u64 result) but the SAME core shape `(i64) -> i64`. Calling `a(3)`
           = 1003 runs a's body. Pins that a's resource + slot dispatch its own code despite the shared
           core functype.")
  (input  (do (def (a) (fn ((: x Int64)) (+ x 1000)))
              (def (b) (fn ((: x Int64)) (UInt64.wrap (* x 7))))
              (export a) (export b)))
  (call   a (: 3 Int64))
  (output (: 1003 Int64)))

(case "the same-core-shape sibling dispatches ITS body, not the first"
  (doc    "The same program, calling `b(3)` = 21 = `x * 7` (b's OWN body), NOT 1003 (a's). Pins the
           soundness property: two closures whose core functypes are identical still run distinct code,
           because the code slot is recovered from the resource rep at call time — a mispick would surface
           here as b returning a's result.")
  (input  (do (def (a) (fn ((: x Int64)) (+ x 1000)))
              (def (b) (fn ((: x Int64)) (UInt64.wrap (* x 7))))
              (export a) (export b)))
  (call   b (: 3 Int64))
  (output (: 21 UInt64)))

; ROUND-TRIP CONSUMER BODY RICHNESS — a consumer's body is ordinary Cadenza code, and the handed-back
; closure is a first-class value in it that may be applied CONDITIONALLY (an `if`/`match` branch that does
; NOT apply it on every path) or bound through a `let`. This exercises a correctness property: the consumer
; wrapper `resource.rep`s the handle → cell and DROPs the cell (own<t> release) around the body call —
; sound even when the body never dispatches the closure on the taken path (the cell is still reclaimed).

(case "a round-trip consumer applies the closure only in the taken if-branch"
  (doc    "`(def (app (: g (-> Int64 Int64)) (: x Int64)) (if (< x 0) 0 (g x)))` — applies `g` only when x
           ≥ 0. `mk()` + `app(handle, 5)` = (g 5) = 6. Pins that a consumer applies the handed-back closure
           inside control flow.")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (if (< x 0) 0 (g x)))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 6 Int64)))

(case "a round-trip consumer that does NOT apply the closure on the taken branch"
  (doc    "The same consumer, `app(handle, -3)` = 0 — the guarded branch is taken and `g` is NEVER applied.
           Pins the release soundness: the wrapper still `resource.rep`s + DROPs the handed-back cell even
           though the body did not dispatch it (own<t> is consumed at the boundary regardless).")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (if (< x 0) 0 (g x)))
              (export mk) (export app)))
  (call   app (: -3 Int64))
  (output (: 0 Int64)))

(case "a round-trip consumer binds the applied closure through a let"
  (doc    "`(def (app (: g (-> Int64 Int64)) (: x Int64)) (let ((y (g x))) (+ y 1)))` — `mk` multiplies by
           10, so `app(handle, 4)` = (g 4) + 1 = 40 + 1 = 41. Pins a `let`-bound application in a consumer.")
  (input  (do (def (mk) (fn ((: x Int64)) (* x 10)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (let ((y (g x))) (+ y 1)))
              (export mk) (export app)))
  (call   app (: 4 Int64))
  (output (: 41 Int64)))

(case "a round-trip consumer applies the closure in a match wildcard arm"
  (doc    "`(def (app (: g (-> Int64 Int64)) (: x Int64)) (match x (0 999) (_ (g x))))` — `mk` adds 100.
           `app(handle, 5)` takes the wildcard → (g 5) = 105; `app(handle, 0)` takes the literal arm → 999,
           NOT applying `g`. Pins a `match`-dispatched consumer, applying the closure only in one arm.")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 100)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (match x (0 999) (_ (g x))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 105 Int64)))

(case "a round-trip consumer takes the non-applying match arm"
  (doc    "The same match-bodied consumer, `app(handle, 0)` = 999 — the literal arm, `g` NOT applied.
           Confirms the handed-back cell is still released when the body's taken path skips the closure.")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 100)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (match x (0 999) (_ (g x))))
              (export mk) (export app)))
  (call   app (: 0 Int64))
  (output (: 999 Int64)))

; THE DISTINCT-SIGNATURE ROUND-TRIP — the flagship shape unified: a program that both PRODUCES and CONSUMES
; closures of DIFFERENT signatures. Each signature is its own resource type; a producer mints its closure
; and the matching consumer (paired by resource type) applies it. Here `adder`+`appa` work with `(-> Int64
; Int64)` (resource t0) and `isz`+`appb` with `(-> Int64 Bool)` (resource t1), all in one component. The
; host produces from the producer whose result resource type matches the consumer's closure param, then
; threads the handle in. This composes the round-trip (host-as-custodian) with N-resource-type grouping.

(case "a distinct-signature round-trip applies the Int64->Int64 closure"
  (doc    "`adder : (Int64) -> (-> Int64 Int64)` + `appa : ((-> Int64 Int64), Int64) -> Int64` (resource
           t0), alongside `isz` + `appb` on `(-> Int64 Bool)` (resource t1). Calling `appa` produces from
           `adder(10)` (its matching producer, by resource type) → a handle → `appa(handle, 5)` = 15. Pins
           that a round trip mixing signatures pairs each consumer with the producer of its resource type.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
              (export adder) (export appa) (export isz) (export appb)))
  (call   appa (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

(case "a distinct-signature round-trip applies the Int64->Bool closure"
  (doc    "The same four-export program, now calling `appb` (the `(-> Int64 Bool)` consumer, resource t1):
           produced from `isz()` → a handle → `appb(handle, 0)` = true. The Bool-signature closure round-
           trips through its OWN resource type, distinct from the Int64 one.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
              (export adder) (export appa) (export isz) (export appb)))
  (call   appb (: 0 Int64))
  (output (: true Bool)))

(case "a distinct-signature round-trip's Bool closure on a nonzero input"
  (doc    "The same program, `appb(handle, 5)` = false (5 ≠ 0) — the t1 closure's result tracks its input,
           distinct from the t0 group. Confirms both resource types dispatch their own closures.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
              (export adder) (export appa) (export isz) (export appb)))
  (call   appb (: 5 Int64))
  (output (: false Bool)))

; NON-KEBAB EXPORT NAMES — a component-model extern name MUST be kebab-case, but a Cadenza source
; identifier may be camelCase or snake_case (`mkA`, `appA`, `makeAdder`). Every PUBLIC closure-interface
; export name (`make-<src>`, a consumer's own name, `make-<src>` in a multi-export) is normalized at emit
; through `kebab_extern_name` (the same rule a bare scalar export uses); the private per-func wiring names
; are index-derived (`import-func-f<n>`) so a source name never leaks into them. The runner resolves the
; caller's SOURCE name through the SAME rule, so `(call appA …)` still finds the `app-a` export. These pins
; guard the boundary-name normalization end-to-end (a camelCase program used to emit an invalid component).

(case "a camelCase round-trip resolves through kebab boundary-name normalization"
  (doc    "`mkA : (Int64) -> (-> Int64 Int64)` + `appA : ((-> Int64 Int64), Int64) -> Int64`. In a round
           trip a producer is exported under its OWN name, so the public exports emit as `mk-a`/`app-a`
           (kebab); calling `appA` produces from `mkA(10)` → a handle → `appA(handle, 5)` = 15. Pins that a
           camelCase closure round-trip emits a VALID component and the runner still resolves the source
           name.")
  (input  (do (def (mkA (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (appA (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (export mkA) (export appA)))
  (call   appA (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

(case "a camelCase same-signature multi-export normalizes each make-<name>"
  (doc    "Two same-signature closure exports with camelCase names: `makeAdder(k)` = `x + k`, `makeScaler(k)`
           = `x * k`. They share ONE resource type + `call`; each `make-<src>` public name is kebabized
           (`make-make-adder`/`make-make-scaler`). `(call makeScaler 3 4)` → `makeScaler(3)` → a handle →
           `call(handle, 4)` = 4 * 3 = 12. Pins multi-export public-name normalization.")
  (input  (do (def (makeAdder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (makeScaler (: k Int64)) (fn ((: x Int64)) (* x k)))
              (export makeAdder) (export makeScaler)))
  (call   makeScaler (: 3 Int64) (: 4 Int64))
  (output (: 12 Int64)))

; A CLOSURE EXPORT ALONGSIDE A NON-CLOSURE (PLAIN) EXPORT — a MIXED multi-export. The closure(s) cross via
; the resource envelope (`make-<name>` + a shared `call`, under `cadenza:closure/exports`); each plain export
; is aliased off the SAME program instance and published as an ORDINARY top-level component func. Both live
; in ONE component: the host reaches the plain export as a bare func, the closure through `make`/`call`. The
; `oracle_mixed_component` byte anchor proved the resource-instance + top-level-func coexistence. Scope: the
; closure exports share ONE signature; each plain export has an aliased-scalar param/result (a compound plain
; result is a later widening). `cdz-run` routes `(call <plain>)` to the bare func and `(call <closure>)` to
; make/call — a plain export whose name resolves to a top-level func stays on the plain path.

(case "a closure export alongside a plain scalar export — the plain export runs"
  (doc    "`inc : () -> (-> Int64 Int64)` (a closure factory) is exported ALONGSIDE `two : () -> Int64` (a
           plain scalar). `(call two)` reaches the ORDINARY top-level `two` func → 2, unaffected by the
           closure interface riding alongside it. Pins that a plain export coexists with a closure export and
           the host drives it directly.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (two) 2)
              (export inc) (export two)))
  (call   two)
  (output (: 2 Int64)))

(case "a closure export alongside a plain scalar export — the closure runs"
  (doc    "The SAME mixed program, now calling the CLOSURE export `inc`: the host `make`s a handle then
           `call(handle, 5)` = 6, dispatched through the guest's `call_indirect`. Pins that the closure
           interface still works when a plain export shares the component (both envelopes composed).")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (two) 2)
              (export inc) (export two)))
  (call   inc (: 5 Int64))
  (output (: 6 Int64)))

(case "a parameterized plain export alongside a closure export applies its argument"
  (doc    "`adder : (Int64) -> (-> Int64 Int64)` (a capturing closure factory) alongside `dbl : (Int64) ->
           Int64` (a plain function that doubles). `(call dbl 21)` reaches the top-level `dbl` → 42 — a plain
           export with a PARAMETER rides alongside the closure make/call. Pins the non-nullary plain export.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (dbl (: n Int64)) (* n 2))
              (export adder) (export dbl)))
  (call   dbl (: 21 Int64))
  (output (: 42 Int64)))

(case "a parameterized plain export alongside a closure export — the closure captures and applies"
  (doc    "The SAME program, calling the capturing closure `adder`: `make(10)` builds a closure over k=10,
           then `call(handle, 5)` = 15. Confirms the capturing-closure make/call path is intact alongside a
           parameterized plain export.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (dbl (: n Int64)) (* n 2))
              (export adder) (export dbl)))
  (call   adder (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

(case "two same-signature closures alongside a plain export all coexist"
  (doc    "TWO same-signature closure exports (`inc`, `triple`) share ONE resource type + `call`, riding
           alongside a plain `answer : () -> 42`. `(call triple 5)` = 15 (the `* x 3` closure), proving the
           multi-closure shared-`call` dispatch is unaffected by the plain export in the same component.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (triple) (fn ((: x Int64)) (* x 3)))
              (def (answer) 42)
              (export inc) (export triple) (export answer)))
  (call   triple (: 5 Int64))
  (output (: 15 Int64)))

(case "two same-signature closures alongside a plain export — the plain export runs"
  (doc    "The SAME three-export program, calling the plain `answer` → 42. Pins that the plain export is
           reachable when TWO closures share the resource interface beside it.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (triple) (fn ((: x Int64)) (* x 3)))
              (def (answer) 42)
              (export inc) (export triple) (export answer)))
  (call   answer)
  (output (: 42 Int64)))

; DISTINCT-SIGNATURE closures ALONGSIDE a plain export — the distinct-sig case of the mixed shape. Closures
; of DIFFERENT signatures cross as N resource types (each its own `make-<name>`/`call-g<n>`), and a plain
; export rides alongside as an ordinary top-level func. The distinct-sig envelope now carries plain exports
; too (aliased off the same program instance after the closure fns, lifted + exported at the top level).
; `cdz-run` routes `(call <plain>)` to the top-level bare func and `(call <closure>)` to its group's
; make/call-g<n> (matched by resource type). Composes N-resource-type grouping with the plain boundary.

(case "distinct-signature closures alongside a plain export — the Int64->Int64 closure runs"
  (doc    "`inc : (-> Int64 Int64)` (resource t0) and `isz : (-> Int64 Bool)` (resource t1) cross as TWO
           resource types, alongside a plain `two : () -> 2`. Calling the closure `inc`: `make-inc()` → a
           handle → `call-g0(handle, 5)` = 6. Pins that distinct-sig grouping is unaffected by a plain
           export sharing the component.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (two) 2)
              (export inc) (export isz) (export two)))
  (call   inc (: 5 Int64))
  (output (: 6 Int64)))

(case "distinct-signature closures alongside a plain export — the Int64->Bool closure runs"
  (doc    "The SAME program, calling the OTHER-signature closure `isz` (resource t1): `make-isz()` → a
           handle → `call-g1(handle, 0)` = true. Confirms both resource types dispatch their own closures
           with a plain export present.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (two) 2)
              (export inc) (export isz) (export two)))
  (call   isz (: 0 Int64))
  (output (: true Bool)))

(case "distinct-signature closures alongside a plain export — the plain export runs"
  (doc    "The SAME program, calling the plain `two` → 2. Pins that the top-level plain export is reachable
           when TWO distinct resource types ride beside it in `cadenza:closure/exports`.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (two) 2)
              (export inc) (export isz) (export two)))
  (call   two)
  (output (: 2 Int64)))

(case "distinct-signature capturing closures alongside a parameterized plain export"
  (doc    "`adder : (Int64) -> (-> Int64 Int64)` (t0, captures k) and `gte : (Int64) -> (-> Int64 Bool)`
           (t1, captures a threshold) cross as two resource types beside a plain `dbl : (Int64) -> Int64`.
           `(call gte 3 5)` → `make-gte(3)` builds `(fn (x) (>= x 3))`, then `call-g1(handle, 5)` = true
           (5 >= 3). Composes distinct-sig capture with a parameterized plain export.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (gte (: t Int64)) (fn ((: x Int64)) (>= x t)))
              (def (dbl (: n Int64)) (* n 2))
              (export adder) (export gte) (export dbl)))
  (call   gte (: 3 Int64) (: 5 Int64))
  (output (: true Bool)))

(case "distinct-signature capturing closures alongside a parameterized plain export — the plain runs"
  (doc    "The SAME four-export program, calling the parameterized plain `dbl(21)` = 42. Pins the
           non-nullary plain export reachable beside two distinct capturing-closure resource types.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (gte (: t Int64)) (fn ((: x Int64)) (>= x t)))
              (def (dbl (: n Int64)) (* n 2))
              (export adder) (export gte) (export dbl)))
  (call   dbl (: 21 Int64))
  (output (: 42 Int64)))

; A ROUND-TRIP (produce + consume) ALONGSIDE a plain non-closure export. The producer mints a closure, the
; consumer takes it back and applies it, and a plain export rides alongside as an ordinary top-level func —
; all in ONE component. Before this, the round-trip path SILENTLY DROPPED a plain export (a valid component
; missing the name), a miscompile; now the plain body is aliased off the same program instance, lifted, and
; exported at the top level. `cdz-run` routes `(call <plain>)` to the bare func and `(call <consumer>)` to
; the round-trip (produce-then-consume).

(case "a round-trip alongside a plain export — the plain export runs"
  (doc    "`mk : () -> (-> Int64 Int64)` produces, `app : ((-> Int64 Int64), Int64) -> Int64` consumes, and
           a plain `two : () -> 2` rides alongside. `(call two)` reaches the ORDINARY top-level `two` func →
           2. Pins that a plain export is REACHABLE in a round-trip program (was silently dropped).")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (two) 2)
              (export mk) (export app) (export two)))
  (call   two)
  (output (: 2 Int64)))

(case "a round-trip alongside a plain export — the round-trip still works"
  (doc    "The SAME program, driving the ROUND-TRIP consumer `app`: the host produces a closure from `mk()`
           → a handle → `app(handle, 5)` = 6. Pins that the round-trip (produce-then-consume) is intact when
           a plain export shares the component.")
  (input  (do (def (mk) (fn ((: x Int64)) (+ x 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (two) 2)
              (export mk) (export app) (export two)))
  (call   app (: 5 Int64))
  (output (: 6 Int64)))

(case "a round-trip alongside a parameterized plain export applies its argument"
  (doc    "A capturing round trip — `adder : (Int64) -> (-> Int64 Int64)` produces, `app` consumes — beside a
           parameterized plain `dbl : (Int64) -> Int64`. `(call dbl 21)` = 42 reaches the top-level `dbl`.
           Pins a non-nullary plain export beside a capturing round trip.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (dbl (: n Int64)) (* n 2))
              (export adder) (export app) (export dbl)))
  (call   dbl (: 21 Int64))
  (output (: 42 Int64)))

; A DISTINCT-SIGNATURE ROUND-TRIP alongside a plain export. Producers + consumers of DIFFERENT signatures
; cross as N resource types, and a plain export rides alongside. Before this the distinct-sig round-trip
; DECLINED any non-producer/non-consumer export; now it carries plain exports as top-level funcs.

(case "a distinct-signature round-trip alongside a plain export — the Int64->Int64 side runs"
  (doc    "`adder`+`appa` on `(-> Int64 Int64)` (t0) and `isz`+`appb` on `(-> Int64 Bool)` (t1), beside a
           plain `two : () -> 2`. Driving `appa`: produce from `adder(10)` → a handle → `appa(handle, 5)` =
           15. Pins that distinct-sig round-trip grouping is intact with a plain export present.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
              (def (two) 2)
              (export adder) (export appa) (export isz) (export appb) (export two)))
  (call   appa (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

(case "a distinct-signature round-trip alongside a plain export — the Int64->Bool side runs"
  (doc    "The SAME five-export program, driving `appb` (the `(-> Int64 Bool)` side, t1): produce from
           `isz()` → a handle → `appb(handle, 0)` = true. Confirms both resource types round-trip with a
           plain export present.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
              (def (two) 2)
              (export adder) (export appa) (export isz) (export appb) (export two)))
  (call   appb (: 0 Int64))
  (output (: true Bool)))

(case "a distinct-signature round-trip alongside a plain export — the plain export runs"
  (doc    "The SAME five-export program, calling the plain `two` → 2. Pins that the top-level plain export is
           reachable when TWO distinct round-trip resource types share the component.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
              (def (two) 2)
              (export adder) (export appa) (export isz) (export appb) (export two)))
  (call   two)
  (output (: 2 Int64)))

; NOMINAL-over-scalar at the closure boundary. A single-variant nominal like `(type UserId (Mk Int64))`
; ERASES to its underlying scalar at run time (type-system.md §156 — the tag "adds nothing to the value's
; runtime representation"), so a closure whose arg or result is such a nominal crosses the `call` boundary
; as the underlying scalar (`UserId` → `s64`), the tag stripped. `closure_boundary_byte` peels the nominal
; (`strip_nominal`) to pick the boundary byte, and the core `call` functype uses the scalar valtype — so
; the host sends/receives a plain scalar and the nominal identity is a compile-time-only concern. These pin
; that the nominal is transparent at the boundary (the host sees the scalar, not a wrapper resource).

(case "a closure returning a nominal-over-scalar crosses as the underlying scalar"
  (doc    "`(type UserId (Mk Int64))` + `(fn (x) (Mk x))` — the closure result type is `UserId`, which
           erases to Int64. The `call` method's result functype is `s64` (the nominal peeled), so
           `call(handle, 42)` returns 42 rendered as the scalar. Pins that a nominal result is transparent
           at the host boundary — no wrapper resource, just the underlying scalar.")
  (input  (do (type UserId (Mk Int64)) (def (main) (fn ((: x Int64)) (Mk x))) (export main)))
  (call   main (: 42 Int64))
  (output (: 42 Int64)))

(case "a closure taking a nominal-over-scalar argument receives the underlying scalar"
  (doc    "`(fn (u) (+ (unwrap u) 1))` where `u : UserId` — the closure's ARG is a nominal, crossing as
           Int64. The host passes 7, the guest matches out the payload (`(Mk n) → n`), adds 1 → 8. Pins the
           nominal ARG side of the boundary (companion to the result case).")
  (input  (do (type UserId (Mk Int64))
              (def (unwrap (: u UserId)) (match u ((Mk n) n)))
              (def (main) (fn ((: u UserId)) (+ (unwrap u) 1)))
              (export main)))
  (call   main (: 7 Int64))
  (output (: 8 Int64)))

(case "a capturing closure returning a nominal-over-scalar"
  (doc    "`(def (tagger base) (fn (x) (Mk (+ x base))))` captures `base` and returns a `Tag` (nominal over
           Int64). `make(100)` builds a closure over base=100, then `call(handle, 5)` = Mk(105) → 105 at
           the boundary. Composes make-param capture with a nominal result.")
  (input  (do (type Tag (Mk Int64))
              (def (tagger (: base Int64)) (fn ((: x Int64)) (Mk (+ x base))))
              (export tagger)))
  (call   tagger (: 100 Int64) (: 5 Int64))
  (output (: 105 Int64)))

(case "a round-trip consumer applies a closure whose result is a nominal-over-scalar"
  (doc    "Producer `mk : () -> (-> Int64 Tag)` mints a closure returning `Tag`; consumer `app` takes it
           back, applies it, matches out the payload and doubles it. `mk()` → a handle → `app(handle, 7)` =
           `(Mk 7)` → 14. Pins a nominal-result closure through the round trip (produce + consume).")
  (input  (do (type Tag (Mk Int64))
              (def (mk) (fn ((: x Int64)) (Mk x)))
              (def (app (: g (-> Int64 Tag)) (: x Int64)) (match (g x) ((Mk n) (* n 2))))
              (export mk) (export app)))
  (call   app (: 7 Int64))
  (output (: 14 Int64)))

(case "a closure returning a nominal-over-Bool erases to bool at the boundary"
  (doc    "`(type Flag (Mk Bool))` + `(fn (x) (Mk (> x 0)))` — a nominal over Bool, not Int. The `call`
           result crosses as `bool` (the peeled underlying type), so `call(handle, 5)` = Mk(true) → true.
           Confirms the nominal peel is width/kind-agnostic (Bool underlying, not only integers).")
  (input  (do (type Flag (Mk Bool)) (def (main) (fn ((: x Int64)) (Mk (> x 0)))) (export main)))
  (call   main (: 5 Int64))
  (output (: true Bool)))

; A COMPOUND-RESULT closure: the closure's result is a runtime `Bytes`, which crosses the `call` boundary
; as `list<u8>` (the raw payload) rather than a scalar. Unlike a scalar `call`, the emitted core carries a
; MEMORY + `cabi_realloc`, and `call` — after dispatching the lifted closure (which returns a runtime Bytes
; HANDLE) — runs a `bytes-len`/`bytes-get` copy loop writing the payload + the canonical `(ptr, len)` return
; area, then drops both the closure cell and the transient Bytes handle. The `call` is lifted with
; Memory/Realloc canon options (`assemble_closure_bytes_resource`), the shape the compound-result oracle
; proved runs. The host reads the bytes back directly (a bare `list<u8>`, rendered as the byte sequence).

(case "a closure returning Bytes crosses to the host as list<u8>"
  (doc    "`(fn (n) (bin (u8 n) (u8 n+1)))` — the closure's result is a runtime `Bytes`. `make()` → a
           handle; `call(handle, 5)` dispatches the closure (building `[5, 6]` on the value heap), and the
           `call` method copies that Bytes handle into linear memory and returns it as `list<u8>` — the host
           reads `(5 6)`. Pins the compound-result closure boundary end-to-end (memory + cabi_realloc +
           Memory/Realloc-lifted `call` + the bytes copy loop).")
  (input  (do (def (main) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (export main)))
  (call   main (: 5 Int64))
  (output (5 6)))

(case "a Bytes-returning closure on a different argument"
  (doc    "The same `(fn (n) (bin (u8 n) (u8 n+1)))`, called with 100 → the bytes `[100, 101]`. Confirms the
           copied payload tracks the closure's runtime input, not a fixed buffer.")
  (input  (do (def (main) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (export main)))
  (call   main (: 100 Int64))
  (output (100 101)))

(case "a capturing closure returning Bytes"
  (doc    "`(def (tag (: hdr Int64)) (fn (n) (bin (u8 hdr) (u8 n))))` captures a header byte and returns a
           2-byte `Bytes`. `make(9)` builds a closure over hdr=9, then `call(handle, 200)` → `[9, 200]`.
           Composes make-param capture with a compound (`Bytes`) closure result.")
  (input  (do (def (tag (: hdr Int64)) (fn ((: n Int64)) (bin (u8 (UInt8.wrap hdr)) (u8 (UInt8.wrap n)))))
              (export tag)))
  (call   tag (: 9 Int64) (: 200 Int64))
  (output (9 200)))

; A STRING closure result crosses the same way a `Bytes` one does. A `String` is a UTF-8 byte-rope handle,
; representationally IDENTICAL to `Bytes` (the same value-heap `bytes-*` store), so a closure returning a
; `String` takes the very same compound-result `call` path — its `call` copies the UTF-8 bytes into linear
; memory and returns them as `list<u8>` (the encoded bytes, not a decoded string). `emit_closure_resource`
; routes a `String` result to the bytes shape exactly as a `Bytes` result (`ret_is_bytes` accepts both).

(case "a closure returning a constant String crosses as its UTF-8 bytes"
  (doc    "`(fn (n) \"hi\")` — the closure's result is a `String`. `call(handle, 0)` copies the UTF-8 bytes
           of \"hi\" (`[104, 105]`) out through the canonical `list<u8>` ABI, and the host reads `(104 105)`.
           Pins that a `String` result crosses as its bytes on the same path as `Bytes` (a byte-rope handle
           is a byte-rope handle).")
  (input  (do (def (main) (fn ((: n Int64)) "hi")) (export main)))
  (call   main (: 0 Int64))
  (output (104 105)))

(case "a closure returning a runtime String (concat) crosses as its bytes"
  (doc    "`(fn (n) (String.concat \"ab\" \"c\"))` — a RUNTIME String built by `concat` (not a folded
           constant handle). `call(handle, 0)` → the UTF-8 bytes of \"abc\" = `[97, 98, 99]`. Confirms the
           bytes copy reads a genuine runtime byte-rope handle, not only a compile-time-known string.")
  (input  (do (def (main) (fn ((: n Int64)) ((. String concat) "ab" "c"))) (export main)))
  (call   main (: 0 Int64))
  (output (97 98 99)))

(case "a capturing closure returning a String"
  (doc    "`(def (mk k) (fn (n) (String.concat \"x\" \"y\")))` — a make-parameterized closure whose result
           is a String. `make(7)` builds it, then `call(handle, 0)` → the bytes of \"xy\" = `[120, 121]`.
           Composes make-param capture with a `String` closure result.")
  (input  (do (def (mk (: k Int64)) (fn ((: n Int64)) ((. String concat) "x" "y"))) (export mk)))
  (call   mk (: 7 Int64) (: 0 Int64))
  (output (120 121)))

; EMPTY byte-rope closure results — the copy loop must handle n=0 (empty Bytes / empty String). An empty
; compound crosses as an empty `list<u8>`, so the `call` writes a `(ptr, len=0)` return area and the host
; reads the empty list. Pins the boundary edge (a zero-length payload must not read a stray byte or trap).

(case "a closure returning an empty Bytes crosses as the empty list"
  (doc    "`(fn (n) (bin))` — an empty `Bytes`. `call(handle, 0)` copies zero bytes and returns
           `(ptr, len=0)`; the host reads `()`. Pins the n=0 edge of the bytes copy loop (a `bytes-len` of
           0 must skip the loop cleanly).")
  (input  (do (def (main) (fn ((: n Int64)) (bin))) (export main)))
  (call   main (: 0 Int64))
  (output ()))

(case "a closure returning an empty String crosses as the empty list"
  (doc    "The String companion: `(fn (n) \"\")` — an empty String (an empty UTF-8 byte-rope) crosses as the
           empty `list<u8>`. Confirms the n=0 edge on the String result path too.")
  (input  (do (def (main) (fn ((: n Int64)) "")) (export main)))
  (call   main (: 0 Int64))
  (output ()))

; MULTI-EXPORT byte-rope-result closures: N same-signature closures each returning a `Bytes`/`String` share
; ONE `call` that returns `list<u8>` — the multi-export shape (N `make-<name>` + one shared `call`) extended
; to the compound-result `call` (memory + cabi_realloc + the bytes copy loop). The shared `call` recovers the
; code slot from the rep, dispatches whichever closure the handle names, then copies its byte-rope result out.

(case "two same-signature Bytes-returning closures share one call — first"
  (doc    "`a : () -> (-> Int64 Bytes)` (1 byte) and `b` (2 bytes), same signature → ONE resource type + one
           shared list-returning `call`. `make-a()` → a handle; `call(handle, 5)` copies a's `[5]` out. Pins
           the multi-export byte-rope `call` (N makes, one shared memory/realloc list-`call`).")
  (input  (do (def (a) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)))))
              (def (b) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (export a) (export b)))
  (call   a (: 5 Int64))
  (output (5)))

(case "two same-signature Bytes-returning closures share one call — second"
  (doc    "The same program, driving `b`: `make-b()` → a handle; `call(handle, 5)` = `[5, 6]`. The SHARED
           `call` dispatches whichever closure the rep names (b's 2-byte body here), proving the shared
           list-`call` is not fixed to one make.")
  (input  (do (def (a) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)))))
              (def (b) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (export a) (export b)))
  (call   b (: 5 Int64))
  (output (5 6)))

(case "two same-signature String-returning closures share one call"
  (doc    "`greet` and `bye` both `() -> (-> Int64 String)` share one resource type + list-`call`. Driving
           `bye`: `call(handle, 0)` → the UTF-8 bytes of \"by\" = `[98, 121]`. Confirms the multi-export
           byte-rope `call` is agnostic to Bytes-vs-String (both are byte-rope handles).")
  (input  (do (def (greet) (fn ((: n Int64)) "hi"))
              (def (bye) (fn ((: n Int64)) "by"))
              (export greet) (export bye)))
  (call   bye (: 0 Int64))
  (output (98 121)))

; A BYTE-ROPE-result closure ALONGSIDE a PLAIN export — the mixed shape extended to the compound `call`.
; The closure's `Bytes`/`String` result crosses as `list<u8>` (the shared list-returning `call` with
; memory/cabi_realloc), and the plain export rides alongside as an ordinary top-level func. Both live in one
; component; `cdz-run` routes `(call <plain>)` to the bare func and `(call <closure>)` to make/call.

(case "a Bytes-returning closure alongside a plain export — the closure runs"
  (doc    "`mk : () -> (-> Int64 Bytes)` (returns `(bin (u8 n) (u8 n+1))`) alongside a plain `two : () -> 2`.
           `make()` → a handle; `call(handle, 5)` copies the closure's `[5, 6]` out as `list<u8>`. Pins the
           byte-rope closure result on the MIXED path (the compound `call` + a plain top-level export).")
  (input  (do (def (mk) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (def (two) 2)
              (export mk) (export two)))
  (call   mk (: 5 Int64))
  (output (5 6)))

(case "a Bytes-returning closure alongside a plain export — the plain runs"
  (doc    "The SAME mixed program, calling the plain `two` → 2. Pins that the plain top-level export is
           reachable when a compound-result closure shares the component.")
  (input  (do (def (mk) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (def (two) 2)
              (export mk) (export two)))
  (call   two)
  (output (: 2 Int64)))

(case "a String-returning closure alongside a parameterized plain export"
  (doc    "`greet : () -> (-> Int64 String)` returns \"hi\", alongside a plain `dbl : (Int64) -> Int64`.
           `call(greet-handle, 0)` → the UTF-8 bytes `[104, 105]`. Confirms a String-result closure + a
           parameterized plain export coexist.")
  (input  (do (def (greet) (fn ((: n Int64)) "hi"))
              (def (dbl (: x Int64)) (* x 2))
              (export greet) (export dbl)))
  (call   greet (: 0 Int64))
  (output (104 105)))

(case "a String-returning closure alongside a parameterized plain export — the plain runs"
  (doc    "The SAME program, calling `dbl(21)` = 42. Pins the parameterized plain export reachable beside a
           String-result closure.")
  (input  (do (def (greet) (fn ((: n Int64)) "hi"))
              (def (dbl (: x Int64)) (* x 2))
              (export greet) (export dbl)))
  (call   dbl (: 21 Int64))
  (output (: 42 Int64)))

; BYTE-ROPE result on the DISTINCT-SIGNATURE path — closures of DIFFERENT signatures each returning a
; `Bytes`/`String` cross as G distinct resource types, each with its OWN `call-<g>` that returns `list<u8>`
; (memory + cabi_realloc shared across groups). Extends the byte-rope compound `call` from the single/multi/
; mixed shapes to the N-resource-type shape. Also covers a byte-rope group coexisting with a SCALAR group in
; the same component (the scalar `call-<g>` returns by value; the byte-rope one via the copy loop).

(case "distinct-sig byte-rope closures — the Int64→Bytes one"
  (doc    "`mkb : () -> (-> Int64 Bytes)` (returns `(bin n n+1)`) and `mks : () -> (-> Bool Bytes)` cross as
           TWO distinct resource types (different arg types → distinct signatures), each with its own
           `list<u8>`-returning `call`. `call(mkb-handle, 5)` copies `[5,6]` out. Pins the byte-rope result
           on the distinct-signature path.")
  (input  (do (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (def (mks) (fn ((: b Bool)) (bin (u8 (if b 1 0)))))
              (export mkb) (export mks)))
  (call   mkb (: 5 Int64))
  (output (5 6)))

(case "distinct-sig byte-rope closures — the Bool→Bytes one"
  (doc    "The SAME two-resource program, driving the OTHER signature: `call(mks-handle, true)` → `[1]`.
           Confirms each distinct byte-rope resource dispatches its own closure body.")
  (input  (do (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (def (mks) (fn ((: b Bool)) (bin (u8 (if b 1 0)))))
              (export mkb) (export mks)))
  (call   mks (: true Bool))
  (output (1)))

(case "distinct-sig: a byte-rope closure coexists with a SCALAR closure — the byte-rope one"
  (doc    "`mkb : () -> (-> Int64 Bytes)` and `inc : () -> (-> Int64 Int64)` are distinct signatures → two
           resource types. The byte-rope group's `call` returns `list<u8>` (memory + realloc); the scalar
           group's returns by value. `call(mkb-handle, 9)` → `[9,10]`. Pins a byte-rope and a scalar group
           coexisting in ONE component.")
  (input  (do (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (def (inc) (fn ((: x Int64)) (+ x 1)))
              (export mkb) (export inc)))
  (call   mkb (: 9 Int64))
  (output (9 10)))

(case "distinct-sig: a byte-rope closure coexists with a SCALAR closure — the scalar one"
  (doc    "The SAME mixed byte-rope/scalar program, driving the SCALAR group: `call(inc-handle, 41)` → 42
           (returned by value, NOT as a byte list). Confirms the scalar `call-<g>` is unaffected by the
           sibling byte-rope group's memory/realloc plumbing.")
  (input  (do (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
              (def (inc) (fn ((: x Int64)) (+ x 1)))
              (export mkb) (export inc)))
  (call   inc (: 41 Int64))
  (output (: 42 Int64)))

(case "distinct-sig: a String closure + a Bytes closure of different signatures — the String one"
  (doc    "`greet : () -> (-> Int64 String)` returns \"hi\" (UTF-8 `[104,105]`), alongside `mkb : () -> (->
           Bool Bytes)`. Both cross as byte-rope `list<u8>` results but through DISTINCT resource types.
           `call(greet-handle, 0)` → `[104,105]`.")
  (input  (do (def (greet) (fn ((: n Int64)) "hi"))
              (def (mkb) (fn ((: b Bool)) (bin (u8 (if b 7 8)))))
              (export greet) (export mkb)))
  (call   greet (: 0 Int64))
  (output (104 105)))

(case "distinct-sig byte-rope closure alongside a plain export — the closure"
  (doc    "Two distinct byte-rope closures (`mkb : Int64→Bytes`, `isz : Bool→Bytes`) AND a plain `two : ()
           -> 2` all in one component. `call(mkb-handle, 3)` → `[3]`. Pins the byte-rope distinct-sig path
           carrying a plain export alongside (via `assemble_distinct_sig_resource_mixed`).")
  (input  (do (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)))))
              (def (isz) (fn ((: b Bool)) (bin (u8 (if b 0 1)))))
              (def (two) 2)
              (export mkb) (export isz) (export two)))
  (call   mkb (: 3 Int64))
  (output (3)))

(case "distinct-sig byte-rope closure alongside a plain export — the plain"
  (doc    "The SAME program, calling the plain `two` → 2. Confirms the plain top-level export is reachable
           when TWO distinct byte-rope closure resources share the component.")
  (input  (do (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)))))
              (def (isz) (fn ((: b Bool)) (bin (u8 (if b 0 1)))))
              (def (two) 2)
              (export mkb) (export isz) (export two)))
  (call   two)
  (output (: 2 Int64)))

; BYTE-ROPE result on the ROUND-TRIP path — a consumer takes a produced closure back, applies it, and
; RETURNS a `Bytes`/`String`. The consumer crosses as `(own<t>, args…) -> list<u8>` (memory + cabi_realloc
; shared), completing the byte-rope compound `call` across ALL closure shapes (single/multi/mixed/distinct-
; sig/round-trip). A byte-rope consumer can coexist with a scalar consumer of the same closure and with a
; plain export. (Also fixed a latent BinBuild slot-typing bug: two `(g x)` closure applications across two
; `bin` segments aliased one wasm local at two widths — now each segment's value floats above the
; high-water mark, the same disjoint-slot discipline the checked-arith path uses.)

(case "round-trip: a consumer applies the handed-back closure and returns Bytes"
  (doc    "`mk : () -> (-> Int64 Int64)` (adds 1); `app : (own<t>, Int64) -> Bytes` applies the handed-back
           closure TWICE — `(bin (u8 (g x)) (u8 (g x)+1))`. Host produces a handle via `mk`, hands it to
           `app(handle, 5)` → the closure yields 6, so the bytes are `[6, 7]`. Pins the byte-rope result on
           the round-trip path (the consumer returns `list<u8>`).")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64))
                (bin (u8 (UInt8.wrap (g x))) (u8 (UInt8.wrap (+ (g x) 1)))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (6 7)))

(case "round-trip: a consumer returns a byte-rope built from a single closure result"
  (doc    "`mk` doubles; `app : (own<t>, Int64) -> Bytes` = `(bin (u8 (g x)))`. `app(handle, 10)` → the
           closure yields 20 → `[20]`. The single-segment byte-rope consumer result.")
  (input  (do (def (mk) (fn ((: n Int64)) (* n 2)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (export mk) (export app)))
  (call   app (: 10 Int64))
  (output (20)))

(case "round-trip: a String-returning consumer of a closure"
  (doc    "`label : (own<t>, Int64) -> String` returns the constant \"hi\" (UTF-8 `[104,105]`) — a String
           consumer result crosses on the same byte-rope `list<u8>` path as Bytes.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 65)))
              (def (label (: g (-> Int64 Int64)) (: x Int64)) "hi")
              (export mk) (export label)))
  (call   label (: 0 Int64))
  (output (104 105)))

(case "round-trip byte-rope consumer alongside a plain export — the consumer"
  (doc    "`app : (own<t>, Int64) -> Bytes` beside a plain `seven : () -> 7`. `app(handle, 41)` → `[42]`.
           Pins the byte-rope round-trip consumer carrying a plain export alongside.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (def (seven) 7)
              (export mk) (export app) (export seven)))
  (call   app (: 41 Int64))
  (output (42)))

(case "round-trip byte-rope consumer alongside a plain export — the plain"
  (doc    "The SAME program, calling the plain `seven` → 7. Confirms the plain top-level export is reachable
           when a byte-rope round-trip consumer shares the component.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (def (seven) 7)
              (export mk) (export app) (export seven)))
  (call   seven)
  (output (: 7 Int64)))

(case "round-trip: a scalar consumer and a byte-rope consumer of the same closure — the byte-rope one"
  (doc    "One closure signature, TWO consumers: `asnum : (own<t>, Int64) -> Int64` (returns the value) and
           `asbytes : (own<t>, Int64) -> Bytes` (wraps it into a `bin`). `asbytes(handle, 8)` → `[9]`. Pins a
           SCALAR consumer and a BYTE-ROPE consumer of the same resource coexisting (one lifted by value, one
           with Memory/Realloc).")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (asbytes (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (export mk) (export asnum) (export asbytes)))
  (call   asbytes (: 8 Int64))
  (output (9)))

(case "round-trip: a scalar consumer and a byte-rope consumer of the same closure — the scalar one"
  (doc    "The SAME two-consumer program, driving the SCALAR consumer: `asnum(handle, 8)` → 9 (by value, NOT
           a byte list). Confirms the scalar consumer is unaffected by the sibling byte-rope consumer's
           memory/realloc lift.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (asbytes (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (export mk) (export asnum) (export asbytes)))
  (call   asnum (: 8 Int64))
  (output (: 9 Int64)))

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

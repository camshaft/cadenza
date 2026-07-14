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
  (doc    "The same `(fn (x) (+ x 1))` closure export, called with 41 → 42. The `call` method takes
           `borrow<t>`, so the host KEEPS the handle across calls (a repeatable callback — the natural
           host-closure shape) and the resource dtor reclaims the cell when the host finally drops it; this
           case still `make`s + `call`s once. Pins that the closure's dispatch is reusable and its result
           follows the argument.")
  (input  (do (def (main) (fn ((: x Int64)) (+ x 1))) (export main)))
  (call   main (: 41 Int64))
  (output (: 42 Int64)))

; The `call` method takes `borrow<t>`: the host holds the handle and may invoke it REPEATEDLY (the natural
; callback shape), versus a consume-per-call `own<t>` where a second call on the same handle would trap
; "unknown handle index". The gate drives ONE `(call …)` per case, so the REPEATABILITY is pinned by the
; `a_borrow_closure_handle_is_repeatable` unit test (one `make` handle, two `call`s: `adder(10)` then 5→15,
; 7→17 on the SAME handle); this case witnesses the borrow `call` runs end-to-end. A capturing closure makes
; it concrete: the captured `k` survives across calls because the cell is not consumed.

(case "a capturing closure crosses as a repeatable (borrow<t>) callback handle"
  (doc    "`(def (adder (: k Int64)) (fn (x) (+ x k)))` → `adder : (Int64) -> own<closure-s64-s64>`. The host
           `make`s a handle capturing k, then `call`s it — `call` borrows the handle (does NOT consume it),
           so the same handle serves repeated calls (proven twice-over in the unit test). Here `adder(100)` →
           a handle → `call(handle, 5)` = 105. Pins the borrow<t> repeatable-callback `call` end-to-end.")
  (input  (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
  (call   adder (: 100 Int64) (: 5 Int64))
  (output (: 105 Int64)))

; The repeatable `borrow<t>` `call` extends to the VALUE-FORM result closures too (byte-rope / compound /
; collection — all cross `call` as `list<u8>`): the cell is kept across calls, the transient result handle is
; released each call, and the `t-dtor` reclaims the cell on drop. The gate drives one `(call …)`; the
; repeatability is pinned by `a_borrow_compound_result_closure_handle_is_repeatable` (one `pair(100)` handle,
; two `call(5)`s, the SAME `(tuple 5 105)` value form both times — the captured k survived).

(case "a capturing closure returning a COMPOUND is a repeatable (borrow<t>) callback handle"
  (doc    "`(def (pair (: k Int64)) (fn (x) (tuple x (+ x k))))` → a closure whose result is a tuple, crossing
           `call` as the `list<u8>` value form. `call` borrows the handle (repeatable — the same handle serves
           many calls, proven in the unit test), and the returned tuple is value-form-encoded out.
           `pair(100)` → a handle → `call(handle, 5)` = `(: (tuple 5 105) (Tuple Int64 Int64))`. Pins the
           borrow<t> repeatable `call` on a value-form (compound) result end-to-end.")
  (input  (do (def (pair (: k Int64)) (fn ((: x Int64)) (tuple x (+ x k)))) (export pair)))
  (call   pair (: 100 Int64) (: 5 Int64))
  (output (: (tuple 5 105) (Tuple Int64 Int64))))

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

(case "a closure capturing THREE scalars is made and called"
  (doc    "`(def (mk (: a Int64) (: b Int64) (: c Int64)) (fn (x) (+ (+ (+ x a) b) c)))` — three captured
           values in the cell. `make(1, 2, 3)` then `call(10)` = 10 + 1 + 2 + 3 = 16. Extends the two-capture
           case to a wider environment (each capture read back inside the `call` dispatch).")
  (input  (do (def (mk (: a Int64) (: b Int64) (: c Int64)) (fn ((: x Int64)) (+ (+ (+ x a) b) c)))
              (export mk)))
  (call   mk (: 1 Int64) (: 2 Int64) (: 3 Int64) (: 10 Int64))
  (output (: 16 Int64)))

(case "a closure capturing values of DIFFERENT types (Float64 + Int64)"
  (doc    "`(def (mk (: base Float64) (: n Int64)) (fn (x) (+. x base)))` — the cell captures a Float64 AND an
           Int64 (the latter unused in the body, but still stored), and the closure returns a Float64.
           `make(1.5, 7)` then `call(2.5)` = 2.5 +. 1.5 = 4.0. Pins a MIXED-type capture environment (a float
           and an int share one cell) with a float `call` result.")
  (input  (do (def (mk (: base Float64) (: n Int64)) (fn ((: x Float64)) (+. x base)))
              (export mk)))
  (call   mk (: 1.5 Float64) (: 7 Int64) (: 2.5 Float64))
  (output (: 4.0 Float64)))

; The DIRECT-CALL host→guest boundary: when the HOST must supply a value to `make`/`call` OVER the boundary,
; only aliased-width scalars cross (the same restriction host-call `abi_val_type` has). A COMPOUND the host
; supplies — a `make` parameter of type `(List …)`/`(Tuple …)`/a sum — needs a host→guest DECODE into the
; guest value-heap (a `value-decode` runtime op that does not exist), so it declines. This is the mirror of
; the round-trip relaxation: an in-GUEST-built compound arg crosses freely (built guest-side), but a
; host-SUPPLIED compound does not. The compiler DECLINES (a `todo`) rather than emit a component that can't
; accept the argument.

(case "a producer capturing a host-supplied COMPOUND parameter is declined — host→guest decode"
  (doc    "`(def (mk (: xs (List Int64))) (fn (i) ((. List len) xs)))` returns a closure capturing the List
           `xs`, but `xs` is a `make` PARAMETER the HOST supplies over the boundary — a `(List Int64)` has no
           scalar host-boundary representation, and there is no host→guest decode of a compound into the guest
           heap. Declines (a `todo`). Contrast the round-trip cases, where a compound closure argument is
           BUILT in-guest and crosses freely.")
  (input  (do (def (mk (: xs (List Int64))) (fn ((: i Int64)) ((. List len) xs)))
              (export mk)))
  (call   mk (: 5 Int64))
  (output (: 3 Int64)))

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
           the result is 3 + 10 = 13. The component imports BOTH the effect interface (`host`) and the
           value-heap runtime (`heap`, for the closure cell) — the export-time host-call boundary composed
           with the closure resource, so the host response flows into the captured `v` and the returned
           closure is a plain callable.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (let ((v (ask.ask)))
                  (fn ((: x Int64)) (+ x v))))) (export main)))
  (call   main (: 3 Int64))
  (host-responses (respond ask.ask (: 10 Int64)))
  (output (: 13 Int64)))

(case "a closure capturing a build-time host effect preserves the order of two host calls"
  (doc    "The build-time host-capture composes with MULTIPLE host calls in the make code, consumed in the
           order made: `(let ((a (ask.ask)) (b (ask.ask))) …)` binds `a` to the first response and `b` to
           the second. Both are captured as plain values into the returned closure `(fn (x) (+ (+ x a) b))`.
           With responses 10 then 20 and the call argument 3, the result is 3 + 10 + 20 = 33 — the host-call
           order is observable through the captured values (host-calls asserts the two calls).")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (let ((a (ask.ask)) (b (ask.ask)))
                  (fn ((: x Int64)) (+ (+ x a) b))))) (export main)))
  (call   main (: 3 Int64))
  (host-responses (respond ask.ask (: 10 Int64))
                  (respond ask.ask (: 20 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 33 Int64)))

(case "a closure captures the result of a build-time host op called with an argument"
  (doc    "The build-time host op may take an ARGUMENT: `(calc.dbl 5)` crosses the boundary passing 5, the
           host returns its response, and the closure captures it as a plain value. With `calc.dbl`
           responding 10 (the host's answer for input 5) and the call argument 3, the result is 3 + 10 = 13.
           Exercises a scalar host-op parameter composing with the closure-capture path.")
  (input  (do
            (effect calc (op dbl (-> Int64 Int64)))
            (def (main)
              (host (calc)
                (let ((v (calc.dbl 5)))
                  (fn ((: x Int64)) (+ x v))))) (export main)))
  (call   main (: 3 Int64))
  (host-responses (respond calc.dbl (: 10 Int64)))
  (host-calls (call calc.dbl))
  (output (: 13 Int64)))

(case "a Float64 closure captures a Float64 build-time host effect result"
  (doc    "The build-time host-capture is not Int64-specific: a `Float64` host op result crosses the
           boundary as `f64`, is captured as a plain value, and the returned closure is a `Float64 ->
           Float64`. With `ask.ask` responding 2.5 and the call argument 1.5, the result is 1.5 + 2.5 = 4.0.
           Exercises the f64 boundary primitive on BOTH the host op and the closure arg/result composing
           with the closure-capture path (a scalar-result shape, just a non-Int scalar).")
  (input  (do
            (effect ask (op ask (-> Unit Float64)))
            (def (main)
              (host (ask)
                (let ((v (ask.ask)))
                  (fn ((: x Float64)) (+. x v))))) (export main)))
  (call   main (: 1.5 Float64))
  (host-responses (respond ask.ask (: 2.5 Float64)))
  (host-calls (call ask.ask))
  (output (: 4.0 Float64)))

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

; The multi-export SHARED `call` is a repeatable `borrow<t>` method too (C-HOST-6): one `make-<name>` handle
; serves repeated calls through the one shared `call` (the host keeps it; the `t-dtor` reclaims on drop). The
; gate drives one `(call …)`; the repeatability is pinned by `a_multi_export_shared_borrow_call_is_repeatable`
; (one `make-inc` handle, shared `call(5)`=6 then `call(40)`=41).

(case "a multi-export shared call is a repeatable (borrow<t>) callback"
  (doc    "The SAME two-export program witnessed as a borrow<t> shared call: `make-inc()` → a handle the host
           keeps, then the shared `call(5)` = 6. `call` borrows the handle (does NOT consume it), so the same
           handle serves repeated calls through the one shared `call` (proven twice-over in the unit test).")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (triple) (fn ((: x Int64)) (* x 3)))
              (export inc) (export triple)))
  (call   inc (: 5 Int64))
  (output (: 6 Int64)))

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

; Each distinct-signature group's per-group `call-g<n>` is a repeatable `borrow<t_g>` method too (C-HOST-6,
; the last borrow widening): a `make-<name>` handle serves repeated `call-g<n>`s (the host keeps it; the
; `t-dtor` reclaims). The gate drives one `(call …)`; repeatability is pinned by
; `a_distinct_sig_call_g_is_repeatable` (one `make-inc` handle → `call-g(5)`=6 then `call-g(40)`=41). This
; closes the borrow surface: EVERY closure `call` in every shape is now a repeatable borrow<t> handle.

(case "a distinct-signature per-group call-g is a repeatable (borrow<t>) callback"
  (doc    "The SAME two-resource-type program witnessed as a borrow<t> per-group call: `make-inc()` → a handle
           the host keeps (resource t0), its `call-g<n>(5)` = 6. `call-g<n>` borrows the handle (does NOT
           consume it), so the same handle serves repeated calls (proven twice-over in the unit test); the
           distinct `isz` group's `call-g<n>` is independently repeatable.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (isz) (fn ((: x Int64)) (= x 0)))
              (export inc) (export isz)))
  (call   inc (: 5 Int64))
  (output (: 6 Int64)))

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

; A multi-argument arrow may be written FLAT `(-> A B … R)` (the idiomatic spelling) as well as explicitly
; CURRIED `(-> A (-> B R))` — both denote the same n-ary function type `A -> (B -> (… -> R))`. The flat form
; `(-> Int64 Int64 Int64)` used to error "-> takes one or two type arguments" (only arities 1 + 2 were
; handled), so a round-trip consumer whose closure parameter was written flat solved `Any` and declined
; "parameter type is ambiguous — annotate it". The arrow constructor now curries any arity ≥1.

(case "a multi-argument closure round-trips through a consumer — FLAT arrow spelling"
  (doc    "The SAME two-arg round trip as above, but the consumer's closure parameter is written with the
           FLAT arrow `(: g (-> Int64 Int64 Int64))` instead of the explicitly-curried `(-> Int64 (-> Int64
           Int64))`. Both denote `Int64 -> (Int64 -> Int64)`. `app(handle, 3, 4)` = 7. Pins that a flat
           multi-arg arrow annotation curries — previously it errored `-> takes one or two type arguments`
           and the param declined `parameter type is ambiguous`.")
  (input  (do (def (mk) (fn ((: a Int64) (: b Int64)) (+ a b)))
              (def (app (: g (-> Int64 Int64 Int64)) (: a Int64) (: b Int64)) (g a b))
              (export mk) (export app)))
  (call   app (: 3 Int64) (: 4 Int64))
  (output (: 7 Int64)))

(case "a THREE-argument closure round-trips — flat arrow spelling"
  (doc    "A flat three-argument arrow `(-> Int64 Int64 Int64 Int64)` curries to `Int64 -> Int64 -> Int64 ->
           Int64`. `mk` sums three args; `app` applies the handed-back `g` to `x`, `x+1`, `x+2`. `app(handle,
           10)` → `g(10, 11, 12)` = 33.")
  (input  (do (def (mk) (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ a b) c)))
              (def (app (: g (-> Int64 Int64 Int64 Int64)) (: x Int64)) (g x (+ x 1) (+ x 2)))
              (export mk) (export app)))
  (call   app (: 10 Int64))
  (output (: 33 Int64)))

(case "a multi-argument closure with COMPOUND args round-trips — flat arrow spelling"
  (doc    "Composes the flat multi-arg arrow with compound closure arguments (both built in-guest): `g : (->
           (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)` reads `p.0 + q.1`; `app` applies it to `(tuple x
           x)` and `(tuple x (x*2))`. `app(handle, 5)` → `g((tuple 5 5), (tuple 5 10))` = 5 + 10 = 15.")
  (input  (do (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
              (def (app (: g (-> (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)) (: x Int64))
                (g (tuple x x) (tuple x (* x 2))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 15 Int64)))

(case "a multi-argument closure returning a COMPOUND round-trips — flat arrow spelling"
  (doc    "A flat two-arg arrow with a compound RESULT: `g : (-> Int64 Int64 (Tuple Int64 Int64))` pairs its
           two args; `app` applies it to `x` and `x+10` and returns the tuple. `app(handle, 5)` → `g(5, 15)` =
           `(: (tuple 5 15) (Tuple Int64 Int64))`, value-form-encoded out.")
  (input  (do (def (mk) (fn ((: a Int64) (: b Int64)) (tuple a b)))
              (def (app (: g (-> Int64 Int64 (Tuple Int64 Int64))) (: x Int64)) (g x (+ x 10)))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (tuple 5 15) (Tuple Int64 Int64))))

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

; BYTE-ROPE result on the DISTINCT-SIG ROUND-TRIP path — the LAST byte-rope gap. Closures of DIFFERENT
; signatures each cross as their own resource type, and a CONSUMER of one signature can RETURN a
; `Bytes`/`String` (crossing as `(own<t_g>, args…) -> list<u8>`, memory + cabi_realloc shared across groups).
; Completes the byte-rope compound `call` across EVERY closure shape. A byte-rope consumer coexists with a
; scalar consumer of another signature, and two byte-rope consumers of different signatures coexist.

(case "distinct-sig round-trip: a byte-rope consumer + a scalar consumer of another sig — the byte-rope one"
  (doc    "`mka : () -> (-> Int64 Int64)` and `mkb : () -> (-> Bool Int64)` are distinct signatures → two
           resource types. `appa : (own<t0>, Int64) -> Bytes` applies its closure TWICE — `(bin (u8 (g x))
           (u8 (g x)+1))`. Host produces via `mka`, hands to `appa(handle, 5)` → `[6, 7]`. Pins the byte-rope
           consumer result on the distinct-sig round-trip path.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64))
                (bin (u8 (UInt8.wrap (g x))) (u8 (UInt8.wrap (+ (g x) 1)))))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 5 Int64))
  (output (6 7)))

(case "distinct-sig round-trip: a byte-rope consumer + a scalar consumer of another sig — the scalar one"
  (doc    "The SAME two-resource-type program, driving the SCALAR consumer of the OTHER signature: `appb :
           (own<t1>, Bool) -> Int64` applies `mkb`'s closure → `appb(handle, true)` = 10 (by value). Confirms
           the scalar consumer is unaffected by the sibling byte-rope consumer's memory/realloc plumbing.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64))
                (bin (u8 (UInt8.wrap (g x))) (u8 (UInt8.wrap (+ (g x) 1)))))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: true Bool))
  (output (: 10 Int64)))

(case "distinct-sig round-trip: TWO byte-rope consumers of different signatures — the Int64 one"
  (doc    "Both consumers return `Bytes`, but of DISTINCT closure signatures (two resource types, each
           lifted with its own Memory/Realloc). `appa(mka-handle, 40)` → `[41]`.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (bin (u8 (UInt8.wrap (h y))) (u8 99)))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 40 Int64))
  (output (41)))

(case "distinct-sig round-trip: TWO byte-rope consumers of different signatures — the Bool one"
  (doc    "The SAME program, driving the OTHER byte-rope consumer: `appb(mkb-handle, false)` → `mkb`'s
           closure yields 8, so `(bin (u8 8) (u8 99))` = `[8, 99]`. Confirms each distinct byte-rope
           resource dispatches its own closure body + writes its own `list<u8>`.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (bin (u8 (UInt8.wrap (h y))) (u8 99)))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: false Bool))
  (output (8 99)))

; A COMPOUND (tuple/record) closure RESULT — the closure's `call` returns the canonical VALUE FORM as
; `list<u8>` (the value-heap escape's `runtime_value_form_template` + `encode_walk_body` walker, keyed on
; the closure's returned handle), so the host DECODES + pretty-prints the typed `(: value T)` document (not
; a bare byte sequence like the byte-rope path). cdz-run try-decodes the `call` result: the codec's 8-byte
; schema header disambiguates a value form from a raw byte-rope, so both share the `list<u8>` boundary
; unambiguously. Fixed-shape compounds (tuple/record/sum) are supported; a variable-length list still
; declines (no fixed template).

(case "a closure returning a tuple crosses as the typed value form"
  (doc    "`mk : () -> (-> Int64 (Tuple Int64 Int64))` returns `(tuple n n+1)`. `call(handle, 5)` walks the
           returned tuple handle, writes the value form, and the host decodes it to `(: (tuple 5 6) (Tuple
           Int64 Int64))` — the FULL typed document, not a bare byte list.")
  (input  (do (def (mk) (fn ((: n Int64)) (tuple n (+ n 1)))) (export mk)))
  (call   mk (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case "a closure returning a record crosses as the typed value form"
  (doc    "A record result — `(record (x n) (y n+10))` → `(: (record (x 3) (y 13)) (Record (x Int64) (y
           Int64)))`. Field names + the record type node are baked in the template; only the leaf values are
           walked at run time.")
  (input  (do (def (mk) (fn ((: n Int64)) (record (x n) (y (+ n 10))))) (export mk)))
  (call   mk (: 3 Int64))
  (output (: (record (x 3) (y 13)) (Record (x Int64) (y Int64)))))

(case "a closure returning a tuple with a Bool leaf"
  (doc    "A mixed-leaf compound — `(tuple n (< n 5))` → `(: (tuple 2 true) (Tuple Int64 Bool))`. The Bool
           leaf's hole is filled via `get-bool` (its kind byte flipped true/false), the int via `get-int`.")
  (input  (do (def (mk) (fn ((: n Int64)) (tuple n (< n 5)))) (export mk)))
  (call   mk (: 2 Int64))
  (output (: (tuple 2 true) (Tuple Int64 Bool))))

(case "a closure returning a NESTED tuple"
  (doc    "`(tuple n (tuple n+1 n+2))` → `(: (tuple 7 (tuple 8 9)) (Tuple Int64 (Tuple Int64 Int64)))`. The
           walker descends nested `arr-get` paths (the inner tuple is a boxed handle inside the outer).")
  (input  (do (def (mk) (fn ((: n Int64)) (tuple n (tuple (+ n 1) (+ n 2))))) (export mk)))
  (call   mk (: 7 Int64))
  (output (: (tuple 7 (tuple 8 9)) (Tuple Int64 (Tuple Int64 Int64)))))

(case "a CAPTURING closure returning a tuple"
  (doc    "`mk : (Int64) -> (-> Int64 (Tuple Int64 Int64))` — `make(100)` captures `k=100`, then
           `call(handle, 5)` → `(: (tuple 100 5) (Tuple Int64 Int64))`. Confirms a captured value flows into
           the compound result across the boundary.")
  (input  (do (def (mk (: k Int64)) (fn ((: n Int64)) (tuple k n))) (export mk)))
  (call   mk (: 100 Int64) (: 5 Int64))
  (output (: (tuple 100 5) (Tuple Int64 Int64))))

(case "a closure returning a tuple with a negative int leaf"
  (doc    "`(tuple n (- 0 n))` → `(: (tuple 5 -5) (Tuple Int64 Int64))`. The negative leaf flips the value
           form's kind byte to INT_NEG_DEC and writes the absolute magnitude (the escape's neg-int path).")
  (input  (do (def (mk) (fn ((: n Int64)) (tuple n (- 0 n)))) (export mk)))
  (call   mk (: 5 Int64))
  (output (: (tuple 5 -5) (Tuple Int64 Int64))))

; A COMPOUND (tuple/record) closure RESULT on the MULTI-EXPORT path — N same-signature closures each
; returning a tuple/record share ONE `call` that returns the value form as `list<u8>`. The shared `call`
; recovers each closure's code slot from the resource rep, dispatches it, and walks the returned compound
; handle into the ONE value-form template (all exports share the result type → one template). The host
; decodes each result to the typed `(: value T)` document. (Record fields render in CANONICAL sorted-name
; order — `hi` before `lo` — same as the single-export path and the value-heap escape.)

(case "multi-export compound result — the first closure's tuple"
  (doc    "Two same-signature closures — `mkpair : () -> (-> Int64 (Tuple Int64 Int64))` returns `(tuple n
           n+1)`, `mkdbl` returns `(tuple n 2n)`. `call(mkpair-handle, 5)` walks its returned tuple → `(:
           (tuple 5 6) (Tuple Int64 Int64))`. Pins the compound value-form result on the shared-`call`
           multi-export path.")
  (input  (do (def (mkpair) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (mkdbl) (fn ((: n Int64)) (tuple n (* n 2))))
              (export mkpair) (export mkdbl)))
  (call   mkpair (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case "multi-export compound result — the second closure's tuple"
  (doc    "The SAME two-closure program, driving the OTHER export: `call(mkdbl-handle, 5)` → `(tuple 5 10)`.
           Confirms the shared `call` dispatches whichever closure a handle names and walks ITS distinct
           result (the code slot rides in the rep, the value form is shared since the type is).")
  (input  (do (def (mkpair) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (mkdbl) (fn ((: n Int64)) (tuple n (* n 2))))
              (export mkpair) (export mkdbl)))
  (call   mkdbl (: 5 Int64))
  (output (: (tuple 5 10) (Tuple Int64 Int64))))

; The multi-export VALUE-FORM shared `call` (byte-rope/compound/collection — all cross as `list<u8>`) is a
; repeatable `borrow<t>` method too (C-HOST-6): one `make-<name>` handle serves repeated shared calls, each
; re-walking/re-encoding the value form (the host keeps the cell; the `t-dtor` reclaims). Repeatability is
; pinned by `a_multi_export_value_form_shared_borrow_call_is_repeatable` (one `make-lo` handle → the SAME
; `(tuple 5 6)` value form on two shared calls).

(case "a multi-export compound-result shared call is a repeatable (borrow<t>) callback"
  (doc    "The SAME two-tuple-closure program witnessed as a borrow<t> value-form shared call: `make-mkpair()`
           → a handle the host keeps, the shared list-`call(5)` → `(: (tuple 5 6) (Tuple Int64 Int64))`. `call`
           borrows the handle (does NOT consume it), so the same handle serves repeated value-form calls
           (proven twice-over in the unit test).")
  (input  (do (def (mkpair) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (mkdbl) (fn ((: n Int64)) (tuple n (* n 2))))
              (export mkpair) (export mkdbl)))
  (call   mkpair (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case "multi-export record result — canonical field order"
  (doc    "Two closures returning a `(Record (lo Int64) (hi Int64))`. `call(mka-handle, 3)` → `(record (lo 3)
           (hi 103))`, rendered in CANONICAL sorted-name order `(record (hi 103) (lo 3))`.")
  (input  (do (def (mka) (fn ((: n Int64)) (record (lo n) (hi (+ n 100)))))
              (def (mkb) (fn ((: n Int64)) (record (lo (- 0 n)) (hi n))))
              (export mka) (export mkb)))
  (call   mka (: 3 Int64))
  (output (: (record (hi 103) (lo 3)) (Record (hi Int64) (lo Int64)))))

(case "multi-export record result — the second closure, with a negative leaf"
  (doc    "The SAME program's other export: `call(mkb-handle, 3)` → `(record (lo -3) (hi 3))` → canonical
           `(record (hi 3) (lo -3))`. The negative `lo` leaf flips its value form's kind byte.")
  (input  (do (def (mka) (fn ((: n Int64)) (record (lo n) (hi (+ n 100)))))
              (def (mkb) (fn ((: n Int64)) (record (lo (- 0 n)) (hi n))))
              (export mka) (export mkb)))
  (call   mkb (: 3 Int64))
  (output (: (record (hi 3) (lo -3)) (Record (hi Int64) (lo Int64)))))

(case "multi-export compound result — three capturing closures share one call"
  (doc    "THREE same-signature closures (two capturing `k`, one not) each returning `(Tuple Int64 Int64)`.
           `b(7)` captures `k=7`; `call(b-handle, 2)` → `(tuple 2 7)`. Pins the shared value-form `call`
           dispatching among 3 closures, with captured values flowing into the compound result.")
  (input  (do (def (a (: k Int64)) (fn ((: n Int64)) (tuple k n)))
              (def (b (: k Int64)) (fn ((: n Int64)) (tuple n k)))
              (def (c) (fn ((: n Int64)) (tuple n n)))
              (export a) (export b) (export c)))
  (call   b (: 7 Int64) (: 2 Int64))
  (output (: (tuple 2 7) (Tuple Int64 Int64))))

; A COMPOUND (tuple/record) closure RESULT on the MIXED path — a compound-returning closure exported
; ALONGSIDE a plain non-closure export. The closure crosses via the resource envelope (`make-<name>` + a
; shared `call` returning the value form as `list<u8>`); each plain export rides as an ordinary top-level
; component func. Same value-form core as the multi-export compound path, with the plain-export slots the
; mixed shape threads. The host decodes the closure result to `(: value T)`; a plain scalar renders directly.

(case "a tuple-returning closure alongside a plain export — the closure"
  (doc    "`mk : () -> (-> Int64 (Tuple Int64 Int64))` returns `(tuple n n+1)`, alongside a plain `two : ()
           -> 2`. `call(mk-handle, 5)` walks the returned tuple → `(: (tuple 5 6) (Tuple Int64 Int64))`. Pins
           the compound value-form result on the MIXED path (closure + plain export).")
  (input  (do (def (mk) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (two) 2)
              (export mk) (export two)))
  (call   mk (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case "a tuple-returning closure alongside a plain export — the plain"
  (doc    "The SAME mixed program, calling the plain `two` → 2 (a bare scalar, rendered directly — NOT a
           value-form document). Confirms the plain top-level export is reachable when a compound-result
           closure shares the component.")
  (input  (do (def (mk) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (two) 2)
              (export mk) (export two)))
  (call   two)
  (output (: 2 Int64)))

(case "a record-returning closure alongside a parameterized plain export — the closure"
  (doc    "`mk : () -> (-> Int64 (Record (a Int64) (b Int64)))` returns `(record (a n) (b 2n))`, beside a
           parameterized plain `inc : (Int64) -> Int64`. `call(mk-handle, 4)` → `(: (record (a 4) (b 8))
           (Record (a Int64) (b Int64)))`.")
  (input  (do (def (mk) (fn ((: n Int64)) (record (a n) (b (* n 2)))))
              (def (inc (: x Int64)) (+ x 1))
              (export mk) (export inc)))
  (call   mk (: 4 Int64))
  (output (: (record (a 4) (b 8)) (Record (a Int64) (b Int64)))))

(case "a record-returning closure alongside a parameterized plain export — the plain"
  (doc    "The SAME program, calling `inc(41)` = 42. Pins the parameterized plain export reachable beside a
           record-result closure.")
  (input  (do (def (mk) (fn ((: n Int64)) (record (a n) (b (* n 2)))))
              (def (inc (: x Int64)) (+ x 1))
              (export mk) (export inc)))
  (call   inc (: 41 Int64))
  (output (: 42 Int64)))

; A COMPOUND (tuple/record) closure RESULT on the DISTINCT-SIG path — closures of DIFFERENT signatures each
; returning a fixed-shape compound cross as G distinct resource types, each with its OWN `call-g<n>`
; returning THAT group's value form as `list<u8>` (a PER-GROUP template, since the result types differ). A
; compound group, a byte-rope group, and a scalar group can all coexist in one component: compound templates
; occupy their own data-section regions, byte-rope groups write dynamically PAST them, scalars return by
; value — so the three list<u8>/scalar memory uses never collide.

(case "distinct-sig compound result — the Int64→(Tuple Int64 Int64) closure"
  (doc    "`mki : () -> (-> Int64 (Tuple Int64 Int64))` and `mkb : () -> (-> Bool (Tuple Bool Int64))` are
           distinct signatures WITH distinct RESULT types → two resource types, each with its own value-form
           template. `call(mki-handle, 5)` walks its tuple → `(: (tuple 5 6) (Tuple Int64 Int64))`.")
  (input  (do (def (mki) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (mkb) (fn ((: b Bool)) (tuple b (if b 1 0))))
              (export mki) (export mkb)))
  (call   mki (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case "distinct-sig compound result — the Bool→(Tuple Bool Int64) closure"
  (doc    "The SAME program's OTHER group, whose result type differs: `call(mkb-handle, true)` → `(: (tuple
           true 1) (Tuple Bool Int64))`. Confirms each distinct-sig group walks its OWN per-group template.")
  (input  (do (def (mki) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (mkb) (fn ((: b Bool)) (tuple b (if b 1 0))))
              (export mki) (export mkb)))
  (call   mkb (: true Bool))
  (output (: (tuple true 1) (Tuple Bool Int64))))

(case "distinct-sig: a compound group + a byte-rope group + a scalar group — the compound"
  (doc    "THREE distinct signatures, THREE result MODES in one component: `mkt` returns a tuple (value
           form), `mkb` a `Bytes` (raw byte-rope), `inc` an Int64 (by value). `call(mkt-handle, 9)` → `(:
           (tuple 9 10) (Tuple Int64 Int64))`. Pins the disjoint-memory layout (compound template + byte-rope
           payload + scalar all coexisting).")
  (input  (do (def (mkt) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (mkb) (fn ((: b Bool)) (bin (u8 (if b 7 8)))))
              (def (inc) (fn ((: x Int64)) (+ x 1)))
              (export mkt) (export mkb) (export inc)))
  (call   mkt (: 9 Int64))
  (output (: (tuple 9 10) (Tuple Int64 Int64))))

(case "distinct-sig: a compound group + a byte-rope group + a scalar group — the byte-rope"
  (doc    "The SAME 3-mode program, driving the byte-rope group: `call(mkb-handle, false)` → `(8)` (a raw
           byte list, rendered bare — NOT a value-form document). Its payload is written PAST the compound
           template region.")
  (input  (do (def (mkt) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (mkb) (fn ((: b Bool)) (bin (u8 (if b 7 8)))))
              (def (inc) (fn ((: x Int64)) (+ x 1)))
              (export mkt) (export mkb) (export inc)))
  (call   mkb (: false Bool))
  (output (8)))

(case "distinct-sig: a compound group + a byte-rope group + a scalar group — the scalar"
  (doc    "The SAME program's scalar group: `call(inc-handle, 41)` → 42 (returned by value, NOT list<u8>).
           Confirms the scalar `call-<g>` is unaffected by the sibling list-returning groups' memory.")
  (input  (do (def (mkt) (fn ((: n Int64)) (tuple n (+ n 1))))
              (def (mkb) (fn ((: b Bool)) (bin (u8 (if b 7 8)))))
              (def (inc) (fn ((: x Int64)) (+ x 1)))
              (export mkt) (export mkb) (export inc)))
  (call   inc (: 41 Int64))
  (output (: 42 Int64)))

; A COMPOUND (tuple/record) result on the ROUND-TRIP path — a consumer takes a produced closure back,
; applies it, and RETURNS a fixed-shape compound. The consumer crosses as `(own<t>, args…) -> list<u8>`
; carrying the value form (its own template, walked from the body's returned handle). Completes the compound
; result across ALL closure shapes. A compound consumer coexists with a scalar consumer, a byte-rope
; consumer of the same closure, and a plain export (disjoint memory: compound templates in the data section,
; byte-rope payloads written past them, scalars by value).

(case "round-trip: a consumer applies the handed-back closure and returns a tuple"
  (doc    "`mk : () -> (-> Int64 Int64)` (adds 1); `app : (own<t>, Int64) -> (Tuple Int64 Int64)` returns
           `(tuple x (g x))`. Host produces via `mk`, hands the handle to `app(handle, 5)` → the closure
           yields 6, so the tuple is `(5, 6)`, decoded to `(: (tuple 5 6) (Tuple Int64 Int64))`. Pins the
           compound value-form result on the round-trip path.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case "round-trip: a consumer returns a record built from the closure result"
  (doc    "`mk` doubles; `app : (own<t>, Int64) -> (Record (inp Int64) (out Int64))` = `(record (inp x) (out
           (g x)))`. `app(handle, 10)` → `(: (record (inp 10) (out 20)) …)`.")
  (input  (do (def (mk) (fn ((: n Int64)) (* n 2)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (record (inp x) (out (g x))))
              (export mk) (export app)))
  (call   app (: 10 Int64))
  (output (: (record (inp 10) (out 20)) (Record (inp Int64) (out Int64)))))

(case "round-trip: a scalar consumer + a compound consumer of the same closure — the compound"
  (doc    "One closure signature, TWO consumers: `asnum` returns the value, `aspair` returns `(tuple x (g
           x))`. `aspair(handle, 8)` → `(: (tuple 8 9) (Tuple Int64 Int64))`. Pins a scalar consumer and a
           compound (value-form) consumer of the same resource coexisting.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (aspair (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (export mk) (export asnum) (export aspair)))
  (call   aspair (: 8 Int64))
  (output (: (tuple 8 9) (Tuple Int64 Int64))))

(case "round-trip: a scalar consumer + a compound consumer of the same closure — the scalar"
  (doc    "The SAME two-consumer program, driving the SCALAR consumer: `asnum(handle, 8)` → 9 (by value, NOT
           a value-form document). Confirms the scalar consumer is unaffected by the sibling compound
           consumer's memory/template.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (aspair (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (export mk) (export asnum) (export aspair)))
  (call   asnum (: 8 Int64))
  (output (: 9 Int64)))

(case "round-trip: a compound consumer + a byte-rope consumer of the same closure — the compound"
  (doc    "One signature, a COMPOUND consumer (`aspair` → tuple value form) AND a BYTE-ROPE consumer
           (`asbytes` → raw `list<u8>`). `aspair(handle, 3)` → `(: (tuple 3 4) …)`. Pins disjoint memory: the
           compound template region vs the byte-rope payload written past it.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (aspair (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (def (asbytes (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (export mk) (export aspair) (export asbytes)))
  (call   aspair (: 3 Int64))
  (output (: (tuple 3 4) (Tuple Int64 Int64))))

(case "round-trip: a compound consumer + a byte-rope consumer of the same closure — the byte-rope"
  (doc    "The SAME program, driving the byte-rope consumer: `asbytes(handle, 40)` → `(41)` (a raw byte
           list, its payload written PAST the compound template region — the two never collide).")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (aspair (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (def (asbytes (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
              (export mk) (export aspair) (export asbytes)))
  (call   asbytes (: 40 Int64))
  (output (41)))

(case "round-trip: a compound consumer alongside a plain export — the plain"
  (doc    "A tuple-returning consumer `app` beside a plain `five : () -> 5`. Calling `five` → 5. Confirms a
           plain top-level export is reachable when a compound round-trip consumer shares the component.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (def (five) 5)
              (export mk) (export app) (export five)))
  (call   five)
  (output (: 5 Int64)))

; A COMPOUND (tuple/record) result on the DISTINCT-SIG ROUND-TRIP path — the LAST fixed-shape compound-result
; gap. Producers/consumers of DIFFERENT signatures where a consumer RETURNS a fixed-shape compound: each
; consumer crosses as `(own<t_g>, args…) -> list<u8>` carrying the value form (its own per-consumer template).
; Fixed-shape compound results now work across EVERY closure shape. A compound consumer coexists with a
; scalar consumer, another compound consumer of a different sig, and a byte-rope consumer (disjoint memory:
; each compound template its own data region, byte-rope payloads written past them).

(case "distinct-sig round-trip: a compound consumer + a scalar consumer of another sig — the compound"
  (doc    "`mka : () -> (-> Int64 Int64)`, `mkb : () -> (-> Bool Int64)` are distinct sigs → two resource
           types. `appa : (own<t0>, Int64) -> (Tuple Int64 Int64)` returns `(tuple x (g x))`. Host produces
           via `mka`, hands to `appa(handle, 5)` → `(: (tuple 5 6) (Tuple Int64 Int64))`. Pins the compound
           consumer result on the distinct-sig round-trip path.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case "distinct-sig round-trip: a compound consumer + a scalar consumer of another sig — the scalar"
  (doc    "The SAME two-resource-type program, driving the SCALAR consumer of the OTHER signature: `appb :
           (own<t1>, Bool) -> Int64` → `appb(handle, true)` = 10 (by value). Confirms the scalar consumer is
           unaffected by the sibling compound consumer's memory/template.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: true Bool))
  (output (: 10 Int64)))

(case "distinct-sig round-trip: TWO compound consumers of different sigs — the tuple one"
  (doc    "Both consumers return a compound of DIFFERENT shape: `appa` a tuple, `appb` a record.
           `appa(mka-handle, 40)` → `(: (tuple 40 41) (Tuple Int64 Int64))`. Each consumer walks its OWN
           per-consumer value-form template.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (record (flag y) (val (h y))))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 40 Int64))
  (output (: (tuple 40 41) (Tuple Int64 Int64))))

(case "distinct-sig round-trip: TWO compound consumers of different sigs — the record one"
  (doc    "The SAME program's OTHER consumer: `appb(mkb-handle, true)` → `(: (record (flag true) (val 7))
           (Record (flag Bool) (val Int64)))`. Confirms each distinct-sig consumer decodes its own template.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (record (flag y) (val (h y))))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: true Bool))
  (output (: (record (flag true) (val 7)) (Record (flag Bool) (val Int64)))))

(case "distinct-sig round-trip: a compound consumer + a byte-rope consumer of different sigs — the byte-rope"
  (doc    "A COMPOUND consumer (`appa` → tuple value form) AND a BYTE-ROPE consumer (`appb` → raw list<u8>)
           of DISTINCT signatures. `appb(mkb-handle, false)` → `(8)` — its payload written PAST the compound
           template region (disjoint memory).")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (tuple x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (bin (u8 (UInt8.wrap (h y)))))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: false Bool))
  (output (8)))

; A VARIABLE-LENGTH collection (List/Map/Set) closure RESULT — the closure's `call` returns the canonical
; value form as `list<u8>`, rendered at RUN TIME by the runtime `value-encode(rep, desc)` op (the recursive-
; sum escape's "approach C") walking the returned collection handle against a compiler-baked shape
; DESCRIPTOR. Unlike a fixed-shape tuple/record (a static template), a collection is variable-length, so the
; runtime assembles the document; `lower::sum_shape_descriptor`'s List/Map/Set arm builds a parametric
; `Framed` descriptor so the element/key/value types are observable. The host decodes to `(: (list …) (List
; <e>))` / `(: (map (k v) …) (Map <k> <v>))` / `(: ((. Set of) (list …)) (Set <e>))`.

(case "a closure returning a List crosses as the value form"
  (doc    "`mk : () -> (-> Int64 (List Int64))` returns `(list n n+1 n+2)`. `call(handle, 10)` dispatches the
           closure → the list handle, then `value-encode` renders `(: (list 10 11 12) (List Int64))`. Pins a
           VARIABLE-LENGTH collection result (no static template — the runtime walks the handle).")
  (input  (do (def (mk) (fn ((: n Int64)) (list n (+ n 1) (+ n 2)))) (export mk)))
  (call   mk (: 10 Int64))
  (output (: (list 10 11 12) (List Int64))))

(case "a closure returning a Set — canonical member order"
  (doc    "`(Set.of (list n n+1 n))` dedups to `{n, n+1}`; `call(handle, 5)` → `(: ((. Set of) (list 5 6))
           (Set Int64))`, members in canonical order (the runtime CHAMP set encode sorts).")
  (input  (do (def (mk) (fn ((: n Int64)) (Set.of (list n (+ n 1) n)))) (export mk)))
  (call   mk (: 5 Int64))
  (output (: ((. Set of) (list 5 6)) (Set Int64))))

(case "a closure returning a Map — canonical key order"
  (doc    "`(map (1 n) (2 n+1))` → `call(handle, 100)` → `(: (map (1 100) (2 101)) (Map Int64 Int64))`,
           entries in canonical key order.")
  (input  (do (def (mk) (fn ((: n Int64)) (map (1 n) (2 (+ n 1))))) (export mk)))
  (call   mk (: 100 Int64))
  (output (: (map (1 100) (2 101)) (Map Int64 Int64))))

(case "a closure returning a NESTED List"
  (doc    "`(list (list n) (list n+1 n+2))` → `(: (list (list 7) (list 8 9)) (List (List Int64)))`. The shape
           descriptor's type node is recursive, so a nested collection element crosses; `value-encode`
           recurses over the inner lists.")
  (input  (do (def (mk) (fn ((: n Int64)) (list (list n) (list (+ n 1) (+ n 2))))) (export mk)))
  (call   mk (: 7 Int64))
  (output (: (list (list 7) (list 8 9)) (List (List Int64)))))

(case "a CAPTURING closure returning a List"
  (doc    "`mk : (Int64) -> (-> Int64 (List Int64))` — `make(100)` captures `k=100`, then `call(handle, 5)` →
           `(: (list 100 5 105) (List Int64))`. Confirms a captured value flows into the collection result.")
  (input  (do (def (mk (: k Int64)) (fn ((: n Int64)) (list k n (+ k n)))) (export mk)))
  (call   mk (: 100 Int64) (: 5 Int64))
  (output (: (list 100 5 105) (List Int64))))

(case "a closure returning an EMPTY List"
  (doc    "`(: (list) (List Int64))` → `call(handle, 0)` → `(: (list) (List Int64))` — the value-encode
           walker handles a zero-length collection (the empty document).")
  (input  (do (def (mk) (fn ((: n Int64)) (: (list) (List Int64)))) (export mk)))
  (call   mk (: 0 Int64))
  (output (: (list) (List Int64))))

; A VARIABLE-LENGTH collection (List/Map/Set) closure RESULT on the MULTI-EXPORT path — N same-signature
; closures each returning a List/Map/Set share ONE `call` that value-encodes the returned handle against the
; ONE shared shape descriptor (all exports share the result type). The shared `call` recovers each closure's
; code slot from the resource rep, dispatches it, and `value-encode`s its collection result.

(case "multi-export collection result — the first list closure"
  (doc    "Two same-signature closures — `up : () -> (-> Int64 (List Int64))` returns `(list n n+1)`, `dn`
           returns `(list n n-1)`. `call(up-handle, 5)` dispatches then value-encodes → `(: (list 5 6) (List
           Int64))`. Pins the variable-length collection result on the shared-`call` multi-export path.")
  (input  (do (def (up) (fn ((: n Int64)) (list n (+ n 1))))
              (def (dn) (fn ((: n Int64)) (list n (- n 1))))
              (export up) (export dn)))
  (call   up (: 5 Int64))
  (output (: (list 5 6) (List Int64))))

(case "multi-export collection result — the second list closure"
  (doc    "The SAME two-closure program, driving the OTHER export: `call(dn-handle, 5)` → `(: (list 5 4)
           (List Int64))`. Confirms the shared `call` value-encodes whichever closure a handle names (the
           code slot rides in the rep, the descriptor is shared since the type is).")
  (input  (do (def (up) (fn ((: n Int64)) (list n (+ n 1))))
              (def (dn) (fn ((: n Int64)) (list n (- n 1))))
              (export up) (export dn)))
  (call   dn (: 5 Int64))
  (output (: (list 5 4) (List Int64))))

(case "multi-export Set-result closures — three sharing one call"
  (doc    "THREE same-signature Set-returning closures share ONE value-encode `call`. `b(3)` builds `{3, 6}`;
           `call(b-handle, 3)` → `(: ((. Set of) (list 3 6)) (Set Int64))` in canonical member order.")
  (input  (do (def (a) (fn ((: n Int64)) (Set.of (list n n (+ n 1)))))
              (def (b) (fn ((: n Int64)) (Set.of (list n (* n 2)))))
              (def (c) (fn ((: n Int64)) (Set.of (list n))))
              (export a) (export b) (export c)))
  (call   b (: 3 Int64))
  (output (: ((. Set of) (list 3 6)) (Set Int64))))

(case "multi-export Set-result closures — the singleton one"
  (doc    "The SAME three-closure program, driving `c`: `call(c-handle, 9)` → `(: ((. Set of) (list 9)) (Set
           Int64))`. Confirms each of the three shares the one descriptor + value-encodes its own result.")
  (input  (do (def (a) (fn ((: n Int64)) (Set.of (list n n (+ n 1)))))
              (def (b) (fn ((: n Int64)) (Set.of (list n (* n 2)))))
              (def (c) (fn ((: n Int64)) (Set.of (list n))))
              (export a) (export b) (export c)))
  (call   c (: 9 Int64))
  (output (: ((. Set of) (list 9)) (Set Int64))))

; A VARIABLE-LENGTH collection (List/Map/Set) closure RESULT on the MIXED path — a collection-returning
; closure exported ALONGSIDE a plain non-closure export. The closure crosses via the resource envelope
; (`make-<name>` + a shared value-encode `call` returning the value form as `list<u8>`); each plain export
; rides as an ordinary top-level component func. Same value-encode core as the multi-export collection path,
; with the plain-export slots the mixed shape threads.

(case "a List-returning closure alongside a plain export — the closure"
  (doc    "`mk : () -> (-> Int64 (List Int64))` returns `(list n n+1)`, alongside a plain `two : () -> 2`.
           `call(mk-handle, 5)` value-encodes the returned list → `(: (list 5 6) (List Int64))`. Pins the
           variable-length collection result on the MIXED path (closure + plain export).")
  (input  (do (def (mk) (fn ((: n Int64)) (list n (+ n 1))))
              (def (two) 2)
              (export mk) (export two)))
  (call   mk (: 5 Int64))
  (output (: (list 5 6) (List Int64))))

(case "a List-returning closure alongside a plain export — the plain"
  (doc    "The SAME mixed program, calling the plain `two` → 2 (a bare scalar, NOT a value-form document).
           Confirms the plain top-level export is reachable when a collection-result closure shares the
           component.")
  (input  (do (def (mk) (fn ((: n Int64)) (list n (+ n 1))))
              (def (two) 2)
              (export mk) (export two)))
  (call   two)
  (output (: 2 Int64)))

(case "a Map-returning closure alongside a parameterized plain export — the closure"
  (doc    "`mk : () -> (-> Int64 (Map Int64 Int64))` returns `(map (1 n) (2 2n))`, beside a parameterized
           plain `inc : (Int64) -> Int64`. `call(mk-handle, 10)` → `(: (map (1 10) (2 20)) (Map Int64
           Int64))` in canonical key order.")
  (input  (do (def (mk) (fn ((: n Int64)) (map (1 n) (2 (* n 2)))))
              (def (inc (: x Int64)) (+ x 1))
              (export mk) (export inc)))
  (call   mk (: 10 Int64))
  (output (: (map (1 10) (2 20)) (Map Int64 Int64))))

(case "a Map-returning closure alongside a parameterized plain export — the plain"
  (doc    "The SAME program, calling `inc(41)` = 42. Pins the parameterized plain export reachable beside a
           Map-result closure.")
  (input  (do (def (mk) (fn ((: n Int64)) (map (1 n) (2 (* n 2)))))
              (def (inc (: x Int64)) (+ x 1))
              (export mk) (export inc)))
  (call   inc (: 41 Int64))
  (output (: 42 Int64)))

; A VARIABLE-LENGTH collection (List/Map/Set) result on the DISTINCT-SIG path — closures of DIFFERENT
; signatures each returning a List/Map/Set cross as G distinct resource types, each `call-g<n>` value-encoding
; the returned handle against THAT group's shape descriptor. A collection group, a compound group, a byte-rope
; group, and a scalar group can all coexist in one component (compound templates in the data section;
; collection + byte-rope payloads written past them; scalars by value — none collide).

(case "distinct-sig collection result — the Int64→List closure"
  (doc    "`mki : () -> (-> Int64 (List Int64))` returns `(list n n+1)`, `mkb : () -> (-> Bool (List Int64))`
           returns `(list (if b 1 0))` — distinct arg types → two resource types, each `call-g<n>` value-
           encoding its own result. `call(mki-handle, 5)` → `(: (list 5 6) (List Int64))`.")
  (input  (do (def (mki) (fn ((: n Int64)) (list n (+ n 1))))
              (def (mkb) (fn ((: b Bool)) (list (if b 1 0))))
              (export mki) (export mkb)))
  (call   mki (: 5 Int64))
  (output (: (list 5 6) (List Int64))))

(case "distinct-sig collection result — the Bool→List closure"
  (doc    "The SAME two-resource program, driving the OTHER signature: `call(mkb-handle, true)` → `(: (list
           1) (List Int64))`. Confirms each distinct-sig group value-encodes its own result.")
  (input  (do (def (mki) (fn ((: n Int64)) (list n (+ n 1))))
              (def (mkb) (fn ((: b Bool)) (list (if b 1 0))))
              (export mki) (export mkb)))
  (call   mkb (: true Bool))
  (output (: (list 1) (List Int64))))

(case "distinct-sig: a collection + a compound + a byte-rope + a scalar group all coexist — the collection"
  (doc    "FOUR distinct signatures, FOUR result MODES in one component: `lst` a List (value-encode), `pr` a
           tuple (fixed template), `byt` a Bytes (raw byte-rope), `inc` an Int64 (by value). `call(lst-handle,
           7)` → `(: (list 7 8) (List Int64))`. Pins the full disjoint-memory layout (compound template region
           + value-encode/byte-rope payloads past it + scalar-by-value all coexisting).")
  (input  (do (def (lst) (fn ((: n Int64)) (list n (+ n 1))))
              (def (pr) (fn ((: b Bool)) (tuple b (if b 1 0))))
              (def (byt) (fn ((: x Int64)) (bin (u8 (UInt8.wrap x)))))
              (def (inc) (fn ((: y Int64)) (+ y 1)))
              (export lst) (export pr) (export byt) (export inc)))
  (call   lst (: 7 Int64))
  (output (: (list 7 8) (List Int64))))

(case "distinct-sig: a collection + a compound + a byte-rope + a scalar group — the compound"
  (doc    "The SAME 4-mode program, driving the COMPOUND group: `call(pr-handle, false)` → `(: (tuple false
           0) (Tuple Bool Int64))` (a fixed-shape template, distinct from the value-encoded collection).")
  (input  (do (def (lst) (fn ((: n Int64)) (list n (+ n 1))))
              (def (pr) (fn ((: b Bool)) (tuple b (if b 1 0))))
              (def (byt) (fn ((: x Int64)) (bin (u8 (UInt8.wrap x)))))
              (def (inc) (fn ((: y Int64)) (+ y 1)))
              (export lst) (export pr) (export byt) (export inc)))
  (call   pr (: false Bool))
  (output (: (tuple false 0) (Tuple Bool Int64))))

(case "distinct-sig: a collection + a compound + a byte-rope + a scalar group — the byte-rope"
  (doc    "The SAME program's byte-rope group: `call(byt-handle, 65)` → `(65)` (a raw byte list, written past
           the compound template region).")
  (input  (do (def (lst) (fn ((: n Int64)) (list n (+ n 1))))
              (def (pr) (fn ((: b Bool)) (tuple b (if b 1 0))))
              (def (byt) (fn ((: x Int64)) (bin (u8 (UInt8.wrap x)))))
              (def (inc) (fn ((: y Int64)) (+ y 1)))
              (export lst) (export pr) (export byt) (export inc)))
  (call   byt (: 65 Int64))
  (output (65)))

(case "distinct-sig: a collection + a compound + a byte-rope + a scalar group — the scalar"
  (doc    "The SAME program's scalar group: `call(inc-handle, 41)` → 42 (by value, NOT list<u8>). Confirms
           the scalar `call-<g>` is unaffected by the three sibling list-returning groups.")
  (input  (do (def (lst) (fn ((: n Int64)) (list n (+ n 1))))
              (def (pr) (fn ((: b Bool)) (tuple b (if b 1 0))))
              (def (byt) (fn ((: x Int64)) (bin (u8 (UInt8.wrap x)))))
              (def (inc) (fn ((: y Int64)) (+ y 1)))
              (export lst) (export pr) (export byt) (export inc)))
  (call   inc (: 41 Int64))
  (output (: 42 Int64)))

; A VARIABLE-LENGTH collection (List/Map/Set) result on the ROUND-TRIP path — a consumer takes a produced
; closure back, applies it, and RETURNS a List/Map/Set, value-encoded against its shape descriptor. This
; closes the collection-result surface across EVERY closure shape. A collection consumer coexists with a
; scalar consumer of the same closure.

(case "round-trip: a consumer applies the handed-back closure and returns a List"
  (doc    "`mk : () -> (-> Int64 Int64)` (adds 1); `app : (own<t>, Int64) -> (List Int64)` returns `(list x (g
           x))`. Host produces via `mk`, hands to `app(handle, 5)` → the closure yields 6, so `value-encode`
           renders `(: (list 5 6) (List Int64))`. Pins the variable-length collection result on the round-trip
           path.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (list 5 6) (List Int64))))

(case "round-trip: a consumer returns a Set built from the closure result"
  (doc    "`mk` doubles; `app : (own<t>, Int64) -> (Set Int64)` = `(Set.of (list x (g x) x))`. `app(handle,
           3)` → `{3, 6}` → `(: ((. Set of) (list 3 6)) (Set Int64))` in canonical member order.")
  (input  (do (def (mk) (fn ((: n Int64)) (* n 2)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (Set.of (list x (g x) x)))
              (export mk) (export app)))
  (call   app (: 3 Int64))
  (output (: ((. Set of) (list 3 6)) (Set Int64))))

(case "round-trip: a consumer returns a Map from the closure result"
  (doc    "`mk` adds 100; `app : (own<t>, Int64) -> (Map Int64 Int64)` = `(map (0 x) (1 (g x)))`. `app(handle,
           5)` → `(: (map (0 5) (1 105)) (Map Int64 Int64))` in canonical key order.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 100)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (map (0 x) (1 (g x))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (map (0 5) (1 105)) (Map Int64 Int64))))

(case "round-trip: a scalar consumer + a List consumer of the same closure — the list"
  (doc    "One closure signature, TWO consumers: `asnum` returns the value, `aslist` returns `(list x (g x))`.
           `aslist(handle, 8)` → `(: (list 8 9) (List Int64))`. Pins a scalar consumer and a collection
           (value-encode) consumer of the same resource coexisting.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (aslist (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (export mk) (export asnum) (export aslist)))
  (call   aslist (: 8 Int64))
  (output (: (list 8 9) (List Int64))))

(case "round-trip: a scalar consumer + a List consumer of the same closure — the scalar"
  (doc    "The SAME two-consumer program, driving the SCALAR consumer: `asnum(handle, 8)` → 9 (by value, NOT
           a value-encoded document). Confirms the scalar consumer is unaffected by the sibling collection
           consumer's value-encode.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (def (aslist (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (export mk) (export asnum) (export aslist)))
  (call   asnum (: 8 Int64))
  (output (: 9 Int64)))

; A VARIABLE-LENGTH collection (List/Map/Set) consumer RESULT on the DISTINCT-SIGNATURE ROUND-TRIP path —
; closures of DIFFERENT signatures each cross as their own resource type, and a consumer of one of them
; applies its handed-back closure and RETURNS a collection. That collection crosses as `list<u8>` rendered by
; the runtime `value-encode(rep, desc)` op against the consumer's OWN shape descriptor, written PAST all
; compound-template data (disjoint memory) — the last collection sub-shape. A collection consumer and a
; scalar/compound/byte-rope consumer of another signature coexist in one component.

(case "distinct-sig round-trip: a List consumer + a scalar consumer of another sig — the list"
  (doc    "`mka : () -> (-> Int64 Int64)`, `mkb : () -> (-> Bool Int64)` are distinct sigs → two resource
           types. `appa : (own<t0>, Int64) -> (List Int64)` returns `(list x (g x))`. Host produces via `mka`,
           hands to `appa(handle, 5)` → the closure yields 6, so `value-encode` renders `(: (list 5 6) (List
           Int64))`. Pins the variable-length collection consumer result on the distinct-sig round-trip path.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 5 Int64))
  (output (: (list 5 6) (List Int64))))

(case "distinct-sig round-trip: a List consumer + a scalar consumer of another sig — the scalar"
  (doc    "The SAME two-resource-type program, driving the SCALAR consumer of the OTHER signature: `appb :
           (own<t1>, Bool) -> Int64` → `appb(handle, true)` = 10 (by value, NOT a value-encoded document).
           Confirms the scalar consumer is unaffected by the sibling collection consumer's memory/value-encode.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: true Bool))
  (output (: 10 Int64)))

(case "distinct-sig round-trip: TWO collection consumers of different sigs — the List"
  (doc    "Both consumers return a collection of DIFFERENT signature: `appa` a List, `appb` a Map.
           `appa(mka-handle, 40)` → `(: (list 40 41) (List Int64))`. Each consumer value-encodes against its
           OWN per-consumer shape descriptor.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (map (0 (h y))))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 40 Int64))
  (output (: (list 40 41) (List Int64))))

(case "distinct-sig round-trip: TWO collection consumers of different sigs — the Map"
  (doc    "The SAME program's OTHER consumer: `appb(mkb-handle, true)` → `(: (map (0 7)) (Map Int64 Int64))`.
           Confirms each distinct-sig consumer value-encodes its own descriptor.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (map (0 (h y))))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: true Bool))
  (output (: (map (0 7)) (Map Int64 Int64))))

(case "distinct-sig round-trip: a List consumer + a compound consumer of another sig — the list"
  (doc    "A COLLECTION consumer (`appa` → List, value-encode) AND a COMPOUND consumer (`appb` → tuple, static
           value-form template) of DISTINCT signatures coexist. `appa(mka-handle, 3)` → `(: (list 3 4) (List
           Int64))` — its value-encoded doc written PAST the sibling's compound template (disjoint memory).")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (tuple y (h y)))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 3 Int64))
  (output (: (list 3 4) (List Int64))))

(case "distinct-sig round-trip: a List consumer + a compound consumer of another sig — the compound"
  (doc    "The SAME program's OTHER consumer: `appb(mkb-handle, false)` → `(: (tuple false 8) (Tuple Bool
           Int64))`. Confirms the compound consumer walks its own template while a sibling collection consumer
           value-encodes — three result-assembly mechanisms coexisting across two resource types.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (list x (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (tuple y (h y)))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: false Bool))
  (output (: (tuple false 8) (Tuple Bool Int64))))

; A COMPOUND closure ARGUMENT on the ROUND-TRIP path — the closure `g` takes a Tuple/Record/List/Map/Set. On
; the round-trip path the consumer APPLIES the handed-back closure ITSELF, in-guest (`(g <compound>)` inside
; the consumer body), so the closure's argument is BUILT in the guest and NEVER crosses the host boundary —
; only the closure HANDLE (an `own<t>` resource, i32) and the consumer's own scalar params cross. So a
; compound closure argument need only be MACHINE-representable (a value-heap handle, i32), not scalar-boundary.
; This lifts the earlier "a closure argument of type … has no scalar host-boundary representation" fence for
; the round trip. (A compound closure arg on the DIRECT-CALL path — where the HOST supplies the argument —
; still declines: that would need a host→guest decode of the compound into the guest heap.)

(case "round-trip: a consumer applies a closure taking a Tuple arg built in-guest"
  (doc    "`mk : () -> (-> (Tuple Int64 Int64) Int64)` sums the pair; `app : (own<t>, Int64) -> Int64` applies
           the handed-back closure to a guest-built `(tuple x x)`. `app(handle, 5)` → `g((tuple 5 5))` = 10.
           Pins a COMPOUND (Tuple) closure argument crossing the round trip (built in-guest, never over the
           boundary).")
  (input  (do (def (mk) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (def (app (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g (tuple x x)))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 10 Int64)))

(case "round-trip: a consumer applies a closure taking a Record arg built in-guest"
  (doc    "`mk : () -> (-> (Record (a Int64) (b Int64)) Int64)` multiplies the two fields; `app` applies it to
           a guest-built `(record (a x) (b x+1))`. `app(handle, 6)` → `g((record (a 6) (b 7)))` = 42. A RECORD
           closure argument crosses the round trip (field names are compile-time-only; the value is an i32
           heap handle in-guest).")
  (input  (do (def (mk) (fn ((: r (Record (a Int64) (b Int64)))) (* (. r a) (. r b))))
              (def (app (: g (-> (Record (a Int64) (b Int64)) Int64)) (: x Int64))
                (g (record (a x) (b (+ x 1)))))
              (export mk) (export app)))
  (call   app (: 6 Int64))
  (output (: 42 Int64)))

(case "round-trip: a consumer applies a closure taking a List arg built in-guest"
  (doc    "`mk : () -> (-> (List Int64) Int64)` takes the list length; `app` applies it to a guest-built
           `(list x x x)`. `app(handle, 9)` → `g((list 9 9 9))` = `(. List len)` = 3. A VARIABLE-LENGTH
           collection closure argument crosses the round trip (an i32 persistent-vector handle in-guest).")
  (input  (do (def (mk) (fn ((: xs (List Int64))) ((. List len) xs)))
              (def (app (: g (-> (List Int64) Int64)) (: x Int64)) (g (list x x x)))
              (export mk) (export app)))
  (call   app (: 9 Int64))
  (output (: 3 Int64)))

(case "round-trip: a compound-arg closure whose consumer returns a compound"
  (doc    "The compound closure ARGUMENT and a compound consumer RESULT compose: `g : (-> (Tuple Int64 Int64)
           Int64)` returns the pair's first element; `app` returns `(tuple x (g (tuple x+1 x)))`.
           `app(handle, 7)` → `g((tuple 8 7))` = 8, so `(: (tuple 7 8) (Tuple Int64 Int64))`. A guest-built
           compound arg feeds the closure, and the consumer's own compound result is value-form-encoded out.")
  (input  (do (def (mk) (fn ((: p (Tuple Int64 Int64))) (. p 0)))
              (def (app (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64))
                (tuple x (g (tuple (+ x 1) x))))
              (export mk) (export app)))
  (call   app (: 7 Int64))
  (output (: (tuple 7 8) (Tuple Int64 Int64))))

; The SAME compound-closure-argument relaxation applies to the DISTINCT-SIGNATURE round-trip — closures of
; different signatures each cross as their own resource type, and each is applied in-guest by its consumer, so
; a compound argument is built guest-side and never crosses the boundary. Only the closure signature's fence
; is widened (machine-representable rather than scalar-boundary); the per-group resource machinery is unchanged.

(case "distinct-sig round-trip: a compound-arg closure + a scalar-arg closure of another sig — the compound-arg one"
  (doc    "`mka : () -> (-> (Tuple Int64 Int64) Int64)`, `mkb : () -> (-> Bool Int64)` are distinct sigs → two
           resource types. `appa : (own<t0>, Int64) -> Int64` applies its handed-back closure to a guest-built
           `(tuple x x)`. `appa(handle, 5)` → `g((tuple 5 5))` = 10. Pins a COMPOUND closure argument on the
           distinct-sig round-trip path (built in-guest, one of two resource types).")
  (input  (do (def (mka) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
              (def (appa (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g (tuple x x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 5 Int64))
  (output (: 10 Int64)))

(case "distinct-sig round-trip: a compound-arg closure + a scalar-arg closure of another sig — the scalar-arg one"
  (doc    "The SAME two-resource-type program, driving the OTHER (scalar-arg) closure of the other signature:
           `appb : (own<t1>, Bool) -> Int64` → `appb(handle, true)` = 10. Confirms the scalar-arg group is
           unaffected by the sibling compound-arg group.")
  (input  (do (def (mka) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
              (def (appa (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g (tuple x x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: true Bool))
  (output (: 10 Int64)))

(case "distinct-sig round-trip: TWO compound-arg closures of different sigs — the Tuple-arg one"
  (doc    "Both closures take a DIFFERENT compound: `g` a Tuple, `h` a Record → two resource types.
           `appa : (own<t0>, Int64) -> Int64` applies `g` to `(tuple x+1 x)`. `appa(handle, 7)` →
           `g((tuple 8 7))` = 8-7 = 1. Each group's closure takes its own compound argument built in-guest.")
  (input  (do (def (mka) (fn ((: p (Tuple Int64 Int64))) (- (. p 0) (. p 1))))
              (def (mkb) (fn ((: r (Record (a Int64) (b Int64)))) (* (. r a) (. r b))))
              (def (appa (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g (tuple (+ x 1) x)))
              (def (appb (: h (-> (Record (a Int64) (b Int64)) Int64)) (: y Int64))
                (h (record (a y) (b y))))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 7 Int64))
  (output (: 1 Int64)))

(case "distinct-sig round-trip: TWO compound-arg closures of different sigs — the Record-arg one"
  (doc    "The SAME program's OTHER closure: `appb : (own<t1>, Int64) -> Int64` applies `h` to a guest-built
           `(record (a y) (b y))`. `appb(handle, 6)` → `h((record (a 6) (b 6)))` = 36. Confirms each distinct
           signature threads its own compound argument through its own resource type.")
  (input  (do (def (mka) (fn ((: p (Tuple Int64 Int64))) (- (. p 0) (. p 1))))
              (def (mkb) (fn ((: r (Record (a Int64) (b Int64)))) (* (. r a) (. r b))))
              (def (appa (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g (tuple (+ x 1) x)))
              (def (appb (: h (-> (Record (a Int64) (b Int64)) Int64)) (: y Int64))
                (h (record (a y) (b y))))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: 6 Int64))
  (output (: 36 Int64)))

; The in-guest-argument relaxation reaches every MACHINE-representable argument, not just fixed-shape
; compounds: a SUM (Option/Result), a NESTED compound, a String/Bytes, and — most notably — a closure-TYPED
; argument all cross the round trip, because each is built in the guest and only the outer closure HANDLE
; travels. A HIGHER-ORDER closure (`(-> (-> A B) R)`) handed back and applied to a guest-built inner closure
; needs NO extra resource machinery: the inner closure is an ordinary in-guest funcref-table value (an i32
; slot, `valtype_of(Ty::Fn)`), applied by the outer via the usual `call_indirect`.

(case "round-trip: a closure taking a SUM (Option) arg built in-guest"
  (doc    "`mk : () -> (-> (Option Int64) Int64)` unwraps with a default; `app` applies it to a guest-built
           `(Some x)`. `app(handle, 7)` → `g((Some 7))` = 7. A SUM closure argument crosses the round trip (an
           i32 sum handle in-guest).")
  (input  (do (def (mk) (fn ((: o (Option Int64))) (match o ((Some v) v) (None 0))))
              (def (app (: g (-> (Option Int64) Int64)) (: x Int64)) (g (Some x)))
              (export mk) (export app)))
  (call   app (: 7 Int64))
  (output (: 7 Int64)))

(case "round-trip: a closure taking a NESTED compound (Tuple of Tuples) arg"
  (doc    "`mk`'s closure reads `(. (. p 0) 0) + (. p 1)`; `app` applies it to a guest-built
           `(tuple (tuple x x) x)`. `app(handle, 5)` → `g((tuple (tuple 5 5) 5))` = 5 + 5 = 10. A NESTED
           compound argument crosses (still one i32 handle at the top).")
  (input  (do (def (mk) (fn ((: p (Tuple (Tuple Int64 Int64) Int64))) (+ (. (. p 0) 0) (. p 1))))
              (def (app (: g (-> (Tuple (Tuple Int64 Int64) Int64) Int64)) (: x Int64))
                (g (tuple (tuple x x) x)))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 10 Int64)))

(case "round-trip: a closure taking a String arg built in-guest"
  (doc    "`mk`'s closure takes the byte length of a String; `app` applies it to a guest-built literal
           `\"hello\"`. `app(handle, 0)` → `g(\"hello\")` = 5. A byte-rope (String) closure argument crosses the
           round trip (an i32 rope handle in-guest).")
  (input  (do (def (mk) (fn ((: s String)) ((. String byte-len) s)))
              (def (app (: g (-> String Int64)) (: x Int64)) (g "hello"))
              (export mk) (export app)))
  (call   app (: 0 Int64))
  (output (: 5 Int64)))

(case "round-trip: a HIGHER-ORDER closure — its argument is itself a closure built in-guest"
  (doc    "`mk : () -> (-> (-> Int64 Int64) Int64)` applies its function argument to 10; `app` hands it a
           guest-built capturing closure `(fn (y) (+ y x))`. `app(handle, 5)` → `g((fn y -> y+5))` = 15. A
           CLOSURE-TYPED argument crosses the round trip with NO extra resource machinery: the inner closure
           is an ordinary in-guest funcref-table value (an i32 slot), applied by the outer via
           `call_indirect`. Only the OUTER closure handle crosses the host boundary.")
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
              (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (+ y x))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 15 Int64)))

(case "round-trip: a higher-order closure whose inner closure CAPTURES and is applied twice"
  (doc    "`mk`'s closure applies its function arg to BOTH 10 and 20 and sums; `app` hands in a guest-built
           capturing `(fn (y) (* y x))`. `app(handle, 3)` → `g((fn y -> y*3))` = 3*10 + 3*20 = 90. Stresses a
           captured, MULTIPLY-APPLIED inner closure — a wrong funcref slot would give a wrong value.")
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (+ (f 10) (f 20))))
              (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (* y x))))
              (export mk) (export app)))
  (call   app (: 3 Int64))
  (output (: 90 Int64)))

(case "round-trip: a higher-order closure applied to TWO distinct inner closures"
  (doc    "`mk`'s closure applies its function arg to 100; `app` calls the handed-back `g` on TWO different
           guest-built inner closures — `(fn y -> y+x)` and `(fn y -> y*x)` — and sums the results.
           `app(handle, 4)` → `g((fn y->y+4)) + g((fn y->y*4))` = (100+4) + (100*4) = 104 + 400 = 504. Confirms
           two distinct inner closures are NOT crossed (each resolves its own funcref slot).")
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (f 100)))
              (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64))
                (+ (g (fn (y) (+ y x))) (g (fn (y) (* y x)))))
              (export mk) (export app)))
  (call   app (: 4 Int64))
  (output (: 504 Int64)))

(case "distinct-sig round-trip: a higher-order closure + a scalar closure of another sig — the higher-order one"
  (doc    "`mka : () -> (-> (-> Int64 Int64) Int64)` (applies its function arg to 1 and 2, sums) and
           `mkb : () -> (-> Bool Int64)` are distinct sigs → two resource types. `appa` hands `g` a guest-built
           `(fn (y) (* y x))`. `appa(handle, 5)` → `g((fn y->y*5))` = 5*1 + 5*2 = 15. A closure-typed argument
           on the DISTINCT-SIG round-trip path.")
  (input  (do (def (mka) (fn ((: f (-> Int64 Int64))) (+ (f 1) (f 2))))
              (def (mkb) (fn ((: b Bool)) (: (if b 100 200) Int64)))
              (def (appa (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (* y x))))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 5 Int64))
  (output (: 15 Int64)))

(case "a fixed-shape scalar Tuple closure ARG crosses the DIRECT-CALL boundary (host supplies the tuple)"
  (doc    "A single closure export whose closure takes a `(Tuple Int64 Int64)`, called DIRECTLY by the host
           (no consumer to apply it in-guest). This USED to decline (recorded as needing a nonexistent
           `value-decode` runtime op / out of scope), but that conflated two cases: a FIXED-SHAPE SCALAR
           tuple does NOT need runtime decode. It crosses as a native component `tuple<s64,s64>` type, which
           the canonical ABI FLATTENS into scalar core params; the guest `call` wrapper rebuilds the tuple
           cell in-guest from the flat fields with the ORDINARY `arr-alloc`/`box-int`/`arr-set` ops (the
           `TupleArgRebuild` serializer path), then dispatches `call_indirect`. `make()` → the closure
           handle; `call(handle, (3, 4))` → `(. p 0) + (. p 1)` = 7. Proved by the
           `a_fixed_shape_tuple_closure_arg_crosses_by_native_flattening` oracle + the real emit pipeline.
           (A VARIABLE-LENGTH collection arg genuinely still needs runtime decode — out of scope.)")
  (input  (do (def (mk) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (export mk)))
  (call   mk (: (tuple 3 4) (Tuple Int64 Int64)))
  (output (: 7 Int64)))

(case "a fixed-shape scalar RECORD closure ARG crosses the DIRECT-CALL boundary"
  (doc    "Like the tuple-arg case but the closure argument is a RECORD `(Record (a Int64) (b Int64))`. A
           record of aliased-width scalars flattens the same way (its fields in canonical SORTED-key order —
           the value-heap cell's field order), so the guest `call` rebuilds the record cell from the flat
           fields. `call(handle, (record 3 4))` → `(. p a) + (. p b)` = 7. (The corpus arg is the value form
           in field order; the `record` head token is dropped by the runner's tuple-literal parser.)")
  (input  (do (def (mk) (fn ((: p (Record (a Int64) (b Int64)))) (+ (. p a) (. p b))))
              (export mk)))
  (call   mk (: (record 3 4) (Record (a Int64) (b Int64))))
  (output (: 7 Int64)))

(case "a NARROW-int-field Tuple closure ARG flattens + rebuilds (exercises the i32->i64 extend)"
  (doc    "A `(Tuple Int32 Int32)` closure arg: each field crosses as a component `s32` (an i32 core param),
           so the cell rebuild SIGN-EXTENDS each field i32→i64 before `box-int` (the value-heap cell holds
           i64-boxed ints). Distinct from the Int64 case, which needs no extend. `call(handle, (100, 23))`
           → 123, proving the narrow-field extend path in `TupleArgRebuild`.")
  (input  (do (def (mk) (fn ((: p (Tuple Int32 Int32))) (+ (. p 0) (. p 1))))
              (export mk)))
  (call   mk (: (tuple 100 23) (Tuple Int32 Int32)))
  (output (: 123 Int32)))

(case "a CAPTURING closure taking a Tuple ARG crosses the DIRECT-CALL boundary"
  (doc    "The tuple-arg path composes with capture (C-HOST-2): a parameterized export `(def (mk (: k
           Int64)) …)` returns a closure that BOTH captures `k` AND takes a `(Tuple Int64 Int64)` argument.
           `make(10)` → a handle closing over k=10; `call(handle, (3, 4))` → `(. p 0) + (. p 1) + k` = 17.
           The make-forwarded capture cell and the rebuilt arg cell coexist in the one `call`.")
  (input  (do (def (mk (: k Int64)) (fn ((: p (Tuple Int64 Int64))) (+ (+ (. p 0) (. p 1)) k)))
              (export mk)))
  (call   mk (: 10 Int64) (: (tuple 3 4) (Tuple Int64 Int64)))
  (output (: 17 Int64)))

(case "MULTI-EXPORT: two same-sig Tuple-arg closures share one direct-call `call`"
  (doc    "The direct-call fixed-shape compound-arg path extends to the MULTI-EXPORT shape: N same-signature
           closures (`mk-sum`, `mk-diff`, both `(-> (Tuple Int64 Int64) Int64)`) cross as N `make-<name>`s
           sharing ONE `call` whose single argument is a native component `tuple<s64,s64>` — the shared `call`
           rebuilds the tuple cell from the flattened fields (the same `TupleArgRebuild` the single-export
           path uses), dispatched through the guest's funcref table by the handle's resource rep. The host
           `make-diff()` → a handle, `call(handle, (10, 3))` → `(. p 0) - (. p 1)` = 7. The envelope mints
           the `tuple<…>` defined type in the SHARED `call` functype (outer lift + nested re-export).")
  (input  (do (def (mk-sum) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (def (mk-diff) (fn ((: p (Tuple Int64 Int64))) (- (. p 0) (. p 1))))
              (export mk-sum) (export mk-diff)))
  (call   mk-diff (: (tuple 10 3) (Tuple Int64 Int64)))
  (output (: 7 Int64)))

(case "MIXED: a Tuple-arg closure export ALONGSIDE a plain (non-closure) export"
  (doc    "The direct-call fixed-shape compound-arg path extends to the MIXED shape: a tuple-arg closure
           factory `mk : (-> (Tuple Int64 Int64) Int64)` crosses via the resource envelope's `make`+shared
           `call` (the `call` takes a native `tuple<s64,s64>` rebuilt from the flattened fields) WHILE a
           plain (non-closure) export `twice` rides alongside as an ordinary top-level component func. Both
           coexist in one component. Driving the CLOSURE: `make()` → handle, `call(handle, (3, 4))` → 7.")
  (input  (do (def (mk) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (def (twice (: n Int64)) (* n 2))
              (export mk) (export twice)))
  (call   mk (: (tuple 3 4) (Tuple Int64 Int64)))
  (output (: 7 Int64)))

(case "MIXED: driving the PLAIN export alongside a Tuple-arg closure"
  (doc    "The SAME mixed component as above, but the trial drives the PLAIN export `twice` (an ordinary
           top-level func) — proving it coexists with the tuple-arg closure interface and is reachable by
           name. `twice(21)` → 42. Companion to the closure-driving trial above.")
  (input  (do (def (mk) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (def (twice (: n Int64)) (* n 2))
              (export mk) (export twice)))
  (call   twice (: 21 Int64))
  (output (: 42 Int64)))

(case "DISTINCT-SIG: two Tuple-arg closures of DIFFERENT signatures each cross the direct-call boundary"
  (doc    "The direct-call fixed-shape compound-arg path extends to the DISTINCT-SIGNATURE shape: two
           closures taking the SAME `(Tuple Int64 Int64)` arg but returning DIFFERENT types (`mk-sum` → Int64,
           `mk-eq` → Bool) cross as TWO resource types, each with its own `make-<name>` + `call-g<n>`. Each
           group's `call-g<n>` takes a native `tuple<s64,s64>` rebuilt from the flattened fields (per-group
           `TupleArgRebuild`). Driving the Int64 group: `make-sum()` → handle, `call(handle, (3, 4))` → 7.
           (The Bool group is exercised by the companion trial.)")
  (input  (do (def (mk-sum) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (def (mk-eq) (fn ((: p (Tuple Int64 Int64))) (= (. p 0) (. p 1))))
              (export mk-sum) (export mk-eq)))
  (call   mk-sum (: (tuple 3 4) (Tuple Int64 Int64)))
  (output (: 7 Int64)))

(case "DISTINCT-SIG: driving the Bool-returning Tuple-arg closure of the distinct-sig pair"
  (doc    "The SAME distinct-sig component, driving the Bool group `mk-eq : (-> (Tuple Int64 Int64) Bool)` —
           its own resource type + `call-g<n>` taking a `tuple<s64,s64>`. `make-eq()` → handle,
           `call(handle, (5, 5))` → `(= (. p 0) (. p 1))` = true. Companion to the Int64-group trial above.")
  (input  (do (def (mk-sum) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
              (def (mk-eq) (fn ((: p (Tuple Int64 Int64))) (= (. p 0) (. p 1))))
              (export mk-sum) (export mk-eq)))
  (call   mk-eq (: (tuple 5 5) (Tuple Int64 Int64)))
  (output (: true Bool)))

; A fixed-shape compound ARGUMENT now composes with a BYTE-ROPE (`Bytes`/`String`) result: the bytes-result
; core serializer + its envelope thread the `TupleArgRebuild`, so the `call` rebuilds the flattened tuple cell
; then copies its byte-rope result out as `list<u8>`. (A COMPOUND value-form or a variable-length COLLECTION
; result combined with a tuple arg still declines — those two cores don't yet thread the rebuild; see the
; decline anchor below.)

(case "a fixed-shape Tuple ARG with a Bytes RESULT crosses the direct-call boundary"
  (doc    "`(fn (p) (bin (u8 (. p 0)) (u8 (. p 1))))` — a `(Tuple Int64 Int64)` argument AND a `Bytes` result.
           The tuple crosses flattened as a native `tuple<s64,s64>` the `call` rebuilds in-guest; the closure's
           `Bytes` result copies out as `list<u8>`. `make()` → handle, `call(handle, (5, 6))` → the two bytes
           `(5 6)`. Proves the tuple-arg rebuild threads through the byte-rope-result core + envelope.")
  (input  (do (def (mk) (fn ((: p (Tuple Int64 Int64))) (bin (u8 (. p 0)) (u8 (. p 1)))))
              (export mk)))
  (call   mk (: (tuple 5 6) (Tuple Int64 Int64)))
  (output (: (5 6) Bytes)))

; A fixed-shape compound ARGUMENT combined with a fixed-shape COMPOUND or a variable-length COLLECTION result
; is still not emitted: the value-form (`closure_value_resource_core_module`) + value-encode
; (`closure_value_encode_resource_core_module`) result cores inline their own `call` bodies and do NOT yet
; thread the `TupleArgRebuild`, while their envelope takes the scalar `arg_bytes` (empty for a tuple arg) — so
; combining the two would emit a scalar-arg envelope over a flattened-field core, an INVALID component. The
; compiler DECLINES cleanly (a `todo`). A scalar-result OR byte-rope-result compound arg works (the cases
; above); threading the rebuild through those two remaining list-result serializers is a later widening.

(case "a fixed-shape Tuple ARG with a Tuple RESULT is declined (value-form result core lacks the rebuild)"
  (doc    "`(fn (p) (tuple (+ (. p 0) (. p 1)) (- (. p 0) (. p 1))))` — a `(Tuple Int64 Int64)` argument AND a
           `(Tuple Int64 Int64)` result. The arg would cross flattened as a native `tuple<s64,s64>` (rebuilt
           in-guest), but the compound-RESULT core serializer does not yet thread that rebuild, so the compiler
           DECLINES rather than emit an invalid component (a scalar-arg envelope over a flattened-field core).
           A scalar-result tuple arg works; this compound-arg + compound-result combination is a later widening.")
  (input  (do (def (mk) (fn ((: p (Tuple Int64 Int64)))
                         (tuple (+ (. p 0) (. p 1)) (- (. p 0) (. p 1)))))
              (export mk)))
  (call   mk (: (tuple 10 3) (Tuple Int64 Int64)))
  (output (: (tuple 13 7) (Tuple Int64 Int64))))

; A higher-order closure whose INNER closure has an UNANNOTATED COMPOUND parameter now compiles: the inner
; `(fn (p) …)` param `p` types `Any` bottom-up (no annotation, no def entry), but the higher-order parameter
; `g`'s DECLARED arrow `(-> (-> (Tuple …) R) R)` fixes it — `expected_arrow_for_lambda` recovers the inner
; lambda's expected type from a FUNCTION-VALUED head (a variable of function type), not only a lambda/def
; head. So the inner param solves to `(Tuple …)` (an i32 heap handle), matching the explicit-annotation form.

(case "round-trip: a higher-order closure whose inner closure takes an UNANNOTATED compound param"
  (doc    "`mk : () -> (-> (-> (Tuple Int64 Int64) Int64) Int64)` applies its function arg to `(tuple 3 4)`;
           `app` hands `g` a guest-built `(fn (p) (+ (+ (. p 0) (. p 1)) x))` — the inner param `p` is
           UNANNOTATED. Its type is recovered from `g`'s declared arrow `(-> (Tuple Int64 Int64) Int64)`.
           `app(handle, 10)` → `g((fn p -> p.0+p.1+10))` applied to `(tuple 3 4)` = 3+4+10 = 17. Without the
           context recovery the inner param solved `Any` and declined `a closure's parameter type has no
           machine representation`; now it matches the explicit `(: p (Tuple Int64 Int64))` form.")
  (input  (do (def (mk) (fn ((: f (-> (Tuple Int64 Int64) Int64))) (f (tuple 3 4))))
              (def (app (: g (-> (-> (Tuple Int64 Int64) Int64) Int64)) (: x Int64))
                (g (fn (p) (+ (+ (. p 0) (. p 1)) x))))
              (export mk) (export app)))
  (call   app (: 10 Int64))
  (output (: 17 Int64)))

(case "round-trip: an UNANNOTATED inner closure with a List param via the context arrow"
  (doc    "The same context recovery for a variable-length collection param: `mk`'s closure applies its
           function arg to `(list 1 2 3)`; `app` hands `g` a guest-built `(fn (xs) (+ ((. List len) xs) x))`
           whose param `xs` is UNANNOTATED, recovered as `(List Int64)` from `g`'s arrow. `app(handle, 100)` →
           `g((fn xs -> len(xs)+100))` applied to `(list 1 2 3)` = 3 + 100 = 103.")
  (input  (do (def (mk) (fn ((: f (-> (List Int64) Int64))) (f (list 1 2 3))))
              (def (app (: g (-> (-> (List Int64) Int64) Int64)) (: x Int64))
                (g (fn (xs) (+ ((. List len) xs) x))))
              (export mk) (export app)))
  (call   app (: 100 Int64))
  (output (: 103 Int64)))

(case "a closure-typed closure ARG on the DIRECT-CALL path is declined — host would supply the closure"
  (doc    "A single higher-order closure export called DIRECTLY by the host: the host would have to supply the
           `(-> Int64 Int64)` function argument OVER the boundary (itself a closure resource passed INTO a
           call), which the current envelope does not accept. Declines (a `todo`); contrast the round-trip
           cases above, where the inner closure is built in-guest.")
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
              (export mk)))
  (call   mk (: 0 Int64))
  (output (: 10 Int64)))

; A SUM (Option/Result/user sum) result, and a fixed-shape COMPOUND result CONTAINING a variable-length
; element (a tuple/record with a List/Map/Set inside), cross as `list<u8>` via the runtime `value-encode`
; op against a compiler-baked shape DESCRIPTOR — the same walker a variable-length collection uses,
; generalized. Previously only a scalar, a byte-rope, a FIXED-shape compound (static template), or a bare
; List/Map/Set result crossed; a sum or a nested-collection compound declined "no scalar host-boundary
; representation". This holds on BOTH the direct-call `call` result and the round-trip consumer result.

(case "a closure whose CALL returns an Option crosses as the value form"
  (doc    "`mk : () -> (-> Int64 (Option Int64))` returns `(Some (+ n 1))`; `call(handle, 5)` → `(: (Some 6)
           (Option Int64))`. A SUM closure `call` result renders via the runtime `value-encode` descriptor
           (the disc-switching walker), not a static template.")
  (input  (do (def (mk) (fn ((: n Int64)) (Some (+ n 1))))
              (export mk)))
  (call   mk (: 5 Int64))
  (output (: (Some 6) (Option Int64))))

(case "a closure whose CALL returns a user sum crosses as the value form"
  (doc    "A monomorphic user sum: `(type Dir (N) (S))`; `mk`'s closure returns `(N)` when `n>0` else `(S)`.
           `call(handle, 5)` → `(: (N unit) Dir)` (a nullary variant carries a unit payload in the canonical
           form). The value-encode walker switches on the runtime discriminant.")
  (input  (do (type Dir (N) (S))
              (def (mk) (fn ((: n Int64)) (if (> n 0) (N) (S))))
              (export mk)))
  (call   mk (: 5 Int64))
  (output (: (N unit) Dir)))

(case "a closure whose CALL returns a tuple CONTAINING a list"
  (doc    "A fixed-shape compound whose element is VARIABLE-length has no static template, so it escapes via
           the value-encode descriptor too: `mk`'s closure returns `(tuple (list n n+1) n)`. `call(handle, 5)`
           → `(: (tuple (list 5 6) 5) (Tuple (List Int64) Int64))`. The descriptor's Tuple node recurses into
           the List element.")
  (input  (do (def (mk) (fn ((: n Int64)) (tuple (list n (+ n 1)) n)))
              (export mk)))
  (call   mk (: 5 Int64))
  (output (: (tuple (list 5 6) 5) (Tuple (List Int64) Int64))))

(case "round-trip: a consumer returns an Option built from the closure result"
  (doc    "`mk` adds 1; `app : (own<t>, Int64) -> (Option Int64)` returns `(Some (g x))`. `app(handle, 5)` →
           `g(5)` = 6, so `(: (Some 6) (Option Int64))`. A SUM consumer result on the round-trip path — the
           value-encode descriptor, not a static template.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (Some (g x)))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (Some 6) (Option Int64))))

(case "round-trip: a consumer returns a Result (Err type pinned) from the closure result"
  (doc    "`mk` doubles; `app : (own<t>, Int64) -> (Result Int64 Int64)` returns `(: (Ok (g x)) (Result Int64
           Int64))` — the `Err` type is fixed by the annotation (an unconstrained `Err` type is genuinely
           ambiguous and correctly declines). `app(handle, 7)` → `(: (Ok 14) (Result Int64 Int64))`.")
  (input  (do (def (mk) (fn ((: n Int64)) (* n 2)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (: (Ok (g x)) (Result Int64 Int64)))
              (export mk) (export app)))
  (call   app (: 7 Int64))
  (output (: (Ok 14) (Result Int64 Int64))))

(case "round-trip: a consumer returns a Result reaching BOTH variants"
  (doc    "Both `Ok` and `Err` are reachable, so the `Result` type is fully determined WITHOUT an annotation:
           `app` returns `(Ok (g x))` when `x>0` else `(Err 99)`. `app(handle, 7)` → `(: (Ok 7) (Result Int64
           Int64))`. Confirms a genuinely two-variant sum consumer result renders.")
  (input  (do (def (mk) (fn ((: n Int64)) n))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (if (> x 0) (Ok (g x)) (Err 99)))
              (export mk) (export app)))
  (call   app (: 7 Int64))
  (output (: (Ok 7) (Result Int64 Int64))))

(case "round-trip: a consumer returns a tuple CONTAINING a list built from the closure result"
  (doc    "A nested-collection compound consumer result: `app` returns `(tuple (list x (g x)) x)`.
           `app(handle, 5)` → `g(5)` = 6, so `(: (tuple (list 5 6) 5) (Tuple (List Int64) Int64))`. The tuple's
           List element crosses via the same value-encode descriptor (no static template for a variable
           element).")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (tuple (list x (g x)) x))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (tuple (list 5 6) 5) (Tuple (List Int64) Int64))))

; COMPOSED round-trip shapes — the argument surface (every machine type, incl. higher-order) and the result
; surface (every value-encodable type: scalar, byte-rope, fixed compound, collection, sum, and
; compound-containing-collection) COMPOSE freely, across single-sig and distinct-sig grouping. These lock in
; the full round-trip closure surface end-to-end.

(case "round-trip: a consumer returns a Map whose VALUE is a list"
  (doc    "A `Map Int64 (List Int64)` consumer result — the map's VALUE shape is itself variable-length, so
           the value-encode descriptor recurses through the map value into the nested list. `app` returns
           `(map (0 (list x (g x))) (1 (list x)))`. `app(handle, 5)` → `(: (map (0 (list 5 6)) (1 (list 5)))
           (Map Int64 (List Int64)))` in canonical key order.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (map (0 (list x (g x))) (1 (list x))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (map (0 (list 5 6)) (1 (list 5))) (Map Int64 (List Int64)))))

(case "round-trip: a consumer returns an Option of a tuple"
  (doc    "A SUM whose payload is a fixed-shape COMPOUND: `app` returns `(Some (tuple x (g x)))`.
           `app(handle, 5)` → `(: (Some (tuple 5 6)) (Option (Tuple Int64 Int64)))`. The value-encode walker
           switches on the disc, then renders the tuple payload.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (Some (tuple x (g x))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (Some (tuple 5 6)) (Option (Tuple Int64 Int64)))))

(case "round-trip: a consumer returns a list of tuples from repeated closure application"
  (doc    "A `List (Tuple Int64 Int64)` result — a collection whose ELEMENT is a compound. `app` applies `g`
           to two inputs and pairs each. `mk` doubles; `app(handle, 3)` → `(list (tuple 3 6) (tuple 4 8))`, so
           `(: (list (tuple 3 6) (tuple 4 8)) (List (Tuple Int64 Int64)))`.")
  (input  (do (def (mk) (fn ((: n Int64)) (* n 2)))
              (def (app (: g (-> Int64 Int64)) (: x Int64))
                (list (tuple x (g x)) (tuple (+ x 1) (g (+ x 1)))))
              (export mk) (export app)))
  (call   app (: 3 Int64))
  (output (: (list (tuple 3 6) (tuple 4 8)) (List (Tuple Int64 Int64)))))

(case "round-trip: a HIGHER-ORDER closure arg composed with a SUM result"
  (doc    "The argument and result widenings compose: `app : (own<t>, Int64) -> (Option Int64)` applies a
           closure-typed arg (a guest-built inner closure) and wraps the result in `Some`. `mk`'s closure
           applies its function arg to 10; `app(handle, 5)` → `g((fn y -> y+5))` = 15, so `(: (Some 15)
           (Option Int64))`.")
  (input  (do (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
              (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (Some (g (fn (y) (+ y x)))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (Some 15) (Option Int64))))

(case "distinct-sig round-trip: a SUM-result consumer + a COLLECTION-result consumer — the sum one"
  (doc    "Two distinct signatures, two result MODES: `appa : (own<t0>, Int64) -> (Option Int64)` returns
           `(Some (g x))`; `appb : (own<t1>, Bool) -> (List Int64)` returns `(list (h y) (h y))`.
           `appa(handle, 5)` → `(: (Some 6) (Option Int64))`. A sum result and a collection result of DISTINCT
           signatures coexist, each value-encoded against its own descriptor.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 1 0) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (Some (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (list (h y) (h y)))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 5 Int64))
  (output (: (Some 6) (Option Int64))))

(case "distinct-sig round-trip: a SUM-result consumer + a COLLECTION-result consumer — the collection one"
  (doc    "The SAME two-resource-type program, driving the OTHER (collection-result) consumer of the other
           signature: `appb(handle, true)` → `h(true)` = 1 twice, so `(: (list 1 1) (List Int64))`. Confirms a
           sum-result group and a collection-result group render independently.")
  (input  (do (def (mka) (fn ((: n Int64)) (+ n 1)))
              (def (mkb) (fn ((: b Bool)) (: (if b 1 0) Int64)))
              (def (appa (: g (-> Int64 Int64)) (: x Int64)) (Some (g x)))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (list (h y) (h y)))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appb (: true Bool))
  (output (: (list 1 1) (List Int64))))

; FINAL COMPOSITION WITNESSES — the closure surface composes across all its axes at once. These exercise
; combinations not covered by the per-feature cases: a higher-order (closure-typed) argument on the
; DISTINCT-SIG round-trip path; a collection result built by REPEATED closure application; and the mixed
; shape (closures + a plain export) driving the plain side. All run end-to-end under wasmtime.

(case "distinct-sig round-trip: a higher-order closure-typed arg on one group + a scalar closure on another"
  (doc    "`mka : () -> (-> (-> Int64 Int64) Int64)` (applies its function arg to 1 and 2, sums) and `mkb : ()
           -> (-> Bool Int64)` are distinct sigs → two resource types. `appa` hands `g` a guest-built `(fn (y)
           (* y x))`. `appa(handle, 5)` → `g((fn y->y*5))` = 5*1 + 5*2 = 15. Composes the higher-order arg with
           distinct-signature grouping.")
  (input  (do (def (mka) (fn ((: f (-> Int64 Int64))) (+ (f 1) (f 2))))
              (def (mkb) (fn ((: b Bool)) (: (if b 9 8) Int64)))
              (def (appa (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (* y x))))
              (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
              (export mka) (export mkb) (export appa) (export appb)))
  (call   appa (: 5 Int64))
  (output (: 15 Int64)))

(case "round-trip: a consumer returns a Set built from REPEATED closure application"
  (doc    "`mk` multiplies by 10; `app : (own<t>, Int64) -> (Set Int64)` = `(Set.of (list (g x) (g x) x))` —
           the closure `g` is applied TWICE and its result plus `x` form a set (duplicates collapse).
           `app(handle, 3)` → `g(3)`=30 twice, so `{3, 30}` → `(: ((. Set of) (list 3 30)) (Set Int64))` in
           canonical order. Composes repeated in-guest application with a collection value-encode result.")
  (input  (do (def (mk) (fn ((: n Int64)) (* n 10)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (Set.of (list (g x) (g x) x)))
              (export mk) (export app)))
  (call   app (: 3 Int64))
  (output (: ((. Set of) (list 3 30)) (Set Int64))))

(case "mixed: two closure exports alongside a plain export — driving the plain export"
  (doc    "`inc`/`dbl` are two same-signature closure exports (crossing via `make-<name>` + a shared borrow
           `call`) and `two` is a PLAIN (non-closure) export, all in one component. Calling `two` = 2 drives
           the plain top-level func directly, coexisting with the closure-resource interface. Pins that a
           plain export rides alongside the (now borrow<t>) multi-export closure shape.")
  (input  (do (def (inc) (fn ((: x Int64)) (+ x 1)))
              (def (dbl) (fn ((: x Int64)) (* x 2)))
              (def (two) 2)
              (export inc) (export dbl) (export two)))
  (call   two)
  (output (: 2 Int64)))

; A HIGHER-ORDER capture crossing the boundary: a producer whose returned closure CAPTURES another closure
; (built in-guest). The captured inner closure is an ordinary funcref-table value on the heap; the outer
; closure's cell holds it, and the round-trip consumer dispatches the outer via `call_indirect`, which in
; turn dispatches the inner. Only the OUTER handle crosses the host boundary as a resource; the inner closure
; never leaves the guest. (Contrast the `own<t>` TRANSFORMER, still declined: there the HOST supplies the
; inner closure OVER the boundary, which needs a closure-resource passed INTO a call.)

(case "round-trip: a producer's returned closure captured an inner closure built in-guest"
  (doc    "`mk : () -> (-> Int64 Int64)` returns `(fn (x) (let ((f (fn (y) (+ y 1)))) (f (f x))))` — the
           returned closure CAPTURES the inner `f` (a closure) and applies it twice. `app` applies the
           handed-back closure: `app(handle, 5)` → the returned closure on 5 → f(f(5)) = 7. Pins a
           higher-order CAPTURE (a closure whose cell holds another closure) crossing the round-trip boundary,
           dispatched entirely in-guest.")
  (input  (do (def (mk) (fn ((: x Int64)) (let ((f (fn ((: y Int64)) (+ y 1)))) (f (f x)))))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (g x))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: 7 Int64)))

; A SUM whose payload is itself a VARIABLE-LENGTH collection — `Option (List Int64)` — as a round-trip
; consumer result. The value-encode descriptor nests: the sum's disc switch selects the `Some` variant, then
; renders its List payload (element type observable). The deepest result-form nesting witnessed.

(case "round-trip: a consumer returns an Option whose payload is a List"
  (doc    "`mk` adds 1; `app : (own<t>, Int64) -> (Option (List Int64))` returns `(Some (list x (g x)))` — a
           sum wrapping a variable-length collection. `app(handle, 5)` → `g(5)`=6, so `(: (Some (list 5 6))
           (Option (List Int64)))`, value-encoded through the nested descriptor (disc switch → List render).
           Pins a sum-of-collection result form.")
  (input  (do (def (mk) (fn ((: n Int64)) (+ n 1)))
              (def (app (: g (-> Int64 Int64)) (: x Int64)) (Some (list x (g x))))
              (export mk) (export app)))
  (call   app (: 5 Int64))
  (output (: (Some (list 5 6)) (Option (List Int64)))))

; The UNIT closure boundary: a closure ARGUMENT or RESULT of type `Unit` has no machine slot
; (`valtype_of(Unit) = None` — Unit occupies no wasm value, so a lifted lambda taking/returning it cannot be
; represented), so it declines at lambda-lift ("a closure's result type has no machine representation"),
; BEFORE the resource envelope. A `Unit`-returning closure is a pure side-effecting callback — only
; meaningful once a closure may perform an effect (which the scope fence CDZ0406 forbids crossing today), so
; there is nothing for it to DO across the boundary. Declines as a `todo` — a documented boundary, not a
; miscompile.

(case "a closure returning Unit is declined — Unit has no machine representation"
  (doc    "`(def (mk) (fn (x) unit))` — the closure returns `Unit`, which has no machine slot; the lifted
           lambda's result cannot be represented, so it declines at lift (`a closure's result type has no
           machine representation`). A pure Unit-returning closure only makes sense as an effect callback, and
           closures escaping effects are forbidden (CDZ0406) — so a `Unit` result has no boundary role today.
           Declines (a `todo`).")
  (input  (do (def (mk) (fn ((: x Int64)) unit)) (export mk)))
  (call   mk (: 5 Int64))
  (output (: unit Unit)))

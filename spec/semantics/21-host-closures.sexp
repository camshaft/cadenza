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
(diagnostic-quality)

(case
  "a closure exported to the host is called by the host"
  (doc
    "`(def (main) (fn (x) (+ x 1)))` returns a closure whose result type is `(-> Int64 Int64)`, so
           the whole program crosses as a component-model resource `closure-s64-s64` with a `call` method.
           The host calls `make()` to obtain the closure handle, then `call(handle, 5)`, which dispatches
           `(fn (x) (+ x 1))` through the guest's own `call_indirect` — returning 6. The closure logic
           never leaves Cadenza; the host only holds the opaque handle and invokes it. Pins that a Cadenza
           closure crosses to the host as a callable resource.")
  (input (do (def (main) (fn ((: x Int64)) (+ x 1))) (export main)))
  (call main (: 5 Int64))
  (drop)
  (output (: 6 Int64))
  (live-objects 0))

; The SAME exported closure invoked with a different argument — the host mints a fresh handle (`make`) and
; calls it, showing the resource + its `call` dispatch are reusable, and that the result tracks the input.
(case
  "a host-called closure applied to a different argument tracks the input"
  (doc
    "The same `(fn (x) (+ x 1))` closure export, called with 41 → 42. The `call` method takes
           `borrow<t>`, so the host KEEPS the handle across calls (a repeatable callback — the natural
           host-closure shape) and the resource dtor reclaims the cell when the host finally drops it; this
           case still `make`s + `call`s once. Pins that the closure's dispatch is reusable and its result
           follows the argument.")
  (input (do (def (main) (fn ((: x Int64)) (+ x 1))) (export main)))
  (call main (: 41 Int64))
  (drop)
  (output (: 42 Int64))
  (live-objects 0))

; The `call` method takes `borrow<t>`: the host holds the handle and may invoke it REPEATEDLY (the natural
; callback shape), versus a consume-per-call `own<t>` where a second call on the same handle would trap
; "unknown handle index". The gate drives ONE `(call …)` per case, so the REPEATABILITY is pinned by the
; `a_borrow_closure_handle_is_repeatable` unit test (one `make` handle, two `call`s: `adder(10)` then 5→15,
; 7→17 on the SAME handle); this case witnesses the borrow `call` runs end-to-end. A capturing closure makes
; it concrete: the captured `k` survives across calls because the cell is not consumed.
(case
  "a capturing closure crosses as a repeatable (borrow<t>) callback handle"
  (doc
    "`(def (adder (: k Int64)) (fn (x) (+ x k)))` → `adder : (Int64) -> own<closure-s64-s64>`. The host
           `make`s a handle capturing k, then `call`s it — `call` borrows the handle (does NOT consume it),
           so the same handle serves repeated calls (proven twice-over in the unit test). Here `adder(100)` →
           a handle → `call(handle, 5)` = 105. Pins the borrow<t> repeatable-callback `call` end-to-end.")
  (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
  (call adder (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: 105 Int64))
  (live-objects 0))

; The repeatable `borrow<t>` `call` extends to the VALUE-FORM result closures too (byte-rope / compound /
; collection — all cross `call` as `list<u8>`): the cell is kept across calls, the transient result handle is
; released each call, and the `t-dtor` reclaims the cell on drop. The gate drives one `(call …)`; the
; repeatability is pinned by `a_borrow_compound_result_closure_handle_is_repeatable` (one `pair(100)` handle,
; two `call(5)`s, the SAME `(tuple 5 105)` value form both times — the captured k survived).
(case
  "a capturing closure returning a COMPOUND is a repeatable (borrow<t>) callback handle"
  (doc
    "`(def (pair (: k Int64)) (fn (x) (tuple x (+ x k))))` → a closure whose result is a tuple, crossing
           `call` as the `list<u8>` value form. `call` borrows the handle (repeatable — the same handle serves
           many calls, proven in the unit test), and the returned tuple is value-form-encoded out.
           `pair(100)` → a handle → `call(handle, 5)` = `(: (tuple 5 105) (Tuple Int64 Int64))`. Pins the
           borrow<t> repeatable `call` on a value-form (compound) result end-to-end.")
  (input (do (def (pair (: k Int64)) (fn ((: x Int64)) #tuple(x (+ x k)))) (export pair)))
  (call pair (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: (tuple 5 105) (Tuple Int64 Int64)))
  (live-objects 0))

; A closure whose body MULTIPLIES rather than adds — a different lifted code selected through the same
; call_indirect boundary, proving the resource carries the RIGHT closure code (its funcref-table slot).
(case
  "a host-called closure with a different body dispatches the right code"
  (doc
    "`(fn (x) (* x 3))` exported and called with 4 → 12. The closure's own lifted code (a distinct
           funcref-table slot) is what `call` dispatches, so a different closure body yields a different
           result through the identical boundary. Pins that the closure resource carries its code, not a
           fixed operation.")
  (input (do (def (main) (fn ((: x Int64)) (* x 3))) (export main)))
  (call main (: 4 Int64))
  (drop)
  (output (: 12 Int64))
  (live-objects 0))

; C-HOST-2 — a PARAMETERIZED export returning a CAPTURING closure. `(def (adder (: k Int64)) (fn (x) (+ x
; k)))` returns a closure that captures `k`, so the whole export crosses as `adder : (Int64) ->
; own<closure-s64-s64>`. The host computes a DISTINCT closure per input: `make(k)` runs the export body
; (closing over `k` into the cell), then `call(handle, x)` reads `k` back from the cell inside the
; dispatch. The handle genuinely originates in-guest, computed from the host's input. The corpus `(call
; …)` args are SPLIT by `make`'s arity: the first (here `k`) goes to `make`, the rest (here `x`) to `call`.
(case
  "a parameterized export returning a capturing closure is made and called by the host"
  (doc
    "`(def (adder (: k Int64)) (fn (x) (+ x k)))` — the host calls `make(10)` (building a closure
           that captured k=10), then `call(handle, 5)` = 5 + 10 = 15. Pins that the closure handle is
           computed from the host's input (make forwards the export param) AND the captured environment
           rides in the cell, read back inside the closure's `call` dispatch. The first `(call …)` arg
           (10) is make's `k`, the second (5) is the closure's `x`.")
  (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
  (call adder (: 10 Int64) (: 5 Int64))
  (drop)
  (output (: 15 Int64))
  (live-objects 0))

; The same capturing closure with a different capture AND a different call argument — the result tracks
; both, confirming `make`'s input flows into the captured cell and `call`'s input into the dispatch.
(case
  "a capturing closure export tracks both the captured value and the call argument"
  (doc
    "`adder(100)` then `call(7)` = 7 + 100 = 107. A different `k` (100) captured, a different `x` (7)
           applied — the result follows both, so the captured value is genuinely per-`make` and the call
           argument per-`call`.")
  (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
  (call adder (: 100 Int64) (: 7 Int64))
  (output (: 107 Int64))
  (live-objects known-leak))

; DROP — `call` BORROWS the handle, so a plain make+call HOLDS the closure cell (the known leak of 1 above).
; A `(drop)` clause makes the host resource-drop the handle AFTER the call, firing the cell's `t-dtor` to
; reclaim it — so the same `adder` make+call now leaves NO live cell. This contrasts the leaks-1 case above:
; the only difference is the explicit drop.
(case
  "a dropped closure handle leaves no live objects after make + call"
  (doc
    "`adder(10)` allocates a cell holding k=10; `call(5)` = 15 (borrowing the handle); the `(drop)`
           clause then resource-drops the handle, whose t-dtor reclaims the cell. After make+call+drop
           live-objects is 0 — the release the borrowed handle needs, versus the leaks-1 no-drop case.")
  (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
  (call adder (: 10 Int64) (: 5 Int64))
  (drop)
  (output (: 15 Int64))
  (live-objects 0))

; TWO-CALL-ON-ONE-HANDLE — a `borrow<t>` closure `call` does NOT consume its handle, so it is REPEATABLE:
; the host makes the closure ONCE, then calls it TWICE on the SAME handle. A `(then <arg>…)` continuation
; after the `(call …)` supplies the second call's arguments; the first `(call …)` args split by `make`'s
; arity as usual (make's params, then the FIRST call's args). The two results render as a tuple
; `(tuple <r1> <r2>)`. An `own<t>` closure would trap "unknown handle index" on the second call, so a
; matching tuple pins that the borrowed handle stays live across calls (the production single-export `call`
; ABI is `borrow<t>`). The handle is made once → the same known borrow leak of 1 cell as a one-call case.
(case
  "a borrowed closure handle is called twice on the same handle (repeatable)"
  (doc
    "`adder(10)` makes a closure capturing k=10; `call(handle, 5)` = 15, then `(then (: 7))` calls the
           SAME handle again `call(handle, 7)` = 17. The results render as `(tuple 15 17)`: the borrowed
           handle served BOTH calls (an `own<t>` handle would be consumed after the first, trapping the
           second). Arg split: 10 → make's `k`, 5 → the first call's `x`, 7 → the second call's `x`.")
  (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
  (call adder (: 10 Int64) (: 5 Int64))
  (then (: 7 Int64))
  (output (: (tuple 15 17) (Tuple Int64 Int64)))
  (live-objects known-leak))

; C-HOST-3 — a MULTI-ARGUMENT closure. `(-> Int64 (-> Int64 Int64))` (curried sugar `(fn (a b) …)`)
; crosses as a resource whose `call` takes BOTH arguments: `call : (self, a: s64, b: s64) -> s64`. The
; guest's lifted body is `(env, a, b) -> result`, so `call` pushes both args before the `call_indirect`.
; The `call` method's arity generalizes past one argument (C-HOST-1/2 were single-arg).
(case
  "a two-argument closure exported to the host is called with both arguments"
  (doc
    "`(fn (a b) (+ a b))` crosses as a resource whose `call` takes two Int64 args. The host calls
           `make()` then `call(handle, 3, 4)` = 7 — both args pushed to the guest's `call_indirect`. Pins
           that a closure's `call` method carries more than one argument.")
  (input (do (def (main) (fn ((: a Int64) (: b Int64)) (+ a b))) (export main)))
  (call main (: 3 Int64) (: 4 Int64))
  (drop)
  (output (: 7 Int64))
  (live-objects 0))

(case
  "a three-argument closure exported to the host is called with all three"
  (doc
    "`(fn (a b c) (+ (+ a b) c))` → `call(handle, 2, 3, 4)` = 9. Pins that the `call` arity is not
           special-cased to two — any number of scalar args crosses.")
  (input (do (def (main) (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ a b) c))) (export main)))
  (call main (: 2 Int64) (: 3 Int64) (: 4 Int64))
  (drop)
  (output (: 9 Int64))
  (live-objects 0))

; A PARAMETERIZED export returning a MULTI-ARG CAPTURING closure — C-HOST-2 (capture + make-forwarding)
; composed with C-HOST-3 (multi-arg call). `make`'s param (k) and the closure's two args (a, b) are all
; supplied through the split `(call …)` list: the first (k) to `make`, the rest (a, b) to `call`.
(case
  "a parameterized export returning a multi-argument capturing closure"
  (doc
    "`(def (adder3 (: k Int64)) (fn (a b) (+ (+ a b) k)))` — `make(100)` builds a closure capturing
           k=100, then `call(handle, 2, 3)` = 2 + 3 + 100 = 105. Composes make-param forwarding, a
           captured env, and a two-argument `call`.")
  (input
    (do (def (adder3 (: k Int64)) (fn ((: a Int64) (: b Int64)) (+ (+ a b) k))) (export adder3)))
  (call adder3 (: 100 Int64) (: 2 Int64) (: 3 Int64))
  (drop)
  (output (: 105 Int64))
  (live-objects 0))

; A closure whose RESULT type is Bool — `(-> Int64 Bool)`. The `call` method returns a boolean; the host
; renders it. Pins that the closure's result valtype is not fixed to an integer.
(case
  "a closure returning a boolean is called by the host"
  (doc
    "`(fn (x) (= x 0))` is a `(-> Int64 Bool)` closure; `make()` then `call(handle, 0)` = true (0
           equals 0), `call(handle, 5)` = false. The `call` method's result crosses as a boolean.")
  (input (do (def (main) (fn ((: x Int64)) (= x 0))) (export main)))
  (call main (: 0 Int64))
  (drop)
  (output (: true Bool))
  (live-objects 0))

; A closure that PERFORMS AN EFFECT cannot escape to the host — the scope fence for this whole feature. A
; closure's effects are discharged by the `handle`/`(host …)` frame that is DYNAMICALLY OPEN where the
; closure is built; a host-held closure is invoked LATER, outside that frame, so the effect would have no
; home when the host calls it. Here `ask` IS delegated (`(host (ask) …)`), so the effect has a home at the
; export's TOP — but the closure the export RETURNS carries the `ask.ask` past that delegation, out to the
; host, where the delegation no longer applies. We reject this INTENTIONALLY (CDZ0406) rather than compile a
; closure whose effect silently loses its handler. (An effect fully HANDLED inside the closure — reduced to
; plain code with no residual host call — is unaffected; only an effect that would escape is rejected.)
(case
  "a closure that performs a delegated effect cannot cross the host boundary"
  (doc
    "`(def (main) (host (ask) (fn (x) (+ x (ask.ask)))))` returns a closure whose body performs the
           delegated effect `ask.ask`. The delegation `(host (ask) …)` gives the effect a home at the
           export's top, but the RETURNED closure carries `ask.ask` out to the host, to be run when the host
           later invokes `call` — outside the delegation's dynamic extent, where the effect has no home. A
           closure's handler context does not travel with it across the boundary, so this is rejected
           (CDZ0406): closures escaping effects are not supported. Pins the scope fence that a host-held
           closure must be effect-free.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (fn ((: x Int64)) (+ x (ask.ask)))))
      (export main)))
  (error CDZ0406))

(case
  "a closure NESTED IN A TUPLE that performs a delegated effect cannot cross the host boundary"
  (doc
    "The compound-nested face of the escaping-closure fence above: the performing closure is not the
           bare export value but is WRAPPED IN A TUPLE — `(host (ask) (tuple 1 (fn (x) (+ x (ask.ask)))))`.
           The tuple crosses the host boundary carrying the closure, whose body still performs the delegated
           `ask.ask` outside the delegation's dynamic extent → rejected CDZ0406, exactly as the bare closure
           is. Pins that the escaping-closure scan reaches a closure nested in a compound, not just a
           top-level one. Was a generic 'not in the host-import set' decline on wasm before es1 (#1792 hoisted
           the CDZ0406 scan to the emit dispatch); now wasm rejects CDZ0406 matching rust + rust-async.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) #tuple(1 (fn ((: x Int64)) (+ x (ask.ask))))))
      (export main)))
  (error CDZ0406))

(case
  "a LET-bound escaping-effect closure returned from a host block cannot cross the host boundary"
  (doc
    "The let-indirection face of the escaping-closure fence: the performing closure is not returned
           directly but LET-BOUND first — `(host (ask) (let ((f (fn (x) (+ x (ask.ask))))) f))` — then the
           bound `f` is the host block's value. The escaping-closure scan must see through the `let` to the
           returned closure whose body performs `ask.ask` outside the delegation's extent → rejected CDZ0406,
           same as the bare and tuple-nested faces. Pins that a `let`-binding indirection does not smuggle an
           escaping-effect closure past the fence. (breaker es5.) wasm+rust+rust-async all CDZ0406.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (let ((f (fn ((: x Int64)) (+ x (ask.ask))))) f)))
      (export main)))
  (error CDZ0406))

(case
  "a closure performing a HANDLED (non-delegated) effect does NOT trip the escape reject"
  (doc
    "The NEGATIVE control of the escaping-closure CDZ0406 fence (the reject cases above): here the
           closure is APPLIED IN-GUEST inside the enclosing `(handle Ctr …)`, so `Ctr.tick` has a home along
           the dynamic extent — it does NOT escape, so it must RUN, NOT reject. Guards the fence against
           OVER-firing: a closure that merely performs an effect is fine as long as it's discharged
           in-extent; only a closure that CROSSES the host boundary carrying an unhandled perform rejects.
           Seeded 5, main(10): the tick reads 5, 10+5 = 15. (breaker es2.) wasm+rust+rust-async all run.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle Ctr 5 ((tick (u) s (resume s (+ s 1)))) ((fn (x) (+ x (Ctr.tick))) k)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 15 Int64)))

(case
  "a MODULE-exported closure that performs a delegated effect cannot cross the host boundary"
  (doc
    "The COMPOSITION face where the escaping-closure CDZ0406 fence meets the module-member call path:
           the performing closure is produced by a MODULE export — `(module m (def (mk) (fn (x) (+ x
           (ask.ask)))) (export mk))` — and returned via `(. m mk)` from the host block. The escaping-closure
           scan must reach THROUGH the module-member projection to the returned closure whose body performs
           `ask.ask` → rejected CDZ0406, exactly as the bare/tuple-nested/let-bound faces. Pins that a module
           boundary does not smuggle an escaping-effect closure past the fence — the intersection of the
           module-performer resolution and the closure-escape reject. wasm + rust reject CDZ0406; rust-async
           todo pending its host-delegation path.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (module m
        (def (mk) (fn ((: x Int64)) (+ x (ask.ask))))

        (export mk))
      (def (main) (host (ask) (m.mk)))
      (export main)))
  (error CDZ0406))

; --- An exported closure's BODY is type-checked, like an ordinary def / an in-guest-applied lambda ------
; A `(def (a) (fn …))` exported as a host closure crosses the boundary and is NEVER applied in-guest, so
; its body is never β-reduced. An ill-typed body must still be a compile-time rejection — the same CDZ0203
; an ordinary def `(def (main (: x Int64)) (: x Bool))` or an applied `((fn …) 5)` gives — not a silently-
; emitted invalid component. (The closure-export lowering runs the body's type-error collection before emit;
; the closure's params are bound, so an annotation/unification fault in the body surfaces exactly as in an
; ordinary definition.)
(case
  "an exported closure with an annotation-mismatched body is rejected, not emitted invalid"
  (doc
    "`(fn ((: x Int64)) (: x Bool))` — the body annotates an Int64 value as Bool, a type error. An
           ordinary def / an in-guest applied `(fn …)` rejects it CDZ0203; exporting the SAME closure must
           too, rather than skip the body's type-check and emit an invalid component. The closure-export
           path runs the body's `type_errors` before emit.")
  (input (do (def (a) (fn ((: x Int64)) (: x Bool))) (export a)))
  (error CDZ0203))

(case
  "an exported closure with a narrow-arg wide-result mismatched body is rejected, not miscompiled"
  (doc
    "`(fn ((: x Int8)) (: (+ x 100) Int64))` — the `(+ x 100)` over an Int8 param is Int8, annotated
           Int64: an annotation mismatch (CDZ0203). Previously this ill-typed body ESCAPED the type-check
           and emitted an INVALID component (the `call` body left an i32 where the result declared i64:
           'type mismatch: expected i64, found i32'). Now the body is type-checked first, so it rejects
           CDZ0203 — the ill-typed program is caught, not miscompiled. (A WELL-TYPED narrow-arg/wide-result
           closure would use an explicit conversion, e.g. `(fn ((: x Int8)) (Int64.of x))`.)")
  (input (do (def (a) (fn ((: x Int8)) (: (+ x 100) Int64))) (export a)))
  (error CDZ0203))

(case
  "an exported closure whose body applies an arithmetic operator to a non-numeric operand is rejected"
  (doc
    "`(fn ((: x Int64)) (+ x true))` — inside the exported closure body, `+` types at
           `∀a. (Int a) → (Int a) → (Int a)`, so the `Bool` operand `true` fails to unify against `(Int a)`:
           a type mismatch CDZ0203. This is the UNIFICATION-fault face of the closure-export body type-check
           (the annotation-mismatch faces above are the equality-annotation face) — both must reject the
           ill-typed body before emit rather than skip the check and emit an invalid component.")
  (input (do (def (a) (fn ((: x Int64)) (+ x true))) (export a)))
  (error CDZ0203))

; RICHER CAPTURING closures — the C-HOST-2 make-forwarding + captured-cell machinery is arity- and
; body-shape-agnostic, so a closure that captures SEVERAL values, drives control flow off a captured
; Bool, binds a `let` in its body, or calls a top-level helper all cross the boundary and are invoked
; by the host with no additional compiler support. Each `make(captures…)` builds the cell (closing over
; the export's params), and `call(x)` dispatches the lifted body through the guest's `call_indirect`,
; reading the captured environment back from the cell. These witness the CAPTURE path end-to-end past
; the single-scalar-capture cases above.
(case
  "a closure capturing two values is made and called by the host"
  (doc
    "`(def (both (: a Int64) (: b Int64)) (fn (x) (+ (+ a b) x)))` — the closure captures BOTH `a`
           and `b`. The host `make(10, 20)` (closing over a=10, b=20 into the cell), then `call(5)` =
           10 + 20 + 5 = 35. Pins that a closure cell carries MORE THAN ONE captured value, each read
           back inside the `call` dispatch. The first two `(call …)` args are make's captures, the last
           is the closure's argument.")
  (input (do (def (both (: a Int64) (: b Int64)) (fn ((: x Int64)) (+ (+ a b) x))) (export both)))
  (call both (: 10 Int64) (: 20 Int64) (: 5 Int64))
  (drop)
  (output (: 35 Int64))
  (live-objects 0))

(case
  "a capturing closure whose body uses the capture after an inner computation"
  (doc
    "`(def (scale (: k Int64)) (fn (x) (* (+ x 1) k)))` — the captured `k` multiplies an inner
           `(+ x 1)`, so it is used AFTER a nested subexpression rather than as the first operand. The host
           calls `make(k=4)` then `call(x=3)` = (3 + 1) * 4 = 16. Pins that the captured value flows
           through a nested subexpression unchanged.")
  (input (do (def (scale (: k Int64)) (fn ((: x Int64)) (* (+ x 1) k))) (export scale)))
  (call scale (: 4 Int64) (: 3 Int64))
  (drop)
  (output (: 16 Int64))
  (live-objects 0))

(case
  "a capturing closure with a let binding in its body"
  (doc
    "`(def (f (: k Int64)) (fn (x) (let ((y (* x 2))) (+ y k))))` — the closure body binds a local
           `y` then adds the captured `k`. The host `make(100)` then `call(7)` = (7*2) + 100 = 114. Pins
           that a `let` inside an escaping closure body lowers correctly alongside the captured env.")
  (input (do (def (f (: k Int64)) (fn ((: x Int64)) (let ((y (* x 2))) (+ y k)))) (export f)))
  (call f (: 100 Int64) (: 7 Int64))
  (drop)
  (output (: 114 Int64))
  (live-objects 0))

(case
  "a closure driving control flow off a captured boolean"
  (doc
    "`(def (g (: flag Bool)) (fn (x) (if flag (+ x 1) (- x 1))))` — the closure captures a Bool and
           branches on it. The host `make(true)` then `call(10)` = 11 (the then-branch); a `make(false)`
           would yield 9. Pins that a captured Bool drives an `if` inside the `call` dispatch — the
           capture is not restricted to a numeric accumulator.")
  (input (do (def (g (: flag Bool)) (fn ((: x Int64)) (if flag (+ x 1) (- x 1)))) (export g)))
  (call g (: true Bool) (: 10 Int64))
  (drop)
  (output (: 11 Int64))
  (live-objects 0))

(case
  "a closure whose body calls a top-level helper function"
  (doc
    "`(def (dbl (: n Int64)) (* n 2))` `(def (h (: k Int64)) (fn (x) (+ (dbl x) k)))` — the escaping
           closure body CALLS the top-level `dbl`. The host `make(5)` (capturing k=5) then `call(3)` =
           (dbl 3) + 5 = 6 + 5 = 11. Pins that a closure crossing the boundary can call another in-program
           function (the helper is emitted as an ordinary reachable def, called directly from the lifted
           closure body).")
  (input
    (do
      (def (dbl (: n Int64)) (* n 2))
      (def (h (: k Int64)) (fn ((: x Int64)) (+ (dbl x) k)))
      (export h)))
  (call h (: 5 Int64) (: 3 Int64))
  (drop)
  (output (: 11 Int64))
  (live-objects 0))

(case
  "a closure capturing THREE scalars is made and called"
  (doc
    "`(def (mk (: a Int64) (: b Int64) (: c Int64)) (fn (x) (+ (+ (+ x a) b) c)))` — three captured
           values in the cell. `make(1, 2, 3)` then `call(10)` = 10 + 1 + 2 + 3 = 16. Extends the two-capture
           case to a wider environment (each capture read back inside the `call` dispatch).")
  (input
    (do
      (def (mk (: a Int64) (: b Int64) (: c Int64)) (fn ((: x Int64)) (+ (+ (+ x a) b) c)))
      (export mk)))
  (call mk (: 1 Int64) (: 2 Int64) (: 3 Int64) (: 10 Int64))
  (drop)
  (output (: 16 Int64))
  (live-objects 0))

(case
  "a closure capturing values of DIFFERENT types (Float64 + Int64)"
  (doc
    "`(def (mk (: base Float64) (: n Int64)) (fn (x) (+ x base)))` — the cell captures a Float64 AND an
           Int64 (the latter unused in the body, but still stored), and the closure returns a Float64.
           `make(1.5, 7)` then `call(2.5)` = 2.5 + 1.5 = 4.0. Pins a MIXED-type capture environment (a float
           and an int share one cell) with a float `call` result.")
  (input (do (def (mk (: base Float64) (: n Int64)) (fn ((: x Float64)) (+ x base))) (export mk)))
  (call mk (: 1.5 Float64) (: 7 Int64) (: 2.5 Float64))
  (drop)
  (output (: 4.0 Float64))
  (live-objects 0))

; The DIRECT-CALL host→guest boundary: when the HOST must supply a value to `make`/`call` OVER the boundary,
; only aliased-width scalars cross (the same restriction host-call `abi_val_type` has). A COMPOUND the host
; supplies — a `make` parameter of type `(List …)`/`(Tuple …)`/a sum — needs a host→guest DECODE into the
; guest value-heap (a `value-decode` runtime op that does not exist), so it declines. This is the mirror of
; the round-trip relaxation: an in-GUEST-built compound arg crosses freely (built guest-side), but a
; host-SUPPLIED compound does not. The compiler DECLINES (pinned `(declines)`) rather than emit a component
; that can't accept the argument — the "decline rather than miscompile" outcome, now a live guard.
(case
  "a producer capturing a host-supplied COMPOUND List parameter crosses via host→guest decode (should-work)"
  (doc
    "`(def (mk (: xs (List Int64))) (fn (i) ((. List len) xs)))` returns a closure capturing the List
           `xs`, where `xs` is a `make` PARAMETER the HOST supplies over the boundary. A `(List Int64)` param
           SHOULD decode as `list<s64>` into the guest heap (the same list-param decode built for list<scalar>
           members), so combined with the closure-factory export the whole thing works — mk(xs) returns a
           closure and the closure returns `(List.len xs)`. Declines today only because host→guest decode of a
           compound ENTRY param is not yet built (v-rust-backend ruling; the round-trip cases, where a compound
           closure arg is BUILT in-guest, already cross freely). Grades Todo; auto-passes when the decode lands.")
  (input (do (def (mk (: xs (List Int64))) (fn ((: i Int64)) (List.len xs))) (export mk)))
  (call mk (: #list(10 20 30) (List Int64)) (: 0 Int64))
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
(case
  "a build-time delegated effect whose result a returned closure captures does not escape"
  (doc
    "`(def (main) (host (ask) (let ((v (ask.ask))) (fn (x) (+ x v)))))` performs `ask.ask` in the
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
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (let ((v (ask.ask))) (fn ((: x Int64)) (+ x v)))))
      (export main)))
  (call main (: 3 Int64))
  (host-responses (respond ask.ask (: 10 Int64)))
  (output (: 13 Int64)))

(case
  "a closure capturing a build-time host effect preserves the order of two host calls"
  (doc
    "The build-time host-capture composes with MULTIPLE host calls in the make code, consumed in the
           order made: `(let ((a (ask.ask)) (b (ask.ask))) …)` binds `a` to the first response and `b` to
           the second. Both are captured as plain values into the returned closure `(fn (x) (+ (+ x a) b))`.
           With responses 10 then 20 and the call argument 3, the result is 3 + 10 + 20 = 33 — the host-call
           order is observable through the captured values (host-calls asserts the two calls).")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (let ((a (ask.ask)) (b (ask.ask))) (fn ((: x Int64)) (+ (+ x a) b)))))
      (export main)))
  (call main (: 3 Int64))
  (host-responses (respond ask.ask (: 10 Int64)) (respond ask.ask (: 20 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 33 Int64)))

(case
  "a closure captures the result of a build-time host op called with an argument"
  (doc
    "The build-time host op may take an ARGUMENT: `(calc.dbl 5)` crosses the boundary passing 5, the
           host returns its response, and the closure captures it as a plain value. With `calc.dbl`
           responding 10 (the host's answer for input 5) and the call argument 3, the result is 3 + 10 = 13.
           Exercises a scalar host-op parameter composing with the closure-capture path.")
  (input
    (do
      (effect calc (op dbl (-> Int64 Int64)))
      (def (main) (host (calc) (let ((v (calc.dbl 5))) (fn ((: x Int64)) (+ x v)))))
      (export main)))
  (call main (: 3 Int64))
  (host-responses (respond calc.dbl (: 10 Int64)))
  (host-calls (call calc.dbl))
  (output (: 13 Int64)))

(case
  "a Float64 closure captures a Float64 build-time host effect result"
  (doc
    "The build-time host-capture is not Int64-specific: a `Float64` host op result crosses the
           boundary as `f64`, is captured as a plain value, and the returned closure is a `Float64 ->
           Float64`. With `ask.ask` responding 2.5 and the call argument 1.5, the result is 1.5 + 2.5 = 4.0.
           Exercises the f64 boundary primitive on BOTH the host op and the closure arg/result composing
           with the closure-capture path (a scalar-result shape, just a non-Int scalar).")
  (input
    (do
      (effect ask (op ask (-> Unit Float64)))
      (def (main) (host (ask) (let ((v (ask.ask))) (fn ((: x Float64)) (+ x v)))))
      (export main)))
  (call main (: 1.5 Float64))
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
(case
  "one of several same-signature closure exports is made and called by the host"
  (doc
    "Two closure exports `(def (inc) (fn (x) (+ x 1)))` and `(def (triple) (fn (x) (* x 3)))` cross
           together as one resource with `make-inc`/`make-triple` + a shared `call`. Calling `inc` drives
           `make-inc()` then `call(5)` = 6. Pins that several closures coexist as one resource and the
           named `make` selects the right one.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (triple) (fn ((: x Int64)) (* x 3)))
      (export inc)
      (export triple)))
  (call inc (: 5 Int64))
  (drop)
  (output (: 6 Int64))
  (live-objects 0))

(case
  "a second same-signature closure export shares the one call method"
  (doc
    "The SAME two-export program, now calling `triple`: `make-triple()` then the SHARED `call(5)` =
           15. The single `call` dispatches `(* x 3)` here and `(+ x 1)` above — proving one `call` serves
           every same-signature export (the code slot travels in the resource rep, recovered per call).")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (triple) (fn ((: x Int64)) (* x 3)))
      (export inc)
      (export triple)))
  (call triple (: 5 Int64))
  (drop)
  (output (: 15 Int64))
  (live-objects 0))

; The multi-export SHARED `call` is a repeatable `borrow<t>` method too (C-HOST-6): one `make-<name>` handle
; serves repeated calls through the one shared `call` (the host keeps it; the `t-dtor` reclaims on drop). The
; gate drives one `(call …)`; the repeatability is pinned by `a_multi_export_shared_borrow_call_is_repeatable`
; (one `make-inc` handle, shared `call(5)`=6 then `call(40)`=41).
(case
  "a multi-export shared call is a repeatable (borrow<t>) callback"
  (doc
    "The SAME two-export program witnessed as a borrow<t> shared call: `make-inc()` → a handle the host
           keeps, then the shared `call(5)` = 6. `call` borrows the handle (does NOT consume it), so the same
           handle serves repeated calls through the one shared `call` (proven twice-over in the unit test).")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (triple) (fn ((: x Int64)) (* x 3)))
      (export inc)
      (export triple)))
  (call inc (: 5 Int64))
  (drop)
  (output (: 6 Int64))
  (live-objects 0))

(case
  "a multi-export set of parameterized capturing closures is driven per export"
  (doc
    "Three closure exports that each CAPTURE their param: `add` (+ x k), `mul` (* x k), `sub` (- x k),
           all `(Int64) -> (-> Int64 Int64)`. Calling `mul` drives `make-mul(4)` (capturing k=4) then
           `call(5)` = 20. Pins that make-forwarding (the captured param) composes with multi-export: each
           `make-<name>` forwards its own export's parameter into its own cell, and the shared `call` reads
           whichever capture the handle carries.")
  (input
    (do
      (def (add (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (mul (: k Int64)) (fn ((: x Int64)) (* x k)))
      (def (sub (: k Int64)) (fn ((: x Int64)) (- x k)))
      (export add)
      (export mul)
      (export sub)))
  (call mul (: 4 Int64) (: 5 Int64))
  (drop)
  (output (: 20 Int64))
  (live-objects 0))

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
(case
  "a produced closure is handed back into a consumer export (the round trip)"
  (doc
    "`(def (make-adder (: k Int64)) (fn (x) (+ x k)))` PRODUCES a closure capturing k; `(def (apply-it
           (: g (-> Int64 Int64)) (: x Int64)) (g x))` CONSUMES one. The host produces a handle from
           `make-adder(10)`, then threads it back into `apply-it(handle, 5)` = 5 + 10 = 15 — a closure
           crossing OUT of one export call and back IN to another, applied via the guest's own
           `call_indirect`. Pins host-as-custodian: the producer's `resource.new` handle is recovered by the
           consumer's `resource.rep` and dispatched.")
  (input
    (do
      (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (export make-adder)
      (export apply-it)))
  (call apply-it (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

; The same round trip with a different capture and argument — the handle genuinely carries the per-produce
; captured environment across the boundary and back, and the consumer's dispatch reads it.
(case
  "the round trip tracks the produced closure's captured value"
  (doc
    "`make-adder(100)` produces a closure capturing k=100; `apply-it(handle, 7)` = 7 + 100 = 107. A
           different capture (100) and consumer argument (7) — the result follows both, so the captured
           environment rides in the handle the host hands back, not in any shared state.")
  (input
    (do
      (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (export make-adder)
      (export apply-it)))
  (call apply-it (: 100 Int64) (: 7 Int64))
  (output (: 107 Int64)))

; A round-trip consumer's closure param need not be FIRST — it can sit at any source position, interleaved
; with scalars. The driver walks the consumer's params in source order, threading the produced handle into
; the CLOSURE slot(s) and the `(call …)` scalar args into the scalar slots. Here `app`'s closure param is
; SECOND (a scalar precedes it): `make-adder(1)` mints a closure `(+ y 1)`, then `app(5, handle)` = 6. The
; arg list is producer-args (make-adder's `k`) then the consumer's scalar (`x`).
(case
  "a round-trip consumer takes the produced closure in a NON-LEADING param position"
  (doc
    "`(def (app (: x Int64) (: g (-> Int64 Int64))) (g x))` takes its closure param SECOND. The host
           produces a handle from `make-adder(1)` (a closure `(+ y 1)`), then calls `app(5, handle)` — the
           handle threaded into the SECOND slot, the scalar 5 into the first — = 5 + 1 = 6. Pins that a
           consumer's closure param is placed by its source position, not hardcoded leading.")
  (input
    (do
      (def (make-adder (: k Int64)) (fn ((: y Int64)) (+ y k)))
      (def (app (: x Int64) (: g (-> Int64 Int64))) (g x))
      (export make-adder)
      (export app)))
  (call app (: 1 Int64) (: 5 Int64))
  (output (: 6 Int64)))

; A round-trip consumer may take SEVERAL closure params (all the same signature → the one resource type).
; Each gets its OWN fresh handle from the producer, from its own slice of the leading producer args. Here
; `app2(f, g, x)` takes TWO closures: `make-adder(1)` → `f = (+ y 1)`, `make-adder(2)` → `g = (+ y 2)`,
; then `app2(f, g, 5)` = (5+1) + (5+2) = 13. Arg list: producer-args for `f` (1), producer-args for `g`
; (2), then the consumer scalar (5).
(case
  "a round-trip consumer takes TWO produced closure params"
  (doc
    "`(def (app2 (: f (-> Int64 Int64)) (: g (-> Int64 Int64)) (: x Int64)) (+ (f x) (g x)))` takes two
           closures of the same signature. The host mints TWO distinct handles — `make-adder(1)` (`f`) and
           `make-adder(2)` (`g`) — and threads both into their slots: `app2(f, g, 5)` = (5+1) + (5+2) = 13.
           Pins that several same-signature closure params each get their own handle, placed by position.")
  (input
    (do
      (def (make-adder (: k Int64)) (fn ((: y Int64)) (+ y k)))
      (def (app2 (: f (-> Int64 Int64)) (: g (-> Int64 Int64)) (: x Int64)) (+ (f x) (g x)))
      (export make-adder)
      (export app2)))
  (call app2 (: 1 Int64) (: 2 Int64) (: 5 Int64))
  (output (: 13 Int64)))

; C-HOST-5 leak balance — a round trip leaves NO live cell. The producer mints the closure cell
; (`make-adder` → `resource.new`); the consumer takes it back as `own<t>`, applies it, and its wrapper
; RELEASES the cell (`heap.drop`) after the body returns. So after the round trip `live-objects` is 0 —
; distinct from a bare `make`+`call` case, which HOLDS the handle (borrow) and leaks 1. `twice-plus` applies
; the closure TWICE (`(+ (g x) (g x))`) — the drop fires once, AFTER the body, not per application.
(case
  "a round trip releases the produced closure cell (no live objects)"
  (doc
    "`make-adder(1)` mints a closure `(+ y 1)`; `twice-plus(handle, 5)` = (5+1)+(5+1) = 12, then the
           consumer wrapper drops the own<t> cell. After the round trip live-objects is 0 — the consumer
           owns the handed-back handle and reclaims the cell once, after the body.")
  (input
    (do
      (def (make-adder (: k Int64)) (fn ((: y Int64)) (+ y k)))
      (def (twice-plus (: g (-> Int64 Int64)) (: x Int64)) (+ (g x) (g x)))
      (export make-adder)
      (export twice-plus)))
  (call twice-plus (: 1 Int64) (: 5 Int64))
  (output (: 12 Int64))
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects known-leak))

; Release soundness when the consumer NEVER APPLIES the handed-back closure on the taken path: the own<t>
; was consumed at the boundary regardless, so the wrapper still drops the cell. `app`'s body is
; `(if (< x 0) 0 (g x))`; called with x < 0 it takes the guarded branch (returns 0) WITHOUT dispatching the
; closure — yet the handed-back cell is still reclaimed. live-objects 0 on the ignore path too.
(case
  "a round trip releases the closure cell even when the consumer ignores it"
  (doc
    "`app(handle, -3)` with `(if (< x 0) 0 (g x))` takes the guarded branch → 0, never applying the
           closure. The own<t> handle was consumed at the boundary, so its cell is dropped anyway —
           live-objects 0. Pins that release does not depend on the body dispatching the closure.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (if (< x 0) 0 (g x)))
      (export mk)
      (export app)))
  (call app (: -3 Int64))
  (output (: 0 Int64))
  (live-objects 0))

; A consumer that does MORE than apply the closure once — it applies it and adds a constant — showing the
; consumer body is ordinary Cadenza code with the handed-back closure as a first-class value in it.
(case
  "a consumer applies the handed-back closure inside a larger expression"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects known-leak)
  (doc
    "`(def (twice-plus (: g (-> Int64 Int64)) (: x Int64)) (+ (g x) (g x)))` applies the handed-back
           closure TWICE and sums. With `make-adder(1)` producing `(+ x 1)`, `twice-plus(handle, 5)` =
           (5+1) + (5+1) = 12. Pins that the consumer body is ordinary code — the closure param is a
           first-class value it may apply more than once (the `own<t>` handle serves the whole consumer
           call; it is consumed once, at the boundary, not per in-body application).")
  (input
    (do
      (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (twice-plus (: g (-> Int64 Int64)) (: x Int64)) (+ (g x) (g x)))
      (export make-adder)
      (export twice-plus)))
  (call twice-plus (: 1 Int64) (: 5 Int64))
  (output (: 12 Int64)))

; WIDER SCALAR WIDTHS — a closure's `call` boundary crosses EVERY aliased-width scalar the ordinary export
; boundary supports, not just the u32/s64/bool/f64 the value-heap runtime ops model. The closure functype
; is a plain component functype (component primitive byte via `comp_valtype_of` + core valtype via
; `valtype_of`), independent of the runtime-op ABI table — so `(-> Int32 Int32)`, `(-> UInt64 UInt64)`,
; `(-> Int8 Int8)`, a `Float32` closure, and a mixed-width `(-> Int32 Bool)` all cross and dispatch.
(case
  "a 32-bit-integer closure crosses the host boundary"
  (doc
    "`(fn (x) (+ x 1))` at `(-> Int32 Int32)` — the closure's arg and result cross as the component
           `s32` primitive (core i32), narrower than the s64 the value-heap ops use. `call(5)` = 6. Pins
           that a 32-bit closure signature crosses the `call` boundary (the boundary byte comes from
           `comp_valtype_of`, wider than the runtime-op ABI table).")
  (input (do (def (main) (fn ((: x Int32)) (+ x 1))) (export main)))
  (call main (: 5 Int32))
  (drop)
  (output (: 6 Int32))
  (live-objects 0))

(case
  "a 64-bit-unsigned closure crosses the host boundary"
  (doc
    "`(fn (x) (* x 2))` at `(-> UInt64 UInt64)` — crosses as the component `u64` primitive. `call(21)`
           = 42. Pins the UNSIGNED 64-bit width (distinct from the signed s64 the runtime ops model).")
  (input (do (def (main) (fn ((: x UInt64)) (* x 2))) (export main)))
  (call main (: 21 UInt64))
  (drop)
  (output (: 42 UInt64))
  (live-objects 0))

(case
  "an 8-bit-integer closure crosses the host boundary"
  (doc
    "`(fn (x) (- x 1))` at `(-> Int8 Int8)` — the narrowest aliased width, crossing as component `s8`
           (core i32). `call(10)` = 9. Pins that a narrow width crosses (the runtime-op ABI table has no s8,
           but the closure functype does not need it).")
  (input (do (def (main) (fn ((: x Int8)) (- x 1))) (export main)))
  (call main (: 10 Int8))
  (drop)
  (output (: 9 Int8))
  (live-objects 0))

(case
  "a 32-bit-float closure crosses the host boundary"
  (doc
    "`(fn (x) (+ x 1.5))` at `(-> Float32 Float32)` — crosses as component `f32` (core f32), narrower
           than the f64 the runtime ops use. `call(2.5)` = 4.0. Pins the 32-bit float width.")
  (input (do (def (main) (fn ((: x Float32)) (+ x 1.5))) (export main)))
  (call main (: 2.5 Float32))
  (drop)
  (output (: 4.0 Float32))
  (live-objects 0))

(case
  "a capturing 32-bit-integer closure crosses and is called"
  (doc
    "`(def (adder (: k Int32)) (fn (x) (+ x k)))` — a capturing closure at the narrower Int32 width.
           `make(100)` then `call(7)` = 107. Pins that make-forwarding + the captured cell compose with a
           widened scalar width, exactly as at Int64.")
  (input (do (def (adder (: k Int32)) (fn ((: x Int32)) (+ x k))) (export adder)))
  (call adder (: 100 Int32) (: 7 Int32))
  (drop)
  (output (: 107 Int32))
  (live-objects 0))

(case
  "a UInt64 closure round-trips through a consumer export"
  (doc
    "The round trip at a widened width: `(def (make-adder (: k UInt64)) (fn (x) (+ x k)))` produces a
           `(-> UInt64 UInt64)` closure; `(def (apply-it (: g (-> UInt64 UInt64)) (: x UInt64)) (g x))`
           consumes one. `make-adder(100)` → a handle → `apply-it(handle, 7)` = 107. Pins that the
           producer/consumer boundary (own<t> + resource.rep dispatch) crosses a non-Int64 scalar width.")
  (input
    (do
      (def (make-adder (: k UInt64)) (fn ((: x UInt64)) (+ x k)))
      (def (apply-it (: g (-> UInt64 UInt64)) (: x UInt64)) (g x))
      (export make-adder)
      (export apply-it)))
  (call apply-it (: 100 UInt64) (: 7 UInt64))
  (output (: 107 UInt64)))

; A CONSUMER whose closure parameter is NOT FIRST — the consumer's component functype follows SOURCE order,
; so a scalar-then-closure `(def (app (: x Int64) (: g (-> Int64 Int64))) (g x))` crosses as `app : (s64,
; own<t>) -> s64`, not a closure-first shape. (An earlier cut hardcoded the closure as the first param and
; emitted an INVALID component when it wasn't; the functype now mirrors the params in order.) The driver
; threads the produced handle into the closure position and the scalar into its position.
(case
  "a consumer takes the handed-back closure as its SECOND parameter"
  (doc
    "`(def (mk) (fn (x) (+ x 1)))` produces the closure; `(def (app (: x Int64) (: g (-> Int64
           Int64))) (g x))` takes a scalar `x` FIRST, then the closure `g`. `mk()` → a handle, then
           `app(5, handle)` = `(g 5)` = 6. Pins that the consumer's boundary functype follows source
           param order (closure not required to be first).")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 1)))
      (def (app (: x Int64) (: g (-> Int64 Int64))) (g x))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: 6 Int64)))

; A consumer taking MORE THAN ONE closure parameter — both of the same signature, so both cross as
; `own<t>` of the ONE resource type. The host produces a fresh handle per closure param (own<t> is consumed
; per call) and threads each into its position. `(def (app2 (: f …) (: g …) (: x Int64)) (+ (f x) (g x)))`.
(case
  "a consumer applies TWO handed-back closures"
  (doc
    "`(def (app2 (: f (-> Int64 Int64)) (: g (-> Int64 Int64)) (: x Int64)) (+ (f x) (g x)))` takes
           TWO closures + a scalar. With `mk` producing `(+ x 1)`, the host produces two handles and calls
           `app2(h1, h2, 5)` = (5+1) + (5+1) = 12. Pins that several closure params of the same signature
           cross as own<t> of the one resource type, each threaded independently.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 1)))
      (def (app2 (: f (-> Int64 Int64)) (: g (-> Int64 Int64)) (: x Int64)) (+ (f x) (g x)))
      (export mk)
      (export app2)))
  (call app2 (: 5 Int64))
  (output (: 12 Int64)))

; A consumer whose RESULT type differs from the closure's — the consumer functype's result is the
; CONSUMER's own result (`Bool` here), not the applied closure's (`Int64`). `(def (is-pos (: g …) (: x
; Int64)) (> (g x) 0))` returns Bool.
(case
  "a consumer returns a different type than the closure it applies"
  (doc
    "`(def (is-pos (: g (-> Int64 Int64)) (: x Int64)) (> (g x) 0))` applies an `(-> Int64 Int64)`
           closure but RETURNS `Bool`. With `mk` producing `(+ x 1)`, `is-pos(handle, 5)` = (6 > 0) = true.
           Pins that the consumer's boundary result byte is the CONSUMER's result type, not the closure's.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 1)))
      (def (is-pos (: g (-> Int64 Int64)) (: x Int64)) (> (g x) 0))
      (export mk)
      (export is-pos)))
  (call is-pos (: 5 Int64))
  (output (: true Bool)))

; The consumer cases above apply the handed-back closure ONCE (or twice as sibling operands). These
; stress the DISPATCH under repetition and composition: the same handle applied once per iteration of
; a RECURSIVE loop (the funcref/code-slot read must be stable across activations, not a one-shot), and
; a consumed closure's RESULT feeding a trap-guarded division (the trap must surface through the
; consumer's boundary as a trap, not a wrong value — the consumer twin of the closure-body trap pin).
(case
  "a consumed closure is applied once per iteration of a recursive loop"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects known-leak)
  (doc
    "`apply-n` hands the closure to a RECURSIVE worker `iter` that applies it once per iteration:
           `make-adder(10)` then `apply-n(handle, 3)` folds g over 0 three times — 0→10→20→30. The handle
           crosses the boundary ONCE but dispatches N times from loop-carried state; a code-slot read
           that only survived the first activation (or a handle consumed by the first apply) breaks the
           later iterations. Expected: 30.")
  (input
    (do
      (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def
        (iter (: g (-> Int64 Int64)) (: n Int64) (: acc Int64))
        (if (< n 1) acc (iter g (- n 1) (g acc))))
      (def (apply-n (: g (-> Int64 Int64)) (: n Int64)) (iter g n 0))
      (export make-adder)
      (export apply-n)))
  (call apply-n (: 10 Int64) (: 3 Int64))
  (output (: 30 Int64)))

(case
  "a trap raised on a consumed closure's result surfaces through the consumer as a trap"
  (doc
    "`divide-by` applies the handed-back closure and divides by its RESULT: `make-sub(5)` gives
           `(- x 5)`, so `divide-by(handle, 5)` computes `(/ 100 0)` — a genuine divide-by-zero reached
           only through the consumed closure's value. The trap must surface to the host as a trap through
           the consumer export's boundary (not a swallowed error or wrong value) — the consumer-side twin
           of the closure-BODY trap pin above. Expected: trap (integer divide by zero).")
  (input
    (do
      (def (make-sub (: k Int64)) (fn ((: x Int64)) (- x k)))
      (def (divide-by (: g (-> Int64 Int64)) (: x Int64)) (/ 100 (g x)))
      (export make-sub)
      (export divide-by)))
  (call divide-by (: 5 Int64) (: 5 Int64))
  (trap "integer divide by zero"))

; DISTINCT-SIGNATURE multi-export — a program exporting closures of DIFFERENT signatures crosses as one
; resource type PER signature. `inc : (-> Int64 Int64)` and `isz : (-> Int64 Bool)` become resources `t0`
; and `t1`, each with its own `make-<name>` + `call-g<n>` (the group's shared call). The host picks a
; closure export by name; the driver calls `make-<name>` → a handle, then the `call-g<n>` whose `self`
; resource type matches. Each group gets its own `resource.new`/`resource.rep` intrinsics (a core
; `resource.new` is typed to ONE resource); both closures still share the guest funcref table.
(case
  "one of two DIFFERENT-signature closure exports is made and called"
  (doc
    "`(def (inc) (fn (x) (+ x 1)))` is `(-> Int64 Int64)` and `(def (isz) (fn (x) (= x 0)))` is
           `(-> Int64 Bool)` — DIFFERENT signatures, so they cross as two resource types. Calling `inc`
           drives `make-inc()` (resource t0) then its `call`(5) = 6. Pins that distinct signatures each get
           their own resource type + make/call, published in one interface.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (export inc)
      (export isz)))
  (call inc (: 5 Int64))
  (output (: 6 Int64))
  (live-objects 1))

(case
  "the second distinct-signature closure export returns its own type"
  (doc
    "The SAME two-export program, now calling `isz` (resource t1, a `(-> Int64 Bool)` closure):
           `make-isz()` then its `call`(0) = true. The `isz` group's `call` returns Bool, distinct from
           `inc`'s Int64 — proving the two resource types carry independent signatures and results.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (export inc)
      (export isz)))
  (call isz (: 0 Int64))
  (output (: true Bool))
  (live-objects 1))

; Each distinct-signature group's per-group `call-g<n>` is a repeatable `borrow<t_g>` method too (C-HOST-6,
; the last borrow widening): a `make-<name>` handle serves repeated `call-g<n>`s (the host keeps it; the
; `t-dtor` reclaims). The gate drives one `(call …)`; repeatability is pinned by
; `a_distinct_sig_call_g_is_repeatable` (one `make-inc` handle → `call-g(5)`=6 then `call-g(40)`=41). This
; closes the borrow surface: EVERY closure `call` in every shape is now a repeatable borrow<t> handle.
(case
  "a distinct-signature per-group call-g is a repeatable (borrow<t>) callback"
  (doc
    "The SAME two-resource-type program witnessed as a borrow<t> per-group call: `make-inc()` → a handle
           the host keeps (resource t0), its `call-g<n>(5)` = 6. `call-g<n>` borrows the handle (does NOT
           consume it), so the same handle serves repeated calls (proven twice-over in the unit test); the
           distinct `isz` group's `call-g<n>` is independently repeatable.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (export inc)
      (export isz)))
  (call inc (: 5 Int64))
  (output (: 6 Int64))
  (live-objects 1))

(case
  "three closures with a SHARED signature cross as two resource types"
  (doc
    "`inc : (-> Int64 Int64)`, `isz : (-> Int64 Bool)`, `dbl : (-> Int64 Int64)` — note `inc` and
           `dbl` SHARE a signature (one resource type, two makes), while `isz` is distinct (its own).
           Calling `dbl`(7) = 14 exercises the shared-signature group alongside the distinct one. Pins that
           grouping-by-signature composes: same-signature exports share a resource, distinct ones don't.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (dbl) (fn ((: x Int64)) (* x 2)))
      (export inc)
      (export isz)
      (export dbl)))
  (call dbl (: 7 Int64))
  (output (: 14 Int64))
  (live-objects 1))

; DISTINCT-SIGNATURE composed with MULTI-ARG and CAPTURE — the grouping-by-signature path (each signature
; its own resource type) composes with the arity/capture machinery, no new compiler work. `add : (-> Int64
; (-> Int64 Int64))` (two args) and `isz : (-> Int64 Bool)` are distinct signatures → two resource types;
; `add`'s `call` takes both args. And two CAPTURING producers of distinct signatures (`adder`/`eq`) each
; forward their captured param through their own resource.
(case
  "a multi-argument closure among distinct-signature exports"
  (doc
    "`(def (add) (fn (a b) (+ a b)))` is `(-> Int64 (-> Int64 Int64))` (two-arg) and `(def (isz) (fn
           (x) (= x 0)))` is `(-> Int64 Bool)` — distinct signatures, two resource types. Calling `add`
           drives `make-add()` then its `call(3, 4)` = 7 — the two-arg `call` on its own resource, alongside
           the distinct `isz`. Pins that multi-arg composes with distinct-signature grouping.")
  (input
    (do
      (def (add) (fn ((: a Int64) (: b Int64)) (+ a b)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (export add)
      (export isz)))
  (call add (: 3 Int64) (: 4 Int64))
  (output (: 7 Int64))
  (live-objects 1))

(case
  "distinct-signature capturing producers"
  (doc
    "`(def (adder (: k Int64)) (fn (x) (+ x k)))` → `(-> Int64 Int64)` and `(def (eq (: k Int64)) (fn
           (x) (= x k)))` → `(-> Int64 Bool)` — distinct signatures, both CAPTURING their `k`. Calling `eq`
           drives `make-eq(5)` (capturing k=5) then its `call(5)` = true. Pins that make-param capture rides
           through the per-signature resource, distinct from `adder`'s.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (eq (: k Int64)) (fn ((: x Int64)) (= x k)))
      (export adder)
      (export eq)))
  (call eq (: 5 Int64) (: 5 Int64))
  (output (: true Bool))
  (live-objects 1))

; ROUND-TRIP composed with MULTI-ARG and a WIDENED width — the producer/consumer path is arity- and
; width-agnostic (the consumer's `call_indirect` dispatches the guest lifted body over the ONE table).
(case
  "a multi-argument closure round-trips through a consumer"
  (doc
    "`(def (mk) (fn (a b) (+ a b)))` produces a two-arg `(-> Int64 (-> Int64 Int64))` closure; `(def
           (app (: g (-> Int64 (-> Int64 Int64))) (: a Int64) (: b Int64)) (g a b))` applies it with BOTH
           args. The host `mk()` → a handle → `app(handle, 3, 4)` = 7. Pins that the round trip threads a
           MULTI-ARG closure back (the consumer's dispatch pushes both args).")
  (input
    (do
      (def (mk) (fn ((: a Int64) (: b Int64)) (+ a b)))
      (def (app (: g (-> Int64 (-> Int64 Int64))) (: a Int64) (: b Int64)) (g a b))
      (export mk)
      (export app)))
  (call app (: 3 Int64) (: 4 Int64))
  (output (: 7 Int64)))

; A multi-argument arrow may be written FLAT `(-> A B … R)` (the idiomatic spelling) as well as explicitly
; CURRIED `(-> A (-> B R))` — both denote the same n-ary function type `A -> (B -> (… -> R))`. The flat form
; `(-> Int64 Int64 Int64)` used to error "-> takes one or two type arguments" (only arities 1 + 2 were
; handled), so a round-trip consumer whose closure parameter was written flat solved `Any` and declined
; "parameter type is ambiguous — annotate it". The arrow constructor now curries any arity ≥1.
(case
  "a multi-argument closure round-trips through a consumer — FLAT arrow spelling"
  (doc
    "The SAME two-arg round trip as above, but the consumer's closure parameter is written with the
           FLAT arrow `(: g (-> Int64 Int64 Int64))` instead of the explicitly-curried `(-> Int64 (-> Int64
           Int64))`. Both denote `Int64 -> (Int64 -> Int64)`. `app(handle, 3, 4)` = 7. Pins that a flat
           multi-arg arrow annotation curries — previously it errored `-> takes one or two type arguments`
           and the param declined `parameter type is ambiguous`.")
  (input
    (do
      (def (mk) (fn ((: a Int64) (: b Int64)) (+ a b)))
      (def (app (: g (-> Int64 Int64 Int64)) (: a Int64) (: b Int64)) (g a b))
      (export mk)
      (export app)))
  (call app (: 3 Int64) (: 4 Int64))
  (output (: 7 Int64)))

(case
  "a THREE-argument closure round-trips — flat arrow spelling"
  (doc
    "A flat three-argument arrow `(-> Int64 Int64 Int64 Int64)` curries to `Int64 -> Int64 -> Int64 ->
           Int64`. `mk` sums three args; `app` applies the handed-back `g` to `x`, `x+1`, `x+2`. `app(handle,
           10)` → `g(10, 11, 12)` = 33.")
  (input
    (do
      (def (mk) (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ a b) c)))
      (def (app (: g (-> Int64 Int64 Int64 Int64)) (: x Int64)) (g x (+ x 1) (+ x 2)))
      (export mk)
      (export app)))
  (call app (: 10 Int64))
  (output (: 33 Int64)))

(case
  "a multi-argument closure with COMPOUND args round-trips — flat arrow spelling"
  (doc
    "Composes the flat multi-arg arrow with compound closure arguments (both built in-guest): `g : (->
           (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)` reads `p.0 + q.1`; `app` applies it to `(tuple x
           x)` and `(tuple x (x*2))`. `app(handle, 5)` → `g((tuple 5 5), (tuple 5 10))` = 5 + 10 = 15.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def
        (app (: g (-> (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)) (: x Int64))
        (g #tuple(x x) #tuple(x (* x 2))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: 15 Int64))
  (live-objects known-leak))

(case
  "a multi-argument closure returning a COMPOUND round-trips — flat arrow spelling"
  (doc
    "A flat two-arg arrow with a compound RESULT: `g : (-> Int64 Int64 (Tuple Int64 Int64))` pairs its
           two args; `app` applies it to `x` and `x+10` and returns the tuple. `app(handle, 5)` → `g(5, 15)` =
           `(: (tuple 5 15) (Tuple Int64 Int64))`, value-form-encoded out.")
  (input
    (do
      (def (mk) (fn ((: a Int64) (: b Int64)) #tuple(a b)))
      (def (app (: g (-> Int64 Int64 (Tuple Int64 Int64))) (: x Int64)) (g x (+ x 10)))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: (tuple 5 15) (Tuple Int64 Int64))))

(case
  "a round-trip at a widened scalar width (UInt32)"
  (doc
    "The round trip at UInt32, not Int64: `(def (mk (: k UInt32)) (fn (x) (+ x k)))` produces a `(->
           UInt32 UInt32)` closure; `(def (app (: g (-> UInt32 UInt32)) (: x UInt32)) (g x))` applies it.
           `mk(100)` → a handle → `app(handle, 7)` = 107. Pins that the producer/consumer boundary crosses
           a widened scalar width (own<t> + resource.rep dispatch is width-agnostic).")
  (input
    (do
      (def (mk (: k UInt32)) (fn ((: x UInt32)) (+ x k)))
      (def (app (: g (-> UInt32 UInt32)) (: x UInt32)) (g x))
      (export mk)
      (export app)))
  (call app (: 100 UInt32) (: 7 UInt32))
  (output (: 107 UInt32)))

; STRESS the multi-export paths at higher fan-out — THREE distinct signatures (three resource types, one
; with a narrower width) and FOUR same-signature exports (one resource, four makes sharing the call) — plus
; a consumer whose ONLY use of the handed-back closure is to apply it to an INTERNAL constant. Adversarial
; witnesses that the grouping/sharing machinery holds past the two-export cases above.
(case
  "three distinct closure signatures cross as three resource types"
  (doc
    "`p : (-> Int64 Int64)`, `q : (-> Int64 Bool)`, `r : (-> Int32 Int32)` — THREE distinct
           signatures (note `r`'s narrower Int32 width) → three resource types. Calling `r` drives its
           `make`+`call(5)` = 10. Pins that grouping-by-signature scales past two groups and mixes widths.")
  (input
    (do
      (def (p) (fn ((: x Int64)) (+ x 1)))
      (def (q) (fn ((: x Int64)) (= x 0)))
      (def (r) (fn ((: x Int32)) (* x 2)))
      (export p)
      (export q)
      (export r)))
  (call r (: 5 Int32))
  (output (: 10 Int32))
  (live-objects 1))

(case
  "four same-signature closure exports share one resource"
  (doc
    "`a`,`b`,`cc`,`dd` are all `(-> Int64 Int64)` → ONE resource type with four `make-<name>`s sharing
           the one `call`. Calling `cc` drives `make-cc()` then the shared `call(10)` = 13. Pins that the
           shared-call multi-export scales past two same-signature exports.")
  (input
    (do
      (def (a) (fn ((: x Int64)) (+ x 1)))
      (def (b) (fn ((: x Int64)) (+ x 2)))
      (def (cc) (fn ((: x Int64)) (+ x 3)))
      (def (dd) (fn ((: x Int64)) (+ x 4)))
      (export a)
      (export b)
      (export cc)
      (export dd)))
  (call cc (: 10 Int64))
  (drop)
  (output (: 13 Int64))
  (live-objects 0))

(case
  "a consumer applies the handed-back closure to an internal constant"
  (doc
    "`(def (app (: g (-> Int64 Int64))) (g 99))` — the consumer takes ONLY a closure param and applies
           it to a fixed 99 (no scalar param of its own). With `mk(1)` producing `(+ x 1)`, the host `mk(1)`
           → a handle → `app(handle)` = (g 99) = 99 + 1 = 100. Pins a consumer whose sole boundary param is
           the closure (the arg it applies is internal, not a boundary scalar).")
  (input
    (do
      (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (app (: g (-> Int64 Int64))) (g 99))
      (export mk)
      (export app)))
  (call app (: 1 Int64))
  (output (: 100 Int64)))

; CLOSURE BODY RICHNESS — the boundary machinery is agnostic to what the closure's body DOES; these witness
; body constructs (a `match`, a multi-binding `let`, several captures + args at once) crossing and
; dispatching correctly, a dimension distinct from the arity/capture/multi-export shapes above.
(case
  "an escaping closure captures two values and takes three arguments"
  (doc
    "`(def (main (: k Int64)) (fn (a b c) (+ (+ (+ a b) c) k)))` — the export param `k` is captured
           while the closure takes THREE args. `make(100)` (capturing k=100) then `call(1, 2, 3)` = 1 + 2 +
           3 + 100 = 106. Pins capture composing with a 3-arg call.")
  (input
    (do
      (def (main (: k Int64)) (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ (+ a b) c) k)))
      (export main)))
  (call main (: 100 Int64) (: 1 Int64) (: 2 Int64) (: 3 Int64))
  (drop)
  (output (: 106 Int64))
  (live-objects 0))

(case
  "an escaping closure whose body is a match hits the literal arm"
  (doc
    "`(fn (x) (match x (0 100) (_ x)))` — the closure body is a `match`. `call(0)` takes the literal
           arm → 100. Pins that a control-flow body (`match`) lowers and dispatches through the closure
           boundary.")
  (input (do (def (main) (fn ((: x Int64)) (match x (0 100) (_ x)))) (export main)))
  (call main (: 0 Int64))
  (drop)
  (output (: 100 Int64))
  (live-objects 0))

(case
  "an escaping closure whose body is a match hits the wildcard arm"
  (doc
    "The same match-bodied closure, `call(5)` → the wildcard arm → 5. Pins both arms of the closure's
           `match` dispatch across the boundary.")
  (input (do (def (main) (fn ((: x Int64)) (match x (0 100) (_ x)))) (export main)))
  (call main (: 5 Int64))
  (drop)
  (output (: 5 Int64))
  (live-objects 0))

(case
  "an escaping closure whose body binds a multi-variable let"
  (doc
    "`(def (main (: k Int64)) (fn (x) (let ((a (* x 2)) (b (+ x k))) (+ a b))))` — the body binds two
           locals (one using the captured `k`) then sums. `make(10)` then `call(5)` = (5*2) + (5+10) = 10 +
           15 = 25. Pins a multi-binding `let` body composing with capture.")
  (input
    (do
      (def (main (: k Int64)) (fn ((: x Int64)) (let ((a (* x 2)) (b (+ x k))) (+ a b))))
      (export main)))
  (call main (: 10 Int64) (: 5 Int64))
  (drop)
  (output (: 25 Int64))
  (live-objects 0))

; lcap1/lcap2 (breaker): the capture-SOURCE axis for an ESCAPING closure. Every escaping-closure case above
; captures a function DEF-PARAM (k/a/b/base/…). lcap1 (positive control, IN-PROGRAM apply) shows a closure
; capturing a LET-LOCAL scalar `v = (+ k 1)` computes fine: `((mk n) 10)` = (n+1)+10, tri-target. lcap2 is
; the GAP: the SAME closure ESCAPED (returned to the host, then applied) declines CDZ0900 on wasm (and the
; rust value-heap escaping-closure gap on rust/cadenza). So the escape lowering's capture-environment reaches
; function params but NOT let-locals (a match-arm binding behaves identically — same root). This SHOULD
; compile: ch21's own note (~line 275, "the escaping-closure scan must see through the `let`") says the scan
; is meant to see through a let; a def-param capture escapes fine (control above), and the in-program apply
; (lcap1) proves the value is correct — only the escape path drops the let-local from the capture set.
; Idealistic todo: lcap2 SHOULD escape + apply to 16. Routed to the closure-capture owner (concierge).
(case
  "lcap1 a closure capturing a LET-LOCAL scalar computes when applied in-program"
  (input
    (do
      (def (mk (: k Int64)) (let ((v (+ k 1))) (fn ((: y Int64)) (+ v y))))
      (def (main (: n Int64)) ((mk n) 10))
      (export main)))
  (call main (: 5 Int64))
  (output (: 16 Int64)))

(case
  "lcap2 an ESCAPING closure capturing a LET-LOCAL scalar applies across the boundary"
  (input
    (do
      (def (mk (: k Int64)) (let ((v (+ k 1))) (fn ((: y Int64)) (+ v y))))
      (def (main (: n Int64)) (mk n))
      (export main)))
  (call main (: 5 Int64) (: 10 Int64))
  (drop)
  (output (: 16 Int64))
  (live-objects 0))

; SOUNDNESS: distinct component signatures that COLLAPSE to the same CORE valtype shape. `a : (-> Int64
; Int64)` and `b : (-> Int64 UInt64)` are DISTINCT at the component boundary (s64 vs u64 result) — two
; resource types — yet both lower to the SAME core functype `(i32 env, i64) -> i64`. Each must still
; dispatch its OWN lifted body: the code slot rides in the resource rep (make-a → a t0 handle whose cell
; points at a's slot; make-b → a t1 handle at b's slot), recovered per call, so the shared core functype
; index is immaterial to WHICH body runs. If the two ever collided, `b` would run `a`'s body.
(case
  "distinct signatures sharing a core valtype shape dispatch distinct bodies"
  (doc
    "`a : (-> Int64 Int64)` returns `x + 1000`; `b : (-> Int64 UInt64)` returns `x * 7` — distinct
           component signatures (s64 vs u64 result) but the SAME core shape `(i64) -> i64`. Calling `a(3)`
           = 1003 runs a's body. Pins that a's resource + slot dispatch its own code despite the shared
           core functype.")
  (input
    (do
      (def (a) (fn ((: x Int64)) (+ x 1000)))
      (def (b) (fn ((: x Int64)) (UInt64.wrap (* x 7))))
      (export a)
      (export b)))
  (call a (: 3 Int64))
  (output (: 1003 Int64))
  (live-objects 1))

(case
  "the same-core-shape sibling dispatches ITS body, not the first"
  (doc
    "The same program, calling `b(3)` = 21 = `x * 7` (b's OWN body), NOT 1003 (a's). Pins the
           soundness property: two closures whose core functypes are identical still run distinct code,
           because the code slot is recovered from the resource rep at call time — a mispick would surface
           here as b returning a's result.")
  (input
    (do
      (def (a) (fn ((: x Int64)) (+ x 1000)))
      (def (b) (fn ((: x Int64)) (UInt64.wrap (* x 7))))
      (export a)
      (export b)))
  (call b (: 3 Int64))
  (output (: 21 UInt64))
  (live-objects 1))

; ROUND-TRIP CONSUMER BODY RICHNESS — a consumer's body is ordinary Cadenza code, and the handed-back
; closure is a first-class value in it that may be applied CONDITIONALLY (an `if`/`match` branch that does
; NOT apply it on every path) or bound through a `let`. This exercises a correctness property: the consumer
; wrapper `resource.rep`s the handle → cell and DROPs the cell (own<t> release) around the body call —
; sound even when the body never dispatches the closure on the taken path (the cell is still reclaimed).
(case
  "a round-trip consumer applies the closure only in the taken if-branch"
  (doc
    "`(def (app (: g (-> Int64 Int64)) (: x Int64)) (if (< x 0) 0 (g x)))` — applies `g` only when x
           ≥ 0. `mk()` + `app(handle, 5)` = (g 5) = 6. Pins that a consumer applies the handed-back closure
           inside control flow.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (if (< x 0) 0 (g x)))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a round-trip consumer that does NOT apply the closure on the taken branch"
  (doc
    "The same consumer, `app(handle, -3)` = 0 — the guarded branch is taken and `g` is NEVER applied.
           Pins the release soundness: the wrapper still `resource.rep`s + DROPs the handed-back cell even
           though the body did not dispatch it (own<t> is consumed at the boundary regardless).")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (if (< x 0) 0 (g x)))
      (export mk)
      (export app)))
  (call app (: -3 Int64))
  (output (: 0 Int64)))

(case
  "a round-trip consumer binds the applied closure through a let"
  (doc
    "`(def (app (: g (-> Int64 Int64)) (: x Int64)) (let ((y (g x))) (+ y 1)))` — `mk` multiplies by
           10, so `app(handle, 4)` = (g 4) + 1 = 40 + 1 = 41. Pins a `let`-bound application in a consumer.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (* x 10)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (let ((y (g x))) (+ y 1)))
      (export mk)
      (export app)))
  (call app (: 4 Int64))
  (output (: 41 Int64)))

(case
  "a round-trip consumer applies the closure in a match wildcard arm"
  (doc
    "`(def (app (: g (-> Int64 Int64)) (: x Int64)) (match x (0 999) (_ (g x))))` — `mk` adds 100.
           `app(handle, 5)` takes the wildcard → (g 5) = 105; `app(handle, 0)` takes the literal arm → 999,
           NOT applying `g`. Pins a `match`-dispatched consumer, applying the closure only in one arm.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 100)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (match x (0 999) (_ (g x))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: 105 Int64)))

(case
  "a round-trip consumer takes the non-applying match arm"
  (doc
    "The same match-bodied consumer, `app(handle, 0)` = 999 — the literal arm, `g` NOT applied.
           Confirms the handed-back cell is still released when the body's taken path skips the closure.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 100)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (match x (0 999) (_ (g x))))
      (export mk)
      (export app)))
  (call app (: 0 Int64))
  (output (: 999 Int64)))

; THE DISTINCT-SIGNATURE ROUND-TRIP — the flagship shape unified: a program that both PRODUCES and CONSUMES
; closures of DIFFERENT signatures. Each signature is its own resource type; a producer mints its closure
; and the matching consumer (paired by resource type) applies it. Here `adder`+`appa` work with `(-> Int64
; Int64)` (resource t0) and `isz`+`appb` with `(-> Int64 Bool)` (resource t1), all in one component. The
; host produces from the producer whose result resource type matches the consumer's closure param, then
; threads the handle in. This composes the round-trip (host-as-custodian) with N-resource-type grouping.
(case
  "a distinct-signature round-trip applies the Int64->Int64 closure"
  (doc
    "`adder : (Int64) -> (-> Int64 Int64)` + `appa : ((-> Int64 Int64), Int64) -> Int64` (resource
           t0), alongside `isz` + `appb` on `(-> Int64 Bool)` (resource t1). Calling `appa` produces from
           `adder(10)` (its matching producer, by resource type) → a handle → `appa(handle, 5)` = 15. Pins
           that a round trip mixing signatures pairs each consumer with the producer of its resource type.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
      (export adder)
      (export appa)
      (export isz)
      (export appb)))
  (call appa (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

(case
  "a distinct-signature round-trip applies the Int64->Bool closure"
  (doc
    "The same four-export program, now calling `appb` (the `(-> Int64 Bool)` consumer, resource t1):
           produced from `isz()` → a handle → `appb(handle, 0)` = true. The Bool-signature closure round-
           trips through its OWN resource type, distinct from the Int64 one.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
      (export adder)
      (export appa)
      (export isz)
      (export appb)))
  (call appb (: 0 Int64))
  (output (: true Bool)))

(case
  "a distinct-signature round-trip's Bool closure on a nonzero input"
  (doc
    "The same program, `appb(handle, 5)` = false (5 ≠ 0) — the t1 closure's result tracks its input,
           distinct from the t0 group. Confirms both resource types dispatch their own closures.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
      (export adder)
      (export appa)
      (export isz)
      (export appb)))
  (call appb (: 5 Int64))
  (output (: false Bool)))

; NON-KEBAB EXPORT NAMES — a component-model extern name MUST be kebab-case, but a Cadenza source
; identifier may be camelCase or snake_case (`mkA`, `appA`, `makeAdder`). Every PUBLIC closure-interface
; export name (`make-<src>`, a consumer's own name, `make-<src>` in a multi-export) is normalized at emit
; through `kebab_extern_name` (the same rule a bare scalar export uses); the private per-func wiring names
; are index-derived (`import-func-f<n>`) so a source name never leaks into them. The runner resolves the
; caller's SOURCE name through the SAME rule, so `(call appA …)` still finds the `app-a` export. These pins
; guard the boundary-name normalization end-to-end (a camelCase program used to emit an invalid component).
(case
  "a camelCase round-trip resolves through kebab boundary-name normalization"
  (doc
    "`mkA : (Int64) -> (-> Int64 Int64)` + `appA : ((-> Int64 Int64), Int64) -> Int64`. In a round
           trip a producer is exported under its OWN name, so the public exports emit as `mk-a`/`app-a`
           (kebab); calling `appA` produces from `mkA(10)` → a handle → `appA(handle, 5)` = 15. Pins that a
           camelCase closure round-trip emits a VALID component and the runner still resolves the source
           name.")
  (input
    (do
      (def (mkA (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (appA (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (export mkA)
      (export appA)))
  (call appA (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

(case
  "a camelCase same-signature multi-export normalizes each make-<name>"
  (doc
    "Two same-signature closure exports with camelCase names: `makeAdder(k)` = `x + k`, `makeScaler(k)`
           = `x * k`. They share ONE resource type + `call`; each `make-<src>` public name is kebabized
           (`make-make-adder`/`make-make-scaler`). `(call makeScaler 3 4)` → `makeScaler(3)` → a handle →
           `call(handle, 4)` = 4 * 3 = 12. Pins multi-export public-name normalization.")
  (input
    (do
      (def (makeAdder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (makeScaler (: k Int64)) (fn ((: x Int64)) (* x k)))
      (export makeAdder)
      (export makeScaler)))
  (call makeScaler (: 3 Int64) (: 4 Int64))
  (drop)
  (output (: 12 Int64))
  (live-objects 0))

; A CLOSURE EXPORT ALONGSIDE A NON-CLOSURE (PLAIN) EXPORT — a MIXED multi-export. The closure(s) cross via
; the resource envelope (`make-<name>` + a shared `call`, under `cadenza:closure/exports`); each plain export
; is aliased off the SAME program instance and published as an ORDINARY top-level component func. Both live
; in ONE component: the host reaches the plain export as a bare func, the closure through `make`/`call`. The
; `oracle_mixed_component` byte anchor proved the resource-instance + top-level-func coexistence. Scope: the
; closure exports share ONE signature; each plain export has an aliased-scalar param/result (a compound plain
; result is a later widening). `cdz-run` routes `(call <plain>)` to the bare func and `(call <closure>)` to
; make/call — a plain export whose name resolves to a top-level func stays on the plain path.
(case
  "a closure export alongside a plain scalar export — the plain export runs"
  (doc
    "`inc : () -> (-> Int64 Int64)` (a closure factory) is exported ALONGSIDE `two : () -> Int64` (a
           plain scalar). `(call two)` reaches the ORDINARY top-level `two` func → 2, unaffected by the
           closure interface riding alongside it. Pins that a plain export coexists with a closure export and
           the host drives it directly.")
  (input (do (def (inc) (fn ((: x Int64)) (+ x 1))) (def (two) 2) (export inc) (export two)))
  (call two)
  (output (: 2 Int64)))

(case
  "a closure export alongside a plain scalar export — the closure runs"
  (doc
    "The SAME mixed program, now calling the CLOSURE export `inc`: the host `make`s a handle then
           `call(handle, 5)` = 6, dispatched through the guest's `call_indirect`. Pins that the closure
           interface still works when a plain export shares the component (both envelopes composed).")
  (input (do (def (inc) (fn ((: x Int64)) (+ x 1))) (def (two) 2) (export inc) (export two)))
  (call inc (: 5 Int64))
  (drop)
  (output (: 6 Int64))
  (live-objects 0))

(case
  "a parameterized plain export alongside a closure export applies its argument"
  (doc
    "`adder : (Int64) -> (-> Int64 Int64)` (a capturing closure factory) alongside `dbl : (Int64) ->
           Int64` (a plain function that doubles). `(call dbl 21)` reaches the top-level `dbl` → 42 — a plain
           export with a PARAMETER rides alongside the closure make/call. Pins the non-nullary plain export.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (dbl (: n Int64)) (* n 2))
      (export adder)
      (export dbl)))
  (call dbl (: 21 Int64))
  (output (: 42 Int64)))

(case
  "a parameterized plain export alongside a closure export — the closure captures and applies"
  (doc
    "The SAME program, calling the capturing closure `adder`: `make(10)` builds a closure over k=10,
           then `call(handle, 5)` = 15. Confirms the capturing-closure make/call path is intact alongside a
           parameterized plain export.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (dbl (: n Int64)) (* n 2))
      (export adder)
      (export dbl)))
  (call adder (: 10 Int64) (: 5 Int64))
  (drop)
  (output (: 15 Int64))
  (live-objects 0))

(case
  "two same-signature closures alongside a plain export all coexist"
  (doc
    "TWO same-signature closure exports (`inc`, `triple`) share ONE resource type + `call`, riding
           alongside a plain `answer : () -> 42`. `(call triple 5)` = 15 (the `* x 3` closure), proving the
           multi-closure shared-`call` dispatch is unaffected by the plain export in the same component.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (triple) (fn ((: x Int64)) (* x 3)))
      (def (answer) 42)
      (export inc)
      (export triple)
      (export answer)))
  (call triple (: 5 Int64))
  (drop)
  (output (: 15 Int64))
  (live-objects 0))

(case
  "two same-signature closures alongside a plain export — the plain export runs"
  (doc
    "The SAME three-export program, calling the plain `answer` → 42. Pins that the plain export is
           reachable when TWO closures share the resource interface beside it.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (triple) (fn ((: x Int64)) (* x 3)))
      (def (answer) 42)
      (export inc)
      (export triple)
      (export answer)))
  (call answer)
  (output (: 42 Int64)))

; DISTINCT-SIGNATURE closures ALONGSIDE a plain export — the distinct-sig case of the mixed shape. Closures
; of DIFFERENT signatures cross as N resource types (each its own `make-<name>`/`call-g<n>`), and a plain
; export rides alongside as an ordinary top-level func. The distinct-sig envelope now carries plain exports
; too (aliased off the same program instance after the closure fns, lifted + exported at the top level).
; `cdz-run` routes `(call <plain>)` to the top-level bare func and `(call <closure>)` to its group's
; make/call-g<n> (matched by resource type). Composes N-resource-type grouping with the plain boundary.
(case
  "distinct-signature closures alongside a plain export — the Int64->Int64 closure runs"
  (doc
    "`inc : (-> Int64 Int64)` (resource t0) and `isz : (-> Int64 Bool)` (resource t1) cross as TWO
           resource types, alongside a plain `two : () -> 2`. Calling the closure `inc`: `make-inc()` → a
           handle → `call-g0(handle, 5)` = 6. Pins that distinct-sig grouping is unaffected by a plain
           export sharing the component.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (two) 2)
      (export inc)
      (export isz)
      (export two)))
  (call inc (: 5 Int64))
  (output (: 6 Int64))
  (live-objects 1))

(case
  "distinct-signature closures alongside a plain export — the Int64->Bool closure runs"
  (doc
    "The SAME program, calling the OTHER-signature closure `isz` (resource t1): `make-isz()` → a
           handle → `call-g1(handle, 0)` = true. Confirms both resource types dispatch their own closures
           with a plain export present.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (two) 2)
      (export inc)
      (export isz)
      (export two)))
  (call isz (: 0 Int64))
  (output (: true Bool))
  (live-objects 1))

(case
  "distinct-signature closures alongside a plain export — the plain export runs"
  (doc
    "The SAME program, calling the plain `two` → 2. Pins that the top-level plain export is reachable
           when TWO distinct resource types ride beside it in `cadenza:closure/exports`.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (two) 2)
      (export inc)
      (export isz)
      (export two)))
  (call two)
  (output (: 2 Int64)))

(case
  "distinct-signature capturing closures alongside a parameterized plain export"
  (doc
    "`adder : (Int64) -> (-> Int64 Int64)` (t0, captures k) and `gte : (Int64) -> (-> Int64 Bool)`
           (t1, captures a threshold) cross as two resource types beside a plain `dbl : (Int64) -> Int64`.
           `(call gte 3 5)` → `make-gte(3)` builds `(fn (x) (>= x 3))`, then `call-g1(handle, 5)` = true
           (5 >= 3). Composes distinct-sig capture with a parameterized plain export.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (gte (: t Int64)) (fn ((: x Int64)) (>= x t)))
      (def (dbl (: n Int64)) (* n 2))
      (export adder)
      (export gte)
      (export dbl)))
  (call gte (: 3 Int64) (: 5 Int64))
  (output (: true Bool))
  (live-objects 1))

(case
  "distinct-signature capturing closures alongside a parameterized plain export — the plain runs"
  (doc
    "The SAME four-export program, calling the parameterized plain `dbl(21)` = 42. Pins the
           non-nullary plain export reachable beside two distinct capturing-closure resource types.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (gte (: t Int64)) (fn ((: x Int64)) (>= x t)))
      (def (dbl (: n Int64)) (* n 2))
      (export adder)
      (export gte)
      (export dbl)))
  (call dbl (: 21 Int64))
  (output (: 42 Int64)))

; A ROUND-TRIP (produce + consume) ALONGSIDE a plain non-closure export. The producer mints a closure, the
; consumer takes it back and applies it, and a plain export rides alongside as an ordinary top-level func —
; all in ONE component. Before this, the round-trip path SILENTLY DROPPED a plain export (a valid component
; missing the name), a miscompile; now the plain body is aliased off the same program instance, lifted, and
; exported at the top level. `cdz-run` routes `(call <plain>)` to the bare func and `(call <consumer>)` to
; the round-trip (produce-then-consume).
(case
  "a round-trip alongside a plain export — the plain export runs"
  (doc
    "`mk : () -> (-> Int64 Int64)` produces, `app : ((-> Int64 Int64), Int64) -> Int64` consumes, and
           a plain `two : () -> 2` rides alongside. `(call two)` reaches the ORDINARY top-level `two` func →
           2. Pins that a plain export is REACHABLE in a round-trip program (was silently dropped).")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (two) 2)
      (export mk)
      (export app)
      (export two)))
  (call two)
  (output (: 2 Int64)))

(case
  "a round-trip alongside a plain export — the round-trip still works"
  (doc
    "The SAME program, driving the ROUND-TRIP consumer `app`: the host produces a closure from `mk()`
           → a handle → `app(handle, 5)` = 6. Pins that the round-trip (produce-then-consume) is intact when
           a plain export shares the component.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (+ x 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (two) 2)
      (export mk)
      (export app)
      (export two)))
  (call app (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a round-trip alongside a parameterized plain export applies its argument"
  (doc
    "A capturing round trip — `adder : (Int64) -> (-> Int64 Int64)` produces, `app` consumes — beside a
           parameterized plain `dbl : (Int64) -> Int64`. `(call dbl 21)` = 42 reaches the top-level `dbl`.
           Pins a non-nullary plain export beside a capturing round trip.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (dbl (: n Int64)) (* n 2))
      (export adder)
      (export app)
      (export dbl)))
  (call dbl (: 21 Int64))
  (output (: 42 Int64)))

; A DISTINCT-SIGNATURE ROUND-TRIP alongside a plain export. Producers + consumers of DIFFERENT signatures
; cross as N resource types, and a plain export rides alongside. Before this the distinct-sig round-trip
; DECLINED any non-producer/non-consumer export; now it carries plain exports as top-level funcs.
(case
  "a distinct-signature round-trip alongside a plain export — the Int64->Int64 side runs"
  (doc
    "`adder`+`appa` on `(-> Int64 Int64)` (t0) and `isz`+`appb` on `(-> Int64 Bool)` (t1), beside a
           plain `two : () -> 2`. Driving `appa`: produce from `adder(10)` → a handle → `appa(handle, 5)` =
           15. Pins that distinct-sig round-trip grouping is intact with a plain export present.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
      (def (two) 2)
      (export adder)
      (export appa)
      (export isz)
      (export appb)
      (export two)))
  (call appa (: 10 Int64) (: 5 Int64))
  (output (: 15 Int64)))

(case
  "a distinct-signature round-trip alongside a plain export — the Int64->Bool side runs"
  (doc
    "The SAME five-export program, driving `appb` (the `(-> Int64 Bool)` side, t1): produce from
           `isz()` → a handle → `appb(handle, 0)` = true. Confirms both resource types round-trip with a
           plain export present.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
      (def (two) 2)
      (export adder)
      (export appa)
      (export isz)
      (export appb)
      (export two)))
  (call appb (: 0 Int64))
  (output (: true Bool)))

(case
  "a distinct-signature round-trip alongside a plain export — the plain export runs"
  (doc
    "The SAME five-export program, calling the plain `two` → 2. Pins that the top-level plain export is
           reachable when TWO distinct round-trip resource types share the component.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (def (appb (: h (-> Int64 Bool)) (: x Int64)) (h x))
      (def (two) 2)
      (export adder)
      (export appa)
      (export isz)
      (export appb)
      (export two)))
  (call two)
  (output (: 2 Int64)))

; NOMINAL-over-scalar at the closure boundary. A single-variant nominal like `(type UserId (Mk Int64))`
; ERASES to its underlying scalar at run time (type-system.md §156 — the tag "adds nothing to the value's
; runtime representation"), so a closure whose arg or result is such a nominal crosses the `call` boundary
; as the underlying scalar (`UserId` → `s64`), the tag stripped. `closure_boundary_byte` peels the nominal
; (`strip_nominal`) to pick the boundary byte, and the core `call` functype uses the scalar valtype — so
; the host sends/receives a plain scalar and the nominal identity is a compile-time-only concern. These pin
; that the nominal is transparent at the boundary (the host sees the scalar, not a wrapper resource).
(case
  "a closure returning a nominal-over-scalar crosses as the underlying scalar"
  (doc
    "`(type UserId (Mk Int64))` + `(fn (x) (Mk x))` — the closure result type is `UserId`, which
           erases to Int64. The `call` method's result functype is `s64` (the nominal peeled), so
           `call(handle, 42)` returns 42 rendered as the scalar. Pins that a nominal result is transparent
           at the host boundary — no wrapper resource, just the underlying scalar.")
  (input (do (type UserId (Mk Int64)) (def (main) (fn ((: x Int64)) (Mk x))) (export main)))
  (call main (: 42 Int64))
  (drop)
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a closure taking a nominal-over-scalar argument receives the underlying scalar"
  (doc
    "`(fn (u) (+ (unwrap u) 1))` where `u : UserId` — the closure's ARG is a nominal, crossing as
           Int64. The host passes 7, the guest matches out the payload (`(Mk n) → n`), adds 1 → 8. Pins the
           nominal ARG side of the boundary (companion to the result case).")
  (input
    (do
      (type UserId (Mk Int64))
      (def (unwrap (: u UserId)) (match u ((Mk n) n)))
      (def (main) (fn ((: u UserId)) (+ (unwrap u) 1)))
      (export main)))
  (call main (: 7 Int64))
  (drop)
  (output (: 8 Int64))
  (live-objects 0))

(case
  "a capturing closure returning a nominal-over-scalar"
  (doc
    "`(def (tagger base) (fn (x) (Mk (+ x base))))` captures `base` and returns a `Tag` (nominal over
           Int64). `make(100)` builds a closure over base=100, then `call(handle, 5)` = Mk(105) → 105 at
           the boundary. Composes make-param capture with a nominal result.")
  (input
    (do
      (type Tag (Mk Int64))
      (def (tagger (: base Int64)) (fn ((: x Int64)) (Mk (+ x base))))
      (export tagger)))
  (call tagger (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: 105 Int64))
  (live-objects 0))

(case
  "a round-trip consumer applies a closure whose result is a nominal-over-scalar"
  (doc
    "Producer `mk : () -> (-> Int64 Tag)` mints a closure returning `Tag`; consumer `app` takes it
           back, applies it, matches out the payload and doubles it. `mk()` → a handle → `app(handle, 7)` =
           `(Mk 7)` → 14. Pins a nominal-result closure through the round trip (produce + consume).")
  (input
    (do
      (type Tag (Mk Int64))
      (def (mk) (fn ((: x Int64)) (Mk x)))
      (def (app (: g (-> Int64 Tag)) (: x Int64)) (match (g x) ((Mk n) (* n 2))))
      (export mk)
      (export app)))
  (call app (: 7 Int64))
  (output (: 14 Int64)))

(case
  "a closure returning a nominal-over-Bool erases to bool at the boundary"
  (doc
    "`(type Flag (Mk Bool))` + `(fn (x) (Mk (> x 0)))` — a nominal over Bool, not Int. The `call`
           result crosses as `bool` (the peeled underlying type), so `call(handle, 5)` = Mk(true) → true.
           Confirms the nominal peel is width/kind-agnostic (Bool underlying, not only integers).")
  (input (do (type Flag (Mk Bool)) (def (main) (fn ((: x Int64)) (Mk (> x 0)))) (export main)))
  (call main (: 5 Int64))
  (drop)
  (output (: true Bool))
  (live-objects 0))

; A COMPOUND-RESULT closure: the closure's result is a runtime `Bytes`, which crosses the `call` boundary
; as `list<u8>` (the raw payload) rather than a scalar. Unlike a scalar `call`, the emitted core carries a
; MEMORY + `cabi_realloc`, and `call` — after dispatching the lifted closure (which returns a runtime Bytes
; HANDLE) — runs a `bytes-len`/`bytes-get` copy loop writing the payload + the canonical `(ptr, len)` return
; area, then drops both the closure cell and the transient Bytes handle. The `call` is lifted with
; Memory/Realloc canon options (`assemble_closure_bytes_resource`), the shape the compound-result oracle
; proved runs. The host reads the bytes back directly (a bare `list<u8>`, rendered as the byte sequence).
(case
  "a closure returning Bytes crosses to the host as list<u8>"
  (doc
    "`(fn (n) (bin (u8 n) (u8 n+1)))` — the closure's result is a runtime `Bytes`. `make()` → a
           handle; `call(handle, 5)` dispatches the closure (building `[5, 6]` on the value heap), and the
           `call` method copies that Bytes handle into linear memory and returns it as `list<u8>` — the host
           reads `(5 6)`. Pins the compound-result closure boundary end-to-end (memory + cabi_realloc +
           Memory/Realloc-lifted `call` + the bytes copy loop).")
  (input
    (do
      (def (main) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (export main)))
  (call main (: 5 Int64))
  (drop)
  (output #list(5 6))
  (live-objects 0))

(case
  "a Bytes-returning closure on a different argument"
  (doc
    "The same `(fn (n) (bin (u8 n) (u8 n+1)))`, called with 100 → the bytes `[100, 101]`. Confirms the
           copied payload tracks the closure's runtime input, not a fixed buffer.")
  (input
    (do
      (def (main) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (export main)))
  (call main (: 100 Int64))
  (drop)
  (output #list(100 101))
  (live-objects 0))

(case
  "a capturing closure returning Bytes"
  (doc
    "`(def (tag (: hdr Int64)) (fn (n) (bin (u8 hdr) (u8 n))))` captures a header byte and returns a
           2-byte `Bytes`. `make(9)` builds a closure over hdr=9, then `call(handle, 200)` → `[9, 200]`.
           Composes make-param capture with a compound (`Bytes`) closure result.")
  (input
    (do
      (def (tag (: hdr Int64)) (fn ((: n Int64)) (bin (u8 (UInt8.wrap hdr)) (u8 (UInt8.wrap n)))))
      (export tag)))
  (call tag (: 9 Int64) (: 200 Int64))
  (drop)
  (output #list(9 200))
  (live-objects 0))

; A STRING closure result crosses the same way a `Bytes` one does. A `String` is a UTF-8 byte-rope handle,
; representationally IDENTICAL to `Bytes` (the same value-heap `bytes-*` store), so a closure returning a
; `String` takes the very same compound-result `call` path — its `call` copies the UTF-8 bytes into linear
; memory and returns them as `list<u8>` (the encoded bytes, not a decoded string). `emit_closure_resource`
; routes a `String` result to the bytes shape exactly as a `Bytes` result (`ret_is_bytes` accepts both).
(case
  "a closure returning a constant String crosses as its UTF-8 bytes"
  (doc
    "`(fn (n) \"hi\")` — the closure's result is a `String`. `call(handle, 0)` copies the UTF-8 bytes
           of \"hi\" (`[104, 105]`) out through the canonical `list<u8>` ABI, and the host reads `(104 105)`.
           Pins that a `String` result crosses as its bytes on the same path as `Bytes` (a byte-rope handle
           is a byte-rope handle).")
  (input (do (def (main) (fn ((: n Int64)) "hi")) (export main)))
  (call main (: 0 Int64))
  (drop)
  (output #list(104 105))
  (live-objects 0))

(case
  "a closure returning a runtime String (concat) crosses as its bytes"
  (doc
    "`(fn (n) (String.concat \"ab\" \"c\"))` — a RUNTIME String built by `concat` (not a folded
           constant handle). `call(handle, 0)` → the UTF-8 bytes of \"abc\" = `[97, 98, 99]`. Confirms the
           bytes copy reads a genuine runtime byte-rope handle, not only a compile-time-known string.")
  (input (do (def (main) (fn ((: n Int64)) (String.concat "ab" "c"))) (export main)))
  (call main (: 0 Int64))
  (drop)
  (output #list(97 98 99))
  (live-objects 0))

(case
  "a capturing closure returning a String"
  (doc
    "`(def (mk k) (fn (n) (String.concat \"x\" \"y\")))` — a make-parameterized closure whose result
           is a String. `make(7)` builds it, then `call(handle, 0)` → the bytes of \"xy\" = `[120, 121]`.
           Composes make-param capture with a `String` closure result.")
  (input (do (def (mk (: k Int64)) (fn ((: n Int64)) (String.concat "x" "y"))) (export mk)))
  (call mk (: 7 Int64) (: 0 Int64))
  (drop)
  (output #list(120 121))
  (live-objects 0))

; EMPTY byte-rope closure results — the copy loop must handle n=0 (empty Bytes / empty String). An empty
; compound crosses as an empty `list<u8>`, so the `call` writes a `(ptr, len=0)` return area and the host
; reads the empty list. Pins the boundary edge (a zero-length payload must not read a stray byte or trap).
(case
  "a closure returning an empty Bytes crosses as the empty list"
  (doc
    "`(fn (n) (bin))` — an empty `Bytes`. `call(handle, 0)` copies zero bytes and returns
           `(ptr, len=0)`; the host reads `()`. Pins the n=0 edge of the bytes copy loop (a `bytes-len` of
           0 must skip the loop cleanly).")
  (input (do (def (main) (fn ((: n Int64)) (bin))) (export main)))
  (call main (: 0 Int64))
  (drop)
  (output #list())
  (live-objects 0))

(case
  "a closure returning an empty String crosses as the empty list"
  (doc
    "The String companion: `(fn (n) \"\")` — an empty String (an empty UTF-8 byte-rope) crosses as the
           empty `list<u8>`. Confirms the n=0 edge on the String result path too.")
  (input (do (def (main) (fn ((: n Int64)) "")) (export main)))
  (call main (: 0 Int64))
  (drop)
  (output #list())
  (live-objects 0))

; MULTI-EXPORT byte-rope-result closures: N same-signature closures each returning a `Bytes`/`String` share
; ONE `call` that returns `list<u8>` — the multi-export shape (N `make-<name>` + one shared `call`) extended
; to the compound-result `call` (memory + cabi_realloc + the bytes copy loop). The shared `call` recovers the
; code slot from the rep, dispatches whichever closure the handle names, then copies its byte-rope result out.
(case
  "two same-signature Bytes-returning closures share one call — first"
  (doc
    "`a : () -> (-> Int64 Bytes)` (1 byte) and `b` (2 bytes), same signature → ONE resource type + one
           shared list-returning `call`. `make-a()` → a handle; `call(handle, 5)` copies a's `[5]` out. Pins
           the multi-export byte-rope `call` (N makes, one shared memory/realloc list-`call`).")
  (input
    (do
      (def (a) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)))))
      (def (b) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (export a)
      (export b)))
  (call a (: 5 Int64))
  (drop)
  (output #list(5))
  (live-objects 0))

(case
  "two same-signature Bytes-returning closures share one call — second"
  (doc
    "The same program, driving `b`: `make-b()` → a handle; `call(handle, 5)` = `[5, 6]`. The SHARED
           `call` dispatches whichever closure the rep names (b's 2-byte body here), proving the shared
           list-`call` is not fixed to one make.")
  (input
    (do
      (def (a) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)))))
      (def (b) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (export a)
      (export b)))
  (call b (: 5 Int64))
  (drop)
  (output #list(5 6))
  (live-objects 0))

(case
  "two same-signature String-returning closures share one call"
  (doc
    "`greet` and `bye` both `() -> (-> Int64 String)` share one resource type + list-`call`. Driving
           `bye`: `call(handle, 0)` → the UTF-8 bytes of \"by\" = `[98, 121]`. Confirms the multi-export
           byte-rope `call` is agnostic to Bytes-vs-String (both are byte-rope handles).")
  (input
    (do
      (def (greet) (fn ((: n Int64)) "hi"))
      (def (bye) (fn ((: n Int64)) "by"))
      (export greet)
      (export bye)))
  (call bye (: 0 Int64))
  (drop)
  (output #list(98 121))
  (live-objects 0))

; A BYTE-ROPE-result closure ALONGSIDE a PLAIN export — the mixed shape extended to the compound `call`.
; The closure's `Bytes`/`String` result crosses as `list<u8>` (the shared list-returning `call` with
; memory/cabi_realloc), and the plain export rides alongside as an ordinary top-level func. Both live in one
; component; `cdz-run` routes `(call <plain>)` to the bare func and `(call <closure>)` to make/call.
(case
  "a Bytes-returning closure alongside a plain export — the closure runs"
  (doc
    "`mk : () -> (-> Int64 Bytes)` (returns `(bin (u8 n) (u8 n+1))`) alongside a plain `two : () -> 2`.
           `make()` → a handle; `call(handle, 5)` copies the closure's `[5, 6]` out as `list<u8>`. Pins the
           byte-rope closure result on the MIXED path (the compound `call` + a plain top-level export).")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call mk (: 5 Int64))
  (drop)
  (output #list(5 6))
  (live-objects 0))

(case
  "a Bytes-returning closure alongside a plain export — the plain runs"
  (doc
    "The SAME mixed program, calling the plain `two` → 2. Pins that the plain top-level export is
           reachable when a compound-result closure shares the component.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call two)
  (output (: 2 Int64)))

(case
  "a String-returning closure alongside a parameterized plain export"
  (doc
    "`greet : () -> (-> Int64 String)` returns \"hi\", alongside a plain `dbl : (Int64) -> Int64`.
           `call(greet-handle, 0)` → the UTF-8 bytes `[104, 105]`. Confirms a String-result closure + a
           parameterized plain export coexist.")
  (input
    (do
      (def (greet) (fn ((: n Int64)) "hi"))
      (def (dbl (: x Int64)) (* x 2))
      (export greet)
      (export dbl)))
  (call greet (: 0 Int64))
  (drop)
  (output #list(104 105))
  (live-objects 0))

(case
  "a String-returning closure alongside a parameterized plain export — the plain runs"
  (doc
    "The SAME program, calling `dbl(21)` = 42. Pins the parameterized plain export reachable beside a
           String-result closure.")
  (input
    (do
      (def (greet) (fn ((: n Int64)) "hi"))
      (def (dbl (: x Int64)) (* x 2))
      (export greet)
      (export dbl)))
  (call dbl (: 21 Int64))
  (output (: 42 Int64)))

; BYTE-ROPE result on the DISTINCT-SIGNATURE path — closures of DIFFERENT signatures each returning a
; `Bytes`/`String` cross as G distinct resource types, each with its OWN `call-<g>` that returns `list<u8>`
; (memory + cabi_realloc shared across groups). Extends the byte-rope compound `call` from the single/multi/
; mixed shapes to the N-resource-type shape. Also covers a byte-rope group coexisting with a SCALAR group in
; the same component (the scalar `call-<g>` returns by value; the byte-rope one via the copy loop).
(case
  "distinct-sig byte-rope closures — the Int64→Bytes one"
  (doc
    "`mkb : () -> (-> Int64 Bytes)` (returns `(bin n n+1)`) and `mks : () -> (-> Bool Bytes)` cross as
           TWO distinct resource types (different arg types → distinct signatures), each with its own
           `list<u8>`-returning `call`. `call(mkb-handle, 5)` copies `[5,6]` out. Pins the byte-rope result
           on the distinct-signature path.")
  (input
    (do
      (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (def (mks) (fn ((: b Bool)) (bin (u8 (if b 1 0)))))
      (export mkb)
      (export mks)))
  (call mkb (: 5 Int64))
  (drop)
  (output #list(5 6))
  (live-objects 0))

(case
  "distinct-sig byte-rope closures — the Bool→Bytes one"
  (doc
    "The SAME two-resource program, driving the OTHER signature: `call(mks-handle, true)` → `[1]`.
           Confirms each distinct byte-rope resource dispatches its own closure body.")
  (input
    (do
      (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (def (mks) (fn ((: b Bool)) (bin (u8 (if b 1 0)))))
      (export mkb)
      (export mks)))
  (call mks (: true Bool))
  (drop)
  (output #list(1))
  (live-objects 0))

(case
  "distinct-sig: a byte-rope closure coexists with a SCALAR closure — the byte-rope one"
  (doc
    "`mkb : () -> (-> Int64 Bytes)` and `inc : () -> (-> Int64 Int64)` are distinct signatures → two
           resource types. The byte-rope group's `call` returns `list<u8>` (memory + realloc); the scalar
           group's returns by value. `call(mkb-handle, 9)` → `[9,10]`. Pins a byte-rope and a scalar group
           coexisting in ONE component.")
  (input
    (do
      (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (export mkb)
      (export inc)))
  (call mkb (: 9 Int64))
  (drop)
  (output #list(9 10))
  (live-objects 0))

(case
  "distinct-sig: a byte-rope closure coexists with a SCALAR closure — the scalar one"
  (doc
    "The SAME mixed byte-rope/scalar program, driving the SCALAR group: `call(inc-handle, 41)` → 42
           (returned by value, NOT as a byte list). Confirms the scalar `call-<g>` is unaffected by the
           sibling byte-rope group's memory/realloc plumbing.")
  (input
    (do
      (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1))))))
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (export mkb)
      (export inc)))
  (call inc (: 41 Int64))
  (output (: 42 Int64))
  (live-objects 1))

(case
  "distinct-sig: a String closure + a Bytes closure of different signatures — the String one"
  (doc
    "`greet : () -> (-> Int64 String)` returns \"hi\" (UTF-8 `[104,105]`), alongside `mkb : () -> (->
           Bool Bytes)`. Both cross as byte-rope `list<u8>` results but through DISTINCT resource types.
           `call(greet-handle, 0)` → `[104,105]`.")
  (input
    (do
      (def (greet) (fn ((: n Int64)) "hi"))
      (def (mkb) (fn ((: b Bool)) (bin (u8 (if b 7 8)))))
      (export greet)
      (export mkb)))
  (call greet (: 0 Int64))
  (drop)
  (output #list(104 105))
  (live-objects 0))

(case
  "distinct-sig byte-rope closure alongside a plain export — the closure"
  (doc
    "Two distinct byte-rope closures (`mkb : Int64→Bytes`, `isz : Bool→Bytes`) AND a plain `two : ()
           -> 2` all in one component. `call(mkb-handle, 3)` → `[3]`. Pins the byte-rope distinct-sig path
           carrying a plain export alongside (via `assemble_distinct_sig_resource_mixed`).")
  (input
    (do
      (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)))))
      (def (isz) (fn ((: b Bool)) (bin (u8 (if b 0 1)))))
      (def (two) 2)
      (export mkb)
      (export isz)
      (export two)))
  (call mkb (: 3 Int64))
  (drop)
  (output #list(3))
  (live-objects 0))

(case
  "distinct-sig byte-rope closure alongside a plain export — the plain"
  (doc
    "The SAME program, calling the plain `two` → 2. Confirms the plain top-level export is reachable
           when TWO distinct byte-rope closure resources share the component.")
  (input
    (do
      (def (mkb) (fn ((: n Int64)) (bin (u8 (UInt8.wrap n)))))
      (def (isz) (fn ((: b Bool)) (bin (u8 (if b 0 1)))))
      (def (two) 2)
      (export mkb)
      (export isz)
      (export two)))
  (call two)
  (output (: 2 Int64)))

; BYTE-ROPE result on the ROUND-TRIP path — a consumer takes a produced closure back, applies it, and
; RETURNS a `Bytes`/`String`. The consumer crosses as `(own<t>, args…) -> list<u8>` (memory + cabi_realloc
; shared), completing the byte-rope compound `call` across ALL closure shapes (single/multi/mixed/distinct-
; sig/round-trip). A byte-rope consumer can coexist with a scalar consumer of the same closure and with a
; plain export. (Also fixed a latent BinBuild slot-typing bug: two `(g x)` closure applications across two
; `bin` segments aliased one wasm local at two widths — now each segment's value floats above the
; high-water mark, the same disjoint-slot discipline the checked-arith path uses.)
(case
  "round-trip: a consumer applies the handed-back closure and returns Bytes"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects 0)
  (doc
    "`mk : () -> (-> Int64 Int64)` (adds 1); `app : (own<t>, Int64) -> Bytes` applies the handed-back
           closure TWICE — `(bin (u8 (g x)) (u8 (g x)+1))`. Host produces a handle via `mk`, hands it to
           `app(handle, 5)` → the closure yields 6, so the bytes are `[6, 7]`. Pins the byte-rope result on
           the round-trip path (the consumer returns `list<u8>`).")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def
        (app (: g (-> Int64 Int64)) (: x Int64))
        (bin (u8 (UInt8.wrap (g x))) (u8 (UInt8.wrap (+ (g x) 1)))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (drop)
  (output #list(6 7)))

(case
  "round-trip: a consumer returns a byte-rope built from a single closure result"
  (doc
    "`mk` doubles; `app : (own<t>, Int64) -> Bytes` = `(bin (u8 (g x)))`. `app(handle, 10)` → the
           closure yields 20 → `[20]`. The single-segment byte-rope consumer result.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (* n 2)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (export mk)
      (export app)))
  (call app (: 10 Int64))
  (output #list(20)))

(case
  "round-trip: a String-returning consumer of a closure"
  (doc
    "`label : (own<t>, Int64) -> String` returns the constant \"hi\" (UTF-8 `[104,105]`) — a String
           consumer result crosses on the same byte-rope `list<u8>` path as Bytes.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 65)))
      (def (label (: g (-> Int64 Int64)) (: x Int64)) "hi")
      (export mk)
      (export label)))
  (call label (: 0 Int64))
  (output #list(104 105)))

(case
  "round-trip byte-rope consumer alongside a plain export — the consumer"
  (doc
    "`app : (own<t>, Int64) -> Bytes` beside a plain `seven : () -> 7`. `app(handle, 41)` → `[42]`.
           Pins the byte-rope round-trip consumer carrying a plain export alongside.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (def (seven) 7)
      (export mk)
      (export app)
      (export seven)))
  (call app (: 41 Int64))
  (output #list(42)))

(case
  "round-trip byte-rope consumer alongside a plain export — the plain"
  (doc
    "The SAME program, calling the plain `seven` → 7. Confirms the plain top-level export is reachable
           when a byte-rope round-trip consumer shares the component.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (def (seven) 7)
      (export mk)
      (export app)
      (export seven)))
  (call seven)
  (output (: 7 Int64)))

(case
  "round-trip: a scalar consumer and a byte-rope consumer of the same closure — the byte-rope one"
  (doc
    "One closure signature, TWO consumers: `asnum : (own<t>, Int64) -> Int64` (returns the value) and
           `asbytes : (own<t>, Int64) -> Bytes` (wraps it into a `bin`). `asbytes(handle, 8)` → `[9]`. Pins a
           SCALAR consumer and a BYTE-ROPE consumer of the same resource coexisting (one lifted by value, one
           with Memory/Realloc).")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (asbytes (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (export mk)
      (export asnum)
      (export asbytes)))
  (call asbytes (: 8 Int64))
  (output #list(9)))

(case
  "round-trip: a scalar consumer and a byte-rope consumer of the same closure — the scalar one"
  (doc
    "The SAME two-consumer program, driving the SCALAR consumer: `asnum(handle, 8)` → 9 (by value, NOT
           a byte list). Confirms the scalar consumer is unaffected by the sibling byte-rope consumer's
           memory/realloc lift.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (asbytes (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (export mk)
      (export asnum)
      (export asbytes)))
  (call asnum (: 8 Int64))
  (output (: 9 Int64)))

; BYTE-ROPE result on the DISTINCT-SIG ROUND-TRIP path — the LAST byte-rope gap. Closures of DIFFERENT
; signatures each cross as their own resource type, and a CONSUMER of one signature can RETURN a
; `Bytes`/`String` (crossing as `(own<t_g>, args…) -> list<u8>`, memory + cabi_realloc shared across groups).
; Completes the byte-rope compound `call` across EVERY closure shape. A byte-rope consumer coexists with a
; scalar consumer of another signature, and two byte-rope consumers of different signatures coexist.
(case
  "distinct-sig round-trip: a byte-rope consumer + a scalar consumer of another sig — the byte-rope one"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects 0)
  (doc
    "`mka : () -> (-> Int64 Int64)` and `mkb : () -> (-> Bool Int64)` are distinct signatures → two
           resource types. `appa : (own<t0>, Int64) -> Bytes` applies its closure TWICE — `(bin (u8 (g x))
           (u8 (g x)+1))`. Host produces via `mka`, hands to `appa(handle, 5)` → `[6, 7]`. Pins the byte-rope
           consumer result on the distinct-sig round-trip path.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
      (def
        (appa (: g (-> Int64 Int64)) (: x Int64))
        (bin (u8 (UInt8.wrap (g x))) (u8 (UInt8.wrap (+ (g x) 1)))))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 5 Int64))
  (drop)
  (output #list(6 7)))

(case
  "distinct-sig round-trip: a byte-rope consumer + a scalar consumer of another sig — the scalar one"
  (doc
    "The SAME two-resource-type program, driving the SCALAR consumer of the OTHER signature: `appb :
           (own<t1>, Bool) -> Int64` applies `mkb`'s closure → `appb(handle, true)` = 10 (by value). Confirms
           the scalar consumer is unaffected by the sibling byte-rope consumer's memory/realloc plumbing.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
      (def
        (appa (: g (-> Int64 Int64)) (: x Int64))
        (bin (u8 (UInt8.wrap (g x))) (u8 (UInt8.wrap (+ (g x) 1)))))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: true Bool))
  (output (: 10 Int64)))

(case
  "distinct-sig round-trip: TWO byte-rope consumers of different signatures — the Int64 one"
  (doc
    "Both consumers return `Bytes`, but of DISTINCT closure signatures (two resource types, each
           lifted with its own Memory/Realloc). `appa(mka-handle, 40)` → `[41]`.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (bin (u8 (UInt8.wrap (h y))) (u8 99)))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 40 Int64))
  (output #list(41)))

(case
  "distinct-sig round-trip: TWO byte-rope consumers of different signatures — the Bool one"
  (doc
    "The SAME program, driving the OTHER byte-rope consumer: `appb(mkb-handle, false)` → `mkb`'s
           closure yields 8, so `(bin (u8 8) (u8 99))` = `[8, 99]`. Confirms each distinct byte-rope
           resource dispatches its own closure body + writes its own `list<u8>`.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (bin (u8 (UInt8.wrap (h y))) (u8 99)))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: false Bool))
  (output #list(8 99)))

; A COMPOUND (tuple/record) closure RESULT — the closure's `call` returns the canonical VALUE FORM as
; `list<u8>` (the value-heap escape's `runtime_value_form_template` + `encode_walk_body` walker, keyed on
; the closure's returned handle), so the host DECODES + pretty-prints the typed `(: value T)` document (not
; a bare byte sequence like the byte-rope path). cdz-run try-decodes the `call` result: the codec's 8-byte
; schema header disambiguates a value form from a raw byte-rope, so both share the `list<u8>` boundary
; unambiguously. Fixed-shape compounds (tuple/record/sum) are supported; a variable-length list still
; declines (no fixed template).
(case
  "a closure returning a tuple crosses as the typed value form"
  (doc
    "`mk : () -> (-> Int64 (Tuple Int64 Int64))` returns `(tuple n n+1)`. `call(handle, 5)` walks the
           returned tuple handle, writes the value form, and the host decodes it to `(: (tuple 5 6) (Tuple
           Int64 Int64))` — the FULL typed document, not a bare byte list.")
  (input (do (def (mk) (fn ((: n Int64)) #tuple(n (+ n 1)))) (export mk)))
  (call mk (: 5 Int64))
  (drop)
  (output (: (tuple 5 6) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "a closure returning a record crosses as the typed value form"
  (doc
    "A record result — `(record (x n) (y n+10))` → `(: (record (x 3) (y 13)) (Record (: x Int64) (: y Int64)))`. Field names + the record type node are baked in the template; only the leaf values are
           walked at run time.")
  (input (do (def (mk) (fn ((: n Int64)) #record((= x n) (= y (+ n 10))))) (export mk)))
  (call mk (: 3 Int64))
  (drop)
  (output (: (record (= x 3) (= y 13)) (Record (: x Int64) (: y Int64))))
  (live-objects 0))

(case
  "a closure returning a tuple with a Bool leaf"
  (doc
    "A mixed-leaf compound — `(tuple n (< n 5))` → `(: (tuple 2 true) (Tuple Int64 Bool))`. The Bool
           leaf's hole is filled via `get-bool` (its kind byte flipped true/false), the int via `get-int`.")
  (input (do (def (mk) (fn ((: n Int64)) #tuple(n (< n 5)))) (export mk)))
  (call mk (: 2 Int64))
  (drop)
  (output (: (tuple 2 true) (Tuple Int64 Bool)))
  (live-objects 0))

(case
  "a closure returning a NESTED tuple"
  (doc
    "`(tuple n (tuple n+1 n+2))` → `(: (tuple 7 (tuple 8 9)) (Tuple Int64 (Tuple Int64 Int64)))`. The
           walker descends nested `arr-get` paths (the inner tuple is a boxed handle inside the outer).")
  (input (do (def (mk) (fn ((: n Int64)) #tuple(n #tuple((+ n 1) (+ n 2))))) (export mk)))
  (call mk (: 7 Int64))
  (drop)
  (output (: (tuple 7 (tuple 8 9)) (Tuple Int64 (Tuple Int64 Int64))))
  (live-objects 0))

(case
  "a CAPTURING closure returning a tuple"
  (doc
    "`mk : (Int64) -> (-> Int64 (Tuple Int64 Int64))` — `make(100)` captures `k=100`, then
           `call(handle, 5)` → `(: (tuple 100 5) (Tuple Int64 Int64))`. Confirms a captured value flows into
           the compound result across the boundary.")
  (input (do (def (mk (: k Int64)) (fn ((: n Int64)) #tuple(k n))) (export mk)))
  (call mk (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: (tuple 100 5) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "a closure returning a tuple with a negative int leaf"
  (doc
    "`(tuple n (- 0 n))` → `(: (tuple 5 -5) (Tuple Int64 Int64))`. The negative leaf flips the value
           form's kind byte to INT_NEG_DEC and writes the absolute magnitude (the escape's neg-int path).")
  (input (do (def (mk) (fn ((: n Int64)) #tuple(n (- 0 n)))) (export mk)))
  (call mk (: 5 Int64))
  (drop)
  (output (: (tuple 5 -5) (Tuple Int64 Int64)))
  (live-objects 0))

; DEEPER direct-call compound RESULT shapes (single-export): the value-form template / value-encode walker
; descends arbitrarily — a nested RECORD, a tuple containing a LIST (compound-with-collection), a SUM of a
; tuple, a LIST of tuples, and a compound ARG composing with a nested/compound RESULT all cross + decode.
(case
  "a closure returning a NESTED record crosses as the typed value form"
  (doc
    "`(record (a n) (b (record (c n+1) (d n+2))))` → the walker descends the nested record handle.
           `call(handle, 100)` → `(: (record (a 100) (b (record (c 101) (d 102)))) …)`.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) #record((= a n) (= b #record((= c (+ n 1)) (= d (+ n 2)))))))
      (export mk)))
  (call mk (: 100 Int64))
  (output
    (:
      (record (= a 100) (= b (record (= c 101) (= d 102))))
      (Record (: a Int64) (: b (Record (: c Int64) (: d Int64))))))
  (live-objects known-leak))

(case
  "a Tuple ARG composes with a NESTED-tuple RESULT"
  (doc
    "`mk : (-> (Tuple Int64 Int64) (Tuple Int64 (Tuple Int64 Int64)))` — a fixed-shape tuple ARG (rebuilt
           in-guest) feeding a nested-tuple RESULT (value-form-walked out). `call(handle, (10, 3))` → `(tuple
           10 (tuple 3 13))`.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 Int64))) #tuple((. p 0) #tuple((. p 1) (+ (. p 0) (. p 1))))))
      (export mk)))
  (call mk (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: (tuple 10 (tuple 3 13)) (Tuple Int64 (Tuple Int64 Int64))))
  (live-objects 0))

(case
  "a closure returning a tuple whose element is a LIST (compound-with-collection)"
  (doc
    "`mk : (-> Int64 (Tuple Int64 (List Int64)))` — a fixed-shape tuple with a VARIABLE-LENGTH list
           element. The value-encode walker (not a static template) renders it: `call(handle, 100)` → `(tuple
           100 (list 100 101))`.")
  (input (do (def (mk) (fn ((: n Int64)) #tuple(n #list(n (+ n 1))))) (export mk)))
  (call mk (: 100 Int64))
  (drop)
  (output (: #tuple(100 #list(100 101)) (Tuple Int64 (List Int64))))
  (live-objects 0))

(case
  "a NESTED-tuple ARG composes with a NESTED-tuple RESULT"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 Int64)) (Tuple Int64 (Tuple Int64 Int64)))` — a nested arg
           (recursively rebuilt) AND a nested result (value-form-walked). `call(handle, (100, (10, 3)))` →
           `(tuple 100 (tuple 10 3))`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 (Tuple Int64 Int64))))
          #tuple((. p 0) #tuple((. (. p 1) 0) (. (. p 1) 1)))))
      (export mk)))
  (call mk (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: (tuple 100 (tuple 10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (live-objects 0))

(case
  "a closure returning a SUM of a tuple (direct-call)"
  (doc
    "`mk : (-> Int64 (Option (Tuple Int64 Int64)))` — a sum whose payload is a compound. The value-encode
           walker renders the discriminant + the payload tuple. `call(handle, 100)` → `(Some (tuple 100 101))`.")
  (input (do (def (mk) (fn ((: n Int64)) (if (> n 0) (Some #tuple(n (+ n 1))) None))) (export mk)))
  (call mk (: 100 Int64))
  (drop)
  (output (: (Some #tuple(100 101)) (Option (Tuple Int64 Int64))))
  (live-objects 0))

(case
  "a closure returning a LIST of tuples (direct-call)"
  (doc
    "`mk : (-> Int64 (List (Tuple Int64 Int64)))` — a collection whose element is a compound. `call(handle,
           100)` → `(list (tuple 100 101) (tuple 102 103))`.")
  (input
    (do (def (mk) (fn ((: n Int64)) #list(#tuple(n (+ n 1)) #tuple((+ n 2) (+ n 3))))) (export mk)))
  (call mk (: 100 Int64))
  (drop)
  (output (: #list(#tuple(100 101) #tuple(102 103)) (List (Tuple Int64 Int64))))
  (live-objects 0))

; A COMPOUND (tuple/record) closure RESULT on the MULTI-EXPORT path — N same-signature closures each
; returning a tuple/record share ONE `call` that returns the value form as `list<u8>`. The shared `call`
; recovers each closure's code slot from the resource rep, dispatches it, and walks the returned compound
; handle into the ONE value-form template (all exports share the result type → one template). The host
; decodes each result to the typed `(: value T)` document. (Record fields render in CANONICAL sorted-name
; order — `hi` before `lo` — same as the single-export path and the value-heap escape.)
(case
  "multi-export compound result — the first closure's tuple"
  (doc
    "Two same-signature closures — `mkpair : () -> (-> Int64 (Tuple Int64 Int64))` returns `(tuple n
           n+1)`, `mkdbl` returns `(tuple n 2n)`. `call(mkpair-handle, 5)` walks its returned tuple → `(:
           (tuple 5 6) (Tuple Int64 Int64))`. Pins the compound value-form result on the shared-`call`
           multi-export path.")
  (input
    (do
      (def (mkpair) (fn ((: n Int64)) #tuple(n (+ n 1))))
      (def (mkdbl) (fn ((: n Int64)) #tuple(n (* n 2))))
      (export mkpair)
      (export mkdbl)))
  (call mkpair (: 5 Int64))
  (drop)
  (output (: (tuple 5 6) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "multi-export compound result — the second closure's tuple"
  (doc
    "The SAME two-closure program, driving the OTHER export: `call(mkdbl-handle, 5)` → `(tuple 5 10)`.
           Confirms the shared `call` dispatches whichever closure a handle names and walks ITS distinct
           result (the code slot rides in the rep, the value form is shared since the type is).")
  (input
    (do
      (def (mkpair) (fn ((: n Int64)) #tuple(n (+ n 1))))
      (def (mkdbl) (fn ((: n Int64)) #tuple(n (* n 2))))
      (export mkpair)
      (export mkdbl)))
  (call mkdbl (: 5 Int64))
  (drop)
  (output (: (tuple 5 10) (Tuple Int64 Int64)))
  (live-objects 0))

; The multi-export VALUE-FORM shared `call` (byte-rope/compound/collection — all cross as `list<u8>`) is a
; repeatable `borrow<t>` method too (C-HOST-6): one `make-<name>` handle serves repeated shared calls, each
; re-walking/re-encoding the value form (the host keeps the cell; the `t-dtor` reclaims). Repeatability is
; pinned by `a_multi_export_value_form_shared_borrow_call_is_repeatable` (one `make-lo` handle → the SAME
; `(tuple 5 6)` value form on two shared calls).
(case
  "a multi-export compound-result shared call is a repeatable (borrow<t>) callback"
  (doc
    "The SAME two-tuple-closure program witnessed as a borrow<t> value-form shared call: `make-mkpair()`
           → a handle the host keeps, the shared list-`call(5)` → `(: (tuple 5 6) (Tuple Int64 Int64))`. `call`
           borrows the handle (does NOT consume it), so the same handle serves repeated value-form calls
           (proven twice-over in the unit test).")
  (input
    (do
      (def (mkpair) (fn ((: n Int64)) #tuple(n (+ n 1))))
      (def (mkdbl) (fn ((: n Int64)) #tuple(n (* n 2))))
      (export mkpair)
      (export mkdbl)))
  (call mkpair (: 5 Int64))
  (drop)
  (output (: (tuple 5 6) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "multi-export record result — canonical field order"
  (doc
    "Two closures returning a `(Record (: lo Int64) (: hi Int64))`. `call(mka-handle, 3)` → `(record (lo 3)
           (hi 103))`, rendered in CANONICAL sorted-name order `(record (hi 103) (lo 3))`.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) #record((= lo n) (= hi (+ n 100)))))
      (def (mkb) (fn ((: n Int64)) #record((= lo (- 0 n)) (= hi n))))
      (export mka)
      (export mkb)))
  (call mka (: 3 Int64))
  (drop)
  (output (: (record (= hi 103) (= lo 3)) (Record (: hi Int64) (: lo Int64))))
  (live-objects 0))

(case
  "multi-export record result — the second closure, with a negative leaf"
  (doc
    "The SAME program's other export: `call(mkb-handle, 3)` → `(record (lo -3) (hi 3))` → canonical
           `(record (hi 3) (lo -3))`. The negative `lo` leaf flips its value form's kind byte.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) #record((= lo n) (= hi (+ n 100)))))
      (def (mkb) (fn ((: n Int64)) #record((= lo (- 0 n)) (= hi n))))
      (export mka)
      (export mkb)))
  (call mkb (: 3 Int64))
  (drop)
  (output (: (record (= hi 3) (= lo -3)) (Record (: hi Int64) (: lo Int64))))
  (live-objects 0))

(case
  "multi-export compound result — three capturing closures share one call"
  (doc
    "THREE same-signature closures (two capturing `k`, one not) each returning `(Tuple Int64 Int64)`.
           `b(7)` captures `k=7`; `call(b-handle, 2)` → `(tuple 2 7)`. Pins the shared value-form `call`
           dispatching among 3 closures, with captured values flowing into the compound result.")
  (input
    (do
      (def (a (: k Int64)) (fn ((: n Int64)) #tuple(k n)))
      (def (b (: k Int64)) (fn ((: n Int64)) #tuple(n k)))
      (def (c) (fn ((: n Int64)) #tuple(n n)))
      (export a)
      (export b)
      (export c)))
  (call b (: 7 Int64) (: 2 Int64))
  (drop)
  (output (: (tuple 2 7) (Tuple Int64 Int64)))
  (live-objects 0))

; A COMPOUND (tuple/record) closure RESULT on the MIXED path — a compound-returning closure exported
; ALONGSIDE a plain non-closure export. The closure crosses via the resource envelope (`make-<name>` + a
; shared `call` returning the value form as `list<u8>`); each plain export rides as an ordinary top-level
; component func. Same value-form core as the multi-export compound path, with the plain-export slots the
; mixed shape threads. The host decodes the closure result to `(: value T)`; a plain scalar renders directly.
(case
  "a tuple-returning closure alongside a plain export — the closure"
  (doc
    "`mk : () -> (-> Int64 (Tuple Int64 Int64))` returns `(tuple n n+1)`, alongside a plain `two : ()
           -> 2`. `call(mk-handle, 5)` walks the returned tuple → `(: (tuple 5 6) (Tuple Int64 Int64))`. Pins
           the compound value-form result on the MIXED path (closure + plain export).")
  (input
    (do (def (mk) (fn ((: n Int64)) #tuple(n (+ n 1)))) (def (two) 2) (export mk) (export two)))
  (call mk (: 5 Int64))
  (drop)
  (output (: (tuple 5 6) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "a tuple-returning closure alongside a plain export — the plain"
  (doc
    "The SAME mixed program, calling the plain `two` → 2 (a bare scalar, rendered directly — NOT a
           value-form document). Confirms the plain top-level export is reachable when a compound-result
           closure shares the component.")
  (input
    (do (def (mk) (fn ((: n Int64)) #tuple(n (+ n 1)))) (def (two) 2) (export mk) (export two)))
  (call two)
  (output (: 2 Int64)))

(case
  "a record-returning closure alongside a parameterized plain export — the closure"
  (doc
    "`mk : () -> (-> Int64 (Record (: a Int64) (: b Int64)))` returns `(record (a n) (b 2n))`, beside a
           parameterized plain `inc : (Int64) -> Int64`. `call(mk-handle, 4)` → `(: (record (a 4) (b 8))
           (Record (: a Int64) (: b Int64)))`.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) #record((= a n) (= b (* n 2)))))
      (def (inc (: x Int64)) (+ x 1))
      (export mk)
      (export inc)))
  (call mk (: 4 Int64))
  (drop)
  (output (: (record (= a 4) (= b 8)) (Record (: a Int64) (: b Int64))))
  (live-objects 0))

(case
  "a record-returning closure alongside a parameterized plain export — the plain"
  (doc
    "The SAME program, calling `inc(41)` = 42. Pins the parameterized plain export reachable beside a
           record-result closure.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) #record((= a n) (= b (* n 2)))))
      (def (inc (: x Int64)) (+ x 1))
      (export mk)
      (export inc)))
  (call inc (: 41 Int64))
  (output (: 42 Int64)))

; A COMPOUND (tuple/record) closure RESULT on the DISTINCT-SIG path — closures of DIFFERENT signatures each
; returning a fixed-shape compound cross as G distinct resource types, each with its OWN `call-g<n>`
; returning THAT group's value form as `list<u8>` (a PER-GROUP template, since the result types differ). A
; compound group, a byte-rope group, and a scalar group can all coexist in one component: compound templates
; occupy their own data-section regions, byte-rope groups write dynamically PAST them, scalars return by
; value — so the three list<u8>/scalar memory uses never collide.
(case
  "distinct-sig compound result — the Int64→(Tuple Int64 Int64) closure"
  (doc
    "`mki : () -> (-> Int64 (Tuple Int64 Int64))` and `mkb : () -> (-> Bool (Tuple Bool Int64))` are
           distinct signatures WITH distinct RESULT types → two resource types, each with its own value-form
           template. `call(mki-handle, 5)` walks its tuple → `(: (tuple 5 6) (Tuple Int64 Int64))`.")
  (input
    (do
      (def (mki) (fn ((: n Int64)) #tuple(n (+ n 1))))
      (def (mkb) (fn ((: b Bool)) #tuple(b (if b 1 0))))
      (export mki)
      (export mkb)))
  (call mki (: 5 Int64))
  (drop)
  (output (: (tuple 5 6) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "distinct-sig compound result — the Bool→(Tuple Bool Int64) closure"
  (doc
    "The SAME program's OTHER group, whose result type differs: `call(mkb-handle, true)` → `(: (tuple
           true 1) (Tuple Bool Int64))`. Confirms each distinct-sig group walks its OWN per-group template.")
  (input
    (do
      (def (mki) (fn ((: n Int64)) #tuple(n (+ n 1))))
      (def (mkb) (fn ((: b Bool)) #tuple(b (if b 1 0))))
      (export mki)
      (export mkb)))
  (call mkb (: true Bool))
  (drop)
  (output (: (tuple true 1) (Tuple Bool Int64)))
  (live-objects 0))

(case
  "distinct-sig: a compound group + a byte-rope group + a scalar group — the compound"
  (doc
    "THREE distinct signatures, THREE result MODES in one component: `mkt` returns a tuple (value
           form), `mkb` a `Bytes` (raw byte-rope), `inc` an Int64 (by value). `call(mkt-handle, 9)` → `(:
           (tuple 9 10) (Tuple Int64 Int64))`. Pins the disjoint-memory layout (compound template + byte-rope
           payload + scalar all coexisting).")
  (input
    (do
      (def (mkt) (fn ((: n Int64)) #tuple(n (+ n 1))))
      (def (mkb) (fn ((: b Bool)) (bin (u8 (if b 7 8)))))
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (export mkt)
      (export mkb)
      (export inc)))
  (call mkt (: 9 Int64))
  (drop)
  (output (: (tuple 9 10) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "distinct-sig: a compound group + a byte-rope group + a scalar group — the byte-rope"
  (doc
    "The SAME 3-mode program, driving the byte-rope group: `call(mkb-handle, false)` → `(8)` (a raw
           byte list, rendered bare — NOT a value-form document). Its payload is written PAST the compound
           template region.")
  (input
    (do
      (def (mkt) (fn ((: n Int64)) #tuple(n (+ n 1))))
      (def (mkb) (fn ((: b Bool)) (bin (u8 (if b 7 8)))))
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (export mkt)
      (export mkb)
      (export inc)))
  (call mkb (: false Bool))
  (drop)
  (output #list(8))
  (live-objects 0))

(case
  "distinct-sig: a compound group + a byte-rope group + a scalar group — the scalar"
  (doc
    "The SAME program's scalar group: `call(inc-handle, 41)` → 42 (returned by value, NOT list<u8>).
           Confirms the scalar `call-<g>` is unaffected by the sibling list-returning groups' memory.")
  (input
    (do
      (def (mkt) (fn ((: n Int64)) #tuple(n (+ n 1))))
      (def (mkb) (fn ((: b Bool)) (bin (u8 (if b 7 8)))))
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (export mkt)
      (export mkb)
      (export inc)))
  (call inc (: 41 Int64))
  (output (: 42 Int64))
  (live-objects 1))

; A COMPOUND (tuple/record) result on the ROUND-TRIP path — a consumer takes a produced closure back,
; applies it, and RETURNS a fixed-shape compound. The consumer crosses as `(own<t>, args…) -> list<u8>`
; carrying the value form (its own template, walked from the body's returned handle). Completes the compound
; result across ALL closure shapes. A compound consumer coexists with a scalar consumer, a byte-rope
; consumer of the same closure, and a plain export (disjoint memory: compound templates in the data section,
; byte-rope payloads written past them, scalars by value).
(case
  "round-trip: a consumer applies the handed-back closure and returns a tuple"
  (doc
    "`mk : () -> (-> Int64 Int64)` (adds 1); `app : (own<t>, Int64) -> (Tuple Int64 Int64)` returns
           `(tuple x (g x))`. Host produces via `mk`, hands the handle to `app(handle, 5)` → the closure
           yields 6, so the tuple is `(5, 6)`, decoded to `(: (tuple 5 6) (Tuple Int64 Int64))`. Pins the
           compound value-form result on the round-trip path.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case
  "round-trip: a consumer returns a record built from the closure result"
  (doc
    "`mk` doubles; `app : (own<t>, Int64) -> (Record (: inp Int64) (: out Int64))` = `(record (inp x) (out
           (g x)))`. `app(handle, 10)` → `(: (record (inp 10) (out 20)) …)`.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (* n 2)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #record((= inp x) (= out (g x))))
      (export mk)
      (export app)))
  (call app (: 10 Int64))
  (output (: (record (= inp 10) (= out 20)) (Record (: inp Int64) (: out Int64)))))

(case
  "round-trip: a scalar consumer + a compound consumer of the same closure — the compound"
  (doc
    "One closure signature, TWO consumers: `asnum` returns the value, `aspair` returns `(tuple x (g
           x))`. `aspair(handle, 8)` → `(: (tuple 8 9) (Tuple Int64 Int64))`. Pins a scalar consumer and a
           compound (value-form) consumer of the same resource coexisting.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (aspair (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (export mk)
      (export asnum)
      (export aspair)))
  (call aspair (: 8 Int64))
  (output (: (tuple 8 9) (Tuple Int64 Int64))))

(case
  "round-trip: a scalar consumer + a compound consumer of the same closure — the scalar"
  (doc
    "The SAME two-consumer program, driving the SCALAR consumer: `asnum(handle, 8)` → 9 (by value, NOT
           a value-form document). Confirms the scalar consumer is unaffected by the sibling compound
           consumer's memory/template.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (aspair (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (export mk)
      (export asnum)
      (export aspair)))
  (call asnum (: 8 Int64))
  (output (: 9 Int64)))

(case
  "round-trip: a compound consumer + a byte-rope consumer of the same closure — the compound"
  (doc
    "One signature, a COMPOUND consumer (`aspair` → tuple value form) AND a BYTE-ROPE consumer
           (`asbytes` → raw `list<u8>`). `aspair(handle, 3)` → `(: (tuple 3 4) …)`. Pins disjoint memory: the
           compound template region vs the byte-rope payload written past it.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (aspair (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (def (asbytes (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (export mk)
      (export aspair)
      (export asbytes)))
  (call aspair (: 3 Int64))
  (output (: (tuple 3 4) (Tuple Int64 Int64))))

(case
  "round-trip: a compound consumer + a byte-rope consumer of the same closure — the byte-rope"
  (doc
    "The SAME program, driving the byte-rope consumer: `asbytes(handle, 40)` → `(41)` (a raw byte
           list, its payload written PAST the compound template region — the two never collide).")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (aspair (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (def (asbytes (: g (-> Int64 Int64)) (: x Int64)) (bin (u8 (UInt8.wrap (g x)))))
      (export mk)
      (export aspair)
      (export asbytes)))
  (call asbytes (: 40 Int64))
  (output #list(41)))

(case
  "round-trip: a compound consumer alongside a plain export — the plain"
  (doc
    "A tuple-returning consumer `app` beside a plain `five : () -> 5`. Calling `five` → 5. Confirms a
           plain top-level export is reachable when a compound round-trip consumer shares the component.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (def (five) 5)
      (export mk)
      (export app)
      (export five)))
  (call five)
  (output (: 5 Int64)))

; A COMPOUND (tuple/record) result on the DISTINCT-SIG ROUND-TRIP path — the LAST fixed-shape compound-result
; gap. Producers/consumers of DIFFERENT signatures where a consumer RETURNS a fixed-shape compound: each
; consumer crosses as `(own<t_g>, args…) -> list<u8>` carrying the value form (its own per-consumer template).
; Fixed-shape compound results now work across EVERY closure shape. A compound consumer coexists with a
; scalar consumer, another compound consumer of a different sig, and a byte-rope consumer (disjoint memory:
; each compound template its own data region, byte-rope payloads written past them).
(case
  "distinct-sig round-trip: a compound consumer + a scalar consumer of another sig — the compound"
  (doc
    "`mka : () -> (-> Int64 Int64)`, `mkb : () -> (-> Bool Int64)` are distinct sigs → two resource
           types. `appa : (own<t0>, Int64) -> (Tuple Int64 Int64)` returns `(tuple x (g x))`. Host produces
           via `mka`, hands to `appa(handle, 5)` → `(: (tuple 5 6) (Tuple Int64 Int64))`. Pins the compound
           consumer result on the distinct-sig round-trip path.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64))))

(case
  "distinct-sig round-trip: a compound consumer + a scalar consumer of another sig — the scalar"
  (doc
    "The SAME two-resource-type program, driving the SCALAR consumer of the OTHER signature: `appb :
           (own<t1>, Bool) -> Int64` → `appb(handle, true)` = 10 (by value). Confirms the scalar consumer is
           unaffected by the sibling compound consumer's memory/template.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: true Bool))
  (output (: 10 Int64)))

(case
  "distinct-sig round-trip: TWO compound consumers of different sigs — the tuple one"
  (doc
    "Both consumers return a compound of DIFFERENT shape: `appa` a tuple, `appb` a record.
           `appa(mka-handle, 40)` → `(: (tuple 40 41) (Tuple Int64 Int64))`. Each consumer walks its OWN
           per-consumer value-form template.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) #record((= flag y) (= val (h y))))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 40 Int64))
  (output (: (tuple 40 41) (Tuple Int64 Int64))))

(case
  "distinct-sig round-trip: TWO compound consumers of different sigs — the record one"
  (doc
    "The SAME program's OTHER consumer: `appb(mkb-handle, true)` → `(: (record (flag true) (val 7))
           (Record (: flag Bool) (: val Int64)))`. Confirms each distinct-sig consumer decodes its own template.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) #record((= flag y) (= val (h y))))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: true Bool))
  (output (: (record (= flag true) (= val 7)) (Record (: flag Bool) (: val Int64)))))

(case
  "distinct-sig round-trip: a compound consumer + a byte-rope consumer of different sigs — the byte-rope"
  (doc
    "A COMPOUND consumer (`appa` → tuple value form) AND a BYTE-ROPE consumer (`appb` → raw list<u8>)
           of DISTINCT signatures. `appb(mkb-handle, false)` → `(8)` — its payload written PAST the compound
           template region (disjoint memory).")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #tuple(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (bin (u8 (UInt8.wrap (h y)))))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: false Bool))
  (output #list(8)))

; A VARIABLE-LENGTH collection (List/Map/Set) closure RESULT — the closure's `call` returns the canonical
; value form as `list<u8>`, rendered at RUN TIME by the runtime `value-encode(rep, desc)` op (the recursive-
; sum escape's "approach C") walking the returned collection handle against a compiler-baked shape
; DESCRIPTOR. Unlike a fixed-shape tuple/record (a static template), a collection is variable-length, so the
; runtime assembles the document; `lower::sum_shape_descriptor`'s List/Map/Set arm builds a parametric
; `Framed` descriptor so the element/key/value types are observable. The host decodes to `(: (list …) (List
; <e>))` / `(: (map (k v) …) (Map <k> <v>))` / `(: ((. Set of) (list …)) (Set <e>))`.
(case
  "a closure returning a List crosses as the value form"
  (doc
    "`mk : () -> (-> Int64 (List Int64))` returns `(list n n+1 n+2)`. `call(handle, 10)` dispatches the
           closure → the list handle, then `value-encode` renders `(: (list 10 11 12) (List Int64))`. Pins a
           VARIABLE-LENGTH collection result (no static template — the runtime walks the handle).")
  (input (do (def (mk) (fn ((: n Int64)) #list(n (+ n 1) (+ n 2)))) (export mk)))
  (call mk (: 10 Int64))
  (drop)
  (output (: #list(10 11 12) (List Int64)))
  (live-objects 0))

(case
  "a closure returning a Set — canonical member order"
  (doc
    "`(Set.of (list n n+1 n))` dedups to `{n, n+1}`; `call(handle, 5)` → `(: ((. Set of) (list 5 6))
           (Set Int64))`, members in canonical order (the runtime CHAMP set encode sorts).")
  (input (do (def (mk) (fn ((: n Int64)) #set(n (+ n 1) n))) (export mk)))
  (call mk (: 5 Int64))
  (drop)
  (output (: #set(5 6) (Set Int64)))
  (live-objects 0))

(case
  "a closure returning a Map — canonical key order"
  (doc
    "`(map (1 n) (2 n+1))` → `call(handle, 100)` → `(: (map (1 100) (2 101)) (Map Int64 Int64))`,
           entries in canonical key order.")
  (input (do (def (mk) (fn ((: n Int64)) #map((= 1 n) (= 2 (+ n 1))))) (export mk)))
  (call mk (: 100 Int64))
  (drop)
  (output (: #map((= 1 100) (= 2 101)) (Map Int64 Int64)))
  (live-objects 0))

(case
  "a closure returning a NESTED List"
  (doc
    "`(list (list n) (list n+1 n+2))` → `(: (list (list 7) (list 8 9)) (List (List Int64)))`. The shape
           descriptor's type node is recursive, so a nested collection element crosses; `value-encode`
           recurses over the inner lists.")
  (input (do (def (mk) (fn ((: n Int64)) #list(#list(n) #list((+ n 1) (+ n 2))))) (export mk)))
  (call mk (: 7 Int64))
  (drop)
  (output (: #list(#list(7) #list(8 9)) (List (List Int64))))
  (live-objects 0))

(case
  "a CAPTURING closure returning a List"
  (doc
    "`mk : (Int64) -> (-> Int64 (List Int64))` — `make(100)` captures `k=100`, then `call(handle, 5)` →
           `(: (list 100 5 105) (List Int64))`. Confirms a captured value flows into the collection result.")
  (input (do (def (mk (: k Int64)) (fn ((: n Int64)) #list(k n (+ k n)))) (export mk)))
  (call mk (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: #list(100 5 105) (List Int64)))
  (live-objects 0))

(case
  "a closure returning an EMPTY List"
  (doc
    "`(: (list) (List Int64))` → `call(handle, 0)` → `(: (list) (List Int64))` — the value-encode
           walker handles a zero-length collection (the empty document).")
  (input (do (def (mk) (fn ((: n Int64)) (: #list() (List Int64)))) (export mk)))
  (call mk (: 0 Int64))
  (drop)
  (output (: #list() (List Int64)))
  (live-objects 0))

; The capture cases above hold SCALARS (a captured k flowing into a heap RESULT); the host-supplied
; compound capture is a pinned DECLINE (host→guest decode, above). The unpinned middle: a closure
; capturing a GUEST-BUILT heap value — the capture cell holds a live heap HANDLE across the host
; boundary. `mk` builds the value by RECURSION (nothing folds), returns the closure, and the host
; calls it later: the captured spine must stay alive past mk's return (the capture-cell dup) and the
; boxed handle must be readable at call dispatch. Two shapes — an RRB list indexed per call, and a
; rope String measured per call. And a body-semantics face: a TRAP raised inside a host-called
; closure body (the file otherwise never traps a closure body).
(case
  "a closure capturing a RUNTIME-BUILT list indexes the captured spine at call time"
  (doc
    "`mk(3)` builds `[3,2,1]` by List.push recursion, captures it, and returns `(fn (i) (List.at
           xs i))`-with-expect. The host then calls the closure twice — `call(handle, 0)` → 3 and
           `call(handle, 2)` → 1 — so the captured spine is walked per call, not snapshot at make.
           Pins the capture-cell dup (the list outlives mk's activation) and the boxed-handle read at
           the shared call dispatch. Expected: 3 (i=0), 1 (i=2).")
  (input
    (do
      (def
        (build (: n Int64) (: acc (List Int64)))
        (if (= n 0) acc (build (- n 1) (List.push acc n))))
      (def
        (mk (: n Int64))
        (let ((xs (build n #list()))) (fn ((: i Int64)) (Option.expect (List.at xs i) "oob"))))
      (export mk)))
  (call mk (: 3 Int64) (: 0 Int64))
  (drop)
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a closure capturing a runtime String ROPE reads its byte length at call time"
  (doc
    "The rope twin: `mk(3)` builds \"abxxx\" by String.concat recursion (a genuine rope), captures
           it, and the closure returns `byte-len(s) + extra`. `call(handle, 100)` → 5 + 100 = 105. The
           captured heap value here is TEXT with a compactable rope rep — the capture must preserve the
           logical content across the boundary regardless of representation. Expected: 105.")
  (input
    (do
      (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s "x") (- n 1))))
      (def
        (mk (: n Int64))
        (let ((s (rep "ab" n))) (fn ((: extra Int64)) (+ (String.byte-len s) extra))))
      (export mk)))
  (call mk (: 3 Int64) (: 100 Int64))
  (drop)
  (output (: 105 Int64))
  (live-objects 0))

(case
  "a host-called closure capturing a HEAP LIST indexes it at call dispatch"
  (doc
    "The parameterized-make twin of the RUNTIME-BUILT-list capture above: `make(10)` builds
           `[10,11,12]` in the factory's let, the closure captures the spine, and the host's `call(handle,
           2)` reads element 2 through the capture cell → 12. Pins the capture-cell dup for a list built
           DIRECTLY in the factory body (the sibling case builds by recursion) — the minimal
           heap-capture-crosses-the-boundary shape.")
  (input
    (do
      (def
        (make (: k Int64))
        (let
          ((xs #list(k (+ k 1) (+ k 2))))
          (fn ((: i Int64)) (match (List.at xs i) ((Some v) v) ((None _u) -1)))))
      (export make)))
  (call make (: 10 Int64) (: 2 Int64))
  (drop)
  (output (: 12 Int64))
  (live-objects 0))

(case
  "a closure capturing a RUNTIME String.slice→to-bytes VIEW reads it at host-call dispatch"
  (doc
    "The consuming-op-family capture face (the adv-54 op family × the closure env): the factory
           slices a RUNTIME rope (the concat's second operand branches on the runtime k, so nothing
           folds), converts the slice view with String.to-bytes, and the returned closure captures the
           resulting Bytes. The host's
           `call(handle, 0)` reads byte 0 of \"cdefgh\" → 99 ('c'). Pins that a captured runtime
           slice/to-bytes result is evaluated ONCE into the capture cell and read back per call — the
           binding must be KEPT (adv-54's is_runtime_computation discipline) even when its single read
           is inside a lambda that escapes. The corpus's other guest-built captures use List.push spines
           and ropes; this is the borrowed-view-producing op family.")
  (input
    (do
      (def
        (mk (: k Int64))
        (let
          ((s (String.concat "abc" (if (> k 1000) "zzz" "defgh"))))
          (match
            (String.slice s k 6)
            ((Some t)
              (let
                ((b (String.to-bytes t)))
                (fn ((: i Int64)) (match (Bytes.at b i) ((Some v) (Int64.of v)) ((None _u) -1)))))
            ((None _u) (fn ((: _i Int64)) -2)))))
      (export mk)))
  (call mk (: 2 Int64) (: 0 Int64))
  (output (: 99 Int64))
  (live-objects known-leak))

(case
  "TWO closures capturing one let-bound host call share ONE firing (in the exported def)"
  (doc
    "The multi-capture sharing invariant at the host boundary: `(let ((v (io.get unit))) …)` binds
           ONE host response and BOTH closures capture the same `v` — the host call fires exactly ONCE
           (the recorded host-calls list is the assertion), then `(f 3) + 100·(g 3)` = (7+3) + 100·21 =
           2110. A per-closure re-fire would consume a second (unsupplied) response and trap. This is the
           in-exported-def face; the helper-def twin (adv-62) once double-fired but that fix has LANDED —
           both faces now share ONE firing. This pin is the over-rotation guard: in-body sharing must KEEP
           firing exactly once.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def
        (main (: k Int64))
        (host
          (io)
          (let
            ((v (io.get unit)))
            (match
              #tuple((fn ((: x Int64)) (+ v x)) (fn ((: x Int64)) (* v x)))
              (#tuple(f g) (+ (f k) (* 100 (g k))))))))
      (export main)))
  (host-responses (respond io.get (: 7 Int64)))
  (host-calls (call io.get))
  (call main (: 3 Int64))
  (output (: 2110 Int64)))

(case
  "TWO closures capturing one let-bound host call in a HELPER def share ONE firing"
  (doc
    "The HELPER-DEF face of the multi-capture host-boundary sharing invariant (adv-62, the twin the
           in-exported-def case above flagged as filed): a helper `mk` returns `(tuple (fn (x) (+ v x)) (fn
           (x) (* v x)))` from inside `(host (io) (let ((v (io.get unit))) …))`, and `main` destructures + calls
           both. `v` binds ONE host response captured by BOTH closures, so `io.get` fires EXACTLY ONCE (the
           host-calls list is the assertion) — a per-closure re-fire would consume a second (unsupplied)
           response and trap. io.get = 21: f(10) = 31 + g(100) = 2100 = 2131. The bug was `mk` β-inlining into
           the match scrutinee, folding to a Leaf that lost the inlined perform's effect-op meta, so the fold
           re-emitted the whole `(host …)` block once per tuple binder → io.get fired TWICE. Relocated from
           rcdzc adv62_a_let_bound_host_result_captured_by_two_escaping_closures_fires_the_host_op_once.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def
        (mk)
        (host
          (io)
          (let ((v (io.get unit))) #tuple((fn ((: x Int64)) (+ v x)) (fn ((: x Int64)) (* v x))))))
      (def (main) (match (mk) (#tuple(f g) (+ (f 10) (g 100)))))
      (export main)))
  (host-responses (respond io.get (: 21 Int64)))
  (host-calls (call io.get))
  (call main)
  (output (: 2131 Int64))
  (live-objects 0))

(case
  "TWO closures capturing one let-bound host call in a HELPER def's RECORD share ONE firing"
  (doc
    "The RECORD-face sibling of the helper-def tuple case above (adv-62b): a helper `mk` returns a
           RECORD of two closures capturing the let-bound host result `v`, from inside `(host (io) …)`; `main`
           projects `.f` and `.g` and calls both. `v` binds ONE host response captured by both, so `io.get`
           fires EXACTLY ONCE (the host-calls list is the assertion) — a per-projection re-fire consumes a
           second (unsupplied) response and traps. io.get = 21: (. r f)(10) = 31 + (. r g)(100) = 2100 = 2131.
           Relocated from rcdzc adv62b_a_host_result_captured_by_two_closures_in_a_record_fires_the_host_op_once.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def
        (mk)
        (host
          (io)
          (let
            ((v (io.get unit)))
            #record((= f (fn ((: x Int64)) (+ v x))) (= g (fn ((: x Int64)) (* v x)))))))
      (def (main) (let ((r (mk))) (+ (r.f 10) (r.g 100))))
      (export main)))
  (host-responses (respond io.get (: 21 Int64)))
  (host-calls (call io.get))
  (call main)
  (output (: 2131 Int64))
  (live-objects known-leak))

(case
  "a trap raised inside a host-called closure body reaches the host as a trap"
  (doc
    "`mk(100)` captures k=100 and returns `(fn (d) (/ k d))`; the host calls it with d = 0 and the
           division traps INSIDE the closure body — behind the resource `call` dispatch, not in a plain
           export body. The trap must surface to the host as a trap (not a wrong value, not a swallowed
           error). The file's other cases all return values; this pins the trap path through the closure
           call ABI. Expected: trap (integer divide by zero).")
  (input (do (def (mk (: k Int64)) (fn ((: d Int64)) (/ k d))) (export mk)))
  (call mk (: 100 Int64) (: 0 Int64))
  (trap "integer divide by zero"))

; A VARIABLE-LENGTH collection (List/Map/Set) closure RESULT on the MULTI-EXPORT path — N same-signature
; closures each returning a List/Map/Set share ONE `call` that value-encodes the returned handle against the
; ONE shared shape descriptor (all exports share the result type). The shared `call` recovers each closure's
; code slot from the resource rep, dispatches it, and `value-encode`s its collection result.
(case
  "multi-export collection result — the first list closure"
  (doc
    "Two same-signature closures — `up : () -> (-> Int64 (List Int64))` returns `(list n n+1)`, `dn`
           returns `(list n n-1)`. `call(up-handle, 5)` dispatches then value-encodes → `(: (list 5 6) (List
           Int64))`. Pins the variable-length collection result on the shared-`call` multi-export path.")
  (input
    (do
      (def (up) (fn ((: n Int64)) #list(n (+ n 1))))
      (def (dn) (fn ((: n Int64)) #list(n (- n 1))))
      (export up)
      (export dn)))
  (call up (: 5 Int64))
  (drop)
  (output (: #list(5 6) (List Int64)))
  (live-objects 0))

(case
  "multi-export collection result — the second list closure"
  (doc
    "The SAME two-closure program, driving the OTHER export: `call(dn-handle, 5)` → `(: (list 5 4)
           (List Int64))`. Confirms the shared `call` value-encodes whichever closure a handle names (the
           code slot rides in the rep, the descriptor is shared since the type is).")
  (input
    (do
      (def (up) (fn ((: n Int64)) #list(n (+ n 1))))
      (def (dn) (fn ((: n Int64)) #list(n (- n 1))))
      (export up)
      (export dn)))
  (call dn (: 5 Int64))
  (drop)
  (output (: #list(5 4) (List Int64)))
  (live-objects 0))

(case
  "multi-export Set-result closures — three sharing one call"
  (doc
    "THREE same-signature Set-returning closures share ONE value-encode `call`. `b(3)` builds `{3, 6}`;
           `call(b-handle, 3)` → `(: ((. Set of) (list 3 6)) (Set Int64))` in canonical member order.")
  (input
    (do
      (def (a) (fn ((: n Int64)) #set(n n (+ n 1))))
      (def (b) (fn ((: n Int64)) #set(n (* n 2))))
      (def (c) (fn ((: n Int64)) #set(n)))
      (export a)
      (export b)
      (export c)))
  (call b (: 3 Int64))
  (drop)
  (output (: #set(3 6) (Set Int64)))
  (live-objects 0))

(case
  "multi-export Set-result closures — the singleton one"
  (doc
    "The SAME three-closure program, driving `c`: `call(c-handle, 9)` → `(: ((. Set of) (list 9)) (Set
           Int64))`. Confirms each of the three shares the one descriptor + value-encodes its own result.")
  (input
    (do
      (def (a) (fn ((: n Int64)) #set(n n (+ n 1))))
      (def (b) (fn ((: n Int64)) #set(n (* n 2))))
      (def (c) (fn ((: n Int64)) #set(n)))
      (export a)
      (export b)
      (export c)))
  (call c (: 9 Int64))
  (drop)
  (output (: #set(9) (Set Int64)))
  (live-objects 0))

; A VARIABLE-LENGTH collection (List/Map/Set) closure RESULT on the MIXED path — a collection-returning
; closure exported ALONGSIDE a plain non-closure export. The closure crosses via the resource envelope
; (`make-<name>` + a shared value-encode `call` returning the value form as `list<u8>`); each plain export
; rides as an ordinary top-level component func. Same value-encode core as the multi-export collection path,
; with the plain-export slots the mixed shape threads.
(case
  "a List-returning closure alongside a plain export — the closure"
  (doc
    "`mk : () -> (-> Int64 (List Int64))` returns `(list n n+1)`, alongside a plain `two : () -> 2`.
           `call(mk-handle, 5)` value-encodes the returned list → `(: (list 5 6) (List Int64))`. Pins the
           variable-length collection result on the MIXED path (closure + plain export).")
  (input (do (def (mk) (fn ((: n Int64)) #list(n (+ n 1)))) (def (two) 2) (export mk) (export two)))
  (call mk (: 5 Int64))
  (drop)
  (output (: #list(5 6) (List Int64)))
  (live-objects 0))

(case
  "a List-returning closure alongside a plain export — the plain"
  (doc
    "The SAME mixed program, calling the plain `two` → 2 (a bare scalar, NOT a value-form document).
           Confirms the plain top-level export is reachable when a collection-result closure shares the
           component.")
  (input (do (def (mk) (fn ((: n Int64)) #list(n (+ n 1)))) (def (two) 2) (export mk) (export two)))
  (call two)
  (output (: 2 Int64)))

(case
  "a Map-returning closure alongside a parameterized plain export — the closure"
  (doc
    "`mk : () -> (-> Int64 (Map Int64 Int64))` returns `(map (1 n) (2 2n))`, beside a parameterized
           plain `inc : (Int64) -> Int64`. `call(mk-handle, 10)` → `(: (map (1 10) (2 20)) (Map Int64
           Int64))` in canonical key order.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) #map((= 1 n) (= 2 (* n 2)))))
      (def (inc (: x Int64)) (+ x 1))
      (export mk)
      (export inc)))
  (call mk (: 10 Int64))
  (drop)
  (output (: #map((= 1 10) (= 2 20)) (Map Int64 Int64)))
  (live-objects 0))

(case
  "a Map-returning closure alongside a parameterized plain export — the plain"
  (doc
    "The SAME program, calling `inc(41)` = 42. Pins the parameterized plain export reachable beside a
           Map-result closure.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) #map((= 1 n) (= 2 (* n 2)))))
      (def (inc (: x Int64)) (+ x 1))
      (export mk)
      (export inc)))
  (call inc (: 41 Int64))
  (output (: 42 Int64)))

; A VARIABLE-LENGTH collection (List/Map/Set) result on the DISTINCT-SIG path — closures of DIFFERENT
; signatures each returning a List/Map/Set cross as G distinct resource types, each `call-g<n>` value-encoding
; the returned handle against THAT group's shape descriptor. A collection group, a compound group, a byte-rope
; group, and a scalar group can all coexist in one component (compound templates in the data section;
; collection + byte-rope payloads written past them; scalars by value — none collide).
(case
  "distinct-sig collection result — the Int64→List closure"
  (doc
    "`mki : () -> (-> Int64 (List Int64))` returns `(list n n+1)`, `mkb : () -> (-> Bool (List Int64))`
           returns `(list (if b 1 0))` — distinct arg types → two resource types, each `call-g<n>` value-
           encoding its own result. `call(mki-handle, 5)` → `(: (list 5 6) (List Int64))`.")
  (input
    (do
      (def (mki) (fn ((: n Int64)) #list(n (+ n 1))))
      (def (mkb) (fn ((: b Bool)) #list((if b 1 0))))
      (export mki)
      (export mkb)))
  (call mki (: 5 Int64))
  (drop)
  (output (: #list(5 6) (List Int64)))
  (live-objects 0))

(case
  "distinct-sig collection result — the Bool→List closure"
  (doc
    "The SAME two-resource program, driving the OTHER signature: `call(mkb-handle, true)` → `(: (list
           1) (List Int64))`. Confirms each distinct-sig group value-encodes its own result.")
  (input
    (do
      (def (mki) (fn ((: n Int64)) #list(n (+ n 1))))
      (def (mkb) (fn ((: b Bool)) #list((if b 1 0))))
      (export mki)
      (export mkb)))
  (call mkb (: true Bool))
  (drop)
  (output (: #list(1) (List Int64)))
  (live-objects 0))

(case
  "distinct-sig: a collection + a compound + a byte-rope + a scalar group all coexist — the collection"
  (doc
    "FOUR distinct signatures, FOUR result MODES in one component: `lst` a List (value-encode), `pr` a
           tuple (fixed template), `byt` a Bytes (raw byte-rope), `inc` an Int64 (by value). `call(lst-handle,
           7)` → `(: (list 7 8) (List Int64))`. Pins the full disjoint-memory layout (compound template region
           + value-encode/byte-rope payloads past it + scalar-by-value all coexisting).")
  (input
    (do
      (def (lst) (fn ((: n Int64)) #list(n (+ n 1))))
      (def (pr) (fn ((: b Bool)) #tuple(b (if b 1 0))))
      (def (byt) (fn ((: x Int64)) (bin (u8 (UInt8.wrap x)))))
      (def (inc) (fn ((: y Int64)) (+ y 1)))
      (export lst)
      (export pr)
      (export byt)
      (export inc)))
  (call lst (: 7 Int64))
  (drop)
  (output (: #list(7 8) (List Int64)))
  (live-objects 0))

(case
  "distinct-sig: a collection + a compound + a byte-rope + a scalar group — the compound"
  (doc
    "The SAME 4-mode program, driving the COMPOUND group: `call(pr-handle, false)` → `(: (tuple false
           0) (Tuple Bool Int64))` (a fixed-shape template, distinct from the value-encoded collection).")
  (input
    (do
      (def (lst) (fn ((: n Int64)) #list(n (+ n 1))))
      (def (pr) (fn ((: b Bool)) #tuple(b (if b 1 0))))
      (def (byt) (fn ((: x Int64)) (bin (u8 (UInt8.wrap x)))))
      (def (inc) (fn ((: y Int64)) (+ y 1)))
      (export lst)
      (export pr)
      (export byt)
      (export inc)))
  (call pr (: false Bool))
  (drop)
  (output (: (tuple false 0) (Tuple Bool Int64)))
  (live-objects 0))

(case
  "distinct-sig: a collection + a compound + a byte-rope + a scalar group — the byte-rope"
  (doc
    "The SAME program's byte-rope group: `call(byt-handle, 65)` → `(65)` (a raw byte list, written past
           the compound template region).")
  (input
    (do
      (def (lst) (fn ((: n Int64)) #list(n (+ n 1))))
      (def (pr) (fn ((: b Bool)) #tuple(b (if b 1 0))))
      (def (byt) (fn ((: x Int64)) (bin (u8 (UInt8.wrap x)))))
      (def (inc) (fn ((: y Int64)) (+ y 1)))
      (export lst)
      (export pr)
      (export byt)
      (export inc)))
  (call byt (: 65 Int64))
  (drop)
  (output #list(65))
  (live-objects 0))

(case
  "distinct-sig: a collection + a compound + a byte-rope + a scalar group — the scalar"
  (doc
    "The SAME program's scalar group: `call(inc-handle, 41)` → 42 (by value, NOT list<u8>). Confirms
           the scalar `call-<g>` is unaffected by the three sibling list-returning groups.")
  (input
    (do
      (def (lst) (fn ((: n Int64)) #list(n (+ n 1))))
      (def (pr) (fn ((: b Bool)) #tuple(b (if b 1 0))))
      (def (byt) (fn ((: x Int64)) (bin (u8 (UInt8.wrap x)))))
      (def (inc) (fn ((: y Int64)) (+ y 1)))
      (export lst)
      (export pr)
      (export byt)
      (export inc)))
  (call inc (: 41 Int64))
  (output (: 42 Int64))
  (live-objects 1))

; A VARIABLE-LENGTH collection (List/Map/Set) result on the ROUND-TRIP path — a consumer takes a produced
; closure back, applies it, and RETURNS a List/Map/Set, value-encoded against its shape descriptor. This
; closes the collection-result surface across EVERY closure shape. A collection consumer coexists with a
; scalar consumer of the same closure.
(case
  "round-trip: a consumer applies the handed-back closure and returns a List"
  (doc
    "`mk : () -> (-> Int64 Int64)` (adds 1); `app : (own<t>, Int64) -> (List Int64)` returns `(list x (g
           x))`. Host produces via `mk`, hands to `app(handle, 5)` → the closure yields 6, so `value-encode`
           renders `(: (list 5 6) (List Int64))`. Pins the variable-length collection result on the round-trip
           path.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: #list(5 6) (List Int64))))

(case
  "round-trip: a consumer returns a Set built from the closure result"
  (doc
    "`mk` doubles; `app : (own<t>, Int64) -> (Set Int64)` = `(Set.of (list x (g x) x))`. `app(handle,
           3)` → `{3, 6}` → `(: ((. Set of) (list 3 6)) (Set Int64))` in canonical member order.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (* n 2)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #set(x (g x) x))
      (export mk)
      (export app)))
  (call app (: 3 Int64))
  (output (: #set(3 6) (Set Int64))))

(case
  "round-trip: a consumer returns a Map from the closure result"
  (doc
    "`mk` adds 100; `app : (own<t>, Int64) -> (Map Int64 Int64)` = `(map (0 x) (1 (g x)))`. `app(handle,
           5)` → `(: (map (0 5) (1 105)) (Map Int64 Int64))` in canonical key order.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 100)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #map((= 0 x) (= 1 (g x))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: #map((= 0 5) (= 1 105)) (Map Int64 Int64))))

(case
  "round-trip: a scalar consumer + a List consumer of the same closure — the list"
  (doc
    "One closure signature, TWO consumers: `asnum` returns the value, `aslist` returns `(list x (g x))`.
           `aslist(handle, 8)` → `(: (list 8 9) (List Int64))`. Pins a scalar consumer and a collection
           (value-encode) consumer of the same resource coexisting.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (aslist (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (export mk)
      (export asnum)
      (export aslist)))
  (call aslist (: 8 Int64))
  (output (: #list(8 9) (List Int64))))

(case
  "round-trip: a scalar consumer + a List consumer of the same closure — the scalar"
  (doc
    "The SAME two-consumer program, driving the SCALAR consumer: `asnum(handle, 8)` → 9 (by value, NOT
           a value-encoded document). Confirms the scalar consumer is unaffected by the sibling collection
           consumer's value-encode.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (asnum (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (def (aslist (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (export mk)
      (export asnum)
      (export aslist)))
  (call asnum (: 8 Int64))
  (output (: 9 Int64)))

; A VARIABLE-LENGTH collection (List/Map/Set) consumer RESULT on the DISTINCT-SIGNATURE ROUND-TRIP path —
; closures of DIFFERENT signatures each cross as their own resource type, and a consumer of one of them
; applies its handed-back closure and RETURNS a collection. That collection crosses as `list<u8>` rendered by
; the runtime `value-encode(rep, desc)` op against the consumer's OWN shape descriptor, written PAST all
; compound-template data (disjoint memory) — the last collection sub-shape. A collection consumer and a
; scalar/compound/byte-rope consumer of another signature coexist in one component.
(case
  "distinct-sig round-trip: a List consumer + a scalar consumer of another sig — the list"
  (doc
    "`mka : () -> (-> Int64 Int64)`, `mkb : () -> (-> Bool Int64)` are distinct sigs → two resource
           types. `appa : (own<t0>, Int64) -> (List Int64)` returns `(list x (g x))`. Host produces via `mka`,
           hands to `appa(handle, 5)` → the closure yields 6, so `value-encode` renders `(: (list 5 6) (List
           Int64))`. Pins the variable-length collection consumer result on the distinct-sig round-trip path.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 5 Int64))
  (output (: #list(5 6) (List Int64))))

(case
  "distinct-sig round-trip: a List consumer + a scalar consumer of another sig — the scalar"
  (doc
    "The SAME two-resource-type program, driving the SCALAR consumer of the OTHER signature: `appb :
           (own<t1>, Bool) -> Int64` → `appb(handle, true)` = 10 (by value, NOT a value-encoded document).
           Confirms the scalar consumer is unaffected by the sibling collection consumer's memory/value-encode.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: true Bool))
  (output (: 10 Int64)))

(case
  "distinct-sig round-trip: TWO collection consumers of different sigs — the List"
  (doc
    "Both consumers return a collection of DIFFERENT signature: `appa` a List, `appb` a Map.
           `appa(mka-handle, 40)` → `(: (list 40 41) (List Int64))`. Each consumer value-encodes against its
           OWN per-consumer shape descriptor.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) #map((= 0 (h y))))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 40 Int64))
  (output (: #list(40 41) (List Int64))))

(case
  "distinct-sig round-trip: TWO collection consumers of different sigs — the Map"
  (doc
    "The SAME program's OTHER consumer: `appb(mkb-handle, true)` → `(: (map (0 7)) (Map Int64 Int64))`.
           Confirms each distinct-sig consumer value-encodes its own descriptor.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) #map((= 0 (h y))))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: true Bool))
  (output (: #map((= 0 7)) (Map Int64 Int64))))

(case
  "distinct-sig round-trip: a List consumer + a compound consumer of another sig — the list"
  (doc
    "A COLLECTION consumer (`appa` → List, value-encode) AND a COMPOUND consumer (`appb` → tuple, static
           value-form template) of DISTINCT signatures coexist. `appa(mka-handle, 3)` → `(: (list 3 4) (List
           Int64))` — its value-encoded doc written PAST the sibling's compound template (disjoint memory).")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) #tuple(y (h y)))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 3 Int64))
  (output (: #list(3 4) (List Int64))))

(case
  "distinct-sig round-trip: a List consumer + a compound consumer of another sig — the compound"
  (doc
    "The SAME program's OTHER consumer: `appb(mkb-handle, false)` → `(: (tuple false 8) (Tuple Bool
           Int64))`. Confirms the compound consumer walks its own template while a sibling collection consumer
           value-encodes — three result-assembly mechanisms coexisting across two resource types.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 7 8) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) #list(x (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) #tuple(y (h y)))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: false Bool))
  (output (: (tuple false 8) (Tuple Bool Int64))))

; A COMPOUND closure ARGUMENT on the ROUND-TRIP path — the closure `g` takes a Tuple/Record/List/Map/Set. On
; the round-trip path the consumer APPLIES the handed-back closure ITSELF, in-guest (`(g <compound>)` inside
; the consumer body), so the closure's argument is BUILT in the guest and NEVER crosses the host boundary —
; only the closure HANDLE (an `own<t>` resource, i32) and the consumer's own scalar params cross. So a
; compound closure argument need only be MACHINE-representable (a value-heap handle, i32), not scalar-boundary.
; This lifts the earlier "a closure argument of type … has no scalar host-boundary representation" fence for
; the round trip. (A compound closure arg on the DIRECT-CALL path — where the HOST supplies the argument —
; still declines: that would need a host→guest decode of the compound into the guest heap.)
(case
  "round-trip: a consumer applies a closure taking a Tuple arg built in-guest"
  (doc
    "`mk : () -> (-> (Tuple Int64 Int64) Int64)` sums the pair; `app : (own<t>, Int64) -> Int64` applies
           the handed-back closure to a guest-built `(tuple x x)`. `app(handle, 5)` → `g((tuple 5 5))` = 10.
           Pins a COMPOUND (Tuple) closure argument crossing the round trip (built in-guest, never over the
           boundary).")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (def (app (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g #tuple(x x)))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: 10 Int64))
  (live-objects 1))

(case
  "round-trip: a consumer applies a closure taking a Record arg built in-guest"
  (doc
    "`mk : () -> (-> (Record (: a Int64) (: b Int64)) Int64)` multiplies the two fields; `app` applies it to
           a guest-built `(record (a x) (b x+1))`. `app(handle, 6)` → `g((record (a 6) (b 7)))` = 42. A RECORD
           closure argument crosses the round trip (field names are compile-time-only; the value is an i32
           heap handle in-guest).")
  (input
    (do
      (def (mk) (fn ((: r (Record (: a Int64) (: b Int64)))) (* r.a r.b)))
      (def
        (app (: g (-> (Record (: a Int64) (: b Int64)) Int64)) (: x Int64))
        (g #record((= a x) (= b (+ x 1)))))
      (export mk)
      (export app)))
  (call app (: 6 Int64))
  (output (: 42 Int64))
  (live-objects 1))

(case
  "round-trip: a consumer applies a closure taking a List arg built in-guest"
  (doc
    "`mk : () -> (-> (List Int64) Int64)` takes the list length; `app` applies it to a guest-built
           `(list x x x)`. `app(handle, 9)` → `g((list 9 9 9))` = `(. List len)` = 3. A VARIABLE-LENGTH
           collection closure argument crosses the round trip (an i32 persistent-vector handle in-guest).")
  (input
    (do
      (def (mk) (fn ((: xs (List Int64))) (List.len xs)))
      (def (app (: g (-> (List Int64) Int64)) (: x Int64)) (g #list(x x x)))
      (export mk)
      (export app)))
  (call app (: 9 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "round-trip: a compound-arg closure whose consumer returns a compound"
  (doc
    "The compound closure ARGUMENT and a compound consumer RESULT compose: `g : (-> (Tuple Int64 Int64)
           Int64)` returns the pair's first element; `app` returns `(tuple x (g (tuple x+1 x)))`.
           `app(handle, 7)` → `g((tuple 8 7))` = 8, so `(: (tuple 7 8) (Tuple Int64 Int64))`. A guest-built
           compound arg feeds the closure, and the consumer's own compound result is value-form-encoded out.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64))) (. p 0)))
      (def (app (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) #tuple(x (g #tuple((+ x 1) x))))
      (export mk)
      (export app)))
  (call app (: 7 Int64))
  (drop)
  (output (: (tuple 7 8) (Tuple Int64 Int64)))
  (live-objects 0))

; The SAME compound-closure-argument relaxation applies to the DISTINCT-SIGNATURE round-trip — closures of
; different signatures each cross as their own resource type, and each is applied in-guest by its consumer, so
; a compound argument is built guest-side and never crosses the boundary. Only the closure signature's fence
; is widened (machine-representable rather than scalar-boundary); the per-group resource machinery is unchanged.
(case
  "distinct-sig round-trip: a compound-arg closure + a scalar-arg closure of another sig — the compound-arg one"
  (doc
    "`mka : () -> (-> (Tuple Int64 Int64) Int64)`, `mkb : () -> (-> Bool Int64)` are distinct sigs → two
           resource types. `appa : (own<t0>, Int64) -> Int64` applies its handed-back closure to a guest-built
           `(tuple x x)`. `appa(handle, 5)` → `g((tuple 5 5))` = 10. Pins a COMPOUND closure argument on the
           distinct-sig round-trip path (built in-guest, one of two resource types).")
  (input
    (do
      (def (mka) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
      (def (appa (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g #tuple(x x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 5 Int64))
  (output (: 10 Int64))
  (live-objects 1))

(case
  "distinct-sig round-trip: a compound-arg closure + a scalar-arg closure of another sig — the scalar-arg one"
  (doc
    "The SAME two-resource-type program, driving the OTHER (scalar-arg) closure of the other signature:
           `appb : (own<t1>, Bool) -> Int64` → `appb(handle, true)` = 10. Confirms the scalar-arg group is
           unaffected by the sibling compound-arg group.")
  (input
    (do
      (def (mka) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (def (mkb) (fn ((: b Bool)) (: (if b 10 20) Int64)))
      (def (appa (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g #tuple(x x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: true Bool))
  (output (: 10 Int64)))

(case
  "distinct-sig round-trip: TWO compound-arg closures of different sigs — the Tuple-arg one"
  (doc
    "Both closures take a DIFFERENT compound: `g` a Tuple, `h` a Record → two resource types.
           `appa : (own<t0>, Int64) -> Int64` applies `g` to `(tuple x+1 x)`. `appa(handle, 7)` →
           `g((tuple 8 7))` = 8-7 = 1. Each group's closure takes its own compound argument built in-guest.")
  (input
    (do
      (def (mka) (fn ((: p (Tuple Int64 Int64))) (- (. p 0) (. p 1))))
      (def (mkb) (fn ((: r (Record (: a Int64) (: b Int64)))) (* r.a r.b)))
      (def (appa (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g #tuple((+ x 1) x)))
      (def
        (appb (: h (-> (Record (: a Int64) (: b Int64)) Int64)) (: y Int64))
        (h #record((= a y) (= b y))))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 7 Int64))
  (output (: 1 Int64))
  (live-objects 1))

(case
  "distinct-sig round-trip: TWO compound-arg closures of different sigs — the Record-arg one"
  (doc
    "The SAME program's OTHER closure: `appb : (own<t1>, Int64) -> Int64` applies `h` to a guest-built
           `(record (a y) (b y))`. `appb(handle, 6)` → `h((record (a 6) (b 6)))` = 36. Confirms each distinct
           signature threads its own compound argument through its own resource type.")
  (input
    (do
      (def (mka) (fn ((: p (Tuple Int64 Int64))) (- (. p 0) (. p 1))))
      (def (mkb) (fn ((: r (Record (: a Int64) (: b Int64)))) (* r.a r.b)))
      (def (appa (: g (-> (Tuple Int64 Int64) Int64)) (: x Int64)) (g #tuple((+ x 1) x)))
      (def
        (appb (: h (-> (Record (: a Int64) (: b Int64)) Int64)) (: y Int64))
        (h #record((= a y) (= b y))))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: 6 Int64))
  (output (: 36 Int64))
  (live-objects 1))

; The in-guest-argument relaxation reaches every MACHINE-representable argument, not just fixed-shape
; compounds: a SUM (Option/Result), a NESTED compound, a String/Bytes, and — most notably — a closure-TYPED
; argument all cross the round trip, because each is built in the guest and only the outer closure HANDLE
; travels. A HIGHER-ORDER closure (`(-> (-> A B) R)`) handed back and applied to a guest-built inner closure
; needs NO extra resource machinery: the inner closure is an ordinary in-guest funcref-table value (an i32
; slot, `valtype_of(Ty::Fn)`), applied by the outer via the usual `call_indirect`.
(case
  "round-trip: a closure taking a SUM (Option) arg built in-guest"
  (doc
    "`mk : () -> (-> (Option Int64) Int64)` unwraps with a default; `app` applies it to a guest-built
           `(Some x)`. `app(handle, 7)` → `g((Some 7))` = 7. A SUM closure argument crosses the round trip (an
           i32 sum handle in-guest).")
  (input
    (do
      (def (mk) (fn ((: o (Option Int64))) (match o ((Some v) v) (None 0))))
      (def (app (: g (-> (Option Int64) Int64)) (: x Int64)) (g (Some x)))
      (export mk)
      (export app)))
  (call app (: 7 Int64))
  (output (: 7 Int64))
  (live-objects 1))

(case
  "round-trip: a closure taking a NESTED compound (Tuple of Tuples) arg"
  (doc
    "`mk`'s closure reads `(. (. p 0) 0) + (. p 1)`; `app` applies it to a guest-built
           `(tuple (tuple x x) x)`. `app(handle, 5)` → `g((tuple (tuple 5 5) 5))` = 5 + 5 = 10. A NESTED
           compound argument crosses (still one i32 handle at the top).")
  (input
    (do
      (def (mk) (fn ((: p (Tuple (Tuple Int64 Int64) Int64))) (+ (. (. p 0) 0) (. p 1))))
      (def
        (app (: g (-> (Tuple (Tuple Int64 Int64) Int64) Int64)) (: x Int64))
        (g #tuple(#tuple(x x) x)))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))

(case
  "round-trip: a closure taking a String arg built in-guest"
  (doc
    "`mk`'s closure takes the byte length of a String; `app` applies it to a guest-built literal
           `\"hello\"`. `app(handle, 0)` → `g(\"hello\")` = 5. A byte-rope (String) closure argument crosses the
           round trip (an i32 rope handle in-guest).")
  (input
    (do
      (def (mk) (fn ((: s String)) (String.byte-len s)))
      (def (app (: g (-> String Int64)) (: x Int64)) (g "hello"))
      (export mk)
      (export app)))
  (call app (: 0 Int64))
  (output (: 5 Int64))
  (live-objects 1))

(case
  "round-trip: a HIGHER-ORDER closure — its argument is itself a closure built in-guest"
  (doc
    "`mk : () -> (-> (-> Int64 Int64) Int64)` applies its function argument to 10; `app` hands it a
           guest-built capturing closure `(fn (y) (+ y x))`. `app(handle, 5)` → `g((fn y -> y+5))` = 15. A
           CLOSURE-TYPED argument crosses the round trip with NO extra resource machinery: the inner closure
           is an ordinary in-guest funcref-table value (an i32 slot), applied by the outer via
           `call_indirect`. Only the OUTER closure handle crosses the host boundary.")
  (input
    (do
      (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
      (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (+ y x))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: 15 Int64))
  (live-objects 1))

(case
  "round-trip: a higher-order closure whose inner closure CAPTURES and is applied twice"
  (doc
    "`mk`'s closure applies its function arg to BOTH 10 and 20 and sums; `app` hands in a guest-built
           capturing `(fn (y) (* y x))`. `app(handle, 3)` → `g((fn y -> y*3))` = 3*10 + 3*20 = 90. Stresses a
           captured, MULTIPLY-APPLIED inner closure — a wrong funcref slot would give a wrong value.")
  (input
    (do
      (def (mk) (fn ((: f (-> Int64 Int64))) (+ (f 10) (f 20))))
      (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (* y x))))
      (export mk)
      (export app)))
  (call app (: 3 Int64))
  (output (: 90 Int64))
  (live-objects 1))

(case
  "round-trip: a higher-order closure applied to TWO distinct inner closures"
  (doc
    "`mk`'s closure applies its function arg to 100; `app` calls the handed-back `g` on TWO different
           guest-built inner closures — `(fn y -> y+x)` and `(fn y -> y*x)` — and sums the results.
           `app(handle, 4)` → `g((fn y->y+4)) + g((fn y->y*4))` = (100+4) + (100*4) = 104 + 400 = 504. Confirms
           two distinct inner closures are NOT crossed (each resolves its own funcref slot).")
  (input
    (do
      (def (mk) (fn ((: f (-> Int64 Int64))) (f 100)))
      (def
        (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64))
        (+ (g (fn (y) (+ y x))) (g (fn (y) (* y x)))))
      (export mk)
      (export app)))
  (call app (: 4 Int64))
  (output (: 504 Int64))
  (live-objects known-leak))

(case
  "distinct-sig round-trip: a higher-order closure + a scalar closure of another sig — the higher-order one"
  (doc
    "`mka : () -> (-> (-> Int64 Int64) Int64)` (applies its function arg to 1 and 2, sums) and
           `mkb : () -> (-> Bool Int64)` are distinct sigs → two resource types. `appa` hands `g` a guest-built
           `(fn (y) (* y x))`. `appa(handle, 5)` → `g((fn y->y*5))` = 5*1 + 5*2 = 15. A closure-typed argument
           on the DISTINCT-SIG round-trip path.")
  (input
    (do
      (def (mka) (fn ((: f (-> Int64 Int64))) (+ (f 1) (f 2))))
      (def (mkb) (fn ((: b Bool)) (: (if b 100 200) Int64)))
      (def (appa (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (* y x))))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 5 Int64))
  (output (: 15 Int64))
  (live-objects 1))

(case
  "a fixed-shape scalar Tuple closure ARG crosses the DIRECT-CALL boundary (host supplies the tuple)"
  (doc
    "A single closure export whose closure takes a `(Tuple Int64 Int64)`, called DIRECTLY by the host
           (no consumer to apply it in-guest). This USED to decline (recorded as needing a nonexistent
           `value-decode` runtime op / out of scope), but that conflated two cases: a FIXED-SHAPE SCALAR
           tuple does NOT need runtime decode. It crosses as a native component `tuple<s64,s64>` type, which
           the canonical ABI FLATTENS into scalar core params; the guest `call` wrapper rebuilds the tuple
           cell in-guest from the flat fields with the ORDINARY `arr-alloc`/`box-int`/`arr-set` ops (the
           `TupleArgRebuild` serializer path), then dispatches `call_indirect`. `make()` → the closure
           handle; `call(handle, (3, 4))` → `(. p 0) + (. p 1)` = 7. Proved by the
           `a_fixed_shape_tuple_closure_arg_crosses_by_native_flattening` oracle + the real emit pipeline.
           (A VARIABLE-LENGTH collection arg genuinely still needs runtime decode — out of scope.)")
  (input (do (def (mk) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1)))) (export mk)))
  (call mk (: #tuple(3 4) (Tuple Int64 Int64)))
  (output (: 7 Int64)))

(case
  "a fixed-shape scalar RECORD closure ARG crosses the DIRECT-CALL boundary"
  (doc
    "Like the tuple-arg case but the closure argument is a RECORD `(Record (: a Int64) (: b Int64))`. A
           record of aliased-width scalars flattens the same way (its fields in canonical SORTED-key order —
           the value-heap cell's field order), so the guest `call` rebuilds the record cell from the flat
           fields. `call(handle, (record 3 4))` → `(. p a) + (. p b)` = 7. (The corpus arg is the value form
           in field order; the `record` head token is dropped by the runner's tuple-literal parser.)")
  (input (do (def (mk) (fn ((: p (Record (: a Int64) (: b Int64)))) (+ p.a p.b))) (export mk)))
  (call mk (: #record(3 4) (Record (: a Int64) (: b Int64))))
  (output (: 7 Int64)))

(case
  "a NARROW-int-field Tuple closure ARG flattens + rebuilds (exercises the i32->i64 extend)"
  (doc
    "A `(Tuple Int32 Int32)` closure arg: each field crosses as a component `s32` (an i32 core param),
           so the cell rebuild SIGN-EXTENDS each field i32→i64 before `box-int` (the value-heap cell holds
           i64-boxed ints). Distinct from the Int64 case, which needs no extend. `call(handle, (100, 23))`
           → 123, proving the narrow-field extend path in `TupleArgRebuild`.")
  (input (do (def (mk) (fn ((: p (Tuple Int32 Int32))) (+ (. p 0) (. p 1)))) (export mk)))
  (call mk (: #tuple(100 23) (Tuple Int32 Int32)))
  (output (: 123 Int32)))

(case
  "a BOOL-field Tuple closure ARG flattens + rebuilds (box-bool imported)"
  (doc
    "A `(Tuple Int32 Bool)` closure arg — a Bool field beside an int. Each field crosses flattened; the
           cell rebuild boxes the Bool field with `box-bool` (the int with `box-int`). This exercises a box op
           OTHER than `box-int`: the `TupleArgRebuild` `field_box_ops` list names `box-bool` for the Bool
           field, and the closure `call`'s import-collection pass must register it — an all-int tuple worked
           only because `box-int` was pulled in elsewhere, so a Bool/Float field's box op was ABSENT from the
           import index and `emit_tuple_rebuild`'s `imp(bop)` PANICKED the compiler ('rebuild op imported',
           serialize.rs). Now imports are keyed off `field_box_ops`, so every field-type mix compiles.
           `call(handle, (42, true))` → `(if (. p 1) (. p 0) 0)` = 42.")
  (input (do (def (mk) (fn ((: p (Tuple Int32 Bool))) (if (. p 1) (. p 0) 0))) (export mk)))
  (call mk (: #tuple(42 true) (Tuple Int32 Bool)))
  (output (: 42 Int32)))

(case
  "a BOOL-field Tuple closure ARG, false discriminant"
  (doc
    "The discriminant control for the box-bool case above: with the flag false, `(if (. p 1) (. p 0) 0)`
           takes the else arm → 0. Together they prove the rebuilt Bool field carries its real value across the
           boundary (not a constant), on the same `(Tuple Int32 Bool)` closure arg.")
  (input (do (def (mk) (fn ((: p (Tuple Int32 Bool))) (if (. p 1) (. p 0) 0))) (export mk)))
  (call mk (: #tuple(42 false) (Tuple Int32 Bool)))
  (output (: 0 Int32)))

(case
  "a FLOAT-field Tuple closure ARG flattens + rebuilds (box-float imported)"
  (doc
    "A `(Tuple Float64 Float64)` closure arg — two Float fields. The cell rebuild boxes each with
           `box-float` (a native-width float box, not `box-int`), so the closure `call`'s imports must include
           `box-float`; without it the compiler PANICKED exactly as the Bool case. `call(handle, (2.5, 9.0))`
           → `(. p 0)` = 2.5. Pins the Float-field box op alongside the Bool one.")
  (input (do (def (mk) (fn ((: p (Tuple Float64 Float64))) (. p 0))) (export mk)))
  (call mk (: #tuple(2.5 9.0) (Tuple Float64 Float64)))
  (output (: 2.5 Float64)))

(case
  "a MIXED Int-and-Float Tuple closure ARG flattens + rebuilds (box-int + box-float)"
  (doc
    "A `(Tuple Int64 Float64)` closure arg mixes an int field (`box-int`) with a float field
           (`box-float`) in ONE rebuild — the import set must carry BOTH box ops. Before the fix the float
           field's `box-float` was absent and the compiler panicked. `call(handle, (7, 3.5))` → `(. p 0)` = 7.
           Confirms a per-field mix of box ops all resolve.")
  (input (do (def (mk) (fn ((: p (Tuple Int64 Float64))) (. p 0))) (export mk)))
  (call mk (: #tuple(7 3.5) (Tuple Int64 Float64)))
  (output (: 7 Int64)))

(case
  "a CAPTURING closure taking a Tuple ARG crosses the DIRECT-CALL boundary"
  (doc
    "The tuple-arg path composes with capture (C-HOST-2): a parameterized export `(def (mk (: k
           Int64)) …)` returns a closure that BOTH captures `k` AND takes a `(Tuple Int64 Int64)` argument.
           `make(10)` → a handle closing over k=10; `call(handle, (3, 4))` → `(. p 0) + (. p 1) + k` = 17.
           The make-forwarded capture cell and the rebuilt arg cell coexist in the one `call`.")
  (input
    (do
      (def (mk (: k Int64)) (fn ((: p (Tuple Int64 Int64))) (+ (+ (. p 0) (. p 1)) k)))
      (export mk)))
  (call mk (: 10 Int64) (: #tuple(3 4) (Tuple Int64 Int64)))
  (output (: 17 Int64)))

(case
  "MULTI-EXPORT: two same-sig Tuple-arg closures share one direct-call `call`"
  (doc
    "The direct-call fixed-shape compound-arg path extends to the MULTI-EXPORT shape: N same-signature
           closures (`mk-sum`, `mk-diff`, both `(-> (Tuple Int64 Int64) Int64)`) cross as N `make-<name>`s
           sharing ONE `call` whose single argument is a native component `tuple<s64,s64>` — the shared `call`
           rebuilds the tuple cell from the flattened fields (the same `TupleArgRebuild` the single-export
           path uses), dispatched through the guest's funcref table by the handle's resource rep. The host
           `make-diff()` → a handle, `call(handle, (10, 3))` → `(. p 0) - (. p 1)` = 7. The envelope mints
           the `tuple<…>` defined type in the SHARED `call` functype (outer lift + nested re-export).")
  (input
    (do
      (def (mk-sum) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (def (mk-diff) (fn ((: p (Tuple Int64 Int64))) (- (. p 0) (. p 1))))
      (export mk-sum)
      (export mk-diff)))
  (call mk-diff (: #tuple(10 3) (Tuple Int64 Int64)))
  (output (: 7 Int64)))

(case
  "MIXED: a Tuple-arg closure export ALONGSIDE a plain (non-closure) export"
  (doc
    "The direct-call fixed-shape compound-arg path extends to the MIXED shape: a tuple-arg closure
           factory `mk : (-> (Tuple Int64 Int64) Int64)` crosses via the resource envelope's `make`+shared
           `call` (the `call` takes a native `tuple<s64,s64>` rebuilt from the flattened fields) WHILE a
           plain (non-closure) export `twice` rides alongside as an ordinary top-level component func. Both
           coexist in one component. Driving the CLOSURE: `make()` → handle, `call(handle, (3, 4))` → 7.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call mk (: #tuple(3 4) (Tuple Int64 Int64)))
  (output (: 7 Int64)))

(case
  "MIXED: driving the PLAIN export alongside a Tuple-arg closure"
  (doc
    "The SAME mixed component as above, but the trial drives the PLAIN export `twice` (an ordinary
           top-level func) — proving it coexists with the tuple-arg closure interface and is reachable by
           name. `twice(21)` → 42. Companion to the closure-driving trial above.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call twice (: 21 Int64))
  (output (: 42 Int64)))

(case
  "DISTINCT-SIG: two Tuple-arg closures of DIFFERENT signatures each cross the direct-call boundary"
  (doc
    "The direct-call fixed-shape compound-arg path extends to the DISTINCT-SIGNATURE shape: two
           closures taking the SAME `(Tuple Int64 Int64)` arg but returning DIFFERENT types (`mk-sum` → Int64,
           `mk-eq` → Bool) cross as TWO resource types, each with its own `make-<name>` + `call-g<n>`. Each
           group's `call-g<n>` takes a native `tuple<s64,s64>` rebuilt from the flattened fields (per-group
           `TupleArgRebuild`). Driving the Int64 group: `make-sum()` → handle, `call(handle, (3, 4))` → 7.
           (The Bool group is exercised by the companion trial.)")
  (input
    (do
      (def (mk-sum) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (def (mk-eq) (fn ((: p (Tuple Int64 Int64))) (= (. p 0) (. p 1))))
      (export mk-sum)
      (export mk-eq)))
  (call mk-sum (: #tuple(3 4) (Tuple Int64 Int64)))
  (output (: 7 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: driving the Bool-returning Tuple-arg closure of the distinct-sig pair"
  (doc
    "The SAME distinct-sig component, driving the Bool group `mk-eq : (-> (Tuple Int64 Int64) Bool)` —
           its own resource type + `call-g<n>` taking a `tuple<s64,s64>`. `make-eq()` → handle,
           `call(handle, (5, 5))` → `(= (. p 0) (. p 1))` = true. Companion to the Int64-group trial above.")
  (input
    (do
      (def (mk-sum) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (def (mk-eq) (fn ((: p (Tuple Int64 Int64))) (= (. p 0) (. p 1))))
      (export mk-sum)
      (export mk-eq)))
  (call mk-eq (: #tuple(5 5) (Tuple Int64 Int64)))
  (output (: true Bool))
  (live-objects 1))

; The tuple-AMONG-scalars arg shape now works on the DISTINCT-SIGNATURE path too — the LAST direct-call
; arg-position gap — for EVERY result shape. `emit_distinct_sig_resource` detects each group's arg via
; `single_compound_among_scalars`; every per-group `call-g<n>` body pushes prefix scalars, the rebuilt tuple,
; and suffix scalars (via the shared `emit_closure_call_args`); the per-group envelope functypes interleave the
; scalar boundary bytes around the `tuple<…>` type. Groups of DIFFERENT signatures (incl. a narrow Bool field)
; each keep their own resource type.
(case
  "DISTINCT-SIG among-scalars: two scalar-then-Tuple closures of DIFFERENT sigs — driving the Int64 group"
  (doc
    "`mk-a : (-> Int64 (Tuple Int64 Int64) Int64)` and `mk-b : (-> Int64 (Tuple Int64 Bool) Int64)` — a
           scalar `n` then a tuple, of DIFFERENT signatures (Int64-pair vs Int64/Bool tuple), each its own
           resource type + `call-g<n>` interleaving `n` around the rebuilt tuple. `make-a()` → handle,
           `call(handle, 100, (10, 3))` → `n + p.0 + p.1` = 113.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (+ n (+ (. p 0) (. p 1)))))
      (def (mk-b) (fn ((: n Int64) (: q (Tuple Int64 Bool))) (. q 0)))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (output (: 113 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG among-scalars: driving the Int64/Bool-tuple group (Bool field in the rebuild)"
  (doc
    "The SAME distinct-sig component, driving `mk-b : (-> Int64 (Tuple Int64 Bool) Int64)` — its arg
           tuple has a NARROW Bool field (boxed via `box-bool` in the rebuild), the scalar `n` a prefix.
           `make-b()` → handle, `call(handle, 5, (7, true))` → `(. q 0)` = 7.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (+ n (+ (. p 0) (. p 1)))))
      (def (mk-b) (fn ((: n Int64) (: q (Tuple Int64 Bool))) (. q 0)))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: 5 Int64) (: #tuple(7 true) (Tuple Int64 Bool)))
  (output (: 7 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG among-scalars: two scalar-then-Tuple closures of DIFFERENT sigs each returning a LIST"
  (doc
    "`mk-a : (-> Int64 (Tuple Int64 Int64) (List Int64))` and `mk-b : (-> Int64 (Tuple Int64 Bool) (List
           Int64))` — a scalar then a tuple, DIFFERENT sigs, each its own resource type + list-returning
           `call-g<n>` that interleaves the scalar around the rebuilt tuple then value-encodes the returned
           List. `make-a()` → handle, `call(handle, 100, (10, 3))` → `(list 100 10 3)`.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #list(n (. p 0) (. p 1))))
      (def (mk-b) (fn ((: n Int64) (: q (Tuple Int64 Bool))) #list((. q 0) n)))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

(case
  "DISTINCT-SIG among-scalars: driving the Int64/Bool-tuple LIST group (tuple then suffix scalar)"
  (doc
    "The SAME distinct-sig List component, driving `mk-b` — the tuple field FIRST then the suffix scalar
           `n`: `call(handle, 100, (7, true))` → `(list (. q 0) n)` = `(list 7 100)`. Confirms the interleaving
           handles a Bool field + a suffix scalar per distinct-sig group's list-`call-g`.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #list(n (. p 0) (. p 1))))
      (def (mk-b) (fn ((: n Int64) (: q (Tuple Int64 Bool))) #list((. q 0) n)))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: 100 Int64) (: #tuple(7 true) (Tuple Int64 Bool)))
  (drop)
  (output (: #list(7 100) (List Int64)))
  (live-objects 0))

; A fixed-shape compound ARGUMENT now composes with a BYTE-ROPE (`Bytes`/`String`) result: the bytes-result
; core serializer + its envelope thread the `TupleArgRebuild`, so the `call` rebuilds the flattened tuple cell
; then copies its byte-rope result out as `list<u8>`. (A COMPOUND value-form or a variable-length COLLECTION
; result combined with a tuple arg still declines — those two cores don't yet thread the rebuild; see the
; decline anchor below.)
(case
  "a fixed-shape Tuple ARG with a Bytes RESULT crosses the direct-call boundary"
  (doc
    "`(fn (p) (bin (u8 (. p 0)) (u8 (. p 1))))` — a `(Tuple Int64 Int64)` argument AND a `Bytes` result.
           The tuple crosses flattened as a native `tuple<s64,s64>` the `call` rebuilds in-guest; the closure's
           `Bytes` result copies out as `list<u8>`. `make()` → handle, `call(handle, (5, 6))` → the two bytes
           `(5 6)`. Proves the tuple-arg rebuild threads through the byte-rope-result core + envelope.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 Int64))) (bin (u8 (UInt8.wrap (. p 0))) (u8 (UInt8.wrap (. p 1))))))
      (export mk)))
  (call mk (: #tuple(5 6) (Tuple Int64 Int64)))
  (drop)
  (output #list(5 6))
  (live-objects 0))

; A fixed-shape compound ARGUMENT now composes with a fixed-shape COMPOUND result too (the value-form result
; core + the shared list<u8> envelope thread the `TupleArgRebuild`): the `call` rebuilds the flattened tuple
; arg cell, dispatches, then walks the closure's returned compound handle into the value-form template. Only a
; VARIABLE-LENGTH COLLECTION result (List/Map/Set, the value-encode core) combined with a tuple arg still
; declines — see the anchor below.
(case
  "a fixed-shape Tuple ARG with a Tuple RESULT crosses the direct-call boundary"
  (doc
    "`(fn (p) (tuple (+ (. p 0) (. p 1)) (- (. p 0) (. p 1))))` — a `(Tuple Int64 Int64)` argument AND a
           `(Tuple Int64 Int64)` result. The arg crosses flattened as a native `tuple<s64,s64>` the `call`
           rebuilds in-guest; the closure's returned tuple is walked into the value-form template + crosses as
           `list<u8>`, decoded to the typed `(: value T)`. `make()` → handle, `call(handle, (10, 3))` →
           `(tuple 13 7)`. Proves the tuple-arg rebuild threads through the value-form (fixed compound) result
           core + envelope.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64))) #tuple((+ (. p 0) (. p 1)) (- (. p 0) (. p 1)))))
      (export mk)))
  (call mk (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: (tuple 13 7) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "a fixed-shape Tuple ARG with a RECORD RESULT crosses the direct-call boundary"
  (doc
    "Like the tuple-result case but the closure returns a RECORD `(Record (: sum Int64) (: diff Int64))` —
           a fixed-shape compound rendered via the value-form template (fields in canonical sorted-key order).
           `call(handle, (10, 3))` → `(record (diff 7) (sum 13))`. Confirms a record result rides the same
           value-form tuple-arg path.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 Int64)))
          #record((= sum (+ (. p 0) (. p 1))) (= diff (- (. p 0) (. p 1))))))
      (export mk)))
  (call mk (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: (record (= diff 7) (= sum 13)) (Record (: diff Int64) (: sum Int64))))
  (live-objects 0))

; A fixed-shape compound ARGUMENT now composes with a VARIABLE-LENGTH COLLECTION result (List/Map/Set) too —
; the value-encode result core + the shared list<u8> envelope thread the `TupleArgRebuild`. So a single-export
; tuple-arg closure now works with EVERY result shape: scalar, byte-rope, fixed-shape compound, AND
; variable-length collection. The `call` rebuilds the flattened tuple arg cell, dispatches, then renders the
; returned collection via the runtime `value-encode(rep, desc)` op. (The genuinely-unsupported arg shape is a
; compound arg with a VARIABLE-LENGTH FIELD — no fixed flattened form — which declines at arg detection.)
(case
  "a fixed-shape Tuple ARG with a List RESULT crosses the direct-call boundary"
  (doc
    "`(fn (p) (list (. p 0) (. p 1)))` — a `(Tuple Int64 Int64)` argument AND a `(List Int64)` result.
           The arg crosses flattened as a native `tuple<s64,s64>` the `call` rebuilds in-guest; the returned
           List renders at run time via `value-encode(rep, desc)`, crossing as `list<u8>`. `make()` → handle,
           `call(handle, (10, 3))` → `(list 10 3)`. The last single-export list-result core threaded — a
           tuple-arg closure now composes with EVERY result shape.")
  (input (do (def (mk) (fn ((: p (Tuple Int64 Int64))) #list((. p 0) (. p 1)))) (export mk)))
  (call mk (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(10 3) (List Int64)))
  (live-objects 0))

(case
  "a fixed-shape Tuple ARG with a Map RESULT crosses the direct-call boundary"
  (doc
    "A tuple arg + a `(Map Int64 Int64)` result — a variable-length collection rendered via
           `value-encode`, keyed in canonical sorted-key order. `(fn (p) (Map.insert (Map.insert (map) (. p 0)
           100) (. p 1) 200))` with `(1, 2)` → `(map (1 100) (2 200))`. Confirms Map rides the same tuple-arg
           value-encode path as List.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 Int64))) (Map.insert (Map.insert #map() (. p 0) 100) (. p 1) 200)))
      (export mk)))
  (call mk (: #tuple(1 2) (Tuple Int64 Int64)))
  (drop)
  (output (: #map((= 1 100) (= 2 200)) (Map Int64 Int64)))
  (live-objects 0))

(case
  "a fixed-shape Tuple ARG with a STRING RESULT crosses the direct-call boundary"
  (doc
    "A tuple arg + a `String` result — the byte-rope-result core copies the UTF-8 bytes out as
           `list<u8>` AFTER rebuilding the flattened tuple arg. `(fn (p) (if (= p.0 p.1) \"eq\" \"ne\"))` with
           `(5, 5)` → `\"eq\"` = the raw bytes `(101 113)`. Confirms the tuple-arg rebuild threads through the
           byte-rope `call` for a String result (representationally identical to Bytes).")
  (input
    (do (def (mk) (fn ((: p (Tuple Int64 Int64))) (if (= (. p 0) (. p 1)) "eq" "ne"))) (export mk)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)))
  (drop)
  (output #list(101 113))
  (live-objects 0))

(case
  "a fixed-shape Tuple ARG with a SUM (Option) RESULT crosses the direct-call boundary"
  (doc
    "A tuple arg + an `(Option Int64)` result — a SUM crosses as `list<u8>` rendered by the runtime
           `value-encode(rep, desc)` walker (the shape descriptor covers Option). `(fn (p) (if (= p.0 p.1)
           (Some p.0) None))` with `(5, 5)` → `(Some 5)`. Confirms a tuple arg composes with the sum
           value-encode result path (distinct from the fixed-compound static-template path).")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64))) (if (= (. p 0) (. p 1)) (Some (. p 0)) None)))
      (export mk)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)))
  (drop)
  (output (: (: (Some 5) (Option Int64)) (Option Int64)))
  (live-objects 0))

; The tuple-arg × list-result composition extends to the MULTI-EXPORT shape: N same-signature closures sharing
; ONE list-returning `call` each rebuild the flattened tuple arg cell (the multi bytes/value-form/value-encode
; cores + the shared multi list<u8> envelope thread the `TupleArgRebuild`).
(case
  "MULTI-EXPORT: two Tuple-arg closures sharing a List-returning `call`"
  (doc
    "Two same-sig `(-> (Tuple Int64 Int64) (List Int64))` closures (`mk-fwd`, `mk-rev`) share one
           value-encode `call` that rebuilds the flattened tuple arg. Driving `mk-rev`: `make-rev()` → handle,
           `call(handle, (10, 3))` → `(list 3 10)`. The multi value-encode core + the shared multi list<u8>
           envelope thread the tuple rebuild.")
  (input
    (do
      (def (mk-fwd) (fn ((: p (Tuple Int64 Int64))) #list((. p 0) (. p 1))))
      (def (mk-rev) (fn ((: p (Tuple Int64 Int64))) #list((. p 1) (. p 0))))
      (export mk-fwd)
      (export mk-rev)))
  (call mk-rev (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(3 10) (List Int64)))
  (live-objects 0))

(case
  "MULTI-EXPORT: two Tuple-arg closures sharing a Tuple-returning `call`"
  (doc
    "The same multi-export shape with a fixed-shape COMPOUND (value-form) result: `(-> (Tuple Int64
           Int64) (Tuple Int64 Int64))`. Driving `mk-sum`: `call(handle, (10, 3))` → `(tuple 13 7)`. The multi
           value-form core threads the tuple-arg rebuild.")
  (input
    (do
      (def
        (mk-sum)
        (fn ((: p (Tuple Int64 Int64))) #tuple((+ (. p 0) (. p 1)) (- (. p 0) (. p 1)))))
      (def (mk-prod) (fn ((: p (Tuple Int64 Int64))) #tuple((* (. p 0) (. p 1)) (. p 0))))
      (export mk-sum)
      (export mk-prod)))
  (call mk-sum (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: (tuple 13 7) (Tuple Int64 Int64)))
  (live-objects 0))

; The tuple-arg × list-result composition extends to the MIXED shape: a List-returning tuple-arg closure
; ALONGSIDE a plain (non-closure) export. The shared multi list-result core + the shared multi list<u8> tuple
; envelope thread the `TupleArgRebuild`; the plain export rides alongside as an ordinary top-level func.
(case
  "MIXED: a List-returning Tuple-arg closure ALONGSIDE a plain export — driving the closure"
  (doc
    "`mk : (-> (Tuple Int64 Int64) (List Int64))` (a tuple-arg closure returning a collection) crosses
           via make + shared value-encode `call` that rebuilds the flattened tuple arg, WHILE a plain `twice`
           rides alongside as a top-level func. Driving the closure: `make()` → handle, `call(handle, (10, 3))`
           → `(list 10 3)`.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64))) #list((. p 0) (. p 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call mk (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(10 3) (List Int64)))
  (live-objects 0))

(case
  "MIXED: driving the PLAIN export alongside a List-returning Tuple-arg closure"
  (doc
    "The SAME mixed component, driving the plain `twice` — proving it coexists with the tuple-arg
           list-returning closure interface. `twice(21)` → 42.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64))) #list((. p 0) (. p 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call twice (: 21 Int64))
  (output (: 42 Int64)))

; The tuple-AMONG-scalars arg shape now works on the MIXED path too, for EVERY result shape: a closure taking a
; tuple among scalar args, exported ALONGSIDE a plain non-closure export. `emit_mixed_closure_resource` uses
; `single_compound_among_scalars` (like multi-export), threading prefix/suffix scalar bytes into the shared
; scalar/list `call` functype; the plain export rides alongside as a top-level func.
(case
  "MIXED among-scalars: a scalar-then-Tuple closure ALONGSIDE a plain export — driving the closure"
  (doc
    "`mk : (-> Int64 (Tuple Int64 Int64) Int64)` — a scalar `n` then a tuple `p`, exported beside a plain
           `two`. The shared `call` interleaves `n` around the rebuilt tuple. `make()` → handle, `call(handle,
           100, (10, 3))` → `n + p.0 + p.1` = 113.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (+ n (+ (. p 0) (. p 1)))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call mk (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (output (: 113 Int64)))

(case
  "MIXED among-scalars: driving the PLAIN export alongside a scalar-then-Tuple closure"
  (doc
    "The SAME mixed component, driving the plain `two` — it coexists with the among-scalars tuple-arg
           closure interface. `two()` → 2.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (+ n (+ (. p 0) (. p 1)))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call two)
  (output (: 2 Int64)))

(case
  "MIXED among-scalars: a scalar-then-Tuple closure with a LIST result ALONGSIDE a plain export"
  (doc
    "`mk : (-> Int64 (Tuple Int64 Int64) (List Int64))` — a scalar then a tuple, returning a List, beside
           a plain `two`. The shared value-encode `call` interleaves `n` around the rebuilt tuple, value-encodes
           the returned List. `call(handle, 100, (10, 3))` → `(list 100 10 3)`.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #list(n (. p 0) (. p 1))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call mk (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

(case
  "MIXED among-scalars: a scalar-then-Tuple closure with a COMPOUND result ALONGSIDE a plain export"
  (doc
    "`mk : (-> Int64 (Tuple Int64 Int64) (Tuple Int64 Int64 Int64))` — a scalar then a tuple, returning a
           fixed-shape tuple, beside a plain `two`. The shared value-form `call` interleaves `n` around the
           rebuilt arg tuple, walks the returned handle into the template. `call(handle, 100, (10, 3))` →
           `(tuple 100 10 3)`.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #tuple(n (. p 0) (. p 1))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call mk (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: (tuple 100 10 3) (Tuple Int64 Int64 Int64)))
  (live-objects 0))

; The tuple-arg × list-result composition extends to the DISTINCT-SIGNATURE shape — the LAST list-result gap:
; closures of DIFFERENT signatures each taking a fixed-shape scalar tuple arg AND returning a list<u8>-crossing
; result (byte-rope / fixed-compound / collection) cross as G resource types, each per-group `call-g<n>`
; rebuilding ITS OWN flattened tuple arg. All four per-group `call-g` body branches + the per-group envelope
; functypes now thread the `TupleArgRebuild`.
(case
  "DISTINCT-SIG: two DIFFERENT-signature Tuple-arg closures each returning a List"
  (doc
    "`mk-a : (-> (Tuple Int64 Int64) (List Int64))` and `mk-b : (-> (Tuple Int64 Bool) (List Int64))` —
           DIFFERENT tuple-arg signatures, each returning a collection. They cross as TWO resource types, each
           `call-g<n>` rebuilding its own flattened tuple arg then value-encoding the returned List. Driving
           the Int64-tuple group: `make-a()` → handle, `call(handle, (10, 3))` → `(list 10 3)`.")
  (input
    (do
      (def (mk-a) (fn ((: p (Tuple Int64 Int64))) #list((. p 0) (. p 1))))
      (def (mk-b) (fn ((: p (Tuple Int64 Bool))) #list((. p 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(10 3) (List Int64)))
  (live-objects 0))

(case
  "DISTINCT-SIG: driving the (Tuple Int64 Bool)-arg closure of the distinct-sig List pair"
  (doc
    "The SAME distinct-sig component, driving `mk-b : (-> (Tuple Int64 Bool) (List Int64))` — its arg
           tuple has a NARROW Bool field (boxed via `box-bool` in the rebuild). `make-b()` → handle,
           `call(handle, (7, true))` → `(list 7)`. Exercises a distinct tuple-arg shape per group + a Bool
           field in the flattened-tuple rebuild.")
  (input
    (do
      (def (mk-a) (fn ((: p (Tuple Int64 Int64))) #list((. p 0) (. p 1))))
      (def (mk-b) (fn ((: p (Tuple Int64 Bool))) #list((. p 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: #tuple(7 true) (Tuple Int64 Bool)))
  (drop)
  (output (: #list(7) (List Int64)))
  (live-objects 0))

; A fixed-shape scalar tuple ARGUMENT can now sit AMONG scalar args (single-export, scalar result): the tuple
; crosses flattened as a native `tuple<…>` at its own arg position, and the `call` pushes the closure's args in
; ORDER — prefix scalars, the rebuilt tuple cell, suffix scalars — with the tuple's flattened fields starting
; at core param `1 + prefix-count` (`TupleArgRebuild.base_param`). The tuple may be at any position.
(case
  "a Tuple ARG AFTER a scalar arg crosses the direct-call boundary (scalar, then tuple)"
  (doc
    "`(fn (n) (p)) : (-> Int64 (Tuple Int64 Int64) Int64)` — a scalar arg `n` THEN a tuple arg `p`. The
           `call` receives `[n, p.0, p.1]` flattened; it pushes `n`, rebuilds the tuple from params 2..4
           (base_param=2), dispatches. `call(handle, 100, (10, 3))` → `n + p.0 + p.1` = 113. The tuple sits
           after the scalar — a prefix scalar + the rebuilt tuple.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (+ n (+ (. p 0) (. p 1)))))
      (export mk)))
  (call mk (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (output (: 113 Int64)))

(case
  "a Tuple ARG BEFORE a scalar arg crosses the direct-call boundary (tuple, then scalar)"
  (doc
    "`(fn (p) (n)) : (-> (Tuple Int64 Int64) Int64 Int64)` — the tuple arg `p` FIRST (base_param=1), then
           a SUFFIX scalar `n`. The `call` rebuilds the tuple from params 1..3, then pushes `n` (param 3).
           `call(handle, (10, 3), 100)` → `p.0 + p.1 + n` = 113. Confirms the tuple + a suffix scalar.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: n Int64)) (+ (+ (. p 0) (. p 1)) n)))
      (export mk)))
  (call mk (: #tuple(10 3) (Tuple Int64 Int64)) (: 100 Int64))
  (output (: 113 Int64)))

(case
  "a Tuple ARG BETWEEN two scalar args crosses the direct-call boundary (scalar, tuple, scalar)"
  (doc
    "`(fn (a) (p) (b)) : (-> Int64 (Tuple Int64 Int64) Int64 Int64)` — a PREFIX scalar `a`, the tuple `p`
           (base_param=2), and a SUFFIX scalar `b`. The `call` pushes `a`, rebuilds `p` from params 2..4,
           pushes `b` (param 4). `call(handle, 1, (10, 3), 100)` → `a + p.0 + p.1 + b` = 114. The tuple at an
           interior position between prefix + suffix scalars.")
  (input
    (do
      (def
        (mk)
        (fn ((: a Int64) (: p (Tuple Int64 Int64)) (: b Int64)) (+ (+ a (+ (. p 0) (. p 1))) b)))
      (export mk)))
  (call mk (: 1 Int64) (: #tuple(10 3) (Tuple Int64 Int64)) (: 100 Int64))
  (output (: 114 Int64)))

; The tuple-among-scalars arg shape now composes with EVERY result shape on the SINGLE-export path: the shared
; `emit_closure_call_args` helper threads prefix scalars, the rebuilt tuple, and suffix scalars into the SCALAR
; `call` body AND the three list-result cores (byte-rope / fixed-shape compound value-form / collection
; value-encode) alike, and the `call` functype interleaves the scalar boundary bytes around the `tuple<…>` type.
(case
  "a Tuple ARG among scalars with a LIST result crosses the direct-call boundary"
  (doc
    "`(fn (n) (p)) : (-> Int64 (Tuple Int64 Int64) (List Int64))` — a prefix scalar `n` then a tuple `p`,
           returning a variable-length List. The list-result `call` rebuilds the tuple from params 2..4, pushes
           `n` before it, dispatches, then value-encodes the returned List handle. `call(handle, 100, (10, 3))`
           → `(list 100 10 3)`. The among-scalars interleaving now reaches the list-result cores.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #list(n (. p 0) (. p 1))))
      (export mk)))
  (call mk (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

(case
  "a Tuple ARG among scalars with a BYTE-ROPE result crosses the direct-call boundary"
  (doc
    "`(fn (n) (p)) : (-> Int64 (Tuple Int64 Int64) Bytes)` — a prefix scalar then a tuple, returning a
           byte rope. The bytes `call` interleaves `n` around the rebuilt tuple, dispatches, copies the returned
           Bytes out as `list<u8>`. `call(handle, 100, (10, 3))` → the bytes `(100 10 3)`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: n Int64) (: p (Tuple Int64 Int64)))
          (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (. p 0))) (u8 (UInt8.wrap (. p 1))))))
      (export mk)))
  (call mk (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output #list(100 10 3))
  (live-objects 0))

(case
  "a Tuple ARG among scalars with a fixed-shape COMPOUND result crosses the direct-call boundary"
  (doc
    "`(fn (n) (p)) : (-> Int64 (Tuple Int64 Int64) (Tuple Int64 Int64 Int64))` — a prefix scalar then a
           tuple, returning a fixed-shape tuple. The value-form `call` interleaves `n` around the rebuilt arg
           tuple, dispatches, walks the returned handle into the value-form template. `call(handle, 100, (10,
           3))` → `(tuple 100 10 3)`, decoded by the host to the typed document.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #tuple(n (. p 0) (. p 1))))
      (export mk)))
  (call mk (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: (tuple 100 10 3) (Tuple Int64 Int64 Int64)))
  (live-objects 0))

(case
  "a Tuple ARG BEFORE a scalar with a LIST result crosses the direct-call boundary"
  (doc
    "`(fn (p) (n)) : (-> (Tuple Int64 Int64) Int64 (List Int64))` — the tuple FIRST (base_param=1) then a
           SUFFIX scalar, returning a List. The list-result `call` rebuilds the tuple from params 1..3, pushes
           the suffix scalar `n` (param 3), dispatches. `call(handle, (10, 3), 100)` → `(list 10 3 100)`.
           Confirms the interleaving handles a suffix scalar on the list-result path too.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: n Int64)) #list((. p 0) (. p 1) n)))
      (export mk)))
  (call mk (: #tuple(10 3) (Tuple Int64 Int64)) (: 100 Int64))
  (drop)
  (output (: #list(10 3 100) (List Int64)))
  (live-objects 0))

; A RECORD closure argument crosses the direct-call boundary just like a tuple: it erases to a component
; `tuple<…>` whose fields are laid in canonical SORTED-NAME order (`tuple_field_abi` / `Core::Record` use a
; `BTreeMap`), which the canonical ABI flattens into scalar core params the `call` rebuilds. The host supplies
; it as `(record (name value)…)`; cdz-run sorts the named fields to match the boundary tuple's positions.
(case
  "a RECORD closure ARG among scalars crosses the direct-call boundary (scalar result)"
  (doc
    "`mk : (-> Int64 (Record (: x Int64) (: y Int64)) Int64)` — a scalar `n` then a RECORD `r`. The record
           erases to a `tuple<s64,s64>` (fields sorted: x, y), flattened into core params the `call` rebuilds
           into the cell. `call(handle, 100, (record (x 10) (y 3)))` → `n + r.x + r.y` = 113.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: r (Record (: x Int64) (: y Int64)))) (+ n (+ r.x r.y))))
      (export mk)))
  (call mk (: 100 Int64) (: #record((= x 10) (= y 3)) (Record (: x Int64) (: y Int64))))
  (output (: 113 Int64)))

(case
  "a SOLE RECORD closure ARG crosses the direct-call boundary"
  (doc
    "`mk : (-> (Record (: a Int64) (: b Int64)) Int64)` — the record is the SOLE arg (base_param=1). Erases
           to `tuple<s64,s64>`, rebuilt in the `call`. `call(handle, (record (a 10) (b 3)))` → `r.a + r.b` = 13.")
  (input (do (def (mk) (fn ((: r (Record (: a Int64) (: b Int64)))) (+ r.a r.b))) (export mk)))
  (call mk (: #record((= a 10) (= b 3)) (Record (: a Int64) (: b Int64))))
  (output (: 13 Int64)))

(case
  "a RECORD closure ARG whose fields are NOT in sorted source order"
  (doc
    "`mk : (-> (Record (: z Int64) (: a Int64)) Int64)` — fields declared `z` THEN `a`, but the boundary
           tuple sorts them to `(a, z)`. The guest reads `. r z`/`. r a` by name (correct regardless of layout);
           cdz-run sorts the corpus's `(z 100)(a 3)` fields to match. `call(handle, (record (z 100) (a 3)))` →
           `r.z - r.a` = 97 (proving the sorted-field round-trip is sound, not a coincidental positional match).")
  (input (do (def (mk) (fn ((: r (Record (: z Int64) (: a Int64)))) (- r.z r.a))) (export mk)))
  (call mk (: #record((= z 100) (= a 3)) (Record (: z Int64) (: a Int64))))
  (output (: 97 Int64)))

(case
  "a RECORD closure ARG with a narrow Bool field, among scalars"
  (doc
    "`mk : (-> Int64 (Record (: v Int64) (: flag Bool)) Int64)` — the record has a NARROW Bool field (sorted
           BEFORE `v`: flag, v). The rebuild boxes the Bool via `box-bool`. `call(handle, 100, (record (v 10)
           (flag true)))` → `if r.flag then n + r.v else n` = 110.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: r (Record (: v Int64) (: flag Bool)))) (if r.flag (+ n r.v) n)))
      (export mk)))
  (call mk (: 100 Int64) (: #record((= v 10) (= flag true)) (Record (: v Int64) (: flag Bool))))
  (output (: 110 Int64)))

(case
  "a RECORD closure ARG with a LIST result"
  (doc
    "`mk : (-> Int64 (Record (: x Int64) (: y Int64)) (List Int64))` — a record arg composes with a
           collection result: the list-`call` rebuilds the record cell, dispatches, value-encodes the List.
           `call(handle, 100, (record (x 10) (y 3)))` → `(list 100 10 3)`.")
  (input
    (do
      (def (mk) (fn ((: n Int64) (: r (Record (: x Int64) (: y Int64)))) #list(n r.x r.y)))
      (export mk)))
  (call mk (: 100 Int64) (: #record((= x 10) (= y 3)) (Record (: x Int64) (: y Int64))))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

(case
  "a RECORD closure ARG on the MULTI-EXPORT path"
  (doc
    "Two same-sig record-arg closures `(-> (Record (: x Int64) (: y Int64)) Int64)` share ONE `call` that
           rebuilds the record cell. Driving `mk-b` (subtract): `call(handle, (record (x 10) (y 3)))` → 7.")
  (input
    (do
      (def (mk-a) (fn ((: r (Record (: x Int64) (: y Int64)))) (+ r.x r.y)))
      (def (mk-b) (fn ((: r (Record (: x Int64) (: y Int64)))) (- r.x r.y)))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: #record((= x 10) (= y 3)) (Record (: x Int64) (: y Int64))))
  (output (: 7 Int64)))

; A NESTED fixed-shape compound ARG — a tuple/record whose FIELD is itself a tuple/record — crosses the
; direct-call boundary (single-export, scalar result): the canonical ABI flattens the nested `tuple<…,
; tuple<…>>` RECURSIVELY to its leaf scalar core params (depth-first), the core `call` rebuilds the nested cell
; recursively (`emit_cell_rebuild` threads a leaf cursor; a nested field builds its own sub-cell + the parent
; `arr-set`s the sub-handle), and the envelope mints the INNER `tuple<…>` type by index. Proven by the
; `a_nested_fixed_shape_tuple_closure_arg_crosses_by_recursive_flattening` oracle. (A nested arg alongside
; scalars, or with a list<u8>-crossing result, or on multi/mixed/distinct-sig, is a later widening.)
(case
  "a NESTED Tuple ARG (a tuple containing a tuple) crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 Int64)) Int64)` — the arg's SECOND field is itself a tuple. It
           crosses as a nested `tuple<s64, tuple<s64,s64>>` flattened to THREE leaf core params; the `call`
           rebuilds the inner cell then the outer. `call(handle, (100, (10, 3)))` → `p.0 + p.1.0 + p.1.1` = 113.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (export mk)))
  (call mk (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 113 Int64))
  (live-objects 0))

(case
  "a NESTED Record ARG (a record containing a record) crosses the direct-call boundary"
  (doc
    "`mk : (-> (Record (: n Int64) (: inner (Record (: x Int64) (: y Int64)))) Int64)` — a record with a record
           field. Each level flattens to its sorted-key leaves + rebuilds recursively. `call(handle, (record
           (n 100) (inner (record (x 10) (y 3)))))` → `r.n + r.inner.x + r.inner.y` = 113.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: r (Record (: n Int64) (: inner (Record (: x Int64) (: y Int64))))))
          (+ r.n (+ r.inner.x r.inner.y))))
      (export mk)))
  (call
    mk
    (:
      #record((= n 100) (= inner #record((= x 10) (= y 3))))
      (Record (: n Int64) (: inner (Record (: x Int64) (: y Int64))))))
  (output (: 113 Int64))
  (live-objects 0))

(case
  "a NESTED mixed ARG (a tuple containing a record) crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 (Record (: x Int64) (: y Int64))) Int64)` — a tuple whose second field is a
           RECORD. The nested-compound rebuild is kind-agnostic (tuple or record at any level). `call(handle,
           (100, (record (x 10) (y 3))))` → `p.0 + p.1.x + p.1.y` = 113.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 (Record (: x Int64) (: y Int64)))))
          (+ (. p 0) (+ (. (. p 1) x) (. (. p 1) y)))))
      (export mk)))
  (call mk (: #tuple(100 #record((= x 10) (= y 3))) (Tuple Int64 (Record (: x Int64) (: y Int64)))))
  (output (: 113 Int64))
  (live-objects 0))

(case
  "a DOUBLY-nested Tuple ARG (three levels deep) crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 (Tuple Int64 Int64))) Int64)` — three tuple levels. The
           recursive mint emits the innermost `tuple<s64,s64>` first, then the middle, then the outer; the
           rebuild recurses to match. `call(handle, (1000, (100, (10, 3))))` → 1000+100+10+3 = 1113.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 (Tuple Int64 (Tuple Int64 Int64)))))
          (+ (. p 0) (+ (. (. p 1) 0) (+ (. (. (. p 1) 1) 0) (. (. (. p 1) 1) 1))))))
      (export mk)))
  (call
    mk
    (: #tuple(1000 #tuple(100 #tuple(10 3))) (Tuple Int64 (Tuple Int64 (Tuple Int64 Int64)))))
  (output (: 1113 Int64))
  (live-objects 0))

(case
  "a NESTED Tuple ARG with a narrow Bool leaf crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 Bool)) Int64)` — the inner tuple has a Bool leaf (boxed via
           `box-bool` in the recursive rebuild). `call(handle, (100, (10, true)))` → `if p.1.1 then p.0 + p.1.0
           else p.0` = 110. Confirms the recursive rebuild handles a narrow leaf at depth.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 (Tuple Int64 Bool))))
          (if (. (. p 1) 1) (+ (. p 0) (. (. p 1) 0)) (. p 0))))
      (export mk)))
  (call mk (: #tuple(100 #tuple(10 true)) (Tuple Int64 (Tuple Int64 Bool))))
  (output (: 110 Int64))
  (live-objects 0))

(case
  "a RECORD with a nested TUPLE field crosses the direct-call boundary"
  (doc
    "`mk : (-> (Record (: n Int64) (: pair (Tuple Int64 Int64))) Int64)` — a record whose `pair` field is a
           TUPLE (mixed nesting kinds). The recursive rebuild + type mint are kind-agnostic. `call(handle,
           (record (n 100) (pair (tuple 10 3))))` → `r.n + r.pair.0 + r.pair.1` = 113.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: r (Record (: n Int64) (: pair (Tuple Int64 Int64)))))
          (+ r.n (+ (. r.pair 0) (. r.pair 1)))))
      (export mk)))
  (call
    mk
    (: #record((= n 100) (= pair #tuple(10 3))) (Record (: n Int64) (: pair (Tuple Int64 Int64)))))
  (output (: 113 Int64))
  (live-objects 0))

(case
  "a TRIPLY-nested Record ARG crosses the direct-call boundary"
  (doc
    "`mk : (-> (Record (: a Int64) (: b (Record (: c Int64) (: d (Record (: e Int64) (: f Int64)))))) Int64)` — three
           record levels. The recursive rebuild + mint descend arbitrarily deep. `call(handle, (record (a 1000)
           (b (record (c 100) (d (record (e 10) (f 3)))))))` → 1000+100+10+3 = 1113.")
  (input
    (do
      (def
        (mk)
        (fn
          ((:
              r
              (Record (: a Int64) (: b (Record (: c Int64) (: d (Record (: e Int64) (: f Int64))))))))
          (+ r.a (+ r.b.c (+ r.b.d.e r.b.d.f)))))
      (export mk)))
  (call
    mk
    (:
      #record((= a 1000) (= b #record((= c 100) (= d #record((= e 10) (= f 3))))))
      (Record (: a Int64) (: b (Record (: c Int64) (: d (Record (: e Int64) (: f Int64))))))))
  (output (: 1113 Int64))
  (live-objects 0))

; A NESTED compound ARG can also sit AMONG scalar args (single-export): the recursive rebuild interleaves the
; prefix scalars, the reassembled nested cell, and the suffix scalars (the `base_param` shifts past the prefix,
; and the envelope's interleaved functype surrounds the minted `tuple<…>` types with the scalar boundary bytes).
(case
  "a NESTED Tuple ARG among scalar args crosses the direct-call boundary"
  (doc
    "`mk : (-> Int64 (Tuple Int64 (Tuple Int64 Int64)) Int64)` — a prefix scalar `n` then a NESTED tuple.
           The `call` pushes `n`, rebuilds the nested cell (its leaves flattened past the prefix scalar),
           dispatches. `call(handle, 1000, (100, (10, 3)))` → `n + p.0 + p.1.0 + p.1.1` = 1113.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          (+ n (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))))
      (export mk)))
  (call mk (: 1000 Int64) (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 1113 Int64))
  (live-objects 0))

(case
  "a NESTED Tuple ARG between two scalar args (prefix + suffix)"
  (doc
    "`mk : (-> Int64 (Tuple Int64 (Tuple Int64 Int64)) Int64 Int64)` — a prefix scalar `a`, the nested
           tuple `p`, and a SUFFIX scalar `c`. `call(handle, 1, (100, (10, 3)), 1000)` → `a + c + p.0 + p.1.0 +
           p.1.1` = 1114. Confirms the nested rebuild interleaves both prefix + suffix scalars.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: a Int64) (: p (Tuple Int64 (Tuple Int64 Int64))) (: c Int64))
          (+ (+ a c) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))))
      (export mk)))
  (call
    mk
    (: 1 Int64)
    (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64)))
    (: 1000 Int64))
  (output (: 1114 Int64))
  (live-objects 0))

(case
  "a NESTED Tuple ARG among scalars with a LIST result"
  (doc
    "`mk : (-> Int64 (Tuple Int64 (Tuple Int64 Int64)) (List Int64))` — a prefix scalar then a nested
           tuple, returning a List. The value-encode `call` interleaves `n` around the recursively-rebuilt
           nested cell, dispatches, value-encodes. `call(handle, 1000, (100, (10, 3)))` → `(list 1000 100 10
           3)`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          #list(n (. p 0) (. (. p 1) 0) (. (. p 1) 1))))
      (export mk)))
  (call mk (: 1000 Int64) (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: #list(1000 100 10 3) (List Int64)))
  (live-objects 0))

; A NESTED compound ARG composes with EVERY result shape (single-export): the list-result cores rebuild the
; nested cell recursively (`emit_cell_rebuild`), and the list<u8> envelope mints the inner `tuple<…>` types by
; index (`tuple_shape`, the same recursive minting as the scalar-result path). So a nested arg crosses with a
; byte-rope, a fixed-shape compound value-form, or a variable-length collection result.
(case
  "a NESTED Tuple ARG with a LIST result crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 Int64)) (List Int64))` — a nested tuple arg AND a collection
           result. The value-encode `call` rebuilds the nested cell recursively, dispatches, then value-encodes
           the returned List. `call(handle, (100, (10, 3)))` → `(list p.0 p.1.0 p.1.1)` = `(list 100 10 3)`.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) #list((. p 0) (. (. p 1) 0) (. (. p 1) 1))))
      (export mk)))
  (call mk (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

(case
  "a NESTED Tuple ARG with a BYTE-ROPE result crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 Int64)) Bytes)` — a nested tuple arg + a byte rope. The bytes
           `call` rebuilds the nested cell, dispatches, copies the returned Bytes out as `list<u8>`.
           `call(handle, (100, (10, 3)))` → the bytes `(100 10 3)`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 (Tuple Int64 Int64))))
          (bin
            (u8 (UInt8.wrap (. p 0)))
            (u8 (UInt8.wrap (. (. p 1) 0)))
            (u8 (UInt8.wrap (. (. p 1) 1))))))
      (export mk)))
  (call mk (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output #list(100 10 3))
  (live-objects 0))

(case
  "a NESTED Tuple ARG with a fixed-shape COMPOUND result crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 Int64)) (Tuple Int64 Int64 Int64))` — a nested tuple arg + a
           fixed-shape compound result. The value-form `call` rebuilds the nested arg cell, dispatches, walks
           the returned handle into the template. `call(handle, (100, (10, 3)))` → `(tuple 100 10 3)`.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) #tuple((. p 0) (. (. p 1) 0) (. (. p 1) 1))))
      (export mk)))
  (call mk (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: (tuple 100 10 3) (Tuple Int64 Int64 Int64)))
  (live-objects 0))

(case
  "a NESTED Record ARG with a LIST result crosses the direct-call boundary"
  (doc
    "`mk : (-> (Record (: n Int64) (: inner (Record (: x Int64) (: y Int64)))) (List Int64))` — a nested RECORD
           arg + a collection result (both the nested-record rebuild + the value-encode result compose).
           `call(handle, (record (n 100) (inner (record (x 10) (y 3)))))` → `(list 100 10 3)`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: r (Record (: n Int64) (: inner (Record (: x Int64) (: y Int64))))))
          #list(r.n r.inner.x r.inner.y)))
      (export mk)))
  (call
    mk
    (:
      #record((= n 100) (= inner #record((= x 10) (= y 3))))
      (Record (: n Int64) (: inner (Record (: x Int64) (: y Int64))))))
  (output (: #list(100 10 3) (List Int64)))
  (live-objects known-leak))

; The NESTED compound ARG extends to the MULTI-EXPORT path: N same-sig nested-tuple-arg closures share ONE
; `call` that rebuilds the nested cell recursively, and the shared envelope mints the inner `tuple<…>` types by
; index (`tuple_shape`) — for a scalar result AND a list<u8>-crossing result. (Distinct-sig nested still
; declines — that path's per-group detection doesn't yet use the nested classifier.)
(case
  "MULTI-EXPORT: two nested-Tuple-arg closures share one call — driving the sum"
  (doc
    "`mk-a`/`mk-b : (-> (Tuple Int64 (Tuple Int64 Int64)) Int64)` — a nested tuple arg, two exports, one
           shared `call` rebuilding the nested cell. `make-a()` → handle, `call(handle, (100, (10, 3)))` →
           `p.0 + p.1.0 + p.1.1` = 113.")
  (input
    (do
      (def
        (mk-a)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (def
        (mk-b)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (- (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 113 Int64))
  (live-objects 0))

(case
  "MULTI-EXPORT: driving the second nested-Tuple-arg closure (subtract)"
  (doc
    "The SAME multi-export component, driving `mk-b` (subtract): `call(handle, (100, (10, 3)))` → `p.0 -
           (p.1.0 + p.1.1)` = 87. Confirms both same-sig nested-arg closures share the one recursive-rebuild
           `call`.")
  (input
    (do
      (def
        (mk-a)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (def
        (mk-b)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (- (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 87 Int64))
  (live-objects 0))

(case
  "MULTI-EXPORT: a nested-Tuple-arg closure with a LIST result"
  (doc
    "`mk-a`/`mk-b : (-> (Tuple Int64 (Tuple Int64 Int64)) (List Int64))` — a nested tuple arg + a
           collection result, shared `call`. The value-encode `call` rebuilds the nested cell, dispatches,
           value-encodes the returned List. `call(handle, (100, (10, 3)))` → `(list 100 10 3)`.")
  (input
    (do
      (def
        (mk-a)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) #list((. p 0) (. (. p 1) 0) (. (. p 1) 1))))
      (def
        (mk-b)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) #list((. (. p 1) 1) (. (. p 1) 0) (. p 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

; The NESTED compound ARG extends to the MIXED shape too: a nested-tuple-arg closure exported ALONGSIDE a
; plain (non-closure) export. The shared `call` rebuilds the nested cell recursively + mints the inner
; `tuple<…>` types by index (`tuple_shape`); the plain export rides alongside as a top-level func — for a
; scalar result AND a list<u8>-crossing result.
(case
  "MIXED: a nested-Tuple-arg closure (scalar result) ALONGSIDE a plain export — driving the closure"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 Int64)) Int64)` beside a plain `two`. The shared `call`
           rebuilds the nested cell. `make()` → handle, `call(handle, (100, (10, 3)))` → `p.0 + p.1.0 + p.1.1`
           = 113.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call mk (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 113 Int64))
  (live-objects 0))

(case
  "MIXED: driving the PLAIN export alongside a nested-Tuple-arg closure"
  (doc
    "The SAME mixed component, driving the plain `two` — it coexists with the nested-tuple-arg closure
           interface. `two()` → 2.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call two)
  (output (: 2 Int64)))

(case
  "MIXED: a nested-Tuple-arg closure with a LIST result ALONGSIDE a plain export"
  (doc
    "`mk : (-> (Tuple Int64 (Tuple Int64 Int64)) (List Int64))` beside a plain `two`. The value-encode
           `call` rebuilds the nested cell, dispatches, value-encodes the returned List. `call(handle, (100,
           (10, 3)))` → `(list 100 10 3)`.")
  (input
    (do
      (def
        (mk)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) #list((. p 0) (. (. p 1) 0) (. (. p 1) 1))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call mk (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

; The NESTED-arg-AMONG-scalars shape extends to the MULTI-EXPORT + MIXED paths too (via the shared
; `nested_sole_or_among_scalars` classifier + the interleaved envelope functype): a nested tuple at its own
; position among aliased-width scalars, shared across N exports or beside a plain export. (Distinct-sig
; nested-among-scalars still declines — its per-group detector doesn't yet take the among-scalars variant.)
(case
  "MULTI-EXPORT: two closures each taking a scalar arg THEN a NESTED tuple"
  (doc
    "`mk-a`/`mk-b : (-> Int64 (Tuple Int64 (Tuple Int64 Int64)) Int64)` — a scalar `n` then a NESTED
           tuple, shared across two exports. The shared `call` interleaves `n` around the recursively-rebuilt
           nested cell. Driving `mk-a`: `call(handle, 1000, (100, (10, 3)))` → `n + p.0 + p.1.0 + p.1.1` =
           1113.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          (+ n (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))))
      (def
        (mk-b)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          (- n (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 1000 Int64) (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 1113 Int64))
  (live-objects 0))

(case
  "MULTI-EXPORT: driving the second scalar-then-NESTED closure (subtract)"
  (doc
    "The SAME multi-export component, driving `mk-b` (subtract): `call(handle, 1000, (100, (10, 3)))` →
           `n - (p.0 + p.1.0 + p.1.1)` = 887.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          (+ n (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))))
      (def
        (mk-b)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          (- n (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: 1000 Int64) (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 887 Int64))
  (live-objects 0))

(case
  "MIXED: a scalar-then-NESTED-tuple closure with a LIST result ALONGSIDE a plain export"
  (doc
    "`mk : (-> Int64 (Tuple Int64 (Tuple Int64 Int64)) (List Int64))` beside a plain `two`. The shared
           value-encode `call` interleaves the prefix scalar around the recursively-rebuilt nested cell, then
           value-encodes the returned List. `call(handle, 1000, (100, (10, 3)))` → `(list 1000 100 10 3)`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          #list(n (. p 0) (. (. p 1) 0) (. (. p 1) 1))))
      (def (two) 2)
      (export mk)
      (export two)))
  (call mk (: 1000 Int64) (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: #list(1000 100 10 3) (List Int64)))
  (live-objects 0))

; The NESTED compound ARG completes the export-shape matrix on the DISTINCT-SIGNATURE path: closures of
; DIFFERENT signatures each taking a sole nested tuple/record arg cross as G distinct resource types, each
; per-group `call-g<n>` rebuilding ITS nested cell recursively + minting ITS inner `tuple<…>` types by index
; (`SigGroupAbi.tuple_shape`). A nested arg now works on ALL FOUR export shapes for every result shape.
(case
  "DISTINCT-SIG: two DIFFERENT-signature nested-Tuple-arg closures each cross the direct-call boundary"
  (doc
    "`mk-a : (-> (Tuple Int64 (Tuple Int64 Int64)) Int64)` and `mk-b : (-> (Tuple Int64 (Tuple Int64
           Bool)) Int64)` — two DIFFERENT nested-tuple signatures (Int64-inner vs Int64/Bool-inner), each its
           own resource type + `call-g<n>` rebuilding its nested cell. Driving `mk-a`: `make-a()` → handle,
           `call(handle, (100, (10, 3)))` → `p.0 + p.1.0 + p.1.1` = 113.")
  (input
    (do
      (def
        (mk-a)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (def
        (mk-b)
        (fn
          ((: q (Tuple Int64 (Tuple Int64 Bool))))
          (if (. (. q 1) 1) (+ (. q 0) (. (. q 1) 0)) (. q 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 113 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: driving the Int64/Bool-inner nested-Tuple-arg closure (Bool leaf at depth)"
  (doc
    "The SAME distinct-sig component, driving `mk-b : (-> (Tuple Int64 (Tuple Int64 Bool)) Int64)` — its
           inner tuple has a Bool leaf (boxed via `box-bool` in the recursive rebuild), its own resource type +
           `call-g<n>`. `make-b()` → handle, `call(handle, (100, (10, true)))` → `if q.1.1 then q.0 + q.1.0 else
           q.0` = 110. Confirms distinct-sig groups mint their own nested types + rebuild independently.")
  (input
    (do
      (def
        (mk-a)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))))
      (def
        (mk-b)
        (fn
          ((: q (Tuple Int64 (Tuple Int64 Bool))))
          (if (. (. q 1) 1) (+ (. q 0) (. (. q 1) 0)) (. q 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: #tuple(100 #tuple(10 true)) (Tuple Int64 (Tuple Int64 Bool))))
  (output (: 110 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: a nested-Tuple-arg closure with a LIST result"
  (doc
    "`mk-a : (-> (Tuple Int64 (Tuple Int64 Int64)) (List Int64))` + `mk-b : (-> (Tuple Int64 (Tuple Int64
           Bool)) (List Int64))` — DIFFERENT nested sigs, each list-returning `call-g<n>` rebuilding its nested
           cell then value-encoding the List. Driving `mk-a`: `call(handle, (100, (10, 3)))` → `(list 100 10
           3)`.")
  (input
    (do
      (def
        (mk-a)
        (fn ((: p (Tuple Int64 (Tuple Int64 Int64)))) #list((. p 0) (. (. p 1) 0) (. (. p 1) 1))))
      (def (mk-b) (fn ((: q (Tuple Int64 (Tuple Int64 Bool)))) #list((. q 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

; The NESTED-arg-AMONG-scalars shape completes on the DISTINCT-SIG path too — the LAST nested-arg gap. Each
; group's per-`call-g<n>` detection takes the shared `nested_sole_or_among_scalars` classifier, so a nested
; tuple sits at its own position among aliased-width scalars; the per-group functype interleaves the scalar
; boundary bytes around the minted `tuple<…>` types. A nested fixed-shape compound ARG now crosses on ALL FOUR
; export shapes, SOLE or AMONG scalars, for every result shape.
(case
  "DISTINCT-SIG: two DIFFERENT-sig scalar-then-NESTED-tuple closures — driving the Int64-inner group"
  (doc
    "`mk-a : (-> Int64 (Tuple Int64 (Tuple Int64 Int64)) Int64)` and `mk-b : (-> Int64 (Tuple Int64
           (Tuple Int64 Bool)) Int64)` — a scalar `n` then a NESTED tuple, of DIFFERENT nested signatures. Each
           its own resource type + `call-g<n>` interleaving `n` around the recursively-rebuilt nested cell.
           `make-a()` → handle, `call(handle, 1000, (100, (10, 3)))` → `n + p.0 + p.1.0 + p.1.1` = 1113.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          (+ n (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))))
      (def
        (mk-b)
        (fn ((: n Int64) (: q (Tuple Int64 (Tuple Int64 Bool)))) (if (. (. q 1) 1) (+ n (. q 0)) n)))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 1000 Int64) (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (output (: 1113 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: driving the Int64/Bool-inner scalar-then-NESTED closure (Bool leaf at depth)"
  (doc
    "The SAME distinct-sig component, driving `mk-b` — its inner tuple has a Bool leaf. `call(handle,
           1000, (100, (10, true)))` → `if q.1.1 then n + q.0 else n` = `1000 + 100` = 1100 (q.0 is the OUTER
           first field). Confirms distinct-sig groups interleave the prefix scalar + rebuild independently.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          (+ n (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1))))))
      (def
        (mk-b)
        (fn ((: n Int64) (: q (Tuple Int64 (Tuple Int64 Bool)))) (if (. (. q 1) 1) (+ n (. q 0)) n)))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: 1000 Int64) (: #tuple(100 #tuple(10 true)) (Tuple Int64 (Tuple Int64 Bool))))
  (output (: 1100 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: a scalar-then-NESTED-tuple closure with a LIST result"
  (doc
    "`mk-a`/`mk-b` of DIFFERENT nested sigs, each `(-> Int64 (Tuple Int64 (Tuple Int64 …)) (List Int64))`.
           The list-returning `call-g<n>` interleaves `n` around the recursively-rebuilt nested cell then
           value-encodes. Driving `mk-a`: `call(handle, 1000, (100, (10, 3)))` → `(list 1000 100 10 3)`.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: n Int64) (: p (Tuple Int64 (Tuple Int64 Int64))))
          #list(n (. p 0) (. (. p 1) 0) (. (. p 1) 1))))
      (def (mk-b) (fn ((: n Int64) (: q (Tuple Int64 (Tuple Int64 Bool)))) #list(n (. q 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 1000 Int64) (: #tuple(100 #tuple(10 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (drop)
  (output (: #list(1000 100 10 3) (List Int64)))
  (live-objects 0))

; A WIDER fixed-shape tuple (3+ fields) and DEEPER scalar interleaving (2 prefix + 1 suffix) also cross — the
; flatten/rebuild + interleave machinery is field-count- and position-agnostic.
(case
  "a 3-FIELD Tuple ARG among scalars crosses the direct-call boundary"
  (doc
    "`mk : (-> Int64 (Tuple Int64 Int64 Int64) Int64)` — a scalar then a 3-field tuple (flattened to 3
           core params). `call(handle, 100, (10, 3, 1))` → `n + p.0 + p.1 + p.2` = 114.")
  (input
    (do
      (def
        (mk)
        (fn ((: n Int64) (: p (Tuple Int64 Int64 Int64))) (+ n (+ (. p 0) (+ (. p 1) (. p 2))))))
      (export mk)))
  (call mk (: 100 Int64) (: #tuple(10 3 1) (Tuple Int64 Int64 Int64)))
  (output (: 114 Int64)))

(case
  "TWO prefix scalars then a Tuple then a suffix scalar cross the direct-call boundary"
  (doc
    "`mk : (-> Int64 Int64 (Tuple Int64 Int64) Int64 Int64)` — TWO prefix scalars `a`,`b`, the tuple `p`
           (base_param=3), and a SUFFIX scalar `c`. The `call` pushes `a`,`b`, rebuilds `p`, pushes `c`.
           `call(handle, 1, 2, (10, 3), 100)` → `a + b + p.0 + p.1 + c` = 116.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: a Int64) (: b Int64) (: p (Tuple Int64 Int64)) (: c Int64))
          (+ (+ a b) (+ (+ (. p 0) (. p 1)) c))))
      (export mk)))
  (call mk (: 1 Int64) (: 2 Int64) (: #tuple(10 3) (Tuple Int64 Int64)) (: 100 Int64))
  (output (: 116 Int64)))

; The flatten/rebuild machinery is FIELD-TYPE agnostic: a tuple's fields may be FLOATS, mixed WIDTHS, or a
; Bool interleaved with ints — each field crosses as its own aliased-width component scalar. A single-variant
; NOMINAL over a tuple erases to the tuple (per §156), so a nominal-tuple arg crosses as the bare tuple.
(case
  "a Tuple ARG of FLOAT fields crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Float64 Float64) Float64)` — a tuple of two f64 fields (each crosses as an f64
           core param). `call(handle, (1.5, 2.5))` → `p.0 + p.1` = 4.0. The field type need not be an integer.")
  (input (do (def (mk) (fn ((: p (Tuple Float64 Float64))) (+ (. p 0) (. p 1)))) (export mk)))
  (call mk (: #tuple(1.5 2.5) (Tuple Float64 Float64)))
  (output (: 4.0 Float64)))

(case
  "a Tuple ARG with a Bool field between Int64 fields crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 Bool Int64) Int64)` — a Bool field interleaved among ints (the rebuild boxes
           it via `box-bool`). `call(handle, (10, true, 100))` → `if p.1 then p.0 + p.2 else p.2` = 110.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Bool Int64))) (if (. p 1) (+ (. p 0) (. p 2)) (. p 2))))
      (export mk)))
  (call mk (: #tuple(10 true 100) (Tuple Int64 Bool Int64)))
  (output (: 110 Int64)))

(case
  "a Tuple ARG of MIXED widths (Int32, Int64) crosses the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int32 Int64) Int32)` — the fields have DIFFERENT machine widths; each still crosses
           as its own aliased-width core param (Int32 → i32, Int64 → i64). `call(handle, (42, 100))` → `p.0` = 42.")
  (input (do (def (mk) (fn ((: p (Tuple Int32 Int64))) (. p 0))) (export mk)))
  (call mk (: #tuple(42 100) (Tuple Int32 Int64)))
  (output (: 42 Int32)))

(case
  "a NOMINAL-over-tuple ARG crosses as the underlying tuple (direct-call)"
  (doc
    "`(type Pair (Mk (Tuple Int64 Int64)))` + `mk : (-> Pair Int64)` — a single-variant nominal over a
           tuple erases to the tuple (§156), so `Pair` crosses as `tuple<s64,s64>` (NO wrapper resource). The
           host supplies the bare underlying tuple. `call(handle, (10, 3))` → `(match p ((Mk t) (+ t.0 t.1)))`
           = 13. The peel is kind-agnostic — it applies to a nominal over a compound just as over a scalar.")
  (input
    (do
      (type Pair (Mk (Tuple Int64 Int64)))
      (def (mk) (fn ((: p Pair)) (match p ((Mk t) (+ (. t 0) (. t 1))))))
      (export mk)))
  (call mk (: #tuple(10 3) Pair))
  (output (: 13 Int64)))

; The tuple-among-scalars arg shape extends to the MULTI-EXPORT path — for EVERY result shape: N same-sig
; closures share one `call` that interleaves the prefix scalar with the rebuilt tuple, then produces a scalar,
; a byte-rope, a fixed-shape compound value-form, or a collection value-encode result. (The among-scalars tuple
; arg is now supported on ALL FOUR export shapes — single, multi, mixed, and distinct-sig — for every result
; shape; the shared `emit_closure_call_args` + interleaved functypes thread it uniformly.)
(case
  "MULTI-EXPORT: two closures each taking a scalar arg THEN a Tuple arg"
  (doc
    "`mk-a`/`mk-b : (-> Int64 (Tuple Int64 Int64) Int64)` — a scalar `n` then a tuple `p`, shared across
           two exports. The shared `call` pushes `n`, rebuilds the tuple, dispatches. Driving `mk-a`: `make-a()`
           → handle, `call(handle, 100, (10, 3))` → `n + p.0 + p.1` = 113. The tuple-among-scalars interleaving
           on the multi-export shared `call`.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (+ n (+ (. p 0) (. p 1)))))
      (def (mk-b) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (- n (+ (. p 0) (. p 1)))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (output (: 113 Int64)))

(case
  "MULTI-EXPORT: driving the second among-scalars closure (subtract)"
  (doc
    "The SAME multi-export component, driving `mk-b` (subtract): `call(handle, 100, (10, 3))` → `n -
           (p.0 + p.1)` = 87. Confirms both same-sig among-scalars closures share the one interleaving `call`.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (+ n (+ (. p 0) (. p 1)))))
      (def (mk-b) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (- n (+ (. p 0) (. p 1)))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (output (: 87 Int64)))

(case
  "MULTI-EXPORT: among-scalars tuple arg with a LIST result (shared interleaving list-`call`)"
  (doc
    "`mk-a`/`mk-b : (-> Int64 (Tuple Int64 Int64) (List Int64))` — a scalar `n` then a tuple `p`, sharing
           ONE list-returning `call`. The shared value-encode `call` interleaves `n` around the rebuilt tuple,
           dispatches, then value-encodes the returned List. Driving `mk-a`: `call(handle, 100, (10, 3))` →
           `(list 100 10 3)`. The among-scalars interleaving now reaches the multi-export list-result core.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #list(n (. p 0) (. p 1))))
      (def (mk-b) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #list((. p 0) (. p 1) n)))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(100 10 3) (List Int64)))
  (live-objects 0))

(case
  "MULTI-EXPORT: driving the second among-scalars LIST closure (tuple then suffix scalar)"
  (doc
    "The SAME multi-export List component, driving `mk-b` — the tuple fields FIRST then the suffix scalar
           `n`: `call(handle, 100, (10, 3))` → `(list p.0 p.1 n)` = `(list 10 3 100)`. Confirms both same-sig
           among-scalars List closures share the one interleaving list-`call`, prefix AND suffix.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #list(n (. p 0) (. p 1))))
      (def (mk-b) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #list((. p 0) (. p 1) n)))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(10 3 100) (List Int64)))
  (live-objects 0))

(case
  "MULTI-EXPORT: among-scalars tuple arg with a BYTE-ROPE result"
  (doc
    "`mk-a`/`mk-b : (-> Int64 (Tuple Int64 Int64) Bytes)` sharing one bytes-returning `call`. The shared
           bytes `call` interleaves `n` around the rebuilt tuple, dispatches, copies the returned Bytes out as
           `list<u8>`. Driving `mk-a`: `call(handle, 100, (10, 3))` → the bytes `(100 10 3)`.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: n Int64) (: p (Tuple Int64 Int64)))
          (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (. p 0))) (u8 (UInt8.wrap (. p 1))))))
      (def (mk-b) (fn ((: n Int64) (: p (Tuple Int64 Int64))) (bin (u8 (UInt8.wrap (. p 0))))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output #list(100 10 3))
  (live-objects 0))

(case
  "MULTI-EXPORT: among-scalars tuple arg with a fixed-shape COMPOUND result"
  (doc
    "`mk-a`/`mk-b : (-> Int64 (Tuple Int64 Int64) (Tuple Int64 Int64 Int64))` sharing one value-form
           `call`. The shared `call` interleaves `n` around the rebuilt arg tuple, dispatches, walks the
           returned handle into the value-form template. Driving `mk-a`: `call(handle, 100, (10, 3))` →
           `(tuple 100 10 3)`, decoded by the host to the typed document.")
  (input
    (do
      (def (mk-a) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #tuple(n (. p 0) (. p 1))))
      (def (mk-b) (fn ((: n Int64) (: p (Tuple Int64 Int64))) #tuple((. p 0) n (. p 1))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 100 Int64) (: #tuple(10 3) (Tuple Int64 Int64)))
  (drop)
  (output (: (tuple 100 10 3) (Tuple Int64 Int64 Int64)))
  (live-objects 0))

; N-COMPOUND-ARGS: a closure taking TWO OR MORE fixed-shape tuple/record arguments crosses the direct-call
; boundary. Each tuple crosses as its OWN native component `tuple<…>` (the canonical ABI flattens them all
; into scalar core params); the guest `call` rebuilds every arg cell from its own `TupleArgRebuild` (a slice
; of rebuilds, base-param'd across the flattened leaves in order), and the envelope mints N `tuple<…>` defined
; types interleaved with any scalar args (the `ArgSlot` model). This generalizes the single-tuple among-scalars
; path (ONE compound) to any number of compound args. Scoped to a SCALAR result this increment; the two
; single-tuple classifiers keep exactly-one-compound byte-identical, so ≥2 compounds is the new path.
(case
  "TWO Tuple args cross the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)` reads `p.0 + q.1`. Both tuple args cross
           as native `tuple<s64,s64>` (two independent flattenings → four scalar core params); the `call`
           rebuilds BOTH cells (one `TupleArgRebuild` each, at base-params 1 and 3) and dispatches.
           `call(handle, (5,5), (5,10))` → `5 + 10` = 15. The N-compound-args path with N=2.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (export mk)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (output (: 15 Int64)))

(case
  "TWO Tuple args with a scalar BETWEEN them (tuple, scalar, tuple)"
  (doc
    "`mk : (-> (Tuple Int64 Int64) Int64 (Tuple Int64 Int64) Int64)` — a scalar interleaved between two
           tuple args. The `call` rebuilds the first tuple, pushes the scalar, rebuilds the second tuple, and
           dispatches — proving the `ArgSlot` model interleaves scalars among N tuples. `call(handle, (5,5),
           10, (1,20))` → `p.0 + n + q.1` = `5 + 10 + 20` = 35.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 Int64)) (: n Int64) (: q (Tuple Int64 Int64)))
          (+ (+ (. p 0) n) (. q 1))))
      (export mk)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)) (: 10 Int64) (: #tuple(1 20) (Tuple Int64 Int64)))
  (output (: 35 Int64)))

(case
  "THREE Tuple args cross the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)` reads `p.0 + q.1 +
           r.0`. Three native `tuple<s64,s64>` args flatten to six scalar core params; the `call` rebuilds all
           three cells (base-params 1, 3, 5) and dispatches. `call(handle, (1,2), (3,4), (100,200))` → `1 + 4
           + 100` = 105. The N-compound-args path at N=3.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64)) (: r (Tuple Int64 Int64)))
          (+ (+ (. p 0) (. q 1)) (. r 0))))
      (export mk)))
  (call
    mk
    (: #tuple(1 2) (Tuple Int64 Int64))
    (: #tuple(3 4) (Tuple Int64 Int64))
    (: #tuple(100 200) (Tuple Int64 Int64)))
  (output (: 105 Int64)))

(case
  "a NESTED Tuple arg ALONGSIDE a flat Tuple arg"
  (doc
    "The N-compound-args path composes with a NESTED compound arg: `mk : (-> (Tuple Int64 (Tuple Int64
           Int64)) (Tuple Int64 Int64) Int64)` reads `p.1.0 + q.1`. The first arg's inner tuple flattens
           RECURSIVELY (its own minted inner `tuple<…>` type), the second is flat; the `call` rebuilds both
           (the nested one via a recursive `FieldRebuild`). `call(handle, (1,(7,8)), (3,40))` → `7 + 40` = 47.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 (Tuple Int64 Int64))) (: q (Tuple Int64 Int64)))
          (+ (. (. p 1) 0) (. q 1))))
      (export mk)))
  (call
    mk
    (: #tuple(1 #tuple(7 8)) (Tuple Int64 (Tuple Int64 Int64)))
    (: #tuple(3 40) (Tuple Int64 Int64)))
  (output (: 47 Int64)))

(case
  "a CAPTURING closure taking TWO Tuple args"
  (doc
    "The N-compound-args path composes with capture (C-HOST-2): a parameterized export `(def (mk (: k
           Int64)) …)` returns a closure that captures `k` AND takes two `(Tuple Int64 Int64)` args.
           `make(100)` → a handle closing over k=100; `call(handle, (5,5), (5,10))` → `p.0 + q.1 + k` = `5 +
           10 + 100` = 115. The forwarded capture cell and BOTH rebuilt arg cells coexist in the one `call`.")
  (input
    (do
      (def
        (mk (: k Int64))
        (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (+ (. p 0) (. q 1)) k)))
      (export mk)))
  (call mk (: 100 Int64) (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (output (: 115 Int64)))

(case
  "TWO Record args cross the direct-call boundary"
  (doc
    "The N-compound-args path is by cell shape, not spelling: two RECORD args (sorted-key field order)
           cross exactly as two tuples do. `mk : (-> {a:Int64 b:Int64} {a:Int64 b:Int64} Int64)` reads `p.a +
           q.b`; each record flattens to its sorted-key field scalars, rebuilt in-guest. `call(handle,
           {a:5 b:5}, {a:5 b:10})` → `5 + 10` = 15.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Record (: a Int64) (: b Int64))) (: q (Record (: a Int64) (: b Int64))))
          (+ p.a q.b)))
      (export mk)))
  (call
    mk
    (: #record((= a 5) (= b 5)) (Record (: a Int64) (: b Int64)))
    (: #record((= a 5) (= b 10)) (Record (: a Int64) (: b Int64))))
  (output (: 15 Int64)))

; N-COMPOUND-ARGS × LIST-RESULT: the N-compound-args path composes with EVERY closure RESULT shape that crosses
; as `list<u8>` — a byte-rope (`Bytes`), a fixed-shape compound (tuple/record), and a variable-length collection
; (`List`). The `call` rebuilds each tuple arg cell (a slice of `TupleArgRebuild`), dispatches, then runs the
; SAME result path the single-tuple list-returning cores use (bytes copy / value-form template / value-encode
; walker). The envelope mints N `tuple<…>` arg types before the shared `list<u8>` result type (the `ArgSlot`
; slot model, reused). So ≥2 compound args reach the deep result surface, not only a scalar result.
(case
  "TWO Tuple args with a LIST result cross the direct-call boundary"
  (doc
    "`mk : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) (List Int64))` pairs `p.0` and `q.1` into a list.
           Both tuple args cross as native `tuple<s64,s64>` (rebuilt in-guest); the value-encode `call`
           dispatches then renders the returned List as the value-form document. `call(handle, (5,5), (5,10))`
           → `(list 5 10)`. The N-compound-args path now reaches the collection-result core.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. p 0) (. q 1))))
      (export mk)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(5 10) (List Int64)))
  (live-objects 0))

(case
  "TWO Tuple args with a fixed-shape COMPOUND result"
  (doc
    "`mk : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) (Tuple Int64 Int64))` re-pairs `p.0` and `q.1`.
           The value-form `call` rebuilds both arg tuples, dispatches, and walks the returned tuple handle into
           the value-form template. `call(handle, (5,5), (5,10))` → `(tuple 5 10)`, decoded to the typed doc.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #tuple((. p 0) (. q 1))))
      (export mk)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output (: (tuple 5 10) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "TWO Tuple args with a BYTE-ROPE result"
  (doc
    "`mk : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) Bytes)` builds the bytes `(p.0, q.1)`. The bytes
           `call` rebuilds both arg tuples, dispatches, and copies the returned Bytes out as `list<u8>`.
           `call(handle, (5,5), (5,10))` → the bytes `(5 10)`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64)))
          (bin (u8 (UInt8.wrap (. p 0))) (u8 (UInt8.wrap (. q 1))))))
      (export mk)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output #list(5 10))
  (live-objects 0))

(case
  "THREE Tuple args with a LIST result"
  (doc
    "N=3 tuple args reaching the collection-result core: `mk : (-> (Tuple Int64 Int64) (Tuple Int64
           Int64) (Tuple Int64 Int64) (List Int64))` lists `p.0`, `q.1`, `r.0`. All three tuples flatten to six
           scalar core params, are rebuilt, and the returned List is value-encoded. `call(handle, (1,2), (3,4),
           (100,200))` → `(list 1 4 100)`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64)) (: r (Tuple Int64 Int64)))
          #list((. p 0) (. q 1) (. r 0))))
      (export mk)))
  (call
    mk
    (: #tuple(1 2) (Tuple Int64 Int64))
    (: #tuple(3 4) (Tuple Int64 Int64))
    (: #tuple(100 200) (Tuple Int64 Int64)))
  (output (: #list(1 4 100) (List Int64)))
  (live-objects known-leak))

(case
  "a scalar BETWEEN two Tuple args with a LIST result"
  (doc
    "The interleaved-scalar `ArgSlot` model reaches the list-result core: `mk : (-> (Tuple Int64 Int64)
           Int64 (Tuple Int64 Int64) (List Int64))` lists `p.0`, `n`, `q.1`. The `call` rebuilds the first
           tuple, pushes the scalar, rebuilds the second, dispatches, value-encodes the List. `call(handle,
           (5,5), 10, (1,20))` → `(list 5 10 20)`.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 Int64)) (: n Int64) (: q (Tuple Int64 Int64)))
          #list((. p 0) n (. q 1))))
      (export mk)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)) (: 10 Int64) (: #tuple(1 20) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(5 10 20) (List Int64)))
  (live-objects 0))

(case
  "a CAPTURING closure taking TWO Tuple args with a LIST result"
  (doc
    "N-compound-args + capture + collection result compose: `(def (mk (: k Int64)) …)` returns a closure
           closing over `k` and taking two tuple args, returning `(list p.0 q.1 k)`. `make(100)` → a handle;
           `call(handle, (5,5), (5,10))` → `(list 5 10 100)`. The forwarded capture cell + both rebuilt arg
           cells coexist, and the returned List is value-encoded out.")
  (input
    (do
      (def
        (mk (: k Int64))
        (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. p 0) (. q 1) k)))
      (export mk)))
  (call mk (: 100 Int64) (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(5 10 100) (List Int64)))
  (live-objects 0))

; N-COMPOUND-ARGS × MULTI-EXPORT + MIXED: the ≥2-fixed-shape-compound-arg path (SCALAR result) now composes
; with the MULTI-EXPORT shape (N same-sig closures share ONE `call`) and the MIXED shape (a compound-arg closure
; ALONGSIDE a plain non-closure export). The shared `call` rebuilds every tuple arg cell (a slice of
; `TupleArgRebuild`), dispatches through the guest funcref table by the handle's resource rep, and the shared
; envelope mints N `tuple<…>` arg types via the SAME `ArgSlot` slot model the single-export path uses. (A LIST
; result over ≥2 compound args on these shapes remains a follow-on — declines honestly; the distinct-signature
; shape is likewise not yet wired.)
(case
  "MULTI-EXPORT: two same-sig closures each taking TWO Tuple args share one `call`"
  (doc
    "`mk-sum`/`mk-diff : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)` — two same-signature closures
           each taking two tuple args, crossing as two `make-<name>`s sharing ONE `call` whose two arguments are
           native `tuple<s64,s64>`s (four flattened core params, both rebuilt in-guest). Driving `mk-diff`:
           `make-diff()` → a handle, `call(handle, (10,3), (1,2))` → `p.0 - q.1` = `10 - 2` = 8.")
  (input
    (do
      (def (mk-sum) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def (mk-diff) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (- (. p 0) (. q 1))))
      (export mk-sum)
      (export mk-diff)))
  (call mk-diff (: #tuple(10 3) (Tuple Int64 Int64)) (: #tuple(1 2) (Tuple Int64 Int64)))
  (output (: 8 Int64)))

(case
  "MULTI-EXPORT: driving the FIRST two-Tuple-arg closure (add)"
  (doc
    "The SAME multi-export component, driving `mk-sum`: `call(handle, (10,3), (1,2))` → `p.0 + q.1` = `10
           + 2` = 12. Confirms both same-sig two-tuple-arg closures share the one shared `call` (dispatched by
           the handle's resource rep).")
  (input
    (do
      (def (mk-sum) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def (mk-diff) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (- (. p 0) (. q 1))))
      (export mk-sum)
      (export mk-diff)))
  (call mk-sum (: #tuple(10 3) (Tuple Int64 Int64)) (: #tuple(1 2) (Tuple Int64 Int64)))
  (output (: 12 Int64)))

(case
  "MULTI-EXPORT: two same-sig closures each taking THREE Tuple args"
  (doc
    "N=3 tuple args on the multi-export shared `call`: `mk-a`/`mk-b : (-> (Tuple Int64 Int64) (Tuple Int64
           Int64) (Tuple Int64 Int64) Int64)`. Six flattened core params, three rebuilt cells, one shared
           `call`. Driving `mk-a`: `call(handle, (1,2), (3,4), (100,200))` → `p.0 + q.1 + r.0` = `1 + 4 + 100` =
           105.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64)) (: r (Tuple Int64 Int64)))
          (+ (+ (. p 0) (. q 1)) (. r 0))))
      (def
        (mk-b)
        (fn
          ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64)) (: r (Tuple Int64 Int64)))
          (- (. p 0) (. r 1))))
      (export mk-a)
      (export mk-b)))
  (call
    mk-a
    (: #tuple(1 2) (Tuple Int64 Int64))
    (: #tuple(3 4) (Tuple Int64 Int64))
    (: #tuple(100 200) (Tuple Int64 Int64)))
  (output (: 105 Int64)))

(case
  "MULTI-EXPORT: two capturing closures each taking TWO Tuple args"
  (doc
    "Multi-export ≥2-compound-args composes with capture: `mk-a`/`mk-b : (-> Int64 …)` each close over `k`
           AND take two tuple args. `make-a(100)` → a handle closing over k=100; `call(handle, (5,5), (5,10))`
           → `p.0 + q.1 + k` = `5 + 10 + 100` = 115. The forwarded capture cell + both rebuilt arg cells
           coexist in the one shared `call`.")
  (input
    (do
      (def
        (mk-a (: k Int64))
        (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (+ (. p 0) (. q 1)) k)))
      (def
        (mk-b (: k Int64))
        (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (- (. p 0) k)))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: 100 Int64) (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (output (: 115 Int64)))

(case
  "MULTI-EXPORT: two Tuple args with a scalar BETWEEN them, shared `call`"
  (doc
    "The interleaved-scalar `ArgSlot` model on the multi-export shared `call`: `mk-a`/`mk-b : (-> (Tuple
           Int64 Int64) Int64 (Tuple Int64 Int64) Int64)`. The shared `call` rebuilds the first tuple, pushes
           the scalar, rebuilds the second, dispatches. Driving `mk-a`: `call(handle, (5,5), 10, (1,20))` →
           `p.0 + n + q.1` = 35.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: p (Tuple Int64 Int64)) (: n Int64) (: q (Tuple Int64 Int64)))
          (+ (+ (. p 0) n) (. q 1))))
      (def
        (mk-b)
        (fn ((: p (Tuple Int64 Int64)) (: n Int64) (: q (Tuple Int64 Int64))) (- (. p 0) n)))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(5 5) (Tuple Int64 Int64)) (: 10 Int64) (: #tuple(1 20) (Tuple Int64 Int64)))
  (output (: 35 Int64)))

(case
  "MIXED: a TWO-Tuple-arg closure ALONGSIDE a plain (non-closure) export"
  (doc
    "The ≥2-compound-args path composes with the MIXED shape: a two-tuple-arg closure factory `mk : (->
           (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)` crosses via the resource envelope's `make` + shared
           `call` (rebuilding both native `tuple<s64,s64>`s) WHILE a plain export `twice` rides alongside as an
           ordinary top-level func. Driving the CLOSURE: `make()` → handle, `call(handle, (5,5), (5,10))` → 15.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (output (: 15 Int64)))

(case
  "MIXED: driving the PLAIN export alongside a two-Tuple-arg closure"
  (doc
    "The SAME mixed component, but the trial drives the PLAIN export `twice` — proving it coexists with
           the two-tuple-arg closure interface and is reachable by name. `twice(21)` → 42.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call twice (: 21 Int64))
  (output (: 42 Int64)))

(case
  "MIXED: a NESTED tuple + flat tuple arg closure ALONGSIDE a plain export"
  (doc
    "The mixed ≥2-compound-args path composes with a NESTED compound arg: `mk : (-> (Tuple Int64 (Tuple
           Int64 Int64)) (Tuple Int64 Int64) Int64)` reads `p.1.0 + q.1`, the nested arg flattening recursively
           (its own inner `tuple<…>` type), alongside a plain `twice`. `call(handle, (1,(7,8)), (3,40))` → `7 +
           40` = 47.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: p (Tuple Int64 (Tuple Int64 Int64))) (: q (Tuple Int64 Int64)))
          (+ (. (. p 1) 0) (. q 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call
    mk
    (: #tuple(1 #tuple(7 8)) (Tuple Int64 (Tuple Int64 Int64)))
    (: #tuple(3 40) (Tuple Int64 Int64)))
  (output (: 47 Int64)))

; N-COMPOUND-ARGS × MULTI-EXPORT/MIXED × LIST-CROSSING RESULT: the ≥2-compound-arg path now reaches EVERY
; `list<u8>`-crossing result shape (byte-rope / value-form compound / value-encode collection) on the
; MULTI-EXPORT and MIXED shapes too — the three multi list-result cores each rebuild every tuple arg cell (a
; slice of `TupleArgRebuild`), and the shared multi `list<u8>` envelope mints N `tuple<…>` arg types via the
; SAME `ArgSlot` slot model before the shared `list<u8>` result type. So ≥2 compound args × the full result
; matrix is closed on single-export + multi-export + mixed. (The DISTINCT-SIG shape stays a follow-on.)
(case
  "MULTI-EXPORT: two two-Tuple-arg closures with a LIST result share one `call`"
  (doc
    "`mk-a`/`mk-b : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) (List Int64))` — two same-sig closures
           each taking two tuple args, returning a List, sharing ONE value-encode `call`. Both tuple args are
           rebuilt in-guest; the returned List is value-encoded out. Driving `mk-a`: `call(handle, (5,5),
           (5,10))` → `(list p.0 q.1)` = `(list 5 10)`. The N-compound-args path reaches the multi-export
           collection-result core.")
  (input
    (do
      (def (mk-a) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. p 0) (. q 1))))
      (def (mk-b) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. q 1) (. p 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(5 10) (List Int64)))
  (live-objects 0))

(case
  "MULTI-EXPORT: driving the SECOND two-Tuple-arg LIST closure"
  (doc
    "The SAME multi-export List component, driving `mk-b` (reversed): `call(handle, (5,5), (5,10))` →
           `(list q.1 p.0)` = `(list 10 5)`. Confirms both same-sig two-tuple-arg List closures share the one
           value-encode `call`.")
  (input
    (do
      (def (mk-a) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. p 0) (. q 1))))
      (def (mk-b) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. q 1) (. p 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(10 5) (List Int64)))
  (live-objects 0))

(case
  "MULTI-EXPORT: two two-Tuple-arg closures with a fixed COMPOUND result"
  (doc
    "`mk-a`/`mk-b : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) (Tuple Int64 Int64))` sharing one
           value-form `call`. Both arg tuples are rebuilt; the returned tuple is walked into the value-form
           template. Driving `mk-a`: `call(handle, (5,5), (5,10))` → `(tuple p.0 q.1)` = `(tuple 5 10)`.")
  (input
    (do
      (def
        (mk-a)
        (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #tuple((. p 0) (. q 1))))
      (def
        (mk-b)
        (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #tuple((. q 1) (. p 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output (: (tuple 5 10) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "MULTI-EXPORT: two two-Tuple-arg closures with a BYTE-ROPE result"
  (doc
    "`mk-a`/`mk-b : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) Bytes)` sharing one bytes `call`. Both arg
           tuples are rebuilt; the returned Bytes is copied out as `list<u8>`. Driving `mk-a`: `call(handle,
           (5,5), (5,10))` → the bytes `(p.0, q.1)` = `(5 10)`.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64)))
          (bin (u8 (UInt8.wrap (. p 0))) (u8 (UInt8.wrap (. q 1))))))
      (def
        (mk-b)
        (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (bin (u8 (UInt8.wrap (. p 1))))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output #list(5 10))
  (live-objects 0))

(case
  "MULTI-EXPORT: THREE tuple args with a LIST result, shared `call`"
  (doc
    "N=3 tuple args reaching the multi-export collection-result core: `mk-a`/`mk-b : (-> (Tuple Int64
           Int64) (Tuple Int64 Int64) (Tuple Int64 Int64) (List Int64))`. Six flattened core params, three
           rebuilt cells, one shared value-encode `call`. Driving `mk-a`: `call(handle, (1,2), (3,4),
           (100,200))` → `(list p.0 q.1 r.0)` = `(list 1 4 100)`.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64)) (: r (Tuple Int64 Int64)))
          #list((. p 0) (. q 1) (. r 0))))
      (def
        (mk-b)
        (fn
          ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64)) (: r (Tuple Int64 Int64)))
          #list((. r 1))))
      (export mk-a)
      (export mk-b)))
  (call
    mk-a
    (: #tuple(1 2) (Tuple Int64 Int64))
    (: #tuple(3 4) (Tuple Int64 Int64))
    (: #tuple(100 200) (Tuple Int64 Int64)))
  (output (: #list(1 4 100) (List Int64)))
  (live-objects known-leak))

(case
  "MIXED: a two-Tuple-arg closure with a LIST result ALONGSIDE a plain export"
  (doc
    "The MIXED shape reaches the collection-result core: a two-tuple-arg closure `mk : (-> (Tuple Int64
           Int64) (Tuple Int64 Int64) (List Int64))` crosses via `make` + a shared value-encode `call` (both
           arg tuples rebuilt) WHILE a plain `twice` rides alongside. Driving the CLOSURE: `call(handle, (5,5),
           (5,10))` → `(list 5 10)`.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. p 0) (. q 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call mk (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(5 10) (List Int64)))
  (live-objects 0))

(case
  "MIXED: driving the PLAIN export alongside a two-Tuple-arg LIST closure"
  (doc
    "The SAME mixed List component, driving the PLAIN export `twice` — proving it coexists with the
           two-tuple-arg list-returning closure interface. `twice(21)` → 42.")
  (input
    (do
      (def (mk) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. p 0) (. q 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call twice (: 21 Int64))
  (output (: 42 Int64)))

; N-COMPOUND-ARGS × DISTINCT-SIGNATURE: the ≥2-fixed-shape-compound-arg path now reaches the LAST export shape
; — closures of DIFFERENT signatures crossing as G distinct resource types, each with its own per-signature
; `call-g<n>`. A group whose closure takes ≥2 tuple args rebuilds each arg cell (a slice of `TupleArgRebuild`)
; in its `call-g<n>` and mints N `tuple<…>` arg types via the SAME `ArgSlot` slot model — independently per
; group, so distinct groups may each take a different number of tuple args, at different widths, with any
; result shape. This CLOSES the N-compound-args feature across ALL FOUR export shapes (single/multi/mixed/
; distinct-sig).
(case
  "DISTINCT-SIG: two DIFFERENT-signature two-Tuple-arg closures each cross the boundary"
  (doc
    "`mk-i : (-> (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)` and `mk-b : (-> (Tuple Int32 Int32)
           (Tuple Int32 Int32) Int32)` — two closures of DIFFERENT signatures (Int64 vs Int32 tuples), each
           taking TWO tuple args, crossing as TWO distinct resource types with their own `call-g0`/`call-g1`.
           Driving `mk-i`: `make-i()` → a handle, `call-g0(handle, (5,5), (5,10))` → `p.0 + q.1` = 15. Each
           group mints its own two `tuple<…>` arg types via the slot model.")
  (input
    (do
      (def (mk-i) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def (mk-b) (fn ((: p (Tuple Int32 Int32)) (: q (Tuple Int32 Int32))) (- (. p 0) (. q 1))))
      (export mk-i)
      (export mk-b)))
  (call mk-i (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (output (: 15 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: driving the Int32 two-Tuple-arg group"
  (doc
    "The SAME distinct-sig component, driving `mk-b` (the Int32 group, subtract): `call-g1(handle,
           (10,3), (1,2))` → `p.0 - q.1` = `10 - 2` = 8. Confirms the second resource type's `call-g1` rebuilds
           its own two `tuple<s32,s32>` args independently of group 0.")
  (input
    (do
      (def (mk-i) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def (mk-b) (fn ((: p (Tuple Int32 Int32)) (: q (Tuple Int32 Int32))) (- (. p 0) (. q 1))))
      (export mk-i)
      (export mk-b)))
  (call mk-b (: #tuple(10 3) (Tuple Int32 Int32)) (: #tuple(1 2) (Tuple Int32 Int32)))
  (output (: 8 Int32))
  (live-objects 1))

(case
  "DISTINCT-SIG: one group takes TWO tuples, the other ONE tuple"
  (doc
    "Distinct groups may take a DIFFERENT NUMBER of tuple args: `mk-i : (-> (Tuple Int64 Int64) (Tuple
           Int64 Int64) Int64)` takes two, `mk-b : (-> (Tuple Int32 Int32) Int32)` takes one. Each group's
           `call-g<n>` mints exactly its own arg tuples. Driving `mk-i`: `call-g0(handle, (5,5), (5,10))` →
           `p.0 + q.1` = 15. Proves the slot model (≥2 tuples) and the single-tuple path coexist per group.")
  (input
    (do
      (def (mk-i) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def (mk-b) (fn ((: p (Tuple Int32 Int32))) (- (. p 0) (. p 1))))
      (export mk-i)
      (export mk-b)))
  (call mk-i (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (output (: 15 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: a two-Tuple-arg group with a LIST result"
  (doc
    "The distinct-sig ≥2-compound-arg path composes with a LIST result: `mk-i : (-> (Tuple Int64 Int64)
           (Tuple Int64 Int64) (List Int64))` returns a list, alongside a scalar-result Int32 group `mk-b`. The
           group's value-encode `call-g0` rebuilds both arg tuples then renders the returned List. Driving
           `mk-i`: `call-g0(handle, (5,5), (5,10))` → `(list 5 10)`.")
  (input
    (do
      (def (mk-i) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) #list((. p 0) (. q 1))))
      (def (mk-b) (fn ((: p (Tuple Int32 Int32)) (: q (Tuple Int32 Int32))) (- (. p 0) (. q 1))))
      (export mk-i)
      (export mk-b)))
  (call mk-i (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (drop)
  (output (: #list(5 10) (List Int64)))
  (live-objects 0))

(case
  "DISTINCT-SIG: capturing two-Tuple-arg closures of different signatures"
  (doc
    "Distinct-sig ≥2-compound-args composes with capture: `mk-i (: k Int64)` and `mk-b (: k Int32)` each
           close over `k` AND take two tuple args of their own width. `make-i(100)` → a handle over k=100;
           `call-g0(handle, (5,5), (5,10))` → `p.0 + q.1 + k` = 115. The forwarded capture cell + both rebuilt
           arg cells coexist in the group's `call-g0`.")
  (input
    (do
      (def
        (mk-i (: k Int64))
        (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (+ (. p 0) (. q 1)) k)))
      (def
        (mk-b (: k Int32))
        (fn ((: p (Tuple Int32 Int32)) (: q (Tuple Int32 Int32))) (- (. p 0) k)))
      (export mk-i)
      (export mk-b)))
  (call mk-i (: 100 Int64) (: #tuple(5 5) (Tuple Int64 Int64)) (: #tuple(5 10) (Tuple Int64 Int64)))
  (output (: 115 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: two-Tuple-arg closures of different signatures ALONGSIDE a plain export"
  (doc
    "The distinct-sig ≥2-compound-arg path coexists with a PLAIN (non-closure) export: two closures of
           different tuple-arg signatures cross as two resource types WHILE `twice` rides alongside as an
           ordinary top-level func. Driving the plain export: `twice(21)` → 42.")
  (input
    (do
      (def (mk-i) (fn ((: p (Tuple Int64 Int64)) (: q (Tuple Int64 Int64))) (+ (. p 0) (. q 1))))
      (def (mk-b) (fn ((: p (Tuple Int32 Int32)) (: q (Tuple Int32 Int32))) (- (. p 0) (. q 1))))
      (def (twice (: n Int64)) (* n 2))
      (export mk-i)
      (export mk-b)
      (export twice)))
  (call twice (: 21 Int64))
  (output (: 42 Int64)))

; DIRECT-CALL SUM ARG: a closure whose argument is an `(Option scalar)` crosses the host boundary as a NATIVE
; component `option<payload>` — the canonical ABI FLATTENS it into `(disc: i32, payload)` core params (no
; memory/realloc/runtime decode). The guest `call` rebuilds the sum cell in-guest: branch on the flattened disc
; (the component `option` sends Some=1, None=0 — INDEPENDENT of Cadenza's `(Some a) None` decl order, so the
; guest tests the BOUNDARY disc but builds the cell with the DECL disc), then `sum-new(decl-disc, box payload)`
; for Some / `sum-new(decl-disc, unit)` for None, before dispatching the closure's `match`. This is the first
; host→guest SUM decode; the `option<…>` boundary type is a new component-type former. Scope: a SOLE `(Option
; scalar)` arg, single-export, scalar result (a Result/user-sum arg, or a list result over a sum, are later).
(case
  "a closure taking an Option Int64 ARG — Some crosses the direct-call boundary"
  (doc
    "`mk : () -> (-> (Option Int64) Int64)` returns `(fn (o) (match o ((Some x) x) (None 0)))`. The
           `(Option Int64)` arg crosses as a native `option<s64>` the ABI flattens to `(disc, payload)`; the
           guest rebuilds the sum cell (branch on the boundary disc → `sum-new`) and matches it. `make()` → a
           handle; `call(handle, Some(42))` → 42.")
  (input (do (def (mk) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0)))) (export mk)))
  (call mk (: (Some 42) (Option Int64)))
  (output (: 42 Int64)))

(case
  "a closure taking an Option Int64 ARG — None crosses the direct-call boundary"
  (doc
    "The SAME closure driven with `None`: the boundary option's disc 0 (None) → the guest builds the
           `None` cell (`sum-new(decl-none-disc, unit)`), the match takes the `None` arm → 0.")
  (input (do (def (mk) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0)))) (export mk)))
  (call mk (: None (Option Int64)))
  (output (: 0 Int64)))

(case
  "a closure taking an Option Bool ARG crosses the direct-call boundary"
  (doc
    "The payload need not be Int64: `(Option Bool)` crosses as `option<bool>`, the guest boxes the Bool
           payload (`box-bool`) into the `Some` cell. `call(handle, Some(true))` → `(if b 1 2)` = 1.")
  (input
    (do (def (mk) (fn ((: o (Option Bool))) (match o ((Some b) (if b 1 2)) (None 0)))) (export mk)))
  (call mk (: (Some true) (Option Bool)))
  (output (: 1 Int64)))

(case
  "a closure taking an Option Float64 ARG crosses the direct-call boundary"
  (doc
    "A Float64 payload: `(Option Float64)` crosses as `option<f64>`, the guest boxes the float
           (`box-float`) into the `Some` cell. `call(handle, Some(2.5))` → the payload 2.5.")
  (input
    (do (def (mk) (fn ((: o (Option Float64))) (match o ((Some x) x) (None 0.0)))) (export mk)))
  (call mk (: (Some 2.5) (Option Float64)))
  (output (: 2.5 Float64)))

(case
  "a closure taking an Option Int32 ARG (narrow-int payload) crosses the direct-call boundary"
  (doc
    "A NARROW-int payload: `(Option Int32)` crosses as `option<s32>`; the guest i32→i64-extends the
           flattened payload before `box-int` (a narrow int widens to the boxed i64). `call(handle, Some(7))`
           → 7.")
  (input (do (def (mk) (fn ((: o (Option Int32))) (match o ((Some x) x) (None 0)))) (export mk)))
  (call mk (: (Some 7) (Option Int32)))
  (output (: 7 Int32)))

(case
  "a CAPTURING closure taking an Option Int64 ARG crosses the direct-call boundary"
  (doc
    "The sum-arg path composes with capture (C-HOST-2): a parameterized export `(def (mk (: k Int64)) …)`
           returns a closure that BOTH captures `k` AND takes an `(Option Int64)` arg. `make(100)` → a handle
           closing over k=100; `call(handle, Some(5))` → `(match o ((Some x) (+ x k)) (None k))` = `5 + 100` =
           105. The make-forwarded capture cell + the rebuilt sum-arg cell coexist in the one `call`.")
  (input
    (do
      (def (mk (: k Int64)) (fn ((: o (Option Int64))) (match o ((Some x) (+ x k)) (None k))))
      (export mk)))
  (call mk (: 100 Int64) (: (Some 5) (Option Int64)))
  (output (: 105 Int64)))

(case
  "a CAPTURING closure taking an Option Int64 ARG — None takes the captured value"
  (doc
    "The SAME capturing closure driven with `None`: the `None` arm returns the captured `k`.
           `call(make(100), None)` → 100.")
  (input
    (do
      (def (mk (: k Int64)) (fn ((: o (Option Int64))) (match o ((Some x) (+ x k)) (None k))))
      (export mk)))
  (call mk (: 100 Int64) (: None (Option Int64)))
  (output (: 100 Int64)))

; DIRECT-CALL RESULT ARG: a closure taking a `(Result scalar scalar)` — a TWO-payload sum (Ok a, Err b) —
; crosses as a native component `result<ok, err>` (the `0x6a` former). A general `variant<…>` must be a NAMED
; component type, but `result`/`option` are anonymous-allowed, so `Result` maps to `result<…>`. The canonical
; ABI flattens it to `(disc: i32, payload)` (the payload slot the JOIN of both cases' scalars — same width this
; increment), and the guest branches on the boundary disc (result sends Ok=0, Err=1) then `sum-new(decl-disc,
; box payload)` per arm. Scope: a SOLE `(Result scalar scalar)` arg with same-width ok/err payloads,
; single-export, scalar result (different-width payloads + list results are later widenings).
(case
  "a closure taking a Result Int64 Int64 ARG — Ok crosses the direct-call boundary"
  (doc
    "`mk : () -> (-> (Result Int64 Int64) Int64)` returns `(fn (r) (match r ((Ok x) x) ((Err e) (- 0
           e))))`. The `(Result Int64 Int64)` arg crosses as a native `result<s64,s64>` the ABI flattens to
           `(disc, payload)`; the guest rebuilds the sum cell (branch on the boundary disc → `sum-new`) and
           matches it. `call(handle, Ok(7))` → 7.")
  (input
    (do
      (def (mk) (fn ((: r (Result Int64 Int64))) (match r ((Ok x) x) ((Err e) (- 0 e)))))
      (export mk)))
  (call mk (: (Ok 7) (Result Int64 Int64)))
  (output (: 7 Int64)))

(case
  "a closure taking a Result Int64 Int64 ARG — Err crosses the direct-call boundary"
  (doc
    "The SAME closure driven with `Err(3)`: the boundary result's disc 1 (Err) → the guest builds the
           `Err` cell, the match takes the `Err` arm → `-3` = -3. Both variants carry a payload (unlike
           Option's nullary None), boxed into the rebuilt cell.")
  (input
    (do
      (def (mk) (fn ((: r (Result Int64 Int64))) (match r ((Ok x) x) ((Err e) (- 0 e)))))
      (export mk)))
  (call mk (: (Err 3) (Result Int64 Int64)))
  (output (: -3 Int64)))

(case
  "a closure taking a Result Bool Bool ARG crosses the direct-call boundary"
  (doc
    "A Bool payload on both sides: `(Result Bool Bool)` crosses as `result<bool,bool>`, each arm boxes
           its Bool (`box-bool`). `call(handle, Ok(true))` → `(if b 1 2)` = 1.")
  (input
    (do
      (def (mk) (fn ((: r (Result Bool Bool))) (match r ((Ok b) (if b 1 2)) ((Err b) (if b 3 4)))))
      (export mk)))
  (call mk (: (Ok true) (Result Bool Bool)))
  (output (: 1 Int64)))

(case
  "a CAPTURING closure taking a Result Int64 Int64 ARG crosses the direct-call boundary"
  (doc
    "The result-arg path composes with capture: `(def (mk (: k Int64)) …)` returns a closure that
           captures `k` AND takes a `(Result Int64 Int64)`. `make(100)` → a handle; `call(handle, Ok(5))` →
           `(match r ((Ok x) (+ x k)) …)` = `5 + 100` = 105. The forwarded capture cell + the rebuilt sum-arg
           cell coexist.")
  (input
    (do
      (def
        (mk (: k Int64))
        (fn ((: r (Result Int64 Int64))) (match r ((Ok x) (+ x k)) ((Err e) (- k e)))))
      (export mk)))
  (call mk (: 100 Int64) (: (Ok 5) (Result Int64 Int64)))
  (output (: 105 Int64)))

(case
  "a CAPTURING closure taking a Result Int64 Int64 ARG — Err arm uses the captured value"
  (doc
    "The SAME capturing closure driven with `Err(30)`: the `Err` arm computes `(- k e)` = `100 - 30` =
           70.")
  (input
    (do
      (def
        (mk (: k Int64))
        (fn ((: r (Result Int64 Int64))) (match r ((Ok x) (+ x k)) ((Err e) (- k e)))))
      (export mk)))
  (call mk (: 100 Int64) (: (Err 30) (Result Int64 Int64)))
  (output (: 70 Int64)))

; DIRECT-CALL SUM ARG × MULTI-EXPORT + MIXED: the Option/Result-arg path now composes with the MULTI-EXPORT
; shape (N same-sig closures share ONE `call` taking the sum) and the MIXED shape (a sum-arg closure alongside
; a plain export). The shared `call` rebuilds the sum cell from the flattened `(disc, payload)` (branch on the
; boundary disc → `sum-new`, dispatched through the guest funcref table by the handle's rep), and the shared
; envelope mints the `option<…>`/`result<…>` boundary type via the SAME `ArgSlot` slot model the tuple path
; uses. Scoped to a SCALAR result (a list result over a sum arg on these shapes declines honestly).
(case
  "MULTI-EXPORT: two same-sig Option-arg closures share one `call`"
  (doc
    "`mk-a`/`mk-b : (-> (Option Int64) Int64)` — two same-signature closures each taking an `(Option
           Int64)`, crossing as two `make-<name>`s sharing ONE `call` whose argument is a native `option<s64>`
           (rebuilt in-guest per closure). Driving `mk-a`: `make-a()` → a handle, `call(handle, Some(42))` →
           `(match o ((Some x) x) (None 0))` = 42.")
  (input
    (do
      (def (mk-a) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (mk-b) (fn ((: o (Option Int64))) (match o ((Some x) (+ x 1)) (None -1))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: (Some 42) (Option Int64)))
  (output (: 42 Int64)))

(case
  "MULTI-EXPORT: driving the SECOND Option-arg closure (Some and None)"
  (doc
    "The SAME multi-export component, driving `mk-b`: `call(handle, Some(42))` → `(+ x 1)` = 43. Both
           same-sig Option-arg closures share the one `call` (dispatched by the handle's resource rep).")
  (input
    (do
      (def (mk-a) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (mk-b) (fn ((: o (Option Int64))) (match o ((Some x) (+ x 1)) (None -1))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: (Some 42) (Option Int64)))
  (output (: 43 Int64)))

(case
  "MULTI-EXPORT: two same-sig Result-arg closures share one `call`"
  (doc
    "`mk-a`/`mk-b : (-> (Result Int64 Int64) Int64)` sharing one `call` whose argument is a native
           `result<s64,s64>`. Driving `mk-a` with `Err(3)`: `(match r ((Ok x) x) ((Err e) (- 0 e)))` = -3.")
  (input
    (do
      (def (mk-a) (fn ((: r (Result Int64 Int64))) (match r ((Ok x) x) ((Err e) (- 0 e)))))
      (def (mk-b) (fn ((: r (Result Int64 Int64))) (match r ((Ok x) (+ x 1)) ((Err e) e))))
      (export mk-a)
      (export mk-b)))
  (call mk-a (: (Err 3) (Result Int64 Int64)))
  (output (: -3 Int64)))

(case
  "MIXED: an Option-arg closure ALONGSIDE a plain (non-closure) export"
  (doc
    "The sum-arg path composes with the MIXED shape: a `(-> (Option Int64) Int64)` closure crosses via
           the resource envelope's `make` + shared `call` (rebuilding the native `option<s64>`) WHILE a plain
           export `twice` rides alongside as an ordinary top-level func. Driving the CLOSURE: `make()` →
           handle, `call(handle, Some(42))` → 42.")
  (input
    (do
      (def (mk) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call mk (: (Some 42) (Option Int64)))
  (output (: 42 Int64)))

(case
  "MIXED: driving the PLAIN export alongside an Option-arg closure"
  (doc
    "The SAME mixed component, driving the PLAIN export `twice` — proving it coexists with the Option-arg
           closure interface. `twice(21)` → 42.")
  (input
    (do
      (def (mk) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (twice (: n Int64)) (* n 2))
      (export mk)
      (export twice)))
  (call twice (: 21 Int64))
  (output (: 42 Int64)))

; DIRECT-CALL SUM ARG × DISTINCT-SIGNATURE: the Option/Result-arg path now composes with the DISTINCT-SIG shape
; — closures of DIFFERENT signatures crossing as G distinct resource types, each with its own per-signature
; `call-g<n>`. A group whose closure takes an `(Option/Result scalar)` mints its OWN `option<…>`/`result<…>`
; boundary type (via the per-group `ArgSlot`) and rebuilds the sum cell in its `call-g<n>` — INDEPENDENTLY per
; group, so distinct groups may mix Option and Result, different payload widths, or a sum group beside a
; tuple/scalar group. This CLOSES the sum-arg feature across ALL FOUR export shapes (single/multi/mixed/
; distinct-sig). Scope: scalar result (a list result over a sum arg is a later widening).
(case
  "DISTINCT-SIG: an Option-arg closure + a Result-arg closure each cross the boundary"
  (doc
    "`mk-o : (-> (Option Int64) Int64)` and `mk-r : (-> (Result Int64 Int64) Int64)` — two DIFFERENT-sig
           sum-arg closures crossing as TWO distinct resource types with their own `call-g0`/`call-g1`. Each
           group mints its own boundary type (`option<s64>` for g0, `result<s64,s64>` for g1). Driving `mk-o`:
           `call-g0(handle, Some(42))` → 42.")
  (input
    (do
      (def (mk-o) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (mk-r) (fn ((: r (Result Int64 Int64))) (match r ((Ok x) x) ((Err e) (- 0 e)))))
      (export mk-o)
      (export mk-r)))
  (call mk-o (: (Some 42) (Option Int64)))
  (output (: 42 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: driving the Result-arg group (Err)"
  (doc
    "The SAME distinct-sig component, driving `mk-r` with `Err(3)`: `call-g1(handle, Err(3))` → `(- 0 e)`
           = -3. Confirms the second resource type's `call-g1` rebuilds its own `result<s64,s64>` arg
           independently of group 0's `option<…>`.")
  (input
    (do
      (def (mk-o) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (mk-r) (fn ((: r (Result Int64 Int64))) (match r ((Ok x) x) ((Err e) (- 0 e)))))
      (export mk-o)
      (export mk-r)))
  (call mk-r (: (Err 3) (Result Int64 Int64)))
  (output (: -3 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: two Option-arg closures of DIFFERENT payload widths"
  (doc
    "Distinct groups may take different-width sum payloads: `mk-a : (-> (Option Int64) Int64)` mints
           `option<s64>`, `mk-b : (-> (Option Int32) Int32)` mints `option<s32>`. Driving `mk-b`:
           `call-g1(handle, Some(7))` → 7. Each group's `call-g<n>` flattens + rebuilds its own width.")
  (input
    (do
      (def (mk-a) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (mk-b) (fn ((: o (Option Int32))) (match o ((Some x) x) (None 0))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: (Some 7) (Option Int32)))
  (output (: 7 Int32))
  (live-objects 1))

(case
  "DISTINCT-SIG: an Option-arg group BESIDE a Tuple-arg group"
  (doc
    "A sum-arg group coexists with a tuple-arg group in one distinct-sig component: `mk-o : (-> (Option
           Int64) Int64)` (mints `option<s64>`) and `mk-t : (-> (Tuple Int64 Int64) Int64)` (mints
           `tuple<s64,s64>`). Driving `mk-t`: `call-g1(handle, (3,4))` → `p.0 + p.1` = 7. The two argument-cell
           rebuild kinds (sum vs tuple) coexist across groups.")
  (input
    (do
      (def (mk-o) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (mk-t) (fn ((: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))))
      (export mk-o)
      (export mk-t)))
  (call mk-t (: #tuple(3 4) (Tuple Int64 Int64)))
  (output (: 7 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: two capturing sum-arg closures of different signatures"
  (doc
    "Distinct-sig sum args compose with capture: `mk-o (: k Int64)` and `mk-r (: k Int32)` each close
           over `k` AND take a sum arg of their own shape. `make-o(100)` → a handle over k=100;
           `call-g0(handle, Some(5))` → `(+ x k)` = 105.")
  (input
    (do
      (def (mk-o (: k Int64)) (fn ((: o (Option Int64))) (match o ((Some x) (+ x k)) (None k))))
      (def (mk-r (: k Int32)) (fn ((: r (Result Int32 Int32))) (match r ((Ok x) x) ((Err e) e))))
      (export mk-o)
      (export mk-r)))
  (call mk-o (: 100 Int64) (: (Some 5) (Option Int64)))
  (output (: 105 Int64))
  (live-objects 1))

(case
  "DISTINCT-SIG: sum-arg closures ALONGSIDE a plain export"
  (doc
    "Distinct-sig sum groups coexist with a PLAIN (non-closure) export: an Option-arg + a Result-arg
           closure cross as two resource types WHILE `twice` rides alongside as a top-level func. Driving the
           plain export: `twice(21)` → 42.")
  (input
    (do
      (def (mk-o) (fn ((: o (Option Int64))) (match o ((Some x) x) (None 0))))
      (def (mk-r) (fn ((: r (Result Int64 Int64))) (match r ((Ok x) x) ((Err e) (- 0 e)))))
      (def (twice (: n Int64)) (* n 2))
      (export mk-o)
      (export mk-r)
      (export twice)))
  (call twice (: 21 Int64))
  (output (: 42 Int64)))

; DIFFERENT-WIDTH RESULT ARG: a `(Result ok err)` whose ok/err payloads have DIFFERENT core widths (one i64,
; one i32 — e.g. `(Result Int64 Int32)`) now crosses. The canonical ABI flattens `result<s64,s32>` to `(disc:
; i32, payload: JOIN)` where the payload core valtype is the JOIN of the two sides = the WIDER core (i64). The
; narrow side arrives SIGN-EXTENDED into that joined i64 slot; the guest `call` recovers it with `i32.wrap_i64`
; (its low 32 bits) before the arm's own extend, then boxes it into the sum cell via `sum-new`. The
; `SumArgArm.wrap_join` flag drives this (set on whichever side's core is narrower than the join). Same-width
; Result (both i64, or both i32) reads the join directly (unchanged, byte-neutral). Proven runnable by the
; `a_diff_width_result_scalar_closure_arg_crosses_by_native_flattening` oracle, incl. a NEGATIVE narrow payload.
; Result payload avoids a runtime `T.of` widen (not yet emitted) by returning Bool (`> 0`) — the ARG crossing,
; not the body, is what these exercise.
(case
  "DIFF-WIDTH RESULT: (Result Int64 Int32) arg, drive Ok with a big s64"
  (doc
    "`mk : (-> (Result Int64 Int32) Bool)`. The ok side is s64 (i64 core), the err side s32 (i32 core);
           the flattened payload join is i64. Driving `Ok(5_000_000_000)` (a value that does NOT fit in i32,
           proving the ok side reads the full i64 join directly): `x > 0` → true.")
  (input
    (do
      (def (mk) (fn ((: r (Result Int64 Int32))) (match r ((Ok x) (> x 0)) ((Err e) (> e 0)))))
      (export mk)))
  (call mk (: (Ok 5000000000) (Result Int64 Int32)))
  (output (: true Bool)))

(case
  "DIFF-WIDTH RESULT: (Result Int64 Int32) arg, drive Err with a NEGATIVE narrow payload"
  (doc
    "The SAME closure, driving `Err(-7)` — the narrow s32 err arrives sign-extended into the joined i64;
           the guest `i32.wrap_i64`s to recover it, then the arm re-extends. `e > 0` on `-7` → false. Pins the
           sign is preserved through the wider join (a bug that read the raw join or zero-extended would give
           true).")
  (input
    (do
      (def (mk) (fn ((: r (Result Int64 Int32))) (match r ((Ok x) (> x 0)) ((Err e) (> e 0)))))
      (export mk)))
  (call mk (: (Err -7) (Result Int64 Int32)))
  (output (: false Bool)))

(case
  "DIFF-WIDTH RESULT: (Result Int32 Int64) — the NARROW side is Ok, negative"
  (doc
    "The mirror shape: the narrow (s32, i32-core) side is now `Ok`, the wide (s64) side `Err`. The join
           is still i64; `Ok(-3)` arrives sign-extended and is recovered by `wrap_join` on the OK arm. `x > 0`
           on `-3` → false. Confirms `wrap_join` is set per-arm by which side is narrower, not fixed to `err`.")
  (input
    (do
      (def (mk) (fn ((: r (Result Int32 Int64))) (match r ((Ok x) (> x 0)) ((Err e) (> e 0)))))
      (export mk)))
  (call mk (: (Ok -3) (Result Int32 Int64)))
  (output (: false Bool)))

(case
  "DIFF-WIDTH RESULT: two same-sig (Result Int64 Int32) closures share one call"
  (doc
    "Different-width Result composes with the MULTI-EXPORT shape (N same-sig closures sharing one `call`):
           `mk-a`/`mk-b` both take `(Result Int64 Int32)`. Driving `mk-b` with `Err(-7)`: `e < 0` → true. The
           join + `wrap_join` recovery threads through the shared `call` unchanged.")
  (input
    (do
      (def (mk-a) (fn ((: r (Result Int64 Int32))) (match r ((Ok x) (> x 0)) ((Err e) (> e 0)))))
      (def (mk-b) (fn ((: r (Result Int64 Int32))) (match r ((Ok x) (< x 0)) ((Err e) (< e 0)))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: (Err -7) (Result Int64 Int32)))
  (output (: true Bool)))

(case
  "DIFF-WIDTH RESULT: a (Result Int64 Int32) group + a (Result Int32 Int64) group, distinct-sig"
  (doc
    "Different-width Result composes with the DISTINCT-SIG shape: two closures of DIFFERENT Result
           signatures (opposite narrow sides) cross as two resource types, each minting its own `result<…>`
           boundary type + `wrap_join` recovery. Driving the second group `mk-q : (-> (Result Int32 Int64)
           Bool)` with `Ok(-3)`: `x > 0` → false.")
  (input
    (do
      (def (mk-p) (fn ((: r (Result Int64 Int32))) (match r ((Ok x) (> x 0)) ((Err e) (> e 0)))))
      (def (mk-q) (fn ((: r (Result Int32 Int64))) (match r ((Ok x) (> x 0)) ((Err e) (> e 0)))))
      (export mk-p)
      (export mk-q)))
  (call mk-q (: (Ok -3) (Result Int32 Int64)))
  (output (: false Bool))
  (live-objects 1))

; COMPOUND SUM PAYLOAD: an `(Option compound)` closure arg whose payload is itself a fixed-shape TUPLE/record
; (not a bare scalar) now crosses. It crosses as a native `option<tuple<…>>` — BOTH the `option` and `tuple`
; formers are anonymous-allowed (unlike a general `variant`, which would need a NAMED type export — a separate
; widening), so no naming wall. The canonical ABI flattens `option<tuple<s64,s64>>` to `(disc: i32, f0: i64, f1:
; i64)` — the disc then the payload tuple's OWN recursively-flattened leaves (depth-first, exactly as a bare
; tuple arg flattens). The guest `call` rebuilds the payload cell from those leaves (arr-alloc + box + arr-set,
; recursively for a nested field) then `sum-new`s the Some over that handle; None builds `sum-new(None, unit)`.
; `SumArmPayload::Compound` drives the arm rebuild; `ArgSlot::OptionCompound` mints the boundary type. Composes
; with multi-export + distinct-sig for free. Proven runnable by the `an_option_tuple_payload_closure_arg_
; crosses_by_native_flattening` oracle. (A COMPOUND Result payload / a general user-sum `variant<…>` remain
; later widenings — the latter needs the named-type-export step.)
(case
  "COMPOUND SUM PAYLOAD: (Option (Tuple Int64 Int64)) arg, drive Some"
  (doc
    "`mk : (-> (Option (Tuple Int64 Int64)) Int64)`. The Some payload is a 2-field tuple; the arg crosses
           as `option<tuple<s64,s64>>` flattening to `(disc, f0, f1)`. Driving `Some((3,4))`: the guest rebuilds
           the payload tuple cell, matches `(Some p)`, and folds `p.0 + p.1` → 7.")
  (input
    (do
      (def
        (mk)
        (fn ((: o (Option (Tuple Int64 Int64)))) (match o ((Some p) (+ (. p 0) (. p 1))) (None 0))))
      (export mk)))
  (call mk (: (Some #tuple(3 4)) (Option (Tuple Int64 Int64))))
  (output (: 7 Int64)))

(case
  "COMPOUND SUM PAYLOAD: (Option (Tuple Int64 Int64)) arg, drive None"
  (doc
    "The SAME closure driving `None` → the guest builds `sum-new(None, unit)` (no payload cell) and the
           `(None 0)` arm folds to 0. Pins the nullary arm coexists with the compound Some arm.")
  (input
    (do
      (def
        (mk)
        (fn ((: o (Option (Tuple Int64 Int64)))) (match o ((Some p) (+ (. p 0) (. p 1))) (None 0))))
      (export mk)))
  (call mk (: None (Option (Tuple Int64 Int64))))
  (output (: 0 Int64)))

(case
  "COMPOUND SUM PAYLOAD: (Option (Record (: x Int64) (: y Int64))) — a RECORD payload"
  (doc
    "The payload is a RECORD (fields in canonical sorted-key order) rather than a positional tuple —
           crosses as `option<tuple<s64,s64>>` all the same (a record IS a positional cell; field names are
           compile-time). Driving `Some({x:10, y:20})`: `r.x + r.y` → 30.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: o (Option (Record (: x Int64) (: y Int64)))))
          (match o ((Some r) (+ r.x r.y)) (None -1))))
      (export mk)))
  (call mk (: (Some #record((= x 10) (= y 20))) (Option (Record (: x Int64) (: y Int64)))))
  (output (: 30 Int64)))

(case
  "COMPOUND SUM PAYLOAD: (Option (Tuple Int64 (Tuple Int64 Int64))) — a NESTED tuple payload"
  (doc
    "The Some payload is a NESTED fixed-shape tuple; its leaves flatten DEPTH-FIRST after the disc
           (`option<tuple<s64, tuple<s64,s64>>>` → `(disc, a, b, c)`) and the guest rebuilds the nested cell
           recursively. Driving `Some((1,(2,3)))`: `p.0 + p.1.0 + p.1.1` → 6.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: o (Option (Tuple Int64 (Tuple Int64 Int64)))))
          (match o ((Some p) (+ (. p 0) (+ (. (. p 1) 0) (. (. p 1) 1)))) (None 0))))
      (export mk)))
  (call mk (: (Some #tuple(1 #tuple(2 3))) (Option (Tuple Int64 (Tuple Int64 Int64)))))
  (output (: 6 Int64))
  ; PIN the reclaim: this NESTED-tuple payload regressed to live-objects 2 when
  ; collect_sumpayload_escape_dup_sites (#5833) over-marked `p` as escaping via the borrow-then-dead
  ; compound projection `(. p 1)` (the 177 over-retention). The payload-safety gate suppresses that
  ; spurious dup → reclaims to 0. Pinned so it cannot silently re-regress.
  (live-objects 0))

(case
  "COMPOUND SUM PAYLOAD: mixed-width tuple payload (Int32, Int64, Bool)"
  (doc
    "The payload tuple mixes core widths — `option<tuple<s32,s64,bool>>` flattens to `(disc, i32, i64,
           i32)`. Driving `Some((5, 100, true))`: the `if p.2` picks `p.1` → 100. Pins the per-leaf boxing +
           the bool/narrow-int leaves inside a compound sum payload.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: o (Option (Tuple Int32 Int64 Bool))))
          (match o ((Some p) (if (. p 2) (. p 1) 0)) (None -1))))
      (export mk)))
  (call mk (: (Some #tuple(5 100 true)) (Option (Tuple Int32 Int64 Bool))))
  (output (: 100 Int64)))

(case
  "COMPOUND SUM PAYLOAD: two same-sig Option-tuple closures share one call (multi-export)"
  (doc
    "Compound sum payload composes with the MULTI-EXPORT shape: `mk-a`/`mk-b` both take `(Option (Tuple
           Int64 Int64))` and share one `call`. Driving `mk-b` with `Some((6,7))`: `p.0 * p.1` → 42.")
  (input
    (do
      (def
        (mk-a)
        (fn ((: o (Option (Tuple Int64 Int64)))) (match o ((Some p) (+ (. p 0) (. p 1))) (None 0))))
      (def
        (mk-b)
        (fn ((: o (Option (Tuple Int64 Int64)))) (match o ((Some p) (* (. p 0) (. p 1))) (None 1))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: (Some #tuple(6 7)) (Option (Tuple Int64 Int64))))
  (output (: 42 Int64)))

(case
  "COMPOUND SUM PAYLOAD: an Option-tuple closure beside a scalar closure (distinct-sig)"
  (doc
    "Compound sum payload composes with the DISTINCT-SIG shape: an `(Option (Tuple Int64 Int64))`-arg
           closure crosses as its own resource type beside a plain scalar-arg closure. Driving the Option-tuple
           group with `Some((8,9))`: `p.0 + p.1` → 17.")
  (input
    (do
      (def
        (mk-o)
        (fn ((: o (Option (Tuple Int64 Int64)))) (match o ((Some p) (+ (. p 0) (. p 1))) (None 0))))
      (def (mk-s) (fn ((: n Int64)) (* n 3)))
      (export mk-o)
      (export mk-s)))
  (call mk-o (: (Some #tuple(8 9)) (Option (Tuple Int64 Int64))))
  (output (: 17 Int64))
  (live-objects 1))

; COMPOUND RESULT PAYLOAD: a `(Result ok err)` closure arg where AT LEAST ONE side's payload is a fixed-shape
; TUPLE/record (not a bare scalar) now crosses — the Result counterpart to the compound Option payload. It
; crosses as a native `result<ok, err>` whose ok/err valtypes are each a primitive (scalar side) or a minted
; `tuple<…>` (compound side); both `result` and `tuple` are anonymous-allowed (no `variant` naming wall). The
; canonical ABI flattens it to `(disc: i32, <joined leaves…>)`: each arm's payload flattens to a leaf list and
; the two are JOINED position-by-position (the join length = the LONGER arm; each position's width = the wider
; arm). The guest rebuilds the SELECTED arm's cell over a PREFIX of the joined slots — `SumArmPayload::Compound`
; for a compound arm, `Scalar` for a scalar arm. `ArgSlot::ResultCompound(ResultSide, ResultSide)` mints the
; boundary type. Scope this increment: each shared position has the SAME core width across both arms (a differing
; per-position width would need per-leaf `wrap` inside the compound rebuild — declines cleanly, a later widening).
; Proven runnable by the `a_result_tuple_payload_closure_arg_crosses_by_native_flattening` oracle. Composes with
; multi-export + distinct-sig for free. Bodies return the summed fields (no runtime `T.of`).
(case
  "COMPOUND RESULT PAYLOAD: (Result (Tuple Int64 Int64) Int64) arg, drive Ok (tuple side)"
  (doc
    "`mk : (-> (Result (Tuple Int64 Int64) Int64) Int64)`. The ok payload is a 2-field tuple, the err a
           bare scalar; the arg crosses as `result<tuple<s64,s64>, s64>` flattening to `(disc, j0, j1)` where
           the ok tuple's fields are the joined slots j0,j1 and the err scalar joins into j0. Driving `Ok((3,4))`:
           the guest rebuilds the ok tuple cell, matches `(Ok p)`, folds `p.0 + p.1` → 7.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: r (Result (Tuple Int64 Int64) Int64)))
          (match r ((Ok p) (+ (. p 0) (. p 1))) ((Err e) (- 0 e)))))
      (export mk)))
  (call mk (: (Ok #tuple(3 4)) (Result (Tuple Int64 Int64) Int64)))
  (output (: 7 Int64)))

(case
  "COMPOUND RESULT PAYLOAD: (Result (Tuple Int64 Int64) Int64) arg, drive Err (scalar side)"
  (doc
    "The SAME closure driving `Err(5)` — the scalar err joins into slot j0; the guest reads j0 for the
           `(Err e)` arm (j1 unused) and folds `0 - e` → -5. Pins that the scalar arm reads only its PREFIX of
           the joined slots.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: r (Result (Tuple Int64 Int64) Int64)))
          (match r ((Ok p) (+ (. p 0) (. p 1))) ((Err e) (- 0 e)))))
      (export mk)))
  (call mk (: (Err 5) (Result (Tuple Int64 Int64) Int64)))
  (output (: -5 Int64)))

(case
  "COMPOUND RESULT PAYLOAD: (Result (Tuple Int64 Int64) (Tuple Int64 Int64)) — BOTH sides compound"
  (doc
    "Both arms carry a 2-field tuple; the join is `(disc, j0, j1)` and each arm rebuilds its own tuple
           cell from both slots. Driving `Err((6,7))`: `q.0 * q.1` → 42.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: r (Result (Tuple Int64 Int64) (Tuple Int64 Int64))))
          (match r ((Ok p) (+ (. p 0) (. p 1))) ((Err q) (* (. q 0) (. q 1))))))
      (export mk)))
  (call mk (: (Err #tuple(6 7)) (Result (Tuple Int64 Int64) (Tuple Int64 Int64))))
  (output (: 42 Int64)))

(case
  "COMPOUND RESULT PAYLOAD: (Result Int64 (Tuple Int64 Int64)) — the ERR side is compound"
  (doc
    "The mirror shape: the OK side is a bare scalar, the ERR side a tuple. The join length is the longer
           (err) arm = 2 slots; the ok scalar reads j0. Driving `Err((10,20))`: `q.0 + q.1` → 30. Confirms the
           compound side may be either arm.")
  (input
    (do
      (def
        (mk)
        (fn
          ((: r (Result Int64 (Tuple Int64 Int64))))
          (match r ((Ok x) x) ((Err q) (+ (. q 0) (. q 1))))))
      (export mk)))
  (call mk (: (Err #tuple(10 20)) (Result Int64 (Tuple Int64 Int64))))
  (output (: 30 Int64)))

(case
  "COMPOUND RESULT PAYLOAD: two same-sig closures share one call (multi-export)"
  (doc
    "Compound Result payload composes with the MULTI-EXPORT shape: `mk-a`/`mk-b` both take `(Result (Tuple
           Int64 Int64) Int64)` and share one `call`. Driving `mk-b` with `Ok((6,7))`: `p.0 * p.1` → 42.")
  (input
    (do
      (def
        (mk-a)
        (fn
          ((: r (Result (Tuple Int64 Int64) Int64)))
          (match r ((Ok p) (+ (. p 0) (. p 1))) ((Err e) e))))
      (def
        (mk-b)
        (fn
          ((: r (Result (Tuple Int64 Int64) Int64)))
          (match r ((Ok p) (* (. p 0) (. p 1))) ((Err e) e))))
      (export mk-a)
      (export mk-b)))
  (call mk-b (: (Ok #tuple(6 7)) (Result (Tuple Int64 Int64) Int64)))
  (output (: 42 Int64)))

(case
  "COMPOUND RESULT PAYLOAD: a compound-Result closure beside a scalar closure (distinct-sig)"
  (doc
    "Compound Result payload composes with the DISTINCT-SIG shape: a `(Result (Tuple Int64 Int64) Int64)`-arg
           closure crosses as its own resource type beside a plain scalar-arg closure. Driving the compound-Result
           group with `Ok((8,9))`: `p.0 + p.1` → 17.")
  (input
    (do
      (def
        (mk-r)
        (fn
          ((: r (Result (Tuple Int64 Int64) Int64)))
          (match r ((Ok p) (+ (. p 0) (. p 1))) ((Err e) (- 0 e)))))
      (def (mk-s) (fn ((: n Int64)) (* n 3)))
      (export mk-r)
      (export mk-s)))
  (call mk-r (: (Ok #tuple(8 9)) (Result (Tuple Int64 Int64) Int64)))
  (output (: 17 Int64))
  (live-objects 1))

; SUM ARG + LIST RESULT — a clean DECLINE (a `todo`), NOT a miscompile. A closure that takes a sum
; (Option/Result) argument AND returns a variable-length List/Map/Set (or byte-rope / fixed compound) crosses
; its ARG as a flattened `(disc, payload)` — but the three list-result cores (byte-rope / value-form /
; value-encode) thread a TUPLE-arg rebuild, not a `SumArgRebuild`. Emitting anyway produced an INVALID
; component (the boundary `call` functype carried the sum's flattened params while the core rebuilt no sum
; cell — "lowered parameter types [I32] do not match [I32, I32, I64]"). The compiler now DECLINES cleanly
; rather than emit a module that fails to parse. (Threading sums through the list-result cores — mechanical,
; the envelopes already accept the sum `ArgSlot` — is the next increment; the ARG's ABI is already proven by
; the scalar-result sum cases above.) These cases grade `todo` today and flip to `pass` when it lands.
(case
  "SUM ARG + LIST RESULT declines cleanly (Option arg, List result)"
  (doc
    "`(fn (o) (match o ((Some x) (list x x)) (None (list))))` : `(-> (Option Int64) (List Int64))`. The
           Option arg flattens to `(disc, payload)`, but the List-result core threads tuple rebuilds, not sum
           rebuilds — so the compiler DECLINES (a `todo`) rather than emit an invalid component. Was a
           MISCOMPILE (an unparseable module); the fix makes it an honest decline. Intended value when the
           list-cores thread sums: `Some(5)` → `(list 5 5)`.")
  (input
    (do
      (def (mk) (fn ((: o (Option Int64))) (match o ((Some x) #list(x x)) (None #list()))))
      (export mk)))
  (call mk (: (Some 5) (Option Int64)))
  (output (: (list 5 5) (List Int64))))

; A higher-order closure whose INNER closure has an UNANNOTATED COMPOUND parameter now compiles: the inner
; `(fn (p) …)` param `p` types `Any` bottom-up (no annotation, no def entry), but the higher-order parameter
; `g`'s DECLARED arrow `(-> (-> (Tuple …) R) R)` fixes it — `expected_arrow_for_lambda` recovers the inner
; lambda's expected type from a FUNCTION-VALUED head (a variable of function type), not only a lambda/def
; head. So the inner param solves to `(Tuple …)` (an i32 heap handle), matching the explicit-annotation form.
(case
  "round-trip: a higher-order closure whose inner closure takes an UNANNOTATED compound param"
  (doc
    "`mk : () -> (-> (-> (Tuple Int64 Int64) Int64) Int64)` applies its function arg to `(tuple 3 4)`;
           `app` hands `g` a guest-built `(fn (p) (+ (+ (. p 0) (. p 1)) x))` — the inner param `p` is
           UNANNOTATED. Its type is recovered from `g`'s declared arrow `(-> (Tuple Int64 Int64) Int64)`.
           `app(handle, 10)` → `g((fn p -> p.0+p.1+10))` applied to `(tuple 3 4)` = 3+4+10 = 17. Without the
           context recovery the inner param solved `Any` and declined `a closure's parameter type has no
           machine representation`; now it matches the explicit `(: p (Tuple Int64 Int64))` form.")
  (input
    (do
      (def (mk) (fn ((: f (-> (Tuple Int64 Int64) Int64))) (f #tuple(3 4))))
      (def
        (app (: g (-> (-> (Tuple Int64 Int64) Int64) Int64)) (: x Int64))
        (g (fn (p) (+ (+ (. p 0) (. p 1)) x))))
      (export mk)
      (export app)))
  (call app (: 10 Int64))
  (output (: 17 Int64))
  (live-objects known-leak))

(case
  "round-trip: an UNANNOTATED inner closure with a List param via the context arrow"
  (doc
    "The same context recovery for a variable-length collection param: `mk`'s closure applies its
           function arg to `(list 1 2 3)`; `app` hands `g` a guest-built `(fn (xs) (+ ((. List len) xs) x))`
           whose param `xs` is UNANNOTATED, recovered as `(List Int64)` from `g`'s arrow. `app(handle, 100)` →
           `g((fn xs -> len(xs)+100))` applied to `(list 1 2 3)` = 3 + 100 = 103.")
  (input
    (do
      (def (mk) (fn ((: f (-> (List Int64) Int64))) (f #list(1 2 3))))
      (def
        (app (: g (-> (-> (List Int64) Int64) Int64)) (: x Int64))
        (g (fn (xs) (+ (List.len xs) x))))
      (export mk)
      (export app)))
  (call app (: 100 Int64))
  (output (: 103 Int64))
  (live-objects known-leak))

(case
  "a higher-order closure applied to a GUEST-produced closure arg (should-work)"
  (doc
    "A higher-order closure `(fn (f) (f 10))` applied to a closure argument. The host can never supply
           the `(-> Int64 Int64)` arg over the boundary (every guest is Cadenza; a `(call …)` supplies DATA,
           never behavior), so the witness routes a GUEST-PRODUCED closure `inc` in: `((mk) inc)` = `(inc 10)`
           = 11. Declines today only because higher-order closure params are a deferred build (v-rust-backend
           roadmap; the simple sync closure-param shape already landed). Grades Todo; auto-passes when the
           higher-order-closure boundary lands.")
  (input
    (do
      (def (inc) (fn ((: n Int64)) (+ n 1)))
      (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
      (def (main) ((mk) inc))
      (export main)))
  (call main)
  (output (: 11 Int64)))

; A SUM (Option/Result/user sum) result, and a fixed-shape COMPOUND result CONTAINING a variable-length
; element (a tuple/record with a List/Map/Set inside), cross as `list<u8>` via the runtime `value-encode`
; op against a compiler-baked shape DESCRIPTOR — the same walker a variable-length collection uses,
; generalized. Previously only a scalar, a byte-rope, a FIXED-shape compound (static template), or a bare
; List/Map/Set result crossed; a sum or a nested-collection compound declined "no scalar host-boundary
; representation". This holds on BOTH the direct-call `call` result and the round-trip consumer result.
(case
  "a closure whose CALL returns an Option crosses as the value form"
  (doc
    "`mk : () -> (-> Int64 (Option Int64))` returns `(Some (+ n 1))`; `call(handle, 5)` → `(: (Some 6)
           (Option Int64))`. A SUM closure `call` result renders via the runtime `value-encode` descriptor
           (the disc-switching walker), not a static template.")
  (input (do (def (mk) (fn ((: n Int64)) (Some (+ n 1)))) (export mk)))
  (call mk (: 5 Int64))
  (drop)
  (output (: (Some 6) (Option Int64)))
  (live-objects 0))

(case
  "a closure whose CALL returns a user sum crosses as the value form"
  (doc
    "A monomorphic user sum: `(type Dir (N) (S))`; `mk`'s closure returns `(N)` when `n>0` else `(S)`.
           `call(handle, 5)` → `(: (N unit) Dir)` (a nullary variant carries a unit payload in the canonical
           form). The value-encode walker switches on the runtime discriminant.")
  (input (do (type Dir (N) (S)) (def (mk) (fn ((: n Int64)) (if (> n 0) (N) (S)))) (export mk)))
  (call mk (: 5 Int64))
  (drop)
  (output (: (N unit) Dir))
  (live-objects 0))

(case
  "a closure whose CALL returns a tuple CONTAINING a list"
  (doc
    "A fixed-shape compound whose element is VARIABLE-length has no static template, so it escapes via
           the value-encode descriptor too: `mk`'s closure returns `(tuple (list n n+1) n)`. `call(handle, 5)`
           → `(: (tuple (list 5 6) 5) (Tuple (List Int64) Int64))`. The descriptor's Tuple node recurses into
           the List element.")
  (input (do (def (mk) (fn ((: n Int64)) #tuple(#list(n (+ n 1)) n))) (export mk)))
  (call mk (: 5 Int64))
  (drop)
  (output (: #tuple(#list(5 6) 5) (Tuple (List Int64) Int64)))
  (live-objects 0))

(case
  "round-trip: a consumer returns an Option built from the closure result"
  (doc
    "`mk` adds 1; `app : (own<t>, Int64) -> (Option Int64)` returns `(Some (g x))`. `app(handle, 5)` →
           `g(5)` = 6, so `(: (Some 6) (Option Int64))`. A SUM consumer result on the round-trip path — the
           value-encode descriptor, not a static template.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (Some (g x)))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: (Some 6) (Option Int64))))

(case
  "round-trip: a consumer returns a Result (Err type pinned) from the closure result"
  (doc
    "`mk` doubles; `app : (own<t>, Int64) -> (Result Int64 Int64)` returns `(: (Ok (g x)) (Result Int64
           Int64))` — the `Err` type is fixed by the annotation (an unconstrained `Err` type is genuinely
           ambiguous and correctly declines). `app(handle, 7)` → `(: (Ok 14) (Result Int64 Int64))`.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (* n 2)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (: (Ok (g x)) (Result Int64 Int64)))
      (export mk)
      (export app)))
  (call app (: 7 Int64))
  (output (: (Ok 14) (Result Int64 Int64))))

(case
  "round-trip: a consumer returns a Result reaching BOTH variants"
  (doc
    "Both `Ok` and `Err` are reachable, so the `Result` type is fully determined WITHOUT an annotation:
           `app` returns `(Ok (g x))` when `x>0` else `(Err 99)`. `app(handle, 7)` → `(: (Ok 7) (Result Int64
           Int64))`. Confirms a genuinely two-variant sum consumer result renders.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) n))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (if (> x 0) (Ok (g x)) (Err 99)))
      (export mk)
      (export app)))
  (call app (: 7 Int64))
  (output (: (Ok 7) (Result Int64 Int64))))

(case
  "round-trip: a consumer returns a tuple CONTAINING a list built from the closure result"
  (doc
    "A nested-collection compound consumer result: `app` returns `(tuple (list x (g x)) x)`.
           `app(handle, 5)` → `g(5)` = 6, so `(: (tuple (list 5 6) 5) (Tuple (List Int64) Int64))`. The tuple's
           List element crosses via the same value-encode descriptor (no static template for a variable
           element).")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #tuple(#list(x (g x)) x))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: #tuple(#list(5 6) 5) (Tuple (List Int64) Int64))))

; COMPOSED round-trip shapes — the argument surface (every machine type, incl. higher-order) and the result
; surface (every value-encodable type: scalar, byte-rope, fixed compound, collection, sum, and
; compound-containing-collection) COMPOSE freely, across single-sig and distinct-sig grouping. These lock in
; the full round-trip closure surface end-to-end.
(case
  "round-trip: a consumer returns a Map whose VALUE is a list"
  (doc
    "A `Map Int64 (List Int64)` consumer result — the map's VALUE shape is itself variable-length, so
           the value-encode descriptor recurses through the map value into the nested list. `app` returns
           `(map (0 (list x (g x))) (1 (list x)))`. `app(handle, 5)` → `(: (map (0 (list 5 6)) (1 (list 5)))
           (Map Int64 (List Int64)))` in canonical key order.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #map((= 0 #list(x (g x))) (= 1 #list(x))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: #map((= 0 #list(5 6)) (= 1 #list(5))) (Map Int64 (List Int64)))))

(case
  "round-trip: a consumer returns an Option of a tuple"
  (doc
    "A SUM whose payload is a fixed-shape COMPOUND: `app` returns `(Some (tuple x (g x)))`.
           `app(handle, 5)` → `(: (Some (tuple 5 6)) (Option (Tuple Int64 Int64)))`. The value-encode walker
           switches on the disc, then renders the tuple payload.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (Some #tuple(x (g x))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: (Some #tuple(5 6)) (Option (Tuple Int64 Int64)))))

(case
  "round-trip: a consumer returns a list of tuples from repeated closure application"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects 0)
  (doc
    "A `List (Tuple Int64 Int64)` result — a collection whose ELEMENT is a compound. `app` applies `g`
           to two inputs and pairs each. `mk` doubles; `app(handle, 3)` → `(list (tuple 3 6) (tuple 4 8))`, so
           `(: (list (tuple 3 6) (tuple 4 8)) (List (Tuple Int64 Int64)))`.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (* n 2)))
      (def
        (app (: g (-> Int64 Int64)) (: x Int64))
        #list(#tuple(x (g x)) #tuple((+ x 1) (g (+ x 1)))))
      (export mk)
      (export app)))
  (call app (: 3 Int64))
  (drop)
  (output (: #list(#tuple(3 6) #tuple(4 8)) (List (Tuple Int64 Int64)))))

(case
  "round-trip: a HIGHER-ORDER closure arg composed with a SUM result"
  (doc
    "The argument and result widenings compose: `app : (own<t>, Int64) -> (Option Int64)` applies a
           closure-typed arg (a guest-built inner closure) and wraps the result in `Some`. `mk`'s closure
           applies its function arg to 10; `app(handle, 5)` → `g((fn y -> y+5))` = 15, so `(: (Some 15)
           (Option Int64))`.")
  (input
    (do
      (def (mk) (fn ((: f (-> Int64 Int64))) (f 10)))
      (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (Some (g (fn (y) (+ y x)))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (drop)
  (output (: (Some 15) (Option Int64)))
  (live-objects 0))

(case
  "distinct-sig round-trip: a SUM-result consumer + a COLLECTION-result consumer — the sum one"
  (doc
    "Two distinct signatures, two result MODES: `appa : (own<t0>, Int64) -> (Option Int64)` returns
           `(Some (g x))`; `appb : (own<t1>, Bool) -> (List Int64)` returns `(list (h y) (h y))`.
           `appa(handle, 5)` → `(: (Some 6) (Option Int64))`. A sum result and a collection result of DISTINCT
           signatures coexist, each value-encoded against its own descriptor.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 1 0) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (Some (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) #list((h y) (h y)))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 5 Int64))
  (output (: (Some 6) (Option Int64))))

(case
  "distinct-sig round-trip: a SUM-result consumer + a COLLECTION-result consumer — the collection one"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects 0)
  (doc
    "The SAME two-resource-type program, driving the OTHER (collection-result) consumer of the other
           signature: `appb(handle, true)` → `h(true)` = 1 twice, so `(: (list 1 1) (List Int64))`. Confirms a
           sum-result group and a collection-result group render independently.")
  (input
    (do
      (def (mka) (fn ((: n Int64)) (+ n 1)))
      (def (mkb) (fn ((: b Bool)) (: (if b 1 0) Int64)))
      (def (appa (: g (-> Int64 Int64)) (: x Int64)) (Some (g x)))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) #list((h y) (h y)))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appb (: true Bool))
  (drop)
  (output (: #list(1 1) (List Int64))))

; FINAL COMPOSITION WITNESSES — the closure surface composes across all its axes at once. These exercise
; combinations not covered by the per-feature cases: a higher-order (closure-typed) argument on the
; DISTINCT-SIG round-trip path; a collection result built by REPEATED closure application; and the mixed
; shape (closures + a plain export) driving the plain side. All run end-to-end under wasmtime.
(case
  "distinct-sig round-trip: a higher-order closure-typed arg on one group + a scalar closure on another"
  (doc
    "`mka : () -> (-> (-> Int64 Int64) Int64)` (applies its function arg to 1 and 2, sums) and `mkb : ()
           -> (-> Bool Int64)` are distinct sigs → two resource types. `appa` hands `g` a guest-built `(fn (y)
           (* y x))`. `appa(handle, 5)` → `g((fn y->y*5))` = 5*1 + 5*2 = 15. Composes the higher-order arg with
           distinct-signature grouping.")
  (input
    (do
      (def (mka) (fn ((: f (-> Int64 Int64))) (+ (f 1) (f 2))))
      (def (mkb) (fn ((: b Bool)) (: (if b 9 8) Int64)))
      (def (appa (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (* y x))))
      (def (appb (: h (-> Bool Int64)) (: y Bool)) (h y))
      (export mka)
      (export mkb)
      (export appa)
      (export appb)))
  (call appa (: 5 Int64))
  (output (: 15 Int64))
  (live-objects 1))

(case
  "round-trip: a consumer returns a Set built from REPEATED closure application"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects 0)
  (doc
    "`mk` multiplies by 10; `app : (own<t>, Int64) -> (Set Int64)` = `(Set.of (list (g x) (g x) x))` —
           the closure `g` is applied TWICE and its result plus `x` form a set (duplicates collapse).
           `app(handle, 3)` → `g(3)`=30 twice, so `{3, 30}` → `(: ((. Set of) (list 3 30)) (Set Int64))` in
           canonical order. Composes repeated in-guest application with a collection value-encode result.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (* n 10)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) #set((g x) (g x) x))
      (export mk)
      (export app)))
  (call app (: 3 Int64))
  (drop)
  (output (: #set(3 30) (Set Int64))))

(case
  "mixed: two closure exports alongside a plain export — driving the plain export"
  (doc
    "`inc`/`dbl` are two same-signature closure exports (crossing via `make-<name>` + a shared borrow
           `call`) and `two` is a PLAIN (non-closure) export, all in one component. Calling `two` = 2 drives
           the plain top-level func directly, coexisting with the closure-resource interface. Pins that a
           plain export rides alongside the (now borrow<t>) multi-export closure shape.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (dbl) (fn ((: x Int64)) (* x 2)))
      (def (two) 2)
      (export inc)
      (export dbl)
      (export two)))
  (call two)
  (output (: 2 Int64)))

; A HIGHER-ORDER capture crossing the boundary: a producer whose returned closure CAPTURES another closure
; (built in-guest). The captured inner closure is an ordinary funcref-table value on the heap; the outer
; closure's cell holds it, and the round-trip consumer dispatches the outer via `call_indirect`, which in
; turn dispatches the inner. Only the OUTER handle crosses the host boundary as a resource; the inner closure
; never leaves the guest. (Contrast the `own<t>` TRANSFORMER, still declined: there the HOST supplies the
; inner closure OVER the boundary, which needs a closure-resource passed INTO a call.)
(case
  "round-trip: a producer's returned closure captured an inner closure built in-guest"
  (doc
    "`mk : () -> (-> Int64 Int64)` returns `(fn (x) (let ((f (fn (y) (+ y 1)))) (f (f x))))` — the
           returned closure CAPTURES the inner `f` (a closure) and applies it twice. `app` applies the
           handed-back closure: `app(handle, 5)` → the returned closure on 5 → f(f(5)) = 7. Pins a
           higher-order CAPTURE (a closure whose cell holds another closure) crossing the round-trip boundary,
           dispatched entirely in-guest.")
  (input
    (do
      (def (mk) (fn ((: x Int64)) (let ((f (fn ((: y Int64)) (+ y 1)))) (f (f x)))))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (g x))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: 7 Int64)))

; A SUM whose payload is itself a VARIABLE-LENGTH collection — `Option (List Int64)` — as a round-trip
; consumer result. The value-encode descriptor nests: the sum's disc switch selects the `Some` variant, then
; renders its List payload (element type observable). The deepest result-form nesting witnessed.
(case
  "round-trip: a consumer returns an Option whose payload is a List"
  (doc
    "`mk` adds 1; `app : (own<t>, Int64) -> (Option (List Int64))` returns `(Some (list x (g x)))` — a
           sum wrapping a variable-length collection. `app(handle, 5)` → `g(5)`=6, so `(: (Some (list 5 6))
           (Option (List Int64)))`, value-encoded through the nested descriptor (disc switch → List render).
           Pins a sum-of-collection result form.")
  (input
    (do
      (def (mk) (fn ((: n Int64)) (+ n 1)))
      (def (app (: g (-> Int64 Int64)) (: x Int64)) (Some #list(x (g x))))
      (export mk)
      (export app)))
  (call app (: 5 Int64))
  (output (: (Some #list(5 6)) (Option (List Int64)))))

; The UNIT closure boundary: a closure ARGUMENT or RESULT of type `Unit` has no machine slot
; (`valtype_of(Unit) = None` — Unit occupies no wasm value, so a lifted lambda taking/returning it cannot be
; represented), so it declines at lambda-lift ("a closure's result type has no machine representation"),
; BEFORE the resource envelope. A `Unit`-returning closure is a pure side-effecting callback — only
; meaningful once a closure may perform an effect (which the scope fence CDZ0406 forbids crossing today), so
; there is nothing for it to DO across the boundary. Declines (pinned `(declines)`) — a documented boundary,
; not a miscompile.
(case
  "a closure returning Unit crosses the boundary — unit is a zero-result (should-work)"
  (doc
    "`(def (mk) (fn (x) unit))` — the closure returns `Unit`. Unit IS representable at the boundary: a
           unit-returning function crosses as a ZERO-RESULT (the serializer emits `0x60 <params> <>`; a plain
           `(def (main) unit)` export already crosses + passes on both backends). CDZ0406 does NOT apply — this
           closure is PURE (performs no effect), so it is not an escaping effect-callback, merely vacuous, and
           vacuous is not forbidden (v-rust-backend ruling, correcting the old `no machine representation`
           reason). Declines today only because the closure-LIFT guard does not yet map a Unit closure-result
           to a zero-result functype (the internal-closure path already does). Grades Todo; auto-passes when the
           lift guard is fixed.")
  (input (do (def (mk) (fn ((: x Int64)) unit)) (export mk)))
  (call mk (: 0 Int64))
  (output (: unit Unit)))

; CONTRAST — the INTERNAL boxed Unit-result closure COMPILES. The sound decline above is about EXPORTING
; a Unit-result closure to the HOST (the host `call` boundary needs a scalar result). But a Unit-result
; closure boxed in a GUEST sum, extracted by a match, and applied via `call_indirect` crosses no host
; boundary — it is an ordinary internal runtime closure. `valtype_of(Unit) = None`, but a Unit result is a
; ZERO-RESULT wasm functype (the serializer already emits `0x60 <params> <>` for a Unit-returning
; function), so the lift guard / `closure_type_index` / the unreached-lift stub must map Unit to a
; zero-result functype, NOT decline. Was a MISCOMPILE-adjacent DECLINE (Copilot PR #388): the whole program
; declined "a runtime closure application has no matching function type". (A pure Unit-returning call is
; also dead — no observable result, no escaping effect — so the optimizer may fold the dispatch; the point
; pinned is that the program COMPILES and RUNS rather than declining.)
(case
  "a boxed runtime closure returning Unit applies without declining"
  (doc
    "A closure `(-> Int64 Unit)` boxed in a sum, extracted by a match, then applied — the runtime
           `call_indirect` path (Copilot PR #388). `closure_type_index` did `valtype_of(&result_ty)?`
           which is `None` for `Ty::Unit`, so the whole program declined 'no matching function type' —
           even though the serializer already treats a Unit result as a ZERO-RESULT functype. The fix maps
           a Unit result to a zero-result functype in the lift guard, `closure_type_index`, and the
           unreached-lift stub (a Unit stub body is EMPTY, not `const 0`). `main` runs the boxed Unit
           closure for its (absent) effect, then returns 42. Contrast the sound decline above: EXPORTING a
           Unit-result closure to the HOST still declines (the host `call` needs a scalar); only the
           INTERNAL boxed path compiles.")
  (input
    (do
      (type Box (C (-> Int64 Unit)))
      (def (run (: b Box) (: x Int64)) (match b ((Box.C f) (do (f x) x))))
      (def (ignore (: n Int64)) unit)
      (def (main) (do (run (Box.C ignore) 5) 42))
      (export main)))
  (output (: 42 Int64)))

; The Unit-PARAM face — the canonical lazy THUNK `Susp(Unit -> T)` (the iterators proposal's delayed
; computation). A closure `(-> Unit Int64)` boxed in a sum, extracted by a match, and FORCED (`(f unit)`).
; `valtype_of(Unit) = None`, so the boxed-closure lift guard declined "a closure's parameter type has no
; machine representation" — even though a Unit param, like a Unit result, occupies NO wasm slot. The fix
; ELIDES a Unit param from the closure's functype (mirroring the Unit-result zero-result functype and a
; Unit argument pushing nothing): the lift guard, `select_function_of`'s slot assignment, a `Core::Param`
; read of a Unit binder (emits nothing), `closure_type_index`, and the extra-functype registration
; (`collect_closure_call_sigs`) all drop the Unit param in lockstep. Unlike the Unit-RESULT face, the
; forced call is NOT dead (its result is observed), so a REAL `call_indirect` runs. (A Unit param on a
; CURRIED closure-returning-closure — `(-> Unit (-> A B))` or `(-> A (-> Unit B))` — still declines: that
; boxed-curried path is pre-existing-broken independent of Unit, so eliding there would only expose the
; same "indirect call type mismatch" trap.)
(case
  "a boxed runtime closure taking Unit (a lazy thunk) is forced without declining"
  (doc
    "The lazy-thunk shape `Thunk = Susp(Unit -> Int64)`: `mk` boxes `(fn (u) 42)` into the sum
           variant; `force` matches it out and applies it to `unit`. `valtype_of(Unit) = None`, so the
           boxed-closure lift declined 'a closure's parameter type has no machine representation' — but a
           Unit param occupies NO wasm slot, so it is ELIDED from the closure's functype (like a Unit
           result's zero-result functype and a Unit argument pushing nothing). `force(mk())` runs the
           thunk through a real `call_indirect` → 42. This unblocks the ideal thunk-based lazy `Iter`
           (`Iter a = Susp(Unit -> Option (a, Iter a))`), which today uses a defunctionalized encoding to
           avoid this wall.")
  (input
    (do
      (type Thunk (Susp (-> Unit Int64)))
      (def (force (: t Thunk)) (match t ((Thunk.Susp f) (f unit))))
      (def (mk) (Thunk.Susp (fn ((: u Unit)) 42)))
      (def (main) (force (mk)))
      (export main)))
  (output (: 42 Int64)))

(case
  "closures built one per iteration each capture their OWN loop value"
  (doc
    "N distinct closures from ONE recursive build (vs the pinned one-closure-applied-N-times
           at :600): each iteration pushes `(fn (y) (+ (* 10 i) y))` capturing THAT iteration's i.
           Applying slot j must see i=j — y=1 reads 1/11/21 (231), y=5 reads 5/15/25 (675), y=0
           isolates the captures (120). A capture-by-frame (all closures sharing the final i=3) reads
           31/31/31 (per-slot 31·111 pattern); a capture of the LAST value reads 21/21/21.")
  (input
    (do
      (def
        (build (: i Int64) (: acc (List (-> Int64 Int64))))
        (if (= i 3) acc (build (+ i 1) (List.push acc (fn ((: y Int64)) (+ (* 10 i) y))))))
      (def
        (main (: y Int64))
        (do
          (def fs (build 0 #list()))
          (def (app (: j Int64)) (match (List.at fs j) ((Some f) (f y)) ((None _u) -1)))
          (+ (* 100 (app 0)) (+ (* 10 (app 1)) (app 2)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 231 Int64))
  (call main (: 5 Int64))
  (output (: 675 Int64))
  (call main (: 0 Int64))
  (output (: 120 Int64))
  (live-objects 0))

(case
  "closures capture successive SNAPSHOTS of a growing heap list"
  (doc
    "The heap-snapshot companion of the per-iteration scalar capture: each build iteration
           captures the list AS IT WAS (persistence through capture) before pushing — f0 sees len 0,
           f1 len 1, f2 len 2, applied AFTER the build completes (y=0 -> 012 = 12, y=7 -> 789). A
           capture holding a shared reference to the final list reads 3/3/3 (333+y·111); one
           snapshotting AFTER the push reads 1/2/3. The persistent-list copy-on-write is what makes
           the by-value capture cheap — this pins that the closure env actually gets the snapshot.")
  (input
    (do
      (def
        (build (: i Int64) (: xs (List Int64)) (: acc (List (-> Int64 Int64))))
        (if
          (= i 3)
          acc
          (build (+ i 1) (List.push xs i) (List.push acc (fn ((: y Int64)) (+ (List.len xs) y))))))
      (def
        (main (: y Int64))
        (do
          (def fs (build 0 #list() #list()))
          (def (app (: j Int64)) (match (List.at fs j) ((Some f) (f y)) ((None _u) -1)))
          (+ (* 100 (app 0)) (+ (* 10 (app 1)) (app 2)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 12 Int64))
  (call main (: 7 Int64))
  (output (: 789 Int64))
  (live-objects known-leak))

(case
  "closures stored as MAP VALUES dispatch by key with distinct captures"
  (doc
    "The dispatch-table shape — closures as CHAMP map VALUES (the collection pins cover
           closures RETURNING collections; storing them IN one crosses the CHAMP value slot +
           Option projection before the call): {1 -> double, 2 -> add-100}, looked up and applied —
           y=5 reads 10 and 105 (10105), y=0 isolates the bodies (100). Each closure must come back
           out of the map with ITS OWN code and env — a value slot that unified same-signature
           closures (or dropped the env on the way in) answers with the wrong arm.")
  (input
    (do
      (def
        (main (: y Int64))
        (do
          (def m #map((= 1 (fn ((: v Int64)) (* v 2))) (= 2 (fn ((: v Int64)) (+ v 100)))))
          (def (app (: k Int64)) (match (Map.lookup m k) ((Some f) (f y)) ((None _u) -1)))
          (+ (* 1000 (app 1)) (app 2))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10105 Int64))
  (call main (: 0 Int64))
  (output (: 100 Int64))
  (live-objects 0))

(case
  "a closure returned from a match arm carries the arm's HEAP payload binding"
  (doc
    "The payload-binder capture face: the closure is built INSIDE a `(Some s)` arm, capturing the
           extracted rope payload, and OUTLIVES the match — its env must carry the heap value out of
           the arm's scope (mode 1: len \"abcde\" + y -> 65 encoded over two applications; mode 0 the
           None arm's closure negates -> -10). The payload lives only as long as the arm unless the
           env takes ownership — a by-reference capture of the scrutinee's payload slot reads freed
           or wrong bytes after the match ends (the #20-UAF family's closure face, now from the
           PATTERN side).")
  (input
    (do
      (def
        (mk (: mode Int64))
        (match
          (if (> mode 0) (Some (String.concat "ab" "cde")) (None unit))
          ((Some s) (fn ((: y Int64)) (+ (String.byte-len s) y)))
          ((None _u) (fn ((: y Int64)) (- 0 y)))))
      (def (main (: mode Int64)) (do (def f (mk mode)) (+ (* 10 (f 1)) (f 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 65 Int64))
  (call main (: 0 Int64))
  (output (: -10 Int64))
  (live-objects 0))

; --- A closure env carrying TWO collection handles across the host boundary. ---
(case
  "a host-crossing closure captures a CHAMP map and set and reads both per call"
  (doc
    "The capture family holds scalars/lists/snapshot-lists — never a CHAMP Map (rope value) AND a Set together: the env carries two collection handles across the resource crossing + repeated borrow calls (a drop-on-first-call or env-slot confusion breaks calls 2/3). Faces: map-hit+set-miss (300), map-miss+set-hit (-99), both-miss (-100).")
  (input
    (do
      (def
        (main (: seed Int64))
        (do
          (def m #map((= 1 (String.concat "on" "e")) (= 2 "two")))
          (def s #set(seed 20 30))
          (fn
            ((: k Int64))
            (+
              (*
                100
                (match (Map.lookup m k) ((Option.Some v) (String.byte-len v)) ((Option.None _u) -1)))
              (if (Set.contains s k) 1 0)))))
      (export main)))
  (call main (: 10 Int64) (: 1 Int64))
  (output (: 300 Int64))
  (call main (: 10 Int64) (: 10 Int64))
  (output (: -99 Int64))
  (call main (: 10 Int64) (: 5 Int64))
  (output (: -100 Int64))
  (live-objects known-leak))

(case
  "a performing closure stored in a TUPLE and applied IN-GUEST fires normally"
  (doc
    "The legal-side contrast of the CDZ0406 escape family (whose reject witnesses cover the bare,
           tuple-nested, and let-bound ESCAPING shapes above/nearby): the SAME tuple-stored performing
           closure, extracted with `(. pair 1)` and applied entirely IN-GUEST, is not an escape — the
           application sits inside the delegation's dynamic extent, so the effect homes and fires exactly
           once (10+3=13, one host call). Pins the in-guest/escape LINE from the working side: compound
           storage of an effectful closure is legal; only crossing the host boundary is fenced. A scan
           that keyed on 'effectful closure inside a compound' rather than on ESCAPE would false-reject
           this.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def
        (main (: k Int64))
        (host (io) (let ((pair #tuple(99 (fn ((: x Int64)) (+ x (io.get)))))) ((. pair 1) k))))
      (export main)))
  (host-responses (respond io.get (: 3 Int64)))
  (host-calls (call io.get))
  (call main (: 10 Int64))
  (output (: 13 Int64)))

; Repeatable borrow<t> across MORE closure shapes (the (then) two-call drive): the single-export adder
; repeatable case above proves the scalar bare-export shape; these extend it to a compound-result export,
; a same-signature multi-export shared call, a multi-export value-form (compound) shared call, and a
; distinct-signature per-group call-g. Each makes ONE handle then calls it TWICE via (then), rendering
; (tuple r1 r2) on wasm; (then) cleanly declines (todo) on rust/rust-async (the two-call drive is wasm-only).
(case
  "a compound-result borrowed closure handle is called twice on the same handle (repeatable)"
  (doc
    "`pair(100)` captures k=100; the closure `(fn (x) (tuple x (+ x k)))` returns a COMPOUND. make(100)
           once, then call(5) TWICE on the SAME borrowed handle -> each yields (tuple 5 105); repeatability
           renders (tuple (tuple 5 105) (tuple 5 105)). An own<t> cell would be consumed on the first call.")
  (input (do (def (pair (: k Int64)) (fn ((: x Int64)) #tuple(x (+ x k)))) (export pair)))
  (call pair (: 100 Int64) (: 5 Int64))
  (then (: 5 Int64))
  (output (: (tuple (tuple 5 105) (tuple 5 105)) (Tuple (Tuple Int64 Int64) (Tuple Int64 Int64))))
  (live-objects known-leak))

(case
  "a same-signature multi-export shared call is repeatable on one make-<name> handle"
  (doc
    "Two same-signature closure exports `inc`/`triple` share one `call`. make-inc() once, then the
           shared call(5)=6 then call(40)=41 on the SAME borrowed handle -> (tuple 6 41). An own<t> shared
           call would trap on the second.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (triple) (fn ((: x Int64)) (* x 3)))
      (export inc)
      (export triple)))
  (call inc (: 5 Int64))
  (then (: 40 Int64))
  (output (: (tuple 6 41) (Tuple Int64 Int64)))
  (live-objects known-leak))

(case
  "a multi-export VALUE-FORM shared call is repeatable on one make-<name> handle"
  (doc
    "Two same-signature tuple-returning exports `lo`/`hi` share one value-form list-call. make-lo()
           once, then the shared call(5) TWICE -> each (tuple 5 6); repeatability renders (tuple (tuple 5 6)
           (tuple 5 6)).")
  (input
    (do
      (def (lo) (fn ((: x Int64)) #tuple(x (+ x 1))))
      (def (hi) (fn ((: x Int64)) #tuple(x (* x 10))))
      (export lo)
      (export hi)))
  (call lo (: 5 Int64))
  (then (: 5 Int64))
  (output (: (tuple (tuple 5 6) (tuple 5 6)) (Tuple (Tuple Int64 Int64) (Tuple Int64 Int64))))
  (live-objects known-leak))

(case
  "a distinct-signature per-group call-g is repeatable on one make-<name> handle"
  (doc
    "Two distinct-signature closures `inc : Int64->Int64` and `isz : Int64->Bool` cross as two resource
           types. make-inc() once, then its per-group call-g(5)=6 then call-g(40)=41 on the SAME borrowed
           handle -> (tuple 6 41). An own<t_g> would consume it on the first call.")
  (input
    (do
      (def (inc) (fn ((: x Int64)) (+ x 1)))
      (def (isz) (fn ((: x Int64)) (= x 0)))
      (export inc)
      (export isz)))
  (call inc (: 5 Int64))
  (then (: 40 Int64))
  (output (: (tuple 6 41) (Tuple Int64 Int64)))
  (live-objects known-leak))

; VALUE-RESOURCE METHOD (VM-1) — a RUNTIME VALUE crossing as a resource exposes compiler-EMITTED members
; besides `encode`: a `Bytes` value's `len : borrow<t> -> u32` (and `is-empty`/`to-bytes`). The host makes
; the value once, then reaches a NAMED member via `(call-method <member>)` and calls it. A borrow method is
; REPEATABLE — `(then)` calls `len` again on the SAME handle, rendering the pair as a tuple. `(uleb 624485)`
; builds a genuine RUNTIME Bytes `E5 8E 26` (3 bytes) via a recursive LEB128 concat — a FOLDABLE constant
; Bytes emits NO value-resource `len` member (only a runtime value does), so the recursive builder is
; load-bearing. `len` = 3 both times. The recursive concat leaves 5 live cells (intermediates + result).
(case
  "a runtime Bytes value exposes a repeatable len member (call-method)"
  (doc
    "`(uleb 624485)` builds the runtime Bytes `E5 8E 26`; the host reaches its emitted `len` member and
           calls it TWICE on the same borrowed handle (repeatable) -> (tuple 3 3). Pins the value-resource
           METHOD ABI (a named member besides encode), driven by (call-method len) + (then).")
  (input
    (do
      (def
        (uleb (: n UInt64))
        (if
          (< n 128)
          (Bytes.of #list((UInt8.wrap n)))
          (Bytes.concat (Bytes.of #list((UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
      (def (main) (uleb 624485))
      (export main)))
  (call-method len)
  (then)
  (output (: (tuple 3 3) (Tuple UInt32 UInt32)))
  (live-objects known-leak))

(case
  "a runtime Bytes value exposes a repeatable is-empty member (call-method)"
  (doc
    "The 3-byte runtime Bytes `(uleb 624485)` = E5 8E 26 reached via its emitted `is-empty` member
           -> false (a non-empty Bytes). A scalar borrow method besides len.")
  (input
    (do
      (def
        (uleb (: n UInt64))
        (if
          (< n 128)
          (Bytes.of #list((UInt8.wrap n)))
          (Bytes.concat (Bytes.of #list((UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
      (def (main) (uleb 624485))
      (export main)))
  (call-method is-empty)
  (output (: false Bool))
  (live-objects known-leak))

(case
  "a runtime Bytes value exposes a to-bytes member returning the raw payload (call-method)"
  (doc
    "`to-bytes` returns the RAW payload (no value-form framing) of the 3-byte runtime Bytes
           `(uleb 624485)` = E5 8E 26.")
  (input
    (do
      (def
        (uleb (: n UInt64))
        (if
          (< n 128)
          (Bytes.of #list((UInt8.wrap n)))
          (Bytes.concat (Bytes.of #list((UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
      (def (main) (uleb 624485))
      (export main)))
  (call-method to-bytes)
  (output #list(229 142 38))
  (live-objects known-leak))

(case
  "a runtime Bytes value's encode member still renders after other member calls (call-method)"
  (doc
    "`encode` returns the Value.encode value-form of the 3-byte runtime Bytes `(uleb 624485)`
           = E5 8E 26 -> `(: b\"\\xe5\\x8e&\" Bytes)`.")
  (input
    (do
      (def
        (uleb (: n UInt64))
        (if
          (< n 128)
          (Bytes.of #list((UInt8.wrap n)))
          (Bytes.concat (Bytes.of #list((UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
      (def (main) (uleb 624485))
      (export main)))
  (call-method encode)
  (output (: b"\xe5\x8e&" Bytes))
  (live-objects known-leak))

; VM-2 — the value-resource's OTHER emitted members: `is-empty : borrow<t> -> bool` and
; `to-bytes : borrow<t> -> list<u8>` (besides `len`/`encode`, backend/wasm/mod.rs). Same `(call-method
; <member>)` drive, different result shapes: a bool (rendered directly) and a raw byte sequence (rendered
; as the bare list, not a value-form). Pins that a value-resource member of a NON-u32 result type crosses.
(case
  "a runtime Bytes value's is-empty member crosses (call-method, bool result)"
  (doc
    "`(uleb 624485)` = the 3-byte Bytes `E5 8E 26`; `(call-method is-empty)` reaches its emitted
           `is-empty` member -> false (non-empty). Pins a value-resource member with a Bool result.")
  (input
    (do
      (def
        (uleb (: n UInt64))
        (if
          (< n 128)
          (Bytes.of #list((UInt8.wrap n)))
          (Bytes.concat (Bytes.of #list((UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
      (def (main) (uleb 624485))
      (export main)))
  (call-method is-empty)
  (output (: false Bool))
  (live-objects known-leak))

(case
  "a runtime Bytes value's to-bytes member crosses (call-method, raw bytes result)"
  (doc
    "`(uleb 624485)` = the 3-byte Bytes `E5 8E 26`; `(call-method to-bytes)` reaches its emitted
           `to-bytes` member -> the raw byte sequence 229 142 38. Pins a value-resource member with a raw
           list<u8> result (rendered as the bare byte list, not a decoded value-form).")
  (input
    (do
      (def
        (uleb (: n UInt64))
        (if
          (< n 128)
          (Bytes.of #list((UInt8.wrap n)))
          (Bytes.concat (Bytes.of #list((UInt8.wrap (| (& n 127) 128)))) (uleb (>> n 7)))))
      (def (main) (uleb 624485))
      (export main)))
  (call-method to-bytes)
  (output #list(229 142 38))
  (live-objects known-leak))

; ── breaker batch 566: the host-closure × immortal-era campaign opens. hcp1-3 = the green capture
; cells with truthful census (whole-tuple return; immortal-trie + scalar captures; runtime-list
; capture — the host-held handle retains the closure cell + mortal captures, never dropped by the
; single-call harness). hcx1 = the MINIMAL no-local-slot ICE (tuple-index projection of a captured
; tuple in the body; effects-free — the chr1 ICE's second face), tracked known-FAIL until the
; closure-conversion slot fix lands (see issues/BUG-captured-tuple-projection…).
; hcp1 is hcz1's SAME program minus the (drop) — the capture-escape read-site dup (hcz fix,
; select.rs collect_captured_escape_dup_sites) fires here too (a compound capture returned once),
; giving the returned tuple an independent ref. With NO drop the env cell is never reclaimed, so it
; retains that ref → the tuple leaks alongside the cell: known-leak 1 → 2. This is the CORRECT
; ownership accounting (before the dup the leaked cell held a DANGLING ref to the host-freed tuple);
; the same dup makes hcz1 (the (drop) twin) reclaim to 0. The compiler cannot condition the dup on a
; runtime drop the guest code does not encode, so the escape dup is unconditional (leak-beats-UAF).
(case
  "hcp1 a captured tuple returned WHOLE from a host-called closure works (projection is the ICE, not the capture)"
  (input (do (def (f (: n Int64)) (let ((a #tuple(n 7))) (fn ((: q Int64)) a))) (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: (tuple 1 7) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "hcp2 a closure capturing an IMMORTAL 33-trie plus a runtime scalar crosses and reads through the immortal"
  (input
    (do
      (def
        (reader (: n Int64))
        (let
          ((c
              #list(1
                2
                3
                4
                5
                6
                7
                8
                9
                10
                11
                12
                13
                14
                15
                16
                17
                18
                19
                20
                21
                22
                23
                24
                25
                26
                27
                28
                29
                30
                31
                32
                33)))
          (fn ((: i Int64)) (+ n (match (List.at c i) ((Option.Some v) v) ((Option.None) -1))))))
      (export reader)))
  (call reader (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: 106 Int64))
  (live-objects 0))

(case
  "hcp3 a closure capturing a runtime-BUILT list crosses and reads it (mortal capture retained by the host-held handle)"
  (input
    (do
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def
        (holder (: n Int64))
        (let
          ((xs (bld n)))
          (fn
            ((: i Int64))
            (+ (List.len xs) (match (List.at xs i) ((Option.Some v) v) ((Option.None) -1))))))
      (export holder)))
  (call holder (: 4 Int64) (: 1 Int64))
  (drop)
  (output (: 6 Int64))
  (live-objects 0))

(case
  "hcx1 tuple-index projection of a captured tuple in a host-called closure body FOLDS — the projection reads the captured tuple env cell, not the inlined element (was the chr1 ICE's effects-free face: no-local-slot)"
  (input
    (do (def (f (: n Int64)) (let ((a #tuple(n 7))) (fn ((: q Int64)) (+ q (. a 0))))) (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: 6 Int64))
  (live-objects 0))

(case
  "hcx2 a NESTED tuple-index projection of a captured tuple in a host-called closure body FOLDS — the projection chain stays runtime over the captured env cell (the nested face of the hcx1 no-local-slot ICE)"
  (input
    (do
      (def (f (: n Int64)) (let ((a #tuple(#tuple(n 1) 7))) (fn ((: q Int64)) (+ q (. (. a 0) 0)))))
      (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (output (: 6 Int64))
  (live-objects known-leak))

; ── breaker batch 567: drop-cascade fences + the capture-escape double-release (see
; issues/BUG-closure-drop-after-capture-escape-double-release-silent-abort). hcd1/hcd2 pin the
; CORRECT cascade: dropping the handle reclaims mortal captures and no-ops immortal ones. hcz1/
; hcz2 pin the ESCAPE-then-drop face — a body that RETURNED its captured compound + (drop). Was a
; double-release (silent abort); FIXED by the read-site capture dup (a compound capture read once +
; escaping dup's at its Core::Captured read so the returned ref is independent of the env-cell drop
; — select.rs collect_captured_escape_dup_sites). Now pass with 0-census. (A MULTI-read escaping
; compound capture is a tracked residual — needs per-occurrence marking; not yet exercised.)
(case
  "hcd1 dropping a closure whose capture is a runtime-BUILT list cascades the reclaim"
  (input
    (do
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def
        (holder (: n Int64))
        (let
          ((xs (bld n)))
          (fn
            ((: i Int64))
            (+ (List.len xs) (match (List.at xs i) ((Option.Some v) v) ((Option.None) -1))))))
      (export holder)))
  (call holder (: 4 Int64) (: 1 Int64))
  (drop)
  (output (: 6 Int64))
  (live-objects 0))

(case
  "hcd2 dropping a closure capturing an IMMORTAL trie + scalar reclaims the mortal env and no-ops the immortal"
  (input
    (do
      (def
        (reader (: n Int64))
        (let
          ((c
              #list(1
                2
                3
                4
                5
                6
                7
                8
                9
                10
                11
                12
                13
                14
                15
                16
                17
                18
                19
                20
                21
                22
                23
                24
                25
                26
                27
                28
                29
                30
                31
                32
                33)))
          (fn ((: i Int64)) (+ n (match (List.at c i) ((Option.Some v) v) ((Option.None) -1))))))
      (export reader)))
  (call reader (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: 106 Int64))
  (live-objects 0))

(case
  "hcz1 dropping a closure whose body RETURNED its captured TUPLE reclaims cleanly (read-site dup balances the env-cell drop)"
  (input (do (def (f (: n Int64)) (let ((a #tuple(n 7))) (fn ((: q Int64)) a))) (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: (tuple 1 7) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "hcz2 dropping a closure whose body RETURNED its captured LIST reclaims cleanly (read-site dup balances the env-cell drop)"
  (input
    (do
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def (h (: n Int64)) (let ((xs (bld n))) (fn ((: q Int64)) xs)))
      (export h)))
  (call h (: 3 Int64) (: 5 Int64))
  (drop)
  (output (: #list(1 2 3) (List Int64)))
  (live-objects 0))

(case
  "hcz3 dropping a closure whose body RETURNED its captured MAP reclaims cleanly"
  (doc
    "The Map face of the hcz1/hcz2 escape-then-drop pair (breaker flip-watch on #5007): the read-site
        capture dup must balance the env-cell drop for a CHAMP capture exactly as for tuple/list — a
        double-release here corrupts the returned map's shared nodes. Renders canonically key-sorted.")
  (input
    (do (def (f (: n Int64)) (let ((m #map((= n 10) (= 0 20)))) (fn ((: q Int64)) m))) (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: #map((= 0 20) (= 1 10)) (Map Int64 Int64)))
  (live-objects 0))

(case
  "hcz4 dropping a closure whose body RETURNED its captured SET reclaims cleanly"
  (doc "The Set twin of hcz3 — same read-site-dup balance over the set CHAMP.")
  (input (do (def (f (: n Int64)) (let ((s #set(n 5))) (fn ((: q Int64)) s))) (export f)))
  (call f (: 1 Int64) (: 9 Int64))
  (drop)
  (output (: #set(1 5) (Set Int64)))
  (live-objects 0))

(case
  "hcz5 dropping a closure whose body RETURNED its captured WRAPPED compound (record holding a tuple) reclaims cleanly"
  (doc
    "The wrapped face: the capture is a record whose field is itself heap (a tuple) — the escape dup
        and the drop cascade must balance through the nesting (a single-level dup would double-release the
        inner tuple). Completes the hcz escape-then-drop faces except STRING (a returned captured String
        currently mis-renders as a bare byte tuple — routed to v-rust-backend; pin it on that fix).")
  (input
    (do
      (def (f (: n Int64)) (let ((r #record((= t #tuple(n 7))))) (fn ((: q Int64)) r)))
      (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: (record (= t (tuple 1 7))) (Record (: t (Tuple Int64 Int64)))))
  (live-objects 0))

; ── hczm/ifcap: PER-OCCURRENCE capture escape-dup (#5857 Increment A, Perceus borrowed-param rule).
; A captured heap value that ESCAPES the closure body in N>1 positions needs one dup PER escaping
; occurrence (not one total): the monolithic closure-cell drop then nets each escaped ref to a live
; rc. The old collector punted `occs.len() != 1` → ZERO dups for a multi-escape capture → the cell
; drop freed the shared capture while the result still held N refs (over-free, release `unreachable`).
; ifcap1 is the anti-OVER-dup control: two syntactic escapes across mutually-exclusive if-arms, but
; the dup is placed AT the occurrence (inside the arm), so only the taken arm's dup fires → exactly
; one dup per dynamic path (a flat 2-dup count would LEAK the untaken arm's dup here).
(case
  "hczm1 a captured TUPLE escaping TWICE in the returned value reclaims cleanly (per-occurrence escape-dup)"
  (input
    (do (def (f (: n Int64)) (let ((a #tuple(n 7))) (fn ((: q Int64)) #tuple(a a)))) (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: (tuple (tuple 1 7) (tuple 1 7)) (Tuple (Tuple Int64 Int64) (Tuple Int64 Int64))))
  (live-objects 0))

(case
  "hczm2 a captured TUPLE READ once (projection) AND escaping once reclaims cleanly"
  (input
    (do
      (def (f (: n Int64)) (let ((a #tuple(n 7))) (fn ((: q Int64)) #tuple((. a 0) a))))
      (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: (tuple 1 (tuple 1 7)) (Tuple Int64 (Tuple Int64 Int64))))
  (live-objects 0))

(case
  "hczm3 a captured LIST escaping TWICE reclaims cleanly (vec-rep twin of hczm1)"
  (input
    (do
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def (h (: n Int64)) (let ((xs (bld n))) (fn ((: q Int64)) #tuple(xs xs))))
      (export h)))
  (call h (: 3 Int64) (: 5 Int64))
  (drop)
  (output (: #tuple(#list(1 2 3) #list(1 2 3)) (Tuple (List Int64) (List Int64))))
  (live-objects 0))

(case
  "hczm4 a captured MAP escaping TWICE reclaims cleanly (CHAMP twin)"
  (input
    (do
      (def (f (: n Int64)) (let ((m #map((= n 10) (= 0 20)))) (fn ((: q Int64)) #tuple(m m))))
      (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output
    (:
      #tuple(#map((= 0 20) (= 1 10)) #map((= 0 20) (= 1 10)))
      (Tuple (Map Int64 Int64) (Map Int64 Int64))))
  (live-objects 0))

(case
  "ifcap1 a captured TUPLE escaping via BOTH mutually-exclusive if-arms reclaims cleanly (anti-over-dup control: one dup per PATH, not two)"
  (doc
    "Two syntactic escapes of `a`, one per if-arm. Only the taken arm runs, so exactly one dup fires
        at runtime — the dup is placed at the occurrence node, inside its arm. A flat per-occurrence
        count (2 dups) would leak the untaken arm's orphaned dup; per-occurrence PLACEMENT gives the
        MAX-over-arms behaviour for free. live-objects 0 proves it is one dup, not two.")
  (input
    (do
      (def (f (: n Int64)) (let ((a #tuple(n 7))) (fn ((: q Int64)) (if (> q 0) a a))))
      (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: (tuple 1 7) (Tuple Int64 Int64)))
  (live-objects 0))

(case
  "ifcap2 a captured TUPLE escaping via ONE if-arm only reclaims cleanly"
  (input
    (do
      (def (f (: n Int64)) (let ((a #tuple(n 7))) (fn ((: q Int64)) (if (> q 0) a #tuple(0 0)))))
      (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: (tuple 1 7) (Tuple Int64 Int64)))
  (live-objects 0))

; ── breaker batch 568: nested-closure + CHAMP-capture cells (campaign cells 3-4; the tuple-capture
; projection cells stay blocked on the hcx1 ICE). A closure capturing ANOTHER closure dispatches
; through it; a CHAMP (Map) capture serves lookups by the call arg; and the handle drop cascades
; through the CHAMP capture to zero.
(case
  "hcn1 a closure CAPTURING another closure crosses and dispatches through it"
  (input
    (do
      (def (f (: k Int64)) (let ((g (fn ((: x Int64)) (+ x k)))) (fn ((: y Int64)) (g (* y 2)))))
      (export f)))
  (call f (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: 110 Int64))
  (live-objects 0))

; hcn4/hcn5 (breaker, #8239-adjacent): the ownership-conditional CallClosure relax (#8239 — applying a
; BORROWED captured closure does not escape it, closing the hcn1 leak) generalizes to MULTIPLE and
; CONDITIONAL application sites. hcn4 applies the borrowed inner closure `g` TWICE in one body; hcn5
; applies it in ONE if-arm only. Both still reclaim to zero after the outer closure is dropped (census
; discriminating: live-objects 1 FAILs both). Guards the relax against a regression that re-escapes on a
; second or a conditionally-reached application site. wasm-only, like the rest of the hcn family (the
; closure-capture heap shape declines on rust + the cadenza hop — the known non-blocking value-heap gap).
(case
  "hcn4 a borrowed captured closure applied TWICE in one body reclaims"
  (input
    (do
      (def (f (: k Int64)) (let ((g (fn ((: x Int64)) (+ x k)))) (fn ((: y Int64)) (+ (g y) (g (* y 2))))))
      (export f)))
  (call f (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: 215 Int64))
  (live-objects 0))

(case
  "hcn5 a borrowed captured closure applied in ONE if-arm reclaims"
  (input
    (do
      (def (f (: k Int64)) (let ((g (fn ((: x Int64)) (+ x k)))) (fn ((: y Int64)) (if (> y 0) (g y) 0))))
      (export f)))
  (call f (: 100 Int64) (: 5 Int64))
  (drop)
  (output (: 105 Int64))
  (live-objects 0))

(case
  "hcn2 a closure capturing a runtime-built MAP looks up by the call argument"
  (input
    (do
      (def
        (bld (: i Int64) (: m (Map Int64 Int64)))
        (if (= i 0) m (bld (- i 1) (Map.insert m i (* i 10)))))
      (def
        (f (: n Int64))
        (let
          ((m (bld n (Map.empty))))
          (fn ((: k Int64)) (match (Map.lookup m k) ((Option.Some v) v) ((Option.None) -1)))))
      (export f)))
  (call f (: 5 Int64) (: 3 Int64))
  (drop)
  (output (: 30 Int64))
  (live-objects 0))

(case
  "hcn3 dropping a closure with a CHAMP capture cascades the reclaim to zero"
  (input
    (do
      (def
        (bld (: i Int64) (: m (Map Int64 Int64)))
        (if (= i 0) m (bld (- i 1) (Map.insert m i (* i 10)))))
      (def
        (f (: n Int64))
        (let
          ((m (bld n (Map.empty))))
          (fn ((: k Int64)) (match (Map.lookup m k) ((Option.Some v) v) ((Option.None) -1)))))
      (export f)))
  (call f (: 5 Int64) (: 3 Int64))
  (drop)
  (output (: 30 Int64))
  (live-objects 0))

; ── breaker batch 580: dual-use INSIDE a closure body (unblocked by the #4707 projection fold).
; hce1 = tuple-index projection of a captured tuple in the closure body FOLDS + runs (the dqe
; leg-1 projection now works through closure conversion; the capture env leaks its cell). hce2 =
; a borrowing op (=) over a captured value in a closure body still DECLINES honestly ("borrowing
; op operand has an ownership this backend cannot yet prove") — the closure-body face of the
; ownership-proof gap, a todo auto-flip witness.
(case
  "hce1 tuple-index projection of a captured tuple in a closure body folds and runs (dqe leg-1 through closure conversion)"
  (input
    (do
      (def (f (: n Int64)) (let ((a #tuple(n #tuple(n 9)))) (fn ((: q Int64)) (+ q (. (. a 1) 1)))))
      (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (drop)
  (output (: 14 Int64))
  (live-objects 0))

(case
  "hce2 a borrowing op (=) over a captured value in a closure body declines pending the ownership proof (todo)"
  (input
    (do
      (def (f (: n Int64)) (let ((a #tuple(n n))) (fn ((: q Int64)) (+ q (if (= a a) 100 0)))))
      (export f)))
  (call f (: 1 Int64) (: 5 Int64))
  (output (: 105 Int64)))

; szf4: the CLOSURE face of the >64-KiB value-escape size-class (szf1-szf3 in 05 pin the direct
; string/bytes/list escapes). A closure's big String RESULT crosses as UTF-8 bytes through the
; closure copy-out (the third #7800 fix site); (output-byte-len N) (#7816) pins the canonical
; encoding of the crossed byte-list at the exact 64-KiB payload boundary. Doubling builder, tiny
; source.
(case
  "szf4 a closure returning a string built past the 64-KiB page crosses whole"
  (input
    (do
      (def (dbl (: k Int64) (: acc String)) (if (> k 0) (dbl (- k 1) (String.concat acc acc)) acc))
      (def (mk) (fn ((: n Int64)) (dbl n "ab")))
      (export mk)))
  (call mk (: 15 Int64))
  (output-byte-len 311199)
  (live-objects known-leak))

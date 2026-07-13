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

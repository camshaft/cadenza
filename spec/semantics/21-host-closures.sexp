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

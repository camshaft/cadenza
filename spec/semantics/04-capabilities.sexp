; Capabilities — witnesses the mandatory capability-declaration floor of capabilities-and-effects.md
; (no ambient authority) and host-interface-binding.md. A program reaches a host function only
; through a capability its manifest enumerates; reaching an undeclared one is a compile-time error.
; Host functions are the target's concern, not the language's: a case declares whatever WIT-typed
; host function it imports via (import (host <name> (func (<param-type>...) <result-type>))). The
; optional effect-row TYPING layer is NOT witnessed here (it is a later capability).

(case "a declared capability lets a program reach its host function"
  (doc    "Witnesses capabilities-and-effects.md #Capabilities Are Declared Up Front: main reaches an
           imported host function `log`, and the module declares it, so the operation is bound
           (host-interface-binding.md #A Host Import Is A WIT-Typed Function The Manifest Enumerates)
           and the run makes the host call, then terminates normally with the unit value (the host
           function's WIT result is unit). The (output …) clause pins the terminal condition and
           (host-calls …) pins the ordered host-call observation.")
  (input  (module m
            (import (host log (func (String) unit)))
            (use (capability log))
            (def (main)
              (log "ready"))))
  (output (: unit Unit))
  (host-calls (call log (: "ready" String))))

(case "reaching an undeclared host function is rejected at compile time"
  (doc    "Witnesses capabilities-and-effects.md #Undeclared Capability Is A Compile-Time Error and
           host-interface-binding.md #Ungranted Access Is Rejected At Compile Time: main calls a host
           function the module neither imports nor declares, so the program is rejected (CDZ0401) — the
           capability floor is intrinsic to lowering, so this rejection is one every generation makes.")
  (input  (module m
            (def (main)
              (log "ready"))))
  (error  CDZ0401))

(case "the program manifest is the union of its modules' declared capabilities"
  (doc    "Witnesses capabilities-and-effects.md #The Program Manifest Is The Union Of Its Modules:
           main reaches the host function `log`, declared in this module, so the manifest grants it
           and the run makes the host call — the union includes each module's declaration — then
           terminates normally with the unit value. The (output …) clause pins the terminal condition.")
  (input  (module m
            (import (host log (func (String) unit)))
            (use (capability log))
            (def (main)
              (log "1"))))
  (output (: unit Unit))
  (host-calls (call log (: "1" String))))

(case "a program that imports no host function is pure and makes no host call"
  (doc    "Witnesses capabilities-and-effects.md #A Host Import Is A Boundary Effect And The Manifest
           Is Its Row: a program that imports nothing has the empty effect row, runs straight to normal
           termination with no suspension, and its manifest is empty. (host-calls) asserts none was made.")
  (input  (module m
            (def (main)
              42)))
  (output (: 42 Int64))
  (host-calls))

(case "a program uses a response-returning host function's return value"
  (doc    "Witnesses capabilities-and-effects.md #Suspension Is Replay From The Host's Log: a
           response-returning host function `ask` is imported and its return value used. The
           (host-responses …) fixture supplies the response the host feeds in call order, so the run's
           result is a deterministic function of input and that response; (host-calls …) pins the call.")
  (input  (module m
            (import (host ask (func () Int64)))
            (use (capability ask))
            (def (main)
              (+ 1 (ask)))))
  (host-responses (respond ask (: 41 Int64)))
  (host-calls (call ask))
  (output (: 42 Int64)))

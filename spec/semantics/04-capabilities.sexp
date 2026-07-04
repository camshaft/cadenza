; Capabilities — witnesses the mandatory capability-declaration floor of capabilities-and-effects.md
; (no ambient authority) and host-interface-binding.md. A program reaches a host operation only
; through a capability its manifest enumerates; reaching an undeclared one is a compile-time error.
; The optional effect-tracking layer is NOT witnessed here (it is a later capability).

(case "a declared capability lets a program reach its host operation"
  (doc    "Witnesses capabilities-and-effects.md #Capabilities Are Declared Up Front: main reaches
           emit-event, and the module declares (capability emit-event), so the operation is bound
           (host-interface-binding.md #Imports Mirror The Manifest Exactly) and the run emits, then
           terminates normally with the unit value (core-semantics.md #An Effect-Only Expression
           Yields The Unit Value). The (output …) clause pins the terminal condition.")
  (input  (module m
            (use (capability emit-event))
            (def (main)
              (emit-event "k" "v"))))
  (output (: unit Unit))
  (events (event "k" (: "v" String))))

(case "reaching an undeclared host operation is rejected at compile time"
  (doc    "Witnesses capabilities-and-effects.md #Undeclared Capability Is A Compile-Time Error and
           host-interface-binding.md #Ungranted Access Is Rejected At Compile Time: main calls
           emit-event but the module declares no emit-event capability, so the program is rejected.
           The dynamic seed sees this as an unbound name (CDZ0101) since it only binds host ops
           when (use (capability ...)) is present; a typed generation rejects as CDZ0401.")
  (input  (module m
            (def (main)
              (emit-event "k" "v"))))
  (error  CDZ0101)
  (compiler (error CDZ0401)))

(case "the program manifest is the union of its modules' declared capabilities"
  (doc    "Witnesses capabilities-and-effects.md #The Program Manifest Is The Union Of Its Modules:
           main reaches emit-event, declared in this module, so the manifest grants it and the run
           emits — the union includes each module's declaration — then terminates normally with the
           unit value (core-semantics.md #An Effect-Only Expression Yields The Unit Value). The
           (output …) clause pins the terminal condition.")
  (input  (module m
            (use (capability emit-event))
            (def (main)
              (emit-event "ready" "1"))))
  (output (: unit Unit))
  (events (event "ready" (: "1" String))))

; Capabilities — witnesses the mandatory capability-declaration floor of capabilities-and-effects.md
; (no ambient authority) and host-interface-binding.md. A program reaches a host operation only
; through a capability its manifest enumerates; reaching an undeclared one is a compile-time error.
; The optional effect-tracking layer is NOT witnessed here (it is a later capability).

(case "a declared capability lets a program reach its host operation"
  (doc    "Witnesses capabilities-and-effects.md #Capabilities Are Declared Up Front: main reaches
           emit-event, and the module declares (capability emit-event), so the operation is bound
           (host-interface-binding.md #Imports Mirror The Manifest Exactly) and the run emits.")
  (input  (module m
            (use (capability emit-event))
            (def (main)
              (emit-event "k" "v"))))
  (events (event "k" (: "v" String))))

(case "reaching an undeclared host operation is rejected at compile time"
  (doc    "Witnesses capabilities-and-effects.md #Undeclared Capability Is A Compile-Time Error and
           host-interface-binding.md #Ungranted Access Is Rejected At Compile Time: main calls
           emit-event but the module declares no emit-event capability, so the program is rejected
           (CDZ0401) rather than compiled to a component carrying a latent import.")
  (input  (module m
            (def (main)
              (emit-event "k" "v"))))
  (error  CDZ0401))

(case "the program manifest is the union of its modules' declared capabilities"
  (doc    "Witnesses capabilities-and-effects.md #The Program Manifest Is The Union Of Its Modules:
           main reaches emit-event, declared in this module, so the manifest grants it and the run
           emits — the union includes each module's declaration.")
  (input  (module m
            (use (capability emit-event))
            (def (main)
              (emit-event "ready" "1"))))
  (events (event "ready" (: "1" String))))

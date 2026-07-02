; Equality, ordering, and the observable-behavior projection — witnesses core-semantics.md
; #Equality And Ordering, #Floating-Point Equality Follows The Canonical Byte Form, #Observable
; Behavior, and #A Program Terminates In Exactly One Terminal Condition. Results are (: <value> <Type>);
; observation of emitted events uses (events ...); resource-measure exhaustion uses (exhausted).

(case "structural equality holds component-wise"
  (doc    "Witnesses core-semantics.md #Equality Is Structural.")
  (input  (= 3 3))
  (output (: true Bool)))

(case "negative zero is not equal to positive zero"
  (doc    "Witnesses core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form:
           -0.0 and 0.0 have distinct canonical byte forms, so they are not equal.")
  (input  (= -0.0 0.0))
  (output (: false Bool)))

(case "every not-a-number value is equal to every not-a-number value"
  (doc    "Witnesses core-semantics.md #Floating-Point Equality Follows The Canonical Byte Form:
           all NaN values share one canonical byte form, so they compare equal. `nan` denotes the
           canonical not-a-number value (options/code-shape/, deterministic-value-form.md).")
  (input  (= nan nan))
  (output (: true Bool)))

(case "an offered ordering is total and deterministic"
  (doc    "Witnesses core-semantics.md #Ordering Where Offered Is Total: Int64 offers a total order.")
  (input  (< 2 3))
  (output (: true Bool)))

(case "a program that emits an event has that event in its observable behavior"
  (doc    "Witnesses core-semantics.md #Emitted Events Are Ordered And Part Of Observable Behavior.
           The module declares the emit-event capability, so the emit-event host operation is bound
           (host-interface-binding.md); the run emits one event and returns.")
  (input  (module m
            (use (capability emit-event))
            (def (main)
              (emit-event "greeting" "hello"))))
  (events (event "greeting" (: "hello" String))))

(case "emitted events are observed in the order they were emitted"
  (doc    "Witnesses core-semantics.md #Emitted Events Are Ordered And Part Of Observable Behavior:
           the sequence is observed in emission order.")
  (input  (module m
            (use (capability emit-event))
            (def (main)
              (let ((_ (emit-event "step" "first")))
                (emit-event "step" "second")))))
  (events (event "step" (: "first" String))
          (event "step" (: "second" String))))

(case "a program halts by exhausting the deterministic resource measure"
  (doc    "Witnesses core-semantics.md #Evaluation Is Bounded and #A Program Terminates In Exactly One
           Terminal Condition (the third terminal condition). Unbounded self-recursion consumes the
           resource measure (determinism-and-fuel.md §Resource Accounting) and halts at a defined
           point rather than running forever.")
  (input  (module m
            (def (loop n) (loop n))
            (def (main) (loop 0))))
  (exhausted))

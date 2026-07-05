; Equality, ordering, and the observable-behavior projection — witnesses core-semantics.md
; #Equality And Ordering, #Floating-Point Equality Follows The Canonical Byte Form, #Observable
; Behavior, and #A Program Terminates In Exactly One Terminal Condition. Results are (: <value> <Type>);
; observation of ordered host calls uses (host-calls ...); resource-measure exhaustion uses (exhausted).

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

; Float64 equality is a REALIZED seed capability (options/realized-capability-set/: "Float64
; literals/equality"), so it must hold for a RUNTIME float operand — one from a function parameter,
; a call, an if — not only for two compile-time-constant literals. The cases above compare constant
; floats; these compare a runtime float against a constant. The seed emits only the CONSTANT float
; equality (folded at compile time) and declines a runtime one ("non-constant float equality
; (canonical byte form) not yet emitted") — a not-yet-emitted runtime path within a realized
; capability. The value itself is carried correctly (a runtime float identity `(f 3.5)` → 3.5); only
; the equality comparison of a runtime float is missing.

(case "runtime float equality compares by canonical byte form"
  (doc    "`f` takes a Float64 parameter and compares it to the literal 3.5; f(3.5) is true. Float
           equality is realized (options/realized-capability-set/), so it must apply to a runtime
           float operand, matching the canonical-byte-form comparison the constant cases above use.
           The seed declines (\"non-constant float equality … not yet emitted\") — it folds constant
           float equality but has not emitted the runtime comparison.")
  (input  (module m
            (def (f x) (= x 3.5))
            (def (main) (f 3.5))))
  (output (: true Bool)))

(case "runtime float inequality compares by canonical byte form"
  (doc    "The companion with an unequal runtime operand: f(2.5) compares 2.5 to 3.5 and is false.
           Confirms the runtime float comparison is a genuine value test (true for 3.5, false for
           2.5), not a constant fold. The seed declines the same way.")
  (input  (module m
            (def (f x) (= x 3.5))
            (def (main) (f 2.5))))
  (output (: false Bool)))

(case "an offered ordering is total and deterministic"
  (doc    "Witnesses core-semantics.md #Ordering Where Offered Is Total: Int64 offers a total order.")
  (input  (< 2 3))
  (output (: true Bool)))

(case "a program that makes a host call has that call in its observable behavior"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior.
           The module imports and declares a unit-returning host function `log`, so it is bound
           (host-interface-binding.md #A Host Import Is A WIT-Typed Function The Manifest Enumerates);
           the run makes one host call and returns the unit value — the normal-termination value of a
           program evaluated only for its effect (core-semantics.md #An Expression Evaluated Only For
           Its Effect Yields The Unit Value). The (output …) primary clause pins the terminal
           condition; the (host-calls …) observation pins the call sequence.")
  (input  (module m
            (import (host log (func (String) unit)))
            (use (capability log))
            (def (main)
              (log "hello"))))
  (output (: unit Unit))
  (host-calls (call log (: "hello" String))))

(case "host calls are observed in the order they were made"
  (doc    "Witnesses core-semantics.md #Host Calls Are Ordered And Part Of Observable Behavior and
           #A Sequencing Block Evaluates Its Forms In Order (3rd sentence: an earlier form's host call is
           observed before a later form's): the two host calls are sequenced by a (do …) block, so
           \"first\" is observed before \"second\". The run terminates normally with the unit value
           (core-semantics.md #An Expression Evaluated Only For Its Effect Yields The Unit Value); the
           (output …) clause pins that terminal condition and the (host-calls …) observation pins the order.")
  (input  (module m
            (import (host log (func (String) unit)))
            (use (capability log))
            (def (main)
              (do
                (log "first")
                (log "second")))))
  (output (: unit Unit))
  (host-calls (call log (: "first" String))
              (call log (: "second" String))))

(case "a program halts by exhausting the deterministic resource measure"
  (doc    "Witnesses core-semantics.md #Evaluation Is Bounded and #A Program Terminates In Exactly One
           Terminal Condition (the third terminal condition). Unbounded self-recursion consumes the
           resource measure (determinism-and-fuel.md §Resource Accounting) and halts at a defined
           point rather than running forever.")
  (input  (module m
            (def (loop n) (loop n))
            (def (main) (loop 0))))
  (exhausted))

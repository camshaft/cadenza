; Type system — witnesses type-system.md. The seed is a DYNAMIC interpreter and does no static
; type-checking (constitution VII bootstrap carve-out), so each input has an interpreter terminal
; clause (the oracle) that the seed reproduces; a generation that realizes static typing checks the
; inline (compiler …) rejection. There is no separate typed corpus — the divergence is annotated in
; place. Diagnostic codes are from options/diagnostics-schema/.

(case "a type annotation consistent with the value is transparent"
  (doc    "Witnesses type-system.md #Annotations Constrain, Never Contradict: an annotation agreeing
           with the value changes nothing. Both the dynamic interpreter and a typed generation accept
           it; the interpreter evaluates to the annotated value.")
  (input  (: 42 Int64))
  (output (: 42 Int64)))

(case "an annotation that contradicts the value is rejected by the compiler"
  (doc    "Witnesses type-system.md #Annotations Constrain, Never Contradict. A typed generation
           rejects the conflicting annotation (CDZ0203). The dynamic interpreter ignores the
           annotation and evaluates the underlying value, so the oracle for a dynamic generation is
           the plain integer.")
  (input    (: 42 Bool))
  (output   (: 42 Int64))
  (compiler (error CDZ0203)))

(case "an unannotated program with a valid typing runs and type-checks"
  (doc    "Witnesses type-system.md #Inference Yields The Most General Type (2nd sentence): a valid
           typing need not be written by the author. Interpreter and typed generation agree.")
  (input  (let ((x 1)) (+ x 2)))
  (output (: 3 Int64)))

(case "an operation on mismatched types traps at runtime, rejected at compile time"
  (doc    "Witnesses type-system.md #A Well-Typed Program Does Not Go Wrong via its contrapositive:
           the ill-typed (+ 1 \"two\") is caught. The dynamic interpreter has no result for adding a
           string, so it traps (the oracle); a typed generation rejects it at compile time (CDZ0201).")
  (input    (+ 1 "two"))
  (trap     "numeric type mismatch")
  (compiler (error CDZ0201)))

(case "Type is a first-class value"
  (doc    "Witnesses core-semantics.md #Types Are First-Class Values (1st sentence): a Type can be
           bound to a name, passed as an argument, returned from a function. The seed treats Type as
           a value kind; a typed generation validates Type values statically.")
  (input  (let ((t Int64)) t))
  (output (: Int64 Type)))

(case "type annotations are runtime-checked in the dynamic seed"
  (doc    "Witnesses core-semantics.md #Types Are First-Class Values (3rd sentence): the seed validates
           type annotations at runtime, trapping on mismatch. (: 42 Bool) traps because 42 is Int64,
           not Bool. A typed generation rejects this at compile time, but the seed's runtime check
           enforces the same type system.")
  (input  (: 42 Bool))
  (trap   "type annotation mismatch"))

(case "runtime and static checking enforce the same type system"
  (doc    "Witnesses core-semantics.md #Types Are First-Class Values (4th sentence): seed's runtime
           checking and later static checking agree. (: (+ 1 2) Int64) passes both — runtime check
           sees Int64 value, static check infers Int64 type, both agree with annotation.")
  (input  (: (+ 1 2) Int64))
  (output (: 3 Int64)))

; Bootstrap interpreter — witnesses the behavioral requirements of bootstrap-interpreter.md that a
; later generation realizes when the reference interpreter is authored in Cadenza. Tagged
; (needs bootstrap-interpreter): the dynamic seed does NOT realize this capability
; (options/realized-capability-set/), so it skips these cases; the generation that authors the
; interpreter in Cadenza runs them. See spec/capabilities/bootstrap-interpreter.md.

(case "reading the text a printer produced round-trips to an equal value"
  (doc    "Witnesses bootstrap-interpreter.md #A Printer Renders The Canonical Representation As
           Re-Readable Text: read(print(v)) is equal to v under structural equality. Here v is the
           AST value for (+ 1 2); print renders it as text, read parses it back, and the two ASTs
           compare equal.")
  (needs bootstrap-interpreter)
  (input  (= (read (print (quote (+ 1 2))))
             (quote (+ 1 2))))
  (output (: true Bool)))

(case "an interpreter authored in the language maps a program to its observable behavior"
  (doc    "Witnesses bootstrap-interpreter.md #The Interpreter Maps A Program To Its Observable
           Behavior: eval over the AST of (+ 2 3) yields the behavior whose terminal value is 5.
           The generation that authors the interpreter in Cadenza realizes `eval`.")
  (needs bootstrap-interpreter)
  (input  (eval (quote (+ 2 3))))
  (output (: 5 Int64)))

; Self-hosting surface — witnesses the behavioral requirements of self-hosting-surface.md that a
; later generation realizes when the reader/printer are authored in Cadenza. Tagged
; (needs self-hosting-surface): the seed provides the reader/printer natively and does NOT realize the
; Cadenza-authored surface (options/realized-capability-set/), so it skips these cases; the generation
; that authors the reader/printer in Cadenza runs them. See spec/capabilities/self-hosting-surface.md.
;
; Note: there is no `eval` case. The compiler the bootstrap targets needs AST *construction and
; analysis*, not AST *execution* — `eval` (meta-interpretation of a runtime-constructed AST) is an
; optional macro/REPL capability, not part of the self-hosting surface
; (spec/learnings/2026-07-03-ast-construction-vs-ast-evaluation.md).

(case "reading the text a printer produced round-trips to an equal value"
  (doc    "Witnesses self-hosting-surface.md #A Printer Renders The Canonical Representation As
           Re-Readable Text: read(print(v)) is equal to v under structural equality. Here v is the
           AST value for (+ 1 2); print renders it as text, read parses it back, and the two ASTs
           compare equal.")
  (input  (= (read (print (quote (+ 1 2))))
             (quote (+ 1 2))))
  (output (: true Bool)))

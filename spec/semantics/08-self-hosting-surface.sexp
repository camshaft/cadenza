; Self-hosting surface — witnesses the behavioral requirements of self-hosting-surface.md. The seed
; provides the reader/printer NATIVELY and const-folds `read`/`print`/`quote`, so a round-trip like
; `read(print(v)) = v` RUNS and PASSES today (it is a compile-time-known value). What a later generation
; realizes is the CADENZA-AUTHORED reader/printer — the same behavior re-implemented in Cadenza source,
; consuming a runtime-constructed AST — which the seed does not yet build (options/realized-capability-
; set/). So these cases pin the observable read/print/round-trip CONTRACT the Cadenza-authored surface
; must also meet; the ones the seed already folds pass now, and the runtime-authored versions land later.
; See spec/capabilities/self-hosting-surface.md.
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

(case "print/read round-trips a tree carrying every leaf kind in one canonical form"
  (doc    "The all-leaf-kinds face of the round-trip law: one tree carries an int, a FLOAT (2.5 must
           re-read as a float — a printer dropping the trailing '.' breaks it), a STRING (quote/escape
           discipline), a bare NAME, and a nested compound with a NEGATIVE int and ZERO. read∘print
           must be identity over the whole leaf alphabet at once, not per-kind.")
  (input (= (read (print (quote (f 1 2.5 "s" x (g -3 0)))))
            (quote (f 1 2.5 "s" x (g -3 0)))))
  (output (: true Bool)))

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
(diagnostic-quality)

(case
  "reading the text a printer produced round-trips to an equal value"
  (doc
    "Witnesses self-hosting-surface.md #A Printer Renders The Canonical Representation As
           Re-Readable Text: read(print(v)) is equal to v under structural equality. Here v is the
           AST value for (+ 1 2); print renders it as text, read parses it back, and the two ASTs
           compare equal.")
  (input (= (Ast.read (Ast.print (quote (+ 1 2)))) (quote (+ 1 2))))
  (output (: true Bool)))

(case
  "print/read round-trips a tree carrying every leaf kind in one canonical form"
  (doc
    "The all-leaf-kinds face of the round-trip law: one tree carries an int, a FLOAT (2.5 must
           re-read as a float — a printer dropping the trailing '.' breaks it), a STRING (quote/escape
           discipline), a bare NAME, and a nested compound with a NEGATIVE int and ZERO. read∘print
           must be identity over the whole leaf alphabet at once, not per-kind.")
  (input
    (= (Ast.read (Ast.print (quote (f 1 2.5 "s" x (g -3 0))))) (quote (f 1 2.5 "s" x (g -3 0)))))
  (output (: true Bool)))

(case
  "print/read round-trips escape-laden, multibyte, and empty string leaves"
  (doc
    "The adversarial-CONTENT face of the round-trip law (the all-leaf-kinds pin covers the leaf
           ALPHABET; this stresses what lives INSIDE a string leaf): an embedded double-quote and a
           backslash (the printer must re-escape exactly what the reader unescapes), a MULTIBYTE
           é😀 leaf (byte-faithful, no re-encode), and a NEGATIVE float beside an EMPTY string (the
           \"\"-vs-dropped-token boundary) — three round-trips true plus an inequality control
           (1110). A printer that emitted raw quotes, normalized unicode, or dropped an empty
           string's quotes breaks its digit.")
  (input
    (do
      (def
        (main (: k Int64))
        (+
          (*
            1000
            (if (= (Ast.read (Ast.print (quote (f "a\"b" "c\\d")))) (quote (f "a\"b" "c\\d"))) 1 0))
          (+
            (* 100 (if (= (Ast.read (Ast.print (quote (g "é😀")))) (quote (g "é😀"))) 1 0))
            (+
              (* 10 (if (= (Ast.read (Ast.print (quote (h -2.5 "")))) (quote (h -2.5 ""))) 1 0))
              (if (= (Ast.read (Ast.print (quote (f "a\"b")))) (quote (f "other"))) 1 0)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1110 Int64)))

(case
  "print/read round-trips a 12-deep nest and a 25-wide list"
  (doc
    "The SHAPE-stress face of the round-trip law (08's pins are shallow): a 12-level
           single-spine nest exercises the reader's recursion discipline (every close-paren run at
           the end must pop exactly one frame) and a 25-sibling list exercises the token loop with
           multi-digit ints whose boundaries the printer must separate (a printer eliding a space
           between siblings fuses 1 2 into 12 and the re-read is a DIFFERENT valid AST — equality,
           not readability, is what catches it). Both true (11). Runtime-built Ast print/read remain
           const-only declines (documented while probing; the recursion-built deep tree declines
           cleanly).")
  (input
    (do
      (def
        (main (: mode Int64))
        (+
          (*
            10
            (if
              (=
                (Ast.read (Ast.print (quote (f (f (f (f (f (f (f (f (f (f (f (f 7)))))))))))))))
                (quote (f (f (f (f (f (f (f (f (f (f (f (f 7))))))))))))))
              1
              0))
          (if
            (=
              (Ast.read
                (Ast.print
                  (quote (g 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25))))
              (quote (g 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25)))
            1
            0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64)))

(case
  "read normalizes numeric token spellings to canonical float values"
  (doc
    "Number-token equivalence at the read boundary: \"2.50\" reads to the SAME float as the
           canonical 2.5 spelling, \"-0.5\" keeps its sign, and the EXPONENT form \"1e3\" reads
           equal to 1000.0 — three spellings, one value each (111·10 + false-control 0 → 1110; the
           control reads \"2.5\" against 2.25 and must MISS). A reader that compared float tokens
           textually (or parsed the exponent as a name) breaks a digit; equality is over the read
           VALUE, so canonical-byte float equality is what unifies the spellings.")
  (input
    (do
      (def
        (main (: _mode Int64))
        (+
          (*
            10
            (+
              (* 100 (if (= (Ast.read "(f 2.50)") (quote (f 2.5))) 1 0))
              (+
                (* 10 (if (= (Ast.read "(f -0.5)") (quote (f -0.5))) 1 0))
                (if (= (Ast.read "(f 1e3)") (quote (f 1000.0))) 1 0))))
          (if (= (Ast.read "(f 2.5)") (quote (f 2.25))) 1 0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1110 Int64)))

(case
  "read of malformed text is a coded reject, not a wrong value"
  (doc
    "The unreadable-input face of the reader law (08's pins are happy-path): `read` of an
           UNBALANCED \"(+ 1\" must never yield a partial AST. TODAY it is refused through the
           DECLINE channel ('read of text that is not a well-formed s-expression over the Ast
           subset') — scored todo; the message is a PERMANENT-sounding malformedness fact, so when
           the #35-comments fix touches lower_read this should become a CODED reject (this pin
           flips from todo to the error verdict then). Empty input and trailing content share the
           same refusal (checked; one uniform message).")
  (input
    (do (def (main (: mode Int64)) (if (= (Ast.read "(+ 1") (quote (+ 1))) 1 0)) (export main)))
  (error CDZ0101))

; --- The print/read FIXPOINT laws. ---
(case
  "print/read is IDEMPOTENT after one trip and read normalizes whitespace to one canonical tree"
  (doc
    "The FIXPOINT laws over the one-trip round-trip pin: read∘print∘read∘print = id on an escape+negative-float tree (a printer that re-escapes or re-spells diverges on trip TWO); print∘read∘print = print (canonical TEXT is a fixpoint — string equality catches spelling drift value-eq cannot); and read of whitespace-laden text lands on the canonical tree.")
  (input
    (do
      (def
        (main (: _m Int64))
        (+
          (*
            100
            (if
              (=
                (Ast.read (Ast.print (Ast.read (Ast.print (quote (a "x\"y" -0.5))))))
                (quote (a "x\"y" -0.5)))
              1
              0))
          (+
            (*
              10
              (if
                (= (Ast.print (quote (f 2.5))) (Ast.print (Ast.read (Ast.print (quote (f 2.5))))))
                1
                0))
            (if (= (Ast.read "( a   ( b )  )") (quote (a (b)))) 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 111 Int64)))

; --- The self-hosting members are NAMESPACED on the built-in Ast record, so a user Ast shadows them. ---
(case
  "a user (type Ast …) shadows the built-in, so Ast.print is no longer the printer"
  (doc
    "`Ast.print` (Ast -> String) is the NAMESPACED printer — an associated function on the BUILT-IN
           `Ast` record (the former top-level `print`), reached by ordinary member access. A user
           `(type Ast …)` shadows the built-in (a top-level type declaration resolves before the prelude)
           and carries no associated `print` member, so `Ast.print` is an ordinary unknown-member access
           (CDZ0201), NOT the printer. This is the binding-respecting property namespacing on a shadowable
           record delivers and a bare global `print` could not: the self-hosting surface follows binding.")
  (input (do (type Ast (Foo)) (def (main) (Ast.print 0)) (export main)))
  (error CDZ0201))

(case
  "a user (type Ast …) shadows the built-in, so Ast.read is no longer the reader"
  (doc
    "The reader twin of the printer shadow: `Ast.read` (String -> Ast) is a NAMESPACED associated
           function on the built-in `Ast` record (the former top-level `read`). A user `(type Ast …)`
           carries no associated `read`, so `Ast.read` against it is an ordinary unknown-member access
           (CDZ0201), NOT the reader.")
  (input (do (type Ast (Foo)) (def (main) (Ast.read "x")) (export main)))
  (error CDZ0201))

(case
  "a user (type Ast …) shadows the built-in, so Ast.module is no longer the self-reflection"
  (doc
    "`Ast.module` is the NAMESPACED self-reflection member (the enclosing module as an `Ast` value)
           on the BUILT-IN `Ast` record — the type-directed, binding-respecting replacement for the retired
           blind `(. Ast self)` syntax-rewrite. A user `(type Ast …)` shadows the built-in and carries no
           associated `module` member, so `Ast.module` against it is an ordinary unknown-member access
           (CDZ0201), NOT the reflection — a user shadowing `Ast` routes away from the reflection, which a
           blind syntax-rewrite could never honor.")
  (input (do (type Ast (Foo)) (def (main) Ast.module) (export main)))
  (error CDZ0201))

(case
  "prr1 a RUNTIME print-then-read Ast round-trip declines pending runtime Ast.read (constant strings only today)"
  (doc
    "Every existing read(print(quote …)) round-trip is fully CONSTANT and const-folds; a runtime-
           SELECTED quote through the same pipeline declines honestly (\"read of a runtime string is not
           yet computed (constant strings only)\"). Auto-flip witness for the runtime-Ast.read increment.
           Oracle traced twice: 50 frames x depth(read(print(quote (f (g 1))))) = 50 x 2 = 100; the walk
           will also carry the per-walk extraction-dup leak (aq2: ~10/walk -> ~500) when it flips —
           re-measure census at flip time rather than pre-pinning a leak clause on a decline.")
  (input
    (do
      (def
        (depth (: node Ast))
        (match
          node
          ((Ast.List es) (match es (#list() 1) (#list(h (.. rest)) (+ 1 (depth h)))))
          (_ 1)))
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (+
            (depth (Ast.read (Ast.print (if (> k 0) (quote (f (g 1))) (quote z)))))
            (frames (- k 1)))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 100 Int64)))

; Metaprogramming — quote/quasiquote and the AST as a sum type. Witnesses metaprogramming.md.
; Quote produces an AST value without evaluating; quasiquote allows selective evaluation for
; construction. The AST is a sum type deconstructible by pattern matching, so the compiler
; operates on AST values natively rather than using string-tagged reflection. Eval (executing
; AST as code) is optional for macros/REPL, not needed by the core compiler.

(case "quote produces an AST value without evaluating"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value: (quote <expr>) returns an
           AST sum type value representing <expr>'s structure, without evaluating <expr>.
           (quote (+ 1 2)) produces an AST value, not 3.")
  (input  (quote (+ 1 2)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2))) Ast)))

(case "eval is optional for macros and interactive use"
  (doc    "Witnesses metaprogramming.md #Eval Is Optional For Macros And Interactive Use: (eval <ast>)
           executes AST as code, optional for macros/REPL. Seed provides it; static generations need
           not. (eval (quote (+ 1 2))) produces 3.")
  (needs  eval)
  (input  (eval (quote (+ 1 2))))
  (output (: 3 Int64)))

(case "the AST is a sum type deconstructible by pattern matching"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value (2nd sentence): the AST is a
           sum type with variants for each syntactic form. Pattern matching over (quote 42) binds
           the integer payload, demonstrating AST variants are proper sum types.")
  (input  (match (quote 42)
            ((Ast.Int n) n)
            ((Ast.Name _) 0)))
  (output (: 42 Int64)))

(case "pattern matching over AST distinguishes forms"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value: the compiler pattern-matches
           over AST sums to distinguish syntactic forms. Matching (quote (+ 1 2)) as an Ast.List
           allows inspecting its structure recursively.")
  (input  (match (quote (+ 1 2))
            ((Ast.List elems) (List.len elems))
            ((Ast.Int _)      0)))
  (output (: 3 Int64)))

(case "eval on malformed AST traps"
  (doc    "Witnesses metaprogramming.md #Eval Is Optional: eval on malformed AST traps. An Ast.List
           with no elements is malformed (no operator), so eval traps.")
  (needs  eval)
  (input  (eval (Ast.List (list))))
  (trap   "malformed AST"))

(case "quasiquote constructs AST with selective evaluation"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           `<template> quotes like quote, but ,<expr> evaluates <expr> normally and inserts result
           into the AST being constructed. `(+ ,x 10) with x=2 produces AST for (+ 2 10), not (+ x 10).
           This is construction, not eval — ,x evaluates the variable x, not an AST.")
  (input  (let ((x 2)) `(+ ,x 10)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 2) (Ast.Int 10))) Ast)))

(case "unquote in quasiquote evaluates normally and embeds"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           ,<expr> evaluates <expr> normally (not as AST) and embeds the result.
           `(+ ,(+ 1 1) 10) evaluates (+ 1 1) to 2, constructs AST with that value.")
  (input  `(+ ,(+ 1 1) 10))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 2) (Ast.Int 10))) Ast)))

; An AST built by quasiquote-with-unquote is an ordinary AST VALUE: it must be structurally EQUAL to
; the same AST built any other way, and encode to the same bytes (core-semantics.md #Equality Is
; Structural; the AST is an ordinary sum type — type-system.md #The Abstract Syntax Tree Type Is An
; Ordinary Sum Type). So `` `(f ,x) `` with x=1 equals `` `(f ,1) `` and `(quote (f 1))` — all three
; are `(Ast.List (Ast.Name "f") (Ast.Int 1))`. An unquote that embeds a RUNTIME (let-bound) value
; must build the same `(Ast.Int 1)` node a const fold produces, so structural equality and encoding
; see the two as identical. This is the compiler's own idiom: it builds instruction ASTs by
; quasiquoting runtime values, then compares/encodes them.

(case "an AST from quasiquoting a runtime value equals the same AST built by quote"
  (doc    "`` `(f ,x) `` with x bound to 1 builds `(Ast.List (Ast.Name \"f\") (Ast.Int 1))`, the same
           AST `(quote (f 1))` builds — so they are structurally equal (core-semantics.md #Equality Is
           Structural). An unquote that embeds a runtime value produces the same node as a const fold,
           so the two compare equal. MUST be true.")
  (input  (let ((x 1)) (= `(f ,x) (quote (f 1)))))
  (output (: true Bool)))

(case "quasiquotes unquoting a runtime variable and a literal build equal ASTs"
  (doc    "The companion isolating the runtime-vs-const embedding: `` `(f ,x) `` (x=1, a runtime local)
           and `` `(f ,1) `` (a literal) build the same AST and MUST be equal — the runtime-unquoted
           node is structurally identical to the const-unquoted one.")
  (input  (let ((x 1)) (= `(f ,x) `(f ,1))))
  (output (: true Bool)))

; --- A quoted AST equals the same AST built by applying the Ast.* constructors -------------
; `quote` produces an AST SUM value (metaprogramming.md #Quote Produces An AST Value: "MUST evaluate
; to an AST sum type value"; type-system.md #The Abstract Syntax Tree Is An Ordinary Sum Type). The
; AST's variants are the ordinary constructors `Ast.Int` / `Ast.Name` / `Ast.List` / …, so a value
; `quote` builds and the SAME value built by applying those constructors are ONE sum value — the
; corpus already records `(quote (+ 1 2))`'s value form AS `(Ast.List (list (Ast.Name "+") (Ast.Int 1)
; (Ast.Int 2)))` (the first case in this file) and matches `(quote 42)` against `(Ast.Int n)` binding
; n=42. Structural equality (core-semantics.md #Equality Is Structural) must therefore see the two as
; equal: `(= (quote 42) (Ast.Int 42))` is true, exactly as `(= (quote (f 1)) (quote (f 1)))` and
; `(= (Ast.Int 42) (Ast.Int 42))` are. Pattern matching already normalizes the two — `(match (quote
; 42) ((Ast.Int n) n) …)` binds 42 — so equality must agree, or the AST is a sum value under `match`
; but a distinct thing under `=`, splitting the one value form the encoding is a bijection over
; (ast-encoding.md #The Encoding Is A Bijection With One Canonical Byte Form).

(case "a quoted integer equals the same node built by the Ast.Int constructor"
  (doc    "`(quote 42)` is the AST sum value `(Ast.Int 42)` (metaprogramming.md #Quote Produces An AST
           Value — quote evaluates to an AST SUM value). The corpus records quote outputs in exactly
           this constructor form and matches `(quote 42)` as `(Ast.Int n)` binding 42, so the two
           denote ONE sum value and structural equality MUST be true. A representation that stores a
           quote result differently from an applied Ast.* constructor — comparing them unequal — splits
           the single AST value form the encoding bijection is defined over. MUST be true.")
  (input  (= (quote 42) (Ast.Int 42)))
  (output (: true Bool)))

(case "a quoted name equals the same node built by the Ast.Name constructor"
  (doc    "The Name companion: `(quote foo)` is `(Ast.Name \"foo\")` — a quoted bare name is the
           Ast.Name sum value carrying the name as a String payload (metaprogramming.md #Quote Produces
           An AST Value). `(= (quote foo) (Ast.Name \"foo\"))` MUST be true, exactly as the Int case.
           Pins that the quote-vs-constructor equality holds for the leaf name node too.")
  (input  (= (quote foo) (Ast.Name "foo")))
  (output (: true Bool)))

(case "a quoted compound form equals the same AST built by the Ast.List constructor"
  (doc    "The list companion, and the sharpest case: `(quote (+ 1 2))` is
           `(Ast.List (list (Ast.Name \"+\") (Ast.Int 1) (Ast.Int 2)))` — the very value form the FIRST
           case in this file records as `(quote (+ 1 2))`'s output. So comparing the quote against that
           hand-built Ast.List MUST be true (core-semantics.md #Equality Is Structural), because they
           are the same sum value. This equality is the compiler's own idiom: it builds an instruction
           AST by quasiquote and compares it against an expected AST built by constructors.")
  (input  (= (quote (+ 1 2)) (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))))
  (output (: true Bool)))

(case "quote-vs-constructor AST equality holds regardless of operand order"
  (doc    "The order-flipped companion: `(= (Ast.Int 42) (quote 42))` is the same comparison of one
           sum value against itself and MUST be true. Pins that neither operand order (constructor-built
           vs quote-built) is treated as a distinct type — structural equality is symmetric over the
           one AST value form.")
  (input  (= (Ast.Int 42) (quote 42)))
  (output (: true Bool)))

; --- Ast.encode / Ast.decode consume an AST built by the Ast.* constructors too ------------
; The encoding is a bijection over THE abstract syntax tree value (ast-encoding.md #The Encoding Is A
; Bijection With One Canonical Byte Form: "Decoding a canonical binary encoding MUST yield the abstract
; syntax tree it was encoded from"; "Two abstract syntax trees that are equal MUST have identical binary
; encodings"). Since an AST built by applying the Ast.* constructors is the same value form `quote`
; produces (the cases above), `Ast.encode` and `Ast.decode` must consume it exactly as they consume a
; quote-built AST — the round-trip is over AST VALUES, not over one privileged construction path. These
; are the encode/decode companions of the equality cases above: the same representation must reach the
; encoder however the AST was built. The seed encodes/decodes a quote-built AST (the round-trip cases
; earlier in this file) but declines a constructor-built one ("unsupported dotted-application") — the
; encoder resolves a quote value but not an applied Ast.* constructor, the same construct/consume split
; the equality cases expose, here on the encode path. A generation that does not yet bridge the two
; declines rather than miscompiling (reject-don't-miscompile); the gate scores it todo.

(case "encoding and decoding a constructor-built AST round-trips to an equal value"
  (doc    "`(Ast.Int 7)` is an AST value (the same form `(quote 7)` produces); encoding then decoding it
           MUST yield an equal AST (ast-encoding.md #The Encoding Is A Bijection — decode(encode t) is t).
           The quote-built round-trip is witnessed earlier in this file; a constructor-built AST is the
           same value form, so it MUST round-trip identically. The seed declines (\"unsupported
           dotted-application\") — Ast.encode consumes a quote value but not an applied Ast.* constructor.")
  (input  (= (Ast.decode (Ast.encode (Ast.Int 7))) (Ast.Int 7)))
  (output (: true Bool)))

(case "encoding and decoding a constructor-built compound AST round-trips"
  (doc    "The compound companion: a hand-built `(Ast.List (list (Ast.Name \"g\") (Ast.Int 5)))` MUST
           encode and decode back to an equal AST, exactly as a quote-built list does. Pins that the
           bijection round-trip reaches a constructor-built compound AST, not only a leaf node.")
  (input  (= (Ast.decode (Ast.encode (Ast.List (list (Ast.Name "g") (Ast.Int 5)))))
             (Ast.List (list (Ast.Name "g") (Ast.Int 5)))))
  (output (: true Bool)))

(case "a quote-built and constructor-built AST of the same tree encode to identical bytes"
  (doc    "ast-encoding.md #The Encoding Is A Bijection With One Canonical Byte Form: \"Two abstract
           syntax trees that are equal MUST have identical binary encodings.\" `(quote 42)` and
           `(Ast.Int 42)` are the same AST (the equality cases above), so their encodings MUST be
           byte-identical. This is the encode-path witness of the one-canonical-byte-form requirement:
           the encoder must produce the same bytes for the one AST value however it was constructed. The
           seed declines the constructor-built operand, so it cannot yet witness the agreement.")
  (input  (= (Ast.encode (quote 42)) (Ast.encode (Ast.Int 42))))
  (output (: true Bool)))

(case "unquote-splicing splices list elements into parent"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           ,@<list-expr> evaluates <list-expr> to a list and splices its elements into the parent,
           not nested. `(+ ,@args) with args=(list 1 2 3) produces AST for (+ 1 2 3), not (+ (1 2 3)).")
  (input  (let ((args (list 1 2 3))) `(+ ,@args)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2) (Ast.Int 3))) Ast)))

(case "splice flattens where unquote nests"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           , nests the value; ,@ splices it. `(f ,x) embeds x as one element; `(f ,@x) with
           x=(list 1 2) splices to produce (f 1 2).")
  (input  (let ((x (list 1 2)))
            (= `(f ,@x) `(f 1 2))))
  (output (: true Bool)))

; --- Splicing requires a list --------------------------------------------------------------
; metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation (witnessed above): `,@`
; "evaluates <list-expr> to a LIST and splices its elements into the parent." So splicing a value
; that is NOT a list — a scalar, a tuple, a string — has no elements to splice and is ill-typed:
; the compiler MUST reject it (CDZ0201) with the splice's non-list operand named — `unquote-splicing`
; is a recognized form, not an unbound name. A generation that does not yet check the splice operand's
; list type declines rather than running the program (reject-don't-miscompile).

(case "splicing a non-list value into a quasiquote is a type error"
  (doc    "`,@` splices the ELEMENTS of a list; splicing a non-list has no elements to splice.
           `(f ,@x)` with x bound to the Int64 `5` is ill-typed — the compiler MUST reject it
           (CDZ0201, metaprogramming.md: ,@ evaluates its operand to a LIST). A generation that does
           not yet check the splice operand's list type declines rather than running the program.")
  (input  (let ((x 5)) `(f ,@x)))
  (error  CDZ0201))

(case "splicing an integer literal directly is a type error"
  (doc    "The directly-written companion: `(unquote-splicing 5)` inside a quasiquote splices the
           literal `5`, which is not a list — a type error (CDZ0201). The rejection names the splice's
           non-list operand; `unquote-splicing`/`quasiquote` are recognized forms, not names, so this
           is not an unbound-name failure.")
  (input  (quasiquote ((unquote-splicing 5) 3)))
  (error  CDZ0201))

(case "quasiquote nests with inner unquote evaluated"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           quasiquote nests, so ``(+ ,,x) evaluates the inner , to produce `(+ ,<x-value>).
           With x=2, ``(+ ,,x) constructs an AST representing `(+ ,2).")
  (input  (let ((x 2)) ``(+ ,,x)))
  (output (: (Ast.List (list (Ast.Name "quasiquote")
                             (Ast.List (list (Ast.Name "+")
                                           (Ast.List (list (Ast.Name "unquote") (Ast.Int 2)))))))
             Ast)))

(case "unquote outside quasiquote is a syntax error"
  (doc    "Witnesses metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation:
           , and ,@ are only valid inside quasiquote context. Bare ,x is a syntax error — there's
           no quasiquote template to insert into — so the compiler rejects it at parse time (CDZ0401)
           rather than running the program.")
  (input  ,x)
  (error  CDZ0401))

; `unquote` takes EXACTLY ONE operand — the expression to evaluate and embed. `(unquote 1 2)` supplies
; two, so it is malformed and the compiler MUST reject it (CDZ0201), never index just the first and
; drop the rest. The same arity check applies to an unquote encountered during quasiquote expansion as
; to one outside a quasiquote, so `` `(unquote 1 2) `` is rejected rather than silently truncated to
; `(Ast.Int 1)`. (Same class as over-applying a constructor `(Some 1 2)`, here for the `unquote` form
; inside a template.)

(case "unquote with more than one operand inside a quasiquote is malformed"
  (doc    "`(unquote 1 2)` inside a quasiquote gives `unquote` two operands where it takes exactly one —
           a malformed form the compiler MUST reject at compile time (CDZ0201) rather than silently
           take the first operand and drop the rest to yield `(Ast.Int 1)`. The same arity check
           applies during quasiquote expansion as outside a quasiquote.")
  (input  (quasiquote (unquote 1 2)))
  (error  CDZ0201))

(case "quasiquote makes instruction construction readable"
  (doc    "Witnesses compiler-pipeline.md #The Compiler Constructs Instructions Via Quasiquote:
           building instructions via quasiquote is readable. Compare `(op-const ,n) vs
           (Ast.List (list (Ast.Name \"op-const\") n)) — quasiquote reads like the instruction.
           Note: dotted names like i64.const expand to member access; instruction tags use
           hyphenated names to avoid this.")
  (input  (let ((n 42)) `(op-const ,n)))
  (output (: (Ast.List (list (Ast.Name "op-const") (Ast.Int 42))) Ast)))

(case "Ast.decode converts bytes to an AST sum type value"
  (doc    "Witnesses compiler-pipeline.md #The Compiler Operates On AST Values: the compiler receives
           a program as binary bytes and decodes it to an AST sum type value. Ast.decode takes Bytes
           and returns an Ast value (the same sum type quote produces). The compiler then pattern-matches
           over the decoded AST.")
  (input  (match (Ast.decode (Ast.encode (quote 42)))
            ((Ast.Int n) n)
            (else        0)))
  (output (: 42 Int64)))

(case "Ast.encode and Ast.decode round-trip"
  (doc    "Witnesses contracts/ast-encoding.md: encoding an AST to binary and decoding it back
           produces the same AST value. The compiler relies on this: it decodes the input, operates
           on AST values, and the encoding is faithful.")
  (input  (= (Ast.decode (Ast.encode (quote (+ 1 2))))
             (quote (+ 1 2))))
  (output (: true Bool)))

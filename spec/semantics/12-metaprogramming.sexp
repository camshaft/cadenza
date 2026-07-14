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

; --- A plain quote does not evaluate a nested unquote --------------------------------------------
; metaprogramming.md #Quote Produces An AST Value: `(quote <expr>)` produces the AST of <expr>
; "WITHOUT evaluating <expr> itself" — UNCONDITIONALLY, whatever <expr> contains. A quasiquote
; nested inside a plain quote is INERT: `(quote `(+ ,x))` is the AST of the template `(+ ,x)`, in
; which the `,x` (unquote) is ordinary structure — the plain quote does not put it in a
; quasiquote-active context (that context is established by an EVALUATED quasiquote, and this
; quasiquote is quoted, not evaluated). So the `,x` MUST NOT be evaluated: the quoted structure
; mentions the NAME `x`, not x's value. This is the EVALUATION-side dual of "an unquote nested
; inside a plain quote is a syntax error, not an active unquote" (a bare `(quote (g ,x))` rejects
; CDZ0003): here the unquote is one level deeper — under a quasiquote under the quote — so it is
; inert data rather than a stray unquote, but the same principle holds: a plain quote evaluates
; NOTHING in its body. Discriminator that does not depend on the inert node's exact spelling: bind
; two DISTINCT names to the SAME value and quote each — the quoted templates mention different
; names (`x` vs `y`), so they are NOT structurally equal. A compiler that evaluates the nested
; unquote collapses both to the AST of `(+ 1)` and wrongly answers `true`.
(case "a plain quote does not evaluate a quasiquote's unquote nested inside it"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value (\"without evaluating <expr>\").
           `(quote `(+ ,x))` and `(quote `(+ ,y))` with x and y both bound to 1 quote two templates
           that mention DIFFERENT names (`x` vs `y`); a plain quote does not evaluate the nested
           `,x`/`,y`, so the two quoted structures differ and `=` is FALSE. A compiler that evaluates
           the nested unquote (embedding x's value 1 and y's value 1) collapses both to the AST of
           `(+ 1)` and wrongly answers true — it treated the quoted quasiquote as an active one,
           evaluating inside a plain quote. Companion (rejection side) below: a bare stray unquote
           under a plain quote is CDZ0003.")
  (input  (let ((x 1)) (let ((y 1)) (= (quote `(+ ,x)) (quote `(+ ,y))))))
  (output (: false Bool)))

(case "eval is optional for macros and interactive use"
  (doc    "Witnesses metaprogramming.md #Eval Is Optional For Macros And Interactive Use: (eval <ast>)
           executes AST as code, optional for macros/REPL. Seed provides it; static generations need
           not. (eval (quote (+ 1 2))) produces 3.")
  (input  (eval (quote (+ 1 2))))
  (output (: 3 Int64)))

(case "eval of a quasiquote splicing a compile-time-known value"
  (doc    "The core macro idiom (metaprogramming.md #Eval Is Optional / #Quasiquote Constructs AST With
           Selective Evaluation): eval a quasiquoted form whose unquote splices a compile-time-known VALUE,
           not just a bare literal. `(let ((x 3)) (eval `(+ ,x 4)))` reconstructs `(+ x 4)` and folds to 7.
           The eval desugar reconstructs `(eval AST)` to the source the AST denotes; an active unquote lifts
           its live operand into `(Ast.Int <e>)`, so reconstruction unwraps that back to `<e>` — a let-bound
           name, a def-const, or a computed constant, all resolving in the eval's enclosing scope. (A bare-
           LITERAL splice `(unquote 3)` and a plain `(quote …)` already worked; a NON-literal unquote once
           left the eval un-desugared, so its head `eval` reported a misleading 'unbound name eval'.) The
           reconstructed source must reach the enclosing `let`, so the desugar blanks the dead reified-
           argument wrappers, leaving the spliced `x` node parented at the eval position. Expected 7.")
  (input  (do
            (def (main) (let ((x 3)) (eval (quasiquote (+ (unquote x) 4)))))
            (export main)))
  (output (: 7 Int64)))

(case "the AST is a sum type deconstructible by pattern matching"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value (2nd sentence): the AST is a
           sum type with variants for each syntactic form. Pattern matching over (quote 42) binds
           the integer payload, demonstrating AST variants are proper sum types. Because the AST is
           an ORDINARY sum (type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type),
           its match is subject to the same exhaustiveness rule any sum match is (#A Match Is
           Exhaustive Against The Sum Type's Variant Set), so a match that inspects one form carries a
           catch-all `_` arm for the others.")
  (input  (match (quote 42)
            ((Ast.Int n) n)
            (_ 0)))
  (output (: 42 Int64)))

(case "pattern matching over AST distinguishes forms"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value: the compiler pattern-matches
           over AST sums to distinguish syntactic forms. Matching (quote (+ 1 2)) as an Ast.List
           allows inspecting its structure recursively. The AST is an ordinary sum, so the match
           covers the remaining variants with a catch-all `_` arm (#A Match Is Exhaustive Against The
           Sum Type's Variant Set).")
  (input  (match (quote (+ 1 2))
            ((Ast.List elems) (List.len elems))
            (_                0)))
  (output (: 3 Int64)))

(case "eval on malformed AST traps"
  (doc    "Witnesses metaprogramming.md #Eval Is Optional: eval on malformed AST traps. An Ast.List
           with no elements is malformed (no operator), so eval traps.")
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

; An unquote MUST EVALUATE its expression (metaprogramming.md #Quasiquote Constructs AST With Selective
; Evaluation: ",<expr> … MUST evaluate <expr> normally and insert its result"). If that expression cannot
; be evaluated because it references an UNBOUND name, that is the ordinary unbound-name error
; (core-semantics.md #Binding Is Lexical: "A reference to a name with no enclosing binding MUST be a
; compile-time error", unconditional) — NOT an occasion to fall back to quoting the expression as inert
; AST. `` `(a ,(+ b 1)) `` with `b` unbound must be rejected CDZ0101, exactly as the bare `(+ b 1)` is,
; because the unquote evaluates `(+ b 1)` and `b` has no binding. A compiler that, when an unquote's
; expression fails to const-evaluate, silently QUOTES it (yielding `(Ast.List (Ast.Name "+") (Ast.Name
; "b") (Ast.Int 1))`) turns a scope error into a valid AST value — the unquote stopped evaluating and
; became a second quote, contradicting the selective-EVALUATION rule.

(case "an unquote of an expression with an unbound name is rejected, not quoted"
  (doc    "`` `(a ,(+ b 1)) `` unquotes `(+ b 1)`, which references the unbound name `b` — the unquote
           MUST evaluate its expression (metaprogramming.md #Quasiquote Constructs AST With Selective
           Evaluation), so this is the ordinary unbound-name error (CDZ0101, core-semantics.md #Binding
           Is Lexical — unconditional), exactly as the bare `(+ b 1)` is. Pins that an unquote whose
           expression cannot be evaluated is rejected, NOT silently quoted as inert AST: a compiler that
           falls back to quoting the un-evaluable expression (yielding an `(Ast.List …)` for `(+ b 1)`)
           turns the selective-evaluation unquote into a second quote and swallows the scope error. With
           `b` bound (`(let ((b 5)) `(a ,(+ b 1)))`) the unquote evaluates to 6; unbound, it is CDZ0101.")
  (input  `(a ,(+ b 1)))
  (error  CDZ0101))

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

; The built-in `Ast` is an ordinary sum type — "a variant per syntactic form (an integer, a float, a
; string, a boolean, a name, and a list of child nodes)" (type-system.md #The Abstract Syntax Tree Type
; Is An Ordinary Sum Type) — so its constructors carry TYPED payloads: `Ast.Int` an Int64, `Ast.Name` a
; String, `Ast.List` a list of Ast. A constructor is a single-arity function whose argument is type-checked
; (core-semantics.md #A Sum Type Constructor Is A Single-Arity Function + #Applying A Function Binds Its
; Parameter To Its Argument), so `(Ast.Int "x")` — a String where `Ast.Int`'s payload is Int64 — is a type
; mismatch the compiler MUST reject (CDZ0201), exactly as a user sum's `(T.Mk "x")` for `(type T (Mk
; Int64))` is. A compiler that checks a USER sum variant's payload type (that check landed) but not the
; BUILT-IN `Ast` constructors' declared payloads builds a malformed `(Ast.Int "x")` node: matching it binds
; the String where an Int64 is declared, and `(String.byte-len n)` reads it as a String and succeeds —
; running the ill-typed program. This is the built-in-Ast companion of the user-sum unary-variant
; payload-type case (05-compound-types.sexp): the Ast constructors are ordinary sum constructors and MUST
; type-check their payloads identically. A self-hosted front end that builds AST nodes with the Ast.*
; constructors depends on this — an unchecked `(Ast.Int "x")` is a malformed node it could emit. A
; generation that does not yet check the Ast constructors' payload types declines rather than building the
; mistyped node.

(case "a built-in Ast constructor applied to a wrong-type payload is a type error"
  (doc    "`Ast.Int`'s payload type is Int64 (the built-in `Ast` is an ordinary sum type, a variant per
           syntactic form — type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type), so
           `(Ast.Int \"x\")` applies it to a String — a type mismatch the compiler MUST reject (CDZ0201),
           exactly as a user sum's `(T.Mk \"x\")` for `(type T (Mk Int64))` is. Pins that the built-in
           `Ast` constructors type-check their declared payloads just as a user sum variant does — a
           compiler that checks user variants but not the built-in Ast constructors builds a malformed
           `(Ast.Int \"x\")` node (matching it binds the String where an Int64 is declared, and a
           downstream String use of the payload succeeds, running the ill-typed program). A self-hosted
           front end that constructs AST nodes with `Ast.*` depends on this check. A generation that does
           not yet check the Ast constructors' payload types declines rather than building the mistyped
           node.")
  (input  (Ast.Int "x"))
  (error  CDZ0201))

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
; encoder however the AST was built. A quote-built AST (`(quote 7)`) and a constructor-built one
; (`(Ast.Int 7)`) are the ONE AST value form (the equality cases above), so `Ast.encode`/`Ast.decode`
; consume both identically — the round-trip is over AST VALUES, not one privileged construction path.
; A generation that has not yet bridged the two would decline the constructor-built operand rather than
; miscompiling (reject-don't-miscompile); the seed now bridges an applied `Ast.*` constructor to the AST
; value it denotes (via the constructor→node bridge), so these round-trip like the quote-built cases.

(case "encoding and decoding a constructor-built AST round-trips to an equal value"
  (doc    "`(Ast.Int 7)` is an AST value (the same form `(quote 7)` produces); encoding then decoding it
           MUST yield an equal AST (ast-encoding.md #The Encoding Is A Bijection — decode(encode t) is t).
           The quote-built round-trip is witnessed earlier in this file; a constructor-built AST is the
           same value form, so it round-trips identically — the encoder bridges an applied `Ast.*`
           constructor to the AST value it denotes. `Ast.decode : Bytes → Result<Ast, _>` is total
           (value-interchange.md — decode of possibly-external bytes yields the error case, never traps),
           so the round-trip matches the `Ok` arm and equates its payload.")
  (input  (match (Ast.decode (Ast.encode (Ast.Int 7)))
            ((Ok a)  (= a (Ast.Int 7)))
            ((Err _) false)))
  (output (: true Bool)))

(case "encoding and decoding a constructor-built compound AST round-trips"
  (doc    "The compound companion: a hand-built `(Ast.List (list (Ast.Name \"g\") (Ast.Int 5)))` MUST
           encode and decode back to an equal AST, exactly as a quote-built list does. Pins that the
           bijection round-trip reaches a constructor-built compound AST, not only a leaf node. `Ast.decode`
           is total (`Bytes → Result<Ast, _>`), so the round-trip matches the `Ok` arm.")
  (input  (match (Ast.decode (Ast.encode (Ast.List (list (Ast.Name "g") (Ast.Int 5)))))
            ((Ok a)  (= a (Ast.List (list (Ast.Name "g") (Ast.Int 5)))))
            ((Err _) false)))
  (output (: true Bool)))

(case "Ast.decode decodes bytes that arrive as a function argument"
  (doc    "`Ast.decode : Bytes → Result<Ast, _>` MUST decode bytes whatever their PROVENANCE — a literal,
           or (here) bytes passed in as a function ARGUMENT. The round-trip cases above decode a literal in
           tail position; this decodes `b`, a parameter of `dec`, which is how a program that reads its
           input decodes it (a compiler receives its program bytes as an argument, not a literal). The
           result is the same total `Result<Ast, _>`, so `(dec (Ast.encode (Ast.Int 42)))` matches the `Ok`
           arm and yields 42. A generation that realizes `Ast.decode` only over a compile-time-constant
           argument (folding it away) declines the runtime-argument form (\"unsupported dotted-application\")
           — but decode is an ordinary total operation on a runtime `Bytes` value, so it MUST run here.")
  (input  (do
            (def (main) (dec (Ast.encode (Ast.Int 42))))
            (def (dec b) (match (Ast.decode b)
                           ((Ok a)  (match a ((Ast.Int n) n) (other -1)))
                           ((Err _) -2))) (export main)))
  (output (: 42 Int64)))

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
           no quasiquote template to insert into — so the compiler rejects it at parse time (CDZ0003,
           the syntax-band unquote-outside-quasiquote code) rather than running the program.")
  (input  ,x)
  (error  CDZ0003))

; An `unquote` nested inside a PLAIN `quote` is still outside any quasiquote context — a `(quote …)`
; body is inert data, not a selective-evaluation template. metaprogramming.md #Quote Produces An AST
; Value: "(quote <expr>) MUST evaluate to an AST value representing the structure of <expr>, WITHOUT
; evaluating <expr> itself"; #Quasiquote Constructs AST With Selective Evaluation: "Unquote and
; unquote-splicing OUTSIDE a quasiquote context MUST be a syntax error." So `(quote (g ,x))` must NOT
; evaluate `,x` — it is exactly the "unquote outside quasiquote" the bare `,x` case above pins, one
; layer of quote deep, and MUST be rejected CDZ0003 (or preserved inert), NEVER evaluated. A compiler
; that treats the plain-quote nesting level as an active quasiquote level silently EVALUATES `,x` under
; `quote`, making `(quote (g ,x))` behave identically to the quasiquote `` `(g ,x) `` — a `quote` that
; is not inert, contradicting #Quote Produces An AST Value. (The companion arity leak: `(quote (unquote
; 1 2))` must be rejected CDZ0201 like `(quasiquote (unquote 1 2))`, not silently truncated to
; `(Ast.Int 1)` — the arity check the quasiquote path enforces applies under plain quote too.)

(case "an unquote nested inside a plain quote is a syntax error, not an active unquote"
  (doc    "`(quote (g ,x))` places an unquote inside a PLAIN quote — still outside any quasiquote
           context (a quote body is inert data, not a selective-evaluation template), so it is the same
           `,`-outside-quasiquote syntax error the bare `,x` case pins, rejected CDZ0003. metaprogramming.md
           #Quote Produces An AST Value forbids `quote` from evaluating its body; a compiler that treats
           plain quote as an active quasiquote level evaluates `,x` and makes `(quote (g ,x))` behave as
           the quasiquote `` `(g ,x) ``. The bug: the active-unquote test fires at quote's own nesting
           level rather than only inside a quasiquote (spec/learnings/2026-07-07-plain-quote-evaluated-a-nested-unquote-instead-of-treating-it-as-inert.md).")
  (input  (quote (g ,x)))
  (error  CDZ0003))

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

(case "quasiquote makes AST construction readable"
  (doc    "Witnesses compiler-pipeline.md #The Compiler Constructs AST Values Via Quasiquote:
           building an AST value via quasiquote is readable. Compare `(op-const ,n) vs
           (Ast.List (list (Ast.Name \"op-const\") n)) — quasiquote reads like the form it builds.
           This is the frontend/macro role quasiquote serves; the compiler's instruction backend
           instead uses a dedicated typed instruction sum built by ordinary constructors and matched
           to bytes (compiler-pipeline.md #The Compiler Operates On AST Values). Note: dotted names
           like i64.const expand to member access; hyphenated names avoid this.")
  (input  (let ((n 42)) `(op-const ,n)))
  (output (: (Ast.List (list (Ast.Name "op-const") (Ast.Int 42))) Ast)))

(case "Ast.decode converts bytes to an AST sum type value"
  (doc    "Witnesses compiler-pipeline.md #The Compiler Operates On AST Values: the compiler receives
           a program as binary bytes and decodes it to an AST sum type value. `Ast.decode : Bytes →
           Result<Ast, _>` is total over possibly-external bytes (value-interchange.md — it never traps),
           so the compiler matches the `Ok` arm and then pattern-matches the AST within it.")
  (input  (match (Ast.decode (Ast.encode (quote 42)))
            ((Ok (Ast.Int n)) n)
            (_                0)))
  (output (: 42 Int64)))

(case "Ast.encode and Ast.decode round-trip"
  (doc    "Witnesses contracts/ast-encoding.md: encoding an AST to binary and decoding it back
           produces the same AST value. The compiler relies on this: it decodes the input, operates
           on AST values, and the encoding is faithful. `Ast.decode` is total (`Bytes → Result<Ast, _>`),
           so the round-trip matches the `Ok` arm and equates its payload to the original.")
  (input  (match (Ast.decode (Ast.encode (quote (+ 1 2))))
            ((Ok a)  (= a (quote (+ 1 2))))
            ((Err _) false)))
  (output (: true Bool)))

(case "decoding bytes that are not a canonical AST yields the error case, not a trap"
  (doc    "contracts/value-interchange.md #Decode Inverts Serialize And Refuses Otherwise + #A Decode Over
           External Bytes Is Total: `Ast.decode` consumes bytes that may come from an EXTERNAL source, so it
           MUST be total — a byte sequence that is not the canonical encoding of any AST yields the error
           case (`Err`), NOT a trap. `(Bytes.of (list 255 255 255))` is not a valid AST encoding, so the
           decode returns `Err` and the program handles it as an ordinary value. This is the fallible-reader
           discipline (like `String.from-bytes`), not reject-don't-miscompile: malformed EXTERNAL input is a
           handleable condition, not a program bug that traps.")
  (input  (match (Ast.decode (Bytes.of (list 255 255 255)))
            ((Ok _)  1)
            ((Err _) 0)))
  (output (: 0 Int64)))

(case "decoding canonical bytes followed by a trailing byte yields the error case"
  (doc    "contracts/deterministic-value-form.md #Decoding Is The Inverse Of The Canonical Byte Form: a byte
           sequence that is valid canonical bytes FOLLOWED BY additional bytes MUST NOT decode as the value
           the valid prefix encodes — the trailing byte is a detected error, not silently ignored. So
           `Ast.decode` of `(encode (Ast.Int 7)) ++ [99]` yields `Err`, not `Ok (Ast.Int 7)`. The total-decode
           companion of the round-trip cases: decode consumes the WHOLE input or reports an error, so a
           truncated or concatenated external input is caught rather than half-read.")
  (input  (match (Ast.decode (Bytes.concat (Ast.encode (Ast.Int 7)) (Bytes.of (list 99))))
            ((Ok _)  1)
            ((Err _) 0)))
  (output (: 0 Int64)))

; ============================================================================================
; Quote patterns — the quasiquote surface in PATTERN position (options/quote-patterns/)
; ============================================================================================
; The same `` ` ``/`,`/`,@` surface that CONSTRUCTS an AST value (above) serves the DUAL direction in
; pattern position: `` `<template> `` inside a `match` DESTRUCTURES an Ast scrutinee, reusing the
; constructor/pattern duality the language already has (a variant `(Some 5)` builds and `(Some n)`
; destructures). A quote pattern is EXACTLY EQUIVALENT to the pattern built from the `Ast.*` sum
; constructors — `` `(+ ,a ,b) `` IS `(Ast.List (list (Ast.Name "+") a b))` as a pattern — so it adds a
; surface, not a new value or a second matching mechanism. In the template: a literal subterm matches by
; equality (an integer as `(Ast.Int n)`, a bare name as `(Ast.Name "…")`), a compound `(h a b)` matches
; an `Ast.List` of EXACTLY that arity element-by-element, `,<pattern>` binds/further-matches the sub-AST
; at its position, and a FINAL `,@<name>` binds the remaining list elements. Exhaustiveness is the
; existing rule (a quote pattern never covers every AST — different head, different arity, a leaf where a
; list is expected all fail — so an Ast match needs a catch-all bare-name/`_` arm or it is CDZ0210), and
; equality/encoding are the constructor form's (a value matched through a quote pattern is matched through
; the very sum patterns the un-tagged cases above already run). NOTE the `,`/`,@` marks are meaningful
; only INSIDE a `` ` `` template — a top-level catch-all arm is an ordinary bare-name or `_` pattern, as
; a bare `,other` outside a quasiquote is the existing syntax error (the "unquote outside quasiquote"
; case above, CDZ0003), not a pattern.
;
; The seed ALREADY destructures AST sums by the `Ast.*` constructors — the un-tagged
; `(match (quote (+ 1 2)) ((Ast.List elems) …))` cases above run on it. The one NEW piece is the
; reader/lowering that recognizes a backtick in PATTERN position and desugars it to those constructor
; patterns. A later generation realizes that lowering
; (options/realized-capability-set/); the seed declines these — they pin the contract the realization
; must meet.

(case "a quote pattern binds an unquoted operand of a compound form"
  (doc    "`` `(+ ,a ,b) `` in pattern position IS `(Ast.List (list (Ast.Name \"+\") a b))` as a pattern
           (options/quote-patterns/quasiquote-pattern.md): the literal head `+` matches `(Ast.Name \"+\")`
           by equality, and `,a`/`,b` bind the two operand sub-ASTs. Matching `(quote (+ 3 5))` binds
           a=`(Ast.Int 3)` and b=`(Ast.Int 5)`; the arm returns b, so the AST for 5. Pins the core
           destructuring: unquote is the binder, a literal subterm matches by equality. The catch-all
           `other` is an ordinary bare-name pattern — `,` is meaningful only inside a `` ` `` template.")
  (input  (match (quote (+ 3 5))
            (`(+ ,a ,b) b)
            (other      other)))
  (output (: (Ast.Int 5) Ast)))

(case "a quote pattern is equivalent to the Ast.* constructor pattern"
  (doc    "A quote pattern lowers to the `Ast.*` constructor pattern, so the two spellings bind
           identically. `` `(+ ,a ,b) `` and `(Ast.List (list (Ast.Name \"+\") a b))` matched against the
           same `(quote (+ 1 2))` both bind a=`(Ast.Int 1)`; comparing the two bound values is true. Pins
           the equivalence the form rests on — the pattern adds a surface, not a second mechanism.")
  (input  (= (match (quote (+ 1 2)) (`(+ ,a ,b) a) (_ (Ast.Int 0)))
             (match (quote (+ 1 2)) ((Ast.List (list (Ast.Name "+") a b)) a) (_ (Ast.Int 0)))))
  (output (: true Bool)))

(case "a literal subterm in a quote pattern matches by equality"
  (doc    "A literal head/subterm matches by equality — the direct analogue of a literal value pattern
           `(match 2 (2 \"two\") …)`. `` `(+ ,a ,b) `` matches only a form headed by `+`; against
           `(quote (- 3 5))`, whose head is `-`, it does NOT match, so control falls to the `other`
           catch-all. Pins that the literal name in the template constrains the head, not merely the
           arity.")
  (input  (match (quote (- 3 5))
            (`(+ ,a ,b) 1)
            (other      0)))
  (output (: 0 Int64)))

(case "a quote pattern matches a fixed arity"
  (doc    "A compound template `` `(f ,a ,b) `` matches an `Ast.List` of EXACTLY three elements — the
           reading of `(Ast.List (list (Ast.Name \"f\") a b))`, whose `(list …)` sub-pattern fixes
           length. `(quote (f 1 2 3))` has four elements, so it does NOT match the two-operand pattern and
           falls to the catch-all. Pins fixed arity: variable length is expressed only through `,@`.")
  (input  (match (quote (f 1 2 3))
            (`(f ,a ,b) 2)
            (other      9)))
  (output (: 9 Int64)))

(case "a nested unquote pattern matches a sub-AST by shape"
  (doc    "`,<pattern>` nests an ordinary pattern at the sub-AST's position, so `` `(+ ,(Ast.Int n) ,b) ``
           matches only an addition whose first operand is an INTEGER LITERAL and binds its value to n.
           Against `(quote (+ 7 x))`, the first operand `(Ast.Int 7)` matches `(Ast.Int n)` binding n=7;
           the arm returns n. Pins that unquote takes a full pattern, not only a bare name.")
  (input  (match (quote (+ 7 x))
            (`(+ ,(Ast.Int n) ,b) n)
            (other                0)))
  (output (: 7 Int64)))

(case "a final unquote-splice binds the remaining elements as a list"
  (doc    "A final `,@<name>` binds the remaining list elements as a LIST (never a single element), the
           pattern-position dual of splicing construction. `` `(f ,@args) `` against `(quote (f 1 2 3))`
           binds args to the list `(Ast.Int 1) (Ast.Int 2) (Ast.Int 3)`; `List.len` of it is 3. Pins the
           tail splice binds the rest and that the elements are a list.")
  (input  (match (quote (f 1 2 3))
            (`(f ,@args) (List.len args))
            (other       0)))
  (output (: 3 Int64)))

(case "a quote pattern used to recognize a compiler form reads as that form"
  (doc    "The self-hosting payoff (options/quote-patterns/quasiquote-pattern.md #Why This Matters For
           Self-Hosting): the compiler's core is a `match` over the decoded AST, and a quote-pattern arm
           reads as the surface it lowers. Here a tiny `lower` distinguishes `(+ …)` from everything else
           by quote pattern; against `(quote (+ 4 6))` it selects the add arm and returns the first
           operand's node. Mirrors the construction idiom `` `(op-const ,n) `` on the pattern side.")
  (input  (match (quote (+ 4 6))
            (`(+ ,a ,b) a)
            (`(- ,a ,b) b)
            (other      other)))
  (output (: (Ast.Int 4) Ast)))

; An Ast match whose arms are only quote patterns does not cover the AST sum — a different head, a
; different arity, or a leaf scrutinee all fail to match — so it is non-exhaustive and rejected CDZ0210,
; exactly as a sum match missing a variant (core-semantics.md #Matching Is Exhaustive Or Rejected). A
; bare-name pattern (equivalently `_`) matches any AST and is the catch-all, so its ABSENCE is what makes
; the match non-exhaustive. Quote matching reuses exhaustiveness rather than adding a rule.

(case "a quote-pattern match with no catch-all is non-exhaustive"
  (doc    "`` `(+ ,a ,b) `` covers only additions; an Ast scrutinee can be a name, an integer, or a
           differently-headed list, none of which it matches. With no bare-name/`_` catch-all arm the
           match does not cover the AST sum and is rejected CDZ0210 — the same rejection a sum match
           missing a variant gets. Pins that quote matching reuses the existing exhaustiveness rule.")
  (input  (match (quote (+ 1 2))
            (`(+ ,a ,b) a)))
  (error  CDZ0210))

; `,@` binds the REST and so is only meaningful as the final element of its template: a `,@` before other
; elements would match a variable-length gap in the middle of a fixed sequence, turning a single
; positional scan into a search. That is an ill-formed quote pattern, rejected CDZ0221 (the CDZ02xx
; types-and-patterns band, the quote-pattern companion of the binary-form CDZ0220). Mirrors `bin`, where
; an unsized `(bytes rest)` is legal only as the final segment.

(case "a non-final unquote-splice in a quote pattern is ill-formed"
  (doc    "`,@<name>` binds the remaining elements, so it is meaningful only as the FINAL element of a
           template. `` `(f ,@init ,last) `` puts `,@init` before `,last`, requiring a variable-length gap
           flanked by a fixed tail — an ill-formed quote pattern, rejected CDZ0221
           (options/quote-patterns/quasiquote-pattern.md #Tail Splice Is Final-Position Only). Mirrors the
           binary-form rule that an unsized `bytes` segment is legal only last.")
  (input  (match (quote (f 1 2 3))
            (`(f ,@init ,last) last)
            (other             other)))
  (error  CDZ0221))

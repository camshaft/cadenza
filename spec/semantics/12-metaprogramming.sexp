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
           no quasiquote template to insert into.")
  (input    ,x)
  (compiler (error CDZ0401)))

(case "quasiquote makes instruction construction readable"
  (doc    "Witnesses compiler-pipeline.md #The Compiler Constructs Instructions Via Quasiquote:
           building instructions via quasiquote is readable. Compare `(i64.const ,n) vs
           (Ast.List (list (Ast.Name \"i64.const\") n)) — quasiquote reads like the instruction.")
  (input  (let ((n 42)) `(i64.const ,n)))
  (output (: (Ast.List (list (Ast.Name "i64.const") (Ast.Int 42))) Ast)))

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

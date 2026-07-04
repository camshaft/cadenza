; Metaprogramming — quote/unquote and the AST as a sum type. Witnesses metaprogramming.md.
; Quote produces an AST value without evaluating; unquote evaluates an AST as code. The AST is a
; sum type deconstructible by pattern matching, so the compiler operates on AST values natively
; rather than using string-tagged reflection.

(case "quote produces an AST value without evaluating"
  (doc    "Witnesses metaprogramming.md #Quote Produces An AST Value (1st sentence): (quote <expr>)
           returns an AST sum type value representing <expr>'s structure, without evaluating <expr>.
           (quote (+ 1 2)) produces an AST value, not 3.")
  (input  (quote (+ 1 2)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2))) Ast)))

(case "unquote evaluates an AST value as code"
  (doc    "Witnesses metaprogramming.md #Unquote Evaluates An AST Value As Code (1st sentence):
           (unquote <ast-value>) evaluates the AST as code. Unquoting the AST of (+ 1 2) produces 3.")
  (input  (unquote (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))))
  (output (: 3 Int64)))

(case "quote and unquote are inverses"
  (doc    "Witnesses metaprogramming.md #Quote And Unquote Are Inverses: (unquote (quote <expr>))
           equals <expr>. Quoting then unquoting (+ 1 2) produces 3, same as evaluating (+ 1 2).")
  (input  (unquote (quote (+ 1 2))))
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

(case "unquoting a malformed AST traps"
  (doc    "Witnesses metaprogramming.md #Unquote Evaluates An AST Value As Code (2nd sentence):
           unquoting an AST that doesn't represent a well-formed expression traps. An Ast.List with
           no elements is malformed (no operator), so unquoting it traps.")
  (input  (unquote (Ast.List (list))))
  (trap   "malformed AST"))

(case "quasiquote quotes with selective evaluation via unquote"
  (doc    "Witnesses metaprogramming.md #Quasiquote Allows Selective Evaluation (1st-2nd sentences):
           `<template> quotes like quote, but ,<expr> (unquote) evaluates <expr> and inserts result.
           `(+ ,x 10) with x=2 produces AST for (+ 2 10), not (+ x 10).")
  (input  (let ((x 2)) `(+ ,x 10)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 2) (Ast.Int 10))) Ast)))

(case "unquote evaluates and inserts the result"
  (doc    "Witnesses metaprogramming.md #Quasiquote Allows Selective Evaluation: unquote inserts
           the evaluated result. `(+ ,(+ 1 1) 10) evaluates (+ 1 1) to 2, then inserts 2.")
  (input  `(+ ,(+ 1 1) 10))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 2) (Ast.Int 10))) Ast)))

(case "unquote-splicing splices a list into the parent"
  (doc    "Witnesses metaprogramming.md #Quasiquote Allows Selective Evaluation (3rd sentence):
           ,@<list-expr> splices list elements into parent, not nested. `(+ ,@args) with
           args=(list 1 2 3) produces (+ 1 2 3), not (+ (1 2 3)).")
  (input  (let ((args (list 1 2 3))) `(+ ,@args)))
  (output (: (Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2) (Ast.Int 3))) Ast)))

(case "unquote-splicing vs unquote differ in list handling"
  (doc    "Witnesses metaprogramming.md #Quasiquote Allows Selective Evaluation: unquote nests the
           list as one element; unquote-splicing flattens it. `(f ,x) vs `(f ,@x) with x=(list 1 2).")
  (input  (let ((x (list 1 2)))
            (= `(f ,x) `(f (list 1 2)))))
  (output (: true Bool)))

(case "quasiquote nests"
  (doc    "Witnesses metaprogramming.md #Quasiquote Allows Selective Evaluation (4th sentence):
           quasiquote nests, so ``(+ ,,x) evaluates inner unquote to produce `(+ ,<x-value>).
           With x=2, ``(+ ,,x) produces `(+ ,2), which when evaluated produces (+ 2).")
  (input  (let ((x 2)) ``(+ ,,x)))
  (output (: (Ast.List (list (Ast.Name "quasiquote")
                             (Ast.List (list (Ast.Name "+")
                                           (Ast.List (list (Ast.Name "unquote") (Ast.Int 2)))))))
             Ast)))

(case "unquote outside quasiquote is an error"
  (doc    "Witnesses metaprogramming.md #Quasiquote Allows Selective Evaluation (5th sentence):
           unquote and unquote-splicing are only valid inside quasiquote. Bare ,x is a syntax error.")
  (input    ,x)
  (compiler (error CDZ0401)))

(case "quasiquote makes instruction construction readable"
  (doc    "Witnesses compiler-pipeline.md #The Compiler Constructs Instructions Via Quasiquote:
           building instructions via quasiquote is readable. Compare `(i64.const ,n) vs
           (Ast.List (list (Ast.Name \"i64.const\") n)) — quasiquote reads like the instruction.")
  (input  (let ((n 42)) `(i64.const ,n)))
  (output (: (Ast.List (list (Ast.Name "i64.const") (Ast.Int 42))) Ast)))

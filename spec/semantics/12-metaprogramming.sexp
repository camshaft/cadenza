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

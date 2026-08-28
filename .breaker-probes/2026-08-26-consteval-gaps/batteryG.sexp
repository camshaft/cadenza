; breaker sweep 6 — isolate WHY cd01 (recursive AST leaf count) declines while the pinned
; Ast.module transforms (12-metaprogramming:2509/2544) fold. Ingredient A/B:
;   cd01 (KNOWN DECLINE): indexed List.at walk, BOTH params const.
;   cg01: list-REST-pattern recursion (corpus style) instead of indexed walk.
;   cg02: indexed List.at walk but the index param NON-const (corpus child style).
;   cg04: rest-pattern count over Ast.module source (content-agnostic > 0).

(case "cg01 leaf count via list-REST-pattern recursion folds (CDZ0304 detector)"
  (input  (do
            (def (leaves (const (: a Ast)))
              (match a ((Ast.List xs) (leaves-list xs)) (_ 1)))
            (def (leaves-list (const (: xs (List Ast))))
              (match xs
                ((list) 0)
                ((list h .. t) (+ (leaves h) (leaves-list t)))))
            (def (main)
              (if (= (leaves (quote (f 1 2))) 3)
                  (trap "cg01 folded three")
                  (trap "cg01 WRONG")))
            (export main)))
  (error  CDZ0304 (message "cg01 folded three")))

(case "cg02 leaf count via indexed walk with NON-const index folds (CDZ0304 detector)"
  (input  (do
            (def (leaves (const (: a Ast)))
              (match a ((Ast.List xs) (leaves-of xs 0)) (_ 1)))
            (def (leaves-of (const (: xs (List Ast))) (: i Int64))
              (match (List.at xs i)
                ((Option.Some c) (+ (leaves c) (leaves-of xs (+ i 1))))
                ((Option.None) 0)))
            (def (main)
              (if (= (leaves (quote (f 1 2))) 3)
                  (trap "cg02 folded three")
                  (trap "cg02 WRONG")))
            (export main)))
  (error  CDZ0304 (message "cg02 folded three")))

(case "cg04 leaf count over the reflected Ast.module folds to a positive count"
  (input  (do
            (def (leaves (const (: a Ast)))
              (match a ((Ast.List xs) (leaves-list xs)) (_ 1)))
            (def (leaves-list (const (: xs (List Ast))))
              (match xs
                ((list) 0)
                ((list h .. t) (+ (leaves h) (leaves-list t)))))
            (def (forms-of (const (: mm Ast)))
              (match mm ((Ast.List fs) fs) (_ (: (list) (List Ast)))))
            (def (main) (> (leaves-list (forms-of Ast.module)) 0))
            (export main)))
  (output (: true Bool)))

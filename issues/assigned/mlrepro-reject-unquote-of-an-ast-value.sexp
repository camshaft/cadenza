;; GAP/BUG (2026-07-14): a quasiquote `unquote` of an expression whose value is ALREADY an `Ast` is
;; rejected — you cannot splice a COMPUTED Ast subtree into a quasiquote template.
;;
;;   `(quasiquote (+ (unquote sub) 1))` with `sub : Ast` →
;;     CDZ0201 "a variant constructor's payload has declared type Int64, but a value of type Ast was
;;     applied"  (cdz check).
;;
;; WORKS: `(unquote n)` where `n : Int64` (or a literal) — the plain value is WRAPPED as `Ast.Int n`
;; and inserted (verified: `(quasiquote (+ (unquote 5) 2))` → a 3-element `Ast.List`). The gap is only
;; when the unquoted value is ALREADY an `Ast`.
;;
;; metaprogramming.md §Quasiquote Constructs AST With Selective Evaluation: ",<expr> MUST evaluate
;; <expr> normally and INSERT ITS RESULT into the AST at that position." When the result IS an Ast,
;; "insert its result" should mean splice that node as-is — not re-wrap it in `Ast.Int(...)`. The current
;; behavior always wraps by the template slot's leaf type, so an Ast-valued unquote type-errors.
;;
;; Impact: the canonical AST-building macro — `(def (wrap sub) (quasiquote (+ (unquote sub) 1)))` that
;; embeds a computed subtree — cannot be written. A compiler/macro layer needs exactly this.
(do
  (def (wrap (: sub Ast)) (quasiquote (+ (unquote sub) 1)))
  (def (main) (match (wrap (Ast.Int 9)) ((Ast.List es) (List.len es)) (_ -1)))
  (export main))

;; LIMITATION (2026-07-14): a LIST pattern arm may contain at most ONE refutable (constructor) element.
;; `(match xs ((list (A.I x) (A.N y) c) …) …)` → "a list arm with more than one refutable constructor
;; element is not yet supported (match one tagged element per arm)". This blocks the natural way to
;; destructure a fixed-shape form — e.g. a constant-folder matching `[Name op, Int x, Int y]` in one arm.
;;
;; ALLOWED: one ctor element + bare binders — `((list (A.I x) b c) …)`.
;; WORKAROUND: bind every element as a plain binder, then nested-`match` each: `((list a b c) (match a
;; ((A.I x) (match b …) …) …))`. Verbose but works. (This is what the port's fold.cdz does.)
;;
;; Ask: support N refutable elements in one list arm (a fixed-length form is the common shape a compiler
;; pass destructures — head + typed args). `cdz check` REJECTS cleanly (not a miscompile), so a Todo.
(do
  (type A (I Int64) (N String))
  (def (f (: xs (List A)))
    (match xs
      ((list (A.I x) (A.N y) c) (+ x 0))
      (_ 0)))
  (def (main) (f (list)))
  (export main))

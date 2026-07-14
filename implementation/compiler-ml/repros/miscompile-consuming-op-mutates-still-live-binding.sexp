;; MISCOMPILE — SILENT WRONG VALUE (2026-07-14, seed rcdzc). The ROOT + MINIMAL form of the
;; persistent-collection mutation family (supersedes the self-call / interpreter repros as the core case).
;; `cdz check` CLEAN; `cdz compile` SUCCEEDS; runs WRONG.
;;
;; A consuming persistent-collection op (`List.push` / `Map.insert` / `Set.insert`) MUTATES its operand
;; IN PLACE when that operand is a binding that is STILL LIVE afterward — so a later read of the same
;; binding sees the mutated value. No recursion, no self-call, no function param needed — a single `let`
;; used twice suffices:
;;
;;   let xs = [7] in (List.len (List.push xs 9)) + (List.len xs)
;;   should be len([7,9]) + len([7]) = 2 + 1 = 3.  IT RETURNS 4  (the second `len xs` sees [7,9]).
;;
;; ORDER-SENSITIVE (the tell): swapping the operands so the BORROW happens BEFORE the consume —
;;   (List.len xs) + (List.len (List.push xs 9))  — returns 3 (CORRECT): the borrow reads `xs` before the
;; `push` mutates it. So `push` consumes `xs` in place; whatever reads `xs` AFTER the push gets the
;; mutated list. The consuming op is not preceded by a `dup` of a binding that outlives it.
;;
;; ROOT: the Perceus last-use analysis. A binding consumed by an op but READ AGAIN LATER must be `dup`'d
;; before the consume (the consuming op takes ownership; the still-live binding needs its own reference).
;; This is the general defect the self-recursive-param case
;; (`miscompile-selfcall-plus-consuming-op-share-param`) and the interpreter case
;; (`miscompile-map-insert-mutates-shared-recursive-param`) are instances of — but THIS is the simplest
;; trigger: a plain `let` binding used by a consuming op AND a later borrow, in ONE expression.
;; Reproduces for `Map.insert` and `Set.insert` too. Blocks ANY pass that builds a modified collection
;; while still needing the original (a diff, a "with one more binding", a before/after comparison).
(do
  (def (main (: d Int64))
    (let ((xs (List.push (list) 7)))
      (+ (List.len (List.push xs 9)) (List.len xs))))
  (export main))

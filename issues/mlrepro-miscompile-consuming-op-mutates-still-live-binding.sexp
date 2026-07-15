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
;; ROOT (WAT-CONFIRMED, iter 34): the emitted body is
;;     call build → local.tee 1        ;; slot1 = xs, AND xs left on the stack
;;     …box 9… vec-push                ;; CONSUMES the stack copy at rc==1 → MUTATES the trie in place
;;     vec-len                          ;; = 3
;;     local.get 1  vec-len             ;; reads slot1 — but the push mutated the SAME cell → 3 (BUG)
;; There is NO `dup` before the consuming `vec-push`, and the tee'd stack copy shares the cell with slot1.
;; A persistent op mutates in place when it receives its operand at refcount 1; the still-live binding
;; needs the op to `dup` its operand first (rc→2: the op consumes one, the slot keeps the other).
;;
;; ⚠ SCOPE (iter 34, corrected from iters 27–29): this is EXCLUSIVELY a `let`-binding-in-body defect.
;;   • A plain PARAMETER used the same way computes CORRECTLY — `(def (g xs) (+ (len (push xs 9)) (len
;;     xs)))` returns 3, because a param is borrowed-from-caller and the call boundary keeps it live.
;;   • Passing a let-binding ACROSS A CALL is also correct (`(let ((xs …)) (g xs))` → 5): the arg is
;;     protected by the boundary (often the callee inlines and RE-EVALUATES its arg, two fresh lists).
;;   • ONLY a consuming op applied to a let-binding DIRECTLY IN THE LET BODY, with a later read of the
;;     same binding, miscompiles. So my earlier "only a self-recursive shared PARAM fails / let-bound is
;;     fine" was BACKWARDS — the param path is the safe one; the let-body path is the broken one. The
;;     self-recursive-param repro (`miscompile-selfcall-plus-consuming-op-share-param`) and the
;;     interpreter repro (`miscompile-map-insert-mutates-shared-recursive-param`) miscompile because
;;     their consuming op + surviving read both land in a LET/ARM BODY over the same slot, not because of
;;     recursion or parameter-sharing per se.
;; Fix locus: the consuming-op emit (`Core::ListPush`/`MapInsert`/`SetInsert` in backend/wasm/select.rs)
;; must `dup` its heap operand when that operand is a `LocalRef`/`Param` still live after the op — i.e.
;; the operand's binding is READ AGAIN in the enclosing body. (A delicate refcount-discipline change:
;; over-dup leaks, so it is a seed-agent job — this repro + the WAT above give the exact missing dup.)
;; Reproduces for `Map.insert` and `Set.insert` too. Blocks ANY pass that builds a modified collection
;; while still needing the original (a diff, a "with one more binding", a before/after comparison).
(do
  (def (main (: d Int64))
    (let ((xs (List.push (list) 7)))
      (+ (List.len (List.push xs 9)) (List.len xs))))
  (export main))

;; MISCOMPILE — SILENT WRONG VALUE (2026-07-14, seed rcdzc, iter 37). A member of the still-live-binding
;; family (iter 34), but a DISTINCT face: `List.concat` corrupts its still-live LHS in a way `List.push`
;; does NOT — and the corruption shows on an ELEMENT read, not the length.
;; `cdz check` CLEAN; `cdz compile` SUCCEEDS; runs WRONG.
;;
;;   let xs = [5, 7] in (List.len (List.concat xs [9])) + (e xs 0)     where e xs i = (List.at xs i) or -1
;;   should be len([5,7,9]) + xs[0] = 3 + 5 = 8.   IT RETURNS 3   (the `e xs 0` read yields None → -1+... ,
;;   in the ML form `0 - 1` default; the net is that xs reads as EMPTY after the concat consumed it).
;;
;; 🔑 THE DISCRIMINATOR (why this is not just iter-34 re-skinned): the IDENTICAL shape with `List.push`
;; instead of `List.concat` computes CORRECTLY (returns 8). So:
;;   (List.len (List.push   xs 9)) + (e xs 0)  → 8   CORRECT
;;   (List.len (List.concat xs [9])) + (e xs 0) → 3   WRONG
;; `push` (vec-push) dup-guards its still-live list operand; `concat` (vec-concat) does NOT — its LHS is
;; consumed at rc==1, so a later read observes the emptied/mutated list.
;;
;; SHARP CONDITIONS (each necessary — drop any and it computes correctly):
;;   • the consuming op is `List.concat` (push/update behave differently; update corrupts too but via a
;;     different signature — see the family; concat specifically empties the LHS for later ELEMENT reads);
;;   • `xs` is a `let` binding still LIVE after the concat (a parameter is safe — the caller owns it);
;;   • the surviving read is `xs[0]` (List.at) reached AFTER the concat result is consumed — reading `xs`'s
;;     LENGTH after the concat is CORRECT (only element reads via List.at see the corruption), and reading
;;     `xs[0]` BEFORE the concat is CORRECT (ORDER-SENSITIVE, like the whole family);
;;   • re-`concat`ing xs a SECOND time is also correct (the LHS handle is not freed — it is the element
;;     ARRAY that reads empty), so this is a rc/dup-timing bug on the element-array, not a use-after-free.
;;
;; ROOT: the `List.concat` emit (`Core::ListConcat` in backend/wasm/select.rs) does not `dup` its LHS
;; operand when that operand is a `LocalRef`/`Param` still read later in the enclosing body — the same
;; missing-dup-before-a-consuming-op defect as iter 34's `List.push`/`Map.insert`/`Set.insert`, but the
;; `push` emit was evidently patched (or never had it) for the length face while `concat` still consumes
;; its LHS in place. A delicate refcount change (over-dup leaks) — a seed-agent job; this repro + its
;; push-twin give the exact op whose emit omits the dup. Reproduces from a fully-inline list (no build fn).
;; Companion: `miscompile-consuming-op-mutates-still-live-binding` (the length face, via push).
(do
  (def (e (: xs (List Int64)) (: i Int64))
    (match (List.at xs i)
      ((Some v) v)
      ((None _) (- 0 1))))
  (def (main (: d Int64))
    (let ((xs (List.push (List.push (list) 5) 7)))
      (+ (List.len (List.concat xs (list 9))) (e xs 0))))
  (export main))

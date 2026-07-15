;; MISCOMPILE (found 2026-07-14, wasm backend): a TAIL-position self-recursive call inside a
;; `MatchList` (list-`match`) CONS arm returns the wrong value (the seed accumulator, recursion lost).
;;
;; This program should return 15 (sum of [5,5,5]) but returns 0.
;;
;; input:
(module m
  (def (sum-acc (: xs (List Int64)) (: acc Int64))
    (match xs
      ((list) acc)
      ((list x .. rest) (sum-acc rest (+ acc x)))))
  (def (main) (sum-acc (list 5 5 5) 0))
  (export main))
;; expected: (: 15 Int64)
;; actual:   (: 0 Int64)
;;
;; ── ROOT CAUSE (diagnosed) ────────────────────────────────────────────────────────────────────────
;; The recursion is a TAIL call: the whole cons-arm body IS `(sum-acc rest (+ acc x))`. Contrast:
;;   - `(+ x (sum rest))`  (NON-tail, result consumed by `+`) → WORKS (user's original sum → 15)
;;   - `(+ 1 (cnt rest acc))` (non-tail) → WORKS (returns 3 for 3 elems)
;;   - `(sum-acc rest (+ acc x))` (TAIL) → returns 0 (the seed acc, unchanged by recursion)
;;   - the same TAIL shape over a NUMERIC `if` scrutinee (`(if (= n 0) acc (cnt (- n 1) (+ acc n)))`)
;;     → WORKS + loops (5050). So the defect is specific to a list `match` (`Core::MatchList`).
;;
;; MECHANISM: `select.rs::emit_tail` handles `Core::Call`/`If`/`Let`/`Match` (SCALAR match) but NOT
;; `Core::MatchList` (nor `Core::MatchSum`). And `body_has_member_tail_call`/`tail_callees` (which drive
;; the loop transform) likewise omit `MatchList`/`MatchSum`. So a tail self-call in a list-match arm is
;; (a) never turned into a loop, and (b) emitted through a path that mishandles the tail-`Core::Call`
;; result. A non-tail recursive call (consumed by an enclosing op) goes through the ordinary `emit`
;; path (correct); only the TAIL call in a list-match arm is wrong.
;;
;; FIX DIRECTION: teach `emit_tail` to thread tail position into `Core::MatchList` (and `MatchSum`) arm
;; bodies (so a tail `Core::Call` there becomes a `return_call` / loop iteration), AND add both variants
;; to `body_has_member_tail_call` + `tail_callees` so the loop transform fires (constant-stack loop, the
;; numeric shape's behavior). Verify: the emit must place the arm result correctly AND, when looped,
;; thread the recursion args through the param slots. This also unblocks the accumulator-introduction
;; INCREMENT 5 (list-fold) attempt (see [[rcdzc-accumulator-introduction]] increment-5 note) — that
;; transform was correct but its output hit this same backend gap.

;; ── UPDATE (cycle 144, deeper investigation) ──────────────────────────────────────────────────────
;; Attempted the fix (add MatchList to emit_tail + body_has_member_tail_call + tail_callees, with a new
;; `emit_list_arms_tailable` threading TailPos + deeper_tail). RESULT: regression-free (full suite
;; 1302/0) AND the list fold now LOOPS and ITERATES correctly for a CONSTANT combine — `(cntdown xs acc)
;; = (match xs ((list) acc) ((list x .. rest) (cntdown rest (+ acc 1))))` returns the right length (2
;; for a 2-elem list). BUT a fold that reads the HEAD ELEMENT `x` still returns wrong: `(+ acc x)` reads
;; `x` as 0 (proven: `(+ acc (+ x 1000))` returns 1000, not 1042). So there are TWO bugs:
;;   BUG 1 (fixed by the attempt): MatchList tail calls weren't loop-transformed / tail-emitted.
;;   BUG 2 (STILL OPEN): the head binder `x` (a SumPayload read of the scrutinee) reads 0 when the arm
;;     is a TAIL CALL. It reads CORRECTLY in a NON-tail arm (`(+ x (sum rest))` → correct) and
;;     non-recursively (`(+ x (List.len rest))` → correct). So BUG 2 is specific to reading the head
;;     during a tail-call's argument evaluation (`emit_loop_iteration` / the tail-`Core::Call` arg emit),
;;     where the scrutinee slot is also the loop's list param being restored — likely an eval-order /
;;     slot-restore interaction: the head `x = vec-get(list-slot, 0)` and the tail `rest =
;;     vec-drop(list-slot, 1)` both read the list param slot, evaluated during the same parallel-move that
;;     will overwrite that slot; the head read ends up seeing 0.
;; ⚠ The base spec was ALREADY broken (returns 0 with AND without the attempt) — BUG 2 is pre-existing,
;; NOT introduced. The attempt was REVERTED (it enables a loop over a still-miscompiling fold — muddies
;; correctness without fixing BUG 2). A COMPLETE fix must address BOTH: enable the loop (BUG 1 fix is
;; correct + regression-free, reconstructable) AND fix the head-read-0 in the tail-call arg eval (BUG 2).
;; Reconstruct BUG-1 fix from this note; then root-cause BUG 2 in emit_loop_iteration's arg evaluation
;; (all args read OLD param values before any store — verify the head-read actually reads the OLD slot).

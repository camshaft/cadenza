;; SHARPER BOUND (2026-07-14) of the sum-in-tuple-through-a-recursive-fn miscompile. Trigger: inside a
;; SELF-RECURSIVE function, project a boxed-sum element out of a tuple that was built by an `if`
;; (`(. (one …) 0)` where `one` returns `(if … (tuple (W.Atom …) pos) (tuple (W.Atom …) pos))`), then
;; `match` it. The result is INVALID WASM ("expected i64, found i32" at the recursive fn) — even when
;; the recursive branch never runs (here `n = 0` takes the base case, yet the module still fails to
;; validate), so it is the loop-transform ANALYSIS mis-slotting, not the recursive path executing.
;;
;; The self-tail-loop sibling (miscompile-tail-loop-projected-sum-arg.sexp) gives a silent WRONG VALUE
;; instead of invalid wasm — same root (a projected boxed-sum i32 handle mis-typed by the loop
;; transform), two faces. Common essential ingredient: the `if` INSIDE the tuple-producing function.
;; CONTROLS (return 5): the SAME compose in a NON-recursive `main` (W1/W2 in the session notes), or
;; `one` with a single branch (no `if`).
(do
  (type W (Atom Int64) (Node (List Int64)))
  (def (one (: b Bytes) (: p Int64))
    (if (= (Option.expect (Bytes.at b p) "t") 0)
      (tuple ((. W Atom) (Option.expect (Bytes.at b (+ p 1)) "v")) (+ p 2))
      (tuple ((. W Atom) 99) (+ p 2))))
  (def (wval (: s W)) (match s (((. W Atom) v) v) (((. W Node) _) 0)))
  (def (loop (: b Bytes) (: p Int64) (: n Int64))
    (if (= n 0) (wval (. (one b p) 0)) (+ 0 (loop b p (- n 1)))))
  (def (main (: p Int64)) (loop b"\x00\x05" p 0))
  (export main))

;; RESOLVED 2026-07-15 (trunk@dd77ccc1b): VERIFIED FIXED — compiles to valid wasm + runs to the correct value (graded via (case) wrapper). The invalid-wasm/mis-typed-projection face is closed.

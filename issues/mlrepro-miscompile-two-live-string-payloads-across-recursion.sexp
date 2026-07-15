;; MISCOMPILE — SILENT WRONG VALUE (2026-07-14, seed rcdzc). `cdz check` CLEAN; `cdz compile -t wasm`
;; SUCCEEDS; runs and returns the WRONG count. A recursive walk over a binary tree whose nodes carry a
;; STRING key, where at each node BOTH the node's own key AND its child's key are consulted (two matched
;; `String` sum-payloads live at once) across a >=3-deep recursion, drops one of the per-node decisions.
;;
;; `pc` counts nodes whose LEFT child binds looser: `(< (top l) (pv op))` where `pv op` reads the node's
;; own key's precedence and `top l` reads the LEFT CHILD's key's precedence (both `String` → Int64 via a
;; `Map String Int64`). On `c{ b{ a{L,L}, L }, L }` the correct count is 2 (c: top(b)=2<3 → 1; b:
;; top(a)=1<2 → 1; a: top(Leaf)=99<1 → 0). It returns 1.
;;
;; SHARP BISECTION (2026-07-14):
;;   - The IDENTICAL tree with an Int64 key instead of String (control below, `pc-int`) returns 2. So it
;;     is specific to the STRING payload, not the tree/recursion logic.
;;   - Using the node's key ONCE per node (`(< (pv op) 3)`, a constant threshold — parent key only) → 2 (OK).
;;   - Using only the CHILD's key (`(< (top l) 3)` — parent key UNUSED) → 2 (OK).
;;   - Using BOTH at the same node (`(< (top l) (pv op))` — parent key AND child key live together) → 1 (BUG).
;;   - A 2-DEEP tree of the both-keys form returns the right answer; the miscompile needs depth >= 3.
;; So the trigger is: TWO matched `String` sum-payloads (a node's own and its child's) live SIMULTANEOUSLY
;; at one node, across a recursion at least 3 deep. This is the borrow/ownership-of-a-matched-heap-payload
;; family (cf. `decline-borrow-map-lookup-returned-then-matched`, `miscompile-runtime-string-at-content-
;; equality`): a matched String payload's lifetime is mishandled when a second one overlaps it, so one
;; read gets a stale/wrong value that flips its comparison. Depth-threshold sensitivity (like the
;; slot-alias family) points at the same borrow-analysis-at-scale root.
;;
;; IMPACT: this is a pretty-printer's parenthesization pass — decide if a child needs parens by comparing
;; the child operator's precedence to the parent's. The natural formulation (both operators are String-
;; named, looked up in a precedence Map) silently produces the wrong parenthesization on a nested
;; expression. `src/prec.cdz` uses this exact shape and its `paren-count-nested` @test fails because of it.
(do
  (type T (Leaf Int64) (Node String T T))
  (def (pv (: op String))
    (match (Map.lookup (Map.insert (Map.insert (Map.insert (map) "a" 1) "b" 2) "c" 3) op)
      (((. Option Some) p) p)
      (((. Option None) _) 0)))
  (def (top (: t T)) (match t (((. T Leaf) _) 99) (((. T Node) op _ _) (pv op))))
  (def (pc (: t T))
    (match t
      (((. T Leaf) _) 0)
      (((. T Node) op l r) (+ (if (< (top l) (pv op)) 1 0) (+ (pc l) (pc r))))))
  (def (main (: d Int64))
    (pc ((. T Node) "c"
          ((. T Node) "b" ((. T Node) "a" ((. T Leaf) 0) ((. T Leaf) 0)) ((. T Leaf) 0))
          ((. T Leaf) 0))))
  (export main))

; PERF FINDING (v-compiler-perf, 2026-07-15) — 🔴 EXPONENTIAL compile time (and exponential decision-tree
; SIZE for a runtime scrutinee): a match whose arms each test TWO OR MORE literal columns
; (`(tuple tag1 tag2 payload)` — a transition-table / parser-state shape) makes `lower::build_tree` run in
; O(2^arms). A modest 20-arm table already takes ~6s; a 25-arm one hangs the compiler for minutes. This is
; realistic code — a lexer/parser dispatch on `(tuple state token)`, or any table keyed by two tags.
;
; MECHANISM: `build_tree`'s lit-test arm (lower.rs ~7036) lowers a row carrying N literal tests as a
; `LitTest{then_, els}`. `els = build_tree(rows[1..])` (the fall-through). For a MULTI-test arm the MATCHED
; branch's row still carries its remaining tests, so `then_` recurses into a matrix that STILL contains
; `rows[1..]`, and `rows[1..]` is lowered AGAIN in that inner else — so the shared fall-through tail is
; re-lowered in BOTH the matched-arm subtree and the outer else → T(N) = 2·T(N-1) = O(2^N). The emitted
; `SumCont` tree is itself exponentially SIZED (each `LitTest.els` is an owned `Box`, the tail is DUPLICATED
; 2^N times), so this is not only a build-time cost — a runtime scrutinee emits an exponential module.
; (A CONSTANT scrutinee folds the tree away, so `cdz compile` of a constant-fed call stays 92 bytes while
; `cdz check` still pays the exponential build.)
;
; SINGLE-column literal arms do NOT explode (the matched row becomes a LEAF after consuming its one test, so
; `then_` is a Leaf and the tail is not re-lowered there — that WIDE single-column O(N²) was the fix landed
; at b4e5e6a2, "skip the wasted leaf-row tail clone"). The exponential blowup is specific to arms with ≥2
; literal columns, which that fix does not touch.
;
; MEASUREMENTS (`cdz check`, trunk@~2ac25eab, release cdz):
;   arms (tuple i i a):  N=5 → 14ms   N=10 → 20ms   N=15 → 146ms   N=20 → 6118ms   (~7× per +5 arms)
;   arms (tuple i a):    N=400 → 13ms  N=800 → 23ms  N=1600 → 67ms  N=3200 → 75ms   (LINEAR — the b4e5e6a2 fix)
;
; REPRODUCER (a 2-column literal match; grows exponentially with the arm count):
(module m
  (def (f (: t (Tuple Int64 Int64 Int64)))
    (match t
      ((tuple 0 0 a) 0)
      ((tuple 1 1 a) 1)
      ((tuple 2 2 a) 2)
      ((tuple 3 3 a) 3)
      ((tuple 4 4 a) 4)
      ((tuple 5 5 a) 5)
      ((tuple 6 6 a) 6)
      ((tuple 7 7 a) 7)
      ((tuple 8 8 a) 8)
      ((tuple 9 9 a) 9)
      ((tuple 10 10 a) 10)
      ((tuple 11 11 a) 11)
      ((tuple 12 12 a) 12)
      ((tuple 13 13 a) 13)
      ((tuple 14 14 a) 14)
      ((tuple 15 15 a) 15)
      ((tuple 16 16 a) 16)
      ((tuple 17 17 a) 17)
      ((tuple 18 18 a) 18)
      ((tuple 19 19 a) 19)
      (_ -1)))
  (def (main) (f (tuple 3 3 5)))
  (export main))
;
; The FIX is not a one-tick perf tweak: the decision tree must SHARE the fall-through continuation across
; the matched-arm subtree and the else (a join point / decision DAG) rather than duplicating it. That is
; either (a) memoize `build_tree` on the row-matrix and Rc-SHARE the resulting `SumCont` (needs `SumCont`
; children behind `Rc` + a backend that emits a shared cont once and branches to it — a `block`/label join),
; or (b) restructure the lit-test lowering so an arm's later tests nest UNDER the first test's `then_`
; without re-threading the whole tail. Both touch the core match-lowering seam + the wasm backend's
; `emit_sum_cont`, so this is a focused multi-tick vertical, filed here for tracking. Owned by
; v-compiler-perf.

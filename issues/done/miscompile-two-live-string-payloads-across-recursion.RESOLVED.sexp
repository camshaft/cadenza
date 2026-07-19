; MISCOMPILE — SILENT WRONG VALUE (found via the compiler-to-Cadenza port, root-caused 2026-07-14): a
; recursive walk over a tree whose nodes carry a STRING key, where at each node BOTH the node's own key AND
; its child's key are matched (TWO `String` sum-payloads live at once) across a recursion at least 3 deep,
; drops one of the per-node decisions → wrong count. `cdz check` is CLEAN and `cdz compile -t wasm` SUCCEEDS
; (valid wasm); the run returns the wrong answer.
;
; `pc` counts nodes whose LEFT child binds looser: `(< (top l) (pv op))` — `pv op` reads the node's OWN
; key's precedence, `top l` reads the LEFT CHILD's key's precedence (both `String` → Int64 via a
; `Map String Int64`). On `c{ b{ a{L,L}, L }, L }` the correct count is 2 (c: top(b)=2<3 → 1; b:
; top(a)=1<2 → 1; a: top(Leaf)=99<1 → 0). It returns 1.
;
; SHARP BISECTION (verified):
;   - IDENTICAL tree with an Int64 key instead of String            → 2  (OK — not the tree/recursion logic)
;   - the node's key used ONCE per node (`(< (pv op) 3)`, constant)  → 2  (OK — parent key alone)
;   - only the CHILD's key (`(< (top l) 3)`, parent key unused)      → 2  (OK — child key alone)
;   - BOTH at the same node (`(< (top l) (pv op))`)                  → 1  (🔴 BUG — two String payloads live)
;   - a 2-DEEP tree of the both-keys form                           → correct; the bug needs depth >= 3.
;   So the trigger is: TWO matched `String` sum-payloads (a node's own and its child's) live SIMULTANEOUSLY
;   at one node, across a recursion at least 3 deep.
;
; ROOT (same family as `decline-borrow-map-lookup-returned-then-matched` and
; `miscompile-runtime-string-at-content-equality`): a matched `String` sum-payload's lifetime is mishandled
; when a SECOND matched String payload overlaps it — one read gets a stale/wrong value that flips its
; comparison. The depth>=3 threshold (like the slot-alias family) points at the borrow-analysis-at-scale
; root: the ownership/liveness of a heap value MATCHED OUT of a sum is not tracked correctly when two such
; live payloads coexist, so the second overwrites or aliases the first's slot.
;
; FIX (hypothesis): the same ownership-of-a-matched-heap-payload analysis the String.at / Map.lookup family
; needs — a `String` (or other heap value) bound by a sum-match arm must keep a distinct owned/borrowed
; handle for its whole live range, so two simultaneously-live matched payloads do not share a slot. Fixing
; the projected-value ownership analysis (backend/wasm/select.rs `heap_operand_ownership` + the match-arm
; binder liveness) should clear this alongside the sibling String miscompiles.
;
; SEVERITY: SILENT WRONG-VALUE MISCOMPILE (valid wasm, no diagnostic) — a PRETTY-PRINTER's parenthesization
; pass: decide whether a child needs parens by comparing the child operator's precedence to the parent's.
; The natural formulation (both operators String-named, looked up in a precedence Map) silently produces the
; wrong parenthesization on a nested expression. The port's `src/prec.cdz` `paren-count-nested` @test fails
; on exactly this. Oracle 2 confirmed by the Int64-key control (identical tree/recursion logic), which
; compiles and runs to 2.

(case "a recursive walk consulting a node's own and its child's String key computes correctly"
  (doc    "A binary tree whose `Node`s carry a `String` operator key; `pc` counts nodes whose left child
           binds LOOSER — `(< (top l) (pv op))`, comparing the LEFT CHILD's key precedence `(top l)` to the
           node's OWN key precedence `(pv op)` (both looked up in a `Map String Int64`). On the nested tree
           `c{ b{ a{L,L}, L }, L }` the count is 2 (c: top(b)=2 < 3 → 1; b: top(a)=1 < 2 → 1; a: 99 < 1 →
           0). Today it returns 1 — with TWO matched `String` sum-payloads (the node's own key and its
           child's) live at once across a recursion ≥3 deep, one read gets a stale value and its comparison
           flips (a silent wrong value; the wasm validates). The IDENTICAL tree with Int64 keys returns 2
           (pinning the oracle and that the tree/recursion logic is right); using only one key per node also
           returns 2. The pretty-printer's precedence-parenthesization pass. A generation that keeps each
           matched String payload's ownership distinct across its live range runs it to 2.")
  (input  (do
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
            (export main)))
  (call   main (: 0 Int64)) (output (: 2 Int64)))

;; RESOLVED 2026-07-15 (trunk@2ac25eab): fix landed, gate PASSes. Agent self-removed.

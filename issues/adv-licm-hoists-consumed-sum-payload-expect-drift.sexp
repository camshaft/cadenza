; ADVERSARIAL FINDING (breaker, 2026-07-16) — 🔴 MISCOMPILE (silent loop-carried drift, wasm only):
; the SumExpect face of the JUST-FIXED LICM consumed-heap-invariant bug (aac1b72bc). That fix guards
; a heap-TYPED hoist root via is_heap_type on the Proj root — but `(Option.expect s "v")` (a
; SumExpect extraction) over a loop-threaded Option is still hoisted with ONE prologue dup while the
; body CONSUMES it (List.push) per iteration: iteration 2 onward FBIP-mutates the shared payload in
; place and the accumulated lengths DRIFT 3,4,5,6,… instead of 3,3,3,3.
;
; REPRODUCER (wasm trunk@0e799205a, fresh store; Rust backend CORRECT on the same program):
;   go(s, n, acc) = if n=0 then acc else go(s, n-1, acc + List.len(List.push(Option.expect(s,"v"), 9)))
;   main d = go(Some [d, 8], 4, 0)
;   → 18   WANT 12 (each iteration: len(push([7,8],9)) = 3; 4 iterations = 12)
;
; ISOLATION (hand-verified; the drift threshold + the working faces):
;   n=1 → 3   ✓ (one dup covers the first consume)
;   n=2 → 7   🔴 (3 + 4 — the second consume mutated the shared payload)
;   n=4 → 18  🔴 (3+4+5+6)
;   straight-line DOUBLE consume of the same payload (no loop)      → 6  ✓ (the sibling-liveness retain works)
;   the expect hoisted MANUALLY to a let, list consumed per iter    → 12 ✓ (a List-typed param root is guarded)
;   tuple-projection root `(. pr 0)` in the same loop               → 12 ✓ (the landed fix's own face)
;   record-field root `(. r f)` in the same loop                    → 12 ✓
;   BOTH backends compared: rust = 3/6/6/12 all correct → the bug is the wasm LICM emit, exactly like
;   the fixed Proj face.
;
; ROOT CAUSE (hypothesis): collect_hoistable's is_heap_type guard keys on the hoist ROOT's Core
; head; the Proj arm is covered but the SumExpect head (Option.expect / Result.expect extraction)
; is not — so a consumed sum-payload extraction still hoists with hoist-time refcounts. Fix: the
; same heap-typed refusal on a SumExpect root (or key the guard on the root's TYPE, not its head —
; which the commit message says was the intent).
;
; SEVERITY: 🔴 silent wrong value in a loop over an Option-carried collection — the compiler-in-ML
; port's resolver threads exactly this shape (an env Option threaded through a walker). Same class
; as the fixed face, one Core head over. Graded case below Fails on wasm, passes on rust.

(case "a sum-payload loop invariant consumed per iteration accumulates stably"
  (doc    "`go(s, n, acc) = go(s, n-1, acc + List.len(List.push(Option.expect s, 9)))` over
           `Some [7, 8]` — the extracted payload is loop-INVARIANT and CONSUMED (List.push) each
           iteration, so each push must path-copy the still-shared [7, 8]: every iteration adds 3 and
           four iterations give 12. Instead the wasm LICM hoists the SumExpect extraction with one
           prologue dup (the aac1b72bc guard covers a Proj root but not a SumExpect root), so from
           iteration 2 the consume FBIP-mutates the shared payload and the sums drift 3+4+5+6 = 18.
           The Rust backend computes 12 (backend disagreement). Same class as the fixed
           tuple-projection face — the guard must refuse ANY heap-typed hoist root, whatever its
           extraction head. Expected: 12.")
  (input  (do
            (def (go (: s (Option (List Int64))) (: n Int64) (: acc Int64))
              (if (= n 0) acc
                  (go s (- n 1) (+ acc (List.len (List.push (Option.expect s "v") 9))))))
            (def (main (: d Int64))
              (go (Option.Some (List.push (List.push (list) d) 8)) 4 0))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 12 Int64)))

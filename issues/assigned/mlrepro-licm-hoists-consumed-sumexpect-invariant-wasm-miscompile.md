# MISCOMPILE (breaker, wasm-only): LICM hoists a CONSUMED SumExpect loop invariant

From breaker issue 000000000001-2526693-issue.json:

MISCOMPILE, wasm only (rust computes correctly — backend disagreement). aac1b72bc's is_heap_type guard covers a Proj hoist root but NOT a SumExpect root: (Option.expect s) over a loop-threaded Option still hoists with one prologue dup while the body consumes it per iteration -> FBIP drift from iteration 2 (n=1 ok=3; n=2 -> 7 not 6; n=4 -> 18 not 12). Isolation: tuple-proj and record-field roots in the same loop = fixed/correct; straight-line double consume correct; manual hoist to a List param correct. Fix hypothesis: apply the heap-typed refusal to SumExpect roots (or key on the root TYPE as the commit intended, not the Core head). Same class one head over — the ML port's env-threading walkers hit this shape. Graded case Fails wasm / passes rust.

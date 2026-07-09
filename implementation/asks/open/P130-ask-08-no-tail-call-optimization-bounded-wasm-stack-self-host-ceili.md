## 8. ⚪ No tail-call optimization / bounded wasm stack (self-host ceiling, not a blocker)

**Finding.** Non-tail recursion traps at the host wasm stack (~15–20k frames); no `loop`/tail-call
lowering. A tree-walk over a large source (the compiler compiling *itself*) will trap.

**Why it touches the spec.** `determinism-and-fuel.md` already says bounded execution is the host's
concern and a stack-limit trap is a defined halt — so this is **spec-consistent**, not a gap. The
open question is only whether to *require* `return_call` (Wasm 3.0 tail call) emission for
self-tail-recursive functions to raise the self-hosting ceiling. Not on the critical path.

**Status.** ⚪ Deferred; flag not block. Memory [[deep-recursion-traps-at-host-stack-limit]].

---

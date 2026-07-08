# Implementing effects in the seed: inlining resolves cross-function effects — until recursion

*2026-07-06*

**What happened.** The classify-first design
([[2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code]]) was executed in
the seed compiler: Stages 0–3 landed (gate 469 pass, 0 fail). The intra-program effect layer — the #1
self-hosting blocker — now runs on stock wasm with zero continuation machinery, across function
boundaries (including recursion), composed with the host boundary. Four findings were sharp enough to
record.

**1. Cross-function effect resolution is inlining for the non-recursive case; recursion needs
monomorphization by args + multi-value return.** The design's "effect-context monomorphization"
reduces, for a NON-recursive callee, to the machinery the seed already has for a lambda argument: when a
router (`handle`/`host`) is active and a called function (transitively) performs an effect, INLINE it
(`gen_call` binds params to arg nodes as aliases and emits the callee body under the caller's router
stack). All six cross-function corpus cases turn green with **no new mechanism**. But a **recursive**
effectful function inlines its own body without bound (a compile hang, caught by an `inlining:
Vec<String>` guard on `FnCtx`), so it is discharged by **effect-context monomorphization**: emit the
function ONCE PER HANDLER CONTEXT as a real wasm function whose enclosing handlers' states are threaded
as **hidden trailing parameters and returned as extra results** — `f#ctx(orig-params…, s_in…) ->
(result, s_out…)`. The self-call resolves to the same specialization (same context key) → a plain
`call`, terminating the recursion. Crucially this is **args + multi-value return, NOT a mutable
global**: a global holds one live state per effect and clobbers the moment an effect nests or a
recursion re-installs a handler; threading each context's state on the call stack gives every handler
context its own state, so nested/wrapped effects compose — witnessed by a corpus case that threads TWO
handler states (a countdown governing depth AND an accumulator folded across steps) through one
recursion, and by mutual recursion. The specialization registry reserves a slot before emitting the
body (so the self-call finds it) and appends the specialized functions after the user functions.
**Add the recursive-effect test BEFORE you trust cross-function inlining** — the non-recursive cases
pass without any recursion handling, so the wall is invisible until you write the recursive one.

**1b. The unbounded-context case must decline, not crash — and the guard's depth is set by the SMALLEST
target stack.** A recursive function that installs a FRESH `(handle …)` wrapping its own recursive call
grows the handler context by one frame per recursion, so every self-call has a DISTINCT context key and
interning never converges — unbounded specializations, which overflowed the compiler stack (a CRASH,
the one thing forbidden). This is the genuine limit of monomorphization (no finite specialization set
covers an unbounded context — it needs reified continuations, a general-one-shot tier). **First guard
was a per-function specialization COUNT cap (64) — too deep:** each new context recursively emits
another specialization body, and the count guard trips only after 64 nested `emit_specialization_body`
frames, which the NATIVE compiler survives but the **wasm-compiled compiler's smaller stack does not**
— the differential component-check caught native-declines-vs-wasm-traps as a DISAGREEMENT. Fix: guard
on the handler-context DEPTH (`handlers.len() > 8`) EARLY (before interning/emitting), so the recursion
bottoms out at ~8 frames on either target. **Lesson: a decline guard against runaway recursion must
trip shallow enough for the smallest stack the compiler runs on (wasm), not just the host** — and the
two-compilers component-check is what surfaces a guard that is target-dependent. A corpus case pins the
decline so it can never regress to a crash. **A recursive effectful function has two axes: runtime
recursion DEPTH (fine — one specialization, the loop runs at run time) vs compile-time context GROWTH
(declines — a handler per call).** The guard distinguishes them; a fixed handler with a large runtime
seed (count to 50) is one specialization and compiles.

**2. The host-delegation extern name forced the interface encoding — the component model rejects a dot.**
A delegated operation `log.emit` cannot be a top-level component **import name**: the component model
requires kebab-case extern names, and `wasm-tools validate` rejects `log.emit` ("not in kebab case").
This is exactly why the design mandated **effect = WIT interface, op = function in it**: import an
INSTANCE named `log` (kebab-valid) whose exported function is `emit`. The dot lives only in the flat
`effect.op` string the host records as the observed call — never as a component extern name. The
`interface heap` runtime shape was the proven template; `host_import_component` was rewritten to emit
one instance-type import per effect (grouped from the flat `effect.op` manifest), and the host
(`bind_host_imports`) binds interface-nested funcs, recording each call as `effect.op`. `Unit`
params/results carry no boundary representation and are stripped, so `ask : Unit -> Int64` imports as
`ask: func() -> s64`.

**3. `handle`/`host`/perform must be transparent to BOTH inference and shape inference, or the
runtime-compound path silently misfires.** A `Fresh.next` counter threaded through
`(tuple (label) (label))` produced an *invalid component* (`i32` vs `i64` at the add helper) — because
`infer_list` had no `handle` arm, so `main`'s return kind stayed at the `Int64` default instead of
`Heap`, `call_base` was never shifted to `RT_FUNC_BASE`, and the tuple constructor emitted its helper
calls at scalar-path indices. The fix is small (a `handle`→body / `host`→body / perform→op-result arm
in `infer_list`, and the mirror in `shape_of_list`), but the lesson is structural: any new value-form
that can be a function's tail must be added to the SAME three places — `emit` (codegen), `infer_list`
(so the return kind flows), and `shape_of_list` (so a compound result drives the runtime-compound
renderer). Miss one and the paths disagree; the symptom is an invalid component, not a decline.

**State threading is a mutable wasm local, and the value it hands back reads the OLD state.** A
non-unit handler state (`Fresh` counter, `Diag` list) is a wasm local seeded to `<init>`; a tail
`(resume value next-state)` emits `value` (leaving it on the stack), then `next-state`, then
`local.set` — both expressions read the local BEFORE the set, so `(resume s (+ s 1))` hands back the
current `s` and advances to `s+1`. The unit-state case (seed `unit`, thread `s` unchanged) allocates no
local and emits no threading — byte-identical to a stateless inline, which is why collapsing the old
TailPure/TailState split costs nothing. The heap stays immutable; the mutation is the threaded scalar.

Related: [[2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code]] (the
design this executes), [[2026-07-05-effects-are-declared-with-one-surface-the-declaration-is-the-grant]]
(the routing model — declaration is routing-agnostic, the entrypoint delegates),
[[2026-07-05-authoring-the-compiler-in-cadenza-surfaces-the-language-gaps]] (why effects were
the #1 blocker).

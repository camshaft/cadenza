# Effects: a genuine FOLD of an effectful helper that performs UNDER a conditional, in a recursive self-call arg

## ✅✅ RESOLVED 2026-08-14 (trunk 4259bbfff) — the FOLD gap this tracks is CLOSED; verified + corpus-pinned
The remaining FOLD gap (a helper performing UNDER an `if`, on a recursive self-call arg) now FOLDS correctly
on trunk — no longer a clean decline. Verified with a freshly-built worktree-local `cdz`: both the if-BRANCH
shape (`if c then acc + B.b(x) else acc`) and the if-CONDITION-guard shape (`if B.b(x) = 1 then …`) COMPILE
and produce the CORRECT value (hand-traced + empirically confirmed). Pinned as a corpus case in
`spec/semantics/14-effects-and-handlers.sexp` ("a non-recursive helper that performs the discharged op UNDER
a conditional, in a recursive self-call arg, folds"): `loop 4` seed n folds to `2n+1` — main(1)=3, main(0)=1,
PASS on all three backends (wasm / rust / rust-async), baselined. The consumer (v-agent-harness) no longer
needs the inline workaround for this shape. Was fixed by an intervening branch-threading/specialization
landing (the clean-decline facet closed at `ed3ea9561`/#1415; the fold itself now works). Closing the assigned
item; the historical notes below are retained for provenance.

**Owner: v-effects. NON-URGENT (declines CLEANLY as of `ed3ea9561`, no miscompile / no leak). A future
enhancement — the consumer (v-agent-harness) inlines the helper as a workaround.**

## Status
The effectful-helper-in-a-recursive-self-call-arg family is otherwise COMPLETE (v-agent-harness Inc-3):
- FOLDS (merged `00a5342b2` + `73f9dd5e9`): a helper that performs on its UNCONDITIONAL spine —
  single-param, unannotated, two-param, driver-param-reading, even 2-nested-effect. e.g.
  `turn(a,acc) = acc + Tools.dispatch(a)` in `run(fuel-1, turn(fuel,acc))` → folds.
- DECLINES CLEANLY (`ed3ea9561`): a helper that performs UNDER a conditional (`if`/`match`) — a perform in
  a branch (`if c then acc + B.b(x) else acc`) OR in the if-condition (`if B.b(x)==1 then …`). Was a
  confusing CDZ0101 leaking `f#ctx$s0`; now an honest "not yet reducible" todo.

## The remaining FOLD gap (what this queue item tracks)
Genuinely FOLD the conditional-perform-helper case (currently cleanly declined). NOT nesting/two-effects —
SINGLE effect reproduces. Minimal repro (declines "not yet reducible"):
```
effect B = | b : Int64 -> Int64 | done : Int64 -> Int64
def turn(x, acc) = if x == 1 then acc + B.b(x) else acc   // perform in an if BRANCH
def run(fuel: Int64, acc: Int64) =
  if fuel == 0 then B.done(acc) else run(fuel - 1, turn(fuel, acc))
def main() = handle B(0) with | b(x,s) => resume(x,x) | done(x,s) => resume(x,x) in run(4, 0)
```
Also the cond variant `if B.b(x)==1 then acc+1 else acc`. And the 2-nested-effect version = `/tmp/inc3c/authz.ml`
(Cedar.authorize in an if + Tools.dispatch).

## Root cause (diagnosed)
Threading the self-call arg inlines the helper's `if`; the branch-local `if`/`match` threading of a perform
produces a state reference (`f#ctx$s0`) that it does NOT bind into the synthesized def's sig → the ref is
unresolved. The FIX (fold) needs the inlined conditional's branch state-threading to bind its state refs to
the specialized def's `$s{k}` params (or thread the arg's post-conditional out-state correctly). Deeper than
the deep-fresh-copy fixes; specialization+branch-threading interaction — build carefully.
Full working notes: v-effects memory `queued-effectful-helper-in-selfcall-arg-loses-driver-params`.

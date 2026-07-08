# Projecting the result of a tuple-threading function traps

*2026-07-08*

**What happened.** Adversarial probing of recursive tuple accumulators found another
trap-on-a-well-typed-program in the compound-shape-through-a-return family. `(def (go n t) (if (=
n 0) t (go (- n 1) (tuple (+ (tuple.0 t) n) (tuple.1 t))))) (def (main) (tuple.0 (go 3 (tuple 0
0))))` emits a VALID component that TRAPS at the caller's `tuple.0`, where the value should be 6.
The trap is not depth-dependent — it happens even at recursion depth 0 (`(tuple.0 (go 0 (tuple 5
0)))`, where `go` returns its tuple parameter immediately, traps where it should be 5).

**The trigger, isolated.** The break needs a function that (a) takes a tuple PARAMETER, (b)
PROJECTS that parameter in its body (`(tuple.0 t)`) — which `tuple.N`-on-a-parameter now DECLINES
as "unknown tuple shape" — and (c) returns a tuple, whose RESULT the caller then projects. Two
controls prove the program is well-typed and the value is representable:
- a SCALAR accumulator threaded through the same recursion computes correctly (`(go 3 0)` = 6);
- a function returning a FRESH tuple without projecting a parameter has its result projected fine
  (`(def (mk n) (tuple n (+ n 1)))`, `(tuple.0 (mk 5))` = 5).
So it is specifically a tuple-typed parameter that is projected-in-body AND returned, then
projected at the call site, that traps.

**Why it is a break.** The program is well-typed (the controls compute), and self-hosting-and-
bootstrap.md #An Unsupported Construct Is Declined, Not Miscompiled requires a shape the compiler
cannot yet handle to DECLINE, never to emit a component that traps on a valued program. This is a
decline-don't-miscompile violation of the emit-a-broken-component kind.

**Root cause (likely) — the parameter-projection decline poisons the return.** `tuple.N` on a
tuple parameter declines "unknown tuple shape" (the c18 fix, correct on its own). But when that
same parameter is projected inside `go` AND `go` returns a tuple, `go`'s inferred return shape is
left unknown/degraded, so the caller's `(tuple.0 (go …))` lowers a projection against a value whose
shape the compiler could not recover — emitting a `tuple.N` access that traps rather than declining
the whole program or recovering the shape. The fix is the same shape-threading the payload-return
gap needs: recover a tuple's shape across a function boundary (parameter in, tuple out) so the
call-result projection either computes or the program declines uniformly — never a trap.

**The lesson.** A "decline this projection, I can't recover the shape" decision at one site
(`tuple.N` on a parameter) must not leave a HALF-compiled function whose return shape is degraded
enough to make a DOWNSTREAM projection trap. A local decline has to be a decline of the whole
program (or a recovery), not a silent shape-erasure that defers the failure to a caller as a trap.
The tell: the parameter-projection site declines cleanly, but the call-result-projection site — one
step removed — traps; the decline did not propagate to the enclosing program, it just erased the
shape and let codegen proceed. Same family as the built-in-Option payload-return trap (c12): a
compound's shape lost across a function return surfaces as a trap at the caller's projection.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"projecting the result of a
function that threads a tuple parameter must not trap" — `(tuple.0 (go 3 (tuple 0 0)))` MUST be 6
(or decline), never trap. Native seed; the behavior gate catches it (expected output 6, observed a
trap). Companion of the built-in-Option payload-return case — here the tuple flows through an
ordinary tuple-typed parameter rather than a sum payload.

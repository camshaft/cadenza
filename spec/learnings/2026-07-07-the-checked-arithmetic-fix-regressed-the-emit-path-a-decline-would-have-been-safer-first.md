# The checked-arithmetic fix regressed the emit path — and it's a crash, not a miscompile, which is the right kind of regression to have

*2026-07-07*

**What happened.** ask-37 (runtime `+ - *` emit bare `i64.add/sub/mul` that WRAP on overflow instead of
trapping — a wrong-value miscompile) got a fix attempt: `compiler.cdz` grew +6.5 KB and `lower`'s `KAdd/KSub/KMul`
arms now route through a new `checked-binop` → `checked-add`/`checked-sub`/`checked-mul` path that emits inline
overflow guards over 3 scratch locals (compute the wrapping result into a scratch local, test signed overflow,
`unreachable` if overflowed, else the result — matching native's checked-helper bodies). The emit sequence
itself is correct. But probing the running artifact showed it **regressed**: a runtime `+`/`-`/`*` now makes the
`compiler.cdz` component **trap at runtime** — an infinite recursion (wasm function 64 self-calling to stack
overflow) — instead of emitting a component. Isolated cleanly: `id`/`<`/`&` (non-checked ops) still compile;
only the three checked ops error. The component builds and *validates* (31 KB) but crashes when its own
arithmetic code path runs, so the defect is the **scratch-local reservation** — `sb` (the base of the 3 scratch
slots) is almost certainly not reserved past params + let-locals by `locals-decl`/`count-lets`, so the
`local.set (sb+2)` etc. alias a live slot and corrupt control flow into self-recursion. The byte gate regressed
140 → 172 disagree (declines 369 → 337): the runtime-arithmetic cases that were `soft` or clean declines now
error.

**Why.** Two things, and the second is the one worth keeping.

*The finding is the regression, caught by re-probing — not by the gate.* The behavior gate stayed green (562),
because it runs the *native* seed, which is unaffected; only the byte gate (component-check on the
Cadenza-authored compiler) and a direct `compile-run` show it. A fix that lands in `compiler.cdz` is invisible to
every gate that isn't the self-hosting one, so the loop's re-probe of the actual artifact is the only thing that
catches a self-hosted regression. This is the standing discipline paying off again: the pending note / commit
would say "checked arithmetic added"; the probe says "and it stack-overflows on every runtime `+`."

*A crash-regression is the RIGHT kind of regression to have — reject-don't-miscompile held through the mistake.*
Before the fix, runtime `+ - *` on overflow was a silent WRONG VALUE (`* MAX 2` → -2). After the buggy fix, it
is a TRAP (stack overflow). The fix is broken, but it moved the failure from the worst category (wrong value
accepted) to a safe one (crash/decline) — the program never computes a wrong answer, it just fails to compile.
This is the same ordering ask-34 taught (a miscompile fixed by declining is the right first fix): even a *buggy*
attempt at the checked emit is safer than the correct-looking bare opcode, because the bug manifests as a crash,
not a lie. The lesson for sequencing the fix: **had the fix landed as "decline runtime `+ - *` until the checked
emit is proven" (ask-37's option 2) it would have regressed nothing — the cases were already `soft`/`decline`;
attempting the full checked emit (option 1) directly is what introduced the crash.** When the faithful fix has
moving parts (here, scratch-local allocation the fold-only Lir never had — the exact architectural step the
shifts-decline learning named), land the decline first, then the emit behind it; the decline is the safety net
the in-progress emit trips into rather than out of.

**The requirement it drove.** No new corpus case — the overflow-traps cases are already pinned, and the
in-range arithmetic cases (which now crash) are pinned too; the byte gate measures the regression directly. The
output is the ask-37 update: the fix is on the right track (the emit sequence is correct), the bug is the
scratch-local reservation (`sb` must be `params + let-count`, and `locals-decl` must declare 3 more i64 locals),
and it stays TOP priority because it broke the arithmetic core — but it is a crash, not a miscompile, so
reject-don't-miscompile is intact. Reported to the compiler agent via the `📡 FROM THE CONFORMANCE LOOP` channel
with the isolation (checked-ops-only) and the root-cause hypothesis (scratch-local slots). General lesson:
**when landing a faithful fix that needs new machinery, land the decline first and the emit second — a
half-built emit that crashes is tolerable only because the decline underneath would have caught it; without that
net, the half-built emit is a regression with nothing below it.**

---

**Follow-up (2026-07-07, next cycle) — the crash was fixed by REVERTING to the bare opcode, which is a step
BACKWARD on reject-don't-miscompile.** The stack-overflow regression was resolved not by fixing the scratch-local
reservation but by reverting `lower`'s `KAdd/KSub/KMul` arms to the bare `binop … (IAdd/ISub/IMul)` — verified:
the emitted `(+ a b)` is `local.get 0; local.get 1; i64.add`, no guard; byte gate back to 140 disagree; in-range
arithmetic works again. But this restored the ORIGINAL ask-37 miscompile: `(+ MAX 1)` → MIN, `(* MAX 2)` → -2
(wrap, not trap). So the revert traded a **safe crash (a trap)** back for an **unsafe silent wrong value** — the
crash was the *better* state under reject-don't-miscompile, and reverting past it went the wrong way. (Also left
stale artifacts: the `lower` doc comment still claims "OVERFLOW-TRAP via inline checked guards," and
`checked-binop`/`checked-add/sub/mul` are now dead defs — the emit sequence is correct and present, just
unwired.) The lesson this sharpens: **when a fix breaks, revert toward the SAFER failure, not the original.**
The ordering of outcomes is wrong-value (worst) < crash < decline < correct. The buggy checked-emit was a crash;
the pre-fix state was a wrong value. Reverting a crash back to a wrong value moves *down* that ordering to unblock
the build — the right move is to revert (or gate) to a DECLINE instead (ask-37 option 2: `KError` on runtime
`+ - *`), which is strictly safer than both the crash and the wrong value, and keeps the build green. A revert is
not automatically safe just because it restores a known-"working" state; if that state was a miscompile, the
revert reintroduces the miscompile. Direction matters: unblock toward decline, never back toward wrong value.

## 11. 🟢 The front end's unknown-head path needs a real diagnostic, not a placeholder trap — RESOLVED (honest trap) 2026-07-07

**Finding.** With the front end now closed end-to-end (item 1), the compiler resolves a form's head from
its name string (`head-prim`). An **unrecognized head** resolves to `PUnknown` — a genuine front-end
error (the reader produced a form the compiler does not know) — but the spike currently "declines" it by
constructing an out-of-range `Bytes` value to force a **runtime trap** (`unknown-head-trap`), a
placeholder because the compiler-in-Cadenza has no diagnostics channel yet.

**Why it touches the spec.** This is a *compile-time rejection* masquerading as a *runtime trap*. An
unknown head is exactly the reader/front-end error class that should carry a `CDZ` diagnostic code and be
the program's recorded `(error CDZ…)` outcome — not a component that builds and then traps when run. It
also connects to the effects/diagnostics work: `compiler-pipeline.md` §"Phases Recover From Errors"
already envisions a diagnostics effect (record-and-continue), which is the natural home for this once the
compiler-in-Cadenza can perform effects. Interim behavior is honest (it halts rather than miscompiles),
but the end state is a front-end diagnostic.

**Status.** 🟢 **RESOLVED 2026-07-07 (honest trap) — and the placeholder was actively harmful.** The
`unknown-head-trap` (an out-of-range `Bytes.of (list 256)`) was replaced with a proper `Core.KError`
variant that lowers to `unreachable` — a defined trap, no Bytes hack. This was not just cleanup: the
out-of-range-Bytes placeholder was a `Never`-typed value that, on the runtime-heap path, made the whole
runtime-called `resolve` emit an invalid component — it was the true cause of the "cannot box" decline I
mis-diagnosed as a seed scale limit (see item 16, withdrawn). So the honest form both removed a real bug
and is the correct design. **Remaining (deferred, not blocking):** a proper `CDZ` diagnostic code (rather
than a bare `unreachable` trap) once the compiler-in-Cadenza grows a diagnostics channel (the `Diag`
effect) — but the miscompiling placeholder is gone and the interim behavior is now an honest defined
trap. Learnings:
`spec/learnings/2026-07-07-the-workaround-was-the-bug-correcting-the-scale-limit-diagnosis.md`,
`spec/learnings/2026-07-07-the-nested-payload-binder-fix-closes-the-front-end.md`.

---

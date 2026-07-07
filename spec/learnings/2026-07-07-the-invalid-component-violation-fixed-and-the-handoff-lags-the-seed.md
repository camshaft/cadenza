# The invalid-component violation is fixed — completing the withheld-case cycle — and the handoff doc lags the seed

*2026-07-07*

**What happened.** The `let`-free `tuple.N`-on-a-named-def-result decline-don't-miscompile violation
([[2026-07-07-runtime-tuple-projection-needs-a-let-and-the-direct-path-miscompiles.md]], SPEC-BACKLOG
item 15) is **fixed in the seed**. `(def (main) (tuple.0 (dec 4)))` — which previously emitted an
INVALID component (failed wasm validation) — now compiles and runs to 40. The fix is thorough, not
partial: verified across the whole-program-result case (`tuple.0 → 40`, `tuple.1 → 5`), the
scalar-consumed case (`(+ 100 (tuple.0 (dec 4))) → 140`), and even the compound-element case
(`(ev (tuple.0 (mk 7))) → 7`, a sum element projected directly from a named-def tuple and matched). So
`tuple.N` now recovers the operand's shape at the projection site regardless of whether the runtime
tuple was `let`-bound, lambda-produced, or returned from a named def.

This completes a clean **withheld-case cycle** that shows the corpus discipline working as intended.
Last cycle the violation was found; pinning it as a corpus case scored **FAIL** (an invalid component
is a gate *disagreement*, not a clean `todo`), so the case was withheld — recorded only in the learning
and backlog — to keep the gate green. This cycle the seed fixed it, and the case
`(def (main) (tuple.0 (dec 4))) → 40` was added and **passes**. The progression
**invalid → withheld (learning + backlog) → fixed → pinned green** is the right lifecycle for a
decline-don't-miscompile violation: it is never allowed to sit in the corpus as a FAIL, but it is never
lost either — the backlog carries it until the fix lands, then it becomes a permanent green gate
obligation.

**Why.** The methodological point worth recording is a second-order one: **the spike's handoff docs lag
the seed, so a "Remaining/still-broken" note in SEED-GAPS is not authoritative — the running seed is.**
SEED-GAPS still carries a ⚠ *"Remaining (narrow, deferred)"* note under Tier 2e claiming the no-`let`
render case "still produces a VALID component that TRAPS at the renderer" — but direct measurement
shows it runs to the correct value, and moreover it was never "valid-but-traps": last cycle it was an
*invalid* component (a strictly worse severity the handoff also under-stated). This is the second stale
claim caught in two cycles (the first: `compiler.cdz`'s header still calls the now-live `name-eq` dead
code "until Tier 2d is fixed"). The pattern is expected — a fast-moving spike writes docs as it goes
and does not revisit them — and it is exactly why the loop **probes the running seed rather than
trusting the handoff**: every finding this session was confirmed by compiling a program, not by reading
a Tier note. A handoff doc is a lead, not an oracle; the corpus (which executes) is the oracle. Where a
stale doc claim and a live probe disagree, the probe wins and the corpus records it.

**The requirement it drove.** The corpus case withheld last cycle now lands green: *"a scalar element
is projected directly from a function's runtime tuple result"* in `05-compound-types.sexp`
(`(tuple.0 (dec 4)) → 40`), the `let`-free companion of the existing let-bound projection cases,
deliberately over a **named-def** result (not an inline tuple or a lambda — the shape that was
miscompiling). It **PASSES**, turning the former invalid-component violation into a permanent gate
obligation and closing item 15. No new backlog item — this consolidates the state: with items 14 and 15
fixed this session, the runtime-tuple projection and recursive-Bool paths the reader depends on are
both green, and self-hosting's remaining gates narrow to the reader's top-level `read : Bytes → Node`
wiring plus items 12 (symbol-table `from-bytes`) and 13 (list patterns). The durable rule — probe the
seed, don't trust a fast-moving handoff's status notes — is recorded here for future loop iterations.

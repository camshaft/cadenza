# Diagnosis is complete and cascade-aware: everything wrong, once, root causes marked

*2026-07-04*

**What happened.** "Tell the agent *everything* that is wrong" is made a requirement: the compiler MUST
recover from each error and report the **maximal set of independent problems in a single pass**, rather
than halt at the first — and it MUST distinguish a **root cause** from a **derived** (cascade) diagnostic
so the agent fixes causes, not symptoms. Today `diagnostics.md` requires only a *deterministic order* of
diagnostics, not *completeness* and not cascade suppression.

**Why.** For an unattended agent, first-error-only is the difference between an **O(n) recompile-per-error
loop** and a single round-trip. But naive "report everything" is worse than useless — one root cause
(a mistyped binding) can spawn twenty downstream errors, and an agent that can't tell which is the cause
chases symptoms. Completeness only helps if it is *cascade-aware*. This forces real compiler machinery,
which the spec must therefore require the compiler to have:
- **Error recovery in every phase.** The reader emits error nodes and continues; the type-checker
  assigns an **error type** (⊥) that unifies with anything so one unification failure does not abort the
  whole inference pass ([[2026-07-04-inference-is-hindley-milner]]). Recovery is what makes a maximal
  independent set *reachable* in one pass.
- **Primary vs. derived.** A diagnostic that follows *only* from an already-reported one is marked
  **derived** (or suppressed), so the agent's fix loop targets the small set of **primary** diagnostics.
  This is a new field the schema must carry.
- **Determinism of the SET, not just the order.** The *set* of recovered diagnostics — not merely their
  emission order — MUST be a deterministic function of source (Constitution II), so the agent sees the
  same problem set on every build and its fix loop is reproducible.

**The rejection / decline / trap taxonomy — critical, and subtle here.** Because the compiler grows
incrementally and MUST **decline, not miscompile**, a construct it does not yet support
([[2026-07-03-decline-do-not-miscompile]]), an unattended agent must be able to branch on *why* its
program did not compile:
- **Rejection** — "your program is wrong; here is the verified route to fix it"
  ([[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]]). The agent fixes it.
- **Decline** — "this compiler *generation* does not realize this construct yet"
  (`options/realized-capability-set/`). This is **not the agent's fault and not fixable by editing the
  program's logic** — the agent must *route around* the construct, not chase a fix. Conflating decline
  with rejection sends the agent trying to "repair" a correct program against a compiler limitation.
- **Trap** — a defined-kind *runtime* halt (`coded-span-record.md` trap table), not a compile-time
  diagnostic at all.
The diagnostic MUST carry which kind it is, so the agent's control flow can differ per kind. This
taxonomy is latent across the spec (the `(compiler (error …))` vs `(trap …)` corpus split, the
decline-don't-miscompile rule) but was never stated as a *machine-branchable field*.

**Consequences.**
- **The fix loop converges in bounded round-trips.** Complete + cascade-aware + verified-fix
  ([[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]]) means one compile yields
  every primary problem, each with a route — the agent applies the routes and reconverges, rather than
  peeling one error per recompile.
- **Recovery must not fabricate.** A recovered (error-type) node must never cause a *miscompile* — if
  recovery cannot preserve well-formedness, the outcome is a decline/rejection, never emitted bytes
  ([[2026-07-03-decline-do-not-miscompile]]). Recovery is for *diagnosis*, not for limping to output.

**The requirements it drives.** `spec/capabilities/diagnostics.md` gains §"Diagnosis Is Complete"
(the compiler recovers and reports the maximal set of independent problems in one pass; the *set* is a
deterministic function of source) and §"A Diagnostic Distinguishes Primary From Derived" (cascade
diagnostics are marked so an agent fixes root causes) and §"A Diagnostic Names Its Kind" (rejection vs.
decline vs. trap is a machine-branchable field). `constitution.md` §XI (already being amended —
[[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]], Amendment 0.5.0) gains a
completeness clause so "report everything wrong" is an invariant, not only a capability detail.
`options/diagnostics-schema/coded-span-record.md` gains `kind` (rejection|decline|trap-note) and
`derived_from` (the primary diagnostic a derived one follows from) fields. Composes with
[[2026-07-04-type-errors-report-the-minimal-conflict]].

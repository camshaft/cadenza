## 2. 🔴 M-ordering tension: effects are the #1 self-host blocker but scheduled M6

**Finding.** The spike counted `DECLINE` markers across the flagship compiler: **effects = 10**
(ahead of numeric = 5, sum-decl = 3) — the compiler's own ambient state (`Fresh`, `Diag`, `Unify`)
is expressed as intra-program effects. The roadmap schedules effects at **M6**, after numeric (M4)
and traits (M5). So the single largest blocker to authoring the compiler in the intended style sits
two milestones out.

**Why it touches the spec/roadmap.** This is a sequencing decision only the operator can make. Two
coherent options, cost now visible:
- **(a) Pull effects earlier** (before/at M4) — unblocks authoring the compiler in the effectful
  style the spec's `compiler-pipeline.md` §"Phases Recover From Errors" already implies (record-and-
  continue is elegant *as* an effect).
- **(b) Keep the ladder** and author the compiler's state as **threaded immutable context** — the
  option already refined by [[dynamic-extent-is-an-effect-lexical-extent-is-a-parameter]]: lexical
  data threads as a parameter, only genuinely dynamic-extent state (diagnostics, fresh supply, unify
  store) needs effects. Under this refinement much of the "10" collapses to parameter-threading and
  effects at M6 may be fine.

**Status.** 🔴 **Operator call.** Note the spike has since *partially* de-risked this: Stages 0–3 of
effect lowering **landed in the seed** (tail-resumptive + state-threading + cross-fn inlining +
recursive-effectful monomorphization), so effects are further along than the roadmap's M6 implies.
The tension may already be softening in practice; the operator should decide whether to formally
re-order or let the flywheel resolve it. Recorded in
`spec/learnings/2026-07-05-authoring-the-compiler-in-cadenza-surfaces-the-language-gaps.md`.

---

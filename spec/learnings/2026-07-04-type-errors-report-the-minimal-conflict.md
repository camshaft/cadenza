# Type errors report the minimal conflict and both disagreeing sites, not one blamed location

*2026-07-04*

**What happened.** The type-error *reporting* discipline is pinned so it serves an unattended agent: a
type rejection MUST report the **minimal unsatisfiable constraint set** and **both program locations
that disagree** — *"this use requires `Int`; that use requires `Bool`"* — not a single blamed site with
a guessed cause. This is where a naive implementation of the language's own HM commitment
([[2026-07-04-inference-is-hindley-milner]]) fails agents hardest.

**Why.** Unification reports a failure *wherever unification happens to fail*, which is frequently **not**
where the mistake is — the classic "HM blames the wrong function / the error surfaces three calls away"
problem. A human squints and infers the real cause; an agent that cannot intuit it hits a dead end, which
breaks the zero-feedback loop the language exists to enable
([[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]]). The fix is well-studied —
type-error **slicing** (Haack & Wells) and the helpful-error line (Helium / Heeren for Haskell) — and its
essence is: report the *whole minimal contradiction*, both ends, not one endpoint.
- **Both sites are the fix.** Showing "this position imposes `Int`, that position imposes `Bool`" *is*
  "what to do to fix it": the agent sees the two constraints that cannot both hold and can change either.
  A single blamed location hides half the information the agent needs.
- **The bidirectional boundary sharpens it.** With inference reconciled to first-class types via a
  bidirectional-checking boundary ([[2026-07-04-inference-meets-first-class-types-at-a-bidirectional-boundary]]):
  in **check** mode the expected type is known from context, so the diagnostic is precise —
  *"expected `T` here because …"*; in **infer** mode there is no privileged expectation, so the
  diagnostic reports the **conflicting use sites** rather than inventing a blame target. Which mode a
  position is in tells the compiler *how* to phrase the conflict — a concrete reporting payoff of that
  boundary, not just a checking-decidability one.
- **Minimality keeps it actionable.** The reported constraint set must be *minimal* (dropping any
  constraint makes it satisfiable), so the agent is not handed the whole typing derivation — only the
  sentences that actually conflict. This is the type-error analogue of the primary-vs-derived
  distinction ([[2026-07-04-diagnosis-is-complete-and-cascade-aware]]): report the root contradiction,
  not every downstream inconsistency it induces.

**Consequences.**
- **The `related` spans carry the second site.** The diagnostic record already has a `related` list of
  `{span, message}` (`coded-span-record.md`); a type conflict populates it with the *other* disagreeing
  location(s), and the primary `span` is the one the fix most naturally attaches to. So this needs
  reporting discipline, not a new record shape.
- **It feeds a verified fix.** Once both sites and the minimal conflict are known, the compiler can
  often propose a determinable edit (insert the explicit conversion, correct the annotation — `CDZ0203`)
  and verify it by recompile
  ([[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]]).
- **Determinism.** *Which* minimal conflict is reported when several exist MUST be a deterministic
  function of source (Constitution II), so the agent sees a stable conflict to act on across builds.

**The requirements it drives.** `spec/capabilities/type-system.md` §Inference (or a new
§"A Type Error Reports Its Minimal Conflict") gains a requirement that a unification failure be reported
as a minimal unsatisfiable constraint set naming the disagreeing sites, rather than a single blamed
location — and that the choice among several minimal conflicts is deterministic. `spec/capabilities/diagnostics.md`
is annotated that a type-conflict diagnostic populates `related` with the co-conflicting span(s). This
is the reporting discipline behind `CDZ0201`/`CDZ0203`; it changes what those codes *carry*, not the
codes. Composes with [[2026-07-04-inference-meets-first-class-types-at-a-bidirectional-boundary]],
[[2026-07-04-diagnosis-is-complete-and-cascade-aware]], and
[[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]].

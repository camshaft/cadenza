# Effects are algebraic; a capability is a boundary effect; mutation is a State effect

*2026-07-04*

**What happened.** The open question `capabilities-and-effects.md` flagged — effect *checking* vs.
effect *handling* — is resolved in favor of **algebraic effects with handlers**, and the mechanism is
**unified with the capability model** rather than sitting beside it:

- **A capability is an effect the *host* handles at the component boundary.** An **intra-program
  effect** is one a **handler** within the program discharges. One mechanism, two handler locations.
- **The capability manifest is the top-level effect row that escapes to the host.** "Reaching an
  undeclared capability" (Constitution IV) becomes "an effect reached the boundary that the manifest
  does not list" — a compile-time rejection. The manifest stops being a second, separate system and
  becomes a projection of the effect typing.
- **Mutation re-enters as a `State` effect over the pure core.** The surface stays pure and immutable
  ([[2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete]]); stateful *programming*
  is obtained through a `State` effect whose handler is implemented as pure state-passing. "Immutable"
  and "I need mutable state here" stop being in conflict.
- **Continuations are one-shot (affine) by default.** A handler may resume its captured continuation
  **at most once**.

**Why.** The goals already committed make algebraic effects the natural fit, and each of the language's
hard constraints is *satisfied* by one-shot handlers rather than threatened by them:
- **It collapses two mechanisms into one.** Cadenza already had a mandatory capability layer
  (`capabilities-and-effects.md` §Capability Declaration) and an optional effect-*checking* layer. If a
  capability simply *is* an effect handled at the boundary, the manifest, capability-safety, and effect
  checking are one typing discipline. This matches the language's standing preference for one mechanism
  over parallel special cases ([[2026-07-03-uniform-single-arity-constructors]]).
- **It gives dependency injection, reader context, generators, and exceptions for free.** These are all
  handled effects, so the language does not need a separate construct for each.
- **Mutation-as-effect keeps RC sound.** Because the `State` handler is pure state-passing, the value
  heap stays acyclic and immutable, so reference counting stays complete
  ([[2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete]]). A mutation *primitive*
  would break that; a mutation *effect* does not.
- **One-shot resumption is what keeps the core guarantees.** Multi-shot resumption (resuming a captured
  continuation more than once) is what would break **(a)** fuel accounting — Constitution V requires
  bounded, accountable consumption, and a multiply-resumed continuation re-runs bounded work an
  unbounded number of times; **(b)** reference counting — a resumed continuation re-shares its captured
  values, defeating the unique-reference reuse analysis; and **(c)** local reasoning. One-shot
  continuations are **affine** — used at most once — which is the one surgical place linearity earns its
  keep ([[2026-07-04-linearity-is-surgical-not-core]]). OCaml 5's effects are one-shot for exactly these
  reasons; multi-shot is deferred as an explicit open point, not stumbled into.
- **Effect rows reuse the row machinery.** Typing an effectful function as `a -> b / {E…}` types the
  effects as a **row** of labels on the arrow — the *same* row polymorphism the record surface uses
  ([[2026-07-04-records-are-rows-open-by-default]]) — so effect inference is the record-row inference,
  and principal types are preserved (Rémy-style rows keep HM decidable —
  [[2026-07-04-inference-is-hindley-milner]]).

**Prior art.** **Unison** is the closest existing system to Cadenza's whole thesis: its *abilities* are
capabilities-as-effects, *and Unison is content-addressed* like Cadenza — it should be studied before
this is finalized. **Koka** combines algebraic effects with the Perceus memory model, addressing the
memory and effect stories in one system. **OCaml 5** (one-shot effect handlers) and **Eff** are the
other references.

**Determinism boundary to hold.** Effect *handling* has runtime meaning, so — unlike effect *checking*,
which `capabilities-and-effects.md` §"The Layer Preserves Meaning" keeps inert — installing a handler
is a semantic construct, not a no-op annotation. The behavior of an effect operation is the behavior its
handler gives it, and that must be a deterministic function of the program and its capability responses
(Constitution III). Handler resolution (which enclosing handler discharges an operation) must be
lexical/deterministic, never dependent on discovery or iteration order (Constitution II). A handled
effect that never reaches the boundary is *not* a capability and does not enter the manifest; only the
effects that escape to the host do.

**The requirements it drives.** `spec/capabilities/capabilities-and-effects.md` grows from "checking
only, handling deferred" to include the **operational** flavor: an effect-operation/handler construct,
one-shot (affine) continuations as the default with multi-shot as a recorded open point, the
identification of a capability as a boundary effect (so the manifest is the escaping effect row), and
the determinism constraints on handler resolution. The resolution is recorded as a new decision
**`options/effects-model/`** — `algebraic-one-shot` as the default choice — per the note that document
already carries ("its resolution is a declared default recorded under `options/` at that time"). Effect
tracking remains an optional capability the seed does not realize
(`options/realized-capability-set/`), so this reshapes the target language without obligating the seed
generation.

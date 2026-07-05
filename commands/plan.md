# Command — plan

**Purpose.** Derive the **climb**: the ordered, concrete sequence of steps that carries this build
from the generation it is standing on to the full realized language. The specification states the
*end state* (`spec/overview.md`, the capability specs) and `options/` states the *decisions*; once a
build has resolved those decisions (recorded in `implementation/DECISIONS.md`), exactly one climb
connects the two. This command computes it. The result is a **disposable, build-specific projection**
— a function of `(the full-language specification) × (the option choices this build made)` — so it is
written to the gitignored `implementation/PLAN.md`, never committed, exactly like the generated
compiler and `implementation/DECISIONS.md` it derives from. What is durable is *this process*, not any
one plan it produces.

**Agent-agnostic.** Neutral prompt body; assumes a shell and the `duvet` CLI. It is the source of
truth for the command; per-agent command directories are generated from it.

**Why the plan is not committed.** A plan pins concrete choices (which numeric model, which effects
model, whether verification is included) and an order that only makes sense for *this* build's
resolved decisions. Committing it would freeze one build's projection into the durable spec tree and
put a forward-looking artifact where `analyze` — which checks the *committed* spec — would have to
either ignore it or wrongly police it. So the plan lives beside the thing it plans (`implementation/`),
is re-derived whenever an input changes, and self-checks at derivation time rather than through
`analyze`.

## Usage

`plan [--check]`

- Default: derive `implementation/PLAN.md` fresh from the current inputs, overwriting any prior plan.
- `--check`: do not rewrite; re-run the consistency checks (below) against the *existing*
  `implementation/PLAN.md` and the current inputs, and report drift — a plan whose `Status` no longer
  matches the gate, an input that changed since the plan was derived, or a rung that no longer
  satisfies a check. Use this after a tuned option, a folded learning, or a promotion, to decide
  whether the plan needs re-deriving.

## Inputs (the plan is a pure function of these)

- **The end state.** `spec/overview.md` (the intent arbiter, §1–15 describe the full language),
  `spec/glossary.md`, and every capability spec under `spec/capabilities/**` — the language *when it
  is built*.
- **The resolved decisions.** `implementation/DECISIONS.md` — the build mode, every `options/` choice
  this build adopted (accepted default, tuned, or operator-authored), and every optional capability
  **included** or **excluded**. **This input is required:** the plan cannot be derived until the
  choices are made (see Preconditions).
- **The decision menu.** `options/**` — each `options/<decision>/README.md` and its `DEFAULT:` line,
  used to bind each remaining capability to its concrete chosen realization and to detect a decision a
  capability needs that does not yet exist.
- **The current floor.** `options/realized-capability-set/` — the capabilities the *current* generation
  realizes, which the climb subtracts from the end state.
- **The requirement and behavior scopes.** `.duvet/bootstrap.toml` (the ignition requirement subset)
  and `.duvet/config.toml` (the full set), and `spec/semantics/*.sexp` with its `(needs …)`
  annotations — the two gates each rung must clear.
- **The starting rungs.** `options/bootstrap-strategy/` names the short staged path (seed →
  Cadenza-authored compiler → self-hosting) the climb begins from and extends past.
- **The pending direction.** `spec/learnings/**` — learnings that name a requirement they drive but
  whose RFC-2119 pass is not yet written; each becomes a fold-step on the rung that consumes it.
- **The current position.** The promotion record (which generation is current) and the last gate
  result, used to derive `Status` — never hand-asserted.

## Preconditions

- `implementation/DECISIONS.md` records the `options/` posture and the included/excluded optional
  capabilities. If it does not, STOP with "choices not yet made — run `/build` Phase 1 first"; do not
  invent a posture. The plan is the phase *after* the choices, not a substitute for them.
- `analyze` ends `ANALYZE: PASS` on the current spec. A plan derived from an ill-formed spec plans
  against a moving target; if `analyze` fails, report that and stop.

## Procedure — deriving the climb

Produce `implementation/PLAN.md`. Work in these steps; the checks in the next section must pass before
the plan is written.

1. **Assemble the end state.** Enumerate every capability the build's language includes: every spec
   under `spec/capabilities/**`, minus any optional capability `DECISIONS.md` records as **excluded**
   (its requirements are non-load-bearing and it is not on the climb). Each capability is enumerated at
   *full* realization. Cross-check the union against `spec/overview.md` §1–15 so the end state is the
   whole language and nothing is planned out of existence.
2. **Subtract the floor.** Remove what the *current* generation already realizes
   (`options/realized-capability-set/` — for a fresh build, the seed-ignition-set). What remains is the
   **climb**: the capability deltas still to realize.
3. **Bind the concrete choices, and flag the missing ones.** For each capability on the climb, pin the
   *chosen* realization from `DECISIONS.md`/`options/`. Where a capability needs a decision that has no
   `options/<decision>/` directory yet (e.g. an effects model, a memory-ownership model, an ad-hoc-
   polymorphism strategy, a verification strategy — decisions a direction-setting learning names but no
   choice realizes), record that missing decision as an explicit **author-decision step** on the rung
   that first needs it, executed via `specify`/`clarify`. A missing decision is a named prerequisite,
   never a silent gap.
4. **Attach the pending requirement folds.** For each learning under `spec/learnings/**` that names a
   requirement it drives but whose RFC-2119 sentence is not yet written, record a **fold-learning step**
   (executed via `learn`/`specify`) on the rung whose capability consumes it, so the direction already
   captured becomes a sequenced obligation rather than a homeless backlog.
5. **Order the climb into rungs.** Group the steps into **rungs**, each a generation — a promotable
   unit that realizes a coherent capability delta. Order the rungs so dependencies flow forward only:
   no rung may realize a capability, or need a decision or a folded requirement, that a *later* rung
   introduces (e.g. effect rows need row-polymorphic records first; monomorphized generics need the
   inference core first). Seed the first rungs from the `options/bootstrap-strategy/` staged path
   (seed / ignition → Cadenza-authored compiler → self-hosting), then extend past self-hosting through
   the remaining deltas. The terminal rung's cumulative realized set MUST equal the end state from
   step 1.
6. **Derive the current position.** From the promotion record and the last gate result, mark which
   rung is `current`, which are `realized`, and which are `not-started`; report the *next* rung and its
   entry criteria (which author-decision and fold-learning steps must complete before it can be
   realized). Never assert a `Status` the gate does not support.

Each rung is one section of `implementation/PLAN.md` with this shape:

```markdown
## Rung N — <name> (generation N)

**Goal.** One sentence: what this generation makes possible that the prior rung could not.

**Realizes (delta over the prior rung).** The capabilities newly realized, each bound to its chosen
  option — e.g. `type-system → full Hindley-Milner inference + generics/monomorphization
  (options/… default)`, `capabilities-and-effects → optional effect-tracking layer (options/effects-model)`.

**Requires (entry criteria — the steps that must complete first).**
  - author-decision: `options/<decision>/` (does not exist yet) — via `specify`/`clarify`
  - fold-learning: [[learning-slug]] → capability §section — via `learn`/`specify`
  - depends-on: Rung K (this rung's capability builds on Rung K's)

**Bar (exit criteria).** The new `spec/semantics/*.sexp` cases this rung must pass (by `(needs …)`),
  the requirements it must newly cover in the gate, and two-compiler agreement holding.

**Executed by.** The command(s) that realize this rung — `ignite` for the seed, otherwise
  `regen` (whose own plan-synthesis phase decomposes this rung internally), then `gate` and `promote`.

**Status.** not-started | current | realized  ← derived from the promotion record + gate, not asserted.

**Rationale.** Why this rung is ordered here — the dependency or risk that fixes its position.
```

## Consistency checks (run before writing; re-run under `--check`)

These are the plan's own guardrails — they live here, not in `analyze`, because the plan is gitignored
and build-specific and `analyze` checks only the committed spec.

- **Coverage.** The union of every rung's `Realizes` equals the end state from step 1: every included,
  non-excluded capability is realized by exactly one rung, and no rung realizes a capability the current
  generation already has or the language does not include.
- **Dependency soundness.** No rung depends on a capability, a decision, or a folded requirement that a
  later rung introduces; the rung order is a valid topological order of those dependencies.
- **Pending-fold coverage.** Every learning that names an unwritten requirement maps to a fold-learning
  step on exactly one rung, so no captured direction is dropped from the climb.
- **Missing-decision surfacing.** Every capability on the climb whose `options/<decision>/` does not yet
  exist has an author-decision step on the rung that first needs it; no rung silently assumes a decision
  that has not been made.
- **Status honesty.** Each rung's `Status` matches the promotion record and last gate result: a rung
  marked `realized` passed both gates and was promoted; the single `current` rung is the promoted
  generation; nothing is marked ahead of what the gate supports.

## Output

- Write `implementation/PLAN.md`: a short preamble (the build's mode, the resolved posture it was
  derived from, the current position) followed by the ordered rungs.
- Print the derived climb as a table — rung, goal, entry criteria not yet met, status — and end with a
  single verdict line: `PLAN: DERIVED` (fresh derivation, all checks passed) or, under `--check`,
  `PLAN: CURRENT` (no drift) / `PLAN: STALE` followed by the drift found (which input changed, which
  check now fails, which `Status` no longer matches the gate).
- If any consistency check fails during a fresh derivation, do not write a plan that violates it:
  report `PLAN: BLOCKED` with the failing check and the offending rung, because an inconsistent climb
  is worse than none.

## Guardrails

- `implementation/PLAN.md` is a **disposable projection**, never the source of truth and never
  committed; the spec and the corpus remain the sole authority on what a construct means, and
  `DECISIONS.md` remains the authority on which choices were made. If the plan and the spec disagree,
  the spec wins and the plan is re-derived.
- **Re-derive when an input changes.** A tuned `options/` choice, a folded learning, a newly authored
  `options/` decision, or a promoted generation changes an input; re-run `plan` (or `plan --check`
  first) so the climb reflects reality rather than a stale snapshot.
- The plan **records order and intent; it discharges nothing.** It carries no RFC-2119 requirements, is
  not a `[[specification]]` in the gate, and its rungs are not a promotion bar — the two gates remain
  the only bar. A rung is *done* when `gate` and `promote` say so, not when the plan lists it.
- The plan does **not** author spec or code. It names the author-decision and fold-learning steps but
  leaves them to `specify`/`clarify`/`learn`, and it names the realize-generation steps but leaves them
  to `ignite`/`regen`. It plans the climb; the other commands take it.
```

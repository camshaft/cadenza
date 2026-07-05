# Command — regen

**Purpose.** Produce the next generation of the Cadenza compiler from the
specification tree. A generation is a compiler synthesized as Cadenza source,
derived by the *previous* generation, and passed through both gates — the step
that turns the seed→generation→generation chain toward the compiler being
authored in Cadenza (self-hosting). It is not a hand-edited component; it is
regenerated from the durable spec.

**Agent-agnostic.** Neutral prompt body. A pipeline of phases, each re-runnable.

## Usage

`regen [--lang <language>] [--from <phase>]`

- `--lang` selects the target language for this generation (default: the
  repository's declared default in `options/bootstrap-strategy/`). A self-hosting
  generation is a Cadenza compiler authored in Cadenza
  (`spec/capabilities/self-hosting-and-bootstrap.md` §"Each Generation Is Derived
  By The Previous").
- `--from` resumes the pipeline at a named phase (`plan`, `generate`,
  `self-check`, `gate`, `propose`) so a gate failure need not re-plan.

## Pipeline

### Phase 1 — plan-synthesis

- Read the whole specification tree: `constitution.md`, `spec/overview.md` (the
  intent arbiter), `spec/glossary.md`, `spec/contracts/**`,
  `spec/capabilities/**`, `spec/bootstrap.md`, and the executable-semantics corpus
  under `spec/semantics/`.
- Produce an ephemeral synthesis plan: the module decomposition, the target
  language, and a map from each MUST/SHALL requirement to the source site and test
  that will cite it. This plan is not committed spec.
- Treat every `spec/contracts/**` file as a READ-ONLY input to cite, never to
  rewrite.

### Phase 2 — generate

- Emit a fresh compiler codebase into `implementation/` (gitignored, disposable),
  plus, via `setup-gate`, the per-language `[[source]]` configuration.
- Every source site carries its capability manifest and cites the requirements it
  satisfies by quoting their sentences.

### Phase 3 — self-check

- Run the `analyze`-equivalent checks against the generated output: does every
  MUST/SHALL requirement have a planned citation, does the code build, does it
  lint? This is a cheap gate before the real ones.

### Phase 4 — gate

- Invoke both gates against the full config (`.duvet/config.toml`): the
  requirement gate (`duvet report`) and the behavior gate that derives and runs every
  `spec/semantics/*.sexp` case and confirms it reproduces its recorded output
  (`conformance-gate.md` §"The Behavior Gate"). On either gate's failure, stop; the
  pipeline may be resumed at `--from generate` after fixes without re-planning.
- Confirm the compiled output agrees with the recorded corpus semantics on every
  executable-semantics case before proposing it
  (`self-hosting-and-bootstrap.md` §"The Generated Path Is Exercised Before It Is
  Trusted").

### Phase 5 — propose

- Derive the generation with the previous generation of the compiler and **run
  the derived component**: a claim that a new generation exists MUST be backed by a
  component that was actually derived and is now running, not by the emission of
  the artifacts that would accompany a regeneration
  (`self-hosting-and-bootstrap.md` §"A Regeneration Is Derived, Gated, And Run").
- Re-demonstrate, on that real derived-and-run component, that its imports mirror
  its manifest and that re-deriving the same source reproduces a byte-identical
  component (`self-hosting-and-bootstrap.md` §"Every Generation Re-Demonstrates
  The End-To-End Path").
- Propose the candidate generation for promotion via `promote`, and record any
  spec gap the regeneration surfaced via `learn`. Record the toolchain identity
  alongside the derived component (`reproducible-derivation.md`;
  `options/toolchain/`).

## Invariants

- The frozen contracts under `spec/contracts/**` are read-only inputs; a
  regeneration that would change a frozen contract MUST stop and route that change
  through the constitution's Governance Floors ("The Component ABI Changes Only By
  Coordinated Act").
- Only the `[[source]]` half of the gate configuration is rewritten;
  `[[specification]]` is stable (`conformance-gate.md` §"The Requirement And Code
  Sides Are Separated").
- A generation that does not pass **both** gates MUST NOT be promoted
  (`conformance-gate.md` §"The Gate Is The Promotion Bar").
- A generation whose behaving is demonstrated only by a stand-in that never
  executed the derived component MUST NOT be treated as a conforming generation
  (`self-hosting-and-bootstrap.md` §"A Regeneration Is Derived, Gated, And Run").

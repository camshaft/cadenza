# Command — build

**Purpose.** The front door. This is the single orchestration command an agent runs to take this
repository from specification to a working Cadenza compiler. It reads the spec, asks the user a small
number of high-level questions, de-risks the load-bearing assumptions, then synthesizes the seed
toolchain and clears the ignition bar. Every low-level decision is answered by the specification, not
by the user.

**Agent-agnostic.** Neutral prompt body. Run interactively or headless; the mode below decides how it
treats an ambiguity.

## The two modes (read `spec/capabilities/build-modes.md` first)

This command runs in exactly one of two modes, fixed for the whole run and recorded in
`implementation/DECISIONS.md`:

- **Attended (`--author`)** — the driver is working on the spec. When the build reaches a
  *specification ambiguity*, HALT, surface it to the human or another agent, fold the resolution into
  the spec as a new requirement (via `clarify`, which also adds the declared default), then RESTART
  the build from Phase 0 on the corrected spec. Halting is the point: it is how the spec hardens.
- **Autonomous (default)** — the driver only wants a working compiler and cannot resolve internals.
  NEVER halt on a specification ambiguity. Resolve it by applying the point's *declared default* and
  record that you did. If an open point has no declared default, record it as a spec defect and
  proceed on a conforming choice. You may still ask the user-facing choices below, but only when
  interactive; non-interactive → take their declared defaults.

The mode is passed in by `start.sh` (`--author` selects attended; its absence selects autonomous). If
you cannot tell, assume autonomous.

## Operating principle

The specification is detailed and normative. Do NOT ask the user to make decisions the spec already
makes. Distinguish three kinds of decision, per `spec/capabilities/build-modes.md`:

- a **user-facing choice** (target host language for the seed, runtime engine, which `options/`
  choices to adopt) — may be asked in either mode when interactive, and otherwise takes its declared
  default;
- a **specification ambiguity** — handled per the mode above (halt-and-harden vs. declared-default);
- an **operator-gated point** (the core symbol namespace, a frozen-contract byte-level pin) — resolve
  deliberately in attended mode; in autonomous mode it MUST already be resolved in the committed spec,
  and if it is not, STOP with a "spec not ready for an autonomous build" report rather than inventing
  it or asking the user.

When you must ask, batch the questions up front, keep them high-level, and record each answer in
`implementation/DECISIONS.md` so a later run does not re-ask.

## Phase 0 — Orient

1. Read `README.md`, `AGENTS.md`, `constitution.md`, and `spec/overview.md` in full — the intent and
   the invariants.
2. Read `spec/bootstrap.md` — the line of sight to self-hosting and exactly what the seed toolchain
   must do to clear the ignition bar.
3. Read every `spec/contracts/**` file (the frozen byte/ABI forms the compiler must honor) and the
   ignition-subset `spec/capabilities/**` (those listed in `.duvet/bootstrap.toml`).
4. Run `analyze` (see `commands/analyze.md`) and confirm `ANALYZE: PASS` before building anything. If
   it fails, stop and report — the spec is not gate-ready.
5. **Autonomous mode only:** verify every operator-gated point your target depends on is already
   resolved in the committed spec (per `build-modes.md` §"Operator-Gated Points Precede An Autonomous
   Build"). In particular, confirm the `options/` directory exists and pins every frozen-contract
   byte-level choice the ignition subset depends on — the AST encoding (`options/ast-encoding/`), the
   hashing and encoding (`options/hashing-and-encoding/`), the type mapping (`options/type-mapping/`),
   and the execution model (`options/execution-model/`) — and that the core symbol set the corpus
   references is settled. If `options/` is absent (a "start from scratch" was requested) or a required
   choice is missing, STOP with a report that the spec is not yet ready for an autonomous build, and
   do NOT invent the resolution or ask the user for it.

## Phase 1 — The high-level questions (the ONLY user-facing decisions)

Ask the user these, and only these unless something is genuinely underdetermined by the spec:

1. **Seed host language.** The seed compiler is authored in a foreign language because no Cadenza
   toolchain yet exists to derive it (`spec/bootstrap.md`). Confirm the language;
   `options/bootstrap-strategy/` names the declared default.
2. **Scope of this run.** Offer: (a) run the de-risking spikes first (recommended), then the seed
   toolchain; or (b) go straight to the seed toolchain. Warn that (b) bets the toolchain on unverified
   assumptions.
3. **Runtime host choice.** The `host-interface-binding` contract fixes the interface, not the engine.
   Confirm the embeddable component-model runtime to target (`options/execution-model/` names the
   declared default).
4. **Which `options/` choices to adopt.** For each decision under `options/`, the default choice
   applies unless the user selects another listed choice or authors their own for that one decision
   (per `options/README.md`). Ask the user's posture: (a) **accept** every default (recommended; what
   an autonomous run does); (b) **tune** specific decisions; or (c) **start from scratch** on a
   decision by removing its directory, after which you investigate it from first principles and — in
   attended mode — bring the proposed choice back before committing. Warn that changing a choice that
   fixes bytes produced from unchanged source (the AST encoding, the type mapping, the hashing rule,
   the numeric byte forms) is an ABI-level change under the constitution's Governance Floors.
5. **Optional capabilities.** For each capability that declares itself optional (effect tracking,
   verification layers, property-based testing, units of measure), ask whether to include or exclude
   it. This is a user-facing choice, not a specification ambiguity: apply the capability's declared
   default when not told (each declares its default as **include**). Record every optional capability
   included and excluded in `implementation/DECISIONS.md`, and hold an excluded capability's
   requirements non-load-bearing for the gate (per `conformance-gate.md` §"An Excluded Optional
   Capability Is Not Load-Bearing").
6. **Anything the spec marks open.** Handle per mode: attended → HALT and `clarify`; autonomous → apply
   the declared default, or record the missing default as a spec defect and proceed.

Items 1–5 are user-facing choices: ask them only when interactive, and otherwise take their declared
defaults (for item 4, the default posture is **accept**; for item 5, each capability's own default).
Record all answers and every default applied in `implementation/DECISIONS.md`.

## Phase 1.5 — Derive the climb (`plan`)

Now that the choices are resolved and recorded in `implementation/DECISIONS.md`, run `plan` (see
`commands/plan.md`) to derive `implementation/PLAN.md`: the ordered sequence of generations that
carries this build from the seed to the full realized language, with each rung bound to the concrete
option choices just made and each step naming the command that executes it (`ignite` for the seed,
then `regen`/`gate`/`promote`, with `specify`/`clarify`/`learn` for the author-decision and
fold-learning steps a later rung needs). The plan is a disposable, gitignored projection of the
end-state spec × the resolved choices — it is derived here, *after* Phase 1 fixes the choices and
before synthesis, and re-derived (or `plan --check`ed) whenever an input changes. This build's front
door only needs to clear the ignition bar (the seed rung), but the plan makes the whole climb past it
explicit so execution of each later generation is a matter of taking the next rung, not re-planning
from scratch. Confirm it ends `PLAN: DERIVED` before proceeding.

## Phase 2 — De-risk (unless the user chose to skip)

These spikes validate the assumptions the whole design rests on. Keep them tiny and throwaway (build
them under `implementation/spikes/`).

1. **Cross-language gate spike.** Author a throwaway 2-requirement markdown spec, cite both
   requirements from a source file in the chosen language, run the gate, and confirm coverage. Then
   edit one requirement's sentence and confirm the gate flags the now-stale citation as a hard error.
   This proves the quoted-text identity model works for the chosen language.
2. **Reproducible derivation spike (the ignition bar in miniature).** Take a trivial Cadenza program
   as a binary AST (per `spec/contracts/ast-encoding.md` and `options/ast-encoding/`), derive it to a
   component by compiled codegen — the seed's derivation mode (`options/bootstrap-strategy/`): the
   compiler generates a core module and wraps it into a real component whose interface world declares
   exactly the program's manifest, per `spec/bootstrap.md` §"Compiled Output Agrees With The Recorded
   Semantics" — confirm the component's imports mirror its manifest, run it
   and observe its output, confirm the output agrees with the recorded corpus semantics (the oracle)
   over the same input, then re-derive and confirm byte-identical output. This proves the
   source→component→run path is real and reproducible. (An optional reference interpreter — embedding
   the interpreter over the AST — is an independent oracle only, never what the seed spikes.)

If either spike fails: in **attended** mode, stop and report — the failing assumption must be resolved
(a `learn` entry and a spec change) before the toolchain is worth building, then restart. In
**autonomous** mode, a spike failure is a genuine environment-capability failure, not a spec
ambiguity, so stop and report it too — an autonomous build never fabricates around a broken
assumption.

## Phase 3 — Synthesize the seed toolchain

Follow `commands/ignite.md` exactly. In summary:

1. Run `setup-gate` for the chosen host language so the gate's `[[source]]` half points at
   `implementation/` and uses the language's citation comment style.
2. Synthesize the seed compiler source into `implementation/`: a reader for the binary AST and its
   symbol prelude; the static-typing floor that rejects an ill-typed program at compile time
   (constitution §VII), realized incrementally; compiled derivation (the seed's mode,
   `options/bootstrap-strategy/`) that generates a component — a core module wrapped into a real
   component whose interface world declares exactly the program's manifest, so its imports mirror the
   manifest natively — whose observable behavior agrees with the recorded corpus semantics (the
   oracle); and the machine-readable diagnostics. Cite every frozen-contract and ignition-subset
   requirement you satisfy.
3. Gate against the ignition subset: `duvet report --config-path .duvet/bootstrap.toml`, iterating
   until every MUST/SHALL in the subset is covered and there are zero broken citations. Coverage must
   be *honest*, per `conformance-gate.md` §"A Citation Discharges Its Own Requirement": each citation
   annotates the code that performs its behavior and each cited test fails when that behavior breaks.
   Do NOT manufacture coverage by pointing many requirements at one shared exercise or by citing a
   placeholder — a gate green on vacuous citations is a failed build. Report interim coverage honestly
   as it climbs; never jump to 100% by generating a citation layer.
4. Run the behavior gate: execute every case in `spec/semantics/*.sexp` through the reference
   interpreter and confirm each reproduces its recorded output (per `conformance-gate.md` §"The
   Behavior Gate"). A generation that passes the requirement gate while failing the behavior gate is
   not promotable.

## Phase 4 — Clear the ignition bar

The goal is not to observe a plausible-looking build; it is to observe a real, executed, reproducible
derivation, per `spec/bootstrap.md` §"The Ignition Bar":

1. Derive a real Cadenza source program to a content-addressed component and run it to produce its
   output.
2. Confirm the derived component's imports mirror its declared capability manifest — capability
   binding exercised, not merely configured.
3. Re-derive the same source with the same toolchain and confirm a byte-identical component —
   reproducibility exercised, not asserted.
4. Confirm the compiled output agrees with the recorded corpus semantics (the oracle) over the same input.

A build demonstrated only by emitting the artifacts a derivation would produce, without a component
that was actually derived and run, is a model of an ignition, not an ignition, and MUST NOT be
reported as one. If a real dependency is genuinely out of reach in this environment, STOP and report
the seam explicitly; do not satisfy it with a stub the gate cannot tell from the real thing.

## Discipline throughout

- Generated code lives under `implementation/` (gitignored). It is a disposable projection; never
  treat it as the source of truth.
- Cite requirements by quoting their exact sentences. A `GATE: FAIL` is never worked around by
  weakening a spec; if the spec is wrong, use `learn`.
- If you get stuck on something the spec should have answered, that is a spec gap. In attended mode,
  halt and record it via `learn`/`clarify` (narrative + a requirement edit + its declared default),
  then restart. In autonomous mode, apply the declared default (or, absent one, a conforming choice),
  record the gap as a spec defect in `implementation/DECISIONS.md`, and keep going.
- Keep the user informed at phase boundaries; otherwise proceed on the spec's authority.

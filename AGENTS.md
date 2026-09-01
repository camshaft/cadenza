# Working in this repository

This file orients any agent (or person) working on the Cadenza specification. It is
agent-agnostic.

## What this repository is

The specifications are the durable artifact; the Cadenza compiler is a disposable,
regenerable projection of them. Read [README.md](README.md) for the idea and
[spec/overview.md](spec/overview.md) for the architecture and intent. The non-negotiable
invariants are in [constitution.md](constitution.md).

This branch contains **only** the specification tree. The compiler that these specs
describe is regenerated into a gitignored `implementation/` directory and is never the
source of truth.

## The rules that govern editing the specs

1. **Requirements are single, atomic RFC-2119 sentences under stable headings.** Every
   normative statement (MUST / MUST NOT / SHALL / SHOULD / MAY) is one self-contained
   sentence carrying exactly one obligation, under a heading that does not change casually.
   This is load-bearing: the conformance gate identifies a requirement by `(file, section,
   quoted sentence)`, so a sentence buried in a paragraph extracts ambiguously, and rewording
   a sentence invalidates its citations. **No compound requirements:** a sentence that joins
   two independent obligations (typically with "and", or two separate `MUST`/`MUST NOT`
   clauses, or obligations on two different actors) MUST be split into one sentence per
   obligation. A single "and" is fine only when it joins parts of *one* obligation (a list of
   inputs, a subordinate "so that…" rationale), not two obligations that could be satisfied or
   violated independently.

2. **Stay standalone.** No normative specification names a concrete engine, a hashing
   algorithm, a numeric width, a prior prototype, a library, or a source-file path. Describe
   the runnable form as "component", the execution environment as "runtime" / "sandbox", the
   bound on execution as "a deterministic resource measure". A concrete technology choice
   lives in [options/](options/) as a declared-default choice, never in a requirement. The only
   exception is [spec/learnings/](spec/learnings/), whose entries may name prior
   implementations as historical reference.

3. **Use the glossary.** Vocabulary comes from [spec/glossary.md](spec/glossary.md); add a
   new term there before using it normatively elsewhere.

4. **Descriptive vs. normative.** `overview.md`, `glossary.md`, `traceability.md`, and
   everything under `learnings/` are descriptive and carry no requirements — they are NOT
   listed as `[[specification]]` in the gate. The constitution, everything under
   `spec/contracts/`, everything under `spec/capabilities/`, and `spec/bootstrap.md` are
   normative. `spec/semantics/` is normative *by execution*, not by RFC-2119 extraction (see
   "The two gates" below).

5. **Frozen contracts change only under discipline.** A change to `spec/contracts/**` is
   additive with respect to already-published source and already-derived components, or it
   carries a version increment and a stated migration path — per the constitution's Governance
   Floors. The component ABI, the deterministic value form, and the source-tree encoding are
   the byte-level forms that outlive any single compiler; treat a change to them as a
   coordinated act.

6. **Traceability is bidirectional.** Every normative section maps to an `overview.md` section
   in [spec/traceability.md](spec/traceability.md), and every `overview.md` section is served
   by at least one normative section.

7. **Every open point is a decision with choices and a declared default.** When you resolve a
   specification point a conforming generation could decide more than one way, record it as a
   decision under [options/](options/): a `options/<decision>/` directory whose README states the
   requirements a choice must satisfy and names the default with a `DEFAULT: <choice>` line, plus one
   `<choice>.md` per candidate realization. This is what lets an autonomous build apply the default
   without halting, and lets an attended build surface the choices or accept an operator-authored
   choice for that one decision.

## The two gates

Cadenza's specs are checked by two independent gates.

**The requirement gate — [duvet](https://github.com/awslabs/duvet).** Every normative
sentence is extracted and mapped to the implementation and test that satisfy it.
Configuration is in [.duvet/config.toml](.duvet/config.toml) (full) and
[.duvet/bootstrap.toml](.duvet/bootstrap.toml) (the seed-toolchain ignition subset). Three
facts to respect:

- **Every `[[specification]]` needs `format = "markdown"`.** The default format is IETF and
  silently extracts zero requirements from our prose.
- **Source paths are project-root-relative**, not relative to `.duvet/`.
- **A citation is two markers: `//=` then `//#`.** `//= <spec>#<section>` names the
  requirement; `//# <exact sentence>` quotes it, and duvet validates the quoted text against
  the section (hard error if the cited words are gone — this enforces the quoted-sentence
  identity model).

**The behavior gate — the executable semantics corpus.** Every case in
[spec/semantics/](spec/semantics/) is an Input paired with an expected Output. A promoted
compiler MUST reproduce every recorded Output. This is the single source of truth for what a
construct *does*; the compiler and every tool agree with the corpus rather than encoding their
own semantics. The corpus is not listed in the duvet config — it has no RFC-2119 sentences to
extract; its gate is execution.

Quick checks:

```sh
# extract requirements from one spec (sanity-check a new file)
duvet extract -f markdown -o /tmp/ex ./spec/contracts/<name>.md

# run the full requirement gate
duvet report
```

## Tests belong in the corpus, NOT baked into the host language — OPERATOR DIRECTIVE

**Every behavioral test MUST be a host-language-independent case in the executable-semantics
corpus ([spec/semantics/](spec/semantics/)), not a test baked into the Rust seed toolchain.** This
is the direction the whole project is going, and it is not optional. When you want to assert what a
construct *does* — what it evaluates to, what it declines, what diagnostic it emits — write it as a
corpus case (an Input paired with its expected Output), because the corpus is the
implementation-independent, runnable language specification and every tool agrees with it. A Rust
`#[test]` that encodes a *language behavior* is the anti-pattern this directive exists to stop:
push it into the corpus.

- **Do not add new Rust `#[test]`s for language/semantic behavior.** Reserve host-language tests for
  what genuinely cannot live in the corpus — internal data-structure invariants, parser/printer
  plumbing with no observable Cadenza-level behavior, and the like. Anything a Cadenza program can
  observe belongs in the corpus.
- **If the corpus is missing the functionality you need to express a test, RAISE A FLAG** — do not
  fall back to a Rust test and do not work around the gap. Per the corpus policy, lock in the
  idealistic (spec-correct) expectation as a corpus `TODO` and route the underlying gap to its owner;
  a found gap is a win, not a reason to bake a test into Rust.

This restates, at repository level, the behavior-gate above and the corpus policy every fleet agent
already follows: the corpus is the single source of truth for what a construct does.

## The hub is BARE — you cannot edit it; always work in your own worktree

The hub repository is **bare**: it has no working tree, so there is no central checkout to edit
and no shared files to clobber. Every change MUST happen in a git worktree under
`.claude/worktrees/`. This is enforced structurally, not by convention — there is nothing at the
hub path to `git add`, and the pre-commit hook refuses a direct commit onto the integration branch
from anywhere but the integrator's own worktree.

The workflow for any change:

1. **Create a fresh worktree off the tip of `trunk`** — the local integration branch:

   ```sh
   git worktree add -b <topic> .claude/worktrees/<topic> refs/heads/trunk
   ```

   Give it a name unique to your task; do not reuse an existing named worktree, since a
   concurrent agent may be mid-edit inside it. (If you are a fleet agent, `cargo xtask fleet add`
   does this for you.)

2. **Make your change there, alone.** All worktrees share the hub's one object store, so a commit
   you make is visible fleet-wide immediately — but the files are yours. If you must read another
   worktree, `git status --short` first and treat any file you did not create as foreign — commit
   only your own files by path, never `git add -A`.

3. **Run the gate from the worktree.** From inside it, `cargo xtask gate` (and `cargo xtask check`
   for the full health signal) — the seed toolchain builds and runs against your isolated tree, so
   a sibling's mid-edit cannot corrupt your result. Do not request integration for a change whose
   gate you have not seen pass.

4. **Integrate through the single writer.** `trunk` is advanced by ONE agent only — `pr-sync`.
   You never write `trunk` yourself (no `update-ref` CAS, no fast-forward race). Commit in your
   worktree, then send the integrator a merge request:

   ```sh
   cargo xtask fleet send --to pr-sync --kind merge-request \
       --subject "<your-branch>" --ref "$(git rev-parse HEAD)" --body "<gate summary>"
   ```

   `pr-sync` merges your commit into `trunk`, re-gates, and replies `merged` or `reject`. Because
   one agent serializes every merge, there are no dropped commits and no phantom-stale hub.

5. **Remove the worktree when the work is merged** (`git worktree remove`), so `.claude/worktrees/`
   does not accumulate stale trees. (A fleet agent calls `cargo xtask fleet remove <self>`.)

`trunk` is the LOCAL integration branch only — it does not exist on the remote.

## Publishing to the remote — `pr-sync` maps `trunk` onto `origin/main` via a PR

The remote default branch is **`main`** (protected: a ruleset requires the `checks / …` CI jobs to
pass, so it cannot be pushed to directly). Local `trunk` is what everything integrates onto, and
the `pr-sync` agent maps it onto `origin/main` through a pull request as part of its cycle:

```sh
git push origin trunk:staging-<topic>         # push the integrated tip to a staging branch
gh pr create --base main --head staging-<topic> --fill
```

CI runs on the PR; it merges into `main` once the required checks are green. Do NOT try to push
`trunk` straight to `main` — the mapping is only at publish time, and the direct push is refused by
the ruleset. (Prior generations live on the remote `old` branch.)

## The autonomous-agent fleet

Day-to-day work is driven by a fleet of looping agents managed by `cargo xtask fleet`
(`up`/`down`/`status`/`add`/`remove`/`send`/`archive`), each a tmux window running one role from
`.claude/fleet/loops/`. The durable manifest is `.claude/fleet/registry.json`; the agent contract
(inbox protocol, the single-writer integration model, the run-unattended rule) is
`.claude/fleet/AGENTS-fleet.md`. Agents coordinate through a file-based inbox, never by editing a
shared checkout, and only the `concierge` agent talks to the operator. `cargo xtask fleet up`
reconstitutes the whole fleet from the manifest after a reboot.

## The build loop

`./start.sh` is the front door: it installs the neutral commands in `commands/` as Claude Code slash
commands (into the gitignored `.claude/commands/`), scaffolds the gitignored `implementation/`
workspace, and launches a session that runs [`/build`](commands/build.md) in the selected mode
(autonomous by default; `--author` for attended). `/build` orchestrates the rest: `constitution` /
`specify` / `clarify` to author the spec, `analyze` to check it is gate-ready, `plan` to derive the
ordered climb from the seed to the full realized language once the choices are made, `setup-gate` /
`gate` to run the two gates, and `ignite` / `regen` / `promote` / `learn` to synthesize the seed
toolchain, produce a generation, promote a gated candidate, and feed a spec gap back into the
specification. The
commands are agent-agnostic prompt bodies and are the durable, committed way to drive a build; the
build modes they obey are fixed by [spec/capabilities/build-modes.md](spec/capabilities/build-modes.md).

After any spec edit, run `analyze` and confirm it ends `ANALYZE: PASS`.

## Keep the compiler's own docs current — a doc is part of the change

The seed toolchain under `implementation/seed/crates/` (`cadenza-syntax`, `rcdzc`, `cdz-run`,
`cdz-runtime`, `cdz-corpus`) is a regenerable projection of the specs, but it is *committed*,
and its module docs and comments are read as the current truth by the next agent. Treat a
comment the same way you treat code: **if your change makes a comment false, fixing the comment
is part of your change, not a follow-up.** The failure mode this rule exists to prevent is
already visible in the tree — comments that say "Stage 0 only", "when we get type inference",
"not yet built", or "for now" for a capability that has since shipped. A stale status comment
is worse than no comment: it actively misinforms.

Concretely, when you touch a module:

- **State what is, not what was.** Describe the code's current behavior and the invariant it
  keeps. If a limitation is genuinely still present, say so and say *why it is declined*, not
  "not yet" (which rots the moment it ships). A forward-looking note belongs in a design doc or
  a memory, not in a `//!` header that outlives it.
- **Delete the scaffolding narrative.** "Stage 0's thin slice", "this is temporary", and
  "later stages widen this" describe a construction history the reader does not have and cannot
  verify. If the staging still matters, cite the normative sentence that mandates the shape (see
  below); otherwise cut it.
- **Prefer a duvet citation to a prose justification.** When a module is shaped the way it is
  *because the specification requires it*, cite the requirement inline with a `//=` / `//#`
  pair rather than paraphrasing it. The pair is machine-checked: `//= <spec>#<section>` names
  the requirement and `//# <exact sentence>` quotes it verbatim, so duvet fails the requirement
  gate if the quoted words drift out of the spec. This turns "why is it like this?" from a
  comment that can silently go stale into a link the gate keeps honest — and it credits the
  seed compiler with the spec coverage it actually provides. The seed sources are scanned by
  both `.duvet/config.toml` and `.duvet/bootstrap.toml` under
  `implementation/seed/crates/**/*.rs`; run `duvet report` to confirm a new citation resolves.

## Authoring order (of the specification itself)

Author for internal consistency: glossary + constitution together, then `overview.md`, then
the frozen contracts (ABI first), then the `options/` decisions each contract points at, then the
capability specs (core semantics and type system first), then migrate the executable-semantics
corpus, then `bootstrap.md`, and finally `traceability.md` and the gate wiring. Freeze the
component ABI and the determinism contracts *before* writing capabilities — a capability
references the ABI, and the whole reason for this reboot is that the byte-level target was
never pinned first.

## What not to commit

`implementation/` (the regenerable compiler), `.claude/` (the local slash-command install target and
agent-local files), and `.duvet/reports/` are ignored. `.duvet/config.toml` and
`.duvet/bootstrap.toml` (the gate configuration), `commands/` and `start.sh` (the build loop),
`templates/`, and the entire specification tree ARE committed.

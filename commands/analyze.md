# Command — analyze

**Purpose.** Cross-check the specification tree for internal consistency,
extractability, traceability, and standalone-ness. This is the command that makes
"the path to self-hosting is real" verifiable: a clean `analyze` on the
hand-authored spec tree means the specifications are well-formed and gate-ready,
so a generation synthesized from them can be judged rather than second-guessed.

**Agent-agnostic.** This file is a neutral prompt body. It is the source of truth
for the command; per-agent command directories are generated from it. It assumes
only a shell and the `duvet` CLI.

## Inputs

- The specification tree: `constitution.md`, `spec/overview.md`,
  `spec/glossary.md`, `spec/contracts/**`, `spec/capabilities/**`,
  `spec/semantics/*.sexp`, `spec/bootstrap.md`, `spec/traceability.md`,
  `spec/learnings/**`.
- The declared-default location: `options/**` (each `options/<decision>/` names a
  `DEFAULT:` choice).
- The gate configuration: `.duvet/config.toml`, `.duvet/bootstrap.toml`.

## Checks to run

Run every check and report all findings; do not stop at the first failure.

### 1. Requirement form

- Every normative statement is a single self-contained RFC-2119 sentence
  (MUST / MUST NOT / SHALL / SHALL NOT / SHOULD / SHOULD NOT / MAY) under a stable
  section heading (constitution §XIII, AGENTS.md rule 1).
- Confirm extractability: for each normative file, run
  `duvet extract -f markdown -o <tmp> ./<file>` and verify the extracted
  requirement count is non-zero and matches the visible RFC-2119 sentences.
- Flag any RFC-2119 keyword that appears outside a single-sentence requirement
  (e.g. buried in a paragraph), because it will extract ambiguously.
- **Atomicity:** flag any requirement sentence that carries more than one
  obligation — two clauses joined by "and", two separate `MUST`/`MUST NOT`
  keywords, or obligations placed on two different actors — because each obligation
  must be independently citable and testable (constitution §XIII, conformance-gate.md
  §"Requirements Are Written To Be Extractable"). A single "and" is acceptable only
  when it joins parts of one obligation (a list of inputs, or a subordinate
  "so that…" rationale), not two obligations that could be satisfied or violated
  independently.
- **Enforceability (constitution §XV):** flag any normative requirement for which
  no enforcing line can exist — no generation implementation-and-test citation, no
  citation in code that performs a governance act the requirement mandates, and no
  change-process check that guards a future edit it constrains (the three
  enforcing-line kinds in conformance-gate.md §"Every Requirement Binds To An
  Enforcing Line"). Such a statement is descriptive prose miscast as a requirement
  and MUST be reclassified rather than left unenforceable. A requirement carved out
  as non-load-bearing for a generation is not a finding here so long as it binds to
  an enforcing line elsewhere — a governance or change-process line named in
  `options/gate-non-load-bearing/` (conformance-gate.md §"A Change-Process
  Requirement Is Not Load-Bearing For A Generation").

### 2. Gate wiring

- Every normative file (constitution, all contracts, all capabilities, bootstrap)
  appears as a `[[specification]]` block in `.duvet/config.toml` with
  `format = "markdown"`.
- The descriptive files (overview, glossary, traceability, and everything under
  `spec/learnings/`) are NOT listed as specifications (they carry no requirements),
  and the executable-semantics corpus under `spec/semantics/` is NOT listed either —
  it is gated by execution, not by RFC-2119 extraction (AGENTS.md §"The two gates").
- `duvet report` runs without a path-resolution or parse error. Source paths in the
  config resolve relative to the project root (the parent of `.duvet/`), not to the
  config file's directory.
- The bootstrap subset in `.duvet/bootstrap.toml` lists exactly the constitution,
  all frozen contracts under `spec/contracts/`, `spec/bootstrap.md`, and the six
  ignition-set capabilities the seed compiler must satisfy to clear the ignition bar
  (`spec/bootstrap.md` §"The Ignition Bar"), enumerated in `.duvet/bootstrap.toml`:
  `core-semantics`, `type-system`, `capabilities-and-effects`, `compiler-pipeline`,
  `conformance-gate`, `self-hosting-and-bootstrap` — and does NOT list any capability
  outside that ignition set (the richer capabilities realized by later generations,
  e.g. `numeric-model`, `verification-layers`, `diagnostics`, `tooling-and-lsp`).
- Every `[[specification]]` `source` in `.duvet/bootstrap.toml` also appears in
  `.duvet/config.toml` — the ignition subset is a strict subset of the full set,
  never a superset (bootstrap.toml's stated INVARIANT). Diff the two source lists and
  flag any bootstrap-only entry.
- The two configs do not silently diverge on the shared entries: any file gated by
  both lists it with the same `source` path and `format = "markdown"` in each.

### 3. Traceability (bidirectional)

- Every normative document in `spec/traceability.md` maps to at least one real
  `spec/overview.md` section heading (AGENTS.md rule 6, constitution §Governance).
- Every `spec/overview.md` section appears in the reverse table, served by at least
  one normative document — except §16, which is descriptive and served by
  `spec/learnings/`.
- Flag any normative document that traces to a non-existent overview heading, and
  any overview section (other than §16) with no normative document serving it.

### 4. Standalone-ness (no prototype, no implementation)

- No normative file names a concrete engine, a hashing algorithm, a numeric width,
  a prior prototype, a library, a cloud service, or a source-file path (constitution
  §XIII, AGENTS.md rule 2). Grep for banned tokens (a prior generation's name, `.rs`
  and other source-file extensions, `implementation/`, `crates/`, a concrete engine
  or hash name, a fixed integer/float width) and report any hit in a normative file.
  Hits under `spec/learnings/` (historical reference) and under `options/` (the
  declared-default location, where concrete choices intentionally live) are not
  findings.
- Contract and capability bodies describe the execution environment as "runtime" and
  "sandbox" and the runnable form as "component", rather than naming a specific
  runtime engine or format; no normative file, and no spec filename, names a concrete
  execution technology (the concrete engine/format lives only in `options/`, per
  constitution §XIII third sentence).

### 5. Glossary and vocabulary

- Every term used normatively is defined once in `spec/glossary.md` (AGENTS.md
  rule 3).
- No term is redefined in a contract or capability that the glossary already defines.

### 6. Frozen-contract drift

- If run against a change, diff each `spec/contracts/**` file against its committed
  version and flag any change to a frozen contract that alters the bytes produced
  from unchanged source and is not accompanied by a documented version increment and
  migration path (each contract's §"Additive Evolution Of This Contract",
  constitution §Governance Floors "The Component ABI Changes Only By Coordinated
  Act").

### 7. Behavioral witnessing (the behavior gate's corpus is complete)

- Every behavioral requirement of an included capability is witnessed by at least
  one case in `spec/semantics/*.sexp` (conformance-gate.md §"Every Behavioral
  Requirement Is Witnessed By A Case", `spec/semantics/README.md` §"Authoring rules").
- Flag any load-bearing behavioral requirement — one that describes runtime behavior
  and could be discharged only by execution (conformance-gate.md §"A Behavior
  Requirement Is Covered Only By Execution") — for which no case in the corpus
  exercises it, because a generation whose corpus omits a witnessing case for such a
  requirement MUST NOT be promoted.
- Confirm each `.sexp` case is well-formed against the case vocabulary
  (`case`/`input`/`output`/`error`/`trap`/`doc`) and carries a definite result, so a
  behavior-gate run has an exact expectation to compare against.

### 8. Decisions carry declared defaults

- Every open point the specification leaves resolvable more than one way is recorded
  as a decision under `options/<decision>/` whose `README.md` names its default with
  a `DEFAULT: <choice>` line (AGENTS.md rule 7, `options/README.md`).
- Flag any `options/<decision>/` whose README has no `DEFAULT:` line, and any
  `DEFAULT:` naming a `<choice>.md` that does not exist in that directory, because an
  autonomous build cannot proceed with a decision that has no applicable default.
- Confirm `options/gate-non-load-bearing/` names the pinned non-load-bearing set, so
  that two builds subtract an identical set from the load-bearing total
  (conformance-gate.md §"A Change-Process Requirement Is Not Load-Bearing For A
  Generation").

## Output

Produce a report grouped by check, listing each finding with the file and section
it concerns. End with a single verdict line: `ANALYZE: PASS` only if every check
passes, otherwise `ANALYZE: FAIL` followed by the count of findings per check.

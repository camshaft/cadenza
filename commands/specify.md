# Command — specify

**Purpose.** Author or revise one capability or contract specification. This is an authoring command:
it changes files under `spec/` only, never code.

**Agent-agnostic.** Neutral prompt body.

## Usage

`specify <target>` where `<target>` names a capability (e.g. `capabilities/type-system`) or a
contract (e.g. `contracts/component-abi`).

## Procedure

1. Choose the template: `templates/capability-spec.md` for a capability, `templates/contract.md` for a
   frozen contract.
2. Author the specification following the house conventions:
   - Every normative statement is a single self-contained RFC-2119 sentence carrying exactly one
     obligation, under a stable section heading, so it extracts and cites unambiguously.
   - Use only vocabulary defined in `spec/glossary.md`; if a new term is needed, add it to the
     glossary first.
   - Keep it standalone: no reference to a prior prototype, a concrete engine, a hashing algorithm, a
     numeric width, a library, or a source-file path. Describe the runnable form as "component" and
     its execution environment as "runtime" / "sandbox"; a concrete technology choice lives in
     `options/`, never in a requirement.
   - State behavior and invariants (what must hold), not implementation (how).
3. Trace it: add the new file and its sections to `spec/traceability.md`, mapping each to the
   `spec/overview.md` section(s) it serves, and confirm the reverse table still covers every overview
   section.
4. Wire it: add a `[[specification]]` block for the new file to `.duvet/config.toml` (and
   `.duvet/bootstrap.toml` if it is part of the seed-toolchain ignition subset) with
   `format = "markdown"`.
5. Verify: run `analyze` and confirm the new file extracts cleanly and passes every check; for a
   capability, witness each behavioral requirement with at least one case in the executable-semantics
   corpus (`spec/semantics/`, `templates/semantics-case.md`) so the behavior gate exercises what the
   requirement gate cites.

## Guardrails

- A frozen contract is authored or changed only under the constitution's frozen-contract change
  discipline (its Governance Floors): a change is additive with respect to already-derived components,
  or it carries a version increment and a stated migration path evaluated against already-derived
  components.
- A capability spec states what must hold and leaves implementation free, so that two regenerations
  may differ in code and both be correct.

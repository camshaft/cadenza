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

## Authoring order

Author for internal consistency: glossary + constitution together, then `overview.md`, then
the frozen contracts (ABI first), then the `options/` decisions each contract points at, then the
capability specs (core semantics and type system first), then migrate the executable-semantics
corpus, then `bootstrap.md`, and finally `traceability.md` and the gate wiring. Freeze the
component ABI and the determinism contracts *before* writing capabilities — a capability
references the ABI, and the whole reason for this reboot is that the byte-level target was
never pinned first.

## What not to commit

`implementation/` (the regenerable compiler), `.duvet/reports/`, and any agent-local
directories are ignored. `.duvet/config.toml` and `.duvet/bootstrap.toml` (the gate
configuration) ARE committed, as are `templates/` and the entire specification tree.

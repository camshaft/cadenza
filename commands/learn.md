# Command — learn

**Purpose.** Capture a learning from a regeneration or from a build that hit a
spec gap or defect, and turn it into a specification change. This is how the loop
closes the class of a bug for every future generation of the compiler rather than
patching one instance.

**Agent-agnostic.** Neutral prompt body. Changes `spec/learnings/` and the
specification the learning drives.

## Procedure

1. Write the learning as a new file `spec/learnings/YYYY-MM-DD-short-title.md`, in
   the entry format `templates/learning.md` defines, and add a line for it to the
   index `spec/learnings/README.md`. Do not append to an existing entry — one file
   per learning keeps two authors from conflicting on a shared file.
   - **What happened** — the concrete observation: what was built, derived, or run,
     and what went wrong. This is the one place a specification artifact may name a
     prior prototype, a concrete language, or a specific implementation, because a
     learning is historical reference.
   - **Why** — the root cause, stated as a property of the design or process,
     especially whether the spec under-determined the behavior or the synthesis
     missed something that was specified.
   - **The requirement it drove** — the requirement sentence, by spec file and
     section, that this learning adds or tightens.
2. Make the requirement change: add or tighten the RFC-2119 requirement in the
   relevant capability or contract (or, for an invariant, the constitution). A
   learning that changes no requirement is a diary entry and changes no future
   generation — this pairing is mandatory.
3. If the learning is a recurring nondeterminism, ambiguity, or a subsystem that
   passed a shape check without running, prefer turning it into an explicit
   conformance point — a behavior requirement witnessed by a case in
   `spec/semantics/`, discharged by execution per `conformance-gate.md` §"A
   Behavior Requirement Is Covered Only By Execution" — so the gate catches the
   class thereafter.
4. Run `analyze` to confirm the new or changed requirement extracts cleanly (a
   single self-contained RFC-2119 sentence under a stable heading, per
   `conformance-gate.md` §"Requirements Are Written To Be Extractable") and that
   traceability stays complete.

## Guardrails

- Every `learn` entry MUST pair its narrative with a concrete requirement edit.
- `spec/learnings/` is append-only in spirit: a superseded learning file is
  annotated, not deleted.
- A requirement change that would alter a frozen contract or weaken a governance
  floor requires the explicit human approval the constitution's Governance Floors
  require.

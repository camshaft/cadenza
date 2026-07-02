# Command — constitution

**Purpose.** Author or amend the constitution — the non-negotiable invariants and governance floors
the Cadenza language and its compiler inherit. This is the most consequential authoring command;
amendments follow strict discipline.

**Agent-agnostic.** Neutral prompt body. Changes `constitution.md` only.

## Procedure

1. Read the current `constitution.md`, `spec/overview.md` (the intent arbiter), and
   `spec/glossary.md`.
2. Author or amend invariants following the house conventions:
   - Each invariant is a single self-contained RFC-2119 sentence carrying exactly one obligation,
     under a stable heading in Core Principles or Governance Floors.
   - Use only glossary vocabulary; stay standalone (no concrete engine, hashing algorithm, numeric
     width, prior prototype, library, or source-file path — those live in `options/`).
3. Trace: confirm `constitution.md` maps to the `spec/overview.md` sections it realizes in
   `spec/traceability.md`, and that the reverse table (every overview section served by a normative
   section) stays complete.
4. Amendment discipline (per the constitution's own Governance Floors):
   - Record the amendment and its rationale in a new `spec/learnings/` entry
     (`templates/learning.md`), paired with the requirement it added or tightened.
   - An amendment that weakens a governance floor requires explicit human approval.
   - Update the `Version` / `Last Amended` footer.
5. Verify: run `analyze`; confirm the constitution still extracts cleanly and every downstream spec —
   the frozen contracts under `spec/contracts/` and the capability specifications under
   `spec/capabilities/` — remains consistent with it.

## Guardrails

- The constitution supersedes all other specifications on an invariant; never introduce a principle a
  frozen contract or a capability specification already contradicts without reconciling both.
- Governance floors bound the amendable governance policy; do not express a floor as policy data.

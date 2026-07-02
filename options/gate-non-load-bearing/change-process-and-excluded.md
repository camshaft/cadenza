# Gate Non-Load-Bearing Set — Choice: change-process-and-excluded

> **The default choice for the `gate-non-load-bearing` decision** (see [README.md](./README.md)). It
> enumerates the requirements the gate does not hold load-bearing for a generation, each routed to a
> change-process or governance enforcing line rather than exempted from enforcement.

## Not load-bearing for a generation's requirement gate

These requirements govern how the specification itself changes, or mandate a human governance act.
They are enforced at the point an edit or an amendment is made — the change-process check — not by a
generation's implementation-and-test citations. They remain enforceable; they are simply enforced by
a different mechanism.

- **`constitution.md` §Governance Floors** — every requirement under "The Component ABI Changes Only
  By Coordinated Act", "Determinism And Capability-Safety Are Never Downgradable", "Reproducibility
  Outranks Optimization", and "Amendment Discipline". Enforced by the change-process check that guards
  an edit to a frozen contract, a compiler configuration, or this constitution.
- **`constitution.md` §XV third sentence** — "A requirement that pins the shape of an artifact
  without an accompanying requirement that some path exercises that artifact MUST NOT be treated as
  sufficient." Enforced by the authoring/analyze check, not by a generation's runtime.
- **`constitution.md` §XIII third sentence** — "A concrete technology choice MUST be recorded at the
  declared-defaults location rather than in a normative requirement." Enforced by the standalone lint.
- **Each `spec/contracts/*` §Additive Evolution** — the additive-or-version-increment requirement.
  Enforced by the change-process check that guards a frozen-contract edit.
- **`conformance-gate.md` §Requirements That Do Not Gate A Generation** and §"Every Requirement Binds
  To An Enforcing Line" — the meta-requirements about the gate itself. Enforced by the gate's own
  configuration and the analyze check.

## How the routing works

A requirement in this set MUST still carry a citation — but to the change-process check or governance
code that enforces it, not to a generation's implementation and test. The gate reads this set to
subtract these requirements from the load-bearing total it computes for a generation, so that a
generation is not failed for lacking a runtime implementation of a rule about how the spec may change.

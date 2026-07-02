# Decision — Structural Interface

**The decision.** The concrete shape of the interface through which an agent reads and rewrites a
program's canonical representation — the affordance the north star's top priority rests on. The
constitution requires that a structural interface exist and operate without re-parsing unrelated code
(constitution X; agent-authoring.md), but it does not fix how nodes are addressed or what edit and
query operations are offered, because those are the choices this decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A structural interface exists to read and rewrite the canonical representation without textual
  patching (agent-authoring.md §A Structural Interface Exists).
- A structural edit yields a well-formed program or a machine-readable rejection (agent-authoring.md).
- Node addressing is a deterministic function of the representation, and a query result is
  reproducible (agent-authoring.md §Structural Addressing Is Deterministic).

## Choices

- [`content-addressed-nodes`](./content-addressed-nodes.md) — nodes addressed by a stable path plus a
  content-derived id, a closed set of edit operations (insert/replace/delete/move), span↔node
  mapping, and query by kind and by name. **The default.**

DEFAULT: content-addressed-nodes

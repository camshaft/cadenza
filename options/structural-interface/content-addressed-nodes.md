# Structural Interface — Choice: content-addressed-nodes

> **The default choice for the `structural-interface` decision** (see [README.md](./README.md) for
> the decision and the requirements a choice must satisfy). It pins how an agent addresses, edits, and
> queries the canonical representation.

## Node addressing

- **Path address** — a node is addressable by its path from the program root: a sequence of
  field/index steps that is a deterministic function of the representation, so the same node has the
  same path on every conforming compiler.
- **Content-derived id** — a node also carries an id derived by hashing its subtree under the
  canonical value form, so a node can be re-found after edits elsewhere and two structurally identical
  subtrees share an id.
- **Span mapping** — each node maps to its source span in a display, and each span maps back to the
  smallest node covering it, so an agent can move between a display position and a node.

## Edit operations

A closed set, each yielding either a well-formed program or a machine-readable rejection:

- **`insert`** — add a node at an addressed position.
- **`replace`** — replace the node at an address with a new subtree.
- **`delete`** — remove the node at an address.
- **`move`** — relocate the node at an address to another position.

An edit operates without re-parsing code unrelated to its target, and preserves the documentation
attached to nodes it does not change (agent-authoring.md §Documentation Survives Round-Trip And Edits).

## Query operations

- **query by kind** — all nodes of a given construct kind.
- **query by name** — the definition or references bound to a name.
- **query by address** — the node at a path or content-derived id.

Every query result is a deterministic function of the representation, so an agent targeting and
re-targeting edits gets reproducible answers.

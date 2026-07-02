# Code Shape — Choice: conventional-primary

> **An alternative choice for the `code-shape` decision** (see [README.md](./README.md)). Not the
> default. It satisfies every requirement the decision names; it trades differently against the north
> star than the default.

## The choice

The **canonical stored form is the binary AST**, as in every choice, but the **primary textual syntax
is a conventional ML/Rust-family surface** (expression-oriented, keyword- and brace-delimited,
indentation-insensitive) — the form authors read and write by default. An s-expression syntax exists
as a *secondary* view for metaprogramming, but it is not the primary one.

## How it satisfies the requirements

- **Canonical form round-trips:** the conventional syntax is delimiter-explicit and
  indentation-insensitive, so it parses to and prints from the binary AST losslessly.
- **Structural interface:** operates on the binary AST, as in the default.
- **Reproducible codegen:** unaffected — the hash is over the binary AST, not any textual rendering.

## Trade-off vs. the default

- **Toward priority #2 (human readability):** the conventional surface is unambiguously primary, so
  the "read by humans" story needs no projection indirection.
- **Away from priority #1 (agent authoring) and #3 (verification):** an agent and the verification
  tooling manipulate the tree, but the tree is a *secondary* artifact behind the primary surface
  rather than the homoiconic thing an agent most naturally emits; metaprogramming is a second-class
  view. The default's decoupling serves agents and verification better while keeping the same human
  surface, which is why the default is preferred.

## When to pick this

Pick this if the deployment values a single unambiguous human surface over first-class code-as-data,
and does not expect heavy agent-driven structural generation or metaprogramming.

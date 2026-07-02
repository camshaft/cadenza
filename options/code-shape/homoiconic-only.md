# Code Shape — Choice: homoiconic-only

> **An alternative choice for the `code-shape` decision** (see [README.md](./README.md)). Not the
> default. It satisfies every requirement the decision names; it trades away a north-star priority.

## The choice

The **canonical stored form is the binary AST**, as in every choice, and the **s-expression syntax is
the sole textual syntax** — there is no conventional human-facing surface.

## How it satisfies the requirements

- **Canonical form round-trips:** s-expressions are delimiter-explicit, so the syntax parses to and
  prints from the binary AST losslessly.
- **Structural interface:** operates on the binary AST directly, as in the default.
- **Reproducible codegen:** unaffected — the hash is over the binary AST.

## Trade-off vs. the default

- **Toward priorities #1 and #3:** maximal for agent authoring and verification — the representation
  is the surface, so there is no projection layer at all.
- **Away from priority #2 (human readability):** deeply nested parenthesized forms read poorly to
  humans compared with a conventional surface. The default keeps this same code-as-data core *and*
  offers a conventional display for humans, so it dominates this choice on the north star; this
  choice is preferable only where human readability is explicitly a non-goal.

## When to pick this

Pick this if the deployment is agent-only or tooling-only, human authorship is a non-goal, and the
simplicity of "no projection layer" is worth more than a human-friendly display.

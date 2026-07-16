# Mirrored from GitHub #468 — "Notebook application"

- **Issue:** https://github.com/camshaft/cadenza/issues/468
- **Author:** camshaft (the operator)
- **Created:** 2026-07-16T10:58:40Z
- **Labels:** (none)
- **Routing:** NOT-YET-DESIGNED capability → concierge `backlog` note recommending a `design` agent
  (a new-vertical "we should build" feature, not a concrete bug). Like #400 (rsolid CAD), the liaison
  does NOT spin up agents — the human-facing agent-spinup judgment stays with the concierge.

## Operator's text (verbatim)
> Similar to the calculator and cad applications, we should build out a notebook experience (think
> jupyter) where you write markdown with inline cadenza programs in specific code blocks and can do
> formulas, render graphs, display tables, etc. it would be great if you could also render widgets the
> user can interact with so the programs need to be able to take input values at runtime and recalculate
> their output. This would be a new vertical owner.

## Liaison notes (for the design agent / concierge)
A large new vertical — a Jupyter-like notebook, explicitly "a new vertical owner." Key design points:
- **Surface:** markdown documents with inline Cadenza programs in designated code blocks (calculator/CAD
  are the multi-surface precedents — reuse the browser IDE + run-worker infra, not browser-limited).
- **Outputs:** formulas, rendered graphs, tables — a rich cell-output model.
- **Interactivity:** render WIDGETS the reader can manipulate → programs must accept **runtime input
  values and recalculate** their output (a reactive/dataflow recompute story, not just static eval).
- **Precedent:** models on the calculator + CAD apps' architecture (the guide playground / run workers).
- Needs a design pass to carve into vertical-ready increments (cell model → run-and-render → graph/table
  renderers → interactive-widget/runtime-input recompute). The runtime-input-recompute is the novel
  piece and likely wants the design agent's attention first.

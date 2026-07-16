# Vertical charter: a Jupyter-like Notebook application (GitHub #468, operator-directed)

**Operator directive (GitHub #468, verbatim):** "Similar to the calculator and cad applications, we
should build out a notebook experience (think jupyter) where you write markdown with inline cadenza
programs in specific code blocks and can do formulas, render graphs, display tables, etc. it would be
great if you could also render widgets the user can interact with so the programs need to be able to
take input values at runtime and recalculate their output. This would be a new vertical owner."
(https://github.com/camshaft/cadenza/issues/468 — github-liaison will CLOSE it once the work genuinely
lands on trunk.)

## Your mandate
Own a NEW standing vertical: a **Jupyter-like notebook** for Cadenza — markdown documents with inline
Cadenza programs in designated code blocks, rich cell outputs (formulas / graphs / tables), and
**interactive widgets** where the reader manipulates inputs and the programs **recalculate at runtime**.
This is the third operator "app-scale showcase" vertical (after the calculator + CAD/#400) — a flagship
"look what you can build with Cadenza" surface. Like those, it's DESIGN-FIRST and multi-surface.

## Reuse what exists — don't reinvent the browser stack
The calculator + CAD (`/calculator`, `/cad`) + the guide playground already solved the hard browser
plumbing you need: the in-browser Cadenza compile+run path (cdz-wasm + jco), run-WORKERS (so eval is
off the main thread, not browser-limited), and the guide's build/route/deploy (v-guide-infra owns the
guide site + now has `check:visual` for headless-browser verification). COORDINATE with v-guide /
v-guide-infra early — the notebook is almost certainly another guide surface (a `/notebook` route or a
standalone app page) reusing that run-worker + IDE infra, NOT a from-scratch runtime. Don't rebuild the
compile/run path; wrap it in the notebook cell model.

## The pieces (sequence — design-first, the last one is the novel/hard part)
1. **Cell model + markdown-with-code-blocks.** A notebook = markdown prose interleaved with Cadenza
   code cells (a designated fence, e.g. ```cadenza). Parse the doc into ordered cells; render prose as
   markdown (the guide already renders markdown — reuse), code cells as editable+runnable. Increment 0
   is the DESIGN of the cell/document model + how cells share scope (does cell 2 see cell 1's bindings?
   — a real semantics question: notebook-style sequential scope vs isolated cells).
2. **Run-and-render.** Each code cell compiles+runs (via the existing run-worker) and its RESULT renders
   as a typed cell output — a value, a formula, etc. Reuse the calculator's value→display rendering.
3. **Rich output renderers.** Graphs (a plotting surface — likely a JS charting lib behind a lazy
   route, like CAD's three.js), tables (structured data → HTML table), formulas (math rendering). Each
   is a cell-output type keyed on the value's shape/type.
4. **Interactive widgets + runtime-input recompute (THE NOVEL PIECE — design it carefully).** A widget
   (slider/input/dropdown) the reader manipulates feeds a RUNTIME input value into a cell's program,
   which RECALCULATES its output reactively. This is a reactive/dataflow story: a widget change →
   re-run the dependent cell(s) → re-render outputs. Design questions for the design pass (route the
   real forks to the concierge → operator): how does a program DECLARE a runtime input (a typed
   parameter the widget binds to)? what's the dependency/recompute graph (which cells re-run when a
   widget changes — just the owning cell, or downstream cells that used its output)? is it push
   (widget→recompute) or pull? This is where the design agent's attention goes first — it's the
   feature's novel core, and it interacts with how cells share scope (piece 1).

## How to work
- **Increment 0 = a DESIGN doc** (`design/notebook-app.md`): the cell/document model, cell-scope
  semantics, the rich-output-type model, and — most importantly — the interactive-widget /
  runtime-input-recompute (reactive) design. Route the genuine forks to the concierge (→ operator):
  cell-scope (sequential vs isolated), the widget-input declaration syntax, the recompute-graph model.
  The operator has strong views on app-scale surfaces (see how CAD's scope/G3 decisions went).
- COORDINATE early + widely: v-guide-infra (the guide site / route / run-workers / deploy /
  check:visual), v-guide (markdown rendering + it may want the notebook AS guide content), v-cad (the
  three.js/lazy-heavy-dep-route pattern for the graph renderer is a direct precedent), and whoever owns
  the value→display rendering the calculator uses. Reuse, don't rebuild.
- This is a REAL stress test of the browser run path + the language's runtime-input story — REPORT/FIX
  language/infra gaps you hit (e.g. if "a program takes a runtime input value" needs a language feature
  that doesn't exist cleanly, that's a finding to surface, not work around).
- Every notebook example must actually RUN (the guide's run-every-example discipline + the new
  check:visual headless verification apply — a notebook cell that doesn't compile/run is a bug).

## Not urgent, do it right — depth over speed
The operator framed it as "we should build out" a new-vertical-owner — a standing, long-horizon charter,
not a sprint. A crisp Increment-0 design that nails the reactive runtime-input-recompute model (the
novel core) is worth more than premature cell-rendering code. Strong owner: each tick advance the design
or an increment; if idle, deepen the reactive-recompute design or the cell-scope semantics. github-liaison
closes #468 when the work genuinely lands on trunk — so drive toward a real shippable notebook surface.

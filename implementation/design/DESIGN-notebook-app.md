# DESIGN — Cadenza Notebook: a Jupyter-like markdown+code surface with reactive widgets

*2026-07-16. Owner: `v-notebook` (new standing vertical). Operator directive (GH #468, verbatim):*
> "Similar to the calculator and cad applications, we should build out a notebook experience (think
> jupyter) where you write markdown with inline cadenza programs in specific code blocks and can do
> formulas, render graphs, display tables, etc. it would be great if you could also render widgets the
> user can interact with so the programs need to be able to take input values at runtime and recalculate
> their output. This would be a new vertical owner."
> (https://github.com/camshaft/cadenza/issues/468 — github-liaison closes it when the work lands on trunk.)

> **STATUS (updated 2026-07-16) — SHIPPED. `/notebook` is LIVE + functional on trunk, check:visual-green.**
> The full feature landed as a stack of gated slices: the pure model layer (`guide/src/notebook/` —
> `parseDocument`, `assembleCell`, `parseWidgets`, `sexpr`, `extractTable`, `extractChart`, `renderOutput`,
> `recomputePlan`, `parseProse`, `assembleForRun`; ~210 unit tests), the presentational components
> (`ProseView`, `OutputView`, `WidgetControls`), and the live route `NotebookPage.tsx` (lazy `/notebook` in
> `main.tsx`, mirroring /cad). The reactive widget→recompute core works end-to-end in the browser and is
> guarded by v-guide-infra's `check:visual` case (first cell → 1050; rate drag → 1200) + a standing
> `npm run check:notebook` headless gate. The three §6 forks all shipped on their recommended defaults
> (D1 sequential scope, D2 notebook-directive widget DSL + a filed first-class-`input` language finding,
> D3 hand-rolled SVG charts). Two implementation decisions confirmed in the build (not in the original
> design): (a) `replEval`'s entry is an EXPRESSION, so a cell's def-block goes in the buffer and the entry
> is a `(main)` call; (b) the notebook runs in a FIXED s-expr surface (the /cad approach), not the global
> editing surface. GH #468's core is delivered + regression-protected. Remaining work is polish (see the
> vertical log's improvement backlog: `hidden`-cell output UX, a real formula renderer, a quantity/rational
> cell renderer, a doc-editor pane, more widget kinds, a notebook-as-guide-chapter content type).
>
> *The original Increment-0 design pass (below) was written against `trunk` @ `88d84df66`; it held up — the
> forks shipped on their defaults and the reuse-the-browser-pipeline thesis was borne out.*

Like the calculator (`DESIGN-calculator-repl.md`) and CAD (`DESIGN-cad-solid-modeling.md`) app-scale
showcases, this leans on one structural finding: **almost none of it is new language work.** A notebook is
*ordinary Cadenza programs*, run by the *existing* browser compile+run pipeline (cdz-wasm + jco + the
run-worker), with their results rendered by the *same* value→display path the calculator already uses. The
genuinely new work is narrow and lives at the **edges**: (1) a document model that interleaves markdown
prose with runnable code cells; (2) cell-output *renderers* keyed on a value's shape (tables, graphs,
formulas); (3) the reactive **widget→recompute** loop, which — this is the key finding — needs **no new
language feature** because "a program takes a runtime input" is expressible today as re-compiling the cell
with the widget's current value spliced in as a `let` binding (§5). We report any gap we hit rather than
work around it.

---

## §0 — Vision, and what it showcases

A `.cdznb` notebook is a markdown document. Prose renders as markdown (the guide already does this via
`Prose.tsx`). Fenced ` ```cadenza ` blocks become editable, runnable **code cells** whose result renders
inline as a typed output — a value, a table, a chart, a formula. A `widget` directive renders a
slider/number/dropdown/checkbox the reader drags; the cell(s) that read it **recompute and re-render
reactively**. The flagship demo: a compound-interest notebook where a "principal" slider and a "rate"
slider drive a table of yearly balances and a line chart that redraw as you drag — all computed by real
Cadenza, in-browser, off the main thread.

```markdown
# Compound interest

Adjust the inputs and watch the schedule recompute.

~~~cadenza widget
principal : Float64 = slider(1000, 100000, step: 1000, default: 10000)
rate      : Float64 = slider(0.01, 0.15, step: 0.005, default: 0.05)
years     : Int64   = slider(1, 30, step: 1, default: 10)
~~~

~~~cadenza chart:line
def (schedule) = ...uses principal, rate, years...   -- returns List of (year, balance)
~~~
```

What it showcases: the *same* language and runtime that power the CLI, the calculator, and CAD now drive a
live, interactive document — the "look what you can build with Cadenza" surface #468 asks for.

---

## §1 — Where it lives: a `/notebook` guide route (reuse, don't rebuild)

Confirmed direction (to be ratified with v-guide-infra by `note`): the notebook is **another lazy guide
route**, `/notebook`, exactly like `/calculator` and `/cad` (`guide/src/main.tsx:14-19` code-splits each
heavy full-screen route behind `lazy()`). It reuses, verbatim:

- **The compile worker** (`guide/src/compiler/client.ts`) — `compile`, `replEval`, `definedNames`,
  `exportTypes`, `renderValueInSurface`. Off-main-thread via Comlink.
- **The run worker** (`guide/src/runner/client.ts`) — `run(component, surface, display)` with the 5 s
  watchdog + stale-runtime guard. Each cell run goes through this untouched.
- **The editor** (`guide/src/editor/`, `useCadenzaEditor`) — CodeMirror with the Cadenza language, for
  editable cells.
- **Markdown prose rendering** (`guide/src/components/Prose.tsx`) — for the non-code cells.
- **Value→display rendering** — the calculator's `renderSyntaxDisplay` path (a rational bare, a quantity in
  its concise surface) for the default scalar/value output.

New code this vertical owns, all code-split behind `/notebook`:

- `guide/src/notebook/` — the route shell, the cell model + document parser, the cell components, the
  output renderers, and the reactive recompute engine.
- Heavy deps (a charting lib — see §4) code-split behind this route, never touching first paint (the CAD
  precedent: three.js/manifold-3d live only under `/cad`).

**Territory split to confirm with v-guide-infra (owns the guide site/route/deploy/run-workers/`check:visual`)
and v-guide (owns markdown rendering + may want notebooks AS guide content):** we own `guide/src/notebook/*`
+ the `/notebook` route entry + the chart dep; they own the shared run-worker/compile-worker/Prose/deploy
plumbing we consume unchanged. A notebook may later become a *content type* the guide embeds (a chapter that
IS a live notebook) — designed-for but out of scope for the first increments.

---

## §2 — The document + cell model (Increment 1)

A notebook document is an **ordered list of cells** parsed from a single markdown string:

- **Prose cell** — any markdown between code fences. Rendered read-only by `Prose.tsx`.
- **Code cell** — a ` ```cadenza ` (or `~~~cadenza`) fenced block, with an optional **directive** on the
  fence info string: ` ```cadenza chart:line `, ` ```cadenza table `, ` ```cadenza widget `,
  ` ```cadenza hidden ` (a setup cell: runs for its scope/defs but shows NEITHER source NOR its success
  output — only a failure is surfaced, so a broken hidden cell isn't invisible). No directive ⇒ auto-render
  by value shape (§3).

Parsing is a plain markdown-fence scan (no language work): split on fence boundaries, tag each code fence
with its info string, everything else is prose. This mirrors how the guide's chapter content already
carries runnable snippets — we generalize it to a whole-document ordered model. The parser is pure
(no worker imports) so it's unit-testable under `node --test`, following `calculator/classify.ts`.

**Cell-scope semantics — see the fork in §6 (D1).** The default this doc proposes and will build unless the
operator rules otherwise: **sequential/accumulating scope** (Jupyter's model) — cell *N* sees the top-level
definitions of all cells 1..*N*−1, like the calculator's REPL buffer accumulates assignments. This is the
natural notebook mental model and maps directly onto the existing `replEval(buffer, expr, surface)`: the
"buffer" is the concatenation of all prior cells' definitions, and the cell's own body is the expr. §6-D1
lays out the alternative (isolated cells) and why sequential is the recommended default.

---

## §3 — Run-and-render + rich output types (Increment 2 + 3)

Each code cell compiles+runs through the existing pipeline and its **result renders by the cell's directive,
else auto-detected from the value's shape/type** (`exportTypes` gives us the solved type of the exported
value — the same signal the calculator uses to format a whole-number Float with its `.0`):

| Directive / shape                       | Renderer                                                        |
|-----------------------------------------|-----------------------------------------------------------------|
| (none) scalar / small value             | `renderSyntaxDisplay` text — the calculator's value view        |
| `table` or a `List` of records/tuples   | HTML `<table>` — columns from record fields / tuple positions   |
| `chart:line` / `chart:bar` / `chart:scatter` | a charting canvas fed `List (x, y)` (or labelled series)   |
| `formula` / a math-shaped value         | math rendering (KaTeX-class) of the expression/result           |
| (none) compound value                   | the canonical value text (playground's `renderSyntax`)          |

The renderer contract is small and value-shape-keyed: `render(outcome: RunOutcome, exportType: string) →
ReactNode`. Table/chart renderers parse the **rendered s-expr value** (the machine form, like CAD's
`meshFromSolid` parses the rendered `Solid` s-expr — a canonical form, not the display surface) into rows /
points, then hand off to the HTML table or chart component. This keeps the language↔JS boundary at one
well-tested parse, reused across renderers.

Increment 2 = value + table renderers (no new deps). Increment 3 = the chart renderer (one lazy charting
dep — see §4) + formula rendering. Each renderer ships with a runnable example (the guide's
run-every-example discipline; verified under `check:visual`).

---

## §4 — The chart dependency (Increment 3)

Following CAD's lazy-heavy-dep precedent, a single charting library (candidate: a small, dependency-light
lib — `uPlot` for line/scatter/bar, or a thin `<svg>` renderer we own for the first cut to avoid *any* new
dep) is **code-split behind `/notebook`**. We prefer the smallest thing that renders line/bar/scatter from a
`List (x, y)`; if a hand-rolled SVG renderer covers the flagship demos, we ship that first and add a lib
only when a demo needs it (zoom, many series). Decision recorded in §6-D3 (low-stakes, owner's call unless
the operator has a preference).

---

## §5 — THE NOVEL CORE: interactive widgets + runtime-input recompute (Increment 4)

The operator's headline ask: *"render widgets the user can interact with so the programs need to be able to
take input values at runtime and recalculate their output."* This is a reactive dataflow story.

**The key finding — no new language feature is required.** "A program takes a runtime input value" is
already expressible: a widget is a *named typed input with a current value*, and feeding it into a cell is
splicing a **`let name = <current-value> in ...`** (or a prepended `def name = <current-value>`) into the
cell's source before it goes through the existing `compile`+`run` path. A widget change = re-splice the new
literal + re-run the dependent cell(s) + re-render. Push-based, re-run on change. The value is a plain
literal (a Float64/Int64/Bool/String the widget's type dictates), so it compiles and runs on the *unchanged*
pipeline. **This is the same trick the calculator uses** — it wraps a REPL expression in a `let` chain over
the accumulated assignments (`calculator/engine.ts` → `replEval`); we generalize "accumulated assignments"
to "current widget values."

### The widget declaration surface

A ` ```cadenza widget ` cell declares typed inputs bound to controls. Proposed surface (pending §6-D2):

```
principal : Float64 = slider(1000, 100000, step: 1000, default: 10000)
rate      : Float64 = slider(0.01, 0.15, step: 0.005, default: 0.05)
label     : String  = text(default: "balance")
compound  : Bool     = checkbox(default: true)
mode      : String   = dropdown("annual", "monthly", default: "annual")
```

Each line is `name : Type = <control>(...)`. `slider`/`text`/`checkbox`/`dropdown` are **notebook
directives, not Cadenza functions** — parsed by the notebook, not the compiler (they never reach the
compile worker). The notebook renders the control, holds its current value in React state, and the
*binding* it contributes to downstream cells is `name = <current-value>` as a spliced definition. So a
downstream cell just writes `principal`, `rate`, … as ordinary in-scope names. §6-D2 is the fork on this
surface (a directive-parsed mini-DSL vs a first-class language `input`/parameter — the latter WOULD be a
language feature and is the thing to *report*, not assume).

### The recompute graph

A widget change must re-run the right cells. Model (pending §6-D1's scope ruling):

- Under **sequential scope** (proposed default): a cell *depends on* a widget if the widget's name is free
  in the cell's source, OR the cell is downstream of another cell that depends on it (transitive, via the
  accumulating buffer). On a widget change we re-run the **owning widget's dependents in document order**,
  re-rendering each. A simple, correct first cut: recompute the dependent cell and every code cell *after*
  it (sequential scope means a later cell may have consumed an earlier one's now-changed binding). An
  optimization (only re-run cells whose free names actually changed) is a later increment — correctness
  first, minimal recompute second.
- Debounced (a slider drag fires many events) + serialized through the single run-worker (`run()` already
  guards one run at a time); a newer widget value supersedes an in-flight stale run.

This is where the design attention goes, and it interacts with §2's scope decision — hence D1 is the
first fork.

### The reported language question (a finding, not a workaround)

The directive-DSL approach (widgets parsed by the notebook, spliced as `let`) ships **today** with no
language change — that's the recommendation for the first shippable notebook. BUT it means widget inputs
aren't a first-class *language* concept: a `.cdz` file run by the CLI has no notion of "a runtime input the
caller supplies." If the operator wants notebooks to be *portable programs* (the same `.cdznb` runnable
headless with inputs supplied on a CLI/host boundary), that needs a real language feature — a typed
top-level `input name : T`  the host binds at runtime (distinct from a `def`). **We surface this as a design
question (D2), not a workaround**: the notebook works without it, but a first-class runtime-input construct
is the "right" long-horizon answer and is exactly the kind of language gap #468 says to report.

---

## §6 — Forks to route to the concierge → operator (before code)

Three genuine decisions, stated with a concrete recommended default so the operator can rule in one line.

**⚑ D1 — Cell-scope semantics: sequential (accumulating) vs isolated.**
- *Sequential (RECOMMENDED):* cell *N* sees all prior cells' top-level defs — Jupyter's model, maps onto
  the existing `replEval` buffer, matches reader intuition ("the notebook runs top to bottom"). Cost: the
  recompute graph is order-sensitive (a widget change re-runs downstream cells).
- *Isolated:* each cell is a standalone program; sharing is explicit (an `import`/reference). Simpler
  recompute (only the owning cell re-runs) but a worse notebook UX (can't build up state across prose).
- **Recommendation: sequential.** Route because it's the load-bearing semantics decision and the operator
  had strong views on CAD's scope model.

**⚑ D2 — Widget-input declaration: notebook-directive DSL vs a first-class language `input`.**
- *Directive DSL (RECOMMENDED for the first ship):* `name : T = slider(...)` parsed by the notebook, spliced
  as a `let`. Zero language change, ships now, but widgets aren't a portable language concept.
- *First-class `input name : T`:* a real language feature — a typed runtime input the host binds. Portable
  (a `.cdznb` runs headless), the "right" long-horizon answer, but it's genuine compiler work (parse, type,
  a host-binding ABI) and blocks the notebook on a language increment.
- **Recommendation: ship the DSL now, and FILE the first-class `input` as a reported language finding** (a
  `.sexp`/backlog item) so the operator can decide whether/when to elevate it. This honors "report gaps,
  don't work around" while not blocking the showcase.

**⚑ D3 — Chart rendering: hand-rolled SVG first vs adopt a charting lib now.** Low-stakes, owner's call
unless the operator prefers a specific lib. Recommendation: hand-rolled SVG for line/bar/scatter in the
first chart increment (zero new dep, code-split anyway), adopt a lib only when a demo needs interaction the
SVG can't cheaply do.

---

## §7 — Increment plan (one gated slice per tick)

- **Inc 0 — DESIGN (this doc).** Route D1/D2/D3 to the concierge. `note` v-guide-infra + v-guide to confirm
  the `/notebook` route + territory split. ← *current*
- **Inc 1 — Document + cell model.** The pure markdown-fence parser (`notebook/parseDocument.ts`) + cell
  types + unit tests (`node --test`, mirroring `classify.test.ts`). No route UI yet — just the model + tests.
- **Inc 2 — Route shell + run-and-render (value + table).** `/notebook` lazy route, cells render prose +
  editable code, each code cell runs via the existing worker and renders its value / a table. One runnable
  example notebook, `check:visual`-verified.
- **Inc 3 — Chart + formula renderers.** The chart renderer (§4) + formula rendering, shape-keyed.
- **Inc 4 — Widgets + reactive recompute (THE NOVEL CORE).** Widget cells, the current-value→`let`-splice
  binding, the recompute graph, debounce + serialize. The compound-interest flagship demo, `check:visual`.
- **Inc 5+ — Hardening + coverage.** Recompute minimization (only re-run cells whose free names changed),
  more widget kinds, persistence/share-URL, and (if D2 elevates it) wiring the first-class `input` construct.

## §8 — Gate coverage (how this vertical protects itself)
- Pure-model unit tests (`node --test`) for `parseDocument` + the widget-line parser + the recompute
  dependency computation — these fail if the cell model or reactive graph regresses.
- Every example notebook must compile+run every cell (the guide's run-every-example discipline).
- `check:visual` headless-browser smoke over the `/notebook` route (v-guide-infra's harness): the flagship
  notebook loads, a cell renders a value/table/chart, and a widget drag recomputes an output.
- The gate for this vertical is the **guide's own build/checks** (`cd guide && npm run build` + the notebook
  unit tests + `check:visual`), NOT the rcdzc corpus (per the vertical role: a `guide` vertical's gate is the
  site build/smoke).

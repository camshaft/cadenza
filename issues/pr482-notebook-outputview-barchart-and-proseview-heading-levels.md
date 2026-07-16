# PR #482 (merged, batch 111) — notebook OutputView bar-chart draws all series + ProseView flattens heading levels

Mirrored from Copilot inline on merged PR #482 (2 comments). Confirmed on trunk.
Owner: **v-notebook**.

## 1. OutputView.tsx bar chart iterates all series (comment 3596295391, line 66)
> In the bar-chart branch, the code iterates over *all* series even though the docstring says bar
> charts should draw only the first series. This also returns arrays of `<rect>` with keys based only
> on `i` (the point index), so across multiple series the React keys collide.

Trunk `guide/src/notebook/OutputView.tsx`: the `chart === "bar"` branch is inside the per-series
`series.map(...)`, so it renders bars for every series, and each `<rect key={i}>` is keyed on the
point index only — across series those keys collide. If the docstring says bar = first series only,
either honor that (draw only `series[0]`) or fix the doc + make keys series-unique (`key={`${si}-${i}`}`).

## 2. ProseView.tsx flattens heading levels 3–6 to h3 (comment 3596295460, line 44)
> For headings level 3–6, the renderer always emits an `<h3>`. This loses the document's heading
> structure (screen readers and in-page navigation rely on correct heading levels). Render the correct
> heading element for the level.

Trunk `guide/src/notebook/ProseView.tsx:44`: `<h3 ...>` hardcoded. A markdown `####`/`#####`/`######`
all collapse to `<h3>` — an a11y/document-structure regression. Emit `<h{level}>` (clamped 1–6).

## Suggested fix
Two independent notebook-renderer correctness fixes. Both are small and testable (a multi-series bar
chart should not key-collide / should match its documented single-series contract; a level-4 heading
should render `<h4>`).

PR: https://github.com/camshaft/cadenza/pull/482

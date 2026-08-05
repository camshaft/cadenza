# PR #2146 review — guide/scripts/check-examples.mjs (v-guide) — OPEN — test-precision [VERIFIED, LOW-MED]

https://github.com/camshaft/cadenza/pull/2146 (guide capstone chapter "Writing a reducer" — agent
harness). Copilot 1 inline on the example-checker's new ML-authored toggle gate.

## the new `authoredIn === "ml"` s-expr-toggle gate compiles raw `render_syntax(mlProgram,"ml","sexpr")` directly, but the app's toggle pipeline (and this harness's OWN mirror path) is `wrapModule → render_syntax → stripModule → wrapModule` — skipping strip+rewrap means the gate can't catch bugs in the scaffolding the reader actually exercises when toggling ML-authored snippets (Copilot, check-examples.mjs:635) — test-precision [VERIFIED, LOW-MED]
> In the `authoredIn === "ml"` path, the s-expr toggle pass currently compiles the raw
> `render_syntax(mlProgram, "ml", "sexpr")` output directly. The app's toggle/render pipeline for wrapped
> snippets is `wrapModule` → `render_syntax` → `stripModule` (for display) → `wrapModule` again before
> compile/run. Skipping `stripModule`+rewrap means this gate can miss bugs in the same scaffolding logic
> the reader actually uses when toggling ML-authored snippets.

VERIFIED by the harness's OWN mirror path (asymmetry, not just an app claim):
- DEFAULT path (sexpr-authored, toggles to ML), check-examples.mjs:624: `wrapModule(renderToMl(ex.snippet),
  "ml")`, where `renderToMl` (line 89) = `stripModule(render_syntax(wrapModule(snippet,"sexpr"),"sexpr",
  "ml"),"ml")`. Full pipeline = **wrap → render → strip → wrap** before compile.
- NEW ml-authored path (#2146 diff:28,34): `mlProgram = wrapModule(ex.snippet,"ml")`; then the toggle pass
  does `sexprProgram = render_syntax(mlProgram,"ml","sexpr")` and compiles THAT directly (diff:34-35). =
  **wrap → render** only.

So the new path drops the `stripModule` + rewrap that its mirror default path performs. The reader's actual
toggle re-renders the wrapped ML to s-expr, STRIPS it back to the bare displayed snippet, then REWRAPS to
compile/run — a bug in stripModule/wrapModule for the s-expr surface on ML-authored content would slip past
this gate (it compiles the un-stripped render_syntax output, a shape the reader never runs). This is exactly
the class of scaffolding bug the surrounding comments (lines 732-734) say this arc has repeatedly hit
(dropping a `main`, mangling `(do …)`, list/tuple heads). LOW-MED / test-precision — guide-gate only, no
shipped-code bug, but the gate is less faithful than its mirror. Fix per Copilot + matching line 624: run
the s-expr render through `stripModule(_, "sexpr")` then `wrapModule(_, "sexpr")` before `checkProgram`
(i.e. mirror `renderToMl`'s strip + the caller's rewrap, in the ML→s-expr direction). Then both toggle
directions exercise the identical strip+rewrap the reader hits. PR OPEN → foldable pre-merge. v-guide
executes guide content (github-liaison directs). Copilot bot is reliable; this one is well-founded and
self-confirmed by the mirror path.

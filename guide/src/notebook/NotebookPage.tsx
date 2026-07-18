/// The `/notebook` route — a Jupyter-like notebook over the real Cadenza language (GH #468). A markdown
/// document interleaves prose with runnable ```cadenza code cells; each cell compiles+runs in-browser
/// (reusing the guide's compile + run workers, exactly like /calculator and /cad), and its result renders
/// as a typed output (value / table / chart / formula). A ```cadenza widget cell renders interactive
/// controls (slider/number/text/checkbox/dropdown) whose values are spliced into downstream cells as
/// `def name = <value>` — so dragging a widget RECOMPUTES the dependent cells reactively (the novel core).
///
/// The heavy work lives in tested pure modules (parseDocument / assembleForRun / recomputePlan /
/// renderOutput / parse*). This component is the orchestration + React state: a SERIALIZED run queue
/// (the run worker is one-at-a-time — v-guide-infra's constraint), and a debounced widget→recompute loop.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { replEval, renderSyntax } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import type { Surface } from "../compiler/worker.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { SyntaxToggle } from "../syntax/SyntaxToggle.tsx";
import { parseDocument, assignIds, setCellSource, setProseSource, serializeDocument, renderDocToSurface, insertCell, removeCell, moveCell, type Cell } from "./parseDocument.ts";
import { encodeNbShareUrl, decodeNbShare } from "./nbShare.ts";
import { resolveExampleParam, writeExampleParam } from "../components/exampleParam.ts";
import { parseWidgets, type Widget } from "./parseWidgets.ts";
import { assembleForRun, type WidgetValues } from "./assembleForRun.ts";
import { recomputePlan, initialRunOrder } from "./recomputePlan.ts";
import { renderOutput, type CellOutput, type RunOutcome } from "./renderOutput.ts";
import { cellIde } from "./cellIde.ts";
import { ProseView } from "./ProseView.tsx";
import { OutputView } from "./OutputView.tsx";
import { WidgetControls } from "./WidgetControls.tsx";
import { EXAMPLES, DEFAULT_EXAMPLE } from "./examples.ts";
import { LazyCodeEditor } from "../editor/LazyCodeEditor.tsx";

/// The notebook honors the guide's GLOBAL surface toggle (like /calculator, /playground, /cad): the reader
/// edits cells in whichever surface is selected, and the pick STICKS (SyntaxContext persists it to
/// localStorage + the URL). The examples are authored s-expr, so on a surface change the whole document's
/// code cells are RE-RENDERED through the target surface (`renderDocToSurface`, v-notebook's doc-model half,
/// using the compiler's `renderSyntax`) — flipping the surface without re-rendering would feed s-expr cells
/// to the ML compiler (the /cad-class "expected a name" bug). The RUN path compiles each cell in the LIVE
/// surface (`assembleForRun(...,from)` + `replEval(...,from)`) and renders the OUTPUT value in s-expr
/// (`runComponent(...,"sexpr")`, the canonical form the output parser reads) — so run/lint stay correct in
/// either surface.

/// The notebook is IN-PLACE PER-CELL editable (operator ruling — Jupyter-style): each code cell is its
/// own Cadenza editor, live-linted in its REAL sequential scope via `cellIde` (`./cellIde.ts`, which
/// composes `assembleCell` + widget bindings → a `prepare` that maps diagnostics back onto the cell).
/// There is NO whole-doc "Edit source" editor — editing the whole markdown+code blob as one Cadenza
/// program was the source of the all-red squiggles (operator IDE #13); per-cell editing scopes the
/// language service correctly by construction. A cell edit round-trips via `setCellSource` →
/// `serializeDocument` → the doc, driving the existing debounce → re-parse → re-run.

/// The notebook is a RATIONAL-mode app (operator-directed, app-level like the calculator): a bare numeric
/// literal — integer OR float — grounds to Rational, so cells compute EXACTLY (right for scientific use).
/// This is `replEval`'s `exact` flag (C6's default-fraction pragma), NOT a language-wide default — it's the
/// notebook's app-level choice, the same knob /calculator and /cad select.
const NOTEBOOK_EXACT = true;

/// Per-code-cell run state, keyed by the cell's index in the parsed cell list.
type CellState = { phase: "idle" } | { phase: "running" } | { phase: "done"; output: CellOutput };

export default function NotebookPage() {
  const { surface, setSurface } = useSyntax();
  // A SHARED link (`#nb/…` in the URL hash, operator #7184) reconstructs a specific notebook: decode it
  // ONCE at first render (synchronous, no effect race). If present, its `doc` seeds the document and its
  // surface pins `docSurface` (the shared doc is already in that surface — don't re-render it). Null (the
  // common case) → the normal STARTER. Cell ids aren't shared; assignIds re-stamps on parse like any load.
  const nbSharedRef = useRef(typeof window !== "undefined" ? decodeNbShare(window.location.hash) : null);
  const nbShared = nbSharedRef.current;
  // A `?example=<slug>` deep-link (operator: per-example nav) opens /notebook with THAT notebook selected.
  // Share hash WINS (a shared doc shouldn't be clobbered by a stale example id); else the deep-link slug
  // (when known) selects it, else the default. The examples are authored in s-expr; the surface effect
  // re-renders to the active surface on mount (same as a normal load), so seeding the raw markdown is right.
  const initialSlug = nbShared ? DEFAULT_EXAMPLE.slug : resolveExampleParam(EXAMPLES.map((e) => e.slug), DEFAULT_EXAMPLE.slug);
  const initialMarkdown = (EXAMPLES.find((e) => e.slug === initialSlug) ?? DEFAULT_EXAMPLE).markdown;
  // The notebook document. `doc` is the markdown source of truth (edited PER CELL — a cell editor commits
  // its change up via `onCellEdit` → `setCellSource` → `serializeDocument`); `committedDoc` is a DEBOUNCED
  // copy that drives parsing + re-running, so typing in a cell doesn't re-parse + thrash the run worker
  // (or flicker every output to "not run") on each keystroke. They coincide except during an active edit.
  const [doc, setDoc] = useState(nbShared?.doc ?? initialMarkdown);
  const [committedDoc, setCommittedDoc] = useState(nbShared?.doc ?? initialMarkdown);
  // The loaded example's slug (drives the example-picker's selection). Switching examples REPLACES the
  // whole document (both the live + committed copy so the re-parse/re-run fires immediately, not after the
  // edit-debounce) and clears widget values so a stale value from the previous notebook can't linger.
  const [exampleSlug, setExampleSlug] = useState(initialSlug);
  const docDebounce = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (docDebounce.current) clearTimeout(docDebounce.current);
    docDebounce.current = setTimeout(() => setCommittedDoc(doc), 400);
    return () => {
      if (docDebounce.current) clearTimeout(docDebounce.current);
    };
  }, [doc]);

  // Honor the GLOBAL surface: the examples + STARTER are AUTHORED in s-expr, so whenever the live surface
  // isn't s-expr the document's code cells must be RE-RENDERED through it (`renderDocToSurface` — prose +
  // widget cells pass through). This fires on BOTH the initial mount (the persisted/default surface may be
  // ML — the global DEFAULT is `ml`, so a fresh visitor lands in ML and the authored s-expr cells MUST be
  // converted or they'd compile as ML → "expected a name") AND on every toggle.
  //
  // 🔑 `docSurface` is STATE, not a ref: it is the surface the current `committedDoc` cells are ACTUALLY in,
  // and EVERYTHING that consumes the cells (cellIde's linter, assembleForRun/replEval, the run path) reads
  // `docSurface` — NOT the raw `surface` toggle. The render is ASYNC, so between a toggle and the render
  // completing (or if one cell's render transiently rejects and keeps its old source) the displayed cells
  // lag the toggle; linting them against the raw (new) `surface` is a DISPLAY-vs-LINTER mismatch (the
  // "expected a name" squiggle v-guide-infra co-verified). Advancing `docSurface` only WHEN the rendered doc
  // is committed keeps the linter/runner in lockstep with what's actually displayed.
  // Seed `docSurface` to the shared link's surface when present (the shared doc is ALREADY in that surface,
  // so the render effect below must NOT convert it); else s-expr (STARTER/examples are authored in s-expr).
  const [docSurface, setDocSurface] = useState<Surface>(nbShared?.s ?? "sexpr");
  // A shared `#nb/` link pins the surface to what it was authored in: on first mount, flip the global toggle
  // to the shared surface so the toggle UI + docSurface agree (docSurface is already the shared surface, so
  // the render effect sees docSurface===surface and won't re-convert the shared doc). Once only.
  const nbSharedApplied = useRef(false);
  useEffect(() => {
    if (nbShared && !nbSharedApplied.current) {
      nbSharedApplied.current = true;
      if (nbShared.s !== surface) setSurface(nbShared.s);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // A monotonic token for ASYNC doc re-renders (surface toggle + example switch). `renderDocToSurface` is
  // async, so a render started earlier can resolve LATER and clobber newer doc state (last-write-wins races,
  // PR #556). Every async doc-render bumps this token + captures its value; on resolution it commits ONLY if
  // still current. Shared by the toggle effect and `onSelectExample` so a toggle mid-example-switch (or vice
  // versa) also can't stomp the newer one.
  const docRenderToken = useRef(0);
  useEffect(() => {
    if (docSurface === surface) return;
    const from = docSurface;
    const token = ++docRenderToken.current;
    let cancelled = false;
    void renderDocToSurface(committedDoc, from, surface, renderSyntax).then((rendered) => {
      // Drop a stale render: the effect re-ran (a newer commit/toggle) OR another async doc-render superseded
      // this one. Guarding on BOTH `cancelled` (this effect instance) and the shared token (any newer render).
      if (cancelled || docRenderToken.current !== token) return;
      setDoc(rendered);
      setCommittedDoc(rendered);
      setDocSurface(surface); // advance ONLY after the render is committed — keeps lint/run in lockstep
    });
    return () => {
      cancelled = true;
    };
    // Depend on committedDoc + docSurface too: if the doc changes (an edit / debounce commit) mid-conversion,
    // the effect re-runs → cleanup cancels the in-flight render → the LATEST doc is what gets converted,
    // rather than a stale render resolving and overwriting newer edits (PR #556 race #1).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [surface, committedDoc, docSurface]);
  // Cells carry a stable `id` (via `assignIds`) so the stacked per-cell editor list keys by identity — an
  // edit keeps a cell's editor mounted (focus/cursor preserved) rather than remounting on re-parse.
  const cells = useMemo<Cell[]>(() => assignIds(parseDocument(committedDoc)), [committedDoc]);

  // A per-cell source edit: rewrite that cell's source in the LIVE doc and re-serialize (the debounce
  // above then commits → re-parse → re-run). Parse the live `doc` (not `cells`, which lags at the
  // committed copy) so concurrent edits compose. Guarded to code cells (prose isn't edited here).
  const onCellEdit = useCallback((index: number, newSource: string) => {
    setDoc((prev) => {
      const current = parseDocument(prev);
      if (current[index]?.kind !== "code") return prev;
      return serializeDocument(setCellSource(current, index, newSource));
    });
  }, []);

  // A per-cell PROSE edit (operator UX #4): rewrite that prose cell's markdown in the live doc + re-serialize
  // (the debounce commits → re-parse → re-render). Mirrors `onCellEdit` but for prose cells (setProseSource).
  // Guarded to prose cells; parses the LIVE `doc` (not `cells`, which lags at the committed copy) so an edit
  // composes with concurrent edits. Prose isn't Cadenza, so there's no run/lint — just the doc round-trip.
  const onProseEdit = useCallback((index: number, newMarkdown: string) => {
    setDoc((prev) => {
      const current = parseDocument(prev);
      if (current[index]?.kind !== "prose") return prev;
      return serializeDocument(setProseSource(current, index, newMarkdown));
    });
  }, []);

  // MARKDOWN-STRUCTURE editing (operator #2): add / delete / reorder cells via v-notebook's pure doc-model
  // ops (insertCell/removeCell/moveCell — immutable + id-preserving). Unlike a cell EDIT (debounced), a
  // structure change is a deliberate action, so commit BOTH `doc` and `committedDoc` immediately (re-parse +
  // re-run now, no 400ms lag). Parse the LIVE `doc` so it composes with a concurrent in-flight edit. A new
  // cell is a blank code cell (the common "add a cell to compute something" case); assignIds re-stamps on
  // the next parse so the keyed editor list stays stable (surviving cells keep their ids — no remount).
  const applyStructure = useCallback((f: (cells: Cell[]) => Cell[]) => {
    const commit = (prev: string) => serializeDocument(f(parseDocument(prev)));
    setDoc((prev) => {
      const next = commit(prev);
      setCommittedDoc(next); // structure edits are deliberate — apply immediately, don't wait for the debounce
      return next;
    });
  }, []);
  const BLANK_CELL: Cell = { kind: "code", source: "", directive: { kind: "none" } };
  const onInsertBelow = useCallback((index: number) => applyStructure((cells) => insertCell(cells, index + 1, BLANK_CELL)), [applyStructure]);
  // Delete, but never empty the notebook (a zero-cell doc has no editor to recover from) — a no-op on the
  // last remaining cell.
  const onRemoveCell = useCallback((index: number) => applyStructure((cells) => (cells.length <= 1 ? cells : removeCell(cells, index))), [applyStructure]);
  const onMoveCell = useCallback((index: number, dir: -1 | 1) => applyStructure((cells) => moveCell(cells, index, index + dir)), [applyStructure]);

  // All widgets declared across every widget cell, and their live values.
  const widgets = useMemo<Widget[]>(
    () => cells.flatMap((c) => (c.kind === "code" && c.directive.kind === "widget" ? parseWidgets(c.source).widgets : [])),
    [cells],
  );
  const [values, setValues] = useState<WidgetValues>({});
  // Switch to another example notebook: replace the whole document (live AND committed, so the switch
  // re-parses + re-runs at once rather than waiting on the edit-debounce) and reset widget values (the new
  // notebook declares its own widgets; `setValues({})` lets the reconcile effect re-seed them from defaults).
  // The example markdown is AUTHORED s-expr — so if the live surface is ML, render it through first (and
  // reset `docSurface` to the authored s-expr baseline either way, so the surface-effect stays consistent).
  const onSelectExample = useCallback(
    (slug: string) => {
      const example = EXAMPLES.find((e) => e.slug === slug);
      if (!example) return;
      setExampleSlug(slug);
      writeExampleParam(slug); // reflect the selection in the URL (?example=…) — a copy-shareable deep-link
      setValues({});
      // Bump the doc-render token up-front so ANY in-flight async render (from a prior toggle or example
      // switch) is invalidated — even the synchronous s-expr branch below must not be stomped by a stale
      // render resolving late (PR #556).
      const token = ++docRenderToken.current;
      if (surface === "sexpr") {
        // Authored s-expr goes in as-is; the doc IS s-expr now.
        setDoc(example.markdown);
        setCommittedDoc(example.markdown);
        setDocSurface("sexpr");
      } else {
        // Render the authored s-expr example → the live surface; advance docSurface only when committed.
        // Token-guard the async render: selecting example A then B must not let A's later-resolving render
        // overwrite B (PR #556 race #2). Commit only if `token` is still current.
        void renderDocToSurface(example.markdown, "sexpr", surface, renderSyntax).then((rendered) => {
          if (docRenderToken.current !== token) return; // a newer selection/toggle superseded this render
          setDoc(rendered);
          setCommittedDoc(rendered);
          setDocSurface(surface);
        });
      }
    },
    [surface],
  );

  // SHARE (operator #7184): copy an `#nb/…` URL that reconstructs the current notebook — its serialized
  // document (v-notebook's canonical form) + the surface its cells are in — so a shared link opens the same
  // notebook. Uses `doc` (the live source of truth) so an in-progress edit is included. A brief "Copied!"
  // confirms; no-ops gracefully if the clipboard API is unavailable (non-secure context).
  const [shareCopied, setShareCopied] = useState(false);
  const onShare = useCallback(() => {
    const url = encodeNbShareUrl({ s: docSurface, doc });
    void navigator.clipboard?.writeText(url).then(
      () => {
        setShareCopied(true);
        setTimeout(() => setShareCopied(false), 1500);
      },
      () => {
        /* clipboard denied — keep the UI stable */
      },
    );
  }, [docSurface, doc]);

  // A ref mirror of `values`, so the debounced widget-change handler can read the LATEST committed values
  // to build the recompute buffer WITHOUT running a side effect inside a setState updater (that updater
  // can be double-invoked under StrictMode/batching, which would enqueue duplicate runs).
  const valuesRef = useRef<WidgetValues>(values);
  useEffect(() => { valuesRef.current = values; }, [values]);
  // Reconcile widget values with the current widget set (on doc/widget change): seed a missing widget
  // with its default AND prune a value whose widget no longer exists (a doc edit that removed/renamed it),
  // so a stale value can't linger and be picked up by a cell that references the old name.
  useEffect(() => {
    setValues((prev) => {
      const names = new Set(widgets.map((w) => w.name));
      // NULL-prototype object + hasOwnProperty/Object.keys (not `in`/`for...in`): a widget may legally be
      // named `toString`/`__proto__`/`constructor` (IDENT_RE allows them), and a plain `{}` + prototype-
      // chain checks would misreconcile those (spurious changed / prototype value) — PR #510.
      const next: WidgetValues = Object.create(null);
      let changed = false;
      for (const w of widgets) {
        const had = Object.prototype.hasOwnProperty.call(prev, w.name);
        next[w.name] = had ? prev[w.name] : w.default;
        if (!had) changed = true;
      }
      // A previously-held value whose widget is gone → dropped (changed if `prev` had an extra key).
      for (const k of Object.keys(prev)) if (!names.has(k)) changed = true;
      return changed ? next : prev;
    });
  }, [widgets]);

  const [states, setStates] = useState<Record<number, CellState>>({});

  // A SERIALIZED run queue: the run worker is one-at-a-time (v-guide-infra), so we chain runs on a single
  // promise. A newer enqueue supersedes nothing — each planned cell runs in order. `runToken` lets a
  // fresh recompute (or doc edit) abandon stale in-flight renders.
  const runChain = useRef<Promise<void>>(Promise.resolve());
  const runToken = useRef(0);

  const runCells = useCallback(
    (indices: number[], vals: WidgetValues, from: Surface, token: number) => {
      // Chain onto the prior run (serialized worker). `.catch` resets a poisoned chain: if a run callback
      // ever rejected, a bare `.then` on the rejected promise would silently skip EVERY future run — so
      // swallow any rejection to keep `runChain.current` a resolved promise the next enqueue can build on.
      runChain.current = runChain.current.then(async () => {
        for (const i of indices) {
          if (runToken.current !== token) return; // superseded — stop the stale chain
          // Mark THIS cell running only when the chain reaches it — NOT all cells up-front. Up-front
          // marking left a cell stuck at "running…" forever if the run was superseded before reaching it
          // (a partial widget-recompute plan re-marks only its own cells, orphaning the rest).
          setStates((s) => ({ ...s, [i]: { phase: "running" } }));
          let output: CellOutput;
          try {
            // assembleForRun is inside the try so a bad-shape assembly error renders as a cell error
            // rather than rejecting the chain callback (which would poison all subsequent runs).
            const { buffer, entry } = assembleForRun(cells, i, widgets, vals, from);
            const compiled = await replEval(buffer, entry, from, NOTEBOOK_EXACT);
            if (!compiled.component) {
              const d = compiled.diagnostics.find((x) => x.error) ?? compiled.diagnostics[0];
              output = { render: "error", message: d ? `${d.code} ${d.message}`.trim() : "compile declined" };
            } else {
              // Run + render in s-expr (the canonical machine form the extractors parse — the /cad lesson).
              const outcome = (await runComponent(compiled.component, "sexpr")) as RunOutcome;
              const cell = cells[i];
              output = renderOutput(cell.kind === "code" ? cell.directive : { kind: "none" }, outcome);
            }
          } catch (e) {
            output = { render: "error", message: e instanceof Error ? e.message : String(e) };
          }
          if (runToken.current !== token) return;
          setStates((s) => ({ ...s, [i]: { phase: "done", output } }));
        }
      }).catch(() => {
        // Defensive: the loop already catches per-cell errors, but never let an unexpected rejection
        // poison the chain — a rejected runChain.current would make every future `.then` a no-op.
      });
    },
    [cells, widgets],
  );

  // Initial + on-doc-change: run every code cell top-to-bottom.
  useEffect(() => {
    const token = ++runToken.current;
    setStates({});
    // Null-prototype merge (defaults ∪ live values) so `__proto__`/`toString`-named widgets are plain keys.
    const runValues: WidgetValues = Object.assign(Object.create(null), defaultsOf(widgets), values);
    // Run the cells in the surface they're ACTUALLY in (`docSurface`), not the raw toggle — the doc lags an
    // async re-render, and compiling a not-yet-rendered cell in the new surface would spuriously error.
    runCells(initialRunOrder(cells), runValues, docSurface, token);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cells, docSurface]);

  // A widget change: debounce (a slider drag fires many events), then recompute only the dependent cells.
  const debounce = useRef<ReturnType<typeof setTimeout> | null>(null);

  // On unmount (navigating away from /notebook), clear any pending debounced recompute AND bump runToken
  // so a run chain still in flight sees a stale token and stops calling setStates/setValues — otherwise a
  // slow compile/run finishing after unmount triggers a React post-unmount state-update warning + leaked
  // work (PR #483).
  useEffect(() => {
    return () => {
      if (debounce.current) clearTimeout(debounce.current);
      runToken.current++;
    };
  }, []);

  const onWidgetChange = useCallback(
    (name: string, value: number | boolean | string) => {
      // Commit the control's value immediately (the slider thumb tracks the drag). valuesRef keeps a live
      // mirror so the debounced recompute reads the latest committed values without a state read.
      setValues((v) => withValue(v, name, value));
      if (debounce.current) clearTimeout(debounce.current);
      debounce.current = setTimeout(() => {
        const token = ++runToken.current;
        // Build the recompute values from the live ref (+ this change) and kick the run OUTSIDE any state
        // updater — a side effect in a setState updater can double-fire under StrictMode/batching.
        const next = withValue(valuesRef.current, name, value);
        runCells(recomputePlan(cells, widgets, name, docSurface), next, docSurface, token);
      }, 150);
    },
    [cells, widgets, docSurface, runCells],
  );

  return (
    <div className="mx-auto min-h-screen max-w-4xl px-4 py-6">
      <div className="mb-4 flex items-baseline justify-between gap-3">
        <h1 className="text-lg font-bold text-slate-100 sm:text-xl">Cadenza Notebook</h1>
        {/* Mobile touch target: the controls get a 44px min-height below `sm`, compact at sm+. */}
        <div className="flex shrink-0 items-center gap-1 text-xs sm:gap-3">
          {/* The GLOBAL surface toggle (ML / s-expr) — the app routes (/notebook, /cad, …) render under
              RootLayout, which has no header nav, so the chapter Layout's toggle isn't here; surfacing it
              lets a reader switch + STICK the surface on the notebook too (operator UX: same toggle
              everywhere). On change, the surface-effect above re-renders the cells through the new surface. */}
          <SyntaxToggle />
          {/* Example picker — swap between the canonical notebooks (examples.ts). */}
          <label className="flex min-h-11 items-center gap-1 sm:min-h-0">
            <span className="sr-only">Example notebook</span>
            <select
              data-testid="notebook-example-picker"
              value={exampleSlug}
              onChange={(e) => onSelectExample(e.target.value)}
              className="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-slate-200 focus:border-cadenza-500 focus:outline-none"
            >
              {EXAMPLES.map((e) => (
                <option key={e.slug} value={e.slug}>
                  {e.title}
                </option>
              ))}
            </select>
          </label>
          {/* SHARE — copy a URL that reconstructs this notebook for another reader (operator #7184, same
              mechanism as the playground + /cad share). */}
          <button
            data-testid="notebook-share"
            onClick={onShare}
            title="Copy a shareable link to this notebook"
            className="flex min-h-11 items-center rounded px-2 text-cadenza-400 transition hover:text-cadenza-300 sm:min-h-0 sm:py-1"
          >
            {shareCopied ? "✓ Copied!" : "🔗 Share"}
          </button>
          <Link
            to="/playground"
            className="flex min-h-11 items-center px-2 text-cadenza-400 hover:text-cadenza-300 sm:min-h-0 sm:px-0"
          >
            Playground →
          </Link>
        </div>
      </div>

      <div className="space-y-2" data-testid="notebook">
        {cells.map((cell, i) => {
          const inner =
            cell.kind === "prose" ? (
              <ProseCellView markdown={cell.markdown} onEdit={(md) => onProseEdit(i, md)} />
            ) : cell.directive.kind === "widget" ? (
              (() => {
                const parsed = parseWidgets(cell.source);
                return <WidgetControls widgets={parsed.widgets} errors={parsed.errors} values={values} onChange={onWidgetChange} />;
              })()
            ) : (
              <CodeCellView
                source={cell.source}
                hidden={cell.directive.kind === "hidden"}
                state={states[i]}
                ide={cellIde(cells, i, widgets, values, docSurface)}
                onEdit={(src) => onCellEdit(i, src)}
              />
            );
          // Wrap every cell with its structure controls (operator #2 — add/delete/reorder). The toolbar is
          // muted + reveals on hover/focus-within so it doesn't clutter the reading view, but stays keyboard-
          // reachable. Disable ↑ on the first cell / ↓ on the last (no wrap).
          return (
            <CellShell
              key={cell.id ?? i}
              index={i}
              isFirst={i === 0}
              isLast={i === cells.length - 1}
              onMoveUp={() => onMoveCell(i, -1)}
              onMoveDown={() => onMoveCell(i, 1)}
              onInsertBelow={() => onInsertBelow(i)}
              onRemove={() => onRemoveCell(i)}
            >
              {inner}
            </CellShell>
          );
        })}
        {/* Add a cell at the very END (the common "append a new cell" affordance) — insert below the last. */}
        <button
          data-testid="notebook-add-cell"
          onClick={() => onInsertBelow(cells.length - 1)}
          className="flex min-h-11 w-full items-center justify-center rounded border border-dashed border-slate-700 text-xs text-slate-500 transition hover:border-cadenza-600/50 hover:text-cadenza-300 sm:min-h-0 sm:py-2"
        >
          + Add cell
        </button>
      </div>
    </div>
  );
}

/// Wraps a notebook cell with its markdown-structure controls (operator #2): reorder (↑/↓), insert-below
/// (+), and delete (🗑). Owned by /notebook's UI (v-guide-infra); it calls the parent's handlers, which
/// drive v-notebook's pure insertCell/removeCell/moveCell doc-model ops. The toolbar is muted and only
/// prominent on hover / keyboard focus-within, so a reader's view stays clean but the controls are always
/// reachable. Delete is guarded when it would empty the notebook (keep at least one cell).
function CellShell({
  index,
  isFirst,
  isLast,
  onMoveUp,
  onMoveDown,
  onInsertBelow,
  onRemove,
  children,
}: {
  index: number;
  isFirst: boolean;
  isLast: boolean;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onInsertBelow: () => void;
  onRemove: () => void;
  children: React.ReactNode;
}) {
  const btn =
    "flex h-6 w-6 items-center justify-center rounded text-slate-500 transition hover:bg-slate-700/60 hover:text-slate-200 disabled:opacity-30 disabled:hover:bg-transparent";
  // Left GUTTER for the structure toolbar (NOT top-right): the cell's OWN controls — a prose cell's "Edit"
  // toggle, a code cell's run affordance — live at the top-right, so a top-right toolbar overlapped + stole
  // their clicks (the prose-Edit regression). A left gutter (negative margin into the page's left padding)
  // is collision-free. Muted, reveals on hover/focus-within; a vertical stack of the 4 ops.
  return (
    <div className="group relative rounded pl-8" data-testid={`notebook-cell-${index}`}>
      <div className="absolute left-0 top-1 z-10 flex flex-col items-center gap-0.5 rounded bg-slate-800/70 opacity-0 backdrop-blur transition group-hover:opacity-100 focus-within:opacity-100">
        <button data-testid={`cell-move-up-${index}`} onClick={onMoveUp} disabled={isFirst} title="Move cell up" className={btn}>↑</button>
        <button data-testid={`cell-move-down-${index}`} onClick={onMoveDown} disabled={isLast} title="Move cell down" className={btn}>↓</button>
        <button data-testid={`cell-insert-below-${index}`} onClick={onInsertBelow} title="Insert a cell below" className={btn}>+</button>
        <button data-testid={`cell-delete-${index}`} onClick={onRemove} title="Delete this cell" className={btn}>🗑</button>
      </div>
      {children}
    </div>
  );
}

function defaultsOf(widgets: Widget[]): WidgetValues {
  // Null-prototype: a widget named `__proto__`/`toString` must be an ordinary data key, not touch the
  // prototype chain (PR #510). Same reason as the reconcile effect.
  const v: WidgetValues = Object.create(null);
  for (const w of widgets) v[w.name] = w.default;
  return v;
}

/// A copy of `values` with one widget's value set, preserving the null-prototype (a plain `{...v}` spread
/// would create an Object.prototype-carrying object again, re-exposing the `__proto__`/`toString` footgun).
function withValue(values: WidgetValues, name: string, value: number | boolean | string): WidgetValues {
  return Object.assign(Object.create(null), values, { [name]: value });
}

/// A code cell: its source (unless hidden) + its computed output.
/// Whether a CellOutput is a FAILURE (trap/timeout/error) vs a normal success (value/table/chart/formula).
/// A hidden cell suppresses its successful output but MUST still surface a failure (a silently-broken
/// hidden setup cell would make downstream cells fail mysteriously).
function isFailure(output: CellOutput): boolean {
  return output.render === "trap" || output.render === "timeout" || output.render === "error";
}

/// The IdeConfig a code cell's editor gets (from `cellIde`) — Cadenza highlighting + squiggles + hover,
/// scoped to this cell in its sequential scope.
type NotebookIde = ReturnType<typeof cellIde>;

/// A prose cell (operator UX #4: editable prose, not just code). Renders the markdown READ-ONLY via
/// `ProseView` by default, with an "Edit" toggle that swaps to a PLAIN-text editor (`language="plain"` — no
/// Cadenza language/highlight/ide, since prose isn't Cadenza; v-guide-infra's editor config). Edits commit up
/// via `onEdit` → `setProseSource` → the debounced doc round-trip; "Done" returns to the rendered view. The
/// editor holds a LOCAL buffer seeded from `markdown`, re-synced if `markdown` changes from OUTSIDE (a fresh
/// load / example switch), guarded so our own edit round-trip doesn't clobber the live buffer (mirrors
/// `CodeCellView`). Keeping edit a per-cell toggle (not always-on) preserves the rendered reading view.
function ProseCellView({ markdown, onEdit }: { markdown: string; onEdit: (markdown: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(markdown);
  const lastEmitted = useRef(markdown);
  useEffect(() => {
    if (markdown !== lastEmitted.current) {
      setText(markdown);
      lastEmitted.current = markdown;
    }
  }, [markdown]);
  const onChange = useCallback(
    (next: string) => {
      setText(next);
      lastEmitted.current = next;
      onEdit(next);
    },
    [onEdit],
  );

  if (!editing) {
    return (
      <div className="group relative" data-testid="prose-cell">
        <ProseView markdown={markdown} />
        {/* Edit affordance — subtle until hover, so the reading view stays clean. */}
        <button
          type="button"
          data-testid="prose-edit-toggle"
          onClick={() => setEditing(true)}
          className="absolute right-0 top-0 rounded px-2 py-0.5 text-xs text-slate-500 opacity-0 hover:text-cadenza-400 group-hover:opacity-100"
        >
          Edit
        </button>
      </div>
    );
  }
  return (
    <div className="my-3 rounded-lg border border-slate-800 bg-slate-900/40" data-testid="prose-cell">
      <div className="flex items-center justify-between border-b border-slate-800 px-3 py-1">
        <span className="text-xs text-slate-500">Markdown</span>
        <button
          type="button"
          data-testid="prose-done"
          onClick={() => setEditing(false)}
          className="rounded px-2 py-0.5 text-xs text-cadenza-400 hover:text-cadenza-300"
        >
          Done
        </button>
      </div>
      {/* PLAIN-text editor (no Cadenza language/ide) — prose is markdown, not Cadenza (v-guide-infra config). */}
      <LazyCodeEditor value={text} onChange={onChange} language="plain" minHeight="4rem" />
    </div>
  );
}

/// An editable code cell: its own Cadenza editor (live per-cell diagnostics via `ide`) + its computed
/// output. The editor holds a LOCAL buffer seeded from `source` so typing is responsive; each change
/// commits UP via `onEdit` (the parent debounces the doc → re-parse → re-run). When `source` changes from
/// OUTSIDE (a fresh load / a programmatic edit), the local buffer re-syncs — guarded so a round-trip of our
/// own edit (edit → doc → re-parse → same source) doesn't clobber the live buffer.
function CodeCellView({
  source,
  hidden,
  state,
  ide,
  onEdit,
}: {
  source: string;
  hidden: boolean;
  state?: CellState;
  ide: NotebookIde;
  onEdit: (source: string) => void;
}) {
  const [text, setText] = useState(source);
  const lastEmitted = useRef(source);
  useEffect(() => {
    if (source !== lastEmitted.current) {
      setText(source);
      lastEmitted.current = source;
    }
  }, [source]);
  const onChange = useCallback(
    (next: string) => {
      setText(next);
      lastEmitted.current = next;
      onEdit(next);
    },
    [onEdit],
  );

  // A HIDDEN cell runs for its scope (defs used downstream) but shows NO source editor and NO success
  // output — it's a setup cell. Only a FAILURE is surfaced (so a broken hidden cell isn't invisible).
  if (hidden) {
    if (state?.phase === "done" && isFailure(state.output)) {
      return (
        <div className="my-3 rounded-lg border border-rose-900/50 bg-slate-900/40 px-3 py-2" data-testid="cell-output">
          <OutputView output={state.output} />
        </div>
      );
    }
    return null; // hidden + (idle | running | succeeded) → render nothing
  }

  return (
    <div className="my-3 rounded-lg border border-slate-800 bg-slate-900/40">
      {/* Per-cell editable Cadenza editor — live diagnostics/hover in this cell's sequential scope via
          `ide` (cellIde). Code-split behind React.lazy. */}
      <div className="overflow-hidden border-b border-slate-800">
        <LazyCodeEditor value={text} onChange={onChange} ide={ide} minHeight="2.5rem" />
      </div>
      <div className="px-3 py-2" data-testid="cell-output">
        {!state || state.phase === "idle" ? (
          <span className="text-xs text-slate-600">not run</span>
        ) : state.phase === "running" ? (
          <span className="text-xs text-slate-500">running…</span>
        ) : (
          <OutputView output={state.output} />
        )}
      </div>
    </div>
  );
}

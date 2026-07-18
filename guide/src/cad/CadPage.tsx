/// The `/cad` route — a live browser 3D preview of a Cadenza CAD model: edit a Solid-producing program,
/// Run it, and see the meshed result rotate in a three.js canvas. Mirrors /calculator's shape (an
/// editable program over the real language, executed in-browser), but the result is geometry instead of
/// a value. Part of the operator's "showcase every use-case as a working example" push.
///
/// THE SPLIT (confirmed with v-cad): this vertical owns the route + shell + the react-three-fiber canvas
/// + the 3 npm deps (three, @react-three/fiber, manifold-3d) — all code-split behind this lazy route so
/// they never touch the guide's first paint. v-cad owns `guide/src/cad/index.ts` (`meshFromSolid`: parse
/// a rendered Solid S-EXPR → manifold-3d CSG → triangle buffers) AND the preloaded CAD library
/// (`implementation/cad/src/exact.cdz`).
///
/// PRELOADED LIBRARY (operator P5, ruling A): the reader's buffer holds ONLY the model — a program that
/// builds and returns a `Solid`. The CAD vocabulary (`Solid`/`Vec3`/`v3r`/`lower`/…) is a real Cadenza
/// module (`exact.cdz`) link-merged at compile via `compileWithPreloaded` (no inline `type` defs). The
/// host AUTO-INJECTS the `import { Solid, v3r, lower } from "exact"` clause before compiling (ruling A),
/// so the buffer stays clean. The model returns `lower(model)`: `exact.cdz`'s `Solid` is GENERIC
/// (`Solid(a)`), and a generic recursive-sum value can't yet be host-rendered — `lower` maps it to the
/// monomorphic `SolidR` mirror the compiler CAN emit and the mesh driver parses (v-cad shipped both).
///
/// SURFACE: /cad respects the global surface toggle for EDITING (like /calculator + /playground) — a
/// per-surface starter, edited in whichever surface the reader has selected — but the compiled value is
/// always RUN + rendered in s-expr (`runComponent(component, "sexpr")`) before it reaches the driver.
/// `meshFromSolid` parses the RENDERED value as an s-expr `(: (Difference …) SolidR)`; an ML render uses
/// commas + backtick-rationals the s-expr parser can't read, so the driver consumes the canonical machine
/// form, not the display surface. Both surfaces render IDENTICALLY to
/// `(: (Difference (Cube (: (tuple 4/1 4/1 4/1) Vec3R)) (Sphere 5/2)) SolidR)` — the driver discards the
/// trailing type-name atom, so `SolidR`/`Vec3R` parse exactly like `Solid`/`Vec3` (v-cad-verified end to
/// end — 584 triangles), so the driver behaves the same whichever surface the reader edits in.

import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { compileWithPreloaded } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { SyntaxToggle } from "../syntax/SyntaxToggle.tsx";
import { meshFromSolid, type MeshResult } from "./index.ts";
import { MeshView } from "./MeshView.tsx";
import { wrapPrefixOf } from "../components/wrapModule.ts";
import { injectImport, CAD_LIB_NAME, CAD_LIB_FORMAT } from "./preloadModel.ts";
import { EXAMPLES, DEFAULT_EXAMPLE } from "./examples.ts";
import { DEFAULT_PARAMETRIC } from "./parametric.ts";
import { ParametricControls, fracOf, type Frac } from "./ParametricControls.tsx";
import type { Surface } from "../compiler/client.ts";
import { LazyCodeEditor } from "../editor/LazyCodeEditor.tsx";
// The CAD library sources, staged into the guide tree by `stage-wasm.mjs` (same pattern as runtime.wasm)
// and `?raw`-imported here as strings. PRELOADED via `compile_with_preloaded` (operator P5, ruling A) so a
// buffer holds only the model — the CAD vocab (`Solid`/`v3r`/`lower`/…) is link-merged. `exact` is the base
// geometry lib; `helpers` (box/cyl/hole-through/…) is the ergonomic surface the PARAMETRIC models import.
import EXACT_CDZ from "../wasm/cad/exact.cdz?raw";
import HELPERS_CDZ from "../wasm/cad/helpers.cdz?raw";

// /cad's IDE config is built INSIDE the component (it must read the LIVE edit surface) — see `cadIde`
// below. The program is a self-contained module (no wrapping), so the compiled text IS the editor text
// (prefix 0). This turns on the Cadenza lexical + semantic highlighting + squiggles/hover.
//
// ⚠ The linter surface MUST match the surface the BUFFER is written in. The buffer is seeded per the global
// toggle (`STARTER[surface]`), so a reader with the toggle on ML edits ML source — hardcoding the linter to
// "sexpr" (an earlier bug) compiled that ML buffer AS s-expr → every line a parse error → all-red squiggles
// (operator P-C). The s-expr requirement is for the DRIVER (meshFromSolid parses s-expr), which lives in the
// RUN path (`runComponent(component, "sexpr")` in `runModel`), NOT the linter — so it's kept separate: the
// IDE surface tracks the edit surface; the mesh path forces s-expr on the compiled value.

/// The starter models now live in `./examples.ts` (v-cad-authored `EXAMPLES`, each a `source: Record<Surface,
/// string>`): a bare model built against the PRELOADED CAD library (`Solid`/`v3r`/`lower` from `exact.cdz`) —
/// no inline `type` defs. Both the `import` AND the `@!default-fraction Rational` pragma are AUTO-INJECTED
/// (`injectImport`) — the reader's buffer is just the model (no import, no pragma line; the pragma grounds a
/// bare `n/d` to an exact Rational so `v3r(4/1,…)` type-checks). The model returns `lower(<Solid model>)` (the
/// generic `Solid` isn't host-renderable, so `lower` maps to the monomorphic `SolidR` the driver meshes). The
/// example-picker swaps `source[surface]`; `/cad` opens with `DEFAULT_EXAMPLE` (the cube-with-dent — the
/// historical starter). Both surfaces render to the same canonical s-expr `SolidR` value (v-cad-verified:
/// 584 triangles for the default).

type Status =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "meshed"; mesh: Extract<MeshResult, { ok: true }> }
  | { phase: "error"; message: string };

export default function CadPage() {
  const { surface } = useSyntax();
  // The loaded example (drives the picker). Its `source[surface]` seeds the editor; switching examples or
  // toggling the surface re-seeds from `example.source[newSurface]` (v-cad ships every example in BOTH
  // surfaces, so a toggle is a clean re-seed — source can't be reinterpreted across surfaces, same as /calc).
  const [exampleSlug, setExampleSlug] = useState(DEFAULT_EXAMPLE.slug);
  const example = EXAMPLES.find((e) => e.slug === exampleSlug) ?? DEFAULT_EXAMPLE;
  const [source, setSource] = useState(() => DEFAULT_EXAMPLE.source[surface] ?? DEFAULT_EXAMPLE.source.ml);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  const runningRef = useRef(false);

  // MODE: "edit" = the reader edits a model buffer (the example-picker + editor above); "parametric" = a
  // PARAMETRIC showcase driven by sliders (operator directive — the @param models). Distinct affordance
  // (v-cad Q3) since it's a different interaction (drag sliders vs edit source). Parametric compiles the
  // fixed showcase model against exact+helpers + supplies each @param's {num,den} as a host-response
  // (run-worker step-2 wiring) → recompute+re-mesh — an EXACT fractional dim (7/2) is carried live.
  const [mode, setMode] = useState<"edit" | "parametric">("edit");
  const paramModel = DEFAULT_PARAMETRIC;
  // Each @param's current value as an exact fraction, seeded from the manifest defaults.
  const [paramValues, setParamValues] = useState<Record<string, Frac>>(() => {
    const init: Record<string, Frac> = {};
    for (const p of paramModel.params) {
      const den = p.fractional ? 2 : 1;
      init[p.name] = fracOf(p.default[1] === 0 ? 0 : p.default[0] / p.default[1], den);
    }
    return init;
  });

  // The IDE config for the editor — the linter surface tracks the LIVE edit surface (the global toggle),
  // so the buffer is diagnosed in the surface it's written in (fixes the all-red-squiggles P-C bug).
  // `prepare` AUTO-INJECTS the `import … from "exact"` clause (the same one `runModel` compiles), and
  // `preload` supplies the CAD library so the linter uses `diagnosticsWithPreloaded` — otherwise the
  // preloaded vocab (`Solid`/`v3r`/`lower`) would fault as unbound (6 red squiggles) on a program that
  // actually runs. `wrapPrefixBytes` is the injected prefix's byte length (from `wrapPrefixOf`, which
  // locates the reader's verbatim text in the injected output) so a squiggle maps back onto the buffer.
  // `surface` is read through a ref so the getter always sees the current toggle without rebuilding the
  // editor's extension array (which would remount on every toggle). The s-expr-for-driver requirement is
  // enforced separately in `runModel` (the mesh path), not here.
  const surfaceRef2 = useRef(surface);
  surfaceRef2.current = surface;
  const cadIde = useRef({
    surface: () => surfaceRef2.current,
    prepare: (editorText: string, from: Surface) => {
      const compiled = injectImport(editorText, from);
      return { compiled, wrapPrefixBytes: wrapPrefixOf(editorText, compiled) };
    },
    preload: () => ({ names: [CAD_LIB_NAME], sources: [EXACT_CDZ], formats: [CAD_LIB_FORMAT] }),
  }).current;

  const runModel = useCallback(
    async (src: string, from: Surface) => {
      if (runningRef.current) return;
      runningRef.current = true;
      setStatus({ phase: "running" });
      try {
        // Auto-inject the `import … from "exact"` (ruling A) and compile the model against the PRELOADED
        // CAD library — the buffer stays clean, the vocab is link-merged from `exact.cdz`.
        const program = injectImport(src, from);
        const out = await compileWithPreloaded(program, from, [CAD_LIB_NAME], [EXACT_CDZ], [CAD_LIB_FORMAT]);
        if (!out.component) {
          const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
          setStatus({ phase: "error", message: d ? `${d.code} ${d.message}` : "compile declined" });
          return;
        }
        // Render the value in s-expr regardless of the EDIT surface — meshFromSolid parses the canonical
        // s-expr Solid grammar (an ML render's commas/backtick-rationals aren't parseable by the driver).
        const result = await runComponent(out.component, "sexpr");
        if (result.kind !== "value") {
          const msg =
            result.kind === "trap" ? `trap: ${result.message}`
            : result.kind === "timeout" ? "timed out"
            : `error: ${result.message}`;
          setStatus({ phase: "error", message: msg });
          return;
        }
        // Hand the rendered s-expr Solid value to v-cad's mesh driver → manifold-3d CSG → triangles.
        const mesh = await meshFromSolid(result.text);
        if (!mesh.ok) {
          setStatus({ phase: "error", message: mesh.error });
          return;
        }
        setStatus({ phase: "meshed", mesh });
      } catch (e) {
        setStatus({ phase: "error", message: e instanceof Error ? e.message : String(e) });
      } finally {
        runningRef.current = false;
      }
    },
    [],
  );

  // Run the PARAMETRIC showcase model with the given @param values: compile it (preloaded against BOTH
  // exact + helpers — it imports both), run supplying each @param's {num,den} as a host-response (the
  // run-worker step-2 wiring), render the SolidR value in s-expr, and mesh it. Driven by the sliders.
  const runParametric = useCallback(
    async (from: Surface, values: Record<string, Frac>) => {
      if (runningRef.current) return;
      runningRef.current = true;
      setStatus({ phase: "running" });
      try {
        // The manifest's model source carries its own imports + @param decls + `def main`, but NO export —
        // add one so `main` is public (else "nothing is public"). ML: append `export { main }`. s-expr: wrap
        // the top-level forms in `(do … (export main))` (s-expr has no bare multi-form top level).
        const raw = (paramModel.source[from] ?? paramModel.source.ml).trim();
        const program = from === "sexpr" ? `(do\n${raw}\n(export main))` : `${raw}\nexport { main }`;
        const out = await compileWithPreloaded(
          program,
          from,
          [CAD_LIB_NAME, "helpers"],
          [EXACT_CDZ, HELPERS_CDZ],
          [CAD_LIB_FORMAT, CAD_LIB_FORMAT],
        );
        if (!out.component) {
          const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
          setStatus({ phase: "error", message: d ? `${d.code} ${d.message}` : "compile declined" });
          return;
        }
        // Supply the @param host-responses (name → {num,den}); the model reads them + renders a driven SolidR.
        const params: Record<string, { num: number; den: number }> = {};
        for (const [name, f] of Object.entries(values)) params[name] = { num: f.num, den: f.den };
        const result = await runComponent(out.component, "sexpr", false, params);
        if (result.kind !== "value") {
          const msg =
            result.kind === "trap" ? `trap: ${result.message}`
            : result.kind === "timeout" ? "timed out"
            : `error: ${result.message}`;
          setStatus({ phase: "error", message: msg });
          return;
        }
        const mesh = await meshFromSolid(result.text);
        if (!mesh.ok) {
          setStatus({ phase: "error", message: mesh.error });
          return;
        }
        setStatus({ phase: "meshed", mesh });
      } catch (e) {
        setStatus({ phase: "error", message: e instanceof Error ? e.message : String(e) });
      } finally {
        runningRef.current = false;
      }
    },
    [paramModel],
  );

  // A slider change: update that param's value + re-run the parametric model with the new values.
  const onParamChange = useCallback(
    (name: string, value: Frac) => {
      setParamValues((prev) => {
        const next = { ...prev, [name]: value };
        void runParametric(surface, next);
        return next;
      });
    },
    [surface, runParametric],
  );

  // On a surface change, re-seed the CURRENT example in the new surface (a source typed in the old surface
  // can't be blindly reinterpreted — same as /calculator) and re-run. Also covers the initial mount, so the
  // reader sees a meshed shape immediately.
  // Re-run on a surface change OR a mode switch. EDIT mode: re-seed the current example in the new surface
  // (source can't cross surfaces — like /calculator) + run it. PARAMETRIC mode: run the showcase model with
  // the current slider values. Fires on initial mount too, so the reader sees a meshed shape immediately.
  const lastRun = useRef<{ surface: Surface; mode: "edit" | "parametric" } | null>(null);
  useEffect(() => {
    const prev = lastRun.current;
    const first = prev === null;
    if (prev && prev.surface === surface && prev.mode === mode) return;
    lastRun.current = { surface, mode };
    if (mode === "parametric") {
      void runParametric(surface, paramValues);
      return;
    }
    // edit mode
    const next = first ? source : (example.source[surface] ?? example.source.ml);
    if (!first) setSource(next);
    void runModel(next, surface);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [surface, mode]);

  // Switch to another example model: seed the editor with its source in the CURRENT surface + re-run.
  const onSelectExample = useCallback(
    (slug: string) => {
      const picked = EXAMPLES.find((e) => e.slug === slug);
      if (!picked) return;
      setExampleSlug(slug);
      const next = picked.source[surface] ?? picked.source.ml;
      setSource(next);
      void runModel(next, surface);
    },
    [surface, runModel],
  );

  return (
    <div className="mx-auto flex min-h-screen max-w-6xl flex-col px-4 py-4">
      {/* `flex-wrap` so the controls (surface toggle + example picker + Playground link) drop to a second
          row on a narrow phone rather than overflowing horizontally (a 390px viewport can't fit the title +
          all three on one line). */}
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-x-3 gap-y-2">
        <h1 className="text-lg font-bold text-slate-100 sm:text-xl">Cadenza CAD</h1>
        {/* Mobile touch target: the controls get a 44px min-height below `sm`, compact at sm+. `flex-wrap`
            so the mode toggle + surface toggle + picker + link wrap across rows on a narrow phone (390px)
            instead of overflowing horizontally (there are now up to 4 control groups). */}
        <div className="flex min-w-0 shrink flex-wrap items-center justify-end gap-1 text-xs sm:gap-3">
          {/* Mode toggle: EDIT a model (picker + editor) vs the PARAMETRIC showcase (sliders). Parametric is
              the operator's "super cool" payoff — drag a slider, the model re-meshes live w/ exact dims. */}
          <div className="flex items-center rounded-md border border-slate-700 p-0.5" role="radiogroup" aria-label="Mode">
            {(["edit", "parametric"] as const).map((m) => (
              <button
                key={m}
                role="radio"
                aria-checked={mode === m}
                data-testid={`cad-mode-${m}`}
                onClick={() => setMode(m)}
                className={
                  "flex min-h-11 items-center rounded px-2 font-medium transition sm:min-h-0 sm:py-1 " +
                  (mode === m ? "bg-cadenza-600 text-white" : "text-slate-400 hover:text-slate-200")
                }
              >
                {m === "edit" ? "Model" : "Parametric"}
              </button>
            ))}
          </div>
          {/* The GLOBAL surface toggle (ML / s-expr) — /cad reads the live surface for editing (the model
              buffer is re-seeded per surface), but the app routes render under RootLayout, which has no
              header nav, so the chapter Layout's toggle isn't reachable here. Surface it so a reader can
              switch + STICK the surface on /cad too (operator UX: the same toggle everywhere). */}
          <SyntaxToggle />
          {/* Example picker — swap between the CAD models (cad/examples.ts, v-cad-authored). Edit-mode only
              (the parametric showcase is a single slider-driven model, no example list). */}
          {mode === "edit" && (
            <label className="flex min-h-11 items-center gap-1 sm:min-h-0">
              <span className="sr-only">Example model</span>
              <select
                data-testid="cad-example-picker"
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
          )}
          <Link
            to="/playground"
            className="flex min-h-11 items-center px-2 text-cadenza-400 hover:text-cadenza-300 sm:min-h-0 sm:px-0"
          >
            Playground →
          </Link>
        </div>
      </div>
      <p className="mb-3 text-xs text-slate-500 sm:text-sm">
        A solid model, in the real language — edit and Run to mesh it live in your browser. Geometry is
        computed with exact (Rational) arithmetic, meshed with manifold-3d, and drawn with three.js.
      </p>

      <div className="flex min-h-0 flex-1 flex-col gap-4 md:flex-row">
        {/* Left pane: EDIT = the model editor + Run; PARAMETRIC = the @param sliders (drag → re-mesh live). */}
        <div className="flex min-h-0 flex-1 flex-col rounded-lg border border-slate-800 bg-slate-900/40">
          {mode === "parametric" ? (
            <div className="min-h-[8rem] flex-1 overflow-auto p-3">
              <p className="mb-3 text-xs text-slate-500">
                {paramModel.description ?? "Drag a slider — the model recomputes and re-meshes live, with exact (Rational) dimensions."}
              </p>
              <ParametricControls params={paramModel.params} values={paramValues} onChange={onParamChange} />
              <p className="mt-3 font-mono text-xs text-slate-500" data-testid="cad-status">
                {status.phase === "error" ? (
                  <span className="text-rose-300">{status.message}</span>
                ) : status.phase === "running" ? (
                  "meshing…"
                ) : (
                  "exact Rational dimensions — a fractional slider (e.g. 7/2) is carried precisely"
                )}
              </p>
            </div>
          ) : (
            <>
              <div className="min-h-[8rem] flex-1 overflow-auto">
                <LazyCodeEditor value={source} onChange={setSource} ide={cadIde} minHeight="8rem" />
              </div>
              <div className="flex items-center justify-between border-t border-slate-800 px-3 py-2">
                <span className="font-mono text-xs text-slate-500">
                  {status.phase === "error" ? (
                    <span className="text-rose-300">{status.message}</span>
                  ) : status.phase === "running" ? (
                    "meshing…"
                  ) : (
                    "a Solid → 3D mesh"
                  )}
                </span>
                <button
                  onClick={() => void runModel(source, surface)}
                  disabled={status.phase === "running"}
                  className="flex min-h-11 items-center justify-center rounded bg-cadenza-600 px-3 text-xs font-semibold text-white transition enabled:hover:bg-cadenza-500 disabled:opacity-40 sm:min-h-0 sm:py-1"
                >
                  ▶ Run
                </button>
              </div>
            </>
          )}
        </div>

        {/* 3D preview. On MOBILE (stacked, flex-col) the preview is the star: give it a tall
            viewport-relative floor (`min-h-[60vh]`) so it fills most of the section instead of the
            small `flex-1`-split box it collapsed to before. At `md`+ (side-by-side) it drops back to
            the flex-driven `16rem` floor and fills its column. */}
        <div className="min-h-[60vh] flex-1 overflow-hidden rounded-lg border border-slate-800 bg-slate-950 md:min-h-[16rem]">
          {status.phase === "meshed" ? (
            <MeshView positions={status.mesh.positions} indices={status.mesh.indices} normals={status.mesh.normals} />
          ) : (
            <div className="flex h-full items-center justify-center text-sm text-slate-600">
              {status.phase === "error" ? "—" : status.phase === "running" ? "meshing…" : "Run to preview"}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

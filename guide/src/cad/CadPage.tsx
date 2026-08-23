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
/// builds and returns a `Solid`. The CAD vocabulary (`Solid`/`Vec3`/`v3`/`lower`/…) is a real Cadenza
/// module (`exact.cdz`) link-merged at compile via `compileWithPreloaded` (no inline `type` defs). The
/// host AUTO-INJECTS the `import { Solid, v3, lower } from "exact"` clause before compiling (ruling A),
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
import { compileWithPreloaded, paramManifest } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { SyntaxToggle } from "../syntax/SyntaxToggle.tsx";
import { meshFromSolid, DEFAULT_SEGMENTS, MIN_SEGMENTS, type MeshResult } from "./index.ts";
import { MeshView } from "./MeshView.tsx";
import { wrapPrefixOf } from "../components/wrapModule.ts";
import { injectImport, CAD_LIB_NAME, CAD_HELPERS_NAME, CAD_UNITS_NAME, CAD_LIB_FORMAT } from "./preloadModel.ts";
import { EXAMPLES, DEFAULT_EXAMPLE } from "./examples.ts";
import { slidersFromManifest } from "./manifestSlider.ts";
import { downloadMesh } from "./download.ts";
import { encodeCadShareUrl, decodeCadShare } from "./cadShare.ts";
import { resolveExampleParam, writeExampleParam } from "../components/exampleParam.ts";
import { ParametricControls, fracOf, type Frac } from "./ParametricControls.tsx";
import type { ParamSlider } from "./parametric.ts";
import type { Surface } from "../compiler/client.ts";
import { LazyCodeEditor } from "../editor/LazyCodeEditor.tsx";

// The upper bound of the preview quality slider. The exact model has no resolution ceiling (it's a mesh
// hint, not geometry), but a preview past ~128 sides buys no visible smoothness while slowing the live
// re-mesh — so the slider spans MIN_SEGMENTS..MAX_SEGMENTS with DEFAULT_SEGMENTS the initial value.
const MAX_SEGMENTS = 128;
// The CAD library sources, staged into the guide tree by `stage-wasm.mjs` (same pattern as runtime.wasm)
// and `?raw`-imported here as strings. PRELOADED via `compile_with_preloaded` (operator P5, ruling A) so a
// buffer holds only the model — the CAD vocab (`Solid`/`v3`/`lower`/…) is link-merged. `exact` is the base
// geometry lib; `helpers` (box/cyl/hole-through/…) is the ergonomic surface. SINGLE-MODE preloads BOTH for
// EVERY model, so any buffer (plain, curved, or `@param` parametric) can reach the full vocabulary.
import EXACT_CDZ from "../wasm/cad/exact.cdz?raw";
import HELPERS_CDZ from "../wasm/cad/helpers.cdz?raw";
import UNITS_CDZ from "../wasm/cad/units.cdz?raw";

// The preloaded-library triple passed to compile_with_preloaded — names/sources/formats, aligned 1:1 and
// SINGLE-SOURCE (previously the three arrays were hand-written + DUPLICATED at both the linter-preload and
// runModel call sites, so adding a CAD lib meant editing 4 literals in lockstep — the same parallel-array drift
// that broke /music when `pattern` was added to names but not sources). One constant, referenced everywhere, +
// an arity assertion: compile_with_preloaded requires equal lengths, so a mismatch throws a clear error at
// module load, not a cryptic "must be equal length" mid-compile. (Mirrors MusicPage's PRELOAD arity guard.)
const CAD_PRELOAD = {
  names: [CAD_LIB_NAME, CAD_HELPERS_NAME, CAD_UNITS_NAME],
  sources: [EXACT_CDZ, HELPERS_CDZ, UNITS_CDZ],
  formats: [CAD_LIB_FORMAT, CAD_LIB_FORMAT, CAD_LIB_FORMAT],
};
if (
  CAD_PRELOAD.names.length !== CAD_PRELOAD.sources.length ||
  CAD_PRELOAD.names.length !== CAD_PRELOAD.formats.length
) {
  throw new Error(
    `CadPage preload arity mismatch: names=${CAD_PRELOAD.names.length}, sources=${CAD_PRELOAD.sources.length}, ` +
      `formats=${CAD_PRELOAD.formats.length} — a CAD lib was added to one array but not the others (add the ` +
      `matching "../wasm/cad/<name>.cdz?raw" import + its sources/formats entry).`,
  );
}

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
/// string>`): a bare model built against the PRELOADED CAD library (`Solid`/`v3`/`lower` from `exact.cdz`) —
/// no inline `type` defs. Both the `import` AND the `@!default-fraction Rational` pragma are AUTO-INJECTED
/// (`injectImport`) — the reader's buffer is just the model (no import, no pragma line; the pragma grounds a
/// bare `n/d` to an exact Rational so `v3(4/1,…)` type-checks). The model returns `lower(<Solid model>)` (the
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
  const { surface, setSurface } = useSyntax();
  // A SHARED link (`#cad/…` in the URL hash, operator #7184) reconstructs a specific model: decode it ONCE
  // at first render (synchronous, like the playground's share seed — no effect race). If present, it seeds
  // the editor + surface + params below instead of DEFAULT_EXAMPLE; its params flow into the mount run so a
  // shared PARAMETRIC model restores its exact dragged dims. Null (the common case) → the normal default.
  const sharedRef = useRef(typeof window !== "undefined" ? decodeCadShare(window.location.hash) : null);
  const shared = sharedRef.current;
  // A `?example=<slug>` deep-link (operator: per-example nav) opens /cad with THAT model selected. The share
  // hash WINS if present (a shared edit shouldn't be clobbered by a stale example id); else the deep-link
  // slug (when it names a known example) selects it, else the default. Resolved once at first render.
  const initialSlug = shared ? DEFAULT_EXAMPLE.slug : resolveExampleParam(EXAMPLES.map((e) => e.slug), DEFAULT_EXAMPLE.slug);
  // The loaded example (drives the picker). Its `source[surface]` seeds the editor; switching examples or
  // toggling the surface re-seeds from `example.source[newSurface]` (v-cad ships every example in BOTH
  // surfaces, so a toggle is a clean re-seed — source can't be reinterpreted across surfaces, same as /calc).
  const [exampleSlug, setExampleSlug] = useState(initialSlug);
  const example = EXAMPLES.find((e) => e.slug === exampleSlug) ?? DEFAULT_EXAMPLE;
  const initialExample = EXAMPLES.find((e) => e.slug === initialSlug) ?? DEFAULT_EXAMPLE;
  const [source, setSource] = useState(() => shared?.src ?? initialExample.source[surface] ?? initialExample.source.ml);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  // The most recent SUCCESSFUL mesh — kept SEPARATELY from `status` so the 3D viewer stays MOUNTED across a
  // recompute (a param drag / re-Run cycles status meshed→running→meshed). Unmounting MeshView on every
  // recompute destroys its <Canvas> + camera, resetting the reader's vantage each drag (the operator's top
  // irritant). Instead we keep MeshView mounted showing `lastMesh` + overlay a "meshing…" hint while a new
  // run is in flight, and swap `lastMesh` when the new mesh arrives — so the camera persists.
  const [lastMesh, setLastMesh] = useState<Extract<MeshResult, { ok: true }> | null>(null);
  // Auto-spin the 3D view — DEFAULT OFF (operator: the constant spin is "annoying" + fights manual orbit);
  // a small toggle in the viewer turns it on. Fixed-by-default, spin on demand.
  const [spin, setSpin] = useState(false);
  const runningRef = useRef(false);

  // OpenSCAD-`$fn`-style PREVIEW resolution (increment 1): the tessellation quality the mesh driver uses for
  // every curved leaf + the revolve/Bézier sweep. Dragging the slider re-meshes the CURRENT model live at the
  // new resolution WITHOUT recompiling (it re-meshes the last rendered Solid s-expr, kept in `lastSolidTextRef`)
  // — a mesh hint only, so the exact Rational model is unchanged. This same value threads into the STL/3MF
  // export (increment 3) so what the reader sees is what they download. Read through a ref so the run/remesh
  // callbacks aren't rebuilt on every drag (which would churn the editor's onChange identity).
  const [segments, setSegments] = useState(DEFAULT_SEGMENTS);
  const segmentsRef = useRef(segments);
  segmentsRef.current = segments;
  // The most recent rendered Solid s-expr (the driver's INPUT). A slider drag re-meshes THIS text at the new
  // resolution — no recompile/re-run of the buffer — so the quality knob is cheap and live.
  const lastSolidTextRef = useRef<string | null>(null);

  // SINGLE-MODE (operator): there is ONE mode — the reader edits a model buffer, and if that buffer DECLARES
  // `@param`s, a slider auto-surfaces per param (read LIVE from the compiled model's manifest, not a hardcoded
  // list). `sliders` is the current buffer's `@param` widget metadata (empty for a non-parametric model —
  // then no sliders show, just the editor + preview). `paramValues` is each param's current value as an exact
  // fraction, seeded from the manifest defaults and updated as the reader drags; /cad supplies each as a
  // `Param.<name>-num/-den` host-response so an EXACT fractional dim (7/2) is carried into the recompute.
  const [sliders, setSliders] = useState<ParamSlider[]>([]);
  const [paramValues, setParamValues] = useState<Record<string, Frac>>({});
  // Read across callbacks without re-creating them: the current slider values feed every run's host-responses,
  // and the surface feeds the manifest scan. Refs so `runModel` isn't rebuilt on every drag (which would
  // churn the editor's `onChange` identity).
  const paramValuesRef = useRef(paramValues);
  paramValuesRef.current = paramValues;

  // The IDE config for the editor — the linter surface tracks the LIVE edit surface (the global toggle),
  // so the buffer is diagnosed in the surface it's written in (fixes the all-red-squiggles P-C bug).
  // `prepare` AUTO-INJECTS the `import … from "exact"` clause (the same one `runModel` compiles), and
  // `preload` supplies the CAD library so the linter uses `diagnosticsWithPreloaded` — otherwise the
  // preloaded vocab (`Solid`/`v3`/`lower`) would fault as unbound (6 red squiggles) on a program that
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
    // Preload the general CAD vocab the injected imports reference (exact + helpers + units), so the linter
    // resolves the full injected vocab — otherwise a model using `box`/`inch` faults them as unbound (the same
    // class as the earlier Solid/v3 all-red-squiggles bug), even though it runs fine. No snowflake/prng lib:
    // the snowflake showcase is self-contained (its builder is inline in the buffer), needing only this vocab.
    preload: () => ({ names: CAD_PRELOAD.names, sources: CAD_PRELOAD.sources, formats: CAD_PRELOAD.formats }),
  }).current;

  // THE single run path. Inject the imports (exact + helpers) + pragma + export (ruling A — the buffer stays
  // clean), compile against the PRELOADED libraries, RUN supplying each `@param`'s current value as a
  // `Param.<name>-num/-den` host-response (so a parametric model recomputes with the live slider values; a
  // non-parametric model just ignores an empty param map), render the SolidR value in s-expr (the driver
  // parses the canonical s-expr grammar, not the ML display surface), and mesh it. `values` defaults to the
  // current slider values (a Run / example-load), or the freshly-dragged values (a slider change) via the
  // explicit arg — read from a ref-free explicit param so a drag runs the NEW values, not a stale snapshot.
  const runModel = useCallback(
    async (src: string, from: Surface, values: Record<string, Frac> = paramValuesRef.current) => {
      if (runningRef.current) return;
      runningRef.current = true;
      setStatus({ phase: "running" });
      try {
        const program = injectImport(src, from);
        const out = await compileWithPreloaded(
          program,
          from,
          CAD_PRELOAD.names,
          CAD_PRELOAD.sources,
          CAD_PRELOAD.formats,
        );
        if (!out.component) {
          const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
          setStatus({ phase: "error", message: d ? `${d.code} ${d.message}` : "compile declined" });
          return;
        }
        // Supply the @param host-responses (name → {num,den}) — empty for a non-parametric model, which then
        // just runs its `main` directly. A parametric `main` reads `Param.<name>()` and renders a driven SolidR.
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
        // Hand the rendered s-expr Solid value to v-cad's mesh driver → manifold-3d CSG → triangles, at the
        // reader's current preview resolution. Remember the rendered text so a segments-slider drag can
        // re-mesh THIS value at a new resolution without recompiling the buffer (see `onSegmentsChange`).
        lastSolidTextRef.current = result.text;
        const mesh = await meshFromSolid(result.text, segmentsRef.current);
        if (!mesh.ok) {
          setStatus({ phase: "error", message: mesh.error });
          return;
        }
        setLastMesh(mesh);
        setStatus({ phase: "meshed", mesh });
      } catch (e) {
        setStatus({ phase: "error", message: e instanceof Error ? e.message : String(e) });
      } finally {
        runningRef.current = false;
      }
    },
    [],
  );

  // Refresh the `@param` sliders from a buffer's LIVE manifest: scan the buffer, convert each `@param` entry
  // to a slider, and seed any NEW param's value from its manifest default (preserving a value the reader has
  // already dragged, keyed by name). Returns the seeded values so the caller can run with them immediately
  // (a fresh example-load must run with the seeded defaults, not the stale previous map). A buffer with no
  // `@param` yields no sliders (a plain model shows just the editor + preview).
  const refreshManifest = useCallback(
    async (src: string, from: Surface): Promise<Record<string, Frac>> => {
      let next: ParamSlider[] = [];
      try {
        next = slidersFromManifest(await paramManifest(injectImport(src, from), from));
      } catch {
        next = [];
      }
      setSliders(next);
      const seeded: Record<string, Frac> = {};
      for (const p of next) {
        const den = p.fractional ? 2 : 1;
        // Keep an already-dragged value; else seed from the manifest default.
        seeded[p.name] = paramValuesRef.current[p.name] ?? fracOf(p.default[1] === 0 ? 0 : p.default[0] / p.default[1], den);
      }
      setParamValues(seeded);
      paramValuesRef.current = seeded;
      return seeded;
    },
    [],
  );

  // A slider change: update that param's value + re-run the model with the new values (no recompile churn —
  // same buffer, new host-responses). Runs against the freshly-merged values, not a stale snapshot.
  const onParamChange = useCallback(
    (name: string, value: Frac) => {
      const next = { ...paramValuesRef.current, [name]: value };
      setParamValues(next);
      paramValuesRef.current = next;
      void runModel(source, surface, next);
    },
    [source, surface, runModel],
  );

  // A preview-quality slider change (increment 1): update the resolution + re-mesh the CURRENT model live at
  // the new segment count. This does NOT recompile/re-run the buffer — it re-meshes the last rendered Solid
  // s-expr (`lastSolidTextRef`) at the new resolution, so raising quality is cheap and instant. If no model
  // has meshed yet (nothing rendered), or a compile/run is in flight, just record the value (the next run
  // picks it up via `segmentsRef`). Guarded by `runningRef` so a drag mid-run doesn't race the run's mesh.
  const onSegmentsChange = useCallback((n: number) => {
    setSegments(n);
    segmentsRef.current = n;
    const text = lastSolidTextRef.current;
    if (text === null || runningRef.current) return;
    runningRef.current = true;
    setStatus({ phase: "running" });
    void (async () => {
      try {
        const mesh = await meshFromSolid(text, n);
        if (!mesh.ok) {
          setStatus({ phase: "error", message: mesh.error });
          return;
        }
        setLastMesh(mesh);
        setStatus({ phase: "meshed", mesh });
      } catch (e) {
        setStatus({ phase: "error", message: e instanceof Error ? e.message : String(e) });
      } finally {
        runningRef.current = false;
      }
    })();
  }, []);

  // On a surface change, re-seed the CURRENT example in the new surface (a source typed in the old surface
  // can't be blindly reinterpreted — same as /calculator), refresh its `@param` sliders from the manifest,
  // and re-run. Fires on initial mount too, so the reader sees a meshed shape (+ any sliders) immediately.
  const lastSurface = useRef<Surface | null>(null);
  useEffect(() => {
    const first = lastSurface.current === null;
    // A shared `#cad/` link pins the surface to what it was authored in (the source can't be reinterpreted
    // across surfaces). On first mount, if the shared surface differs from the reader's current toggle, flip
    // the toggle to it — that re-fires this effect at the right surface, so we defer the run to that pass.
    if (first && shared && shared.s !== surface) {
      setSurface(shared.s);
      return;
    }
    if (!first && lastSurface.current === surface) return;
    lastSurface.current = surface;
    // On first mount seed the editor `source` (the shared src if any, else the default set in useState);
    // on a later surface flip, re-seed the current example in the new surface. A shared model isn't in the
    // example list, so a surface flip after a shared load would lose it — but the shared surface is pinned
    // above, so we only reach a non-first pass here for a reader's own toggle (no shared model in play then).
    const next = first ? source : (example.source[surface] ?? example.source.ml);
    if (!first) setSource(next);
    // First mount of a shared PARAMETRIC model: seed its exact param values so the run restores the dragged
    // dims (refreshManifest keeps an already-present value by name, so pre-seeding the ref wins over defaults).
    if (first && shared?.params) paramValuesRef.current = { ...shared.params };
    void (async () => {
      const seeded = await refreshManifest(next, surface);
      void runModel(next, surface, seeded);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [surface]);

  // Switch to another example model: seed the editor with its source in the CURRENT surface, refresh its
  // `@param` sliders from the manifest (a parametric example surfaces sliders; a plain one clears them), and
  // re-run with the freshly-seeded param defaults.
  const onSelectExample = useCallback(
    (slug: string) => {
      const picked = EXAMPLES.find((e) => e.slug === slug);
      if (!picked) return;
      setExampleSlug(slug);
      writeExampleParam(slug); // reflect the selection in the URL (?example=…) so it's a copy-shareable deep-link
      const next = picked.source[surface] ?? picked.source.ml;
      setSource(next);
      // Clear any dragged values from the previous example so the new one seeds fresh from its own defaults.
      paramValuesRef.current = {};
      void (async () => {
        const seeded = await refreshManifest(next, surface);
        void runModel(next, surface, seeded);
      })();
    },
    [surface, runModel, refreshManifest],
  );

  // A manual Run (the reader edited the buffer): refresh sliders from the edited buffer's manifest (they may
  // have added / changed / removed a `@param`) then run. This is what makes "examples declare their own
  // params and they show up automatically" work for a READER-edited buffer, not just the seed examples.
  const onRun = useCallback(() => {
    void (async () => {
      const seeded = await refreshManifest(source, surface);
      void runModel(source, surface, seeded);
    })();
  }, [source, surface, refreshManifest, runModel]);

  // SHARE (operator #7184): copy a `#cad/…` URL that reconstructs the current model — the editor source, its
  // surface, and (for a parametric model) the current slider values as exact fractions — so a shared link
  // restores the exact view, including dragged dims. A brief "Copied!" confirms. Falls back to no-op if the
  // clipboard API is unavailable (non-secure context) — the URL is still built (a future prompt could show it).
  const [shareCopied, setShareCopied] = useState(false);
  const onShare = useCallback(() => {
    const payload = sliders.length > 0 ? { s: surface, src: source, params: paramValues } : { s: surface, src: source };
    const url = encodeCadShareUrl(payload);
    void navigator.clipboard?.writeText(url).then(
      () => {
        setShareCopied(true);
        setTimeout(() => setShareCopied(false), 1500);
      },
      () => {
        /* clipboard denied (non-secure context / permissions) — no-op, keep the UI stable */
      },
    );
  }, [sliders.length, surface, source, paramValues]);

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
          {/* The GLOBAL surface toggle (ML / s-expr) — /cad reads the live surface for editing (the model
              buffer is re-seeded per surface), but the app routes render under RootLayout, which has no
              header nav, so the chapter Layout's toggle isn't reachable here. Surface it so a reader can
              switch + STICK the surface on /cad too (operator UX: the same toggle everywhere). */}
          <SyntaxToggle />
          {/* Example picker — swap between the CAD models (cad/examples.ts, v-cad-authored). SINGLE-MODE: one
              list for every model, plain or parametric; picking a parametric example (the plate) surfaces its
              sliders automatically from the compiled manifest. */}
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
          {/* SHARE — copy a URL that reconstructs this exact model (+ params) for another reader (operator
              #7184, same mechanism as the playground's share). */}
          <button
            data-testid="cad-share"
            onClick={onShare}
            title="Copy a shareable link to this model"
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
      <p className="mb-3 text-xs text-slate-500 sm:text-sm">
        A solid model, in the real language — edit and Run to mesh it live in your browser. Geometry is
        computed with exact (Rational) arithmetic, meshed with manifold-3d, and drawn with three.js.
      </p>

      <div className="flex min-h-0 flex-1 flex-col gap-4 md:flex-row">
        {/* Left pane: the model editor + Run, and — when the buffer declares `@param`s — a slider per param
            below it (auto-surfaced from the compiled manifest; drag → re-mesh live with exact dims). ONE mode:
            a plain model shows just the editor; a parametric model additionally shows its sliders.
            `md:min-w-0` is LOAD-BEARING on desktop: without it a flex item defaults to `min-width:auto` and
            REFUSES to shrink below its content's min-content width — CodeMirror's content min-width is large,
            so the editor column overflowed past its share and squished the 3D preview to a ~400px sliver
            (operator's desktop bug). With `min-w-0` the editor can shrink, and the preview's larger flex-grow
            (below) makes IT the primary pane. `md:flex-[2]` editor : `md:flex-[3]` preview = a 40/60 split. */}
        <div className="flex min-h-0 flex-col rounded-lg border border-slate-800 bg-slate-900/40 md:min-w-0 md:flex-[2]">
          <div className="min-h-[8rem] flex-1 overflow-auto">
            <LazyCodeEditor value={source} onChange={setSource} ide={cadIde} minHeight="8rem" />
          </div>
          {/* Auto-surfaced `@param` sliders — present only when the current model declares params (operator:
              "examples declare their own parameters and those show up in the UI automatically"). Drag → the
              model recomputes with the exact {num,den} + re-meshes, no recompile of the buffer. */}
          {sliders.length > 0 && (
            <div className="border-t border-slate-800 p-3" data-testid="cad-params">
              <p className="mb-2 text-xs text-slate-500">
                This model declares parameters — drag a slider and it re-meshes live, with exact (Rational)
                dimensions (a fractional value like 7/2 is carried precisely).
              </p>
              <ParametricControls params={sliders} values={paramValues} onChange={onParamChange} />
            </div>
          )}
          <div className="flex items-center justify-between border-t border-slate-800 px-3 py-2">
            <span className="font-mono text-xs text-slate-500" data-testid="cad-status">
              {status.phase === "error" ? (
                <span className="text-rose-300">{status.message}</span>
              ) : status.phase === "running" ? (
                "meshing…"
              ) : sliders.length > 0 ? (
                "a parametric Solid → 3D mesh"
              ) : (
                "a Solid → 3D mesh"
              )}
            </span>
            <button
              onClick={onRun}
              disabled={status.phase === "running"}
              className="flex min-h-11 items-center justify-center rounded bg-cadenza-600 px-3 text-xs font-semibold text-white transition enabled:hover:bg-cadenza-500 disabled:opacity-40 sm:min-h-0 sm:py-1"
            >
              ▶ Run
            </button>
          </div>
        </div>

        {/* 3D preview — the PRIMARY pane. On MOBILE (stacked, flex-col) give it a CONCRETE height (`h-[65vh]`,
            not just a min) so the react-three-fiber <Canvas> — which sizes to its parent's measured box — gets
            a definite ~500px box instead of collapsing to ~150px in the flex-col (the earlier "stuck ~300px"
            mobile bug). On DESKTOP (`md`+, side-by-side) it's the LARGER pane: `md:flex-[3]` vs the editor's
            `md:flex-[2]` (a 60/40 split) + `md:min-w-[24rem]` so the viewer never squishes to a sliver again
            (the operator's "~400px off to the side" desktop bug — the editor column lacked `min-w-0` and ate
            the width; fixed there + the preview now claims the majority share). `md:h-auto` fills the column. */}
        <div
          data-testid="cad-preview"
          // The rendered geometry's triangle count — a DOM-visible signal for a VISIBLE-render check (a
          // headless "a <canvas> mounted" assertion is NOT enough: a 0-triangle empty mesh still mounts a
          // canvas and shows BLANK, which is exactly the empty-Solid-annihilation blank the operator hit).
          // 0 (or absent) = nothing to see; >0 = real geometry on the canvas. check:visual asserts this > 0.
          data-mesh-tris={lastMesh ? lastMesh.indices.length / 3 : 0}
          className="relative h-[65vh] shrink-0 overflow-hidden rounded-lg border border-slate-800 bg-slate-950 md:h-auto md:min-h-[16rem] md:min-w-[24rem] md:flex-[3] md:shrink"
        >
          {lastMesh ? (
            // Keep MeshView MOUNTED once we have any mesh — a recompute (running) swaps its geometry to the
            // latest `lastMesh` WITHOUT unmounting, so the camera vantage persists (operator's top irritant).
            <>
              <MeshView positions={lastMesh.positions} indices={lastMesh.indices} normals={lastMesh.normals} spin={spin} />
              {/* Spin vs FIXED toggle (default fixed) — top-left so it doesn't collide with the meshing chip. */}
              <button
                data-testid="cad-spin-toggle"
                onClick={() => setSpin((s) => !s)}
                aria-pressed={spin}
                className="absolute left-2 top-2 flex min-h-11 items-center rounded bg-slate-800/80 px-2 text-xs text-slate-300 transition hover:bg-slate-700 sm:min-h-0 sm:py-1"
              >
                {spin ? "◼ Stop spin" : "↻ Spin"}
              </button>
              {status.phase === "running" && (
                // A subtle non-blocking "meshing…" chip while a new run is in flight (viewer stays interactive).
                <div className="pointer-events-none absolute right-2 top-2 rounded bg-slate-800/80 px-2 py-1 text-xs text-slate-300">
                  meshing…
                </div>
              )}
              {status.phase === "error" && (
                <div className="pointer-events-none absolute inset-x-2 top-2 rounded bg-rose-900/80 px-2 py-1 text-xs text-rose-100">
                  {status.message}
                </div>
              )}
              {/* DOWNLOAD the current mesh as STL / 3MF (operator ask — for real CAD/print use). Bottom-left so
                  it clears the spin toggle (top-left) + meshing/error chips (top). v-cad's serializers take the
                  mesh's positions+indices directly (no adapter); 3MF defaults to millimeter (printer-world). */}
              <div className="absolute bottom-2 left-2 flex items-center gap-1">
                <button
                  data-testid="cad-download-stl"
                  onClick={() => downloadMesh(lastMesh, "stl")}
                  title="Download this model as a binary STL"
                  className="flex min-h-11 items-center rounded bg-slate-800/80 px-2 text-xs text-slate-300 transition hover:bg-slate-700 sm:min-h-0 sm:py-1"
                >
                  ↓ STL
                </button>
                <button
                  data-testid="cad-download-3mf"
                  onClick={() => downloadMesh(lastMesh, "3mf")}
                  title="Download this model as a 3MF (unit-declared, for 3D printing)"
                  className="flex min-h-11 items-center rounded bg-slate-800/80 px-2 text-xs text-slate-300 transition hover:bg-slate-700 sm:min-h-0 sm:py-1"
                >
                  ↓ 3MF
                </button>
              </div>
              {/* PREVIEW QUALITY (increment 1) — an OpenSCAD-`$fn`-style resolution slider. Drag it and the
                  current model re-meshes live at the new tessellation (curved leaves + revolve/Bézier sweep
                  refine together); a mesh hint only, so the exact model is unchanged. Bottom-right so it
                  clears the download buttons (bottom-left) + the spin/meshing chips (top). */}
              <label
                data-testid="cad-quality"
                className="absolute bottom-2 right-2 flex items-center gap-2 rounded bg-slate-800/80 px-2 py-1 text-xs text-slate-300"
                title="Preview tessellation resolution (OpenSCAD $fn-style) — higher is smoother"
              >
                <span className="whitespace-nowrap">Quality</span>
                <input
                  data-testid="cad-quality-slider"
                  type="range"
                  min={MIN_SEGMENTS}
                  max={MAX_SEGMENTS}
                  step={1}
                  value={segments}
                  onChange={(e) => onSegmentsChange(Number(e.target.value))}
                  className="h-1 w-20 cursor-pointer accent-cadenza-500"
                  aria-label="Preview resolution (segments)"
                />
                <span className="w-6 text-right font-mono tabular-nums" data-testid="cad-quality-value">
                  {segments}
                </span>
              </label>
            </>
          ) : (
            <div className="flex h-full items-center justify-center text-sm text-slate-600">
              {status.phase === "error" ? status.message : status.phase === "running" ? "meshing…" : "Run to preview"}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

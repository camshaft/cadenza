/// The `/cad` route — a live browser 3D preview of a Cadenza CAD model: edit a Solid-producing program,
/// Run it, and see the meshed result rotate in a three.js canvas. Mirrors /calculator's shape (an
/// editable program over the real language, executed in-browser), but the result is geometry instead of
/// a value. Part of the operator's "showcase every use-case as a working example" push.
///
/// THE SPLIT (confirmed with v-cad): this vertical owns the route + shell + the react-three-fiber canvas
/// + the 3 npm deps (three, @react-three/fiber, manifold-3d) — all code-split behind this lazy route so
/// they never touch the guide's first paint. v-cad owns `guide/src/cad/index.ts` (`meshFromSolid`: parse
/// a rendered Solidr S-EXPR → manifold-3d CSG → triangle buffers).
///
/// SURFACE: /cad respects the global surface toggle for EDITING (like /calculator + /playground) — a
/// per-surface starter, edited in whichever surface the reader has selected — but the compiled value is
/// always RUN + rendered in s-expr (`runComponent(component, "sexpr")`) before it reaches the driver.
/// `meshFromSolid` parses the RENDERED value as an s-expr `(: (Differencer …) Solidr)`; an ML render uses
/// commas + backtick-rationals the s-expr parser can't read, so the driver consumes the canonical machine
/// form, not the display surface. Both starters are self-contained (inline `type` defs + `def main`): the
/// CAD library modules aren't resolvable in the browser compiler, so each program defines its own
/// `Vec3r`/`Solidr` and returns a `Solidr` value. Both render IDENTICALLY to
/// `(: (Differencer (Cuber (: (tuple 4/1 4/1 4/1) Vec3r)) (Spherer 5/2)) Solidr)` (v-cad-verified end to
/// end — 584 triangles), so the driver behaves the same whichever surface the reader edits in.

import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { compile } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { wrapModule } from "../components/wrapModule.ts";
import { meshFromSolid, type MeshResult } from "./index.ts";
import { MeshView } from "./MeshView.tsx";
import type { Surface } from "../compiler/client.ts";
import { LazyCodeEditor } from "../editor/LazyCodeEditor.tsx";

// /cad's IDE config for the shared editor: the program is a self-contained module (no wrapping), so the
// compiled text IS the editor text (prefix 0), and the surface is fixed to CAD_SURFACE. This turns on the
// Cadenza lexical + semantic highlighting + squiggles/hover — the operator-requested IDE editor.
const CAD_IDE = {
  surface: () => "sexpr" as Surface,
  prepare: (editorText: string) => ({ compiled: editorText, wrapPrefixBytes: 0 }),
};

/// The starter Solid model per surface — a 4mm cube with a 2.5-radius spherical dent (the classic CSG
/// difference). Self-contained (inline `type` defs + `def main`): the CAD library modules aren't resolvable
/// in the browser compiler, so each program defines its own `Vec3r`/`Solidr` and returns a `Solidr` value
/// that renders to exactly the grammar `meshFromSolid` parses. Rationals are `Rational.of(n, d)` / `(. Rational of)` —
/// a bare `n/d` in source is Int64 division. Both surfaces render to the same canonical s-expr value, so
/// the driver meshes them identically (v-cad-verified: 584 triangles end to end).
const STARTER: Record<Surface, string> = {
  ml: `type Vec3r = | V3r(Rational, Rational, Rational)
type Solidr =
  | Cuber(Vec3r)
  | Spherer(Rational)
  | Differencer(Solidr, Solidr)
def r(n: Int64) = Rational.of(n, 1)
def main() =
  Solidr.Differencer(
    Solidr.Cuber(V3r(r(4), r(4), r(4))),
    Solidr.Spherer(Rational.of(5, 2)))`,
  sexpr: `(do
  (type Vec3r (V3r Rational Rational Rational))
  (type Solidr (Cuber Vec3r) (Spherer Rational) (Differencer Solidr Solidr))
  (def (r (: n Int64)) ((. Rational of) n 1))
  (def (main) ((. Solidr Differencer) ((. Solidr Cuber) ((. Vec3r V3r) (r 4) (r 4) (r 4))) ((. Solidr Spherer) ((. Rational of) 5 2))))
  (export main))`,
};

type Status =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "meshed"; mesh: Extract<MeshResult, { ok: true }> }
  | { phase: "error"; message: string };

export default function CadPage() {
  const { surface } = useSyntax();
  const [source, setSource] = useState(() => STARTER[surface] ?? STARTER.ml);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  const runningRef = useRef(false);

  const runModel = useCallback(
    async (src: string, from: Surface) => {
      if (runningRef.current) return;
      runningRef.current = true;
      setStatus({ phase: "running" });
      try {
        const program = wrapModule(src, from);
        const out = await compile(program, from);
        if (!out.component) {
          const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
          setStatus({ phase: "error", message: d ? `${d.code} ${d.message}` : "compile declined" });
          return;
        }
        // Render the value in s-expr regardless of the EDIT surface — meshFromSolid parses the canonical
        // s-expr Solidr grammar (an ML render's commas/backtick-rationals aren't parseable by the driver).
        const result = await runComponent(out.component, "sexpr");
        if (result.kind !== "value") {
          const msg =
            result.kind === "trap" ? `trap: ${result.message}`
            : result.kind === "timeout" ? "timed out"
            : `error: ${result.message}`;
          setStatus({ phase: "error", message: msg });
          return;
        }
        // Hand the rendered s-expr Solidr value to v-cad's mesh driver → manifold-3d CSG → triangles.
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

  // On a surface change, re-seed the starter in the new surface (a source typed in the old surface can't
  // be blindly reinterpreted — same as /calculator) and re-run. Also covers the initial mount, so the
  // reader sees a meshed shape immediately.
  const surfaceRef = useRef<Surface | null>(null);
  useEffect(() => {
    if (surfaceRef.current === surface) return;
    const first = surfaceRef.current === null;
    surfaceRef.current = surface;
    const next = first ? source : (STARTER[surface] ?? STARTER.ml);
    if (!first) setSource(next);
    void runModel(next, surface);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [surface]);

  return (
    <div className="mx-auto flex min-h-screen max-w-6xl flex-col px-4 py-4">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h1 className="text-lg font-bold text-slate-100 sm:text-xl">Cadenza CAD</h1>
        {/* Mobile touch target: the header link gets a 44px min-height below `sm`, compact at sm+. */}
        <div className="flex shrink-0 items-center gap-3 text-xs">
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
        {/* Editor + Run */}
        <div className="flex min-h-0 flex-1 flex-col rounded-lg border border-slate-800 bg-slate-900/40">
          <div className="min-h-[8rem] flex-1 overflow-auto">
            <LazyCodeEditor value={source} onChange={setSource} ide={CAD_IDE} minHeight="8rem" />
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

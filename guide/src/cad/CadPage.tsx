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
/// SURFACE (why /cad is s-expr, not the global surface): `meshFromSolid` parses the RENDERED value as an
/// s-expr `(: (Differencer …) Solidr)`. The value is therefore always RUN + rendered in s-expr
/// (`runComponent(component, "sexpr")`) regardless of the editing surface — the driver consumes a
/// canonical machine form, not the display surface (an ML render uses commas + backtick-rationals the
/// s-expr parser can't read). The starter program is s-expr too: the equivalent ML program currently
/// trips a front-end parse divergence in the browser guide-wasm compiler (a nested multi-arg ctor in an
/// ML type-def block — filed to v-syntax; native `cdz` accepts it). Once that lands, an ML starter can
/// follow. Self-contained by design (inline `type` defs + `def main`): the CAD library modules aren't
/// resolvable in the browser compiler, and the driver only needs the rendered Solidr value.

import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { compile } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import { wrapModule } from "../components/wrapModule.ts";
import { meshFromSolid, type MeshResult } from "./index.ts";
import { MeshView } from "./MeshView.tsx";

/// The /cad editing surface is fixed to s-expr (see the header: the ML equivalent trips a browser
/// front-end parse divergence, and the driver parses the s-expr render anyway).
const CAD_SURFACE = "sexpr" as const;

/// The starter Solid model — a 4mm cube with a spherical dent (the classic CSG difference), verified by
/// v-cad to compile → render → mesh to 560 triangles end-to-end. Self-contained (inline `type` defs +
/// `def main`): the CAD library modules aren't resolvable in the browser compiler, so the program
/// defines its own `Vec3r`/`Solidr` and returns a `Solidr` value that renders to exactly the grammar
/// `meshFromSolid` parses. Rationals are `(Rational.of n d)` — a bare `n/d` in source is Int64 division.
const STARTER = `(do
  (type Vec3r (V3r Rational Rational Rational))
  (type Solidr (Cuber Vec3r) (Spherer Rational) (Differencer Solidr Solidr))
  (def (r (: n Int64)) ((. Rational of) n 1))
  (def (main) ((. Solidr Differencer) ((. Solidr Cuber) ((. Vec3r V3r) (r 4) (r 4) (r 4))) ((. Solidr Spherer) ((. Rational of) 5 2))))
  (export main))`;

type Status =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "meshed"; mesh: Extract<MeshResult, { ok: true }> }
  | { phase: "error"; message: string };

export default function CadPage() {
  const [source, setSource] = useState(STARTER);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  const runningRef = useRef(false);

  const runModel = useCallback(async () => {
    if (runningRef.current) return;
    runningRef.current = true;
    setStatus({ phase: "running" });
    try {
      const program = wrapModule(source, CAD_SURFACE);
      const out = await compile(program, CAD_SURFACE);
      if (!out.component) {
        const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
        setStatus({ phase: "error", message: d ? `${d.code} ${d.message}` : "compile declined" });
        return;
      }
      // Render the value in s-expr regardless of display surface — meshFromSolid parses the canonical
      // s-expr Solidr grammar (an ML render's commas/backtick-rationals aren't parseable by the driver).
      const result = await runComponent(out.component, CAD_SURFACE);
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
  }, [source]);

  // Auto-run once on mount so the reader sees a shape immediately.
  useEffect(() => {
    void runModel();
    // mount only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="mx-auto flex min-h-screen max-w-6xl flex-col px-4 py-4">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h1 className="text-lg font-bold text-slate-100 sm:text-xl">Cadenza CAD</h1>
        <div className="flex shrink-0 items-center gap-3 text-xs">
          <Link to="/playground" className="text-cadenza-400 hover:text-cadenza-300">
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
          <textarea
            value={source}
            onChange={(e) => setSource(e.target.value)}
            spellCheck={false}
            className="min-h-[8rem] flex-1 resize-none bg-transparent p-3 font-mono text-sm text-slate-100 focus:outline-none"
          />
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
              onClick={() => void runModel()}
              disabled={status.phase === "running"}
              className="rounded bg-cadenza-600 px-3 py-1 text-xs font-semibold text-white transition enabled:hover:bg-cadenza-500 disabled:opacity-40"
            >
              ▶ Run
            </button>
          </div>
        </div>

        {/* 3D preview */}
        <div className="min-h-[16rem] flex-1 overflow-hidden rounded-lg border border-slate-800 bg-slate-950">
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

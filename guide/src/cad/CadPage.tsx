/// The `/cad` route — a live browser 3D preview of a Cadenza CAD model: edit a Solid-producing program,
/// Run it, and see the meshed result rotate in a three.js canvas. Mirrors /calculator's shape (an
/// editable program over the real language, executed in-browser), but the result is geometry instead of
/// a value. Part of the operator's "showcase every use-case as a working example" push.
///
/// THE SPLIT (confirmed with v-cad): this vertical owns the route + shell + the react-three-fiber canvas
/// + the 3 npm deps (three, @react-three/fiber, manifold-3d) — all code-split behind this lazy route so
/// they never touch the guide's first paint. v-cad owns `guide/src/cad/index.ts` (`meshFromSolid`: parse
/// the run worker's Solid s-expr → manifold-3d CSG → triangle buffers). Today it's a stub returning a
/// unit cube; v-cad drops in the real parser/driver as a pure module swap, no changes here.

import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { compile } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { wrapModule } from "../components/wrapModule.ts";
import { meshFromSolid, type MeshResult } from "./index.ts";
import { MeshView } from "./MeshView.tsx";

/// A starter Solid model per surface. Kept tiny — the point is the live 3D loop, not CAD depth. v-cad's
/// real models (Rational/Qty-exact) will supply richer examples once the parser lands.
const STARTER: Record<string, string> = {
  ml: "cube(2.0)",
  sexpr: "(cube 2.0)",
};

type Status =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "meshed"; mesh: Extract<MeshResult, { ok: true }> }
  | { phase: "error"; message: string };

export default function CadPage() {
  const { surface } = useSyntax();
  const [source, setSource] = useState(STARTER[surface] ?? STARTER.ml);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  const runningRef = useRef(false);

  const runModel = useCallback(async () => {
    if (runningRef.current) return;
    runningRef.current = true;
    setStatus({ phase: "running" });
    try {
      const program = wrapModule(source, surface);
      const out = await compile(program, surface);
      if (!out.component) {
        const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
        setStatus({ phase: "error", message: d ? `${d.code} ${d.message}` : "compile declined" });
        return;
      }
      const result = await runComponent(out.component, surface);
      if (result.kind !== "value") {
        const msg =
          result.kind === "trap" ? `trap: ${result.message}`
          : result.kind === "timeout" ? "timed out"
          : `error: ${result.message}`;
        setStatus({ phase: "error", message: msg });
        return;
      }
      // Hand the rendered Solid value text to v-cad's mesh driver (stub today).
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
  }, [source, surface]);

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

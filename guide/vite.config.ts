import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";
import { copyFileSync, existsSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

// A per-BUILD id, stamped once at config time. Injected into the bundle (so the running app knows the
// version it was built from) AND written to `dist/version.json` (which the app polls). When a new
// deploy publishes a different id, a still-open tab sees version.json change and prompts a refresh —
// proactive detection of a stale bundle, complementing the reactive chunk-404 recovery (RouteError).
// A timestamp is enough: it is monotonic across builds and needs no git. (Config runs in Node, where
// Date.now() is available — unlike a workflow script.)
const BUILD_ID = String(Date.now());

// GitHub Pages has no server-side SPA rewrite: a deep link like `/cadenza/basics` 404s. Pages serves
// `404.html` for any unknown path, so copying the built `index.html` to `404.html` makes every deep
// link boot the same SPA (React Router then resolves the path client-side).
function spaFallback(): Plugin {
  return {
    name: "spa-404-fallback",
    apply: "build",
    // `closeBundle` runs even when the build FAILED before writing `index.html`. Guard on the file
    // existing so this fallback never fails a build itself — and, crucially, never MASKS the real
    // build error with a confusing `ENOENT … 404.html`.
    closeBundle() {
      const out = resolve(fileURLToPath(new URL("./dist", import.meta.url)));
      const index = resolve(out, "index.html");
      if (!existsSync(index)) return;
      copyFileSync(index, resolve(out, "404.html"));
    },
  };
}

// Write `dist/version.json` = `{ "version": <BUILD_ID> }`. The running app polls this (on focus / route
// change) and, when it differs from the id baked into the bundle, prompts a refresh — so a reader on an
// old tab learns a new version shipped before they hit a 404. Emitted at build only (dev serves the
// same value through the `define` below; the poll fetch simply 404s in dev, which the client ignores).
function emitVersion(): Plugin {
  return {
    name: "emit-version-json",
    apply: "build",
    closeBundle() {
      const out = resolve(fileURLToPath(new URL("./dist", import.meta.url)));
      if (!existsSync(out)) return;
      writeFileSync(resolve(out, "version.json"), JSON.stringify({ version: BUILD_ID }) + "\n");
    },
  };
}

// The run worker transpiles a compiled component to browser-runnable JS with `@bytecodealliance/
// jco-transpile`. Its real work is done by vendored WASM (browser-safe), but its module graph
// statically imports Node-only bits that never run when we transpile WITHOUT minify/optimize/asmjs:
//   - `oxc-minify` is a native addon (only called under `opts.minify`) — alias to a throwing stub.
//   - `binaryen`'s wasm-opt CLI is only reached under `opts.optimize` — left to Vite (unused path).
// Aliasing the import so it RESOLVES (to a stub) is enough; the code path is never executed.
const stub = fileURLToPath(new URL("./src/runner/oxc-minify-stub.ts", import.meta.url));
// jco-transpile top-level-imports several Node built-ins with NAMED imports (e.g. `import { platform }
// from 'node:process'`). Vite's default `node:*` externalization is a default-only empty stub, so the
// named imports fail to bind at build time. These paths never execute in the guide (no optimize/minify/
// asm.js/writeFiles), so we alias the offending built-ins to a local stub that provides the names.
const nodeStub = fileURLToPath(new URL("./src/runner/node-stubs.ts", import.meta.url));

// Base public path. GitHub Pages serves a project site under `/<repo>/`, so CI sets
// `VITE_BASE=/cadenza/`; local dev and a user/org page keep the default `/`. The router reads the
// same value at runtime (import.meta.env.BASE_URL) so links resolve under the base.
const base = process.env.VITE_BASE ?? "/";

export default defineConfig({
  base,
  // Bake the build id into the bundle so the running app can compare itself to the polled version.json.
  define: {
    __BUILD_ID__: JSON.stringify(BUILD_ID),
  },
  plugins: [react(), tailwindcss(), spaFallback(), emitVersion()],
  resolve: {
    alias: {
      "oxc-minify": stub,
      "node:process": nodeStub,
      "node:os": nodeStub,
      "node:child_process": nodeStub,
      "node:fs/promises": nodeStub,
      "node:util": nodeStub,
      "node:path": nodeStub,
      "node:url": nodeStub,
      "node:buffer": nodeStub,
    },
  },
  optimizeDeps: {
    // jco/jco-transpile do their own dynamic wasm loading the dep-optimizer would break. manifold-3d is
    // the same shape: its emscripten glue fetches `manifold.wasm` relative to its own module URL, and the
    // dep-optimizer bundles the JS into `.vite/deps/` WITHOUT copying that wasm beside it (esbuild can't
    // follow the runtime `locateFile` path) — so `/node_modules/.vite/deps/manifold.wasm` 404s to the SPA
    // shell and /cad meshing dies in dev. Excluding it serves manifold from `node_modules/manifold-3d/`
    // where `manifold.wasm` sits next to `manifold.js`, so the relative fetch resolves.
    exclude: [
      "@bytecodealliance/jco",
      "@bytecodealliance/jco-transpile",
      "@bytecodealliance/preview2-shim",
      "manifold-3d",
    ],
  },
  worker: {
    format: "es",
  },
  build: {
    target: "es2022",
    rollupOptions: {
      output: {
        // Split the big, rarely-changing vendor libraries into their own chunks. Without this they're
        // hoisted into the single entry chunk (~748 kB), which (a) bloats first paint and (b) busts the
        // WHOLE bundle's cache on any app change. Isolating them means a chapter/prose edit re-hashes
        // only the small app chunk while readers keep the cached CodeMirror/React across deploys.
        //   - codemirror: the editor stack (@codemirror/*, @uiw/react-codemirror, @lezer/*) — pulled in
        //     eagerly by every <Runnable>; the single largest slice of the entry chunk.
        //   - react-vendor: react + react-dom + react-router, the framework core.
        // NOTE: the 3D stack (three/@react-three/manifold-3d) is deliberately NOT manualChunk'd — the
        // lazy CadPage route already splits it into its own chunk, and forcing a manual chunk pulled a
        // shared Vite helper into it, which made the ENTRY depend on (and modulepreload) the 908 kB 3D
        // bundle. Leaving Rollup's automatic lazy-route splitting alone keeps three OFF first paint.
        manualChunks(id) {
          if (/node_modules\/(@codemirror|@uiw|@lezer)\//.test(id)) return "codemirror";
          if (/node_modules\/(react|react-dom|react-router|react-router-dom|scheduler)\//.test(id))
            return "react-vendor";
          return undefined;
        },
      },
    },
  },
});

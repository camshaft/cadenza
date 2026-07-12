import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";
import { copyFileSync } from "node:fs";
import { resolve } from "node:path";

// GitHub Pages has no server-side SPA rewrite: a deep link like `/cadenza/basics` 404s. Pages serves
// `404.html` for any unknown path, so copying the built `index.html` to `404.html` makes every deep
// link boot the same SPA (React Router then resolves the path client-side).
function spaFallback(): Plugin {
  return {
    name: "spa-404-fallback",
    apply: "build",
    closeBundle() {
      const out = resolve(fileURLToPath(new URL("./dist", import.meta.url)));
      copyFileSync(resolve(out, "index.html"), resolve(out, "404.html"));
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
  plugins: [react(), tailwindcss(), spaFallback()],
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
    // jco/jco-transpile do their own dynamic wasm loading the dep-optimizer would break.
    exclude: [
      "@bytecodealliance/jco",
      "@bytecodealliance/jco-transpile",
      "@bytecodealliance/preview2-shim",
    ],
  },
  worker: {
    format: "es",
  },
  build: {
    target: "es2022",
  },
});

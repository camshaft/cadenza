/// <reference types="vite/client" />

// The per-build id, injected by Vite's `define` (see vite.config.ts). The app compares it to the polled
// `version.json` to detect that a newer version was deployed while this tab stayed open.
declare const __BUILD_ID__: string;

// `?url` asset imports resolve to a string URL Vite fingerprints (the compiler/runtime wasm).
declare module "*.wasm?url" {
  const url: string;
  export default url;
}

// `?raw` imports resolve to the file's text. `.cdz` is the Cadenza source extension (the staged CAD
// library `exact.cdz`, preloaded by /cad); vite/client's built-in `?raw` types don't cover it.
declare module "*.cdz?raw" {
  const src: string;
  export default src;
}

// The jco-transpile package ships no bundled types under our resolution; declare the surface we use.
declare module "@bytecodealliance/jco-transpile" {
  export function transpileBytes(
    component: Uint8Array,
    opts?: {
      name?: string;
      instantiation?: "async" | "sync";
      minify?: boolean;
      optimize?: boolean;
      map?: [string, string][];
      [key: string]: unknown;
    },
  ): Promise<{ files: Record<string, Uint8Array>; imports: string[]; exports: [string, string][] }>;
}

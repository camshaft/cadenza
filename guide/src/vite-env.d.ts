/// <reference types="vite/client" />

// `?url` asset imports resolve to a string URL Vite fingerprints (the compiler/runtime wasm).
declare module "*.wasm?url" {
  const url: string;
  export default url;
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

/// Client-only shareable links: the source + surface are LZ-compressed into the URL hash (`#code/…`),
/// exactly as the TypeScript Playground does. No backend, no gist token — a static-hosted site can
/// share a program by URL alone. The hash never hits the server and doesn't trigger a navigation.

// lz-string is a CommonJS module. Import shape matters and is easy to get wrong (it's been flagged
// twice in review):
//   - a NAMED import (`import { compress… }`) fails a strict ESM loader ("Named export not found") and
//     blocks node-based unit tests;
//   - a NAMESPACE import (`import * as LZString`) type-checks, but at NODE runtime the fns live under
//     `.default`, so `LZString.compress…` is `undefined` — breaks the tests;
//   - the DEFAULT import (below) gets the whole module.exports object and works under BOTH Vite and node.
// It type-checks under `verbatimModuleSyntax:true` WITHOUT `esModuleInterop` because tsconfig uses
// `moduleResolution:"bundler"`, which implies `allowSyntheticDefaultImports`. Keep this a default import.
import LZString from "lz-string";
import type { Surface } from "../compiler/client.ts";

const { compressToEncodedURIComponent, decompressFromEncodedURIComponent } = LZString;

export interface Shared {
  s: Surface;
  src: string;
}

/// Build a shareable URL (into the current page) with the program in its hash.
export function encodeShareUrl(shared: Shared): string {
  const payload = compressToEncodedURIComponent(JSON.stringify(shared));
  return `${location.origin}${location.pathname}#code/${payload}`;
}

/// The hash fragment (without the leading `#`), for pushing to a router/history.
export function encodeShareHash(shared: Shared): string {
  return `code/${compressToEncodedURIComponent(JSON.stringify(shared))}`;
}

/// Decode a shared program from the current URL hash, or null if there isn't one / it's malformed.
export function decodeShareHash(hash: string): Shared | null {
  const m = hash.replace(/^#/, "").match(/^code\/(.+)$/);
  if (!m) return null;
  try {
    const json = decompressFromEncodedURIComponent(m[1]);
    if (!json) return null;
    const parsed = JSON.parse(json) as Shared;
    if ((parsed.s === "ml" || parsed.s === "sexpr") && typeof parsed.src === "string") return parsed;
    return null;
  } catch {
    return null;
  }
}

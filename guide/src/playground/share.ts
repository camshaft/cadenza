/// Client-only shareable links: the source + surface are LZ-compressed into the URL hash (`#code/…`),
/// exactly as the TypeScript Playground does. No backend, no gist token — a static-hosted site can
/// share a program by URL alone. The hash never hits the server and doesn't trigger a navigation.

// lz-string is a CommonJS module: a NAMED import (`import { compress… }`) fails under a strict ESM
// loader ("Named export not found") — Vite's bundler papers over it, but it's fragile and blocks
// node-based unit tests. The default import gets the whole module.exports object, which works under
// both Vite and node.
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

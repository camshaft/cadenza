/// Client-only shareable links for the PLAYGROUND: the source + surface are LZ-compressed into the URL
/// hash (`#code/…`), exactly as the TypeScript Playground does. No backend, no gist token — a static-hosted
/// site can share a program by URL alone. The hash never hits the server and doesn't trigger a navigation.
///
/// This is now the `code`-kind specialization over the shared `shareLink.ts` (which /cad + notebook also
/// build on, operator #6820/#7184). The public API here is unchanged (encodeShareUrl/encodeShareHash/
/// decodeShareHash over a `Shared`), so PlaygroundPage + its test are untouched; only the encoding moved to
/// the generic module. The `#code/` hash format is byte-identical, so existing shared links still resolve.

import {
  encodeShareHash as encodeKind,
  encodeShareUrl as encodeKindUrl,
  decodeShareHash as decodeKind,
} from "../share/shareLink.ts";
import type { Surface } from "../compiler/client.ts";

const KIND = "code";

export interface Shared {
  s: Surface;
  src: string;
}

/// Runtime shape guard: a valid playground `Shared` is a known surface + a string source.
function isShared(v: unknown): v is Shared {
  const o = v as Shared;
  return !!o && (o.s === "ml" || o.s === "sexpr") && typeof o.src === "string";
}

/// Build a shareable URL (into the current page) with the program in its hash.
export function encodeShareUrl(shared: Shared): string {
  return encodeKindUrl(KIND, shared);
}

/// The hash fragment (without the leading `#`), for pushing to a router/history.
export function encodeShareHash(shared: Shared): string {
  return encodeKind(KIND, shared);
}

/// Decode a shared program from the current URL hash, or null if there isn't one / it's malformed.
export function decodeShareHash(hash: string): Shared | null {
  return decodeKind(hash, KIND, isShared);
}

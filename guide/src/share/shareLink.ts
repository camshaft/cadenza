/// Client-only shareable links, generalized across the guide's interactive surfaces (playground / cad /
/// notebook). A typed payload is LZ-compressed into the URL hash as `#<kind>/<payload>` — exactly the
/// TypeScript-Playground pattern the playground already used, now shared so /cad and the notebook get the
/// same no-backend, no-navigation share (operator ask #7184). The hash never hits the server.
///
/// The `kind` prefix namespaces the payload so a decoder only accepts hashes meant for it: the playground
/// reads `#code/…`, /cad reads `#cad/…`, the notebook reads `#nb/…`. Each surface supplies a `validate`
/// that narrows the decompressed JSON to its own payload type (rejecting a wrong-shape or wrong-kind hash),
/// so a decode is total (returns null on anything malformed) — the surface never crashes on a hand-edited
/// or stale link. The playground's original `share.ts` is a thin `code`-kind specialization over this.

import LZString from "lz-string";

// lz-string is CommonJS; the DEFAULT import is the shape that works under BOTH Vite and node (a named or
// namespace import breaks one of them — see the long note this replaces in playground/share.ts). Keep it.
const { compressToEncodedURIComponent, decompressFromEncodedURIComponent } = LZString;

/// The hash fragment (without the leading `#`) for a payload of the given kind: `<kind>/<lz-payload>`.
/// Push this to a router/history, or prefix with `location.origin+pathname+#` for a full URL.
export function encodeShareHash<T>(kind: string, payload: T): string {
  return `${kind}/${compressToEncodedURIComponent(JSON.stringify(payload))}`;
}

/// A full shareable URL into the current page with the payload in its hash.
export function encodeShareUrl<T>(kind: string, payload: T): string {
  return `${location.origin}${location.pathname}#${encodeShareHash(kind, payload)}`;
}

/// Decode a payload of `kind` from a URL hash, or null if the hash isn't this kind / is malformed / fails
/// `validate`. `validate` narrows the parsed JSON to `T` (a runtime shape guard) — return false to reject a
/// well-formed-but-wrong-shape payload. Total: never throws (a corrupt LZ payload or bad JSON → null).
export function decodeShareHash<T>(hash: string, kind: string, validate: (v: unknown) => v is T): T | null {
  const m = hash.replace(/^#/, "").match(/^([^/]+)\/(.+)$/);
  if (!m || m[1] !== kind) return null;
  try {
    const json = decompressFromEncodedURIComponent(m[2]);
    if (!json) return null;
    const parsed: unknown = JSON.parse(json);
    return validate(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

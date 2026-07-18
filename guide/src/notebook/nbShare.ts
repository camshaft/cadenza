/// Share-link payload for the NOTEBOOK — the operator's ask (#7184): a Share button that encodes the
/// current notebook into a URL others can open to see the same document, like the playground/cad share.
/// Built on the generic `shareLink.ts` (kind `"nb"`, so an `#nb/…` hash only decodes here). Pure + dep-free.
///
/// The notebook's canonical representation IS its serialized markdown (v-notebook's `serializeDocument` —
/// the same string NotebookPage holds as its source of truth + round-trips through `parseDocument`), so the
/// share payload is just that markdown plus the surface it's authored in. Cell ids are UI-only and NOT
/// serialized, so a shared URL carries no stale keys — the receiver re-parses + re-assigns ids on load, the
/// same path as a normal load. (Confirmed with v-notebook: serializeDocument/parseDocument is a stable
/// public contract, so reusing the serialized doc as the share payload is correct, not a coupling leak.)

import { encodeShareUrl, decodeShareHash } from "../share/shareLink.ts";
import type { Surface } from "../compiler/client.ts";

const KIND = "nb";

/// A shared notebook: the serialized document markdown + the surface its code cells are authored in.
export interface NbShared {
  s: Surface;
  doc: string;
}

/// Runtime shape guard — a valid payload has a known surface + a string document.
function isNbShared(v: unknown): v is NbShared {
  const o = v as NbShared;
  return !!o && (o.s === "ml" || o.s === "sexpr") && typeof o.doc === "string";
}

/// A full shareable URL for the current notebook (into the current page, doc in the `#nb/` hash).
export function encodeNbShareUrl(shared: NbShared): string {
  return encodeShareUrl(KIND, shared);
}

/// Decode a shared notebook from a URL hash, or null if the hash isn't an `#nb/` link / is malformed.
export function decodeNbShare(hash: string): NbShared | null {
  return decodeShareHash(hash, KIND, isNbShared);
}

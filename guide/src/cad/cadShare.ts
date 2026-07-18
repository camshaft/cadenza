/// Share-link payload for /cad — the operator's ask (#7184): a Share button that encodes the current CAD
/// model into a URL others can open to see the same thing, exactly like the playground's share. Built on
/// the generic `shareLink.ts` (kind `"cad"`, so a `#cad/…` hash only decodes here). Pure + dep-free (no
/// React) so it's unit-testable and the CadPage component just calls encode/decode.
///
/// The payload carries what fully reconstructs the /cad view: the editor SOURCE, the SURFACE it's written
/// in (ML vs s-expr — the buffer can't be reinterpreted across surfaces), and — for a parametric model —
/// the current `@param` slider values as EXACT fractions, so a shared parametric link restores the exact
/// dragged dimensions (a shared 7/2 thickness comes back 7/2, not a re-defaulted 5). Params are optional
/// (a plain model has none).

import { encodeShareUrl, decodeShareHash } from "../share/shareLink.ts";
import type { Surface } from "../compiler/client.ts";
import type { Frac } from "./ParametricControls.tsx";

const KIND = "cad";

/// A shared /cad model: the source, its surface, and (optional) each `@param`'s exact fraction value.
export interface CadShared {
  s: Surface;
  src: string;
  /// Per-param exact value (name → {num,den}); omitted / empty for a non-parametric model.
  params?: Record<string, Frac>;
}

/// Runtime shape guard — a valid payload has a known surface + string source; `params` (if present) is an
/// object of {num,den} pairs. Kept lenient on params (a malformed params entry just won't drive a slider).
function isCadShared(v: unknown): v is CadShared {
  const o = v as CadShared;
  if (!o || (o.s !== "ml" && o.s !== "sexpr") || typeof o.src !== "string") return false;
  if (o.params !== undefined) {
    if (typeof o.params !== "object" || o.params === null) return false;
    for (const k of Object.keys(o.params)) {
      const f = o.params[k] as Frac;
      if (!f || typeof f.num !== "number" || typeof f.den !== "number") return false;
    }
  }
  return true;
}

/// A full shareable URL for the current /cad model (into the current page, program in the `#cad/` hash).
export function encodeCadShareUrl(shared: CadShared): string {
  return encodeShareUrl(KIND, shared);
}

/// Decode a shared /cad model from a URL hash, or null if the hash isn't a `#cad/` link / is malformed.
export function decodeCadShare(hash: string): CadShared | null {
  return decodeShareHash(hash, KIND, isCadShared);
}

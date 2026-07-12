/// The global syntax mode — which concrete surface every code sample is displayed in.
///
/// Cadenza is one homoiconic binary AST with lossless text projections, so switching surface never
/// changes a program; the guide re-serializes each sample through a different printer. The choice is
/// a single page-global mode (a header segmented control), persisted to localStorage and reflected in
/// the URL `?syntax=` so a shared link opens in the same surface. On load the URL param wins over
/// storage (matching the Docusaurus tab convention).

import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { Surface } from "../compiler/client.ts";

export type { Surface };

export const SURFACES: { id: Surface; label: string }[] = [
  { id: "ml", label: "Conventional" },
  { id: "sexpr", label: "S-expression" },
];

const STORAGE_KEY = "cadenza.syntax";
const DEFAULT: Surface = "ml";

function isSurface(v: string | null): v is Surface {
  return v === "ml" || v === "sexpr";
}

function initialSurface(): Surface {
  const url = new URLSearchParams(window.location.search).get("syntax");
  if (isSurface(url)) return url; // URL wins on load
  const stored = localStorage.getItem(STORAGE_KEY);
  if (isSurface(stored)) return stored;
  return DEFAULT;
}

interface SyntaxState {
  surface: Surface;
  setSurface: (s: Surface) => void;
}

const SyntaxCtx = createContext<SyntaxState | null>(null);

export function SyntaxProvider({ children }: { children: ReactNode }) {
  const [surface, setSurfaceState] = useState<Surface>(initialSurface);

  const setSurface = useCallback((s: Surface) => {
    setSurfaceState(s);
    localStorage.setItem(STORAGE_KEY, s);
    const url = new URL(window.location.href);
    url.searchParams.set("syntax", s);
    // Replace (not push) and don't touch the hash — no scroll jump, no history spam.
    window.history.replaceState(window.history.state, "", url);
  }, []);

  // Keep the URL in sync on first mount so a shared link without ?syntax reflects the active choice.
  useEffect(() => {
    const url = new URL(window.location.href);
    if (url.searchParams.get("syntax") !== surface) {
      url.searchParams.set("syntax", surface);
      window.history.replaceState(window.history.state, "", url);
    }
    // run once on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const value = useMemo(() => ({ surface, setSurface }), [surface, setSurface]);
  return <SyntaxCtx.Provider value={value}>{children}</SyntaxCtx.Provider>;
}

export function useSyntax(): SyntaxState {
  const ctx = useContext(SyntaxCtx);
  if (!ctx) throw new Error("useSyntax must be used within a SyntaxProvider");
  return ctx;
}

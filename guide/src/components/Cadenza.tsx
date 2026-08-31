/// Surface-aware INLINE Cadenza code, authored in chapters as `(cdz "<sexpr>")` (vs plain `(c …)`, which
/// stays literal — CLI commands, error codes, other-surface snippets, or an s-expr shown for pedagogical
/// contrast). The body is authored s-expr. Because the guide codegen (xtask-codegen-guide) only links the
/// s-expr printer, it CANNOT pre-render the conventional (ml) surface — so, like <Runnable>, we render the
/// conventional surface at RUNTIME via the compiler client:
///   • s-expr surface  → show the authored text verbatim (no compiler round-trip).
///   • conventional/ml → renderSyntax(sexpr, "sexpr", "ml"), memoized per source, with a graceful fallback
///     to the s-expr text while the compiler warms up or if the convert fails.
/// The visual is the shared inline-code <C> chip, so a `(cdz …)` reads identically to a `(c …)` — it just
/// tracks the page-global surface toggle (SyntaxContext).
import { useEffect, useState } from "react";
import { C } from "./Prose.tsx";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { renderSyntax } from "../compiler/client.ts";

// Process-lifetime memo: a given inline s-expr renders to the same ml text every time, and the same snippet
// recurs across a chapter, so cache globally (keyed by source) to avoid re-converting on every mount/toggle.
const mlCache = new Map<string, string>();

export function Cadenza({ children }: { children: string }) {
  const sexpr = String(children);
  const { surface } = useSyntax();
  const [ml, setMl] = useState<string | null>(() => mlCache.get(sexpr) ?? null);

  useEffect(() => {
    // Only the conventional surface needs a convert; s-expr is shown verbatim. Skip if already cached.
    if (surface !== "ml" || mlCache.has(sexpr)) return;
    let cancelled = false;
    renderSyntax(sexpr, "sexpr", "ml").then(
      (out) => {
        if (cancelled) return;
        mlCache.set(sexpr, out);
        setMl(out);
      },
      () => {}, // compiler not ready / unparseable → keep the s-expr fallback below
    );
    return () => {
      cancelled = true;
    };
  }, [sexpr, surface]);

  const text = surface === "ml" ? (mlCache.get(sexpr) ?? ml ?? sexpr) : sexpr;
  return <C>{text}</C>;
}

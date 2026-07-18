/// Shared KaTeX math rendering for the guide — `<Math tex>` (inline) and `<MathBlock tex>` (display).
/// Used by BOTH the notebook (its `$…$`/`$$…$$` prose spans) and chapters (prose math). Owned by this
/// vertical (the app-shell/bundle seam); v-notebook owns the parse that produces the `tex` strings.
///
/// LOAD-BEARING (why this is the bundle owner's component): KaTeX + its CSS is heavy (~270KB JS + fonts),
/// and chapters render on EAGER routes (not a lazy full-screen route like /cad where three.js hides). So a
/// static `import "katex"` would pull KaTeX into the guide's FIRST-PAINT bundle — a real regression. This
/// component LAZY-loads katex on first mount via dynamic `import()` (of both the library AND its CSS), so
/// KaTeX lands in its OWN async chunk, fetched only when a page actually renders math. Until it resolves,
/// it shows the raw TeX source as a skeleton (so there's never a blank flash, and it degrades gracefully if
/// the chunk fails to load). `throwOnError: false` makes a malformed expression render its source in red
/// rather than crash the page.

import { useEffect, useState } from "react";

/// The lazily-loaded katex renderer, shared across all <Math> instances (loaded once, then cached). A
/// module-level promise so concurrent mounts don't each kick off a separate import.
let katexPromise: Promise<typeof import("katex").default> | null = null;
function loadKatex(): Promise<typeof import("katex").default> {
  if (!katexPromise) {
    katexPromise = Promise.all([
      import("katex"),
      // The stylesheet — side-effect import; Vite bundles it into the same async chunk as katex.
      import("katex/dist/katex.min.css"),
    ]).then(([mod]) => mod.default);
  }
  return katexPromise;
}

/// Render `tex` to a KaTeX HTML string, or null until katex has loaded / on failure (caller shows the raw
/// source meanwhile). `displayMode` is block (centered, own line) vs inline.
function useKatexHtml(tex: string, displayMode: boolean): string | null {
  const [html, setHtml] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    loadKatex()
      .then((katex) => {
        if (cancelled) return;
        // throwOnError:false → a bad expression renders its source (in KaTeX's error color), never throws.
        setHtml(katex.renderToString(tex, { displayMode, throwOnError: false }));
      })
      .catch(() => {
        // Chunk failed to load (offline / blocked) — leave html null so the raw-TeX fallback shows.
        if (!cancelled) setHtml(null);
      });
    return () => {
      cancelled = true;
    };
  }, [tex, displayMode]);
  return html;
}

/// Inline math: `<Math tex="a^2 + b^2" />`. Renders KaTeX once loaded; shows the raw TeX (in a monospace
/// hint) until then / on failure — so the reader always sees *something* and math never blanks the line.
export function Math({ tex }: { tex: string }) {
  const html = useKatexHtml(tex, false);
  if (html === null) return <code className="text-slate-300">{tex}</code>;
  return <span dangerouslySetInnerHTML={{ __html: html }} />;
}

/// Display (block) math: `<MathBlock tex="\int_0^1 x\,dx" />` — centered on its own line.
export function MathBlock({ tex }: { tex: string }) {
  const html = useKatexHtml(tex, true);
  if (html === null) return <pre className="overflow-x-auto text-slate-300">{tex}</pre>;
  return <div className="overflow-x-auto" dangerouslySetInnerHTML={{ __html: html }} />;
}

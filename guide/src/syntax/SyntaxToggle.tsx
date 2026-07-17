/// The global syntax segmented control (header). A single exclusive page-global mode: flipping it
/// re-renders every `Runnable` on the page into the chosen surface. A segmented control (rather than
/// per-block tabs) reads as "one global mode", which is the intended mental model.

import { SURFACES, useSyntax } from "./SyntaxContext.tsx";

export function SyntaxToggle() {
  const { surface, setSurface } = useSyntax();
  return (
    <div
      role="radiogroup"
      aria-label="Display syntax"
      className="inline-flex rounded-lg border border-slate-700/70 bg-slate-800/60 p-0.5"
    >
      {SURFACES.map((s) => {
        const active = s.id === surface;
        return (
          <button
            key={s.id}
            role="radio"
            aria-checked={active}
            onClick={() => setSurface(s.id)}
            className={
              // Mobile touch target: a 44px-tall segment below `sm` (the touch guideline), compact at sm+.
              "flex min-h-11 items-center rounded-md px-2 text-xs font-medium transition sm:min-h-0 sm:px-3 sm:py-1 " +
              (active
                ? "bg-cadenza-600 text-white shadow"
                : "text-slate-400 hover:text-slate-200")
            }
          >
            {/* Short labels on phones (ML / S-expr), full on wider screens. */}
            <span className="sm:hidden">{s.short}</span>
            <span className="hidden sm:inline">{s.label}</span>
          </button>
        );
      })}
    </div>
  );
}

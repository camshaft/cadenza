/// Small prose primitives so chapters read consistently without pulling in an MDX pipeline. Chapters
/// are plain TSX (they embed <Runnable>), and these give headings/paragraphs/inline-code a shared look.

import type { ReactNode } from "react";

// Surface-aware inline Cadenza code, authored as (cdz …). Re-exported here so the chapter codegen can pull
// it from the same Prose.tsx import line as the other inline primitives (C/em/…). Its own module carries the
// compiler-client + SyntaxContext deps; C below is a hoisted function declaration, so the Cadenza→Prose
// import it makes is a safe ES-module cycle.
export { Cadenza } from "./Cadenza.tsx";

export function H1({ children }: { children: ReactNode }) {
  return <h1 className="mb-2 text-3xl font-bold tracking-tight text-slate-100">{children}</h1>;
}

export function Lede({ children }: { children: ReactNode }) {
  return <p className="mb-6 text-lg text-slate-400">{children}</p>;
}

export function H2({ children }: { children: ReactNode }) {
  return <h2 className="mt-10 mb-3 text-xl font-semibold text-slate-200">{children}</h2>;
}

export function P({ children }: { children: ReactNode }) {
  return <p className="my-4 leading-7 text-slate-300">{children}</p>;
}

export function C({ children }: { children: ReactNode }) {
  return (
    <code className="rounded bg-slate-800 px-1.5 py-0.5 font-mono text-[0.85em] text-cadenza-300">
      {children}
    </code>
  );
}

export function Note({ children }: { children: ReactNode }) {
  return (
    <div className="my-5 rounded-lg border-l-2 border-cadenza-500 bg-slate-800/40 px-4 py-3 text-sm text-slate-300">
      {children}
    </div>
  );
}

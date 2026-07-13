/// Shared editor logic for `Runnable` and `Exercise`: it owns the editor text, keeps it in the
/// reader's globally-chosen surface (re-serializing through the compiler when the toggle flips,
/// preserving edits), and exposes `run()` which compiles + executes the current buffer and returns a
/// normalized outcome. The two components layer their own UI (a value pane vs. a graded check) on top.

import { useCallback, useEffect, useRef, useState } from "react";
import { compile, renderSyntax } from "../compiler/client.ts";
import { run as runComponent, type RunOutcome } from "../runner/client.ts";
import { useSyntax, type Surface } from "../syntax/SyntaxContext.tsx";
import type { Diag } from "../compiler/client.ts";

/// The outcome of compiling + running the current buffer — a superset of the runner's `RunOutcome`
/// that also carries a compile decline (diagnostics, no component).
export type EditorOutcome =
  | { kind: "value"; text: string }
  | { kind: "trap"; message: string }
  | { kind: "timeout" }
  | { kind: "declined"; diags: Diag[] };

/// Wrap a bare expression into a minimal runnable module in the given surface. The compiler needs an
/// `(export …)`; a bare expression alone declines. Authored snippets are usually bare expressions.
/// Wrap an example snippet into a compilable module, so a chapter can show just the interesting part.
/// Three shapes are recognized so `module m { … }` / `(export main)` boilerplate only appears when the
/// author actually wrote it:
///   1. Already a full `module` — left untouched.
///   2. DEFINITIONS (starts with `def`/`type`) — one or more top-level defs with a `main` among them;
///      wrapped in a module + `(export main)`. This is the common "helper + main" example.
///   3. A bare EXPRESSION — wrapped as the body of `main` (+ module + export).
export function wrapModule(src: string, surface: Surface): string {
  const trimmed = src.trim();
  if (surface === "sexpr") {
    if (/^\(module\b/.test(trimmed)) return trimmed;
    if (/^\((def|type)\b/.test(trimmed)) return `(module m ${trimmed} (export main))`;
    return `(module m (def (main) ${trimmed}) (export main))`;
  }
  if (/^module\b/.test(trimmed)) return trimmed;
  if (/^(def|type)\b/.test(trimmed)) return `module m {\n${indent(trimmed)}\n  export(main)\n}`;
  return `module m {\n  def main() = ${trimmed}\n  export(main)\n}`;
}

/// Indent each line of a multi-line ML definitions block by two spaces (module-body indentation).
function indent(src: string): string {
  return src
    .split("\n")
    .map((line) => (line ? `  ${line}` : line))
    .join("\n");
}

/// Strip the `module m { … }` / `(module m … (export main))` scaffolding a `wrapModule` added, back to
/// the bare definitions (or expression), for DISPLAY. The inverse of `wrapModule` over a RENDERED
/// module; used so the surface toggle can round-trip a defs-only snippet (which isn't a single form)
/// through the compiler by wrapping first, rendering, then stripping. Returns the input unchanged if
/// it doesn't look like a generated wrapper (so a hand-written full module the author showed is kept).
export function stripModule(rendered: string, surface: Surface): string {
  const t = rendered.trim();
  if (surface === "sexpr") {
    // `(module m <body…> (export main))` → the body, minus a trailing `(export …)`.
    const m = /^\(module\s+\w+\s+([\s\S]*)\)\s*$/.exec(t);
    if (!m) return rendered;
    let body = m[1].trim().replace(/\(export\s+[^)]*\)\s*$/, "").trim();
    // A single `(def (main) <expr>)` that we synthesized for a bare expression → unwrap to the expr.
    const bare = /^\(def\s+\(main\)\s+([\s\S]*)\)$/.exec(body);
    if (bare && !/\(def\b|\(type\b/.test(bare[1])) return bare[1].trim();
    return body;
  }
  // ML: `module m {\n <body> \n}` → dedented body minus the `export(...)` line.
  const m = /^module\s+\w+\s*\{\s*\n([\s\S]*)\n\s*\}\s*$/.exec(t);
  if (!m) return rendered;
  const lines = m[1].split("\n").filter((l) => !/^\s*export\(/.test(l));
  const dedented = dedent(lines.join("\n"));
  // Unwrap a synthesized `def main() = <expr>` (single def, no helpers) back to the expression.
  const bare = /^def\s+main\(\)\s*=\s*([\s\S]*)$/.exec(dedented.trim());
  if (bare && !/^\s*(def|type)\b/m.test(bare[1])) return bare[1].trim();
  return dedented;
}

/// Remove the common leading indentation from a block (the two spaces `wrapModule` added, plus any).
function dedent(src: string): string {
  const lines = src.split("\n");
  const indents = lines.filter((l) => l.trim()).map((l) => l.match(/^ */)![0].length);
  const min = indents.length ? Math.min(...indents) : 0;
  return lines.map((l) => l.slice(min)).join("\n");
}

/// Re-render a DISPLAY snippet from one surface to another for the syntax toggle. A defs-only or bare
/// snippet isn't a single parseable form, so we wrap it into a module, render THAT (a single form), and
/// strip the wrapper back off — round-tripping through the compiler without exposing the scaffolding.
/// A `wrap={false}` example (a full module the author wrote) renders directly.
export async function renderSnippet(
  text: string,
  from: Surface,
  to: Surface,
  wrap: boolean,
): Promise<string> {
  if (from === to) return text;
  if (!wrap) return renderSyntax(text, from, to);
  const wrapped = wrapModule(text, from);
  const rendered = await renderSyntax(wrapped, from, to);
  return stripModule(rendered, to);
}

export interface CadenzaEditor {
  /** Current editor text, in `surface`. */
  text: string;
  setText: (t: string) => void;
  /** The surface the text is currently shown in. */
  surface: Surface;
  /** Reset the buffer to `source` (re-rendered to the active surface) and clear any result. */
  reset: () => void;
  /** Compile + run the current buffer, returning a normalized outcome. */
  run: () => Promise<EditorOutcome>;
}

/// `source` is authored once in `authoredIn`; the hook keeps the live text in the active surface.
/// `wrap` (default true) wraps a bare expression into a module before compiling.
export function useCadenzaEditor(
  source: string,
  authoredIn: Surface = "sexpr",
  wrap = true,
): CadenzaEditor {
  const { surface } = useSyntax();
  const [text, setText] = useState(source);
  // The surface the editor text currently reflects — so a toggle re-renders the CURRENT text
  // (preserving edits) rather than clobbering it with the original `source`.
  const shownSurface = useRef<Surface>(authoredIn);

  // On mount, convert the authored source into the active surface once.
  useEffect(() => {
    let cancelled = false;
    if (authoredIn !== surface) {
      renderSnippet(source, authoredIn, surface, wrap)
        .then((r) => {
          if (!cancelled) {
            setText(r);
            shownSurface.current = surface;
          }
        })
        .catch(() => {});
    } else {
      shownSurface.current = authoredIn;
    }
    return () => {
      cancelled = true;
    };
    // mount only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // React to a global surface change: re-render the current text into the new surface.
  useEffect(() => {
    if (surface === shownSurface.current) return;
    const from = shownSurface.current;
    let cancelled = false;
    renderSnippet(text, from, surface, wrap)
      .then((r) => {
        if (!cancelled) {
          setText(r);
          shownSurface.current = surface;
        }
      })
      .catch(() => {
        // A mid-edit unparseable buffer can't be re-rendered; keep the text but mark the surface so
        // we don't retry every keystroke.
        shownSurface.current = surface;
      });
    return () => {
      cancelled = true;
    };
  }, [surface, text]);

  const run = useCallback(async (): Promise<EditorOutcome> => {
    const program = wrap ? wrapModule(text, shownSurface.current) : text;
    const out = await compile(program, shownSurface.current);
    if (!out.component) return { kind: "declined", diags: out.diagnostics };
    const result: RunOutcome = await runComponent(out.component, shownSurface.current);
    switch (result.kind) {
      case "value":
        return { kind: "value", text: result.text };
      case "trap":
        return { kind: "trap", message: result.message };
      case "timeout":
        return { kind: "timeout" };
      case "error":
        return {
          kind: "declined",
          diags: [{ error: true, code: "", message: result.message, node: -1, from: 0, to: 0 }],
        };
    }
  }, [text, wrap]);

  const reset = useCallback(() => {
    const target = shownSurface.current;
    if (authoredIn === target) {
      setText(source);
    } else {
      renderSnippet(source, authoredIn, target, wrap)
        .then(setText)
        .catch(() => setText(source));
    }
  }, [authoredIn, source, wrap]);

  return { text, setText, surface: shownSurface.current, reset, run };
}

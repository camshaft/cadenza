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
export function wrapModule(src: string, surface: Surface): string {
  const trimmed = src.trim();
  if (surface === "sexpr") {
    if (/^\(module\b/.test(trimmed)) return trimmed;
    return `(module m (def (main) ${trimmed}) (export main))`;
  }
  if (/^module\b/.test(trimmed)) return trimmed;
  return `module m {\n  def main() = ${trimmed}\n  export(main)\n}`;
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
      renderSyntax(source, authoredIn, surface)
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
    renderSyntax(text, from, surface)
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
    const result: RunOutcome = await runComponent(out.component);
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
      renderSyntax(source, authoredIn, target)
        .then(setText)
        .catch(() => setText(source));
    }
  }, [authoredIn, source]);

  return { text, setText, surface: shownSurface.current, reset, run };
}

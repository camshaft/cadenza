/// Shared editor logic for `Runnable` and `Exercise`: it owns the editor text, keeps it in the
/// reader's globally-chosen surface (re-serializing through the compiler when the toggle flips,
/// preserving edits), and exposes `run()` which compiles + executes the current buffer and returns a
/// normalized outcome. The two components layer their own UI (a value pane vs. a graded check) on top.

import { useCallback, useEffect, useRef, useState } from "react";
import { compile, renderSyntax } from "../compiler/client.ts";
import { run as runComponent, type RunOutcome } from "../runner/client.ts";
import { useSyntax, type Surface } from "../syntax/SyntaxContext.tsx";
import type { Diag } from "../compiler/client.ts";
import { applyFix as applyFixToText } from "../playground/applyFix.ts";

/// The outcome of compiling + running the current buffer — a superset of the runner's `RunOutcome`
/// that also carries a compile decline (diagnostics, no component). A decline carries `wrapPrefixBytes`
/// — the scaffolding `wrapModule` prepended before the snippet — so a fix's byte range (over the
/// compiled text) maps back onto the editor text.
export type EditorOutcome =
  | { kind: "value"; text: string }
  | { kind: "trap"; message: string }
  | { kind: "timeout" }
  | { kind: "declined"; diags: Diag[]; wrapPrefixBytes: number };

/// Make a snippet compilable by supplying the `export` (and, for a bare expression, a `main` to hold
/// it) the compiler needs — a bare expression alone declines ("nothing is public"). Authored snippets
/// are usually bare expressions or a `helper` + `main` pair, shown WITHOUT boilerplate; this adds only
/// what's missing, at TOP LEVEL (no `module m { … }` shell — a wrapper compiles byte-identically to
/// bare top-level forms, so the shell was pure ceremony). Four shapes:
///   1. Already a full `module` / `(module …)` — the author wrote it; left untouched.
///   2. Already complete — has a top-level `export` (ML) / is a `(do …)` (s-expr) — left untouched. NOT
///      merely "contains a `;`": a `;` can be an internal `do`-sequence separator inside `def main() =
///      (…; …)`, which still needs an export.
///   3. DEFINITIONS / TOP-LEVEL STATEMENTS (leads with a top-level keyword — `def`/`type`/`effect`, or a
///      top-level statement like `Unit.define`) — a `main` is among them; append an `export`. `effect`
///      matters: an effects snippet leads with `(effect …)`, and mis-classifying it as a bare expression
///      wraps the WHOLE multi-form snippet as `(def (main) …)`, which is malformed (`def`(main(),
///      effect …, def main() …)). `Unit.define` (declare a custom unit) is the same shape: it MUST be
///      top-level (it declines inside a def body), so a `(Unit.define …) (def (main) …)` pair is a
///      definitions block, not a bare expression.
///   4. A bare EXPRESSION — becomes `def main() = <expr>` plus the export.
/// In s-expr the top-level forms must be gathered under one `(do …)` (s-expr has no bare multi-form
/// top level); in ML they are newline-separated (the surface's native top-level form).
const DECL_HEAD = "def|type|effect";
// A top-level STATEMENT that isn't a `def`/`type`/`effect` but still sits at top level (never wrapped as
// `(def (main) …)`) and needs an `export` appended. `Unit.define` declares a custom unit of measure and
// only resolves at top level. Escaped for a RegExp (the `.` is literal). Treated like a defs block.
const STMT_HEAD = "Unit\\.define";
export function wrapModule(src: string, surface: Surface): string {
  const trimmed = src.trim();
  if (surface === "sexpr") {
    if (/^\(module\b/.test(trimmed) || /^\(do\b/.test(trimmed)) return trimmed;
    if (new RegExp(`^\\((${DECL_HEAD}|${STMT_HEAD})\\b`).test(trimmed))
      return `(do ${trimmed} (export main))`;
    return `(do (def (main) ${trimmed}) (export main))`;
  }
  // Already complete — a full `module …`, or a program that already declares an `export` — is left
  // as-is. ⚠ Do NOT treat a mere `;` as "complete": a `;` can be an INTERNAL sequence separator (e.g.
  // `def main() = (module M { … }; M.f x)`), which still needs an `export` appended. Only a real
  // top-level `export` marks a snippet the author already made whole.
  if (/^module\b/.test(trimmed) || /(^|\n)\s*export\b/.test(trimmed)) return trimmed;
  if (new RegExp(`^(${DECL_HEAD}|${STMT_HEAD})\\b`).test(trimmed)) return `${trimmed}\nexport { main }`;
  return `def main() = ${trimmed}\nexport { main }`;
}

/// Strip the `export` (and synthesized `main`) that `wrapModule` supplied, back to the author's bare
/// definitions or expression, for DISPLAY — the inverse of `wrapModule` over a RENDERED program.
/// Because the wrapper adds only top-level forms (no `module` shell), this is a trailing-`export`
/// removal plus an optional lone-`def main()` unwrap — no shell to peel, no re-indentation. Returns
/// the input unchanged if it isn't a generated wrapper (so a full `module` the author wrote is kept).
export function stripModule(rendered: string, surface: Surface): string {
  const t = rendered.trim();
  // A hand-authored full module is displayed as-is.
  if (surface === "sexpr" ? /^\(module\b/.test(t) : /^module\b/.test(t)) return rendered;

  if (surface === "sexpr") {
    // `(do <form…> (export …))` → the forms, minus the trailing export. Unwrap the outer `(do …)`.
    const m = /^\(do\b([\s\S]*)\)\s*$/.exec(t);
    const body = (m ? m[1] : t).trim().replace(/\(export\s+[^)]*\)\s*$/, "").trim();
    // A synthesized single `(def (main) <expr>)` (no other defs) → unwrap to the bare expression.
    const bare = /^\(def\s+\(main\)\s+([\s\S]*)\)$/.exec(body);
    if (bare && !/\(def\b|\(type\b/.test(bare[1])) return bare[1].trim();
    return body;
  }
  // ML: top-level forms are separated by a trailing `;` and a blank line. Drop the `export { … }`
  // (or legacy `export(…)`) line, then remove the TOP-LEVEL `;` separators. A top-level separator is a
  // `;` either on an UNINDENTED line (between two top-level forms — e.g. `def helper() = 1;`) or on the
  // LAST content line (the separator that preceded the now-removed `export`). A `;` at deeper
  // indentation that isn't the last line is INSIDE a construct — the `};` closing a nested
  // `module { … };`, or a `let … in;` — and dropping it would corrupt the display into unparseable text.
  const lines = t.split("\n").filter((l) => !/^\s*export\s*[({]/.test(l));
  const lastContent = lines.reduce((acc, l, i) => (l.trim() ? i : acc), -1);
  const body = lines
    .map((l, i) => (/^\S/.test(l) || i === lastContent ? l.replace(/;\s*$/, "") : l))
    .join("\n")
    .trim();
  // A synthesized single `def main() = <expr>` (no other defs) → unwrap to the bare expression.
  // A multi-line body's continuation lines carry the `def`-body indentation; `dedent` removes it
  // uniformly (a lone `.trim()` would fix only the first line).
  const bare = /^def\s+main\(\)\s*=[^\S\n]*([\s\S]*)$/.exec(body);
  if (bare && !/^\s*(def|type)\b/m.test(bare[1])) return dedent(bare[1]).trim();
  return body;
}

/// Remove the common leading indentation from a block — used to un-indent a multi-line `def main()`
/// body when unwrapping it back to the bare expression it holds.
function dedent(src: string): string {
  const lines = src.split("\n");
  const indents = lines.filter((l) => l.trim()).map((l) => l.match(/^ */)![0].length);
  const min = indents.length ? Math.min(...indents) : 0;
  return lines.map((l) => l.slice(min)).join("\n");
}

/// Re-render a DISPLAY snippet from one surface to another for the syntax toggle. A defs-only or bare
/// snippet isn't a complete program, so we wrap it (adding the `export`/`main`), render the whole
/// program, and strip the added scaffolding back off — round-tripping through the compiler without
/// exposing it. A `wrap={false}` example (a full module the author wrote) renders directly.
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

/// The UTF-8 byte length of the scaffolding `wrapModule` prepended before the snippet — the offset
/// that maps a compiled-text byte position back onto the editor text. `wrapModule` embeds the trimmed
/// snippet verbatim, so we locate it in the wrapped output; 0 if it isn't found (already-complete).
function wrapPrefixOf(editorText: string, wrapped: string): number {
  const trimmed = editorText.trim();
  const idx = trimmed ? wrapped.indexOf(trimmed) : -1;
  return idx < 0 ? 0 : new TextEncoder().encode(wrapped.slice(0, idx)).length;
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
  /** Apply a diagnostic's structural fix to the snippet (`wrapPrefixBytes` from the decline). */
  applyFix: (d: Diag, wrapPrefixBytes: number) => void;
}

/// `source` is authored once in `authoredIn`; the hook keeps the live text in the active surface.
/// `wrap` (default true) supplies the `export`/`main` a bare snippet needs before compiling.
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
    const wrapPrefixBytes = wrap ? wrapPrefixOf(text, program) : 0;
    const out = await compile(program, shownSurface.current);
    if (!out.component) return { kind: "declined", diags: out.diagnostics, wrapPrefixBytes };
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
          diags: [{ error: true, code: "", message: result.message, node: -1, from: 0, to: 0, fix: null }],
          wrapPrefixBytes,
        };
    }
  }, [text, wrap]);

  /// Apply a diagnostic's structural fix to the snippet. The fix's byte range is over the COMPILED
  /// (wrapped) text; `wrapPrefixBytes` maps it back onto the editor text, which `setText` then updates.
  const applyFix = useCallback(
    (d: Diag, wrapPrefixBytes: number) => {
      if (!d.fix) return;
      const next = applyFixToText(text, d.fix, wrapPrefixBytes);
      if (next != null) setText(next);
    },
    [text],
  );

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

  return { text, setText, surface: shownSurface.current, reset, run, applyFix };
}

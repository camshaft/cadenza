/// Shared editor logic for `Runnable` and `Exercise`: it owns the editor text, keeps it in the
/// reader's globally-chosen surface (re-serializing through the compiler when the toggle flips,
/// preserving edits), and exposes `run()` which compiles + executes the current buffer and returns a
/// normalized outcome. The two components layer their own UI (a value pane vs. a graded check) on top.

import { useCallback, useEffect, useRef, useState } from "react";
import { compile, renderSyntax, exportTypes } from "../compiler/client.ts";
import { run as runComponent, type RunOutcome } from "../runner/client.ts";
import { formatScalarByType, resultTypeOf } from "../runner/scalarFormat.ts";
import { useSyntax, type Surface } from "../syntax/SyntaxContext.tsx";
import type { Diag } from "../compiler/client.ts";
import { applyFix as applyFixToText } from "../playground/applyFix.ts";
import { patchOnce } from "./tryChangePatch.ts";
import { wrapModule, stripModule, wrapPrefixOf, gatherTestForms, ungatherTestForms } from "./wrapModule.ts";

// Re-exported so existing importers (`Runnable`, others) keep their `./useCadenzaEditor.ts` path; the
// pure logic itself lives in `./wrapModule.ts` (React-free, so `node --test` can unit-test it).
export { wrapModule, stripModule } from "./wrapModule.ts";

/// The outcome of compiling + running the current buffer — a superset of the runner's `RunOutcome`
/// that also carries a compile decline (diagnostics, no component). A decline carries `wrapPrefixBytes`
/// — the scaffolding `wrapModule` prepended before the snippet — so a fix's byte range (over the
/// compiled text) maps back onto the editor text.
export type EditorOutcome =
  | { kind: "value"; text: string }
  | { kind: "trap"; message: string }
  | { kind: "timeout" }
  | { kind: "declined"; diags: Diag[]; wrapPrefixBytes: number };

/// Re-render a DISPLAY snippet from one surface to another for the syntax toggle. A defs-only or bare
/// snippet isn't a complete program, so we wrap it (adding the `export`/`main`), render the whole
/// program, and strip the added scaffolding back off — round-tripping through the compiler without
/// exposing it. A `wrap={false}` example (a full module the author wrote) renders directly.
///
/// SAME surface (`from === to`) is NOT a no-op: it still round-trips through the compiler's pretty-
/// printer, so an authored single-line `source` displays PRETTY (indented / line-broken by the
/// printer) rather than as the raw string the author typed. This is what makes the guide's formatting
/// uniform everywhere — every editor's initial buffer is the printer's canonical layout regardless of
/// how the example was hand-written. (A snippet that doesn't round-trip cleanly falls back to the
/// original text via the caller's `.catch`.)
///
/// The `wrap={false}` path (a `mode="test"` panel — several `@test`/`def` forms, no export/main) still
/// has to reach the pretty-printer, and `renderSyntax` renders ONE top-level form: s-expr has no bare
/// multi-form top level, so a multi-def test snippet threw "trailing input" here, the caller's `.catch`
/// kept the raw s-expr but marked the surface ML, and Run then fed s-expr to the ML parser → "expected a
/// name" (the reader-visible break on the testing page's first, multi-`@test`, examples). So gather a
/// multi-form s-expr snippet under `(do …)` before rendering and peel it back off with `stripModule` —
/// the SAME move the `check:examples` gate's `renderTestSnippet` makes, so app and gate render alike. ML
/// top level is already multi-form, so an ML snippet renders directly.
export async function renderSnippet(
  text: string,
  from: Surface,
  to: Surface,
  wrap: boolean,
): Promise<string> {
  if (!wrap) {
    // A `wrap={false}` (test) snippet is bare multi-form; gather it into one top-level form for the
    // single-form pretty-printer, then ungather. Shared with the gate so they can't diverge.
    const rendered = await renderSyntax(gatherTestForms(text, from), from, to);
    return ungatherTestForms(rendered, to);
  }
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
  /** Compile + run the buffer, returning a normalized outcome. Pass `override` to run a SPECIFIC text
   *  (in a given surface) rather than the current `text` state — needed because `run()` closes over the
   *  `text` snapshot, so `setText(x)` then `run()` in the same tick would run the STALE buffer. */
  run: (override?: { text: string; surface: Surface }) => Promise<EditorOutcome>;
  /** Apply a diagnostic's structural fix to the snippet (`wrapPrefixBytes` from the decline). */
  applyFix: (d: Diag, wrapPrefixBytes: number) => void;
  /** Swap the buffer to an authored snippet (rendered from `srcSurface` into the buffer's CURRENT
   *  surface, so the display stays in the reader's chosen syntax) and run it, atomically — the "apply
   *  this variant + show the result" primitive behind the guide's clickable "change X to Y" prose.
   *  Runs against the freshly-rendered text (not stale state) via `run`'s override. */
  applyAuthored: (authoredSrc: string, srcSurface: Surface) => Promise<EditorOutcome>;
  /** Replace the single occurrence of `find` with `replace` in the CURRENT buffer, then run —
   *  the one-token clickable-prose patch (e.g. "change the index 1 to 9"). Returns null WITHOUT running
   *  if `find` does not occur EXACTLY ONCE (0 or >1 matches) so the caller can surface a failure; the
   *  authoring gate rejects such patches at build time, this is the runtime backstop. Patches the text
   *  in its displayed surface, so `find`/`replace` must be surface-stable tokens (digits/operators). */
  applyPatch: (find: string, replace: string) => Promise<EditorOutcome | null>;
}

/// `source` is authored once in `authoredIn`; the hook keeps the live text in the active surface.
/// `wrap` (default true) supplies the `export`/`main` a bare snippet needs before compiling.
/// `expectsTrap` — the example is SUPPOSED to trap (`expect="error"`), so a trap is the intended outcome:
/// forwarded to the runner so a stale-runtime mismatch never rewrites the expected trap to hard-reload advice.
export function useCadenzaEditor(
  source: string,
  authoredIn: Surface = "sexpr",
  wrap = true,
  expectsTrap = false,
): CadenzaEditor {
  const { surface } = useSyntax();
  const [text, setText] = useState(source);
  // The surface the editor text currently reflects — so a toggle re-renders the CURRENT text
  // (preserving edits) rather than clobbering it with the original `source`.
  const shownSurface = useRef<Surface>(authoredIn);

  // On mount, render the authored source into the active surface via the pretty-printer. We ALWAYS
  // round-trip (even when `authoredIn === surface`) so the displayed buffer is the printer's canonical
  // layout — an authored single-line `source` shows PRETTY (indented / line-broken), giving uniform
  // formatting across every example. A snippet that doesn't round-trip cleanly keeps the raw source.
  useEffect(() => {
    let cancelled = false;
    renderSnippet(source, authoredIn, surface, wrap)
      .then((r) => {
        if (!cancelled) {
          setText(r);
          shownSurface.current = surface;
        }
      })
      .catch(() => {
        if (!cancelled) shownSurface.current = surface;
      });
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

  const run = useCallback(async (override?: { text: string; surface: Surface }): Promise<EditorOutcome> => {
    // Run the OVERRIDE text/surface when given (so an apply-then-run in one tick runs the fresh buffer,
    // not the stale `text` snapshot this callback closes over); otherwise the current editor state.
    const src = override ? override.text : text;
    const srcSurface = override ? override.surface : shownSurface.current;
    const program = wrap ? wrapModule(src, srcSurface) : src;
    const wrapPrefixBytes = wrap ? wrapPrefixOf(src, program) : 0;
    const out = await compile(program, srcSurface);
    if (!out.component) return { kind: "declined", diags: out.diagnostics, wrapPrefixBytes };
    const result: RunOutcome = await runComponent(out.component, srcSurface, false, undefined, expectsTrap);
    switch (result.kind) {
      case "value": {
        // A scalar Float that jco lowered to a whole JS number lost its `.0` (String(5) === "5"); the
        // static export type restores it. Only an INTEGER-LOOKING render could need the `.0`, so gate
        // the export-type lookup on that — a compound/fractional/non-numeric result skips the extra
        // query entirely (the common case). Best-effort — if the lookup fails, show the value as-is.
        let valueText = result.text;
        if (/^-?\d+$/.test(result.text.trim())) {
          try {
            valueText = formatScalarByType(result.text, resultTypeOf(await exportTypes(program, srcSurface)));
          } catch {
            /* keep result.text */
          }
        }
        return { kind: "value", text: valueText };
      }
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
    // Round-trip through the pretty-printer even for the authored surface, so a reset restores the
    // same PRETTY layout the reader first saw (not the raw single-line `source`). Falls back to the
    // raw source if it doesn't round-trip.
    renderSnippet(source, authoredIn, target, wrap)
      .then(setText)
      .catch(() => setText(source));
  }, [authoredIn, source, wrap]);

  /// Swap the buffer to an authored variant + run it, atomically — the primitive behind the clickable
  /// "change X to Y → result" prose. The variant is authored in `srcSurface` (a chapter constant, like
  /// `source`); we render it into the buffer's CURRENT surface so the display stays in the reader's
  /// chosen syntax, `setText` it, then `run` the RENDERED text via the override (dodging the stale-`text`
  /// closure — a plain `run()` here would execute the pre-swap buffer). If the variant doesn't round-trip
  /// cleanly (rare — an authored variant should be valid), fall back to running its raw authored form.
  const applyAuthored = useCallback(
    async (authoredSrc: string, srcSurface: Surface): Promise<EditorOutcome> => {
      const target = shownSurface.current;
      let rendered: string;
      try {
        rendered = await renderSnippet(authoredSrc, srcSurface, target, wrap);
      } catch {
        // Couldn't re-render into the display surface — show + run the raw authored text in its own surface.
        setText(authoredSrc);
        shownSurface.current = srcSurface;
        return run({ text: authoredSrc, surface: srcSurface });
      }
      setText(rendered);
      shownSurface.current = target;
      return run({ text: rendered, surface: target });
    },
    [wrap, run],
  );

  /// One-token patch of the CURRENT buffer (in its displayed surface): replace the single occurrence of
  /// `find` with `replace`, then run. Declines (returns null, no run) unless `find` occurs EXACTLY ONCE —
  /// the same rule the authoring gate enforces at build time (shared via `patchOnce`). Runs the patched
  /// text via the override so the fresh buffer runs, not the stale `text` snapshot.
  const applyPatch = useCallback(
    async (find: string, replace: string): Promise<EditorOutcome | null> => {
      const result = patchOnce(text, find, replace);
      if (!result.ok) return null;
      const target = shownSurface.current;
      setText(result.text);
      return run({ text: result.text, surface: target });
    },
    [text, run],
  );

  return { text, setText, surface: shownSurface.current, reset, run, applyFix, applyAuthored, applyPatch };
}

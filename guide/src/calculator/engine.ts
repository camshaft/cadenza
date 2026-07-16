/// The browser calculator's state + evaluation engine — the TypeScript sibling of the native
/// `cdz-calc` crate's `Calculator` (implementation/seed/crates/cdz-calc/src/lib.rs). It holds the same
/// state model so the two surfaces behave identically: variables accumulate as a NESTED-`let` chain over
/// stored SOURCES (oldest outermost), and evaluating a line wraps the target expression in that chain.
///
/// Why the `let` chain (not top-level defs, not frozen value forms) — the same two dead-ends the native
/// crate hit: a re-binding `ans = ans + 5` as a top-level `def` recurses forever, and a Rational's
/// display form `1/2` is not re-readable source. An inner `let` that lexically SHADOWS the outer one
/// fixes both (`ans + 5` reads the previous `ans`; sources re-read fine).
///
/// Evaluation itself is NOT reimplemented: `evalExpr` hands the `let`-wrapped expression to `replEval`
/// (the shared `cadenza_syntax::repl` assembler, via the compile worker) and runs the component through
/// the same run worker the playground uses — so a calculator result is identical to what the language
/// would produce.

import { replEval, type Surface } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import { classify } from "./classify.ts";
import { type Binding, visibleBindings, wrapInLets } from "./letChain.ts";

// `classify`/`isIdentifier` moved to the React-free `./classify.ts` so `node --test` can cover them
// (this module transitively imports the wasm runtime). Re-exported for existing importers.
export { classify } from "./classify.ts";

/// The implicit variable holding the last result's source — `ans` recalls (and, by `let`-shadowing,
/// composes with) the previous line.
export const ANS = "ans";

/// One evaluated line's outcome — the shape the tape renders.
export type Eval =
  | { kind: "value"; text: string }
  | { kind: "bound"; name: string; text: string }
  | { kind: "trap"; message: string }
  | { kind: "timeout" }
  | { kind: "error"; message: string };

/// `Binding`, `wrapInLets`, and the variables-panel dedup moved to the React-free `./letChain.ts` so
/// `node --test` can cover the state model (this module transitively imports the wasm runtime).

/// The calculator's session state. One surface per instance (ML by default), matching the native crate.
export class Calculator {
  private surface: Surface;
  /// Bindings in insertion order, APPEND-only (a re-binding is a new inner `let` that shadows the outer).
  private bindings: Binding[] = [];
  /// EXACT MODE (forced rationals by default): a bare numeric literal grounds to Rational, so `1 / 3` is
  /// `1/3` with no `R` suffix — via `replEval`'s exact flag (C6's default-fraction pragma). On by default.
  private exact: boolean;

  constructor(surface: Surface, exact = true) {
    this.surface = surface;
    this.exact = exact;
  }

  /// The distinct names in scope (newest binding visible), for completion.
  names(): string[] {
    return this.values().map((v) => v.name);
  }

  /// Each in-scope name and its last rendered value (newest binding visible) — for the variables panel.
  /// Reads the STORED display captured at assignment time; does NOT re-run (a re-run would contend with
  /// the run worker's one-at-a-time guard and race the next input). Synchronous.
  values(): { name: string; text: string }[] {
    return visibleBindings(this.bindings);
  }

  /// Evaluate one typed line, updating state. An assignment commits its source (and echoes the value);
  /// an expression sets `ans` to its source. A line that fails to compile/trap does NOT commit (state
  /// unchanged), so a mistyped line never poisons the session.
  async eval(line: string): Promise<Eval> {
    const c = classify(line);
    const r = await this.evalExpr(c.expr);
    if (r.kind !== "value") return r; // trap/timeout/error — do not commit
    if (c.kind === "assign") {
      this.bindings.push({ name: c.name, src: c.expr, display: r.text });
      return { kind: "bound", name: c.name, text: r.text };
    }
    this.bindings.push({ name: ANS, src: c.expr, display: r.text });
    return { kind: "value", text: r.text };
  }

  /// Compile + run `expr` wrapped in the binding `let` chain, returning the rendered value / trap /
  /// error. Uses the shared `replEval` assembler (empty buffer — all state rides in the `let` chain) and
  /// the run worker, so the result matches a normal run in the reader's surface.
  private async evalExpr(expr: string): Promise<Eval> {
    const wrapped = this.wrapInLets(expr);
    // The buffer holds NO definitions (all state rides in the `let` chain), but `replEval` PARSES the
    // buffer string and rejects an empty one ("empty program"). Pass a bare `0` — it parses on both
    // surfaces and `buffer_items` yields nothing for a bare (non-def) expression, so the assembled
    // program is exactly our `let`-wrapped expression as the entry.
    const out = await replEval("0", wrapped, this.surface, this.exact);
    if (!out.component) {
      const firstErr = out.diagnostics.find((d) => d.error);
      return {
        kind: "error",
        message: firstErr ? `${firstErr.code || ""} ${firstErr.message}`.trim() : "declined",
      };
    }
    // `display: true` renders a result for a human — a rational bare (`1/4`), a quantity in its concise
    // `<value> <unit>` surface, the result type annotation dropped — the calculator's mode (the
    // playground keeps the canonical, re-readable form).
    const r = await runComponent(out.component, this.surface, true);
    switch (r.kind) {
      case "value":
        return { kind: "value", text: r.text };
      case "trap":
        return { kind: "trap", message: r.message };
      case "timeout":
        return { kind: "timeout" };
      default:
        return { kind: "error", message: r.message };
    }
  }

  /// Wrap `expr` in a `let` per binding, OLDEST OUTERMOST — so a later binding sees earlier ones and a
  /// re-binding shadows. ML `let x = 5 in …`, s-expr `(let ((x 5)) …)`. No bindings → `expr` unwrapped.
  /// Mirrors the native crate's `wrap_in_lets`.
  wrapInLets(expr: string): string {
    return wrapInLets(this.surface, this.bindings, expr);
  }

  /// Reset the session (clear all bindings). The surface is unchanged.
  clear(): void {
    this.bindings = [];
  }

  /// Rebuild the binding chain in a new surface (on a global surface toggle). The stored SOURCES were
  /// typed in the old surface, so they cannot simply be reinterpreted; the honest thing is to clear —
  /// the caller decides. Exposed so the page can re-create the engine on a surface change.
  currentSurface(): Surface {
    return this.surface;
  }
}

/// The calculator's PURE state model — the binding list and how it maps to source, extracted from
/// `engine.ts` so `node --test` can cover it (engine.ts transitively imports the wasm runtime, so it is
/// not unit-testable). These two operations are the ones that MUST stay byte-identical to the native
/// `cdz-calc` crate (`wrap_in_lets` + the variables-panel dedup), so they are worth pinning independently
/// of the run worker. Mirrors implementation/seed/crates/cdz-calc/src/lib.rs.

import type { Surface } from "../compiler/client.ts";

/// A stored binding: a name, the SOURCE expression it was assigned (never a value form — a Rational's
/// display `1/2` is not re-readable source), and the rendered value it evaluated to at assignment time
/// (for the variables panel, so the panel never has to RE-run).
export interface Binding {
  name: string;
  src: string;
  display: string;
}

/// Wrap `expr` in a `let` per binding, OLDEST OUTERMOST — so a later binding sees earlier ones and a
/// re-binding lexically SHADOWS the outer. ML `let x = 5 in …`, s-expr `(let ((x 5)) …)`. No bindings →
/// `expr` returned unwrapped. Mirrors the native crate's `wrap_in_lets`.
export function wrapInLets(surface: Surface, bindings: readonly Binding[], expr: string): string {
  let wrapped = expr;
  for (let i = bindings.length - 1; i >= 0; i--) {
    const { name, src } = bindings[i];
    wrapped =
      surface === "ml"
        ? `let ${name} = ${src} in ${wrapped}`
        : `(let ((${name} ${src})) ${wrapped})`;
  }
  return wrapped;
}

/// The distinct in-scope names + their last rendered value, NEWEST BINDING VISIBLE, in insertion order
/// (a re-binding is an append that shadows, so only the latest of each name shows). Reads the STORED
/// display captured at assignment time; does NOT re-run. For the variables panel and completion.
export function visibleBindings(bindings: readonly Binding[]): { name: string; text: string }[] {
  const seen = new Set<string>();
  const out: { name: string; text: string }[] = [];
  for (let i = bindings.length - 1; i >= 0; i--) {
    const b = bindings[i];
    if (!seen.has(b.name)) {
      seen.add(b.name);
      out.push({ name: b.name, text: b.display });
    }
  }
  out.reverse();
  return out;
}

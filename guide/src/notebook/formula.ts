/// Hand-rolled formula classification for a `formula` cell (design: concierge-approved option A — no
/// KaTeX dep; render in HTML/SVG, keyed on the value's SHAPE, and surface a gap rather than faking an
/// expression the hand-roll can't represent). Covers exactly what Cadenza's number model produces:
///   - a RATIONAL `n/d` → a stacked fraction
///   - a QUANTITY `(Qty.of v unit)` (the runtime's shape) → value + unit
///   - any other scalar (Int64/Float64/Bool/String/symbol) → a plain (large) display of its friendly text
///   - a compound (list/tuple/record) → NOT a formula: surface "needs richer math rendering" (this is the
///     signal that would justify elevating to KaTeX — filed as an operator follow-up).
///
/// PURE (no worker/React) — reuses the tested sexpr reader; unit-testable under `node --test`.

import { parseSexpr, stripAscription, isAtom, unquoteAtom, type Node } from "./sexpr.ts";
import { displayNode, asQuantity } from "./formatValue.ts";

/// A classified formula, ready for FormulaView to render.
export type Formula =
  | { kind: "fraction"; num: string; den: string; negative: boolean }
  | { kind: "quantity"; value: string; unit: string }
  | { kind: "plain"; text: string }
  /// The value isn't a shape the hand-rolled renderer typesets (a compound expression) — surface the gap
  /// instead of faking it; `text` is the compact value for a fallback display.
  | { kind: "unrenderable"; text: string; reason: string };

/// Classify a rendered value string (`(: value type)` or bare) into a Formula. On a parse failure the
/// value is shown plain (never throws).
export function classifyFormula(rendered: string): Formula {
  let node: Node;
  try {
    node = stripAscription(parseSexpr(rendered));
  } catch {
    return { kind: "plain", text: rendered };
  }

  if (isAtom(node)) {
    const a = node.atom;
    // A rational `n/d` (optionally negative) → a stacked fraction. A WHOLE-valued rational is canonicalized
    // to `n/1` by Cadenza's number model; render it as a plain integer (`4/1` → `4`, `-4/1` → `-4`) rather
    // than an ugly stacked fraction over `1`.
    const rat = /^(-?)(\d+)\/(\d+)$/.exec(a);
    if (rat) {
      if (rat[3] === "1") return { kind: "plain", text: `${rat[1]}${rat[2]}` };
      return { kind: "fraction", num: rat[2], den: rat[3], negative: rat[1] === "-" };
    }
    // Any other atom (number, bool, symbol, quoted string) → plain friendly text.
    return { kind: "plain", text: unquoteAtom(a) };
  }

  // A quantity → value + unit (via the shared asQuantity helper, so the formula classifier and the
  // value/table display path agree on the shape — the runtime emits `(Qty.of v unit)`, not `(quantity …)`).
  const q = asQuantity(node);
  if (q) return { kind: "quantity", value: q.value, unit: q.unit };

  // A compound value (list/tuple/record) isn't a formula shape — surface the gap (don't fake it).
  return {
    kind: "unrenderable",
    text: displayNode(node),
    reason: "this value isn't a scalar/rational/quantity — richer math rendering (e.g. KaTeX) would be needed",
  };
}

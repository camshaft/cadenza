/// Friendly display formatting for a cell's plain scalar/simple value — turns the canonical s-expr render
/// (`(: 42 Int64)`, `(: 5/2 Rational)`, `(: "hi" String)`, `(: (quantity 2192 meter) …)`) into the
/// human-readable text a reader expects (`42`, `5/2`, `hi`, `2192 meter`) instead of the raw ascribed
/// form. Used by the `value` + `formula` output renderers. A compound value (list/tuple/record) is NOT
/// specialized — it's rendered compactly (ascription stripped, string atoms unquoted), e.g.
/// `(: (list 1 2) …)` → `(list 1 2)`; it's a plain fallback since the table/chart renderers own those
/// shapes when a cell asks for them.
///
/// PURE (no worker/React) — reuses the tested sexpr reader; unit-testable under `node --test`.

import { parseSexpr, stripAscription, isAtom, isList, unquoteAtom, type Node } from "./sexpr.ts";

/// The head symbol of a list node (`(quantity …)` → "quantity"), or null for an atom / empty list.
function head(n: Node): string | null {
  if (isList(n) && n.list.length > 0 && isAtom(n.list[0])) return n.list[0].atom;
  return null;
}

/// Render a value node for human display. Atoms show bare (a rational `5/2`, a number, a symbol; a
/// "quoted string" loses its quotes). A `(quantity <value> <unit>)` shows as `<value> <unit>`. Any other
/// compound (list/tuple/record) falls back to the compact canonical render (the shape renderers own those).
/// Exported so table CELLS render the same friendly form (a quantity in a table shows `5 meter`, not
/// `(quantity 5 meter)`) — one display path shared with the value/formula renderers.
export function displayNode(n: Node): string {
  if (isAtom(n)) return displayAtom(n.atom);
  if (head(n) === "quantity") {
    // (quantity <value> <unit>) → "<value> <unit>"; be lenient about extra/missing fields.
    const parts = (n as { list: Node[] }).list.slice(1).map(displayNode);
    return parts.join(" ");
  }
  return compact(n);
}

/// Render a single atom for friendly display: unquote a string, and collapse a WHOLE-valued rational
/// `n/1` (Cadenza canonicalizes integer-valued rationals to `n/1`) to its plain integer (`4/1` → `4`,
/// `-4/1` → `-4`), so a rational-typed whole number reads the same in a value / table cell as it does in
/// a formula cell (which already collapses `n/1`). A genuine fraction (den ≠ 1) is left as `n/d`.
function displayAtom(atom: string): string {
  const unquoted = unquoteAtom(atom);
  const rat = /^(-?)(\d+)\/1$/.exec(unquoted);
  return rat ? `${rat[1]}${rat[2]}` : unquoted;
}

/// A compact one-line canonical render of an arbitrary node (for a compound value the friendly path
/// doesn't specialize).
function compact(n: Node): string {
  if (isAtom(n)) return displayAtom(n.atom);
  return "(" + n.list.map(compact).join(" ") + ")";
}

/// Format a rendered value string (`(: value type)` or a bare value) for human display. On any parse
/// failure, returns the input unchanged (never throws) — a value we can't parse is shown as-is rather
/// than hidden. A whole scalar like `(: 42 Int64)` → `42`; `(: 5/2 Rational)` → `5/2`; `(: "hi" String)`
/// → `hi`; `(: (quantity 2192 meter) …)` → `2192 meter`. A compound value (list/tuple/record) is rendered
/// compactly (ascription stripped, string atoms unquoted), e.g. `(: (list 1 2) …)` → `(list 1 2)`.
export function formatValue(rendered: string): string {
  let node: Node;
  try {
    node = stripAscription(parseSexpr(rendered));
  } catch {
    return rendered;
  }
  return displayNode(node);
}

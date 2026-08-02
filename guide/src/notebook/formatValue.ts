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
/// "quoted string" loses its quotes). A quantity `(Qty.of <value> <unit>)` shows as `<value> <unit>`
/// (`5 meter`, `5/2 meter`, `9 meter/(second^2)`) — the concise unit surface, matching the compiler's
/// display render for the shapes a notebook produces. A dimensionless quantity (`Unit.one`) shows just
/// its value. Any other compound (list/tuple/record) falls back to the compact canonical render (the
/// shape renderers own those). Exported so table CELLS render the same friendly form (a quantity in a
/// table shows `5 meter`, not the raw `(Qty.of 5 (Unit.base # meter))`) — one display path shared with
/// the value/formula renderers.
///
/// 🔑 The RUNTIME renders a quantity as `(Qty.of <value> <unit>)` (spec 18-units case 72), NOT
/// `(quantity …)` — the notebook runs + renders output in the canonical s-expr surface, so this friendly
/// path is what turns `(Qty.of 5 (Unit.base #"meter"))` into `5 meter`. The quantity shape decision lives
/// in the shared `asQuantity` helper (also used by the formula classifier); the legacy `quantity`-head
/// shape is kept as a harmless fallback but the runtime never emits it.
export function displayNode(n: Node): string {
  if (isAtom(n)) return displayAtom(n.atom);
  const q = asQuantity(n);
  if (q) return q.unit ? `${q.value} ${q.unit}` : q.value;
  return compact(n);
}

/// Decompose a quantity node into its friendly `{ value, unit }` display parts, or null if the node isn't
/// a quantity. The RUNTIME emits `(Qty.of <value> <unit>)` (spec 18-units case 72); the legacy
/// `(quantity <value> <unit>)` shape (never emitted now) is handled as a lenient fallback. `value` is the
/// friendly magnitude (`5`, `5/2`); `unit` is the concise unit surface (`meter`, `meter/second`,
/// `meter/(second^2)`), or `""` for a dimensionless quantity (`Unit.one`). Exported so both the value/table
/// display path (`displayNode`) AND the formula classifier (`classifyFormula`) share ONE quantity-shape
/// decision — a change here can't leave one renderer matching a stale shape while the other doesn't.
export function asQuantity(n: Node): { value: string; unit: string } | null {
  if (!isList(n)) return null;
  const h = head(n);
  if (h === "Qty.of" && n.list.length === 3) {
    return { value: displayNode(n.list[1]), unit: displayUnit(n.list[2]) };
  }
  if (h === "quantity" && n.list.length >= 2) {
    // Legacy shape (the runtime emits `Qty.of`, not this): value + remaining atoms as the unit.
    return { value: displayNode(n.list[1]), unit: n.list.slice(2).map(displayNode).join(" ") };
  }
  return null;
}

/// The name a unit-symbol node carries (`(Unit.base #"meter")` tokenizes to `[Unit.base, #, "meter"]` — the
/// `#"…"` symbol is two atoms), unquoted, or null if the node isn't a `#"…"`-named unit. Used by
/// `displayUnit` to pull `meter` out of a base unit.
function unitSymbolName(list: Node[]): string | null {
  // The last atom of a unit builder is the quoted symbol name (`(Unit.base # "meter")`).
  const last = list[list.length - 1];
  if (last !== undefined && isAtom(last)) {
    const s = last.atom;
    if (s.length >= 2 && s.startsWith('"') && s.endsWith('"')) return unquoteAtom(s);
  }
  return null;
}

/// Pretty-print a unit node into the concise unit surface (`meter`, `meter/second`, `meter^2`,
/// `meter/(second^2)`). Handles the algebraic combinators the notebook produces — `Unit.base`, the
/// dimensionless identity `Unit.one`, product `Unit.*`, quotient `Unit./`, and integer power `Unit.^`.
/// A compound sub-unit is PARENTHESIZED so the result is unambiguous (`meter/(second^2)`, `(a^2)*b`),
/// matching the compiler's nested display; a base unit / power stays bare. Any unrecognized unit shape
/// (e.g. the `Unit.of`/`Unit.prefix` family/scale layer, which the compiler itself renders as a raw call)
/// falls back to the compact canonical render, so a value with an unusual unit is still shown, never lost.
/// Returns "" for the dimensionless identity so a dimensionless quantity renders as just its value.
function displayUnit(n: Node): string {
  if (isAtom(n)) {
    if (n.atom === "Unit.one") return "";
    return n.atom;
  }
  const list = n.list;
  const h = list.length > 0 && isAtom(list[0]) ? list[0].atom : null;
  switch (h) {
    case "Unit.base":
      return unitSymbolName(list) ?? compact(n);
    case "Unit.*":
      if (list.length === 3) return `${wrapUnit(list[1])}*${wrapUnit(list[2])}`;
      break;
    case "Unit./":
      if (list.length === 3) return `${wrapUnit(list[1])}/${wrapUnit(list[2])}`;
      break;
    case "Unit.^":
      // (Unit.^ <unit> <n>) → "<unit>^<n>"; the base is wrapped if compound so `(a/b)^2` reads `(a/b)^2`.
      if (list.length === 3) return `${wrapUnit(list[1])}^${displayNode(list[2])}`;
      break;
  }
  return compact(n);
}

/// A unit sub-expression, parenthesized when it is a compound (product/quotient/power) so the surrounding
/// operator is unambiguous; a base unit or the dimensionless identity stays bare.
function wrapUnit(n: Node): string {
  const s = displayUnit(n);
  if (isAtom(n)) return s;
  const h = isList(n) && n.list.length > 0 && isAtom(n.list[0]) ? n.list[0].atom : null;
  if (h === "Unit.base") return s;
  return `(${s})`;
}

/// Render a single atom for friendly display: unquote a string, and collapse a WHOLE-valued rational
/// `n/1` (Cadenza canonicalizes integer-valued rationals to `n/1`) to its plain integer (`4/1` → `4`,
/// `-4/1` → `-4`), so a rational-typed whole number reads the same in a value / table cell as it does in
/// a formula cell (which already collapses `n/1`). A genuine fraction (den ≠ 1) is left as `n/d`.
///
/// CRITICAL: the `n/1` collapse must run on the RAW (still-quoted) atom, only when it is NOT a quoted
/// string — a String value like `(: "4/1" String)` arrives here as the atom `"4/1"`; unquoting first and
/// then matching `n/1` would corrupt it to `4` (a String that merely LOOKS like a rational). Only a bare
/// (unquoted) atom can be a genuine Rational, so gate the collapse on that (PR #523 Copilot).
function displayAtom(atom: string): string {
  const isQuotedString = atom.length >= 2 && atom.startsWith('"') && atom.endsWith('"');
  if (!isQuotedString) {
    const rat = /^(-?)(\d+)\/1$/.exec(atom);
    if (rat) return `${rat[1]}${rat[2]}`;
  }
  return unquoteAtom(atom);
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

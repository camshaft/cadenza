/// The widget declaration DSL for a ```cadenza widget cell (design D2, ratified: ship the notebook
/// mini-DSL now; a first-class language `input` construct was filed separately as a language finding).
///
/// A widget cell is a block of `name : Type = control(...)` lines. Each declares a typed runtime input
/// bound to an interactive control (slider / number / text / checkbox / dropdown). These lines are
/// parsed HERE by the notebook — they are NOT Cadenza and never reach the compiler. The reactive engine
/// (Inc 4) renders each control, holds its current value in React state, and splices that value into
/// downstream cells as a `let`/`def` binding (§5 of the design doc) — the mechanism that lets an ordinary
/// Cadenza program "take a runtime input" with no language change.
///
/// PURE by design — NO worker/React imports — so it's unit-testable under `node --test` (mirrors
/// `classify.ts` and this vertical's other pure modules). This module: (1) parses widget lines into typed
/// descriptors; (2) turns a descriptor + its current value into the Cadenza binding literal to splice.

/// The Cadenza type a widget binds to. We support the scalar types a control can produce a literal for.
export type WidgetType = "Float64" | "Int64" | "Bool" | "String";

/// A parsed widget declaration. `control` carries the kind-specific config; every widget has a `name`,
/// a declared `type`, and a `default` (the initial current value, as a JS value matching the type).
export type Widget =
  | { name: string; type: WidgetType; control: "slider"; min: number; max: number; step: number; default: number }
  | { name: string; type: WidgetType; control: "number"; min?: number; max?: number; step?: number; default: number }
  | { name: string; type: "String"; control: "text"; default: string }
  | { name: string; type: "Bool"; control: "checkbox"; default: boolean }
  | { name: string; type: "String"; control: "dropdown"; options: string[]; default: string };

/// A parse error for one widget line — carries the (1-based) line number + the offending source + why,
/// so the notebook can show an actionable inline message rather than silently dropping a control.
export interface WidgetError {
  line: number;
  source: string;
  message: string;
}

export interface ParsedWidgets {
  widgets: Widget[];
  errors: WidgetError[];
}

const IDENT_RE = /^[A-Za-z_][A-Za-z0-9_.-]*$/;
const TYPES: WidgetType[] = ["Float64", "Int64", "Bool", "String"];

/// Split a control's argument list on top-level commas (commas NOT inside quotes). The DSL args are
/// simple (numbers, quoted strings, `key: value`), so a quote-aware split suffices — no nesting.
function splitArgs(inner: string): string[] {
  const args: string[] = [];
  let buf = "";
  let inStr = false;
  for (let i = 0; i < inner.length; i++) {
    const ch = inner[i];
    if (ch === '"') inStr = !inStr;
    if (ch === "," && !inStr) {
      args.push(buf.trim());
      buf = "";
    } else {
      buf += ch;
    }
  }
  if (buf.trim().length > 0) args.push(buf.trim());
  return args;
}

/// Parse a single arg token: a `key: value` named arg → { key, value }, else a positional → { value }.
function parseArg(tok: string): { key?: string; value: string } {
  const colon = tok.indexOf(":");
  // A leading identifier then `:` marks a named arg. A quoted string may contain `:`, so require the
  // part before `:` to be a bare identifier.
  if (colon > 0) {
    const key = tok.slice(0, colon).trim();
    if (IDENT_RE.test(key)) return { key, value: tok.slice(colon + 1).trim() };
  }
  return { value: tok.trim() };
}

/// Strip surrounding double-quotes from a string literal token (the DSL uses `"..."`). Returns null if
/// the token isn't a quoted string.
function asString(tok: string): string | null {
  if (tok.length >= 2 && tok.startsWith('"') && tok.endsWith('"')) return tok.slice(1, -1);
  return null;
}

/// Parse one widget declaration line `name : Type = control(args)`. Returns a Widget or an error string.
function parseLine(raw: string): Widget | string {
  // Split off the `name : Type =` header from the `control(...)` RHS at the FIRST top-level `=`.
  const eq = raw.indexOf("=");
  if (eq < 0) return "expected `name : Type = control(...)` (no `=`)";
  const header = raw.slice(0, eq).trim();
  const rhs = raw.slice(eq + 1).trim();

  const colon = header.indexOf(":");
  if (colon < 0) return "missing `: Type` — a widget must declare its type (e.g. `x : Float64 = ...`)";
  const name = header.slice(0, colon).trim();
  const typeStr = header.slice(colon + 1).trim();
  if (!IDENT_RE.test(name)) return `\`${name}\` is not a valid widget name`;
  if (!TYPES.includes(typeStr as WidgetType)) {
    return `unknown type \`${typeStr}\` — expected one of ${TYPES.join(", ")}`;
  }
  const type = typeStr as WidgetType;

  const call = /^([A-Za-z]+)\s*\((.*)\)$/.exec(rhs);
  if (!call) return `expected a control call like \`slider(...)\`, got \`${rhs}\``;
  const control = call[1];
  const args = splitArgs(call[2]).map(parseArg);
  const positional = args.filter((a) => a.key === undefined).map((a) => a.value);
  const named = new Map(args.filter((a) => a.key !== undefined).map((a) => [a.key!, a.value]));

  const num = (s: string | undefined): number | undefined => {
    if (s === undefined) return undefined;
    const n = Number(s);
    return Number.isFinite(n) ? n : undefined;
  };

  switch (control) {
    case "slider": {
      // slider(min, max, step: s, default: d)
      const min = num(positional[0]);
      const max = num(positional[1]);
      if (min === undefined || max === undefined) return "slider needs numeric min + max: `slider(min, max, ...)`";
      const step = num(named.get("step")) ?? (type === "Int64" ? 1 : (max - min) / 100);
      const def = num(named.get("default")) ?? min;
      return { name, type, control: "slider", min, max, step, default: def };
    }
    case "number": {
      const def = num(named.get("default")) ?? num(positional[0]) ?? 0;
      return {
        name, type, control: "number",
        min: num(named.get("min")), max: num(named.get("max")), step: num(named.get("step")), default: def,
      };
    }
    case "text": {
      if (type !== "String") return "text(...) produces a String; declare the widget `: String`";
      const def = asString(named.get("default") ?? '""') ?? "";
      return { name, type: "String", control: "text", default: def };
    }
    case "checkbox": {
      if (type !== "Bool") return "checkbox(...) produces a Bool; declare the widget `: Bool`";
      const def = (named.get("default") ?? "false") === "true";
      return { name, type: "Bool", control: "checkbox", default: def };
    }
    case "dropdown": {
      if (type !== "String") return "dropdown(...) produces a String; declare the widget `: String`";
      const options = positional.map(asString).filter((s): s is string => s !== null);
      if (options.length === 0) return 'dropdown needs quoted string options: `dropdown("a", "b", ...)`';
      const declaredDefault = named.get("default") !== undefined ? asString(named.get("default")!) : null;
      const def = declaredDefault !== null && options.includes(declaredDefault) ? declaredDefault : options[0];
      return { name, type: "String", control: "dropdown", options, default: def };
    }
    default:
      return `unknown control \`${control}\` — expected slider / number / text / checkbox / dropdown`;
  }
}

/// Parse a whole widget cell (its source is a block of `name : Type = control(...)` lines). Blank lines
/// and `--`/`#` comment lines are skipped. Each non-blank line yields a Widget or an error (collected, so
/// one bad line doesn't drop the rest — the notebook shows all errors + renders the valid controls).
export function parseWidgets(source: string): ParsedWidgets {
  const widgets: Widget[] = [];
  const errors: WidgetError[] = [];
  const lines = source.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    if (trimmed === "" || trimmed.startsWith("--") || trimmed.startsWith("#")) continue;
    const r = parseLine(trimmed);
    if (typeof r === "string") errors.push({ line: i + 1, source: trimmed, message: r });
    else widgets.push(r);
  }
  return { widgets, errors };
}

/// Render a widget's CURRENT value as a Cadenza binding, for splicing into a downstream cell (§5). The
/// current value is supplied by the reactive engine (the control's live state); this produces the
/// `def name = <literal>` line that makes `name` an in-scope top-level definition, typed correctly:
///   - Float64: always emit a decimal point (`10` → `10.0`) so it grounds to Float64, not Int64.
///   - Int64: a bare integer.
///   - Bool: `true` / `false`.
///   - String: a double-quoted literal (quotes + backslashes escaped).
/// `current` must match the widget's type (number for Float64/Int64, boolean for Bool, string for String).
export function bindingFor(widget: Widget, current: number | boolean | string): string {
  const lit = literalFor(widget.type, current);
  return `def ${widget.name} = ${lit}`;
}

/// The Cadenza literal for a value of `type`. Exported for the reactive engine + testable in isolation.
export function literalFor(type: WidgetType, value: number | boolean | string): string {
  switch (type) {
    case "Float64": {
      const n = Number(value);
      // Number(n).toString() drops a trailing `.0`; force a decimal point so it grounds to Float64.
      return Number.isInteger(n) ? `${n}.0` : `${n}`;
    }
    case "Int64":
      return `${Math.trunc(Number(value))}`;
    case "Bool":
      return value ? "true" : "false";
    case "String":
      return `"${String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  }
}

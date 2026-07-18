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
  | { name: string; type: "String"; control: "dropdown"; options: string[]; default: string }
  | { name: string; type: "String"; control: "radio"; options: string[]; default: string };

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

/// A valid widget NAME — a simple Cadenza binding identifier. A widget name flows through `bindingFor`
/// into emitted `def <name> = …` source, so it must be a name the compiler accepts: a letter/`_` start,
/// then letters/digits/`_`, with single `-` kebab separators ONLY BETWEEN alphanumerics (no `.` member
/// paths, no leading/trailing/doubled `-`). `def a--b`, `def rate-`, `def a.b` are all invalid Cadenza,
/// so this must reject them (PR #474). Kebab segments: `[A-Za-z_][A-Za-z0-9_]*` joined by single `-`.
const IDENT_RE = /^[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z0-9_]+)*$/;
/// A named-arg KEY (`step:`, `default:`) — a plain identifier; no kebab needed here, but reuse the same
/// binding-name rule so it's equally strict.
const KEY_RE = IDENT_RE;
const TYPES: WidgetType[] = ["Float64", "Int64", "Bool", "String"];

/// Split a control's argument list on top-level commas (commas NOT inside quotes). Escape-aware: a
/// backslash inside a `"..."` string escapes the next char (so `dropdown("a \"q\" opt", "b")` splits
/// into two args, not four). The DSL args are simple (numbers, quoted strings, `key: value`), no nesting.
function splitArgs(inner: string): string[] {
  const args: string[] = [];
  let buf = "";
  let inStr = false;
  for (let i = 0; i < inner.length; i++) {
    const ch = inner[i];
    if (inStr && ch === "\\" && i + 1 < inner.length) {
      // Keep the escape sequence verbatim in the token (asString unescapes later); consume both chars
      // so an escaped `"` does NOT toggle out of the string.
      buf += ch + inner[i + 1];
      i++;
      continue;
    }
    if (ch === '"') {
      inStr = !inStr;
      buf += ch;
      continue;
    }
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
    if (KEY_RE.test(key)) return { key, value: tok.slice(colon + 1).trim() };
  }
  return { value: tok.trim() };
}

/// Strip surrounding double-quotes from a string literal token (the DSL uses `"..."`) and UNESCAPE its
/// content (`\"` → `"`, `\\` → `\`), so a quoted default containing quotes/backslashes round-trips.
/// Returns null if the token isn't a quoted string.
function asString(tok: string): string | null {
  if (tok.length >= 2 && tok.startsWith('"') && tok.endsWith('"')) {
    return tok.slice(1, -1).replace(/\\(["\\])/g, "$1");
  }
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
  // `main` is every cell's reserved per-cell entry slot (the run path calls `(main)`/`main()`, and
  // assembleForRun already carries each cell's own `def (main)`). A widget named `main` would splice a
  // second `def (main) = <value>` into the buffer → two `main`s in one module → CDZ0201 at run time. Reject
  // it up front with an actionable message rather than emitting source that fails to compile downstream.
  if (name === "main") return "`main` is reserved (each cell's entry point) — name the widget something else";
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
      // A slider needs a POSITIVE range (max > min) — an inverted/zero-width range yields a broken/invalid
      // <input type=range> (and a negative default step from (max-min)/100). Reject it clearly.
      if (max <= min) return `slider max (${max}) must be greater than min (${min})`;
      // The step must be POSITIVE; a declared step ≤ 0 (or non-finite) is invalid for a range input, so
      // fall back to the sensible default (1 for Int64, 1/100 of the range for Float64).
      const declaredStep = num(named.get("step"));
      const step = declaredStep !== undefined && declaredStep > 0 ? declaredStep : type === "Int64" ? 1 : (max - min) / 100;
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
    // dropdown + radio are the same shape (a String single-choice over quoted-string options); they
    // differ only in how WidgetControls renders them (a <select> vs a radio-button group).
    case "dropdown":
    case "radio": {
      if (type !== "String") return `${control}(...) produces a String; declare the widget \`: String\``;
      const options = positional.map(asString).filter((s): s is string => s !== null);
      if (options.length === 0) return `${control} needs quoted string options: \`${control}("a", "b", ...)\``;
      const declaredDefault = named.get("default") !== undefined ? asString(named.get("default")!) : null;
      const def = declaredDefault !== null && options.includes(declaredDefault) ? declaredDefault : options[0];
      return { name, type: "String", control, options, default: def };
    }
    default:
      return `unknown control \`${control}\` — expected slider / number / text / checkbox / dropdown / radio`;
  }
}

/// Parse a whole widget cell (its source is a block of `name : Type = control(...)` lines). Blank lines
/// and `--`/`#` comment lines are skipped. Each non-blank line yields a Widget or an error (collected, so
/// one bad line doesn't drop the rest — the notebook shows all errors + renders the valid controls).
export function parseWidgets(source: string): ParsedWidgets {
  const widgets: Widget[] = [];
  const errors: WidgetError[] = [];
  const seen = new Set<string>();
  const lines = source.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    if (trimmed === "" || trimmed.startsWith("--") || trimmed.startsWith("#")) continue;
    const r = parseLine(trimmed);
    if (typeof r === "string") {
      errors.push({ line: i + 1, source: trimmed, message: r });
      continue;
    }
    // A DUPLICATE widget name is an error, not a second control: each widget splices a `def <name> = …`
    // binding into the run buffer, so two same-named widgets emit two `def <name>` — a duplicate top-level
    // definition that fails to compile (CDZ0201) with no obvious cause. Flag it here (like min>max) so the
    // IDE surfaces it, and keep only the FIRST (the binding a downstream cell's `name` reference resolves to).
    if (seen.has(r.name)) {
      errors.push({ line: i + 1, source: trimmed, message: `duplicate widget name \`${r.name}\` — each widget name must be unique (it becomes a top-level binding)` });
      continue;
    }
    seen.add(r.name);
    widgets.push(r);
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
/// GUARDS emitted-source validity (the recurring lexer-hardening class): a NON-FINITE number (NaN /
/// ±Infinity — reachable from a malformed `number(default: 1e999)` or a bad control value) would emit
/// `def x = NaN`/`Infinity`, which is NOT valid Cadenza and would break the cell's compile, so it's
/// clamped to a safe literal (`0.0`/`0`). (Exponent notation like `1e+21` is FINE for Float64 — Cadenza's
/// lexer accepts exponent floats — but is NOT a valid Int64 literal, so the Int64 branch renders large
/// integers in full via BigInt.)
export function literalFor(type: WidgetType, value: number | boolean | string): string {
  switch (type) {
    case "Float64": {
      const n = Number(value);
      if (!Number.isFinite(n)) return "0.0"; // NaN / ±Infinity → safe default (no `def x = Infinity`)
      // A whole-valued number needs an explicit `.0` to ground to Float64 (not Int64); a number that
      // already has a `.` or an `e` exponent (e.g. `1e+21`) is a valid Float64 literal as-is (Cadenza's
      // lexer accepts exponent floats), so pass it through unchanged.
      const s = `${n}`;
      return /[.e]/.test(s) ? s : `${s}.0`;
    }
    case "Int64": {
      const n = Math.trunc(Number(value));
      if (!Number.isFinite(n)) return "0"; // NaN / ±Infinity → safe default
      // BigInt(n) renders a large integer in FULL, avoiding the exponential form (`1e+21`) that `${n}`
      // would produce — exponent notation is NOT a valid Int64 literal (it is fine for Float64 above).
      return BigInt(n).toString();
    }
    case "Bool":
      return value ? "true" : "false";
    case "String":
      return `"${String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  }
}

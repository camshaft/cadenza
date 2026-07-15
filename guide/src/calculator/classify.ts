/// Pure line-classification for the calculator — NO worker/compiler imports, so `node --test` can cover
/// it (the `engine.ts` module it used to live in transitively imports the wasm runtime, un-loadable in
/// node). This mirrors the native `cdz-calc` crate's `classify`/`is_identifier`; keeping the two in sync
/// is what makes the browser calculator behave identically to the CLI, so it's worth pinning with tests.

/// A classified input line: an assignment `name = expr`, or a bare expression.
export type Line =
  | { kind: "assign"; name: string; expr: string }
  | { kind: "expr"; expr: string };

/// A single Cadenza identifier: letters/`_` to start, then letters/digits/`-`/`_`/`.` (kebab + dotted
/// member paths like `String.scalar-len`). Mirrors the native crate's `is_identifier`.
export function isIdentifier(s: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_.-]*$/.test(s);
}

/// Classify a typed line as an assignment `name = expr` or a bare expression. An assignment is a single
/// leading identifier, one `=` that is not part of `==`/`<=`/`>=`/`!=`, then a non-empty expression.
/// Everything else — an equality `a == b`, a comparison, a multi-token left side — is an expression.
/// Mirrors the native crate's `classify`.
export function classify(line: string): Line {
  const trimmed = line.trim();
  for (let i = 0; i < trimmed.length; i++) {
    if (trimmed[i] !== "=") continue;
    const prev = i > 0 ? trimmed[i - 1] : "";
    const next = i + 1 < trimmed.length ? trimmed[i + 1] : "";
    const isComparison =
      next === "=" || prev === "=" || prev === "<" || prev === ">" || prev === "!";
    if (!isComparison) {
      const lhs = trimmed.slice(0, i).trim();
      const rhs = trimmed.slice(i + 1).trim();
      if (isIdentifier(lhs) && rhs.length > 0) {
        return { kind: "assign", name: lhs, expr: rhs };
      }
      // A `=` that isn't a clean `ident = expr` → treat the whole line as an expression.
      return { kind: "expr", expr: trimmed };
    }
    // Skip the whole comparison operator so `a == b == c` isn't misread.
    if (next === "=") i++;
  }
  return { kind: "expr", expr: trimmed };
}

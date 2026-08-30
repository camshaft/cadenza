/// Pure snippet-scaffolding logic — NO React, NO worker/compiler imports — so it is unit-testable
/// under `node --test` (which strips types but cannot load a `.tsx` React module). `useCadenzaEditor`
/// re-exports these; `Runnable`/`Exercise` import them through that hook module.
///
/// The job: an authored guide snippet is usually a bare expression or a `def`/`type`/`effect` block
/// shown WITHOUT the `export`/`main` ceremony a compilable program needs. `wrapModule` supplies only
/// what's missing; `stripModule` is its inverse (peel the scaffolding back off a RENDERED program for
/// display); `renderSnippet` (in `useCadenzaEditor`) round-trips a snippet between surfaces through it.

import type { Surface } from "../compiler/worker.ts";

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
///      top-level statement like `Unit.define`) — append an `export`. If a `main` def is present it is
///      the public entry (the guide convention); otherwise export the snippet's own top-level `def`
///      names, so a lone non-`main` def (`def c-to-f(c) = …`) is public rather than tripping CDZ0101
///      "export `main` names no definition". `effect` matters: an effects snippet leads with
///      `(effect …)`, and mis-classifying it as a bare expression wraps the WHOLE multi-form snippet as
///      `(def (main) …)`, which is malformed. `Unit.define` (declare a custom unit) is the same shape:
///      it MUST be top-level (it declines inside a def body), so a `(Unit.define …) (def (main) …)` pair
///      is a definitions block, not a bare expression.
///   4. A bare EXPRESSION — becomes `def main() = <expr>` plus the export.
/// In s-expr the top-level forms must be gathered under one `(do …)` (s-expr has no bare multi-form
/// top level); in ML they are newline-separated (the surface's native top-level form).
const DECL_HEAD = "def|type|effect";
// A top-level STATEMENT that isn't a `def`/`type`/`effect` but still sits at top level (never wrapped as
// `(def (main) …)`) and needs an `export` appended. `Unit.define` declares a custom unit of measure and
// only resolves at top level. Escaped for a RegExp (the `.` is literal). Treated like a defs block.
const STMT_HEAD = "Unit\\.define";
// A leading compiler PRAGMA is a top-level statement, NOT an expression: ML `@!default-fraction Rational`
// (line starts `@!`), s-expr `(pragma default-fraction Rational)` (form starts `(pragma`). A snippet may
// LEAD with a pragma (e.g. a `@!default-fraction Rational` model header) and then declare defs; treated
// like `Unit.define` — scan past it to the following defs, never bare-expr-wrap it (wrapping
// `def main() = @!default-fraction Rational` is malformed → CDZ0101 pragma/unbound-name squiggles).
const PRAGMA_HEAD_SEXPR = /^\(pragma\b/;
const PRAGMA_HEAD_ML = /^@!/;

/// The names to export from a DEFINITIONS-block snippet. The guide convention is a `(def (main) …)`
/// entry point, so `main` — when present — is the sole public name (matches the historical behavior).
/// When there is NO top-level `main`, export the snippet's own top-level `def` names, so an editable
/// inline snippet that defines only `def c-to-f(c) = …` compiles with `c-to-f` public (a real,
/// actionable diagnostic if its param needs annotating) instead of the phantom-`main` CDZ0101. Falls
/// back to `main` if no top-level `def` name is found (unchanged from the pre-fix behavior).
export function exportNames(trimmed: string, surface: Surface): string[] {
  const names = topLevelDefNames(trimmed, surface);
  if (names.includes("main")) return ["main"];
  return names.length ? names : ["main"];
}

/// The names of the top-level `def`s in a snippet, in source order. ML: an unindented `def NAME` /
/// `def NAME(args)`. S-expr: `(def NAME …)` / `(def (NAME args) …)`. A Cadenza definition name may
/// contain `-` (`c-to-f`). Duplicates are collapsed (a name declared once). This is a lightweight scan,
/// not a parser — it walks the top-level forms the guide's snippets use; nested defs inside a body are
/// not filtered out, but they only ever matter when there is no top-level `main`, where exporting an
/// extra name yields a clear compile diagnostic rather than a wrong render.
export function topLevelDefNames(trimmed: string, surface: Surface): string[] {
  const names: string[] = [];
  const re =
    surface === "sexpr"
      ? /\(def\s+\(?\s*([A-Za-z_][\w-]*)/g
      : /^def[ \t]+([A-Za-z_][\w-]*)/gm;
  for (let m = re.exec(trimmed); m; m = re.exec(trimmed)) {
    if (!names.includes(m[1])) names.push(m[1]);
  }
  return names;
}

/// Count TOP-LEVEL s-expr forms in `s` (a form = a depth-0 balanced list, string, char literal, or bare-atom
/// run). Skips `"…"` strings (with `\` escapes), `#\x` char literals (the char after `#\` is literal, not a
/// paren), and `;` line comments, so a `(`/`)` inside them never miscounts. Used to detect a MULTI-FORM
/// (defs-block) source even when its lead form isn't a recognized head. Lightweight scan, not a parser.
function sexprTopLevelFormCount(s: string): number {
  // `boundary` = the next depth-0 form-start begins a NEW form (true at start + after depth-0 whitespace/
  // comment). A form-start CONTIGUOUS with the previous form is a continuation — Cadenza's application/
  // compound syntax `f(x)`, `#tuple(1 2)`, `f(x)(y)` is ONE form, not two.
  let i = 0, depth = 0, forms = 0, boundary = true;
  while (i < s.length) {
    const c = s[i];
    if (c === ";") {
      while (i < s.length && s[i] !== "\n") i++;
      if (depth === 0) boundary = true;
    } else if (c === '"') {
      if (depth === 0) { if (boundary) forms++; boundary = false; }
      i++;
      while (i < s.length) {
        if (s[i] === "\\") i += 2;
        else if (s[i] === '"') { i++; break; }
        else i++;
      }
    } else if (c === "#" && s[i + 1] === "\\") {
      if (depth === 0) { if (boundary) forms++; boundary = false; }
      i += 2;
      if (i < s.length && /[A-Za-z0-9]/.test(s[i])) while (i < s.length && /[A-Za-z0-9]/.test(s[i])) i++;
      else i++;
    } else if (c === "(") {
      if (depth === 0) { if (boundary) forms++; boundary = false; }
      depth++; i++;
    } else if (c === ")") {
      if (depth > 0) depth--;
      if (depth === 0) boundary = false; // a chained `(` after this close is the same form
      i++;
    } else if (/\s/.test(c)) {
      if (depth === 0) boundary = true;
      i++;
    } else {
      if (depth === 0) { if (boundary) forms++; boundary = false; }
      i++;
    }
  }
  return forms;
}

export function wrapModule(src: string, surface: Surface): string {
  const trimmed = src.trim();
  if (surface === "sexpr") {
    if (/^\(module\b/.test(trimmed) || /^\(do\b/.test(trimmed)) return trimmed;
    // A recognized decl/stmt head OR any MULTI-FORM source is a defs-block (gather under `(do …)` + export).
    // The multi-form check is what makes a defs-block whose LEAD form isn't a literal head wrap correctly —
    // e.g. a `Unit.define` statement in its canonical `((. Unit define) …)` form (as the canonical formatter
    // or the binary-AST printer emit it): head-matching alone would miss it and collapse every form into one
    // `(def (main) …)` body → CDZ0201 "more than one body". A single non-head form is a bare expression.
    if (
      sexprTopLevelFormCount(trimmed) > 1 ||
      PRAGMA_HEAD_SEXPR.test(trimmed) ||
      new RegExp(`^\\((${DECL_HEAD}|${STMT_HEAD})\\b`).test(trimmed)
    )
      return `(do ${trimmed} (export ${exportNames(trimmed, surface).join(" ")}))`;
    return `(do (def (main) ${trimmed}) (export main))`;
  }
  // Already complete — a full `module …`, or a program that already declares an `export` — is left
  // as-is. ⚠ Do NOT treat a mere `;` as "complete": a `;` can be an INTERNAL sequence separator (e.g.
  // `def main() = (module M { … }; M.f x)`), which still needs an `export` appended. Only a real
  // top-level `export` marks a snippet the author already made whole.
  if (/^module\b/.test(trimmed) || /(^|\n)\s*export\b/.test(trimmed)) return trimmed;
  if (PRAGMA_HEAD_ML.test(trimmed) || new RegExp(`^(${DECL_HEAD}|${STMT_HEAD})\\b`).test(trimmed))
    return `${trimmed}\nexport { ${exportNames(trimmed, surface).join(", ")} }`;
  return `def main() = ${trimmed}\nexport { main }`;
}

/// Gather a `mode="test"` snippet (bare `@test`/`def` forms, NO export/main) into a SINGLE top-level
/// form so the pretty-printer — which renders one top-level form — can round-trip it. S-expr has no bare
/// multi-form top level, so several `@test`s (or a helper `def` + a `@test`) must be gathered under a
/// `(do …)`; ML's native top level IS multi-form, so an ML snippet passes through untouched. This is the
/// display/render counterpart of a test panel's `wrap={false}` (the boundary is laid out from the `@test`
/// defs, not an export, so `wrapModule`'s export ceremony is wrong here). Pair with [`ungatherTestForms`].
/// Both the app's `renderSnippet` and the `check:examples` gate call THIS — a prior copy in each drifted,
/// leaving the gate green while the app fed raw s-expr to the ML parser ("expected a name").
export function gatherTestForms(snippet: string, surface: Surface): string {
  return surface === "sexpr" ? `(do ${snippet.trim()})` : snippet.trim();
}

/// Inverse of [`gatherTestForms`] over a RENDERED program: peel the `(do …)` back off when the output is
/// s-expr (via `stripModule`, which unwraps a bare `(do …)`); an ML output is already the native multi-
/// form top level, so it's returned trimmed. `to` is the surface the rendered text is IN.
export function ungatherTestForms(rendered: string, to: Surface): string {
  return to === "sexpr" ? stripModule(rendered, "sexpr") : rendered.trim();
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
    // The canonical printer indents `(do` children 2 spaces (and blank-line-separates top-level defs), so
    // the unwrapped body's SIBLING forms carry that 2-space indent on their continuation lines. `dedent`
    // strips it uniformly so the unwrapped top-level forms sit flush-left; a lone `.trim()` fixes only the
    // FIRST line, leaving later siblings indented 2 spaces (the "weird indentation" the reader sees). Same
    // continuation-line fix the ML branch already applies to a multi-line `def main()` body below.
    const m = /^\(do\b([\s\S]*)\)\s*$/.exec(t);
    const body = dedent(m ? m[1] : t)
      .replace(/\(export\s+[^)]*\)\s*$/, "")
      .trim();
    // A synthesized single `(def (main) <expr>)` (no other defs) → unwrap to the bare expression.
    // Capture the body WITHOUT consuming its leading whitespace (no `\s+`) so `dedent` sees the expr's own
    // indent on every line and strips the leaked `def`-body indent uniformly — a multi-line expr's
    // continuation lines otherwise keep the 2-space `def`-body indent (a lone `.trim()` fixes only line 1).
    // Mirrors the ML branch's `dedent(bare[1])` below.
    const bare = /^\(def\s+\(main\)([\s\S]*)\)$/.exec(body);
    if (bare && !/\(def\b|\(type\b/.test(bare[1])) return dedent(bare[1]).trim();
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

/// The UTF-8 byte length of the scaffolding `wrapModule` prepended before the snippet — the offset
/// that maps a compiled-text byte position back onto the editor text. `wrapModule` embeds the trimmed
/// snippet verbatim, so we locate it in the wrapped output; 0 if it isn't found (already-complete).
export function wrapPrefixOf(editorText: string, wrapped: string): number {
  const trimmed = editorText.trim();
  const idx = trimmed ? wrapped.indexOf(trimmed) : -1;
  return idx < 0 ? 0 : new TextEncoder().encode(wrapped.slice(0, idx)).length;
}

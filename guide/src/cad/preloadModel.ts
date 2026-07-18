/// Pure, dep-free scaffolding for /cad's PRELOADED-library model buffer — NO React, NO worker/compiler
/// imports — so it is unit-testable under `node --test` (which strips types but can't load a `.tsx`
/// module). `CadPage` imports these; the tests in `preloadModel.test.ts` pin the invariants.
///
/// /cad compiles a BARE model buffer against the preloaded CAD library (`exact.cdz`) via
/// `compileWithPreloaded` (operator P5, ruling A): the reader edits only the model, the CAD vocabulary
/// (`Solid`/`v3r`/`lower`) is link-merged. The host AUTO-INJECTS the `import … from "exact"` clause + an
/// `export main` around the reader's buffer — this module is that injection.

import type { Surface } from "../compiler/client.ts";

/// The preloaded CAD module's name (the `import from "<name>"` link target) + the names /cad imports from
/// it. `exact.cdz` is authored in ML (`.cdz`), so the preload format string passed to the compiler is `ml`.
export const CAD_LIB_NAME = "exact";
export const CAD_LIB_FORMAT: Surface = "ml";
/// The names /cad's model buffer imports from the CAD library: the `Solid` type + the `v3r` vector
/// constructor + `lower` (maps the generic `Solid(Rational)` to the monomorphic `SolidR` the host renders).
export const CAD_IMPORTED_NAMES = ["Solid", "v3r", "lower"] as const;

/// Auto-inject the `import … from "exact"` clause + the `@!default-fraction Rational` pragma + the
/// `export main` around the reader's model buffer before compiling — so the buffer shows ONLY the model
/// (operator UX): no import (ruling A) AND no pragma line (the reader shouldn't have to write
/// `@!default-fraction Rational` — the exact-Rational default is a /cad property, injected here; this also
/// removes the red-squiggle the operator saw on an authored pragma line). The pragma is module-scoped +
/// position-insensitive, so a bare `n/d` in the model grounds to an exact Rational.
///
/// The reader's text is embedded VERBATIM and CONTIGUOUS so `wrapPrefixOf` can map a diagnostic's byte
/// span back onto the editor buffer (interior injection would break the linter's linear prefix
/// subtraction — the injected import+pragma form a clean PREFIX, the editor text follows unbroken):
///   - ML: `import …` line, then `@!default-fraction Rational`, then the editor text, then a trailing
///     `export { main }` (import-before-pragma + pragma-before-body both compile — module-scoped).
///   - s-expr: the reader edits the INNER forms (`(def (main) …)`), wrapped in
///     `(do (import …) (pragma default-fraction Rational) <editor forms> (export main))`.
/// This is /cad's own scaffolding (not `wrapModule`, whose bare-expression path would mis-wrap this).
export function injectImport(editorText: string, surface: Surface): string {
  const t = editorText.trim();
  if (surface === "sexpr") {
    // s-expr import spec is a bare name LIST (no commas): `(import "exact" (Solid v3r lower))`.
    const names = CAD_IMPORTED_NAMES.join(" ");
    return `(do\n(import "${CAD_LIB_NAME}" (${names}))\n(pragma default-fraction Rational)\n${t}\n(export main))`;
  }
  const names = CAD_IMPORTED_NAMES.join(", ");
  return `import { ${names} } from "${CAD_LIB_NAME}"\n@!default-fraction Rational\n${t}\nexport { main }`;
}

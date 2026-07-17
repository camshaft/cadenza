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

/// Auto-inject the `import … from "exact"` clause + the `export main` around the reader's model buffer
/// before compiling — ruling A: the buffer shows only the model, the compiled program carries the explicit
/// import. The reader's text is embedded VERBATIM and CONTIGUOUS so `wrapPrefixOf` can map a diagnostic's
/// byte span back onto the editor buffer (interior injection would break the linter's linear prefix
/// subtraction):
///   - ML: the import is a PREFIX line (import-before-`@!pragma` is legal), then the editor text, then a
///     trailing `export { main }`.
///   - s-expr: the reader edits the INNER forms (`(pragma …)` + `(def (main) …)`), which this wraps in a
///     `(do (import …) <editor forms> (export main))`. The editor text sits contiguously between the
///     injected `(do (import …)` prefix and the `(export main))` suffix.
/// This is /cad's own scaffolding (not `wrapModule`, whose bare-expression path would mis-wrap a
/// pragma-led buffer).
export function injectImport(editorText: string, surface: Surface): string {
  const t = editorText.trim();
  if (surface === "sexpr") {
    // s-expr import spec is a bare name LIST (no commas): `(import "exact" (Solid v3r lower))`.
    const names = CAD_IMPORTED_NAMES.join(" ");
    return `(do\n(import "${CAD_LIB_NAME}" (${names}))\n${t}\n(export main))`;
  }
  const names = CAD_IMPORTED_NAMES.join(", ");
  return `import { ${names} } from "${CAD_LIB_NAME}"\n${t}\nexport { main }`;
}

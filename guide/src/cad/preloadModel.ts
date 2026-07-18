/// Pure, dep-free scaffolding for /cad's PRELOADED-library model buffer — NO React, NO worker/compiler
/// imports — so it is unit-testable under `node --test` (which strips types but can't load a `.tsx`
/// module). `CadPage` imports these; the tests in `preloadModel.test.ts` pin the invariants.
///
/// /cad compiles a BARE model buffer against the preloaded CAD library (`exact.cdz`) via
/// `compileWithPreloaded` (operator P5, ruling A): the reader edits only the model, the CAD vocabulary
/// (`Solid`/`v3`/`lower`) is link-merged. The host AUTO-INJECTS the `import … from "exact"` clause + an
/// `export main` around the reader's buffer — this module is that injection.

import type { Surface } from "../compiler/client.ts";

/// The preloaded CAD modules' names (the `import from "<name>"` link targets). Both `exact.cdz` and
/// `helpers.cdz` are authored in ML (`.cdz`), so the preload format string passed to the compiler is `ml`.
/// SINGLE-MODE (operator): a /cad buffer is a bare model whose imports are auto-injected — and it may use
/// the ergonomic HELPERS (`box`/`cyl`/`hole-through`/…) as well as the base `exact` vocab, so BOTH modules'
/// import clauses are injected + both are preloaded. This lets ANY model (a plain shape, a curved part, or a
/// reader-authored `@param` parametric model) reach the full vocabulary without writing an import line.
export const CAD_LIB_NAME = "exact";
export const CAD_HELPERS_NAME = "helpers";
export const CAD_UNITS_NAME = "units";
export const CAD_LIB_FORMAT: Surface = "ml";
/// The names /cad's model buffer imports from `exact` (the auto-injected superset — a model only uses the
/// ones it needs; an UNUSED import is benign, verified, so all models share one import clause):
///   - `Solid` (the CSG type), `v3` (3-D vector ctor), `lower` (generic `Solid(Rational)` → the
///     monomorphic `SolidR` the host renders) — the base every model uses;
///   - `Profile`, `path-start`, `line-to`, `cubic-to`, `v2` — the 2-D PATH builders a curved part uses
///     (a `PathProfile` extruded/revolved — the arch-fin spline showcase).
export const CAD_IMPORTED_NAMES = ["Solid", "v3", "lower", "Profile", "path-start", "line-to", "cubic-to", "v2"] as const;
/// The names /cad's model buffer imports from `helpers` — the ergonomic wrappers (`helpers.cdz` exports):
/// primitives (`box`/`cube`/`ball`/`cyl`), moves (`move`/`move-x`/`move-y`/`move-z`), scales, and the
/// boolean wrappers (`fuse`/`cut`/`common`/`hole-through`). A parametric model (e.g. the mounting plate)
/// builds from these; a plain model leaves them unused (benign).
export const CAD_HELPER_NAMES = ["box", "cube", "ball", "cyl", "move", "move-x", "move-y", "move-z", "scale", "scale-xyz", "fuse", "cut", "common", "hole-through"] as const;
/// The names /cad's model buffer imports from `units` — the real-world UNIT edge constructors
/// (`units.cdz`): `inch` (and future mm/cm/…), each converting a Rational magnitude to the model's exact-mm
/// scale. The units-parametric showcase (an imperial bracket authored in inches) uses `inch`; a plain model
/// leaves it unused (benign — an unused import is verified).
export const CAD_UNIT_NAMES = ["inch"] as const;

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
    // s-expr import spec is a bare name LIST (no commas): `(import "exact" (Solid v3 lower))`.
    const exact = CAD_IMPORTED_NAMES.join(" ");
    const helpers = CAD_HELPER_NAMES.join(" ");
    const units = CAD_UNIT_NAMES.join(" ");
    return `(do\n(import "${CAD_LIB_NAME}" (${exact}))\n(import "${CAD_HELPERS_NAME}" (${helpers}))\n(import "${CAD_UNITS_NAME}" (${units}))\n(pragma default-fraction Rational)\n${t}\n(export main))`;
  }
  const exact = CAD_IMPORTED_NAMES.join(", ");
  const helpers = CAD_HELPER_NAMES.join(", ");
  const units = CAD_UNIT_NAMES.join(", ");
  return `import { ${exact} } from "${CAD_LIB_NAME}"\nimport { ${helpers} } from "${CAD_HELPERS_NAME}"\nimport { ${units} } from "${CAD_UNITS_NAME}"\n@!default-fraction Rational\n${t}\nexport { main }`;
}

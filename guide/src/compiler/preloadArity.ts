/// Pure preload-arity guard for the compiler worker's `compile_with_preloaded` / `diagnostics_with_preloaded`
/// entry points — NO wasm/Comlink imports, so `node --test` covers it. `worker.ts` composes it at the boundary.
///
/// WHY: those wasm entry points require the three parallel arrays (names / sources / formats) to be EQUAL
/// LENGTH; a mismatch throws a raw "must be equal length" that surfaces as a cryptic parse-error diagnostic
/// (this bit /music when `pattern` was added to MUSIC_PRELOAD_NAMES but not the PRELOAD_SOURCES ?raw list).
/// Checking arity at the WORKER BOUNDARY turns that into a clear, actionable decline diagnostic for EVERY call
/// site — present and future — not just the two pages that have their own module-load assertions.

/// The minimal diagnostic shape the worker returns (a subset of its `Diag`); worker.ts widens it to `Diag`.
export interface ArityDiag {
  error: true;
  code: "";
  message: string;
  node: 0;
  from: 0;
  to: 0;
  fix: null;
}

/// Returns a decline diagnostic when names/sources/formats are NOT all equal length, else null (arity fine).
export function preloadArityError(
  names: readonly unknown[],
  sources: readonly unknown[],
  formats: readonly unknown[],
): ArityDiag | null {
  if (names.length === sources.length && names.length === formats.length) return null;
  return {
    error: true,
    code: "",
    message:
      `preload arity mismatch: names=${names.length}, sources=${sources.length}, formats=${formats.length} ` +
      `must be equal length. A preloaded library was added to one array but not the others — align all three.`,
    node: 0,
    from: 0,
    to: 0,
    fix: null,
  };
}

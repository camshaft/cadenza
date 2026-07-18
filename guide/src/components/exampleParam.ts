/// The `?example=<slug>` deep-link param — every guide example gets a stable URL that opens its surface
/// with THAT example selected (operator: call out each example individually in the nav). A nav entry is
/// `/cad?example=hollow-tube` etc.; each surface reads this on load to pick the example. Kept SEPARATE from
/// the share hash (`#code/`/`#cad/`/`#nb/`, which carries a full serialized program): a share link
/// reconstructs an exact edited buffer, a deep-link just selects a named starter — so the hash wins over
/// `?example=` when both are present (a shared edit shouldn't be clobbered by a stale example id).
///
/// Mirrors `SyntaxContext`'s `?syntax=` handling (the guide's one existing query-param precedent): read via
/// `URLSearchParams(location.search)` on load, write via `history.replaceState` (replace, no scroll jump, no
/// history spam, hash untouched). Pure + dep-free (guards a non-browser env) so it's unit-testable.

/// The `?example=` slug from the current URL, or null if absent. `search` defaults to the live location;
/// pass an explicit string in tests.
export function readExampleParam(search: string = typeof window !== "undefined" ? window.location.search : ""): string | null {
  try {
    const v = new URLSearchParams(search).get("example");
    return v && v.length > 0 ? v : null;
  } catch {
    return null;
  }
}

/// Reflect the selected example slug in the URL as `?example=<slug>` (replace, not push — no scroll jump /
/// history spam, hash preserved). A no-op outside a browser. Call when the reader picks an example so the
/// URL is copy-shareable as a deep-link. Pass `null`/empty to leave the param as-is (we never delete it —
/// harmless, and avoids churn).
export function writeExampleParam(slug: string): void {
  if (typeof window === "undefined" || !slug) return;
  try {
    const url = new URL(window.location.href);
    if (url.searchParams.get("example") === slug) return; // already current — no replaceState churn
    url.searchParams.set("example", slug);
    window.history.replaceState(window.history.state, "", url);
  } catch {
    /* URL APIs unavailable (non-browser / locked-down) — the selection still works, just no URL sync */
  }
}

/// Resolve the example to open on load, given the available slugs and a default. Returns the `?example=`
/// slug when it names a KNOWN example, else the default — so a stale/typo'd deep-link degrades to the
/// default rather than a blank surface. The caller applies the share-hash-wins precedence separately (only
/// call this when there's no share hash to honor).
export function resolveExampleParam(knownSlugs: readonly string[], defaultSlug: string, search?: string): string {
  const req = readExampleParam(search);
  return req && knownSlugs.includes(req) ? req : defaultSlug;
}

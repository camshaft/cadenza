/// Detecting a failed dynamic-import ("chunk load") error — the stale-deployment failure mode.
///
/// The guide lazy-loads each chapter/route as a content-hashed chunk (`Rationals-<hash>.js`). When a
/// new version is deployed to Pages, a tab still running the OLD `index.html` references the OLD chunk
/// hashes; navigating to a not-yet-loaded chapter fetches a URL that no longer exists → the dynamic
/// import REJECTS. `<Suspense>` only catches thrown promises, not rejections, so the rejection
/// propagates as a render error and (without a boundary) white-screens the app. We catch it in a route
/// `errorElement` and offer a reload; this module holds the pure logic so it's unit-testable.

/// Whether an error is a failed dynamic-import (the chunk 404 after a new deploy). The message differs
/// by browser, so match the known shapes:
///   - Chromium: "Failed to fetch dynamically imported module: <url>"
///   - Firefox:  "error loading dynamically imported module: <url>"
///   - Safari:   "Importing a module script failed."
/// Also Vite's own preload-error phrasing ("Unable to preload CSS/... " / "dynamically imported module").
export function isChunkLoadError(error: unknown): boolean {
  const message =
    error instanceof Error ? error.message : typeof error === "string" ? error : String((error as { message?: unknown })?.message ?? "");
  return /dynamically imported module|Importing a module script failed|Failed to fetch dynamically|error loading dynamically/i.test(
    message,
  );
}

/// A one-shot guard so an auto-reload can't loop: if the reloaded page ALSO fails to load the chunk
/// (e.g. the deploy is genuinely broken, or the network is down), we must not reload forever. The
/// caller records that it reloaded (keyed in `sessionStorage`, which is per-tab and cleared on close);
/// `shouldAutoReload` returns true only the FIRST time. Pass a storage (defaults to sessionStorage) so
/// this is testable with a plain object stand-in.
const RELOAD_KEY = "cadenza:chunk-reload-attempted";
export interface KVStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/// True at most once per tab-session: the first chunk-load failure returns true (and marks the guard);
/// a repeat within the same session returns false (so the reader sees the manual "reload" UI instead of
/// an endless reload loop). A successful load should call `clearAutoReloadGuard` to re-arm it.
export function shouldAutoReload(store: KVStore | null | undefined): boolean {
  if (!store) return false; // no storage (SSR / disabled) → never auto-reload, show manual UI
  try {
    if (store.getItem(RELOAD_KEY)) return false;
    store.setItem(RELOAD_KEY, "1");
    return true;
  } catch {
    // storage unavailable/blocked → don't auto-reload (fall back to the manual reload button)
    return false;
  }
}

/// Clear the one-shot guard — call after a route/chapter loads SUCCESSFULLY, so a future stale-deploy
/// navigation can auto-reload again (the guard is only meant to break a same-session reload LOOP).
export function clearAutoReloadGuard(store: KVStore | null | undefined): void {
  try {
    store?.removeItem(RELOAD_KEY);
  } catch {
    // ignore — a failure to clear just leaves auto-reload disarmed until the tab closes.
  }
}

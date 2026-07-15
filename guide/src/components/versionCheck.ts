/// Pure logic for proactive deploy detection — NO DOM/React, so `node --test` can cover it. The app
/// bakes its build id in (`__BUILD_ID__`) and polls `version.json` (written per build); when the polled
/// version differs from the running one, a newer bundle was deployed while this tab stayed open. This
/// is the PROACTIVE complement to the reactive chunk-404 recovery (RouteError): instead of waiting for
/// a navigation to 404 on a stale chunk, we prompt the reader to refresh as soon as we notice.

/// Whether a polled version indicates a NEWER deploy than the one currently running. True only when the
/// polled value is a non-empty string that DIFFERS from the running id — a missing/blank/equal value
/// (dev, a failed fetch, the same build) is not an update, so we never nag spuriously.
export function isNewerVersion(running: string, polled: string | null | undefined): boolean {
  return typeof polled === "string" && polled.length > 0 && polled !== running;
}

/// Parse the `version.json` body into its `version` string, or null if it's missing/malformed. Tolerant
/// of a non-object or a non-string `version` (a broken deploy shouldn't throw in the poll loop).
export function parseVersion(body: unknown): string | null {
  if (body && typeof body === "object" && typeof (body as { version?: unknown }).version === "string") {
    return (body as { version: string }).version;
  }
  return null;
}

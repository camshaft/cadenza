/// A non-blocking "a new version is available — refresh" toast, shown when a newer bundle was deployed
/// while this tab stayed open. It POLLS `version.json` (written per build) on window focus and on a slow
/// interval, comparing the polled id to the one baked into this bundle (`__BUILD_ID__`). This is the
/// PROACTIVE half of stale-deployment handling; RouteError is the reactive half (recovers a 404 that
/// already happened). The toast never force-reloads — the reader keeps their place until they choose to.

import { useEffect, useRef, useState } from "react";
import { isNewerVersion, parseVersion } from "./versionCheck.ts";

/// How often to poll while the tab is open, as a backstop to the focus-triggered check. Deploys are
/// infrequent, so a slow poll (5 min) is plenty and costs a single tiny JSON fetch.
const POLL_MS = 5 * 60 * 1000;

/// Fetch `version.json` (relative to the app base) and return its version id, or null on any failure
/// (dev has no such file, offline, malformed) — a poll failure must never throw into React state.
async function fetchDeployedVersion(): Promise<string | null> {
  try {
    // `import.meta.env.BASE_URL` ends in "/"; cache-bust so a stale CDN/browser cache can't hide a new
    // deploy behind the old version.json.
    const url = `${import.meta.env.BASE_URL}version.json?t=${Date.now()}`;
    const res = await fetch(url, { cache: "no-store" });
    if (!res.ok) return null;
    return parseVersion(await res.json());
  } catch {
    return null;
  }
}

export function UpdateToast() {
  const [updateAvailable, setUpdateAvailable] = useState(false);
  // A REF, not `updateAvailable`, gates the poll: the effect is mount-only, so a state value read
  // inside its closure is frozen at the initial `false` (the bug PR #396 caught — polling never
  // stopped and, worse, the next poll re-set `updateAvailable` after the reader dismissed the toast,
  // making it reappear). The ref holds the LIVE decision: once we've shown the toast OR the reader
  // dismissed it, `settled` is true and `check()` stops — Dismiss sticks for the tab session.
  const settled = useRef(false);

  useEffect(() => {
    let cancelled = false;
    const running = typeof __BUILD_ID__ === "string" ? __BUILD_ID__ : "";

    async function check() {
      if (cancelled || settled.current) return; // stop once the toast is shown or has been dismissed
      const polled = await fetchDeployedVersion();
      if (!cancelled && !settled.current && isNewerVersion(running, polled)) {
        settled.current = true;
        setUpdateAvailable(true);
      }
    }

    // Check on focus (the reader returning to a long-open tab is the prime moment a deploy happened)
    // and on a slow interval as a backstop.
    const onFocus = () => void check();
    window.addEventListener("focus", onFocus);
    const timer = setInterval(() => void check(), POLL_MS);
    void check(); // an initial check shortly after load

    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
      clearInterval(timer);
    };
    // Intentionally mount-only — re-subscribing on every state change would thrash the focus listener
    // and interval. The live decision is read through the `settled` REF (not a captured state value),
    // so the mount-only closure never goes stale on it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!updateAvailable) return null;

  return (
    <div className="fixed inset-x-0 bottom-4 z-50 flex justify-center px-4" role="status" aria-live="polite">
      <div className="flex items-center gap-3 rounded-lg border border-cadenza-600/50 bg-slate-900/95 px-4 py-2.5 shadow-xl backdrop-blur">
        <span className="text-sm text-slate-200">A new version of the guide is available.</span>
        <button
          onClick={() => window.location.reload()}
          className="rounded-md bg-cadenza-600 px-3 py-1 text-xs font-semibold text-white transition hover:bg-cadenza-500"
        >
          Refresh
        </button>
        <button
          onClick={() => {
            settled.current = true; // keep it dismissed — don't let the next poll re-show it
            setUpdateAvailable(false);
          }}
          aria-label="Dismiss"
          className="rounded p-1 text-slate-400 transition hover:bg-slate-800/60 hover:text-slate-200"
        >
          ✕
        </button>
      </div>
    </div>
  );
}

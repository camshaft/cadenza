/// The router's `errorElement` — what renders when a route (or a lazily-loaded chapter under it) throws
/// during load/render. Its most important job is the STALE-DEPLOYMENT case: after a new Pages deploy, a
/// tab still on the old bundle 404s when it fetches a chapter's (now-renamed) content-hashed chunk. That
/// dynamic-import rejection would otherwise white-screen the guide. Here we detect it, auto-reload ONCE
/// (which fetches the fresh index.html + current chunk hashes), and — if reloading already happened this
/// session — show a clear "refresh" prompt instead of looping. Any OTHER error falls back to a generic
/// "something went wrong" with a reload, so a render bug never leaves a blank page.

import { useEffect } from "react";
import { useRouteError } from "react-router-dom";
import { isChunkLoadError, shouldAutoReload } from "./chunkError.ts";

function reloadStore(): Storage | null {
  try {
    return typeof sessionStorage !== "undefined" ? sessionStorage : null;
  } catch {
    return null; // sessionStorage can throw in some privacy modes
  }
}

export function RouteError() {
  const error = useRouteError();
  const isChunk = isChunkLoadError(error);

  // On a stale-deployment chunk failure, reload ONCE to pick up the fresh bundle. The guard makes this
  // a no-op if we already reloaded this tab-session (so a genuinely-broken deploy doesn't loop).
  useEffect(() => {
    if (isChunk && shouldAutoReload(reloadStore())) {
      window.location.reload();
    }
  }, [isChunk]);

  const title = isChunk ? "A new version is available" : "Something went wrong";
  const detail = isChunk
    ? "This page was updated since you loaded it, so part of it couldn't be fetched. Reloading picks up the latest version."
    : "The guide hit an unexpected error rendering this page. Reloading usually clears it.";

  return (
    <div className="flex min-h-screen items-center justify-center bg-slate-950 px-4 text-slate-200">
      <div className="max-w-md rounded-lg border border-slate-800 bg-slate-900/60 p-6 text-center">
        <h1 className="mb-2 text-lg font-bold text-slate-100">{title}</h1>
        <p className="mb-4 text-sm text-slate-400">{detail}</p>
        <button
          onClick={() => window.location.reload()}
          className="rounded-md border border-cadenza-600/60 bg-cadenza-600/15 px-4 py-1.5 text-sm font-medium text-cadenza-300 transition hover:bg-cadenza-600/25"
        >
          Reload
        </button>
      </div>
    </div>
  );
}

import { StrictMode, lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, Outlet, RouterProvider, ScrollRestoration } from "react-router-dom";
import { Layout } from "./components/Layout.tsx";
import HomePage from "./components/HomePage.tsx";
import { RouteError } from "./components/RouteError.tsx";
import { UpdateToast } from "./components/UpdateToast.tsx";
import { clearAutoReloadGuard } from "./components/chunkError.ts";
import { SyntaxProvider } from "./syntax/SyntaxContext.tsx";
import { ProgressProvider } from "./progress/ProgressContext.tsx";
import { RunnableRegistryProvider } from "./components/RunnableRegistry.tsx";
import "./index.css";

// The playground is a heavy, full-screen route — lazy-load it so the guide's first paint stays light.
const PlaygroundPage = lazy(() => import("./playground/PlaygroundPage.tsx"));
// The calculator is a full-screen route too (pulls in the compile + run workers) — also lazy.
const CalculatorPage = lazy(() => import("./calculator/CalculatorPage.tsx"));
// The CAD 3D preview pulls in the HEAVY three.js + manifold-3d stack — MUST be lazy so those deps
// code-split behind /cad and never touch the guide's first paint (operator-accepted weight, gated here).
const CadPage = lazy(() => import("./cad/CadPage.tsx"));
// The notebook is a full-screen route that pulls in the compile + run workers (like /calculator) — lazy
// so it code-splits behind /notebook and stays off the guide's first paint.
const NotebookPage = lazy(() => import("./notebook/NotebookPage.tsx"));
// The music showcase is a full-screen route that pulls in the compile + run workers + the preloaded music
// libs (like /cad) — lazy so it code-splits behind /music.
const MusicPage = lazy(() => import("./music/MusicPage.tsx"));
// The platform explorer is a full-screen multi-file route that pulls in the compile + run workers + the
// CodeMirror editor stack (like /calculator) — lazy so it code-splits behind /explorer.
const ExplorerPage = lazy(() => import("./explorer/ExplorerPage.tsx"));

// Root layout: `ScrollRestoration` scrolls a new navigation to the top and RESTORES the previous
// scroll position on back/forward (a per-history-entry memory). The playground manages its own
// full-height scroll, so we only restore for other routes (keyed by pathname there).
function RootLayout() {
  return (
    <>
      <ScrollRestoration getKey={(location) => location.pathname} />
      <Outlet />
      {/* Proactive stale-deploy detection: polls version.json, prompts a refresh when a newer bundle
          ships while this tab is open. Sits at the root so it's present on every route. */}
      <UpdateToast />
    </>
  );
}

// `import.meta.env.BASE_URL` is Vite's configured `base` (e.g. `/cadenza/` on GitHub Pages, `/`
// locally). React Router's basename wants it without a trailing slash.
const basename = import.meta.env.BASE_URL.replace(/\/$/, "");

const router = createBrowserRouter(
  [
    {
      element: <RootLayout />,
      // Catches errors from any child route — most importantly a lazily-loaded chapter chunk that 404s
      // after a new deploy (stale bundle). `RouteError` auto-reloads once to pick up the fresh bundle.
      errorElement: <RouteError />,
      children: [
        { path: "/", element: <HomePage /> },
        {
          path: "/playground",
          element: (
            <Suspense fallback={<div className="p-6 text-slate-500">Loading playground…</div>}>
              <PlaygroundPage />
            </Suspense>
          ),
        },
        {
          path: "/calculator",
          element: (
            <Suspense fallback={<div className="p-6 text-slate-500">Loading calculator…</div>}>
              <CalculatorPage />
            </Suspense>
          ),
        },
        {
          path: "/cad",
          element: (
            <Suspense fallback={<div className="p-6 text-slate-500">Loading CAD preview…</div>}>
              <CadPage />
            </Suspense>
          ),
        },
        {
          path: "/notebook",
          element: (
            <Suspense fallback={<div className="p-6 text-slate-500">Loading notebook…</div>}>
              <NotebookPage />
            </Suspense>
          ),
        },
        {
          path: "/music",
          element: (
            <Suspense fallback={<div className="p-6 text-slate-500">Loading music showcase…</div>}>
              <MusicPage />
            </Suspense>
          ),
        },
        {
          path: "/explorer",
          element: (
            <Suspense fallback={<div className="p-6 text-slate-500">Loading platform explorer…</div>}>
              <ExplorerPage />
            </Suspense>
          ),
        },
        { path: "/:slug", element: <Layout /> },
      ],
    },
  ],
  { basename },
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ProgressProvider>
      <SyntaxProvider>
        <RunnableRegistryProvider>
          <RouterProvider router={router} />
        </RunnableRegistryProvider>
      </SyntaxProvider>
    </ProgressProvider>
  </StrictMode>,
);

// Re-arm the stale-deploy auto-reload once the app has stayed up briefly. `RouteError` sets a one-shot
// sessionStorage guard when it auto-reloads on a chunk 404, so a genuinely-broken deploy can't reload-
// loop. But if the reload SUCCEEDED (we're here, running, seconds later), the incident is resolved —
// clear the guard so a LATER stale deploy in the same tab-session can auto-reload again. The delay is
// what preserves loop protection: a broken deploy re-throws the chunk error almost immediately (before
// this fires), so the guard is still set and the second reload is suppressed into the manual prompt.
setTimeout(() => {
  try {
    clearAutoReloadGuard(sessionStorage);
  } catch {
    // sessionStorage can throw in some privacy modes — a failure to re-arm just leaves auto-reload
    // disarmed until the tab closes, which is safe.
  }
}, 10_000);

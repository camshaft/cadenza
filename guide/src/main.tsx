import { StrictMode, lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, Outlet, RouterProvider, ScrollRestoration } from "react-router-dom";
import { Layout } from "./components/Layout.tsx";
import HomePage from "./components/HomePage.tsx";
import { SyntaxProvider } from "./syntax/SyntaxContext.tsx";
import { ProgressProvider } from "./progress/ProgressContext.tsx";
import "./index.css";

// The playground is a heavy, full-screen route — lazy-load it so the guide's first paint stays light.
const PlaygroundPage = lazy(() => import("./playground/PlaygroundPage.tsx"));
// The calculator is a full-screen route too (pulls in the compile + run workers) — also lazy.
const CalculatorPage = lazy(() => import("./calculator/CalculatorPage.tsx"));

// Root layout: `ScrollRestoration` scrolls a new navigation to the top and RESTORES the previous
// scroll position on back/forward (a per-history-entry memory). The playground manages its own
// full-height scroll, so we only restore for other routes (keyed by pathname there).
function RootLayout() {
  return (
    <>
      <ScrollRestoration getKey={(location) => location.pathname} />
      <Outlet />
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
        <RouterProvider router={router} />
      </SyntaxProvider>
    </ProgressProvider>
  </StrictMode>,
);

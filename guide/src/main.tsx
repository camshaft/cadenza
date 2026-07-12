import { StrictMode, lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, Navigate, RouterProvider } from "react-router-dom";
import { Layout } from "./components/Layout.tsx";
import { SyntaxProvider } from "./syntax/SyntaxContext.tsx";
import { ProgressProvider } from "./progress/ProgressContext.tsx";
import { CHAPTERS } from "./content/chapters.ts";
import "./index.css";

// The playground is a heavy, full-screen route — lazy-load it so the guide's first paint stays light.
const PlaygroundPage = lazy(() => import("./playground/PlaygroundPage.tsx"));

// `import.meta.env.BASE_URL` is Vite's configured `base` (e.g. `/cadenza/` on GitHub Pages, `/`
// locally). React Router's basename wants it without a trailing slash.
const basename = import.meta.env.BASE_URL.replace(/\/$/, "");

const router = createBrowserRouter(
  [
    { path: "/", element: <Navigate to={`/${CHAPTERS[0].slug}`} replace /> },
    {
      path: "/playground",
      element: (
        <Suspense fallback={<div className="p-6 text-slate-500">Loading playground…</div>}>
          <PlaygroundPage />
        </Suspense>
      ),
    },
    { path: "/:slug", element: <Layout /> },
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

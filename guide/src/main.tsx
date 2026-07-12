import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, Navigate, RouterProvider } from "react-router-dom";
import { Layout } from "./components/Layout.tsx";
import { SyntaxProvider } from "./syntax/SyntaxContext.tsx";
import { CHAPTERS } from "./content/chapters.ts";
import "./index.css";

// `import.meta.env.BASE_URL` is Vite's configured `base` (e.g. `/cadenza/` on GitHub Pages, `/`
// locally). React Router's basename wants it without a trailing slash.
const basename = import.meta.env.BASE_URL.replace(/\/$/, "");

const router = createBrowserRouter(
  [
    { path: "/", element: <Navigate to={`/${CHAPTERS[0].slug}`} replace /> },
    { path: "/:slug", element: <Layout /> },
  ],
  { basename },
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <SyntaxProvider>
      <RouterProvider router={router} />
    </SyntaxProvider>
  </StrictMode>,
);

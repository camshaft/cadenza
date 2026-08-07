/// The guide's two shared, router-aware link components — the single styled-link source the chapters use.
///
/// `Ch` is an INTERNAL link to another chapter (`/slug`); `AppLink` is a link to a standalone app route
/// (`/playground`, `/explorer`, …) and reads slightly heavier (`font-medium`). Both were previously copy-
/// pasted as identical local helpers in several chapter files; centralizing them here gives the sexp→TSX
/// codegen (cadenza-docs I4) a single styled target to emit — `(link (slug …))` → `<Ch>`, `(app-link
/// (route …))` → `<AppLink>` — so a generated chapter styles links exactly as the hand-written ones do,
/// and the palette lives in one place. (v-guide verified the class strings are 100% uniform across the 40
/// chapter links, so this is byte-exact, not lossy.)

import { Link } from "react-router-dom";
import type { ReactNode } from "react";

/// An internal link to another chapter (served by the `/:slug` catch-all route).
export function Ch({ to, children }: { to: string; children: ReactNode }) {
  return (
    <Link to={to} className="text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

/// A link to a standalone app route (playground / calculator / cad / notebook / music / explorer). Reads
/// heavier than a chapter cross-link (`font-medium`) — it sends the reader OUT to a full app, not another page.
export function AppLink({ to, children }: { to: string; children: ReactNode }) {
  return (
    <Link to={to} className="font-medium text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

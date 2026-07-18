/// The sidebar's EXAMPLES section — every runnable example gets its own deep-linked nav entry (operator:
/// call each example out individually, not one lumped page). v-guide-editor owns the ORGANIZATION (this
/// grouping); this vertical owns the render + the `?example=` deep-link mechanism.
///
/// ~50 examples would swamp the sidebar, so it's a COLLAPSIBLE top-level section (collapsed by default so
/// it doesn't dominate), with three surface groups: Playground (sub-grouped by THEME into 4 buckets, since
/// 37 is a lot), CAD, and Notebook (small, flat). Every entry is a `NavLink` deep-link — `/playground?
/// example=<id>` / `/cad?example=<slug>` / `/notebook?example=<slug>` — that opens the surface with THAT
/// example selected (the surfaces read `?example=` on load). Rendered from the example DATA (the three
/// `EXAMPLES` arrays + playground's `theme` field), NOT a hardcoded list, so adding an example extends the
/// nav automatically — same derive-from-source spirit as the CHAPTERS registry.

import { NavLink } from "react-router-dom";
import { useState } from "react";
import { EXAMPLES as PLAYGROUND_EXAMPLES } from "../playground/examples.ts";
import { EXAMPLES as CAD_EXAMPLES } from "../cad/examples.ts";
import { EXAMPLES as NOTEBOOK_EXAMPLES } from "../notebook/examples.ts";

/// The playground theme buckets, in display order, with human labels (v-guide-editor's categorization).
const PLAYGROUND_THEMES: { theme: string; label: string }[] = [
  { theme: "basics", label: "Basics" },
  { theme: "algorithms", label: "Algorithms" },
  { theme: "data-and-collections", label: "Data & collections" },
  { theme: "numbers", label: "Numbers" },
];

/// One deep-linkable example: its display title + the route that opens it with the example selected.
interface ExampleLink {
  title: string;
  to: string;
}

/// A labeled group of example links (a surface, or a playground theme bucket).
interface ExampleGroup {
  label: string;
  links: ExampleLink[];
}

/// Build the grouped example links from the three surfaces' data. Playground splits into its 4 theme
/// buckets (skipping any empty bucket); CAD + notebook stay flat. Deep-links: playground by `id`, cad +
/// notebook by `slug` (their existing stable ids) — the `?example=` value each surface resolves on load.
function buildGroups(): ExampleGroup[] {
  const groups: ExampleGroup[] = [];
  for (const { theme, label } of PLAYGROUND_THEMES) {
    const links = PLAYGROUND_EXAMPLES.filter((e) => e.theme === theme).map((e) => ({
      title: e.name,
      to: `/playground?example=${e.id}`,
    }));
    if (links.length > 0) groups.push({ label: `Playground · ${label}`, links });
  }
  groups.push({
    label: "CAD",
    links: CAD_EXAMPLES.map((e) => ({ title: e.title, to: `/cad?example=${e.slug}` })),
  });
  groups.push({
    label: "Notebook",
    links: NOTEBOOK_EXAMPLES.map((e) => ({ title: e.title, to: `/notebook?example=${e.slug}` })),
  });
  return groups;
}

export function ExamplesNav() {
  // Collapsed by default (v-guide-editor's call) so the ~50 entries don't dominate the sidebar; the reader
  // expands it when they want to browse examples.
  const [open, setOpen] = useState(false);
  const groups = buildGroups();
  const total = groups.reduce((n, g) => n + g.links.length, 0);
  return (
    <div>
      <button
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        data-testid="nav-examples-toggle"
        className="flex min-h-11 w-full items-center gap-2 rounded-md px-2.5 text-xs font-semibold uppercase tracking-wider text-slate-500 transition hover:text-slate-300 md:min-h-0 md:py-1"
      >
        <span aria-hidden className={"transition-transform " + (open ? "rotate-90" : "")}>▸</span>
        <span className="flex-1 text-left">Examples</span>
        <span className="text-[10px] tabular-nums text-slate-600">{total}</span>
      </button>
      {open && (
        <div className="mt-2 space-y-4" data-testid="nav-examples-groups">
          {groups.map((g) => (
            <div key={g.label}>
              <div className="mb-1 px-2.5 text-[10px] font-semibold uppercase tracking-wider text-slate-600">{g.label}</div>
              <ul className="space-y-0.5">
                {g.links.map((l) => (
                  <li key={l.to}>
                    <NavLink
                      to={l.to}
                      className={({ isActive }) =>
                        // isActive is route-only (the query isn't part of the match), so every example under
                        // a surface would light up on that route — keep it a plain hover style, not active-bg.
                        "flex min-h-11 items-center rounded-md px-2.5 py-1.5 text-sm text-slate-400 transition hover:bg-slate-800/60 hover:text-slate-200 md:min-h-0 " +
                        (isActive ? "text-slate-300" : "")
                      }
                    >
                      {l.title}
                    </NavLink>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

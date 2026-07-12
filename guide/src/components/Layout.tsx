/// The guide shell: a sticky header (brand + global syntax toggle), a sidebar table of contents, and
/// the chapter content column with prev/next navigation.

import { NavLink, useParams } from "react-router-dom";
import { Suspense } from "react";
import { CHAPTERS, chapterAt } from "../content/chapters.ts";
import { SyntaxToggle } from "../syntax/SyntaxToggle.tsx";
import { useProgress } from "../progress/ProgressContext.tsx";

export function Layout() {
  const { slug } = useParams();
  const active = slug ?? CHAPTERS[0].slug;
  const found = chapterAt(active);
  const progress = useProgress();

  const sections = groupBySection();

  return (
    <div className="min-h-screen bg-slate-950 text-slate-200">
      <Header />
      <div className="mx-auto flex max-w-7xl gap-8 px-4 py-8">
        <aside className="hidden w-60 shrink-0 md:block">
          <nav className="sticky top-24 space-y-6">
            <ProgressSummary />
            {sections.map(([section, chapters]) => (
              <div key={section}>
                <div className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-500">
                  {section}
                </div>
                <ul className="space-y-0.5">
                  {chapters.map((c) => {
                    const total = c.exercises ?? 0;
                    const done = total > 0 ? progress.countFor(c.slug) : 0;
                    return (
                      <li key={c.slug}>
                        <NavLink
                          to={`/${c.slug}`}
                          className={({ isActive }) =>
                            "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm transition " +
                            (isActive
                              ? "bg-cadenza-600/15 font-medium text-cadenza-300"
                              : "text-slate-400 hover:bg-slate-800/60 hover:text-slate-200")
                          }
                        >
                          <span className="flex-1">{c.title}</span>
                          {total > 0 && (
                            <span
                              className={
                                "shrink-0 text-[10px] tabular-nums " +
                                (done >= total ? "text-emerald-400" : "text-slate-500")
                              }
                              title={`${done} of ${total} exercises done`}
                            >
                              {done >= total ? "✓" : `${done}/${total}`}
                            </span>
                          )}
                        </NavLink>
                      </li>
                    );
                  })}
                </ul>
              </div>
            ))}
          </nav>
        </aside>

        <main className="min-w-0 flex-1">
          <div className="mx-auto max-w-3xl">
            {found ? (
              <Suspense fallback={<div className="text-slate-500">Loading…</div>}>
                <found.chapter.Component />
              </Suspense>
            ) : (
              <div className="text-slate-400">Chapter not found.</div>
            )}
            {found && <PrevNext index={found.index} />}
          </div>
        </main>
      </div>
    </div>
  );
}

/// Overall exercise progress across the whole tour, with a reset. Sits atop the sidebar to give the
/// reader a sense of momentum. Hidden until there is at least one exercise completed, so it doesn't
/// nag a first-time reader.
function ProgressSummary() {
  const progress = useProgress();
  const total = CHAPTERS.reduce((n, c) => n + (c.exercises ?? 0), 0);
  const done = CHAPTERS.reduce((n, c) => n + (c.exercises ? progress.countFor(c.slug) : 0), 0);
  if (done === 0) return null;
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900/50 p-3">
      <div className="mb-1.5 flex items-center justify-between text-xs">
        <span className="font-medium text-slate-300">
          Exercises: {done}/{total}
        </span>
        <button
          onClick={progress.clear}
          className="text-slate-500 transition hover:text-slate-300"
          title="Reset your progress"
        >
          reset
        </button>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-slate-800">
        <div className="h-full rounded-full bg-cadenza-500 transition-all" style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

function Header() {
  return (
    <header className="sticky top-0 z-20 border-b border-slate-800/80 bg-slate-950/80 backdrop-blur">
      <div className="mx-auto flex max-w-7xl items-center justify-between px-4 py-3">
        <NavLink to="/" className="flex items-center gap-2">
          <span className="text-lg font-bold tracking-tight text-slate-100">Cadenza</span>
          <span className="hidden text-sm text-slate-500 sm:inline">the interactive guide</span>
        </NavLink>
        <SyntaxToggle />
      </div>
    </header>
  );
}

function PrevNext({ index }: { index: number }) {
  const prev = index > 0 ? CHAPTERS[index - 1] : null;
  const next = index < CHAPTERS.length - 1 ? CHAPTERS[index + 1] : null;
  return (
    <nav className="mt-16 flex items-stretch justify-between gap-4 border-t border-slate-800 pt-6">
      {prev ? (
        <NavLink
          to={`/${prev.slug}`}
          className="group flex-1 rounded-lg border border-slate-800 p-3 transition hover:border-cadenza-600/60"
        >
          <div className="text-xs text-slate-500">← Previous</div>
          <div className="text-sm font-medium text-slate-200 group-hover:text-cadenza-300">
            {prev.title}
          </div>
        </NavLink>
      ) : (
        <div className="flex-1" />
      )}
      {next ? (
        <NavLink
          to={`/${next.slug}`}
          className="group flex-1 rounded-lg border border-slate-800 p-3 text-right transition hover:border-cadenza-600/60"
        >
          <div className="text-xs text-slate-500">Next →</div>
          <div className="text-sm font-medium text-slate-200 group-hover:text-cadenza-300">
            {next.title}
          </div>
        </NavLink>
      ) : (
        <div className="flex-1" />
      )}
    </nav>
  );
}

function groupBySection(): [string, typeof CHAPTERS][] {
  const map = new Map<string, typeof CHAPTERS>();
  for (const c of CHAPTERS) {
    const arr = map.get(c.section) ?? [];
    arr.push(c);
    map.set(c.section, arr);
  }
  return [...map.entries()];
}

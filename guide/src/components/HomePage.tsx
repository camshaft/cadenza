/// The site's front door — a hero that says what Cadenza is, a live example you can run immediately,
/// and clear paths into the two halves of the site: the guided tour and the full playground.

import { Link } from "react-router-dom";
import { Runnable } from "./Runnable.tsx";
import { SyntaxToggle } from "../syntax/SyntaxToggle.tsx";
import { CHAPTERS } from "../content/chapters.ts";
import { HOMEPAGE_RUNNABLES } from "../content/HomePageExamples.ts";

export default function HomePage() {
  const firstChapter = CHAPTERS[0].slug;
  return (
    <div className="min-h-screen bg-slate-950 text-slate-200">
      <header className="sticky top-0 z-20 border-b border-slate-800/80 bg-slate-950/80 backdrop-blur">
        <div className="mx-auto flex max-w-5xl items-center justify-between px-4 py-3">
          <span className="text-lg font-bold tracking-tight text-slate-100">Cadenza</span>
          {/* Mobile touch targets: the header nav links get a 44px min-height below `sm`, compact at sm+. */}
          <div className="flex items-center gap-1 sm:gap-4">
            <Link
              to={`/${firstChapter}`}
              className="flex min-h-11 items-center px-2 text-sm text-slate-400 transition hover:text-slate-200 sm:min-h-0 sm:px-0"
            >
              Guide
            </Link>
            <Link
              to="/playground"
              className="flex min-h-11 items-center px-2 text-sm text-slate-400 transition hover:text-slate-200 sm:min-h-0 sm:px-0"
            >
              Playground
            </Link>
            {/* The syntax toggle is a nicety here (the guide/playground carry their own); hide it on
                the narrowest screens so the landing header never overflows. */}
            <div className="hidden sm:block">
              <SyntaxToggle />
            </div>
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-3xl px-4">
        {/* Hero */}
        <section className="pt-16 pb-8 text-center">
          <h1 className="mb-4 text-5xl font-bold tracking-tight text-slate-50">Cadenza</h1>
          <p className="mx-auto mb-2 max-w-2xl text-xl text-slate-300">
            A programming language written and read by AI agents, read by humans, verified for its
            properties, and compiled to sandboxed WebAssembly components.
          </p>
          <p className="mx-auto max-w-2xl text-sm text-slate-500">
            Everything on this site runs in your browser, since the compiler itself is WebAssembly, so
            there's no server. Edit any example and press Run.
          </p>
        </section>

        {/* Live example */}
        <section className="pb-4">
          <Runnable title={HOMEPAGE_RUNNABLES[0].title} source={HOMEPAGE_RUNNABLES[0].source} />
        </section>

        {/* CTAs */}
        <section className="grid gap-4 py-8 sm:grid-cols-2">
          <Link
            to={`/${firstChapter}`}
            className="group rounded-xl border border-slate-700/60 bg-slate-900/60 p-5 transition hover:border-cadenza-600/60"
          >
            <div className="mb-1 text-lg font-semibold text-slate-100 group-hover:text-cadenza-300">
              Take the tour →
            </div>
            <p className="text-sm text-slate-400">
              Learn Cadenza chapter by chapter, with runnable examples and graded exercises. Start from
              the basics; every idea comes with the reasoning behind it.
            </p>
          </Link>
          <Link
            to="/playground"
            className="group rounded-xl border border-slate-700/60 bg-slate-900/60 p-5 transition hover:border-cadenza-600/60"
          >
            <div className="mb-1 text-lg font-semibold text-slate-100 group-hover:text-cadenza-300">
              Open the playground →
            </div>
            <p className="text-sm text-slate-400">
              A full in-browser IDE: type-checking as you type, hover for inferred types, run your code,
              and share it with a link.
            </p>
          </Link>
        </section>

        {/* The pitch: three tenets, briefly */}
        <section className="border-t border-slate-800 py-10">
          <div className="grid gap-6 sm:grid-cols-3">
            <Tenet title="Two syntaxes, one program">
              Code is data. Read it as a conventional ML/Rust-family surface or as s-expressions, then flip
              the toggle and every example re-renders. It's the same program either way.
            </Tenet>
            <Tenet title="Trust by construction">
              Determinism and capability-safety are the floor, not features. A program can only reach
              the outside world through capabilities it declares.
            </Tenet>
            <Tenet title="Decline, don't miscompile">
              When the compiler can't be sure it would compile your program correctly, it refuses, with a
              diagnostic, rather than emitting something that quietly misbehaves.
            </Tenet>
          </div>
          <p className="mt-8 text-center text-sm text-slate-500">
            Curious why?{" "}
            <Link to="/philosophy" className="text-cadenza-300 hover:underline">
              Read the design tenets
            </Link>
            .
          </p>
        </section>
      </main>

      <footer className="border-t border-slate-800 py-6 text-center text-xs text-slate-600">
        Cadenza: the interactive guide &amp; playground
      </footer>
    </div>
  );
}

function Tenet({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="mb-1.5 text-sm font-semibold text-cadenza-300">{title}</div>
      <p className="text-sm leading-6 text-slate-400">{children}</p>
    </div>
  );
}

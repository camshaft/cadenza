/// The tour's chapter registry — the ordered list that drives the sidebar, routing, and prev/next.
///
/// Each chapter is a lazily-loaded TSX module (so it can embed <Runnable> examples). The order here
/// is the reading order; front-loads Cadenza's differentiators (effects, the value model) after the
/// fundamentals, per the guide's information architecture.

import type { ComponentType } from "react";
import { lazy } from "react";

export interface Chapter {
  slug: string;
  title: string;
  /** One-line blurb shown in the sidebar / cards. */
  blurb: string;
  /** Section grouping for the sidebar. */
  section: string;
  /** Number of graded exercises in the chapter (drives the sidebar progress badge). Default 0. */
  exercises?: number;
  Component: ComponentType;
}

export const CHAPTERS: Chapter[] = [
  {
    slug: "welcome",
    title: "Welcome",
    blurb: "What Cadenza is, and how this interactive guide works.",
    section: "Getting started",
    Component: lazy(() => import("./chapters/Welcome.tsx")),
  },
  {
    slug: "philosophy",
    title: "Why Cadenza is the way it is",
    blurb: "The core tenets underneath the language.",
    section: "Getting started",
    Component: lazy(() => import("./chapters/Philosophy.tsx")),
  },
  {
    slug: "basics",
    title: "Values & functions",
    blurb: "Literals, bindings, functions, and type inference.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Basics.tsx")),
  },
  {
    slug: "control-flow",
    title: "Control flow",
    blurb: "if / then / else, sequencing, and short-circuit logic.",
    section: "Fundamentals",
    exercises: 1,
    Component: lazy(() => import("./chapters/ControlFlow.tsx")),
  },
  {
    slug: "ordering",
    title: "Comparison & ordering",
    blurb: "Comparisons, and the three-way shape of a total order.",
    section: "Fundamentals",
    exercises: 1,
    Component: lazy(() => import("./chapters/Ordering.tsx")),
  },
  {
    slug: "data",
    title: "Data: tuples, records, sums",
    blurb: "Compound values and pattern matching.",
    section: "Fundamentals",
    Component: lazy(() => import("./chapters/Data.tsx")),
  },
  {
    slug: "pattern-matching",
    title: "Pattern matching",
    blurb: "Deciding by shape; sum types and exhaustiveness.",
    section: "Fundamentals",
    Component: lazy(() => import("./chapters/PatternMatching.tsx")),
  },
  {
    slug: "lists",
    title: "Lists",
    blurb: "Ordered, immutable sequences on the value heap.",
    section: "Fundamentals",
    exercises: 1,
    Component: lazy(() => import("./chapters/Lists.tsx")),
  },
  {
    slug: "numbers",
    title: "The numeric model",
    blurb: "Checked integers and no silent promotion.",
    section: "Distinctives",
    Component: lazy(() => import("./chapters/Numbers.tsx")),
  },
];

export function chapterAt(slug: string): { chapter: Chapter; index: number } | null {
  const index = CHAPTERS.findIndex((c) => c.slug === slug);
  if (index < 0) return null;
  return { chapter: CHAPTERS[index], index };
}

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
    slug: "basics",
    title: "Values & functions",
    blurb: "Literals, bindings, functions, and type inference.",
    section: "Fundamentals",
    Component: lazy(() => import("./chapters/Basics.tsx")),
  },
  {
    slug: "control-flow",
    title: "Control flow",
    blurb: "if / then / else, sequencing, and short-circuit logic.",
    section: "Fundamentals",
    Component: lazy(() => import("./chapters/ControlFlow.tsx")),
  },
  {
    slug: "data",
    title: "Data: tuples, records, sums",
    blurb: "Compound values and pattern matching.",
    section: "Fundamentals",
    Component: lazy(() => import("./chapters/Data.tsx")),
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

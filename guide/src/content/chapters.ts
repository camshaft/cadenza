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
    slug: "functions",
    title: "Composing functions",
    blurb: "Pipelines, composition, and applying one argument at a time.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Functions.tsx")),
  },
  {
    slug: "control-flow",
    title: "Control flow",
    blurb: "if / then / else, sequencing, and short-circuit logic.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/ControlFlow.tsx")),
  },
  {
    slug: "ordering",
    title: "Comparison & ordering",
    blurb: "Comparisons, and the three-way shape of a total order.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Ordering.tsx")),
  },
  {
    slug: "data",
    title: "Tuples & records",
    blurb: "Bundling values together, by position or by name.",
    section: "Fundamentals",
    exercises: 1,
    Component: lazy(() => import("./chapters/Data.tsx")),
  },
  {
    slug: "records-tuples",
    title: "Working with records & tuples",
    blurb: "Passing structured data around and reaching into it.",
    section: "Fundamentals",
    exercises: 3,
    Component: lazy(() => import("./chapters/RecordsTuples.tsx")),
  },
  {
    slug: "pattern-matching",
    title: "Pattern matching",
    blurb: "Deciding by shape; sum types and exhaustiveness.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/PatternMatching.tsx")),
  },
  {
    slug: "lists",
    title: "Lists",
    blurb: "Ordered, immutable sequences of values.",
    section: "Fundamentals",
    exercises: 3,
    Component: lazy(() => import("./chapters/Lists.tsx")),
  },
  {
    slug: "maps-sets",
    title: "Maps & sets",
    blurb: "Membership and key→value association, without duplicates.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/MapsSets.tsx")),
  },
  {
    slug: "strings",
    title: "Strings & text",
    blurb: "Unicode text, joining, and character vs byte length.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Strings.tsx")),
  },
  {
    slug: "bytes",
    title: "Bytes",
    blurb: "Raw octet sequences, and the bridge from text.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Bytes.tsx")),
  },
  {
    slug: "symbols",
    title: "Symbols",
    blurb: "Interned names, compared by identity.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Symbols.tsx")),
  },
  {
    slug: "errors",
    title: "Errors & absence",
    blurb: "Option, Result, safe indexing, and checked arithmetic.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Errors.tsx")),
  },
  {
    slug: "numbers",
    title: "The numeric model",
    blurb: "Checked integers and no silent promotion.",
    section: "Fundamentals",
    exercises: 1,
    Component: lazy(() => import("./chapters/Numbers.tsx")),
  },
  {
    slug: "sized-integers",
    title: "Sized integers",
    blurb: "Fixed-width integers that never convert silently.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/SizedIntegers.tsx")),
  },
  {
    slug: "floats",
    title: "Floating-point numbers",
    blurb: "Real-valued arithmetic with its own operators.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Floats.tsx")),
  },
  {
    slug: "effects",
    title: "Effects & handlers",
    blurb: "Perform an operation; a handler decides what it means.",
    section: "What makes Cadenza different",
    exercises: 3,
    Component: lazy(() => import("./chapters/Effects.tsx")),
  },
  {
    slug: "modules",
    title: "Modules",
    blurb: "Grouping definitions under a name; a module is a record of its exports.",
    section: "What makes Cadenza different",
    exercises: 2,
    Component: lazy(() => import("./chapters/Modules.tsx")),
  },
  {
    slug: "units",
    title: "Units of measure",
    blurb: "Carry a unit with a value; mix dimensions and it won't compile.",
    section: "What makes Cadenza different",
    exercises: 4,
    Component: lazy(() => import("./chapters/Units.tsx")),
  },
  {
    // NOT "playground": the top-level `/playground` route (the full IDE) is matched before `/:slug`,
    // so a chapter with that slug would be shadowed and unreachable. Use a distinct slug.
    slug: "using-the-playground",
    title: "The playground",
    blurb: "The full editor: a REPL, compiled-output views, and shareable links.",
    section: "Wrapping up",
    Component: lazy(() => import("./chapters/Playground.tsx")),
  },
  {
    slug: "whats-next",
    title: "Where to go next",
    blurb: "A recap, one last program, and how to keep exploring.",
    section: "Wrapping up",
    Component: lazy(() => import("./chapters/WhatsNext.tsx")),
  },
];

export function chapterAt(slug: string): { chapter: Chapter; index: number } | null {
  const index = CHAPTERS.findIndex((c) => c.slug === slug);
  if (index < 0) return null;
  return { chapter: CHAPTERS[index], index };
}

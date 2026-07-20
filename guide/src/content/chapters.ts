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
    exercises: 2,
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
    slug: "irrefutable-patterns",
    title: "Irrefutable patterns",
    blurb: "Destructuring that always matches, in a let and in arguments.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/IrrefutablePatterns.tsx")),
  },
  {
    slug: "iteration",
    title: "Iteration without loops",
    blurb: "Cadenza has no for or while; you repeat work with recursion and the fold family. Here's how, and why.",
    section: "Fundamentals",
    exercises: 0,
    Component: lazy(() => import("./chapters/Iteration.tsx")),
  },
  {
    slug: "lists",
    title: "Lists",
    blurb: "Ordered, immutable sequences of values.",
    section: "Fundamentals",
    exercises: 4,
    Component: lazy(() => import("./chapters/Lists.tsx")),
  },
  {
    slug: "iterators",
    title: "Iterators & ranges",
    blurb: "Lazy pull sequences: describe an endless range, produce only the elements you use.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/Iterators.tsx")),
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
    slug: "binary-matching",
    title: "Binary matching",
    blurb: "Describe a byte layout as typed segments; the (bin …) form builds and destructures Bytes.",
    section: "Fundamentals",
    exercises: 2,
    Component: lazy(() => import("./chapters/BinaryMatching.tsx")),
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
    slug: "numbers",
    title: "The numeric model",
    blurb: "Checked integers and no silent promotion.",
    section: "Fundamentals",
    exercises: 2,
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
    slug: "rationals",
    title: "Exact fractions",
    blurb: "Rationals: exact ratios, always in lowest terms — where floats round, these don't.",
    section: "Fundamentals",
    exercises: 3,
    Component: lazy(() => import("./chapters/Rationals.tsx")),
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
    slug: "effects",
    title: "Effects & handlers",
    blurb: "Perform an operation; a handler decides what it means.",
    section: "What makes Cadenza different",
    exercises: 3,
    Component: lazy(() => import("./chapters/Effects.tsx")),
  },
  {
    slug: "contracts",
    title: "Design by contract",
    blurb: "@requires and @ensures turn an assumption into an enforced check at the function boundary.",
    section: "What makes Cadenza different",
    exercises: 0,
    Component: lazy(() => import("./chapters/DesignByContract.tsx")),
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
    slug: "opaque-types",
    title: "Opaque types",
    blurb: "Export a type's name but keep its constructor private: values flow only through the module's functions, so an invariant can't be broken.",
    section: "What makes Cadenza different",
    exercises: 2,
    Component: lazy(() => import("./chapters/OpaqueTypes.tsx")),
  },
  {
    slug: "units",
    title: "Units of measure",
    blurb: "Carry a unit with a value; mix dimensions and it won't compile.",
    section: "What makes Cadenza different",
    exercises: 5,
    Component: lazy(() => import("./chapters/Units.tsx")),
  },
  {
    slug: "types-as-values",
    title: "Types as values",
    blurb: "A type is an ordinary value: reflect it, compare it, branch on it — all at compile time.",
    section: "What makes Cadenza different",
    exercises: 2,
    Component: lazy(() => import("./chapters/TypesAsValues.tsx")),
  },
  {
    slug: "const-parameters",
    title: "Const parameters",
    blurb: "Arguments known at compile time: inlined into a specialized copy and erased — a constant, a type, or a dictionary of behaviour.",
    section: "What makes Cadenza different",
    exercises: 2,
    Component: lazy(() => import("./chapters/ConstParameters.tsx")),
  },
  {
    slug: "ad-hoc-polymorphism",
    title: "Ad-hoc polymorphism",
    blurb: "One name, a per-type meaning: operators dispatch on operand type, generics specialize per type — all at compile time.",
    section: "What makes Cadenza different",
    exercises: 2,
    Component: lazy(() => import("./chapters/AdHocPolymorphism.tsx")),
  },
  {
    slug: "metaprogramming",
    title: "Metaprogramming",
    blurb: "Code is data: quote a program to an AST value, take it apart, build it up, eval it.",
    section: "What makes Cadenza different",
    exercises: 3,
    Component: lazy(() => import("./chapters/Metaprogramming.tsx")),
  },
  {
    slug: "property-testing",
    title: "Testing & properties",
    blurb: "@test marks a function as a test; give it parameters and the runner generates inputs and shrinks failures.",
    section: "What makes Cadenza different",
    exercises: 2,
    Component: lazy(() => import("./chapters/PropertyTesting.tsx")),
  },
  {
    slug: "example-apps",
    title: "Example applications",
    blurb: "Full interactive apps built in Cadenza — the differentiators running for real: playground, calculator, CAD, notebook.",
    section: "Example applications",
    Component: lazy(() => import("./chapters/ExampleApps.tsx")),
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
    slug: "cdz-toolchain",
    title: "The cdz toolchain",
    blurb: "The same compiler as one binary on your PATH: compile, run, a cargo-style project workflow, and code queries for humans and agents.",
    section: "Wrapping up",
    Component: lazy(() => import("./chapters/CdzToolchain.tsx")),
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

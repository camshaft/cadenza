/// Canonical example notebooks — authored markdown documents that showcase the notebook surface (value
/// cells, tables, charts, and the reactive widget→recompute core). These are the starter content the
/// /notebook route can offer and the fixtures check:visual + docs draw from. Each is a plain string so
/// it's pure + testable (examples.test.ts pins that every one parses into well-formed cells).
///
/// All code cells are s-expr + self-contained (no imports — the browser compiler can't resolve library
/// modules; the /cad rationale). A Float64 slider value grounds with a decimal point.

export interface ExampleNotebook {
  slug: string;
  title: string;
  markdown: string;
}

/// The flagship: a reactive compound-interest notebook — a rate slider drives a balance value + a
/// yearly-schedule table that recompute as you drag.
const COMPOUND_INTEREST = `# Compound interest

Adjust the **rate** and watch the balance and schedule recompute.

~~~cadenza widget
rate : Float64 = slider(0.0, 0.2, step: 0.01, default: 0.05)
~~~

The balance after one year on a 1000.0 principal:

~~~cadenza
(def (main) (* 1000.0 (+ 1.0 rate)))
~~~

A short growth schedule (year, factor):

~~~cadenza table
(def (main) (list (tuple 1 (+ 1.0 rate)) (tuple 2 (* (+ 1.0 rate) (+ 1.0 rate)))))
~~~`;

/// A table-focused example: structured rows render as an HTML table.
const TABLE_DEMO = `# Tables

A **table** cell renders a List of tuples (positional columns) or records (named columns).

~~~cadenza table
(def (main) (list (tuple 1 100) (tuple 2 121) (tuple 3 133)))
~~~`;

/// A chart-focused example: a List of points renders as a hand-rolled SVG line chart.
const CHART_DEMO = `# Charts

A **chart:line** cell plots a List of (x, y) points.

~~~cadenza chart:line
(def (main) (list (tuple 1 10) (tuple 2 20) (tuple 3 15) (tuple 4 25)))
~~~`;

/// A plain-value example: any cell with no directive renders its value.
const VALUE_DEMO = `# Values

A code cell with no directive shows its computed value.

~~~cadenza
(def (main) (+ (* 6 7) 0))
~~~`;

export const EXAMPLES: ExampleNotebook[] = [
  { slug: "compound-interest", title: "Compound interest", markdown: COMPOUND_INTEREST },
  { slug: "tables", title: "Tables", markdown: TABLE_DEMO },
  { slug: "charts", title: "Charts", markdown: CHART_DEMO },
  { slug: "values", title: "Values", markdown: VALUE_DEMO },
];

/// The default notebook the /notebook route opens with.
export const DEFAULT_EXAMPLE = EXAMPLES[0];

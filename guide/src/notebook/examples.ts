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

/// A loan-repayment showcase: a `number` principal + a `rate` slider + a `payment` slider drive a live
/// declining-balance value + chart. Shows off THREE widget types (number + two sliders) plus a chart:line,
/// all recomputing as you drag. Simple (non-compounding) interest, unrolled with +/-/* only so it compiles
/// self-contained in the browser (no division/pow, no imports — the /cad + prelude-only constraint).
const LOAN = `# Loan repayment

Set the **principal**, the interest **rate**, and a fixed yearly **payment**, then watch the balance pay
down.

~~~cadenza widget
principal : Float64 = number(default: 1000.0)
rate : Float64 = slider(0.0, 0.2, step: 0.01, default: 0.05)
payment : Float64 = slider(0.0, 500.0, step: 50.0, default: 200.0)
~~~

The balance after each year is last year's balance plus interest, minus the payment:

~~~cadenza
(def (year1) (- (+ principal (* principal rate)) payment))
(def (year2) (- (+ year1 (* year1 rate)) payment))
(def (year3) (- (+ year2 (* year2 rate)) payment))
(def (main) year1)
~~~

The balance over three years — drag a control and the curve moves:

~~~cadenza chart:line
(def (main) (list (tuple 0 principal) (tuple 1 year1) (tuple 2 year2) (tuple 3 year3)))
~~~`;

/// A projectile-motion showcase: two sliders (upward velocity + gravity) drive a live height-vs-time
/// parabola. Height at time t is v*t - 0.5*g*t*t, evaluated at a few discrete t with +/-/* on Float64
/// literals (no sqrt/division — keeps it browser-self-contained like the others). The chart:line traces
/// the arc; drag either slider and the parabola reshapes.
const PROJECTILE = `# Projectile motion

Launch straight up with a **velocity**, pulled down by **gravity** — the height traces a parabola.

~~~cadenza widget
velocity : Float64 = slider(0.0, 50.0, step: 1.0, default: 30.0)
gravity : Float64 = slider(1.0, 20.0, step: 0.5, default: 9.8)
~~~

Height at a few times \`t\` (height = velocity·t − ½·gravity·t·t):

~~~cadenza
(def (h1) (- (* velocity 1.0) (* (* 0.5 gravity) (* 1.0 1.0))))
(def (h2) (- (* velocity 2.0) (* (* 0.5 gravity) (* 2.0 2.0))))
(def (h3) (- (* velocity 3.0) (* (* 0.5 gravity) (* 3.0 3.0))))
(def (h4) (- (* velocity 4.0) (* (* 0.5 gravity) (* 4.0 4.0))))
(def (main) h2)
~~~

The trajectory over time — drag velocity or gravity and the arc moves:

~~~cadenza chart:line
(def (main) (list (tuple 0 0.0) (tuple 1 h1) (tuple 2 h2) (tuple 3 h3) (tuple 4 h4)))
~~~`;

/// A quadratic-explorer showcase: three Int64 sliders (a, b, c) reshape the parabola y = a·x² + b·x + c live.
/// Evaluated at a spread of x (including NEGATIVE x, exercising the chart's negative-coordinate axis). The
/// notebook is rational-by-default (no floats), so `y-at`'s param is `Rational` to compose with the
/// coefficients; +/* on the rational-grounded integers. Drag a/b/c and the curve bends.
const QUADRATIC = `# Quadratic explorer

Shape the parabola **y = a·x² + b·x + c** — drag the coefficients and watch it bend.

~~~cadenza widget
a : Int64 = slider(-5, 5, default: 1)
b : Int64 = slider(-5, 5, default: 0)
c : Int64 = slider(-5, 5, default: 0)
~~~

Evaluate y at each x (y = a·x·x + b·x + c). The parameter is \`Rational\` so it composes with the
rational-by-default coefficients:

~~~cadenza
(def (y-at (: x Rational)) (+ (+ (* a (* x x)) (* b x)) c))
(def (main) (y-at 0))
~~~

The parabola from x = −3 to 3 — drag a, b, or c to reshape it:

~~~cadenza chart:line
(def (main) (list
  (tuple -3 (y-at -3)) (tuple -2 (y-at -2)) (tuple -1 (y-at -1))
  (tuple 0 (y-at 0)) (tuple 1 (y-at 1)) (tuple 2 (y-at 2)) (tuple 3 (y-at 3))))
~~~`;

/// A formula-focused example: the `formula` directive typesets a scalar / exact fraction / quantity.
/// Two Int64 sliders drive plain `num / den` — the notebook runs RATIONAL-BY-DEFAULT (assembleForRun/cellIde
/// prepend `default-fraction Rational`), so the bare integers ground to exact Rationals and `/` yields the
/// exact fraction 3/4 (NOT integer division's 0) the FormulaView renders as a stacked n/d. Completes the
/// rich-output family showcase (value / table / chart / formula). Operator directive: no floats, division
/// just works under the rational default — no `Rational.of` workaround.
const FORMULA_DEMO = `# Formulas

A **formula** cell typesets a scalar, an exact fraction, or a quantity. Drag **num** and **den** to
reshape the fraction:

~~~cadenza widget
num : Int64 = slider(1, 9, default: 3)
den : Int64 = slider(1, 9, default: 4)
~~~

The ratio as an **exact fraction** — the notebook uses rational-by-default literals, so plain \`num / den\`
is exact (3 / 4 = 3/4, not integer division's 0):

~~~cadenza formula
(def (main) (/ num den))
~~~`;

export const EXAMPLES: ExampleNotebook[] = [
  { slug: "compound-interest", title: "Compound interest", markdown: COMPOUND_INTEREST },
  { slug: "loan", title: "Loan repayment", markdown: LOAN },
  { slug: "projectile", title: "Projectile motion", markdown: PROJECTILE },
  { slug: "quadratic", title: "Quadratic explorer", markdown: QUADRATIC },
  { slug: "tables", title: "Tables", markdown: TABLE_DEMO },
  { slug: "charts", title: "Charts", markdown: CHART_DEMO },
  { slug: "formulas", title: "Formulas", markdown: FORMULA_DEMO },
  { slug: "values", title: "Values", markdown: VALUE_DEMO },
];

/// The default notebook the /notebook route opens with.
export const DEFAULT_EXAMPLE = EXAMPLES[0];

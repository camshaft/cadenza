/// Canonical example notebooks — authored markdown documents that showcase the notebook surface (value
/// cells, tables, charts, and the reactive widget→recompute core). These are the starter content the
/// /notebook route can offer and the fixtures check:visual + docs draw from. Each is a plain string so
/// it's pure + testable (examples.test.ts pins that every one parses into well-formed cells).
///
/// All code cells are s-expr + self-contained (no imports — the browser compiler can't resolve library
/// modules; the /cad rationale). The notebook is RATIONAL-BY-DEFAULT (operator: no floats) — every literal
/// grounds to an exact Rational, so widgets are Int64 sliders and cells use exact rational arithmetic.

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
rate : Int64 = slider(0, 20, default: 5)
~~~

The balance after one year on a 1000 principal (rate is a whole-percent; \`rate / 100\` is exact):

~~~cadenza
(def (main) (* 1000 (+ 1 (/ rate 100))))
~~~

A short growth schedule (year, factor):

~~~cadenza table
(def (main) #list(#tuple(1 (+ 1 (/ rate 100))) #tuple(2 (* (+ 1 (/ rate 100)) (+ 1 (/ rate 100))))))
~~~`;

/// A table-focused example: structured rows render as an HTML table.
const TABLE_DEMO = `# Tables

A **table** cell renders a List of tuples (positional columns) or records (named columns).

~~~cadenza table
(def (main) #list(#tuple(1 100) #tuple(2 121) #tuple(3 133)))
~~~`;

/// A chart-focused example: a List of points renders as a hand-rolled SVG line chart.
const CHART_DEMO = `# Charts

A **chart:line** cell plots a List of (x, y) points.

~~~cadenza chart:line
(def (main) #list(#tuple(1 10) #tuple(2 20) #tuple(3 15) #tuple(4 25)))
~~~`;

/// A chart-TYPES showcase: the newer `area` (filled line) + `stacked` (bars accumulated per x) renderers,
/// driven by a slider so a reader sees them recompute. Stacked uses multi-y tuples `(tuple x y0 y1)` — two
/// series sharing an x that stack into a per-x total. Rational-by-default (no floats): the `boost` slider is
/// a whole Int64 the cells add in. Completes the chart-family showcase beyond the line/scatter/bar basics.
const CHART_TYPES_DEMO = `# Chart types

Beyond a plain line, a chart cell can render an **area** (filled line) or **stacked** bars. Drag **boost** and
watch both recompute.

~~~cadenza widget
boost : Int64 = slider(0, 20, default: 5)
~~~

An **area** chart fills the region under the curve:

~~~cadenza chart:area
(def (main) #list(#tuple(1 (+ 10 boost)) #tuple(2 (+ 20 boost)) #tuple(3 (+ 15 boost)) #tuple(4 (+ 25 boost))))
~~~

A **stacked** chart accumulates each series per x, so the column total is the sum. Each row is \`(x y0 y1)\`, so
two series stack:

~~~cadenza chart:stacked
(def (main) #list(#tuple(1 (+ 5 boost) 3) #tuple(2 (+ 8 boost) 6) #tuple(3 (+ 6 boost) 4)))
~~~`;

/// A RECORDS showcase: build a record with named fields and read one back with field access `(. r field)`.
/// Exercises the record literal + field-access binding-pattern shapes on the notebook run + surface-toggle
/// round-trip path (v-syntax landed record/ctor ML round-trip; this pins it stays clean through s-expr↔ML for
/// a shipped notebook cell). A `width` slider drives the record so it recomputes live. Rational-by-default
/// (no floats — width + height are whole Int64).
const RECORDS_DEMO = `# Records

A **record** groups named fields. Drag **width** and the rectangle's area recomputes.

~~~cadenza widget
width : Int64 = slider(1, 20, default: 4)
~~~

Build a rectangle record, then read its fields with \`(. r field)\`:

~~~cadenza
(def (rect) #record((= w width) (= h 3)))
(def (main) (* (. (rect) w) (. (rect) h)))
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
principal : Int64 = number(default: 1000)
rate : Int64 = slider(0, 20, default: 5)
payment : Int64 = slider(0, 500, step: 50, default: 200)
~~~

The balance after each year is last year's balance plus interest (rate is a whole-percent, so \`rate / 100\`
is exact), minus the payment:

~~~cadenza
(def (year1) (- (+ principal (* principal (/ rate 100))) payment))
(def (year2) (- (+ year1 (* year1 (/ rate 100))) payment))
(def (year3) (- (+ year2 (* year2 (/ rate 100))) payment))
(def (main) year1)
~~~

The balance over three years, drag a control and the curve moves:

~~~cadenza chart:line
(def (main) #list(#tuple(0 principal) #tuple(1 year1) #tuple(2 year2) #tuple(3 year3)))
~~~`;

/// A projectile-motion showcase: two sliders (upward velocity + gravity) drive a live height-vs-time
/// parabola. Height at time t is v·t − ½·g·t·t, evaluated at a few discrete integer t with +/-/* and the
/// exact rational ½ = `1 / 2` (rational-by-default — no floats, browser-self-contained). The chart:line
/// traces the arc; drag either slider and the parabola reshapes.
const PROJECTILE = `# Projectile motion

Launch straight up with a **velocity**, pulled down by **gravity**, so the height traces a parabola.

~~~cadenza widget
velocity : Int64 = slider(0, 50, default: 30)
gravity : Int64 = slider(1, 20, default: 10)
~~~

Height at a few times \`t\` (height = velocity·t − ½·gravity·t·t; ½ is the exact rational \`1 / 2\`):

~~~cadenza
(def (h1) (- (* velocity 1) (* (* (/ 1 2) gravity) (* 1 1))))
(def (h2) (- (* velocity 2) (* (* (/ 1 2) gravity) (* 2 2))))
(def (h3) (- (* velocity 3) (* (* (/ 1 2) gravity) (* 3 3))))
(def (h4) (- (* velocity 4) (* (* (/ 1 2) gravity) (* 4 4))))
(def (main) h2)
~~~

The trajectory over time, drag velocity or gravity and the arc moves:

~~~cadenza chart:line
(def (main) #list(#tuple(0 0) #tuple(1 h1) #tuple(2 h2) #tuple(3 h3) #tuple(4 h4)))
~~~`;

/// A quadratic-explorer showcase: three Int64 sliders (a, b, c) reshape the parabola y = a·x² + b·x + c live.
/// Evaluated at a spread of x (including NEGATIVE x, exercising the chart's negative-coordinate axis). The
/// notebook is rational-by-default (no floats), so `y-at`'s param is `Rational` to compose with the
/// coefficients; +/* on the rational-grounded integers. Drag a/b/c and the curve bends.
const QUADRATIC = `# Quadratic explorer

Shape the parabola **y = a·x² + b·x + c**: drag the coefficients and watch it bend.

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

The parabola from x = −3 to 3, drag a, b, or c to reshape it:

~~~cadenza chart:line
(def (main) #list(
  #tuple(-3 (y-at -3)) #tuple(-2 (y-at -2)) #tuple(-1 (y-at -1))
  #tuple(0 (y-at 0)) #tuple(1 (y-at 1)) #tuple(2 (y-at 2)) #tuple(3 (y-at 3))))
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

The ratio as an **exact fraction**, since the notebook uses rational-by-default literals, so plain \`num / den\`
is exact (3 / 4 = 3/4, not integer division's 0):

~~~cadenza formula
(def (main) (/ num den))
~~~`;

/// A CONTROLS showcase: the four widget kinds beyond the slider — a checkbox (Bool), a dropdown and a radio
/// (String single-choice), and a text field (String). Each drives its own cell so every control is a live,
/// referenced input (checkbox → `(if on …)`; the String pickers/text return their value, which renders as a
/// String cell). Closes the widget-family showcase gap: the shipped examples demonstrated only slider +
/// number, leaving checkbox/dropdown/radio/text implemented-but-invisible. Rational-by-default (no floats —
/// the checkbox cell's branches are whole Int64 that ground to Rational).
const CONTROLS_DEMO = `# Controls

Beyond sliders, a widget cell offers a **checkbox** (a Bool), a **dropdown** and a **radio** (pick one
String), and a **text** field. Each control below drives its own cell, so flip, pick, and type to see them
recompute.

~~~cadenza widget
on : Bool = checkbox(default: true)
mode : String = dropdown("balance", "schedule", default: "balance")
scale : String = radio("linear", "log", default: "linear")
label : String = text(default: "Demo")
~~~

A **checkbox** is a Bool, so branch on it with \`if\`:

~~~cadenza
(def (main) (if on 100 0))
~~~

A **dropdown** and a **radio** each pick one String; a cell reads the chosen value:

~~~cadenza
(def (main) mode)
~~~

~~~cadenza
(def (main) scale)
~~~

A **text** field is free String input:

~~~cadenza
(def (main) label)
~~~`;

/// A UNITS-OF-MEASURE showcase: a value cell whose result is a QUANTITY renders in the concise unit
/// surface (`100 meter`, `25/2 meter/second`) rather than the raw canonical form. Two sliders (a distance
/// and a time) drive a base-unit quantity and a DERIVED-unit quantity (a speed = distance / time), so both
/// the base-unit and the `Unit./` quotient display paths are exercised live. A quantity is built with
/// `Qty.of <magnitude> <unit>`; `Unit.base` names a base dimension by a `#"…"` symbol and `Unit./` forms a
/// quotient. The notebook runs rational-by-default, so `distance / time` is an EXACT fraction magnitude
/// (operator: no floats). Gives the quantity value-render path (value/table friendly display) an
/// end-to-end example + gate coverage; before this, no shipped example produced a quantity.
const UNITS_DEMO = `# Units of measure

A cell whose value is a **quantity** shows its magnitude with the unit attached, so drag the sliders and
watch the units come along.

~~~cadenza widget
distance : Int64 = slider(1, 100, default: 100)
time : Int64 = slider(1, 60, default: 8)
~~~

The **distance** is a base-unit quantity, a length in \`meter\`:

~~~cadenza value
(def (main) (Qty.of distance (Unit.base #"meter")))
~~~

Dividing distance by **time** gives a **speed** with a derived unit, the quotient \`meter / second\`:

~~~cadenza value
(def (main) (Qty.of (/ distance time) (Unit./ (Unit.base #"meter") (Unit.base #"second"))))
~~~`;

export const EXAMPLES: ExampleNotebook[] = [
  { slug: "compound-interest", title: "Compound interest", markdown: COMPOUND_INTEREST },
  { slug: "loan", title: "Loan repayment", markdown: LOAN },
  { slug: "projectile", title: "Projectile motion", markdown: PROJECTILE },
  { slug: "quadratic", title: "Quadratic explorer", markdown: QUADRATIC },
  { slug: "tables", title: "Tables", markdown: TABLE_DEMO },
  { slug: "charts", title: "Charts", markdown: CHART_DEMO },
  { slug: "chart-types", title: "Chart types", markdown: CHART_TYPES_DEMO },
  { slug: "records", title: "Records", markdown: RECORDS_DEMO },
  { slug: "formulas", title: "Formulas", markdown: FORMULA_DEMO },
  { slug: "controls", title: "Controls", markdown: CONTROLS_DEMO },
  { slug: "values", title: "Values", markdown: VALUE_DEMO },
  { slug: "units", title: "Units of measure", markdown: UNITS_DEMO },
];

/// The default notebook the /notebook route opens with.
export const DEFAULT_EXAMPLE = EXAMPLES[0];

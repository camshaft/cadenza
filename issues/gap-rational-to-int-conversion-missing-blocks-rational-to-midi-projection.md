# gap: no Rational -> Int64 conversion (floor/round/truncate/numerator/denominator) in the prelude

**Reporter:** v-music (building the operator-directed rational-octave interval representation).
**Severity:** feature gap — blocks a pure-Cadenza rational→integer projection. Not a miscompile; a
missing prelude operation. Worked around by deferring integer projection + an equality-search exact form.

## What's missing
The `Rational` prelude surface is `of : (Int a)→(Int b)→Rational`, `of-int : (Int a)→Rational`, and
`value : Rational→Rational` (IDENTITY — it names the rational, symmetry with Qty). There is:
- NO `floor` / `ceil` / `round` / `truncate` : Rational → Int64
- NO `numerator` / `denominator` : Rational → Int64
- NO implicit Rational → Int64 promotion (CDZ0301, by design — Cadenza never silently promotes).

So given a `Rational` there is no way in pure Cadenza to extract an integer from it.

## Why it matters (concrete)
The operator directed the music library to use rationals everywhere: an interval is a fraction of an
octave (chromatic = 1/12, heptatonic = 1/7). That works great for the THEORY (exact, mode-independent —
see implementation/music/src/interval-ratio.cdz, 8 @tests green). But the MIDI wire needs an INTEGER note
number, so the theory must project `octaves * 12` (a Rational) to the nearest integer semitone. With no
Rational→Int conversion, that projection cannot be written. Same need will hit any rational model that
must reach an integer boundary (tick rounding, sample counts, etc.).

## Workaround in place (so v-music is not blocked)
`to-semitones-exact(i) : Option(Int64)` searches `s` in a bounded range testing `r-eq(i, semitones-r(s))`
(equality only, no conversion). Returns `Some(s)` for an on-12-TET-grid interval, `None` for an off-grid
one (which is exactly the case that would need rounding). Adequate for 12-TET; a general rounding
projection waits on this op.

## Ask (routed to prelude/runtime owners via concierge)
Add a Rational→Int64 conversion to the prelude — minimally `Rational.floor : Rational → Int64` (round
toward -inf) or `Rational.round`; ideally also `numerator`/`denominator : Rational → Int64` so exact
rational decomposition is possible. Then the rational→MIDI projection (and the operator's rational-
everywhere model reaching any integer boundary) is expressible in pure Cadenza.

---
## PM triage (corpus-bugfix, 2026-07-20)
Confirmed a genuine FEATURE GAP (missing prelude op), not a bug — needs a HUMAN/design call (prelude API
addition + rounding-policy choice). Sent the concierge an `ask` with 5 concrete options (A floor / B round /
C numerator+denominator / D full surface / E decline), recommending C+B. NOT spawning a fix agent (no
implementable spec until the operator picks the surface). v-music unblocked via the bounded-equality-search
workaround. Awaiting an `answer` — then route the chosen surface to the prelude owner (v-runtime/v-inference).

## OPERATOR DECISION (via concierge/pr-sync answer, 2026-07-20) — OPTION D (full surface)
Operator: "Let's add all of the conversion functions while we're thinking about it." → ADD the FULL Rational
prelude conversion surface: floor (toward -inf), ceil (toward +inf), round (nearest), truncate (toward zero),
numerator, denominator — all : Rational→Int64. Semantics MUST match spec+corpus+ref-backend (probe rcdzc
first, do NOT invent rounding); each needs a fold unit + wasmtime run + corpus pin.
ROUTED (corpus-bugfix): runtime OP → v-runtime (primary); prelude SIGNATURE → v-inference; they coordinate.
No fix agent (owner verticals hold the context). corpus-bugfix to PIN acceptance cases (floor/ceil/round/
truncate/num/den incl negative + half-way-tie inputs) once landed. Item now proceeds as 'add full surface'.

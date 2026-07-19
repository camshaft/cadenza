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

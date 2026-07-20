# Cadenza Music — design & architecture

Pure-Cadenza music-theory + synthesis library (operator directive 2026-07-19). The whole model is
ordinary Cadenza data; per-surface **drivers** (native audio, browser WebMIDI/WebAudio) consume it —
the same model-vs-driver split as CAD's CSG. Reference: `camshaft/euphony-rs` (cloned read-only under
`reference/`, gitignored — study the primitives, port the ideas, write them fresh; **not** a dependency).

## The two-layer representation (the core architectural decision)

The operator directed **rationals everywhere**: an interval is a fraction of an OCTAVE, a duration a
fraction of a WHOLE NOTE — so relationships are **mode-, tempo-, and resolution-independent** and exact
(no float, no drift). This is euphony's model (`Interval`, `Beat`, `Instant` are all ratios). But the
MIDI wire needs **integer** note numbers and tick counts. So the library is two layers:

| Layer | Representation | Modules | Role |
|-------|---------------|---------|------|
| **Theory CORE** | exact `Rational` | `interval-ratio`, `scale-ratio`, `chord-ratio`, `rhythm-ratio` | the mode-independent music model — the operator's rationals-everywhere directive |
| **MIDI PROJECTION** | `Int64` (semitones / ticks) | `pitch`, `scale`, `chord`, `rhythm`, `schedule`, `compose`, `piece` | the driver-facing wire form; what WebMIDI / a scheduler consumes |

Concierge-confirmed (tick 18): rational is the CORE; integer note numbers survive **only** as the thin
driver-edge projection (euphony keeps `frequency.rs` separate from `interval.rs`; matches the
maximal-logic-in-Cadenza mandate — theory in Cadenza, wire-integers only where the boundary forces them).

**Known gap (reported → in progress):** projecting a general rational interval to an integer semitone
count needs a `Rational → Int64` conversion (floor/ceil/round/truncate). `Rational.numerator`/`denominator`
now exist in the prelude but return **BigInt** (no BigInt→Int64 narrowing yet), so the general projection
is still not expressible in pure Cadenza. The full conversion surface (floor/ceil/round/truncate +
Int64 numerator/denominator, `round` = half-away-from-zero) is operator-approved and being built by
v-runtime/v-inference; v-music is the consumer and will wire the projection when it lands. Meanwhile the
on-12-TET-grid case works via `interval-ratio.to-semitones-exact : Option(Int64)` (equality search over the
grid), and `layer-bridge` gates that the rational core and the Int64 projection agree across the full
12-TET grid (0..12 semitones).

## Modules

### Theory core (rational)
- **interval-ratio** — `RInterval(Rational)` = octave-count. `edo-step(n,m)=n/m`, `semitones-r`,
  `degree-r`, exact arithmetic, `r-complement` (inversion), named intervals.
- **scale-ratio** — `RScale(tonic, degrees)` over octave-fraction degrees (heptatonic `k/7`, chromatic
  `k/12`); membership + degrees exact, mode-independent, transposition-invariant.
- **chord-ratio** — `RChord(root, stack)`; triad/7th qualities as twelfths, exact membership.
- **rhythm-ratio** — `RDuration(Rational)` = whole-note-fraction; note values, dotted (3/2), tuplets
  (exact — a triplet is `2/3`, three triplet-eighths close a quarter with zero drift).

### MIDI projection (Int64)
- **pitch** — `Pitch` over MIDI note numbers (60 = middle C); `pitch-class`/`octave` floored,
  `Interval` signed semitones, transpose/invert/interval-class.
- **scale / chord / rhythm** — the 12-TET integer forms (modes as step rotations; triads/7ths as semitone
  stacks; PPQ-960 tick durations + `Meter`/`quantize`).
- **schedule** — the MIDI event model + the note-on/off scheduler. `Note` → paired on/off `MidiEvent`s;
  the **stuck-key invariant** `balanced` (per-(channel,note) non-negative-prefix + return-to-zero — the
  correctness backbone, strengthened after PR#648); transforms transpose/remap/loop; `play-order`
  (stable tick-sort → any fixed polyphonic piece plays over ONE task).
- **compose / piece** — `chord-block`/`arpeggiate`/`sequence` place realized notes in time; `piece` is a
  I–V–vi–IV showcase (accompaniment + bass) built from the primitives, an end-to-end gate.

### Live-coding (pattern layer)
- **pattern** — a Strudel/Tidal-style cycle-based pattern as a recursive DATA tree
  (`Silence | Atom | Seq | Stack | Fast`) rendered to timed `Note`s over a cycle (which `schedule` lowers
  to balanced MIDI). Combinators: `silence`/`note-pat`/`seq`/`stack`/`fast`, `euclid` (Bjorklund/Euclidean
  rhythms, E(3,8)=tresillo), `rev` (reverse-in-time), `render-cycles`/`render-every` (Tidal `every n f p`,
  a higher-order per-cycle transform). Seq tiles on exact cumulative integer boundaries (no drift).

### Analysis & derived layers (Int64 pitch-class space)
- **interval-name** — name an interval by size: `IntervalName` (unison/m2..M7/P4/P5/tritone/octave/Compound)
  + `is-perfect` classifier; direction- and octave-independent.
- **consonance** — common-practice `is-consonant`/`is-dissonant`/`is-perfect-consonance` over interval size.
- **analysis** — chord IDENTIFICATION (inverse of `chord` construction): `identify(notes, root) : Quality`
  names a note set (triads + sevenths, else `Unknown`) via a root-relative pitch-class signature;
  octave/inversion/order/duplicate-invariant. (`analysis-roundtrip` is a gate pinning the construct↔identify
  inverse.)
- **key** — key DETECTION: `candidate-keys(pcs)` = the major keys whose scale contains all input pitch
  classes (a C triad → C/F/G; G7 → C uniquely); `fits-key`/`candidate-count`.
- **progression** — diatonic (Roman-numeral) harmony: `diatonic-triad`/`diatonic-seventh` stack scale
  thirds so qualities fall out of the scale (I/IV/V major, ii/iii/vi minor, vii° dim); `progression` maps a
  degree list to chords (a I-IV-V).
- **voicing** — voice-leading: `nearest-voicing`/`nearest-inversion` pick the inversion minimizing total
  per-voice semitone motion from a reference; `voice-distance` metric.
- **melody** — melodic contour: `steps` (note-to-note intervals), `range` (ambitus), `leap-count`/
  `is-conjunct`, `net-direction`.

### Synthesis
- **synth** — the synth graph as DATA (CSG-for-CAD idiom): a recursive `Synth` sum (osc/gain/envelope/mix)
  + folds; a build-your-own-synth surface for a thin WebAudio driver.
- **adsr** — a full ADSR amplitude envelope: `Adsr` data + `sample(env, held, t)` (exact integer-lerp
  per-mille amplitude at a tick), `is-active` (voice-freeing), `total-span`. Extends synth's linear `Env`.

## Phase 2 — demos (in progress)
1. **DES-composed piece** — a piece played over the discrete-event-sim clock. v-music is the forcing
   consumer of `implementation/des`; the seam is `run-sim(List(MidiEvent), fn(_u) => play-task(...))`
   where `play-task` walks `play-order(schedule(piece))`, `Sim.sleep`s to each tick, and returns the
   played-in-order event stream. Monophonic and flattened-polyphonic both play over the single-task
   scheduler; concurrent per-voice tasks (parked multi-task run-sim) are only needed for LIVE add/remove.
2. **Live-coding env**, 3. **MIDI pipeline processor** (parametric), 4. **MIDI looping pedal** — browser
   demos consuming the model through the WebMIDI/WebAudio drivers.

## Gate / ownership
Standing vertical (`--vertical music --area music`). Gate = `cdz test implementation/music` (the `@test`
suites), wired into `cargo xtask check` alongside cad/compiler-ml/iterators/choreography. One gated slice
per tick. Compiler/language gaps hit while building are REPORTED (a `.sexp`/`.md` probe into the queue),
not papered over — that's the point of the vertical (stress the language).

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

**Known gap (reported):** projecting a rational interval to an integer semitone count needs a
`Rational → Int64` conversion (floor/round/numerator/denominator), and the prelude has none
(`Rational.value` is identity). So the general rational→MIDI projection is deferred to the driver edge;
`interval-ratio.to-semitones-exact : Option(Int64)` covers the on-12-TET-grid case via equality search.
Filed for the prelude/runtime owners.

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

### Synthesis
- **synth** — the synth graph as DATA (CSG-for-CAD idiom): a recursive `Synth` sum (osc/gain/envelope/mix)
  + folds; a build-your-own-synth surface for a thin WebAudio driver.

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

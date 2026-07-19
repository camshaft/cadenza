# Cadenza Music — a music-theory + synthesis library, in Cadenza (operator directive 2026-07-19)

Build music **in Cadenza code**: the theory (intervals, scales, modes, rhythm), the events (a
composable piece), the synthesis (synth defs, audio effects, synthesizers), and the browser demos
(live-coding, a MIDI pipeline, a looping pedal). Reference: [camshaft/euphony-rs](https://github.com/camshaft/euphony-rs)
— cloned read-only as the design reference for the primitives, **not a dependency** (Cadenza is
dependency-free; this mirrors the rcdzc-copies-syntax-verbatim pattern — study it, port the ideas,
write them fresh in Cadenza).

## Architecture mandate (operator)
**Move as much logic into Cadenza as absolutely possible** — only small browser integration layers
(WebMIDI/WebAudio glue, the live-coding editor shell). The theory, the event model, the DSP/synthesis
graph, the scheduler: all pure Cadenza. The GOAL is to **stress the language** — show its capability
and surface real compiler/runtime bugs + feature gaps. A bug you hit is a WIN: report it (file a
`.sexp` probe into the queue for the breaker/PM path) rather than papering over it, exactly like the
compiler-ml self-host vertical.

This directory is the pure-Cadenza model layer. Per-surface **drivers** (native audio-out, browser
WebMIDI/WebAudio) consume the model as data, kept separate — same split as CAD's model-vs-driver.

## Phase 1 — PRIMITIVES first (the whole of phase 1 before demos)
Track every music-theory primitive, built + gated in Cadenza:
- **pitch/intervals** — pitch classes, octaves, an `Interval` (semitone distance + quality),
  transposition, inversion, enharmonics.
- **scales/modes** — the diatonic modes (Ionian…Locrian), building a scale from a root + mode,
  degrees, scale-membership, collapsing a chromatic note into a mode.
- **chords** — triads/7ths from a root + quality, inversions, voicing.
- **rhythm/beats** — beats, subdivisions, meter, a duration/time model (align with DES time later).
Each primitive lands with corpus/`@test` coverage that FAILS if it regresses (you own your gate).

## Phase 2 — DEMOS (only after the primitives)
1. **DES-composed piece** — use the discrete-event-sim (design-des) to emit events composed
   euphony-style into a piece. (Forcing consumer of DES; coordinate via `note`.)
2. **Live-coding env** — a Strudel/Tidal-Cycles-style browser live-coding environment for music.
3. **MIDI pipeline processor** — WebMIDI in → transform → schedule out, same IDE/demo interface,
   PARAMETRIC (CAD-`@param`-slider style). E.g. collapse chromatic notes to a mode (say major), then
   transpose + expand into another mode.
4. **MIDI looping pedal** — record MIDI events, quantize, play back on different channels; a full
   browser looping pedal.

## Synthesis (operator extension) — synth defs + effects + synthesizers, in Cadenza
Besides note/event theory, define **synths, audio effects, and synthesizers IN Cadenza**: an audio
graph / DSP description as ordinary Cadenza data (like CSG is for CAD), consumed by a thin WebAudio (or
native) driver. Build-your-own effects + synthesizers is the target. Fits the maximal-logic-in-Cadenza
mandate; slots alongside the theory primitives (primitives still come first).

## MIDI scheduler (required)
A scheduler that tracks note-on/note-off pairing so we never get **stuck keys** — count outstanding
on/off so every on is eventually offed (across looping, transposition, channel remap). This is the
correctness backbone of the MIDI demos; design it early.

## Gate / ownership
Standing vertical (`--vertical music --area music`), pure-Cadenza `.cdz` under `implementation/music/`.
Gate = `cdz test implementation/music` (`@test` suites) wired into `cargo xtask check` (mirror how
`implementation/cad` + `implementation/compiler-ml` suites are gated). One gated slice per tick,
merge-request to pr-sync. Report language/compiler gaps you hit — that's the point of this vertical.
Reference clone: `implementation/music/reference/euphony-rs` (gitignored, read-only).

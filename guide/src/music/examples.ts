/// The starter example models for the /music route's example-switcher — the three v1 music-theory showcases.
/// Each is a self-contained model built against the PRELOADED music libs (staged from implementation/music):
/// the reader's buffer holds ONLY the model — the `import … from "<mod>"` clauses are auto-injected by
/// MusicPage's `injectImport` (musicPreload.ts), so the buffers stay clean. Mirrors /cad's `examples.ts`.
///
/// CONTENT OWNERSHIP: these three showcases are v-guide's to author + narrate (v-music = feature authority on
/// music semantics); v-guide-infra (this file's scaffolding) owns the page mechanism. These are STARTER bodies
/// (the confirmed import surface, minimal payoff-witness) so the page + gate have runnable content now — the
/// final tone-passed, value-pinned versions land from v-guide against this harness. The import surface is
/// authoritative (v-music via v-guide-editor): R1 interval-ratio, R2 chord+pitch, R3 piece+schedule.

import type { Surface } from "../compiler/client.ts";

export interface ExampleModel {
  /// A stable kebab-case id (the picker's value + a test key).
  slug: string;
  /// The human label shown in the picker.
  title: string;
  /// One line describing the showcase, shown alongside the picker.
  description: string;
  /// The model source per surface (both compile against the preloaded music libs). A surface toggle re-seeds.
  source: Record<Surface, string>;
}

/// R1 — RATIONAL INTERVAL IDENTITY: intervals are exact fractions of an octave (a perfect fifth = 7/12 octave,
/// not a cents approximation), so a perfect fifth plus a perfect fourth is EXACTLY an octave. Returns a Bool
/// (the page renders it as a "true" verdict, not a MIDI table).
const RATIONAL_INTERVALS: ExampleModel = {
  slug: "rational-intervals",
  title: "Rational intervals (a fifth + a fourth = an octave)",
  description: "Intervals as exact octave-fractions, so a perfect fifth (7/12) plus a perfect fourth (5/12) is exactly one octave.",
  source: {
    ml: `def main() = r-eq(r-add(r-perfect-fifth, r-perfect-fourth), r-octave)`,
    sexpr: `(def (main) (r-eq (r-add r-perfect-fifth r-perfect-fourth) r-octave))`,
  },
};

/// R2 — CHORD → MIDI: build a C-major triad from a MIDI pitch and read out its note numbers. Returns a
/// List(Int64) ([60,64,67] = C-E-G); the page renders it as a value (no on/off timing — it's pitches, not events).
const CHORD_TO_MIDI: ExampleModel = {
  slug: "chord-to-midi",
  title: "Chord to MIDI (a C-major triad)",
  description: "Stack a major triad on middle C (MIDI 60) and read its note numbers, giving 60, 64, 67 (C, E, G).",
  source: {
    ml: `def main() = chord-notes(major-triad(pitch(60)))`,
    sexpr: `(def (main) (chord-notes (major-triad (pitch 60))))`,
  },
};

/// R3 — PIECE END-TO-END: lay out the I-V-vi-IV progression and SCHEDULE it into a stream of timed MIDI
/// events. Returns the List(MidiEvent) — the page renders it as the event-stream TABLE (tick | note | on/off)
/// + a balanced() badge (every note that switches on switches off again — no stuck keys).
const PIECE_TO_EVENTS: ExampleModel = {
  slug: "piece-to-events",
  title: "A piece as a MIDI event stream (no stuck keys)",
  description: "Schedule the I-V-vi-IV progression into timed MIDI events, where every note-on has a matching note-off (balanced).",
  source: {
    ml: `def main() = schedule(progression)`,
    sexpr: `(def (main) (schedule progression))`,
  },
};

/// R4 — LIVE-CODED PATTERN: write a rhythm the way a live-coder does, as a Euclidean pattern (3 onsets spread
/// evenly across 8 steps — a tresillo) over a cycle, then render it to timed MIDI events. Returns the
/// List(MidiEvent) — the page renders it in the same event-stream TABLE + balanced() badge as R3, so the reader
/// reuses the mental model. This is where "Music theory, live" means live-coding (the Strudel/Tidal idiom).
const EUCLIDEAN_PATTERN: ExampleModel = {
  slug: "euclidean-pattern",
  title: "A Euclidean rhythm, live-coded (a tresillo)",
  description: "Write a tresillo as a Euclidean pattern (3 onsets across 8 steps) and render it to timed MIDI events, the way a live-coder builds rhythm from combinators.",
  source: {
    ml: `def main() = schedule(render-pattern(euclid(3, 8, 60), 0, 100, 0, 3840))`,
    sexpr: `(def (main) (schedule (render-pattern (euclid 3 8 60) 0 100 0 3840)))`,
  },
};

/// The showcases the /music example-switcher offers, in display order. The switcher BUILDS to live-coding as
/// the climax — foundations first (exact rational intervals), then structure (a chord), then a fixed composed
/// piece, then improvising rhythm live: R1 → R2 → R3 → R4, a crescendo. The page still OPENS on `DEFAULT_EXAMPLE`
/// (the piece-to-events marquee — the strongest "no stuck keys" first impression), then the reader can climb.
export const EXAMPLES: ExampleModel[] = [RATIONAL_INTERVALS, CHORD_TO_MIDI, PIECE_TO_EVENTS, EUCLIDEAN_PATTERN];

/// The model the /music route opens with (the balanced-events marquee, R3 — proven strongest first impression).
export const DEFAULT_EXAMPLE = PIECE_TO_EVENTS;

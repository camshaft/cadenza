/// Unit tests for the /music MidiEvent parser (`midiEvents.ts`) — pure + wasm-free, pinning the parse of a
/// rendered `(: (tuple on chan note vel tick) MidiEvent)` list into table rows + the balanced() cross-check.
/// The end-to-end parse of a real `schedule(progression)` render is gated by `check-music-preload.mjs`
/// (needs the staged wasm); these pin the parsing logic on fixed fixtures.

import test from "node:test";
import assert from "node:assert/strict";
import { parseMidiEvents, isBalanced, type MidiEventRow } from "./midiEvents.ts";

// A minimal balanced 2-note fixture in the canonical render shape (on, chan, note, vel, tick order).
const BALANCED = `(: (list
  (: (tuple true 0 60 90 0) MidiEvent)
  (: (tuple false 0 60 0 960) MidiEvent)
  (: (tuple true 0 64 90 960) MidiEvent)
  (: (tuple false 0 64 0 1920) MidiEvent)
) (List MidiEvent))`;

test("parses each MidiEvent tuple into a row with the correct field mapping (on, chan, note, vel, tick)", () => {
  const p = parseMidiEvents(BALANCED);
  assert.ok(p.ok, "parses as a MidiEvent list");
  if (!p.ok) return;
  assert.equal(p.rows.length, 4, "four events");
  assert.deepEqual(p.rows[0], { on: true, chan: 0, note: 60, vel: 90, tick: 0 }, "first row = note-on middle C at tick 0");
  assert.deepEqual(p.rows[1], { on: false, chan: 0, note: 60, vel: 0, tick: 960 }, "second row = its note-off at tick 960");
});

test("returns rows tick-sorted (the table's play-order)", () => {
  // Deliberately out-of-tick-order input; parser must sort by tick.
  const unsorted = `(: (list
    (: (tuple false 0 60 0 960) MidiEvent)
    (: (tuple true 0 60 90 0) MidiEvent)
  ) (List MidiEvent))`;
  const p = parseMidiEvents(unsorted);
  assert.ok(p.ok);
  if (!p.ok) return;
  assert.deepEqual(p.rows.map((r) => r.tick), [0, 960], "sorted ascending by tick");
});

test("finds MidiEvent forms regardless of the outer type-wrapper nesting", () => {
  // A bare list (no outer `(: … (List MidiEvent))`) still yields the events.
  const bare = `((: (tuple true 1 67 90 0) MidiEvent) (: (tuple false 1 67 0 480) MidiEvent))`;
  const p = parseMidiEvents(bare);
  assert.ok(p.ok);
  if (!p.ok) return;
  assert.equal(p.rows.length, 2);
  assert.equal(p.rows[0].note, 67);
});

test("ok:false for a non-MidiEvent value (a Bool or Int list), so the page falls back to scalar render", () => {
  assert.equal(parseMidiEvents(`(: true Bool)`).ok, false, "a Bool (R1/R3-balanced result) is not a MIDI list");
  assert.equal(parseMidiEvents(`(: (list 60 64 67) (List Int64))`).ok, false, "an Int64 list (R2 chord-notes) is not a MIDI list");
});

test("ok:false (not a throw) on a paren-mismatched / garbage value", () => {
  assert.equal(parseMidiEvents(`(: (tuple true 0 60`).ok, false, "unbalanced parens → ok:false, no throw");
  assert.equal(parseMidiEvents(``).ok, false, "empty → ok:false");
});

// balanced() cross-check — the "no stuck keys" property the badge shows.
const rows = (spec: [boolean, number, number][]): MidiEventRow[] =>
  spec.map(([on, note, tick]) => ({ on, chan: 0, note, vel: on ? 90 : 0, tick }));

test("isBalanced: every on has a matching off → true", () => {
  assert.equal(isBalanced(rows([[true, 60, 0], [false, 60, 960], [true, 64, 960], [false, 64, 1920]])), true);
});
test("isBalanced: an on with no off (stuck key) → false", () => {
  assert.equal(isBalanced(rows([[true, 60, 0], [false, 60, 960], [true, 64, 960]])), false, "note 64 never turns off");
});
test("isBalanced: an off with no prior on → false", () => {
  assert.equal(isBalanced(rows([[false, 60, 0]])), false, "off before any on");
});
test("isBalanced: same note on different channels tracked independently", () => {
  // chan differs, so these are two distinct keys — both balanced.
  const two: MidiEventRow[] = [
    { on: true, chan: 0, note: 60, vel: 90, tick: 0 }, { on: true, chan: 1, note: 60, vel: 90, tick: 0 },
    { on: false, chan: 0, note: 60, vel: 0, tick: 960 }, { on: false, chan: 1, note: 60, vel: 0, tick: 960 },
  ];
  assert.equal(isBalanced(two), true);
});
test("isBalanced: an empty stream is vacuously balanced (no notes, no stuck keys)", () => {
  assert.equal(isBalanced([]), true);
});
test("isBalanced: a re-struck note (same chan+note) balances by COUNT, not a flag — on,on,off,off → true", () => {
  // A voice re-struck before release: two ons then two offs must net to zero. This pins the counter
  // semantics (a boolean-per-key refactor would wrongly report this balanced-or-not by last-write) — the
  // net-outstanding approach is what makes overlapping same-note events correct.
  assert.equal(isBalanced(rows([[true, 60, 0], [true, 60, 480], [false, 60, 960], [false, 60, 1440]])), true, "2 ons + 2 offs net to 0");
  assert.equal(isBalanced(rows([[true, 60, 0], [true, 60, 480], [false, 60, 960]])), false, "2 ons + 1 off leaves 1 outstanding (stuck)");
  assert.equal(isBalanced(rows([[true, 60, 0], [false, 60, 480], [false, 60, 960]])), false, "1 on + 2 offs goes negative (extra off)");
});

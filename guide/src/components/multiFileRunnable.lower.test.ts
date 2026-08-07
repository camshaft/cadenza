/// Pins that the multi-file <Runnable>'s lowering path (the platform-explorer seam it reuses) turns an
/// authored file SET into the exact compileWithPreloaded arguments — entry as `text`, the rest as the three
/// equal-length preload arrays in model order. This is the load-bearing logic behind MultiFileRunnable (a
/// React component, so not jsdom-tested here); the compile/run wiring is thin over these pure functions.
/// Uses v-guide-editor's VERIFIED agent-loop 2-file split (events records + reducer fold) as the fixture, so
/// the demo the operator asked for is pinned to lower correctly (a regression in the seam trips this).

import { test } from "node:test";
import assert from "node:assert/strict";
import { createWorkspace } from "../explorer/workspace.ts";
import { lowerWorkspace } from "../explorer/explorerView.ts";
import type { ExplorerFile } from "../explorer/fileModel.ts";

// The verified agent-loop split: events.cdz (preloaded, structural records) + reducer.cdz (entry, the fold).
// Structural records (not a cross-module sum type) because importing a custom sum type does NOT bring its
// constructors across the module boundary (CDZ0101) — a real language limit; structural data is the clean
// events-vs-reducer boundary regardless.
const EVENTS = `(do
  (def turn (list (record (kind #"task")  (val "count files"))
                  (record (kind #"model") (val "shell"))
                  (record (kind #"tool")  (val "3"))
                  (record (kind #"done")  (val "there are 3"))))
  (export turn))`;

const REDUCER = `(do
  (import "events" (turn))
  (def (step acc e)
    (let ((k (. e kind)) (v (. e val)))
      (if (= k #"task")  (String.concat acc "asked-model; ")
      (if (= k #"model") (String.concat acc (String.concat "run-tool:" (String.concat v "; ")))
      (if (= k #"tool")  (String.concat acc (String.concat "folded-result:" (String.concat v "; ")))
      (String.concat acc (String.concat "done:" v)))))))
  (def (run xs acc) (match xs ((list) acc) ((list e .. rest) (run rest (step acc e)))))
  (def (main) (run turn ""))
  (export main))`;

const files: ExplorerFile[] = [
  { name: "events", source: EVENTS, surface: "sexpr", entry: false },
  { name: "reducer", source: REDUCER, surface: "sexpr", entry: true },
];

test("the agent-loop 2-file split lowers to compileWithPreloaded args: reducer=entry text, events=preloaded", () => {
  const ws = createWorkspace(files);
  assert.ok(ws.ok, ws.ok ? "" : ws.reason);
  if (!ws.ok) return;
  const lowered = lowerWorkspace(ws.workspace);
  assert.ok(lowered.ok, lowered.ok ? "" : lowered.reason);
  if (!lowered.ok) return;

  // entry (reducer) is the compiled `text`; events is the single preloaded module.
  assert.match(lowered.lowered.text, /\(def \(main\) \(run turn ""\)\)/);
  assert.match(lowered.lowered.text, /\(import "events" \(turn\)\)/);
  assert.deepEqual(lowered.lowered.names, ["events"]);
  assert.equal(lowered.lowered.sources.length, 1);
  assert.match(lowered.lowered.sources[0], /\(def turn \(list/);
  assert.deepEqual(lowered.lowered.formats, ["sexpr"]);

  // the three preload arrays are equal length (compileWithPreloaded's preloadArity requirement).
  assert.equal(lowered.lowered.names.length, lowered.lowered.sources.length);
  assert.equal(lowered.lowered.names.length, lowered.lowered.formats.length);
});

test("swapping which file is the entry re-points what compiles as text (entry invariant)", () => {
  // If events were (wrongly) the entry, it would be the text and reducer the preload — the lowering follows
  // the entry flag, not file order. Pins that entry selection drives the compile, not position.
  const swapped: ExplorerFile[] = [
    { name: "events", source: EVENTS, surface: "sexpr", entry: true },
    { name: "reducer", source: REDUCER, surface: "sexpr", entry: false },
  ];
  const ws = createWorkspace(swapped);
  assert.ok(ws.ok);
  if (!ws.ok) return;
  const lowered = lowerWorkspace(ws.workspace);
  assert.ok(lowered.ok);
  if (!lowered.ok) return;
  assert.deepEqual(lowered.lowered.names, ["reducer"]);
  assert.match(lowered.lowered.text, /\(def turn \(list/); // events is now the entry text
});

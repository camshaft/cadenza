/// Pins the explorer view's pure READ-SIDE projections (E1-view core): the starter workspace boots clean and
/// lowers, the tree/tab rows carry model order + the entry/active flags the view highlights on, and the Run
/// seam (lowerWorkspace) reports a decline reason instead of throwing. A bug here would mis-render the file
/// tree or let Run fire on an un-lowerable set — both invisible without a DOM, so pinned as plain unit tests.

import { test } from "node:test";
import assert from "node:assert/strict";
import { starterWorkspace, treeItems, lowerWorkspace } from "./explorerView.ts";
import { addFile, setActive, deleteFile } from "./workspace.ts";

test("starterWorkspace boots a well-formed multi-file set (entry active, imports a preloaded module)", () => {
  const ws = starterWorkspace();
  assert.deepEqual(ws.files.map((f) => f.name), ["main", "greeting"]);
  assert.equal(ws.activeName, "main");
  assert.equal(ws.files.find((f) => f.name === "main")!.entry, true);
  assert.equal(ws.files.find((f) => f.name === "greeting")!.entry, false);
  // the genesis imports the preloaded module by its bare name — the multi-file link-merge the explorer shows.
  assert.match(ws.files[0].source, /\(import "greeting"/);
});

test("starterWorkspace lowers cleanly (a broken default would fail HERE, not as a reader-facing decline)", () => {
  const r = lowerWorkspace(starterWorkspace());
  assert.ok(r.ok, r.ok ? "" : r.reason);
  if (!r.ok) return;
  assert.match(r.lowered.text, /\(def \(main\)/);
  assert.deepEqual(r.lowered.names, ["greeting"]);
  assert.equal(r.lowered.sources.length, 1);
  assert.deepEqual(r.lowered.formats, ["sexpr"]);
});

test("treeItems projects rows in model order with exactly one entry and one active", () => {
  const items = treeItems(starterWorkspace());
  assert.deepEqual(items.map((i) => i.name), ["main", "greeting"]);
  assert.equal(items.filter((i) => i.isEntry).length, 1);
  assert.equal(items.filter((i) => i.isActive).length, 1);
  assert.equal(items.find((i) => i.name === "main")!.isEntry, true);
  assert.equal(items.find((i) => i.name === "main")!.isActive, true);
  assert.equal(items[0].surface, "sexpr");
});

test("treeItems tracks the active flag across a setActive", () => {
  const moved = setActive(starterWorkspace(), "greeting");
  assert.ok(moved.ok);
  if (!moved.ok) return;
  const items = treeItems(moved.workspace);
  assert.equal(items.find((i) => i.name === "greeting")!.isActive, true);
  assert.equal(items.find((i) => i.name === "main")!.isActive, false);
  assert.equal(items.filter((i) => i.isActive).length, 1);
});

test("treeItems reflects an added file (model order preserved, new file not entry)", () => {
  const added = addFile(starterWorkspace(), "notes", "ml");
  assert.ok(added.ok);
  if (!added.ok) return;
  const items = treeItems(added.workspace);
  assert.deepEqual(items.map((i) => i.name), ["main", "greeting", "notes"]);
  assert.equal(items.find((i) => i.name === "notes")!.isEntry, false);
  assert.equal(items.find((i) => i.name === "notes")!.surface, "ml");
});

test("lowerWorkspace surfaces the extra preloaded module after an addFile+edit", () => {
  const added = addFile(starterWorkspace(), "extra", "sexpr");
  assert.ok(added.ok);
  if (!added.ok) return;
  const r = lowerWorkspace(added.workspace);
  assert.ok(r.ok);
  if (!r.ok) return;
  // main is entry; greeting + extra are the two preloaded modules, in model order.
  assert.deepEqual(r.lowered.names, ["greeting", "extra"]);
  assert.equal(r.lowered.sources.length, 2);
  assert.equal(r.lowered.formats.length, 2);
});

test("deleteFile keeps the workspace lowerable (removing a preloaded module is fine)", () => {
  const r = deleteFile(starterWorkspace(), "greeting");
  assert.ok(r.ok);
  if (!r.ok) return;
  const lowered = lowerWorkspace(r.workspace);
  assert.ok(lowered.ok);
  if (!lowered.ok) return;
  assert.deepEqual(lowered.lowered.names, []); // only the entry remains
});

/// Pins the explorer's pure WORKSPACE reducer (E1) — the create/delete/rename/update/setEntry/setActive
/// lifecycle and its three invariants (unique+non-empty names, exactly one entry, `activeName` always
/// references a live file). These are the edge cases that scattered React setState gets wrong: deleting the
/// entry, deleting the active tab, renaming to a dup, renaming the active file. A bug here would corrupt the
/// file set the view renders and lowerToCompile compiles, so every transition's ok AND reject path is pinned.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  createWorkspace,
  addFile,
  deleteFile,
  renameFile,
  updateSource,
  setEntry,
  setActive,
  activeFile,
  type Workspace,
} from "./workspace.ts";
import type { ExplorerFile } from "./fileModel.ts";

const f = (name: string, source = "", entry = false, surface: "ml" | "sexpr" = "sexpr"): ExplorerFile => ({
  name,
  source,
  surface,
  entry,
});

// A well-formed seed: one entry + two preloaded modules, entry starts active.
function seed(): Workspace {
  const r = createWorkspace([f("main", "(main)", true), f("greeting", "(greet)"), f("helpers", "(aux)")]);
  assert.ok(r.ok);
  if (!r.ok) throw new Error(r.reason);
  return r.workspace;
}

test("createWorkspace focuses the entry file and preserves model order", () => {
  const ws = seed();
  assert.equal(ws.activeName, "main");
  assert.deepEqual(ws.files.map((x) => x.name), ["main", "greeting", "helpers"]);
});

test("createWorkspace rejects a malformed seed (no entry / multi entry / dup / empty set)", () => {
  assert.equal(createWorkspace([]).ok, false);
  assert.equal(createWorkspace([f("a"), f("b")]).ok, false); // no entry
  assert.equal(createWorkspace([f("a", "", true), f("b", "", true)]).ok, false); // two entries
  assert.equal(createWorkspace([f("a", "", true), f("a")]).ok, false); // dup name
  assert.equal(createWorkspace([f("", "", true)]).ok, false); // empty name
});

test("addFile appends a non-entry file and focuses it", () => {
  const r = addFile(seed(), "notes", "ml");
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.equal(r.workspace.activeName, "notes");
  assert.deepEqual(r.workspace.files.map((x) => x.name), ["main", "greeting", "helpers", "notes"]);
  assert.equal(r.workspace.files.at(-1)!.entry, false);
  assert.equal(r.workspace.files.at(-1)!.surface, "ml");
});

test("addFile rejects a duplicate or empty name", () => {
  assert.equal(addFile(seed(), "greeting", "sexpr").ok, false);
  assert.equal(addFile(seed(), "", "sexpr").ok, false);
});

test("deleteFile removes a preloaded module and keeps the active tab when it survives", () => {
  const r = deleteFile(seed(), "helpers");
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.deepEqual(r.workspace.files.map((x) => x.name), ["main", "greeting"]);
  assert.equal(r.workspace.activeName, "main"); // main was active and untouched
});

test("deleteFile re-points the active tab to the entry when the active file is deleted", () => {
  const active = setActive(seed(), "greeting");
  assert.ok(active.ok);
  if (!active.ok) return;
  const r = deleteFile(active.workspace, "greeting");
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.equal(r.workspace.activeName, "main"); // fell back to the entry, not a dead tab
});

test("deleteFile refuses to delete the entry file", () => {
  const r = deleteFile(seed(), "main");
  assert.equal(r.ok, false);
  if (r.ok) return;
  assert.match(r.reason, /entry file/);
});

test("deleteFile refuses to delete the last remaining file", () => {
  const one = createWorkspace([f("solo", "", true)]);
  assert.ok(one.ok);
  if (!one.ok) return;
  const r = deleteFile(one.workspace, "solo");
  assert.equal(r.ok, false);
});

test("renameFile renames and re-points the active tab when the active file is renamed", () => {
  const r = renameFile(seed(), "main", "genesis");
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.deepEqual(r.workspace.files.map((x) => x.name), ["genesis", "greeting", "helpers"]);
  assert.equal(r.workspace.activeName, "genesis"); // active followed the rename
  assert.equal(r.workspace.files[0].entry, true); // entry flag preserved across rename
});

test("renameFile rejects a collision or empty new name, and no-ops on same name", () => {
  assert.equal(renameFile(seed(), "main", "greeting").ok, false); // collision
  assert.equal(renameFile(seed(), "main", "").ok, false); // empty
  assert.equal(renameFile(seed(), "nope", "x").ok, false); // unknown source
  const same = renameFile(seed(), "main", "main");
  assert.ok(same.ok);
});

test("updateSource edits only the target buffer, leaving names/entry/active untouched", () => {
  const r = updateSource(seed(), "greeting", "(def (greet) 99)");
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.equal(r.workspace.files.find((x) => x.name === "greeting")!.source, "(def (greet) 99)");
  assert.equal(r.workspace.activeName, "main");
  assert.equal(updateSource(seed(), "ghost", "x").ok, false); // unknown file
});

test("setEntry moves the sole entry flag and clears it everywhere else", () => {
  const r = setEntry(seed(), "greeting");
  assert.ok(r.ok);
  if (!r.ok) return;
  const entries = r.workspace.files.filter((x) => x.entry);
  assert.equal(entries.length, 1);
  assert.equal(entries[0].name, "greeting");
  assert.equal(r.workspace.files.find((x) => x.name === "main")!.entry, false);
});

test("setActive focuses an existing file and rejects an unknown one", () => {
  const r = setActive(seed(), "helpers");
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.equal(r.workspace.activeName, "helpers");
  assert.equal(setActive(seed(), "ghost").ok, false);
});

test("activeFile returns the focused file after transitions", () => {
  const ws = seed();
  assert.equal(activeFile(ws)!.name, "main");
  const moved = setActive(ws, "helpers");
  assert.ok(moved.ok);
  if (!moved.ok) return;
  assert.equal(activeFile(moved.workspace)!.name, "helpers");
});

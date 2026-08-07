/// The pure WORKSPACE STATE MODEL behind the platform explorer's file-tree/tabs UI (E1) — the mutable-set
/// half of the explorer, sitting on top of E0's `ExplorerFile` model + `lowerToCompile` (fileModel.ts). NO
/// React, NO worker/compiler/wasm imports, so `node --test` covers the whole editing lifecycle directly
/// (same pure-core discipline as fileModel/pillarGroups): the React file-tree/tabs view (next increment) is
/// a thin projection over this state + these transitions, never re-deriving the invariants inline.
///
/// WHY a reducer, not ad-hoc setState in the view: the file set has real invariants — unique + non-empty
/// names (the import link targets, which `lowerToCompile` also guards), exactly ONE entry file, and an
/// `active` tab that must always reference a file that exists. Threading those through scattered React
/// handlers is exactly how a rename orphans the active tab or a delete drops the last entry. Concentrating
/// every mutation here — each returning a discriminated ok/reason result — keeps the view dumb and lets the
/// gate pin the edge cases (delete-the-entry, rename-to-a-dup, delete-the-active-tab) as plain unit tests.

import type { ExplorerFile } from "./fileModel.ts";
import type { Surface } from "../compiler/client.ts";

/// The full editable state of the explorer: the ordered file set (model order = tab/tree order, NOT sorted;
/// deep-links + tab strips rely on it, same as lowerToCompile's preload order) plus the name of the file
/// currently focused in the editor. `activeName` ALWAYS references a file in `files` — every transition
/// re-points it when the active file is renamed or deleted, so the view never dereferences a dead tab.
export interface Workspace {
  files: readonly ExplorerFile[];
  activeName: string;
}

/// A transition result: either the next workspace, or a human-readable reason the edit was refused (dup
/// name, empty name, deleting the sole entry, …). Discriminated rather than throwing so the view surfaces
/// the reason inline (a toast / inline decline) without a try/catch — mirrors fileModel's LowerResult.
export type WorkspaceResult =
  | { ok: true; workspace: Workspace }
  | { ok: false; reason: string };

/// Build the initial workspace from a starting file set. Validates the same invariants a transition would
/// preserve (non-empty, unique+non-empty names, exactly one entry) so a malformed seed fails LOUD here
/// rather than surfacing later as a lowerToCompile decline. The entry file starts active (it's the genesis
/// program the reader runs first); a caller may re-point with setActive.
export function createWorkspace(files: readonly ExplorerFile[]): WorkspaceResult {
  const invalid = validateSet(files);
  if (invalid) return { ok: false, reason: invalid };
  const entry = files.find((f) => f.entry)!; // validateSet guarantees exactly one
  return { ok: true, workspace: { files: files.slice(), activeName: entry.name } };
}

/// Add a new, empty file and focus it. Rejects a blank or duplicate name (the name is the import link
/// target + dedup key). Never an entry — a fresh file is always a preloaded module; promote it with setEntry.
export function addFile(ws: Workspace, name: string, surface: Surface): WorkspaceResult {
  const bad = validateName(name);
  if (bad) return { ok: false, reason: bad };
  if (ws.files.some((f) => f.name === name)) {
    return { ok: false, reason: `a file named "${name}" already exists — names are the import link targets and must be unique.` };
  }
  const file: ExplorerFile = { name, source: "", surface, entry: false };
  return { ok: true, workspace: { files: [...ws.files, file], activeName: name } };
}

/// Delete a file. Refuses to delete the entry file (that would leave the workspace with no genesis to run —
/// promote another file with setEntry first) and refuses to delete the last remaining file. When the active
/// tab is the one removed, focus falls back to the entry file so the editor never points at a dead tab.
export function deleteFile(ws: Workspace, name: string): WorkspaceResult {
  const target = ws.files.find((f) => f.name === name);
  if (!target) return { ok: false, reason: `no file named "${name}" to delete.` };
  if (target.entry) return { ok: false, reason: `"${name}" is the entry file — make another file the entry (setEntry) before deleting it.` };
  if (ws.files.length === 1) return { ok: false, reason: "can't delete the last file — a workspace needs at least the entry file." };
  const files = ws.files.filter((f) => f.name !== name);
  const entry = files.find((f) => f.entry)!; // target wasn't the entry, so it survives
  const activeName = ws.activeName === name ? entry.name : ws.activeName;
  return { ok: true, workspace: { files, activeName } };
}

/// Rename a file (its import link target). Rejects a blank name or a collision with another file. Re-points
/// the active tab when the renamed file was the active one so focus survives the rename. NOTE: this does NOT
/// rewrite `import … from "<old>"` references in other files' sources — that's a source-level rewrite the
/// caller can layer on; the model only guarantees name-set integrity.
export function renameFile(ws: Workspace, oldName: string, newName: string): WorkspaceResult {
  if (!ws.files.some((f) => f.name === oldName)) return { ok: false, reason: `no file named "${oldName}" to rename.` };
  if (newName === oldName) return { ok: true, workspace: ws };
  const bad = validateName(newName);
  if (bad) return { ok: false, reason: bad };
  if (ws.files.some((f) => f.name === newName)) {
    return { ok: false, reason: `a file named "${newName}" already exists — names must be unique.` };
  }
  const files = ws.files.map((f) => (f.name === oldName ? { ...f, name: newName } : f));
  const activeName = ws.activeName === oldName ? newName : ws.activeName;
  return { ok: true, workspace: { files, activeName } };
}

/// Update a file's source buffer (the editor's onChange). Pure, always succeeds for an existing file — the
/// hot path, so it never re-validates the whole set (names/entry are unchanged by an edit).
export function updateSource(ws: Workspace, name: string, source: string): WorkspaceResult {
  if (!ws.files.some((f) => f.name === name)) return { ok: false, reason: `no file named "${name}" to edit.` };
  const files = ws.files.map((f) => (f.name === name ? { ...f, source } : f));
  return { ok: true, workspace: { ...ws, files } };
}

/// Move the single `entry` flag onto `name` (the file compiled as the genesis `text`). Clears the flag on
/// every other file so the exactly-one-entry invariant is preserved by construction. No-op-safe if `name`
/// is already the entry.
export function setEntry(ws: Workspace, name: string): WorkspaceResult {
  if (!ws.files.some((f) => f.name === name)) return { ok: false, reason: `no file named "${name}" to make the entry.` };
  const files = ws.files.map((f) => ({ ...f, entry: f.name === name }));
  return { ok: true, workspace: { ...ws, files } };
}

/// Focus a tab. The only transition that touches `activeName` without touching `files`; rejects an unknown
/// name so the invariant (active always references an existing file) holds.
export function setActive(ws: Workspace, name: string): WorkspaceResult {
  if (!ws.files.some((f) => f.name === name)) return { ok: false, reason: `no file named "${name}" to focus.` };
  return { ok: true, workspace: { ...ws, activeName: name } };
}

/// Convenience projection for the view: the currently-focused file (always defined given the active-name
/// invariant, but typed optional so a caller that hand-builds a Workspace can't crash on a bad activeName).
export function activeFile(ws: Workspace): ExplorerFile | undefined {
  return ws.files.find((f) => f.name === ws.activeName);
}

/// A blank/duplicate/entry-count check on a whole set — the seed-time guard for createWorkspace. Returns a
/// reason string on the first violation, or null when the set is well-formed.
function validateSet(files: readonly ExplorerFile[]): string | null {
  if (files.length === 0) return "empty file set — a workspace needs at least an entry file.";
  const seen = new Set<string>();
  for (const f of files) {
    const bad = validateName(f.name);
    if (bad) return bad;
    if (seen.has(f.name)) return `duplicate file name "${f.name}" — names must be unique.`;
    seen.add(f.name);
  }
  const entries = files.filter((f) => f.entry).length;
  if (entries !== 1) return `a workspace needs exactly one entry file (found ${entries}).`;
  return null;
}

/// A single name is non-empty (the import link target can't be blank). Returns a reason or null.
function validateName(name: string): string | null {
  if (typeof name !== "string" || name.length === 0) return "a file needs a non-empty name (the import link target).";
  return null;
}

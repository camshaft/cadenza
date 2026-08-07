/// The pure, dep-free MULTI-FILE MODEL behind the in-browser platform explorer — the foundation increment
/// (E0) of the operator-prioritized explorer (a multi-file IDE that compiles+runs a genesis program +
/// reducer(s) + content-addressed deps in the browser). NO React, NO worker/compiler/wasm imports, so
/// `node --test` covers it directly (the same pure-core discipline as scalarFormat/classify/pillarGroups).
///
/// WHY a model + a lowering function: the compiler already link-merges N named modules via
/// `compileWithPreloaded(text, from, names[], sources[], formats[])` (compiler/client.ts) — that's the
/// multi-file primitive. /music and /cad drive it with a FIXED preload set (see musicPreload.ts). The
/// explorer generalizes that fixed set to a USER-EDITABLE file set: the reader's files ARE the preloaded
/// modules, and one designated ENTRY file is compiled as the `text`. This module owns the file set + the
/// pure transform that lowers it to the (entryText, from, names, sources, formats) call shape — so the
/// UI/worker layers (E1/E2) build on a tested seam rather than re-deriving the arrays ad hoc.

import type { Surface } from "../compiler/client.ts";

/// One file in the explorer: its import NAME (the `import from "<name>"` link target — a bare module name,
/// NOT a path; matches how compileWithPreloaded's `names` entries are bare like "chord"/"pitch"), its
/// SOURCE text, and the SURFACE it's authored in (ml | sexpr). `entry` marks the single file compiled as
/// the top-level program (the genesis); every other file is a preloaded/link-merged module.
export interface ExplorerFile {
  /// Bare module name — the link target other files import via `import … from "<name>"`. Unique per model.
  name: string;
  source: string;
  surface: Surface;
  /// Exactly one file in a model is the entry (the genesis program compiled as `text`); the rest preload.
  entry?: boolean;
}

/// The arguments a lowered model feeds to `compileWithPreloaded(text, from, names, sources, formats)`:
/// the entry file's text+surface, and the three EQUAL-LENGTH parallel arrays for the preloaded (non-entry)
/// modules. Shaped to drop straight into the existing worker call (and to satisfy preloadArityError).
export interface LoweredCompile {
  text: string;
  from: Surface;
  names: string[];
  sources: string[];
  formats: string[];
}

/// The result of validating + lowering a file set: either the ready-to-compile args, or a human-readable
/// reason it can't be lowered (no entry / duplicate names / empty set). A discriminated result rather than
/// a throw so the UI can surface the reason inline (like a decline diagnostic) without a try/catch.
export type LowerResult =
  | { ok: true; lowered: LoweredCompile }
  | { ok: false; reason: string };

/// Lower a file set to the compileWithPreloaded call shape. Rules:
///   - exactly ONE file must be `entry: true` (the genesis compiled as `text`); 0 or >1 is an error.
///   - file `name`s must be unique + non-empty (they're the import link targets AND the dedup key; a dup
///     silently shadows a module, exactly the class the playground id-guard pins for examples).
///   - the preloaded (non-entry) files become the names/sources/formats arrays, in stable model order, all
///     equal length by construction (so preloadArityError never fires on a lowered model).
export function lowerToCompile(files: readonly ExplorerFile[]): LowerResult {
  if (files.length === 0) return { ok: false, reason: "empty file set — add at least an entry file." };

  const empties = files.filter((f) => typeof f.name !== "string" || f.name.length === 0);
  if (empties.length) return { ok: false, reason: "every file needs a non-empty `name` (the import link target)." };

  const counts = new Map<string, number>();
  for (const f of files) counts.set(f.name, (counts.get(f.name) ?? 0) + 1);
  const dups = [...counts].filter(([, n]) => n > 1).map(([name]) => name);
  if (dups.length) return { ok: false, reason: `duplicate file name(s): ${dups.join(", ")} — each file needs a unique name (imports resolve by name).` };

  const entries = files.filter((f) => f.entry);
  if (entries.length === 0) return { ok: false, reason: "no entry file — mark exactly one file `entry: true` (the genesis program)." };
  if (entries.length > 1) return { ok: false, reason: `multiple entry files (${entries.map((f) => f.name).join(", ")}) — exactly one file may be the entry.` };

  const entry = entries[0];
  const preloaded = files.filter((f) => f !== entry);
  return {
    ok: true,
    lowered: {
      text: entry.source,
      from: entry.surface,
      names: preloaded.map((f) => f.name),
      sources: preloaded.map((f) => f.source),
      formats: preloaded.map((f) => f.surface),
    },
  };
}

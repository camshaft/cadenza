/// The pure READ-SIDE PROJECTIONS the platform-explorer view renders (E1-view core) — sitting on top of the
/// workspace reducer (workspace.ts) + file model (fileModel.ts). NO React: the file-tree/tab-strip/Run-button
/// React panel (next increment) is a thin projection over these three functions, so `node --test` pins the
/// display + run-gate logic without a DOM. Same pure-core-first split as E0→E1: the state machine and its
/// read model land + get pinned before any component consumes them.
///
///   - starterWorkspace() — the default multi-file set the /explorer route boots with (a genesis that imports
///     a preloaded module), guaranteed to round-trip through createWorkspace + lower cleanly.
///   - treeItems() — the file-tree / tab-strip rows in stable model order, each flagged entry/active.
///   - lowerWorkspace() — the seam the Run button uses: lower the current file set to the compileWithPreloaded
///     args, or report WHY it can't (surfaced inline like a decline, so Run disables with a reason).

import { lowerToCompile, type ExplorerFile, type LowerResult } from "./fileModel.ts";
import { createWorkspace, type Workspace } from "./workspace.ts";

/// One row in the file tree / tab strip: the file's name (the label + the setActive key), the surface badge,
/// and the two flags the view highlights on — `isEntry` (the genesis marker) and `isActive` (the focused
/// tab). Derived purely from the workspace; the view never re-reads `workspace.files`/`activeName` directly.
export interface TreeItem {
  name: string;
  surface: ExplorerFile["surface"];
  isEntry: boolean;
  isActive: boolean;
}

/// Project the workspace to its file-tree / tab-strip rows, in stable MODEL order (never sorted — tabs +
/// deep-links rely on it, matching lowerToCompile's preload order). Exactly one row is `isEntry` and at most
/// one is `isActive` (both invariants held by the reducer).
export function treeItems(ws: Workspace): TreeItem[] {
  return ws.files.map((f) => ({
    name: f.name,
    surface: f.surface,
    isEntry: f.entry === true,
    isActive: f.name === ws.activeName,
  }));
}

/// Lower the current workspace to the compileWithPreloaded call shape (the Run seam), or a decline reason.
/// A thin, NAMED pass-through to lowerToCompile so the view has one call site to gate the Run button on (and
/// surface `reason` inline) rather than reaching into fileModel from a component.
export function lowerWorkspace(ws: Workspace): LowerResult {
  return lowerToCompile(ws.files);
}

/// The default file set the /explorer route boots with: a genesis `main` that imports a preloaded `greeting`
/// module and calls it, demonstrating the multi-file link-merge (the whole point of the explorer) out of the
/// box. Returns a ready Workspace — it round-trips through createWorkspace, so a malformed default would fail
/// the unit test here, not surface as a runtime decline for the reader.
export function starterWorkspace(): Workspace {
  const files: ExplorerFile[] = [
    {
      name: "main",
      surface: "sexpr",
      entry: true,
      source: STARTER_MAIN,
    },
    {
      name: "greeting",
      surface: "sexpr",
      entry: false,
      source: STARTER_GREETING,
    },
  ];
  const r = createWorkspace(files);
  // createWorkspace only fails on a malformed set; this literal is well-formed, so `ok` is guaranteed. We
  // throw rather than return a result because a broken STARTER is a build-time bug (pinned by the unit test),
  // not a user-facing decline the view should handle.
  if (!r.ok) throw new Error(`starterWorkspace is malformed: ${r.reason}`);
  return r.workspace;
}

const STARTER_GREETING = `(do
  (def (greet name)
    (string-append "hello, " name))
  (export greet))`;

const STARTER_MAIN = `(do
  (import "greeting" (greet))
  (def (main) (greet "explorer"))
  (export main))`;

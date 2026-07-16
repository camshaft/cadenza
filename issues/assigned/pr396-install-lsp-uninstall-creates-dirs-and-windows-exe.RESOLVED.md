# PR review comments — mirrored from GitHub PR #396 (Copilot inline)

- **PR:** #396 "fleet: twenty-second batch (Perceus nested-proj fix, open-sums OS1, LSP goto-def, guide chapters + infra)" (MERGED)
- **File:** `xtask/src/install_lsp.rs` (uninstall @39, build_cdz @92)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3590624254, 3590624280
- **Links:** https://github.com/camshaft/cadenza/pull/396#discussion_r3590624254 , #discussion_r3590624280

## Comments (verbatim)
> `--uninstall` currently calls `editor_extension_dirs()` before branching, and that helper may create missing `extensions/` directories (e.g. `~/.vscode-server/extensions`). Uninstall should be non-destructive; it shouldn't create new directories just to remove a symlink.
>
> `build_cdz` hard-codes the expected output path as `target/release/cdz`. On Windows, Cargo produces `cdz.exe`, so this check will fail and `install-lsp` will error even though the build succeeded.

## Liaison triage — CONFIRMED against trunk
Both confirmed in xtask/src/install_lsp.rs:
- `editor_extension_dirs()` is called BEFORE the `if uninstall` branch, so if that helper mkdir's the
  editor `extensions/` dirs, `--uninstall` creates directories it's only meant to clean out of — a
  non-destructive-uninstall violation.
- `build_cdz` returns `paths.repo.join("target/release/cdz")` with no `.exe` suffix; on Windows Cargo
  emits `cdz.exe`, so the post-build existence check fails and `install-lsp` errors despite a successful
  build. (The LSP installer targets user editors, which can be on Windows, so this isn't purely
  hypothetical even though the fleet itself runs Linux.)
Fleet-tooling territory (`v-fleet-tooling` owns xtask). Fixes: move `editor_extension_dirs()` after the
uninstall branch (or make it non-creating), and pick `cdz.exe` on Windows via `cfg!(windows)` /
`std::env::consts::EXE_SUFFIX`. Fix on `trunk`. Quotes + links in queue file.

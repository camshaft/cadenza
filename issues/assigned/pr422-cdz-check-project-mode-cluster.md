# PR review comments — mirrored from GitHub PR #422 (Copilot inline) — cdz check project-mode cluster

- **PR:** #422 "fleet: batch 47+48 (…, cdz check-project)" (MERGED)
- **File:** `implementation/seed/crates/cdz/src/main.rs` (resolve_check_targets @2208, check_one loop @2172)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591975125, 3591975140
- **Links:** https://github.com/camshaft/cadenza/pull/422#discussion_r3591975125 , #discussion_r3591975140

## Comments (verbatim)
> `resolve_check_targets` treats any argument named `Project.cdz` as a manifest target even if that file doesn't exist. In that case `load_manifest(&dir)` returns `Ok(None)` and the code silently falls back to walking the directory, which is surprising for `cdz check Project.cdz` (it should be a clear "no such file").
>
> `cdz check` in project mode resolves a list of project files, then calls `check_one` for each. Since `check_one` follows the import closure (and may run package-wide diagnostics when the entry imports), this can redundantly re-check the same files many times and can duplicate diagnostics output … a noticeable slowdown for larger projects.

## Liaison triage — CONFIRMED against trunk
Both confirmed in cdz/src/main.rs:
1. `is_manifest_arg = path.file_name() == Some(MANIFEST_NAME)` matches by NAME only, and the manifest
   branch calls `load_manifest(&dir)` which returns `Ok(None)` when the file is absent → silent dir-walk
   fallback. `cdz check Project.cdz` on a missing file should error "no such file", not walk the dir.
2. Project-mode `cdz check` calls `check_one` per resolved file, and `check_one` follows the import
   closure — so shared modules get re-checked many times + duplicate diagnostics (module errors once via
   the entry's closure, again when checked directly). Scales badly on larger projects.
Both cdz-CLI (v-cdz-tooling). FIX: (1) reject a named-but-missing manifest arg explicitly; (2) dedup the
check set / check the import closure once (or suppress duplicate diagnostics across the per-file runs).
Fixes on `trunk`. Quotes + links in queue file.

## Note
amazon-q's #422 review SUMMARY claimed "6 crash risks (unreachable!/unchecked array access)" on a wasm
vselect SIMD change but posted ZERO inline anchors — not actionable, not filed (consistent with its
recent hard-flag unreliability).

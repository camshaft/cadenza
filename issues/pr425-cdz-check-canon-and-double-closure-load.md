# PR review comments — mirrored from GitHub PR #425 (Copilot inline) — cdz check perf (follow-on to pr422)

- **PR:** #425 (MERGED)
- **File:** `implementation/seed/crates/cdz/src/main.rs` (canon @2218, run_check @2216)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592073466, 3592073489
- **Links:** https://github.com/camshaft/cadenza/pull/425#discussion_r3592073466 , #discussion_r3592073489

## Comments (verbatim)
> `canon(f)` performs filesystem canonicalization + allocation and is currently recomputed multiple times per target (in `covered.contains(...)` and again when inserting into `covered`). Compute the canonical form once per `f` and reuse it.
>
> `run_check` reloads the import closure via `load_import_closure_with(f, ...)` immediately after `check_one(f, ...)`, but `check_one` already loads (and parses) the same closure. This makes project checks do redundant disk reads/parses per target … Consider restructuring so `check_one` returns the loaded closure paths (or accepts a precomputed closure) so the closure is loaded once per target.

## Liaison triage
Two efficiency follow-ons in the same `cdz check` project-mode path as my pr422 cluster (which flagged
per-file check_one re-following the import closure). Here: (1) `canon(f)` (fs canonicalize + alloc) is
recomputed multiple times per target — hoist it once per `f`; (2) `run_check` reloads the import closure
right after `check_one` already loaded+parsed it — restructure so the closure is loaded once (check_one
returns/accepts it). Both v-cdz-tooling; pair with the pr422 dedup work. Fixes on `trunk`. Quotes +
links in queue file.

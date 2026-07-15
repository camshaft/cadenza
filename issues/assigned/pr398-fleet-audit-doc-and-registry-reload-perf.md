# PR review comments — mirrored from GitHub PR #398 (Copilot inline)

- **PR:** #398 (MERGED)
- **File:** `xtask/src/fleet.rs` (Audit doc @384, audit loop @1010)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3590785491, 3590785512
- **Links:** https://github.com/camshaft/cadenza/pull/398#discussion_r3590785491 , #discussion_r3590785512

## Comments (verbatim)
> The `Audit` command doc comment says it "Exits non-zero if any orphan is found", but the implementation only exits non-zero when `--strict` is set. Please align the docs with the actual behavior (or change the behavior to match the docs).
>
> `audit` calls `fleet.load()` for every processed merge-request to determine whether the sender is active. For a large `processed/` history this repeatedly re-reads/parses the registry and can dominate runtime. Consider loading the registry once (or precomputing a `HashSet` of active agent names) outside the loop and reusing it for all entries.

## Liaison triage — CONFIRMED against trunk
Both confirmed in xtask/src/fleet.rs:
- DOC MISMATCH: the `Audit` command doc says "Exits non-zero if any orphan is found", but the `--strict`
  arg's own doc says exit-nonzero is "OFF by default" (only under `--strict`). The command doc contradicts
  the actual behavior — reword it to say orphans are reported, and non-zero exit is `--strict`-only.
- PERF: the orphan-classification loop calls `fleet.load()` (reads+parses registry.json) INSIDE the
  per-processed-request loop — one full registry parse per orphan-candidate → O(N) reloads on a large
  `processed/` history. Hoist a single `fleet.load()` (or a precomputed HashSet of active agent names)
  out of the loop and reuse it.
Fleet-tooling territory (v-fleet-tooling owns xtask fleet). Fixes on `trunk`. Quotes + links in queue file.

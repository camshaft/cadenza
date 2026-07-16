# PR review comment — mirrored from GitHub PR #415 (Copilot inline)

- **PR:** #415 (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/lower.rs:17192` (`lower_set_to_list`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591538888
- **Link:** https://github.com/camshaft/cadenza/pull/415#discussion_r3591538888

## Comment (verbatim)
> `lower_set_to_list` calls `core_of(db, set)` twice (once for the Poison check, and again to check for an empty `SetOf`). `core_of` is a non-trivial lowering pass; calling it twice is unnecessary work and can duplicate traversal.

## Liaison triage
`lower_set_to_list` invokes `core_of(db, set)` twice — once for the Poison check, once for the empty-
SetOf check. `core_of` is a non-trivial lowering pass, so the double call is redundant work (and could
duplicate any traversal side-effects). Low-severity efficiency/cleanliness (Core IR lowering). FIX:
bind `core_of(db, set)` once and reuse. Core-IR lowering territory (v-core-opt owns core.rs/ANF
lowering; else corpus-bugfix). Fix on `trunk`. Quote + link in queue file.

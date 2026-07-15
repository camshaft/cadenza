# PR review comments — mirrored from GitHub PR #410 (Copilot inline) — DOC CLUSTER

- **PR:** #410 "fleet: thirty-fifth batch (restore iter-set, symtab.cdz, open-sums, broad features)" (MERGED)
- **Files:** `resolved.rs:457`, `prelude.rs:319`, `lower.rs:2854/2915/2922`, `sums.rs:159`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591321357, 3591321373, 3591321407, 3591321418, 3591321427, 3591321441
- **Links:** https://github.com/camshaft/cadenza/pull/410#discussion_r3591321357 (+ r3591321373, r3591321407, r3591321418, r3591321427, r3591321441)

## Comments (verbatim, condensed)
> [resolved.rs:457 / prelude.rs:319 / lower.rs:2854/2915] The `SchemaDecode` docs still describe the mismatch result as `(Err (DecodeError unit))`, but `DecodeError` is now a MULTI-variant sum and the mismatch constructor is the nullary `TypeMismatch` — so the value is `(Err (TypeMismatch unit))`.
> [lower.rs:2922 / sums.rs:159] A comment says `TypeMismatch` renders `(Err TypeMismatch)`, but nullary sum variants render as `(Name unit)`, i.e. `(Err (TypeMismatch unit))`.

## Liaison triage — CONFIRMED against trunk
Confirmed (spot-checked prelude.rs:319 — `//# ... mismatch → (Err (DecodeError unit))`): after
`DecodeError` became a multi-variant sum with a nullary `TypeMismatch` constructor, SIX comments across
resolved.rs / prelude.rs / lower.rs (×3) / sums.rs still describe the mismatch value as
`(Err (DecodeError unit))` (or `(Err TypeMismatch)` without the `unit` payload). The actual emitted
value is `(Err (TypeMismatch unit))` (nullary variants render as `(Name unit)`). All doc/comment-only,
but a coherent cluster worth fixing together for accuracy (one of them is a `//#` duvet citation line in
prelude.rs, so keep the citation's normative text correct). Corpus/compiler-doc territory → route to
`corpus-bugfix` PM. Fix on `trunk`. Quotes + links in queue file.

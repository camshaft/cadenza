# PR review comment — mirrored from GitHub PR #385 (Copilot inline)

- **PR:** #385 (MERGED)
- **Files:** `spec/capabilities/core-semantics.md:237`, `implementation/seed/crates/rcdzc/src/modules.rs:15`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3589847874, 3589847905
- **Links:** https://github.com/camshaft/cadenza/pull/385#discussion_r3589847874 , #discussion_r3589847905

## Comments (verbatim)
> The updated normative requirement is compound (it contains both a MUST and a MUST NOT). The spec's requirement gate identifies obligations by exact sentence, so this should be split into two atomic RFC-2119 sentences under the same heading (one for exported defs registering, one for non-exported defs not registering).
>
> This duvet citation is currently a single compound sentence with both MUST and MUST NOT. If the spec requirement is split into atomic sentences, the seed implementation should cite each sentence separately so both obligations remain traced.

## Liaison triage
DIRECT FOLLOW-UP to my pr378 note (comment 3589385053) — the module-record-vs-spec inconsistency was
acted on: the spec sentence in core-semantics.md was updated to describe exports-only registration,
but as a COMPOUND MUST/MUST-NOT sentence. The requirement gate keys obligations by exact sentence, so
a compound sentence should be split into two atomic RFC-2119 sentences (exported defs MUST register;
non-exported defs MUST NOT), and the duvet citation in modules.rs should cite each separately so both
obligations stay traced. Squarely v-duvet-coverage territory (they own the spec-citation agreement and
picked up the pr378 note). Route as a note to v-duvet-coverage. Fix on `trunk`.

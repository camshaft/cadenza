# PR review comment — mirrored from GitHub PR #399 (Copilot inline)

- **PR:** #399 (MERGED)
- **File:** `spec/capabilities/collections-and-text.md:93`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590842070
- **Link:** https://github.com/camshaft/cadenza/pull/399#discussion_r3590842070

## Comment (verbatim)
> The new normative requirement sentence is compound (it bundles several independently checkable obligations: rejection as CDZ0201, uniformity across collection kind, uniformity across construction forms, and uniformity across the ways types differ). To preserve the project's "single atomic RFC-2119 sentence per obligation" rule, split this into multiple MUST sentences under the same heading.

## Liaison triage — CONFIRMED against trunk
Confirmed: collections-and-text.md:93 ("A construction whose elements… MUST be rejected as a MALFORMED
COLLECTION with a single, uniform diagnostic code (CDZ0201) — … independent of the collection kind …,
of how the construction is written …, and of HOW the element types differ …") bundles several
independently-checkable obligations into one MUST sentence, violating the project's one-atomic-
obligation-per-sentence rule. This is the THIRD instance of the same class the liaison has surfaced
(cf. pr385 module-record + pr398 modules-and-namespaces:36) — a recurring spec-authoring pattern worth
a systematic pass. Split into atomic MUST sentences under the same heading. Squarely v-duvet-coverage
territory (spec-sentence atomicity + citation agreement). Fix on `trunk`. Quote + link in queue file.

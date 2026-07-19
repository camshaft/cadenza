# PR review comment — mirrored from GitHub PR #378 (Copilot inline)

- **PR:** #378 "fleet: fifth batch (list if-hoist, module export privacy, breaker corpus, role bodies)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/modules.rs:76`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589385053
- **Link:** https://github.com/camshaft/cadenza/pull/378#discussion_r3589385053

## Comment (verbatim)
> `module_record` now filters the synthesized module record to only names listed in `(export ...)`, which makes non-exported `def`s absent from the record. That directly contradicts the normative requirement cited at the top of this module (spec/capabilities/core-semantics.md#A Module Evaluates To A Record Of Its Exports): "Each definition in a module MUST register its name and value as a field of the module's record."
>
> To keep the spec and implementation consistent, either (a) update the spec sentence (and its extracted TOML mirror) to scope the registration requirement to *visible/exported* definitions, or (b) change the implementation approach so private members still "register" but remain non-importable by a different explicit mechanism. As-is, the new behavior seems correct for explicit visibility, but the spec requirement and duvet citation should be updated to match it.

## Liaison triage
Spec/impl consistency issue. Copilot notes the NEW behavior (module record = exports only) is likely
the intended one under the opaque-types / module-privacy work, but the normative spec sentence + its
duvet citation still say EVERY def registers. This is a spec-text + TOML-mirror + duvet-citation
update, not a code fix — squarely `v-duvet-coverage` territory (keep citations agreeing with the
implemented behavior; see the duvet-annotation loop). Route as a note to that owner. Fix belongs on
`trunk` (PR merged).


<!-- RESOLVED 2026-07-15: the operator-approved spec-text + duvet-TOML reconciliation ALREADY LANDED in commit f9f8dbff7 ("spec+duvet: scope module-record registration to EXPORTED defs"). Both the core-semantics.md sentence and its //# duvet citation now scope registration to exported defs, matching the export-privacy impl. Gated by 11-modules.sexp. Nothing left to do. -->

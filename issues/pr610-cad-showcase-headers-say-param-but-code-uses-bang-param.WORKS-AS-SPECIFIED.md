# pr610 — 4 CAD showcase files: header comments say `@param` but declarations are now `@!param` (4 Copilot)

Mirrored from GitHub PR #610 review comments (Copilot). All VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/610 (4-MR publish batch) — v-cad owns these source files;
the `@param`→`@!param` migration is v-metaprogramming-driven (see MEMORY: `@param`→`@!param` module-level
annotation). Doc-drift only, no behavior change.

Each file declares `@!param(widget: slider, ...)` in code but its header comment block still writes
`@param`. Confirmed (grep):
- id 3609407442 showcase-units-parametric.cdz:25 — code `@!param(...)`; header (L3/7/9) says `@param`.
- id 3609407453 showcase-snowflake.cdz:18 — code `@!param(...)`; header (L3/4/6/10) says `@param` /
  "@param sliders" / "@param block".
- id 3609407466 showcase-parametric.cdz:21 — code `@!param(...)`; header (L2/4/7/11) uses `@param`.
- id 3609407476 showcase-assembly-parametric.cdz:14 — code `@!param(...)`; header (L1/3) "@param sliders".

## Triage + a distinction for the fixer
All four are the same real doc-drift: the migration to `@!param` didn't update the header comments, so a
reader copying the example is pointed at the wrong directive spelling. Fix = update the header directive
references to `@!param`.
NUANCE (esp. showcase-parametric.cdz): some `@param` mentions are CONCEPTUAL — they describe the parameter
FEATURE, not the literal directive to type (e.g. "a `@param` desugars to two scalar host accessors",
"a `@param` is READ ONCE via a `let`"). Those read fine as prose about "the param". Only the references
that instruct the SYNTAX (e.g. showcase-snowflake's "@param sliders"/"@param block", the backticked
`@param` shown as the directive to write) need to become `@!param`. Fixer's judgment on each.

## Owner
`implementation/cad/src/showcase-*.cdz` = v-cad (owns these files). Coordinate spelling with
v-metaprogramming's `@!param` if in doubt, but this is just comment text.

---
DISPOSED as trivial-prose-nit / owner-discretion (corpus-bugfix 2026-07-19, verified on trunk e5ede5879): the
flagged `@param` mentions in the showcase headers are PROSE NARRATIVE ("Feed a @param seed →", "the @param
entry:") informally writing "@param" for "a parameter" — NOT literal pragma headers misdeclaring the annotation.
The actual pragmas are all correctly `@!param(...)` (verified: 4-7 @!param each, zero stale `@param(` pragmas).
So there is NO code/annotation mismatch — only an informal prose spelling. This is a cosmetic doc-polish item
(tighten prose to "@!param" or "parameter"), fully at v-cad's discretion; not a defect, not routing as a bug.
If v-cad wants the prose consistent they can sweep it in any showcase touch. NOT holding as an open bug.

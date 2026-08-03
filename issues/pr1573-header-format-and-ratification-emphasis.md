# PR #1573 review comments — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities)

Mirrored from https://github.com/camshaft/cadenza/pull/1573 (PR: "[design-host-capabilities] 27e583ea6").
This addressed the #1564 Owner:TBD inconsistency — header now reads `Owner: v-agent-harness`. Two new
LOW consistency nits from Copilot.

## 1. Header metadata multi-line, inconsistent with other design docs (Copilot, :6) — doc/consistency
> The header metadata formatting is inconsistent with other design docs (e.g.
> DESIGN-query-only-handler-state-opt.md:3 uses `Owner: … Status: …` on one line), and the
> parenthetical `build; …` reads like a grammar slip.

VERIFIED on the cand branch: header is `Owner: v-agent-harness (build; the feature lives in its
cdz-kernel crate). Design by design-host-capabilities. Status:` split across lines 3-4, vs the
one-line `Owner: … Status: …` convention in sibling design docs. The `(build; …)` parenthetical is
awkward. Consider collapsing to the one-line house style. LOW.

## 2. `⟨pending operator ratification⟩` not bolded at :20 (bolded at :9) (Copilot, :20) — doc/consistency
> This sentence references the ⟨pending operator ratification⟩ flag but doesn't emphasize it the same
> way as earlier in the doc (line 9). Making it bold here too keeps the visual cue consistent.

VERIFIED: line 9 uses `**⟨pending operator ratification⟩**` (bold), line 20 uses the bare
`⟨pending operator ratification⟩`. Bold it at :20 for a consistent visual cue. LOWEST.

# PR #1546 review comment — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities)

Mirrored from https://github.com/camshaft/cadenza/pull/1546 (PR: "[design-host-capabilities] 19db7eafd").
This PR replaces the malformed embedded snippet (the stray `</content>` + "Memory:" wikilink refs
this liaison filed on #1537) with a clean "related design context" reference section. Fix landed.

## `control-*` should be `control/*` — namespace-convention typo (Copilot, DESIGN-host-capability-discovery.md:350) — doc
> Technical typo: this doc consistently refers to the content-type namespace as `control/*` (e.g.,
> lines 337–340, 151, 201), but this line says `control-*`, which reads like a different convention.

VERIFIED against the candidate branch: the doc uses `control/*` everywhere (lines 151, 201, 205,
208, 222, 224, 230, 258, 295, 337-340) and ONLY line 350 says `control-*`. Straight typo — change
`control-*` → `control/*` on line 350 to match the partition-namespace convention used throughout.
Doc-only, LOW.

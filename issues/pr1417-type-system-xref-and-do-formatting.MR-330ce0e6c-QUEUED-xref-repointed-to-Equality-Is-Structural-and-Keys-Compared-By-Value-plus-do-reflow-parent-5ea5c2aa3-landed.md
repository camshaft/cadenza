# PR #1417 review — 07-type-system.sexp fn-Set-element pin (my adv-50-residual pin)

github-liaison note 21725: Copilot flagged 2 nits on my PR #1417 (MR 5ea5c2aa3, fn-as-Set-element CDZ0216 case):
1. :1686 membership cross-ref cited "§A Set Is A Collection Of Unique Elements" (uniqueness, NOT membership)
   — repointed to core-semantics.md §Equality Is Structural + collections-and-text §Keys Are Compared By
   Value, Not Representation (maps) / §Set Membership Is Total (sets).
2. :1692 single-line (do …) → reflowed to the multi-line (do block form matching sibling cases.

## Done
- Fixed both; committed 1ec5e078c (fix-forward on 5ea5c2aa3). Docstring + input-reflow only; verdict
  unchanged (CDZ0216 PASS ×3, no baseline change), corpus_roundtrip 3/3.

## HOLD / disposition
- Fix-forward edits the SAME case 5ea5c2aa3 (PR #1417) introduces → stack-dependent, land AFTER #1417.
- COSMETIC doc/formatting → per no-standalone-doc-polish directive, do NOT send a standalone MR now.
  FOLD: send 1ec5e078c after 5ea5c2aa3 lands (replays clean), OR batch with the adv-53/adv-54 pins when
  those peer fixes land. On sync after #1417 lands, 1ec5e078c replays onto the new base.

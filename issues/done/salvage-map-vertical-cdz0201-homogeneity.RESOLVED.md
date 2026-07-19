# Salvaged commit — evaluate for landing

Tag: `salvage/map-vertical`  (commit 2db6ae36)
Subject: spec+rcdzc: collection homogeneity is CDZ0201 UNIFORMLY (was split 0201/0203)

This unmerged commit from the old `map-vertical` worktree unifies collection-homogeneity
rejection under CDZ0201 (previously split across CDZ0201/CDZ0203). NOT in trunk. A patterns /
diagnostics vertical owner should evaluate whether this is still desirable against current trunk,
and if so, cherry-pick it (`git cherry-pick salvage/map-vertical`) in a worktree, gate, and send
pr-sync a merge-request. If superseded, mark this .REJECTED.md and drop the tag.

<!-- RESOLVED 2026-07-15 (trunk@, fix f2998d1ca): heterogeneous-collection error unified to CDZ0201 across list/map/set (the heterogeneous-LIST outlier that was CDZ0203 is fixed). 4 list cases repinned 0203→0201 + normative subsection in collections-and-text.md. Verified: (list 1 true)→CDZ0201. -->

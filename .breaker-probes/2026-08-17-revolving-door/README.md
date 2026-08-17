# rvd1 — revolving door with capacity jam (2026-08-17, tick 1669)

Attack: a whole-group admission clamp (fits-or-jams, no partial entry —
contrasts chr1's partial-take min) where the jam branch counts itself but
leaves occupancy UNTOUCHED, and the spin arm packs the released headcount
with `(% (+ revs 1) 10)` — the incremented field consumed in the answer
BEFORE the rebuild stores it (read-of-next-value shape). Occupancy is zeroed
by spin (field reset mid-protocol).

Differential: first group 3 vs 2 → the second entry (+2) JAMS at occupancy 3
on n=10 (900+1) but rides to 4 on n=0 (42); the post-spin pair then jams the
OTHER way (n=10's 3 enters clean then 2 jams; same on n=0 — but jam counts
end 2 vs 1). Every row differs.

Hand model: n=10 → 33901131033902123; n=0 → 22042141033901113 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk eae898166 (B2 DFS dedup —
code-cleanliness collab touching the bind-plan; no behavior change expected
or seen).

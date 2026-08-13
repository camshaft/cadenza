# 2026-08-13 weighted quorum (tick 1427)

- `quo1.sexp` — THREE-collection state in one nested tuple: (weights-Map,
  (voted-Set, tally)): vote checks the dedup Set, looks up the weight Map,
  advances the scalar tally, answers tally*10+pass-bit (quorum 6). Revote no-ops
  (reads Set only), unknown member -1 (reads Map only), fresh vote touches ALL
  THREE. Seed = member 2's WEIGHT: n=4 crosses quorum on the 3rd vote (71),
  n=1 only on the last (61). Nested-tuple state w/ Map+Set+scalar coordinated
  per-branch — extends mki1 (2 collections, all-branches-touch-both).
  PASS ×3 (303071191/303040161).

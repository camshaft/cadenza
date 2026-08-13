# 2026-08-13 map+keyset invariant pair (tick 1421)

- `mki1.sexp` — the state pairs a Map WITH its key Set, every put/rm updating
  BOTH structures in one transition (two heap writes per arm); chk answers a
  paired membership verdict (map-lookup digit + set-contains digit) that must
  agree at all times. n=5 collapses the two puts into ONE key (len stays 1,1)
  and the rm then EMPTIES both. Two different CHAMP structures maintained in
  lockstep through one thread — the invariant-pair idiom. Set seeded via
  Set.of empty-list (Set.empty doesn't exist). PASS ×3 (1122110011/1111000000).

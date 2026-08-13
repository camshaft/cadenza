# 2026-08-13 generic at two instantiations in one arm (tick 1375)

- `gsx1.sexp` — ONE arm builds the same user generic sum at TWO types per dispatch:
  (Container Int64) wrapping the live state and (Container String) wrapping a
  literal, both unwrapped and combined into the answer. The arm-side sibling of
  the #22 fence (which pinned two instantiations as RECORD FIELDS through
  value-encode); this pins them as ARM-LOCAL VALUES through the specialized fold's
  shape tables — post-#22, the memo must keep (Container Int64) and
  (Container String) distinct inside one arm body too. Extends the gs1-gs8 family
  (each of which uses ONE instantiation per case). PASS ×3 (33043/203213).

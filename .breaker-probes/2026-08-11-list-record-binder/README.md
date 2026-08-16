# List-element nested-record binder (2026-08-11) — 05-target (hold-safe)

Angle: the 97c119eb9 nested-record binder in LIST-ELEMENT pattern position
combined with a rest binder — the head record's field binds while the rest
carries FULL records readable downstream. (The landed nested-binder pins are
tuple-position; list-position + rest is the composition.)

GREEN x3:
- nl2: (list (record (x a)) .. rest) — head field + rest len + rest[0].y —
  3018/18

05 batch pool: lc1 + as1 + dc1 + ug1 + nl2 (5).

## HELD (tick 1286): nl2 fails the ML round-trip
Runs green x3 from the sexp surface, but ml_surface + all_surface_paths both
fail on it (1/6727) — the printer/reader mismatches on a record sub-pattern
inside a list pattern with a rest binder. FILED to v-syntax; pin held until
the ML surface round-trips it. (The corpus-edit-must-run-ML-round-trip rule
caught this pre-send — the gate alone was green.)

## RESOLVED (tick 1287): my authoring bug, not a compiler bug
v-syntax diagnosis: (record (x a)) in the PATTERN was the LEGACY non-canonical
field spelling — Phase B made (= x a) canonical in patterns too; rcdzc
tolerates the legacy form (gate green) but the ML printer canonicalizes, so
the round-trip "mismatch" was legacy-in/canonical-out. Rewrote nl2 to
(record (= x a)) — green x3 and now roundtrip-safe. Appendable again (14c
under the new by-agent convention). Vocab: (= k v) is canonical in PATTERNS
as well as values.

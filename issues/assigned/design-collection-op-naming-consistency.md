# Vertical-ready: consistent collection op naming

**Design doc:** `implementation/design/DESIGN-collection-op-naming-consistency.md` (landed on trunk).
**Subsystem:** rcdzc (prelude + a reject diagnostic) + cdz-tooling (codemod) + corpus/guide migration.
**Coordinate with:** `v-syntax` (surface round-trip), and the sibling `design-record-update-syntax`
(owns the Record op surface — Record naming is DEFERRED to it; it should adopt canonical `remove`).

## The scope (operator-confirmed 2026-07-15)
Three prelude surface renames, hard cutover, no alias:
- `Map.size` → `Map.len`
- `Tuple.cat` → `Tuple.concat`
- `Tuple.pop` → `Tuple.remove`

(`push` vs `insert` kept DISTINCT on purpose; `String.scalar-len`/`byte-len` untouched per spec;
Record `pop`/`with`/`extend`/`without` deferred to the sibling vertical.)

## First increment (C1)
Rename the three surface strings in `prelude.rs` (`map_module`, `tuple_module`) — surface key only,
intrinsics unchanged — and add diagnostic **CDZ0603 "renamed collection operation"**: projecting a
retired key on the module gives a fix-it to the new name (diagnostic-only hint map, the retired name
still fails to resolve — audit against memory `no-keys-outside-the-prelude`). Gate: reject test per
retired name + resolve/run test for each new name; `cargo xtask gate` FAIL SET unchanged.

Then C2 (`cdz rewrite` codemod rule) and C3 (mechanical corpus+guide migration, ~16 files,
`gate --save`). Full increments/seams/gate in the DESIGN doc.

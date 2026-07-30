# PR#910 review comment — recursive-newtype DESIGN doc spec ref missing spec/capabilities/ prefix (v-rust-backend)

Mirrored from GitHub PR#910 review comment (Copilot), id `3680065494`.
File: `implementation/seed/crates/rcdzc/src/backend/rust/DESIGN-recursive-newtype-box-emission.md:90` —
rust backend DESIGN doc → v-rust-backend. Blame `fe0bdf80c` "rcdzc(rust): DESIGN recursive-newtype —
BLOCKER: erasure is shared-lowering + spec-MUST".

## Comment (verbatim)

- (id 3680065494, DESIGN-recursive-newtype-box-emission.md:90) "The spec reference here is missing the
  `spec/capabilities/` prefix, which makes it ambiguous/misleading compared to the actual in-code
  citations in `lower.rs` (e.g., `//= spec/capabilities/type-system.md#...`). Updating the path will
  make the design note easier to verify and keep consistent with the codebase."

## Liaison verification (confirmed on trunk dfc83549e)

The DESIGN doc (line 88-90) cites the spec MUST as bare
`type-system.md#a-nominal-value-is-convertible-to-its-underlying-structural-value`. The ACTUAL duvet
citation in `lower.rs:384/386` uses the full path `spec/capabilities/type-system.md#a-nominal-value-is-
convertible-to-its-underlying-structural-value`, and the spec file exists at
`spec/capabilities/type-system.md`. So the DESIGN doc's bare `type-system.md#…` is inconsistent with the
in-code citation form and the real location — ambiguous to a reader verifying it. Fix: prefix with
`spec/capabilities/` to match `lower.rs`. Doc-only, behavior-neutral.

Owner: **v-rust-backend** (rust backend DESIGN doc, their `fe0bdf80c`). Add the `spec/capabilities/`
prefix to the spec ref.

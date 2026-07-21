# Shared backend space — plan (operator directive, 2026-07-18)

**Policy (operator, verbatim via concierge):** truly-common code shared between the rust and wasm
backends must be HOISTED into a backend-agnostic shared space, NOT made `pub` on one backend and
called from the other, and NOT copy-pasted. Rationale: adding a 3rd backend or removing one must not
break because backend B reaches into backend A's internals.

## Audit (rcdzc, 2026-07-18)

Direction of cross-backend coupling: **rust → wasm only** (5 call sites; no wasm → rust). Every
shared fn is `pub(crate)` in a wasm module but operates purely on backend-agnostic types
(`Db`, `StructId`, `Core`, `Layout`, `Reject`, `FxHashMap`) — genuinely common, misplaced in `wasm`.

| Fn | Defined in | Called from (rust) | Nature |
|----|-----------|--------------------|--------|
| `body_diverges(db, id) -> bool` | `wasm/select.rs:2902` | `rust/mod.rs:644`, `rust/expr.rs:2583` | Core divergence analysis (does a body always trap/never-return) |
| `refined_frame_for_branch(db, cond, then, base) -> FxHashMap` | `wasm/select.rs:12386` | `rust/expr.rs:749` | Core interval-refinement of a branch condition |
| `kebab_export_collision(layout) -> Option<Reject>` | `wasm/mod.rs:156` | `rust/mod.rs:320` | Export-name validation over Layout |
| `invalid_kebab_export_name(db, layout) -> Option<Reject>` | `wasm/mod.rs:195` | `rust/mod.rs:323` | Export-name validation over Layout |

These are 2 natural groups:
- **Core analysis** (`body_diverges`, `refined_frame_for_branch`) — reads `Core`, no backend state.
  These + their private helpers (`refine_from_comparison`, the `And`/`Compare` walk) belong together.
- **Export-name validation** (`kebab_export_collision`, `invalid_kebab_export_name`) — over `Layout`,
  emitted-name policy shared by any backend that names exports.

## Plan

Create `backend/common/` (a backend-agnostic sibling of `rust`/`wasm` under `backend/mod.rs`), with:
- `backend/common/diverge.rs` — `body_diverges` + `refined_frame_for_branch` (+ their private helpers,
  moved wholesale from `wasm/select.rs`). Both backends `use crate::backend::common::diverge::…`.
- `backend/common/export_name.rs` — `kebab_export_collision` + `invalid_kebab_export_name`.

Then: wasm's own call sites switch to the `common::` path; rust's `super::wasm::…` /
`crate::backend::wasm::select::…` calls become `crate::backend::common::…`. NO backend depends on
another backend's module. A 3rd backend uses `common` directly; removing a backend leaves `common` intact.

## Sequencing / ownership (per directive — coordinate, don't unilaterally move wasm's code)

The 4 fns LIVE in wasm modules, so moving them touches wasm — v-wasm-opt's territory. The directive CC'd
v-wasm-opt + the Core-layer owner. Proposed split to avoid a same-file collision:
1. **v-wasm-opt (owns the move):** create `backend/common/`, MOVE the 4 fns (+ private helpers) out of
   `wasm/select.rs`/`wasm/mod.rs` into `common/`, and repoint wasm's own call sites. One MR.
2. **v-rust-backend (me):** once (1) lands, repoint rust's 5 call sites from `super::wasm::…` /
   `crate::backend::wasm::select::…` to `crate::backend::common::…`. Trivial follow-up MR, rust-only.

This ordering keeps each MR single-owner + single-file-region and avoids rust and wasm both editing the
move in parallel. If v-wasm-opt prefers I do the whole hoist (I can — it's mechanical), we swap; but the
CODE being moved is wasm's, so v-wasm-opt landing the move first is the clean default.

## Status (2026-07-18)

- **Concierge APPROVED** `backend/common/` (sibling of rust/wasm) as the hoist location. v-wasm-opt
  raised `lower.rs` as an alternative for the two Core analyses; concierge ruled a dedicated
  `backend/common/` instead — `lower` is the Core-lowering pass, these are backend-shared
  analyses/policy that read cleaner as their own backend-agnostic module (and a 3rd backend can
  depend on `common` without pulling `lower` internals).
- **v-wasm-opt agreed** to own the move MR (the 2 `select.rs` fns are their territory + partly their
  coupling) when their S2 emit-reshape frees a slot. Cross-audits AGREE: rust→wasm only, exactly 5
  call sites (`body_diverges` called from 2 rust sites), zero wasm→rust — no misses.
- **Pending:** v-wasm-opt lands the move → v-rust-backend repoints rust's 5 call sites (same-tick follow-up).
- **WRINKLE — the export-name validators need `kebab_extern_name`/`is_kebab_word`.** ⚠️ A FIRST resolution
  ("call `cadenza_syntax::extern_name::` directly from `common/`") was WRONG and withdrawn:
  **`cadenza-syntax` is a `[dev-dependencies]` of rcdzc (Cargo.toml:72, TESTS ONLY)** — the rcdzc LIB is
  deliberately dependency-free (copy-don't-depend invariant; the wasm `kebab_extern_name`/`is_kebab_word`
  copies at wasm/mod.rs:62/90 EXIST precisely to keep it so). Lib code calling `cadenza_syntax::` would pull
  it into the compile path = the invariant violation. **CORRECT RESOLUTION (v-wasm-opt's Option (a)-with-copies):**
  MOVE the in-crate copies `kebab_extern_name` + `is_kebab_word` themselves INTO `common/export_name.rs`
  (they ARE backend-agnostic boundary-name policy + stay dependency-free copies), alongside the 2 validators;
  then repoint wasm's `envelope.rs` (42 uses) + `mod.rs` to `common::kebab_extern_name`/`is_kebab_word` and
  DELETE the wasm-local copies. ONE copy in `common/`, both backends use it — kills the rust→wasm cross-call
  AND the wasm-internal duplication, stays dep-free. Guard = `is_kebab_word(&kebab_extern_name(n))`. Bigger MR
  (envelope's 42 uses) but the only dep-safe hoist. (v-syntax's `is_kebab_word` pub is fine hygiene + their
  non-ASCII test pins the property, but the rcdzc lib does NOT consume it.) `diverge.rs` clean as-is.
- **SCOPE EXPANDED to the full boundary-name CLUSTER (v-wasm-opt, verified):** `is_valid_interface_name`
  (wasm/mod.rs:121) is ALSO cross-called — from `compile.rs` (the shared driver, above both backends) at 3
  non-test sites (144/173/2664, `crate::backend::wasm::is_valid_interface_name`) — the same anti-pattern, and
  it uses `is_kebab_word` too. So the coherent dep-safe hoist moves the WHOLE cluster of 5 to
  `common/export_name.rs`: `kebab_extern_name` (62) + `is_kebab_word` (90) + `is_valid_interface_name` (121)
  + `kebab_export_collision` (156) + `invalid_kebab_export_name` (195). Repoint ALL callers to
  `common::export_name::`: wasm `envelope.rs` (42) + wasm/mod.rs + **compile.rs (3, is_valid_interface_name)**
  + rust (2, the validators); delete the wasm copies. Kills EVERY cross-call (rust→wasm AND compile→wasm) +
  the wasm dup, one dep-free copy. Splitting would strand `is_valid_interface_name` as a compile→wasm
  cross-call. v-wasm-opt owns the cluster MR (incl. the compile.rs repoints — driver-level, not rust-backend);
  v-rust-backend repoints rust's 2 validator sites (mod.rs:320/323) same-tick after.
- **is_kebab_word BLOCKER + 2-MR split (v-wasm-opt behavior-neutrality check):** `invalid_kebab_export_name`
  calls `is_kebab_word`, which is PRIVATE in `cadenza-syntax`. `is_kebab_extern_name` is NOT a substitute:
  `kebab_extern_name` keeps non-ASCII VERBATIM (idempotent → `is_kebab_extern_name` returns true for an
  invalid non-ASCII name), whereas `is_kebab_word` REJECTS non-ASCII — swapping would silently drop the
  CDZ0201 non-ASCII export-name reject (a regression). So: v-syntax must `pub fn is_kebab_word`. Hoist SPLIT
  into 2 MRs: **(1) `common/diverge.rs` NOW** (clean, no dep) → v-rust-backend repoints rust's 3 diverge
  sites (`body_diverges` mod.rs:644 + expr.rs:2583; `refined_frame_for_branch` expr.rs:749). **(2)
  `common/export_name.rs` AFTER `is_kebab_word` is pub** → v-rust-backend repoints rust's 2 export sites
  (`kebab_export_collision` mod.rs:320; `invalid_kebab_export_name` mod.rs:323). v-syntax pinged for the pub.

## Going-forward rule

New common rust/wasm code goes into `backend/common/` from the start — never `pub` on one backend for the
other to call. This doc is the reference for the backfill + the standing policy.

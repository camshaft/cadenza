# PR review comment — mirrored from GitHub PR #392 (amazon-q inline) — LIKELY FALSE POSITIVE, please confirm

- **PR:** #392 "fleet: eighteenth batch (private-mutrec fix, LSP semanticTokens, iterators flat-map, fleet ack/event-wake)" (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/rcdzc/src/resolve.rs:745` (reviewer cites line 737)
- **Reviewer:** amazon-q-developer[bot] (automated) — flagged "🛑 Logic Error"
- **Comment id:** 3590315784
- **Link:** https://github.com/camshaft/cadenza/pull/392#discussion_r3590315784

## Comment (verbatim)
> 🛑 **Logic Error**: Line 737 uses `scoped.is_none()` but should check `scoped == Some(Err(()))` to avoid bypassing file-scoped visibility. When `scoped` is `Some(Err(()))`, it means the file is known but the variant is not visible in that file. The current logic incorrectly falls through to the flat index when it should reject the access entirely. This allows accessing variants from sibling files that should be hidden.

## Liaison triage — my read is FALSE POSITIVE; asking the PM to confirm against a build
`scoped: Option<Result<Value, ()>>`. Current code on trunk:
```
if let Some(Ok(value)) = scoped { return Ref{value}; }
else if scoped.is_none() && let Some(value) = db.prelude_colliding_variant_ctor(name) { return Ref{value}; }
```
Truth table:
- `Some(Ok(v))`  → first arm returns (visible in this file). ✓
- `Some(Err(()))`→ first arm no; `else if` guard `scoped.is_none()` is FALSE → neither branch fires →
  does NOT consult `prelude_colliding_variant_ctor` → falls through (no sibling leak). ✓
- `None`         → `is_none()` true → consult the flat companion index. ✓

The reviewer's suggested rewrite (`Some(Ok)` return / explicit `Some(Err(()))` no-op / else flat) is
LOGICALLY IDENTICAL — the existing `is_none()` guard already prevents the `Some(Err(()))` case from
reaching the flat index. So the behavioral claim ("allows accessing variants from sibling files") looks
incorrect: `Some(Err(()))` already does NOT leak.

BUT this is a soundness/visibility claim in the prelude-collision area (the fleet tracks related
unsoundness, e.g. leading-rest-list-binding), and amazon-q flagged it as a hard Logic Error, so I'm
NOT silently dropping it. Please confirm against a fresh build with a two-file repro: file A declares a
prelude-colliding variant ctor NOT exported to file B; file B references that name in construct
position — it must REJECT (unbound / prelude), NOT resolve to A's variant. If it rejects, this is a
confirmed false positive (dismiss). If it resolves, amazon-q is right and the guard needs the explicit
`Some(Err(()))` arm. Fix (if any) on `trunk`. Quote + link in queue file.

<!-- RESOLVED 2026-07-15 (FALSE POSITIVE, verified + pinned): PR #392 was WRONG — resolve.rs step 3d scoped.is_none() guard is LOAD-BEARING (weakening it to fire on Some(Err(())) re-opens a real sibling-hidden-variant leak). NO visibility-logic change. fix-resolve-visibility-guard landed a regression pin (5a3bd463c): a_sibling_files_prelude_colliding_variant_ctor_does_not_leak_in_construct_position in link.rs (two-file repro + local-shadow contrast). Merged. -->

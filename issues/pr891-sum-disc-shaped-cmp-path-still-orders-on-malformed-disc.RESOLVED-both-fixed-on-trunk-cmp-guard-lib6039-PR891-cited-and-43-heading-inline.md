# PR#891 review comments — sum_disc_shaped u32::MAX only declines on RENDER, cmp path still orders on a malformed disc (v-runtime)

Mirrored from GitHub PR#891 review comments (Copilot), ids `3672335481` (lib.rs:1432, follow-on to the
PR#889 fix) + `3672335524` (issues/pr889-…md:6, markdown-render nit on the mirrored note). Both trace to
v-runtime's PR#889 defensive #43 fix (`03d585262`).

## Comment 1 (verbatim) — lib.rs:1432 (follow-on, defensive)

- (id 3672335481, cdz-runtime/src/lib.rs:1432) "`sum_disc_shaped` returns `u32::MAX` for non-int
  immediates and the comment claims this will make callers `variants.get(disc)?` decline cleanly. That's
  true for render, but the shaped compare path returns an `Ordering` immediately when discriminants
  differ; a malformed immediate (or a negative int immediate cast to `u32`) can therefore yield a
  deterministic ordering instead of declining (returning `None`) like other malformed-vs-descriptor
  checks in the same matcher."

### Liaison verification (confirmed on trunk 5d9161085)

The PR#889 fix (`03d585262`, lib.rs:1424-1435) makes `sum_disc_shaped` return `u32::MAX` for a non-int
immediate so the RENDER caller's `variants.get(disc)?` declines. But the CMP path (lib.rs:6015-6035,
`Shape::Sum`) does: `let (da, db) = (sum_disc_shaped(a), sum_disc_shaped(b)); match da.cmp(&db) {
Ordering::Equal => { variants.get(da)? … } <else: the Less/Greater from da.cmp(&db) is RETURNED> }`. The
`variants.get(da)?` decline-guard is ONLY on the `Ordering::Equal` arm. When the discs DIFFER, `da.cmp(&db)`
returns `Less`/`Greater` IMMEDIATELY — so a malformed `u32::MAX` disc (or a NEGATIVE int immediate, which
`as u32` turns into a huge value) yields a DETERMINISTIC bogus ordering, not a clean decline. So the #889
fix's "declines cleanly" holds for render + the equal-disc cmp branch, but NOT the differing-disc cmp
branch. To make the cmp path decline consistently: detect `da == u32::MAX || db == u32::MAX` (out-of-range
disc) BEFORE the `da.cmp(&db)`, and bail to `None`/decline there — matching the "other malformed-vs-descriptor
checks in the same matcher". Owner's call whether reachable (same malformed-pairing question as #889) vs a
defensive belt-and-suspenders on the cmp path.

## Comment 2 (verbatim) — issues/pr889-…md:6 (markdown-render nit)

- (id 3672335524, issues/pr889-sum-disc-shaped-imm-tag-unchecked-defensive.md:6) "This line break turns
  `#43` into a Markdown heading (`#43). …`) instead of part of the sentence, which makes the issue note
  render incorrectly."

### Liaison verification (confirmed on trunk 5d9161085)

This is the tracked-archive MIRROR of the PR#889 liaison note (committed via the "fleet: mirror the work
queue … into the tracked archive" commit). Line 6 has `SOUNDNESS\n#43` — a `#43` at line-start renders as
an `<h1>43…`. Trivial doc-render fix: join the line or escape so `#43` stays inline prose. NOTE: this
`issues/` file is the fleet WORK-QUEUE ARCHIVE (mirrored, not v-runtime's source) — if v-runtime doesn't
own the archive mirror, bounce comment 2 to whoever does (concierge/corpus-bugfix archive lane); comment 1
is the substantive v-runtime one.

Owner: **v-runtime** for comment 1 (cmp-path decline consistency, their `03d585262` #43/#889 work).
Comment 2 is a mirror-doc nit — v-runtime or the archive owner; bounce if not theirs.

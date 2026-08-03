# PR #1762 review comments — issues/*.md (v-agent-harness-host PR; mixed ownership) — MERGED

https://github.com/camshaft/cadenza/pull/1762 (MERGED — cdz-agent-host trunk-failure fix: rustfmt + replay
rename). Copilot flagged that the change set ALSO lands unrelated `issues/` files (the archive-mirror
bundling artifact).

## 1. Scope: the trunk-fix PR also lands unrelated issue/review notes + fuzz findings under `issues/` (Copilot, issues/pr1747-*.md:1) — process/mechanism
> The PR is scoped to fixing cdz-agent-host trunk failures, but also adds unrelated issue/review notes and
> fuzz findings under `issues/`. If meant to land, update PR metadata.

This is the ARCHIVE-MIRROR mechanism (fleet mirrors the work-queue/issues into the tracked archive), so a
PR that syncs picks up whatever `issues/` files are staged — including MY github-liaison queue notes (e.g.
pr1747-*) and a fuzzer's `.smith` finding. Not a defect in #1762's actual fix; a known side effect of the
mirror bundling into whatever PR syncs next. NON-ACTIONABLE for #1762's author — noting the mechanism (same
family as the #1751 "my queue files rode into a corpus PR" observation). If the mirror-bundling is noisy in
review, that's a v-fleet-tooling archive-mirror-scoping question, not a per-PR fix.

## 2. A fuzzer `.smith` differential-artifact finding has an inaccurate canned intro (Copilot, issues/differential-artifact-…smith.md:1/7/34) — doc/accuracy [route to fuzzer owner]
> The title says the backends "DISAGREE on value", but the recorded Rust-side outcome is `artifact-error
> error[E0308]` (an error, not a value). The canned intro also claims the program made the compiler
> panic/hang/emit invalid wasm, but it's categorized as a differential-artifact finding. Wording is
> inconsistent with the recorded outcome.

This is a FUZZER-generated finding file (`.smith`), not a review comment on code — its canned template
overclaims (says panic/hang/invalid-wasm + "DISAGREE on value" when the Rust side is an E0308
artifact-error, i.e. a compile-error differential, not a value disagreement). The fuzzer's finding-template
should categorize by the ACTUAL recorded outcome (artifact-error vs value-mismatch). Route to the fuzzer
owner (v-fuzzer / whoever emits `.smith` findings) to fix the template's categorization + intro wording.
LOW/doc — the finding itself may still be a real differential; just the label is wrong.

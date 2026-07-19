# pr616 — cdz/main.rs doc comments say `Test.gen` after op renamed to `Test.gen-int` (GEN_OP_LABEL)

Mirrored from GitHub PR #616 review comment (Copilot), id 3609622231.
PR: https://github.com/camshaft/cadenza/pull/616 (8-MR publish batch)
Location: `implementation/seed/crates/cdz/src/main.rs:4770` (and MANY sibling lines)

## Reviewer comment (verbatim)
> The doc comment still describes the generator op as `Test.gen`, but this PR renames it to
> `Test.gen-int`. Keeping the old name here will mislead future readers and contradict `GEN_OP_LABEL`.

## VERIFIED (git show trunk) — real, and BROADER than the one flagged line
`const GEN_OP_LABEL: &str = "test.gen-int";` (main.rs:4770) is the renamed op, but the doc comment
directly above (lines 4766-4767) says "`Test.gen : Unit -> Int64` ... answers a `Test.gen` performance".
The stale `Test.gen` spelling also appears at LINES 3546, 4027, 4030, 4616, 4645, 4652, 4773 (grep
`Test\.gen` in the file) — so a fix should SWEEP all the doc-comment `Test.gen` → `Test.gen-int`, not
just the one line Copilot anchored to. Doc-comment only, no behavior change (the const + runtime already
use `test.gen-int`).

## Owner
`Test.gen`/`@property` generator op = v-property-testing (OWNS `@property`/`@exhaustive`/`Test.gen`). The
file is in the `cdz` crate but the naming is property-testing's domain. Sweep all stale `Test.gen`
doc-comment refs in cdz/main.rs to `Test.gen-int`.

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk cac57fd66): the stale doc-comments were SWEPT. On trunk
`cdz/src/main.rs` has ZERO occurrences of `Test.gen` (without `-int`) and 16 of `Test.gen-int` — all the
flagged sibling lines (3546/4027/4030/4616/4645/4652/4770/4773) now read `Test.gen-int`, matching
`GEN_OP_LABEL = "test.gen-int"`. Doc-comment-only nit, fully resolved by a peer's rename sweep — no corpus-bugfix action.

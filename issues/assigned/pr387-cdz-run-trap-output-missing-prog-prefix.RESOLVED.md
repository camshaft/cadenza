# PR review comment — mirrored from GitHub PR #387 (Copilot inline)

- **PR:** #387 "fleet: fourteenth batch (cdz run fold, closure-inference coverage, iterators, eval-order pins)" (MERGED)
- **File:** `implementation/seed/crates/cdz-run/src/cli.rs` (trap emission @172 peer-run branch, @204 main branch)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3589956341, 3589956392
- **Links:** https://github.com/camshaft/cadenza/pull/387#discussion_r3589956341 , #discussion_r3589956392

## Comments (verbatim)
> `RunArgs`/`run` take a `prog` name specifically so diagnostics point at the command the user typed, but trap output here is emitted as `trap: …` with no `prog` prefix. When mounted as `cdz run`, this loses the `cdz:` prefix and becomes inconsistent with the rest of the tool's stderr output (and with the missing-file test expectation).
>
> Same issue as the peer-run branch: trap output is printed as `trap: …` without the `prog` prefix, even though other errors are prefixed with `{prog}:`.

## Liaison triage — CONFIRMED against trunk
Confirmed in cdz-run/src/cli.rs: line 65 emits `eprintln!("{prog}: {e:#}")` for other errors, but both
trap paths (lines 170 & 202) emit a bare `eprintln!("trap: {msg}")` with no `{prog}:` prefix. When
mounted as `cdz run`, trap output loses the `cdz:` prefix — inconsistent with the tool's other stderr
and (per the reviewer) with a missing-file test expectation. Small tooling consistency fix in the
cdz-run CLI. No dedicated run-vertical, and it touches a test expectation, so route to `corpus-bugfix`
PM. Fix on `trunk`. Quotes + links in queue file.

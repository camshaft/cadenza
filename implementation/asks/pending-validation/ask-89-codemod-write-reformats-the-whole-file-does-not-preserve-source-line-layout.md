# ask-89 — codemod `rewrite --write` reformats the WHOLE file (collapses layout); no formatting-preserving edit

**State:** pending-validation (tooling change IMPLEMENTED — awaiting a re-probe on the real corpus edit).

## Resolution (implemented)

A **formatting-preserving (span-splicing) rewrite** is now the DEFAULT for `--write`/`--diff`/stdout
whenever the output surface matches the input and the input carries spans. It edits only the changed
subtrees at their source spans and copies every other byte — whitespace, newlines, comments,
hand-alignment — verbatim. Two pieces:

1. **The s-expr reader now records spans.** `sexpr::read_spanned` / `read_all_spanned` produce a
   `SpanTable` in lockstep with the arena (byte-identical to the untracked path — it is the round-trip
   oracle, verified by `spanned_arena_is_identical_to_untracked` + `cargo xtask roundtrip` 1030/0), so a
   `.sexp` target now carries spans exactly like an ML one. This was the blocker: the corpus is `.sexp`,
   and the old `sexpr::read` returned a span-free arena, so there was nothing to anchor a splice to.

2. **A span-guided minimal-edit engine** (`query::textedit`): align the original tree (with spans)
   against the rewritten tree (same LCS child-alignment as `treediff`), emit primitive edits — a changed
   operand is one span splice; a deleted list child is one span deletion (widened to swallow its own
   line's indent + trailing newline, so no blank line dangles); an inserted child is printed and spliced
   after its sibling. Applied as non-overlapping edits over the original text. The result is validated as
   a transaction (re-parsed and checked structurally-equal to the rewritten tree); if a splice can't be
   validated it falls back to a full reprint with a warning (never a corrupt write).

`--reprint` forces the old whole-tree canonical reflow (kept for deliberate normalization); a
cross-surface `--to` always reprints. `cadenza-syntax`: `sexpr.rs` (spanned readers), `query.rs`
(`textedit` module + `driver::apply_rewrite_preserving`), `bin/cdz-syntax.rs` (default + `--reprint`).
Tests: `rewrite_write_preserves_the_hand_formatted_layout`, `rewrite_preserving_diff_is_minimal`,
`rewrite_preserving_inserts_a_child_in_place`, `rewrite_reprint_flag_forces_canonical_layout`.

**Re-probe:** run the `(needs …)`-strip across `spec/semantics/*.sexp` with `--write` and confirm the
`git diff` is only the removed clause lines (not a 1387→1 reflow) and `cargo xtask roundtrip` is clean.
Manually verified on `09-functions.sexp`: 4 clause lines removed, every other byte identical.

---

**Priority (original):** P012 (blocks a real operator-directed bulk edit — see below).

## Finding

`cdz-syntax rewrite --write` writes each file back through the **sexpr pretty-printer**, which does NOT
preserve the source's line layout — it re-serializes the whole tree. On the semantics corpus, where each
`.sexp` file is one big `(do (case …) (case …) …)`, the printer collapses the **entire file onto a
single line**:

```
$ cdz-syntax rewrite '(case ,name ,doc (needs ,_) ,@rest)' '(case ,name ,doc ,@rest)' \
      spec/semantics/*.sexp --write --fixpoint
$ wc -l spec/semantics/06-numeric-model.sexp
1  spec/semantics/06-numeric-model.sexp     # was 1387 lines
```

The *edit itself was correct* (529 `(needs)` → 4, and a re-run would reach 0), but the output is a
single 1387→1-line file: every case, every multi-line `(doc "…")`, all on one line. The `git diff` is
"1 insertion, 1387 deletions" — unreviewable, and the corpus is meant to be read and diffed by humans
line-by-line. So `--write` is unusable for editing a hand-formatted file, even when the structural
transform is exactly right.

## Why it matters

This blocks the operator-directed **`(needs …)`-strip** (DIRECTIVE-retire-needs-tag.md): a mechanical
removal of ~481 clauses across ~28 corpus files — precisely the "kick the tires on the codemod tool" task
the operator wanted it for. The transform works; the *serialization* makes it unlandable. Any bulk edit
of a hand-formatted source file hits this — it is the difference between a codemod tool and a whole-file
reformatter. (Related: ask-88, the one-`,@`-splice limit forces a fragile fixed-position pattern; even
with that solved, this layout problem remains the blocker.)

## Proposed resolution

A **formatting-preserving rewrite**: `--write` (and `--diff`) should splice the replacement text into the
original source at the matched node's SPAN, leaving every unmatched byte — including all surrounding
whitespace/newlines/comments — exactly as it was. The engine already tracks each match's span (query
prints it), so a span-anchored textual splice of just the changed subtrees would yield a minimal,
line-preserving diff (only the removed `(needs …)` clause and its line vanish). This is how mainstream
codemod tools (comby, ast-grep, jscodeshift) work — edit at spans, don't reprint. Absent that, at minimum
`--write` should refuse (or warn) when the round-trip would reflow a file whose layout differs from the
printer's canonical form, so a bulk edit can't silently destroy a hand-formatted corpus.

## Evidence

Reproduce with the command above on any `spec/semantics/*.sexp` (revert with `git checkout --`). The
`(needs)`-strip was reverted for this reason; the strip stays BLOCKED until either this or a
span-splicing edit mode lands. `cadenza-syntax` printer (`printer.rs`) + the `rewrite --write` path in
`bin/cdz-syntax.rs`.

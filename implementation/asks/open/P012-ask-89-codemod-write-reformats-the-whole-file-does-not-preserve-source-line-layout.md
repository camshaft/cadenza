# ask-89 — codemod `rewrite --write` reformats the WHOLE file (collapses layout); no formatting-preserving edit

**State:** open (tooling change). **Priority:** P012 (blocks a real operator-directed bulk edit — see below).

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

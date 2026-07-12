# ask-88 — codemod `rewrite` cannot delete a clause at an arbitrary child position (one-`,@`-splice limit)

**State:** open (tooling change). **Priority:** P020 (ergonomics — the tool works via a workaround; this is about making the natural expression legal).

## Finding

`cdz-syntax rewrite` restricts a pattern list to **at most one `,@` splice**. So the natural way to
*delete a child clause from anywhere in a variadic form* — match the surrounding run before and after
it — is rejected:

```
$ cdz-syntax rewrite '(case ,@a (needs ,_) ,@b)' '(case ,@a ,@b)' file.sexp --diff
cdz-syntax: a pattern list may contain at most one `,@` splice
```

This came up doing the operator-directed bulk edit **"strip every `(needs …)` clause from every `(case
…)` in `spec/semantics/*.sexp`"** (DIRECTIVE-retire-needs-tag.md, ~481 clauses across ~28 files). The
clause-deletion idiom `(F ,@before TARGET ,@after) → (F ,@before ,@after)` is the single most natural
structural edit for a variadic form, and it is exactly the one the one-splice rule forbids.

## Why it matters

Clause deletion (and clause insertion at a position) is a bread-and-butter refactor: remove a `(needs)`
tag, drop a deprecated field from a record, delete a now-defaulted argument. Without two splices the
author must either (a) assume the target sits at a FIXED position and match around it with one trailing
splice — `(case ,name ,doc (needs ,_) ,@rest) → (case ,name ,doc ,@rest)` — which is fragile (breaks if
the clause moves, and misses a case with the clause elsewhere), or (b) enumerate every position. Neither
is robust for a real corpus.

## Workaround used (this task)

The `(needs)` clause happens to sit right after `(doc …)` in 477/481 cases; the other 4 are cases with
TWO consecutive `(needs …)` clauses. So `(case ,name ,doc (needs ,_) ,@rest) → (case ,name ,doc ,@rest)`
with `--fixpoint` (re-apply until stable) strips all of them — the second `(needs)` becomes
doc-then-needs on the next pass. It works, but ONLY because the position is near-fixed and fixpoint
mops up the doubles; a clause at a genuinely arbitrary position would be unreachable.

## Proposed resolution

Allow **more than one `,@` splice per pattern list** when the non-splice pattern elements between them
are enough to make the match unambiguous (the classic "match a bounded sub-sequence" — greedy/anchored
on the fixed middle element). i.e. permit `(F ,@a X ,@b)` where `X` is a concrete/bound element that
anchors the split. If full generality is hard, a narrower dedicated affordance would also close the gap:
a `--delete PATTERN` mode that removes every matched node from its parent's child-list (the deletion dual
of a rewrite), so `cdz-syntax rewrite --delete '(needs ,_)' …` just drops the clause wherever it sits.

## Evidence

`cadenza-syntax` rewrite engine (the one-splice check is in the pattern compiler). Reproduce with the
command above on any `spec/semantics/*.sexp`. Verified the fixpoint+fixed-position workaround strips all
481 clauses with 0 `(needs)` remaining and `cargo xtask roundtrip` clean.

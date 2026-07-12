# ask-88 — codemod `rewrite` cannot delete a clause at an arbitrary child position (one-`,@`-splice limit)

**State:** pending-validation (tooling change IMPLEMENTED — awaiting a re-probe on the real corpus edit).

## Resolution (implemented)

The one-splice-per-list rule is lifted: a pattern list may now contain **several `,@` splices as long
as no two are ADJACENT** — a fixed element between them anchors each run boundary. The matcher grew a
backtracking `match_splice_seq` (each splice tries every feasible run length; snapshot/restore bindings
per branch) alongside the fast paths for zero and one splice. Only directly-adjacent splices (`,@a ,@b`,
nothing to divide the run on) are still rejected, now with a clearer message. So the clause-delete idiom

```
(case ,@before (needs ,_) ,@after) → (case ,@before ,@after)
```

matches a target sitting ANYWHERE in a variadic form (front, middle, back). `cadenza-syntax` `query.rs`
(`compile_pat` adjacency check + `match_seq`/`match_splice_seq`); unit tests
`two_splices_around_a_fixed_anchor_delete_a_clause`, `three_splices_two_anchors`,
`two_splices_backtrack_when_the_first_greedy_run_blocks_the_anchor`, and CLI
`rewrite_deletes_a_clause_at_an_arbitrary_position_via_two_splices`.

**Re-probe:** run the `(needs …)`-strip on `spec/semantics/*.sexp` with the two-splice pattern (no
`--fixpoint` / no fixed-position fragility) and confirm 0 clauses remain. Landed together with ask-89
(the layout-preserving `--write`), which is what makes the strip actually landable.

---

**Priority (original):** P020 (ergonomics — the tool works via a workaround; this is about making the natural expression legal).

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

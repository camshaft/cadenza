# PR review comments — mirrored from GitHub PR #403 (Copilot inline)

- **PR:** #403 "fleet: twenty-eighth batch (collection-op naming cutover LANDED, + 12 features)" (MERGED)
- **Files:** `spec/learnings/README.md:1222`, `rcdzc/src/tests.rs:38084`, `rcdzc/src/lower.rs:17542` + `:17602`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3590973883, 3590973923, 3590973944, 3590973960
- **Links:** https://github.com/camshaft/cadenza/pull/403#discussion_r3590973883 (+ r3590973923, r3590973944, r3590973960)

## Comments (verbatim, condensed)
> [README.md:1222] Still lists the tuple row-op analogue as `pop`, but the cutover renamed `Tuple.pop` → `Tuple.remove`.
> [tests.rs:38084] The assert message describes `Tuple.remove` as "dropping the last element", but `Tuple.remove` is the renamed `Tuple.pop` (removes element 0, returns `(tuple head rest)`).
> [lower.rs:17542] The `Tuple.cat` → `Tuple.concat` rename is in the decline message, but the doc comment and trace string still say `Tuple.cat`.
> [lower.rs:17602] The `Tuple.pop` → `Tuple.remove` rename is applied to user-visible declines, but the doc comment and trace still say `Tuple.pop`.

## Liaison triage
A cluster of stale-name leftovers from the collection-op naming cutover (`Tuple.pop`→`Tuple.remove`,
`Tuple.cat`→`Tuple.concat`) that this very PR (#403) LANDED: the surface/decline messages were renamed,
but README text, a test-assert message describing wrong semantics, and two lower.rs doc-comment/trace
strings still carry the old names (and tests.rs:38084 additionally MISDESCRIBES the semantics —
`Tuple.remove` removes element 0, not "the last element"). All low-severity doc/trace consistency, but
the tests.rs one is a semantics-description error worth fixing. Corpus/compiler-doc territory; route to
`corpus-bugfix` PM (they own the cutover). Fix on `trunk`. Quotes + links in queue file.

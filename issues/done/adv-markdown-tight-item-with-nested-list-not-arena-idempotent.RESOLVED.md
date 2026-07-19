# BREAKER FINDING 2026-07-17 (trunk 502b99eea vintage) — markdown surface NOT arena-idempotent for a TIGHT item carrying a NESTED list

**Violates the module's own declared invariant** (`cadenza-syntax/src/markdown.rs:1153` —
"A tight list (inline text directly under `item`) and a nested list must both round-trip",
`assert_idempotent`). The existing tests cover a tight item and a nested-paragraph item separately;
the COMBINED face — tight text + directly nested list in ONE item — drifts:

```
- item
  - nested
```

- read #1: `(item (text "item") (list …))` — tight.
- print: emits a BLANK line between "item" and the nested list (`- item\n  \n  - nested\n`).
- read #2: the blank line makes the item LOOSE → `(item (paragraph (text "item")) (list …))` —
  a DIFFERENT tree. (Converges at round 2 — the loose form is stable — but idempotence is broken.)

Repro: `cdz convert m.md -f markdown -t sexpr`, reprint with `-t markdown`, re-read, diff trees.
Also observed in a larger document (tight item + inline code + nested link list): same
text→paragraph wrapping on round 2.

Not graded corpus surface (reader-level), so filed as .md not .sexp. Severity: low-moderate —
the literate corpus pipeline (`cdz corpus migrate` → markdown → re-read) and any md-authored
guide content silently reshapes lists on the second pass; `assert_idempotent`-style tooling
diffs will churn. Fix locus: the list PRINTER emitting the separator blank line inside a tight
item before a nested list (markdown.rs print path) — a tight item's nested list should follow
its text directly, matching what the reader accepts as tight.

Suggested owner: v-syntax (markdown.rs is theirs; the module's test just needs the combined
face added once fixed: `assert_idempotent("- item\n  - nested\n")`).

---
ROUTED to v-syntax (corpus-bugfix 2026-07-17): EMPIRICALLY CONFIRMED via a temp assert_idempotent("- item\n  - nested\n") -> FAILED (printer emits a spurious blank line inside the tight item before the nested list, loosening it on re-read; reverted the probe). cadenza-syntax/src/markdown.rs round-trip gap — the modules own test nested_list_and_tight_items (1153) covers tight + nested SEPARATELY, not combined. Fix: suppress the list-printer separator blank inside a tight item before a nested list. Not spawning (fixer cap). Promote when fixed.

---
RESOLVED-PENDING-MERGE (corpus-bugfix 2026-07-17, per v-syntax): FIXED in cadenza-syntax/src/markdown.rs
commit f915a3a7f (MR sent). Root = exactly the identified locus: print_blocks emitted a blank-line
separator before the nested list inside a tight item; suppressed it on the inline-run->list transition
(prev_inline flag) so the sublist hugs the tight text. Combined case added to nested_list_and_tight_items
(+ ordered/2-level/nested-then-siblings). Gate green (lib 589/0 incl generative md idempotency sweep).
Close once f915a3a7f lands.

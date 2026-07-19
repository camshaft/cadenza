# PR review comment — mirrored from GitHub PR #410 (Copilot inline)

- **PR:** #410 "fleet: thirty-fifth batch (restore iter-set, symtab.cdz, open-sums, broad features)" (MERGED)
- **File:** `spec/semantics/.gate-baseline:2828`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591321302
- **Link:** https://github.com/camshaft/cadenza/pull/410#discussion_r3591321302

## Comment (verbatim)
> `.gate-baseline` now contains duplicate entries for the same two open-sum schema-decode case descriptions (first marked `todo`, then `pass`). `xtask gate --check` loads this file into a HashMap keyed by description, so duplicates silently overwrite and mask a verdict.

## Liaison triage — CONFIRMED STILL PRESENT on trunk (NOT fully deduped)
Even though trunk's tip is `7602d2df7 "corpus: dedup the stale todo copies of the two OS2 schema-decode
baseline entries"`, the duplicates PERSIST on trunk. `awk -F'\t' '{print $2}' | sort | uniq -d` shows
THREE duplicated descriptions:
- "an open sum payload that does not match its schema yields a typed failure, not a trap" — appears as
  BOTH `todo` and `pass`.
- "an open sum's payload decodes against a schema to a typed result" — BOTH `todo` and `pass`.
- "three distinct closure signatures cross as three resource types" — a fully-identical `pass` line
  appears TWICE.
`xtask gate --check` keys verdicts by description in a HashMap, so a duplicated description silently
keeps only the last-loaded verdict, masking the other (a `todo` shadowed by a `pass`, or vice-versa —
which can hide a real regression or a not-yet-passing case). The dedup commit did NOT fully take. Gate
tooling / corpus integrity → route to `corpus-bugfix` PM (they own the gate baseline). FIX: remove the
stale duplicate lines so each description is unique (and consider making `gate --save`/`--check` reject
duplicate descriptions rather than silently overwriting). Fix on `trunk`. Quote + link in queue file.

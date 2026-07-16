# PR review comment — mirrored from GitHub PR #440 (Copilot inline)

- **PR:** #440 "fleet: batch 69+70 (…, quantity grounding)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/resolve.rs:2820` (`map_form_binds_name`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592667935
- **Link:** https://github.com/camshaft/cadenza/pull/440#discussion_r3592667935

## Comment (verbatim)
> `map_form_binds_name` treats everything after `..` as a rest-position binder only if it is a bare name. In a malformed pattern like `(map (1 v) .. rest (2 w))`, a body reference to `w` would still resolve as UNBOUND because `(2 w)` is a list item after `..` and this branch never treats its value position as a binder. To fully suppress the misleading CDZ0101 cascade for malformed rest patterns, also treat `(k v)` pairs after `..` as binding `v`.

## Liaison triage — CONFIRMED against trunk
Confirmed in resolve.rs `map_form_binds_name`: after the `..` marker, the code accepts a bare-name item
as a rest binder, but a `(k v)` pair appearing AFTER `..` (only valid in a malformed pattern) isn't
treated as binding its value `v` in that rest-position branch — so a body reference to `w` in
`(map (1 v) .. rest (2 w))` resolves UNBOUND and emits a misleading CDZ0101 cascade on top of the
already-malformed pattern. Low severity (only bites a malformed pattern; the goal is cleaner diagnostics
by suppressing the secondary cascade). FIX: also treat `(k v)` pairs after `..` as binding `v`.
Pattern-matching territory (v-patterns owns map-pattern binding). Fix on `trunk`. Quote + link in queue
file.

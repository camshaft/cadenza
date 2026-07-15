# PR review comment — mirrored from GitHub PR #392 (Copilot inline) — SECURITY (NEW, distinct from pr391)

- **PR:** #392 "fleet: eighteenth batch (private-mutrec fix, LSP semanticTokens, iterators flat-map, fleet ack/event-wake)" (MERGED; file on trunk)
- **File:** `xtask/src/fleet.rs:824` (the `fleet ack` request resolution)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590351482
- **Link:** https://github.com/camshaft/cadenza/pull/392#discussion_r3590351482

## Comment (verbatim)
> `fleet ack` treats a non-existent `request` as a basename and blindly does `fleet.inbox("pr-sync").join(request)`. Inputs like `../registry.json` (or `processed/../…`) can make `ack` read and then `rename` an arbitrary file outside pr-sync's inbox. Require `request` to be either an existing file path [or a validated single-component basename].

## Liaison triage — CONFIRMED against trunk — SECURITY (new)
Confirmed on trunk in `xtask/src/fleet.rs`:
```
let path = { let p = PathBuf::from(request);
    if p.is_file() { p } else { fleet.inbox("pr-sync").join(request) } };
let text = std::fs::read_to_string(&path)...   // then later renames it to processed/
```
`request` is joined into pr-sync's inbox with NO single-component validation, so `../registry.json` (or
`processed/../…`) makes `ack` read and then RENAME an arbitrary file outside the inbox. This is DISTINCT
from the pr391/slack-bridge agent-name traversal (which is now fixed — see below); it's a different
entry point (the `fleet ack` CLI request arg) not covered by the agent-name guard. Fleet-tooling
territory (`v-fleet-tooling` owns xtask fleet). FIX: require `request` to be either an existing file
path OR a validated single path component (no separators, no `..`) before joining; reject otherwise.
Fix on `trunk`.

## Sibling comments on the same PR — ALREADY FIXED on trunk (recorded done, not filed)
- 3590351401 (format.rs `@..` parse) and 3590351439 (inbox.rs `deliver`/`inbox_dir`): the Rust
  slack-bridge port now has defense-in-depth — the parser requires a strict `[A-Za-z0-9][A-Za-z0-9-]*`
  slug (no dots), and `inbox_dir` re-validates via `is_valid_agent_name`, explicitly citing PR #391.
  This is the fix for MY pr391 security note — the loop closed.
- 3590351463 (inbox.rs `drain`): `drain` routes through the same `inbox_dir` chokepoint, so it's
  covered by that guard.

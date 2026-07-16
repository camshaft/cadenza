# PR review comment — mirrored from GitHub PR #430 (Copilot inline)

- **PR:** #430 (MERGED)
- **File:** `xtask/src/fleet.rs:1301` (`merged_floor_value`, the coverage-floor merge driver)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592228076
- **Link:** https://github.com/camshaft/cadenza/pull/430#discussion_r3592228076

## Comment (verbatim)
> `merged_floor_value` only updates counters if `ours` is a JSON object; if the file is valid JSON but not an object (e.g. `null`, a number, or some other accidental shape), the driver will currently treat it as parsed successfully, "resolve" the conflict, and may rewrite an invalid floor. Since the merge driver contract is specifically for `{... "cited": u64, "total": u64 ...}`, it should reject non-object values and leave the conflict for a human.

## Liaison triage
The coverage-floor max-merge driver (part of the gate-baseline/coverage auto-dedup machinery — cf. the
pr426 register_merge_drivers note): `merged_floor_value` only updates counters when `ours` is a JSON
object, but a valid-JSON-non-object (`null`, a number) parses "successfully", so the driver resolves the
conflict and may rewrite an INVALID floor rather than leaving it for a human. FIX: reject non-object
values (the contract is `{cited: u64, total: u64}`) and leave the merge conflict unresolved. Fleet-
tooling (v-fleet-tooling). Fix on `trunk`. Quote + link in queue file.

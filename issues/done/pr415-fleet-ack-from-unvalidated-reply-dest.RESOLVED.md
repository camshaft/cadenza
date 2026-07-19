# PR review comment — mirrored from GitHub PR #415 (Copilot inline)

- **PR:** #415 "fleet: fortieth batch (…, fleet ack routing, …)" (MERGED)
- **File:** `xtask/src/fleet.rs:841`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591538857
- **Link:** https://github.com/camshaft/cadenza/pull/415#discussion_r3591538857

## Comment (verbatim)
> `from` is used later as a reply destination (`fleet ack` delivers to `mr.from`), but here it can be derived from `$FLEET_AGENT` or the git branch without validating it against `validate_agent_name`. If it contains invalid characters (e.g. a branch like `fleet/foo/bar` → `from="foo/bar"`), `fleet ack` will fail when it tries to deliver the reply (recipient validation happens at `deliver`), reintroducing reply-loss / stuck-sender behavior. Validate the resolved sender name here and refuse early with a clear error.

## Liaison triage — CONFIRMED against trunk
Confirmed: the resolved `from` (from arg → `$FLEET_AGENT` → `sender_from_branch` → `"unknown"`) is NOT
passed through `validate_agent_name` before being used as the reply destination. A branch like
`fleet/foo/bar` could yield `from="foo/bar"`, which `deliver` (the traversal-guarded sink) rejects LATE
— so the ack reply is lost and the sender is stuck (the exact reply-loss class the ack routing was meant
to fix). Fleet-tooling territory (v-fleet-tooling). FIX: validate the resolved sender name at
derivation and refuse early with a clear error (rather than failing at deliver). Fix on `trunk`. Quote
+ link in queue file.

# pr556 — cdz-kernel daemon.rs: unescaped payload interpolated into Cedar EntityUid literal

Mirrored from GitHub PR #556 review comment (Copilot), id 3607332045.
PR: https://github.com/camshaft/cadenza/pull/556 (13-MR publish batch)
Location: `implementation/seed/crates/cdz-kernel/src/daemon.rs:150`

## Reviewer comment (verbatim)
> `resource` is built by interpolating raw `payload` into a Cedar EntityUid string literal. If
> `payload` contains `"`, `\`, or control characters, `cdz_agent::cedar::authorize` will fail to
> parse the resource UID (or potentially parse something unintended). Escape the payload before
> embedding it so policies can safely key on arbitrary payloads and the authz check behaves
> deterministically.

## Triage
Real robustness/correctness concern in the agent kernel's authorization path: raw payload
interpolated into a Cedar EntityUid string literal → a payload with `"`/`\`/control chars breaks
UID parsing or (worse) parses something unintended, making the authz check non-deterministic on
arbitrary payloads. Copilot (accurate track record). Owner = v-agent-harness (owns cdz-kernel /
the daemon + Cedar authz). Fix = escape the payload before embedding.

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk c88c950be): FIXED in cdz-kernel/src/daemon.rs. A
dedicated `cedar_escape(s)` fn (daemon.rs:140) escapes backslash + double-quote + control chars per Cedar's
string grammar; it is APPLIED at the EntityUid construction site (daemon.rs:192): `let resource =
format!("Resource::\"{}\"", cedar_escape(&payload))` with an explanatory comment (188). So a payload with
`"`/`\`/control chars can no longer break UID parsing or inject an unintended rule — the authz check is
deterministic on arbitrary payloads. Doc cites "Copilot PR#556 hardening"; dedicated test
`cedar_escape_neutralizes_quote_backslash_and_control_chars` (daemon.rs:667) covers all three char classes.
Owner (v-agent-harness) resolved — no corpus-bugfix action.

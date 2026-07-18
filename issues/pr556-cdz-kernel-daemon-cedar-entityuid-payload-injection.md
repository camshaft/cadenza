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

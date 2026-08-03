# PR #1829 review comment — cdz-kernel/src/name_store.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1829 (§4c set/resolve slice 2 — NameStore). The anti-hijack
namespace (#1747 lineage — security-relevant).

## `authority_prefix_of` admits DEGENERATE prefixes (`system/`, `team/`, `session/`) → weakens fail-closed posture (Copilot, name_store.rs:146) — security/correctness [VERIFIED]
> `authority_prefix_of` classifies any string that merely starts with `system/`/`team/`/`session/`/
> `memory/` as scoped. Degenerate names like `system/`, `team/`, `team/<team>` (no trailing segment),
> `session/` are treated as writable (set only backstops Unscoped). This contradicts the docs
> (`system/…`, `team/<team>/…`, `session/<id>/…`) and weakens fail-closed for malformed prefixes.
VERIFIED on the cand branch: `authority_prefix_of` is bare `if name.starts_with("system/") { System }
else if starts_with("team/") { Team } …`. So `"system/"` (empty tail), `"team/"` (no team), `"session/"`
(no id) all classify as their scoped authority — but the module docs require a non-empty segment
(`team/<team>/…` etc.). A degenerate/malformed prefix being treated as a valid scoped (writable) authority
weakens the intended fail-closed anti-hijack posture. Tighten to require a non-empty segment after the
prefix (e.g. `team/` must be `team/<nonempty>/…`), classifying malformed ones as Unscoped (or a dedicated
reject). MED/security-posture — recommend v-agent-harness confirm the intended grammar + fail-closed. Fix-forward.

# PR #1295 review comment — cdz-agent-host/tests/fixtures/cedar-policy-guest/src/lib.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1295 (PR: "cand: v-agent-harness-host — a6a1bc85b").
VERIFIED against the diff (lines 122/124/126 do use `{:?}`).

## `EntityUid::from_str(&format!("Principal::{:?}", …))` uses Debug to build Cedar syntax (amazon-q, lib.rs:122/124/126) — robustness/correctness
> Using `{:?}` Debug formatting to construct Cedar EntityUid strings will malform the entity IDs.
> Debug formatting adds extra quotes around strings, producing invalid Cedar syntax like
> `Principal::"\"agent\""` instead of `Principal::"agent"`. This will cause EntityUid parsing to fail
> and all authorization requests to be denied with parse errors.
> [suggests explicit `format!("Principal::\"{}\"", request.principal)` etc.]

VERIFIED: the code really is `format!("Principal::{:?}", request.principal)` (+ Action/Resource) at
lines 122/124/126.

⚠ SEVERITY NUANCE (relaying honestly): amazon-q slightly overstates the *normal* case — for a bare id
like `agent`, `{:?}` on a `String` yields `"agent"`, so `Principal::{:?}` → `Principal::"agent"`, which
IS valid Cedar and parses fine (that's likely why the fixture's tests pass). The real problem is
robustness/intent: `{:?}` is Rust Debug escaping, NOT Cedar string escaping — any id containing a quote,
backslash, or control char (or a future non-trivial principal/action/target) would be Debug-escaped
into malformed Cedar and silently DENY with a parse error. Since this is the foundational Cedar authz
guest, prefer the explicit `format!("Principal::\"{}\"", …)` form so entity syntax doesn't ride on
Debug's incidental quoting. (Not a live failure for the current simple ids, but a latent fail-closed
trap on any special-char identity — worth fixing in the authz path specifically.)

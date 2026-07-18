# pr548 — gate harness `rust_factory_param_count` prefix-match picks wrong export

Mirrored from GitHub PR #548 review comment (Copilot), id 3607067424.
PR: https://github.com/camshaft/cadenza/pull/548 (5-MR publish batch)
Location: `xtask/src/main.rs:1614`

## Reviewer comment (verbatim)
> `rust_factory_param_count` looks for the function header via `module.split(&needle).nth(1)`,
> which can pick up a *prefix* match (e.g. `pub fn both2...` when searching for `both`) and then
> return `None` even though the real `pub fn both(` appears later. That would prevent factory-call
> argument splitting and make the gate harness call the wrong arity for closure-factory exports in
> multi-export modules.
>
> Search all occurrences and only accept the one where the post-name boundary actually begins the
> param list (`(`) or async generic list (`<`).

## Triage
Real correctness bug in the gate's Rust harness: a prefix match (`both2` when searching `both`)
makes `rust_factory_param_count` return `None`, so the harness splits factory-call args wrong /
calls the wrong arity for closure-factory exports in a multi-export module. Same bug CLASS as the
`cdz run-rust` first-`pub fn` split filed on PR #547 (id 3607027381) — a naive substring match on a
function name. Fix = match on the name's actual boundary (`(` or `<`), scanning all occurrences.

## Owner AMBIGUITY (defaulting to PM)
Helper lives in `xtask/src/main.rs` (v-fleet-tooling owns xtask/fleet.rs + window.sh) but the
concern is the rust-backend closure-factory feature (v-rust-backend). Filing to corpus-bugfix PM to
triage the owner against a fresh build rather than guessing.

---
ROUTED to v-fleet-tooling (owns xtask), CC v-rust-backend (closure-factory gate concern) — corpus-bugfix 2026-07-18. Prefix-match arity bug in the rust gate harness; fix = name-boundary match + scan all occurrences. Fold into next xtask commit.

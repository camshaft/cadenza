# PR #1295 review comments (cluster) — cdz-agent-host cedar-policy-guest fixture (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1295 (PR: "cand: v-agent-harness-host — a6a1bc85b").
Four related Copilot points about the Cedar guest fixture — WIT vendoring drift + overstated build/CI claims.
(This is in addition to the `{:?}` EntityUid note already filed for this PR.)

## 1. Fixture vendors its own authorizer.wit instead of binding the canonical one (Copilot, src/lib.rs:19) — drift risk
> This fixture vendors its own copy of authorizer.wit and points wit-bindgen at it. In the repo,
> reducer-guest avoids this drift risk by binding directly to the canonical WIT via a relative path;
> doing the same here would remove the need to keep two copies in lockstep.

## 2. "build-time test" claim overstated — it's a fixture-only unit test, crate excluded from workspace (Copilot, src/lib.rs:14, also :73) — doc
> The module docs say a malformed embedded policy is caught by a "build-time test", but the check
> shown is a unit test that only runs when this fixture's tests are executed (and this crate is
> intentionally excluded from the main workspace). Reword to avoid implying the parse is validated
> during normal builds/CI unless a dedicated job runs it.

## 3. Cargo.toml comment describes CI + committed .component.wasm that don't exist yet (Copilot, Cargo.toml:15) — doc/CI
> This comment says a CI job compiles/lifts this fixture and that a committed .component.wasm is what
> e2e loads, but there's currently no committed cedar-policy guest component artifact (unlike
> reducer_guest.component.wasm) and the existing CI workflow only rebuilds reducer-guest. Either add
> the corresponding CI + fixture artifact, or soften this comment so it doesn't describe behavior
> that doesn't exist yet.

## 4. WIT header claims "byte-identical" to canonical but it differs (Copilot, wit/authorizer.wit:5) — doc/drift
> The header claims this vendored WIT file is "byte-identical" to cdz-kernel's canonical
> wit/authorizer.wit, but it differs substantially (at least in comments). Either vendor an exact
> copy or change the wording to reflect that only the interface shape is intended to match.

Theme: the fixture keeps a second copy of the authorizer WIT and its docs describe a build/CI/artifact
regime that isn't wired yet. Best structural fix (points 1+4) is to bind the canonical WIT by relative
path like reducer-guest does, eliminating the drift; failing that, at least correct the
"byte-identical" claim. Points 2+3 are doc-accuracy: either stand up the dedicated build/CI + commit
the component artifact, or soften the comments so they don't describe not-yet-existing behavior.

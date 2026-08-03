# PR #1747 review comments — .github/workflows/checks.yml + cdz-kernel/src/{lib,namespace}.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1747 (titled "ci(checks): build the reducer-guest fixture with
--locked").

## 1. Scope creep: a CI-titled PR also adds a public `cdz-kernel::namespace` module (Copilot, namespace.rs:5) — process/reviewability [VERIFIED]
> The PR title/description are scoped to making the reducer-guest fixture build with `--locked`, but this
> PR also introduces a new `cdz-kernel::namespace` module and exports it publicly. If intentional, update
> the PR metadata to reflect the API surface change; otherwise split it into a separate PR to keep
> review/rollback surface smaller.

VERIFIED against the diff: alongside the checks.yml `--locked` change, the PR adds `pub mod namespace;`
(lib.rs:50) + a whole new namespace.rs — a SECURITY-relevant module ("mutable-name namespaces — the
WRITE-AUTHORITY half of the global store's anti-hijack model", system/… trust root, fail-closed unknown
prefix). That's a substantial public API + security-model surface riding under a CI-build title. Reviewers
scanning by title would under-scrutinize it, and rollback couples the build fix to the kernel change. Per
the operator's meaningful-MR / coherent-unit steer, recommend SPLITTING the namespace module into its own
PR (or at minimum retitling to disclose the kernel API addition). MED/process — worth flagging given the
security relevance. Routed to v-agent-harness (kernel owner).

## 2. Crate module-map doc out of sync — no `namespace` bullet (Copilot, lib.rs:50) — doc
> `cdz-kernel`'s top-level "v0.1 module map" doesn't mention the new `namespace` module, so the crate docs
> are out of sync with the public module surface.

Add a `namespace` bullet to the module-map doc. LOW/doc.

## 3. checks.yml comment stale after --locked (Copilot, checks.yml:258)  — doc
> The reducer-guest rebuild step now uses `cargo build --locked` with a committed Cargo.lock, so the
> comment about transitive deps "aren't lock-pinned" is no longer accurate.

Update the comment to reflect the now-lock-pinned build. LOW/doc.

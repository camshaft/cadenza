# PR #1768 review comments — cdz-kernel/src/blob.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1768 (MERGED — titled "docs(design): global-store signing
proposal"). Same scope-creep pattern as #1747.

## 1. Scope-creep: a "docs(design)" PR makes BlobStore async — a functional kernel API change (Copilot, blob.rs:41) — process [VERIFIED]
> The PR title/description indicate a design-doc proposal, but this hunk makes a functional API change
> (BlobStore becomes async) affecting kernel callers and implementations. Split the runtime changes or
> update the title.

VERIFIED: BlobStore is now `#[async_trait::async_trait(?Send)]` with `async fn put/get/has` — a functional
trait-API change affecting every caller + impl (MemBlobStore/DiskBlobStore/network backends), landed under
a "docs(design): signing proposal" title. Same reviewability/rollback concern as the #1747 namespace
scope-creep. Retitle to disclose the kernel API change (it already merged, so this is a
metadata/changelog-honesty note + a flag that a docs-titled PR carried an API change past review-by-title).
MED/process.

## 2. `has` documented "cheap existence check" but default impl fetches the full blob (Copilot, blob.rs:55) — doc/perf [VERIFIED]
> `BlobStore::has` is documented as a "cheap existence check", but the default implementation calls `get`
> (which fetches the full blob and may be expensive).

VERIFIED: `has` default is `Ok(self.get(hash).await?.is_some())` — fetches the FULL blob, contradicting the
"Cheap existence check (a disk backend stats the file)" doc. A backend that doesn't override `has` pays the
full-fetch cost. Either drop the "cheap" claim from the default's doc (note it's only cheap when overridden)
or make the default a real existence probe where possible. LOW/perf-contract. Fix-forward.

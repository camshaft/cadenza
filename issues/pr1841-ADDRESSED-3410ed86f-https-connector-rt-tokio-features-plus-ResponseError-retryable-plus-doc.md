# PR #1841 review comments — cdz-agent-host/src/model.rs + Cargo.toml (v-agent-harness-host) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1841 (MERGED — live-net Bedrock model transport).

## 1. `default-features = false` may leave the AWS SDK without HTTPS connector / Tokio runtime → .send() breaks under live-net (Copilot, Cargo.toml:70) — correctness/build [substantive]
> With `default-features = false`, the AWS crates won't include the default HTTPS client + Tokio runtime
> wiring the SDK expects. aws-sdk-bedrockruntime only enables `rustls`, which can leave the client without
> the required runtime/HTTP connector for `.send().await` under live-net. Also the comment says the default
> chain includes profile/IMDS, but those are feature-gated in aws-config and won't be present unless enabled.
The live-net Bedrock client may fail at runtime (`.send()`) if default-features=false drops the
connector/runtime the SDK needs. Since live-net is manual/nightly-gated it wouldn't red the default CI, so
it could lurk until someone enables live-net. RECOMMEND v-agent-harness-host verify the enabled feature set
actually gives a working connector + runtime (add the needed features, e.g. the SDK's rustls+runtime
combo), and correct the profile/IMDS comment to match what's feature-enabled. MED (latent live-net runtime
break). Fix-forward.

## 2. classify_bedrock_error omits SdkError::ResponseError as transient (Copilot, model.rs:210) — robustness
> classify_bedrock_error treats only TimeoutError + DispatchFailure as transient; SdkError::ResponseError(_)
> [and other transient variants] may also be retryable.
Review the AWS SDK v1 SdkError variants and classify the genuinely-transient ones (ResponseError, 5xx
ServiceError) as retryable, so live-net retries don't miss recoverable failures. LOW-MED. Fix-forward.

## 3. Cargo.toml comment: "pinned to the crate's own (gitignored) Cargo.lock" (Copilot, Cargo.toml:68) — doc
> The comment claims the live-net build is pinned to the crate's own gitignored Cargo.lock, but [the
> workspace's Cargo.lock is the effective one].
Reconcile the comment with the actual lock resolution (workspace vs crate-local). LOW/doc.

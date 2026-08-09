//! LIVE integration tests for the real network transports — the ONLY tests that touch the network.
//!
//! Doubly gated so they NEVER run in the hermetic default gate:
//! - `#![cfg(feature = "live-net")]` — the whole file compiles only under `live-net`, so a plain
//!   `cargo test` (the default CI gate) doesn't even build it (no network types, no reqwest calls).
//! - an ENV-VAR guard per test — even under `--features live-net`, a test SKIPS (returns cleanly, does
//!   not fail) unless its env var is set. So `cargo test --features live-net` on a runner WITHOUT egress
//!   still passes; only a human/nightly job that sets the var actually hits the network.
//!
//! This mirrors the `cedar_authz_e2e` / `CEDAR_POLICY_COMPONENT` discipline: the real path is EXERCISABLE
//! where egress + creds exist, and INVISIBLE (skipped) everywhere else. It proves the real transports wire
//! up + perform a request end-to-end — the thing the hermetic stub tests can't.
//!
//! Run it for real:
//!   CDZ_LIVE_HTTP_URL=https://example.com cargo test --features live-net --test live_transport_e2e
//!   CDZ_LIVE_BEDROCK_MODEL_ID=... AWS creds in env; cargo test --features live-net --test live_transport_e2e
#![cfg(feature = "live-net")]

use cdz_agent_host::{HttpMethod, HttpTransport, ModelTransport};
use cdz_kernel::hash::Hash;
use std::time::Duration;

/// A hard ceiling on any single live call. reqwest has no default OVERALL request timeout and a Bedrock
/// invoke is likewise unbounded, so an opt-in run against a stalled endpoint could HANG a nightly/CI job
/// forever. Every live call below is wrapped in `tokio::time::timeout(LIVE_CALL_TIMEOUT, …)` so a stall
/// fails the test with a clear message instead of wedging.
const LIVE_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The real reqwest HTTP client performs a GET against a caller-supplied URL and returns a real response.
/// Skips unless `CDZ_LIVE_HTTP_URL` is set (so `--features live-net` without egress passes cleanly).
#[tokio::test]
async fn a_real_http_get_returns_a_live_response() {
    let Ok(url) = std::env::var("CDZ_LIVE_HTTP_URL") else {
        eprintln!(
            "SKIP live_transport_e2e::a_real_http_get_returns_a_live_response: CDZ_LIVE_HTTP_URL unset — \
             set it to a reachable URL to exercise the real reqwest transport (this test needs network \
             egress)."
        );
        return;
    };

    let transport = cdz_agent_host::ReqwestHttpTransport::new()
        .expect("the reqwest client builds (a TLS backend is present)");
    let resp = tokio::time::timeout(
        LIVE_CALL_TIMEOUT,
        transport.request(HttpMethod::Get, &url, &[], None, Hash::of(b"live-http-e2e")),
    )
    .await
    .expect("the live GET completes within LIVE_CALL_TIMEOUT (not a stalled endpoint)")
    .expect("the live GET completes at the transport level");

    // A real endpoint answered: a sane status, and the response is well-formed (headers + body decoded).
    // We don't assert an exact status/body (the URL is caller-chosen) — only that the transport produced a
    // complete, structurally-valid HttpResponse, which is what "the real path works" means.
    assert!(
        (100..=599).contains(&resp.status),
        "a live HTTP status is in the valid range, got {}",
        resp.status
    );
    eprintln!(
        "live HTTP GET {url} -> status {}, {} header(s), {} body byte(s)",
        resp.status,
        resp.headers.len(),
        resp.body.len()
    );
}

/// The real reqwest client does NOT auto-follow redirects (SEC-F1): a URL known to 3xx surfaces the 3xx
/// verbatim rather than fetching the redirect target. Skips unless `CDZ_LIVE_HTTP_REDIRECT_URL` is set to a
/// URL that responds with a redirect (e.g. an `https://…/redirect-to?url=…` endpoint).
#[tokio::test]
async fn a_real_redirect_is_not_followed() {
    let Ok(url) = std::env::var("CDZ_LIVE_HTTP_REDIRECT_URL") else {
        eprintln!(
            "SKIP live_transport_e2e::a_real_redirect_is_not_followed: CDZ_LIVE_HTTP_REDIRECT_URL unset — \
             set it to a URL that returns a 3xx to verify redirects are surfaced, not followed (SEC-F1)."
        );
        return;
    };

    let transport = cdz_agent_host::ReqwestHttpTransport::new().expect("the reqwest client builds");
    let resp = tokio::time::timeout(
        LIVE_CALL_TIMEOUT,
        transport.request(
            HttpMethod::Get,
            &url,
            &[],
            None,
            Hash::of(b"live-redirect-e2e"),
        ),
    )
    .await
    .expect("the live GET to the redirecting URL completes within LIVE_CALL_TIMEOUT")
    .expect("the live GET to the redirecting URL completes");

    // The whole point of Policy::none(): a redirect status is RETURNED, not transparently followed to a
    // 2xx at the target. If reqwest had followed it, we'd see the target's (likely 200) status instead.
    assert!(
        (300..=399).contains(&resp.status),
        "a redirecting endpoint surfaces its 3xx (SEC-F1: not auto-followed), got {}",
        resp.status
    );
}

/// The real Bedrock transport invokes a model and returns a completion. The test RUNS (does not skip)
/// once `CDZ_LIVE_BEDROCK_MODEL_ID` is set — that env var alone is the opt-in gate. AWS credentials +
/// region are ADDITIONALLY required for it to PASS (they come from the ambient environment via the SDK's
/// default provider); if the var is set but creds/region are absent, the invoke fails and the test panics
/// with a "check the env is complete" message rather than skipping. The request body is a minimal
/// Anthropic Messages payload.
#[tokio::test]
async fn a_real_bedrock_invoke_returns_a_completion() {
    let Ok(model_id) = std::env::var("CDZ_LIVE_BEDROCK_MODEL_ID") else {
        eprintln!(
            "SKIP live_transport_e2e::a_real_bedrock_invoke_returns_a_completion: CDZ_LIVE_BEDROCK_MODEL_ID \
             unset — set it (with AWS creds + region also in the environment) to exercise a real Bedrock \
             InvokeModel."
        );
        return;
    };

    // A minimal Anthropic Messages request body (the native schema Bedrock's Anthropic models expect). The
    // model id decides the schema; this is the common case for the crate's headline (an agent's model call).
    let body = br#"{"anthropic_version":"bedrock-2023-05-31","max_tokens":16,"messages":[{"role":"user","content":"ping"}]}"#;

    // Construction ALSO probes the environment (the SDK loads config — credential/region resolution can
    // stall), so it's inside the timeout too: the "hard ceiling on any single live call" must cover the
    // whole path, not just invoke() (PR #1857 review — new() was left uncovered).
    let transport = tokio::time::timeout(
        LIVE_CALL_TIMEOUT,
        cdz_agent_host::BedrockModelTransport::new(),
    )
    .await
    .expect("building the Bedrock transport (AWS config load) completes within LIVE_CALL_TIMEOUT");
    let outcome = tokio::time::timeout(
        LIVE_CALL_TIMEOUT,
        transport.invoke(&model_id, body, Hash::of(b"live-bedrock-e2e")),
    )
    .await
    .expect("the live Bedrock invoke completes within LIVE_CALL_TIMEOUT (not a stalled endpoint)");
    match outcome {
        Ok(completion) => {
            assert!(
                !completion.is_empty(),
                "a real Bedrock completion has a non-empty body"
            );
            eprintln!(
                "live Bedrock invoke {model_id} -> {} completion byte(s)",
                completion.len()
            );
        }
        Err(reason) => panic!(
            "live Bedrock invoke failed (creds/region/model-id/schema?): {reason:?}\n\
             (this test only runs when CDZ_LIVE_BEDROCK_MODEL_ID is set — check the env is complete)"
        ),
    }
}

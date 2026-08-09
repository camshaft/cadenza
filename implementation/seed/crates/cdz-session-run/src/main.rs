//! `cdz-session-run` — drive a Cadenza reducer component through the real `cdz-kernel` `Session` and
//! print its end-state. The DRIVE half of the platform-conformance suite; xtask's `run_platform_case`
//! owns compile + parse + grade (see the crate `Cargo.toml` header for the ownership split).
//!
//! I1 SCOPE: a SINGLE session, a SINGLE kick-off inbound event, driven to quiescence. No cross-session
//! messaging, no effect-handler sessions (those are I2/I3, which add `cdz-agent-host` + a deterministic
//! FIFO fixpoint here). So I1 drives `Session` directly, exactly like the reference
//! `cdz-kernel/src/kernel_e2e_tests/loop_and_recovery.rs`.
//!
//! ## Invocation (called by xtask, one process per case)
//! ```text
//! cdz-session-run --reducer <component.wasm> --store <dir> \
//!     --alias worker --kickoff-family start --kickoff-value ""
//! ```
//! `--reducer` is the ALREADY-COMPILED component bytes (xtask compiled the `(reducer <prog>)` via the
//! cdz-syntax|rcdzc pipeline). `--store` is the content-addressed store (`<hash>.wasm`) the reducer's
//! `cadenza:runtime/heap` dep resolves from — the same `target/cadenza-store` `cargo xtask build`
//! populates. The kick-off is one inbound event of the given family carrying the given payload bytes.
//!
//! ## Output (tab-delimited lines on stdout, for xtask to parse + grade)
//! ```text
//! end-status\t<alias>\t<state>          state in active|quiescent|stalled|closed
//! events-processed\t<alias>\t<n>
//! end-kv\t<alias>\t<key-utf8-or-hex>\t<value-hex>
//! ```
//! KV keys/values are OPAQUE bytes at the kernel boundary, so they are printed as-is (key as UTF-8 when
//! it is valid UTF-8 else `hex:<hex>`; value always `hex:<hex>`). xtask decodes/compares against the
//! case's `(: v T)` value-form. A hard error (bad component, unresolved dep, fold failure) exits non-zero
//! with the reason on stderr so xtask grades the case a Fail rather than a false Pass.

use anyhow::{Context, Result};
use clap::Parser;

use cdz_kernel::authz::Authorizer;
use cdz_kernel::component_store::ComponentStore;
use cdz_kernel::effect::Payload;
use cdz_kernel::event::{ContentType, EventBody};
use cdz_kernel::executor::RecordingExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::{Session, SessionState};
use cdz_kernel::wasm_host::ComponentReducer;

#[derive(Parser)]
#[command(
    name = "cdz-session-run",
    about = "Drive a Cadenza reducer component through the kernel Session and print its end-state (platform-conformance I1)."
)]
struct Args {
    /// Path to the already-compiled reducer wasm component.
    #[arg(long)]
    reducer: std::path::PathBuf,
    /// Content-addressed store dir the reducer's `cadenza:runtime/heap` dep resolves from
    /// (`<hash>.wasm` naming — the `target/cadenza-store` `cargo xtask build` populates).
    #[arg(long)]
    store: std::path::PathBuf,
    /// The session's alias (echoed into the printed end-state lines so xtask keys them per-session).
    #[arg(long)]
    alias: String,
    /// The kick-off inbound event's content-type family (e.g. `start`, `message`).
    #[arg(long)]
    kickoff_family: String,
    /// The kick-off inbound payload as raw UTF-8 bytes (empty for a payload-less stimulus). I1 keeps the
    /// payload a plain string; a later increment can carry a typed value-form.
    #[arg(long, default_value = "")]
    kickoff_value: String,
    /// Deterministic-id salt: the session's genesis nonce is `Hash::of(salt ++ alias)`, so ids are a pure
    /// function of the case (D5.3) — never OS entropy. The reducer-content hash is the genesis `reducer`.
    #[arg(long, default_value = "platform-conformance")]
    salt: String,
    /// Stall threshold (ms) for the terminal status snapshot. I1 has no in-flight effects, so any value
    /// yields Quiescent; kept configurable for later increments. `0` = no clock (Active/Quiescent/Closed).
    #[arg(long, default_value_t = 0)]
    stall_after_ms: u64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load + parse the already-compiled reducer component.
    let component = std::fs::read(&args.reducer)
        .with_context(|| format!("reading reducer component {}", args.reducer.display()))?;
    let reducer = ComponentReducer::from_component_bytes(&component)
        .map_err(|e| anyhow::anyhow!("reducer is not a valid component: {e:?}"))?;

    // Resolve every declared dep (the `cadenza:runtime/heap` a real Cadenza reducer imports) from the
    // content-addressed store, then attach the store so the runtime's transitive bare imports (e.g.
    // `cadenza:nfc/normalize`) compose too — mirroring `reducer_cadenza_b1_e2e.rs`'s resolve path.
    let store = ComponentStore::open(&args.store);
    let deps = reducer.deps().to_vec();
    let mut resolved = Vec::with_capacity(deps.len());
    for dep in &deps {
        let bytes = store.get_by_hash(&dep.hash).map_err(|e| {
            anyhow::anyhow!(
                "store {} has no valid blob for dep {:?} (hash {}): {e:?}",
                args.store.display(),
                dep.import_name,
                dep.hash.to_hex()
            )
        })?;
        resolved.push((dep.clone(), bytes));
    }
    let mut reducer = reducer
        .with_resolved_deps(resolved)
        .with_component_store(ComponentStore::open(&args.store));

    // Genesis with a DETERMINISTIC nonce (D5.3): id = pure function of (salt, alias), not OS entropy, so
    // the run is identical every time and in CI. The reducer-content hash seeds the genesis `reducer`.
    let reducer_hash = Hash::of(&component);
    let mut nonce_seed = args.salt.clone().into_bytes();
    nonce_seed.extend_from_slice(args.alias.as_bytes());
    let spawn_nonce = Hash::of(&nonce_seed);
    let mut session = Session::genesis(reducer_hash, spawn_nonce);

    // Deliver the ONE kick-off event and drive it to quiescence. I1 has no effects, so `deny_all()` is
    // the correct authorizer (a pure fold needs no ambient authority) and a `RecordingExecutor` observes
    // that zero effects were dispatched. `deliver` folds reactively until the session settles (§9d).
    let authz = Authorizer::deny_all();
    let mut exec = RecordingExecutor::new();
    let body = EventBody::Inbound {
        content_type: ContentType {
            family: args.kickoff_family.clone().into(),
            version: 1,
        },
        payload: Payload::Inline(args.kickoff_value.clone().into_bytes().into()),
    };
    session
        .deliver(body, None, &mut reducer, &authz, &mut exec)
        .await
        .map_err(|e| anyhow::anyhow!("kernel deliver failed: {e:?}"))?;

    // Report the terminal end-state as tab lines for xtask to grade.
    let mut out = String::new();
    let snap = session.status_snapshot(None, args.stall_after_ms);
    out.push_str(&format!(
        "end-status\t{}\t{}\n",
        args.alias,
        state_str(snap.state)
    ));
    out.push_str(&format!(
        "events-processed\t{}\t{}\n",
        args.alias,
        session.event_count()
    ));
    for (key, value) in session.kv().prefix_scan(b"") {
        out.push_str(&format!(
            "end-kv\t{}\t{}\t{}\n",
            args.alias,
            render_key(key),
            render_value(value)
        ));
    }
    print!("{out}");
    Ok(())
}

/// The session state as the lowercase token the case's `(status …)` clause uses.
fn state_str(s: SessionState) -> &'static str {
    match s {
        SessionState::Active => "active",
        SessionState::Quiescent => "quiescent",
        SessionState::Stalled => "stalled",
        SessionState::Closed => "closed",
    }
}

/// A KV key: UTF-8 verbatim when valid (the common case — reducers key by string), else `hex:<hex>`.
fn render_key(key: &[u8]) -> String {
    match std::str::from_utf8(key) {
        Ok(s) if !s.contains('\t') && !s.contains('\n') => s.to_string(),
        _ => format!("hex:{}", to_hex(key)),
    }
}

/// A KV value: always `hex:<hex>` — values are opaque bytes and may not be UTF-8, and xtask decodes them
/// against the case's typed `(: v T)` value-form, so a stable hex encoding is the unambiguous wire.
fn render_value(value: &[u8]) -> String {
    format!("hex:{}", to_hex(value))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

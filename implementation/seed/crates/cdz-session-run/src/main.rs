//! `cdz-session-run` — drive a constellation of Cadenza reducer SESSIONS through the real
//! `cdz-kernel`/`cdz-agent-host` machinery from ONE kick-off event to a fixpoint, and print the observed
//! effects plus per-session end-state. The DRIVE half of the platform-conformance suite; xtask's
//! `run_platform_case` owns compile + parse + grade (see the crate `Cargo.toml` header for the split).
//!
//! ## Model (I1 single-session → I2 handler-sessions + fixpoint)
//! Each `--session <alias>=<component>` is a Cadenza reducer already compiled to the
//! `cadenza:agent-kernel/fold` world. A `--serves <alias>=<family>` binds that session as the HANDLER for
//! an effect family: when another session performs an effect of that family, the kernel DEFERS it (via a
//! `UserspaceEffectExecutor` registered as the caller's `CompositeExecutor` fallback), the runner forwards
//! it as an `effect-request/<family>` inbound to the handler session, the handler folds it and emits
//! `effect/reply`, and a `ReplyExecutor` settles the reply onto the caller's open effect — the real
//! in-process round-trip (the SAME machinery the OUTPOST federates over the wire).
//!
//! ## Determinism (D5): a deterministic FIFO fixpoint, NOT the production `select!` loop
//! The runner does NOT use `AsyncAgentHost::run` (it multiplexes with tokio `select!`, which has NO
//! ordering guarantee — unusable for a reproducible gate). It drives `AgentHost::deliver` itself over a
//! single in-memory FIFO: deliver the one kick-off, then drain (in arrival order) every forwarded
//! effect-request → deliver to its handler, and every reply-settle → resume its caller, until the queues
//! are empty and every session is quiescent. Session ids are deterministic (`Hash::of(salt ++ alias)`,
//! never OS entropy), so a run is identical every time and in CI. A per-case STEP BUDGET bounds the drive:
//! an unbounded effect/reply ping-pong is a recorded `SettleUnbounded` fault (exit non-zero), never a hang.
//!
//! ## Output (tab-delimited lines on stdout, for xtask to parse + grade)
//! ```text
//! effect\t<from-alias>\t<family>[\t<payload-hex>]   one per dispatched effect, in whole-run order
//! end-fault\t<alias>\t<reason>                       present ONLY when a fold trapped/failed (grades Fail)
//! end-status\t<alias>\t<state>                       state in active|quiescent|stalled|closed
//! events-processed\t<alias>\t<n>
//! end-kv\t<alias>\t<key-utf8-or-hex>\t<value-hex>
//! ```
//! KV/effect payloads are OPAQUE bytes at the kernel boundary, printed as `hex:<hex>` (a KV key is UTF-8
//! when valid else `hex:<hex>`); xtask decodes/compares against the case's `(: v T)` value-form. A hard
//! error (bad component, unresolved dep, unbounded drive) exits non-zero with the reason on stderr.

use anyhow::{Context, Result};
use clap::Parser;

use cdz_agent_host::effect_reply::ReplyTokenRegistry;
use cdz_agent_host::host::{AgentHost, HostedSession, SessionId};
use cdz_agent_host::reply_exec::{reply_settle_channel, ReplyExecutor};
use cdz_agent_host::userspace_effect_exec::{HandlerResolver, UserspaceEffectExecutor};
use cdz_kernel::component_store::ComponentStore;
use cdz_kernel::effect::{effect_ct, Payload};
use cdz_kernel::event::{ContentType, EventBody};
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::SessionState;
use cdz_kernel::wasm_host::AsyncComponentReducer;
use std::collections::HashMap;
use std::rc::Rc;

/// The `effect-request/<family>` content-type prefix a `UserspaceEffectExecutor` forwards under. The
/// runner strips it to recover the bare family for the observed-effect line + reads the framed payload.
const EFFECT_REQUEST_PREFIX: &str = "effect-request/";

#[derive(Parser)]
#[command(
    name = "cdz-session-run",
    about = "Drive Cadenza reducer sessions through the kernel to a fixpoint and print observed effects + end-state (platform-conformance)."
)]
struct Args {
    /// A session: `<alias>=<path-to-compiled-component>`. Repeatable; at least one. The FIRST is not
    /// special — the kick-off names its target alias.
    #[arg(long = "session", value_name = "ALIAS=PATH")]
    sessions: Vec<String>,
    /// A handler binding: `<alias>=<family>` — bind session `<alias>` as the handler for effect `<family>`.
    /// Repeatable; zero or more. A session may serve several families (repeat the flag).
    #[arg(long = "serves", value_name = "ALIAS=FAMILY")]
    serves: Vec<String>,
    /// Content-addressed store dir the reducers' `cadenza:runtime/heap` dep resolves from.
    #[arg(long)]
    store: std::path::PathBuf,
    /// The kick-off target session alias.
    #[arg(long)]
    kickoff_alias: String,
    /// The kick-off inbound event's content-type family (e.g. `start`, `message`).
    #[arg(long)]
    kickoff_family: String,
    /// The kick-off inbound payload as raw UTF-8 bytes (empty for a payload-less stimulus).
    #[arg(long, default_value = "")]
    kickoff_value: String,
    /// Deterministic-id salt: each session's genesis nonce is `Hash::of(salt ++ alias)` (D5.3).
    #[arg(long, default_value = "platform-conformance")]
    salt: String,
    /// Per-case step budget (total deliveries in the fixpoint drive) — exceeding it is a `SettleUnbounded`
    /// fault (exit non-zero), so an unbounded effect/reply ping-pong is a graded failure, never a hang.
    #[arg(long, default_value_t = 1000)]
    step_budget: u32,
    /// Stall threshold (ms) for the terminal status snapshot. `0` = no clock (Active/Quiescent/Closed).
    #[arg(long, default_value_t = 0)]
    stall_after_ms: u64,
}

/// A family→handler-SessionId resolver built from the case's `serves` map — the `HandlerResolver` the
/// caller's `UserspaceEffectExecutor` consults to route a deferred effect to its handler session.
struct MapResolver(HashMap<String, SessionId>);
impl HandlerResolver for MapResolver {
    fn resolve_handler(&self, family: &str) -> Option<SessionId> {
        self.0.get(family).copied()
    }
}

/// The conformance-suite authorizer: PERMISSIVE by design (the suite tests platform SEMANTICS, not authz
/// policy — a later increment can add a policy-testing genre). It admits:
///  - `effect/reply` UNCONDITIONALLY, regardless of target. A handler settles a caller by echoing the raw
///    32-byte reply-token as the effect's `target`; that token is a non-UTF-8 opaque byte string, and the
///    host `ReplyExecutor` CRYPTOGRAPHICALLY validates+consumes it (unforgeable, one-shot) — strictly
///    stronger than a capability grant, so gating the target is both impossible (a `FamilyGrant` requires a
///    UTF-8 target — `target_str().is_ok`) and redundant. This mirrors the intended kernel end-state where
///    `effect/reply` is authz-exempt like `control/*` (the token IS the security — v-agent-harness-host,
///    design-userspace-effects D2); the kernel exemption is v-agent-harness's seam, so the suite grants it
///    here in the meantime (no rework needed when the kernel formally exempts it).
///  - any OTHER family — the suite lets a reducer perform whatever effect it declares (conformance, not policy).
struct SuiteAuthorizer;

#[async_trait::async_trait(?Send)]
impl cdz_kernel::authz::Authorize for SuiteAuthorizer {
    async fn authorize(&self, _req: &cdz_kernel::effect::EffectRequest) -> Result<(), String> {
        // Permissive: admit everything (incl. effect/reply with its raw-bytes token target). The suite
        // grades platform behavior, not the capability policy; the token-validation in ReplyExecutor is the
        // real security boundary for a reply.
        Ok(())
    }
}

/// Parse an `alias=value` CLI pair (splitting on the FIRST `=`, so a value may contain `=`).
fn split_pair(s: &str) -> Result<(String, String)> {
    let (a, b) = s
        .split_once('=')
        .with_context(|| format!("expected ALIAS=VALUE, got {s:?}"))?;
    Ok((a.to_string(), b.to_string()))
}

/// Load a reducer component, resolve its `cadenza:runtime/heap` dep from the store, and wrap it as an
/// `AsyncComponentReducer` (the handle-ABI path that serves the bound kv — see the I1 lesson).
fn load_reducer(
    path: &std::path::Path,
    store: &ComponentStore,
) -> Result<(AsyncComponentReducer, Hash)> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading reducer {}", path.display()))?;
    let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("{} is not a valid component: {e:?}", path.display()))?;
    let deps = reducer.deps().to_vec();
    let mut resolved = Vec::with_capacity(deps.len());
    for dep in &deps {
        let b = store.get_by_hash(&dep.hash).map_err(|e| {
            anyhow::anyhow!(
                "store has no blob for dep {:?} ({}): {e:?}",
                dep.import_name,
                dep.hash.to_hex()
            )
        })?;
        resolved.push((dep.clone(), b));
    }
    let reducer = reducer
        .with_resolved_deps(resolved)
        .with_component_store(store.clone());
    Ok((reducer, Hash::of(&bytes)))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let store = ComponentStore::open(&args.store);

    // Deterministic alias → SessionId (id = Hash::of(salt ++ alias), D5.3) + the reverse map for labelling
    // observed effects/messages by the human alias rather than a genesis-hash hex.
    let mut alias_of: HashMap<SessionId, String> = HashMap::new();
    let mut id_of: HashMap<String, SessionId> = HashMap::new();
    let mut session_specs: Vec<(String, std::path::PathBuf)> = Vec::new();
    for s in &args.sessions {
        let (alias, path) = split_pair(s)?;
        let mut seed = args.salt.clone().into_bytes();
        seed.extend_from_slice(alias.as_bytes());
        let id = SessionId::new(Hash::of(&seed));
        alias_of.insert(id, alias.clone());
        id_of.insert(alias.clone(), id);
        session_specs.push((alias, path.into()));
    }

    // The serves map: family → handler SessionId (resolved through the alias). Shared by every caller's
    // resolver (a MapResolver is cloned-by-rebuild per session below, all from this table).
    let mut family_handler: HashMap<String, SessionId> = HashMap::new();
    for sv in &args.serves {
        let (alias, family) = split_pair(sv)?;
        let id = *id_of
            .get(&alias)
            .with_context(|| format!("serves names unknown session alias {alias:?}"))?;
        family_handler.insert(family, id);
    }

    // The shared collaborators the round-trip binds through: ONE reply-token registry (minted by each
    // caller's UserspaceEffectExecutor forward, consumed by each handler's ReplyExecutor), ONE forwarded-
    // request inbox, ONE reply-settle channel — exactly the loop's shape, drained by the FIFO drive below.
    let reply_tokens = Rc::new(ReplyTokenRegistry::new());
    let (inbox_tx, mut inbox_rx) = tokio::sync::mpsc::unbounded_channel();
    let (settle_tx, mut settle_rx) = reply_settle_channel();

    // Build the host + spawn every session with its own CompositeExecutor:
    //  - EVERY session gets a ReplyExecutor (family effect/reply) so it can REPLY when it serves an effect,
    //    and a UserspaceEffectExecutor fallback so it can PERFORM an effect served by a peer. A session that
    //    does neither simply never exercises them. This uniform wiring keeps the constellation symmetric.
    let mut host = AgentHost::new();
    for (alias, path) in &session_specs {
        let (reducer, reducer_hash) = load_reducer(path, &store)?;
        let owner = id_of[alias];
        let nonce = owner.hash(); // the nonce IS Hash::of(salt++alias); the id is its genesis hash
                                  // The suite authorizer is permissive (conformance, not policy) + admits effect/reply's raw-bytes
                                  // token target (see SuiteAuthorizer). Under B2 a reducer emits an arbitrary `kind` STRING, so a
                                  // served family is register-by-string; there is no capability-shape distinction to make here.
        let authz = SuiteAuthorizer;
        let executor = cdz_kernel::executor::CompositeExecutor::new()
            .with_effect(
                effect_ct::EFFECT_REPLY,
                Box::new(ReplyExecutor::new(reply_tokens.clone(), settle_tx.clone())),
            )
            .with_fallback(Box::new(UserspaceEffectExecutor::new(
                MapResolver(family_handler.clone()),
                inbox_tx.clone(),
                reply_tokens.clone(),
                owner,
            )));
        let hosted = HostedSession::genesis_with_nonce(
            reducer_hash,
            nonce,
            Box::new(reducer),
            Box::new(authz),
            executor,
        );
        host.spawn(owner, hosted);
    }

    // Observed effects, in whole-run dispatch order (each forwarded effect-request the drive routes).
    let mut observed_effects: Vec<(String, String, Option<Vec<u8>>)> = Vec::new();
    let mut steps: u32 = 0;
    let mut budget_exceeded = false;

    // Deliver the ONE kick-off, then drive the FIFO to a fixpoint.
    let kickoff_target = *id_of.get(&args.kickoff_alias).with_context(|| {
        format!(
            "kickoff names unknown session alias {:?}",
            args.kickoff_alias
        )
    })?;
    let kickoff_body = EventBody::Inbound {
        content_type: ContentType {
            family: args.kickoff_family.clone().into(),
            version: 1,
        },
        payload: Payload::Inline(args.kickoff_value.clone().into_bytes().into()),
    };
    host.deliver(&kickoff_target, kickoff_body, None).await;
    steps += 1;

    // FIFO breadth-first: drain forwarded effect-requests (→ deliver to handler) and reply-settles (→ resume
    // caller) in arrival order until both queues are empty and nothing new is produced. Each drained item is
    // one delivery against the step budget.
    loop {
        // A forwarded effect-request: route it to its handler session (and record the observed effect).
        if let Ok(inbound) = inbox_rx.try_recv() {
            if steps >= args.step_budget {
                budget_exceeded = true;
                break;
            }
            steps += 1;
            // Record the observed effect: the FROM is the reply_to (caller) alias; the FAMILY strips the
            // `effect-request/` prefix; the payload is the request bytes AFTER the framing header.
            if let EventBody::Inbound {
                content_type,
                payload: Payload::Inline(bytes),
            } = &inbound.body
            {
                let family = content_type
                    .family
                    .strip_prefix(EFFECT_REQUEST_PREFIX)
                    .unwrap_or(&content_type.family)
                    .to_string();
                let from = inbound
                    .reply_to
                    .and_then(|c| alias_of.get(&c).cloned())
                    .unwrap_or_else(|| "?".to_string());
                let payload = strip_framing_payload(bytes);
                observed_effects.push((from, family, payload));
            }
            host.deliver(&inbound.session, inbound.body, inbound.cause)
                .await;
            continue;
        }
        // A reply-settle: resume the caller's open effect with the handler's reply outcome.
        if let Ok(settle) = settle_rx.try_recv() {
            if steps >= args.step_budget {
                budget_exceeded = true;
                break;
            }
            steps += 1;
            // Settle the caller's open (Deferred) effect with the handler's reply outcome — resumes its
            // continuation. A no-op (caller gone / already settled) is benign; the caller then simply never
            // resumes, which a case's `(end-state …)` on the caller would catch.
            host.settle_reply(&settle.caller, settle.effect_id, settle.outcome)
                .await;
            continue;
        }
        break; // both queues empty → fixpoint reached
    }

    // Report.
    let mut out = String::new();
    for (from, family, payload) in &observed_effects {
        match payload {
            Some(p) if !p.is_empty() => {
                out.push_str(&format!("effect\t{from}\t{family}\thex:{}\n", to_hex(p)));
            }
            _ => out.push_str(&format!("effect\t{from}\t{family}\n")),
        }
    }
    // Per-session end-state, in the case's declaration order (stable, deterministic).
    for (alias, _) in &session_specs {
        let id = id_of[alias];
        let Some(hosted) = host.get(&id) else {
            continue;
        };
        let session = hosted.session();
        if let Some(reason) = session.last_fault_reason() {
            out.push_str(&format!(
                "end-fault\t{alias}\t{}\n",
                reason.replace(['\t', '\n'], " ")
            ));
        }
        let snap = session.status_snapshot(None, args.stall_after_ms);
        out.push_str(&format!("end-status\t{alias}\t{}\n", state_str(snap.state)));
        out.push_str(&format!(
            "events-processed\t{alias}\t{}\n",
            session.event_count()
        ));
        // KV-Bytes (operator seq364): prefix_scan now yields `(Bytes, Bytes)` (Arc-backed, cheap clone).
        // Take a `&[u8]` view via `as_ref()` — the render fns are byte-oriented (opaque KV wire).
        for (key, value) in session.kv().prefix_scan(b"") {
            out.push_str(&format!(
                "end-kv\t{alias}\t{}\thex:{}\n",
                render_key(key.as_ref()),
                to_hex(value.as_ref())
            ));
        }
    }
    print!("{out}");
    if budget_exceeded {
        anyhow::bail!(
            "SettleUnbounded: the fixpoint drive exceeded the step budget of {} deliveries (an unbounded \
             effect/reply ping-pong) — raise --step-budget only if the case is genuinely that long",
            args.step_budget
        );
    }
    Ok(())
}

/// Recover the opaque request payload from a forwarded effect-request's framed bytes:
/// `[caller_len u32][caller][token_len u32][token][effect_id u64][payload]` (see
/// `userspace_effect_exec::effect_request_inbound`). Returns the trailing payload, or `None` if the header
/// is malformed (the observed-effect line then carries no payload).
fn strip_framing_payload(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut off = 0usize;
    let take_u32 = |b: &[u8], off: &mut usize| -> Option<usize> {
        let v = b.get(*off..*off + 4)?;
        *off += 4;
        Some(u32::from_le_bytes(v.try_into().ok()?) as usize)
    };
    let caller_len = take_u32(bytes, &mut off)?;
    off += caller_len;
    let token_len = take_u32(bytes, &mut off)?;
    off += token_len;
    off += 8; // effect_id u64-le
    bytes.get(off..).map(|s| s.to_vec())
}

fn state_str(s: SessionState) -> &'static str {
    match s {
        SessionState::Active => "active",
        SessionState::Quiescent => "quiescent",
        SessionState::Stalled => "stalled",
        SessionState::Closed => "closed",
    }
}

fn render_key(key: &[u8]) -> String {
    match std::str::from_utf8(key) {
        Ok(s) if !s.contains('\t') && !s.contains('\n') => s.to_string(),
        _ => format!("hex:{}", to_hex(key)),
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

//! Async transport helpers for the daemon — the thin layer between the pure lib and slack-morphism /
//! tokio / subprocesses. Kept out of `main.rs` so `main` reads as a wiring diagram. Not unit-tested (live
//! WebSocket + subprocess); the pure decisions it calls into ARE tested in the lib.

use cadenza_slack_bridge::{
    Config, Message, MirroredAsk, RELAY_QUEUE_WARN, RelayPlan, SlackTokens, ThreadMap, deliver,
    drain, mark_failed, mark_processed, parse_operator_message, relay_plan, render_fleet_message,
    render_fleet_message_plain, sidecar, watchdog,
};
use slack_morphism::errors::SlackClientError;
use slack_morphism::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

// ── WATCHDOG ────────────────────────────────────────────────────────────────────────────────────

/// Fire `cargo xtask fleet watchdog` on the configured cadence, forever. Runs regardless of Slack — it's
/// the reliability backbone. `repo_root` is the cwd to spawn in (so `cargo xtask` resolves). Each run's
/// failure is logged and the loop continues; the watchdog tool itself is the source of truth for what to
/// re-arm/reap.
pub async fn watchdog_loop(repo_root: PathBuf) {
    let spec = watchdog::WatchdogSpec::default();
    tracing::info!(
        interval_secs = spec.interval.as_secs(),
        stale_mult = spec.stale_mult,
        "watchdog runner started"
    );
    // Fire once at startup, then every interval (a fresh daemon should heal immediately).
    let mut last = Instant::now()
        .checked_sub(spec.interval)
        .unwrap_or_else(Instant::now);
    loop {
        if watchdog::due(last.elapsed(), spec.interval) {
            run_watchdog_once(&spec, &repo_root).await;
            last = Instant::now();
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

async fn run_watchdog_once(spec: &watchdog::WatchdogSpec, repo_root: &PathBuf) {
    let (prog, args) = spec.command();
    // BOUND the fire with a timeout. A bare `.output().await` has NO deadline, so ONE hung
    // `cargo xtask fleet watchdog` child (e.g. a cargo/git/tmux call wedging, or a shared-registry lock)
    // blocks this await forever → the `watchdog_loop` never comes back around → the whole daemon SILENTLY
    // stops re-arming for as long as that child hangs. That is exactly what produced the ~12h watchdog gap
    // in watchdog.log during the cred-freeze window (the daemon process stayed up per systemd, but its
    // fires halted). Kill-on-timeout so a single hung fire costs one cycle, never the daemon's liveness —
    // the operator's Track-A resilience ask (the fleet must auto-recover, not freeze for hours).
    let mut child = match tokio::process::Command::new(prog)
        .args(&args)
        .current_dir(repo_root)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "failed to spawn watchdog");
            return;
        }
    };

    // Wait for the fire, draining stdout/stderr CONCURRENTLY (what `wait_with_output` does internally) so a
    // chatty child can't fill a pipe buffer and wedge — while keeping `child` in hand so a timeout can
    // kill+reap it explicitly (see the timeout arm). The two pipe reads run alongside `wait()` so none can
    // deadlock waiting on the others.
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let wait_fut = async {
        let read_out = async {
            let mut b = Vec::new();
            if let Some(s) = stdout.as_mut() {
                let _ = s.read_to_end(&mut b).await;
            }
            b
        };
        let read_err = async {
            let mut b = Vec::new();
            if let Some(s) = stderr.as_mut() {
                let _ = s.read_to_end(&mut b).await;
            }
            b
        };
        let (status, (_out, err)) =
            tokio::join!(child.wait(), async { tokio::join!(read_out, read_err) });
        (status, err)
    };

    match tokio::time::timeout(watchdog::FIRE_TIMEOUT, wait_fut).await {
        Ok((Ok(status), _err)) if status.success() => tracing::debug!("watchdog tick ok"),
        Ok((Ok(status), err)) => tracing::warn!(
            code = ?status.code(),
            stderr = %String::from_utf8_lossy(&err),
            "watchdog tick returned non-zero"
        ),
        Ok((Err(e), _err)) => tracing::warn!(error = %e, "watchdog child wait failed"),
        // Timed out. Explicitly `kill().await` — which SIGKILLs AND reaps (waitpid) — under a bounded grace,
        // rather than relying on `kill_on_drop`, whose reap tokio defers to a best-effort background reaper.
        // Under a REPEATEDLY-hung watchdog (the case this timeout exists for) the deferred reap would let
        // zombies/orphans queue up and leak PIDs (PR#949). The loop continues either way so the next fire
        // runs on schedule; a persistently-hanging watchdog surfaces as a repeated timeout warning.
        Err(_) => match tokio::time::timeout(watchdog::REAP_GRACE, child.kill()).await {
            Ok(Ok(())) => tracing::warn!(
                timeout_secs = watchdog::FIRE_TIMEOUT.as_secs(),
                "watchdog fire exceeded its timeout — killed + reaped; daemon continues (a hung fire must not stall re-arms)"
            ),
            Ok(Err(e)) => tracing::warn!(
                error = %e,
                "watchdog fire timed out; explicit kill/reap failed (kill_on_drop will finalize)"
            ),
            Err(_) => tracing::warn!(
                reap_grace_secs = watchdog::REAP_GRACE.as_secs(),
                "watchdog fire timed out; reap did not complete within grace (kill_on_drop will finalize)"
            ),
        },
    }
}

// ── OUTBOUND (fleet → Slack) ──────────────────────────────────────────────────────────────────────

/// Poll loop: (1) mirror new concierge `ask`/`backlog` to the channel as threads; (2) drain the bridge's
/// OWN inbox and post agent→operator messages. Needs a channel to post into — without one, nothing to do.
pub async fn outbound_loop(cfg: Arc<Config>, tokens: SlackTokens) {
    let Some(channel) = cfg.channel.clone() else {
        tracing::warn!(
            "no SLACK_BRIDGE_CHANNEL — outbound mirror/relay disabled (inbound DMs still work)"
        );
        return;
    };
    let client = match hyper_client() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "could not build slack client for outbound");
            return;
        }
    };
    let bot = SlackApiToken::new(tokens.bot_token.clone().into());
    let channel_id: SlackChannelId = channel.into();

    // Per-message CONTENT-failure counts (inbox/mirror key → count) so a deterministically un-postable
    // message escalates degrade→quarantine (relay) / degrade→give-up (mirror) across ticks WITHOUT blocking
    // the queue. In-memory: a restart resets them, which is fine — the message re-drains and re-clears in a
    // bounded number of ticks (never forever, the ~11h wedge this fixes). Live here so they persist across
    // `outbound_tick` calls. `relay_failures` keys the bridge's own inbox; `mirror_failures` keys concierge
    // asks being mirrored.
    let mut relay_failures: HashMap<String, u32> = HashMap::new();
    let mut mirror_failures: HashMap<String, u32> = HashMap::new();

    loop {
        if let Err(e) = outbound_tick(
            &cfg,
            &client,
            &bot,
            &channel_id,
            &mut relay_failures,
            &mut mirror_failures,
        )
        .await
        {
            tracing::warn!(error = %e, "outbound tick error (will retry)");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Classify a Slack post error: CONTENT (the message itself is un-postable — Slack answered with an API
/// error like `internal_error`/`msg_too_long`) vs TRANSIENT (a transport/HTTP/rate-limit fault — retry in
/// place). Only content failures advance a message toward degrade/quarantine; a transient outage retries
/// forever without dead-lettering. `ratelimited` is an API error but is transient (back off + retry). Mirror
/// of the Node `isContentPostError` in format.js.
fn is_content_post_error(e: &SlackClientError) -> bool {
    match e {
        SlackClientError::ApiError(api) => api.code != "ratelimited",
        // Http/HttpProtocol/System/EndOfStream/Protocol/SocketMode/RateLimit → transport/transient.
        _ => false,
    }
}

async fn outbound_tick(
    cfg: &Config,
    client: &SlackHyperClient,
    bot: &SlackApiToken,
    channel: &SlackChannelId,
    relay_failures: &mut HashMap<String, u32>,
    mirror_failures: &mut HashMap<String, u32>,
) -> Result<(), BoxErr> {
    let session = client.open_session(bot);

    // (1) Mirror new concierge asks/backlog. Same head-of-line-block guard as the relay below: a transport/
    // transient fault retries the batch next tick, but a CONTENT-class failure escalates PER-ITEM (degrade to
    // a plain variant, then give up and mark the ask un-mirrorable so it stops being re-selected) and we SKIP
    // PAST it — one poison ask must not block mirroring every later ask.
    let mut map = ThreadMap::load(&cfg.fleet_dir);
    let pending = drain(&cfg.fleet_dir, &cfg.default_to)?; // watch, don't consume: no mark_processed
    let to_post = sidecar::select_to_mirror(&pending, &map);
    for item in to_post {
        let ask = MirroredAsk {
            asker: item.msg.from.clone(),
            kind: item.msg.kind.clone(),
            subject: item.msg.subject.clone(),
        };
        let fails = mirror_failures.get(&item.key).copied().unwrap_or(0);
        let plan = relay_plan(fails);
        if plan == RelayPlan::Quarantine {
            tracing::warn!(
                key = %item.key, asker = %item.msg.from, content_failures = fails,
                "mirror: giving up on un-postable ask — marking unmirrorable so it stops re-selecting; operator still sees it in the concierge tmux"
            );
            map.mark_unmirrorable(item.key.clone(), ask);
            map.save(&cfg.fleet_dir)?;
            mirror_failures.remove(&item.key);
            continue;
        }
        let text = if plan == RelayPlan::Degraded {
            render_fleet_message_plain(&item.msg)
        } else {
            render_fleet_message(&item.msg)
        };
        let req = SlackApiChatPostMessageRequest::new(
            channel.clone(),
            SlackMessageContent::new().with_text(text),
        );
        match session.chat_post_message(&req).await {
            Ok(resp) => {
                map.record(resp.ts.to_string(), item.key.clone(), ask);
                map.save(&cfg.fleet_dir)?;
                mirror_failures.remove(&item.key);
                tracing::info!(asker = %item.msg.from, kind = %item.msg.kind, "mirrored to slack");
            }
            Err(e) if is_content_post_error(&e) => {
                let n = fails + 1;
                mirror_failures.insert(item.key.clone(), n);
                tracing::warn!(
                    key = %item.key, attempt = n, ?plan, error = %e,
                    "mirror: content post error — skipping past this ask (will degrade then give up), rest continue"
                );
                continue; // do NOT block mirroring of later asks on a poison one
            }
            Err(e) => {
                // Transport/transient (outage, rate limit): leave un-recorded so the batch retries next tick.
                tracing::warn!(error = %e, "failed to post mirror (transient — will retry)");
                break;
            }
        }
    }

    // (2) Relay the bridge's own inbox (agent → operator) to the channel, then archive each. A message that
    // deterministically fails to post must NEVER head-of-line-block the queue (the ~11h/57-message wedge):
    // a transport/transient fault retries in place (order preserved), but a CONTENT-class failure escalates
    // per-message degrade→quarantine and we SKIP PAST it so the rest flow.
    let inbound = drain(&cfg.fleet_dir, &cfg.bridge_agent)?;
    if inbound.len() >= RELAY_QUEUE_WARN {
        let oldest = inbound
            .first()
            .and_then(|e| e.file.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        tracing::warn!(
            depth = inbound.len(),
            %oldest,
            "outbound relay queue backlog — a message may be failing to post"
        );
    }
    for entry in inbound {
        let key = entry
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fails = relay_failures.get(&key).copied().unwrap_or(0);
        let plan = relay_plan(fails);
        if plan == RelayPlan::Quarantine {
            tracing::warn!(
                %key,
                content_failures = fails,
                "outbound relay: quarantining un-postable message — dead-lettering to failed/ (full text preserved), draining the rest"
            );
            mark_failed(&entry.file)?;
            relay_failures.remove(&key);
            continue;
        }
        let text = if plan == RelayPlan::Degraded {
            render_fleet_message_plain(&entry.msg)
        } else {
            render_fleet_message(&entry.msg)
        };
        let req = SlackApiChatPostMessageRequest::new(
            channel.clone(),
            SlackMessageContent::new().with_text(text),
        );
        match session.chat_post_message(&req).await {
            Ok(_) => {
                mark_processed(&entry.file)?;
                relay_failures.remove(&key);
            }
            Err(e) if is_content_post_error(&e) => {
                let n = fails + 1;
                relay_failures.insert(key.clone(), n);
                tracing::warn!(
                    %key, attempt = n, ?plan, error = %e,
                    "outbound relay: content post error — skipping past this message (will degrade then quarantine), rest continue"
                );
                continue; // do NOT block the queue on a poison message
            }
            Err(e) => {
                // Transport/transient (outage, rate limit): retry this + the rest next tick, order preserved.
                tracing::warn!(error = %e, "failed to relay bridge message (transient — will retry)");
                break;
            }
        }
    }
    Ok(())
}

// ── INBOUND (Slack → fleet) ─────────────────────────────────────────────────────────────────────

/// Run the Socket Mode listener until the socket closes. Push message events are routed via
/// [`handle_message`]. State (`cfg`) is shared into the callback through the listener's user state.
pub async fn run_socket_mode(cfg: Arc<Config>, tokens: SlackTokens) -> Result<(), BoxErr> {
    let client = Arc::new(hyper_client()?);
    let callbacks = SlackSocketModeListenerCallbacks::new().with_push_events(on_push_event);

    // The inbound handler only DELIVERS into fleet inboxes (it never posts to Slack — that's the outbound
    // loop's job), so it needs the fleet config but not the bot token.
    let listener_environment = Arc::new(
        SlackClientEventsListenerEnvironment::new(client.clone())
            .with_user_state(BridgeState { cfg: cfg.clone() }),
    );
    let listener = SlackClientSocketModeListener::new(
        &SlackClientSocketModeConfig::new(),
        listener_environment,
        callbacks,
    );

    let app_token = SlackApiToken::new(tokens.app_token.clone().into());
    listener.listen_for(&app_token).await?;
    listener.serve().await;
    Ok(())
}

/// Shared state handed to the push-events callback. Only the fleet config — the inbound path delivers
/// into fleet inboxes and never posts to Slack, so no bot token here.
#[derive(Clone)]
struct BridgeState {
    cfg: Arc<Config>,
}

async fn on_push_event(
    event: SlackPushEventCallback,
    _client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> Result<(), BoxErr> {
    if let SlackEventCallbackBody::Message(msg) = event.event {
        let state = {
            let read = states.read().await;
            read.get_user_state::<BridgeState>().cloned()
        };
        let Some(state) = state else { return Ok(()) };
        handle_message(&state, msg).await;
    }
    Ok(())
}

/// Turn an operator's Slack message into a fleet message and deliver it. A reply IN A THREAD the bridge
/// posted routes a fleet `answer` to the ask's original asker (via the persisted map); anything else is
/// parsed with the `@agent`/`!kind` grammar and delivered to the target (default: the concierge). Skips
/// the bot's own posts and messages outside the bridge channel.
async fn handle_message(state: &BridgeState, msg: SlackMessageEvent) {
    // Ignore bot messages (including our own) and edits/other subtypes.
    if msg.sender.bot_id.is_some() || msg.subtype.is_some() {
        return;
    }
    let cfg = &state.cfg;
    // Only react in the configured bridge channel (if set).
    if let (Some(want), Some(got)) = (&cfg.channel, &msg.origin.channel)
        && got.as_ref() != want.as_str()
    {
        return;
    }
    let text = msg
        .content
        .as_ref()
        .and_then(|c| c.text.clone())
        .unwrap_or_default();
    let text = text.trim();
    if text.is_empty() {
        return;
    }

    // A reply in a mirrored thread → route an `answer` to the original asker.
    if let Some(thread_ts) = &msg.origin.thread_ts {
        let map = ThreadMap::load(&cfg.fleet_dir);
        if let Some(ask) = map.asker_for_thread(thread_ts.as_ref()) {
            let m = Message::new(&cfg.bridge_agent, &ask.asker, "answer", first_line(text))
                .with_body(text);
            match deliver(&cfg.fleet_dir, &ask.asker, m) {
                Ok(_) => {
                    tracing::info!(asker = %ask.asker, "routed operator reply as answer");
                    spawn_operator_wake(ask.asker.clone());
                }
                Err(e) => {
                    tracing::warn!(error = %e, asker = %ask.asker, "failed to deliver answer")
                }
            }
            return;
        }
        // Unknown thread → fall through and treat as a free-form message.
    }

    // Otherwise parse the `@agent`/`!kind` grammar and deliver.
    let intent = parse_operator_message(text, &cfg.default_to);
    let m = Message::new(&cfg.bridge_agent, &intent.to, &intent.kind, &intent.subject)
        .with_body(&intent.body);
    match deliver(&cfg.fleet_dir, &intent.to, m) {
        Ok(_) => {
            tracing::info!(to = %intent.to, kind = %intent.kind, "delivered operator message");
            spawn_operator_wake(intent.to.clone());
        }
        Err(e) => tracing::warn!(error = %e, to = %intent.to, "failed to deliver (rejected name?)"),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// Fire-and-forget: wake an idle recipient NOW so an operator Slack message reaches it in seconds instead
/// of waiting out its full `/loop` interval. The inbound path writes the inbox file with the crate's own
/// `deliver` (it never goes through `cargo xtask fleet send`, which is what normally wakes on delivery), so
/// we shell the standalone `cargo xtask fleet wake --operator-message <agent>` — the same single-source-of-
/// truth `wake_window` the rest of the fleet uses, exactly like the watchdog fire (`--operator-message`
/// relaxes the interactive-role skip so even the concierge is woken; the mid-tick guard still shields an
/// active conversation). It is safe on EVERY deliver: a stopped / no-window / mid-tick recipient is a clean
/// skip (exit 0), never an error.
///
/// Runs inside its OWN `tokio::spawn` so it can't block the message handler, and it awaits the child —
/// which REAPS it (waitpid). A bare fire-and-forget that never waited would leave a zombie until the daemon
/// exits (the same deferred-reap hazard fixed for the watchdog in PR#949); awaiting in a detached task keeps
/// the handler non-blocking AND the child reaped.
///
/// The wait is BOUNDED by [`WAKE_TIMEOUT`] with an explicit kill+reap on elapse, mirroring
/// `run_watchdog_once`. `fleet wake` is a fast tmux/fs op, but if it ever wedged (a stuck tmux/git/registry
/// call) an unbounded await would leak this detached task + its child until the daemon exits. Low-severity
/// on its own (detached per-message — a hang can't stall the handler, unlike the watchdog fire loop it
/// borrows the pattern from), but bounding it keeps the crate consistent with the reap-and-timeout
/// discipline (reviewer consistency note on PR be8c4d98a).
fn spawn_operator_wake(to: String) {
    tokio::spawn(async move {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let mut child = match tokio::process::Command::new("cargo")
            .args(["xtask", "fleet", "wake", "--operator-message", &to])
            .current_dir(&repo_root)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, to = %to, "operator wake spawn failed (non-fatal)");
                return;
            }
        };
        // `fleet wake` exits 0 whether it WOKE or SKIPPED (stopped/no-window/mid-tick) — both are valid
        // outcomes it just prints (xtask/fleet.rs `wake_cmd`). So a NON-ZERO exit is not a skip but a genuine
        // failure (e.g. cargo/xtask couldn't run); surface it (still non-fatal — the recipient's own /loop
        // is the safety net, and we must never derail an inbound deliver over a wake).
        match tokio::time::timeout(WAKE_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) if status.success() => {}
            Ok(Ok(status)) => {
                tracing::warn!(to = %to, code = ?status.code(), "operator wake exited non-zero (non-fatal)")
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, to = %to, "operator wake wait failed (non-fatal)")
            }
            // Timed out: explicit kill().await SIGKILLs AND reaps (not kill_on_drop's deferred reap, PR#949);
            // kill_on_drop still finalizes if that itself errors. Keeps the low-severity leak bounded.
            Err(_) => {
                let _ = child.kill().await;
                tracing::warn!(to = %to, secs = WAKE_TIMEOUT.as_secs(), "operator wake timed out — killed (non-fatal)");
            }
        }
    });
}

/// Bound for a single `cargo xtask fleet wake` fire (see [`spawn_operator_wake`]). Generous vs a healthy
/// wake (a tmux/fs op is sub-second) but finite, so a wedged call can't leak a detached task/child forever.
const WAKE_TIMEOUT: Duration = Duration::from_secs(30);

fn hyper_client() -> Result<SlackHyperClient, BoxErr> {
    Ok(SlackClient::new(SlackClientHyperConnector::new()?))
}

//! Async transport helpers for the daemon — the thin layer between the pure lib and slack-morphism /
//! tokio / subprocesses. Kept out of `main.rs` so `main` reads as a wiring diagram. Not unit-tested (live
//! WebSocket + subprocess); the pure decisions it calls into ARE tested in the lib.

use cadenza_slack_bridge::{
    Config, Message, MirroredAsk, SlackTokens, ThreadMap, deliver, drain, mark_processed,
    parse_operator_message, render_fleet_message, sidecar, watchdog,
};
use slack_morphism::prelude::*;
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

    loop {
        if let Err(e) = outbound_tick(&cfg, &client, &bot, &channel_id).await {
            tracing::warn!(error = %e, "outbound tick error (will retry)");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn outbound_tick(
    cfg: &Config,
    client: &SlackHyperClient,
    bot: &SlackApiToken,
    channel: &SlackChannelId,
) -> Result<(), BoxErr> {
    let session = client.open_session(bot);

    // (1) Mirror new concierge asks/backlog.
    let mut map = ThreadMap::load(&cfg.fleet_dir);
    let pending = drain(&cfg.fleet_dir, &cfg.default_to)?; // watch, don't consume: no mark_processed
    let to_post = sidecar::select_to_mirror(&pending, &map);
    for item in to_post {
        let text = render_fleet_message(&item.msg);
        let req = SlackApiChatPostMessageRequest::new(
            channel.clone(),
            SlackMessageContent::new().with_text(text),
        );
        match session.chat_post_message(&req).await {
            Ok(resp) => {
                map.record(
                    resp.ts.to_string(),
                    item.key.clone(),
                    MirroredAsk {
                        asker: item.msg.from.clone(),
                        kind: item.msg.kind.clone(),
                        subject: item.msg.subject.clone(),
                    },
                );
                map.save(&cfg.fleet_dir)?;
                tracing::info!(asker = %item.msg.from, kind = %item.msg.kind, "mirrored to slack");
            }
            Err(e) => {
                // Leave it un-recorded so it retries next tick rather than being lost.
                tracing::warn!(error = %e, "failed to post mirror; will retry");
                break;
            }
        }
    }

    // (2) Relay the bridge's own inbox (agent → operator) to the channel, then archive each.
    let inbound = drain(&cfg.fleet_dir, &cfg.bridge_agent)?;
    for entry in inbound {
        let req = SlackApiChatPostMessageRequest::new(
            channel.clone(),
            SlackMessageContent::new().with_text(render_fleet_message(&entry.msg)),
        );
        match session.chat_post_message(&req).await {
            Ok(_) => mark_processed(&entry.file)?,
            Err(e) => {
                tracing::warn!(error = %e, "failed to relay bridge message; will retry");
                break; // preserve order; retry this + the rest next tick
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
                    tracing::info!(asker = %ask.asker, "routed operator reply as answer (auto-nudges)")
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
        Ok(_) => tracing::info!(to = %intent.to, kind = %intent.kind, "delivered operator message"),
        Err(e) => tracing::warn!(error = %e, to = %intent.to, "failed to deliver (rejected name?)"),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn hyper_client() -> Result<SlackHyperClient, BoxErr> {
    Ok(SlackClient::new(SlackClientHyperConnector::new()?))
}

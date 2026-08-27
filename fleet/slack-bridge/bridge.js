#!/usr/bin/env node
// Cadenza fleet ↔ Slack bridge, in **Socket Mode**.
//
// Lets the operator talk to the concierge (and the rest of the fleet) from Slack instead of the tmux
// window. It runs a Slack app that dials OUT over a WebSocket (Socket Mode) — NO public HTTPS endpoint, no
// tunnel, no request-signature handling; Slack authenticates the connection with an app-level token.
//
// Two directions, both over the ordinary fleet inbox protocol (the same atomic JSON files
// `cargo xtask fleet send` writes — see inbox.js):
//   Slack → fleet:  a DM to the bot, or a message in the configured bridge channel, is parsed
//                   (`@agent`/`!kind` prefixes, else a `note` to the concierge) and DELIVERED into the
//                   target agent's inbox. The concierge drains it on its next tick exactly as if it came
//                   from a terminal.
//   fleet → Slack:  the bridge has its OWN fleet identity (SLACK_BRIDGE_AGENT, default `slack-bridge`).
//                   Any agent that wants to reach the operator does `fleet send --to slack-bridge …`;
//                   this daemon polls its inbox and posts each message to the bridge channel, then moves
//                   it to processed/. So the concierge answers an operator by sending the bridge a
//                   message — it lands in Slack.
//
// Config via environment variables:
//   SLACK_BOT_TOKEN     — REQUIRED. Bot User OAuth Token, `xoxb-…` (OAuth & Permissions).
//   SLACK_APP_TOKEN     — REQUIRED. App-Level Token with `connections:write`, `xapp-…` (Basic
//                         Information → App-Level Tokens). This is what enables Socket Mode.
//   SLACK_BRIDGE_CHANNEL— REQUIRED for fleet→Slack. Channel ID (e.g. `C0123ABCD`) the bridge posts agent
//                         messages into and listens in. DMs to the bot always work regardless.
//   FLEET_DIR           — the fleet state dir holding `inbox/` (default: `<repo>/.claude/fleet`, resolved
//                         relative to this file).
//   FLEET_DEFAULT_TO    — default recipient when the operator gives no `@agent` (default: `concierge`).
//   SLACK_BRIDGE_AGENT  — this bridge's own fleet agent name / inbox (default: `slack-bridge`).
//   POLL_MS             — how often to poll the bridge inbox for fleet→Slack messages (default: 2000).

"use strict";

const path = require("node:path");
const { spawn } = require("node:child_process");
const { App } = require("@slack/bolt");
const { deliver, drain, markProcessed, markFailed } = require("./inbox.js");
const {
  parseOperatorMessage,
  renderFleetMessage,
  renderFleetMessagePlain,
  relayPlan,
  isContentPostError,
  RELAY_QUEUE_WARN,
  helpText,
  isTransientSocketModeFault,
} = require("./format.js");

const BOT_TOKEN = process.env.SLACK_BOT_TOKEN || "";
const APP_TOKEN = process.env.SLACK_APP_TOKEN || "";
const BRIDGE_CHANNEL = process.env.SLACK_BRIDGE_CHANNEL || "";
const DEFAULT_TO = process.env.FLEET_DEFAULT_TO || "concierge";
const BRIDGE_AGENT = process.env.SLACK_BRIDGE_AGENT || "slack-bridge";
const POLL_MS = Number(process.env.POLL_MS || 2000);
// Default FLEET_DIR: this file's tracked home is <repo>/fleet/slack-bridge/bridge.js, and the runtime
// fleet state (inboxes) lives at <repo>/.claude/fleet — so run from the main checkout, or set FLEET_DIR.
const FLEET_DIR = process.env.FLEET_DIR || path.resolve(__dirname, "..", "..", ".claude", "fleet");
// Repo root (this file lives at <repo>/fleet/slack-bridge/bridge.js) — the cwd for shelling `cargo xtask`,
// same as the watchdog runner uses. Used by wakeOperatorRecipient below.
const REPO_ROOT = path.resolve(__dirname, "..", "..");

// Fire-and-forget: wake an idle recipient NOW so an operator's Slack message reaches it in seconds rather
// than waiting out its full `/loop` interval. The inbound path writes the inbox file with our own `deliver`
// (it never goes through `cargo xtask fleet send`, which is what normally wakes on delivery), so we shell
// the standalone `cargo xtask fleet wake --operator-message <agent>` — the fleet's single-source-of-truth
// wake (same tmux `wake_window` guards). `--operator-message` relaxes the interactive-role skip so even the
// concierge is woken; the mid-tick guard still shields an active conversation. Safe on EVERY deliver: a
// stopped / no-window / mid-tick recipient is still an exit-0 outcome. Detached + unref'd and stdio-ignored
// so it can't block the handler or hold the event loop; libuv reaps the child (no zombie).
function wakeOperatorRecipient(to) {
  try {
    const child = spawn("cargo", ["xtask", "fleet", "wake", "--operator-message", to], {
      cwd: REPO_ROOT,
      stdio: "ignore",
      detached: true,
    });
    // `fleet wake` exits 0 whether it WOKE or SKIPPED (both are valid outcomes it just prints), so a NON-ZERO
    // exit is a genuine failure (e.g. cargo/xtask couldn't run), not a skip — surface it (still non-fatal:
    // the recipient's own /loop is the safety net). `error` fires only on spawn failure (e.g. cargo missing).
    child.on("error", (e) => console.error(`operator wake spawn failed for ${to} (non-fatal):`, e.message));
    child.on("exit", (code, signal) => {
      // A non-zero EXIT code is a real failure; a SIGNAL kill reports code===null + signal set (e.g. the
      // process was killed) — surface that too. A clean exit 0 (woke or skipped) logs nothing.
      if (code) console.error(`operator wake for ${to} exited ${code} (non-fatal)`);
      else if (signal) console.error(`operator wake for ${to} killed by ${signal} (non-fatal)`);
    });
    child.unref();
  } catch (e) {
    console.error(`operator wake spawn threw for ${to} (non-fatal):`, e.message);
  }
}

// A KNOWN, self-recovering Socket Mode fault (`isTransientSocketModeFault`, defined in the dependency-free
// format.js so the smoke test can pin it WITHOUT loading @slack/bolt): `@slack/socket-mode` drives its
// connection with the `finity` state machine, and when Slack sends a `server explicit disconnect` frame
// while the client is still mid-handshake (state `connecting`), finity has no transition for it and THROWS
// synchronously from the WebSocket message handler — an uncaughtException that kills the whole process. The
// client would otherwise just reconnect. Rapid connection cycling made this crash-loop the daemon 5× in a
// minute, risking systemd's StartLimitBurst stopping the restart entirely (= operator comms down). Survive
// this specific transient (client reconnects on its own); re-throw anything else so a genuine fault still
// surfaces + exits for the supervisor to restart cleanly.
function installProcessGuards() {
  process.on("uncaughtException", (err) => {
    if (isTransientSocketModeFault(err)) {
      console.error("survived transient Socket Mode fault (client will reconnect):", err.message);
      return;
    }
    console.error("fatal uncaughtException — exiting for the supervisor to restart:", err);
    process.exit(1);
  });
  process.on("unhandledRejection", (reason) => {
    if (isTransientSocketModeFault(reason)) {
      console.error("survived transient Socket Mode rejection (client will reconnect):", reason && reason.message);
      return;
    }
    console.error("fatal unhandledRejection — exiting for the supervisor to restart:", reason);
    process.exit(1);
  });
}

function main() {
  installProcessGuards();
  const missing = [];
  if (!BOT_TOKEN) missing.push("SLACK_BOT_TOKEN (xoxb-…)");
  if (!APP_TOKEN) missing.push("SLACK_APP_TOKEN (xapp-…, with connections:write)");
  if (missing.length) {
    console.error("Missing required env: " + missing.join(", ") + ".\nSee ./README.md for setup.");
    process.exit(1);
  }

  const app = new App({ token: BOT_TOKEN, appToken: APP_TOKEN, socketMode: true });

  // ── Slack → fleet ────────────────────────────────────────────────────────────────────────────
  // Handle a plain message: a DM to the bot, or a message in the bridge channel. We ignore the bot's
  // own posts and messages elsewhere. `help`/empty prints usage; anything else is delivered to the fleet.
  app.message(async ({ message, say }) => {
    // Only react to ordinary user messages (no subtypes like edits/joins), and only in a DM (im) or the
    // configured bridge channel.
    if (message.subtype || message.bot_id) return;
    const isDM = message.channel_type === "im";
    const inBridgeChannel = BRIDGE_CHANNEL && message.channel === BRIDGE_CHANNEL;
    if (!isDM && !inBridgeChannel) return;

    const text = (message.text || "").trim();
    if (!text || text.toLowerCase() === "help") {
      await say(helpText(DEFAULT_TO));
      return;
    }

    const intent = parseOperatorMessage(text, DEFAULT_TO);
    try {
      deliver(FLEET_DIR, intent.to, {
        from: BRIDGE_AGENT,
        kind: intent.kind,
        subject: intent.subject,
        body: intent.body,
      });
      // Wake the recipient NOW so this operator message doesn't wait out its /loop interval.
      wakeOperatorRecipient(intent.to);
      await say(`:incoming_envelope: → *${intent.to}* _(${intent.kind})_`);
    } catch (e) {
      console.error("deliver failed:", e);
      await say(`:warning: couldn't deliver to *${intent.to}*: ${e.message}`);
    }
  });

  // ── fleet → Slack ────────────────────────────────────────────────────────────────────────────
  // Per-message CONTENT-failure counts (inbox basename → count) so a deterministically un-postable message
  // escalates degrade→quarantine across polls WITHOUT blocking the queue. In-memory: a restart resets it,
  // which is fine — the message re-drains and re-clears in a bounded number of polls (never forever, the
  // ~11h/57-message wedge this fixes).
  const relayFailures = new Map();

  // Poll the bridge's own inbox; post each message to the bridge channel, then archive it. A message that
  // deterministically fails to post must NEVER head-of-line-block the queue: a transport/transient fault
  // retries in place (order preserved), but a CONTENT-class failure escalates per-message (degrade to a
  // plain variant, then dead-letter to failed/) and we SKIP PAST it so the rest flow.
  async function pumpInbox() {
    if (!BRIDGE_CHANNEL) return; // nowhere to post; DMs are inbound-only
    const pending = drain(FLEET_DIR, BRIDGE_AGENT);
    if (pending.length >= RELAY_QUEUE_WARN) {
      console.warn(
        `outbound relay queue backlog: depth ${pending.length}, oldest ${path.basename(pending[0].file)} ` +
          "— a message may be failing to post",
      );
    }
    for (const { file, msg } of pending) {
      const key = path.basename(file);
      const fails = relayFailures.get(key) || 0;
      const plan = relayPlan(fails);
      if (plan === "quarantine") {
        console.error(
          `outbound relay: quarantining un-postable message ${key} after ${fails} content failures ` +
            "— dead-lettering to failed/ (full text preserved), draining the rest",
        );
        markFailed(file);
        relayFailures.delete(key);
        continue; // a poison message must never block the rest
      }
      const text = plan === "degraded" ? renderFleetMessagePlain(msg) : renderFleetMessage(msg);
      const req = { channel: BRIDGE_CHANNEL, text };
      if (plan === "degraded") req.mrkdwn = false; // extra belt: also disable mrkdwn parse for the plain variant
      try {
        await app.client.chat.postMessage(req);
        markProcessed(file);
        relayFailures.delete(key);
      } catch (e) {
        if (isContentPostError(e)) {
          const n = fails + 1;
          relayFailures.set(key, n);
          const reason = (e && e.data && e.data.error) || (e && e.message) || e;
          console.error(`outbound relay: content post error for ${key} (attempt ${n}, plan=${plan}): ${reason}`);
          continue; // skip past it this poll so the rest flow; it degrades/quarantines on later polls
        }
        // Transport/transient fault (outage, rate limit): retry this + the rest next poll, order preserved.
        console.error("post to Slack failed (transient — will retry):", (e && e.message) || e);
        break;
      }
    }
  }

  (async () => {
    await app.start();
    console.log(
      `Cadenza fleet↔Slack bridge running (Socket Mode). agent=${BRIDGE_AGENT} default→${DEFAULT_TO} ` +
        `fleetDir=${FLEET_DIR}` + (BRIDGE_CHANNEL ? ` channel=${BRIDGE_CHANNEL}` : " (no channel; DM-only inbound)"),
    );
    const timer = setInterval(() => {
      pumpInbox().catch((e) => console.error("pumpInbox error:", e));
    }, POLL_MS);
    timer.unref?.();
  })().catch((e) => {
    console.error("failed to start:", e);
    process.exit(1);
  });
}

if (require.main === module) main();

module.exports = { main };

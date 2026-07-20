# Slack bridge — design & staging

This captures the architecture the operator + concierge locked for the Slack bridge, so the seam is
legible regardless of implementation language. **Status: all staged work is DELIVERED and live** (see
[Staging](#staging-smallest-useful-first) below) — this doc is now the reference for how the running
bridge is wired, not a forward plan.

## Goal

The operator drives the Cadenza fleet from Slack and **never logs into the machine**. They talk to the
**concierge** (the fleet's one human interface) in a dedicated channel (e.g. `#cadenza-fleet`), threaded
per topic.

## It is a SIDE-CAR, not a replacement

The bridge does **not** replace the concierge's tmux `AskUserQuestion` flow — it mirrors alongside it.
An `ask` still lands in the concierge's inbox and still surfaces in its tmux window; it **also** appears in
Slack. Whichever channel answers first (the concierge's tmux answer, or the operator's Slack reply) produces
the fleet `answer`; the other is a harmless duplicate the asking agent's next inbox drain ignores (asks are
idempotent by ask-id). The operator is never forced onto one channel.

## Transport

A Slack app in **Socket Mode** — the bridge dials OUT over a WebSocket, so there is no public inbound URL to
expose (works behind a corp network). Auth is an app-level token (`xapp-…`, `connections:write`) plus a bot
token (`xoxb-…`, scopes `chat:write`, `channels:history`/`groups:history`, `app_mentions:read`). Tokens come
from a **gitignored** source (env or `.claude/fleet/slack.toml`); the bridge **fails soft** when they're
absent (logs + retries, never crashes) so it can land and run before the operator has created the Slack app.

## The two directions

### Outbound — a fleet `ask`/`backlog` → a Slack thread

The bridge **watches** the concierge's inbox (`.claude/fleet/inbox/concierge/*.json`). On a newly-arrived
`ask` (or `backlog`), it posts a top-level message to the channel and records the mapping

```
slack_thread_ts  →  { asker, ask subject/seq }
```

in a gitignored `.claude/fleet/slack-threads.json`. **It does not consume or move the concierge's inbox
file** — that's the concierge's job — so the ask still shows in tmux (the dual-channel property above).

### Inbound — an operator Slack reply → a fleet `answer`

The bridge subscribes (Socket Mode) to channel message events.

- A reply **in a thread the bridge posted** → look up `thread_ts → asker` and run
  `cargo xtask fleet send --to <asker> --kind answer --subject <decision> --body <operator text>` — exactly
  what the concierge does by hand today.
- A **top-level** (non-thread) message → a `backlog` add, or a free-form instruction to the concierge.
- A `status` command → the bridge runs `cargo xtask fleet status` and replies with the board.

A Slack-injected `answer` ALSO **nudges the asker's tmux window** so the asker acts on it immediately
rather than waiting for its next `/loop` tick. This is FREE: `cargo xtask fleet send` itself send-keys
the recipient's window after delivering (v-fleet-tooling's built-in nudge; skipped for stopped/no-window/
mid-tick/interactive recipients), so the bridge needs no separate nudge call.

## Staging (smallest-useful first)

- **Stage 0 (landed, on trunk):** the fleet-inbox I/O primitive — a byte-compatible port of the Rust
  `deliver`/`drain` (`inbox.js`), Slack↔fleet message shaping (`format.js`), and a generic
  operator→any-agent bridge (`bridge.js`) with `smoke.test.js` gate coverage. This is the substrate the
  sidecar is built on.
- **Stage 1 — OUTBOUND-ONLY (landed):** watch the concierge inbox, post asks/backlog to the channel as
  threads, persist the thread map. The operator SEES the fleet talking to them.
- **Stage 2 — INBOUND routing (landed):** thread-reply → `answer` to the asker (+ automatic window nudge);
  top-level → backlog / concierge instruction; `status` command.
- **Stage 3 — deployment + reliability backbone (landed):** the daemon also runs the fleet **watchdog**
  out-of-band (`WATCHDOG_ONLY=1`, no Socket Mode → no competing Slack connection) so a stalled agent's
  `/loop` is re-armed even when session-cron can't heal it. Launchers: `run.sh` (auto-restart loop, node|rust
  via `BRIDGE_IMPL`), `systemd/` `--user` units (boot-survival via `loginctl enable-linger`), and `revive.sh`
  (idempotent no-systemd fallback). All three resolve the shared HUB `FLEET_DIR` via the common git dir so
  they work from any worktree. A Rust rewrite (`src/`, slack-morphism, isolated `[workspace]`) reached parity
  with the original Node bridge; the live implementation is selectable with `BRIDGE_IMPL`.

## Decisions (resolved — kept here for the rationale)

1. **Language.** Kickoff specified a Rust crate (slack-morphism); Stage 0 landed zero-Rust Node/`@slack/bolt`
   for speed. RESOLVED: rewrite to Rust for toolchain consistency, keep Node live until proven at parity
   (`BRIDGE_IMPL` is the cutover switch). Both implementations now exist and share the gate.
2. **Placement.** Assign said `integrations/slack/`; the operator reclassified it as internal fleet tooling.
   RESOLVED: `fleet/slack-bridge/` (a standalone crate with its own `[workspace]` so slack-morphism's async
   tree stays out of the seed lockfile + `xtask check`).
3. **Thread "resolved" marker.** Whether a tmux-answered ask should get a "resolved" note posted into its
   Slack thread. Deferred — the dual-channel idempotency (whichever answers first wins, the other is
   ignored) already keeps correctness; the marker is a nice-to-have, not built.

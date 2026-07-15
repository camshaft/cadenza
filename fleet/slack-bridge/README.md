# Slack bridge — drive the Cadenza fleet from Slack

**Internal dev tooling for whoever is driving the Cadenza fleet** — not an external/end-user integration.
It lets *you* (the operator building the language) talk to the fleet **concierge** (and any other agent)
from Slack instead of the tmux window, so you can steer the fleet from your phone or anywhere you have
Slack. You DM the bot (or post in a bridge channel), and your message is delivered into the concierge's
fleet inbox exactly as if you'd typed `cargo xtask fleet send` in a terminal. When the concierge (or any
agent) wants to reach you, it sends a fleet message to the bridge, which posts it back into Slack. Your
phone becomes the fleet's single pane of glass.

It lives under `fleet/` (fleet infrastructure) alongside the loop role bodies and the contract — it is part
of how the fleet is operated, not a product surface.

It runs in **Socket Mode** — the app dials OUT to Slack over a WebSocket, so there is **no public HTTPS
endpoint, no tunnel, and no request-signature handling**. You can run it from a laptop behind a NAT.

## How it works

The bridge is a first-class fleet peer with its own inbox (`slack-bridge` by default). It uses the SAME
atomic inbox files (`<fleet>/inbox/<agent>/<seq>-<pid>-<kind>.json`) that `cargo xtask fleet send` writes
and every agent drains — see [`inbox.js`](inbox.js), a faithful JS port of the Rust `deliver`/`drain` in
`xtask/src/fleet.rs`. So nothing about the fleet changes: the concierge just receives normal messages.

- **Slack → fleet.** A DM to the bot, or a message in the configured bridge channel, becomes a fleet
  message delivered to the target agent's inbox:
  - plain text → a `note` to the default recipient (the **concierge**),
  - `@agent …` → send to a different agent (e.g. `@pr-sync status`),
  - `!kind …` → set the message kind (`ask`, `backlog`, `status`, `assign`, `note`, …),
  - `@design-jsx !assign build the JSX surface` → both at once.
- **fleet → Slack.** Any agent reaches you with `cargo xtask fleet send --to slack-bridge --kind note
  --subject "…" --body "…"`. The bridge polls its own inbox and posts each message into the bridge
  channel, then archives it to `processed/`. **So the concierge answers you by sending the bridge a
  message** — configure its human-interface to target `slack-bridge` (or just send there by hand).

## Setup

### 1. Create the Slack app (Socket Mode)

1. <https://api.slack.com/apps> → **Create New App** → **From scratch**. Name it "Cadenza fleet", pick your
   workspace.
2. **Socket Mode** (left nav) → toggle **Enable Socket Mode** on. When prompted, create an **App-Level
   Token** with the `connections:write` scope — copy it (`xapp-…`); this is `SLACK_APP_TOKEN`.
3. **OAuth & Permissions** → **Bot Token Scopes**, add: `chat:write` (post replies/messages),
   `app_mentions:read`, `im:history` + `im:read` + `im:write` (DMs), and `channels:history` (to read the
   bridge channel; use `groups:history` for a private channel).
4. **Event Subscriptions** → toggle on, then under **Subscribe to bot events** add: `message.im` (DMs) and
   `message.channels` (or `message.groups` for a private bridge channel), plus `app_mention` if you want to
   @-mention the bot in channels. (In Socket Mode there is no Request URL to verify — events arrive over the
   socket.)
5. **Install App** (left nav) → **Install to Workspace**. Copy the **Bot User OAuth Token** (`xoxb-…`); this
   is `SLACK_BOT_TOKEN`.
6. Invite the bot to your bridge channel: `/invite @Cadenza fleet`, and copy the channel ID (channel name →
   **View channel details** → bottom, `C0123ABCD…`).

### 2. Run the bridge

```
cd fleet/slack-bridge
npm install                       # installs @slack/bolt (the only dependency)

export SLACK_BOT_TOKEN=xoxb-…     # Bot User OAuth Token
export SLACK_APP_TOKEN=xapp-…     # App-Level Token (connections:write)
export SLACK_BRIDGE_CHANNEL=C0123ABCD   # channel to post agent messages into (DMs work without this)
npm start                         # node bridge.js
```

It prints `Cadenza fleet↔Slack bridge running (Socket Mode)` and stays up. Leave it running (e.g. under
`tmux`, `pm2`, or a systemd unit) alongside the fleet.

## Configuration

All via environment variables:

| Variable                | Required | Default                         | Meaning |
|-------------------------|----------|---------------------------------|---------|
| `SLACK_BOT_TOKEN`       | yes      | —                               | Bot User OAuth Token (`xoxb-…`). |
| `SLACK_APP_TOKEN`       | yes      | —                               | App-Level Token (`xapp-…`, `connections:write`) — enables Socket Mode. |
| `SLACK_BRIDGE_CHANNEL`  | for fleet→Slack | —                        | Channel ID the bridge posts agent messages into / listens in. DMs work without it (inbound only). |
| `FLEET_DIR`             | no       | `<repo>/.claude/fleet`          | The fleet state dir holding `inbox/`. |
| `FLEET_DEFAULT_TO`      | no       | `concierge`                     | Recipient when you give no `@agent`. |
| `SLACK_BRIDGE_AGENT`    | no       | `slack-bridge`                  | The bridge's own fleet agent name / inbox. |
| `POLL_MS`               | no       | `2000`                          | Fleet→Slack inbox poll interval (ms). |

## Use

DM the bot, or post in the bridge channel:

| You type (in Slack)                     | What the fleet gets |
|-----------------------------------------|---------------------|
| `what's blocked?`                       | a `note` to the concierge |
| `@pr-sync status`                       | a `note` to `pr-sync` |
| `!backlog try a dark-mode guide theme`  | a `backlog` item to the concierge |
| `@design-jsx !assign build the JSX surface` | an `assign` to `design-jsx` |
| `help`                                  | the usage cheatsheet |

The concierge's reply (or any agent's message) comes back as a Slack post in the bridge channel:
`*pr-sync* · _merged_: landed  /  all green`.

## Test

```
npm test        # node smoke.test.js — zero deps, no Slack, no network
```

The smoke test is this integration's **gate coverage**: it pins the on-disk inbox message shape (so a
change to the Rust `deliver` format that this JS port must match can't silently drift), the drain
ordering + `processed/` archival, and the Slack↔fleet message parsing/rendering. The Socket-Mode wiring in
[`bridge.js`](bridge.js) is Bolt's concern (a live WebSocket) and is intentionally kept to thin calls into
the tested [`inbox.js`](inbox.js) + [`format.js`](format.js).

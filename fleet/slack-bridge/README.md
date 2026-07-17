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

> **Two implementations, one design.** The bridge began as a Node/`@slack/bolt` prototype (`bridge.js` +
> `inbox.js` + `format.js`) and is being rewritten in **Rust** (`cadenza-slack-bridge`, this crate, on
> slack-morphism) for toolchain consistency. Both speak the same fleet protocol and follow
> [`DESIGN.md`](DESIGN.md); `run.sh` launches either via `BRIDGE_IMPL=node|rust`. Node is the live path
> until the Rust daemon is proven at live parity, then it's retired.

## How it works

The bridge is a first-class fleet peer with its own inbox (`slack-bridge` by default). It uses the SAME
atomic inbox files (`<fleet>/inbox/<agent>/<seq>-<pid>-<kind>.json`) that `cargo xtask fleet send` writes
and every agent drains — the Rust `inbox.rs` / Node `inbox.js` are faithful ports of the `deliver`/`drain`
in `xtask/src/fleet.rs`. So nothing about the fleet changes: the concierge just receives normal messages.

- **Slack → fleet.** A DM to the bot, or a message in the configured bridge channel, becomes a fleet
  message delivered to the target agent's inbox:
  - plain text → a `note` to the default recipient (the **concierge**),
  - `@agent …` → send to a different agent (e.g. `@pr-sync status`),
  - `!kind …` → set the message kind (`ask`, `backlog`, `status`, `assign`, `note`, …),
  - `@design-jsx !assign build the JSX surface` → both at once.

  (Agent names are validated to a strict slug — no path-traversal from Slack input, see `DESIGN.md`.)
- **fleet → Slack.** Two ways: (a) the bridge WATCHES the concierge inbox and mirrors each new `ask`/
  `backlog` to the channel as a thread, so an operator reply in that thread routes a fleet `answer` back to
  the original asker; (b) any agent reaches you directly with `cargo xtask fleet send --to slack-bridge …`
  and the bridge relays it to the channel, then archives it to `processed/`.
- **Watchdog (reliability backbone).** Because it's a long-lived host process, the bridge ALSO runs
  `cargo xtask fleet watchdog` every ~4 min out-of-band — re-arming stalled loops / reaping dead windows
  even if the concierge itself stalls. This runs **even with no Slack tokens** (watchdog-only mode).

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

### 2. Provide the tokens (out of repo)

Put the two tokens in `~/.cadenza-env` (a home-dir dotenv file — never committed):

```
SLACK_BOT_TOKEN=xoxb-…     # Bot User OAuth Token
SLACK_APP_TOKEN=xapp-…     # App-Level Token (connections:write)
```

`config.rs` reads these in precedence order: process env > `~/.cadenza-env` > `.claude/fleet/slack.toml` >
defaults. With no tokens the bridge still starts (watchdog-only) and never crashes.

### 3. Run the bridge

From a durable host (a `tmux` window, `pm2`, or a systemd unit) so it survives crashes:

```
fleet/slack-bridge/run.sh          # auto-restart loop; reads ~/.cadenza-env; BRIDGE_IMPL=node (default) | rust
```

Or by hand for the Node impl: `cd fleet/slack-bridge && npm install && npm start`. Set
`SLACK_BRIDGE_CHANNEL` (a channel or DM id) so fleet→Slack posts reach you. It prints
`… running (Socket Mode)` / `Now connected to Slack` and stays up.

#### Auto-start on boot (systemd — recommended)

A `tmux`/manual launcher does **not** survive a host reboot (it also clears the `/tmp` runner loop),
leaving the operator with no comms until an agent notices and relaunches. To make the bridge + the
out-of-band watchdog daemon start automatically on boot and restart on crash, install the tracked
systemd `--user` units:

```
fleet/slack-bridge/systemd/install.sh      # substitutes machine paths, enables + starts both units
```

This writes `cadenza-slack-bridge.service` (the Node/Rust bridge, `RUN_ONCE=1` so systemd owns the
restart loop) and `cadenza-watchdog.service` (`WATCHDOG_ONLY=1`, no Socket Mode) into
`~/.config/systemd/user/`, runs `loginctl enable-linger` (so they start without an interactive login),
and enables both `WantedBy=default.target`. Manage them with `systemctl --user {status,restart,stop}
cadenza-slack-bridge cadenza-watchdog`; logs via `journalctl --user -u cadenza-slack-bridge -f`. The
`.service.in` files are checked-in **templates** (`@REPO_ROOT@`/`@HOME@`/`@PATH@`/`@CHANNEL@`
placeholders) — never commit a resolved absolute host path. Re-run `install.sh` after moving the repo
or rebuilding the watchdog binary (`cargo build --release` here, then `systemctl --user restart
cadenza-watchdog`).

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
| `SLACK_CHANNEL`         | no       | —                               | Alias for `SLACK_BRIDGE_CHANNEL` (dotenv files often use the shorter name). |
| `CADENZA_ENV_FILE`      | no       | `~/.cadenza-env`                | Dotenv file `run.sh` sources for the tokens. |
| `BRIDGE_IMPL`           | no       | `node`                          | `run.sh`: which implementation to launch (`node` \| `rust`). |
| `POLL_MS`               | no       | `2000`                          | (Node) fleet→Slack inbox poll interval (ms). |

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
cargo test      # Rust: the pure core (inbox/format/config/sidecar/watchdog) — the primary gate
npm test        # Node: node smoke.test.js — zero deps, no Slack, no network
```

Both suites are this crate's **gate coverage**: they pin the on-disk inbox message shape (so a change to
the Rust `deliver` format both ports must match can't silently drift), the drain ordering + `processed/`
archival, the Slack↔fleet message parsing/rendering, the fail-soft config precedence, the secret redaction,
and the agent-name path-traversal guard. The Socket-Mode transport (Rust `main.rs`/`runner.rs`, Node
`bridge.js`) is a live WebSocket and is intentionally kept to thin calls into the tested pure modules — so
it needs no live-network test. This crate is a **standalone workspace**, so run its gates from this
directory (`cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`); it is not
part of `cargo xtask check`.

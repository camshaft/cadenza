# Slack bridge — design & staging

This captures the architecture the operator + concierge locked for the Slack bridge, so the seam is
legible regardless of implementation language. It is the reference for the Stage-2 routing work.

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

A Slack-injected `answer` must ALSO **nudge the asker's tmux window** (v-fleet-tooling's `rearm_window` /
send-keys primitive) so the asker acts on it immediately rather than waiting for its next `/loop` tick —
coordinate with v-fleet-tooling on whether `fleet send` already nudges or the nudge is a separate call.

## Staging (smallest-useful first)

- **Stage 0 (landed, on trunk):** the fleet-inbox I/O primitive — a byte-compatible port of the Rust
  `deliver`/`drain` (`inbox.js`), Slack↔fleet message shaping (`format.js`), and a generic
  operator→any-agent bridge (`bridge.js`) with `smoke.test.js` gate coverage. This is the substrate the
  sidecar is built on.
- **Stage 1 — OUTBOUND-ONLY:** watch the concierge inbox, post asks/backlog to the channel as threads,
  persist the thread map. The operator SEES the fleet talking to them.
- **Stage 2 — INBOUND routing:** thread-reply → `answer` to the asker (+ window nudge); top-level → backlog /
  concierge instruction; `status` command.

## Open decisions (asked concierge; blocking Stage 2)

1. **Language.** The kickoff assign specified a Rust crate (slack-morphism); Stage 0 landed as zero-Rust
   Node/`@slack/bolt` (accepted by the operator live). Confirm keep-Node vs rewrite-Rust before Stage 2.
2. **Placement.** Assign said `integrations/slack/`; the operator called it internal fleet tooling →
   `fleet/slack-bridge/`. Confirm.
3. **Thread "resolved" marker.** Optional: when the concierge answers an ask via tmux (not Slack), should the
   bridge post a "resolved" note into the Slack thread so the operator sees it's handled? Adds coupling
   (the bridge must also watch the answer flow) — concierge's call.

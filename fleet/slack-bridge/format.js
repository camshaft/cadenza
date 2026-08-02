// Message shaping between Slack and the fleet inbox protocol — pure and dependency-free, so the smoke
// test exercises it directly.
//
// The bridge lets the operator talk to the concierge (or any agent) from Slack. Two directions:
//   Slack → fleet:  what the operator types in the bridge channel/DM becomes a fleet message delivered
//                    into the target agent's inbox. A leading `@agent` (or `/to agent`) retargets; a
//                    leading `!kind` sets the message kind; otherwise it's a `note` to the default agent
//                    (the concierge). This is exactly the `cargo xtask fleet send` shape, just typed in
//                    Slack instead of a terminal.
//   fleet → Slack:  a message drained from the bridge's OWN inbox (agents `send --to <bridge>` to reach
//                    the operator) is rendered as a readable Slack line and posted to the channel.

"use strict";

// The message kinds the fleet understands (AGENTS-fleet.md). The operator can force one with a leading
// `!kind`; otherwise Slack→fleet defaults to `note` (free-form coordination), which every agent accepts.
const KNOWN_KINDS = new Set([
  "merge-request",
  "merged",
  "reject",
  "issue",
  "assign",
  "ask",
  "answer",
  "backlog",
  "status",
  "note",
]);

/// Parse an operator's Slack line into a fleet-send intent: `{ to, kind, subject, body }`.
///
/// Grammar (all prefixes optional, order `@agent` then `!kind`):
///   `@concierge status the compiler`  → to=concierge, kind=note,   subject/body="status the compiler"
///   `@pr-sync !status ping`           → to=pr-sync,   kind=status, text="ping"
///   `!backlog add dark mode`          → to=<default>, kind=backlog, text="add dark mode"
///   `just some words`                 → to=<default>, kind=note,   text="just some words"
///
/// `defaultTo` is the agent used when no `@agent` is given (the concierge). The subject is the first line
/// (trimmed, capped) and the body is the full text — an agent gets a glanceable subject plus the detail.
function parseOperatorMessage(text, defaultTo) {
  let rest = (text || "").trim();
  let to = defaultTo;
  let kind = "note";

  // Agent-name charset is a strict slug — a leading alphanumeric then alphanumerics/hyphens (NO dots or
  // separators), so a `@..`/`@../x` retarget can't become a path-traversal agent name (PR #391). This is
  // the parse-side guard; inbox.js re-validates at the filesystem sink (defense in depth).
  const atMatch = rest.match(/^@([A-Za-z0-9][A-Za-z0-9-]*)\s*(.*)$/s);
  if (atMatch) {
    to = atMatch[1];
    rest = atMatch[2].trim();
  }

  const kindMatch = rest.match(/^!([A-Za-z-]+)\s*(.*)$/s);
  if (kindMatch && KNOWN_KINDS.has(kindMatch[1])) {
    kind = kindMatch[1];
    rest = kindMatch[2].trim();
  }

  const firstLine = rest.split("\n", 1)[0].trim();
  // Cap the subject by CODE POINTS, not UTF-16 code units: `firstLine.length`/`slice` count UTF-16
  // units, so `slice(0, 117)` can cut mid-surrogate-pair (an astral char, e.g. an emoji) into a lone
  // surrogate = an ill-formed subject written to the fleet inbox, AND disagree with the Rust
  // `cap_subject` (which counts Unicode scalars via chars()). Iterate as code points so JS and Rust
  // cap identically — the same PR #405 fix the render path already got, in this sibling path.
  const cp = [...firstLine];
  const subject = cp.length > 120 ? cp.slice(0, 117).join("") + "…" : firstLine || "(from Slack)";
  return { to, kind, subject, body: rest };
}

// Slack's chat.postMessage rejects an over-long `text`; a multi-KB fleet `ask` body would make the post
// FAIL and the outbound relay retry it forever, blocking the queue. Cap well under the limit (matches the
// Rust SLACK_TEXT_CAP); the full text still lives in the fleet inbox.
const SLACK_TEXT_CAP = 3500;

/// Render a fleet message (drained from the bridge's inbox, i.e. an agent → operator message) as a
/// Slack-mrkdwn string. Shows who it's from, the kind, the subject, and the body/ref when present. Kept
/// compact so a stream of them reads well in a channel, and length-capped so a big body can't fail the post.
function renderFleetMessage(msg) {
  const from = msg.from || "unknown";
  const kind = msg.kind || "note";
  const lines = [`*${from}* · _${kind}_` + (msg.subject ? `: ${msg.subject}` : "")];
  if (msg.ref) lines.push("`" + msg.ref + "`");
  if (msg.body && msg.body.trim()) lines.push(msg.body.trim());
  const out = lines.join("\n");
  // Cap by CODE POINTS, not UTF-16 code units: `out.length`/`slice` count UTF-16 units and would split an
  // astral-plane char (surrogate pair) at the boundary into an ill-formed string, AND disagree with the
  // Rust bridge's chars() (Unicode scalars). Iterate as code points ([...out]) so JS and Rust truncate
  // identically (PR #405).
  const cp = [...out];
  if (cp.length <= SLACK_TEXT_CAP) return out;
  const marker = "\n…[truncated — full text in the fleet inbox]";
  const keep = SLACK_TEXT_CAP - [...marker].length;
  return cp.slice(0, keep).join("") + marker;
}

/// A short usage/help string shown when the operator sends an empty message or `help`.
function helpText(defaultTo) {
  return [
    `*Cadenza fleet bridge* — you're talking to the fleet. Default recipient: *${defaultTo}*.`,
    "• plain text → a `note` to " + defaultTo,
    "• `@agent …` → send to a different agent (e.g. `@pr-sync status`)",
    "• `!kind …` → set the message kind (" +
      [...KNOWN_KINDS].join(", ") +
      "), e.g. `!backlog try dark mode`",
    "• `@design-foo !assign build X` → both at once",
  ].join("\n");
}

// A KNOWN, self-recovering Socket Mode fault. `@slack/socket-mode` drives its connection with the `finity`
// state machine; when Slack sends a `server explicit disconnect` frame while the client is still
// mid-handshake (state `connecting`), finity has no transition and THROWS synchronously from the WebSocket
// handler — an uncaughtException that kills the process, even though the client would just reconnect.
// bridge.js's process guards use this to SURVIVE that transient (re-throwing anything else). Lives HERE, in
// the dependency-free module, so the zero-dep smoke test can pin it without loading @slack/bolt.
function isTransientSocketModeFault(err) {
  const msg = (err && (err.message || String(err))) || "";
  // finity's message: "Unhandled event '<event>' in state '<state>'." — the connection-lifecycle events
  // (disconnect/handshake races) are benign and self-heal; match on the finity shape + a connection event.
  return (
    /Unhandled event '.*' in state '.*'\./.test(msg) &&
    /(disconnect|connecting|connected|reconnect|handshake|authenticated)/i.test(msg)
  );
}

module.exports = { parseOperatorMessage, renderFleetMessage, helpText, KNOWN_KINDS, isTransientSocketModeFault };

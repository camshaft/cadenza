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

  const atMatch = rest.match(/^@([A-Za-z0-9._-]+)\s*(.*)$/s);
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
  const subject = firstLine.length > 120 ? firstLine.slice(0, 117) + "…" : firstLine || "(from Slack)";
  return { to, kind, subject, body: rest };
}

/// Render a fleet message (drained from the bridge's inbox, i.e. an agent → operator message) as a
/// Slack-mrkdwn string. Shows who it's from, the kind, the subject, and the body/ref when present. Kept
/// compact so a stream of them reads well in a channel.
function renderFleetMessage(msg) {
  const from = msg.from || "unknown";
  const kind = msg.kind || "note";
  const lines = [`*${from}* · _${kind}_` + (msg.subject ? `: ${msg.subject}` : "")];
  if (msg.ref) lines.push("`" + msg.ref + "`");
  if (msg.body && msg.body.trim()) lines.push(msg.body.trim());
  return lines.join("\n");
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

module.exports = { parseOperatorMessage, renderFleetMessage, helpText, KNOWN_KINDS };

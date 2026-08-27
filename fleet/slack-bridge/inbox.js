// Fleet inbox I/O for the Slack bridge — a faithful JS port of the atomic message delivery/drain the
// Rust `cargo xtask fleet` implements (see xtask/src/fleet.rs `deliver` / `Message`). Pure and
// dependency-free (only Node built-ins), so the smoke test drives it against a temp dir with no Slack and
// no network.
//
// A fleet message is ONE JSON file in the recipient's inbox
// (`<fleetDir>/inbox/<to>/<seq:012>-<pid>-<kind>.json`), written via a temp-file + atomic rename so a
// reader never sees a half-written file and two senders never corrupt a shared file. The concierge (and
// every agent) drains its inbox oldest-first — filenames sort in send order — then moves each handled
// file into `processed/`. This module lets the bridge participate as a first-class fleet peer: it
// delivers the operator's Slack messages into an agent's inbox, and drains an inbox to relay agent
// messages back out to Slack.

"use strict";

const fs = require("node:fs");
const path = require("node:path");

// A per-process monotonic sequence, mirroring the Rust `next_seq` atomic (starts at 1). Combined with the
// pid in the filename, this makes deliveries from THIS process sort in send order and never collide with
// deliveries from the Rust xtask (a different pid). Resets across restarts, which is fine — pid differs.
let SEQ = 1;
function nextSeq() {
  return SEQ++;
}

/// A fleet agent name is a strict slug — a leading alphanumeric then alphanumerics/hyphens
/// (`pr-sync`, `design-jsx`, `v-slack-bridge`, `fix-foo`). NO dots, slashes, or other separators, so it
/// can never be a path component like `..` or contain a directory boundary.
const AGENT_NAME_RE = /^[A-Za-z0-9][A-Za-z0-9-]*$/;

/// True iff `agent` is a safe agent name to use as a path component. SECURITY: the agent name flows from
/// untrusted Slack input (a `@agent` retarget), and `inboxDir` joins it into a filesystem path — an
/// unvalidated `..` or `../../x` would let a Slack sender write OUTSIDE the inbox tree (PR #391). Reject
/// anything that isn't a strict slug.
function isValidAgentName(agent) {
  return typeof agent === "string" && AGENT_NAME_RE.test(agent);
}

/// The inbox directory for `agent` under `fleetDir` (`<fleetDir>/inbox/<agent>`). THROWS on an invalid
/// agent name rather than building a traversal-capable path — this is the single chokepoint both
/// `deliver` and `drain` route through, so validating here sandboxes every filesystem access.
function inboxDir(fleetDir, agent) {
  if (!isValidAgentName(agent)) {
    throw new Error(`invalid fleet agent name (path-traversal guard): ${JSON.stringify(agent)}`);
  }
  return path.join(fleetDir, "inbox", agent);
}

/// Deliver a fleet message into `to`'s inbox as one atomic JSON file, byte-for-byte in the shape the Rust
/// side reads. `msg` supplies `{ from, kind, subject, ref?, body? }`; `to`, `seq`, and pretty-printing are
/// filled here. Creates the inbox dir on demand (matching the Rust behavior of delivering even before the
/// recipient is registered). Returns the written file path.
function deliver(fleetDir, to, msg) {
  const dir = inboxDir(fleetDir, to);
  fs.mkdirSync(dir, { recursive: true });

  const seq = nextSeq();
  const kind = msg.kind || "note";
  const record = {
    from: msg.from || "unknown",
    to,
    kind,
    subject: msg.subject || "",
    ref: msg.ref || "",
    body: msg.body || "",
    seq,
  };
  const json = JSON.stringify(record, null, 2);
  const fname = `${String(seq).padStart(12, "0")}-${process.pid}-${kind}.json`;
  const tmp = path.join(dir, `.${fname}.tmp`);
  fs.writeFileSync(tmp, json);
  fs.renameSync(tmp, path.join(dir, fname));
  return path.join(dir, fname);
}

/// Read every message file in `agent`'s inbox, oldest-first (filename sort = send order). Returns an array
/// of `{ file, name, msg }` where `msg` is the parsed record. Skips the `processed/` subdir, dotfiles
/// (in-flight temp writes), and any non-`.json` file. Does NOT move anything — the caller decides success
/// then calls `markProcessed` per handled file, so a crash mid-relay re-delivers rather than drops.
function drain(fleetDir, agent) {
  const dir = inboxDir(fleetDir, agent);
  let entries;
  try {
    entries = fs.readdirSync(dir);
  } catch (e) {
    if (e.code === "ENOENT") return [];
    throw e;
  }
  const out = [];
  for (const name of entries.filter((n) => n.endsWith(".json") && !n.startsWith(".")).sort()) {
    const file = path.join(dir, name);
    try {
      const msg = JSON.parse(fs.readFileSync(file, "utf8"));
      out.push({ file, name, msg });
    } catch {
      // A malformed/partial file: leave it in place rather than crash the drain loop.
    }
  }
  return out;
}

/// Move a handled inbox file into `processed/` (created on demand), matching how agents archive drained
/// messages. Idempotent-ish: a missing source is ignored (already moved).
function markProcessed(file) {
  const dir = path.dirname(file);
  const processed = path.join(dir, "processed");
  fs.mkdirSync(processed, { recursive: true });
  try {
    fs.renameSync(file, path.join(processed, path.basename(file)));
  } catch (e) {
    if (e.code !== "ENOENT") throw e;
  }
}

/// Move an un-postable inbox file into a `failed/` dead-letter dir (created on demand), a sibling of
/// `processed/`. The outbound relay quarantines a message here when it deterministically fails to post, so
/// it can NEVER head-of-line-block the queue while still being PRESERVED (not dropped) for inspection.
/// Mirrors `markProcessed`; a missing source is ignored (already moved).
function markFailed(file) {
  const dir = path.dirname(file);
  const failed = path.join(dir, "failed");
  fs.mkdirSync(failed, { recursive: true });
  try {
    fs.renameSync(file, path.join(failed, path.basename(file)));
  } catch (e) {
    if (e.code !== "ENOENT") throw e;
  }
}

module.exports = { deliver, drain, markProcessed, markFailed, inboxDir, nextSeq, isValidAgentName };

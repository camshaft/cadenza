#!/usr/bin/env node
// Self-contained smoke test for the Cadenza fleet↔Slack bridge — the gate coverage that protects this
// integration. Runs with `node smoke.test.js` (zero deps, no network, no Slack tokens): it exercises the
// pure core — the fleet inbox round-trip (deliver → drain → processed, in the exact on-disk shape the Rust
// xtask reads) and the Slack↔fleet message shaping. Exits non-zero on any failure so it wires into CI /
// `npm test`.
//
// The Socket-Mode wiring in bridge.js is Bolt's concern (a live Slack WebSocket) and is not unit-tested
// here; bridge.js keeps its logic to calls into inbox.js + format.js, which ARE tested end to end below.

"use strict";

const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

// ⚠ DEP-FREE INVARIANT: require ONLY inbox.js + format.js here — both use node:-builtins only. NEVER
// require("./bridge.js") (it pulls in @slack/bolt): the CI `slack-bridge crate` job runs this with NO
// `npm install`, so any external-dep require throws "Cannot find module" and reds the check. If you need
// to test a helper that lives in bridge.js, MOVE it to format.js (the dep-free module) and require it from
// there — that's exactly what was done for isTransientSocketModeFault. Verify locally the CI way:
// `mv node_modules aside && node smoke.test.js` must still pass.
const { deliver, drain, markProcessed, markFailed, inboxDir, isValidAgentName } = require("./inbox.js");
const {
  parseOperatorMessage,
  renderFleetMessage,
  renderFleetMessagePlain,
  relayPlan,
  isContentPostError,
  RELAY_DEGRADE_AFTER,
  RELAY_QUARANTINE_AFTER,
  helpText,
  isTransientSocketModeFault,
  KNOWN_KINDS,
} = require("./format.js");

let passed = 0;
function test(name, fn) {
  try {
    fn();
    passed++;
    console.log(`  ok  ${name}`);
  } catch (e) {
    console.error(`FAIL  ${name}\n      ${e.stack || e.message}`);
    process.exitCode = 1;
  }
}

function tmpFleet() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "cadenza-slack-bridge-"));
}

// ---- inbox: delivery shape (must match xtask/src/fleet.rs `deliver`) ------------------------------

test("deliver writes one atomic JSON file with the fleet-message shape", () => {
  const dir = tmpFleet();
  const file = deliver(dir, "concierge", { from: "slack-bridge", kind: "note", subject: "hi", body: "hello there" });
  assert.ok(fs.existsSync(file), "file exists");
  const rec = JSON.parse(fs.readFileSync(file, "utf8"));
  assert.strictEqual(rec.from, "slack-bridge");
  assert.strictEqual(rec.to, "concierge");
  assert.strictEqual(rec.kind, "note");
  assert.strictEqual(rec.subject, "hi");
  assert.strictEqual(rec.body, "hello there");
  assert.strictEqual(rec.ref, "", "ref defaults to empty string (serde default)");
  assert.strictEqual(typeof rec.seq, "number");
});

test("delivered filename matches the Rust <seq:012>-<pid>-<kind>.json pattern", () => {
  const dir = tmpFleet();
  const file = deliver(dir, "pr-sync", { from: "slack-bridge", kind: "status", subject: "ping" });
  const base = path.basename(file);
  assert.match(base, /^\d{12}-\d+-status\.json$/, `filename was ${base}`);
});

test("no temp/dotfile is left behind after deliver", () => {
  const dir = tmpFleet();
  deliver(dir, "concierge", { from: "slack-bridge", kind: "note", subject: "x" });
  const names = fs.readdirSync(inboxDir(dir, "concierge"));
  assert.ok(!names.some((n) => n.startsWith(".")), "no leftover .tmp dotfile");
});

// ---- inbox: drain ordering + processed archival ---------------------------------------------------

test("drain returns messages oldest-first and markProcessed archives them", () => {
  const dir = tmpFleet();
  deliver(dir, "slack-bridge", { from: "concierge", kind: "answer", subject: "first" });
  deliver(dir, "slack-bridge", { from: "pr-sync", kind: "merged", subject: "second" });
  const pending = drain(dir, "slack-bridge");
  assert.strictEqual(pending.length, 2);
  assert.strictEqual(pending[0].msg.subject, "first", "oldest first");
  assert.strictEqual(pending[1].msg.subject, "second");

  markProcessed(pending[0].file);
  const after = drain(dir, "slack-bridge");
  assert.strictEqual(after.length, 1, "processed message no longer drained");
  assert.strictEqual(after[0].msg.subject, "second");
  assert.ok(fs.existsSync(path.join(inboxDir(dir, "slack-bridge"), "processed", pending[0].name)));
});

test("drain of a nonexistent inbox is empty, not an error", () => {
  const dir = tmpFleet();
  assert.deepStrictEqual(drain(dir, "nobody"), []);
});

test("drain ignores the processed/ dir and dotfiles", () => {
  const dir = tmpFleet();
  const box = inboxDir(dir, "slack-bridge");
  fs.mkdirSync(path.join(box, "processed"), { recursive: true });
  fs.writeFileSync(path.join(box, ".000000000001-1-note.json.tmp"), "{partial");
  deliver(dir, "slack-bridge", { from: "x", kind: "note", subject: "real" });
  const pending = drain(dir, "slack-bridge");
  assert.strictEqual(pending.length, 1, "only the real .json, not the dotfile-tmp");
  assert.strictEqual(pending[0].msg.subject, "real");
});

// ---- format: Slack → fleet parse ------------------------------------------------------------------

test("plain text becomes a note to the default recipient", () => {
  const i = parseOperatorMessage("what's the status?", "concierge");
  assert.deepStrictEqual(
    { to: i.to, kind: i.kind, subject: i.subject },
    { to: "concierge", kind: "note", subject: "what's the status?" },
  );
});

test("@agent retargets the recipient", () => {
  const i = parseOperatorMessage("@pr-sync are you merging?", "concierge");
  assert.strictEqual(i.to, "pr-sync");
  assert.strictEqual(i.kind, "note");
  assert.strictEqual(i.body, "are you merging?");
});

test("!kind sets a known message kind", () => {
  const i = parseOperatorMessage("!backlog try a dark mode", "concierge");
  assert.strictEqual(i.kind, "backlog");
  assert.strictEqual(i.to, "concierge");
  assert.strictEqual(i.body, "try a dark mode");
});

test("@agent and !kind combine", () => {
  const i = parseOperatorMessage("@design-jsx !assign build the JSX surface", "concierge");
  assert.strictEqual(i.to, "design-jsx");
  assert.strictEqual(i.kind, "assign");
  assert.strictEqual(i.body, "build the JSX surface");
});

test("an unknown !kind is left as literal note text (not silently mis-kinded)", () => {
  const i = parseOperatorMessage("!frobnicate do a thing", "concierge");
  assert.strictEqual(i.kind, "note", "unknown kind falls back to note");
  assert.ok(i.body.startsWith("!frobnicate"), "the !word stays in the text");
});

test("a !kind matches the whole word, not a prefix (parity with the Rust side)", () => {
  // `!statusfoo`/`!status-x` merely START with (or hyphen-extend) a known kind — they are NOT `status`
  // and must fall through to a literal note, else the operator's text is silently mis-kinded. Pins the
  // word-boundary contract so a future change to the !kind boundary-finding can't break it.
  for (const text of ["!statusfoo do", "!status-x do", "!asks it"]) {
    const i = parseOperatorMessage(text, "concierge");
    assert.strictEqual(i.kind, "note", `${text} is not a known kind → note`);
    assert.ok(i.body.startsWith("!"), `the !word stays literal in ${text}`);
  }
  // Sanity: the exact word IS honored, with or without following text.
  assert.strictEqual(parseOperatorMessage("!status", "concierge").kind, "status");
  assert.strictEqual(parseOperatorMessage("!status ping", "concierge").kind, "status");
});

test("KNOWN_KINDS is exactly the fleet protocol set (parity with the Rust side)", () => {
  // The `!kind` prefix is honored only for a fleet message kind (AGENTS-fleet.md's protocol table).
  // Pin the EXACT set so the Node and Rust `KNOWN_KINDS` can't silently drift apart — a `!newkind`
  // honored on one parser but mis-kinded to `note` on the other — and so adding/removing a fleet kind
  // forces a deliberate update on BOTH sides. The Rust side pins the identical set in format.rs.
  const got = [...KNOWN_KINDS].sort();
  const want = [
    "answer", "ask", "assign", "backlog", "issue",
    "merge-request", "merged", "note", "reject", "status",
  ].sort();
  assert.deepStrictEqual(got, want, "KNOWN_KINDS drifted from the fleet protocol set");
});

test("subject is the first line, capped; body keeps the whole text", () => {
  const long = "x".repeat(200);
  const i = parseOperatorMessage(`${long}\nsecond line`, "concierge");
  assert.ok(i.subject.length <= 120, "subject capped");
  assert.ok(i.subject.endsWith("…"), "capped subject is elided");
  assert.ok(i.body.includes("second line"), "body keeps full text");
});

test("empty-ish text yields a non-empty subject placeholder", () => {
  const i = parseOperatorMessage("@concierge   ", "concierge");
  assert.ok(i.subject.length > 0);
});

test("subject cap is by code points — an astral char is never split into a lone surrogate", () => {
  // 200 emoji (each a UTF-16 surrogate PAIR). Capping by `firstLine.length`/`slice` (UTF-16 units)
  // would cut mid-pair at index 117 → a lone surrogate = ill-formed subject, disagreeing with the
  // Rust `cap_subject` (Unicode scalars). Cap by code points so JS and Rust match (PR #405 class).
  const i = parseOperatorMessage("👍".repeat(200), "concierge");
  const cps = [...i.subject];
  assert.ok(cps.length <= 120, `capped by code points: ${cps.length}`);
  assert.ok(i.subject.endsWith("…"), "capped subject is elided");
  // A well-formed string round-trips through UTF-8 unchanged; a lone surrogate becomes U+FFFD.
  assert.ok(!Buffer.from(i.subject, "utf8").toString("utf8").includes("�"), "no lone surrogate");
  assert.strictEqual(cps[cps.length - 2], "👍", "the char before the ellipsis is a whole emoji");
});

// ---- format: fleet → Slack render -----------------------------------------------------------------

test("renderFleetMessage shows from, kind, subject, body, ref", () => {
  const s = renderFleetMessage({ from: "pr-sync", kind: "merged", subject: "landed", ref: "abc123", body: "all green" });
  assert.ok(s.includes("pr-sync"));
  assert.ok(s.includes("merged"));
  assert.ok(s.includes("landed"));
  assert.ok(s.includes("abc123"));
  assert.ok(s.includes("all green"));
});

test("renderFleetMessage omits absent ref/body cleanly", () => {
  const s = renderFleetMessage({ from: "concierge", kind: "answer", subject: "go with B" });
  assert.ok(s.includes("go with B"));
  assert.ok(!s.includes("``"), "no empty code span for a missing ref");
});

test("renderFleetMessage caps a huge body so the Slack post can't fail", () => {
  const s = renderFleetMessage({ from: "v-x", kind: "ask", subject: "big", body: "x".repeat(20000) });
  assert.ok([...s].length <= 3500, `capped under Slack's limit (code points): ${[...s].length}`);
  assert.ok(s.includes("truncated"), "elision marker present");
  assert.ok(s.includes("big"), "header/subject survives");
  // A normal short message is untouched.
  const short = renderFleetMessage({ from: "pr-sync", kind: "merged", subject: "landed", body: "all green" });
  assert.ok(!short.includes("truncated"));
});

test("renderFleetMessage caps by code points and never splits a surrogate pair (PR #405)", () => {
  // Astral-plane char (👍 = 1 code point, 2 UTF-16 units) repeated well past the cap. A UTF-16 `slice`
  // could cut mid-surrogate → an ill-formed string (a lone U+FFFD replacement char). Code-point slicing
  // must not, and must match the Rust chars() cap.
  const s = renderFleetMessage({ from: "v-x", kind: "ask", subject: "emoji", body: "👍".repeat(4000) });
  assert.ok([...s].length <= 3500, `capped by code points: ${[...s].length}`);
  assert.ok(s.includes("truncated"), "elision marker present");
  assert.ok(!s.includes("�"), "no replacement char — surrogate pair not split");
  // The kept body is whole 👍s (no lone surrogate): every char before the marker is intact.
  const body = s.slice(0, s.indexOf("\n…[truncated"));
  assert.ok(!/[\uD800-\uDFFF]/.test(body.replace(/\uD83D[\uDC4D]/g, "")), "no unpaired surrogate remains");
});

test("helpText names the default recipient and the prefixes", () => {
  const h = helpText("concierge");
  assert.ok(h.includes("concierge"));
  assert.ok(h.includes("@agent"));
  assert.ok(h.includes("!kind"));
});

// ---- SECURITY: agent-name path-traversal guard (PR #391) ------------------------------------------

test("isValidAgentName accepts real names and rejects traversal/separators", () => {
  for (const ok of ["pr-sync", "concierge", "v-slack-bridge", "fix-foo", "design-jsx", "a", "a1"]) {
    assert.ok(isValidAgentName(ok), `accept ${ok}`);
  }
  for (const bad of ["..", "../x", "../../etc", ".", ".hidden", "a.b", "a/b", "a\\b", "", "-x", "a b", "inbox/../x"]) {
    assert.ok(!isValidAgentName(bad), `reject ${JSON.stringify(bad)}`);
  }
});

test("inboxDir throws on a traversal agent name (no path built)", () => {
  assert.throws(() => inboxDir("/tmp/fleet", ".."), /path-traversal/);
  assert.throws(() => inboxDir("/tmp/fleet", "../../x"), /path-traversal/);
});

test("deliver to a traversal name throws and writes nothing outside the inbox", () => {
  const dir = tmpFleet();
  assert.throws(
    () => deliver(dir, "..", { from: "attacker", kind: "note", subject: "pwn" }),
    /path-traversal/,
  );
  // The fleet dir must have no stray files/dirs created by the attempt.
  assert.deepStrictEqual(fs.readdirSync(dir), [], "no traversal write happened");
});

test("a @.. retarget never becomes the recipient (parser guard)", () => {
  const i = parseOperatorMessage("@.. hi", "concierge");
  assert.strictEqual(i.to, "concierge", "traversal name never becomes recipient");
  assert.ok(isValidAgentName(i.to));
  const i2 = parseOperatorMessage("@../../etc pwn", "concierge");
  assert.strictEqual(i2.to, "concierge");
});

test("every parsed recipient is a valid agent name", () => {
  for (const msg of ["@pr-sync go", "@.. x", "@a/b y", "plain", "@-bad z", "@design-jsx !assign do"]) {
    const i = parseOperatorMessage(msg, "concierge");
    assert.ok(isValidAgentName(i.to), `unsafe recipient ${JSON.stringify(i.to)} from ${msg}`);
  }
});

// ---- RELIABILITY: transient Socket Mode fault is survived, not crash-looped -----------------------

test("isTransientSocketModeFault matches the finity connecting-disconnect crash (the real crash)", () => {
  // The exact error that crash-looped the daemon 5x/min: finity throws this synchronously from the
  // WebSocket handler when Slack disconnects mid-handshake. It self-heals (client reconnects), so the
  // process must survive it rather than exit and risk tripping systemd's StartLimitBurst.
  const real = new Error("Unhandled event 'server explicit disconnect' in state 'connecting'.");
  assert.ok(isTransientSocketModeFault(real), "the real crash must be classified transient");
  // A string reason (unhandledRejection can pass a non-Error) is handled too.
  assert.ok(isTransientSocketModeFault("Unhandled event 'server explicit disconnect' in state 'connecting'."));
  // Other connection-lifecycle races in the same finity shape also self-heal.
  assert.ok(isTransientSocketModeFault(new Error("Unhandled event 'reconnect' in state 'connected'.")));
});

test("isTransientSocketModeFault does NOT swallow genuine faults (still exits)", () => {
  for (const fatal of [
    new Error("invalid_auth"),
    new Error("Cannot read properties of undefined (reading 'foo')"),
    new Error("Unhandled event 'x' in state 'y'."), // finity shape but NOT a connection event → don't swallow
    "some random string",
    undefined,
    null,
  ]) {
    assert.ok(!isTransientSocketModeFault(fatal), `must NOT swallow ${JSON.stringify(String(fatal))}`);
  }
});

// ---- RESILIENCE: outbound relay must never head-of-line-block on a poison message ----------------

test("markFailed dead-letters a message and it no longer drains (preserved, not dropped)", () => {
  // A quarantined un-postable message moves to failed/ — preserved for inspection, and removed from the
  // drain so it can't block the relay queue (the ~11h/57-message wedge this fixes).
  const dir = tmpFleet();
  const file = deliver(dir, "slack-bridge", { from: "v-x", kind: "ask", subject: "poison" });
  assert.strictEqual(drain(dir, "slack-bridge").length, 1);
  markFailed(file);
  assert.strictEqual(drain(dir, "slack-bridge").length, 0, "dead-lettered message no longer drains");
  const dead = path.join(inboxDir(dir, "slack-bridge"), "failed", path.basename(file));
  assert.ok(fs.existsSync(dead), "message preserved under failed/");
  assert.strictEqual(JSON.parse(fs.readFileSync(dead, "utf8")).subject, "poison");
});

test("renderFleetMessage escapes Slack control entities (&, <, >) in content, not markers", () => {
  // Slack treats &, <, > as text control chars; unescaped they mis-parse / internal_error (a real note
  // with <512KiB/>512KiB failed the rich post in production). Content must be HTML-escaped; markers stay.
  const m = { from: "v-x", kind: "note", subject: "shrink <512KiB & >0", body: "keep Rc<[T]> & Vec<T>" };
  const s = renderFleetMessage(m);
  assert.ok(s.includes("&lt;512KiB"), `< escaped: ${s}`);
  assert.ok(s.includes("&gt;0") && s.includes("Vec&lt;T&gt;"), `> and generics escaped: ${s}`);
  assert.ok(s.includes("&amp;"), `& escaped: ${s}`);
  assert.ok(!s.includes("<") && !s.includes(">"), `no raw angle brackets: ${s}`);
  assert.ok(s.includes("*v-x*") && s.includes("_note_"), "mrkdwn markers untouched");
  // Escape & FIRST so a produced &lt; is never re-escaped into &amp;lt;.
  const dbl = renderFleetMessage({ from: "a", kind: "note", subject: "s", body: "A && B < C" });
  assert.ok(dbl.includes("A &amp;&amp; B &lt; C") && !dbl.includes("&amp;lt;"), `no double-escape: ${dbl}`);
});

test("relayPlan escalates normal → degraded → quarantine at the (drift-proof) thresholds", () => {
  // Escalation must be finite so a poison message is dead-lettered after a bounded number of polls and can
  // NEVER block the queue forever. Assert against the exported constants (not literals) so a future retune
  // can't silently drift the Rust and Node relays apart — the boundaries themselves are the contract.
  assert.strictEqual(relayPlan(0), "normal");
  assert.strictEqual(relayPlan(RELAY_DEGRADE_AFTER - 1), "normal");
  assert.strictEqual(relayPlan(RELAY_DEGRADE_AFTER), "degraded");
  assert.strictEqual(relayPlan(RELAY_QUARANTINE_AFTER - 1), "degraded");
  assert.strictEqual(relayPlan(RELAY_QUARANTINE_AFTER), "quarantine");
  assert.strictEqual(relayPlan(RELAY_QUARANTINE_AFTER + 100), "quarantine");
  assert.ok(RELAY_DEGRADE_AFTER < RELAY_QUARANTINE_AFTER, "must degrade before quarantining");
  // PATIENCE invariant: quarantine must be many minutes of SUSTAINED failure (polls are ~2s), so a brief
  // Slack internal_error blip can't false-dead-letter a good message (the queue never wedges regardless,
  // since a failing message is skipped past — quarantine only bounds a genuinely-undeliverable message).
  assert.ok(RELAY_QUARANTINE_AFTER >= 60, `quarantine must be patient (>=~2min), got ${RELAY_QUARANTINE_AFTER}`);
});

test("isContentPostError classifies Slack API errors as content, transport/rate-limit as transient", () => {
  // Only a CONTENT error (Slack answered with err.data.error, e.g. internal_error — the ~11h wedge) advances
  // a message toward degrade/quarantine. Transport faults + rate limiting are transient → retry in place.
  assert.ok(isContentPostError({ data: { error: "internal_error" } }), "internal_error is content");
  assert.ok(isContentPostError({ data: { error: "msg_too_long" } }), "msg_too_long is content");
  assert.ok(!isContentPostError({ data: { error: "ratelimited" } }), "ratelimited is transient");
  assert.ok(!isContentPostError({ code: "slack_webapi_rate_limited_error", retryAfter: 30 }), "rate limit is transient");
  assert.ok(!isContentPostError(new Error("socket hang up")), "transport error is transient");
  assert.ok(!isContentPostError({ code: "slack_webapi_request_error" }), "request error is transient");
  assert.ok(!isContentPostError(undefined) && !isContentPostError(null), "no error → not content");
});

test("renderFleetMessagePlain strips mrkdwn control chars and hard-truncates", () => {
  // The degraded fallback must carry NO mrkdwn/entity control chars (so it can't re-trigger Slack's parse
  // quirk) and must be short (the proven-safe truncation that posted where the rich render failed).
  const m = { from: "pr-sync", kind: "ask", subject: "decide *A* or `B`", body: "use <https://x> & _em_ ~s~ | p" };
  const s = renderFleetMessagePlain(m);
  for (const bad of ["*", "_", "~", "`", "<", ">", "&", "|"]) {
    assert.ok(!s.includes(bad), `plain render must not contain ${bad}: ${s}`);
  }
  assert.ok(s.includes("pr-sync") && s.includes("ask"), "keeps who/what");
  assert.ok([...s].length <= 400, "bounded to the proven-safe length");

  const huge = { from: "v-x", kind: "ask", subject: "big", body: "x".repeat(9000) };
  const big = renderFleetMessagePlain(huge);
  assert.ok([...big].length <= 400, "huge body is capped");
  assert.ok(big.includes("truncated"), "elision marker present");
  assert.ok(big.includes("v-x"), "header survives the cap");
});

console.log(`\n${passed} checks passed`);
if (process.exitCode) console.error("SMOKE TEST FAILED");

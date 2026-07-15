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

const { deliver, drain, markProcessed, inboxDir, isValidAgentName } = require("./inbox.js");
const { parseOperatorMessage, renderFleetMessage, helpText } = require("./format.js");

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
  assert.ok(s.length <= 3500, `capped under Slack's limit: ${s.length}`);
  assert.ok(s.includes("truncated"), "elision marker present");
  assert.ok(s.includes("big"), "header/subject survives");
  // A normal short message is untouched.
  const short = renderFleetMessage({ from: "pr-sync", kind: "merged", subject: "landed", body: "all green" });
  assert.ok(!short.includes("truncated"));
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

console.log(`\n${passed} checks passed`);
if (process.exitCode) console.error("SMOKE TEST FAILED");

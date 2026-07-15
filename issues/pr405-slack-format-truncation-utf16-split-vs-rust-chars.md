# PR review comments — mirrored from GitHub PR #405 (Copilot inline)

- **PR:** #405 (OPEN at triage; file on trunk)
- **Files:** `fleet/slack-bridge/format.js:83` (renderFleetMessage), `fleet/slack-bridge/smoke.test.js:171`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591071570, 3591071595
- **Links:** https://github.com/camshaft/cadenza/pull/405#discussion_r3591071570 , #discussion_r3591071595

## Comments (verbatim)
> `renderFleetMessage` caps by UTF-16 code units (`out.length`) and truncates with `slice`, which can split a surrogate pair and produce an ill-formed string. It also doesn't match the Rust side, which caps by Unicode scalar count (`chars()`), so a non-ASCII message could be truncated differently in JS vs Rust. Use code-point counting/truncation (`[...str]`) to avoid splitting and to align with the Rust cap semantics.
>
> This test asserts the Slack cap using `s.length` (UTF-16 code units), which can diverge from the intended "character" cap (and from the Rust `chars()` counting) for astral-plane characters. Assert on code points too.

## Liaison triage — CONFIRMED against trunk
Confirmed in format.js: `if (out.length <= SLACK_TEXT_CAP) return out; ... return out.slice(0,
SLACK_TEXT_CAP - marker.length) + marker;` — caps + truncates by UTF-16 code units, so an astral-plane
char (surrogate pair) at the boundary is SPLIT into an ill-formed string, and the cap disagrees with the
Rust bridge's `chars()` (Unicode-scalar) counting. Low severity (only bites a >cap message with
non-BMP chars at the boundary — rare for fleet text), but a real correctness + JS/Rust-parity nit. FIX:
count/truncate by code points (`[...str].length` / `[...str].slice(...)`) to match Rust and avoid
splitting; update the smoke test to assert on code points. Fleet-tooling territory (v-fleet-tooling owns
slack-bridge). Fix on `trunk`. Quotes + links in queue file.

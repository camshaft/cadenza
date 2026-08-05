# PR #1954 review — cdz-agent-host/src/admin_wire.rs (v-agent-harness-host) — MERGED — doc-accuracy [VERIFIED]

https://github.com/camshaft/cadenza/pull/1954 — MERGED (admin wire codec + my #1949 review follow-up).
Copilot (id 3709562863) flags `decode_frame`'s doc contradicts the code on the short-buffer case.

## `decode_frame` doc's first paragraph says short-buffer → `Err`, but the code (and the doc's SECOND paragraph) returns `Ok(None)` (Copilot, admin_wire.rs:205) — doc-accuracy [VERIFIED, self-contradictory doc]
> `decode_frame`'s doc comment says it returns `Err` when the buffer is too short for the declared frame,
> but the implementation returns `Ok(None)` for incomplete frames. This mismatch can mislead streaming
> callers about expected control flow.

VERIFIED on trunk, and the doc is INTERNALLY contradictory:
- Para 1: "`Err` if the buffer is too short for the declared frame (an incomplete read — the caller should
  read more bytes) …"
- Para 2: "Returns `Ok(None)` when `buf` doesn't yet hold a full frame (fewer than 4 header bytes, or
  fewer than the declared body length) — the 'need more bytes' signal a streaming reader loops on."
- Code: `if buf.len() < 4 { return Ok(None) }` and `if buf.len() < end { return Ok(None) }` — BOTH
  short-buffer cases return `Ok(None)`. `Err` is reserved for oversized length (`> MAX_FRAME_LEN`) and bad
  JSON only.

So the CODE and Para 2 agree (`Ok(None)` = need more bytes); Para 1's "too short → Err" clause is simply
wrong and contradicts Para 2 three lines later. A streaming caller reading only the first sentence would
expect an `Err` for a partial read and mis-handle the loop control flow. LOW/doc-accuracy. Fix: delete the
"too short for the declared frame (an incomplete read …)" clause from Para 1's `Err` list — leaving `Err`
for oversized-length + bad-JSON, which is what the code does; Para 2 already documents the `Ok(None)`
short-buffer behavior correctly. v-agent-harness-host owns cdz-agent-host/src.

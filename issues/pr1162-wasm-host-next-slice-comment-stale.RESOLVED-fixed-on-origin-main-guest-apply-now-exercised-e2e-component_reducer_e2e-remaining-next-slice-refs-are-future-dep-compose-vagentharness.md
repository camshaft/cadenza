# PR #1162 review comment — cdz-kernel/src/wasm_host.rs (v-agent-harness) — OPERATOR TOP PRIORITY

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1162
(PR: "cand: v-agent-harness — (OPERATOR TOP PRIORITY)").

## "next slice" comment stale — guest apply path is now exercised e2e (Copilot, wasm_host.rs:655) — doc
> This comment is now outdated: the end-to-end guest `apply` path and a real guest fixture are
> exercised in `tests/component_reducer_e2e.rs`, so it's no longer "the next slice". Updating it
> avoids misleading readers about current coverage/status.

Doc: the comment predates the e2e fixture landing — reword so it reflects that the guest apply path
is now covered by `component_reducer_e2e.rs` rather than described as future work.

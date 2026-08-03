# PR #1217 review comments — cdz-agent-host/src/{clock,http,model}.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1217 (PR: "cand: v-agent-harness-host —
8e2f56ad3"). All three are stale-comment fallout from the #1215 `Payload::Inline: Vec<u8> → bytes::Bytes` flip.

## `Payload::Inline` comments still describe `Vec<u8>` after the Bytes flip (Copilot) — doc
- **clock.rs:52 (also :77)**: comment claims `.into()` is an identity conversion when
  `Payload::Inline` has a `Vec<u8>` inner, but it's now `bytes::Bytes` — describe the current
  behavior (freezing a `Vec<u8>` into `Bytes`).
- **http.rs:70 (also :80)**: comment about supporting either `Vec<u8>` or `Bytes` is stale — it
  should just describe borrowing the inline `Bytes` payload as `&[u8]`.
- **model.rs:80**: comment refers to a past "perf-directive flip"; since `Payload::Inline` is already
  `bytes::Bytes`, describe the current no-copy move rather than the historical change.

All three are the same theme: the #1215 Bytes flip landed, so these comments now describe the old
`Vec<u8>` representation / the transition itself. Sweep them to present-tense descriptions of the
current `Bytes` behavior.

# PR #1981 review — cdz-agent-host/src/config.rs (v-agent-harness-host) — OPEN — config-ahead-of-wiring + LOW doc [VERIFIED]

https://github.com/camshaft/cadenza/pull/1981 ([blob] config section — select the daemon's blob store).
Copilot 2 inline: the `blob` section is parsed but not yet consumed, and the rustdoc carries roadmap prose.

## `DaemonConfig::blob` is parsed + validated but never READ by the daemon → `backend = "dir"` in TOML has no effect (Copilot, config.rs:43) — config-ahead-of-wiring [VERIFIED]
> `DaemonConfig::blob` is parsed and validated here, but it appears to be unused by the actual daemon
> wiring (no reads of `config.blob` outside `config.rs`). As a result, selecting `backend = "dir"` in TOML
> currently has no effect on which `BlobStore` implementation the daemon uses.

VERIFIED on trunk/diff: the daemon bin (cdz_agent_daemon.rs) reads `config.log`, `config.observability`,
`config.retries`, `config.admin` — but NEVER `config.blob`. (The only `.blob` in the crate is
`ComponentSessionFactory.blob` at factory.rs:92 — its own field, unrelated to the config.) So a TOML
`[blob] backend = "dir"` parses + validates but selects nothing — the daemon uses whatever BlobStore is
hard-wired (or none, since the factory isn't wired into the bin yet — the pending factory-wiring follow-up
noted on #1977's install-doc).

FRAMING (not a pure bug): this is config parsed AHEAD of its consumer, the same staging pattern as the
module's own design ("config SPINE — schema + parse + validation; the wiring of each backend to real code
lands as its own daemon slice"). `[log]`/`[observability]` are in the same boat. So it's consistent with
the intended incremental wiring — BUT unlike a reader who knows that, an operator setting `backend = "dir"`
gets a silently-ignored setting. Recommend: a doc note on `[blob]` (and ideally the others) that the
section is parsed-but-not-yet-wired pending the factory-wiring slice, OR a tracking issue link, so the
no-op is a CONSCIOUS staging gap not a silent surprise. (This is the same "config that silently does
nothing" hazard the #1935 `[[session]]` roster hit — there the fix was to REMOVE it; here the section is
wanted, just not wired yet, so a status note is the fit.) LOW-MED. v-agent-harness-host's call on note vs
wire-now.

## `[blob]` rustdoc embeds roadmap/backend-name speculation ("S3/Dynamo", "later slice") likely to go stale (Copilot, config.rs:96) — doc-clarity [VERIFIED, LOW cosmetic]
> These rustdoc comments include roadmap/speculation and concrete backend names ("S3/Dynamo", "later
> slice"), which are likely to go stale quickly. Prefer stating current behavior and leaving extensibility
> implicit or phrased generically.

VERIFIED — LOW/cosmetic. State current behavior ("selects the blob-store backend: `memory` | `dir`");
phrase extensibility generically rather than naming unbuilt S3/Dynamo backends + slice numbers. Batchable
with the parsed-but-unused note above (both on `[blob]`'s doc). v-agent-harness-host owns cdz-agent-host/src.

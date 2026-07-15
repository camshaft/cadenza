# PR review comment — mirrored from GitHub PR #391 (Copilot inline)

- **PR:** #391 (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/cdz/src/lsp.rs:60`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590196691
- **Link:** https://github.com/camshaft/cadenza/pull/391#discussion_r3590196691

## Comment (verbatim)
> The comment says the server announces name/version via `InitializeResult`, but the `InitializeResult` is currently constructed and then discarded; the actual `initialize` response only includes `capabilities()` because that's what gets passed to `connection.initialize(...)`. This is misleading and also prevents clients from seeing `serverInfo`.

## Liaison triage — CONFIRMED against trunk
Confirmed in lsp.rs: `connection.initialize(server_capabilities)` is called with ONLY the capabilities
value, then an `InitializeResult { capabilities, server_info: Some(ServerInfo{name:"cdz-lsp", version}) }`
is built and immediately dropped via `let _ = ...`. So `serverInfo` is never actually sent to the
client, even though the surrounding comment claims the server "announces our name/version in the
InitializeResult". Either the comment overclaims (harmless-but-misleading) or the intent was to send
serverInfo (a small LSP-conformance gap — some clients surface it). New `cdz` LSP server; no dedicated
LSP vertical → route to `corpus-bugfix` PM. Fix on `trunk`. Quote + link in queue file.

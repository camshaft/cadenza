# PR #1881 review comment — cdz-kernel/src/name_store.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1881 (MERGED — name-store durability doc, my #1858/#1868 lineage).

## Doc links `LogSink::append` but the quoted v0 wording is on the disk backend's `LogStore::append` (Copilot, name_store.rs:278) — doc/accuracy
> The doc links to `LogSink::append`, but the quoted v0 wording ("write(frame) + flush to the OS", the
> "v0 is NOT fsync" note) is documented on the disk backend's `LogStore::append` API, not LogSink.
The durability-boundary doc (from the #1858/#1868 reword chain) links the wrong symbol — the v0
write+flush-not-fsync semantics it quotes live on `LogStore::append` (disk backend), not `LogSink::append`.
Repoint the intra-doc link to the correct `LogStore::append` so the quoted contract matches its source
(and the link resolves to where the wording actually is). LOW/doc. Fix-forward.

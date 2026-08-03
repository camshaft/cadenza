# PR #1640 review comment — rcdzc/src/backend/wasm/select.rs (v-rust-backend) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1640 (MERGED).

## Comment now covers String+Bytes host-args but names only `assemble_host_mem` as the memory provider (Copilot, select.rs:11426) — doc
> This comment now applies to both String and Bytes host-arg marshalling, but it specifically names
> `assemble_host_mem` as the provider of the shared memory. Bytes/string args can also flow through the
> host+runtime path.

Doc-scope drift after the String→String+Bytes generalization: the comment names only `assemble_host_mem`
though the shared-memory can also come via the host+runtime path. Broaden the comment to both providers.
LOW/doc, fix-forward.

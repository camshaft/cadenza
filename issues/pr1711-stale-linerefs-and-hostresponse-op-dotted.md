# PR #1711 review comments — rcdzc/src/{backend/wasm/select,tests}.rs (v-effects) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1711 (MERGED).

## 1+2. Comments reference the Core::Call arg loop as "~6330" — stale, it's `emit_call_args` far later (Copilot, select.rs:11420 + tests.rs:66401) — doc/durability
> The new comments reference the Core::Call arg loop as "~6330", but the actual argument-emission helper
> is `emit_call_args` much later in the file. The stale line anchor will mislead.

Two sites (select.rs:11420, tests.rs:66401) — the recurring line-anchor rot pattern. Reference the helper
by NAME (`emit_call_args`) instead of a line number. LOW/doc.

## 3. HostResponse.op documented as dotted (`E.op`) but comment uses bare "send" (Copilot, tests.rs:66428) — doc/consistency
> `cdz_run::HostResponse.op` is documented as a dotted operation name (`E.op`). Using only "send" here is
> inconsistent with the rest of the harness's observed-call naming.

Use the dotted form (`E.send` / the actual effect-qualified name) to match the HostResponse.op contract +
the rest of the harness. LOW/doc.

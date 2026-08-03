# PR #1619 review comments — rcdzc/src/backend/wasm/{serialize,mod}.rs (v-rust-backend / wasm) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1619 (host-arg Bytes scaffolding — HostParam::Bytes).

## 1. slot_count recomputed from f.params, duplicating the per-core-slot logic (Copilot, serialize.rs:72) — cleanliness
> `slot_count` is recomputed from `f.params` even though `params` is already built as one entry per core
> slot (two i32s for Str/Bytes). This duplicates logic and risks drifting. Prefer `params.len()`.

`params` is already one-entry-per-core-slot, so `let slot_count = params.len();` removes the duplicated
count-from-f.params logic and the drift risk. LOW/cleanliness.

## 2. set_needs_memory only checks HostParam::Str — a future Bytes-param won't get the memory envelope (Copilot, mod.rs:891) — latent-correctness
> `HostParam::Bytes` lowers to the same `(ptr,len)` shared-memory core shape as `HostParam::Str`, but
> `host::set_needs_memory` only checks for `HostParam::Str`. If/when a Bytes-param host import becomes
> possible, the envelope selection will be wrong unless `set_needs_memory` treats Bytes as memory-requiring.

This PR is Bytes-param SCAFFOLDING — so the latent gap is worth closing now while it's fresh: `HostParam::
Bytes` needs the same `(ptr,len)` shared-memory envelope as `Str`, but `set_needs_memory` gates only on
`Str`. When a Bytes-param host import lands, it'll miss the memory envelope → wrong lowering. LOW-MED (no
current trigger, but this is the scaffolding PR — add Bytes to set_needs_memory's check now). Recommend
v-rust-backend fix while wiring the rest of HostParam::Bytes.

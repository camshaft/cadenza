# PR #1723 review comments — rcdzc/src/backend/rust/mod.rs (v-rust-backend) — OPEN

https://github.com/camshaft/cadenza/pull/1723 (reject a closure escaping an effect on the Rust backend).

## 1. New direct dep from rust backend into backend::wasm (Copilot, mod.rs:607) — architecture
> This introduces another direct dependency from the Rust backend into `backend::wasm`
> (`crate::backend::wasm::host::collect_host_imports`). [The backends should stay decoupled.]

Reaching into `backend::wasm::host` from `backend::rust` couples the two backends. If `collect_host_imports`
is backend-agnostic host analysis, consider hoisting it to a shared module (`backend::host` or similar)
rather than rust→wasm. LOW-MED/architecture — recommend v-rust-backend weigh the coupling (may be
acceptable as a pragmatic reuse; flagging the direction).

## 2. `escaping` only used via `.first()` — can early-break the scan (Copilot, mod.rs:609) — efficiency
> `escaping` is only used via `escaping.first()`, so once at least one host import is collected we can stop
> scanning additional lifted bodies. Breaking early avoids the full walk.

If only presence-of-first matters, short-circuit the scan on the first collected import instead of
collecting all. LOW/efficiency. Fix-forward.

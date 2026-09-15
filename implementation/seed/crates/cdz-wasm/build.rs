//! Give the browser compiler cdylib a NATIVE-SIZED shadow stack.
//!
//! `wasm32-unknown-unknown` defaults to a 1 MiB linear-memory shadow stack (wasm-ld's `-z stack-size`),
//! whereas the native compiler runs on the platform's ~8 MiB main-thread stack. The compiler front-end +
//! rcdzc lower recursively over the AST, and some perfectly ordinary programs — e.g. a `match` with a
//! `guard` arm, or an equality over a nested arithmetic expression like `(= (+ n 1) n)` — recurse deep
//! enough to exceed 1 MiB. Native (8 MiB) compiles them fine; the wasm build overflowed its shadow stack
//! and trapped `memory access out of bounds` DURING compile, which also left the wasm instance's shadow
//! stack corrupted so EVERY subsequent compile in the same instance trapped too (an instance-poisoning
//! cascade that read like a memory-accumulation "cliff"). This surfaced as the guide-examples CI red:
//! three otherwise-trivial guide examples failed to compile in the browser compiler while compiling +
//! running fine natively (root-caused 2026-09-15, v-cadenza-ci).
//!
//! Raising the shadow stack to 16 MiB (2× native, comfortable headroom for the deepest guide/playground
//! programs) makes the browser compiler accept exactly what the native compiler does. `rustc-cdylib-link-arg`
//! (a build-script link arg) is used deliberately: it is ADDITIVE and is NOT clobbered by a `RUSTFLAGS`
//! env var, so it takes effect in every build path — the nix `cdzWasmPkg` derivation (which exports its own
//! `RUSTFLAGS` for path-remapping), a local `cargo build`, and `wasm-pack` — with no per-caller flag needed.
//!
//! NOTE: the underlying compiler recursion depth for such small inputs is itself a regression worth bounding
//! compiler-side (tracked with the compiler owner); this stack size is the correct, low-risk fix for the
//! browser compiler regardless, since it should match the native stack it mirrors.

fn main() {
    // 16 MiB, in bytes. wasm-ld consumes `-z stack-size=<bytes>`.
    println!("cargo::rustc-cdylib-link-arg=-zstack-size=16777216");
}

//! The guest component's embedded wasm allocator — mirrors cdz-nfc / cdz-runtime. A no_std wasm component
//! needs a `#[global_allocator]`; talc's single-threaded dynamic wasm allocator is the same `GlobalAlloc`
//! the runtime ships (a component instance is single-threaded). Isolated to this one file so it is
//! swappable; no reducer code references talc.

#[global_allocator]
static ALLOCATOR: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();

// A no_std wasm component needs a panic handler. This fixture cannot recover from one (a reducer fold
// should not panic), so trap immediately. (cdz-nfc / cdz-runtime instead build std with
// `panic=immediate-abort`, which compiles panic machinery out entirely; a fixture does not need that
// byte-determinism, so a plain handler keeps the build toolchain-light — no build-std / RUSTC_BOOTSTRAP.)
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

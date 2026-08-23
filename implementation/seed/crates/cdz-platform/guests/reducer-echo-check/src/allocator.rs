//! The guest component's embedded wasm allocator — identical to reducer-echo / cdz-nfc / cdz-runtime. A
//! no_std wasm component needs a `#[global_allocator]`; talc's single-threaded dynamic wasm allocator is the
//! same `GlobalAlloc` the runtime ships (a component instance is single-threaded). Isolated to this one file
//! so it is swappable; no checker code references talc.

#[global_allocator]
static ALLOCATOR: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();

// A no_std wasm component needs a panic handler. A checker should not panic (a malformed log is a fail
// verdict, not a trap), so trap immediately if it ever does. Plain handler (no build-std) keeps the build
// toolchain-light, same as reducer-echo.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

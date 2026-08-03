//! The NFC component's embedded wasm allocator — mirrors cdz-runtime's `allocator.rs`. A no_std wasm
//! component needs a `#[global_allocator]`; talc's single-threaded dynamic wasm allocator is the same
//! lock-free `GlobalAlloc` cdz-runtime ships (a component instance is single-threaded). Isolated to this
//! one file so it is swappable; no NFC code references talc.

#[global_allocator]
static ALLOCATOR: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();

//! The span sidecar — the source-position side-table (`SpanData`/`LineStarts` + its encode/decode),
//! now living in the shared `cadenza-compile-abi` crate (a compile-boundary type: a debug compile
//! takes `spans` as a kinded input, and `cdz` passes it across the delegate boundary). Re-exported
//! here so `crate::spans::…` and `rcdzc::spans::…` stay byte-stable and every consumer keeps resolving.
pub use cadenza_compile_abi::spans::*;

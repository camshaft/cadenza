//! First relocated behavior group using `call_traps` — the runtime trap-assertion driver. A narrowing
//! `.of` conversion range-checks at run time: an in-range argument returns, an out-of-range one traps.
//! Witnesses that `common::call_traps` distinguishes the two (and that the value path is correct).

use crate::common::{call_traps, compile_component, run_returns_with};
use wasmtime::component::Val;

#[test]
fn a_runtime_narrowing_of_conversion_traps_out_of_range_and_returns_in_range() {
    let component =
        compile_component("(module m (def (narrow (: n Int64)) (Int8.of n)) (export narrow))");
    // In range (fits i8): returns the narrowed value, no trap.
    assert!(
        !call_traps(&component, "narrow", &[Val::S64(5)]),
        "Int8.of 5 is in range — must not trap"
    );
    assert_eq!(
        run_returns_with::<i8>(&component, "narrow", &[Val::S64(5)]),
        5
    );
    // Out of range (> i8::MAX): the runtime range-check traps.
    assert!(
        call_traps(&component, "narrow", &[Val::S64(999)]),
        "Int8.of 999 is out of range — must trap"
    );
}

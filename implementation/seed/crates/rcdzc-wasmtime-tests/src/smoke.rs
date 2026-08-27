//! First relocated group — proves the relocation wiring end-to-end: a program compiled through
//! rcdzc's public API, instantiated and run under wasmtime, its result decoded on the Rust side, plus
//! a `wasmparser` opcode-shape probe. If these pass here, the ~1109 in-crate wasmtime tests can move
//! wholesale without a corpus round-trip.

use crate::common::{compile_component, count_opcode, run_returns, run_returns_with};
use wasmtime::component::Val;

#[test]
fn a_scalar_export_returns_its_constant() {
    let component = compile_component("(module m (def (main) 42) (export main))");
    assert_eq!(run_returns::<i64>(&component, "main"), 42);
}

#[test]
fn a_parameterized_export_adds_its_arguments() {
    let component =
        compile_component("(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))");
    assert_eq!(
        run_returns_with::<i64>(&component, "add", &[Val::S64(2), Val::S64(3)]),
        5,
        "add(2,3) crosses two s64 args and returns their sum"
    );
}

#[test]
fn opcode_probe_sees_the_emitted_multiply() {
    let component = compile_component("(module m (def (sq (: n Int64)) (* n n)) (export sq))");
    // Value still correct...
    assert_eq!(
        run_returns_with::<i64>(&component, "sq", &[Val::S64(7)]),
        49
    );
    // ...and the emission-shape probe reaches the core module and finds the multiply.
    let muls = count_opcode(&component, |op| matches!(op, wasmparser::Operator::I64Mul));
    assert!(muls >= 1, "sq must emit at least one i64.mul, saw {muls}");
}

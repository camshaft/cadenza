//! `cdz-rustc` as a WebAssembly component: a thin adapter that implements the generated
//! `compile` export by delegating to the pure `cdz-compiler` core. All compilation logic lives
//! in the core; this file only bridges the component-model `list<u8> → result<list<u8>,
//! list<diagnostic>>` ABI to `cdz_compiler::codegen::compile`.

#[allow(warnings)]
mod bindings;

use bindings::exports::cadenza::compiler::compile::{Diagnostic, Guest};

struct Component;

impl Guest for Component {
    /// Compile a program's canonical binary AST to its component bytes, or a list of
    /// diagnostics on rejection/decline (distinguishing a type error from a not-yet-compiled
    /// construct by the diagnostic's code).
    fn compile(ast: Vec<u8>) -> Result<Vec<u8>, Vec<Diagnostic>> {
        match cdz_compiler::codegen::compile(&ast) {
            Ok(bytes) => Ok(bytes),
            Err(d) => Err(vec![Diagnostic {
                code: d.code().unwrap_or("CDZ0000").to_string(),
                message: d.message().to_string(),
            }]),
        }
    }
}

bindings::export!(Component with_types_in bindings);

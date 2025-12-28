//! Cadenza Compiler
//!
//! This crate implements the Cadenza compiler, which compiles Cadenza source code
//! to WASM components with WIT interfaces. The compiler follows a multi-phase
//! pipeline that progressively annotates the AST through each phase.
//!
//! ## Compilation Pipeline
//!
//! 1. **Parse** - Convert source text to CST (via cadenza-syntax)
//! 2. **Evaluate** - Macro expansion and module building
//! 3. **Type Check** - Infer types, traits, effects, dimensions, contracts
//! 4. **Ownership Analysis** - Track linear types and insert deleters
//! 5. **Monomorphize** - Specialize generics and resolve traits
//! 6. **Lambda Lift** - Convert closures to top-level functions
//! 7. **Contract Instrumentation** - Insert or prove contracts
//! 8. **Generate WIT** - Export interface for component
//! 9. **Emit WASM** - Generate executable code

mod error;
mod object;

pub use error::Error;
pub use object::{Object, Value};

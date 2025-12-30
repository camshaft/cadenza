use cadenza_codegen::{emit, rustfmt};
use cadenza_meta::*;
use std::{env, path::Path};

fn main() {
    // Define the Cadenza language semantics
    let semantics = define_cadenza_semantics();

    // Analyze semantic definitions
    let analysis = analyze(&semantics);

    // Check for errors
    if !analysis.errors.is_empty() {
        eprintln!("Semantic definition errors:");
        for error in &analysis.errors {
            eprintln!("  Query: {}", error.query.as_deref().unwrap_or("<unknown>"));
            eprintln!("    {}", error.message);
        }
        panic!("Cannot proceed with invalid semantic definitions");
    }

    // Generate Rust code
    let code = generate(&semantics, &analysis);
    let code = rustfmt(&code.to_string());
    let out_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = Path::new(&out_dir).join("src");

    emit(out_dir.join("generated/semantics.rs"), &code);

    emit(out_dir.join("generated/mod.rs"), "pub mod semantics;");
}

/// Define the complete semantic specification for the Cadenza language
fn define_cadenza_semantics() -> Semantics {
    Semantics::new()
        .add_query(define_eval_query())
        .add_query(define_type_of_query())
}

/// Define the eval query: evaluate an expression to a value
fn define_eval_query() -> Query {
    query("eval")
        .input(value_type())
        .output(result(value_type(), diagnostics()))
        .extern_impl() // External for now - will expand incrementally
        .build()
}

/// Define the type_of query: infer the type of an expression
fn define_type_of_query() -> Query {
    query("type_of")
        .input(node_id())
        .output(result(type_type(), diagnostics()))
        .extern_impl() // External - will be implemented in Rust
        .build()
}

// Helper to create Result<T, Diagnostics> type
fn result(ok: Type, err: Type) -> Type {
    Type::Result(Box::new(ok), Box::new(err))
}

fn diagnostics() -> Type {
    Type::Diagnostics
}

fn node_id() -> Type {
    Type::NodeId
}

fn value_type() -> Type {
    Type::Value
}

fn type_type() -> Type {
    Type::Type
}

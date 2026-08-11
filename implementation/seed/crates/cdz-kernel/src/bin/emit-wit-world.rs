//! Emit a reducer WIT-world artifact (the `KIND_WIT_WORLD` binary-AST bytes, `db.wit_world`) to a file
//! or stdout — the materialization step the nix derivation runs to feed `rcdzc`'s
//! `wit-world:reducer-world=<path>` input when precompiling a reducer to a bytes-provider component
//! (DESIGN-compiler-platform-separation §3b; the full-A world-driven emit reads this world).
//!
//! The bytes come from the shared `cadenza-ast` world builders (`cdz_kernel::ast_marshal`), so they are
//! byte-identical to v-syntax's inline declaration and v-cml's emit-side read by construction.
//!
//! Usage: `emit-wit-world <pure|full> [out-path]`
//!   - `pure` — the pure-fold world (export `fold.apply` only, no `kv`) for the pure-genesis intermediate.
//!   - `full` — the scope-A world (`fold.apply` + `kv` get/put).
//!   - no `out-path` → write to stdout.

use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let which = args.next();
    let out = args.next();
    let bytes = match which.as_deref() {
        Some("pure") => cdz_kernel::ast_marshal::pure_fold_world_artifact(),
        Some("full") => cdz_kernel::ast_marshal::reducer_world_artifact(),
        _ => {
            eprintln!("usage: emit-wit-world <pure|full> [out-path]  (stdout if no out-path)");
            std::process::exit(2);
        }
    };
    match out {
        Some(path) => std::fs::write(&path, &bytes).unwrap_or_else(|e| {
            eprintln!("emit-wit-world: write {path}: {e}");
            std::process::exit(1);
        }),
        None => std::io::stdout()
            .write_all(&bytes)
            .expect("emit-wit-world: write stdout"),
    }
}
